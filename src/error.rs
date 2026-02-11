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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_error() {
        let err = Error::connection("test connection error");
        assert!(matches!(err, Error::Connection(_)));
        assert_eq!(err.to_string(), "Connection error: test connection error");
    }

    #[test]
    fn test_protocol_error() {
        let err = Error::protocol("invalid packet");
        assert!(matches!(err, Error::Protocol(_)));
        assert_eq!(err.to_string(), "Protocol error: invalid packet");
    }

    #[test]
    fn test_timeout_error() {
        let err = Error::timeout("response");
        assert!(matches!(err, Error::Timeout(_)));
        assert_eq!(err.to_string(), "Timeout waiting for response");
    }

    #[test]
    fn test_device_error() {
        let err = Error::device("busy");
        assert!(matches!(err, Error::Device(_)));
        assert_eq!(err.to_string(), "Device error: busy");
    }

    #[test]
    fn test_invalid_param_error() {
        let err = Error::invalid_param("bad value");
        assert!(matches!(err, Error::InvalidParameter(_)));
        assert_eq!(err.to_string(), "Invalid parameter: bad value");
    }

    #[test]
    fn test_not_connected_error() {
        let err = Error::NotConnected;
        assert_eq!(err.to_string(), "Not connected");
    }

    #[test]
    fn test_disabled_error() {
        let err = Error::Disabled("feature X".to_string());
        assert_eq!(err.to_string(), "Feature disabled: feature X");
    }

    #[test]
    fn test_channel_error() {
        let err = Error::Channel("send failed".to_string());
        assert_eq!(err.to_string(), "Channel error: send failed");
    }

    #[test]
    fn test_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
        assert!(err.to_string().contains("I/O error"));
    }

    #[test]
    fn test_error_debug() {
        let err = Error::connection("test");
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Connection"));
    }
}
