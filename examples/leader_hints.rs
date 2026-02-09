//! Leader Hints Example
//!
//! This example demonstrates how to subscribe to and use leader hints
//! for optimal transaction routing. Leader hints tell you which region
//! is closest to the current block producer, enabling faster landing.
//!
//! # Key Concepts
//!
//! - **LeaderHint**: Contains region, confidence, and timing information
//! - **Preferred Region**: The region closest to the current leader
//! - **Confidence Score**: How certain Slipstream is about the leader location
//! - **Slots Remaining**: How many slots until leader change
//!
//! # Running this example
//!
//! ```bash
//! SLIPSTREAM_API_KEY=sk_test_xxx cargo run --example leader_hints
//! ```

use allenhark_slipstream::{Config, SlipstreamClient, LeaderHint};
use std::env;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    let api_key = env::var("SLIPSTREAM_API_KEY")
        .unwrap_or_else(|_| "sk_test_12345678".to_string());

    println!("=== Slipstream Leader Hints Example ===\n");

    // =========================================================================
    // EXAMPLE 1: Enable Leader Hints
    // =========================================================================
    println!("1. Configuration with Leader Hints");
    println!("   --------------------------------");

    let config = Config::builder()
        .api_key(&api_key)
        .leader_hints(true)     // Enable leader hint streaming (default: true)
        .min_confidence(70)     // Only accept hints with 70%+ confidence
        .build()?;

    println!("   Leader Hints: {}", config.leader_hints);
    println!("   Min Confidence: {}%\n", config.min_confidence);

    // =========================================================================
    // EXAMPLE 2: Connect and Subscribe
    // =========================================================================
    println!("2. Subscribing to Leader Hints");
    println!("   ----------------------------");

    match SlipstreamClient::connect(config).await {
        Ok(client) => {
            println!("   Connected to Slipstream!");

            // Subscribe to leader hint updates
            let mut hints = client.subscribe_leader_hints().await?;

            println!("   Listening for leader hints (15s timeout)...\n");

            let timeout = Duration::from_secs(15);
            let start = std::time::Instant::now();
            let mut hint_count = 0;

            loop {
                tokio::select! {
                    Some(hint) = hints.recv() => {
                        hint_count += 1;
                        print_leader_hint(&hint, hint_count);

                        // After 5 hints, demonstrate usage
                        if hint_count >= 5 {
                            println!("\n   Collected 5 hints. Demonstrating usage...\n");
                            demonstrate_hint_usage(&hint);
                            break;
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        if start.elapsed() > timeout {
                            println!("   Timeout reached.");
                            break;
                        }
                    }
                }
            }

            client.disconnect().await?;
        }
        Err(e) => {
            println!("   ⚠️  Could not connect (expected in test): {}\n", e);
        }
    }

    // =========================================================================
    // EXAMPLE 3: Understanding Leader Hints
    // =========================================================================
    println!("\n3. Understanding Leader Hint Fields");
    println!("   ---------------------------------\n");

    println!("   • preferred_region: The region closest to the current leader");
    println!("     Example: 'us-east', 'eu-central', 'ap-northeast'");
    println!();
    println!("   • backup_regions: Alternative regions in order of preference");
    println!("     Useful for failover if preferred region is unavailable");
    println!();
    println!("   • confidence: How certain Slipstream is (0-100%)");
    println!("     High confidence = reliable routing decision");
    println!();
    println!("   • slots_remaining: Slots until next leader change");
    println!("     Lower = hint will expire soon, higher = more stable");
    println!();
    println!("   • leader_pubkey: The validator's public key (optional)");
    println!("     Useful for advanced routing or analytics");
    println!();
    println!("   • metadata.tpu_rtt_ms: Round-trip time to TPU");
    println!("     Lower = faster transaction landing\n");

    // =========================================================================
    // EXAMPLE 4: Using Leader Hints for Routing
    // =========================================================================
    println!("4. Using Leader Hints for Routing Decisions");
    println!("   -----------------------------------------\n");

    println!(r#"
   // Get the leader hint stream
   let mut hints = client.subscribe_leader_hints().await?;

   // React to leader changes
   tokio::spawn(async move {{
       while let Some(hint) = hints.recv().await {{
           // Only act on high-confidence hints
           if hint.confidence >= 80 {{
               println!("🎯 High confidence leader in {{}}", hint.preferred_region);
               
               // Update your routing preference
               update_preferred_region(&hint.preferred_region);
               
               // Check if leader is about to change
               if hint.expires_at_slot - hint.slot < 4 {{
                   println!("⚡ Leader changing soon, prepare for switch");
               }}
           }}
       }}
   }});

   // Your transaction submission will automatically
   // route to the optimal region
"#);

    // =========================================================================
    // EXAMPLE 5: Filtering by Confidence
    // =========================================================================
    println!("5. Filtering by Confidence Level");
    println!("   ------------------------------\n");

    println!(r#"
   // Configure minimum confidence in SDK
   let config = Config::builder()
       .api_key(api_key)
       .min_confidence(80)  // Only 80%+ confidence
       .build()?;

   // Or filter manually in your code
   while let Some(hint) = hints.recv().await {{
       match hint.confidence {{
           90..=100 => {{
               // Very high confidence - route aggressively
               submit_with_low_timeout(tx, &hint).await?;
           }}
           70..=89 => {{
               // Medium confidence - use normal routing
               submit_normal(tx, &hint).await?;
           }}
           _ => {{
               // Low confidence - wait for better hint
               continue;
           }}
       }}
   }}
"#);

    println!("=== Leader Hints Example Complete ===");

    Ok(())
}

/// Pretty-print a leader hint
fn print_leader_hint(hint: &LeaderHint, count: usize) {
    println!("   🎯 Leader Hint #{}", count);
    println!("      Preferred Region: {}", hint.preferred_region);
    println!("      Backup Regions: {:?}", hint.backup_regions);
    println!("      Confidence: {}%", hint.confidence);
    println!("      Current Slot: {}", hint.slot);
    println!("      Expires at Slot: {}", hint.expires_at_slot);
    if !hint.leader_pubkey.is_empty() && hint.leader_pubkey.len() >= 16 {
        println!("      Leader: {}...", &hint.leader_pubkey[..16]);
    }
    println!("      TPU RTT: {}ms", hint.metadata.tpu_rtt_ms);
    println!("      Region Score: {:.2}", hint.metadata.region_score);
    println!();
}

/// Demonstrate how to use a leader hint
fn demonstrate_hint_usage(hint: &LeaderHint) {
    println!("   Based on current hint:");
    println!("   → Route transactions to: {}", hint.preferred_region);
    println!("   → Fallback to: {:?}", hint.backup_regions.first().unwrap_or(&"none".to_string()));
    println!("   → {} slots until leader change", hint.expires_at_slot - hint.slot);

    if hint.confidence >= 90 {
        println!("   → HIGH confidence: Use aggressive timeouts");
    } else if hint.confidence >= 70 {
        println!("   → MEDIUM confidence: Use standard timeouts");
    } else {
        println!("   → LOW confidence: Consider waiting for better hint");
    }
}
