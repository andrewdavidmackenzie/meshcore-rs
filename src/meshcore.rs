//! Main MeshCore client implementation

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, RwLock};

use crate::commands::CommandHandler;
use crate::connection::frame_packet;
use crate::events::*;
use crate::reader::MessageReader;
use crate::Error;
use crate::Result;

/// MeshCore client for communicating with MeshCore devices
#[allow(dead_code)]
pub struct MeshCore {
    /// Event dispatcher
    dispatcher: Arc<EventDispatcher>,
    /// Message reader
    reader: Arc<MessageReader>,
    /// Command handler
    commands: Arc<Mutex<CommandHandler>>,
    /// Contact cache
    contacts: Arc<RwLock<HashMap<String, Contact>>>,
    /// Self info cache
    self_info: Arc<RwLock<Option<SelfInfo>>>,
    /// Device time cache
    device_time: Arc<RwLock<Option<u32>>>,
    /// Contacts dirty flag
    contacts_dirty: Arc<RwLock<bool>>,
    /// Connection state
    connected: Arc<RwLock<bool>>,
    /// Sender for outgoing data
    sender: mpsc::Sender<Vec<u8>>,
    /// Auto message fetching subscription
    auto_fetch_sub: Arc<Mutex<Option<Subscription>>>,
    /// Default timeout
    default_timeout: Duration,
    /// Background tasks
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl MeshCore {
    /// Create a new MeshCore client with a custom connection
    fn new_with_sender(sender: mpsc::Sender<Vec<u8>>, default_timeout: Duration) -> Self {
        let dispatcher = Arc::new(EventDispatcher::new());
        let reader = Arc::new(MessageReader::new(dispatcher.clone()));

        let commands = CommandHandler::new(sender.clone(), dispatcher.clone(), reader.clone());

        Self {
            dispatcher,
            reader,
            commands: Arc::new(Mutex::new(commands)),
            contacts: Arc::new(RwLock::new(HashMap::new())),
            self_info: Arc::new(RwLock::new(None)),
            device_time: Arc::new(RwLock::new(None)),
            contacts_dirty: Arc::new(RwLock::new(true)),
            connected: Arc::new(RwLock::new(false)),
            sender,
            auto_fetch_sub: Arc::new(Mutex::new(None)),
            default_timeout,
            tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Create a MeshCore client connected via serial port
    #[cfg(feature = "serial")]
    pub async fn serial(port: &str, baud_rate: u32) -> Result<Self> {
        use bytes::BytesMut;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio_serial::SerialPortBuilderExt;

        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
        let meshcore = Self::new_with_sender(tx, Duration::from_secs(5));

        // Open serial port
        let port = tokio_serial::new(port, baud_rate)
            .open_native_async()
            .map_err(|e| Error::connection(format!("Failed to open serial port: {}", e)))?;

        let (mut reader, mut writer) = tokio::io::split(port);

        // Spawn write task
        let write_task = tokio::spawn(async move {
            while let Some(data) = rx.recv().await {
                let framed = frame_packet(&data);
                if writer.write_all(&framed).await.is_err() {
                    break;
                }
            }
        });

        // Spawn read task
        let msg_reader = meshcore.reader.clone();
        let connected = meshcore.connected.clone();
        let dispatcher = meshcore.dispatcher.clone();

        let read_task = tokio::spawn(async move {
            let mut buffer = BytesMut::with_capacity(4096);
            let mut read_buf = [0u8; 1024];

            loop {
                match reader.read(&mut read_buf).await {
                    Ok(0) => {
                        // Connection closed
                        *connected.write().await = false;
                        dispatcher
                            .emit(Event::new(EventType::Disconnected, EventPayload::None))
                            .await;
                        break;
                    }
                    Ok(n) => {
                        buffer.extend_from_slice(&read_buf[..n]);

                        // Parse frames
                        while buffer.len() >= 3 {
                            if buffer[0] != crate::packets::FRAME_START {
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
                            .emit(Event::new(EventType::Disconnected, EventPayload::None))
                            .await;
                        break;
                    }
                }
            }
        });

        // Store tasks
        meshcore.tasks.lock().await.push(write_task);
        meshcore.tasks.lock().await.push(read_task);

        // Mark as connected
        *meshcore.connected.write().await = true;

        // Set up internal event handlers
        meshcore.setup_event_handlers().await;

        Ok(meshcore)
    }

    /// Create a MeshCore client connected via TCP
    #[cfg(feature = "tcp")]
    pub async fn tcp(host: &str, port: u16) -> Result<Self> {
        use bytes::BytesMut;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
        let meshcore = Self::new_with_sender(tx, Duration::from_secs(5));

        // Connect via TCP
        let addr = format!("{}:{}", host, port);
        let stream = tokio::net::TcpStream::connect(&addr)
            .await
            .map_err(|e| Error::connection(format!("Failed to connect to {}: {}", addr, e)))?;

        let (mut reader, mut writer) = tokio::io::split(stream);

        // Spawn write task
        let write_task = tokio::spawn(async move {
            while let Some(data) = rx.recv().await {
                let framed = frame_packet(&data);
                if writer.write_all(&framed).await.is_err() {
                    break;
                }
            }
        });

        // Spawn read task
        let msg_reader = meshcore.reader.clone();
        let connected = meshcore.connected.clone();
        let dispatcher = meshcore.dispatcher.clone();

        let read_task = tokio::spawn(async move {
            let mut buffer = BytesMut::with_capacity(4096);
            let mut read_buf = [0u8; 1024];

            loop {
                match reader.read(&mut read_buf).await {
                    Ok(0) => {
                        *connected.write().await = false;
                        dispatcher
                            .emit(Event::new(EventType::Disconnected, EventPayload::None))
                            .await;
                        break;
                    }
                    Ok(n) => {
                        buffer.extend_from_slice(&read_buf[..n]);

                        while buffer.len() >= 3 {
                            if buffer[0] != crate::packets::FRAME_START {
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
                            .emit(Event::new(EventType::Disconnected, EventPayload::None))
                            .await;
                        break;
                    }
                }
            }
        });

        meshcore.tasks.lock().await.push(write_task);
        meshcore.tasks.lock().await.push(read_task);

        *meshcore.connected.write().await = true;

        meshcore.setup_event_handlers().await;

        Ok(meshcore)
    }

    /// Set up internal event handlers for caching
    async fn setup_event_handlers(&self) {
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

        // Subscribe to self info updates
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

    /// Get cached self info
    pub async fn self_info(&self) -> Option<SelfInfo> {
        self.self_info.read().await.clone()
    }

    /// Get cached device time
    pub async fn device_time(&self) -> Option<u32> {
        *self.device_time.read().await
    }

    /// Check if contacts cache is dirty
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
        F: Fn(Event) + Send + Sync + 'static,
    {
        self.dispatcher.subscribe(event_type, filters, callback).await
    }

    /// Wait for a specific event
    pub async fn wait_for_event(
        &self,
        event_type: EventType,
        filters: HashMap<String, String>,
        timeout: Duration,
    ) -> Option<Event> {
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
                                // Message already emitted by reader
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
            .emit(Event::new(EventType::Disconnected, EventPayload::None))
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
}

impl Drop for MeshCore {
    fn drop(&mut self) {
        // Tasks will be aborted when the JoinHandles are dropped
    }
}
