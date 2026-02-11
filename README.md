# MeshCore-rs
[![codecov](https://codecov.io/gh/andrewdavidmackenzie/meshcore-rs/graph/badge.svg?token=cfyajKsYQa)](https://codecov.io/gh/andrewdavidmackenzie/meshcore-rs)

Rust library for communicating with [MeshCore](https://meshcore.co.uk) companion radio nodes.

This is a Rust port of the [meshcore_py](https://github.com/meshcore-dev/meshcore_py) Python library.

## Features

- **Async/await** - Built on Tokio for async I/O
- **Serial connection** - Connect via USB serial port
- **TCP connection** - Connect via TCP socket
- **BLE connection** - Connect via Bluetooth Low Energy (optional feature)
- **Event-driven** - Subscribe to events with filters
- **Full protocol support** - Contacts, messaging, binary protocol, signing, etc.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
meshcore = "0.1"
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
use meshcore::MeshCore;

#[tokio::main]
async fn main() -> Result<(), meshcore::Error> {
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
use meshcore::{MeshCore, EventType};
use std::collections::HashMap;

// Subscribe to incoming messages
let sub = meshcore.subscribe(
    EventType::ContactMsgRecv,
    HashMap::new(),
    |event| {
        if let meshcore::events::EventPayload::Message(msg) = event.payload {
            println!("Message from {:02x?}: {}", msg.sender_prefix, msg.text);
        }
    }
).await;

// Auto-fetch messages when device signals messages waiting
meshcore.start_auto_message_fetching().await;

// Later, unsubscribe
sub.unsubscribe().await;
```

## API Overview

### Device Commands

- `send_appstart()` - Initialize connection, get device info
- `get_bat()` - Get battery level
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

MIT License - see [LICENSE](LICENSE) for details.

## Related Projects

- [MeshCore](https://github.com/meshcore-dev/MeshCore) - Firmware for MeshCore devices
- [meshcore_py](https://github.com/meshcore-dev/meshcore_py) - Python library (original)
- [meshcore-cli](https://github.com/meshcore-dev/meshcore-cli) - Command-line interface
