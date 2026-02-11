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
        crate::parsing::hex_encode(&self.public_key)
    }

    /// Get the prefix as a hex string
    pub fn prefix_hex(&self) -> String {
        crate::parsing::hex_encode(&self.prefix())
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
pub struct Subscription {
    id: u64,
    #[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn test_event_new() {
        let event = Event::new(EventType::Ok, EventPayload::None);
        assert_eq!(event.event_type, EventType::Ok);
        assert!(matches!(event.payload, EventPayload::None));
        assert!(event.attributes.is_empty());
    }

    #[test]
    fn test_event_with_attribute() {
        let event = Event::new(EventType::Ok, EventPayload::None)
            .with_attribute("key1", "value1")
            .with_attribute("key2", "value2");

        assert_eq!(event.attributes.get("key1"), Some(&"value1".to_string()));
        assert_eq!(event.attributes.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_event_ok() {
        let event = Event::ok();
        assert_eq!(event.event_type, EventType::Ok);
        assert!(matches!(event.payload, EventPayload::None));
    }

    #[test]
    fn test_event_error() {
        let event = Event::error("test error");
        assert_eq!(event.event_type, EventType::Error);
        match event.payload {
            EventPayload::String(s) => assert_eq!(s, "test error"),
            _ => panic!("Expected String payload"),
        }
    }

    #[test]
    fn test_event_matches_filters_empty() {
        let event = Event::new(EventType::Ok, EventPayload::None);
        let filters = HashMap::new();
        assert!(event.matches_filters(&filters));
    }

    #[test]
    fn test_event_matches_filters_match() {
        let event = Event::new(EventType::Ok, EventPayload::None).with_attribute("tag", "abc123");

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), "abc123".to_string());
        assert!(event.matches_filters(&filters));
    }

    #[test]
    fn test_event_matches_filters_no_match() {
        let event = Event::new(EventType::Ok, EventPayload::None).with_attribute("tag", "abc123");

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), "xyz789".to_string());
        assert!(!event.matches_filters(&filters));
    }

    #[test]
    fn test_event_matches_filters_missing_attr() {
        let event = Event::new(EventType::Ok, EventPayload::None);

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), "abc123".to_string());
        assert!(!event.matches_filters(&filters));
    }

    #[test]
    fn test_contact_prefix() {
        let contact = Contact {
            public_key: [
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
                0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C,
                0x1D, 0x1E, 0x1F, 0x20,
            ],
            contact_type: 1,
            flags: 0,
            path_len: -1,
            out_path: Vec::new(),
            adv_name: "Test".to_string(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: 0,
            last_modification_timestamp: 0,
        };

        assert_eq!(contact.prefix(), [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    }

    #[test]
    fn test_contact_public_key_hex() {
        let mut public_key = [0u8; 32];
        public_key[0..4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        let contact = Contact {
            public_key,
            contact_type: 1,
            flags: 0,
            path_len: -1,
            out_path: Vec::new(),
            adv_name: "Test".to_string(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: 0,
            last_modification_timestamp: 0,
        };

        assert!(contact.public_key_hex().starts_with("deadbeef"));
    }

    #[test]
    fn test_contact_prefix_hex() {
        let mut public_key = [0u8; 32];
        public_key[0..6].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02]);

        let contact = Contact {
            public_key,
            contact_type: 1,
            flags: 0,
            path_len: -1,
            out_path: Vec::new(),
            adv_name: "Test".to_string(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: 0,
            last_modification_timestamp: 0,
        };

        assert_eq!(contact.prefix_hex(), "deadbeef0102");
    }

    #[test]
    fn test_contact_latitude() {
        let contact = Contact {
            public_key: [0u8; 32],
            contact_type: 1,
            flags: 0,
            path_len: -1,
            out_path: Vec::new(),
            adv_name: "Test".to_string(),
            last_advert: 0,
            adv_lat: 37774900, // 37.7749 degrees
            adv_lon: 0,
            last_modification_timestamp: 0,
        };

        assert!((contact.latitude() - 37.7749).abs() < 0.0001);
    }

    #[test]
    fn test_contact_longitude() {
        let contact = Contact {
            public_key: [0u8; 32],
            contact_type: 1,
            flags: 0,
            path_len: -1,
            out_path: Vec::new(),
            adv_name: "Test".to_string(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: -122419400, // -122.4194 degrees
            last_modification_timestamp: 0,
        };

        assert!((contact.longitude() - (-122.4194)).abs() < 0.0001);
    }

    #[tokio::test]
    async fn test_event_dispatcher_new() {
        let dispatcher = EventDispatcher::new();
        // Just verify it can be created
        let _receiver = dispatcher.receiver();
    }

    #[tokio::test]
    async fn test_event_dispatcher_default() {
        let dispatcher = EventDispatcher::default();
        let _receiver = dispatcher.receiver();
    }

    #[tokio::test]
    async fn test_event_dispatcher_emit() {
        let dispatcher = EventDispatcher::new();
        let mut receiver = dispatcher.receiver();

        dispatcher.emit(Event::ok()).await;

        let received = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(received.event_type, EventType::Ok);
    }

    #[tokio::test]
    async fn test_event_dispatcher_subscribe() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let _subscription = dispatcher
            .subscribe(EventType::Ok, HashMap::new(), move |_event| {
                call_count_clone.fetch_add(1, Ordering::SeqCst);
            })
            .await;

        dispatcher.emit(Event::ok()).await;

        // Give time for callback to execute
        tokio::time::sleep(Duration::from_millis(10)).await;

        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_event_dispatcher_subscribe_with_filter() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), "match".to_string());

        let _subscription = dispatcher
            .subscribe(EventType::Ack, filters, move |_event| {
                call_count_clone.fetch_add(1, Ordering::SeqCst);
            })
            .await;

        // This should NOT trigger callback (wrong filter)
        dispatcher
            .emit(Event::new(EventType::Ack, EventPayload::None).with_attribute("tag", "nomatch"))
            .await;

        // This SHOULD trigger callback
        dispatcher
            .emit(Event::new(EventType::Ack, EventPayload::None).with_attribute("tag", "match"))
            .await;

        tokio::time::sleep(Duration::from_millis(10)).await;

        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_event_dispatcher_unsubscribe() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let subscription = dispatcher
            .subscribe(EventType::Ok, HashMap::new(), move |_event| {
                call_count_clone.fetch_add(1, Ordering::SeqCst);
            })
            .await;

        // First emit should trigger
        dispatcher.emit(Event::ok()).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // Unsubscribe
        subscription.unsubscribe().await;

        // This emit should process unsubscription
        dispatcher.emit(Event::ok()).await;
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Third emit should NOT trigger (unsubscribed)
        dispatcher.emit(Event::ok()).await;
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Should still be 2 (second emit counted, third did not)
        assert!(call_count.load(Ordering::SeqCst) <= 2);
    }

    #[tokio::test]
    async fn test_event_dispatcher_wait_for_event() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let dispatcher_clone = dispatcher.clone();

        // Spawn a task that emits after a short delay
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            dispatcher_clone.emit(Event::ok()).await;
        });

        let result = dispatcher
            .wait_for_event(EventType::Ok, HashMap::new(), Duration::from_millis(100))
            .await;

        assert!(result.is_some());
        assert_eq!(result.unwrap().event_type, EventType::Ok);
    }

    #[tokio::test]
    async fn test_event_dispatcher_wait_for_event_timeout() {
        let dispatcher = EventDispatcher::new();

        let result = dispatcher
            .wait_for_event(EventType::Ok, HashMap::new(), Duration::from_millis(10))
            .await;

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_event_dispatcher_wait_for_event_with_filter() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let dispatcher_clone = dispatcher.clone();

        // Spawn a task that emits events
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            // First, emit with wrong filter
            dispatcher_clone
                .emit(Event::new(EventType::Ack, EventPayload::None).with_attribute("tag", "wrong"))
                .await;
            tokio::time::sleep(Duration::from_millis(5)).await;
            // Then emit with correct filter
            dispatcher_clone
                .emit(
                    Event::new(EventType::Ack, EventPayload::None).with_attribute("tag", "correct"),
                )
                .await;
        });

        let mut filters = HashMap::new();
        filters.insert("tag".to_string(), "correct".to_string());

        let result = dispatcher
            .wait_for_event(EventType::Ack, filters, Duration::from_millis(100))
            .await;

        assert!(result.is_some());
        assert_eq!(
            result.unwrap().attributes.get("tag"),
            Some(&"correct".to_string())
        );
    }

    #[test]
    fn test_event_type_debug() {
        assert_eq!(format!("{:?}", EventType::Connected), "Connected");
        assert_eq!(format!("{:?}", EventType::Disconnected), "Disconnected");
    }

    #[test]
    fn test_event_type_clone_eq() {
        let e1 = EventType::SelfInfo;
        let e2 = e1;
        assert_eq!(e1, e2);
    }

    #[test]
    fn test_event_payload_clone() {
        let payload = EventPayload::String("test".to_string());
        let cloned = payload.clone();
        match cloned {
            EventPayload::String(s) => assert_eq!(s, "test"),
            _ => panic!("Wrong payload type"),
        }
    }

    #[test]
    fn test_stats_category_eq() {
        assert_eq!(StatsCategory::Core, StatsCategory::Core);
        assert_ne!(StatsCategory::Core, StatsCategory::Radio);
    }

    #[test]
    fn test_self_info_clone() {
        let info = SelfInfo {
            adv_type: 1,
            tx_power: 20,
            max_tx_power: 30,
            public_key: [0u8; 32],
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
            name: "Test".to_string(),
        };

        let cloned = info.clone();
        assert_eq!(cloned.tx_power, 20);
        assert_eq!(cloned.name, "Test");
    }

    #[test]
    fn test_received_message_clone() {
        let msg = ReceivedMessage {
            sender_prefix: [0x01, 0x02, 0x03, 0x04, 0x05, 0x06],
            path_len: 2,
            txt_type: 1,
            sender_timestamp: 1234567890,
            text: "Hello".to_string(),
            snr: Some(10.0),
            signature: None,
            channel: None,
        };

        let cloned = msg.clone();
        assert_eq!(cloned.text, "Hello");
        assert_eq!(cloned.snr, Some(10.0));
    }

    #[test]
    fn test_battery_info_debug() {
        let info = BatteryInfo {
            level: 85,
            storage: 100,
        };
        let debug_str = format!("{:?}", info);
        assert!(debug_str.contains("85"));
    }

    #[test]
    fn test_channel_info_data_clone() {
        let info = ChannelInfoData {
            channel_idx: 1,
            name: "General".to_string(),
            secret: [0xAA; 16],
        };
        let cloned = info.clone();
        assert_eq!(cloned.channel_idx, 1);
        assert_eq!(cloned.name, "General");
    }

    #[test]
    fn test_advertisement_data_debug() {
        let advert = AdvertisementData {
            prefix: [0x01, 0x02, 0x03, 0x04, 0x05, 0x06],
            name: "Node1".to_string(),
            lat: 37774900,
            lon: -122419400,
        };
        let debug_str = format!("{:?}", advert);
        assert!(debug_str.contains("Node1"));
    }

    #[test]
    fn test_path_update_data_clone() {
        let update = PathUpdateData {
            prefix: [0x01, 0x02, 0x03, 0x04, 0x05, 0x06],
            path_len: 3,
            path: vec![0x0A, 0x0B, 0x0C],
        };
        let cloned = update.clone();
        assert_eq!(cloned.path_len, 3);
        assert_eq!(cloned.path, vec![0x0A, 0x0B, 0x0C]);
    }

    #[test]
    fn test_trace_hop_clone() {
        let hop = TraceHop {
            prefix: [0x01, 0x02, 0x03, 0x04, 0x05, 0x06],
            snr: 10.5,
        };
        let cloned = hop.clone();
        assert_eq!(cloned.snr, 10.5);
    }

    #[test]
    fn test_neighbour_clone() {
        let neighbour = Neighbour {
            pubkey: vec![0x01, 0x02, 0x03],
            secs_ago: 300,
            snr: 8.0,
        };
        let cloned = neighbour.clone();
        assert_eq!(cloned.secs_ago, 300);
    }

    #[test]
    fn test_discover_entry_clone() {
        let entry = DiscoverEntry {
            pubkey: vec![0x01, 0x02, 0x03],
            name: "Node".to_string(),
        };
        let cloned = entry.clone();
        assert_eq!(cloned.name, "Node");
    }

    #[test]
    fn test_event_payload_variants() {
        // Test various payload types
        let _none = EventPayload::None;
        let _string = EventPayload::String("test".to_string());
        let _bytes = EventPayload::Bytes(vec![1, 2, 3]);
        let _time = EventPayload::Time(1234567890);
        let _private_key = EventPayload::PrivateKey([0u8; 64]);
        let _signature = EventPayload::Signature(vec![1, 2, 3, 4]);
        let _sign_start = EventPayload::SignStart { max_length: 1000 };
        let _ack = EventPayload::Ack {
            tag: [0x01, 0x02, 0x03, 0x04],
        };
        let _binary = EventPayload::BinaryResponse {
            tag: [0x01, 0x02, 0x03, 0x04],
            data: vec![5, 6, 7, 8],
        };
        let _auto_add = EventPayload::AutoAddConfig { flags: 0x01 };
    }
}
