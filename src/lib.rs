//! MeshCore - Rust library for communicating with MeshCore companion radio nodes
//!
//! This library provides an async interface for communicating with MeshCore devices
//! over serial, TCP, or BLE connections.
//!
//! # Example
//!
//! ```no_run
//! use meshcore::{MeshCore, EventType};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), meshcore::Error> {
//!     // Connect via serial
//!     let meshcore = MeshCore::serial("/dev/ttyUSB0", 115200).await?;
//!
//!     // Get device info
//!     let info = meshcore.commands().send_appstart().await?;
//!     println!("Connected to: {:?}", info);
//!
//!     // Get contacts
//!     let contacts = meshcore.commands().get_contacts(0).await?;
//!     println!("Found {} contacts", contacts.len());
//!
//!     meshcore.disconnect().await?;
//!     Ok(())
//! }
//! ```

pub mod commands;
pub mod connection;
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
