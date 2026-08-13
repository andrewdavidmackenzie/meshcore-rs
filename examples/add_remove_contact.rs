//! Example: add a contact, verify it's there, then remove it again.
//!
//! Exercises `CommandHandler::add_contact` and `CommandHandler::remove_contact`
//! end-to-end against a real node — a regression check for `remove_contact`'s
//! wire format: it must send the contact's full 32-byte public key, not just
//! a 6-byte prefix, or the node never responds (see CHANGELOG/git history).
//!
//! The contact added is obviously synthetic (public key bytes 0x00..0x1F,
//! name "meshcore-rs-example-test") so it's never mistaken for a real
//! contact, and easy to spot/remove by hand should the example be
//! interrupted before it cleans up after itself.
//!
//! Usage: exactly one of the following is required
//!   cargo run --example add_remove_contact --features serial -- --serial <port>
//!   cargo run --example add_remove_contact --features tcp -- --tcp <host:port>
//!   cargo run --example add_remove_contact --features ble -- --ble <device-name>

#[path = "common/mod.rs"]
mod common;

use common::{connect, parse_args, ConnectionArgs};
use meshcore_rs::events::Contact;

/// Public key for the synthetic test contact: sequential bytes 0x00..0x1F,
/// so it's unmistakably not a real device's key.
const TEST_PUBLIC_KEY: [u8; 32] = {
    let mut key = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        key[i] = i as u8;
        i += 1;
    }
    key
};
const TEST_CONTACT_NAME: &str = "meshcore-rs-example-test";

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

    run(args).await
}

async fn run(args: ConnectionArgs) -> Result<(), Box<dyn std::error::Error>> {
    let meshcore = connect(&args).await?;

    let self_info = meshcore.commands().lock().await.send_appstart().await?;
    println!("Connected to device: {}", self_info.name);

    let contact = Contact {
        public_key: TEST_PUBLIC_KEY,
        contact_type: 1, // CLI/Chat, per the firmware's CONTACT_TYPENAMES
        flags: 0,
        path_len: -1, // unknown route: flood
        out_path: Vec::new(),
        adv_name: TEST_CONTACT_NAME.to_string(),
        last_advert: 0,
        adv_lat: 0,
        adv_lon: 0,
        last_modification_timestamp: 0,
    };

    println!("\nAdding test contact {TEST_CONTACT_NAME:?}...");
    meshcore
        .commands()
        .lock()
        .await
        .add_contact(&contact)
        .await?;
    println!("  -> add_contact returned Ok.");

    let after_add = meshcore.commands().lock().await.get_contacts(0).await?;
    let found = after_add.iter().any(|c| c.public_key == TEST_PUBLIC_KEY);
    println!("  Present in the node's contact list: {found}");

    println!("\nRemoving test contact {TEST_CONTACT_NAME:?}...");
    meshcore
        .commands()
        .lock()
        .await
        .remove_contact(&contact)
        .await?;
    println!("  -> remove_contact returned Ok.");

    let after_remove = meshcore.commands().lock().await.get_contacts(0).await?;
    let still_present = after_remove.iter().any(|c| c.public_key == TEST_PUBLIC_KEY);
    println!("  Still present in the node's contact list: {still_present}");
    if still_present {
        eprintln!(
            "WARNING: test contact was not actually removed -- you may need to remove it by hand."
        );
    }

    meshcore.disconnect().await?;
    Ok(())
}
