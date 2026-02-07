//! Example showing how to connect to a MeshCore device via Bluetooth Low Energy (BLE)
//!
//! This example demonstrates connecting to a MeshCore device over BLE instead of serial.
//! The device must be advertising the Nordic UART Service (NUS) which MeshCore uses.
//!
//! Usage:
//!   cargo run --example btle
//!   cargo run --example btle -- "DeviceName"

use meshcore_rs::{EventType, MeshCore};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with DEBUG level for meshcore
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("meshcore=debug".parse().unwrap()),
        )
        .init();

    // Get optional device name from command line
    let device_name = env::args().nth(1);

    match &device_name {
        Some(name) => println!("Scanning for MeshCore device '{}'...", name),
        None => println!("Scanning for any MeshCore device..."),
    }

    // Connect via BLE
    let meshcore = MeshCore::ble(device_name.as_deref()).await?;

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
