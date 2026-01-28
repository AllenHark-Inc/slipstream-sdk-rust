//! Deduplication Example
//!
//! Demonstrates how to use deduplication IDs to prevent duplicate
//! transaction submissions across retries and multiple SDK instances.

use allenhark_slipstream::{Config, SlipstreamClient, SubmitOptions};
use std::env;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let api_key = env::var("SLIPSTREAM_API_KEY").unwrap_or("sk_test_12345678".into());

    println!("=== Deduplication Example ===\n");

    // =========================================================================
    // 1. Basic Deduplication
    // =========================================================================
    println!("1. Basic Deduplication");
    println!("   Use a unique ID per logical transaction\n");

    let dedup_options = SubmitOptions {
        dedup_id: Some("swap-user123-1706472000".to_string()),
        max_retries: 5,
        ..Default::default()
    };
    println!("   Dedup ID: {:?}", dedup_options.dedup_id);

    // =========================================================================
    // 2. UUID-based Deduplication
    // =========================================================================
    println!("\n2. UUID-based Deduplication");
    let uuid_options = SubmitOptions {
        dedup_id: Some(Uuid::new_v4().to_string()),
        ..Default::default()
    };
    println!("   UUID: {:?}", uuid_options.dedup_id);

    // =========================================================================
    // 3. Content-based Deduplication
    // =========================================================================
    println!("\n3. Content-based Deduplication");
    println!("   Hash transaction content for deterministic IDs\n");

    println!(r#"
   use sha2::{{Sha256, Digest}};
   
   fn content_based_dedup(tx: &[u8], user_id: &str) -> String {{
       let mut hasher = Sha256::new();
       hasher.update(tx);
       hasher.update(user_id.as_bytes());
       format!("tx-{{}}", hex::encode(&hasher.finalize()[..8]))
   }}
"#);

    // =========================================================================
    // 4. Retry Safety
    // =========================================================================
    println!("4. Retry Safety Pattern");
    println!("   Same dedup_id across retries prevents double-spend\n");

    println!(r#"
   async fn safe_submit(client: &SlipstreamClient, tx: &[u8]) -> Result<String> {{
       let dedup_id = Uuid::new_v4().to_string();
       
       for attempt in 1..=3 {{
           let options = SubmitOptions {{
               dedup_id: Some(dedup_id.clone()), // Same ID each retry!
               max_retries: 0, // SDK won't auto-retry
               ..Default::default()
           }};
           
           match client.submit_transaction_with_options(tx, &options).await {{
               Ok(result) if result.status.is_terminal() => {{
                   return Ok(result.signature.unwrap());
               }}
               Err(e) if attempt < 3 => {{
                   println!("Attempt {{}} failed: {{}}", attempt, e);
                   tokio::time::sleep(Duration::from_millis(100 * attempt)).await;
               }}
               Err(e) => return Err(e),
               _ => continue,
           }}
       }}
       Err("Max retries exceeded".into())
   }}
"#);

    // =========================================================================
    // 5. Connect and Demo
    // =========================================================================
    let config = Config::builder().api_key(&api_key).build()?;

    match SlipstreamClient::connect(config).await {
        Ok(client) => {
            println!("\n5. Live Demo");
            let dummy_tx = vec![0u8; 100];
            let options = SubmitOptions {
                dedup_id: Some(format!("demo-{}", Uuid::new_v4())),
                ..Default::default()
            };
            
            match client.submit_transaction_with_options(&dummy_tx, &options).await {
                Ok(r) => println!("   Result: {:?}", r.status),
                Err(e) => println!("   Expected error: {}", e),
            }
            client.disconnect().await?;
        }
        Err(e) => println!("\n⚠️  Connection failed: {}", e),
    }

    println!("\n=== Deduplication Complete ===");
    Ok(())
}
