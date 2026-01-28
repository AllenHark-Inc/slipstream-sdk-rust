//! Signer Integration Example
//!
//! This example demonstrates how to integrate the Slipstream SDK with
//! various Solana signing mechanisms. The SDK accepts pre-signed transaction
//! bytes, giving you flexibility in how you manage keys.
//!
//! # Supported Signing Methods
//!
//! - **Keypair**: Local keypair from file or bytes
//! - **Hardware Wallets**: Ledger via solana-remote-wallet
//! - **Multi-sig**: Multiple signers for complex transactions
//! - **MPC/Custodial**: External signing services
//!
//! # Running this example
//!
//! ```bash
//! SLIPSTREAM_API_KEY=sk_test_xxx cargo run --example signer_integration
//! ```

use allenhark_slipstream::{Config, SlipstreamClient};
use std::env;

// Note: In a real application, you would import these from solana_sdk
// use solana_sdk::{
//     signature::{Keypair, Signer},
//     transaction::Transaction,
//     message::Message,
//     pubkey::Pubkey,
//     system_instruction,
// };

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    let api_key = env::var("SLIPSTREAM_API_KEY")
        .unwrap_or_else(|_| "sk_test_12345678".to_string());

    println!("=== Slipstream Signer Integration Example ===\n");

    // =========================================================================
    // EXAMPLE 1: Local Keypair Signing
    // =========================================================================
    println!("1. Local Keypair Signing");
    println!("   ----------------------");
    println!("   The most common approach for bots and automated systems.\n");

    println!(r#"
   use solana_sdk::{{
       signature::{{Keypair, Signer}},
       transaction::Transaction,
       message::Message,
       system_instruction,
   }};
   use std::fs;

   // Load keypair from file
   let keypair_bytes = fs::read("~/.config/solana/id.json")?;
   let keypair = Keypair::from_bytes(&keypair_bytes)?;

   // Or generate a new one
   let new_keypair = Keypair::new();

   // Build your transaction
   let instruction = system_instruction::transfer(
       &keypair.pubkey(),
       &recipient,
       amount_lamports,
   );

   let message = Message::new(&[instruction], Some(&keypair.pubkey()));
   let mut transaction = Transaction::new_unsigned(message);

   // Sign locally
   transaction.sign(&[&keypair], recent_blockhash);

   // Submit to Slipstream
   let result = client.submit_transaction(&transaction.serialize()?).await?;
"#);

    // =========================================================================
    // EXAMPLE 2: Environment Variable / Secure Storage
    // =========================================================================
    println!("\n2. Secure Key Management");
    println!("   ----------------------");
    println!("   Best practices for production environments.\n");

    println!(r#"
   use bs58;
   use std::env;

   // From environment variable (base58 encoded)
   let private_key_b58 = env::var("PRIVATE_KEY")?;
   let keypair_bytes = bs58::decode(&private_key_b58).into_vec()?;
   let keypair = Keypair::from_bytes(&keypair_bytes)?;

   // From AWS Secrets Manager, HashiCorp Vault, etc.
   // let secret = vault_client.get_secret("solana-keypair").await?;
   // let keypair = Keypair::from_bytes(&secret.bytes)?;
"#);

    // =========================================================================
    // EXAMPLE 3: Hardware Wallet (Ledger)
    // =========================================================================
    println!("\n3. Hardware Wallet (Ledger)");
    println!("   -------------------------");
    println!("   For high-security applications requiring physical confirmation.\n");

    println!(r#"
   use solana_remote_wallet::{{
       remote_wallet::RemoteWalletManager,
       locator::Locator,
   }};

   // Initialize the remote wallet manager
   let wallet_manager = RemoteWalletManager::new()?;

   // Get the Ledger device
   let locator = Locator::new_from_uri("usb://ledger")?;
   let ledger = wallet_manager.get_ledger(&locator).await?;

   // Get the public key (for address derivation)
   let pubkey = ledger.get_pubkey(&derivation_path)?;

   // Build transaction
   let message = Message::new(&instructions, Some(&pubkey));
   let transaction = Transaction::new_unsigned(message);

   // Sign with Ledger (user must confirm on device)
   let signature = ledger.sign_message(&transaction.message_data())?;
   transaction.add_signature(&pubkey, signature);

   // Submit to Slipstream
   let result = client.submit_transaction(&transaction.serialize()?).await?;
"#);

    // =========================================================================
    // EXAMPLE 4: Multi-Signature Transactions
    // =========================================================================
    println!("\n4. Multi-Signature Transactions");
    println!("   -----------------------------");
    println!("   For governance or shared custody scenarios.\n");

    println!(r#"
   // Multiple signers
   let signer1 = Keypair::from_bytes(&key1_bytes)?;
   let signer2 = Keypair::from_bytes(&key2_bytes)?;

   // Build transaction requiring multiple signatures
   let message = Message::new_with_payer(
       &instructions,
       Some(&signer1.pubkey()),
   );
   let mut transaction = Transaction::new_unsigned(message);

   // Each signer adds their signature
   transaction.partial_sign(&[&signer1], recent_blockhash);
   transaction.partial_sign(&[&signer2], recent_blockhash);

   // Verify all required signatures are present
   assert!(transaction.is_signed());

   // Submit to Slipstream
   let result = client.submit_transaction(&transaction.serialize()?).await?;
"#);

    // =========================================================================
    // EXAMPLE 5: External/MPC Signing Service
    // =========================================================================
    println!("\n5. External/MPC Signing Service");
    println!("   -----------------------------");
    println!("   For custodial solutions using MPC or external APIs.\n");

    println!(r#"
   use reqwest;

   // Your MPC signing service endpoint
   let signing_service = "https://signing.yourcompany.com";

   // Build unsigned transaction
   let message = Message::new(&instructions, Some(&wallet_pubkey));
   let transaction = Transaction::new_unsigned(message);

   // Send to signing service
   let response = reqwest::Client::new()
       .post(format!("{{}}/sign", signing_service))
       .json(&SignRequest {{
           message: bs58::encode(&transaction.message_data()).into_string(),
           wallet_id: "my-wallet",
       }})
       .send()
       .await?;

   let signed: SignResponse = response.json().await?;
   let signature = Signature::from_str(&signed.signature)?;

   // Add signature to transaction
   transaction.add_signature(&wallet_pubkey, signature);

   // Submit to Slipstream
   let result = client.submit_transaction(&transaction.serialize()?).await?;
"#);

    // =========================================================================
    // EXAMPLE 6: Complete Integration Pattern
    // =========================================================================
    println!("\n6. Complete Integration Pattern");
    println!("   -----------------------------\n");

    let config = Config::builder()
        .api_key(&api_key)
        .stream_tip_instructions(true)
        .build()?;

    match SlipstreamClient::connect(config).await {
        Ok(client) => {
            println!("   Connected to Slipstream!");

            // Get latest tip for optimal routing
            if let Some(tip) = client.get_latest_tip().await {
                println!("   Using tip: {} SOL to {}", tip.tip_amount_sol, tip.tip_wallet_address);
            }

            // In production, you would:
            // 1. Build your transaction with tip
            // 2. Sign with your chosen method
            // 3. Submit to Slipstream

            let dummy_signed_tx = vec![0u8; 200];

            let result = client.submit_transaction(&dummy_signed_tx).await;
            println!("   Submission result: {:?}", result.map(|r| r.status));

            client.disconnect().await?;
        }
        Err(e) => {
            println!("   ⚠️  Could not connect (expected in test): {}", e);
        }
    }

    println!("\n=== Signer Integration Example Complete ===");

    Ok(())
}
