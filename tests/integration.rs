use allenhark_slipstream::Config;

#[tokio::test]
async fn test_client_config_validation() {
    let config = Config::builder()
        .api_key("sk_test_123")
        .build();
    
    assert!(config.is_ok());

    let bad_config = Config::builder()
        .api_key("invalid_prefix")
        .build();
        
    assert!(bad_config.is_err());
}

#[tokio::test]
async fn test_client_connect_failure_no_server() {
    // Attempt to connect to a random port that definitely doesn't have a server
    let config = Config::builder()
        .api_key("sk_test_123")
        .endpoint("http://localhost:54321")
        .build()
        .unwrap();

    let result = allenhark_slipstream::SlipstreamClient::connect(config).await;
    
    // Should fail with connection refused or timeout
    assert!(result.is_err());
    
    println!("Got expected error: {:?}", result.err());
}
