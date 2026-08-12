//! Example showing how to monitor all RF packets received by the node
//!
//! This subscribes to `EventType::LogData`, which the node pushes
//! automatically for *every* packet its radio receives — regardless of
//! whether the packet was addressed to it, or of its payload type. It
//! prints the signal quality (SNR/RSSI), the decoded mesh packet header
//! (route type, payload type, path), and the advertiser's identity for any
//! ADVERT packets it overhears.
//!
//! `EventType::RawData` is a different, much narrower event: it only fires
//! for directly-routed, not-yet-seen `RAW_CUSTOM` payloads (sent by another
//! application via the companion `SEND_RAW_DATA` command) — regular mesh
//! traffic never triggers it. Use `LogData`, as this example does, to
//! observe general network activity.
//!
//! Usage: exactly one of the following is required
//!   cargo run --example rf_packet_monitor --features serial -- --serial <port>
//!   cargo run --example rf_packet_monitor --features tcp -- --tcp <host:port>
//!   cargo run --example rf_packet_monitor --features ble -- --ble <device-name>

#[path = "common/mod.rs"]
mod common;

use common::{connect, parse_args, ConnectionArgs};
use meshcore_rs::events::{EventPayload, MeshPacketHeader, RawAdvertisement};
use meshcore_rs::{EventType, MeshCoreEvent, PayloadType, RouteType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    monitor(args).await
}

/// Connects per `args` and prints every RF packet received until Ctrl+C.
async fn monitor(args: ConnectionArgs) -> Result<(), Box<dyn std::error::Error>> {
    let meshcore = connect(&args).await?;

    let self_info = meshcore.commands().lock().await.send_appstart().await?;
    println!("Connected to device: {}", self_info.name);

    let _sub = meshcore
        .subscribe(
            EventType::LogData,
            std::collections::HashMap::new(),
            print_log_data,
        )
        .await;

    println!("\nListening for RF packets (press Ctrl+C to exit)...");
    tokio::signal::ctrl_c().await?;

    println!("\nDisconnecting...");
    meshcore.disconnect().await?;

    Ok(())
}

fn print_log_data(event: MeshCoreEvent) {
    let EventPayload::LogData(log) = event.payload else {
        return;
    };
    println!("RF packet: SNR {:.1} dB, RSSI {} dBm", log.snr, log.rssi);

    let Some(header) = log.header else {
        println!("  (payload too short to decode a packet header)\n");
        return;
    };

    println!("  Route: {:?}", header.route_type);
    println!("  Payload type: {:?}", header.payload_type);
    print_path(&header);
    print_transport_code(&header);

    if header.payload_type == PayloadType::Advert {
        print_advertisement(log.advertisement.as_ref());
    } else {
        print_opaque_payload(&log.payload);
    }
    println!();
}

fn print_path(header: &MeshPacketHeader) {
    if header.path.is_empty() {
        return;
    }
    // Each hop's hash is `path_hash_size` bytes (1-4, encoded in the path
    // descriptor byte) — group accordingly rather than dumping a flat byte
    // stream, which would be ambiguous whenever hash_size != 1.
    let hops: Vec<String> = header
        .path
        .chunks(header.path_hash_size as usize)
        .map(|hop| hop.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .collect();
    println!(
        "  Path ({} hop(s), {}-byte hash): {}",
        header.path_len,
        header.path_hash_size,
        hops.join(" -> ")
    );
}

fn print_transport_code(header: &MeshPacketHeader) {
    if !matches!(
        header.route_type,
        RouteType::TransportFlood | RouteType::TransportDirect
    ) {
        return;
    }
    let Some(code) = header.transport_code else {
        return;
    };
    println!("  Transport code: {code:02x?}");
}

fn print_advertisement(adv: Option<&RawAdvertisement>) {
    let Some(adv) = adv else {
        return;
    };
    println!("  Advertiser: {:02x?}", &adv.public_key[..6]);
    if let Some(name) = &adv.name {
        println!("  Name: {name}");
    }
    if let (Some(lat), Some(lon)) = (adv.lat, adv.lon) {
        println!(
            "  Location: {:.6}, {:.6}",
            lat as f64 / 1_000_000.0,
            lon as f64 / 1_000_000.0
        );
    }
}

fn print_opaque_payload(payload: &[u8]) {
    let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
    println!("  Payload ({} bytes, opaque): {hex}", payload.len());
}
