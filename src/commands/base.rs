//! Base command handler implementation

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::events::*;
use crate::packets::BinaryReqType;
use crate::parsing::{hex_decode, hex_encode, to_microdegrees};
use crate::reader::MessageReader;
use crate::Error;
use crate::Result;

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
    pub async fn send(&self, data: &[u8], expected_event: EventType) -> Result<Event> {
        self.send_with_timeout(data, expected_event, self.default_timeout)
            .await
    }

    /// Send raw data and wait for a response with the custom timeout
    pub async fn send_with_timeout(
        &self,
        data: &[u8],
        expected_event: EventType,
        timeout: Duration,
    ) -> Result<Event> {
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
    ) -> Result<Event> {
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
        event_type: EventType,
        filters: HashMap<String, String>,
        timeout: Duration,
    ) -> Result<Event> {
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
    ) -> Result<Event> {
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

    /// Send APPSTART command to initialize connection
    ///
    /// Format: [CMD_APP_START=0x01][reserved: 7 bytes][app_name: "mccli"]
    pub async fn send_appstart(&self) -> Result<SelfInfo> {
        // Byte 0: CMD_APP_START (0x01)
        // Bytes 1-7: reserved (zeros)
        // Bytes 8+: app name
        let data = [
            CMD_APP_START,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // reserved
            b'm', b'c', b'c', b'l', b'i', // app name
        ];
        let event = self.send(&data, EventType::SelfInfo).await?;

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
        let event = self.send(&data, EventType::DeviceInfo).await?;

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
        let event = self.send(&data, EventType::Battery).await?;

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
        let event = self.send(&data, EventType::CurrentTime).await?;

        match event.payload {
            EventPayload::Time(t) => Ok(t),
            _ => Err(Error::protocol("Unexpected response to time query")),
        }
    }

    /// Set device time
    ///
    /// Format: [CMD_SET_DEVICE_TIME=0x06][timestamp: u32]
    pub async fn set_time(&self, timestamp: u32) -> Result<()> {
        let mut data = vec![CMD_SET_DEVICE_TIME];
        data.extend_from_slice(&timestamp.to_le_bytes());
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Set the device name
    ///
    /// Format: [CMD_SET_ADVERT_NAME=0x08][name]
    pub async fn set_name(&self, name: &str) -> Result<()> {
        let mut data = vec![CMD_SET_ADVERT_NAME];
        data.extend_from_slice(name.as_bytes());
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Set device coordinates
    ///
    /// Format: [CMD_SET_ADVERT_LATLON=0x0E][lat: i32][lon: i32][alt: i32]
    pub async fn set_coords(&self, lat: f64, lon: f64) -> Result<()> {
        let lat_micro = to_microdegrees(lat);
        let lon_micro = to_microdegrees(lon);

        let mut data = vec![CMD_SET_ADVERT_LATLON];
        data.extend_from_slice(&lat_micro.to_le_bytes());
        data.extend_from_slice(&lon_micro.to_le_bytes());
        // Alt is optional, firmware handles len >= 9
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Set TX power
    ///
    /// Format: [CMD_SET_RADIO_TX_POWER=0x0C][power: u8]
    pub async fn set_tx_power(&self, power: u8) -> Result<()> {
        let data = [CMD_SET_RADIO_TX_POWER, power];
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Send advertisement
    ///
    /// Format: [CMD_SEND_SELF_ADVERT=0x07][flood: optional]
    pub async fn send_advert(&self, flood: bool) -> Result<()> {
        let data = if flood {
            vec![CMD_SEND_SELF_ADVERT, 0x01]
        } else {
            vec![CMD_SEND_SELF_ADVERT]
        };
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Reboot device (no response expected)
    ///
    /// Format: [CMD_REBOOT=0x13]["reboot"]
    pub async fn reboot(&self) -> Result<()> {
        let data = [CMD_REBOOT, b'r', b'e', b'b', b'o', b'o', b't'];
        self.sender
            .send(data.to_vec())
            .await
            .map_err(|e| Error::Channel(e.to_string()))?;
        Ok(())
    }

    /// Get custom variables
    ///
    /// Format: [CMD_GET_CUSTOM_VARS=0x28]
    pub async fn get_custom_vars(&self) -> Result<HashMap<String, String>> {
        let data = [CMD_GET_CUSTOM_VARS];
        let event = self.send(&data, EventType::CustomVars).await?;

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
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Get channel info
    ///
    /// Format: [CMD_GET_CHANNEL=0x1F][channel_idx: u8]
    pub async fn get_channel(&self, channel_idx: u8) -> Result<ChannelInfoData> {
        let data = [CMD_GET_CHANNEL, channel_idx];
        let event = self.send(&data, EventType::ChannelInfo).await?;

        match event.payload {
            EventPayload::ChannelInfo(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to channel query")),
        }
    }

    /// Set channel
    ///
    /// Format: [CMD_SET_CHANNEL=0x20][channel_idx][name: 16 bytes][secret: 16 bytes]
    pub async fn set_channel(&self, channel_idx: u8, name: &str, secret: &[u8; 16]) -> Result<()> {
        let mut data = vec![CMD_SET_CHANNEL, channel_idx];
        // Pad or truncate name to 16 bytes
        let mut name_bytes = [0u8; 16];
        let name_len = name.len().min(16);
        name_bytes[..name_len].copy_from_slice(&name.as_bytes()[..name_len]);
        data.extend_from_slice(&name_bytes);
        data.extend_from_slice(secret);
        self.send(&data, EventType::Ok).await?;
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
        self.send(&data, EventType::Ok).await?;
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
            .send_with_timeout(&data, EventType::Contacts, timeout)
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

        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Remove a contact by public key
    ///
    /// Format: [CMD_REMOVE_CONTACT=0x0F][pubkey: 32]
    pub async fn remove_contact(&self, key: impl Into<Destination>) -> Result<()> {
        let dest: Destination = key.into();
        let prefix = dest.prefix()?;

        let mut data = vec![CMD_REMOVE_CONTACT];
        data.extend_from_slice(&prefix);
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Export contact as URI
    ///
    /// Format: [CMD_EXPORT_CONTACT=0x11][pubkey: 32 optional]
    pub async fn export_contact(&self, key: Option<impl Into<Destination>>) -> Result<String> {
        let data = if let Some(k) = key {
            let dest: Destination = k.into();
            let prefix = dest.prefix()?;
            let mut d = vec![CMD_EXPORT_CONTACT];
            d.extend_from_slice(&prefix);
            d
        } else {
            vec![CMD_EXPORT_CONTACT]
        };

        let event = self.send(&data, EventType::ContactUri).await?;

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
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    // ========== Messaging Commands ==========

    /// Get next message from queue
    pub async fn get_msg(&self) -> Result<Option<ReceivedMessage>> {
        self.get_msg_with_timeout(self.default_timeout).await
    }

    /// Get next message with custom timeout
    ///
    /// Format: [CMD_SYNC_NEXT_MESSAGE=0x0A]
    pub async fn get_msg_with_timeout(&self, timeout: Duration) -> Result<Option<ReceivedMessage>> {
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
            EventType::ContactMsgRecv | EventType::ChannelMsgRecv => match event.payload {
                EventPayload::Message(msg) => Ok(Some(msg)),
                _ => Err(Error::protocol("Unexpected payload for message")),
            },
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
        let mut data = vec![CMD_SEND_TXT_MSG, 0x00, 0x00];
        data.extend_from_slice(&ts.to_le_bytes());
        data.extend_from_slice(&prefix);
        data.extend_from_slice(msg.as_bytes());

        let event = self.send(&data, EventType::MsgSent).await?;

        match event.payload {
            EventPayload::MsgSent(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to send_msg")),
        }
    }

    /// Send a channel message
    ///
    /// Format: [CMD_SEND_CHANNEL_TXT_MSG=0x03][txt_type][channel_idx][timestamp: u32][message]
    pub async fn send_chan_msg(
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

        self.send(&data, EventType::Ok).await?;
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

        let event = self.send(&data, EventType::MsgSent).await?;

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

        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    // ========== Binary Commands ==========

    /// Send a binary request to a contact
    ///
    /// Format: [CMD_SEND_BINARY_REQ=0x32][req_type][pubkey: 32]
    pub async fn send_binary_req(
        &self,
        dest: impl Into<Destination>,
        req_type: BinaryReqType,
    ) -> Result<MsgSentInfo> {
        let dest: Destination = dest.into();
        let pubkey = dest
            .public_key()
            .ok_or_else(|| Error::invalid_param("Binary request requires full 32-byte public key"))?;

        let mut data = vec![CMD_SEND_BINARY_REQ];
        data.push(req_type as u8);
        data.extend_from_slice(&pubkey);

        let event = self.send(&data, EventType::MsgSent).await?;

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
    pub async fn req_status(&self, dest: impl Into<Destination>) -> Result<StatusData> {
        self.req_status_with_timeout(dest, self.default_timeout)
            .await
    }

    /// Request status with the custom timeout
    pub async fn req_status_with_timeout(
        &self,
        dest: impl Into<Destination>,
        timeout: Duration,
    ) -> Result<StatusData> {
        let sent = self.send_binary_req(dest, BinaryReqType::Status).await?;

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), hex_encode(&sent.expected_ack));

        let event = self
            .wait_for_event(EventType::StatusResponse, filters, timeout)
            .await?;

        match event.payload {
            EventPayload::Status(status) => Ok(status),
            _ => Err(Error::protocol("Unexpected response to status request")),
        }
    }

    /// Request telemetry from a contact
    pub async fn req_telemetry(&self, dest: impl Into<Destination>) -> Result<Vec<u8>> {
        self.req_telemetry_with_timeout(dest, self.default_timeout)
            .await
    }

    /// Request telemetry with the custom timeout
    pub async fn req_telemetry_with_timeout(
        &self,
        dest: impl Into<Destination>,
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        let sent = self.send_binary_req(dest, BinaryReqType::Telemetry).await?;

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), hex_encode(&sent.expected_ack));

        let event = self
            .wait_for_event(EventType::TelemetryResponse, filters, timeout)
            .await?;

        match event.payload {
            EventPayload::Telemetry(data) => Ok(data),
            _ => Err(Error::protocol("Unexpected response to telemetry request")),
        }
    }

    /// Request ACL from a contact
    pub async fn req_acl(&self, dest: impl Into<Destination>) -> Result<Vec<AclEntry>> {
        self.req_acl_with_timeout(dest, self.default_timeout).await
    }

    /// Request ACL with the custom timeout
    pub async fn req_acl_with_timeout(
        &self,
        dest: impl Into<Destination>,
        timeout: Duration,
    ) -> Result<Vec<AclEntry>> {
        let sent = self.send_binary_req(dest, BinaryReqType::Acl).await?;

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), hex_encode(&sent.expected_ack));

        let event = self
            .wait_for_event(EventType::AclResponse, filters, timeout)
            .await?;

        match event.payload {
            EventPayload::Acl(entries) => Ok(entries),
            _ => Err(Error::protocol("Unexpected response to ACL request")),
        }
    }

    /// Request neighbors from a contact
    pub async fn req_neighbours(
        &self,
        dest: impl Into<Destination>,
        count: u16,
        offset: u16,
    ) -> Result<NeighboursData> {
        self.req_neighbours_with_timeout(dest, count, offset, self.default_timeout)
            .await
    }

    /// Request neighbors with custom timeout
    ///
    /// Format: [CMD_SEND_BINARY_REQ=0x32][req_type][pubkey: 32][count: u16][offset: u16]
    pub async fn req_neighbours_with_timeout(
        &self,
        dest: impl Into<Destination>,
        count: u16,
        offset: u16,
        timeout: Duration,
    ) -> Result<NeighboursData> {
        let dest: Destination = dest.into();
        let pubkey = dest.public_key().ok_or_else(|| {
            Error::invalid_param("Neighbours request requires full 32-byte public key")
        })?;

        let mut data = vec![CMD_SEND_BINARY_REQ];
        data.push(BinaryReqType::Neighbours as u8);
        data.extend_from_slice(&pubkey);
        data.extend_from_slice(&count.to_le_bytes());
        data.extend_from_slice(&offset.to_le_bytes());

        let event = self.send(&data, EventType::MsgSent).await?;
        let sent = match event.payload {
            EventPayload::MsgSent(info) => info,
            _ => return Err(Error::protocol("Unexpected response to neighbours request")),
        };

        // Register the request
        self.reader
            .register_binary_request(
                &sent.expected_ack,
                BinaryReqType::Neighbours,
                pubkey.to_vec(),
                timeout,
                HashMap::new(),
                false,
            )
            .await;

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), hex_encode(&sent.expected_ack));

        let event = self
            .wait_for_event(EventType::NeighboursResponse, filters, timeout)
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
        let event = self.send(&data, EventType::SignStart).await?;

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
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Finish signing and get the signature
    ///
    /// Format: [CMD_SIGN_FINISH=0x23]
    pub async fn sign_finish(&self, timeout: Duration) -> Result<Vec<u8>> {
        let data = [CMD_SIGN_FINISH];
        let event = self
            .send_with_timeout(&data, EventType::Signature, timeout)
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
