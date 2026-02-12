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
//!
//! #[tokio::main]
//! async fn main() -> Result<(), meshcore_rs::Error> {
//!     // Connect via BLE (scans for any MeshCore device)
//!     let meshcore = MeshCore::ble(None).await?;
//!
//!     // Or connect to a specific device by name
//!     // let mod.rs = MeshCore::ble(Some("MyDevice")).await?;
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

pub use error::Error;
pub use events::{Event, EventDispatcher, EventType, Subscription};
pub use meshcore::MeshCore;
pub use packets::{AnonReqType, BinaryReqType, ControlType, PacketType};

/// Result type alias using the library's Error type
pub type Result<T> = std::result::Result<T, Error>;
