use allenhark_slipstream::{Config, SlipstreamClient, SubmitOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Create configuration
    let config = Config::builder()
        .api_key("sk_test_12345678")
        .region("us-east")
        .build()?;

    println!("Connecting to Slipstream...");
    let client = SlipstreamClient::connect(config).await?;
    println!("Connected!");

    // Mock transaction data (normally this would be a signed Solana transaction)
    let transaction = vec![1, 2, 3, 4, 5];

    println!("Submitting transaction...");
    
    // Submit with default options
    let result = client.submit_transaction(&transaction).await?;
    
    println!("Transaction submitted!");
    println!("Request ID: {}", result.request_id);
    println!("Status: {:?}", result.status);

    // Submit with custom options
    let options = SubmitOptions {
        broadcast_mode: true,
        max_retries: 5,
        ..Default::default()
    };
    
    println!("Submitting with broadcast mode...");
    let result = client.submit_transaction_with_options(&transaction, &options).await?;
    
    println!("Broadcast transaction submitted!");
    println!("Request ID: {}", result.request_id);
    
    client.disconnect().await?;
    Ok(())
}
