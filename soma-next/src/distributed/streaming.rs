use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::peer::{DistributedFailure, StreamedObservation};

use super::remote::RemoteExecutor;

// --- StreamId ---

/// Identifier for an observation stream.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StreamId(pub String);

impl StreamId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for StreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// --- StreamState ---

/// Tracks the state of an active observation stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamState {
    pub stream_id: StreamId,
    pub session_id: Uuid,
    pub source_peer: String,
    /// Last received sequence number for ordering.
    pub last_sequence: u64,
    /// Whether replay is supported for this stream.
    pub replay_supported: bool,
    /// Count of observations received.
    pub observation_count: u64,
    /// Count of detected missing observations.
    pub missing_count: u64,
    /// Count of detected duplicate observations.
    pub duplicate_count: u64,
    /// Count of detected out-of-order observations.
    pub out_of_order_count: u64,
}

// --- PartialDeliveryDetection ---

/// Result of checking an observation against stream state.
/// Detects missing, duplicate, out-of-order, and stale observations
/// per distributed.md partial delivery requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    /// Normal in-order delivery.
    InOrder,
    /// Observation arrived out of order.
    OutOfOrder,
    /// Duplicate observation detected.
    Duplicate,
    /// Gap detected — missing observations before this one.
    MissingPredecessors,
    /// Stale replay data.
    StaleReplay,
}

// --- ObservationStreaming trait ---

/// Observation streaming for distributed execution.
/// Streams MUST be ordered within a session/stream identifier.
/// Supports replay from known sequence point when policy allows.
/// Detects partial delivery: missing, duplicate, out-of-order, stale.
pub trait ObservationStreaming: Send + Sync {
    /// Open a new observation stream for a session.
    fn open_stream(
        &mut self,
        session_id: Uuid,
        source_peer: &str,
        replay_supported: bool,
    ) -> Result<StreamId>;

    /// Receive an observation on a stream. Returns delivery status.
    fn receive_observation(
        &mut self,
        stream_id: &StreamId,
        observation: &StreamedObservation,
    ) -> Result<DeliveryStatus>;

    /// Request replay from a known sequence point.
    /// Returns the stored observations with sequence >= from_sequence.
    /// Returns error if replay is not supported for this stream.
    fn request_replay(
        &self,
        stream_id: &StreamId,
        from_sequence: u64,
    ) -> Result<Vec<StreamedObservation>>;

    /// Close a stream.
    fn close_stream(&mut self, stream_id: &StreamId) -> Result<()>;

    /// Get the current state of a stream.
    fn stream_state(&self, stream_id: &StreamId) -> Option<&StreamState>;
}

// --- DefaultObservationStreaming ---

/// Default implementation that tracks stream state locally.
/// Stores received observations per stream for local replay. When a
/// remote executor is configured, replay requests that find no local
/// observations are forwarded to the peer that owns the stream.
pub struct DefaultObservationStreaming {
    streams: HashMap<String, StreamState>,
    /// Set of (stream_id, sequence) pairs for duplicate detection.
    seen_sequences: HashMap<String, Vec<u64>>,
    /// Stored observations per stream for local replay.
    observations: HashMap<String, Vec<StreamedObservation>>,
    next_stream_id: u64,
    /// Maximum allowed age for an observation before it is considered a stale replay.
    /// Observations whose timestamp is older than this threshold (in milliseconds)
    /// are rejected with `DeliveryStatus::StaleReplay`. Default: 60000 (60 seconds).
    pub max_replay_staleness_ms: u64,
    /// Optional remote executor for forwarding replay requests to peers
    /// when local observations are not available.
    remote: Option<Box<dyn RemoteExecutor>>,
}

impl DefaultObservationStreaming {
    pub fn new() -> Self {
        Self {
            streams: HashMap::new(),
            seen_sequences: HashMap::new(),
            observations: HashMap::new(),
            next_stream_id: 1,
            max_replay_staleness_ms: 60_000,
            remote: None,
        }
    }

    /// Construct with a remote executor for forwarding replay requests to peers.
    pub fn with_remote(remote: Box<dyn RemoteExecutor>) -> Self {
        Self {
            remote: Some(remote),
            ..Self::new()
        }
    }
}

impl Default for DefaultObservationStreaming {
    fn default() -> Self {
        Self::new()
    }
}

impl ObservationStreaming for DefaultObservationStreaming {
    fn open_stream(
        &mut self,
        session_id: Uuid,
        source_peer: &str,
        replay_supported: bool,
    ) -> Result<StreamId> {
        let id = format!("stream-{}", self.next_stream_id);
        self.next_stream_id += 1;
        let stream_id = StreamId::new(&id);
        self.streams.insert(
            id.clone(),
            StreamState {
                stream_id: stream_id.clone(),
                session_id,
                source_peer: source_peer.to_string(),
                last_sequence: 0,
                replay_supported,
                observation_count: 0,
                missing_count: 0,
                duplicate_count: 0,
                out_of_order_count: 0,
            },
        );
        self.seen_sequences.insert(id.clone(), Vec::new());
        self.observations.insert(id, Vec::new());
        Ok(stream_id)
    }

    fn receive_observation(
        &mut self,
        stream_id: &StreamId,
        observation: &StreamedObservation,
    ) -> Result<DeliveryStatus> {
        let state = self
            .streams
            .get_mut(&stream_id.0)
            .ok_or_else(|| SomaError::Distributed {
                failure: DistributedFailure::PartialObservationStream,
                details: format!("unknown stream: {}", stream_id),
            })?;

        let seen = self
            .seen_sequences
            .get_mut(&stream_id.0)
            .expect("seen_sequences must exist for open stream");

        let seq = observation.sequence;

        // Duplicate detection.
        if seen.contains(&seq) {
            state.duplicate_count += 1;
            return Ok(DeliveryStatus::Duplicate);
        }

        // Stale replay detection: reject observations whose timestamp is too old.
        let age_ms = chrono::Utc::now()
            .signed_duration_since(observation.timestamp)
            .num_milliseconds()
            .max(0) as u64;
        if age_ms > self.max_replay_staleness_ms {
            return Ok(DeliveryStatus::StaleReplay);
        }

        seen.push(seq);
        state.observation_count += 1;

        // Store the observation for potential replay.
        if let Some(obs_vec) = self.observations.get_mut(&stream_id.0) {
            obs_vec.push(observation.clone());
        }

        let status = if seq == state.last_sequence + 1 {
            // Perfect in-order delivery.
            state.last_sequence = seq;
            DeliveryStatus::InOrder
        } else if seq <= state.last_sequence {
            // Out-of-order: received an older sequence.
            state.out_of_order_count += 1;
            DeliveryStatus::OutOfOrder
        } else {
            // Gap: seq > last_sequence + 1, some observations are missing.
            let gap = seq - state.last_sequence - 1;
            state.missing_count += gap;
            state.last_sequence = seq;
            DeliveryStatus::MissingPredecessors
        };

        Ok(status)
    }

    fn request_replay(
        &self,
        stream_id: &StreamId,
        from_sequence: u64,
    ) -> Result<Vec<StreamedObservation>> {
        let state = self
            .streams
            .get(&stream_id.0)
            .ok_or_else(|| SomaError::Distributed {
                failure: DistributedFailure::PartialObservationStream,
                details: format!("unknown stream: {}", stream_id),
            })?;

        if !state.replay_supported {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::ReplayRejection,
                details: format!(
                    "replay not supported for stream {}",
                    stream_id
                ),
            });
        }

        // Return locally stored observations from the requested sequence onward.
        let local: Vec<StreamedObservation> = self
            .observations
            .get(&stream_id.0)
            .map(|obs| {
                obs.iter()
                    .filter(|o| o.sequence >= from_sequence)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        // If we have local observations, return them. Otherwise, try the
        // remote peer that owns this stream (if a remote executor is available).
        if !local.is_empty() {
            return Ok(local);
        }

        if let Some(ref remote) = self.remote {
            let peer = &state.source_peer;
            match remote.query_resource(peer, "observations", &stream_id.0) {
                Ok(response) => {
                    if let Some(arr) = response.data.as_array() {
                        let replayed: Vec<StreamedObservation> = arr
                            .iter()
                            .filter_map(|v| serde_json::from_value(v.clone()).ok())
                            .filter(|o: &StreamedObservation| o.sequence >= from_sequence)
                            .collect();
                        return Ok(replayed);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        stream_id = %stream_id,
                        peer = %peer,
                        error = %e,
                        "remote replay request failed, returning empty"
                    );
                }
            }
        }

        Ok(local)
    }

    fn close_stream(&mut self, stream_id: &StreamId) -> Result<()> {
        if self.streams.remove(&stream_id.0).is_none() {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::PartialObservationStream,
                details: format!("unknown stream: {}", stream_id),
            });
        }
        self.seen_sequences.remove(&stream_id.0);
        self.observations.remove(&stream_id.0);
        Ok(())
    }

    fn stream_state(&self, stream_id: &StreamId) -> Option<&StreamState> {
        self.streams.get(&stream_id.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_observation(session_id: Uuid, seq: u64) -> StreamedObservation {
        StreamedObservation {
            session_id,
            step_id: format!("step-{}", seq),
            source_peer: "peer-1".to_string(),
            skill_or_resource_ref: "file.list".to_string(),
            raw_result: serde_json::json!({"files": []}),
            structured_result: serde_json::json!({"count": 0}),
            effect_patch: None,
            success: true,
            latency_ms: 10,
            timestamp: Utc::now(),
            sequence: seq,
        }
    }

    #[test]
    fn open_and_close_stream() {
        let mut streaming = DefaultObservationStreaming::new();
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", true)
            .unwrap();
        assert!(streaming.stream_state(&stream_id).is_some());
        streaming.close_stream(&stream_id).unwrap();
        assert!(streaming.stream_state(&stream_id).is_none());
    }

    #[test]
    fn close_unknown_stream_fails() {
        let mut streaming = DefaultObservationStreaming::new();
        let bogus = StreamId::new("stream-999");
        assert!(streaming.close_stream(&bogus).is_err());
    }

    #[test]
    fn receive_in_order_observations() {
        let mut streaming = DefaultObservationStreaming::new();
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", true)
            .unwrap();

        let status = streaming
            .receive_observation(&stream_id, &make_observation(session_id, 1))
            .unwrap();
        assert_eq!(status, DeliveryStatus::InOrder);

        let status = streaming
            .receive_observation(&stream_id, &make_observation(session_id, 2))
            .unwrap();
        assert_eq!(status, DeliveryStatus::InOrder);

        let state = streaming.stream_state(&stream_id).unwrap();
        assert_eq!(state.observation_count, 2);
        assert_eq!(state.last_sequence, 2);
    }

    #[test]
    fn detect_duplicate_observation() {
        let mut streaming = DefaultObservationStreaming::new();
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", true)
            .unwrap();

        streaming
            .receive_observation(&stream_id, &make_observation(session_id, 1))
            .unwrap();
        let status = streaming
            .receive_observation(&stream_id, &make_observation(session_id, 1))
            .unwrap();
        assert_eq!(status, DeliveryStatus::Duplicate);

        let state = streaming.stream_state(&stream_id).unwrap();
        assert_eq!(state.duplicate_count, 1);
    }

    #[test]
    fn detect_missing_predecessors() {
        let mut streaming = DefaultObservationStreaming::new();
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", true)
            .unwrap();

        // Skip sequence 1, send sequence 3 directly.
        let status = streaming
            .receive_observation(&stream_id, &make_observation(session_id, 3))
            .unwrap();
        assert_eq!(status, DeliveryStatus::MissingPredecessors);

        let state = streaming.stream_state(&stream_id).unwrap();
        assert_eq!(state.missing_count, 2); // sequences 1 and 2 missing
    }

    #[test]
    fn detect_out_of_order_observation() {
        let mut streaming = DefaultObservationStreaming::new();
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", true)
            .unwrap();

        streaming
            .receive_observation(&stream_id, &make_observation(session_id, 1))
            .unwrap();
        streaming
            .receive_observation(&stream_id, &make_observation(session_id, 3))
            .unwrap();

        // Now receive sequence 2 (out of order).
        let status = streaming
            .receive_observation(&stream_id, &make_observation(session_id, 2))
            .unwrap();
        assert_eq!(status, DeliveryStatus::OutOfOrder);

        let state = streaming.stream_state(&stream_id).unwrap();
        assert_eq!(state.out_of_order_count, 1);
    }

    #[test]
    fn replay_rejected_when_not_supported() {
        let mut streaming = DefaultObservationStreaming::new();
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", false) // replay not supported
            .unwrap();

        let result = streaming.request_replay(&stream_id, 1);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::ReplayRejection);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn replay_returns_empty_when_no_observations() {
        let mut streaming = DefaultObservationStreaming::new();
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", true)
            .unwrap();

        let result = streaming.request_replay(&stream_id, 1).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn replay_returns_stored_observations_from_sequence() {
        let mut streaming = DefaultObservationStreaming::new();
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", true)
            .unwrap();

        // Receive observations 1, 2, 3.
        for seq in 1..=3 {
            streaming
                .receive_observation(&stream_id, &make_observation(session_id, seq))
                .unwrap();
        }

        // Replay from sequence 2 should return observations 2 and 3.
        let replayed = streaming.request_replay(&stream_id, 2).unwrap();
        assert_eq!(replayed.len(), 2);
        assert_eq!(replayed[0].sequence, 2);
        assert_eq!(replayed[1].sequence, 3);

        // Replay from sequence 1 should return all 3.
        let replayed_all = streaming.request_replay(&stream_id, 1).unwrap();
        assert_eq!(replayed_all.len(), 3);
    }

    #[test]
    fn replay_does_not_include_stale_observations() {
        let mut streaming = DefaultObservationStreaming::new();
        streaming.max_replay_staleness_ms = 5_000;
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", true)
            .unwrap();

        // Send a stale observation (rejected by receive_observation, not stored).
        let mut stale_obs = make_observation(session_id, 1);
        stale_obs.timestamp = Utc::now() - chrono::Duration::seconds(30);
        streaming
            .receive_observation(&stream_id, &stale_obs)
            .unwrap();

        // Send a fresh observation.
        streaming
            .receive_observation(&stream_id, &make_observation(session_id, 1))
            .unwrap();

        // Replay should only contain the fresh observation.
        let replayed = streaming.request_replay(&stream_id, 1).unwrap();
        assert_eq!(replayed.len(), 1);
    }

    #[test]
    fn receive_observation_on_unknown_stream_fails() {
        let mut streaming = DefaultObservationStreaming::new();
        let bogus = StreamId::new("stream-999");
        let session_id = Uuid::new_v4();
        let result =
            streaming.receive_observation(&bogus, &make_observation(session_id, 1));
        assert!(result.is_err());
    }

    #[test]
    fn streamed_observation_has_all_ten_fields() {
        let session_id = Uuid::new_v4();
        let obs = make_observation(session_id, 1);
        let json = serde_json::to_value(&obs).unwrap();
        // 10 required fields from distributed.md + sequence for ordering.
        assert!(json["session_id"].is_string());
        assert!(json["step_id"].is_string());
        assert!(json["source_peer"].is_string());
        assert!(json["skill_or_resource_ref"].is_string());
        assert!(json["raw_result"].is_object());
        assert!(json["structured_result"].is_object());
        assert!(json["success"].is_boolean());
        assert!(json["latency_ms"].is_number());
        assert!(json["timestamp"].is_string());
        assert!(json["sequence"].is_number());
    }

    #[test]
    fn delivery_status_variants_serialize() {
        let variants = vec![
            (DeliveryStatus::InOrder, "in_order"),
            (DeliveryStatus::OutOfOrder, "out_of_order"),
            (DeliveryStatus::Duplicate, "duplicate"),
            (DeliveryStatus::MissingPredecessors, "missing_predecessors"),
            (DeliveryStatus::StaleReplay, "stale_replay"),
        ];
        for (status, expected) in variants {
            let json = serde_json::to_value(status).unwrap();
            assert_eq!(json, expected);
        }
    }

    #[test]
    fn stream_id_display() {
        let id = StreamId::new("stream-42");
        assert_eq!(format!("{}", id), "stream-42");
    }

    #[test]
    fn stale_replay_detected_for_old_observation() {
        let mut streaming = DefaultObservationStreaming::new();
        streaming.max_replay_staleness_ms = 5_000; // 5 seconds
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", true)
            .unwrap();

        // Create an observation with a timestamp far in the past.
        let mut obs = make_observation(session_id, 1);
        obs.timestamp = Utc::now() - chrono::Duration::seconds(30);

        let status = streaming
            .receive_observation(&stream_id, &obs)
            .unwrap();
        assert_eq!(status, DeliveryStatus::StaleReplay);

        // Stale replay should not increment observation_count.
        let state = streaming.stream_state(&stream_id).unwrap();
        assert_eq!(state.observation_count, 0);
    }

    #[test]
    fn fresh_observation_not_stale_replay() {
        let mut streaming = DefaultObservationStreaming::new();
        streaming.max_replay_staleness_ms = 60_000;
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", true)
            .unwrap();

        // Fresh observation (just created) should not be flagged as stale.
        let obs = make_observation(session_id, 1);
        let status = streaming
            .receive_observation(&stream_id, &obs)
            .unwrap();
        assert_eq!(status, DeliveryStatus::InOrder);
    }

    #[test]
    fn stale_replay_not_counted_in_seen_sequences() {
        let mut streaming = DefaultObservationStreaming::new();
        streaming.max_replay_staleness_ms = 1_000;
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", true)
            .unwrap();

        // Send a stale observation.
        let mut stale_obs = make_observation(session_id, 1);
        stale_obs.timestamp = Utc::now() - chrono::Duration::seconds(10);
        let status = streaming
            .receive_observation(&stream_id, &stale_obs)
            .unwrap();
        assert_eq!(status, DeliveryStatus::StaleReplay);

        // Now send the same sequence number but fresh — it should not
        // be considered a duplicate since the stale one was not recorded.
        let fresh_obs = make_observation(session_id, 1);
        let status = streaming
            .receive_observation(&stream_id, &fresh_obs)
            .unwrap();
        assert_eq!(status, DeliveryStatus::InOrder);
    }

    #[test]
    fn close_stream_cleans_up_observations() {
        let mut streaming = DefaultObservationStreaming::new();
        let session_id = Uuid::new_v4();
        let stream_id = streaming
            .open_stream(session_id, "peer-1", true)
            .unwrap();

        streaming
            .receive_observation(&stream_id, &make_observation(session_id, 1))
            .unwrap();
        streaming.close_stream(&stream_id).unwrap();

        // Observations should be cleaned up after close.
        assert!(!streaming.observations.contains_key(&stream_id.0));
    }
}
