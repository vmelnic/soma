//! Pub/Sub fan-out for topic-based signal distribution (Spec Section 16).
//!
//! Supports wildcard topic matching (`chat:*` matches `chat:room-1`),
//! durable subscriptions with catch-up replay, and per-topic buffering.

use std::collections::{HashMap, VecDeque};

use super::signal::{Signal, SignalType};

/// A single subscription entry binding a connection to a topic pattern.
pub struct Subscription {
    /// Topic pattern this subscription matches against (may contain `:*` wildcard).
    #[allow(dead_code)] // Stored for subscription management
    pub topic: String,
    /// Logical channel within the connection for multiplexing.
    pub channel_id: u32,
    /// Owning connection — used for fan-out routing and cleanup.
    pub connection_id: u64,
    /// Highest sequence number delivered to this subscriber (for catch-up).
    pub last_seen_sequence: u32,
    /// Durable subscriptions survive disconnects; non-durable are removed.
    pub durable: bool,
}

/// Ring buffer of recent signals for a topic, enabling durable subscribers
/// to catch up after reconnecting.
pub struct TopicBuffer {
    /// Buffered `(sequence_number, payload)` pairs, oldest at front.
    pub signals: VecDeque<(u32, Vec<u8>)>,
    /// When full, the oldest entry is evicted on push.
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
    /// Topic pattern -> list of active subscriptions (wildcard patterns are keys).
    subscriptions: HashMap<String, Vec<Subscription>>,
    /// Topic (exact) -> ring buffer of recent payloads for catch-up replay.
    topic_buffers: HashMap<String, TopicBuffer>,
    /// Topic (exact) -> monotonically increasing sequence counter.
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

    /// Register a subscription. When `last_seen_sequence` is provided, the
    /// caller should follow up with [`catch_up`](Self::catch_up) to replay
    /// buffered signals the subscriber missed.
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
            .or_default()
            .push(sub);

        self.topic_buffers
            .entry(topic.to_string())
            .or_insert_with(|| TopicBuffer::new(self.default_buffer_size));
    }

    /// Remove all subscriptions for `connection_id` on the given topic.
    /// Cleans up the topic entry entirely if no subscribers remain.
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
        payload: &[u8],
        channel_id: u32,
    ) -> Vec<(u64, Signal)> {
        let seq = {
            let entry = self.next_sequence.entry(topic.to_string()).or_insert(0);
            let s = *entry;
            *entry = s + 1;
            s
        };

        self.topic_buffers
            .entry(topic.to_string())
            .or_insert_with(|| TopicBuffer::new(self.default_buffer_size))
            .push(seq, payload.to_vec());

        let mut results: Vec<(u64, Signal)> = Vec::new();

        // Collect matching keys first to avoid borrowing `self` mutably and immutably.
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
                    signal.payload = payload.to_vec();
                    signal.metadata = serde_json::json!({ "topic": topic });

                    sub.last_seen_sequence = seq;
                    results.push((sub.connection_id, signal));
                }
            }
        }

        results
    }

    /// Replay buffered signals with sequence numbers strictly greater than
    /// `last_seen_sequence`. Used for durable subscriber catch-up on reconnect.
    pub fn catch_up(&self, topic: &str, last_seen_sequence: u32) -> Vec<Signal> {
        let mut signals = Vec::new();

        if let Some(buffer) = self.topic_buffers.get(topic) {
            for (seq, payload) in &buffer.signals {
                if *seq > last_seen_sequence {
                    let mut signal = Signal::new(SignalType::StreamData, String::new());
                    signal.sequence = *seq;
                    signal.payload.clone_from(payload);
                    signal.metadata = serde_json::json!({ "topic": topic });
                    signals.push(signal);
                }
            }
        }

        signals
    }

    /// Remove all non-durable subscriptions for a disconnected connection.
    /// Durable subscriptions are retained so the subscriber can catch up later.
    pub fn remove_connection(&mut self, connection_id: u64) {
        for subs in self.subscriptions.values_mut() {
            subs.retain(|s| s.durable || s.connection_id != connection_id);
        }
        self.subscriptions.retain(|_, subs| !subs.is_empty());
    }

    /// Match a topic against a subscription pattern.
    ///
    /// Wildcard rules: `chat:*` matches `chat` itself and any `chat:<suffix>`,
    /// but not unrelated prefixes like `chatroom`. Exact patterns require
    /// exact equality.
    fn topic_matches(pattern: &str, topic: &str) -> bool {
        pattern.strip_suffix(":*").map_or_else(
            || pattern == topic,
            |prefix| topic == prefix || topic.starts_with(&format!("{prefix}:")),
        )
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

        let results = mgr.publish("chat:room-1", b"hello", 100);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 1); // connection_id
        assert_eq!(results[1].0, 2);
        assert_eq!(results[0].1.payload, b"hello");
    }

    #[test]
    fn test_wildcard_subscription() {
        let mut mgr = PubSubManager::new();
        mgr.subscribe("chat:*", 100, 1, None, false);

        let results = mgr.publish("chat:room-5", b"msg", 100);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 1);
    }

    #[test]
    fn test_wildcard_no_false_match() {
        let mut mgr = PubSubManager::new();
        mgr.subscribe("chat:*", 100, 1, None, false);

        let results = mgr.publish("events:new", b"msg", 100);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_unsubscribe() {
        let mut mgr = PubSubManager::new();
        mgr.subscribe("topic-a", 100, 1, None, false);
        mgr.subscribe("topic-a", 101, 2, None, false);

        mgr.unsubscribe("topic-a", 1);
        let results = mgr.publish("topic-a", b"data", 100);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 2);
    }

    #[test]
    fn test_catch_up() {
        let mut mgr = PubSubManager::new();
        mgr.subscribe("news", 100, 1, None, true);

        mgr.publish("news", b"a", 100);
        mgr.publish("news", b"b", 100);
        mgr.publish("news", b"c", 100);

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
        let results = mgr.publish("topic", b"x", 100);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 2);
    }

    #[test]
    fn test_remove_connection_durable_kept() {
        let mut mgr = PubSubManager::new();
        mgr.subscribe("topic", 100, 1, None, true);

        mgr.remove_connection(1);
        // Durable sub is still there
        let results = mgr.publish("topic", b"x", 100);
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
