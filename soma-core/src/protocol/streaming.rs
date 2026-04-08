//! Streaming lifecycle management.
//!
//! Tracks `STREAM_START` / `STREAM_DATA` / `STREAM_END` on per-channel streams,
//! counting frames and handling connection-drop interrupts.

use std::collections::HashMap;

use anyhow::{bail, Result};

/// State for a single active stream on a channel.
///
/// Each channel supports at most one concurrent stream. A stream transitions
/// through: `start_stream` (active=true) -> data frames -> `end_stream` (active=false).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct StreamState {
    /// Channel this stream occupies (one stream per channel).
    pub channel_id: u32,
    /// Application-defined type (e.g., "audio", "video", "data").
    pub stream_type: String,
    /// Encoding format (e.g., "opus", "raw").
    pub codec: String,
    /// `true` while the stream is open; `false` after end or interrupt.
    pub active: bool,
    /// Frames sent by the local side (tracked via `on_stream_sent`).
    pub frames_sent: u64,
    /// Frames received from the remote side (tracked via `on_stream_data`).
    pub frames_received: u64,
}

/// Manages the lifecycle of all active streams across channels.
///
/// Enforces single-stream-per-channel: attempting to start a second stream
/// on an occupied channel returns an error.
#[allow(dead_code)]
pub struct StreamManager {
    /// `channel_id` -> stream state. Only active streams are present.
    active_streams: HashMap<u32, StreamState>,
}

#[allow(dead_code)]
impl StreamManager {
    pub fn new() -> Self {
        Self {
            active_streams: HashMap::new(),
        }
    }

    /// Open a new stream on `channel_id`. Reads `type` and `codec` from
    /// `metadata`, defaulting to `"data"` and `"raw"` respectively.
    /// Fails if the channel already has an active stream.
    pub fn start_stream(
        &mut self,
        channel_id: u32,
        metadata: &serde_json::Value,
    ) -> Result<()> {
        if self.active_streams.contains_key(&channel_id) {
            bail!(
                "Stream already active on channel {channel_id} — send STREAM_END first"
            );
        }

        let stream_type = metadata
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("data")
            .to_string();

        let codec = metadata
            .get("codec")
            .and_then(|v| v.as_str())
            .unwrap_or("raw")
            .to_string();

        self.active_streams.insert(
            channel_id,
            StreamState {
                channel_id,
                stream_type,
                codec,
                active: true,
                frames_sent: 0,
                frames_received: 0,
            },
        );

        Ok(())
    }

    /// Record an incoming data frame on `channel_id`. Fails if no active
    /// stream exists on that channel.
    pub fn on_stream_data(&mut self, channel_id: u32) -> Result<()> {
        match self.active_streams.get_mut(&channel_id) {
            Some(state) if state.active => {
                state.frames_received += 1;
                Ok(())
            }
            Some(_) => bail!("Stream on channel {channel_id} is not active"),
            None => bail!("No stream on channel {channel_id}"),
        }
    }

    /// Record an outgoing data frame on `channel_id` (sender side).
    pub fn on_stream_sent(&mut self, channel_id: u32) -> Result<()> {
        match self.active_streams.get_mut(&channel_id) {
            Some(state) if state.active => {
                state.frames_sent += 1;
                Ok(())
            }
            Some(_) => bail!("Stream on channel {channel_id} is not active"),
            None => bail!("No stream on channel {channel_id}"),
        }
    }

    /// Close the stream on `channel_id`, removing it from the active map
    /// and returning its final state with `active = false`.
    pub fn end_stream(&mut self, channel_id: u32) -> Result<StreamState> {
        match self.active_streams.remove(&channel_id) {
            Some(mut state) => {
                state.active = false;
                Ok(state)
            }
            None => bail!("No stream on channel {channel_id} to end"),
        }
    }

    /// Forcibly close all streams (e.g., on connection drop). Returns
    /// the final state of each stream that was active.
    pub fn interrupt_all(&mut self) -> Vec<StreamState> {
        let mut interrupted = Vec::new();
        for (_, mut state) in self.active_streams.drain() {
            state.active = false;
            interrupted.push(state);
        }
        interrupted
    }

    /// Check if a channel has an active stream.
    pub fn is_active(&self, channel_id: u32) -> bool {
        self.active_streams
            .get(&channel_id)
            .is_some_and(|s| s.active)
    }

    /// Get the state of a stream (if any) on a channel.
    pub fn get(&self, channel_id: u32) -> Option<&StreamState> {
        self.active_streams.get(&channel_id)
    }

    /// Number of currently active streams.
    pub fn active_count(&self) -> usize {
        self.active_streams.values().filter(|s| s.active).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_lifecycle() {
        let mut mgr = StreamManager::new();

        let meta = serde_json::json!({
            "type": "audio",
            "codec": "opus",
            "sample_rate": 48000,
        });
        mgr.start_stream(42, &meta).unwrap();
        assert!(mgr.is_active(42));
        assert_eq!(mgr.active_count(), 1);

        // Receive some frames
        mgr.on_stream_data(42).unwrap();
        mgr.on_stream_data(42).unwrap();
        mgr.on_stream_data(42).unwrap();

        let state = mgr.get(42).unwrap();
        assert_eq!(state.frames_received, 3);
        assert_eq!(state.stream_type, "audio");
        assert_eq!(state.codec, "opus");

        // End stream
        let final_state = mgr.end_stream(42).unwrap();
        assert!(!final_state.active);
        assert_eq!(final_state.frames_received, 3);
        assert!(!mgr.is_active(42));
    }

    #[test]
    fn test_duplicate_start_fails() {
        let mut mgr = StreamManager::new();
        let meta = serde_json::json!({"type": "video"});
        mgr.start_stream(1, &meta).unwrap();
        assert!(mgr.start_stream(1, &meta).is_err());
    }

    #[test]
    fn test_data_on_no_stream_fails() {
        let mut mgr = StreamManager::new();
        assert!(mgr.on_stream_data(99).is_err());
    }

    #[test]
    fn test_end_no_stream_fails() {
        let mut mgr = StreamManager::new();
        assert!(mgr.end_stream(99).is_err());
    }

    #[test]
    fn test_interrupt_all() {
        let mut mgr = StreamManager::new();
        mgr.start_stream(1, &serde_json::json!({"type": "audio"}))
            .unwrap();
        mgr.start_stream(2, &serde_json::json!({"type": "video"}))
            .unwrap();
        mgr.on_stream_data(1).unwrap();
        mgr.on_stream_data(2).unwrap();
        mgr.on_stream_data(2).unwrap();

        let interrupted = mgr.interrupt_all();
        assert_eq!(interrupted.len(), 2);
        for s in &interrupted {
            assert!(!s.active);
        }
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn test_sent_frames() {
        let mut mgr = StreamManager::new();
        mgr.start_stream(10, &serde_json::json!({"type": "data"}))
            .unwrap();
        mgr.on_stream_sent(10).unwrap();
        mgr.on_stream_sent(10).unwrap();
        let state = mgr.get(10).unwrap();
        assert_eq!(state.frames_sent, 2);
        assert_eq!(state.frames_received, 0);
    }
}
