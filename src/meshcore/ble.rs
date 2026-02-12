use crate::events::EventPayload;
use crate::{Error, Event, EventType, MeshCore};
use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Manager, Peripheral};
use futures::stream::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Receiver;
use uuid::Uuid;

// MeshCore BLE service and characteristic UUIDs
// These are the standard UUIDs used by MeshCore devices
const MESHCORE_SERVICE_UUID: Uuid = Uuid::from_u128(0x6e400001_b5a3_f393_e0a9_e50e24dcca9e);
const MESHCORE_TX_CHAR_UUID: Uuid = Uuid::from_u128(0x6e400002_b5a3_f393_e0a9_e50e24dcca9e);
const MESHCORE_RX_CHAR_UUID: Uuid = Uuid::from_u128(0x6e400003_b5a3_f393_e0a9_e50e24dcca9e);

impl MeshCore {
    /// Find MeshCore radios on BTLE upto the time specified by `scan_duration` and return their names
    pub async fn ble_discover(scan_duration: Duration) -> crate::Result<Vec<String>> {
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
            .start_scan(ScanFilter {
                services: vec![MESHCORE_SERVICE_UUID],
            })
            .await
            .map_err(|e| Error::connection(format!("Failed to start BLE scan: {}", e)))?;

        tracing::info!("Scanning for MeshCore devices...");

        let mut discovered_meshcore_radios = Vec::new();

        let _ = tokio::time::timeout(scan_duration, async {
            while let Some(event) = events.next().await {
                if let CentralEvent::DeviceDiscovered(id) = event {
                    if let Ok(peripheral) = adapter.peripheral(&id).await {
                        if let Ok(Some(props)) = peripheral.properties().await {
                            if let Some(name) = &props.local_name {
                                discovered_meshcore_radios.push(name.clone());
                            }
                        }
                    }
                }
            }
        })
        .await;

        // Stop scanning
        let _ = adapter.stop_scan().await;
        tracing::info!("Stopped scanning for MeshCore devices...");

        Ok(discovered_meshcore_radios)
    }

    /// Connect to a Btle peripheral that is a MeshCore radio and return the [MeshCore] to use to
    /// communicate with it
    async fn ble_connect_peripheral(
        peripheral: &Peripheral,
    ) -> crate::Result<(MeshCore, Receiver<Vec<u8>>, Characteristic)> {
        // Check if already connected, disconnect first if so
        if peripheral.is_connected().await.unwrap_or(false) {
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

        let (tx, rx) = mpsc::channel::<Vec<u8>>(64);
        Ok((MeshCore::new_with_sender(tx), rx, tx_char))
    }

    /// Given a peripheral's name, get the peripheral object
    async fn find_peripheral(target_name: &str) -> crate::Result<Peripheral> {
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

        adapter
            .start_scan(ScanFilter {
                services: vec![MESHCORE_SERVICE_UUID],
            })
            .await
            .map_err(|e| Error::connection(format!("Failed to start BLE scan: {}", e)))?;

        let target_peripheral: Option<Peripheral> = {
            let timeout = tokio::time::timeout(Duration::from_secs(1), async {
                while let Some(event) = events.next().await {
                    if let CentralEvent::DeviceDiscovered(id) = event {
                        if let Ok(peripheral) = adapter.peripheral(&id).await {
                            if let Ok(Some(props)) = peripheral.properties().await {
                                if props.local_name.as_deref() == Some(target_name) {
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

        adapter
            .stop_scan()
            .await
            .map_err(|e| Error::connection(format!("Failed to stop BLE scan: {}", e)))?;

        target_peripheral.ok_or_else(|| Error::connection("MeshCore device not found"))
    }

    /// This method connects to a MeshCore radio by BTLE device name
    pub async fn ble_connect(name: &str) -> crate::Result<MeshCore> {
        let peripheral = Self::find_peripheral(name).await?;
        let (meshcore, mut rx, tx_char) = Self::ble_connect_peripheral(&peripheral).await?;

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
}
