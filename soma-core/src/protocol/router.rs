//! SignalRouter — centralized signal routing with request-response correlation
//! (Whitepaper Section 14.2).

use dashmap::DashMap;
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};

use super::signal::Signal;

/// Default timeout for request-response correlation.
const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum number of inflight pending requests.
const MAX_INFLIGHT: usize = 1000;

/// Centralized router for correlating outgoing requests with incoming responses.
/// When a SOMA sends an Intent to a peer and expects a Result, it stores
/// a one-shot channel keyed by sequence number (Section 14.3).
pub struct SignalRouter {
    /// Pending request-response correlations: sequence_id -> response sender
    pending_requests: DashMap<u32, oneshot::Sender<Signal>>,
    /// Response timeout
    response_timeout: Duration,
}

impl SignalRouter {
    pub fn new() -> Self {
        Self {
            pending_requests: DashMap::new(),
            response_timeout: DEFAULT_RESPONSE_TIMEOUT,
        }
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            pending_requests: DashMap::new(),
            response_timeout: timeout,
        }
    }

    /// Register a pending request. Returns a receiver that will get the response.
    /// Returns an error if the maximum number of inflight requests is reached.
    pub fn register_pending(&self, sequence_id: u32) -> Result<oneshot::Receiver<Signal>, RouterError> {
        if self.pending_requests.len() >= MAX_INFLIGHT {
            return Err(RouterError::MaxInflight(MAX_INFLIGHT));
        }
        let (tx, rx) = oneshot::channel();
        self.pending_requests.insert(sequence_id, tx);
        Ok(rx)
    }

    /// Try to deliver a response to a pending request.
    /// Returns true if delivered, false if no pending request for this sequence.
    pub fn deliver_response(&self, sequence_id: u32, signal: Signal) -> bool {
        if let Some((_, tx)) = self.pending_requests.remove(&sequence_id) {
            tx.send(signal).is_ok()
        } else {
            false
        }
    }

    /// Send a request and wait for the correlated response with timeout.
    /// The caller must send the signal separately; this just handles correlation.
    pub async fn wait_for_response(&self, sequence_id: u32) -> Result<Signal, RouterError> {
        let rx = self.register_pending(sequence_id)?;
        match timeout(self.response_timeout, rx).await {
            Ok(Ok(signal)) => Ok(signal),
            Ok(Err(_)) => {
                self.pending_requests.remove(&sequence_id);
                Err(RouterError::ChannelClosed)
            }
            Err(_) => {
                self.pending_requests.remove(&sequence_id);
                Err(RouterError::Timeout(self.response_timeout))
            }
        }
    }

    /// Cancel a pending request.
    pub fn cancel(&self, sequence_id: u32) {
        self.pending_requests.remove(&sequence_id);
    }

    /// Number of pending requests.
    pub fn pending_count(&self) -> usize {
        self.pending_requests.len()
    }

    /// Clean up expired entries (called periodically).
    pub fn cleanup(&self) {
        // DashMap entries are cleaned up on deliver/cancel/timeout.
        // This is a no-op but provides a hook for future TTL-based cleanup.
    }

    /// Fail all pending requests (e.g., when a peer is declared dead).
    /// Removes every entry and drops the senders, causing all receivers
    /// to get a `RecvError`.
    pub fn fail_all(&self) {
        let keys: Vec<u32> = self.pending_requests.iter().map(|e| *e.key()).collect();
        let count = keys.len();
        for key in keys {
            self.pending_requests.remove(&key);
            // Sender is dropped here, causing the receiver to get RecvError
        }
        tracing::warn!(
            "Failed all pending requests ({} entries removed)",
            count
        );
    }
}

/// Errors from the signal router.
#[derive(Debug)]
pub enum RouterError {
    /// Response channel was closed (peer disconnected).
    ChannelClosed,
    /// Response timed out.
    Timeout(Duration),
    /// Maximum number of inflight requests reached.
    MaxInflight(usize),
}

impl std::fmt::Display for RouterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouterError::ChannelClosed => write!(f, "response channel closed"),
            RouterError::Timeout(d) => write!(f, "response timed out after {:?}", d),
            RouterError::MaxInflight(max) => write!(f, "max inflight requests reached ({})", max),
        }
    }
}

impl std::error::Error for RouterError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::signal::{Signal, SignalType};

    #[tokio::test]
    async fn test_register_and_deliver() {
        let router = SignalRouter::new();
        let rx = router.register_pending(42).unwrap();

        let response = Signal::new(SignalType::Result, "peer".to_string());
        assert!(router.deliver_response(42, response));

        let received = rx.await.unwrap();
        assert_eq!(received.signal_type, SignalType::Result);
    }

    #[tokio::test]
    async fn test_deliver_unknown_sequence() {
        let router = SignalRouter::new();
        let signal = Signal::new(SignalType::Result, "peer".to_string());
        assert!(!router.deliver_response(99, signal));
    }

    #[tokio::test]
    async fn test_timeout() {
        let router = SignalRouter::with_timeout(Duration::from_millis(50));
        let result = router.wait_for_response(42).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cancel() {
        let router = SignalRouter::new();
        let _rx = router.register_pending(42).unwrap();
        assert_eq!(router.pending_count(), 1);

        router.cancel(42);
        assert_eq!(router.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_fail_all() {
        let router = SignalRouter::new();
        let rx1 = router.register_pending(1).unwrap();
        let rx2 = router.register_pending(2).unwrap();
        let rx3 = router.register_pending(3).unwrap();
        assert_eq!(router.pending_count(), 3);

        router.fail_all();
        assert_eq!(router.pending_count(), 0);

        // All receivers should get RecvError since senders were dropped
        assert!(rx1.await.is_err());
        assert!(rx2.await.is_err());
        assert!(rx3.await.is_err());
    }

    #[tokio::test]
    async fn test_max_inflight() {
        let router = SignalRouter::new();
        // Fill up to MAX_INFLIGHT
        let mut receivers = Vec::new();
        for i in 0..1000 {
            receivers.push(router.register_pending(i).unwrap());
        }
        assert_eq!(router.pending_count(), 1000);

        // Next registration should fail
        let result = router.register_pending(1001);
        assert!(result.is_err());

        // After removing one, should succeed again
        router.cancel(0);
        assert!(router.register_pending(1001).is_ok());
    }
}
