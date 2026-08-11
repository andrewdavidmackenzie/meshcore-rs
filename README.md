# MeshCore-rs

[![codecov](https://codecov.io/gh/andrewdavidmackenzie/meshcore-rs/graph/badge.svg?token=cfyajKsYQa)](https://codecov.io/gh/andrewdavidmackenzie/meshcore-rs)

Rust library for communicating with [MeshCore](https://meshcore.co.uk) companion radio nodes.

This is a Rust port of the [meshcore_py](https://github.com/meshcore-dev/meshcore_py) Python library.

## Features

- **Async/await** - Built on Tokio for async I/O
- **Serial connection** – Connect via USB serial port
- **TCP connection** – Connect via TCP socket
- **BLE connection** – Connect via Bluetooth Low Energy (optional feature)
- **Event-driven** - Subscribe to events with filters
- **Full protocol support** – Contacts, messaging, binary protocol, signing, etc.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
meshcore-rs = "0.1"
tokio = "1"
```

### Optional Features

```toml
[dependencies]
meshcore = { version = "0.1", features = ["ble"] }
```

- `serial` - Serial port support (enabled by default)
- `tcp` - TCP socket support (enabled by default)
- `ble` - Bluetooth Low Energy support (requires btleplug)

## Quick Start

```rust
use meshcore_rs::MeshCore;

#[tokio::main]
async fn main() -> Result<(), meshcore_rs::Error> {
    // Connect via serial port
    let meshcore = MeshCore::serial("/dev/ttyUSB0", 115200).await?;

    // Initialize connection and get device info
    let info = meshcore.commands().lock().await.send_appstart().await?;
    println!("Connected to: {}", info.name);

    // Get contacts
    let contacts = meshcore.commands().lock().await.get_contacts(0).await?;
    println!("Found {} contacts", contacts.len());

    // Send a message
    if let Some(contact) = contacts.first() {
        meshcore.commands().lock().await
            .send_msg(contact, "Hello from Rust!", None)
            .await?;
    }

    meshcore.disconnect().await?;
    Ok(())
}
```

## Event Subscriptions

```rust
use meshcore_rs::{MeshCore, EventType};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), meshcore_rs::Error> {
    // Connect via serial port
    let meshcore = MeshCore::serial("/dev/ttyUSB0", 115200).await?;

    // Initialize connection and get device info
    let info = meshcore.commands().lock().await.send_appstart().await?;
    println!("Connected to: {}", info.name);

    // Subscribe to incoming messages
    let sub = meshcore.subscribe(
        EventType::ContactMsgRecv,
        HashMap::new(),
        |event| {
            if let meshcore_rs::events::EventPayload::ContactMessage(msg) = event.payload {
                println!("Message from {:02x?}: {}", msg.sender_prefix, msg.text);
            }
        }
    ).await;

    // Auto-fetch messages when device signals messages waiting
    meshcore.start_auto_message_fetching().await;

    // Keep main alive
    tokio::signal::ctrl_c().await?;

    // Later, unsubscribe
    sub.unsubscribe().await;

    meshcore.disconnect().await?;

    Ok(())
}
```

## RF Packet Monitoring

The node pushes a `LogData` event automatically for **every** packet its
radio receives, whether or not it was addressed to it — no configuration
required. This is useful for building network visibility tools (coverage
maps, traffic analysis, etc.). The payload carries the signal quality, the
decoded mesh packet header (route type, payload type, hop path) and, for
advertisement packets, the advertiser's identity:

```rust
use meshcore_rs::{MeshCore, EventType};
use meshcore_rs::events::EventPayload;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), meshcore_rs::Error> {
    let meshcore = MeshCore::serial("/dev/ttyUSB0", 115200).await?;
    meshcore.commands().lock().await.send_appstart().await?;

    let _sub = meshcore.subscribe(
        EventType::LogData,
        HashMap::new(),
        |event| {
            if let EventPayload::LogData(log) = event.payload {
                println!("SNR {:.1} dB, RSSI {} dBm", log.snr, log.rssi);
                if let Some(header) = log.header {
                    println!("{:?} / {:?}, {} hop(s)", header.route_type, header.payload_type, header.path_len);
                }
            }
        }
    ).await;

    tokio::signal::ctrl_c().await?;
    meshcore.disconnect().await?;
    Ok(())
}
```

See `examples/rf_packet_monitor.rs` for a complete, runnable version.

Note: `EventType::RawData` is a different, much narrower event — it only
fires for directly-routed, not-yet-seen `RAW_CUSTOM` payloads sent by
another application via the companion `SEND_RAW_DATA` command. Regular mesh
traffic never triggers it; use `LogData` for general monitoring as above.

## API Overview

### Device Commands

- `send_appstart()` - Initialize connection, get device info
- `get_bat()` - Get battery voltage (millivolts) and storage info
- `get_time()` / `set_time()` - Get/set device time
- `set_name()` - Set device name
- `set_coords()` - Set device coordinates
- `set_tx_power()` - Set transmission power
- `send_advert()` - Send advertisement
- `get_channel()` / `set_channel()` - Get/set channel config
- `export_private_key()` / `import_private_key()` - Key management

### Contact Commands

- `get_contacts()` - Get contact list
- `add_contact()` - Add a contact
- `remove_contact()` - Remove a contact
- `export_contact()` - Export contact as URI
- `import_contact()` - Import contact from card data

### Messaging Commands

- `get_msg()` - Get next message from queue
- `send_msg()` - Send a direct message
- `send_chan_msg()` - Send a channel message
- `send_login()` / `send_logout()` - Login/logout to remote node

### Binary Protocol Commands

- `req_status()` - Request device status
- `req_telemetry()` - Request telemetry data
- `req_acl()` - Request ACL entries
- `req_neighbours()` - Request neighbour list

### Signing Commands

- `sign_start()` / `sign_data()` / `sign_finish()` - Low-level signing
- `sign()` - High-level sign helper

## Protocol Details

The library implements the MeshCore serial/TCP protocol:

- Frame format: `[0x3c][len_low][len_high][payload]`
- Little-endian byte ordering
- Coordinates stored as microdegrees (divide by 1,000,000 for decimal degrees)

## License

MIT License

## Related Projects

- [MeshCore](https://github.com/meshcore-dev/MeshCore) – Firmware for MeshCore devices
- [meshcore_py](https://github.com/meshcore-dev/meshcore_py) - Python library (original)
- [meshcore-cli](https://github.com/meshcore-dev/meshcore-cli) - Command-line interface
