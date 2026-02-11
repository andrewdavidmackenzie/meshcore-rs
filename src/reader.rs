//! Message reader for parsing incoming MeshCore packets

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::events::*;
use crate::packets::{BinaryReqType, ControlType, PacketType};
use crate::parsing::*;
use crate::Result;

/// Tracks a pending binary request
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct BinaryRequest {
    /// Request type
    request_type: BinaryReqType,
    /// Public key prefix for matching
    pubkey_prefix: Vec<u8>,
    /// Expiration time
    expires_at: Instant,
    /// Context data
    context: HashMap<String, String>,
    /// Whether this is an anonymous request
    is_anon: bool,
}

/// Message reader that parses packets and emits events
pub struct MessageReader {
    /// Event dispatcher
    dispatcher: Arc<EventDispatcher>,
    /// Pending binary requests
    pending_requests: Arc<RwLock<HashMap<String, BinaryRequest>>>,
    /// Contacts being built during the multi-packet contact list
    pending_contacts: Arc<RwLock<Vec<Contact>>>,
    /// Current contact list last_modification_timestamp value
    contacts_last_modification_timestamp: Arc<RwLock<u32>>,
}

impl MessageReader {
    /// Create a new message reader
    pub fn new(dispatcher: Arc<EventDispatcher>) -> Self {
        Self {
            dispatcher,
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            pending_contacts: Arc::new(RwLock::new(Vec::new())),
            contacts_last_modification_timestamp: Arc::new(RwLock::new(0)),
        }
    }

    /// Register a binary request for response matching
    pub async fn register_binary_request(
        &self,
        tag: &[u8],
        request_type: BinaryReqType,
        pubkey_prefix: Vec<u8>,
        timeout: Duration,
        context: HashMap<String, String>,
        is_anon: bool,
    ) {
        let tag_hex = hex_encode(tag);
        let request = BinaryRequest {
            request_type,
            pubkey_prefix,
            expires_at: Instant::now() + timeout,
            context,
            is_anon,
        };

        self.pending_requests.write().await.insert(tag_hex, request);
    }

    /// Clean up expired requests
    async fn cleanup_expired(&self) {
        let now = Instant::now();
        self.pending_requests
            .write()
            .await
            .retain(|_, req| req.expires_at > now);
    }

    /// Handle received data
    pub async fn handle_rx(&self, data: Vec<u8>) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        // Clean up expired requests periodically
        self.cleanup_expired().await;

        let packet_type = PacketType::from(data[0]);
        let payload = if data.len() > 1 { &data[1..] } else { &[] };

        match packet_type {
            PacketType::Ok => {
                self.dispatcher.emit(Event::ok()).await;
            }

            PacketType::Error => {
                let msg = if !payload.is_empty() {
                    String::from_utf8_lossy(payload).to_string()
                } else {
                    "Unknown error".to_string()
                };
                self.dispatcher.emit(Event::error(msg)).await;
            }

            PacketType::ContactStart => {
                // Clear pending contacts
                self.pending_contacts.write().await.clear();
            }

            PacketType::Contact | PacketType::PushCodeNewAdvert => {
                if let Ok(contact) = parse_contact(payload) {
                    if packet_type == PacketType::PushCodeNewAdvert {
                        // Emit as a new contact event
                        let event =
                            Event::new(EventType::NewContact, EventPayload::Contact(contact));
                        self.dispatcher.emit(event).await;
                    } else {
                        // Add to pending contacts
                        self.pending_contacts.write().await.push(contact);
                    }
                }
            }

            PacketType::ContactEnd => {
                // Get last_modification_timestamp if present
                let last_modification_timestamp = if payload.len() >= 4 {
                    read_u32_le(payload, 0).unwrap_or(0)
                } else {
                    0
                };
                *self.contacts_last_modification_timestamp.write().await =
                    last_modification_timestamp;

                // Emit contacts event
                let contacts = std::mem::take(&mut *self.pending_contacts.write().await);
                let event = Event::new(EventType::Contacts, EventPayload::Contacts(contacts))
                    .with_attribute("lastmod", last_modification_timestamp.to_string());
                self.dispatcher.emit(event).await;
            }

            PacketType::SelfInfo => {
                if let Ok(info) = parse_self_info(payload) {
                    let event = Event::new(EventType::SelfInfo, EventPayload::SelfInfo(info));
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::DeviceInfo => {
                let event = Event::new(
                    EventType::DeviceInfo,
                    EventPayload::DeviceInfo(DeviceInfoData {
                        raw: payload.to_vec(),
                    }),
                );
                self.dispatcher.emit(event).await;
            }

            PacketType::Battery => {
                if payload.len() >= 4 {
                    let level = read_u16_le(payload, 0).unwrap_or(0);
                    let storage = read_u16_le(payload, 2).unwrap_or(0);
                    let event = Event::new(
                        EventType::Battery,
                        EventPayload::Battery(BatteryInfo { level, storage }),
                    );
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::CurrentTime => {
                if payload.len() >= 4 {
                    let time = read_u32_le(payload, 0).unwrap_or(0);
                    let event = Event::new(EventType::CurrentTime, EventPayload::Time(time));
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::MsgSent => {
                if payload.len() >= 9 {
                    let message_type = payload[0];
                    let expected_ack: [u8; 4] = read_bytes(payload, 1).unwrap_or([0; 4]);
                    let suggested_timeout = read_u32_le(payload, 5).unwrap_or(5000);

                    let event = Event::new(
                        EventType::MsgSent,
                        EventPayload::MsgSent(MsgSentInfo {
                            message_type,
                            expected_ack,
                            suggested_timeout,
                        }),
                    )
                    .with_attribute("tag", hex_encode(&expected_ack));
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::ContactMsgRecv => {
                if let Ok(msg) = parse_contact_msg(payload) {
                    let event = Event::new(EventType::ContactMsgRecv, EventPayload::Message(msg));
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::ContactMsgRecvV3 => {
                if let Ok(msg) = parse_contact_msg_v3(payload) {
                    let event = Event::new(EventType::ContactMsgRecv, EventPayload::Message(msg));
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::ChannelMsgRecv | PacketType::ChannelMsgRecvV3 => {
                if let Ok(msg) = parse_channel_msg(payload) {
                    let event = Event::new(EventType::ChannelMsgRecv, EventPayload::Message(msg));
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::NoMoreMsgs => {
                let event = Event::new(EventType::NoMoreMessages, EventPayload::None);
                self.dispatcher.emit(event).await;
            }

            PacketType::ContactUri => {
                let uri = String::from_utf8_lossy(payload).to_string();
                let event = Event::new(EventType::ContactUri, EventPayload::String(uri));
                self.dispatcher.emit(event).await;
            }

            PacketType::PrivateKey => {
                if payload.len() >= 64 {
                    let key: [u8; 64] = read_bytes(payload, 0).unwrap_or([0; 64]);
                    let event = Event::new(EventType::PrivateKey, EventPayload::PrivateKey(key));
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::Disabled => {
                let msg = String::from_utf8_lossy(payload).to_string();
                let event = Event::new(EventType::Disabled, EventPayload::String(msg));
                self.dispatcher.emit(event).await;
            }

            PacketType::ChannelInfo => {
                if payload.len() >= 18 {
                    let channel_idx = payload[0];
                    let name = read_string(payload, 1, 16);
                    let secret: [u8; 16] = if payload.len() >= 33 {
                        read_bytes(payload, 17).unwrap_or([0; 16])
                    } else {
                        [0; 16]
                    };

                    let event = Event::new(
                        EventType::ChannelInfo,
                        EventPayload::ChannelInfo(ChannelInfoData {
                            channel_idx,
                            name,
                            secret,
                        }),
                    );
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::SignStart => {
                if payload.len() >= 4 {
                    let max_length = read_u32_le(payload, 0).unwrap_or(0);
                    let event =
                        Event::new(EventType::SignStart, EventPayload::SignStart { max_length });
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::Signature => {
                let event = Event::new(
                    EventType::Signature,
                    EventPayload::Signature(payload.to_vec()),
                );
                self.dispatcher.emit(event).await;
            }

            PacketType::CustomVars => {
                // Parse key-value pairs
                let mut vars = HashMap::new();
                let text = String::from_utf8_lossy(payload);
                for line in text.lines() {
                    if let Some((key, value)) = line.split_once('=') {
                        vars.insert(key.to_string(), value.to_string());
                    }
                }
                let event = Event::new(EventType::CustomVars, EventPayload::CustomVars(vars));
                self.dispatcher.emit(event).await;
            }

            PacketType::Stats => {
                // Stats have a category byte followed by data
                if !payload.is_empty() {
                    let category = match payload[0] {
                        0 => StatsCategory::Core,
                        1 => StatsCategory::Radio,
                        2 => StatsCategory::Packets,
                        _ => StatsCategory::Core,
                    };
                    let event_type = match category {
                        StatsCategory::Core => EventType::StatsCore,
                        StatsCategory::Radio => EventType::StatsRadio,
                        StatsCategory::Packets => EventType::StatsPackets,
                    };
                    let event = Event::new(
                        event_type,
                        EventPayload::Stats(StatsData {
                            category,
                            raw: payload[1..].to_vec(),
                        }),
                    );
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::AutoaddConfig => {
                let flags = if !payload.is_empty() { payload[0] } else { 0 };
                let event = Event::new(
                    EventType::AutoAddConfig,
                    EventPayload::AutoAddConfig { flags },
                );
                self.dispatcher.emit(event).await;
            }

            PacketType::Advertisement => {
                if payload.len() >= 14 {
                    let prefix: [u8; 6] = read_bytes(payload, 0).unwrap_or([0; 6]);
                    let name = read_string(payload, 6, 32);
                    let lat = if payload.len() >= 42 {
                        read_i32_le(payload, 38).unwrap_or(0)
                    } else {
                        0
                    };
                    let lon = if payload.len() >= 46 {
                        read_i32_le(payload, 42).unwrap_or(0)
                    } else {
                        0
                    };

                    let event = Event::new(
                        EventType::Advertisement,
                        EventPayload::Advertisement(AdvertisementData {
                            prefix,
                            name,
                            lat,
                            lon,
                        }),
                    );
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::PathUpdate => {
                if payload.len() >= 7 {
                    let prefix: [u8; 6] = read_bytes(payload, 0).unwrap_or([0; 6]);
                    let path_len = payload[6] as i8;
                    let path = if payload.len() > 7 {
                        payload[7..].to_vec()
                    } else {
                        Vec::new()
                    };

                    let event = Event::new(
                        EventType::PathUpdate,
                        EventPayload::PathUpdate(PathUpdateData {
                            prefix,
                            path_len,
                            path,
                        }),
                    );
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::Ack => {
                if payload.len() >= 4 {
                    let tag: [u8; 4] = read_bytes(payload, 0).unwrap_or([0; 4]);
                    let event = Event::new(EventType::Ack, EventPayload::Ack { tag })
                        .with_attribute("tag", hex_encode(&tag));
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::MessagesWaiting => {
                let event = Event::new(EventType::MessagesWaiting, EventPayload::None);
                self.dispatcher.emit(event).await;
            }

            PacketType::LoginSuccess => {
                let event = Event::new(EventType::LoginSuccess, EventPayload::None);
                self.dispatcher.emit(event).await;
            }

            PacketType::LoginFailed => {
                let event = Event::new(EventType::LoginFailed, EventPayload::None);
                self.dispatcher.emit(event).await;
            }

            PacketType::StatusResponse => {
                if payload.len() >= 58 {
                    // The first 6 bytes are the sender prefix
                    let sender_prefix: [u8; 6] = read_bytes(payload, 0).unwrap_or([0; 6]);
                    if let Ok(status) = parse_status(&payload[6..], sender_prefix) {
                        let tag_hex = hex_encode(&sender_prefix);
                        let event =
                            Event::new(EventType::StatusResponse, EventPayload::Status(status))
                                .with_attribute("prefix", tag_hex);
                        self.dispatcher.emit(event).await;
                    }
                }
            }

            PacketType::TelemetryResponse => {
                // The first bytes are tag, the rest is LPP data
                if payload.len() >= 4 {
                    let tag: [u8; 4] = read_bytes(payload, 0).unwrap_or([0; 4]);
                    let telemetry = payload[4..].to_vec();
                    let event = Event::new(
                        EventType::TelemetryResponse,
                        EventPayload::Telemetry(telemetry),
                    )
                    .with_attribute("tag", hex_encode(&tag));
                    self.dispatcher.emit(event).await;
                }
            }

            PacketType::BinaryResponse => {
                if payload.len() >= 4 {
                    let tag: [u8; 4] = read_bytes(payload, 0).unwrap_or([0; 4]);
                    let data = payload[4..].to_vec();

                    // Check if we have a pending request for this tag
                    let tag_hex = hex_encode(&tag);
                    let request = self.pending_requests.write().await.remove(&tag_hex);

                    if let Some(req) = request {
                        // Emit typed event based on the request type
                        let event = match req.request_type {
                            BinaryReqType::Status => {
                                if let Ok(status) = parse_status(&data, [0; 6]) {
                                    Event::new(
                                        EventType::StatusResponse,
                                        EventPayload::Status(status),
                                    )
                                } else {
                                    Event::new(
                                        EventType::BinaryResponse,
                                        EventPayload::BinaryResponse { tag, data },
                                    )
                                }
                            }
                            BinaryReqType::Telemetry => Event::new(
                                EventType::TelemetryResponse,
                                EventPayload::Telemetry(data),
                            ),
                            BinaryReqType::Mma => {
                                let entries = parse_mma(&data);
                                Event::new(EventType::MmaResponse, EventPayload::Mma(entries))
                            }
                            BinaryReqType::Acl => {
                                let entries = parse_acl(&data);
                                Event::new(EventType::AclResponse, EventPayload::Acl(entries))
                            }
                            BinaryReqType::Neighbours => {
                                // Default to 6-byte pubkey prefix
                                if let Ok(neighbours) = parse_neighbours(&data, 6) {
                                    Event::new(
                                        EventType::NeighboursResponse,
                                        EventPayload::Neighbours(neighbours),
                                    )
                                } else {
                                    Event::new(
                                        EventType::BinaryResponse,
                                        EventPayload::BinaryResponse { tag, data },
                                    )
                                }
                            }
                            BinaryReqType::KeepAlive => Event::new(
                                EventType::BinaryResponse,
                                EventPayload::BinaryResponse { tag, data },
                            ),
                        }
                        .with_attribute("tag", tag_hex);

                        self.dispatcher.emit(event).await;
                    } else {
                        // No matching request, emit generic binary response
                        let event = Event::new(
                            EventType::BinaryResponse,
                            EventPayload::BinaryResponse { tag, data },
                        )
                        .with_attribute("tag", tag_hex);
                        self.dispatcher.emit(event).await;
                    }
                }
            }

            PacketType::ControlData => {
                if !payload.is_empty() {
                    let control_type = ControlType::from(payload[0]);
                    match control_type {
                        ControlType::NodeDiscoverResp => {
                            // Parse discover response
                            let mut entries = Vec::new();
                            let mut offset = 1;
                            while offset + 38 <= payload.len() {
                                let pubkey = payload[offset..offset + 32].to_vec();
                                let name = read_string(payload, offset + 32, 32);
                                entries.push(DiscoverEntry { pubkey, name });
                                offset += 64;
                            }
                            let event = Event::new(
                                EventType::DiscoverResponse,
                                EventPayload::DiscoverResponse(entries),
                            );
                            self.dispatcher.emit(event).await;
                        }
                        _ => {
                            let event = Event::new(
                                EventType::ControlData,
                                EventPayload::Bytes(payload.to_vec()),
                            );
                            self.dispatcher.emit(event).await;
                        }
                    }
                }
            }

            PacketType::TraceData => {
                // Parse trace hops
                let mut hops = Vec::new();
                let mut offset = 0;
                while offset + 7 <= payload.len() {
                    let prefix: [u8; 6] = read_bytes(payload, offset).unwrap_or([0; 6]);
                    let snr_raw = payload[offset + 6] as i8;
                    let snr = snr_raw as f32 / 4.0;
                    hops.push(TraceHop { prefix, snr });
                    offset += 7;
                }
                let event = Event::new(
                    EventType::TraceData,
                    EventPayload::TraceData(TraceInfo { hops }),
                );
                self.dispatcher.emit(event).await;
            }

            PacketType::AdvertResponse => {
                if payload.len() >= 42 {
                    let tag: [u8; 4] = read_bytes(payload, 0).unwrap_or([0; 4]);
                    let pubkey: [u8; 32] = read_bytes(payload, 4).unwrap_or([0; 32]);
                    let adv_type = payload[36];
                    let node_name = read_string(payload, 37, 32);
                    let timestamp = read_u32_le(payload, 69).unwrap_or(0);
                    let flags = if payload.len() > 73 { payload[73] } else { 0 };

                    let (lat, lon, node_desc) = if payload.len() >= 82 {
                        let lat = Some(read_i32_le(payload, 74).unwrap_or(0));
                        let lon = Some(read_i32_le(payload, 78).unwrap_or(0));
                        let desc = if payload.len() > 82 {
                            Some(read_string(payload, 82, 32))
                        } else {
                            None
                        };
                        (lat, lon, desc)
                    } else {
                        (None, None, None)
                    };

                    let event = Event::new(
                        EventType::AdvertResponse,
                        EventPayload::AdvertResponse(AdvertResponseData {
                            tag,
                            pubkey,
                            adv_type,
                            node_name,
                            timestamp,
                            flags,
                            lat,
                            lon,
                            node_desc,
                        }),
                    )
                    .with_attribute("tag", hex_encode(&tag));
                    self.dispatcher.emit(event).await;
                }
            }

            _ => {
                // Unknown packet type - emit raw data
                tracing::debug!("Unknown packet type: {:?}", packet_type);
                let event = Event::new(EventType::Unknown, EventPayload::Bytes(data));
                self.dispatcher.emit(event).await;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn create_reader() -> (MessageReader, Arc<EventDispatcher>) {
        let dispatcher = Arc::new(EventDispatcher::new());
        let reader = MessageReader::new(dispatcher.clone());
        (reader, dispatcher)
    }

    #[tokio::test]
    async fn test_handle_rx_empty() {
        let (reader, _dispatcher) = create_reader();
        let result = reader.handle_rx(vec![]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_rx_ok() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        reader.handle_rx(vec![PacketType::Ok as u8]).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::Ok);
    }

    #[tokio::test]
    async fn test_handle_rx_error_with_message() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::Error as u8];
        data.extend_from_slice(b"Test error");

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::Error);
        match event.payload {
            EventPayload::String(s) => assert_eq!(s, "Test error"),
            _ => panic!("Expected String payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_error_empty() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        reader
            .handle_rx(vec![PacketType::Error as u8])
            .await
            .unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::Error);
        match event.payload {
            EventPayload::String(s) => assert_eq!(s, "Unknown error"),
            _ => panic!("Expected String payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_contact_start() {
        let (reader, _dispatcher) = create_reader();

        // Add some dummy contacts
        reader.pending_contacts.write().await.push(Contact {
            public_key: [0u8; 32],
            contact_type: 1,
            flags: 0,
            path_len: 0,
            out_path: vec![],
            adv_name: "Old".to_string(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: 0,
            last_modification_timestamp: 0,
        });

        reader
            .handle_rx(vec![PacketType::ContactStart as u8])
            .await
            .unwrap();

        // Verify pending contacts were cleared
        assert!(reader.pending_contacts.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_handle_rx_battery() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::Battery as u8];
        data.extend_from_slice(&85u16.to_le_bytes()); // level
        data.extend_from_slice(&100u16.to_le_bytes()); // storage

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::Battery);
        match event.payload {
            EventPayload::Battery(info) => {
                assert_eq!(info.level, 85);
                assert_eq!(info.storage, 100);
            }
            _ => panic!("Expected Battery payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_current_time() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::CurrentTime as u8];
        data.extend_from_slice(&1234567890u32.to_le_bytes());

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::CurrentTime);
        match event.payload {
            EventPayload::Time(t) => assert_eq!(t, 1234567890),
            _ => panic!("Expected Time payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_no_more_msgs() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        reader
            .handle_rx(vec![PacketType::NoMoreMsgs as u8])
            .await
            .unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::NoMoreMessages);
    }

    #[tokio::test]
    async fn test_handle_rx_contact_uri() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::ContactUri as u8];
        data.extend_from_slice(b"meshcore://contact/abc123");

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::ContactUri);
        match event.payload {
            EventPayload::String(s) => assert_eq!(s, "meshcore://contact/abc123"),
            _ => panic!("Expected String payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_private_key() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::PrivateKey as u8];
        let key = [0xAA; 64];
        data.extend_from_slice(&key);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::PrivateKey);
        match event.payload {
            EventPayload::PrivateKey(k) => assert_eq!(k, [0xAA; 64]),
            _ => panic!("Expected PrivateKey payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_disabled() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::Disabled as u8];
        data.extend_from_slice(b"Feature disabled");

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::Disabled);
        match event.payload {
            EventPayload::String(s) => assert_eq!(s, "Feature disabled"),
            _ => panic!("Expected String payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_sign_start() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::SignStart as u8];
        data.extend_from_slice(&1024u32.to_le_bytes());

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::SignStart);
        match event.payload {
            EventPayload::SignStart { max_length } => assert_eq!(max_length, 1024),
            _ => panic!("Expected SignStart payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_signature() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::Signature as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::Signature);
        match event.payload {
            EventPayload::Signature(sig) => assert_eq!(sig, vec![0x01, 0x02, 0x03, 0x04]),
            _ => panic!("Expected Signature payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_custom_vars() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::CustomVars as u8];
        data.extend_from_slice(b"key1=value1\nkey2=value2");

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::CustomVars);
        match event.payload {
            EventPayload::CustomVars(vars) => {
                assert_eq!(vars.get("key1"), Some(&"value1".to_string()));
                assert_eq!(vars.get("key2"), Some(&"value2".to_string()));
            }
            _ => panic!("Expected CustomVars payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_msg_sent() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::MsgSent as u8];
        data.push(1); // message_type
        data.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]); // expected_ack
        data.extend_from_slice(&5000u32.to_le_bytes()); // suggested_timeout

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::MsgSent);
        match event.payload {
            EventPayload::MsgSent(info) => {
                assert_eq!(info.message_type, 1);
                assert_eq!(info.expected_ack, [0xAA, 0xBB, 0xCC, 0xDD]);
                assert_eq!(info.suggested_timeout, 5000);
            }
            _ => panic!("Expected MsgSent payload"),
        }
        assert_eq!(event.attributes.get("tag"), Some(&"aabbccdd".to_string()));
    }

    #[tokio::test]
    async fn test_handle_rx_ack() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::Ack as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::Ack);
        match event.payload {
            EventPayload::Ack { tag } => assert_eq!(tag, [0x01, 0x02, 0x03, 0x04]),
            _ => panic!("Expected Ack payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_messages_waiting() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        reader
            .handle_rx(vec![PacketType::MessagesWaiting as u8])
            .await
            .unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::MessagesWaiting);
    }

    #[tokio::test]
    async fn test_handle_rx_login_success() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        reader
            .handle_rx(vec![PacketType::LoginSuccess as u8])
            .await
            .unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::LoginSuccess);
    }

    #[tokio::test]
    async fn test_handle_rx_login_failed() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        reader
            .handle_rx(vec![PacketType::LoginFailed as u8])
            .await
            .unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::LoginFailed);
    }

    #[tokio::test]
    async fn test_handle_rx_stats_core() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::Stats as u8];
        data.push(0); // StatsCategory::Core
        data.extend_from_slice(&[0x01, 0x02, 0x03]);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::StatsCore);
        match event.payload {
            EventPayload::Stats(stats) => {
                assert_eq!(stats.category, StatsCategory::Core);
                assert_eq!(stats.raw, vec![0x01, 0x02, 0x03]);
            }
            _ => panic!("Expected Stats payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_stats_radio() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::Stats as u8];
        data.push(1); // StatsCategory::Radio
        data.extend_from_slice(&[0x04, 0x05]);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::StatsRadio);
    }

    #[tokio::test]
    async fn test_handle_rx_stats_packets() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::Stats as u8];
        data.push(2); // StatsCategory::Packets
        data.extend_from_slice(&[0x06, 0x07]);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::StatsPackets);
    }

    #[tokio::test]
    async fn test_handle_rx_autoadd_config() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let data = vec![PacketType::AutoaddConfig as u8, 0x03];

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::AutoAddConfig);
        match event.payload {
            EventPayload::AutoAddConfig { flags } => assert_eq!(flags, 0x03),
            _ => panic!("Expected AutoAddConfig payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_device_info() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::DeviceInfo as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::DeviceInfo);
        match event.payload {
            EventPayload::DeviceInfo(info) => {
                assert_eq!(info.raw, vec![0x01, 0x02, 0x03, 0x04]);
            }
            _ => panic!("Expected DeviceInfo payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_path_update() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::PathUpdate as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]); // prefix
        data.push(3); // path_len
        data.extend_from_slice(&[0x0A, 0x0B, 0x0C]); // path

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::PathUpdate);
        match event.payload {
            EventPayload::PathUpdate(update) => {
                assert_eq!(update.prefix, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
                assert_eq!(update.path_len, 3);
                assert_eq!(update.path, vec![0x0A, 0x0B, 0x0C]);
            }
            _ => panic!("Expected PathUpdate payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_telemetry_response() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::TelemetryResponse as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // tag
        data.extend_from_slice(&[0xAA, 0xBB, 0xCC]); // telemetry data

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::TelemetryResponse);
        match event.payload {
            EventPayload::Telemetry(data) => assert_eq!(data, vec![0xAA, 0xBB, 0xCC]),
            _ => panic!("Expected Telemetry payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_trace_data() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::TraceData as u8];
        // Hop 1: 6 bytes prefix + 1 byte snr
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        data.push(40); // snr = 10.0
                       // Hop 2
        data.extend_from_slice(&[0x11, 0x12, 0x13, 0x14, 0x15, 0x16]);
        data.push(20); // snr = 5.0

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::TraceData);
        match event.payload {
            EventPayload::TraceData(info) => {
                assert_eq!(info.hops.len(), 2);
                assert_eq!(info.hops[0].snr, 10.0);
                assert_eq!(info.hops[1].snr, 5.0);
            }
            _ => panic!("Expected TraceData payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_unknown() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let data = vec![0xFE, 0x01, 0x02, 0x03];

        reader.handle_rx(data.clone()).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::Unknown);
        match event.payload {
            EventPayload::Bytes(d) => assert_eq!(d, data),
            _ => panic!("Expected Bytes payload"),
        }
    }

    #[tokio::test]
    async fn test_register_binary_request() {
        let (reader, _dispatcher) = create_reader();

        reader
            .register_binary_request(
                &[0x01, 0x02, 0x03, 0x04],
                BinaryReqType::Status,
                vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
                Duration::from_secs(30),
                HashMap::new(),
                false,
            )
            .await;

        let requests = reader.pending_requests.read().await;
        assert!(requests.contains_key("01020304"));
    }

    #[tokio::test]
    async fn test_cleanup_expired() {
        let (reader, _dispatcher) = create_reader();

        // Register a request with immediate expiration
        reader
            .register_binary_request(
                &[0x01, 0x02, 0x03, 0x04],
                BinaryReqType::Status,
                vec![],
                Duration::from_millis(1),
                HashMap::new(),
                false,
            )
            .await;

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Trigger cleanup
        reader.cleanup_expired().await;

        let requests = reader.pending_requests.read().await;
        assert!(requests.is_empty());
    }

    #[tokio::test]
    async fn test_handle_rx_binary_response_with_pending_request() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        // Register a pending request
        reader
            .register_binary_request(
                &[0x01, 0x02, 0x03, 0x04],
                BinaryReqType::Telemetry,
                vec![],
                Duration::from_secs(30),
                HashMap::new(),
                false,
            )
            .await;

        let mut data = vec![PacketType::BinaryResponse as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // matching tag
        data.extend_from_slice(&[0xAA, 0xBB, 0xCC]); // telemetry data

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        // Should emit TelemetryResponse due to the pending request type
        assert_eq!(event.event_type, EventType::TelemetryResponse);
    }

    #[tokio::test]
    async fn test_handle_rx_binary_response_no_pending() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::BinaryResponse as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // tag
        data.extend_from_slice(&[0xAA, 0xBB, 0xCC]); // data

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        // Should emit generic BinaryResponse
        assert_eq!(event.event_type, EventType::BinaryResponse);
    }

    #[tokio::test]
    async fn test_handle_rx_channel_info() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::ChannelInfo as u8];
        data.push(1); // channel_idx
        let mut name = [0u8; 16];
        name[..7].copy_from_slice(b"General");
        data.extend_from_slice(&name);
        data.extend_from_slice(&[0xAA; 16]); // secret

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::ChannelInfo);
        match event.payload {
            EventPayload::ChannelInfo(info) => {
                assert_eq!(info.channel_idx, 1);
                assert_eq!(info.name, "General");
                assert_eq!(info.secret, [0xAA; 16]);
            }
            _ => panic!("Expected ChannelInfo payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_channel_info_no_secret() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::ChannelInfo as u8];
        data.push(2); // channel_idx
        let mut name = [0u8; 17]; // 17 bytes to meet the 18-byte minimum (1 idx + 17 name)
        name[..4].copy_from_slice(b"Test");
        data.extend_from_slice(&name);
        // No secret provided - payload is exactly 18 bytes

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::ChannelInfo);
        match event.payload {
            EventPayload::ChannelInfo(info) => {
                assert_eq!(info.channel_idx, 2);
                assert_eq!(info.secret, [0; 16]); // default to zeros when not provided
            }
            _ => panic!("Expected ChannelInfo payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_advertisement() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::Advertisement as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]); // prefix
                                                                       // name (32 bytes padded)
        let mut name_bytes = [0u8; 32];
        name_bytes[..5].copy_from_slice(b"Node1");
        data.extend_from_slice(&name_bytes);
        // lat at offset 38
        data.extend_from_slice(&37774900i32.to_le_bytes());
        // lon at offset 42
        data.extend_from_slice(&(-122419400i32).to_le_bytes());

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::Advertisement);
        match event.payload {
            EventPayload::Advertisement(adv) => {
                assert_eq!(adv.prefix, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
                assert_eq!(adv.name, "Node1");
                assert_eq!(adv.lat, 37774900);
                assert_eq!(adv.lon, -122419400);
            }
            _ => panic!("Expected Advertisement payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_advertisement_minimal() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::Advertisement as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]); // prefix
                                                                       // Just 8 bytes for name (minimal)
        data.extend_from_slice(b"ShortNam");

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::Advertisement);
        match event.payload {
            EventPayload::Advertisement(adv) => {
                assert_eq!(adv.lat, 0); // default when not present
                assert_eq!(adv.lon, 0);
            }
            _ => panic!("Expected Advertisement payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_self_info() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::SelfInfo as u8];
        // Create a minimal valid self_info buffer (52+ bytes)
        let mut payload = vec![0u8; 60];
        payload[0] = 1; // adv_type
        payload[1] = 20; // tx_power
        payload[2] = 30; // max_tx_power
        payload[35..39].copy_from_slice(&37774900i32.to_le_bytes()); // adv_lat
        payload[39..43].copy_from_slice(&(-122419400i32).to_le_bytes()); // adv_lon
        payload[47..51].copy_from_slice(&915000000u32.to_le_bytes()); // radio_freq
        payload[51..55].copy_from_slice(&125000u32.to_le_bytes()); // radio_bw
        payload[55] = 7; // sf
        payload[56] = 5; // cr
        payload[57..60].copy_from_slice(b"Dev");
        data.extend_from_slice(&payload);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::SelfInfo);
        match event.payload {
            EventPayload::SelfInfo(info) => {
                assert_eq!(info.tx_power, 20);
                assert_eq!(info.radio_freq, 915000000);
            }
            _ => panic!("Expected SelfInfo payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_contact_msg_recv() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::ContactMsgRecv as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]); // sender_prefix
        data.push(2); // path_len
        data.push(1); // txt_type
        data.extend_from_slice(&1234567890u32.to_le_bytes()); // sender_timestamp
        data.extend_from_slice(b"Hello!"); // text

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::ContactMsgRecv);
        match event.payload {
            EventPayload::Message(msg) => {
                assert_eq!(msg.text, "Hello!");
                assert_eq!(msg.path_len, 2);
            }
            _ => panic!("Expected Message payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_contact_msg_recv_v3() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::ContactMsgRecvV3 as u8];
        data.push(40); // snr_raw = 40 means SNR = 10.0
        data.extend_from_slice(&[0x00, 0x00]); // reserved
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]); // sender_prefix
        data.push(3); // path_len
        data.push(1); // txt_type
        data.extend_from_slice(&1234567890u32.to_le_bytes()); // sender_timestamp
        data.extend_from_slice(b"V3 msg!"); // text

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::ContactMsgRecv);
        match event.payload {
            EventPayload::Message(msg) => {
                assert_eq!(msg.text, "V3 msg!");
                assert_eq!(msg.snr, Some(10.0));
            }
            _ => panic!("Expected Message payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_channel_msg_recv() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::ChannelMsgRecv as u8];
        data.push(5); // channel
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]); // sender_prefix
        data.push(1); // path_len
        data.push(0); // txt_type
        data.extend_from_slice(&1234567890u32.to_le_bytes()); // sender_timestamp
        data.extend_from_slice(b"Channel msg"); // text

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::ChannelMsgRecv);
        match event.payload {
            EventPayload::Message(msg) => {
                assert_eq!(msg.channel, Some(5));
                assert_eq!(msg.text, "Channel msg");
            }
            _ => panic!("Expected Message payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_status_response() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::StatusResponse as u8];
        // sender_prefix (6 bytes)
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        // status data (52 bytes)
        let mut status_data = vec![0u8; 52];
        status_data[0..2].copy_from_slice(&85u16.to_le_bytes()); // battery
        status_data[2..4].copy_from_slice(&5u16.to_le_bytes()); // tx_queue_len
        status_data[20..24].copy_from_slice(&86400u32.to_le_bytes()); // uptime
        data.extend_from_slice(&status_data);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::StatusResponse);
        match event.payload {
            EventPayload::Status(status) => {
                assert_eq!(status.battery, 85);
                assert_eq!(status.uptime, 86400);
            }
            _ => panic!("Expected Status payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_new_contact() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::PushCodeNewAdvert as u8];
        // Create a minimal valid contact buffer (145+ bytes)
        let mut contact_data = vec![0u8; 149];
        contact_data[0..6].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        contact_data[32] = 1; // contact_type
        contact_data[99..104].copy_from_slice(b"New\0\0");
        data.extend_from_slice(&contact_data);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::NewContact);
        match event.payload {
            EventPayload::Contact(contact) => {
                assert_eq!(contact.adv_name, "New");
            }
            _ => panic!("Expected Contact payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_contact_list_flow() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        // Start contact list
        reader
            .handle_rx(vec![PacketType::ContactStart as u8])
            .await
            .unwrap();

        // Add a contact
        let mut contact_data = vec![PacketType::Contact as u8];
        let mut contact = vec![0u8; 149];
        contact[0..6].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        contact[32] = 1;
        contact[99..105].copy_from_slice(b"Test1\0");
        contact_data.extend_from_slice(&contact);
        reader.handle_rx(contact_data).await.unwrap();

        // End contact list
        let mut end_data = vec![PacketType::ContactEnd as u8];
        end_data.extend_from_slice(&999u32.to_le_bytes());
        reader.handle_rx(end_data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::Contacts);
        match event.payload {
            EventPayload::Contacts(contacts) => {
                assert_eq!(contacts.len(), 1);
                assert_eq!(contacts[0].adv_name, "Test1");
            }
            _ => panic!("Expected Contacts payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_binary_response_acl() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        // Register a pending ACL request
        reader
            .register_binary_request(
                &[0x01, 0x02, 0x03, 0x04],
                BinaryReqType::Acl,
                vec![],
                Duration::from_secs(30),
                HashMap::new(),
                false,
            )
            .await;

        let mut data = vec![PacketType::BinaryResponse as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // matching tag
                                                           // ACL entry data (7 bytes per entry)
        data.extend_from_slice(&[0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x01]);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::AclResponse);
        match event.payload {
            EventPayload::Acl(entries) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].permissions, 0x01);
            }
            _ => panic!("Expected Acl payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_binary_response_mma() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        // Register a pending MMA request
        reader
            .register_binary_request(
                &[0x01, 0x02, 0x03, 0x04],
                BinaryReqType::Mma,
                vec![],
                Duration::from_secs(30),
                HashMap::new(),
                false,
            )
            .await;

        let mut data = vec![PacketType::BinaryResponse as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // matching tag
                                                           // MMA entry (14 bytes)
        data.push(1); // channel
        data.push(2); // entry_type
        data.extend_from_slice(&100i32.to_le_bytes()); // min
        data.extend_from_slice(&200i32.to_le_bytes()); // max
        data.extend_from_slice(&150i32.to_le_bytes()); // avg

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::MmaResponse);
    }

    #[tokio::test]
    async fn test_handle_rx_binary_response_neighbours() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        // Register a pending Neighbours request
        reader
            .register_binary_request(
                &[0x01, 0x02, 0x03, 0x04],
                BinaryReqType::Neighbours,
                vec![],
                Duration::from_secs(30),
                HashMap::new(),
                false,
            )
            .await;

        let mut data = vec![PacketType::BinaryResponse as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // matching tag
                                                           // Neighbours data
        data.extend_from_slice(&1u16.to_le_bytes()); // total
        data.extend_from_slice(&1u16.to_le_bytes()); // count
                                                     // Entry: pubkey (6) + secs_ago (4) + snr (1)
        data.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        data.extend_from_slice(&300i32.to_le_bytes());
        data.push(40); // snr

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::NeighboursResponse);
    }

    #[tokio::test]
    async fn test_handle_rx_binary_response_status() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        // Register a pending Status request
        reader
            .register_binary_request(
                &[0x01, 0x02, 0x03, 0x04],
                BinaryReqType::Status,
                vec![],
                Duration::from_secs(30),
                HashMap::new(),
                false,
            )
            .await;

        let mut data = vec![PacketType::BinaryResponse as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // matching tag
                                                           // Status data (52 bytes)
        let mut status_data = vec![0u8; 52];
        status_data[0..2].copy_from_slice(&100u16.to_le_bytes());
        data.extend_from_slice(&status_data);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::StatusResponse);
    }

    #[tokio::test]
    async fn test_handle_rx_binary_response_keepalive() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        // Register a pending KeepAlive request
        reader
            .register_binary_request(
                &[0x01, 0x02, 0x03, 0x04],
                BinaryReqType::KeepAlive,
                vec![],
                Duration::from_secs(30),
                HashMap::new(),
                false,
            )
            .await;

        let mut data = vec![PacketType::BinaryResponse as u8];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // matching tag
        data.extend_from_slice(&[0xAA, 0xBB]);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::BinaryResponse);
    }

    #[tokio::test]
    async fn test_handle_rx_control_data_discover_resp() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::ControlData as u8];
        data.push(ControlType::NodeDiscoverResp as u8);
        // Entry: 32 bytes pubkey + 32 bytes name
        let mut entry = vec![0u8; 64];
        entry[0..6].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        entry[32..37].copy_from_slice(b"Node1");
        data.extend_from_slice(&entry);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::DiscoverResponse);
        match event.payload {
            EventPayload::DiscoverResponse(entries) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name, "Node1");
            }
            _ => panic!("Expected DiscoverResponse payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_control_data_other() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::ControlData as u8];
        data.push(ControlType::NodeDiscoverReq as u8); // Not a response
        data.extend_from_slice(&[0x01, 0x02, 0x03]);

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::ControlData);
        match event.payload {
            EventPayload::Bytes(d) => {
                assert_eq!(d[0], ControlType::NodeDiscoverReq as u8);
            }
            _ => panic!("Expected Bytes payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_advert_response() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        let mut data = vec![PacketType::AdvertResponse as u8];
        // tag (4) + pubkey (32) + adv_type (1) + node_name (32) + timestamp (4) + flags (1) = 74 bytes min
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // tag
        data.extend_from_slice(&[0xAA; 32]); // pubkey
        data.push(1); // adv_type
        let mut name = [0u8; 32];
        name[..5].copy_from_slice(b"Node1");
        data.extend_from_slice(&name); // node_name (32 bytes)
        data.extend_from_slice(&1234567890u32.to_le_bytes()); // timestamp
        data.push(0x01); // flags
                         // lat/lon
        data.extend_from_slice(&37774900i32.to_le_bytes());
        data.extend_from_slice(&(-122419400i32).to_le_bytes());

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::AdvertResponse);
        match event.payload {
            EventPayload::AdvertResponse(resp) => {
                assert_eq!(resp.tag, [0x01, 0x02, 0x03, 0x04]);
                assert_eq!(resp.adv_type, 1);
                assert_eq!(resp.node_name, "Node1");
                assert_eq!(resp.lat, Some(37774900));
            }
            _ => panic!("Expected AdvertResponse payload"),
        }
    }

    #[tokio::test]
    async fn test_handle_rx_contact_end_with_timestamp() {
        let (reader, dispatcher) = create_reader();
        let mut receiver = dispatcher.receiver();

        // Add a pending contact first
        reader.pending_contacts.write().await.push(Contact {
            public_key: [0u8; 32],
            contact_type: 1,
            flags: 0,
            path_len: 0,
            out_path: vec![],
            adv_name: "Test".to_string(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: 0,
            last_modification_timestamp: 0,
        });

        let mut data = vec![PacketType::ContactEnd as u8];
        data.extend_from_slice(&1234567890u32.to_le_bytes());

        reader.handle_rx(data).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event_type, EventType::Contacts);
        assert_eq!(
            event.attributes.get("lastmod"),
            Some(&"1234567890".to_string())
        );
        match event.payload {
            EventPayload::Contacts(contacts) => assert_eq!(contacts.len(), 1),
            _ => panic!("Expected Contacts payload"),
        }
    }
}
