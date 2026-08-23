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

See `examples/rf_packet_monitor.rs` for a complete, runnable version:

```sh
cargo run --example rf_packet_monitor --features serial -- --serial /dev/ttyUSB0
cargo run --example rf_packet_monitor --features ble -- --ble MeshCore-XXXX
cargo run --example rf_packet_monitor --features tcp -- --tcp 192.168.1.50:5000
```

Exactly one of `--serial`, `--tcp` or `--ble` is required.

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
- `get_autoadd_config()` / `set_autoadd_config()` - Get/set auto-add-contacts configuration (contact-type bitmask + max hops)
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

## Not Yet Supported Commands

The companion firmware (`meshcore-dev/MeshCore`, `examples/companion_radio/MyMesh.cpp`) exposes
more `CMD_*` commands than this crate currently wraps. The table below lists every command with
no equivalent here yet — including a few whose `CMD_*` byte constant is declared in
`src/commands/base.rs` but marked `#[allow(dead_code)]`, i.e. reserved but never actually sent —
with the minimum firmware version that introduced it, and whether the reference Python client
(`meshcore-dev/meshcore_py`) already supports it (checked directly against its source, not
assumed).

| Code | Name | Description | Minimum firmware version | In `meshcore_py`? |
|---|---|---|---|---|
| 11 (0x0B) | `SET_RADIO_PARAMS` | Set radio freq/bandwidth/spreading factor/coding rate | companion-v1.0.0a | ✅ `set_radio()` |
| 13 (0x0D) | `RESET_PATH` | Reset a contact's known route back to flood routing | companion-v1.0.0a | ✅ `reset_path()` |
| 16 (0x10) | `SHARE_CONTACT` | Re-share a known contact's advert with the mesh | companion-v1.0.0a | ✅ `share_contact()` |
| 21 (0x15) | `SET_TUNING_PARAMS` | Set radio tuning params (rx delay base, airtime factor) | companion-v1.0.0a | ✅ `set_tuning()` |
| 25 (0x19) | `SEND_RAW_DATA` | Send an opaque `RAW_CUSTOM` payload to a peer | companion-v1.0.0a | ✅ `send_raw_data()` |
| 27 (0x1B) | `SEND_STATUS_REQ` | Request a contact's status | companion-v1.0.0a | ✅ `send_statusreq()` |
| 28 (0x1C) | `HAS_CONNECTION` | Whether the node has an active BLE/serial companion connection | companion-v1.0.0a | ✅ `has_connection()` |
| 30 (0x1E) | `GET_CONTACT_BY_KEY` | Look up a contact by its full public key | companion-v1.2.0 | ✅ `get_contact_by_key()` |
| 36 (0x24) | `SEND_TRACE_PATH` | Trace/test the route to a node | companion-v1.4.0 | ✅ `send_trace()` |
| 37 (0x25) | `SET_DEVICE_PIN` | Set a device PIN (BLE pairing) | companion-v1.4.0 | ✅ `set_devicepin()` |
| 38 (0x26) | `SET_OTHER_PARAMS` | Legacy `manual_add_contacts` flag, telemetry mode, advert location policy, multi-acks | companion-v1.5.0 | ✅ `set_other_params()`/`set_other_params_from_infos()` |
| 39 (0x27) | `SEND_TELEMETRY_REQ` | Request telemetry from a contact — **deprecated in firmware** in favor of `SEND_BINARY_REQ` (already supported here, see `req_telemetry()`) | companion-v1.6.0 | ✅ `send_telemetry_req()` |
| 42 (0x2A) | `GET_ADVERT_PATH` | Last known route recorded for a contact's public key | companion-v1.7.1 | ✅ `get_advert_path()` |
| 43 (0x2B) | `GET_TUNING_PARAMS` | Read radio tuning params (rx delay base, airtime factor) — note `SET_TUNING_PARAMS` (21, above) isn't actually wired up here either, despite its `CMD_*` constant existing | companion-v1.7.3 | ✅ `get_tuning()` |
| 56 (0x38) | `GET_STATS` | Core/radio/packet statistics (sub-type in byte 2) | companion-v1.11.0 | ✅ `get_stats_core()`/`get_stats_radio()`/`get_stats_packets()` |
| 57 (0x39) | `SEND_ANON_REQ` | Request to a peer that isn't (yet) a known contact | companion-v1.12.0 | ✅ `send_anon_req()` |
| 60 (0x3C) | `GET_ALLOWED_REPEAT_FREQ` | Frequency ranges a repeater is allowed to retransmit on | companion-v1.13.0 | ✅ `get_allowed_repeat_freq()` |
| 61 (0x3D) | `SET_PATH_HASH_MODE` | Path-hash size mode used when building routes | companion-v1.14.0 | ✅ `set_path_hash_mode()`/`get_path_hash_mode()` |
| 62 (0x3E) | `SEND_CHANNEL_DATA` | Send a raw datagram on a group/channel | companion-v1.15.0 | ❌ not found in `meshcore_py` either |
| 63 (0x3F) | `SET_DEFAULT_FLOOD_SCOPE` | Default flood-scope (region) applied when none is given | companion-v1.15.0 | ✅ `set_default_flood_scope()` |
| 64 (0x40) | `GET_DEFAULT_FLOOD_SCOPE` | Read the currently configured default flood-scope | companion-v1.15.0 | ✅ `get_default_flood_scope()` |
| 65 (0x41) | `SEND_RAW_PACKET` | Inject a raw, fully-formed mesh packet directly onto the radio | companion-v1.16.0 | ❌ not found in `meshcore_py` either |

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
