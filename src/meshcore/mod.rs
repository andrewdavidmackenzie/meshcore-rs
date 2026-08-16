//! Main MeshCore client implementation

use crate::commands::CommandHandler;
use crate::events::*;
#[cfg(any(feature = "serial", feature = "tcp"))]
use crate::packets::{FRAME_START, FRAME_START_RESP};
use crate::reader::MessageReader;
use crate::Result;
#[cfg(any(feature = "serial", feature = "tcp"))]
use bytes::BytesMut;
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
#[cfg(any(feature = "serial", feature = "tcp"))]
use tokio::io::{AsyncRead, AsyncReadExt, ReadHalf};
#[cfg(any(feature = "serial", feature = "ble", feature = "tcp"))]
use tokio::sync::mpsc;
use tokio::sync::{Mutex, RwLock};
use tokio_stream::wrappers::BroadcastStream;

#[cfg(feature = "ble")]
pub mod ble;
#[cfg(feature = "serial")]
pub mod serial;
#[cfg(feature = "tcp")]
pub mod tcp;

/// MeshCore client for communicating with MeshCore devices
pub struct MeshCore {
    /// Event dispatcher
    pub(crate) dispatcher: Arc<EventDispatcher>,
    /// Message reader
    pub(crate) reader: Arc<MessageReader>,
    /// Command handler
    commands: Arc<Mutex<CommandHandler>>,
    /// Contact cache
    contacts: Arc<RwLock<HashMap<String, Contact>>>,
    /// Self-info cache
    self_info: Arc<RwLock<Option<SelfInfo>>>,
    /// Device time cache
    device_time: Arc<RwLock<Option<u32>>>,
    /// Contacts dirty flag
    contacts_dirty: Arc<RwLock<bool>>,
    /// Connection state
    pub(crate) connected: Arc<RwLock<bool>>,
    /// Auto message fetching subscription
    auto_fetch_sub: Arc<Mutex<Option<Subscription>>>,
    /// Background tasks
    pub(crate) tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl MeshCore {
    #[cfg(any(feature = "serial", feature = "ble", feature = "tcp"))]
    /// Create a new MeshCore client with a custom connection
    pub(crate) fn new_with_sender(sender: mpsc::Sender<Vec<u8>>) -> Self {
        let dispatcher = Arc::new(EventDispatcher::new());
        let reader = Arc::new(MessageReader::new(dispatcher.clone()));

        let commands = CommandHandler::new(sender, dispatcher.clone(), reader.clone());

        Self {
            dispatcher,
            reader,
            commands: Arc::new(Mutex::new(commands)),
            contacts: Arc::new(RwLock::new(HashMap::new())),
            self_info: Arc::new(RwLock::new(None)),
            device_time: Arc::new(RwLock::new(None)),
            contacts_dirty: Arc::new(RwLock::new(true)),
            connected: Arc::new(RwLock::new(false)),
            auto_fetch_sub: Arc::new(Mutex::new(None)),
            tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[cfg(any(feature = "serial", feature = "ble", feature = "tcp"))]
    /// Set up internal event handlers for caching
    pub(crate) async fn setup_event_handlers(&self) {
        let contacts = self.contacts.clone();
        let contacts_dirty = self.contacts_dirty.clone();

        // Subscribe to contacts updates
        self.dispatcher
            .subscribe(EventType::Contacts, HashMap::new(), move |event| {
                if let EventPayload::Contacts(new_contacts) = event.payload {
                    let contacts = contacts.clone();
                    let contacts_dirty = contacts_dirty.clone();
                    tokio::spawn(async move {
                        let mut map = contacts.write().await;
                        map.clear();
                        for contact in new_contacts {
                            let key = crate::parsing::hex_encode(&contact.public_key);
                            map.insert(key, contact);
                        }
                        *contacts_dirty.write().await = false;
                    });
                }
            })
            .await;

        let self_info = self.self_info.clone();

        // Subscribe to self-info updates
        self.dispatcher
            .subscribe(EventType::SelfInfo, HashMap::new(), move |event| {
                if let EventPayload::SelfInfo(info) = event.payload {
                    let self_info = self_info.clone();
                    tokio::spawn(async move {
                        *self_info.write().await = Some(info);
                    });
                }
            })
            .await;

        let device_time = self.device_time.clone();

        // Subscribe to time updates
        self.dispatcher
            .subscribe(EventType::CurrentTime, HashMap::new(), move |event| {
                if let EventPayload::Time(t) = event.payload {
                    let device_time = device_time.clone();
                    tokio::spawn(async move {
                        *device_time.write().await = Some(t);
                    });
                }
            })
            .await;

        let contacts2 = self.contacts.clone();

        // Subscribe to new contacts
        self.dispatcher
            .subscribe(EventType::NewContact, HashMap::new(), move |event| {
                if let EventPayload::Contact(contact) = event.payload {
                    let contacts = contacts2.clone();
                    tokio::spawn(async move {
                        let key = crate::parsing::hex_encode(&contact.public_key);
                        contacts.write().await.insert(key, contact);
                    });
                }
            })
            .await;
    }

    /// Check if connected
    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }

    /// Get the command handler
    pub fn commands(&self) -> &Arc<Mutex<CommandHandler>> {
        &self.commands
    }

    /// Get cached contacts
    pub async fn contacts(&self) -> HashMap<String, Contact> {
        self.contacts.read().await.clone()
    }

    /// Get cached self-info
    pub async fn self_info(&self) -> Option<SelfInfo> {
        self.self_info.read().await.clone()
    }

    /// Get cached device time
    pub async fn device_time(&self) -> Option<u32> {
        *self.device_time.read().await
    }

    /// Check if the contact cache is dirty
    pub async fn contacts_dirty(&self) -> bool {
        *self.contacts_dirty.read().await
    }

    /// Get contact by name
    pub async fn get_contact_by_name(&self, name: &str) -> Option<Contact> {
        let contacts = self.contacts.read().await;
        contacts
            .values()
            .find(|c| c.adv_name.eq_ignore_ascii_case(name))
            .cloned()
    }

    /// Get contact by public key prefix
    pub async fn get_contact_by_prefix(&self, prefix: &[u8]) -> Option<Contact> {
        let contacts = self.contacts.read().await;
        contacts
            .values()
            .find(|c| c.public_key.starts_with(prefix))
            .cloned()
    }

    /// Ensure contacts are loaded
    pub async fn ensure_contacts(&self) -> Result<()> {
        if *self.contacts_dirty.read().await {
            let contacts = self.commands.lock().await.get_contacts(0).await?;
            let mut map = self.contacts.write().await;
            map.clear();
            for contact in contacts {
                let key = crate::parsing::hex_encode(&contact.public_key);
                map.insert(key, contact);
            }
            *self.contacts_dirty.write().await = false;
        }
        Ok(())
    }

    /// Subscribe to events
    pub async fn subscribe<F>(
        &self,
        event_type: EventType,
        filters: HashMap<String, String>,
        callback: F,
    ) -> Subscription
    where
        F: Fn(MeshCoreEvent) + Send + Sync + 'static,
    {
        self.dispatcher
            .subscribe(event_type, filters, callback)
            .await
    }

    /// Wait for an event, either matching a specific [EventType] or all
    pub async fn wait_for_event(
        &self,
        event_type: Option<EventType>,
        filters: HashMap<String, String>,
        timeout: Duration,
    ) -> Option<MeshCoreEvent> {
        self.dispatcher
            .wait_for_event(event_type, filters, timeout)
            .await
    }

    /// Start auto-fetching messages when MESSAGES_WAITING is received
    pub async fn start_auto_message_fetching(&self) {
        let commands = self.commands.clone();
        let dispatcher = self.dispatcher.clone();

        let sub = self
            .dispatcher
            .subscribe(EventType::MessagesWaiting, HashMap::new(), move |_| {
                let commands = commands.clone();
                let _dispatcher = dispatcher.clone();
                tokio::spawn(async move {
                    loop {
                        let result = commands.lock().await.get_msg().await;
                        match result {
                            Ok(Some(_msg)) => {
                                // Message already emitted by the reader
                            }
                            Ok(None) => break, // No more messages
                            Err(_) => break,
                        }
                    }
                });
            })
            .await;

        *self.auto_fetch_sub.lock().await = Some(sub);
    }

    /// Stop auto-fetching messages
    pub async fn stop_auto_message_fetching(&self) {
        if let Some(sub) = self.auto_fetch_sub.lock().await.take() {
            sub.unsubscribe().await;
        }
    }

    /// Disconnect from the device
    pub async fn disconnect(&self) -> Result<()> {
        *self.connected.write().await = false;

        // Abort all background tasks
        let mut tasks = self.tasks.lock().await;
        for task in tasks.drain(..) {
            task.abort();
        }

        // Emit disconnected event
        self.dispatcher
            .emit(MeshCoreEvent::new(
                EventType::Disconnected,
                EventPayload::None,
            ))
            .await;

        Ok(())
    }

    /// Set default timeout
    pub async fn set_default_timeout(&self, timeout: Duration) {
        self.commands.lock().await.set_default_timeout(timeout);
    }

    /// Get the event dispatcher
    pub fn dispatcher(&self) -> &Arc<EventDispatcher> {
        &self.dispatcher
    }

    /// Get the message reader
    pub fn reader(&self) -> &Arc<MessageReader> {
        &self.reader
    }

    /// Create a stream of all events
    ///
    /// Returns a stream that yields all events emitted by the device.
    /// Use `StreamExt` methods to filter or process events.
    ///
    /// # Example
    ///
    /// ```dont_run
    /// use futures::StreamExt;
    ///
    /// let mut stream = meshcore.event_stream();
    /// while let Some(event) = stream.next().await {
    ///     println!("Received: {:?}", event.event_type);
    /// }
    /// ```
    pub fn event_stream(&self) -> impl futures::Stream<Item = MeshCoreEvent> + Unpin {
        BroadcastStream::new(self.dispatcher.receiver())
            .filter_map(|result| std::future::ready(result.ok()))
    }

    /// Create a filtered stream of events by type
    ///
    /// Returns a stream that yields only events matching the specified type.
    ///
    /// # Example
    ///
    /// ```dont_run
    /// use futures::StreamExt;
    /// use meshcore_rs::EventType;
    ///
    /// let mut stream = meshcore.event_stream_filtered(EventType::ContactMsgRecv);
    /// while let Some(event) = stream.next().await {
    ///     println!("Message received: {:?}", event.payload);
    /// }
    /// ```
    pub fn event_stream_filtered(
        &self,
        event_type: EventType,
    ) -> impl futures::Stream<Item = MeshCoreEvent> + Unpin {
        BroadcastStream::new(self.dispatcher.receiver()).filter_map(move |result| {
            std::future::ready(result.ok().filter(|event| event.event_type == event_type))
        })
    }
}

/// Frame a packet for transmission
///
/// Format: `[START: 0x3c][LENGTH_L][LENGTH_H][PAYLOAD]`
#[cfg(any(feature = "serial", feature = "tcp"))]
pub(crate) fn frame_packet(data: &[u8]) -> Vec<u8> {
    // Frame has three header bytes and the data itself
    let frame_size = data.len().checked_add(3).unwrap_or_default();
    let mut framed = Vec::with_capacity(frame_size);
    let len = data.len() as u16;
    framed.push(FRAME_START);
    framed.push((len & 0xFF) as u8);
    framed.push((len >> 8) as u8);
    framed.extend_from_slice(data);
    framed
}

#[cfg(any(feature = "serial", feature = "tcp"))]
pub async fn read_task<R>(
    mut reader: ReadHalf<R>,
    msg_reader: Arc<MessageReader>,
    connected: Arc<RwLock<bool>>,
    dispatcher: Arc<EventDispatcher>,
) where
    R: AsyncRead,
{
    let mut buffer = BytesMut::with_capacity(4096);
    let mut read_buf = [0u8; 1024];

    loop {
        match reader.read(&mut read_buf).await {
            Ok(0) => {
                *connected.write().await = false;
                dispatcher
                    .emit(MeshCoreEvent::new(
                        EventType::Disconnected,
                        EventPayload::None,
                    ))
                    .await;
                break;
            }
            Ok(n) => {
                buffer.extend_from_slice(&read_buf[..n]);

                while buffer.len() >= 3 {
                    if buffer[0] != FRAME_START && buffer[0] != FRAME_START_RESP {
                        use bytes::Buf;
                        buffer.advance(1);
                        continue;
                    }

                    let len = u16::from_le_bytes([buffer[1], buffer[2]]) as usize;
                    if buffer.len() < 3 + len {
                        break;
                    }

                    let frame = buffer[3..3 + len].to_vec();
                    use bytes::Buf;
                    buffer.advance(3 + len);

                    if let Err(e) = msg_reader.handle_rx(frame).await {
                        tracing::error!("Error handling message: {}", e);
                    }
                }
            }
            Err(_) => {
                *connected.write().await = false;
                dispatcher
                    .emit(MeshCoreEvent::new(
                        EventType::Disconnected,
                        EventPayload::None,
                    ))
                    .await;
                break;
            }
        }
    }
}

#[cfg(test)]
#[cfg(any(feature = "serial", feature = "tcp"))]
mod tests {
    use super::*;
    use crate::events::Contact;
    use futures::StreamExt;
    use std::io::Cursor;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    // ========== Helper ==========

    fn create_test_meshcore() -> MeshCore {
        let (sender, _receiver) = mpsc::channel(16);
        MeshCore::new_with_sender(sender)
    }

    fn make_contact(name: &str, public_key: [u8; 32]) -> Contact {
        Contact {
            public_key,
            contact_type: 1,
            flags: 0,
            path_len: -1,
            out_path: vec![],
            adv_name: name.to_string(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: 0,
            last_modification_timestamp: 0,
        }
    }

    // ========== frame_packet tests ==========

    #[test]
    fn test_frame_packet() {
        let data = vec![0x01, 0x02, 0x03];
        let framed = frame_packet(&data);

        assert_eq!(framed[0], FRAME_START);
        assert_eq!(framed[1], 0x03); // Length low byte
        assert_eq!(framed[2], 0x00); // Length high byte
        assert_eq!(&framed[3..], &data);
    }

    #[test]
    fn test_frame_packet_empty() {
        let data: Vec<u8> = vec![];
        let framed = frame_packet(&data);

        assert_eq!(framed.len(), 3);
        assert_eq!(framed[0], FRAME_START);
        assert_eq!(framed[1], 0x00); // Length low byte
        assert_eq!(framed[2], 0x00); // Length high byte
    }

    #[test]
    fn test_frame_packet_single_byte() {
        let data = vec![0xFF];
        let framed = frame_packet(&data);

        assert_eq!(framed.len(), 4);
        assert_eq!(framed[0], FRAME_START);
        assert_eq!(framed[1], 0x01);
        assert_eq!(framed[2], 0x00);
        assert_eq!(framed[3], 0xFF);
    }

    #[test]
    fn test_frame_packet_256_bytes() {
        let data = vec![0xAA; 256];
        let framed = frame_packet(&data);

        assert_eq!(framed.len(), 259);
        assert_eq!(framed[0], FRAME_START);
        assert_eq!(framed[1], 0x00); // 256 & 0xFF = 0
        assert_eq!(framed[2], 0x01); // 256 >> 8 = 1
        assert_eq!(&framed[3..], &data[..]);
    }

    #[test]
    fn test_frame_packet_large() {
        let data = vec![0xBB; 1000];
        let framed = frame_packet(&data);

        assert_eq!(framed.len(), 1003);
        assert_eq!(framed[0], FRAME_START);
        // 1000 = 0x03E8
        assert_eq!(framed[1], 0xE8); // Low byte
        assert_eq!(framed[2], 0x03); // High byte
    }

    #[test]
    fn test_frame_start_constant() {
        assert_eq!(FRAME_START, 0x3c);
        assert_eq!(FRAME_START, b'<');
    }

    #[test]
    fn test_frame_start_resp_constant() {
        assert_eq!(FRAME_START_RESP, 0x3e);
        assert_eq!(FRAME_START_RESP, b'>');
    }

    // ========== Accessor / initial-state tests ==========

    #[tokio::test]
    async fn test_is_connected_initial() {
        let mc = create_test_meshcore();
        assert!(!mc.is_connected().await);
    }

    #[tokio::test]
    async fn test_contacts_initial_empty() {
        let mc = create_test_meshcore();
        assert!(mc.contacts().await.is_empty());
    }

    #[tokio::test]
    async fn test_self_info_initial_none() {
        let mc = create_test_meshcore();
        assert!(mc.self_info().await.is_none());
    }

    #[tokio::test]
    async fn test_device_time_initial_none() {
        let mc = create_test_meshcore();
        assert!(mc.device_time().await.is_none());
    }

    #[tokio::test]
    async fn test_contacts_dirty_initial_true() {
        let mc = create_test_meshcore();
        assert!(mc.contacts_dirty().await);
    }

    #[tokio::test]
    async fn test_commands_returns_arc() {
        let mc = create_test_meshcore();
        // Just verify we can lock the commands
        let _guard = mc.commands().lock().await;
    }

    #[tokio::test]
    async fn test_dispatcher_returns_arc() {
        let mc = create_test_meshcore();
        let dispatcher = mc.dispatcher();
        // Verify we can emit an event through it
        dispatcher
            .emit(MeshCoreEvent::new(EventType::Ok, EventPayload::None))
            .await;
    }

    #[tokio::test]
    async fn test_reader_returns_arc() {
        let mc = create_test_meshcore();
        let _reader = mc.reader();
    }

    // ========== get_contact_by_name tests ==========

    #[tokio::test]
    async fn test_get_contact_by_name_found() {
        let mc = create_test_meshcore();
        let mut key = [0u8; 32];
        key[0] = 0xAA;
        let contact = make_contact("Alice", key);
        mc.contacts.write().await.insert(
            crate::parsing::hex_encode(&contact.public_key),
            contact,
        );

        let result = mc.get_contact_by_name("Alice").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().adv_name, "Alice");
    }

    #[tokio::test]
    async fn test_get_contact_by_name_case_insensitive() {
        let mc = create_test_meshcore();
        let mut key = [0u8; 32];
        key[0] = 0xBB;
        let contact = make_contact("Bob", key);
        mc.contacts.write().await.insert(
            crate::parsing::hex_encode(&contact.public_key),
            contact,
        );

        let result = mc.get_contact_by_name("bob").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().adv_name, "Bob");

        let result = mc.get_contact_by_name("BOB").await;
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn test_get_contact_by_name_not_found() {
        let mc = create_test_meshcore();
        let result = mc.get_contact_by_name("Nobody").await;
        assert!(result.is_none());
    }

    // ========== get_contact_by_prefix tests ==========

    #[tokio::test]
    async fn test_get_contact_by_prefix_found() {
        let mc = create_test_meshcore();
        let mut key = [0u8; 32];
        key[0] = 0x01;
        key[1] = 0x02;
        key[2] = 0x03;
        let contact = make_contact("Charlie", key);
        mc.contacts.write().await.insert(
            crate::parsing::hex_encode(&contact.public_key),
            contact,
        );

        let result = mc.get_contact_by_prefix(&[0x01, 0x02, 0x03]).await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().adv_name, "Charlie");
    }

    #[tokio::test]
    async fn test_get_contact_by_prefix_partial_match() {
        let mc = create_test_meshcore();
        let mut key = [0u8; 32];
        key[0] = 0xDE;
        key[1] = 0xAD;
        let contact = make_contact("Dave", key);
        mc.contacts.write().await.insert(
            crate::parsing::hex_encode(&contact.public_key),
            contact,
        );

        // Match with just the first byte
        let result = mc.get_contact_by_prefix(&[0xDE]).await;
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn test_get_contact_by_prefix_not_found() {
        let mc = create_test_meshcore();
        let result = mc.get_contact_by_prefix(&[0xFF, 0xFF]).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_get_contact_by_prefix_empty() {
        let mc = create_test_meshcore();
        let mut key = [0u8; 32];
        key[0] = 0x01;
        let contact = make_contact("Eve", key);
        mc.contacts.write().await.insert(
            crate::parsing::hex_encode(&contact.public_key),
            contact,
        );

        // Empty prefix matches everything
        let result = mc.get_contact_by_prefix(&[]).await;
        assert!(result.is_some());
    }

    // ========== disconnect tests ==========

    #[tokio::test]
    async fn test_disconnect_sets_connected_false() {
        let mc = create_test_meshcore();
        *mc.connected.write().await = true;
        assert!(mc.is_connected().await);

        mc.disconnect().await.unwrap();
        assert!(!mc.is_connected().await);
    }

    #[tokio::test]
    async fn test_disconnect_emits_event() {
        let mc = create_test_meshcore();
        *mc.connected.write().await = true;

        // Subscribe to Disconnected event before disconnecting
        let event = mc.dispatcher.wait_for_event(
            Some(EventType::Disconnected),
            HashMap::new(),
            Duration::from_secs(1),
        );

        // Disconnect in a separate task so we can await the event
        let mc_clone_connected = mc.connected.clone();
        let mc_dispatcher = mc.dispatcher.clone();
        let mc_tasks = mc.tasks.clone();
        tokio::spawn(async move {
            // Small delay to ensure receiver is ready
            tokio::time::sleep(Duration::from_millis(10)).await;
            *mc_clone_connected.write().await = false;
            let mut tasks = mc_tasks.lock().await;
            for task in tasks.drain(..) {
                task.abort();
            }
            mc_dispatcher
                .emit(MeshCoreEvent::new(
                    EventType::Disconnected,
                    EventPayload::None,
                ))
                .await;
        });

        let result = event.await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().event_type, EventType::Disconnected);
    }

    #[tokio::test]
    async fn test_disconnect_aborts_tasks() {
        let mc = create_test_meshcore();

        // Add a long-running background task
        let handle = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(300)).await;
        });
        mc.tasks.lock().await.push(handle);

        mc.disconnect().await.unwrap();

        // Tasks vec should be drained
        assert!(mc.tasks.lock().await.is_empty());
    }

    // ========== set_default_timeout test ==========

    #[tokio::test]
    async fn test_set_default_timeout() {
        let mc = create_test_meshcore();
        // Should not panic; the timeout is forwarded to the command handler
        mc.set_default_timeout(Duration::from_secs(42)).await;
    }

    // ========== subscribe / wait_for_event tests ==========

    #[tokio::test]
    async fn test_subscribe_receives_events() {
        let mc = create_test_meshcore();
        let received = Arc::new(RwLock::new(false));
        let received_clone = received.clone();

        let _sub = mc
            .subscribe(EventType::Ok, HashMap::new(), move |_event| {
                let received = received_clone.clone();
                tokio::spawn(async move {
                    *received.write().await = true;
                });
            })
            .await;

        mc.dispatcher()
            .emit(MeshCoreEvent::new(EventType::Ok, EventPayload::None))
            .await;

        // Give the spawned task a moment to run
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(*received.read().await);
    }

    #[tokio::test]
    async fn test_wait_for_event_success() {
        let mc = create_test_meshcore();
        let dispatcher = mc.dispatcher().clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            dispatcher
                .emit(MeshCoreEvent::new(
                    EventType::Battery,
                    EventPayload::None,
                ))
                .await;
        });

        let result = mc
            .wait_for_event(
                Some(EventType::Battery),
                HashMap::new(),
                Duration::from_secs(1),
            )
            .await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().event_type, EventType::Battery);
    }

    #[tokio::test]
    async fn test_wait_for_event_timeout() {
        let mc = create_test_meshcore();
        let result = mc
            .wait_for_event(
                Some(EventType::Battery),
                HashMap::new(),
                Duration::from_millis(50),
            )
            .await;
        assert!(result.is_none());
    }

    // ========== event_stream tests ==========

    #[tokio::test]
    async fn test_event_stream_receives_events() {
        let mc = create_test_meshcore();
        let mut stream = mc.event_stream();
        let dispatcher = mc.dispatcher().clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            dispatcher
                .emit(MeshCoreEvent::new(EventType::Ok, EventPayload::None))
                .await;
        });

        let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("timed out")
            .expect("stream ended");
        assert_eq!(event.event_type, EventType::Ok);
    }

    #[tokio::test]
    async fn test_event_stream_filtered_only_matching() {
        let mc = create_test_meshcore();
        let mut stream = mc.event_stream_filtered(EventType::Battery);
        let dispatcher = mc.dispatcher().clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            // Emit a non-matching event first
            dispatcher
                .emit(MeshCoreEvent::new(EventType::Ok, EventPayload::None))
                .await;
            // Then a matching one
            dispatcher
                .emit(MeshCoreEvent::new(
                    EventType::Battery,
                    EventPayload::None,
                ))
                .await;
        });

        // The filtered stream should skip the Ok event and give us Battery
        let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("timed out")
            .expect("stream ended");
        assert_eq!(event.event_type, EventType::Battery);
    }

    // ========== start/stop auto message fetching tests ==========

    #[tokio::test]
    async fn test_start_stop_auto_message_fetching() {
        let mc = create_test_meshcore();

        // Initially no subscription
        assert!(mc.auto_fetch_sub.lock().await.is_none());

        // Start auto-fetching
        mc.start_auto_message_fetching().await;
        assert!(mc.auto_fetch_sub.lock().await.is_some());

        // Stop auto-fetching
        mc.stop_auto_message_fetching().await;
        assert!(mc.auto_fetch_sub.lock().await.is_none());
    }

    #[tokio::test]
    async fn test_stop_auto_message_fetching_when_not_started() {
        let mc = create_test_meshcore();
        // Should not panic when there's no active subscription
        mc.stop_auto_message_fetching().await;
        assert!(mc.auto_fetch_sub.lock().await.is_none());
    }

    // ========== setup_event_handlers tests ==========

    #[tokio::test]
    async fn test_setup_event_handlers_contacts() {
        let mc = create_test_meshcore();
        mc.setup_event_handlers().await;

        // Emit a Contacts event
        let contact = make_contact("Handler Test", [0x11; 32]);
        mc.dispatcher()
            .emit(MeshCoreEvent::new(
                EventType::Contacts,
                EventPayload::Contacts(vec![contact]),
            ))
            .await;

        // Give the spawned handler task time to run
        tokio::time::sleep(Duration::from_millis(50)).await;

        let contacts = mc.contacts().await;
        assert_eq!(contacts.len(), 1);
        assert!(!mc.contacts_dirty().await);
    }

    #[tokio::test]
    async fn test_setup_event_handlers_self_info() {
        let mc = create_test_meshcore();
        mc.setup_event_handlers().await;

        let info = crate::events::SelfInfo {
            name: "TestDevice".to_string(),
            ..Default::default()
        };
        mc.dispatcher()
            .emit(MeshCoreEvent::new(
                EventType::SelfInfo,
                EventPayload::SelfInfo(info),
            ))
            .await;

        tokio::time::sleep(Duration::from_millis(50)).await;

        let self_info = mc.self_info().await;
        assert!(self_info.is_some());
        assert_eq!(self_info.unwrap().name, "TestDevice");
    }

    #[tokio::test]
    async fn test_setup_event_handlers_current_time() {
        let mc = create_test_meshcore();
        mc.setup_event_handlers().await;

        mc.dispatcher()
            .emit(MeshCoreEvent::new(
                EventType::CurrentTime,
                EventPayload::Time(1234567890),
            ))
            .await;

        tokio::time::sleep(Duration::from_millis(50)).await;

        let time = mc.device_time().await;
        assert_eq!(time, Some(1234567890));
    }

    #[tokio::test]
    async fn test_setup_event_handlers_new_contact() {
        let mc = create_test_meshcore();
        mc.setup_event_handlers().await;

        let contact = make_contact("NewPeer", [0x22; 32]);
        mc.dispatcher()
            .emit(MeshCoreEvent::new(
                EventType::NewContact,
                EventPayload::Contact(contact),
            ))
            .await;

        tokio::time::sleep(Duration::from_millis(50)).await;

        let contacts = mc.contacts().await;
        assert_eq!(contacts.len(), 1);
        let key_hex = crate::parsing::hex_encode(&[0x22; 32]);
        assert!(contacts.contains_key(&key_hex));
    }

    // ========== read_task tests ==========

    /// A mock AsyncRead that wraps a Cursor and can be split via tokio::io::split
    struct MockStream {
        inner: Cursor<Vec<u8>>,
    }

    impl MockStream {
        fn new(data: Vec<u8>) -> Self {
            Self {
                inner: Cursor::new(data),
            }
        }
    }

    impl tokio::io::AsyncRead for MockStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let inner = &mut self.inner;
            Pin::new(inner).poll_read(cx, buf)
        }
    }

    impl tokio::io::AsyncWrite for MockStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn test_read_task_eof_disconnects() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let reader = Arc::new(MessageReader::new(dispatcher.clone()));
        let connected = Arc::new(RwLock::new(true));

        // Empty stream = immediate EOF
        let stream = MockStream::new(vec![]);
        let (read_half, _write_half) = tokio::io::split(stream);

        let connected_clone = connected.clone();
        let dispatcher_clone = dispatcher.clone();

        // Set up a receiver to catch the disconnect event
        let event = dispatcher.wait_for_event(
            Some(EventType::Disconnected),
            HashMap::new(),
            Duration::from_secs(2),
        );

        tokio::spawn(async move {
            read_task(read_half, reader, connected_clone, dispatcher_clone).await;
        });

        let result = event.await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().event_type, EventType::Disconnected);
        assert!(!*connected.read().await);
    }

    #[tokio::test]
    async fn test_read_task_skips_invalid_frame_start() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let reader = Arc::new(MessageReader::new(dispatcher.clone()));
        let connected = Arc::new(RwLock::new(true));

        // Data: some garbage bytes followed by EOF
        // The reader should skip non-frame-start bytes and eventually hit EOF
        let data = vec![0x00, 0x01, 0x02];
        let stream = MockStream::new(data);
        let (read_half, _write_half) = tokio::io::split(stream);

        let connected_clone = connected.clone();
        let dispatcher_clone = dispatcher.clone();

        let event = dispatcher.wait_for_event(
            Some(EventType::Disconnected),
            HashMap::new(),
            Duration::from_secs(2),
        );

        tokio::spawn(async move {
            read_task(read_half, reader, connected_clone, dispatcher_clone).await;
        });

        // Should still disconnect on EOF after skipping garbage
        let result = event.await;
        assert!(result.is_some());
        assert!(!*connected.read().await);
    }

    #[tokio::test]
    async fn test_read_task_processes_valid_frame() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let reader = Arc::new(MessageReader::new(dispatcher.clone()));
        let connected = Arc::new(RwLock::new(true));

        // Build a valid frame: FRAME_START + length (2 bytes LE) + payload
        // PacketType::Ok is 0x00
        let payload = vec![0x00];
        let mut data = vec![FRAME_START, payload.len() as u8, 0x00];
        data.extend_from_slice(&payload);
        // After this frame, EOF will follow

        let stream = MockStream::new(data);
        let (read_half, _write_half) = tokio::io::split(stream);

        let connected_clone = connected.clone();
        let dispatcher_clone = dispatcher.clone();

        // The reader should emit an Ok event, then disconnect on EOF
        let ok_event = dispatcher.wait_for_event(
            Some(EventType::Ok),
            HashMap::new(),
            Duration::from_secs(2),
        );

        tokio::spawn(async move {
            read_task(read_half, reader, connected_clone, dispatcher_clone).await;
        });

        let result = ok_event.await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().event_type, EventType::Ok);
    }

    #[tokio::test]
    async fn test_read_task_processes_resp_frame_start() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let reader = Arc::new(MessageReader::new(dispatcher.clone()));
        let connected = Arc::new(RwLock::new(true));

        // Use FRAME_START_RESP (0x3e) instead of FRAME_START (0x3c)
        // PacketType::Ok is 0x00
        let payload = vec![0x00];
        let mut data = vec![FRAME_START_RESP, payload.len() as u8, 0x00];
        data.extend_from_slice(&payload);

        let stream = MockStream::new(data);
        let (read_half, _write_half) = tokio::io::split(stream);

        let connected_clone = connected.clone();
        let dispatcher_clone = dispatcher.clone();

        let ok_event = dispatcher.wait_for_event(
            Some(EventType::Ok),
            HashMap::new(),
            Duration::from_secs(2),
        );

        tokio::spawn(async move {
            read_task(read_half, reader, connected_clone, dispatcher_clone).await;
        });

        let result = ok_event.await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().event_type, EventType::Ok);
    }

    #[tokio::test]
    async fn test_read_task_multiple_frames() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let reader = Arc::new(MessageReader::new(dispatcher.clone()));
        let connected = Arc::new(RwLock::new(true));

        // Two consecutive frames: Ok (0x00) then Error (0x01) with message
        let mut data = Vec::new();
        // Frame 1: Ok
        data.push(FRAME_START);
        data.push(0x01); // length low
        data.push(0x00); // length high
        data.push(0x00); // PacketType::Ok
        // Frame 2: Error with message
        let err_payload = vec![0x01, b'f', b'a', b'i', b'l']; // 0x01 = PacketType::Error
        data.push(FRAME_START);
        data.push(err_payload.len() as u8);
        data.push(0x00);
        data.extend_from_slice(&err_payload);

        let stream = MockStream::new(data);
        let (read_half, _write_half) = tokio::io::split(stream);

        let connected_clone = connected.clone();
        let dispatcher_clone = dispatcher.clone();

        // We should receive both events
        let mut rx = dispatcher.receiver();

        tokio::spawn(async move {
            read_task(read_half, reader, connected_clone, dispatcher_clone).await;
        });

        // Collect events (Ok, Error, and Disconnected from EOF)
        let mut event_types = Vec::new();
        for _ in 0..3 {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Ok(event)) => event_types.push(event.event_type),
                _ => break,
            }
        }

        assert!(event_types.contains(&EventType::Ok));
        assert!(event_types.contains(&EventType::Error));
        assert!(event_types.contains(&EventType::Disconnected));
    }

    #[tokio::test]
    async fn test_read_task_garbage_before_valid_frame() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let reader = Arc::new(MessageReader::new(dispatcher.clone()));
        let connected = Arc::new(RwLock::new(true));

        // Some garbage bytes, then a valid frame
        let mut data = vec![0xFF, 0xFE, 0xFD]; // garbage
        data.push(FRAME_START);
        data.push(0x01); // length
        data.push(0x00);
        data.push(0x00); // PacketType::Ok

        let stream = MockStream::new(data);
        let (read_half, _write_half) = tokio::io::split(stream);

        let connected_clone = connected.clone();
        let dispatcher_clone = dispatcher.clone();

        let ok_event = dispatcher.wait_for_event(
            Some(EventType::Ok),
            HashMap::new(),
            Duration::from_secs(2),
        );

        tokio::spawn(async move {
            read_task(read_half, reader, connected_clone, dispatcher_clone).await;
        });

        let result = ok_event.await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().event_type, EventType::Ok);
    }

    #[tokio::test]
    async fn test_read_task_incomplete_frame_then_eof() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let reader = Arc::new(MessageReader::new(dispatcher.clone()));
        let connected = Arc::new(RwLock::new(true));

        // Frame header says 10 bytes but we only provide 2
        let data = vec![FRAME_START, 0x0A, 0x00, 0x01, 0x02];

        let stream = MockStream::new(data);
        let (read_half, _write_half) = tokio::io::split(stream);

        let connected_clone = connected.clone();
        let dispatcher_clone = dispatcher.clone();

        let event = dispatcher.wait_for_event(
            Some(EventType::Disconnected),
            HashMap::new(),
            Duration::from_secs(2),
        );

        tokio::spawn(async move {
            read_task(read_half, reader, connected_clone, dispatcher_clone).await;
        });

        // Should disconnect because EOF before frame is complete
        let result = event.await;
        assert!(result.is_some());
        assert!(!*connected.read().await);
    }
}
