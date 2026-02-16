use crate::config::Config;
use crate::connection::{FallbackChain, Transport};
use crate::error::Result;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Maximum backoff delay between reconnection attempts
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Base delay for exponential backoff
const BASE_DELAY: Duration = Duration::from_secs(1);

/// Stream types that can be auto-resubscribed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamType {
    LeaderHints,
    TipInstructions,
    PriorityFees,
    LatestBlockhash,
    LatestSlot,
}

/// Tracks active subscriptions for auto-resubscribe on reconnect
pub struct SubscriptionTracker {
    active: std::sync::RwLock<HashSet<StreamType>>,
}

impl SubscriptionTracker {
    pub fn new() -> Self {
        Self {
            active: std::sync::RwLock::new(HashSet::new()),
        }
    }

    /// Record that a subscription was created
    pub fn track(&self, stream: StreamType) {
        self.active.write().unwrap().insert(stream);
    }

    /// Get all active subscription types
    pub fn active_streams(&self) -> Vec<StreamType> {
        self.active.read().unwrap().iter().copied().collect()
    }
}

/// Monitor connection health and handle automatic reconnection
pub struct HealthMonitor {
    config: Config,
    transport: Arc<RwLock<Box<dyn Transport>>>,
    check_interval: Duration,
    subscription_tracker: Arc<SubscriptionTracker>,
}

impl HealthMonitor {
    /// Create a new health monitor
    pub fn new(
        config: Config,
        transport: Arc<RwLock<Box<dyn Transport>>>,
        subscription_tracker: Arc<SubscriptionTracker>,
    ) -> Self {
        Self {
            config,
            transport,
            check_interval: Duration::from_secs(5),
            subscription_tracker,
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
        let mut interval = tokio::time::interval(self.check_interval);
        let mut consecutive_failures: u32 = 0;

        loop {
            interval.tick().await;

            let is_connected = {
                let transport = self.transport.read().await;
                transport.is_connected()
            };

            if !is_connected {
                warn!("Connection lost, attempting reconnect...");
                match self.reconnect().await {
                    Ok(()) => {
                        consecutive_failures = 0;
                        info!("Successfully reconnected");

                        // Re-establish active subscriptions
                        self.resubscribe().await;
                    }
                    Err(e) => {
                        consecutive_failures += 1;
                        let backoff = Self::calculate_backoff(consecutive_failures);
                        error!(
                            attempt = consecutive_failures,
                            backoff_ms = backoff.as_millis() as u64,
                            "Reconnection failed: {}",
                            e
                        );
                        tokio::time::sleep(backoff).await;
                    }
                }
            } else {
                // Reset failures on successful health check
                consecutive_failures = 0;
            }
        }
    }

    /// Re-establish subscriptions after reconnect
    ///
    /// Note: This creates new subscription channels on the new transport.
    /// The old channels will have closed when the old transport was replaced.
    /// Applications should use `subscribe_*()` methods which handle this
    /// gracefully by creating new receivers.
    async fn resubscribe(&self) {
        let streams = self.subscription_tracker.active_streams();
        if streams.is_empty() {
            return;
        }

        info!(count = streams.len(), "Re-establishing subscriptions after reconnect");
        let transport = self.transport.read().await;

        for stream in streams {
            let result = match stream {
                StreamType::LeaderHints => {
                    transport.subscribe_leader_hints().await.map(|_| ())
                }
                StreamType::TipInstructions => {
                    transport.subscribe_tip_instructions().await.map(|_| ())
                }
                StreamType::PriorityFees => {
                    transport.subscribe_priority_fees().await.map(|_| ())
                }
                StreamType::LatestBlockhash => {
                    transport.subscribe_latest_blockhash().await.map(|_| ())
                }
                StreamType::LatestSlot => {
                    transport.subscribe_latest_slot().await.map(|_| ())
                }
            };

            match result {
                Ok(()) => debug!(stream = ?stream, "Re-subscribed successfully"),
                Err(e) => warn!(stream = ?stream, error = %e, "Failed to re-subscribe"),
            }
        }
    }

    /// Calculate exponential backoff with jitter
    fn calculate_backoff(attempt: u32) -> Duration {
        // 1s, 2s, 4s, 8s, 16s, 32s, 60s (capped)
        let exp = attempt.min(6); // cap exponent to avoid overflow
        let base_ms = BASE_DELAY.as_millis() as u64 * (1u64 << exp);
        let capped_ms = base_ms.min(MAX_BACKOFF.as_millis() as u64);

        // Add jitter: ±25% of the delay
        let jitter_range = capped_ms / 4;
        let jitter = if jitter_range > 0 {
            (rand::random::<u64>() % (jitter_range * 2)) as i64 - jitter_range as i64
        } else {
            0
        };
        let final_ms = (capped_ms as i64 + jitter).max(100) as u64;

        Duration::from_millis(final_ms)
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

                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_increases_exponentially() {
        // With jitter, we check the approximate range
        for attempt in 1..=6 {
            let backoff = HealthMonitor::calculate_backoff(attempt);
            let expected_base = 1000u64 * (1u64 << attempt);
            let expected_capped = expected_base.min(60_000);
            // Should be within ±25% + 100ms floor
            assert!(
                backoff.as_millis() as u64 >= expected_capped * 3 / 4 - 1,
                "attempt {} backoff {}ms too low (expected ~{}ms)",
                attempt,
                backoff.as_millis(),
                expected_capped
            );
            assert!(
                backoff.as_millis() as u64 <= expected_capped * 5 / 4 + 1,
                "attempt {} backoff {}ms too high (expected ~{}ms)",
                attempt,
                backoff.as_millis(),
                expected_capped
            );
        }
    }

    #[test]
    fn test_backoff_caps_at_max() {
        let backoff = HealthMonitor::calculate_backoff(20);
        // Even with jitter, should not exceed max + 25%
        assert!(backoff.as_millis() <= 75_000); // 60s + 25% jitter
    }

    #[test]
    fn test_backoff_first_attempt() {
        let backoff = HealthMonitor::calculate_backoff(1);
        // First attempt: 2s base ± jitter
        assert!(backoff.as_millis() >= 1_500);
        assert!(backoff.as_millis() <= 2_500);
    }

    #[test]
    fn test_subscription_tracker() {
        let tracker = SubscriptionTracker::new();
        assert!(tracker.active_streams().is_empty());

        tracker.track(StreamType::LeaderHints);
        tracker.track(StreamType::PriorityFees);
        assert_eq!(tracker.active_streams().len(), 2);

        // Duplicate tracking is idempotent
        tracker.track(StreamType::LeaderHints);
        assert_eq!(tracker.active_streams().len(), 2);
    }
}
