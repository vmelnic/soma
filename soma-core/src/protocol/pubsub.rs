//! Pub/Sub fan-out for topic-based signal distribution (Spec Section 16).
//!
//! Supports wildcard topic matching (`chat:*` matches `chat:room-1`),
//! durable subscriptions with catch-up replay, and per-topic buffering.

use std::collections::{HashMap, VecDeque};

use super::signal::{Signal, SignalType};

/// A single subscription entry.
pub struct Subscription {
    pub topic: String,
    pub channel_id: u32,
    pub connection_id: u64,
    pub last_seen_sequence: u32,
    pub durable: bool,
}

/// Ring buffer of recent signals for a topic, used for durable subscription
/// catch-up on reconnect.
pub struct TopicBuffer {
    pub signals: VecDeque<(u32, Vec<u8>)>,
    pub max_size: usize,
}

impl TopicBuffer {
    fn new(max_size: usize) -> Self {
        Self {
            signals: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    fn push(&mut self, sequence: u32, payload: Vec<u8>) {
        if self.signals.len() >= self.max_size {
            self.signals.pop_front();
        }
        self.signals.push_back((sequence, payload));
    }
}

/// Manages pub/sub subscriptions, topic buffers, and fan-out delivery.
pub struct PubSubManager {
    subscriptions: HashMap<String, Vec<Subscription>>,
    topic_buffers: HashMap<String, TopicBuffer>,
    next_sequence: HashMap<String, u32>,
    /// Default max buffer size for new topics.
    default_buffer_size: usize,
}

impl PubSubManager {
    pub fn new() -> Self {
        Self {
            subscriptions: HashMap::new(),
            topic_buffers: HashMap::new(),
            next_sequence: HashMap::new(),
            default_buffer_size: 1000,
        }
    }

    /// Handle a SUBSCRIBE signal. If `last_seen_sequence` is provided the
    /// subscriber will receive catch-up signals from the topic buffer.
    pub fn subscribe(
        &mut self,
        topic: &str,
        channel_id: u32,
        connection_id: u64,
        last_seen_sequence: Option<u32>,
        durable: bool,
    ) {
        let sub = Subscription {
            topic: topic.to_string(),
            channel_id,
            connection_id,
            last_seen_sequence: last_seen_sequence.unwrap_or(0),
            durable,
        };

        self.subscriptions
            .entry(topic.to_string())
            .or_insert_with(Vec::new)
            .push(sub);

        // Ensure a buffer exists for the topic
        self.topic_buffers
            .entry(topic.to_string())
            .or_insert_with(|| TopicBuffer::new(self.default_buffer_size));
    }

    /// Handle an UNSUBSCRIBE signal: remove all subscriptions for a
    /// connection on the given topic.
    pub fn unsubscribe(&mut self, topic: &str, connection_id: u64) {
        if let Some(subs) = self.subscriptions.get_mut(topic) {
            subs.retain(|s| s.connection_id != connection_id);
            if subs.is_empty() {
                self.subscriptions.remove(topic);
            }
        }
    }

    /// Publish a payload to all subscribers whose topic pattern matches.
    ///
    /// Returns a list of `(connection_id, Signal)` pairs — the caller is
    /// responsible for routing each signal to the correct connection.
    pub fn publish(
        &mut self,
        topic: &str,
        payload: Vec<u8>,
        channel_id: u32,
    ) -> Vec<(u64, Signal)> {
        // Assign the next sequence number for this topic
        let seq = {
            let entry = self.next_sequence.entry(topic.to_string()).or_insert(0);
            let s = *entry;
            *entry = s + 1;
            s
        };

        // Buffer the signal for durable catch-up
        self.topic_buffers
            .entry(topic.to_string())
            .or_insert_with(|| TopicBuffer::new(self.default_buffer_size))
            .push(seq, payload.clone());

        // Fan-out to matching subscribers
        let mut results: Vec<(u64, Signal)> = Vec::new();

        // Collect matching topic keys first to avoid borrow issues
        let matching_topics: Vec<String> = self
            .subscriptions
            .keys()
            .filter(|pattern| Self::topic_matches(pattern, topic))
            .cloned()
            .collect();

        for pattern in &matching_topics {
            if let Some(subs) = self.subscriptions.get_mut(pattern) {
                for sub in subs.iter_mut() {
                    let mut signal = Signal::new(SignalType::StreamData, String::new());
                    signal.channel_id = if sub.channel_id != 0 {
                        sub.channel_id
                    } else {
                        channel_id
                    };
                    signal.sequence = seq;
                    signal.payload = payload.clone();
                    signal.metadata = serde_json::json!({ "topic": topic });

                    sub.last_seen_sequence = seq;
                    results.push((sub.connection_id, signal));
                }
            }
        }

        results
    }

    /// Get catch-up signals for a subscriber that reconnected and wants
    /// signals after `last_seen_sequence`.
    pub fn catch_up(&self, topic: &str, last_seen_sequence: u32) -> Vec<Signal> {
        let mut signals = Vec::new();

        if let Some(buffer) = self.topic_buffers.get(topic) {
            for (seq, payload) in &buffer.signals {
                if *seq > last_seen_sequence {
                    let mut signal = Signal::new(SignalType::StreamData, String::new());
                    signal.sequence = *seq;
                    signal.payload = payload.clone();
                    signal.metadata = serde_json::json!({ "topic": topic });
                    signals.push(signal);
                }
            }
        }

        signals
    }

    /// Remove all subscriptions for a given connection (e.g., on disconnect).
    /// Durable subscriptions are kept so the subscriber can catch up later.
    pub fn remove_connection(&mut self, connection_id: u64) {
        for subs in self.subscriptions.values_mut() {
            subs.retain(|s| s.durable || s.connection_id != connection_id);
            // For durable subs that match, just mark them by keeping them
            // (they still belong to connection_id but the connection is gone;
            // on reconnect the subscriber will re-subscribe and catch up).
        }
        // Clean up empty topic entries
        self.subscriptions.retain(|_, subs| !subs.is_empty());
    }

    /// Match a topic against a subscription pattern. Supports `:*` suffix
    /// as a wildcard (e.g. `chat:*` matches `chat:room-1`).
    fn topic_matches(pattern: &str, topic: &str) -> bool {
        if pattern.ends_with(":*") {
            let prefix = &pattern[..pattern.len() - 2];
            topic == prefix || topic.starts_with(&format!("{}:", prefix))
        } else {
            pattern == topic
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscribe_publish() {
        let mut mgr = PubSubManager::new();
        mgr.subscribe("chat:room-1", 100, 1, None, false);
        mgr.subscribe("chat:room-1", 101, 2, None, false);

        let results = mgr.publish("chat:room-1", b"hello".to_vec(), 100);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 1); // connection_id
        assert_eq!(results[1].0, 2);
        assert_eq!(results[0].1.payload, b"hello");
    }

    #[test]
    fn test_wildcard_subscription() {
        let mut mgr = PubSubManager::new();
        mgr.subscribe("chat:*", 100, 1, None, false);

        let results = mgr.publish("chat:room-5", b"msg".to_vec(), 100);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 1);
    }

    #[test]
    fn test_wildcard_no_false_match() {
        let mut mgr = PubSubManager::new();
        mgr.subscribe("chat:*", 100, 1, None, false);

        let results = mgr.publish("events:new", b"msg".to_vec(), 100);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_unsubscribe() {
        let mut mgr = PubSubManager::new();
        mgr.subscribe("topic-a", 100, 1, None, false);
        mgr.subscribe("topic-a", 101, 2, None, false);

        mgr.unsubscribe("topic-a", 1);
        let results = mgr.publish("topic-a", b"data".to_vec(), 100);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 2);
    }

    #[test]
    fn test_catch_up() {
        let mut mgr = PubSubManager::new();
        mgr.subscribe("news", 100, 1, None, true);

        mgr.publish("news", b"a".to_vec(), 100);
        mgr.publish("news", b"b".to_vec(), 100);
        mgr.publish("news", b"c".to_vec(), 100);

        // Subscriber missed signals after seq 0
        let catchup = mgr.catch_up("news", 0);
        assert_eq!(catchup.len(), 2); // seqs 1, 2
        assert_eq!(catchup[0].payload, b"b");
        assert_eq!(catchup[1].payload, b"c");
    }

    #[test]
    fn test_remove_connection_non_durable() {
        let mut mgr = PubSubManager::new();
        mgr.subscribe("topic", 100, 1, None, false);
        mgr.subscribe("topic", 101, 2, None, false);

        mgr.remove_connection(1);
        let results = mgr.publish("topic", b"x".to_vec(), 100);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 2);
    }

    #[test]
    fn test_remove_connection_durable_kept() {
        let mut mgr = PubSubManager::new();
        mgr.subscribe("topic", 100, 1, None, true);

        mgr.remove_connection(1);
        // Durable sub is still there
        let results = mgr.publish("topic", b"x".to_vec(), 100);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_topic_matches() {
        assert!(PubSubManager::topic_matches("chat:*", "chat:room-1"));
        assert!(PubSubManager::topic_matches("chat:*", "chat:room-2"));
        assert!(!PubSubManager::topic_matches("chat:*", "events:new"));
        assert!(PubSubManager::topic_matches("exact", "exact"));
        assert!(!PubSubManager::topic_matches("exact", "other"));
        // "chat:*" should not match bare "chat" without colon
        assert!(!PubSubManager::topic_matches("chat:*", "chatroom"));
    }
}
