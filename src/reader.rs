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
                        let event = Event::new(EventType::NewContact, EventPayload::Contact(contact));
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
                *self.contacts_last_modification_timestamp.write().await = last_modification_timestamp;

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
                let event = Event::new(EventType::Signature, EventPayload::Signature(payload.to_vec()));
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
                let event = Event::new(EventType::AutoAddConfig, EventPayload::AutoAddConfig { flags });
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
                        let event = Event::new(EventType::StatusResponse, EventPayload::Status(status))
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
                    let event =
                        Event::new(EventType::TelemetryResponse, EventPayload::Telemetry(telemetry))
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
                                    Event::new(EventType::StatusResponse, EventPayload::Status(status))
                                } else {
                                    Event::new(
                                        EventType::BinaryResponse,
                                        EventPayload::BinaryResponse { tag, data },
                                    )
                                }
                            }
                            BinaryReqType::Telemetry => {
                                Event::new(EventType::TelemetryResponse, EventPayload::Telemetry(data))
                            }
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
                            let event =
                                Event::new(EventType::ControlData, EventPayload::Bytes(payload.to_vec()));
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
                let event = Event::new(EventType::TraceData, EventPayload::TraceData(TraceInfo { hops }));
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
