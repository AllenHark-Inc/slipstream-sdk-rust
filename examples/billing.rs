//! Token Billing Example
//!
//! This example demonstrates how to use the Slipstream SDK to:
//! - Check your token balance
//! - Get your deposit address for SOL top-ups
//! - View deposit history (credited and pending)
//! - Check pending deposit status and minimum requirements
//! - View usage/billing history
//!
//! # Running this example
//!
//! ```bash
//! SLIPSTREAM_API_KEY=sk_test_xxx cargo run --example billing
//! ```

use allenhark_slipstream::{Config, SlipstreamClient};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    println!("=== Slipstream Token Billing Example ===\n");

    let api_key = env::var("SLIPSTREAM_API_KEY")
        .unwrap_or_else(|_| "sk_test_12345678".to_string());

    // Connect to Slipstream
    let config = Config::builder()
        .api_key(&api_key)
        .region("us-east")
        .build()?;

    let client = SlipstreamClient::connect(config).await?;
    println!("Connected to Slipstream\n");

    // =========================================================================
    // 1. Check Token Balance
    // =========================================================================
    println!("1. Token Balance");
    println!("   -------------");
    let balance = client.get_balance().await?;
    println!("   Balance:     {:.6} SOL ({} tokens)", balance.balance_sol, balance.balance_tokens);
    println!("   Lamports:    {}", balance.balance_lamports);
    println!("   Grace left:  {} tokens\n", balance.grace_remaining_tokens);

    // =========================================================================
    // 2. Get Deposit Address
    // =========================================================================
    println!("2. Deposit Address");
    println!("   ---------------");
    let deposit = client.get_deposit_address().await?;
    println!("   Wallet:      {}", deposit.deposit_wallet);
    println!("   Min top-up:  {:.4} SOL ({} lamports)\n",
        deposit.min_amount_sol, deposit.min_amount_lamports);

    // =========================================================================
    // 3. Minimum Deposit Requirement
    // =========================================================================
    println!("3. Minimum Deposit");
    println!("   ---------------");
    let min_usd = client.get_minimum_deposit_usd();
    println!("   Minimum:     ${:.2} USD equivalent in SOL", min_usd);
    println!("   Note:        Deposits below this are held as pending");
    println!("                until cumulative total reaches ${:.2}\n", min_usd);

    // =========================================================================
    // 4. Check Pending Deposits
    // =========================================================================
    println!("4. Pending Deposits");
    println!("   ----------------");
    match client.get_pending_deposit().await {
        Ok(pending) => {
            println!("   Pending:     {:.6} SOL ({} lamports)",
                pending.pending_sol, pending.pending_lamports);
            println!("   Count:       {} uncredited deposits", pending.pending_count);
            println!("   Minimum:     ${:.2} USD\n", pending.minimum_deposit_usd);
        }
        Err(e) => println!("   Error: {} (endpoint may not be available yet)\n", e),
    }

    // =========================================================================
    // 5. Deposit History
    // =========================================================================
    println!("5. Deposit History");
    println!("   ---------------");
    let deposit_opts = allenhark_slipstream::types::DepositHistoryOptions {
        limit: Some(10),
        offset: None,
    };
    match client.get_deposit_history(deposit_opts).await {
        Ok(deposits) => {
            if deposits.is_empty() {
                println!("   No deposits found\n");
            } else {
                for (i, d) in deposits.iter().enumerate() {
                    println!("   [{}] {} | {:.6} SOL | ${:.2} USD | {}",
                        i + 1,
                        &d.signature[..16],
                        d.amount_sol,
                        d.usd_value.unwrap_or(0.0),
                        if d.credited { "CREDITED" } else { "PENDING" },
                    );
                }
                println!();
            }
        }
        Err(e) => println!("   Error: {} (endpoint may not be available yet)\n", e),
    }

    // =========================================================================
    // 6. Usage History
    // =========================================================================
    println!("6. Usage History (last 10)");
    println!("   ----------------------");
    let usage_opts = allenhark_slipstream::types::UsageHistoryOptions {
        limit: Some(10),
        offset: None,
    };
    match client.get_usage_history(usage_opts).await {
        Ok(entries) => {
            if entries.is_empty() {
                println!("   No usage entries found\n");
            } else {
                for entry in &entries {
                    let sign = if entry.amount_lamports >= 0 { "+" } else { "" };
                    println!("   {} | {}{} lamports | balance: {} | {}",
                        entry.tx_type,
                        sign,
                        entry.amount_lamports,
                        entry.balance_after_lamports,
                        entry.description.as_deref().unwrap_or("-"),
                    );
                }
                println!();
            }
        }
        Err(e) => println!("   Error: {}\n", e),
    }

    // Disconnect
    client.disconnect().await?;
    println!("Disconnected.\n");

    Ok(())
}
