//! Main MeshCore client implementation

use crate::commands::CommandHandler;
use crate::events::*;
#[cfg(any(feature = "serial", feature = "tcp"))]
use crate::packets::FRAME_START;
use crate::reader::MessageReader;
use crate::Result;
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
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
    /// ```ignore
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
    /// ```ignore
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
    let len = data.len() as u16;
    let mut framed = Vec::with_capacity(3 + data.len());
    framed.push(FRAME_START);
    framed.push((len & 0xFF) as u8);
    framed.push((len >> 8) as u8);
    framed.extend_from_slice(data);
    framed
}

#[cfg(test)]
#[cfg(any(feature = "serial", feature = "tcp"))]
mod tests {
    use super::*;

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
}
