//! Per-connection rate limiting with graduated response (Spec Section 20).
//!
//! Enforces two independent per-second limits -- signal count and byte
//! throughput. When either limit is exceeded, the limiter returns a
//! `retry_after` interval and begins escalating through graduated
//! violation levels:
//!
//! | Duration   | Level       | Action                           |
//! |------------|-------------|----------------------------------|
//! | < 10 s     | `Warning`   | Send CONTROL signal to peer      |
//! | 10 -- 60 s | `Sustained` | Halve throughput windows         |
//! | > 60 s     | `Severe`    | Close connection and blacklist   |
//!
//! The [`PeerBlacklist`] tracks peers that have been banned after severe
//! violations, with automatic expiry after a configurable duration.

use std::collections::HashMap;
use std::time::Instant;

use super::signal::{Signal, SignalType};

/// Graduated violation levels for rate-limit enforcement (Spec Section 20.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationLevel {
    /// No active violation.
    None,
    /// First limit hit (< 10 s continuous) -- warn the peer via CONTROL signal.
    Warning,
    /// Violation sustained for > 10 seconds -- throughput windows should be halved.
    Sustained,
    /// Violation sustained for > 60 seconds -- connection should be closed and peer blacklisted.
    Severe,
}

/// Per-connection rate limiter with sliding one-second windows.
///
/// Counters reset each second. When a limit is hit, `violation_start`
/// is set and the violation level escalates based on elapsed time until
/// a successful (within-limit) signal clears the violation state.
pub struct RateLimiter {
    /// Current signal-per-second limit (may be reduced by `reduce_window`).
    signals_per_second: u32,
    /// Current byte-per-second limit (may be reduced by `reduce_window`).
    bytes_per_second: u64,
    /// Snapshot of the original signal limit for `restore_window`.
    #[allow(dead_code)] // Used by reduce/restore_window
    original_signals_per_second: u32,
    /// Snapshot of the original byte limit for `restore_window`.
    #[allow(dead_code)] // Used by reduce/restore_window
    original_bytes_per_second: u64,
    #[allow(dead_code)] // Spec feature for channel limits
    max_channels: u32,
    #[allow(dead_code)] // Spec feature for subscription limits
    max_subscriptions: u32,

    signal_count_this_second: u32,
    bytes_this_second: u64,
    /// Start of the current one-second measurement window.
    second_start: Instant,

    /// When the current violation streak began (`None` if no active violation).
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
    #[allow(dead_code)] // Spec feature for custom rate limits
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

    /// Check whether a signal of `signal_bytes` should be admitted.
    ///
    /// Returns `None` if within limits. Returns `Some(retry_after_ms)` with
    /// the suggested backoff when either the signal count or byte budget for
    /// the current second is exhausted.
    pub fn check(&mut self, signal_bytes: usize) -> Option<u32> {
        let now = Instant::now();

        // Slide the window forward when a full second has elapsed
        if now.duration_since(self.second_start).as_millis() >= 1000 {
            self.signal_count_this_second = 0;
            self.bytes_this_second = 0;
            self.second_start = now;
        }

        if self.signal_count_this_second >= self.signals_per_second {
            self.start_violation(now);
            // retry_after = time remaining in the current one-second window
            #[allow(clippy::cast_possible_truncation)] // elapsed within 1s, fits in u32
            let elapsed_in_second = now.duration_since(self.second_start).as_millis() as u32;
            let retry_after = 1000u32.saturating_sub(elapsed_in_second).max(10);
            return Some(retry_after);
        }

        if self.bytes_this_second + signal_bytes as u64 > self.bytes_per_second {
            self.start_violation(now);
            #[allow(clippy::cast_possible_truncation)] // elapsed within 1s, fits in u32
            let elapsed_in_second = now.duration_since(self.second_start).as_millis() as u32;
            let retry_after = 1000u32.saturating_sub(elapsed_in_second).max(10);
            return Some(retry_after);
        }

        // Admitted -- record and clear any active violation streak
        self.signal_count_this_second += 1;
        self.bytes_this_second += signal_bytes as u64;
        self.violation_start = None;
        None
    }

    /// Current graduated violation level based on how long the violation has persisted.
    pub fn violation_level(&self) -> ViolationLevel {
        self.violation_start.map_or(ViolationLevel::None, |start| {
            let duration = start.elapsed();
            if duration.as_secs() >= 60 {
                ViolationLevel::Severe
            } else if duration.as_secs() >= 10 {
                ViolationLevel::Sustained
            } else {
                ViolationLevel::Warning
            }
        })
    }

    /// Returns `true` if opening another channel is within the limit.
    #[allow(dead_code)] // Spec feature for channel limits
    pub const fn check_channel_limit(&self, current_channels: usize) -> bool {
        current_channels < self.max_channels as usize
    }

    /// Returns `true` if adding another subscription is within the limit.
    #[allow(dead_code)] // Spec feature for subscription limits
    pub const fn check_subscription_limit(&self, current_subs: usize) -> bool {
        current_subs < self.max_subscriptions as usize
    }

    /// Halve both throughput limits as a graduated response to sustained violations.
    /// Floors at 1 to avoid zero-limit deadlock. Call [`restore_window`](Self::restore_window) to undo.
    #[allow(dead_code)] // Spec feature for graduated response
    pub fn reduce_window(&mut self) {
        self.signals_per_second = (self.signals_per_second / 2).max(1);
        self.bytes_per_second = (self.bytes_per_second / 2).max(1);
    }

    /// Restore throughput limits to the values set at construction time.
    #[allow(dead_code)] // Spec feature for graduated response
    pub const fn restore_window(&mut self) {
        self.signals_per_second = self.original_signals_per_second;
        self.bytes_per_second = self.original_bytes_per_second;
    }

    /// Clear all counters and violation state, starting a fresh measurement window.
    #[allow(dead_code)] // Spec feature for rate limit reset
    pub fn reset(&mut self) {
        self.signal_count_this_second = 0;
        self.bytes_this_second = 0;
        self.second_start = Instant::now();
        self.violation_start = None;
    }

    /// Record the start of a violation streak (idempotent within a streak).
    const fn start_violation(&mut self, now: Instant) {
        if self.violation_start.is_none() {
            self.violation_start = Some(now);
        }
    }
}

/// Build a CONTROL signal informing the peer to back off for `retry_after_ms` milliseconds.
pub fn create_rate_limit_signal(sender: &str, retry_after_ms: u32) -> Signal {
    let mut signal = Signal::new(SignalType::Control, sender.to_string());
    signal.metadata = serde_json::json!({
        "control_type": "rate_limit",
        "retry_after_ms": retry_after_ms,
    });
    signal
}

/// Time-limited ban list for peers after severe rate-limit violations (Spec Section 20.4).
///
/// Entries expire automatically after `blacklist_duration` (default 5 minutes).
/// Call [`cleanup`](Self::cleanup) periodically to reclaim memory from expired entries.
#[allow(dead_code)] // Spec feature for peer blacklisting
pub struct PeerBlacklist {
    /// Maps peer address to the instant at which the ban expires.
    entries: HashMap<String, Instant>,
    blacklist_duration: std::time::Duration,
}

#[allow(dead_code)] // Spec feature for peer blacklisting
impl PeerBlacklist {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            blacklist_duration: std::time::Duration::from_secs(300), // 5 minutes
        }
    }

    /// Ban a peer for the configured duration.
    pub fn blacklist(&mut self, peer_addr: String) {
        let until = Instant::now() + self.blacklist_duration;
        tracing::warn!(peer = %peer_addr, duration_secs = 300, "Peer blacklisted");
        self.entries.insert(peer_addr, until);
    }

    /// Returns `true` if the peer is currently banned and the ban has not expired.
    pub fn is_blacklisted(&self, peer_addr: &str) -> bool {
        self.entries.get(peer_addr)
            .is_some_and(|until| Instant::now() < *until)
    }

    /// Evict expired entries to reclaim memory.
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
