//! Tip Instructions Example
//!
//! This example demonstrates how to subscribe to and use tip instructions
//! from Slipstream. Tip instructions tell you which wallet to tip and how
//! much, enabling optimal transaction routing through the fastest senders.
//!
//! # Key Concepts
//!
//! - **TipInstruction**: Contains the tip wallet, amount, sender, and confidence
//! - **get_latest_tip()**: Get the most recent cached tip instruction
//! - **subscribe_tip_instructions()**: Stream real-time tip updates
//! - **Tip Tiers**: Different tiers offer different latency guarantees
//!
//! # Running this example
//!
//! ```bash
//! SLIPSTREAM_API_KEY=sk_test_xxx cargo run --example tip_instructions
//! ```

use allenhark_slipstream::{Config, SlipstreamClient, TipInstruction};
use std::env;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    let api_key = env::var("SLIPSTREAM_API_KEY")
        .unwrap_or_else(|_| "sk_test_12345678".to_string());

    println!("=== Slipstream Tip Instructions Example ===\n");

    // =========================================================================
    // EXAMPLE 1: Enable Tip Instruction Streaming
    // =========================================================================
    println!("1. Configuration with Tip Instructions");
    println!("   ------------------------------------");

    let config = Config::builder()
        .api_key(&api_key)
        .stream_tip_instructions(true) // Enable tip instruction streaming
        .min_confidence(70)             // Only accept tips with 70%+ confidence
        .build()?;

    println!("   Tip Instructions Streaming: {}", config.stream_tip_instructions);
    println!("   Minimum Confidence: {}%\n", config.min_confidence);

    // =========================================================================
    // EXAMPLE 2: Connect and Subscribe to Tip Instructions
    // =========================================================================
    println!("2. Subscribing to Tip Instructions");
    println!("   --------------------------------");

    match SlipstreamClient::connect(config).await {
        Ok(client) => {
            println!("   Connected to Slipstream!");

            // Subscribe to tip instruction updates
            let mut tip_stream = client.subscribe_tip_instructions().await?;

            println!("   Listening for tip instructions (10s timeout)...\n");

            let timeout = Duration::from_secs(10);
            let start = std::time::Instant::now();

            loop {
                tokio::select! {
                    Some(tip) = tip_stream.recv() => {
                        print_tip_instruction(&tip);
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        if start.elapsed() > timeout {
                            println!("   Timeout reached.");
                            break;
                        }
                    }
                }
            }

            // =========================================================================
            // EXAMPLE 3: Get Latest Cached Tip
            // =========================================================================
            println!("\n3. Getting Latest Cached Tip");
            println!("   --------------------------");

            if let Some(latest_tip) = client.get_latest_tip().await {
                println!("   ✅ Latest tip available:");
                print_tip_instruction(&latest_tip);
            } else {
                println!("   ⚠️  No tip cached yet (stream may not have received any)");
            }

            client.disconnect().await?;
        }
        Err(e) => {
            println!("   ⚠️  Could not connect (expected in test): {}\n", e);
        }
    }

    // =========================================================================
    // EXAMPLE 4: Building a Transaction with a Tip
    // =========================================================================
    println!("\n4. Building a Transaction with a Tip");
    println!("   ----------------------------------");
    println!("   Here's how to include a tip in your transaction:\n");

    println!(r#"
   // Get the latest tip instruction
   let tip = client.get_latest_tip().await
       .expect("No tip available");

   // Parse the tip wallet address
   let tip_wallet = Pubkey::from_str(&tip.tip_wallet_address)?;

   // Create a transfer instruction for the tip
   let tip_amount_lamports = (tip.tip_amount_sol * 1_000_000_000.0) as u64;
   let tip_ix = system_instruction::transfer(
       &payer.pubkey(),
       &tip_wallet,
       tip_amount_lamports,
   );

   // Add the tip instruction to your transaction
   transaction.instructions.push(tip_ix);

   // Sign and submit
   let result = client.submit_transaction(&transaction.serialize()).await?;
"#);

    // =========================================================================
    // EXAMPLE 5: Using Alternative Senders
    // =========================================================================
    println!("5. Using Alternative Senders");
    println!("   --------------------------");
    println!("   TipInstruction includes alternative sender options:\n");

    println!(r#"
   if let Some(tip) = client.get_latest_tip().await {{
       // Use the primary sender
       println!("Primary: {{}} ({{}}% confidence)", 
           tip.sender, tip.confidence);

       // Or check alternatives
       for alt in &tip.alternative_senders {{
           println!("Alternative: {{}} ({{}} SOL, {{}}% confidence)",
               alt.sender, alt.tip_amount_sol, alt.confidence);
       }}
   }}
"#);

    println!("=== Tip Instructions Example Complete ===");

    Ok(())
}

/// Pretty-print a tip instruction
fn print_tip_instruction(tip: &TipInstruction) {
    println!("   💰 Tip Instruction:");
    println!("      Sender: {} ({})", tip.sender, tip.sender_name);
    println!("      Tip Wallet: {}", tip.tip_wallet_address);
    println!("      Amount: {} SOL", tip.tip_amount_sol);
    println!("      Tier: {}", tip.tip_tier);
    println!("      Expected Latency: {}ms", tip.expected_latency_ms);
    println!("      Confidence: {}%", tip.confidence);
    println!("      Valid Until Slot: {}", tip.valid_until_slot);
    if !tip.alternative_senders.is_empty() {
        println!("      Alternatives: {} available", tip.alternative_senders.len());
    }
    println!();
}
