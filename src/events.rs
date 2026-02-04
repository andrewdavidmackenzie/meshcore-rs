//! Event system for MeshCore communication
//!
//! The reader emits events when packets are received from the device.
//! Users can subscribe to specific event types with optional attribute filtering.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

/// Event types emitted by MeshCore
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    // Connection events
    Connected,
    Disconnected,

    // Command responses
    Ok,
    Error,

    // Contact events
    Contacts,
    NewContact,
    NextContact,

    // Device info events
    SelfInfo,
    DeviceInfo,
    Battery,
    CurrentTime,
    PrivateKey,
    CustomVars,
    ChannelInfo,
    StatsCore,
    StatsRadio,
    StatsPackets,
    AutoAddConfig,

    // Messaging events
    ContactMsgRecv,
    ChannelMsgRecv,
    MsgSent,
    NoMoreMessages,
    ContactUri,

    // Push notifications
    Advertisement,
    PathUpdate,
    Ack,
    MessagesWaiting,
    RawData,
    LoginSuccess,
    LoginFailed,

    // Binary protocol events
    StatusResponse,
    TelemetryResponse,
    MmaResponse,
    AclResponse,
    NeighboursResponse,
    BinaryResponse,
    PathDiscoveryResponse,

    // Trace and logging
    TraceData,
    LogData,

    // Signing
    SignStart,
    Signature,
    Disabled,

    // Control
    ControlData,
    DiscoverResponse,
    AdvertResponse,

    // Unknown
    Unknown,
}

/// Payload data for events
#[derive(Debug, Clone)]
pub enum EventPayload {
    /// No payload
    None,
    /// String payload (error messages, URIs, etc.)
    String(String),
    /// Binary payload
    Bytes(Vec<u8>),
    /// Contact list
    Contacts(Vec<Contact>),
    /// Single contact
    Contact(Contact),
    /// Self-info
    SelfInfo(SelfInfo),
    /// Device info
    DeviceInfo(DeviceInfoData),
    /// Battery info
    Battery(BatteryInfo),
    /// Current time (Unix timestamp)
    Time(u32),
    /// Message received
    Message(ReceivedMessage),
    /// Message sent acknowledgment
    MsgSent(MsgSentInfo),
    /// Status response
    Status(StatusData),
    /// Channel info
    ChannelInfo(ChannelInfoData),
    /// Custom variables
    CustomVars(HashMap<String, String>),
    /// Private key (64 bytes)
    PrivateKey([u8; 64]),
    /// Signature data
    Signature(Vec<u8>),
    /// Sign start info
    SignStart { max_length: u32 },
    /// Advertisement
    Advertisement(AdvertisementData),
    /// Path update
    PathUpdate(PathUpdateData),
    /// ACK
    Ack { tag: [u8; 4] },
    /// Trace data
    TraceData(TraceInfo),
    /// Telemetry response (raw LPP data)
    Telemetry(Vec<u8>),
    /// MMA response
    Mma(Vec<MmaEntry>),
    /// ACL response
    Acl(Vec<AclEntry>),
    /// Neighbours response
    Neighbours(NeighboursData),
    /// Binary response
    BinaryResponse { tag: [u8; 4], data: Vec<u8> },
    /// Discover response
    DiscoverResponse(Vec<DiscoverEntry>),
    /// Advert response
    AdvertResponse(AdvertResponseData),
    /// Stats data
    Stats(StatsData),
    /// AutoAdd config
    AutoAddConfig { flags: u8 },
}

/// Contact information
#[derive(Debug, Clone)]
pub struct Contact {
    /// 32-byte public key
    pub public_key: [u8; 32],
    /// Contact type
    pub contact_type: u8,
    /// Contact flags
    pub flags: u8,
    /// Path length (-1 = flood)
    pub path_len: i8,
    /// Output path (up to 64 bytes)
    pub out_path: Vec<u8>,
    /// Advertised name
    pub adv_name: String,
    /// Last advertisement timestamp
    pub last_advert: u32,
    /// Latitude in microdegrees
    pub adv_lat: i32,
    /// Longitude in microdegrees
    pub adv_lon: i32,
    /// Last modification timestamp
    pub last_modification_timestamp: u32,
}

impl Contact {
    /// Get the 6-byte public key prefix
    pub fn prefix(&self) -> [u8; 6] {
        let mut prefix = [0u8; 6];
        prefix.copy_from_slice(&self.public_key[..6]);
        prefix
    }

    /// Get the public key as a hex string
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key)
    }

    /// Get the prefix as a hex string
    pub fn prefix_hex(&self) -> String {
        hex::encode(self.prefix())
    }

    /// Get latitude as decimal degrees
    pub fn latitude(&self) -> f64 {
        self.adv_lat as f64 / 1_000_000.0
    }

    /// Get longitude as decimal degrees
    pub fn longitude(&self) -> f64 {
        self.adv_lon as f64 / 1_000_000.0
    }
}

/// Device self-info
#[derive(Debug, Clone)]
pub struct SelfInfo {
    /// Advertisement type
    pub adv_type: u8,
    /// TX power
    pub tx_power: u8,
    /// Maximum TX power
    pub max_tx_power: u8,
    /// 32-byte public key
    pub public_key: [u8; 32],
    /// Latitude in microdegrees
    pub adv_lat: i32,
    /// Longitude in microdegrees
    pub adv_lon: i32,
    /// Multi ack setting
    pub multi_acks: u8,
    /// Advertisement location policy
    pub adv_loc_policy: u8,
    /// Base telemetry mode (bits 0-1)
    pub telemetry_mode_base: u8,
    /// Location telemetry mode (bits 2-3)
    pub telemetry_mode_loc: u8,
    /// Environment telemetry mode (bits 4-5)
    pub telemetry_mode_env: u8,
    /// Manually add contact setting
    pub manual_add_contacts: bool,
    /// Radio frequency in mHz
    pub radio_freq: u32,
    /// Radio bandwidth in mHz
    pub radio_bw: u32,
    /// Spreading factor
    pub sf: u8,
    /// Coding rate
    pub cr: u8,
    /// Device name
    pub name: String,
}

/// Device info/capabilities
#[derive(Debug, Clone)]
pub struct DeviceInfoData {
    /// Raw device info bytes
    pub raw: Vec<u8>,
}

/// Battery information
#[derive(Debug, Clone)]
pub struct BatteryInfo {
    /// Battery level (0-100 or raw value)
    pub level: u16,
    /// Storage available
    pub storage: u16,
}

/// Received message
#[derive(Debug, Clone)]
pub struct ReceivedMessage {
    /// Sender public key prefix (6 bytes)
    pub sender_prefix: [u8; 6],
    /// Path length
    pub path_len: u8,
    /// Text type (2 = signed)
    pub txt_type: u8,
    /// Sender timestamp
    pub sender_timestamp: u32,
    /// Message text
    pub text: String,
    /// SNR (only in v3, divided by 4)
    pub snr: Option<f32>,
    /// Signature (if txt_type == 2)
    pub signature: Option<[u8; 4]>,
    /// Channel index (for channel messages)
    pub channel: Option<u8>,
}

/// Message sent acknowledgment
#[derive(Debug, Clone)]
pub struct MsgSentInfo {
    /// Message type
    pub message_type: u8,
    /// Expected ACK tag
    pub expected_ack: [u8; 4],
    /// Suggested timeout in milliseconds
    pub suggested_timeout: u32,
}

/// Status data from a device
#[derive(Debug, Clone)]
pub struct StatusData {
    /// Battery level
    pub battery: u16,
    /// TX queue length
    pub tx_queue_len: u16,
    /// Noise floor (dBm)
    pub noise_floor: i16,
    /// Last RSSI (dBm)
    pub last_rssi: i16,
    /// Number of packets received
    pub nb_recv: u32,
    /// Number of packets sent
    pub nb_sent: u32,
    /// Total airtime (ms)
    pub airtime: u32,
    /// Uptime (seconds)
    pub uptime: u32,
    /// Flood packets sent
    pub flood_sent: u32,
    /// Direct packets sent
    pub direct_sent: u32,
    /// SNR (divided by 4)
    pub snr: f32,
    /// Duplicate packet count
    pub dup_count: u32,
    /// RX airtime (ms)
    pub rx_airtime: u32,
    /// Sender public key prefix
    pub sender_prefix: [u8; 6],
}

/// Channel info
#[derive(Debug, Clone)]
pub struct ChannelInfoData {
    /// Channel index
    pub channel_idx: u8,
    /// Channel name
    pub name: String,
    /// Channel secret (16 bytes)
    pub secret: [u8; 16],
}

/// Advertisement data
#[derive(Debug, Clone)]
pub struct AdvertisementData {
    /// Advertiser public key prefix
    pub prefix: [u8; 6],
    /// Advertisement name
    pub name: String,
    /// Latitude in microdegrees
    pub lat: i32,
    /// Longitude in microdegrees
    pub lon: i32,
}

/// Path update data
#[derive(Debug, Clone)]
pub struct PathUpdateData {
    /// Node public key prefix
    pub prefix: [u8; 6],
    /// New path length
    pub path_len: i8,
    /// New path
    pub path: Vec<u8>,
}

/// Trace info
#[derive(Debug, Clone)]
pub struct TraceInfo {
    /// Hops with SNR values
    pub hops: Vec<TraceHop>,
}

/// Single hop in a trace
#[derive(Debug, Clone)]
pub struct TraceHop {
    /// Node prefix
    pub prefix: [u8; 6],
    /// SNR at this hop
    pub snr: f32,
}

/// Min/Max/Avg entry
#[derive(Debug, Clone)]
pub struct MmaEntry {
    /// Channel
    pub channel: u8,
    /// Type
    pub entry_type: u8,
    /// Minimum value
    pub min: f32,
    /// Maximum value
    pub max: f32,
    /// Average value
    pub avg: f32,
}

/// ACL entry
#[derive(Debug, Clone)]
pub struct AclEntry {
    /// Public key prefix (6 bytes)
    pub prefix: [u8; 6],
    /// Permissions
    pub permissions: u8,
}

/// Neighbours response data
#[derive(Debug, Clone)]
pub struct NeighboursData {
    /// Total neighbours available
    pub total: u16,
    /// Neighbors in this response
    pub neighbours: Vec<Neighbour>,
}

/// Single neighbor entry
#[derive(Debug, Clone)]
pub struct Neighbour {
    /// Public key (variable length)
    pub pubkey: Vec<u8>,
    /// Seconds since last seen
    pub secs_ago: i32,
    /// SNR (divided by 4)
    pub snr: f32,
}

/// Discover entry
#[derive(Debug, Clone)]
pub struct DiscoverEntry {
    /// Node public key
    pub pubkey: Vec<u8>,
    /// Node name
    pub name: String,
}

/// Advertisement response data
#[derive(Debug, Clone)]
pub struct AdvertResponseData {
    /// Tag
    pub tag: [u8; 4],
    /// Public key
    pub pubkey: [u8; 32],
    /// Advertisement type
    pub adv_type: u8,
    /// Node name
    pub node_name: String,
    /// Timestamp
    pub timestamp: u32,
    /// Flags
    pub flags: u8,
    /// Latitude (optional)
    pub lat: Option<i32>,
    /// Longitude (optional)
    pub lon: Option<i32>,
    /// Node description (optional)
    pub node_desc: Option<String>,
}

/// Stats data
#[derive(Debug, Clone)]
pub struct StatsData {
    /// Stats category
    pub category: StatsCategory,
    /// Raw stats bytes
    pub raw: Vec<u8>,
}

/// Stats category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsCategory {
    Core,
    Radio,
    Packets,
}

/// An event emitted by the reader
#[derive(Debug, Clone)]
pub struct Event {
    /// Event type
    pub event_type: EventType,
    /// Event payload
    pub payload: EventPayload,
    /// Filterable attributes
    pub attributes: HashMap<String, String>,
}

impl Event {
    /// Create a new event
    pub fn new(event_type: EventType, payload: EventPayload) -> Self {
        Self {
            event_type,
            payload,
            attributes: HashMap::new(),
        }
    }

    /// Add an attribute to the event
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Create an OK event
    pub fn ok() -> Self {
        Self::new(EventType::Ok, EventPayload::None)
    }

    /// Create an error event
    pub fn error(msg: impl Into<String>) -> Self {
        Self::new(EventType::Error, EventPayload::String(msg.into()))
    }

    /// Check if this event matches the given filters
    pub fn matches_filters(&self, filters: &HashMap<String, String>) -> bool {
        filters
            .iter()
            .all(|(k, v)| self.attributes.get(k) == Some(v))
    }
}

/// Subscription handle returned when subscribing to events
#[derive(Debug)]
#[allow(dead_code)]
pub struct Subscription {
    id: u64,
    event_type: EventType,
    unsubscribe_tx: mpsc::Sender<u64>,
}

impl Subscription {
    /// Unsubscribe from events
    pub async fn unsubscribe(self) {
        let _ = self.unsubscribe_tx.send(self.id).await;
    }
}

/// Callback type for event subscriptions
pub type EventCallback = Box<dyn Fn(Event) + Send + Sync>;

struct SubscriptionEntry {
    id: u64,
    event_type: EventType,
    filters: HashMap<String, String>,
    callback: EventCallback,
}

/// Event dispatcher for managing subscriptions and event distribution
pub struct EventDispatcher {
    subscriptions: Arc<RwLock<Vec<SubscriptionEntry>>>,
    next_id: AtomicU64,
    broadcast_tx: broadcast::Sender<Event>,
    unsubscribe_tx: mpsc::Sender<u64>,
    unsubscribe_rx: Arc<RwLock<mpsc::Receiver<u64>>>,
}

impl EventDispatcher {
    /// Create a new event dispatcher
    pub fn new() -> Self {
        let (broadcast_tx, _) = broadcast::channel(256);
        let (unsubscribe_tx, unsubscribe_rx) = mpsc::channel(64);

        Self {
            subscriptions: Arc::new(RwLock::new(Vec::new())),
            next_id: AtomicU64::new(1),
            broadcast_tx,
            unsubscribe_tx,
            unsubscribe_rx: Arc::new(RwLock::new(unsubscribe_rx)),
        }
    }

    /// Subscribe to events of a specific type
    pub async fn subscribe<F>(
        &self,
        event_type: EventType,
        filters: HashMap<String, String>,
        callback: F,
    ) -> Subscription
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let entry = SubscriptionEntry {
            id,
            event_type,
            filters,
            callback: Box::new(callback),
        };

        self.subscriptions.write().await.push(entry);

        Subscription {
            id,
            event_type,
            unsubscribe_tx: self.unsubscribe_tx.clone(),
        }
    }

    /// Emit an event to all matching subscribers
    pub async fn emit(&self, event: Event) {
        // Process any pending unsubscription events
        {
            let mut rx = self.unsubscribe_rx.write().await;
            while let Ok(id) = rx.try_recv() {
                self.subscriptions.write().await.retain(|s| s.id != id);
            }
        }

        // Notify subscribers
        let subs = self.subscriptions.read().await;
        for sub in subs.iter() {
            if sub.event_type == event.event_type && event.matches_filters(&sub.filters) {
                (sub.callback)(event.clone());
            }
        }

        // Also broadcast for wait_for_event
        let _ = self.broadcast_tx.send(event);
    }

    /// Wait for a specific event type with optional filters
    pub async fn wait_for_event(
        &self,
        event_type: EventType,
        filters: HashMap<String, String>,
        timeout: std::time::Duration,
    ) -> Option<Event> {
        let mut rx = self.broadcast_tx.subscribe();

        tokio::select! {
            _ = tokio::time::sleep(timeout) => None,
            result = async {
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            if event.event_type == event_type && event.matches_filters(&filters) {
                                return Some(event);
                            }
                        }
                        Err(_) => return None,
                    }
                }
            } => result,
        }
    }

    /// Get a broadcast receiver for events
    pub fn receiver(&self) -> broadcast::Receiver<Event> {
        self.broadcast_tx.subscribe()
    }
}

impl Default for EventDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

// We need hex for the Contact methods
mod hex {
    pub fn encode(data: impl AsRef<[u8]>) -> String {
        data.as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}
