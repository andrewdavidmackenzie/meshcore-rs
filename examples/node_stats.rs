//! Example: fetch and print a node's core/radio/packet stats.
//!
//! The Rust-side counterpart to `meshcore_py`'s own `examples/ble_stats.py`.
//! Each category is fetched and reported independently -- one failing (e.g.
//! older firmware missing `recv_errors` in the packets frame, or a node
//! that doesn't support `CMD_GET_STATS` at all, pre companion-v1.11.0)
//! doesn't stop the others from being tried.
//!
//! Usage: exactly one of the following is required
//!   cargo run --example node_stats --features serial -- --serial <port>
//!   cargo run --example node_stats --features tcp -- --tcp <host:port>
//!   cargo run --example node_stats --features ble -- --ble <device-name>

#[path = "common/mod.rs"]
mod common;

use common::{connect, parse_args, ConnectionArgs};

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

    println!("\nFetching core stats...");
    match meshcore.commands().lock().await.get_core_stats().await {
        Ok(stats) => {
            println!("  battery_mv:   {}", stats.battery_mv);
            println!("  uptime_secs:  {}", stats.uptime_secs);
            println!("  errors:       {}", stats.errors);
            println!("  queue_len:    {}", stats.queue_len);
        }
        Err(err) => eprintln!("  Error getting core stats: {err}"),
    }

    println!("\nFetching radio stats...");
    match meshcore.commands().lock().await.get_radio_stats().await {
        Ok(stats) => {
            println!("  noise_floor:  {} dBm", stats.noise_floor);
            println!("  last_rssi:    {} dBm", stats.last_rssi);
            println!("  last_snr:     {} dB", stats.last_snr);
            println!("  tx_air_secs:  {}", stats.tx_air_secs);
            println!("  rx_air_secs:  {}", stats.rx_air_secs);
        }
        Err(err) => eprintln!("  Error getting radio stats: {err}"),
    }

    println!("\nFetching packet stats...");
    match meshcore.commands().lock().await.get_packet_stats().await {
        Ok(stats) => {
            println!("  recv:         {}", stats.recv);
            println!("  sent:         {}", stats.sent);
            println!("  flood_tx:     {}", stats.flood_tx);
            println!("  direct_tx:    {}", stats.direct_tx);
            println!("  flood_rx:     {}", stats.flood_rx);
            println!("  direct_rx:    {}", stats.direct_rx);
            match stats.recv_errors {
                Some(errors) => println!("  recv_errors:  {errors}"),
                None => println!("  recv_errors:  not reported (legacy 26-byte frame)"),
            }
        }
        Err(err) => eprintln!("  Error getting packet stats: {err}"),
    }

    meshcore.disconnect().await?;
    Ok(())
}
