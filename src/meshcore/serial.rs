use crate::events::EventPayload;
use crate::meshcore::frame_packet;
use crate::packets::FRAME_START;
use crate::{Error, Event, EventType, MeshCore};
use tokio::sync::mpsc;

impl MeshCore {
    /// Create a MeshCore client connected via serial port
    pub async fn serial(port: &str, baud_rate: u32) -> crate::Result<MeshCore> {
        use bytes::BytesMut;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio_serial::SerialPortBuilderExt;

        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
        let meshcore = MeshCore::new_with_sender(tx);

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
                            if buffer[0] != FRAME_START {
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
}
