//! Base command handler implementation

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::events::*;
use crate::packets::BinaryReqType;
use crate::parsing::{hex_decode, hex_encode, to_microdegrees};
use crate::reader::MessageReader;
use crate::{Error, Result, CHANNEL_NAME_LEN, CHANNEL_SECRET_LEN};

/// Default command timeout
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

// Command byte constants (from MeshCore firmware)
const CMD_APP_START: u8 = 1;
const CMD_SEND_TXT_MSG: u8 = 2;
const CMD_SEND_CHANNEL_TXT_MSG: u8 = 3;
const CMD_GET_CONTACTS: u8 = 4;
const CMD_GET_DEVICE_TIME: u8 = 5;
const CMD_SET_DEVICE_TIME: u8 = 6;
const CMD_SEND_SELF_ADVERT: u8 = 7;
const CMD_SET_ADVERT_NAME: u8 = 8;
const CMD_ADD_UPDATE_CONTACT: u8 = 9;
const CMD_SYNC_NEXT_MESSAGE: u8 = 10;
#[allow(dead_code)]
const CMD_SET_RADIO_PARAMS: u8 = 11;
const CMD_SET_RADIO_TX_POWER: u8 = 12;
#[allow(dead_code)]
const CMD_RESET_PATH: u8 = 13;
const CMD_SET_ADVERT_LATLON: u8 = 14;
const CMD_REMOVE_CONTACT: u8 = 15;
#[allow(dead_code)]
const CMD_SHARE_CONTACT: u8 = 16;
const CMD_EXPORT_CONTACT: u8 = 17;
const CMD_IMPORT_CONTACT: u8 = 18;
const CMD_REBOOT: u8 = 19;
const CMD_GET_BATT_AND_STORAGE: u8 = 20;
#[allow(dead_code)]
const CMD_SET_TUNING_PARAMS: u8 = 21;
const CMD_DEVICE_QUERY: u8 = 22;
const CMD_EXPORT_PRIVATE_KEY: u8 = 23;
const CMD_IMPORT_PRIVATE_KEY: u8 = 24;
#[allow(dead_code)]
const CMD_SEND_RAW_DATA: u8 = 25;
const CMD_SEND_LOGIN: u8 = 26;
#[allow(dead_code)]
const CMD_SEND_STATUS_REQ: u8 = 27;
#[allow(dead_code)]
const CMD_HAS_CONNECTION: u8 = 28;
const CMD_LOGOUT: u8 = 29;
#[allow(dead_code)]
const CMD_GET_CONTACT_BY_KEY: u8 = 30;
const CMD_GET_CHANNEL: u8 = 31;
const CMD_SET_CHANNEL: u8 = 32;
const CMD_SIGN_START: u8 = 33;
const CMD_SIGN_DATA: u8 = 34;
const CMD_SIGN_FINISH: u8 = 35;
const CMD_GET_CUSTOM_VARS: u8 = 40;
const CMD_SET_CUSTOM_VAR: u8 = 41;
const CMD_SEND_BINARY_REQ: u8 = 50;
const CMD_SET_FLOOD_SCOPE: u8 = 54;

/// Destination type for commands
#[derive(Debug, Clone)]
pub enum Destination {
    /// Raw bytes (6 or 32 bytes)
    Bytes(Vec<u8>),
    /// Hex string
    Hex(String),
    /// Contact reference
    Contact(Contact),
}

impl Destination {
    /// Get the 6-byte prefix
    pub fn prefix(&self) -> Result<[u8; 6]> {
        match self {
            Destination::Bytes(b) => {
                if b.len() >= 6 {
                    let mut prefix = [0u8; 6];
                    prefix.copy_from_slice(&b[..6]);
                    Ok(prefix)
                } else {
                    Err(Error::invalid_param("Destination too short"))
                }
            }
            Destination::Hex(s) => {
                let bytes = hex_decode(s)?;
                if bytes.len() >= 6 {
                    let mut prefix = [0u8; 6];
                    prefix.copy_from_slice(&bytes[..6]);
                    Ok(prefix)
                } else {
                    Err(Error::invalid_param("Destination too short"))
                }
            }
            Destination::Contact(c) => Ok(c.prefix()),
        }
    }

    /// Get the full public key if available (32 bytes)
    pub fn public_key(&self) -> Option<[u8; 32]> {
        match self {
            Destination::Bytes(b) if b.len() >= 32 => {
                let mut key = [0u8; 32];
                key.copy_from_slice(&b[..32]);
                Some(key)
            }
            Destination::Hex(s) => {
                let bytes = hex_decode(s).ok()?;
                if bytes.len() >= 32 {
                    let mut key = [0u8; 32];
                    key.copy_from_slice(&bytes[..32]);
                    Some(key)
                } else {
                    None
                }
            }
            Destination::Contact(c) => Some(c.public_key),
            _ => None,
        }
    }
}

impl From<&[u8]> for Destination {
    fn from(bytes: &[u8]) -> Self {
        Destination::Bytes(bytes.to_vec())
    }
}

impl From<Vec<u8>> for Destination {
    fn from(bytes: Vec<u8>) -> Self {
        Destination::Bytes(bytes)
    }
}

impl From<&str> for Destination {
    fn from(s: &str) -> Self {
        Destination::Hex(s.to_string())
    }
}

impl From<String> for Destination {
    fn from(s: String) -> Self {
        Destination::Hex(s)
    }
}

impl From<Contact> for Destination {
    fn from(c: Contact) -> Self {
        Destination::Contact(c)
    }
}

impl From<&Contact> for Destination {
    fn from(c: &Contact) -> Self {
        Destination::Contact(c.clone())
    }
}

/// Command handler for MeshCore operations
pub struct CommandHandler {
    /// Sender channel for outgoing data
    sender: mpsc::Sender<Vec<u8>>,
    /// Event dispatcher for receiving responses
    dispatcher: Arc<EventDispatcher>,
    /// Message reader for binary request tracking
    reader: Arc<MessageReader>,
    /// Default timeout for commands
    default_timeout: Duration,
}

impl CommandHandler {
    /// Create a new command handler
    pub fn new(
        sender: mpsc::Sender<Vec<u8>>,
        dispatcher: Arc<EventDispatcher>,
        reader: Arc<MessageReader>,
    ) -> Self {
        Self {
            sender,
            dispatcher,
            reader,
            default_timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Set the default timeout for commands
    pub fn set_default_timeout(&mut self, timeout: Duration) {
        self.default_timeout = timeout;
    }

    /// Send raw data and wait for a response
    pub async fn send(
        &self,
        data: &[u8],
        expected_event: Option<EventType>,
    ) -> Result<MeshCoreEvent> {
        self.send_with_timeout(data, expected_event, self.default_timeout)
            .await
    }

    /// Send raw data and wait for a response with the custom timeout
    pub async fn send_with_timeout(
        &self,
        data: &[u8],
        expected_event: Option<EventType>,
        timeout: Duration,
    ) -> Result<MeshCoreEvent> {
        // Send the data
        self.sender
            .send(data.to_vec())
            .await
            .map_err(|e| Error::Channel(e.to_string()))?;

        // Wait for response
        self.wait_for_event(expected_event, HashMap::new(), timeout)
            .await
    }

    /// Send raw data and wait for one of multiple response types
    pub async fn send_multi(
        &self,
        data: &[u8],
        expected_events: &[EventType],
        timeout: Duration,
    ) -> Result<MeshCoreEvent> {
        // Send the data
        self.sender
            .send(data.to_vec())
            .await
            .map_err(|e| Error::Channel(e.to_string()))?;

        // Wait for any of the expected events
        self.wait_for_any_event(expected_events, timeout).await
    }

    /// Wait for a specific event
    pub async fn wait_for_event(
        &self,
        event_type: Option<EventType>,
        filters: HashMap<String, String>,
        timeout: Duration,
    ) -> Result<MeshCoreEvent> {
        self.dispatcher
            .wait_for_event(event_type, filters, timeout)
            .await
            .ok_or_else(|| Error::timeout(format!("{:?}", event_type)))
    }

    /// Wait for any of the specified events
    pub async fn wait_for_any_event(
        &self,
        event_types: &[EventType],
        timeout: Duration,
    ) -> Result<MeshCoreEvent> {
        let mut rx = self.dispatcher.receiver();

        tokio::select! {
            _ = tokio::time::sleep(timeout) => {
                Err(Error::timeout("response"))
            }
            result = async {
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            if event_types.contains(&event.event_type) {
                                return Ok(event);
                            }
                        }
                        Err(_) => return Err(Error::Channel("Receiver closed".to_string())),
                    }
                }
            } => result,
        }
    }

    // ========== Device Commands ==========

    /// Send APPSTART command to initialise connection
    ///
    /// Format: [CMD_APP_START=0x01][reserved: 7 bytes][app_name: "mccli"]
    pub async fn send_appstart(&self) -> Result<SelfInfo> {
        // Byte 0: CMD_APP_START (0x01)
        // Bytes 1-7: reserved (zeros)
        // Bytes 8+: app name
        let data = [
            CMD_APP_START,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00, // reserved
            b'm',
            b'c',
            b'c',
            b'l',
            b'i', // app name TODO review this
        ];
        let event = self.send(&data, Some(EventType::SelfInfo)).await?;

        match event.payload {
            EventPayload::SelfInfo(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to APPSTART")),
        }
    }

    /// Query device info
    ///
    /// Format: [CMD_DEVICE_QUERY=0x16][protocol_version]
    pub async fn send_device_query(&self) -> Result<DeviceInfoData> {
        // Protocol version 8 is the current version
        let data = [CMD_DEVICE_QUERY, 8];
        let event = self.send(&data, Some(EventType::DeviceInfo)).await?;

        match event.payload {
            EventPayload::DeviceInfo(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to device query")),
        }
    }

    /// Get battery level and storage info
    ///
    /// Format: [CMD_GET_BATT_AND_STORAGE=0x14]
    pub async fn get_bat(&self) -> Result<BatteryInfo> {
        let data = [CMD_GET_BATT_AND_STORAGE];
        let event = self.send(&data, Some(EventType::Battery)).await?;

        match event.payload {
            EventPayload::Battery(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to battery query")),
        }
    }

    /// Get device time
    ///
    /// Format: [CMD_GET_DEVICE_TIME=0x05]
    pub async fn get_time(&self) -> Result<u32> {
        let data = [CMD_GET_DEVICE_TIME];
        let event = self.send(&data, Some(EventType::CurrentTime)).await?;

        match event.payload {
            EventPayload::Time(t) => Ok(t),
            _ => Err(Error::protocol("Unexpected response to time query")),
        }
    }

    /// Set device time
    ///
    /// Format: [CMD_SET_DEVICE_TIME=0x06][timestamp: u32]
    pub async fn set_time(&self, timestamp: u32) -> Result<MeshCoreEvent> {
        let mut data = vec![CMD_SET_DEVICE_TIME];
        data.extend_from_slice(&timestamp.to_le_bytes());
        self.send(&data, Some(EventType::Ok)).await
    }

    /// Set the device name
    ///
    /// Format: [CMD_SET_ADVERT_NAME=0x08][name]
    pub async fn set_name(&self, name: &str) -> Result<MeshCoreEvent> {
        let mut data = vec![CMD_SET_ADVERT_NAME];
        data.extend_from_slice(name.as_bytes());
        self.send(&data, Some(EventType::Ok)).await
    }

    /// Set device coordinates
    ///
    /// Format: [CMD_SET_ADVERT_LATLON=0x0E][lat: i32][lon: i32][alt: i32]
    pub async fn set_coords(&self, lat: f64, lon: f64) -> Result<MeshCoreEvent> {
        let lat_micro = to_microdegrees(lat);
        let lon_micro = to_microdegrees(lon);

        let mut data = vec![CMD_SET_ADVERT_LATLON];
        data.extend_from_slice(&lat_micro.to_le_bytes());
        data.extend_from_slice(&lon_micro.to_le_bytes());
        // Alt is optional, firmware handles len >= 9
        self.send(&data, Some(EventType::Ok)).await
    }

    /// Set TX power
    ///
    /// Format: [CMD_SET_RADIO_TX_POWER=0x0C][power: u8]
    pub async fn set_tx_power(&self, power: u8) -> Result<MeshCoreEvent> {
        let data = [CMD_SET_RADIO_TX_POWER, power];
        self.send(&data, Some(EventType::Ok)).await
    }

    /// Send advertisement
    ///
    /// Format: [CMD_SEND_SELF_ADVERT=0x07][flood: optional]
    pub async fn send_advert(&self, flood: bool) -> Result<MeshCoreEvent> {
        let data = if flood {
            vec![CMD_SEND_SELF_ADVERT, 0x01]
        } else {
            vec![CMD_SEND_SELF_ADVERT]
        };
        self.send(&data, Some(EventType::Ok)).await
    }

    /// Reboot device (no response expected)
    ///
    /// Format: [CMD_REBOOT=0x13]["reboot"]
    pub async fn reboot(&self) -> Result<()> {
        let data = [CMD_REBOOT, b'r', b'e', b'b', b'o', b'o', b't'];
        self.sender
            .send(data.to_vec())
            .await
            .map_err(|e| Error::Channel(e.to_string()))
    }

    /// Get custom variables
    ///
    /// Format: [CMD_GET_CUSTOM_VARS=0x28]
    pub async fn get_custom_vars(&self) -> Result<HashMap<String, String>> {
        let data = [CMD_GET_CUSTOM_VARS];
        let event = self.send(&data, Some(EventType::CustomVars)).await?;

        match event.payload {
            EventPayload::CustomVars(vars) => Ok(vars),
            _ => Err(Error::protocol("Unexpected response to custom vars query")),
        }
    }

    /// Set a custom variable
    ///
    /// Format: [CMD_SET_CUSTOM_VAR=0x29][key=value]
    pub async fn set_custom_var(&self, key: &str, value: &str) -> Result<()> {
        let mut data = vec![CMD_SET_CUSTOM_VAR];
        data.extend_from_slice(key.as_bytes());
        data.push(b'=');
        data.extend_from_slice(value.as_bytes());
        self.send(&data, Some(EventType::Ok)).await?;
        Ok(())
    }

    /// Get channel info
    ///
    /// Format: [CMD_GET_CHANNEL=0x1F][channel_idx: u8]
    pub async fn get_channel(&self, channel_idx: u8) -> Result<ChannelInfoData> {
        let data = [CMD_GET_CHANNEL, channel_idx];
        let event = self.send(&data, Some(EventType::ChannelInfo)).await?;

        match event.payload {
            EventPayload::ChannelInfo(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to channel query")),
        }
    }

    /// Set channel
    ///
    /// Format: [CMD_SET_CHANNEL=0x20][channel_idx][name: CHANNEL_NAME_LEN bytes][secret: CHANNEL_SECRET_LEN bytes]
    /// Note: name is null-terminated, so the max usable length is CHANNEL_NAME_LEN - 1 bytes
    pub async fn set_channel(
        &self,
        channel_idx: u8,
        name: &str,
        secret: &[u8; CHANNEL_SECRET_LEN],
    ) -> Result<()> {
        let mut data = vec![CMD_SET_CHANNEL, channel_idx];
        // Pad or truncate the name to CHANNEL_NAME_LEN bytes, reserving the last byte for null terminator
        let mut name_bytes = [0u8; CHANNEL_NAME_LEN];
        // The Max usable length is CHANNEL_NAME_LEN - 1 to ensure null termination
        let name_len = name.len().min(CHANNEL_NAME_LEN - 1);
        name_bytes[..name_len].copy_from_slice(&name.as_bytes()[..name_len]);
        // name_bytes[name_len..] is already zero (null terminator guaranteed)
        data.extend_from_slice(&name_bytes);
        data.extend_from_slice(secret);
        self.send(&data, Some(EventType::Ok)).await?;
        Ok(())
    }

    /// Set flood scope (region code)
    ///
    /// Format: [CMD_SET_FLOOD_SCOPE=0x36][0][scope: 16 bytes]
    /// Format: [CMD_SET_FLOOD_SCOPE=0x36][0] (to reset flood scope)
    pub async fn set_flood_scope(&self, scope: Option<&str>) -> Result<()> {
        let mut data = vec![CMD_SET_FLOOD_SCOPE, 0];
        if let Some(scope) = scope {
            data.extend_from_slice(scope.as_bytes());
            data.resize(18, 0u8);
        }
        self.send(&data, Some(EventType::Ok)).await?;
        Ok(())
    }

    /// Export private key
    ///
    /// Format: [CMD_EXPORT_PRIVATE_KEY=0x17]
    pub async fn export_private_key(&self) -> Result<[u8; 64]> {
        let data = [CMD_EXPORT_PRIVATE_KEY];
        let event = self
            .send_multi(
                &data,
                &[EventType::PrivateKey, EventType::Disabled],
                self.default_timeout,
            )
            .await?;

        match event.payload {
            EventPayload::PrivateKey(key) => Ok(key),
            EventPayload::String(msg) => Err(Error::Disabled(msg)),
            _ => Err(Error::protocol("Unexpected response to export private key")),
        }
    }

    /// Import private key
    ///
    /// Format: [CMD_IMPORT_PRIVATE_KEY=0x18][key: 64 bytes]
    pub async fn import_private_key(&self, key: &[u8; 64]) -> Result<()> {
        let mut data = vec![CMD_IMPORT_PRIVATE_KEY];
        data.extend_from_slice(key);
        self.send(&data, Some(EventType::Ok)).await?;
        Ok(())
    }

    // ========== Contact Commands ==========

    /// Get the contact list
    pub async fn get_contacts(&self, last_modification_timestamp: u32) -> Result<Vec<Contact>> {
        self.get_contacts_with_timeout(last_modification_timestamp, self.default_timeout)
            .await
    }

    /// Get the contact list with a custom timeout
    ///
    /// Format: [CMD_GET_CONTACTS=0x04][last_mod_timestamp: u32]
    pub async fn get_contacts_with_timeout(
        &self,
        last_modification_timestamp: u32,
        timeout: Duration,
    ) -> Result<Vec<Contact>> {
        let mut data = vec![CMD_GET_CONTACTS];
        data.extend_from_slice(&last_modification_timestamp.to_le_bytes());
        let event = self
            .send_with_timeout(&data, Some(EventType::Contacts), timeout)
            .await?;

        match event.payload {
            EventPayload::Contacts(contacts) => Ok(contacts),
            _ => Err(Error::protocol("Unexpected response to get contacts")),
        }
    }

    /// Add or update a contact
    ///
    /// Format: [CMD_ADD_UPDATE_CONTACT=0x09][pubkey: 32][type: u8][flags: u8][path_len: u8][path: 64][name: 32][timestamp: u32][lat: i32][lon: i32]
    pub async fn add_contact(&self, contact: &Contact) -> Result<()> {
        let mut data = vec![CMD_ADD_UPDATE_CONTACT];
        data.extend_from_slice(&contact.public_key);
        data.push(contact.contact_type);
        data.push(contact.flags);
        data.push(contact.path_len as u8);

        // Pad path to 64 bytes
        let mut path = [0u8; 64];
        let path_len = contact.out_path.len().min(64);
        path[..path_len].copy_from_slice(&contact.out_path[..path_len]);
        data.extend_from_slice(&path);

        // Pad name to 32 bytes
        let mut name = [0u8; 32];
        let name_len = contact.adv_name.len().min(32);
        name[..name_len].copy_from_slice(&contact.adv_name.as_bytes()[..name_len]);
        data.extend_from_slice(&name);

        data.extend_from_slice(&contact.last_advert.to_le_bytes());
        data.extend_from_slice(&contact.adv_lat.to_le_bytes());
        data.extend_from_slice(&contact.adv_lon.to_le_bytes());

        self.send(&data, Some(EventType::Ok)).await?;
        Ok(())
    }

    /// Remove a contact by public key
    ///
    /// Format: [CMD_REMOVE_CONTACT=0x0F][pubkey: 32]
    pub async fn remove_contact(&self, key: impl Into<Destination>) -> Result<()> {
        let dest: Destination = key.into();
        let pubkey = dest.public_key().ok_or_else(|| {
            Error::invalid_param("Remove contact requires full 32-byte public key")
        })?;

        let mut data = vec![CMD_REMOVE_CONTACT];
        data.extend_from_slice(&pubkey);
        self.send(&data, Some(EventType::Ok)).await?;
        Ok(())
    }

    /// Export contact as URI
    ///
    /// Format: [CMD_EXPORT_CONTACT=0x11][pubkey: 32 optional]
    pub async fn export_contact(&self, key: Option<impl Into<Destination>>) -> Result<String> {
        let data = if let Some(k) = key {
            let dest: Destination = k.into();
            let pubkey = dest.public_key().ok_or_else(|| {
                Error::invalid_param("Export contact requires full 32-byte public key")
            })?;
            let mut d = vec![CMD_EXPORT_CONTACT];
            d.extend_from_slice(&pubkey);
            d
        } else {
            vec![CMD_EXPORT_CONTACT]
        };

        let event = self.send(&data, Some(EventType::ContactUri)).await?;

        match event.payload {
            EventPayload::String(uri) => Ok(uri),
            _ => Err(Error::protocol("Unexpected response to export contact")),
        }
    }

    /// Import contact from card data
    ///
    /// Format: [CMD_IMPORT_CONTACT=0x12][card_data]
    pub async fn import_contact(&self, card_data: &[u8]) -> Result<()> {
        let mut data = vec![CMD_IMPORT_CONTACT];
        data.extend_from_slice(card_data);
        self.send(&data, Some(EventType::Ok)).await?;
        Ok(())
    }

    // ========== Messaging Commands ==========

    /// Get the next message from the queue
    ///
    /// Returns the event containing either a `ContactMessage` or `ChannelMessage` payload.
    /// Returns `None` if there are no more messages.
    ///
    /// The caller should check `event.event_type` to determine the message type:
    /// - `EventType::ContactMsgRecv` → `EventPayload::ContactMessage(msg)`
    /// - `EventType::ChannelMsgRecv` → `EventPayload::ChannelMessage(msg)`
    pub async fn get_msg(&self) -> Result<Option<MeshCoreEvent>> {
        self.get_msg_with_timeout(self.default_timeout).await
    }

    /// Get the next message with a custom timeout
    ///
    /// Format: [CMD_SYNC_NEXT_MESSAGE=0x0A]
    pub async fn get_msg_with_timeout(&self, timeout: Duration) -> Result<Option<MeshCoreEvent>> {
        let data = [CMD_SYNC_NEXT_MESSAGE];
        let event = self
            .send_multi(
                &data,
                &[
                    EventType::ContactMsgRecv,
                    EventType::ChannelMsgRecv,
                    EventType::NoMoreMessages,
                    EventType::Error,
                ],
                timeout,
            )
            .await?;

        match event.event_type {
            EventType::ContactMsgRecv | EventType::ChannelMsgRecv => Ok(Some(event)),
            EventType::NoMoreMessages => Ok(None),
            EventType::Error => match event.payload {
                EventPayload::String(msg) => Err(Error::device(msg)),
                _ => Err(Error::device("Unknown error")),
            },
            _ => Err(Error::protocol("Unexpected event type")),
        }
    }

    /// Send a message to a contact
    ///
    /// Format: [CMD_SEND_TXT_MSG=0x02][txt_type][attempt][timestamp: u32][pubkey_prefix: 6][message]
    pub async fn send_msg(
        &self,
        dest: impl Into<Destination>,
        msg: &str,
        timestamp: Option<u32>,
    ) -> Result<MsgSentInfo> {
        let dest: Destination = dest.into();
        let prefix = dest.prefix()?;
        let ts = timestamp.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as u32
        });

        // TXT_TYPE_PLAIN = 0, attempt = 0
        let mut data = vec![CMD_SEND_TXT_MSG, 0x00, 0x00]; // Second 0x00 is "attempt"
        data.extend_from_slice(&ts.to_le_bytes());
        data.extend_from_slice(&prefix);
        data.extend_from_slice(msg.as_bytes());

        let event = self
            .send_with_timeout(&data, Some(EventType::MsgSent), Duration::from_secs(10))
            .await?;

        if event.event_type == EventType::Error {
            return match event.payload {
                EventPayload::String(error_message) => Err(Error::protocol(error_message)),
                _ => Err(Error::protocol("Unexpected response to send_msg")),
            };
        }

        match event.payload {
            EventPayload::MsgSent(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to send_msg")),
        }
    }

    /// Send a channel message
    ///
    /// Format: [CMD_SEND_CHANNEL_TXT_MSG=0x03][txt_type][channel_idx][timestamp: u32][message]
    pub async fn send_channel_msg(
        &self,
        channel: u8,
        msg: &str,
        timestamp: Option<u32>,
    ) -> Result<()> {
        let ts = timestamp.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as u32
        });

        // TXT_TYPE_PLAIN = 0
        let mut data = vec![CMD_SEND_CHANNEL_TXT_MSG, 0x00, channel];
        data.extend_from_slice(&ts.to_le_bytes());
        data.extend_from_slice(msg.as_bytes());

        let _ = self.send(&data, Some(EventType::Ok)).await?;

        Ok(())
    }

    /// Send login request
    ///
    /// Format: [CMD_SEND_LOGIN=0x1A][pubkey: 32][password]
    pub async fn send_login(
        &self,
        dest: impl Into<Destination>,
        password: &str,
    ) -> Result<MsgSentInfo> {
        let dest: Destination = dest.into();
        let pubkey = dest
            .public_key()
            .ok_or_else(|| Error::invalid_param("Login requires full 32-byte public key"))?;

        let mut data = vec![CMD_SEND_LOGIN];
        data.extend_from_slice(&pubkey);
        data.extend_from_slice(password.as_bytes());

        let event = self.send(&data, Some(EventType::MsgSent)).await?;

        match event.payload {
            EventPayload::MsgSent(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to send_login")),
        }
    }

    /// Send logout request
    ///
    /// Format: [CMD_LOGOUT=0x1D][pubkey: 32]
    pub async fn send_logout(&self, dest: impl Into<Destination>) -> Result<()> {
        let dest: Destination = dest.into();
        let pubkey = dest
            .public_key()
            .ok_or_else(|| Error::invalid_param("Logout requires full 32-byte public key"))?;

        let mut data = vec![CMD_LOGOUT];
        data.extend_from_slice(&pubkey);

        self.send(&data, Some(EventType::Ok)).await?;
        Ok(())
    }

    // ========== Binary Commands ==========

    /// Send a binary request to a contact
    ///
    /// Format: [CMD_SEND_BINARY_REQ=0x32][pubkey: 32][req_type]
    /// (Firmware and canonical meshcore_py use pubkey-then-type; see
    /// meshcore_py commands/base.py:244.)
    pub async fn send_binary_req(
        &self,
        dest: impl Into<Destination>,
        req_type: BinaryReqType,
    ) -> Result<MsgSentInfo> {
        let dest: Destination = dest.into();
        let pubkey = dest.public_key().ok_or_else(|| {
            Error::invalid_param("Binary request requires full 32-byte public key")
        })?;

        let mut data = vec![CMD_SEND_BINARY_REQ];
        data.extend_from_slice(&pubkey);
        data.push(req_type as u8);

        let event = self.send(&data, Some(EventType::MsgSent)).await?;

        match event.payload {
            EventPayload::MsgSent(info) => {
                // Register the binary request for response matching
                self.reader
                    .register_binary_request(
                        &info.expected_ack,
                        req_type,
                        pubkey.to_vec(),
                        Duration::from_millis(info.suggested_timeout as u64),
                        HashMap::new(),
                        false,
                    )
                    .await;
                Ok(info)
            }
            _ => Err(Error::protocol("Unexpected response to binary request")),
        }
    }

    /// Request status from a contact
    pub async fn request_status(&self, dest: impl Into<Destination>) -> Result<StatusData> {
        self.request_status_with_timeout(dest, self.default_timeout)
            .await
    }

    /// Request status with the custom timeout
    pub async fn request_status_with_timeout(
        &self,
        dest: impl Into<Destination>,
        timeout: Duration,
    ) -> Result<StatusData> {
        let sent = self.send_binary_req(dest, BinaryReqType::Status).await?;

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), hex_encode(&sent.expected_ack));

        let event = self
            .wait_for_event(Some(EventType::StatusResponse), filters, timeout)
            .await?;

        match event.payload {
            EventPayload::Status(status) => Ok(status),
            _ => Err(Error::protocol("Unexpected response to status request")),
        }
    }

    /// Request telemetry from a contact
    pub async fn request_telemetry(&self, dest: impl Into<Destination>) -> Result<Vec<u8>> {
        self.request_telemetry_with_timeout(dest, self.default_timeout)
            .await
    }

    /// Request telemetry with the custom timeout
    pub async fn request_telemetry_with_timeout(
        &self,
        dest: impl Into<Destination>,
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        let sent = self.send_binary_req(dest, BinaryReqType::Telemetry).await?;

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), hex_encode(&sent.expected_ack));

        let event = self
            .wait_for_event(Some(EventType::TelemetryResponse), filters, timeout)
            .await?;

        match event.payload {
            EventPayload::Telemetry(data) => Ok(data),
            _ => Err(Error::protocol("Unexpected response to telemetry request")),
        }
    }

    /// Request ACL from a contact
    pub async fn request_acl(&self, dest: impl Into<Destination>) -> Result<Vec<AclEntry>> {
        self.request_acl_with_timeout(dest, self.default_timeout)
            .await
    }

    /// Request ACL with the custom timeout
    pub async fn request_acl_with_timeout(
        &self,
        dest: impl Into<Destination>,
        timeout: Duration,
    ) -> Result<Vec<AclEntry>> {
        let sent = self.send_binary_req(dest, BinaryReqType::Acl).await?;

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), hex_encode(&sent.expected_ack));

        let event = self
            .wait_for_event(Some(EventType::AclResponse), filters, timeout)
            .await?;

        match event.payload {
            EventPayload::Acl(entries) => Ok(entries),
            _ => Err(Error::protocol("Unexpected response to ACL request")),
        }
    }

    /// Request neighbours from a contact.
    ///
    /// Defaults: `order_by = 0`, `pubkey_prefix_length = 4` (matches meshcore_py default).
    pub async fn request_neighbours(
        &self,
        dest: impl Into<Destination>,
        count: u8,
        offset: u16,
    ) -> Result<NeighboursData> {
        self.request_neighbours_with_timeout(dest, count, offset, 0, 4, self.default_timeout)
            .await
    }

    /// Request neighbours with custom timeout and options.
    ///
    /// Canonical wire format (matches meshcore_py `commands/binary.py::req_neighbours_async`):
    ///
    /// `[CMD_SEND_BINARY_REQ=0x32][pubkey: 32][type=NEIGHBOURS=0x06]`
    /// `[version: u8 = 0][count: u8][offset: u16 LE][order_by: u8][pk_plen: u8][nonce: u32 LE]`
    ///
    /// The firmware echoes `pk_plen` in the response's per-entry pubkey width;
    /// the reader reads that back via the `pubkey_prefix_length` context attribute
    /// so `parse_neighbours` uses the correct entry stride.
    pub async fn request_neighbours_with_timeout(
        &self,
        dest: impl Into<Destination>,
        count: u8,
        offset: u16,
        order_by: u8,
        pubkey_prefix_length: u8,
        timeout: Duration,
    ) -> Result<NeighboursData> {
        let dest: Destination = dest.into();
        let pubkey = dest.public_key().ok_or_else(|| {
            Error::invalid_param("Neighbours request requires full 32-byte public key")
        })?;

        if pubkey_prefix_length > 32 {
            return Err(Error::invalid_param(
                "pubkey_prefix_length cannot exceed 32",
            ));
        }

        let nonce = {
            let n = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(1);
            if n == 0 {
                1
            } else {
                n
            }
        };

        let mut data = vec![CMD_SEND_BINARY_REQ];
        data.extend_from_slice(&pubkey);
        data.push(BinaryReqType::Neighbours as u8);
        data.push(0u8); // version
        data.push(count);
        data.extend_from_slice(&offset.to_le_bytes());
        data.push(order_by);
        data.push(pubkey_prefix_length);
        data.extend_from_slice(&nonce.to_le_bytes());

        let event = self.send(&data, Some(EventType::MsgSent)).await?;
        let sent = match event.payload {
            EventPayload::MsgSent(info) => info,
            _ => return Err(Error::protocol("Unexpected response to neighbours request")),
        };

        let mut ctx = HashMap::new();
        ctx.insert(
            "pubkey_prefix_length".to_string(),
            pubkey_prefix_length.to_string(),
        );
        self.reader
            .register_binary_request(
                &sent.expected_ack,
                BinaryReqType::Neighbours,
                pubkey.to_vec(),
                timeout,
                ctx,
                false,
            )
            .await;

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), hex_encode(&sent.expected_ack));

        let event = self
            .wait_for_event(Some(EventType::NeighboursResponse), filters, timeout)
            .await?;

        match event.payload {
            EventPayload::Neighbours(data) => Ok(data),
            _ => Err(Error::protocol("Unexpected response to neighbours request")),
        }
    }

    // ========== Signing Commands ==========

    /// Start a signing session
    ///
    /// Format: [CMD_SIGN_START=0x21]
    pub async fn sign_start(&self) -> Result<u32> {
        let data = [CMD_SIGN_START];
        let event = self.send(&data, Some(EventType::SignStart)).await?;

        match event.payload {
            EventPayload::SignStart { max_length } => Ok(max_length),
            _ => Err(Error::protocol("Unexpected response to sign_start")),
        }
    }

    /// Send data chunk for signing
    ///
    /// Format: [CMD_SIGN_DATA=0x22][chunk]
    pub async fn sign_data(&self, chunk: &[u8]) -> Result<()> {
        let mut data = vec![CMD_SIGN_DATA];
        data.extend_from_slice(chunk);
        self.send(&data, Some(EventType::Ok)).await?;
        Ok(())
    }

    /// Finish signing and get the signature
    ///
    /// Format: [CMD_SIGN_FINISH=0x23]
    pub async fn sign_finish(&self, timeout: Duration) -> Result<Vec<u8>> {
        let data = [CMD_SIGN_FINISH];
        let event = self
            .send_with_timeout(&data, Some(EventType::Signature), timeout)
            .await?;

        match event.payload {
            EventPayload::Signature(sig) => Ok(sig),
            _ => Err(Error::protocol("Unexpected response to sign_finish")),
        }
    }

    /// Sign data (high-level helper)
    pub async fn sign(&self, data_to_sign: &[u8], chunk_size: usize) -> Result<Vec<u8>> {
        let max_length = self.sign_start().await?;

        if data_to_sign.len() > max_length as usize {
            return Err(Error::invalid_param(format!(
                "Data too large: {} > {}",
                data_to_sign.len(),
                max_length
            )));
        }

        // Send data in chunks
        for chunk in data_to_sign.chunks(chunk_size) {
            self.sign_data(chunk).await?;
        }

        // Get signature with extended timeout
        let timeout = Duration::from_secs(30);
        self.sign_finish(timeout).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========== Destination Tests ==========

    #[test]
    fn test_destination_from_bytes_slice() {
        let bytes: &[u8] = &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let dest: Destination = bytes.into();
        assert!(matches!(dest, Destination::Bytes(_)));
    }

    #[test]
    fn test_destination_from_vec() {
        let bytes = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        let dest: Destination = bytes.into();
        assert!(matches!(dest, Destination::Bytes(_)));
    }

    #[test]
    fn test_destination_from_str() {
        let dest: Destination = "0102030405060708".into();
        assert!(matches!(dest, Destination::Hex(_)));
    }

    #[test]
    fn test_destination_from_string() {
        let dest: Destination = String::from("0102030405060708").into();
        assert!(matches!(dest, Destination::Hex(_)));
    }

    #[test]
    fn test_destination_from_contact() {
        let contact = Contact {
            public_key: [0xAA; 32],
            contact_type: 1,
            flags: 0,
            path_len: -1,
            out_path: vec![],
            adv_name: "Test".to_string(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: 0,
            last_modification_timestamp: 0,
        };
        let dest: Destination = contact.into();
        assert!(matches!(dest, Destination::Contact(_)));
    }

    #[test]
    fn test_destination_from_contact_ref() {
        let contact = Contact {
            public_key: [0xBB; 32],
            contact_type: 1,
            flags: 0,
            path_len: -1,
            out_path: vec![],
            adv_name: "Test".to_string(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: 0,
            last_modification_timestamp: 0,
        };
        let dest: Destination = (&contact).into();
        assert!(matches!(dest, Destination::Contact(_)));
    }

    #[test]
    fn test_destination_prefix_from_bytes() {
        let bytes = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let dest: Destination = bytes.into();
        let prefix = dest.prefix().unwrap();
        assert_eq!(prefix, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    }

    #[test]
    fn test_destination_prefix_from_bytes_too_short() {
        let bytes = vec![0x01, 0x02, 0x03];
        let dest: Destination = bytes.into();
        assert!(dest.prefix().is_err());
    }

    #[test]
    fn test_destination_prefix_from_hex() {
        let dest: Destination = "010203040506".into();
        let prefix = dest.prefix().unwrap();
        assert_eq!(prefix, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    }

    #[test]
    fn test_destination_prefix_from_hex_too_short() {
        let dest: Destination = "0102".into();
        assert!(dest.prefix().is_err());
    }

    #[test]
    fn test_destination_prefix_from_contact() {
        let mut public_key = [0u8; 32];
        public_key[0..6].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let contact = Contact {
            public_key,
            contact_type: 1,
            flags: 0,
            path_len: -1,
            out_path: vec![],
            adv_name: "Test".to_string(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: 0,
            last_modification_timestamp: 0,
        };
        let dest: Destination = contact.into();
        let prefix = dest.prefix().unwrap();
        assert_eq!(prefix, [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn test_destination_public_key_from_bytes_32() {
        let bytes = vec![0xAA; 32];
        let dest: Destination = bytes.into();
        let key = dest.public_key().unwrap();
        assert_eq!(key, [0xAA; 32]);
    }

    #[test]
    fn test_destination_public_key_from_bytes_short() {
        let bytes = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        let dest: Destination = bytes.into();
        assert!(dest.public_key().is_none());
    }

    #[test]
    fn test_destination_public_key_from_hex_32() {
        // 32 bytes = 64 hex chars
        let hex = "aa".repeat(32);
        let dest: Destination = hex.into();
        let key = dest.public_key().unwrap();
        assert_eq!(key, [0xAA; 32]);
    }

    #[test]
    fn test_destination_public_key_from_hex_short() {
        let dest: Destination = "010203040506".into();
        assert!(dest.public_key().is_none());
    }

    #[test]
    fn test_destination_public_key_from_contact() {
        let contact = Contact {
            public_key: [0xCC; 32],
            contact_type: 1,
            flags: 0,
            path_len: -1,
            out_path: vec![],
            adv_name: "Test".to_string(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: 0,
            last_modification_timestamp: 0,
        };
        let dest: Destination = contact.into();
        let key = dest.public_key().unwrap();
        assert_eq!(key, [0xCC; 32]);
    }

    #[test]
    fn test_destination_clone() {
        let dest = Destination::Hex("0102030405060708".to_string());
        let cloned = dest.clone();
        assert!(matches!(cloned, Destination::Hex(_)));
    }

    #[test]
    fn test_destination_debug() {
        let dest = Destination::Bytes(vec![1, 2, 3]);
        let debug_str = format!("{:?}", dest);
        assert!(debug_str.contains("Bytes"));
    }

    // ========== Constants Tests ==========

    #[test]
    fn test_default_timeout() {
        assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(5));
    }

    #[test]
    fn test_command_constants() {
        assert_eq!(CMD_APP_START, 1);
        assert_eq!(CMD_SEND_TXT_MSG, 2);
        assert_eq!(CMD_SEND_CHANNEL_TXT_MSG, 3);
        assert_eq!(CMD_GET_CONTACTS, 4);
        assert_eq!(CMD_GET_DEVICE_TIME, 5);
        assert_eq!(CMD_SET_DEVICE_TIME, 6);
        assert_eq!(CMD_SEND_SELF_ADVERT, 7);
        assert_eq!(CMD_SET_ADVERT_NAME, 8);
        assert_eq!(CMD_ADD_UPDATE_CONTACT, 9);
        assert_eq!(CMD_SYNC_NEXT_MESSAGE, 10);
        assert_eq!(CMD_SET_RADIO_TX_POWER, 12);
        assert_eq!(CMD_SET_ADVERT_LATLON, 14);
        assert_eq!(CMD_REMOVE_CONTACT, 15);
        assert_eq!(CMD_EXPORT_CONTACT, 17);
        assert_eq!(CMD_IMPORT_CONTACT, 18);
        assert_eq!(CMD_REBOOT, 19);
        assert_eq!(CMD_GET_BATT_AND_STORAGE, 20);
        assert_eq!(CMD_DEVICE_QUERY, 22);
        assert_eq!(CMD_EXPORT_PRIVATE_KEY, 23);
        assert_eq!(CMD_IMPORT_PRIVATE_KEY, 24);
        assert_eq!(CMD_SEND_LOGIN, 26);
        assert_eq!(CMD_LOGOUT, 29);
        assert_eq!(CMD_GET_CHANNEL, 31);
        assert_eq!(CMD_SET_CHANNEL, 32);
        assert_eq!(CMD_SIGN_START, 33);
        assert_eq!(CMD_SIGN_DATA, 34);
        assert_eq!(CMD_SIGN_FINISH, 35);
        assert_eq!(CMD_GET_CUSTOM_VARS, 40);
        assert_eq!(CMD_SET_CUSTOM_VAR, 41);
        assert_eq!(CMD_SEND_BINARY_REQ, 50);
    }

    // ========== CommandHandler Tests with Mock Infrastructure ==========

    fn create_test_handler() -> (
        CommandHandler,
        mpsc::Receiver<Vec<u8>>,
        Arc<EventDispatcher>,
    ) {
        let (sender, receiver) = mpsc::channel(16);
        let dispatcher = Arc::new(EventDispatcher::new());
        let reader = Arc::new(MessageReader::new(dispatcher.clone()));
        let handler = CommandHandler::new(sender, dispatcher.clone(), reader);
        (handler, receiver, dispatcher)
    }

    #[tokio::test]
    async fn test_command_handler_new() {
        let (handler, _rx, _dispatcher) = create_test_handler();
        assert_eq!(handler.default_timeout, DEFAULT_TIMEOUT);
    }

    #[tokio::test]
    async fn test_command_handler_set_default_timeout() {
        let (mut handler, _rx, _dispatcher) = create_test_handler();
        handler.set_default_timeout(Duration::from_secs(10));
        assert_eq!(handler.default_timeout, Duration::from_secs(10));
    }

    #[tokio::test]
    async fn test_command_handler_send_timeout() {
        let (handler, mut rx, _dispatcher) = create_test_handler();

        // Spawn a task to receive the data
        let recv_task = tokio::spawn(async move { rx.recv().await });

        // Send with a short timeout - should the timeout since no response comes
        let result = handler
            .send_with_timeout(&[0x01], Some(EventType::Ok), Duration::from_millis(10))
            .await;

        assert!(result.is_err());

        // Verify data was sent
        let sent = recv_task.await.unwrap();
        assert_eq!(sent, Some(vec![0x01]));
    }

    #[tokio::test]
    async fn test_command_handler_send_with_response() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        // Spawn a task that sends a response
        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            // Wait for the command to be sent
            let _sent = rx.recv().await;
            // Emit the expected response
            dispatcher_clone.emit(MeshCoreEvent::ok()).await;
        });

        let result = handler
            .send_with_timeout(&[0x01], Some(EventType::Ok), Duration::from_millis(100))
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().event_type, EventType::Ok);
    }

    #[tokio::test]
    async fn test_command_handler_wait_for_event_timeout() {
        let (handler, _rx, _dispatcher) = create_test_handler();

        let result = handler
            .wait_for_event(
                Some(EventType::Ok),
                HashMap::new(),
                Duration::from_millis(10),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_command_handler_wait_for_event_success() {
        let (handler, _rx, dispatcher) = create_test_handler();

        // Emit an event
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            dispatcher.emit(MeshCoreEvent::ok()).await;
        });

        let result = handler
            .wait_for_event(
                Some(EventType::Ok),
                HashMap::new(),
                Duration::from_millis(100),
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_command_handler_wait_for_any_event() {
        let (handler, _rx, dispatcher) = create_test_handler();

        // Emit an error event
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            dispatcher.emit(MeshCoreEvent::error("test")).await;
        });

        let result = handler
            .wait_for_any_event(
                &[EventType::Ok, EventType::Error],
                Duration::from_millis(100),
            )
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().event_type, EventType::Error);
    }

    #[tokio::test]
    async fn test_command_handler_wait_for_any_event_timeout() {
        let (handler, _rx, _dispatcher) = create_test_handler();

        let result = handler
            .wait_for_any_event(
                &[EventType::Ok, EventType::Error],
                Duration::from_millis(10),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_command_handler_send_multi() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        // Spawn responder
        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let _sent = rx.recv().await;
            dispatcher_clone
                .emit(MeshCoreEvent::error("device busy"))
                .await;
        });

        let result = handler
            .send_multi(
                &[0x01],
                &[EventType::Ok, EventType::Error],
                Duration::from_millis(100),
            )
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().event_type, EventType::Error);
    }

    #[tokio::test]
    async fn test_send_appstart_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            // Verify the APPSTART command format
            assert_eq!(sent[0], CMD_APP_START);
            assert_eq!(&sent[8..13], b"mccli");

            // Send SelfInfo response
            let info = SelfInfo {
                adv_type: 1,
                tx_power: 20,
                max_tx_power: 30,
                public_key: [0; 32],
                adv_lat: 0,
                adv_lon: 0,
                multi_acks: 0,
                adv_loc_policy: 0,
                telemetry_mode_base: 0,
                telemetry_mode_loc: 0,
                telemetry_mode_env: 0,
                manual_add_contacts: false,
                radio_freq: 915000000,
                radio_bw: 125000,
                sf: 7,
                cr: 5,
                name: "TestDevice".to_string(),
            };
            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::SelfInfo,
                    EventPayload::SelfInfo(info),
                ))
                .await;
        });

        let result = handler.send_appstart().await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.name, "TestDevice");
        assert_eq!(info.tx_power, 20);
    }

    #[tokio::test]
    async fn test_get_bat_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_GET_BATT_AND_STORAGE);

            let info = BatteryInfo {
                battery_mv: 4200,
                used_kb: Some(512),
                total_kb: Some(4096),
            };
            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::Battery,
                    EventPayload::Battery(info),
                ))
                .await;
        });

        let result = handler.get_bat().await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.battery_mv, 4200);
        assert!((info.voltage() - 4.2).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_get_time_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_GET_DEVICE_TIME);

            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::CurrentTime,
                    EventPayload::Time(1234567890),
                ))
                .await;
        });

        let result = handler.get_time().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1234567890);
    }

    #[tokio::test]
    async fn test_set_time_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_SET_DEVICE_TIME);
            // Verify the timestamp is included
            let ts = u32::from_le_bytes([sent[1], sent[2], sent[3], sent[4]]);
            assert_eq!(ts, 1234567890);

            dispatcher_clone.emit(MeshCoreEvent::ok()).await;
        });

        let result = handler.set_time(1234567890).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_name_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_SET_ADVERT_NAME);
            assert_eq!(&sent[1..], b"MyNode");

            dispatcher_clone.emit(MeshCoreEvent::ok()).await;
        });

        let result = handler.set_name("MyNode").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_coords_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_SET_ADVERT_LATLON);
            // Verify coordinates are present (lat + lon = 8 bytes after command)
            assert!(sent.len() >= 9);

            dispatcher_clone.emit(MeshCoreEvent::ok()).await;
        });

        let result = handler.set_coords(37.7749, -122.4194).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_tx_power_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_SET_RADIO_TX_POWER);
            assert_eq!(sent[1], 20);

            dispatcher_clone.emit(MeshCoreEvent::ok()).await;
        });

        let result = handler.set_tx_power(20).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_advert_flood() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_SEND_SELF_ADVERT);
            assert_eq!(sent[1], 0x01); // flood flag

            dispatcher_clone.emit(MeshCoreEvent::ok()).await;
        });

        let result = handler.send_advert(true).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_advert_no_flood() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_SEND_SELF_ADVERT);
            assert_eq!(sent.len(), 1); // no flood flag

            dispatcher_clone.emit(MeshCoreEvent::ok()).await;
        });

        let result = handler.send_advert(false).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_reboot() {
        let (handler, mut rx, _dispatcher) = create_test_handler();

        let recv_task = tokio::spawn(async move { rx.recv().await });

        let result = handler.reboot().await;
        assert!(result.is_ok());

        let sent = recv_task.await.unwrap().unwrap();
        assert_eq!(sent[0], CMD_REBOOT);
        assert_eq!(&sent[1..], b"reboot");
    }

    #[tokio::test]
    async fn test_get_contacts_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_GET_CONTACTS);

            let contacts = vec![Contact {
                public_key: [0xAA; 32],
                contact_type: 1,
                flags: 0,
                path_len: 2,
                out_path: vec![],
                adv_name: "Contact1".to_string(),
                last_advert: 0,
                adv_lat: 0,
                adv_lon: 0,
                last_modification_timestamp: 0,
            }];
            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::Contacts,
                    EventPayload::Contacts(contacts),
                ))
                .await;
        });

        let result = handler.get_contacts(0).await;
        assert!(result.is_ok());
        let contacts = result.unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].adv_name, "Contact1");
    }

    #[tokio::test]
    async fn test_export_contact_self() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_EXPORT_CONTACT);
            assert_eq!(sent.len(), 1); // no pubkey for self

            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::ContactUri,
                    EventPayload::String("mod.rs://...".to_string()),
                ))
                .await;
        });

        let result: Result<String> = handler.export_contact(None::<&str>).await;
        assert!(result.is_ok());
        assert!(result.unwrap().starts_with("mod.rs://"));
    }

    #[tokio::test]
    async fn test_export_contact_with_key_sends_full_public_key() {
        // Regression test: this used to send only a 6-byte prefix, despite
        // the documented wire format requiring the full 32-byte public key.
        let (handler, mut rx, dispatcher) = create_test_handler();
        let pubkey_hex = "bb".repeat(32);

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_EXPORT_CONTACT);
            assert_eq!(sent.len(), 1 + 32);
            assert_eq!(&sent[1..], [0xbbu8; 32].as_slice());

            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::ContactUri,
                    EventPayload::String("mc://...".to_string()),
                ))
                .await;
        });

        let result: Result<String> = handler.export_contact(Some(pubkey_hex.as_str())).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_export_contact_with_prefix_only_errors() {
        let (handler, _rx, _dispatcher) = create_test_handler();
        // Only a 6-byte prefix (12 hex chars) -- not enough for the
        // required 32-byte public key.
        let result: Result<String> = handler.export_contact(Some("aabbccddeeff")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_remove_contact_sends_full_public_key() {
        // Regression test: this used to send only a 6-byte prefix, despite
        // the documented wire format requiring the full 32-byte public key
        // -- causing the real firmware to never respond (timeout).
        let (handler, mut rx, dispatcher) = create_test_handler();
        let pubkey_hex = "aa".repeat(32);

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_REMOVE_CONTACT);
            assert_eq!(sent.len(), 1 + 32);
            assert_eq!(&sent[1..], [0xaau8; 32].as_slice());

            dispatcher_clone
                .emit(MeshCoreEvent::new(EventType::Ok, EventPayload::None))
                .await;
        });

        let result = handler.remove_contact(pubkey_hex.as_str()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_remove_contact_with_prefix_only_errors() {
        let (handler, _rx, _dispatcher) = create_test_handler();
        let result = handler.remove_contact("aabbccddeeff").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_channel_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_GET_CHANNEL);
            assert_eq!(sent[1], 0); // channel idx

            let info = ChannelInfoData {
                channel_idx: 0,
                name: "General".to_string(),
                secret: [0; 16],
            };
            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::ChannelInfo,
                    EventPayload::ChannelInfo(info),
                ))
                .await;
        });

        let result = handler.get_channel(0).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().name, "General");
    }

    #[tokio::test]
    async fn test_set_channel_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_SET_CHANNEL);
            assert_eq!(sent[1], 1); // channel_idx
                                    // Verify name is padded to CHANNEL_NAME_LEN bytes
            assert_eq!(sent.len(), 1 + 1 + CHANNEL_NAME_LEN + CHANNEL_SECRET_LEN);
            // Check name starts with "Test"
            assert_eq!(&sent[2..6], b"Test");
            // Check the rest of the name is zero-padded
            assert!(sent[6..2 + CHANNEL_NAME_LEN].iter().all(|&b| b == 0));
            // Check secret
            assert_eq!(&sent[2 + CHANNEL_NAME_LEN..], &[0xAA; CHANNEL_SECRET_LEN]);

            dispatcher_clone
                .emit(MeshCoreEvent::new(EventType::Ok, EventPayload::None))
                .await;
        });

        let secret = [0xAA; CHANNEL_SECRET_LEN];
        let result = handler.set_channel(1, "Test", &secret).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_channel_name_truncation_with_null_terminator() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_SET_CHANNEL);
            // Verify the total length is correct
            assert_eq!(sent.len(), 1 + 1 + CHANNEL_NAME_LEN + CHANNEL_SECRET_LEN);
            // Name should be truncated to CHANNEL_NAME_LEN - 1 byte to leave room for null
            let expected_name = b"This is a very long channel nam"; // 31 bytes
            assert_eq!(
                &sent[2..2 + CHANNEL_NAME_LEN - 1],
                &expected_name[..CHANNEL_NAME_LEN - 1]
            );
            // The last byte of the name field must be null terminator
            assert_eq!(
                sent[2 + CHANNEL_NAME_LEN - 1],
                0,
                "Last byte of name field must be null terminator"
            );

            dispatcher_clone
                .emit(MeshCoreEvent::new(EventType::Ok, EventPayload::None))
                .await;
        });

        let secret = [0xBB; CHANNEL_SECRET_LEN];
        // Name longer than CHANNEL_NAME_LEN - 1 should be truncated to ensure null termination
        let long_name = "This is a very long channel name that exceeds the limit";
        let result = handler.set_channel(2, long_name, &secret).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_sign_start_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_SIGN_START);

            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::SignStart,
                    EventPayload::SignStart { max_length: 4096 },
                ))
                .await;
        });

        let result = handler.sign_start().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 4096);
    }

    #[tokio::test]
    async fn test_get_custom_vars_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_GET_CUSTOM_VARS);

            let mut vars = HashMap::new();
            vars.insert("key1".to_string(), "value1".to_string());
            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::CustomVars,
                    EventPayload::CustomVars(vars),
                ))
                .await;
        });

        let result = handler.get_custom_vars().await;
        assert!(result.is_ok());
        let vars = result.unwrap();
        assert_eq!(vars.get("key1"), Some(&"value1".to_string()));
    }

    #[tokio::test]
    async fn test_set_custom_var_success() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_SET_CUSTOM_VAR);
            // Should contain "key=value"
            let payload = String::from_utf8_lossy(&sent[1..]);
            assert!(payload.contains("mykey=myvalue"));

            dispatcher_clone.emit(MeshCoreEvent::ok()).await;
        });

        let result = handler.set_custom_var("mykey", "myvalue").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_get_msg_no_more() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            assert_eq!(sent[0], CMD_SYNC_NEXT_MESSAGE);

            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::NoMoreMessages,
                    EventPayload::None,
                ))
                .await;
        });

        let result = handler.get_msg().await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_get_msg_with_message() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let _sent = rx.recv().await.unwrap();

            let msg = ContactMessage {
                sender_prefix: [0x01, 0x02, 0x03, 0x04, 0x05, 0x06],
                path_len: 2,
                txt_type: 1,
                sender_timestamp: 1234567890,
                text: "Hello!".to_string(),
                snr: None,
                signature: None,
            };
            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::ContactMsgRecv,
                    EventPayload::ContactMessage(msg),
                ))
                .await;
        });

        let result = handler.get_msg().await;
        assert!(result.is_ok());
        let event = result.unwrap().unwrap();
        assert_eq!(event.event_type, EventType::ContactMsgRecv);
        match event.payload {
            EventPayload::ContactMessage(msg) => {
                assert_eq!(msg.text, "Hello!");
            }
            _ => panic!("Expected ContactMessage payload"),
        }
    }

    #[tokio::test]
    async fn test_send_binary_req_field_order() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            // Verify wire format: [CMD=0x32][pubkey:32][req_type]
            assert_eq!(sent[0], CMD_SEND_BINARY_REQ);
            // Bytes 1..33 should be the pubkey
            assert_eq!(&sent[1..33], &[0xAA; 32]);
            // Byte 33 should be the request type (Telemetry = 0x03)
            assert_eq!(sent[33], BinaryReqType::Telemetry as u8);

            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::MsgSent,
                    EventPayload::MsgSent(MsgSentInfo {
                        message_type: 0,
                        expected_ack: [0x01, 0x02, 0x03, 0x04],
                        suggested_timeout: 5000,
                    }),
                ))
                .await;
        });

        let dest = vec![0xAAu8; 32];
        let result = handler
            .send_binary_req(dest, BinaryReqType::Telemetry)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_request_neighbours_wire_format() {
        let (handler, mut rx, dispatcher) = create_test_handler();

        let dispatcher_clone = dispatcher.clone();
        tokio::spawn(async move {
            let sent = rx.recv().await.unwrap();
            // Wire format: [CMD=0x32][pubkey:32][type=NEIGHBOURS=0x06]
            //              [version:u8=0][count:u8][offset:u16 LE]
            //              [order_by:u8][pk_plen:u8][nonce:u32 LE]
            assert_eq!(sent[0], CMD_SEND_BINARY_REQ);
            assert_eq!(&sent[1..33], &[0xBB; 32]); // pubkey
            assert_eq!(sent[33], BinaryReqType::Neighbours as u8); // 0x06
            assert_eq!(sent[34], 0); // version
            assert_eq!(sent[35], 10); // count
            assert_eq!(u16::from_le_bytes([sent[36], sent[37]]), 5); // offset
            assert_eq!(sent[38], 2); // order_by
            assert_eq!(sent[39], 4); // pubkey_prefix_length
            let nonce = u32::from_le_bytes([sent[40], sent[41], sent[42], sent[43]]);
            assert_ne!(nonce, 0);
            assert_eq!(sent.len(), 44);

            // Subscribe for the response *before* emitting MsgSent, so we
            // don't miss the NeighboursResponse window.
            let mut resp_rx = dispatcher_clone.receiver();

            dispatcher_clone
                .emit(MeshCoreEvent::new(
                    EventType::MsgSent,
                    EventPayload::MsgSent(MsgSentInfo {
                        message_type: 0,
                        expected_ack: [0x01, 0x02, 0x03, 0x04],
                        suggested_timeout: 5000,
                    }),
                ))
                .await;

            // Wait until the caller has registered the binary request (it
            // emits MsgSent handling internally, then registers, then waits).
            // We detect readiness by watching for any activity after MsgSent.
            while let Ok(ev) = tokio::time::timeout(Duration::from_millis(50), resp_rx.recv()).await
            {
                if let Ok(e) = ev {
                    if e.event_type == EventType::MsgSent {
                        continue;
                    }
                }
            }

            dispatcher_clone
                .emit(
                    MeshCoreEvent::new(
                        EventType::NeighboursResponse,
                        EventPayload::Neighbours(NeighboursData {
                            total: 0,
                            neighbours: vec![],
                        }),
                    )
                    .with_attribute("tag", "01020304".to_string()),
                )
                .await;
        });

        let dest = vec![0xBBu8; 32];
        let result = handler
            .request_neighbours_with_timeout(dest, 10, 5, 2, 4, Duration::from_millis(500))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_request_neighbours_invalid_pubkey_prefix_length() {
        let (handler, _rx, _dispatcher) = create_test_handler();
        let dest = vec![0xAAu8; 32];
        let result = handler
            .request_neighbours_with_timeout(dest, 10, 0, 0, 33, Duration::from_millis(100))
            .await;
        assert!(result.is_err());
    }
}
