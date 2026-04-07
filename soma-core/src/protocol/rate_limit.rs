//! Rate limiting with graduated response (Spec Section 20).
//!
//! Tracks per-second signal count and byte throughput. When limits are
//! exceeded the limiter returns a suggested `retry_after` interval and
//! escalates through warning / sustained / severe violation levels.

use std::time::Instant;

/// Graduated violation levels for rate-limit enforcement.
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
    #[allow(dead_code)]
    max_channels: u32,
    #[allow(dead_code)]
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
    pub fn new(signals_per_second: u32, bytes_per_second: u64) -> Self {
        Self {
            signals_per_second,
            bytes_per_second,
            max_channels: 256,
            max_subscriptions: 1024,
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
}
