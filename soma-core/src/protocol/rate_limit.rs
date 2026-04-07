//! Rate limiting with graduated response (Spec Section 20).
//!
//! Tracks per-second signal count and byte throughput. When limits are
//! exceeded the limiter returns a suggested `retry_after` interval and
//! escalates through warning / sustained / severe violation levels.

use std::collections::HashMap;
use std::time::Instant;

use super::signal::{Signal, SignalType};

/// Graduated violation levels for rate-limit enforcement.
// CONTROL signal emission is implemented via `create_rate_limit_signal()`
// (Section 20.3). Peer blacklisting after sustained violations is
// handled by `PeerBlacklist` (Section 20.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationLevel {
    /// No active violation.
    None,
    /// First limit hit — warn the peer.
    Warning,
    /// Continuous violation for >10 seconds.
    Sustained,
    /// Continuous violation for >60 seconds — connection should be closed.
    Severe,
}

/// Per-connection rate limiter.
pub struct RateLimiter {
    signals_per_second: u32,
    bytes_per_second: u64,
    /// Original signals_per_second for restoration after `reduce_window`.
    original_signals_per_second: u32,
    /// Original bytes_per_second for restoration after `reduce_window`.
    original_bytes_per_second: u64,
    max_channels: u32,
    max_subscriptions: u32,

    // Current-second tracking
    signal_count_this_second: u32,
    bytes_this_second: u64,
    second_start: Instant,

    // Graduated response
    violation_start: Option<Instant>,
}

impl RateLimiter {
    /// Create a new rate limiter with the given per-second limits.
    ///
    /// Defaults: `max_channels` = 256, `max_subscriptions` = 100 (Spec Section 20.2).
    pub fn new(signals_per_second: u32, bytes_per_second: u64) -> Self {
        Self {
            signals_per_second,
            bytes_per_second,
            original_signals_per_second: signals_per_second,
            original_bytes_per_second: bytes_per_second,
            max_channels: 256,
            max_subscriptions: 100,
            signal_count_this_second: 0,
            bytes_this_second: 0,
            second_start: Instant::now(),
            violation_start: None,
        }
    }

    /// Create a rate limiter with custom channel/subscription caps.
    pub fn with_caps(
        signals_per_second: u32,
        bytes_per_second: u64,
        max_channels: u32,
        max_subscriptions: u32,
    ) -> Self {
        Self {
            max_channels,
            max_subscriptions,
            ..Self::new(signals_per_second, bytes_per_second)
        }
    }

    /// Check if a signal of `signal_bytes` size should be allowed.
    ///
    /// Returns `None` if the signal is within limits, or
    /// `Some(retry_after_ms)` if the peer should back off.
    pub fn check(&mut self, signal_bytes: usize) -> Option<u32> {
        let now = Instant::now();

        // Reset counters if a full second has elapsed
        if now.duration_since(self.second_start).as_millis() >= 1000 {
            self.signal_count_this_second = 0;
            self.bytes_this_second = 0;
            self.second_start = now;
        }

        // Check signals/sec
        if self.signal_count_this_second >= self.signals_per_second {
            self.start_violation(now);
            // Suggest retrying after the current second window resets
            let elapsed_in_second = now.duration_since(self.second_start).as_millis() as u32;
            let retry_after = 1000u32.saturating_sub(elapsed_in_second).max(10);
            return Some(retry_after);
        }

        // Check bytes/sec
        if self.bytes_this_second + signal_bytes as u64 > self.bytes_per_second {
            self.start_violation(now);
            let elapsed_in_second = now.duration_since(self.second_start).as_millis() as u32;
            let retry_after = 1000u32.saturating_sub(elapsed_in_second).max(10);
            return Some(retry_after);
        }

        // Signal is allowed — record it and clear any violation
        self.signal_count_this_second += 1;
        self.bytes_this_second += signal_bytes as u64;
        self.violation_start = None;
        None
    }

    /// Return the current graduated violation level.
    pub fn violation_level(&self) -> ViolationLevel {
        match self.violation_start {
            None => ViolationLevel::None,
            Some(start) => {
                let duration = start.elapsed();
                if duration.as_secs() >= 60 {
                    ViolationLevel::Severe
                } else if duration.as_secs() >= 10 {
                    ViolationLevel::Sustained
                } else {
                    ViolationLevel::Warning
                }
            }
        }
    }

    /// Check whether opening another channel would exceed the limit.
    ///
    /// Returns `true` if `current_channels < max_channels` (i.e. allowed),
    /// `false` if the limit would be exceeded.
    pub fn check_channel_limit(&self, current_channels: usize) -> bool {
        current_channels < self.max_channels as usize
    }

    /// Check whether adding another subscription would exceed the limit.
    ///
    /// Returns `true` if `current_subs < max_subscriptions` (i.e. allowed),
    /// `false` if the limit would be exceeded.
    pub fn check_subscription_limit(&self, current_subs: usize) -> bool {
        current_subs < self.max_subscriptions as usize
    }

    /// Halve the throughput windows as a graduated response to sustained
    /// violations (Spec Section 20). The original values are preserved so
    /// they can be restored later via `restore_window()`.
    pub fn reduce_window(&mut self) {
        self.signals_per_second = (self.signals_per_second / 2).max(1);
        self.bytes_per_second = (self.bytes_per_second / 2).max(1);
    }

    /// Restore throughput windows to their original values after a
    /// previous `reduce_window()`.
    pub fn restore_window(&mut self) {
        self.signals_per_second = self.original_signals_per_second;
        self.bytes_per_second = self.original_bytes_per_second;
    }

    /// Reset all counters and violation state.
    pub fn reset(&mut self) {
        self.signal_count_this_second = 0;
        self.bytes_this_second = 0;
        self.second_start = Instant::now();
        self.violation_start = None;
    }

    fn start_violation(&mut self, now: Instant) {
        if self.violation_start.is_none() {
            self.violation_start = Some(now);
        }
    }
}

/// Create a CONTROL signal for rate limiting (Section 20.3).
pub fn create_rate_limit_signal(sender: &str, retry_after_ms: u32) -> Signal {
    let mut signal = Signal::new(SignalType::Control, sender.to_string());
    signal.metadata = serde_json::json!({
        "control_type": "rate_limit",
        "retry_after_ms": retry_after_ms,
    });
    signal
}

/// Tracks blacklisted peers after sustained rate limit violations (Section 20.4).
pub struct PeerBlacklist {
    /// peer_addr → blacklist_until
    entries: HashMap<String, Instant>,
    /// Default blacklist duration
    blacklist_duration: std::time::Duration,
}

impl PeerBlacklist {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            blacklist_duration: std::time::Duration::from_secs(300), // 5 minutes
        }
    }

    /// Blacklist a peer for the configured duration.
    pub fn blacklist(&mut self, peer_addr: String) {
        let until = Instant::now() + self.blacklist_duration;
        tracing::warn!(peer = %peer_addr, duration_secs = 300, "Peer blacklisted");
        self.entries.insert(peer_addr, until);
    }

    /// Check if a peer is currently blacklisted.
    pub fn is_blacklisted(&self, peer_addr: &str) -> bool {
        self.entries.get(peer_addr)
            .map(|until| Instant::now() < *until)
            .unwrap_or(false)
    }

    /// Remove expired blacklist entries.
    pub fn cleanup(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, until| now < *until);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_within_limits() {
        let mut rl = RateLimiter::new(100, 1_000_000);
        for _ in 0..100 {
            assert!(rl.check(100).is_none());
        }
    }

    #[test]
    fn test_signal_count_exceeded() {
        let mut rl = RateLimiter::new(5, 1_000_000);
        for _ in 0..5 {
            assert!(rl.check(10).is_none());
        }
        // 6th signal should be rate-limited
        let retry = rl.check(10);
        assert!(retry.is_some());
        assert!(retry.unwrap() > 0);
    }

    #[test]
    fn test_bytes_exceeded() {
        let mut rl = RateLimiter::new(1000, 500);
        assert!(rl.check(400).is_none());
        // This would push us over 500 bytes
        let retry = rl.check(200);
        assert!(retry.is_some());
    }

    #[test]
    fn test_violation_level_none() {
        let rl = RateLimiter::new(100, 1_000_000);
        assert_eq!(rl.violation_level(), ViolationLevel::None);
    }

    #[test]
    fn test_violation_level_warning() {
        let mut rl = RateLimiter::new(1, 1_000_000);
        rl.check(10); // allowed
        rl.check(10); // rate-limited, starts violation
        assert_eq!(rl.violation_level(), ViolationLevel::Warning);
    }

    #[test]
    fn test_reset() {
        let mut rl = RateLimiter::new(1, 100);
        rl.check(10);
        rl.check(10); // triggers violation
        assert_ne!(rl.violation_level(), ViolationLevel::None);

        rl.reset();
        assert_eq!(rl.violation_level(), ViolationLevel::None);
        assert!(rl.check(10).is_none()); // should be allowed again
    }

    #[test]
    fn test_default_max_subscriptions() {
        let rl = RateLimiter::new(100, 1_000_000);
        // Spec Section 20.2: default max_subscriptions = 100
        assert!(rl.check_subscription_limit(99));
        assert!(!rl.check_subscription_limit(100));
        assert!(!rl.check_subscription_limit(200));
    }

    #[test]
    fn test_check_channel_limit() {
        let rl = RateLimiter::new(100, 1_000_000);
        // Default max_channels = 256
        assert!(rl.check_channel_limit(0));
        assert!(rl.check_channel_limit(255));
        assert!(!rl.check_channel_limit(256));
        assert!(!rl.check_channel_limit(500));
    }

    #[test]
    fn test_check_subscription_limit_with_caps() {
        let rl = RateLimiter::with_caps(100, 1_000_000, 64, 50);
        assert!(rl.check_subscription_limit(49));
        assert!(!rl.check_subscription_limit(50));
        assert!(rl.check_channel_limit(63));
        assert!(!rl.check_channel_limit(64));
    }

    #[test]
    fn test_reduce_and_restore_window() {
        let mut rl = RateLimiter::new(100, 10_000);

        // Reduce halves the limits
        rl.reduce_window();
        // After reduction, only 50 signals should be allowed
        for _ in 0..50 {
            assert!(rl.check(1).is_none());
        }
        assert!(rl.check(1).is_some()); // 51st should be rate-limited

        // Restore brings limits back
        rl.restore_window();
        rl.reset();
        for _ in 0..100 {
            assert!(rl.check(1).is_none());
        }
        assert!(rl.check(1).is_some()); // 101st should be rate-limited
    }

    #[test]
    fn test_reduce_window_floor_at_one() {
        let mut rl = RateLimiter::new(1, 1);
        rl.reduce_window();
        // Should floor at 1, not go to 0
        assert!(rl.check(1).is_none()); // 1 signal of 1 byte allowed
    }
}
