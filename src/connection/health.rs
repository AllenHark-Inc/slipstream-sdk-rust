use crate::config::Config;
use crate::connection::{FallbackChain, Transport};
use crate::error::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Monitor connection health and handle automatic reconnection
pub struct HealthMonitor {
    config: Config,
    transport: Arc<RwLock<Box<dyn Transport>>>,
    interval: Duration,
}

impl HealthMonitor {
    /// Create a new health monitor
    pub fn new(config: Config, transport: Arc<RwLock<Box<dyn Transport>>>) -> Self {
        Self {
            config,
            transport,
            interval: Duration::from_secs(5),
        }
    }

    /// Start the monitoring loop in the background
    pub fn start(self) {
        tokio::spawn(async move {
            self.monitor_loop().await;
        });
    }

    /// Main monitoring loop
    async fn monitor_loop(&self) {
        let mut interval = tokio::time::interval(self.interval);

        loop {
            interval.tick().await;

            let is_connected = {
                let transport = self.transport.read().await;
                transport.is_connected()
            };

            if !is_connected {
                warn!("Connection lost, attempting reconnect...");
                if let Err(e) = self.reconnect().await {
                    error!("Reconnection failed: {}", e);
                }
            }
        }
    }

    /// Attempt to reconnect using the fallback chain
    async fn reconnect(&self) -> Result<()> {
        let fallback_chain = FallbackChain::new(self.config.protocol_timeouts.clone());
        
        match fallback_chain.connect(&self.config).await {
            Ok(mut new_transport) => {
                // Initialize the new transport
                new_transport.connect(&self.config).await?;
                
                // Atomically replace the old transport
                let mut transport = self.transport.write().await;
                *transport = new_transport;
                
                info!("Successfully reconnected");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}
