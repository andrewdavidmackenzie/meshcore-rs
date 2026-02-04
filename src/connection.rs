//! Connection types for MeshCore communication
//!
//! This module provides async connection implementations for serial, TCP, and BLE.

use async_trait::async_trait;
use bytes::BytesMut;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, Mutex};

use crate::error::Error;
use crate::packets::FRAME_START;
use crate::Result;

/// Callback type for received data
pub type DataCallback = Box<dyn Fn(Vec<u8>) + Send + Sync>;

/// Callback type for disconnect events
pub type DisconnectCallback = Box<dyn Fn() + Send + Sync>;

/// Connection trait for different transport types
#[async_trait]
pub trait Connection: Send + Sync {
    /// Connect to the device
    async fn connect(&mut self) -> Result<()>;

    /// Disconnect from the device
    async fn disconnect(&mut self) -> Result<()>;

    /// Send raw data to the device
    async fn send(&self, data: &[u8]) -> Result<()>;

    /// Check if connected
    fn is_connected(&self) -> bool;

    /// Set the data receive callback
    fn set_reader(&mut self, callback: DataCallback);

    /// Set the disconnect callback
    fn set_disconnect_callback(&mut self, callback: DisconnectCallback);
}

/// Frame a packet for transmission
///
/// Format: [START: 0x3c][LENGTH_L][LENGTH_H][PAYLOAD]
pub fn frame_packet(data: &[u8]) -> Vec<u8> {
    let len = data.len() as u16;
    let mut framed = Vec::with_capacity(3 + data.len());
    framed.push(FRAME_START);
    framed.push((len & 0xFF) as u8);
    framed.push((len >> 8) as u8);
    framed.extend_from_slice(data);
    framed
}

/// Serial connection implementation
#[cfg(feature = "serial")]
#[allow(dead_code)]
pub struct SerialConnection {
    port_name: String,
    baud_rate: u32,
    port: Option<tokio_serial::SerialStream>,
    connected: Arc<std::sync::atomic::AtomicBool>,
    reader_tx: Option<mpsc::Sender<Vec<u8>>>,
    read_task: Option<tokio::task::JoinHandle<()>>,
    data_callback: Option<DataCallback>,
    disconnect_callback: Option<DisconnectCallback>,
}

#[cfg(feature = "serial")]
impl SerialConnection {
    /// Create a new serial connection
    pub fn new(port_name: impl Into<String>, baud_rate: u32) -> Self {
        Self {
            port_name: port_name.into(),
            baud_rate,
            port: None,
            connected: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            reader_tx: None,
            read_task: None,
            data_callback: None,
            disconnect_callback: None,
        }
    }
}

#[cfg(feature = "serial")]
#[async_trait]
impl Connection for SerialConnection {
    async fn connect(&mut self) -> Result<()> {
        use std::sync::atomic::Ordering;
        use tokio_serial::SerialPortBuilderExt;

        let port = tokio_serial::new(&self.port_name, self.baud_rate)
            .open_native_async()
            .map_err(|e| Error::connection(format!("Failed to open serial port: {}", e)))?;

        self.port = Some(port);
        self.connected.store(true, Ordering::SeqCst);

        // Start read task
        self.start_read_task();

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        use std::sync::atomic::Ordering;

        self.connected.store(false, Ordering::SeqCst);

        if let Some(task) = self.read_task.take() {
            task.abort();
        }

        self.port = None;
        Ok(())
    }

    async fn send(&self, _data: &[u8]) -> Result<()> {
        // We need interior mutability for the port
        // This is a simplified implementation - in production you'd use Arc<Mutex<>>
        Err(Error::connection(
            "Serial send requires mutable access - use Arc<Mutex<SerialConnection>>",
        ))
    }

    fn is_connected(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.connected.load(Ordering::SeqCst)
    }

    fn set_reader(&mut self, callback: DataCallback) {
        self.data_callback = Some(callback);
    }

    fn set_disconnect_callback(&mut self, callback: DisconnectCallback) {
        self.disconnect_callback = Some(callback);
    }
}

#[cfg(feature = "serial")]
impl SerialConnection {
    fn start_read_task(&mut self) {
        // Read task implementation would go here
        // This requires more complex async handling with the serial port
    }
}

/// TCP connection implementation
#[cfg(feature = "tcp")]
pub struct TcpConnection {
    host: String,
    port: u16,
    stream: Option<Arc<Mutex<tokio::net::TcpStream>>>,
    connected: Arc<std::sync::atomic::AtomicBool>,
    read_task: Option<tokio::task::JoinHandle<()>>,
    data_callback: Arc<Mutex<Option<DataCallback>>>,
    disconnect_callback: Arc<Mutex<Option<DisconnectCallback>>>,
}

#[cfg(feature = "tcp")]
impl TcpConnection {
    /// Create a new TCP connection
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            stream: None,
            connected: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            read_task: None,
            data_callback: Arc::new(Mutex::new(None)),
            disconnect_callback: Arc::new(Mutex::new(None)),
        }
    }
}

#[cfg(feature = "tcp")]
#[async_trait]
impl Connection for TcpConnection {
    async fn connect(&mut self) -> Result<()> {
        use std::sync::atomic::Ordering;

        let addr = format!("{}:{}", self.host, self.port);
        let stream = tokio::net::TcpStream::connect(&addr)
            .await
            .map_err(|e| Error::connection(format!("Failed to connect to {}: {}", addr, e)))?;

        let stream = Arc::new(Mutex::new(stream));
        self.stream = Some(stream.clone());
        self.connected.store(true, Ordering::SeqCst);

        // Start read task
        let connected = self.connected.clone();
        let data_callback = self.data_callback.clone();
        let disconnect_callback = self.disconnect_callback.clone();

        self.read_task = Some(tokio::spawn(async move {
            let mut buffer = BytesMut::with_capacity(4096);
            let mut read_buf = [0u8; 1024];

            loop {
                if !connected.load(Ordering::SeqCst) {
                    break;
                }

                let n = {
                    let mut stream = stream.lock().await;
                    match stream.read(&mut read_buf).await {
                        Ok(0) => {
                            // Connection closed
                            connected.store(false, Ordering::SeqCst);
                            if let Some(cb) = disconnect_callback.lock().await.as_ref() {
                                cb();
                            }
                            break;
                        }
                        Ok(n) => n,
                        Err(_) => {
                            connected.store(false, Ordering::SeqCst);
                            if let Some(cb) = disconnect_callback.lock().await.as_ref() {
                                cb();
                            }
                            break;
                        }
                    }
                };

                buffer.extend_from_slice(&read_buf[..n]);

                // Parse frames from buffer
                while buffer.len() >= 3 {
                    if buffer[0] != FRAME_START {
                        // Skip invalid bytes
                        buffer.advance(1);
                        continue;
                    }

                    let len = u16::from_le_bytes([buffer[1], buffer[2]]) as usize;
                    if buffer.len() < 3 + len {
                        // Wait for more data
                        break;
                    }

                    // Extract frame
                    let frame = buffer[3..3 + len].to_vec();
                    buffer.advance(3 + len);

                    // Call data callback
                    if let Some(cb) = data_callback.lock().await.as_ref() {
                        cb(frame);
                    }
                }
            }
        }));

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        use std::sync::atomic::Ordering;

        self.connected.store(false, Ordering::SeqCst);

        if let Some(task) = self.read_task.take() {
            task.abort();
        }

        if let Some(stream) = self.stream.take() {
            let mut stream = stream.lock().await;
            let _ = stream.shutdown().await;
        }

        Ok(())
    }

    async fn send(&self, data: &[u8]) -> Result<()> {
        let stream = self
            .stream
            .as_ref()
            .ok_or_else(|| Error::NotConnected)?;

        let framed = frame_packet(data);
        let mut stream = stream.lock().await;
        stream
            .write_all(&framed)
            .await
            .map_err(|e| Error::connection(format!("Failed to send: {}", e)))?;

        Ok(())
    }

    fn is_connected(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.connected.load(Ordering::SeqCst)
    }

    fn set_reader(&mut self, callback: DataCallback) {
        // We need to set this synchronously, but the field is async
        // This is a design limitation - in production, use a different pattern
        let cb = self.data_callback.clone();
        tokio::spawn(async move {
            *cb.lock().await = Some(callback);
        });
    }

    fn set_disconnect_callback(&mut self, callback: DisconnectCallback) {
        let cb = self.disconnect_callback.clone();
        tokio::spawn(async move {
            *cb.lock().await = Some(callback);
        });
    }
}

/// Shared connection wrapper with interior mutability
pub struct SharedConnection<C: Connection> {
    inner: Arc<Mutex<C>>,
}

impl<C: Connection> SharedConnection<C> {
    /// Create a new shared connection
    pub fn new(conn: C) -> Self {
        Self {
            inner: Arc::new(Mutex::new(conn)),
        }
    }

    /// Connect
    pub async fn connect(&self) -> Result<()> {
        self.inner.lock().await.connect().await
    }

    /// Disconnect
    pub async fn disconnect(&self) -> Result<()> {
        self.inner.lock().await.disconnect().await
    }

    /// Send data
    pub async fn send(&self, data: &[u8]) -> Result<()> {
        self.inner.lock().await.send(data).await
    }

    /// Check if connected
    pub async fn is_connected(&self) -> bool {
        self.inner.lock().await.is_connected()
    }
}

impl<C: Connection> Clone for SharedConnection<C> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Connection manager with auto-reconnect capability
pub struct ConnectionManager<C: Connection> {
    connection: SharedConnection<C>,
    auto_reconnect: bool,
    max_retries: usize,
}

impl<C: Connection + 'static> ConnectionManager<C> {
    /// Create a new connection manager
    pub fn new(connection: C, auto_reconnect: bool, max_retries: usize) -> Self {
        Self {
            connection: SharedConnection::new(connection),
            auto_reconnect,
            max_retries,
        }
    }

    /// Get the underlying connection
    pub fn connection(&self) -> &SharedConnection<C> {
        &self.connection
    }

    /// Connect with optional auto-retry
    pub async fn connect(&self) -> Result<()> {
        let mut attempts = 0;

        loop {
            match self.connection.connect().await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    attempts += 1;
                    if !self.auto_reconnect || attempts >= self.max_retries {
                        return Err(e);
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    /// Disconnect
    pub async fn disconnect(&self) -> Result<()> {
        self.connection.disconnect().await
    }

    /// Send data
    pub async fn send(&self, data: &[u8]) -> Result<()> {
        self.connection.send(data).await
    }
}

use bytes::Buf;

#[cfg(test)]
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
}
