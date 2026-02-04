//! Basic example showing how to connect to a MeshCore device and send messages

use meshcore::{EventType, MeshCore};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Get serial port from command line or use default
    let port = env::args().nth(1).unwrap_or_else(|| "/dev/ttyUSB0".to_string());

    println!("Connecting to MeshCore device on {}...", port);

    // Connect via serial
    let meshcore = MeshCore::serial(&port, 115200).await?;

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

    // Get contacts
    println!("\nFetching contacts...");
    let contacts = meshcore.commands().lock().await.get_contacts(0).await?;
    println!("Found {} contacts:", contacts.len());

    for contact in &contacts {
        println!(
            "  - {} (prefix: {})",
            contact.adv_name,
            contact.prefix_hex()
        );
    }

    // Send a message to the first contact (if any)
    if let Some(contact) = contacts.first() {
        println!("\nSending test message to {}...", contact.adv_name);
        let result = meshcore
            .commands()
            .lock()
            .await
            .send_msg(contact, "Hello from Rust!", None)
            .await?;
        println!(
            "Message sent! Expected ACK: {:02x?}",
            result.expected_ack
        );
    }

    // Subscribe to incoming messages
    println!("\nListening for messages (press Ctrl+C to exit)...");

    let _sub = meshcore
        .subscribe(EventType::ContactMsgRecv, std::collections::HashMap::new(), |event| {
            if let meshcore::events::EventPayload::Message(msg) = event.payload {
                println!(
                    "Received message from {:02x?}: {}",
                    msg.sender_prefix, msg.text
                );
            }
        })
        .await;

    // Start auto-fetching messages
    meshcore.start_auto_message_fetching().await;

    // Keep running until Ctrl+C
    tokio::signal::ctrl_c().await?;

    println!("\nDisconnecting...");
    meshcore.disconnect().await?;

    Ok(())
}
