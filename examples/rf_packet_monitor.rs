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
//! Usage:
//!   cargo run --example rf_packet_monitor -- [serial-port]

use meshcore_rs::events::EventPayload;
use meshcore_rs::{EventType, MeshCore, PayloadType, RouteType};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let port = env::args()
        .nth(1)
        .unwrap_or_else(|| "/dev/ttyUSB0".to_string());

    println!("Connecting to MeshCore device on {port}...");
    let meshcore = MeshCore::serial(&port, 115200).await?;

    let self_info = meshcore.commands().lock().await.send_appstart().await?;
    println!("Connected to device: {}", self_info.name);

    let _sub = meshcore
        .subscribe(
            EventType::LogData,
            std::collections::HashMap::new(),
            |event| {
                if let EventPayload::LogData(log) = event.payload {
                    println!("RF packet: SNR {:.1} dB, RSSI {} dBm", log.snr, log.rssi);

                    match log.header {
                        Some(header) => {
                            println!("  Route: {:?}", header.route_type);
                            println!("  Payload type: {:?}", header.payload_type);
                            if !header.path.is_empty() {
                                // Each hop's hash is `path_hash_size` bytes
                                // (1-4, encoded in the path descriptor byte)
                                // — group accordingly rather than dumping a
                                // flat byte stream, which would be ambiguous
                                // whenever hash_size != 1.
                                let hops: Vec<String> = header
                                    .path
                                    .chunks(header.path_hash_size as usize)
                                    .map(|hop| {
                                        hop.iter().map(|b| format!("{b:02x}")).collect::<String>()
                                    })
                                    .collect();
                                println!(
                                    "  Path ({} hop(s), {}-byte hash): {}",
                                    header.path_len,
                                    header.path_hash_size,
                                    hops.join(" -> ")
                                );
                            }

                            if header.route_type == RouteType::TransportFlood
                                || header.route_type == RouteType::TransportDirect
                            {
                                if let Some(code) = header.transport_code {
                                    println!("  Transport code: {:02x?}", code);
                                }
                            }

                            if header.payload_type == PayloadType::Advert {
                                if let Some(adv) = log.advertisement {
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
                            } else {
                                println!("  Payload ({} bytes, opaque): {}", log.payload.len(), {
                                    log.payload
                                        .iter()
                                        .map(|b| format!("{b:02x}"))
                                        .collect::<String>()
                                });
                            }
                        }
                        None => println!("  (payload too short to decode a packet header)"),
                    }
                    println!();
                }
            },
        )
        .await;

    println!("\nListening for RF packets (press Ctrl+C to exit)...");
    tokio::signal::ctrl_c().await?;

    println!("\nDisconnecting...");
    meshcore.disconnect().await?;

    Ok(())
}
