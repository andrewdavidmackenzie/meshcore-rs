use crate::meshcore::{frame_packet, read_task};
use crate::{Error, MeshCore};
use tokio::sync::mpsc;

impl MeshCore {
    /// Create a MeshCore client connected via serial port
    pub async fn serial(port: &str, baud_rate: u32) -> crate::Result<MeshCore> {
        use tokio::io::AsyncWriteExt;
        use tokio_serial::SerialPortBuilderExt;

        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
        let meshcore = MeshCore::new_with_sender(tx, None);

        // Open serial port
        let port = tokio_serial::new(port, baud_rate)
            .open_native_async()
            .map_err(|e| Error::connection(format!("Failed to open serial port: {}", e)))?;

        let (reader, mut writer) = tokio::io::split(port);

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

        let read_task = tokio::spawn(read_task(reader, msg_reader, connected, dispatcher));

        // Store tasks
        meshcore.tasks.lock().await.push(write_task);
        meshcore.tasks.lock().await.push(read_task);

        // Mark as connected
        *meshcore.connected.write().await = true;

        // Set up internal event handlers
        meshcore.setup_event_handlers().await;

        Ok(meshcore)
    }
}
