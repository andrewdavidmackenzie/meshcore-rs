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
    /// Contact lookup function
    contact_getter: Option<Box<dyn Fn(&[u8; 6]) -> Option<Contact> + Send + Sync>>,
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
            contact_getter: None,
        }
    }

    /// Set the default timeout for commands
    pub fn set_default_timeout(&mut self, timeout: Duration) {
        self.default_timeout = timeout;
    }

    /// Set the contact getter function
    pub fn set_contact_getter<F>(&mut self, getter: F)
    where
        F: Fn(&[u8; 6]) -> Option<Contact> + Send + Sync + 'static,
    {
        self.contact_getter = Some(Box::new(getter));
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
    pub async fn send_appstart(&self) -> Result<SelfInfo> {
        let data = [0x01, 0x03, b'm', b'c', b'c', b'l', b'i'];
        let event = self.send(&data, EventType::SelfInfo).await?;

        match event.payload {
            EventPayload::SelfInfo(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to APPSTART")),
        }
    }

    /// Query device info
    pub async fn send_device_query(&self) -> Result<DeviceInfoData> {
        let data = [0x01, 0x02];
        let event = self.send(&data, EventType::DeviceInfo).await?;

        match event.payload {
            EventPayload::DeviceInfo(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to device query")),
        }
    }

    /// Get battery level
    pub async fn get_bat(&self) -> Result<BatteryInfo> {
        let data = [0x01, 0x04];
        let event = self.send(&data, EventType::Battery).await?;

        match event.payload {
            EventPayload::Battery(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to battery query")),
        }
    }

    /// Get device time
    pub async fn get_time(&self) -> Result<u32> {
        let data = [0x05];
        let event = self.send(&data, EventType::CurrentTime).await?;

        match event.payload {
            EventPayload::Time(t) => Ok(t),
            _ => Err(Error::protocol("Unexpected response to time query")),
        }
    }

    /// Set device time
    pub async fn set_time(&self, timestamp: u32) -> Result<()> {
        let mut data = vec![0x06];
        data.extend_from_slice(&timestamp.to_le_bytes());
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Set the device name
    pub async fn set_name(&self, name: &str) -> Result<()> {
        let mut data = vec![0x08];
        data.extend_from_slice(name.as_bytes());
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Set device coordinates
    pub async fn set_coords(&self, lat: f64, lon: f64) -> Result<()> {
        let lat_micro = to_microdegrees(lat);
        let lon_micro = to_microdegrees(lon);

        let mut data = vec![0x09];
        data.extend_from_slice(&lat_micro.to_le_bytes());
        data.extend_from_slice(&lon_micro.to_le_bytes());
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Set TX power
    pub async fn set_tx_power(&self, power: u8) -> Result<()> {
        let data = [0x0A, power];
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Send advertisement
    pub async fn send_advert(&self, flood: bool) -> Result<()> {
        let data = if flood { vec![0x07, 0x01] } else { vec![0x07] };
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Reboot device (no response expected)
    pub async fn reboot(&self) -> Result<()> {
        let data = [0x0B];
        self.sender
            .send(data.to_vec())
            .await
            .map_err(|e| Error::Channel(e.to_string()))?;
        Ok(())
    }

    /// Get custom variables
    pub async fn get_custom_vars(&self) -> Result<HashMap<String, String>> {
        let data = [0x01, 0x08];
        let event = self.send(&data, EventType::CustomVars).await?;

        match event.payload {
            EventPayload::CustomVars(vars) => Ok(vars),
            _ => Err(Error::protocol("Unexpected response to custom vars query")),
        }
    }

    /// Set a custom variable
    pub async fn set_custom_var(&self, key: &str, value: &str) -> Result<()> {
        let mut data = vec![0x01, 0x09];
        data.extend_from_slice(key.as_bytes());
        data.push(b'=');
        data.extend_from_slice(value.as_bytes());
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Get channel info
    pub async fn get_channel(&self, channel_idx: u8) -> Result<ChannelInfoData> {
        let data = [0x01, 0x05, channel_idx];
        let event = self.send(&data, EventType::ChannelInfo).await?;

        match event.payload {
            EventPayload::ChannelInfo(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to channel query")),
        }
    }

    /// Set channel
    pub async fn set_channel(&self, channel_idx: u8, name: &str, secret: &[u8; 16]) -> Result<()> {
        let mut data = vec![0x01, 0x06, channel_idx];
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
    pub async fn export_private_key(&self) -> Result<[u8; 64]> {
        let data = [0x01, 0x0A];
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
    pub async fn import_private_key(&self, key: &[u8; 64]) -> Result<()> {
        let mut data = vec![0x01, 0x0B];
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
    pub async fn get_contacts_with_timeout(
        &self,
        last_modification_stimestamp: u32,
        timeout: Duration,
    ) -> Result<Vec<Contact>> {
        let mut data = vec![0x04];
        data.extend_from_slice(&last_modification_stimestamp.to_le_bytes());
        let event = self
            .send_with_timeout(&data, EventType::Contacts, timeout)
            .await?;

        match event.payload {
            EventPayload::Contacts(contacts) => Ok(contacts),
            _ => Err(Error::protocol("Unexpected response to get contacts")),
        }
    }

    /// Add a contact
    pub async fn add_contact(&self, contact: &Contact) -> Result<()> {
        let mut data = vec![0x0C];
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
    pub async fn remove_contact(&self, key: impl Into<Destination>) -> Result<()> {
        let dest: Destination = key.into();
        let prefix = dest.prefix()?;

        let mut data = vec![0x0D];
        data.extend_from_slice(&prefix);
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Export contact as URI
    pub async fn export_contact(&self, key: Option<impl Into<Destination>>) -> Result<String> {
        let data = if let Some(k) = key {
            let dest: Destination = k.into();
            let prefix = dest.prefix()?;
            let mut d = vec![0x0E];
            d.extend_from_slice(&prefix);
            d
        } else {
            vec![0x0E]
        };

        let event = self.send(&data, EventType::ContactUri).await?;

        match event.payload {
            EventPayload::String(uri) => Ok(uri),
            _ => Err(Error::protocol("Unexpected response to export contact")),
        }
    }

    /// Import contact from card data
    pub async fn import_contact(&self, card_data: &[u8]) -> Result<()> {
        let mut data = vec![0x0F];
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
    pub async fn get_msg_with_timeout(&self, timeout: Duration) -> Result<Option<ReceivedMessage>> {
        let data = [0x10];
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

        let mut data = vec![0x02, 0x01];
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
    pub async fn send_chan_msg(&self, channel: u8, msg: &str, timestamp: Option<u32>) -> Result<()> {
        let ts = timestamp.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as u32
        });

        let mut data = vec![0x03, 0x00, channel];
        data.extend_from_slice(&ts.to_le_bytes());
        data.extend_from_slice(msg.as_bytes());

        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Send login request
    pub async fn send_login(&self, dest: impl Into<Destination>, password: &str) -> Result<MsgSentInfo> {
        let dest: Destination = dest.into();
        let prefix = dest.prefix()?;

        let mut data = vec![0x02, 0x02];
        data.extend_from_slice(&prefix);
        data.extend_from_slice(password.as_bytes());

        let event = self.send(&data, EventType::MsgSent).await?;

        match event.payload {
            EventPayload::MsgSent(info) => Ok(info),
            _ => Err(Error::protocol("Unexpected response to send_login")),
        }
    }

    /// Send logout request
    pub async fn send_logout(&self, dest: impl Into<Destination>) -> Result<()> {
        let dest: Destination = dest.into();
        let prefix = dest.prefix()?;

        let mut data = vec![0x02, 0x03];
        data.extend_from_slice(&prefix);

        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    // ========== Binary Commands ==========

    /// Send a binary request to a contact
    pub async fn send_binary_req(
        &self,
        dest: impl Into<Destination>,
        req_type: BinaryReqType,
    ) -> Result<MsgSentInfo> {
        let dest: Destination = dest.into();
        let prefix = dest.prefix()?;

        let mut data = vec![0x32]; // PacketType::BinaryReq
        data.push(req_type as u8);
        data.extend_from_slice(&prefix);

        let event = self.send(&data, EventType::MsgSent).await?;

        match event.payload {
            EventPayload::MsgSent(info) => {
                // Register the binary request for response matching
                self.reader
                    .register_binary_request(
                        &info.expected_ack,
                        req_type,
                        prefix.to_vec(),
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
    pub async fn req_neighbors(
        &self,
        dest: impl Into<Destination>,
        count: u16,
        offset: u16,
    ) -> Result<NeighboursData> {
        self.req_neighbors_with_timeout(dest, count, offset, self.default_timeout)
            .await
    }

    /// Request neighbors with custom timeout
    pub async fn req_neighbors_with_timeout(
        &self,
        dest: impl Into<Destination>,
        count: u16,
        offset: u16,
        timeout: Duration,
    ) -> Result<NeighboursData> {
        let dest: Destination = dest.into();
        let prefix = dest.prefix()?;

        let mut data = vec![0x32]; // PacketType::BinaryReq
        data.push(BinaryReqType::Neighbours as u8);
        data.extend_from_slice(&prefix);
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
                prefix.to_vec(),
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
    pub async fn sign_start(&self) -> Result<u32> {
        let data = [0x01, 0x0C];
        let event = self.send(&data, EventType::SignStart).await?;

        match event.payload {
            EventPayload::SignStart { max_length } => Ok(max_length),
            _ => Err(Error::protocol("Unexpected response to sign_start")),
        }
    }

    /// Send data chunk for signing
    pub async fn sign_data(&self, chunk: &[u8]) -> Result<()> {
        let mut data = vec![0x01, 0x0D];
        data.extend_from_slice(chunk);
        self.send(&data, EventType::Ok).await?;
        Ok(())
    }

    /// Finish signing and get the signature
    pub async fn sign_finish(&self, timeout: Duration) -> Result<Vec<u8>> {
        let data = [0x01, 0x0E];
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
