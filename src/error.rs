//! Error types for the MeshCore library

use thiserror::Error;

/// The main error type for MeshCore operations
#[derive(Error, Debug)]
pub enum Error {
    /// Connection-related errors
    #[error("Connection error: {0}")]
    Connection(String),

    /// Serial port errors
    #[cfg(feature = "serial")]
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
    Device(#[from] DeviceError),

    /// Invalid parameter provided
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    /// Feature is disabled on a device
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

/// Error codes the companion firmware sends in an error frame
/// (`ERR_CODE_*` in MeshCore's `examples/companion_radio/MyMesh.cpp`)
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeviceError {
    /// Unknown or unsupported command (`ERR_CODE_UNSUPPORTED_CMD`)
    #[error("unsupported command (code 1)")]
    UnsupportedCommand,

    /// Target not found, e.g. a contact or channel (`ERR_CODE_NOT_FOUND`)
    #[error("not found (code 2)")]
    NotFound,

    /// Internal table or queue is full (`ERR_CODE_TABLE_FULL`)
    #[error("table full (code 3)")]
    TableFull,

    /// Operation not valid in the device's current state (`ERR_CODE_BAD_STATE`)
    #[error("bad state (code 4)")]
    BadState,

    /// Filesystem or storage failure (`ERR_CODE_FILE_IO_ERROR`)
    #[error("file I/O error (code 5)")]
    FileIo,

    /// Invalid argument (`ERR_CODE_ILLEGAL_ARG`)
    #[error("illegal argument (code 6)")]
    IllegalArgument,

    /// A code this library doesn't know
    #[error("unknown (code {0})")]
    Unknown(u8),

    /// The error frame carried no code (companion firmware before 2025-03)
    #[error("unspecified")]
    Unspecified,
}

impl From<u8> for DeviceError {
    fn from(code: u8) -> Self {
        match code {
            1 => DeviceError::UnsupportedCommand,
            2 => DeviceError::NotFound,
            3 => DeviceError::TableFull,
            4 => DeviceError::BadState,
            5 => DeviceError::FileIo,
            6 => DeviceError::IllegalArgument,
            code => DeviceError::Unknown(code),
        }
    }
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

    /// Create an invalid parameter error
    pub fn invalid_param(msg: impl Into<String>) -> Self {
        Error::InvalidParameter(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;

    #[test]
    fn test_connection_error() {
        let err = Error::connection("test connection error");
        assert_matches!(err, Error::Connection(_));
        assert_eq!(err.to_string(), "Connection error: test connection error");
    }

    #[test]
    fn test_protocol_error() {
        let err = Error::protocol("invalid packet");
        assert_matches!(err, Error::Protocol(_));
        assert_eq!(err.to_string(), "Protocol error: invalid packet");
    }

    #[test]
    fn test_timeout_error() {
        let err = Error::timeout("response");
        assert_matches!(err, Error::Timeout(_));
        assert_eq!(err.to_string(), "Timeout waiting for response");
    }

    #[test]
    fn test_device_error() {
        let err: Error = DeviceError::TableFull.into();
        assert_matches!(err, Error::Device(DeviceError::TableFull));
        assert_eq!(err.to_string(), "Device error: table full (code 3)");
    }

    #[test]
    fn test_device_error_from_code() {
        let cases = [
            (
                1,
                DeviceError::UnsupportedCommand,
                "unsupported command (code 1)",
            ),
            (2, DeviceError::NotFound, "not found (code 2)"),
            (3, DeviceError::TableFull, "table full (code 3)"),
            (4, DeviceError::BadState, "bad state (code 4)"),
            (5, DeviceError::FileIo, "file I/O error (code 5)"),
            (6, DeviceError::IllegalArgument, "illegal argument (code 6)"),
            (42, DeviceError::Unknown(42), "unknown (code 42)"),
        ];
        for (code, expected, message) in cases {
            let err = DeviceError::from(code);
            assert_eq!(err, expected);
            assert_eq!(err.to_string(), message);
        }
    }

    #[test]
    fn test_device_error_unspecified() {
        assert_eq!(DeviceError::Unspecified.to_string(), "unspecified");
    }

    #[test]
    fn test_invalid_param_error() {
        let err = Error::invalid_param("bad value");
        assert_matches!(err, Error::InvalidParameter(_));
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
        assert_matches!(err, Error::Io(_));
        assert!(err.to_string().contains("I/O error"));
    }

    #[test]
    fn test_error_debug() {
        let err = Error::connection("test");
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("Connection"));
    }
}
