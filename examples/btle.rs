//! Example showing how to connect to a MeshCore device via Bluetooth Low Energy (BLE)
//!
//! This example demonstrates connecting to a MeshCore device over BLE instead of serial.
//! It will connect to the first MeshCore radio found
//!
//! Usage:
//!   cargo run --example btle

use meshcore_rs::{EventType, MeshCore};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with DEBUG level for mod.rs
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("mod.rs-rs=debug".parse().unwrap()),
        )
        .init();

    // Connect via BLE
    let radios = MeshCore::ble_discover(Duration::from_secs(4)).await?;
    let meshcore = MeshCore::ble_connect(radios.get(0).unwrap()).await?;

    println!("Connected via BLE!");

    // Send APPSTART to initialize connection and get device info
    let self_info = meshcore.commands().lock().await.send_appstart().await?;
    println!("Connected to device: {}", self_info.name);
    println!("  Public key: {:02x?}", &self_info.public_key[..6]);
    println!("  TX power: {}", self_info.tx_power);
    println!(
        "  Location: {:.6}, {:.6}",
        self_info.adv_lat as f64 / 1_000_000.0,
        self_info.adv_lon as f64 / 1_000_000.0
    );

    // Get battery info
    let battery = meshcore.commands().lock().await.get_bat().await?;
    println!("  Battery: {}%", battery.level);

    // Get contacts (use longer timeout for BLE - contacts can take a while)
    println!("\nFetching contacts...");
    let contacts = meshcore
        .commands()
        .lock()
        .await
        .get_contacts_with_timeout(0, std::time::Duration::from_secs(30))
        .await?;
    println!("Found {} contacts:", contacts.len());

    for contact in &contacts {
        println!(
            "  - {} (prefix: {})",
            contact.adv_name,
            contact.prefix_hex()
        );
    }

    // Subscribe to incoming messages
    println!("\nListening for messages (press Ctrl+C to exit)...");

    let _sub = meshcore
        .subscribe(
            EventType::ContactMsgRecv,
            std::collections::HashMap::new(),
            |event| {
                if let meshcore_rs::events::EventPayload::Message(msg) = event.payload {
                    println!(
                        "Received message from {:02x?}: {}",
                        msg.sender_prefix, msg.text
                    );
                }
            },
        )
        .await;

    // Start auto-fetching messages
    meshcore.start_auto_message_fetching().await;

    // Keep running until Ctrl+C
    tokio::signal::ctrl_c().await?;

    println!("\nDisconnecting...");
    meshcore.disconnect().await?;

    Ok(())
}
