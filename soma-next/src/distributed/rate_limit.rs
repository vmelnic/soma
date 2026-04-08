//! Per-peer rate limiting for the distributed transport layer.
//!
//! Uses a token bucket algorithm per peer with graduated response:
//!
//! | Excess count | Decision   | Effect                                   |
//! |-------------|------------|------------------------------------------|
//! | 1           | Throttle   | Suggest backoff (wait_ms)                |
//! | 2 -- N      | Deny       | Reject the request outright              |
//! | > threshold | Blacklisted| Peer is banned for a configurable period  |
//!
//! The [`RateLimiter`] tracks state per peer and the [`RateDecision`] enum
//! communicates the outcome to the transport layer.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tracing::warn;

/// Outcome of a rate-limit check for a single request.
#[derive(Debug, Clone, PartialEq)]
pub enum RateDecision {
    /// Request is within budget — proceed normally.
    Allow,
    /// Peer has exceeded the steady-state rate but not yet hit repeated violations.
    /// The transport should delay or ask the peer to back off for `wait_ms`.
    Throttle { wait_ms: u64 },
    /// Peer has repeatedly exceeded the rate — reject this request.
    Deny,
    /// Peer has been blacklisted due to persistent abuse.
    Blacklisted,
}

/// Per-peer token bucket state.
struct PeerRateState {
    /// Available tokens (fractional to support smooth refill).
    tokens: f64,
    /// Maximum tokens the bucket can hold (burst capacity).
    max_tokens: f64,
    /// Tokens added per second (steady-state rate).
    refill_rate: f64,
    /// Last time tokens were refilled.
    last_refill: Instant,
    /// Consecutive violation count (resets when a request is allowed).
    violation_count: u32,
}

impl PeerRateState {
    fn new(max_tokens: f64, refill_rate: f64) -> Self {
        Self {
            tokens: max_tokens,
            max_tokens,
            refill_rate,
            last_refill: Instant::now(),
            violation_count: 0,
        }
    }

    /// Refill tokens based on elapsed time since the last refill.
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = now;
    }

    /// Try to consume one token. Returns true if a token was available.
    fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            self.violation_count = 0;
            true
        } else {
            self.violation_count += 1;
            false
        }
    }

    /// Estimated milliseconds until the next token becomes available.
    fn wait_until_available_ms(&self) -> u64 {
        if self.tokens >= 1.0 {
            return 0;
        }
        let deficit = 1.0 - self.tokens;
        let wait_secs = deficit / self.refill_rate;
        (wait_secs * 1000.0).ceil() as u64
    }
}

/// Configuration for the rate limiter, loaded from `[distributed]` config.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum sustained requests per second per peer.
    pub max_requests_per_second: u32,
    /// Extra burst capacity above the steady-state rate.
    pub burst_limit: u32,
    /// Number of consecutive violations before a peer is blacklisted.
    pub blacklist_threshold: u32,
    /// How long a blacklisted peer stays banned.
    pub blacklist_duration: Duration,
    /// Master switch for rate limiting. When false, all requests are allowed.
    pub rate_limit_enabled: bool,
    /// Master switch for the blacklist. When false, peers are never banned
    /// even if they exceed the violation threshold.
    pub blacklist_enabled: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests_per_second: 100,
            burst_limit: 20,
            blacklist_threshold: 50,
            blacklist_duration: Duration::from_secs(300),
            rate_limit_enabled: true,
            blacklist_enabled: true,
        }
    }
}

/// Per-peer rate limiter using token buckets with graduated response.
///
/// Each peer gets an independent token bucket. The bucket refills at
/// `max_requests_per_second` tokens/sec and holds up to
/// `max_requests_per_second + burst_limit` tokens. When a peer exceeds
/// its budget, responses escalate from throttle to deny to blacklist.
pub struct RateLimiter {
    limits: HashMap<String, PeerRateState>,
    max_requests_per_second: u32,
    burst_limit: u32,
    blacklist_threshold: u32,
    blacklist: HashSet<String>,
    /// When each blacklisted peer's ban expires.
    blacklist_expiry: HashMap<String, Instant>,
    blacklist_duration: Duration,
    /// When false, `check()` always returns Allow without touching buckets.
    rate_limit_enabled: bool,
    /// When false, the blacklist check and blacklisting logic are skipped.
    blacklist_enabled: bool,
}

impl RateLimiter {
    /// Create a new rate limiter with the given configuration.
    pub fn new(config: &RateLimitConfig) -> Self {
        Self {
            limits: HashMap::new(),
            max_requests_per_second: config.max_requests_per_second,
            burst_limit: config.burst_limit,
            blacklist_threshold: config.blacklist_threshold,
            blacklist: HashSet::new(),
            blacklist_expiry: HashMap::new(),
            blacklist_duration: config.blacklist_duration,
            rate_limit_enabled: config.rate_limit_enabled,
            blacklist_enabled: config.blacklist_enabled,
        }
    }

    /// Check whether a request from `peer_id` should be allowed.
    ///
    /// Creates a new token bucket for the peer on first contact.
    /// Graduated response:
    /// - First violation: Throttle (return suggested wait time).
    /// - Repeated violations: Deny outright.
    /// - Persistent abuse (exceeding `blacklist_threshold`): Blacklist the peer.
    pub fn check(&mut self, peer_id: &str) -> RateDecision {
        // When rate limiting is disabled, unconditionally allow.
        if !self.rate_limit_enabled {
            return RateDecision::Allow;
        }

        // Check blacklist first (with expiry cleanup), unless blacklisting is disabled.
        if self.blacklist_enabled && self.is_blacklisted(peer_id) {
            return RateDecision::Blacklisted;
        }

        let max_tokens = self.max_requests_per_second as f64 + self.burst_limit as f64;
        let refill_rate = self.max_requests_per_second as f64;
        let blacklist_threshold = self.blacklist_threshold;
        let blacklist_duration = self.blacklist_duration;

        let state = self
            .limits
            .entry(peer_id.to_string())
            .or_insert_with(|| PeerRateState::new(max_tokens, refill_rate));

        if state.try_consume() {
            return RateDecision::Allow;
        }

        // Token bucket is empty — graduated response based on violation count.
        let violations = state.violation_count;

        if self.blacklist_enabled && violations >= blacklist_threshold {
            // Persistent abuse — blacklist the peer.
            let peer_id_owned = peer_id.to_string();
            warn!(
                peer = %peer_id,
                violations = violations,
                duration_secs = blacklist_duration.as_secs(),
                "peer blacklisted due to persistent rate-limit violations"
            );
            self.blacklist.insert(peer_id_owned.clone());
            self.blacklist_expiry
                .insert(peer_id_owned, Instant::now() + blacklist_duration);
            return RateDecision::Blacklisted;
        }

        if violations <= 1 {
            // First excess — throttle with suggested backoff.
            let wait_ms = state.wait_until_available_ms().max(10);
            RateDecision::Throttle { wait_ms }
        } else {
            // Repeated excess — deny outright.
            RateDecision::Deny
        }
    }

    /// Returns true if the peer is currently blacklisted and the ban has not expired.
    pub fn is_blacklisted(&mut self, peer_id: &str) -> bool {
        if !self.blacklist.contains(peer_id) {
            return false;
        }

        // Check expiry.
        if let Some(expiry) = self.blacklist_expiry.get(peer_id)
            && Instant::now() >= *expiry {
                // Ban has expired — remove from blacklist and allow.
                self.blacklist.remove(peer_id);
                self.blacklist_expiry.remove(peer_id);
                self.limits.remove(peer_id);
                return false;
            }

        true
    }

    /// Remove expired blacklist entries and stale peer state to reclaim memory.
    pub fn cleanup(&mut self) {
        let now = Instant::now();
        let expired: Vec<String> = self
            .blacklist_expiry
            .iter()
            .filter(|(_, expiry)| now >= **expiry)
            .map(|(peer, _)| peer.clone())
            .collect();

        for peer in expired {
            self.blacklist.remove(&peer);
            self.blacklist_expiry.remove(&peer);
            self.limits.remove(&peer);
        }
    }

    /// Manually remove a peer from the blacklist (e.g., administrative override).
    pub fn unblacklist(&mut self, peer_id: &str) {
        self.blacklist.remove(peer_id);
        self.blacklist_expiry.remove(peer_id);
        self.limits.remove(peer_id);
    }

    /// Number of peers currently being tracked.
    pub fn tracked_peer_count(&self) -> usize {
        self.limits.len()
    }

    /// Number of currently blacklisted peers.
    pub fn blacklisted_count(&self) -> usize {
        self.blacklist.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> RateLimitConfig {
        RateLimitConfig {
            max_requests_per_second: 10,
            burst_limit: 5,
            blacklist_threshold: 5,
            blacklist_duration: Duration::from_millis(100),
            rate_limit_enabled: true,
            blacklist_enabled: true,
        }
    }

    #[test]
    fn allow_within_budget() {
        let mut rl = RateLimiter::new(&test_config());
        // 10 rps + 5 burst = 15 tokens in the bucket initially
        for i in 0..15 {
            let decision = rl.check("peer-1");
            assert_eq!(decision, RateDecision::Allow, "request {} should be allowed", i);
        }
    }

    #[test]
    fn throttle_on_first_excess() {
        let mut rl = RateLimiter::new(&test_config());
        // Drain the bucket (15 tokens).
        for _ in 0..15 {
            assert_eq!(rl.check("peer-1"), RateDecision::Allow);
        }
        // Next request should be throttled (first violation).
        let decision = rl.check("peer-1");
        match decision {
            RateDecision::Throttle { wait_ms } => {
                assert!(wait_ms > 0, "wait_ms should be positive");
            }
            other => panic!("expected Throttle, got {:?}", other),
        }
    }

    #[test]
    fn deny_on_repeated_excess() {
        let mut rl = RateLimiter::new(&test_config());
        // Drain the bucket.
        for _ in 0..15 {
            rl.check("peer-1");
        }
        // First excess -> Throttle.
        let d1 = rl.check("peer-1");
        assert!(matches!(d1, RateDecision::Throttle { .. }));
        // Second excess -> Deny (violation_count is now 2).
        let d2 = rl.check("peer-1");
        assert_eq!(d2, RateDecision::Deny);
        // Third excess -> still Deny.
        let d3 = rl.check("peer-1");
        assert_eq!(d3, RateDecision::Deny);
    }

    #[test]
    fn blacklist_on_persistent_abuse() {
        let mut rl = RateLimiter::new(&test_config());
        // Drain the bucket (15 tokens).
        for _ in 0..15 {
            rl.check("peer-1");
        }
        // Keep hitting until blacklisted (threshold = 5 violations).
        // violation_count increments each failed try_consume.
        for _ in 0..4 {
            rl.check("peer-1");
        }
        // 5th violation should trigger blacklist.
        let decision = rl.check("peer-1");
        assert_eq!(decision, RateDecision::Blacklisted);
        // Subsequent requests should also return Blacklisted.
        assert_eq!(rl.check("peer-1"), RateDecision::Blacklisted);
    }

    #[test]
    fn blacklist_expires() {
        let mut rl = RateLimiter::new(&test_config());
        // Force a blacklist.
        for _ in 0..15 {
            rl.check("peer-1");
        }
        for _ in 0..5 {
            rl.check("peer-1");
        }
        assert_eq!(rl.check("peer-1"), RateDecision::Blacklisted);

        // Wait for the blacklist to expire (100ms in test config).
        std::thread::sleep(Duration::from_millis(150));

        // After expiry, the peer should be allowed again.
        let decision = rl.check("peer-1");
        assert_eq!(decision, RateDecision::Allow);
    }

    #[test]
    fn independent_peer_tracking() {
        let mut rl = RateLimiter::new(&test_config());
        // Drain peer-1's bucket.
        for _ in 0..15 {
            rl.check("peer-1");
        }
        assert!(matches!(rl.check("peer-1"), RateDecision::Throttle { .. }));

        // peer-2 should be unaffected.
        assert_eq!(rl.check("peer-2"), RateDecision::Allow);
    }

    #[test]
    fn tokens_refill_over_time() {
        let mut rl = RateLimiter::new(&test_config());
        // Drain the bucket.
        for _ in 0..15 {
            rl.check("peer-1");
        }
        assert!(matches!(rl.check("peer-1"), RateDecision::Throttle { .. }));

        // Wait long enough for at least one token to refill (10 rps = 1 per 100ms).
        std::thread::sleep(Duration::from_millis(150));

        // Should be allowed again.
        assert_eq!(rl.check("peer-1"), RateDecision::Allow);
    }

    #[test]
    fn violation_count_resets_on_success() {
        // Use a lenient config so we can drain without hitting the blacklist.
        let config = RateLimitConfig {
            max_requests_per_second: 10,
            burst_limit: 5,
            blacklist_threshold: 50, // high threshold so drain doesn't blacklist
            blacklist_duration: Duration::from_millis(100),
            rate_limit_enabled: true,
            blacklist_enabled: true,
        };
        let mut rl = RateLimiter::new(&config);

        // Drain the bucket (5 burst tokens).
        for _ in 0..5 {
            rl.check("peer-1");
        }
        // Two more exceed the bucket — triggers violations.
        rl.check("peer-1"); // violation_count = 1
        rl.check("peer-1"); // violation_count = 2

        // Wait for refill.
        std::thread::sleep(Duration::from_millis(200));

        // Successful request resets violation count.
        assert_eq!(rl.check("peer-1"), RateDecision::Allow);

        // Drain all available tokens (burst refilled partially in 200ms).
        loop {
            let d = rl.check("peer-1");
            if d != RateDecision::Allow {
                // First non-Allow after reset should be Throttle, not Deny/Blacklisted.
                assert!(
                    matches!(d, RateDecision::Throttle { .. }),
                    "expected Throttle after reset, got {:?}",
                    d
                );
                break;
            }
        }
    }

    #[test]
    fn cleanup_removes_expired_bans() {
        let mut rl = RateLimiter::new(&test_config());
        // Blacklist peer-1.
        for _ in 0..20 {
            rl.check("peer-1");
        }
        assert_eq!(rl.blacklisted_count(), 1);

        // Wait for expiry.
        std::thread::sleep(Duration::from_millis(150));
        rl.cleanup();

        assert_eq!(rl.blacklisted_count(), 0);
        assert_eq!(rl.tracked_peer_count(), 0);
    }

    #[test]
    fn unblacklist_removes_ban_immediately() {
        let mut rl = RateLimiter::new(&test_config());
        // Blacklist peer-1.
        for _ in 0..20 {
            rl.check("peer-1");
        }
        assert!(rl.is_blacklisted("peer-1"));

        rl.unblacklist("peer-1");
        assert!(!rl.is_blacklisted("peer-1"));
        assert_eq!(rl.check("peer-1"), RateDecision::Allow);
    }

    #[test]
    fn default_config_values() {
        let config = RateLimitConfig::default();
        assert_eq!(config.max_requests_per_second, 100);
        assert_eq!(config.burst_limit, 20);
        assert_eq!(config.blacklist_threshold, 50);
        assert_eq!(config.blacklist_duration, Duration::from_secs(300));
    }
}
