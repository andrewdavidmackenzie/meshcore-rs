//! Main MeshCore client implementation

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, RwLock};
use uuid::Uuid;
use crate::commands::CommandHandler;
use crate::connection::frame_packet;
use crate::events::*;
use crate::reader::MessageReader;
use crate::Error;
use crate::Result;

// MeshCore BLE service and characteristic UUIDs
// These are the standard UUIDs used by MeshCore devices
pub const MESHCORE_SERVICE_UUID: Uuid = Uuid::from_u128(0x6e400001_b5a3_f393_e0a9_e50e24dcca9e);
pub const MESHCORE_TX_CHAR_UUID: Uuid = Uuid::from_u128(0x6e400002_b5a3_f393_e0a9_e50e24dcca9e);
pub const MESHCORE_RX_CHAR_UUID: Uuid = Uuid::from_u128(0x6e400003_b5a3_f393_e0a9_e50e24dcca9e);


/// MeshCore client for communicating with MeshCore devices
pub struct MeshCore {
    /// Event dispatcher
    dispatcher: Arc<EventDispatcher>,
    /// Message reader
    reader: Arc<MessageReader>,
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
    connected: Arc<RwLock<bool>>,
    /// Auto message fetching subscription
    auto_fetch_sub: Arc<Mutex<Option<Subscription>>>,
    /// Background tasks
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl MeshCore {
    /// Create a new MeshCore client with a custom connection
    fn new_with_sender(sender: mpsc::Sender<Vec<u8>>) -> Self {
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

    /// Create a MeshCore client connected via serial port
    #[cfg(feature = "serial")]
    pub async fn serial(port: &str, baud_rate: u32) -> Result<Self> {
        use bytes::BytesMut;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio_serial::SerialPortBuilderExt;

        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
        let meshcore = Self::new_with_sender(tx);

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

        // TODO the read task should be extractable from the discovery methods I think
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

    /// Create a MeshCore client connected via BLE
    ///
    /// This method scans for BLE devices and connects to a MeshCore device.
    /// If `device_name` is provided, it will connect to a device with that name.
    /// Otherwise, it will connect to the first device advertising the MeshCore service.
    #[cfg(feature = "ble")]
    pub async fn ble(device_name: Option<&str>) -> Result<Self> {
        use btleplug::api::{
            Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter, WriteType,
        };
        use btleplug::platform::{Manager, Peripheral};
        use futures::stream::StreamExt;
        use uuid::Uuid;

        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
        let meshcore = Self::new_with_sender(tx);

        // Get the Bluetooth adapter
        let manager = Manager::new()
            .await
            .map_err(|e| Error::connection(format!("Failed to create BLE manager: {}", e)))?;

        let adapters = manager
            .adapters()
            .await
            .map_err(|e| Error::connection(format!("Failed to get BLE adapters: {}", e)))?;

        let adapter = adapters
            .into_iter()
            .next()
            .ok_or_else(|| Error::connection("No BLE adapters found"))?;

        // Subscribe to adapter events
        let mut events = adapter
            .events()
            .await
            .map_err(|e| Error::connection(format!("Failed to get adapter events: {}", e)))?;

        // Start scanning
        adapter
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| Error::connection(format!("Failed to start BLE scan: {}", e)))?;

        tracing::info!("Scanning for MeshCore devices...");

        // Find the target device
        let target_peripheral: Option<Peripheral> = {
            let timeout = tokio::time::timeout(Duration::from_secs(30), async {
                while let Some(event) = events.next().await {
                    if let CentralEvent::DeviceDiscovered(id) = event {
                        if let Ok(peripheral) = adapter.peripheral(&id).await {
                            if let Ok(Some(props)) = peripheral.properties().await {
                                let name = props.local_name.as_deref();
                                tracing::debug!("Found device: {:?}", name);

                                // Check if this is our target device
                                let is_target = if let Some(target_name) = device_name {
                                    name.map(|n| n.contains(target_name)).unwrap_or(false)
                                } else {
                                    // Check if device advertises MeshCore service
                                    props.services.contains(&MESHCORE_SERVICE_UUID)
                                        || name
                                            .map(|n| {
                                                n.contains("MeshCore") || n.contains("meshcore")
                                            })
                                            .unwrap_or(false)
                                };

                                if is_target {
                                    tracing::info!("Found MeshCore device: {:?}", props.local_name);
                                    return Some(peripheral);
                                }
                            }
                        }
                    }
                }
                None
            })
            .await;

            timeout.unwrap_or_else(|_| None)
        };

        // Stop scanning
        let _ = adapter.stop_scan().await;

        let peripheral =
            target_peripheral.ok_or_else(|| Error::connection("MeshCore device not found"))?;

        // Check if already connected, disconnect first if so
        if peripheral.is_connected().await.unwrap_or(false) {
            tracing::info!("Device already connected, disconnecting first...");
            let _ = peripheral.disconnect().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // Connect to the device with retry
        let mut connect_attempts = 0;
        const MAX_CONNECT_ATTEMPTS: u32 = 3;

        loop {
            connect_attempts += 1;
            tracing::info!(
                "Connecting to device (attempt {}/{})",
                connect_attempts,
                MAX_CONNECT_ATTEMPTS
            );

            match peripheral.connect().await {
                Ok(_) => {
                    tracing::info!("Connected to MeshCore device");
                    break;
                }
                Err(e) => {
                    tracing::warn!("Connection attempt {} failed: {}", connect_attempts, e);
                    if connect_attempts >= MAX_CONNECT_ATTEMPTS {
                        return Err(Error::connection(format!(
                            "Failed to connect after {} attempts: {}",
                            MAX_CONNECT_ATTEMPTS, e
                        )));
                    }
                    // Short delay before retry
                    tokio::time::sleep(Duration::from_millis(1000)).await;
                }
            }
        }

        // Discover services
        peripheral
            .discover_services()
            .await
            .map_err(|e| Error::connection(format!("Failed to discover services: {}", e)))?;

        // Find the MeshCore service and characteristics
        let services = peripheral.services();
        let meshcore_service = services
            .iter()
            .find(|s| s.uuid == MESHCORE_SERVICE_UUID)
            .ok_or_else(|| Error::connection("MeshCore service not found on device"))?;

        let tx_char = meshcore_service
            .characteristics
            .iter()
            .find(|c| c.uuid == MESHCORE_TX_CHAR_UUID)
            .ok_or_else(|| Error::connection("TX characteristic not found"))?
            .clone();

        let rx_char = meshcore_service
            .characteristics
            .iter()
            .find(|c| c.uuid == MESHCORE_RX_CHAR_UUID)
            .ok_or_else(|| Error::connection("RX characteristic not found"))?
            .clone();

        // Subscribe to notifications on RX characteristic
        peripheral
            .subscribe(&rx_char)
            .await
            .map_err(|e| Error::connection(format!("Failed to subscribe to RX: {}", e)))?;

        tracing::info!("Subscribed to MeshCore notifications");

        // Clone peripheral for tasks
        let peripheral_write = peripheral.clone();
        let peripheral_read = peripheral.clone();

        // Spawn write task
        // BLE does NOT use framing - send raw payload directly (unlike serial which uses [0x3c][len][payload])
        let write_task = tokio::spawn(async move {
            while let Some(data) = rx.recv().await {
                tracing::debug!("BLE TX: {} bytes: {:02x?}", data.len(), &data);
                // BLE has MTU limits, so we may need to chunk the data
                for chunk in data.chunks(244) {
                    match peripheral_write
                        .write(&tx_char, chunk, WriteType::WithoutResponse)
                        .await
                    {
                        Ok(_) => tracing::trace!("BLE TX chunk: {} bytes sent", chunk.len()),
                        Err(e) => {
                            tracing::error!("BLE TX error: {}", e);
                            break;
                        }
                    }
                }
            }
        });

        // Spawn read task
        let msg_reader = meshcore.reader.clone();
        let connected = meshcore.connected.clone();
        let dispatcher = meshcore.dispatcher.clone();

        let read_task = tokio::spawn(async move {
            let mut notification_stream = match peripheral_read.notifications().await {
                Ok(stream) => stream,
                Err(_) => {
                    *connected.write().await = false;
                    dispatcher
                        .emit(Event::new(EventType::Disconnected, EventPayload::None))
                        .await;
                    return;
                }
            };

            while let Some(data) = notification_stream.next().await {
                // BLE does NOT use framing - each notification IS a complete packet
                // (unlike serial which uses [0x3c][len][payload])
                let frame = data.value;
                tracing::debug!(
                    "BLE RX: type=0x{:02x}, len={}, data={:02x?}",
                    frame.first().unwrap_or(&0),
                    frame.len(),
                    &frame
                );

                if !frame.is_empty() {
                    if let Err(e) = msg_reader.handle_rx(frame).await {
                        tracing::error!("Error handling BLE message: {}", e);
                    }
                }
            }

            // Notification stream ended - disconnected
            *connected.write().await = false;
            dispatcher
                .emit(Event::new(EventType::Disconnected, EventPayload::None))
                .await;
        });

        meshcore.tasks.lock().await.push(write_task);
        meshcore.tasks.lock().await.push(read_task);

        *meshcore.connected.write().await = true;

        meshcore.setup_event_handlers().await;

        Ok(meshcore)
    }

    /// Create a MeshCore client connected via TCP
    #[cfg(feature = "tcp")]
    pub async fn tcp(host: &str, port: u16) -> Result<Self> {
        use bytes::BytesMut;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
        let meshcore = Self::new_with_sender(tx);

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

        // TODO the read task should be extractable from the discovery methods I think
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
        F: Fn(Event) + Send + Sync + 'static,
    {
        self.dispatcher
            .subscribe(event_type, filters, callback)
            .await
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
