//! Offline signal queue for store-and-forward during disconnects
//! (Spec Section 21.4).
//!
//! Signals are queued with a priority and a maximum age. On reconnect
//! the queue is drained in priority order, dropping expired entries.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::signal::Signal;

/// A signal waiting for delivery, annotated with priority and retry count.
#[allow(dead_code)] // Spec feature for offline signal queuing
pub struct QueuedSignal {
    pub signal: Signal,
    pub queued_at: Instant,
    pub priority: u8,
    pub retries_left: u8,
}

/// Bounded offline queue that orders by priority and drops expired or
/// lowest-priority signals when full.
///
/// `max_age` is a queue-level setting (per spec): signals older than
/// `max_age` are considered expired and silently dropped on drain.
#[allow(dead_code)] // Spec feature for offline signal queuing
pub struct OfflineQueue {
    signals: VecDeque<QueuedSignal>,
    max_size: usize,
    /// Maximum age for any queued signal before it is considered expired.
    max_age: Duration,
}

#[allow(dead_code)] // Spec feature for offline signal queuing
impl OfflineQueue {
    pub fn new(max_size: usize) -> Self {
        Self {
            signals: VecDeque::with_capacity(max_size.min(1024)),
            max_size,
            max_age: Duration::from_secs(300), // 5 minute default
        }
    }

    /// Create a queue with a custom maximum signal age.
    pub fn with_max_age(max_size: usize, max_age: Duration) -> Self {
        Self {
            signals: VecDeque::with_capacity(max_size.min(1024)),
            max_size,
            max_age,
        }
    }

    /// Queue a signal for later delivery.
    ///
    /// If the queue is full, the lowest-priority signal is dropped to make
    /// room (unless the new signal itself has the lowest priority, in which
    /// case it is silently discarded).
    pub fn enqueue(&mut self, signal: Signal, priority: u8) {
        if self.signals.len() >= self.max_size {
            // Find the index of the lowest-priority entry
            let min_idx = self
                .signals
                .iter()
                .enumerate()
                .min_by_key(|(_, qs)| qs.priority)
                .map(|(i, qs)| (i, qs.priority));

            if let Some((idx, min_priority)) = min_idx {
                if priority <= min_priority {
                    // The new signal is not higher priority than what we'd
                    // drop — discard it instead.
                    return;
                }
                self.signals.remove(idx);
            }
        }

        // Insert maintaining priority order (highest first).
        // Find the first position where the existing entry has lower priority.
        let pos = self
            .signals
            .iter()
            .position(|qs| qs.priority < priority)
            .unwrap_or(self.signals.len());

        self.signals.insert(
            pos,
            QueuedSignal {
                signal,
                queued_at: Instant::now(),
                priority,
                retries_left: 3,
            },
        );
    }

    /// Drain the queue on reconnect, returning signals in priority order
    /// (highest first). Expired signals are silently dropped.
    ///
    /// The caller should attempt delivery of each returned signal.
    /// For signals that fail delivery, call `requeue()` to put them back
    /// (with decremented `retries_left`).
    pub fn drain(&mut self) -> Vec<Signal> {
        let now = Instant::now();
        let mut result = Vec::with_capacity(self.signals.len());

        while let Some(qs) = self.signals.pop_front() {
            if now.duration_since(qs.queued_at) <= self.max_age {
                result.push(qs.signal);
            }
            // else: expired, skip
        }

        result
    }

    /// Re-queue a signal that failed delivery.
    ///
    /// Decrements `retries_left`. If no retries remain, the signal is
    /// silently discarded. Expired signals are also discarded.
    pub fn requeue(&mut self, signal: Signal, priority: u8, retries_left: u8) {
        if retries_left == 0 {
            // No retries remaining — discard.
            return;
        }

        // Check if re-enqueueing would exceed max_size; apply same
        // lowest-priority eviction as enqueue().
        if self.signals.len() >= self.max_size {
            let min_idx = self
                .signals
                .iter()
                .enumerate()
                .min_by_key(|(_, qs)| qs.priority)
                .map(|(i, qs)| (i, qs.priority));

            if let Some((idx, min_priority)) = min_idx {
                if priority <= min_priority {
                    return;
                }
                self.signals.remove(idx);
            }
        }

        let pos = self
            .signals
            .iter()
            .position(|qs| qs.priority < priority)
            .unwrap_or(self.signals.len());

        self.signals.insert(
            pos,
            QueuedSignal {
                signal,
                queued_at: Instant::now(),
                priority,
                retries_left: retries_left - 1,
            },
        );
    }

    /// Number of signals currently queued (including potentially expired ones).
    pub fn len(&self) -> usize {
        self.signals.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.signals.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::signal::{Signal, SignalType};

    fn make_signal(label: &str) -> Signal {
        let mut s = Signal::new(SignalType::Data, "test".to_string());
        s.payload = label.as_bytes().to_vec();
        s
    }

    #[test]
    fn test_enqueue_and_drain() {
        let mut q = OfflineQueue::new(10);
        q.enqueue(make_signal("a"), 1);
        q.enqueue(make_signal("b"), 5);
        q.enqueue(make_signal("c"), 3);

        assert_eq!(q.len(), 3);

        let drained = q.drain();
        assert_eq!(drained.len(), 3);
        // Highest priority first
        assert_eq!(drained[0].payload, b"b");
        assert_eq!(drained[1].payload, b"c");
        assert_eq!(drained[2].payload, b"a");
        assert!(q.is_empty());
    }

    #[test]
    fn test_overflow_drops_lowest_priority() {
        let mut q = OfflineQueue::new(2);
        q.enqueue(make_signal("low"), 1);
        q.enqueue(make_signal("mid"), 3);

        // Queue is full. Adding a higher-priority signal should drop "low".
        q.enqueue(make_signal("high"), 5);
        assert_eq!(q.len(), 2);

        let drained = q.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].payload, b"high");
        assert_eq!(drained[1].payload, b"mid");
    }

    #[test]
    fn test_overflow_discards_if_lowest() {
        let mut q = OfflineQueue::new(2);
        q.enqueue(make_signal("a"), 5);
        q.enqueue(make_signal("b"), 3);

        // Adding priority 1 should be discarded (lower than both existing)
        q.enqueue(make_signal("c"), 1);
        assert_eq!(q.len(), 2);

        let drained = q.drain();
        assert!(!drained.iter().any(|s| s.payload == b"c"));
    }

    #[test]
    fn test_expired_signals_dropped_on_drain() {
        // Use queue-level max_age of 0ms so signals expire immediately
        let mut q = OfflineQueue::with_max_age(10, Duration::from_millis(0));
        q.enqueue(make_signal("expired"), 5);
        // Sleep a tiny bit to ensure expiry
        std::thread::sleep(Duration::from_millis(2));

        let drained = q.drain();
        assert_eq!(drained.len(), 0);

        // A queue with generous max_age keeps signals
        let mut q2 = OfflineQueue::with_max_age(10, Duration::from_secs(60));
        q2.enqueue(make_signal("fresh"), 3);
        let drained2 = q2.drain();
        assert_eq!(drained2.len(), 1);
        assert_eq!(drained2[0].payload, b"fresh");
    }

    #[test]
    fn test_empty_queue() {
        let mut q = OfflineQueue::new(10);
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
        let drained = q.drain();
        assert!(drained.is_empty());
    }

    #[test]
    fn test_requeue_decrements_retries() {
        let mut q = OfflineQueue::new(10);
        q.enqueue(make_signal("a"), 5);

        // Drain and simulate failed delivery
        let drained = q.drain();
        assert_eq!(drained.len(), 1);
        assert!(q.is_empty());

        // Requeue with 2 retries left
        q.requeue(drained[0].clone(), 5, 2);
        assert_eq!(q.len(), 1);

        // Drain again — should still be there
        let drained2 = q.drain();
        assert_eq!(drained2.len(), 1);

        // Requeue with 1 retry left
        q.requeue(drained2[0].clone(), 5, 1);
        assert_eq!(q.len(), 1);

        // Drain and requeue with 0 retries — should be discarded
        let drained3 = q.drain();
        assert_eq!(drained3.len(), 1);
        q.requeue(drained3[0].clone(), 5, 0);
        assert!(q.is_empty());
    }

    #[test]
    fn test_with_max_age_constructor() {
        let q = OfflineQueue::with_max_age(50, Duration::from_secs(120));
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }
}
