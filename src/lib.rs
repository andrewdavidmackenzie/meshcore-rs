//! MeshCore - Rust library for communicating with MeshCore companion radio nodes
//!
//! This library provides an async interface for communicating with MeshCore devices
//! over serial, TCP, or BLE connections.
//!
//! # Serial Example
//!
//! ```no_run
//! use meshcore_rs::MeshCore;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), meshcore_rs::Error> {
//!     // Connect via serial
//!     let meshcore = MeshCore::serial("/dev/ttyUSB0", 115200).await?;
//!
//!     // Get device info
//!     let info = meshcore.commands().lock().await.send_appstart().await?;
//!     println!("Connected to: {}", info.name);
//!
//!     // Get contacts
//!     let contacts = meshcore.commands().lock().await.get_contacts(0).await?;
//!     println!("Found {} contacts", contacts.len());
//!
//!     meshcore.disconnect().await?;
//!     Ok(())
//! }
//! ```
//!
//! # BLE Example
//!
//! Requires the `ble` feature.
//!
//! ```no_run
//! use meshcore_rs::MeshCore;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), meshcore_rs::Error> {
//!     // First discover available MeshCore devices
//!     let devices = MeshCore::ble_discover(Duration::from_secs(5)).await?;
//!     println!("Found devices: {:?}", devices);
//!
//!     // Connect to a specific device by name
//!     let meshcore = MeshCore::ble_connect("MyDevice").await?;
//!
//!     // Get device info
//!     let info = meshcore.commands().lock().await.send_appstart().await?;
//!     println!("Connected to: {}", info.name);
//!
//!     // Get contacts
//!     let contacts = meshcore.commands().lock().await.get_contacts(0).await?;
//!     println!("Found {} contacts", contacts.len());
//!
//!     meshcore.disconnect().await?;
//!     Ok(())
//! }
//! ```

pub mod commands;
pub mod error;
pub mod events;
pub mod packets;
pub mod parsing;
pub mod reader;

mod meshcore;

// Protocol constants
/// Length of the channel name field in bytes
pub const CHANNEL_NAME_LEN: usize = 32;
/// Length of channel secret field in bytes
pub const CHANNEL_SECRET_LEN: usize = 16;
/// Total length of channel info payload (idx + name + secret)
pub const CHANNEL_INFO_LEN: usize = 1 + CHANNEL_NAME_LEN + CHANNEL_SECRET_LEN;

// Auto-add-contacts config bits (`CommandHandler::get_autoadd_config`/
// `set_autoadd_config`'s `config` byte). Matches firmware's
// `examples/companion_radio/MyMesh.cpp` `AUTO_ADD_*` defines exactly,
// including their naming, so the two stay easy to cross-reference.
/// When set, auto-adding a new contact while the table is full overwrites
/// the oldest non-favourite entry instead of being rejected.
pub const AUTO_ADD_OVERWRITE_OLDEST: u8 = 1 << 0;
/// Auto-add Chat (Companion) contacts (`ADV_TYPE_CHAT`).
pub const AUTO_ADD_CHAT: u8 = 1 << 1;
/// Auto-add Repeater contacts (`ADV_TYPE_REPEATER`).
pub const AUTO_ADD_REPEATER: u8 = 1 << 2;
/// Auto-add Room Server contacts (`ADV_TYPE_ROOM`).
pub const AUTO_ADD_ROOM_SERVER: u8 = 1 << 3;
/// Auto-add Sensor contacts (`ADV_TYPE_SENSOR`).
pub const AUTO_ADD_SENSOR: u8 = 1 << 4;

pub use error::Error;
pub use events::{
    ChannelMessage, ContactMessage, EventDispatcher, EventPayload, EventType, MeshCoreEvent,
    MsgSentInfo, Subscription,
};
pub use meshcore::MeshCore;
pub use packets::{AnonReqType, BinaryReqType, ControlType, PacketType, PayloadType, RouteType};

/// Result type alias using the library's Error type
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_name_len() {
        assert_eq!(CHANNEL_NAME_LEN, 32);
    }

    #[test]
    fn test_channel_secret_len() {
        assert_eq!(CHANNEL_SECRET_LEN, 16);
    }

    #[test]
    fn test_channel_info_len() {
        // 1 byte idx + 32 bytes name + 16 bytes secret = 49
        assert_eq!(CHANNEL_INFO_LEN, 49);
        assert_eq!(CHANNEL_INFO_LEN, 1 + CHANNEL_NAME_LEN + CHANNEL_SECRET_LEN);
    }

    #[test]
    fn test_auto_add_bits_match_firmware_values() {
        assert_eq!(AUTO_ADD_OVERWRITE_OLDEST, 0x01);
        assert_eq!(AUTO_ADD_CHAT, 0x02);
        assert_eq!(AUTO_ADD_REPEATER, 0x04);
        assert_eq!(AUTO_ADD_ROOM_SERVER, 0x08);
        assert_eq!(AUTO_ADD_SENSOR, 0x10);
    }

    #[test]
    fn test_auto_add_bits_are_disjoint() {
        let bits = [
            AUTO_ADD_OVERWRITE_OLDEST,
            AUTO_ADD_CHAT,
            AUTO_ADD_REPEATER,
            AUTO_ADD_ROOM_SERVER,
            AUTO_ADD_SENSOR,
        ];
        assert_eq!(
            bits.iter().fold(0u8, |acc, &b| acc | b).count_ones() as usize,
            bits.len()
        );
    }
}
