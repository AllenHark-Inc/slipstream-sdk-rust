//! Streaming & Callbacks Example
//!
//! Demonstrates all streaming subscriptions (Rust's channel-based callbacks)
//! and how to react to real-time data from Slipstream.

use allenhark_slipstream::{Config, SlipstreamClient};
use std::env;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let api_key = env::var("SLIPSTREAM_API_KEY").unwrap_or("sk_test_12345678".into());

    println!("=== Streaming & Callbacks Example ===\n");
    println!("Rust uses channels instead of callbacks. Here's the pattern:\n");

    // =========================================================================
    // 1. Configuration for Streaming
    // =========================================================================
    println!("1. Enable All Streams in Config");
    let config = Config::builder()
        .api_key(&api_key)
        .leader_hints(true)           // onLeaderHint equivalent
        .stream_tip_instructions(true) // onTipInstruction equivalent  
        .stream_priority_fees(true)    // onPriorityFee equivalent
        .min_confidence(70)
        .build()?;

    println!("   Leader Hints: {}", config.leader_hints);
    println!("   Tip Instructions: {}", config.stream_tip_instructions);
    println!("   Priority Fees: {}", config.stream_priority_fees);

    // =========================================================================
    // 2. Subscribing to All Streams
    // =========================================================================
    println!("\n2. Subscribe and Handle All Streams");
    println!(r#"
   // Subscribe to each stream
   let mut hints = client.subscribe_leader_hints().await?;
   let mut tips = client.subscribe_tip_instructions().await?;
   let mut fees = client.subscribe_priority_fees().await?;

   // Handle all streams concurrently with tokio::select!
   loop {{
       tokio::select! {{
           Some(hint) = hints.recv() => {{
               // onLeaderHint callback
               println!("Leader: {{}} ({{}}%)", hint.preferred_region, hint.confidence);
           }}
           Some(tip) = tips.recv() => {{
               // onTipInstruction callback
               println!("Tip: {{}} SOL to {{}}", tip.tip_amount_sol, tip.tip_wallet_address);
           }}
           Some(fee) = fees.recv() => {{
               // onPriorityFee callback
               println!("Fee: {{}} micro-lamports", fee.compute_unit_price);
           }}
       }}
   }}
"#);

    // =========================================================================
    // 3. Connection Status (onConnectionStatusChange)
    // =========================================================================
    println!("3. Connection Status Monitoring");
    println!(r#"
   // Poll connection status (equivalent to onConnectionStatusChange)
   let status = client.connection_status().await;
   println!("State: {{:?}}, Protocol: {{:?}}", status.state, status.protocol);
   
   // For continuous monitoring:
   tokio::spawn(async move {{
       loop {{
           let status = client.connection_status().await;
           if status.state != ConnectionState::Connected {{
               println!("Connection changed: {{:?}}", status.state);
           }}
           tokio::time::sleep(Duration::from_secs(5)).await;
       }}
   }});
"#);

    // =========================================================================
    // 4. Performance Metrics (onMetrics)
    // =========================================================================
    println!("4. Performance Metrics");
    println!(r#"
   // Get metrics (equivalent to onMetrics)
   let metrics = client.metrics();
   println!("Submitted: {{}}", metrics.transactions_submitted);
   println!("Confirmed: {{}}", metrics.transactions_confirmed);
   println!("Success Rate: {{:.1}}%", metrics.success_rate * 100.0);
   println!("Avg Latency: {{:.1}}ms", metrics.average_latency_ms);
"#);

    // =========================================================================
    // 5. Error Handling (onError)
    // =========================================================================
    println!("5. Error Handling");
    println!(r#"
   // Errors are returned via Result<T, SdkError>
   match client.submit_transaction(&tx).await {{
       Ok(result) => println!("Success: {{:?}}", result.signature),
       Err(e) => {{
           // onError equivalent
           match e {{
               SdkError::Connection(msg) => println!("Connection error: {{}}", msg),
               SdkError::Protocol(msg) => println!("Protocol error: {{}}", msg),
               SdkError::RateLimited => println!("Rate limited, back off"),
               _ => println!("Other error: {{}}", e),
           }}
       }}
   }}
"#);

    // =========================================================================
    // 6. Live Demo
    // =========================================================================
    println!("6. Live Demo\n");

    match SlipstreamClient::connect(config).await {
        Ok(client) => {
            println!("   Connected!");
            
            // Get status
            let status = client.connection_status().await;
            println!("   Status: {:?} via {:?}", status.state, status.protocol);
            
            // Get metrics
            let metrics = client.metrics();
            println!("   Metrics: {} tx, {:.0}% success", 
                metrics.transactions_submitted, metrics.success_rate * 100.0);

            // Subscribe briefly
            let mut hints = client.subscribe_leader_hints().await?;
            
            println!("   Listening 5s for leader hints...");
            let timeout = Duration::from_secs(5);
            let start = std::time::Instant::now();
            
            loop {
                tokio::select! {
                    Some(hint) = hints.recv() => {
                        println!("   🎯 Leader: {} ({}%)", hint.preferred_region, hint.confidence);
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        if start.elapsed() > timeout { break; }
                    }
                }
            }

            client.disconnect().await?;
        }
        Err(e) => println!("   ⚠️  Connection failed: {}", e),
    }

    println!("\n=== Streaming Complete ===");
    Ok(())
}
