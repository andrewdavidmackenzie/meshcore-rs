//! Error types for the MeshCore library

use thiserror::Error;

/// The main error type for MeshCore operations
#[derive(Error, Debug)]
pub enum Error {
    /// Connection-related errors
    #[error("Connection error: {0}")]
    Connection(String),

    /// Serial port errors
    #[error("Serial error: {0}")]
    Serial(#[from] tokio_serial::Error),

    /// I/O errors
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Protocol errors (malformed packets, unexpected responses)
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// Timeout waiting for response
    #[error("Timeout waiting for {0}")]
    Timeout(String),

    /// Device returned an error
    #[error("Device error: {0}")]
    Device(String),

    /// Invalid parameter provided
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    /// Feature is disabled on device
    #[error("Feature disabled: {0}")]
    Disabled(String),

    /// Not connected to a device
    #[error("Not connected")]
    NotConnected,

    /// BLE-specific errors
    #[cfg(feature = "ble")]
    #[error("BLE error: {0}")]
    Ble(#[from] btleplug::Error),

    /// Channel send error
    #[error("Channel error: {0}")]
    Channel(String),
}

impl Error {
    /// Create a connection error
    pub fn connection(msg: impl Into<String>) -> Self {
        Error::Connection(msg.into())
    }

    /// Create a protocol error
    pub fn protocol(msg: impl Into<String>) -> Self {
        Error::Protocol(msg.into())
    }

    /// Create a timeout error
    pub fn timeout(msg: impl Into<String>) -> Self {
        Error::Timeout(msg.into())
    }

    /// Create a device error
    pub fn device(msg: impl Into<String>) -> Self {
        Error::Device(msg.into())
    }

    /// Create an invalid parameter error
    pub fn invalid_param(msg: impl Into<String>) -> Self {
        Error::InvalidParameter(msg.into())
    }
}
