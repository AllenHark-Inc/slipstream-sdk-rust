use allenhark_slipstream::{Config, SlipstreamClient};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create configuration
    let config = Config::builder()
        .api_key("sk_test_12345678")
        .region("us-east")
        .build()?;

    println!("Connecting to Slipstream...");
    
    // Connect to the network
    // This will automatically select the best available worker
    let client = SlipstreamClient::connect(config).await?;

    println!("Connected successfully!");
    let info = client.connection_info();
    println!("Session ID: {}", info.session_id);
    println!("Protocol: {}", info.protocol);
    if let Some(region) = &info.region {
        println!("Region: {}", region);
    }

    // Keep connection alive for a bit
    println!("Keeping connection alive for 5 seconds...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Disconnect
    client.disconnect().await?;
    println!("Disconnected");

    Ok(())
}
