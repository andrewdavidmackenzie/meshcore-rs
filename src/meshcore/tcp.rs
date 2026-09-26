use crate::meshcore::read_task;
use crate::{Error, MeshCore};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

impl MeshCore {
    /// Create a MeshCore client connected via TCP
    pub async fn tcp(host: &str, port: u16) -> crate::Result<MeshCore> {
        use tokio::io::AsyncWriteExt;

        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
        let meshcore = MeshCore::new_with_sender(tx, None);

        // Connect via TCP
        let addr = format!("{}:{}", host, port);
        let stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| Error::connection(format!("Failed to connect to {}: {}", addr, e)))?;

        let (reader, mut writer) = tokio::io::split(stream);

        // Spawn write task
        let write_task = tokio::spawn(async move {
            while let Some(data) = rx.recv().await {
                let framed = crate::meshcore::frame_packet(&data);
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

        meshcore.tasks.lock().await.push(write_task);
        meshcore.tasks.lock().await.push(read_task);

        *meshcore.connected.write().await = true;

        meshcore.setup_event_handlers().await;

        Ok(meshcore)
    }
}
