use std::collections::HashMap;
use std::io::{BufReader, Read as IoRead, Write as IoWrite};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::config::TlsConfig;
use crate::errors::{Result, SomaError};
use crate::runtime::goal::GoalRuntime;
use crate::runtime::session::SessionRuntime;
use crate::types::peer::{DistributedFailure, RemoteGoalRequest, RoutineTransfer, SchemaTransfer};

use super::chunked::{Chunk, ResumeRequest, TransferManifest};
use super::rate_limit::{RateDecision, RateLimitConfig, RateLimiter};
use super::remote::{
    RemoteExecutor, RemoteGoalResponse, RemoteGoalStatus, RemoteResourceResponse,
    RemoteSkillResponse, ResourceDataMode,
};

// ---------------------------------------------------------------------------
// Wire protocol messages
// ---------------------------------------------------------------------------

/// Envelope for all messages on the wire. Each message carries a type tag
/// so the receiver can dispatch to the appropriate handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransportMessage {
    InvokeSkill {
        peer_id: String,
        skill_id: String,
        input: serde_json::Value,
    },
    QueryResource {
        peer_id: String,
        resource_type: String,
        resource_id: String,
    },
    SubmitGoal {
        peer_id: String,
        request: RemoteGoalRequest,
    },
    TransferSchema {
        peer_id: String,
        schema: SchemaTransfer,
    },
    TransferRoutine {
        peer_id: String,
        routine: RoutineTransfer,
    },
    /// Begin a chunked transfer by sending the manifest. The receiver uses
    /// this to allocate storage and know the expected chunks and hashes.
    ChunkedTransferStart {
        peer_id: String,
        manifest: TransferManifest,
    },
    /// Deliver a single chunk of an ongoing chunked transfer.
    ChunkedTransferData {
        peer_id: String,
        chunk: Chunk,
    },
    /// Request to resume an interrupted transfer. The receiver reports which
    /// chunks it already has so the sender can skip them.
    ChunkedTransferResume {
        peer_id: String,
        resume: ResumeRequest,
    },
    /// Heartbeat ping. The sender includes a nonce so the receiver can echo
    /// it back in the pong, allowing the sender to measure round-trip time.
    Ping {
        nonce: u64,
    },
    /// Query a peer for its full capability inventory (primitives + routines).
    /// The receiver responds with `Capabilities`. Used by the brain to
    /// discover what a leaf or peer can do without invoking it.
    ListCapabilities,
    /// Remove a previously transferred routine from a peer by ID.
    /// The receiver responds with `RoutineRemoved`.
    RemoveRoutine {
        routine_id: String,
    },
}

/// Envelope for all responses on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransportResponse {
    SkillResult {
        response: RemoteSkillResponse,
    },
    ResourceResult {
        response: RemoteResourceResponse,
    },
    GoalResult {
        response: RemoteGoalResponse,
    },
    SchemaOk,
    RoutineOk,
    Error {
        details: String,
    },
    /// Acknowledge receipt of a chunked transfer manifest.
    ChunkedTransferAccepted {
        transfer_id: Uuid,
    },
    /// Acknowledge receipt and verification of a single chunk.
    ChunkedChunkAck {
        transfer_id: Uuid,
        chunk_index: u32,
    },
    /// Response to a resume request: the list of chunk indices the receiver
    /// still needs.
    ChunkedResumeMissing {
        transfer_id: Uuid,
        missing_indices: Vec<u32>,
    },
    /// The chunked transfer completed: all chunks received and overall
    /// integrity verified.
    ChunkedTransferComplete {
        transfer_id: Uuid,
    },
    /// Heartbeat pong. Echoes the nonce from the ping and includes the
    /// responding peer's current load so the sender can update its view.
    Pong {
        nonce: u64,
        load: f64,
    },
    /// Capability inventory of a peer. Returned in response to
    /// `ListCapabilities`. The brain reads this to learn what skills the
    /// peer provides — primitive ports the peer ships with, plus routines
    /// previously transferred and stored on the peer.
    Capabilities {
        primitives: Vec<PeerCapability>,
        routines: Vec<PeerRoutineSummary>,
    },
    /// Acknowledge that a transferred routine is now stored on the peer.
    /// The receiver returns this in response to `TransferRoutine`. Newer
    /// peers may return this instead of `RoutineOk` to provide more detail.
    RoutineStored {
        routine_id: String,
        step_count: u32,
    },
    /// Acknowledge that a stored routine has been removed from the peer.
    RoutineRemoved {
        routine_id: String,
    },
}

/// Description of one capability a peer provides. Mirrors the leaf's
/// CapabilityDescriptor but lives in soma-next so the desktop runtime can
/// reason about peer capabilities directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerCapability {
    pub skill_id: String,
    pub description: String,
    pub input_schema: String,
    pub output_schema: String,
    pub effect: PeerCapabilityEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerCapabilityEffect {
    ReadOnly,
    StateMutation,
    ExternalEffect,
}

/// Summary of one routine currently stored on a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRoutineSummary {
    pub routine_id: String,
    pub description: String,
    pub step_count: u32,
}

// ---------------------------------------------------------------------------
// Framing helpers: 4-byte big-endian length prefix + JSON payload
// ---------------------------------------------------------------------------

async fn write_frame(stream: &mut TcpStream, payload: &[u8]) -> std::io::Result<()> {
    let len = payload.len() as u32;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame too large: {} bytes", len),
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Write a length-prefixed frame to a generic async writer (used by TLS streams).
async fn write_frame_async<W: AsyncWriteExt + Unpin>(
    stream: &mut W,
    payload: &[u8],
) -> std::io::Result<()> {
    let len = payload.len() as u32;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

/// Read a length-prefixed frame from a generic async reader (used by TLS streams).
async fn read_frame_async<R: AsyncReadExt + Unpin>(stream: &mut R) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame too large: {} bytes", len),
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Blocking versions for the sync RemoteExecutor trait methods.
fn write_frame_sync(stream: &mut std::net::TcpStream, payload: &[u8]) -> std::io::Result<()> {
    let len = payload.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(payload)?;
    stream.flush()?;
    Ok(())
}

fn read_frame_sync(stream: &mut std::net::TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame too large: {} bytes", len),
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// TcpRemoteExecutor — client side
// ---------------------------------------------------------------------------

/// Maps peer IDs to their TCP addresses so the executor knows where to connect.
pub type PeerAddressMap = Arc<Mutex<HashMap<String, SocketAddr>>>;

/// Implements `RemoteExecutor` by opening a TCP connection to the target peer,
/// sending a length-prefixed JSON message, and reading the response.
///
/// Each call opens a fresh connection. Connection pooling can be added later
/// without changing the trait interface.
pub struct TcpRemoteExecutor {
    peer_addresses: PeerAddressMap,
}

impl TcpRemoteExecutor {
    pub fn new(peer_addresses: PeerAddressMap) -> Self {
        Self { peer_addresses }
    }

    /// Send a request to the given peer and return the parsed response.
    fn send_request(
        &self,
        peer_id: &str,
        msg: &TransportMessage,
    ) -> Result<TransportResponse> {
        let addr = {
            let map = self.peer_addresses.lock().unwrap();
            map.get(peer_id).copied().ok_or_else(|| SomaError::Distributed {
                failure: DistributedFailure::PeerUnreachable,
                details: format!("no address registered for peer '{}'", peer_id),
            })?
        };

        let mut stream = std::net::TcpStream::connect(addr).map_err(|e| {
            SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("TCP connect to {} (peer {}) failed: {}", addr, peer_id, e),
            }
        })?;
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(30)))
            .ok();
        stream
            .set_write_timeout(Some(std::time::Duration::from_secs(10)))
            .ok();

        let payload = serde_json::to_vec(msg).map_err(|e| SomaError::Distributed {
            failure: DistributedFailure::TransportFailure,
            details: format!("failed to serialize request: {}", e),
        })?;

        write_frame_sync(&mut stream, &payload).map_err(|e| SomaError::Distributed {
            failure: DistributedFailure::TransportFailure,
            details: format!("failed to send frame to peer {}: {}", peer_id, e),
        })?;

        let resp_bytes = read_frame_sync(&mut stream).map_err(|e| SomaError::Distributed {
            failure: DistributedFailure::TransportFailure,
            details: format!("failed to read response from peer {}: {}", peer_id, e),
        })?;

        let response: TransportResponse =
            serde_json::from_slice(&resp_bytes).map_err(|e| SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("failed to deserialize response from peer {}: {}", peer_id, e),
            })?;

        Ok(response)
    }

    fn check_error(response: &TransportResponse) -> Result<()> {
        if let TransportResponse::Error { details } = response {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: details.clone(),
            });
        }
        Ok(())
    }

    /// Send a Ping to the given peer and return the Pong response.
    /// Used by HeartbeatManager to measure RTT and collect load information.
    pub fn send_ping(&self, peer_id: &str, nonce: u64) -> Result<TransportResponse> {
        let msg = TransportMessage::Ping { nonce };
        self.send_request(peer_id, &msg)
    }
}

impl RemoteExecutor for TcpRemoteExecutor {
    fn submit_goal(
        &self,
        peer_id: &str,
        request: &RemoteGoalRequest,
    ) -> Result<RemoteGoalResponse> {
        let msg = TransportMessage::SubmitGoal {
            peer_id: peer_id.to_string(),
            request: request.clone(),
        };
        let response = self.send_request(peer_id, &msg)?;
        Self::check_error(&response)?;
        match response {
            TransportResponse::GoalResult { response } => Ok(response),
            other => Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("unexpected response type for submit_goal: {:?}", other),
            }),
        }
    }

    fn invoke_skill(
        &self,
        peer_id: &str,
        skill_id: &str,
        input: serde_json::Value,
    ) -> Result<RemoteSkillResponse> {
        let msg = TransportMessage::InvokeSkill {
            peer_id: peer_id.to_string(),
            skill_id: skill_id.to_string(),
            input,
        };
        let response = self.send_request(peer_id, &msg)?;
        Self::check_error(&response)?;
        match response {
            TransportResponse::SkillResult { mut response } => {
                // Smaller peers (e.g. ESP32 leaf firmware) don't fill in
                // peer_id on the wire. Patch it in post-receive so the
                // returned record carries the caller-side peer ID.
                if response.peer_id.is_empty() {
                    response.peer_id = peer_id.to_string();
                }
                Ok(response)
            }
            other => Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("unexpected response type for invoke_skill: {:?}", other),
            }),
        }
    }

    fn query_resource(
        &self,
        peer_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<RemoteResourceResponse> {
        let msg = TransportMessage::QueryResource {
            peer_id: peer_id.to_string(),
            resource_type: resource_type.to_string(),
            resource_id: resource_id.to_string(),
        };
        let response = self.send_request(peer_id, &msg)?;
        Self::check_error(&response)?;
        match response {
            TransportResponse::ResourceResult { response } => Ok(response),
            other => Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("unexpected response type for query_resource: {:?}", other),
            }),
        }
    }

    fn transfer_schema(&self, peer_id: &str, schema: &SchemaTransfer) -> Result<()> {
        let msg = TransportMessage::TransferSchema {
            peer_id: peer_id.to_string(),
            schema: schema.clone(),
        };
        let response = self.send_request(peer_id, &msg)?;
        Self::check_error(&response)?;
        match response {
            TransportResponse::SchemaOk => Ok(()),
            other => Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("unexpected response type for transfer_schema: {:?}", other),
            }),
        }
    }

    fn transfer_routine(&self, peer_id: &str, routine: &RoutineTransfer) -> Result<()> {
        // Serialize and check size. If the payload exceeds 64KB, use chunked
        // transfer for reliability and resumability.
        let payload = serde_json::to_vec(routine).map_err(|e| SomaError::Distributed {
            failure: DistributedFailure::TransportFailure,
            details: format!("failed to serialize routine: {}", e),
        })?;

        if payload.len() > 64 * 1024 {
            let sender = super::chunked::ChunkedSender::default();
            let (manifest, chunks) = sender.prepare(&payload);

            let manifest_msg = TransportMessage::ChunkedTransferStart {
                peer_id: peer_id.to_string(),
                manifest: manifest.clone(),
            };
            let resp = self.send_request(peer_id, &manifest_msg)?;
            Self::check_error(&resp)?;

            for chunk in &chunks {
                let chunk_msg = TransportMessage::ChunkedTransferData {
                    peer_id: peer_id.to_string(),
                    chunk: chunk.clone(),
                };
                let resp = self.send_request(peer_id, &chunk_msg)?;
                Self::check_error(&resp)?;
            }

            Ok(())
        } else {
            let msg = TransportMessage::TransferRoutine {
                peer_id: peer_id.to_string(),
                routine: routine.clone(),
            };
            let response = self.send_request(peer_id, &msg)?;
            Self::check_error(&response)?;
            match response {
                TransportResponse::RoutineOk | TransportResponse::RoutineStored { .. } => Ok(()),
                other => Err(SomaError::Distributed {
                    failure: DistributedFailure::TransportFailure,
                    details: format!("unexpected response type for transfer_routine: {:?}", other),
                }),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// TcpTransport — server side (listener + request dispatch)
// ---------------------------------------------------------------------------

/// Callback trait for handling incoming requests on the server side.
/// The listener dispatches parsed messages to this handler and sends back
/// the returned response.
pub trait IncomingHandler: Send + Sync + 'static {
    fn handle(&self, msg: TransportMessage) -> TransportResponse;
}

/// A handler that uses the local runtime's skill executor to service
/// incoming invoke_skill requests. Also stores transferred schemas and
/// routines when received from remote peers.
pub struct LocalDispatchHandler {
    /// The local runtime, used to execute skills on behalf of remote peers.
    runtime: Arc<Mutex<crate::bootstrap::Runtime>>,
    /// Active chunked transfer receivers, keyed by transfer_id.
    chunked_receivers: Mutex<HashMap<Uuid, super::chunked::ChunkedReceiver>>,
    /// Schema store for receiving transferred schemas.
    schema_store: Option<Arc<Mutex<dyn crate::memory::schemas::SchemaStore + Send>>>,
    /// Routine store for receiving transferred routines.
    routine_store: Option<Arc<Mutex<dyn crate::memory::routines::RoutineStore + Send>>>,
}

impl LocalDispatchHandler {
    pub fn new(runtime: Arc<Mutex<crate::bootstrap::Runtime>>) -> Self {
        Self {
            runtime,
            chunked_receivers: Mutex::new(HashMap::new()),
            schema_store: None,
            routine_store: None,
        }
    }

    /// Create a handler with schema and routine stores for receiving transfers.
    pub fn with_stores(
        runtime: Arc<Mutex<crate::bootstrap::Runtime>>,
        schema_store: Arc<Mutex<dyn crate::memory::schemas::SchemaStore + Send>>,
        routine_store: Arc<Mutex<dyn crate::memory::routines::RoutineStore + Send>>,
    ) -> Self {
        Self {
            runtime,
            chunked_receivers: Mutex::new(HashMap::new()),
            schema_store: Some(schema_store),
            routine_store: Some(routine_store),
        }
    }
}

impl IncomingHandler for LocalDispatchHandler {
    fn handle(&self, msg: TransportMessage) -> TransportResponse {
        match msg {
            TransportMessage::InvokeSkill {
                peer_id: _,
                skill_id,
                input: _,
            } => {
                // Execute the skill locally by creating a session for the goal,
                // then stepping it to completion.
                let mut rt = self.runtime.lock().unwrap();
                let goal_text = format!("remote invoke: {}", skill_id);
                let goal_input = crate::runtime::goal::GoalInput::NaturalLanguage {
                    text: goal_text,
                    source: crate::types::goal::GoalSource {
                        source_type: crate::types::goal::GoalSourceType::Peer,
                        identity: None,
                        session_id: None,
                        peer_id: None,
                    },
                };
                let goal_spec = match rt.goal_runtime.parse_goal(goal_input) {
                    Ok(g) => g,
                    Err(e) => {
                        return TransportResponse::Error {
                            details: format!(
                                "failed to parse goal for skill {}: {}",
                                skill_id, e
                            ),
                        };
                    }
                };
                let mut session = match rt.session_controller.create_session(goal_spec) {
                    Ok(s) => s,
                    Err(e) => {
                        return TransportResponse::Error {
                            details: format!("failed to create session: {}", e),
                        };
                    }
                };

                // Step the session until it reaches a terminal state.
                let mut success = false;
                let mut final_observation = serde_json::json!(null);
                let max_steps = 100;
                for _ in 0..max_steps {
                    match rt.session_controller.run_step(&mut session) {
                        Ok(step_result) => match step_result {
                            crate::runtime::session::StepResult::Continue => {
                                // Capture the latest observation from trace.
                                if let Some(last_step) = session.trace.steps.last() {
                                    for pc in &last_step.port_calls {
                                        if pc.success {
                                            success = true;
                                            final_observation =
                                                pc.structured_result.clone();
                                        }
                                    }
                                }
                            }
                            crate::runtime::session::StepResult::Completed => {
                                if let Some(last_step) = session.trace.steps.last() {
                                    for pc in &last_step.port_calls {
                                        if pc.success {
                                            success = true;
                                            final_observation =
                                                pc.structured_result.clone();
                                        }
                                    }
                                }
                                break;
                            }
                            crate::runtime::session::StepResult::Failed(reason) => {
                                return TransportResponse::Error { details: reason };
                            }
                            crate::runtime::session::StepResult::Aborted => {
                                return TransportResponse::Error {
                                    details: "session aborted".to_string(),
                                };
                            }
                            _ => break,
                        },
                        Err(e) => {
                            return TransportResponse::Error {
                                details: format!("session step failed: {}", e),
                            };
                        }
                    }
                }

                TransportResponse::SkillResult {
                    response: RemoteSkillResponse {
                        skill_id,
                        peer_id: "local".to_string(),
                        success,
                        observation: final_observation,
                        latency_ms: 0,
                        timestamp: Utc::now(),
                        trace_id: Uuid::new_v4(),
                    },
                }
            }
            TransportMessage::QueryResource {
                peer_id: _,
                resource_type,
                resource_id,
            } => {
                TransportResponse::ResourceResult {
                    response: RemoteResourceResponse {
                        resource_type,
                        resource_id,
                        data: serde_json::json!(null),
                        data_mode: ResourceDataMode::Snapshot,
                        version: 0,
                        provenance: "local".to_string(),
                        freshness_ms: 0,
                        timestamp: Utc::now(),
                    },
                }
            }
            TransportMessage::SubmitGoal { peer_id: _, request } => {
                let mut rt = self.runtime.lock().unwrap();
                let goal_input = crate::runtime::goal::GoalInput::NaturalLanguage {
                    text: request
                        .goal
                        .as_str()
                        .unwrap_or("remote goal")
                        .to_string(),
                    source: crate::types::goal::GoalSource {
                        source_type: crate::types::goal::GoalSourceType::Peer,
                        identity: None,
                        session_id: None,
                        peer_id: None,
                    },
                };
                match rt.goal_runtime.parse_goal(goal_input) {
                    Ok(goal_spec) => {
                        match rt.session_controller.create_session(goal_spec) {
                            Ok(session) => TransportResponse::GoalResult {
                                response: RemoteGoalResponse {
                                    status: RemoteGoalStatus::Accepted,
                                    session_id: Some(session.session_id.to_string()),
                                    reason: None,
                                    required_adjustments: None,
                                },
                            },
                            Err(e) => TransportResponse::GoalResult {
                                response: RemoteGoalResponse {
                                    status: RemoteGoalStatus::Rejected,
                                    session_id: None,
                                    reason: Some(format!("session creation failed: {}", e)),
                                    required_adjustments: None,
                                },
                            },
                        }
                    }
                    Err(e) => TransportResponse::GoalResult {
                        response: RemoteGoalResponse {
                            status: RemoteGoalStatus::Rejected,
                            session_id: None,
                            reason: Some(format!("goal parse failed: {}", e)),
                            required_adjustments: None,
                        },
                    },
                }
            }
            TransportMessage::TransferSchema { peer_id, schema } => {
                if let Some(ref store) = self.schema_store {
                    let schema_to_store = crate::types::schema::Schema {
                        schema_id: schema.schema_id.clone(),
                        namespace: format!("peer:{}", peer_id),
                        pack: String::new(),
                        name: schema.schema_id.clone(),
                        version: semver::Version::parse(&schema.version)
                            .unwrap_or_else(|_| semver::Version::new(0, 1, 0)),
                        trigger_conditions: schema.trigger_conditions,
                        resource_requirements: vec![],
                        subgoal_structure: schema.subgoal_structure.iter().map(|v| {
                            crate::types::schema::SubgoalNode {
                                subgoal_id: v["subgoal_id"].as_str().unwrap_or("unknown").to_string(),
                                description: v["description"].as_str().unwrap_or("").to_string(),
                                skill_candidates: v["skill_candidates"]
                                    .as_array()
                                    .map(|arr| arr.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                                    .unwrap_or_default(),
                                dependencies: v["dependencies"]
                                    .as_array()
                                    .map(|arr| arr.iter().filter_map(|d| d.as_str().map(String::from)).collect())
                                    .unwrap_or_default(),
                                optional: v["optional"].as_bool().unwrap_or(false),
                            }
                        }).collect(),
                        candidate_skill_ordering: schema.candidate_skill_ordering,
                        stop_conditions: schema.stop_conditions,
                        rollback_bias: crate::types::schema::RollbackBias::Cautious,
                        confidence: schema.confidence,
                    };
                    match store.lock().unwrap().register(schema_to_store) {
                        Ok(()) => {
                            info!(schema_id = %schema.schema_id, peer = %peer_id, "stored transferred schema");
                            TransportResponse::SchemaOk
                        }
                        Err(e) => TransportResponse::Error {
                            details: format!("failed to store transferred schema: {}", e),
                        },
                    }
                } else {
                    // No store attached — accept silently (backward-compatible stub).
                    TransportResponse::SchemaOk
                }
            }
            TransportMessage::TransferRoutine { peer_id, routine } => {
                if let Some(ref store) = self.routine_store {
                    let routine_to_store = crate::types::routine::Routine {
                        routine_id: routine.routine_id.clone(),
                        namespace: format!("peer:{}", peer_id),
                        origin: crate::types::routine::RoutineOrigin::PeerTransferred,
                        match_conditions: routine.match_conditions,
                        compiled_skill_path: routine.compiled_skill_path,
                        compiled_steps: routine.compiled_steps,
                        guard_conditions: routine.guard_conditions,
                        expected_cost: routine.expected_cost,
                        expected_effect: routine.expected_effect,
                        confidence: routine.confidence,
                        autonomous: routine.autonomous,
                        priority: routine.priority,
                        exclusive: routine.exclusive,
                        policy_scope: routine.policy_scope,
                        version: routine.version,
                    };
                    match store.lock().unwrap().register(routine_to_store) {
                        Ok(()) => {
                            info!(routine_id = %routine.routine_id, peer = %peer_id, "stored transferred routine");
                            TransportResponse::RoutineOk
                        }
                        Err(e) => TransportResponse::Error {
                            details: format!("failed to store transferred routine: {}", e),
                        },
                    }
                } else {
                    // No store attached — accept silently (backward-compatible stub).
                    TransportResponse::RoutineOk
                }
            }
            TransportMessage::ChunkedTransferStart { peer_id: _, manifest } => {
                let transfer_id = manifest.transfer_id;
                match super::chunked::ChunkedReceiver::new(manifest) {
                    Ok(receiver) => {
                        let mut receivers = self.chunked_receivers.lock().unwrap();
                        receivers.insert(transfer_id, receiver);
                        TransportResponse::ChunkedTransferAccepted { transfer_id }
                    }
                    Err(e) => TransportResponse::Error {
                        details: format!("failed to start chunked transfer: {}", e),
                    },
                }
            }
            TransportMessage::ChunkedTransferData { peer_id: _, chunk } => {
                let transfer_id = chunk.transfer_id;
                let chunk_index = chunk.chunk_index;
                let mut receivers = self.chunked_receivers.lock().unwrap();
                match receivers.get_mut(&transfer_id) {
                    Some(receiver) => match receiver.receive_chunk(&chunk) {
                        Ok(()) => {
                            if receiver.is_complete() {
                                let receiver = receivers.remove(&transfer_id).unwrap();
                                match receiver.finalize() {
                                    Ok(_payload) => {
                                        TransportResponse::ChunkedTransferComplete { transfer_id }
                                    }
                                    Err(e) => TransportResponse::Error {
                                        details: format!("chunked transfer finalization failed: {}", e),
                                    },
                                }
                            } else {
                                TransportResponse::ChunkedChunkAck { transfer_id, chunk_index }
                            }
                        }
                        Err(e) => TransportResponse::Error {
                            details: format!("chunk receive failed: {}", e),
                        },
                    },
                    None => TransportResponse::Error {
                        details: format!("no active chunked transfer for id {}", transfer_id),
                    },
                }
            }
            TransportMessage::ChunkedTransferResume { peer_id: _, resume } => {
                let receivers = self.chunked_receivers.lock().unwrap();
                match receivers.get(&resume.transfer_id) {
                    Some(receiver) => TransportResponse::ChunkedResumeMissing {
                        transfer_id: resume.transfer_id,
                        missing_indices: receiver.missing_indices(),
                    },
                    None => TransportResponse::Error {
                        details: format!("no active chunked transfer for id {}", resume.transfer_id),
                    },
                }
            }
            TransportMessage::Ping { nonce } => TransportResponse::Pong { nonce, load: 0.0 },
            TransportMessage::ListCapabilities => {
                // For now we report only stored routines because the
                // SchemaStore/RoutineStore know about them. Primitive
                // capabilities require pack-skill enumeration which is not
                // yet wired into LocalDispatchHandler. Returning an empty
                // primitives list keeps the wire compatible while leaving
                // room for the runtime to populate it later.
                let routines: Vec<PeerRoutineSummary> = if let Some(ref store) =
                    self.routine_store
                {
                    store
                        .lock()
                        .unwrap()
                        .list_all()
                        .iter()
                        .map(|r| PeerRoutineSummary {
                            routine_id: r.routine_id.clone(),
                            description: r.namespace.clone(),
                            step_count: r.compiled_skill_path.len() as u32,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                TransportResponse::Capabilities {
                    primitives: Vec::new(),
                    routines,
                }
            }
            TransportMessage::RemoveRoutine { routine_id } => {
                if let Some(ref store) = self.routine_store {
                    match store.lock().unwrap().invalidate(&routine_id) {
                        Ok(()) => TransportResponse::RoutineRemoved {
                            routine_id: routine_id.clone(),
                        },
                        Err(e) => TransportResponse::Error {
                            details: format!("failed to remove routine: {}", e),
                        },
                    }
                } else {
                    TransportResponse::RoutineRemoved {
                        routine_id: routine_id.clone(),
                    }
                }
            }
        }
    }
}

/// TCP listener that accepts incoming connections and dispatches requests
/// to an `IncomingHandler`. Runs as a background tokio task.
///
/// Includes per-peer rate limiting: each incoming request is checked against
/// a shared `RateLimiter` before being dispatched to the handler.
pub struct TcpTransport {
    bind_addr: SocketAddr,
    rate_limiter: Arc<Mutex<RateLimiter>>,
}

impl TcpTransport {
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(&RateLimitConfig::default()))),
        }
    }

    /// Create a transport with a custom rate limit configuration.
    pub fn with_rate_limit(bind_addr: SocketAddr, config: &RateLimitConfig) -> Self {
        Self {
            bind_addr,
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(config))),
        }
    }

    /// Start the TCP listener in the provided tokio runtime. Returns a
    /// `JoinHandle` that can be used to await shutdown. The listener
    /// accepts connections in a loop, spawning a task per connection.
    pub async fn listen(
        self,
        handler: Arc<dyn IncomingHandler>,
    ) -> std::result::Result<(), std::io::Error> {
        let listener = TcpListener::bind(self.bind_addr).await?;
        info!(addr = %self.bind_addr, "TCP transport listening");

        loop {
            let (mut stream, peer_addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    warn!(error = %e, "failed to accept TCP connection");
                    continue;
                }
            };

            let handler = Arc::clone(&handler);
            let rate_limiter = Arc::clone(&self.rate_limiter);
            tokio::spawn(async move {
                debug!(peer = %peer_addr, "accepted TCP connection");
                if let Err(e) = handle_connection(&mut stream, &*handler, &rate_limiter, &peer_addr).await {
                    debug!(peer = %peer_addr, error = %e, "connection handler finished with error");
                }
            });
        }
    }
}

/// Handle a single TCP connection: check rate limits, read one request,
/// dispatch, write response.
async fn handle_connection(
    stream: &mut TcpStream,
    handler: &dyn IncomingHandler,
    rate_limiter: &Arc<Mutex<RateLimiter>>,
    peer_addr: &SocketAddr,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Check rate limit before reading the full request payload.
    let peer_key = peer_addr.ip().to_string();
    let decision = {
        let mut limiter = rate_limiter.lock().unwrap();
        limiter.check(&peer_key)
    };

    match decision {
        RateDecision::Allow => {}
        RateDecision::Throttle { wait_ms } => {
            debug!(peer = %peer_addr, wait_ms, "rate-limiting peer (throttle)");
            let response = TransportResponse::Error {
                details: format!("rate limited: retry after {}ms", wait_ms),
            };
            let resp_bytes = serde_json::to_vec(&response)?;
            write_frame(stream, &resp_bytes).await?;
            return Ok(());
        }
        RateDecision::Deny => {
            warn!(peer = %peer_addr, "rate-limiting peer (deny)");
            let response = TransportResponse::Error {
                details: "rate limited: too many requests".to_string(),
            };
            let resp_bytes = serde_json::to_vec(&response)?;
            write_frame(stream, &resp_bytes).await?;
            return Ok(());
        }
        RateDecision::Blacklisted => {
            warn!(peer = %peer_addr, "rejecting blacklisted peer");
            return Ok(());
        }
    }

    let req_bytes = read_frame(stream).await?;
    let msg: TransportMessage = serde_json::from_slice(&req_bytes)?;

    debug!(msg_type = ?std::mem::discriminant(&msg), "dispatching incoming request");
    let response = handler.handle(msg);

    let resp_bytes = serde_json::to_vec(&response)?;
    write_frame(stream, &resp_bytes).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Convenience: start listener in a background thread with its own tokio runtime
// ---------------------------------------------------------------------------

/// Start the TCP listener on a new thread with a dedicated tokio runtime.
/// Returns a handle to the thread. The listener runs until the process exits.
pub fn start_listener_background(
    bind_addr: SocketAddr,
    handler: Arc<dyn IncomingHandler>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("soma-tcp-listener".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime for TCP listener");
            rt.block_on(async {
                let transport = TcpTransport::new(bind_addr);
                if let Err(e) = transport.listen(handler).await {
                    error!(error = %e, "TCP listener failed");
                }
            });
        })
        .expect("failed to spawn TCP listener thread")
}

// ---------------------------------------------------------------------------
// TLS helpers: build rustls configs from file paths
// ---------------------------------------------------------------------------

/// Load a PEM certificate chain from a file path.
fn load_certs(path: &str) -> std::io::Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
}

/// Load the first PEM private key from a file path.
fn load_private_key(path: &str) -> std::io::Result<rustls::pki_types::PrivateKeyDer<'static>> {
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    for item in rustls_pemfile::read_all(&mut reader) {
        match item? {
            rustls_pemfile::Item::Pkcs8Key(key) => {
                return Ok(rustls::pki_types::PrivateKeyDer::Pkcs8(key));
            }
            rustls_pemfile::Item::Pkcs1Key(key) => {
                return Ok(rustls::pki_types::PrivateKeyDer::Pkcs1(key));
            }
            rustls_pemfile::Item::Sec1Key(key) => {
                return Ok(rustls::pki_types::PrivateKeyDer::Sec1(key));
            }
            _ => continue,
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "no private key found in PEM file",
    ))
}

/// Build a `rustls::ServerConfig` from a TLS config.
fn build_server_tls_config(
    tls: &TlsConfig,
) -> std::result::Result<rustls::ServerConfig, Box<dyn std::error::Error>> {
    let certs = load_certs(&tls.cert_path)?;
    let key = load_private_key(&tls.key_path)?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("invalid server TLS config: {}", e))?;

    Ok(config)
}

/// Build a `rustls::ClientConfig` from an optional CA path. When a CA is
/// provided, only that CA is trusted. Otherwise the root store is empty
/// (appropriate for testing with self-signed certs when combined with a
/// dangerous verifier, or when the server cert chains to a well-known CA
/// added via platform roots).
fn build_client_tls_config(
    ca_path: Option<&str>,
) -> std::result::Result<rustls::ClientConfig, Box<dyn std::error::Error>> {
    let mut root_store = rustls::RootCertStore::empty();

    if let Some(ca) = ca_path {
        let ca_certs = load_certs(ca)?;
        for cert in ca_certs {
            root_store.add(cert)?;
        }
    }

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(config)
}

// ---------------------------------------------------------------------------
// TlsTcpTransport — TLS-wrapped server side
// ---------------------------------------------------------------------------

/// TCP listener with TLS encryption. Accepts incoming TLS connections and
/// dispatches requests to an `IncomingHandler`, same as `TcpTransport` but
/// with encrypted channels.
pub struct TlsTcpTransport {
    bind_addr: SocketAddr,
    tls_config: Arc<rustls::ServerConfig>,
    rate_limiter: Arc<Mutex<RateLimiter>>,
}

impl TlsTcpTransport {
    pub fn new(
        bind_addr: SocketAddr,
        tls: &TlsConfig,
    ) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let server_config = build_server_tls_config(tls)?;
        Ok(Self {
            bind_addr,
            tls_config: Arc::new(server_config),
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(&RateLimitConfig::default()))),
        })
    }

    /// Start the TLS TCP listener. Same interface as `TcpTransport::listen`.
    pub async fn listen(
        self,
        handler: Arc<dyn IncomingHandler>,
    ) -> std::result::Result<(), std::io::Error> {
        let listener = TcpListener::bind(self.bind_addr).await?;
        let acceptor = tokio_rustls::TlsAcceptor::from(self.tls_config);
        info!(addr = %self.bind_addr, "TLS TCP transport listening");

        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    warn!(error = %e, "failed to accept TCP connection");
                    continue;
                }
            };

            let acceptor = acceptor.clone();
            let handler = Arc::clone(&handler);
            let rate_limiter = Arc::clone(&self.rate_limiter);
            tokio::spawn(async move {
                debug!(peer = %peer_addr, "accepted TCP connection, performing TLS handshake");
                match acceptor.accept(stream).await {
                    Ok(tls_stream) => {
                        if let Err(e) =
                            handle_tls_connection(tls_stream, &*handler, &rate_limiter, &peer_addr)
                                .await
                        {
                            debug!(peer = %peer_addr, error = %e, "TLS connection handler finished with error");
                        }
                    }
                    Err(e) => {
                        debug!(peer = %peer_addr, error = %e, "TLS handshake failed");
                    }
                }
            });
        }
    }
}

/// Handle a single TLS connection: check rate limits, read one request,
/// dispatch, write response.
async fn handle_tls_connection(
    mut stream: tokio_rustls::server::TlsStream<TcpStream>,
    handler: &dyn IncomingHandler,
    rate_limiter: &Arc<Mutex<RateLimiter>>,
    peer_addr: &SocketAddr,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let peer_key = peer_addr.ip().to_string();
    let decision = {
        let mut limiter = rate_limiter.lock().unwrap();
        limiter.check(&peer_key)
    };

    match decision {
        RateDecision::Allow => {}
        RateDecision::Throttle { wait_ms } => {
            debug!(peer = %peer_addr, wait_ms, "rate-limiting TLS peer (throttle)");
            let response = TransportResponse::Error {
                details: format!("rate limited: retry after {}ms", wait_ms),
            };
            let resp_bytes = serde_json::to_vec(&response)?;
            write_frame_async(&mut stream, &resp_bytes).await?;
            return Ok(());
        }
        RateDecision::Deny => {
            warn!(peer = %peer_addr, "rate-limiting TLS peer (deny)");
            let response = TransportResponse::Error {
                details: "rate limited: too many requests".to_string(),
            };
            let resp_bytes = serde_json::to_vec(&response)?;
            write_frame_async(&mut stream, &resp_bytes).await?;
            return Ok(());
        }
        RateDecision::Blacklisted => {
            warn!(peer = %peer_addr, "rejecting blacklisted TLS peer");
            return Ok(());
        }
    }

    let req_bytes = read_frame_async(&mut stream).await?;
    let msg: TransportMessage = serde_json::from_slice(&req_bytes)?;

    debug!(msg_type = ?std::mem::discriminant(&msg), "dispatching incoming TLS request");
    let response = handler.handle(msg);

    let resp_bytes = serde_json::to_vec(&response)?;
    write_frame_async(&mut stream, &resp_bytes).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// TlsTcpRemoteExecutor — TLS-wrapped client side
// ---------------------------------------------------------------------------

/// Implements `RemoteExecutor` by opening a TLS-wrapped TCP connection to the
/// target peer, sending a length-prefixed JSON message, and reading the response.
///
/// Falls through to the same wire protocol as `TcpRemoteExecutor`, but all
/// traffic is encrypted.
pub struct TlsTcpRemoteExecutor {
    peer_addresses: PeerAddressMap,
    client_config: Arc<rustls::ClientConfig>,
}

impl TlsTcpRemoteExecutor {
    pub fn new(
        peer_addresses: PeerAddressMap,
        tls: &TlsConfig,
    ) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let client_config = build_client_tls_config(tls.ca_path.as_deref())?;
        Ok(Self {
            peer_addresses,
            client_config: Arc::new(client_config),
        })
    }

    /// Send a request over TLS to the given peer and return the parsed response.
    fn send_request(
        &self,
        peer_id: &str,
        msg: &TransportMessage,
    ) -> Result<TransportResponse> {
        let addr = {
            let map = self.peer_addresses.lock().unwrap();
            map.get(peer_id).copied().ok_or_else(|| SomaError::Distributed {
                failure: DistributedFailure::PeerUnreachable,
                details: format!("no address registered for peer '{}'", peer_id),
            })?
        };

        // Use a per-call tokio runtime for the async TLS handshake + I/O.
        // This mirrors TcpRemoteExecutor's blocking approach.
        let rt = tokio::runtime::Runtime::new().map_err(|e| SomaError::Distributed {
            failure: DistributedFailure::TransportFailure,
            details: format!("failed to create tokio runtime for TLS: {}", e),
        })?;

        let client_config = Arc::clone(&self.client_config);
        let payload = serde_json::to_vec(msg).map_err(|e| SomaError::Distributed {
            failure: DistributedFailure::TransportFailure,
            details: format!("failed to serialize request: {}", e),
        })?;

        // Use the peer address as the SNI server name.
        let server_name = rustls::pki_types::ServerName::try_from(addr.ip().to_string())
            .map_err(|e| SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("invalid TLS server name for {}: {}", addr, e),
            })?
            .to_owned();

        rt.block_on(async {
            let connector = tokio_rustls::TlsConnector::from(client_config);
            let tcp_stream = TcpStream::connect(addr).await.map_err(|e| {
                SomaError::Distributed {
                    failure: DistributedFailure::TransportFailure,
                    details: format!("TCP connect to {} (peer {}) failed: {}", addr, peer_id, e),
                }
            })?;

            let mut tls_stream = connector
                .connect(server_name, tcp_stream)
                .await
                .map_err(|e| SomaError::Distributed {
                    failure: DistributedFailure::TransportFailure,
                    details: format!(
                        "TLS handshake with {} (peer {}) failed: {}",
                        addr, peer_id, e
                    ),
                })?;

            write_frame_async(&mut tls_stream, &payload)
                .await
                .map_err(|e| SomaError::Distributed {
                    failure: DistributedFailure::TransportFailure,
                    details: format!("failed to send TLS frame to peer {}: {}", peer_id, e),
                })?;

            let resp_bytes = read_frame_async(&mut tls_stream)
                .await
                .map_err(|e| SomaError::Distributed {
                    failure: DistributedFailure::TransportFailure,
                    details: format!(
                        "failed to read TLS response from peer {}: {}",
                        peer_id, e
                    ),
                })?;

            let response: TransportResponse =
                serde_json::from_slice(&resp_bytes).map_err(|e| SomaError::Distributed {
                    failure: DistributedFailure::TransportFailure,
                    details: format!(
                        "failed to deserialize response from peer {}: {}",
                        peer_id, e
                    ),
                })?;

            Ok(response)
        })
    }

    fn check_error(response: &TransportResponse) -> Result<()> {
        if let TransportResponse::Error { details } = response {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: details.clone(),
            });
        }
        Ok(())
    }

    /// Send a Ping to the given peer over TLS and return the Pong response.
    pub fn send_ping(&self, peer_id: &str, nonce: u64) -> Result<TransportResponse> {
        let msg = TransportMessage::Ping { nonce };
        self.send_request(peer_id, &msg)
    }
}

impl RemoteExecutor for TlsTcpRemoteExecutor {
    fn submit_goal(
        &self,
        peer_id: &str,
        request: &RemoteGoalRequest,
    ) -> Result<RemoteGoalResponse> {
        let msg = TransportMessage::SubmitGoal {
            peer_id: peer_id.to_string(),
            request: request.clone(),
        };
        let response = self.send_request(peer_id, &msg)?;
        Self::check_error(&response)?;
        match response {
            TransportResponse::GoalResult { response } => Ok(response),
            other => Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("unexpected response type for submit_goal: {:?}", other),
            }),
        }
    }

    fn invoke_skill(
        &self,
        peer_id: &str,
        skill_id: &str,
        input: serde_json::Value,
    ) -> Result<RemoteSkillResponse> {
        let msg = TransportMessage::InvokeSkill {
            peer_id: peer_id.to_string(),
            skill_id: skill_id.to_string(),
            input,
        };
        let response = self.send_request(peer_id, &msg)?;
        Self::check_error(&response)?;
        match response {
            TransportResponse::SkillResult { mut response } => {
                // Smaller peers (e.g. ESP32 leaf firmware) don't fill in
                // peer_id on the wire. Patch it in post-receive so the
                // returned record carries the caller-side peer ID.
                if response.peer_id.is_empty() {
                    response.peer_id = peer_id.to_string();
                }
                Ok(response)
            }
            other => Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("unexpected response type for invoke_skill: {:?}", other),
            }),
        }
    }

    fn query_resource(
        &self,
        peer_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<RemoteResourceResponse> {
        let msg = TransportMessage::QueryResource {
            peer_id: peer_id.to_string(),
            resource_type: resource_type.to_string(),
            resource_id: resource_id.to_string(),
        };
        let response = self.send_request(peer_id, &msg)?;
        Self::check_error(&response)?;
        match response {
            TransportResponse::ResourceResult { response } => Ok(response),
            other => Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("unexpected response type for query_resource: {:?}", other),
            }),
        }
    }

    fn transfer_schema(&self, peer_id: &str, schema: &SchemaTransfer) -> Result<()> {
        let msg = TransportMessage::TransferSchema {
            peer_id: peer_id.to_string(),
            schema: schema.clone(),
        };
        let response = self.send_request(peer_id, &msg)?;
        Self::check_error(&response)?;
        match response {
            TransportResponse::SchemaOk => Ok(()),
            other => Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("unexpected response type for transfer_schema: {:?}", other),
            }),
        }
    }

    fn transfer_routine(&self, peer_id: &str, routine: &RoutineTransfer) -> Result<()> {
        let payload = serde_json::to_vec(routine).map_err(|e| SomaError::Distributed {
            failure: DistributedFailure::TransportFailure,
            details: format!("failed to serialize routine: {}", e),
        })?;

        if payload.len() > 64 * 1024 {
            let sender = super::chunked::ChunkedSender::default();
            let (manifest, chunks) = sender.prepare(&payload);

            let manifest_msg = TransportMessage::ChunkedTransferStart {
                peer_id: peer_id.to_string(),
                manifest: manifest.clone(),
            };
            let resp = self.send_request(peer_id, &manifest_msg)?;
            Self::check_error(&resp)?;

            for chunk in &chunks {
                let chunk_msg = TransportMessage::ChunkedTransferData {
                    peer_id: peer_id.to_string(),
                    chunk: chunk.clone(),
                };
                let resp = self.send_request(peer_id, &chunk_msg)?;
                Self::check_error(&resp)?;
            }

            Ok(())
        } else {
            let msg = TransportMessage::TransferRoutine {
                peer_id: peer_id.to_string(),
                routine: routine.clone(),
            };
            let response = self.send_request(peer_id, &msg)?;
            Self::check_error(&response)?;
            match response {
                TransportResponse::RoutineOk | TransportResponse::RoutineStored { .. } => Ok(()),
                other => Err(SomaError::Distributed {
                    failure: DistributedFailure::TransportFailure,
                    details: format!("unexpected response type for transfer_routine: {:?}", other),
                }),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience: start TLS listener in a background thread
// ---------------------------------------------------------------------------

/// Start the TLS TCP listener on a new thread with a dedicated tokio runtime.
/// Returns a handle to the thread.
pub fn start_tls_listener_background(
    bind_addr: SocketAddr,
    handler: Arc<dyn IncomingHandler>,
    tls: &TlsConfig,
) -> std::result::Result<std::thread::JoinHandle<()>, Box<dyn std::error::Error>> {
    let transport = TlsTcpTransport::new(bind_addr, tls)?;
    let handle = std::thread::Builder::new()
        .name("soma-tls-tcp-listener".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new()
                .expect("failed to create tokio runtime for TLS TCP listener");
            rt.block_on(async {
                if let Err(e) = transport.listen(handler).await {
                    error!(error = %e, "TLS TCP listener failed");
                }
            });
        })
        .expect("failed to spawn TLS TCP listener thread");
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener as StdTcpListener;

    /// A trivial handler that echoes the skill_id back in the response.
    struct EchoHandler;

    impl IncomingHandler for EchoHandler {
        fn handle(&self, msg: TransportMessage) -> TransportResponse {
            match msg {
                TransportMessage::InvokeSkill {
                    peer_id: _,
                    skill_id,
                    input: _,
                } => TransportResponse::SkillResult {
                    response: RemoteSkillResponse {
                        skill_id: skill_id.clone(),
                        peer_id: "echo".to_string(),
                        success: true,
                        observation: serde_json::json!({"echoed_skill": skill_id}),
                        latency_ms: 0,
                        timestamp: Utc::now(),
                        trace_id: Uuid::new_v4(),
                    },
                },
                TransportMessage::SubmitGoal { .. } => TransportResponse::GoalResult {
                    response: RemoteGoalResponse {
                        status: RemoteGoalStatus::Accepted,
                        session_id: Some("test-session".to_string()),
                        reason: None,
                        required_adjustments: None,
                    },
                },
                TransportMessage::QueryResource {
                    resource_type,
                    resource_id,
                    ..
                } => TransportResponse::ResourceResult {
                    response: RemoteResourceResponse {
                        resource_type,
                        resource_id,
                        data: serde_json::json!({"test": true}),
                        data_mode: ResourceDataMode::Snapshot,
                        version: 1,
                        provenance: "echo".to_string(),
                        freshness_ms: 0,
                        timestamp: Utc::now(),
                    },
                },
                TransportMessage::TransferSchema { .. } => TransportResponse::SchemaOk,
                TransportMessage::TransferRoutine { .. } => TransportResponse::RoutineOk,
                TransportMessage::ChunkedTransferStart { manifest, .. } => {
                    TransportResponse::ChunkedTransferAccepted {
                        transfer_id: manifest.transfer_id,
                    }
                }
                TransportMessage::ChunkedTransferData { chunk, .. } => {
                    TransportResponse::ChunkedChunkAck {
                        transfer_id: chunk.transfer_id,
                        chunk_index: chunk.chunk_index,
                    }
                }
                TransportMessage::ChunkedTransferResume { resume, .. } => {
                    TransportResponse::ChunkedResumeMissing {
                        transfer_id: resume.transfer_id,
                        missing_indices: vec![],
                    }
                }
                TransportMessage::Ping { nonce } => {
                    TransportResponse::Pong { nonce, load: 0.42 }
                }
                TransportMessage::ListCapabilities => TransportResponse::Capabilities {
                    primitives: vec![],
                    routines: vec![],
                },
                TransportMessage::RemoveRoutine { routine_id } => {
                    TransportResponse::RoutineRemoved { routine_id }
                }
            }
        }
    }

    /// Find an available port on localhost.
    fn free_port() -> u16 {
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    #[test]
    fn roundtrip_invoke_skill() {
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        // Start listener in background.
        let _listener_handle = start_listener_background(addr, handler);

        // Give the listener a moment to bind.
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Create the client executor with the peer address mapped.
        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("peer-1".to_string(), addr);

        let executor = TcpRemoteExecutor::new(peer_map);

        let result = executor.invoke_skill("peer-1", "file.list", serde_json::json!({"path": "/tmp"}));
        let resp = result.expect("invoke_skill should succeed");
        assert_eq!(resp.skill_id, "file.list");
        assert!(resp.success);
        assert_eq!(resp.peer_id, "echo");
    }

    #[test]
    fn roundtrip_submit_goal() {
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle = start_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("peer-1".to_string(), addr);

        let executor = TcpRemoteExecutor::new(peer_map);

        let request = RemoteGoalRequest {
            goal: serde_json::json!("list files"),
            constraints: vec![],
            budgets: crate::types::peer::RemoteBudget {
                risk_limit: 0.5,
                latency_limit_ms: 5000,
                resource_limit: 100.0,
                step_limit: 10,
            },
            trust_required: crate::types::common::TrustLevel::Verified,
            request_result: true,
            request_trace: false,
        };

        let result = executor.submit_goal("peer-1", &request);
        let resp = result.expect("submit_goal should succeed");
        assert_eq!(resp.status, RemoteGoalStatus::Accepted);
        assert_eq!(resp.session_id, Some("test-session".to_string()));
    }

    #[test]
    fn roundtrip_query_resource() {
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle = start_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("peer-1".to_string(), addr);

        let executor = TcpRemoteExecutor::new(peer_map);

        let result = executor.query_resource("peer-1", "filesystem", "root");
        let resp = result.expect("query_resource should succeed");
        assert_eq!(resp.resource_type, "filesystem");
        assert_eq!(resp.resource_id, "root");
        assert_eq!(resp.provenance, "echo");
    }

    #[test]
    fn roundtrip_transfer_schema() {
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle = start_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("peer-1".to_string(), addr);

        let executor = TcpRemoteExecutor::new(peer_map);

        let schema = SchemaTransfer {
            schema_id: "s1".to_string(),
            version: "1.0".to_string(),
            trigger_conditions: vec![],
            subgoal_structure: vec![],
            candidate_skill_ordering: vec![],
            stop_conditions: vec![],
            confidence: 0.9,
        };

        let result = executor.transfer_schema("peer-1", &schema);
        assert!(result.is_ok());
    }

    #[test]
    fn roundtrip_transfer_routine() {
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle = start_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("peer-1".to_string(), addr);

        let executor = TcpRemoteExecutor::new(peer_map);

        let routine = RoutineTransfer {
            routine_id: "r1".to_string(),
            match_conditions: vec![],
            compiled_skill_path: vec![],
            compiled_steps: vec![],
            guard_conditions: vec![],
            expected_cost: 1.0,
            expected_effect: vec![],
            confidence: 0.8,
            autonomous: false,
            priority: 0,
            exclusive: false,
            policy_scope: None,
            version: 0,
        };

        let result = executor.transfer_routine("peer-1", &routine);
        assert!(result.is_ok());
    }

    #[test]
    fn unknown_peer_returns_error() {
        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        let executor = TcpRemoteExecutor::new(peer_map);

        let result = executor.invoke_skill("ghost", "file.list", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::PeerUnreachable);
                assert!(details.contains("ghost"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn connection_refused_returns_transport_failure() {
        // Use a port that nothing is listening on.
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();

        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("peer-1".to_string(), addr);

        let executor = TcpRemoteExecutor::new(peer_map);

        let result = executor.invoke_skill("peer-1", "file.list", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TransportFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn tls_config_from_distributed_section() {
        use crate::config::SomaConfig;

        let toml_str = r#"
[distributed]
tls_cert = "/tmp/test-cert.pem"
tls_key = "/tmp/test-key.pem"
tls_ca = "/tmp/test-ca.pem"
"#;
        let cfg: SomaConfig = toml::from_str(toml_str).unwrap();
        let tls = cfg.distributed.tls_config();
        assert!(tls.is_some());
        let tls = tls.unwrap();
        assert_eq!(tls.cert_path, "/tmp/test-cert.pem");
        assert_eq!(tls.key_path, "/tmp/test-key.pem");
        assert_eq!(tls.ca_path.as_deref(), Some("/tmp/test-ca.pem"));
    }

    #[test]
    fn tls_config_none_without_key() {
        use crate::config::SomaConfig;

        let toml_str = r#"
[distributed]
tls_cert = "/tmp/test-cert.pem"
"#;
        let cfg: SomaConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.distributed.tls_config().is_none());
    }

    #[test]
    fn tls_config_none_when_section_absent() {
        use crate::config::SomaConfig;

        let toml_str = r#"
[soma]
id = "test"
"#;
        let cfg: SomaConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.distributed.tls_config().is_none());
    }

    #[test]
    fn tls_config_ca_optional() {
        use crate::config::SomaConfig;

        let toml_str = r#"
[distributed]
tls_cert = "/tmp/cert.pem"
tls_key = "/tmp/key.pem"
"#;
        let cfg: SomaConfig = toml::from_str(toml_str).unwrap();
        let tls = cfg.distributed.tls_config();
        assert!(tls.is_some());
        let tls = tls.unwrap();
        assert_eq!(tls.cert_path, "/tmp/cert.pem");
        assert_eq!(tls.key_path, "/tmp/key.pem");
        assert!(tls.ca_path.is_none());
    }

    #[test]
    fn tls_transport_rejects_missing_cert() {
        let tls = TlsConfig {
            cert_path: "/nonexistent/cert.pem".to_string(),
            key_path: "/nonexistent/key.pem".to_string(),
            ca_path: None,
        };
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let result = TlsTcpTransport::new(addr, &tls);
        assert!(result.is_err());
    }

    #[test]
    fn tls_executor_rejects_missing_cert() {
        let tls = TlsConfig {
            cert_path: "/nonexistent/cert.pem".to_string(),
            key_path: "/nonexistent/key.pem".to_string(),
            ca_path: Some("/nonexistent/ca.pem".to_string()),
        };
        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        // The executor only loads the CA, not the server cert/key. When the
        // CA path is invalid, construction should fail.
        let result = TlsTcpRemoteExecutor::new(peer_map, &tls);
        assert!(result.is_err());
    }

    #[test]
    fn tls_executor_ok_without_ca() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let tls = TlsConfig {
            cert_path: String::new(),
            key_path: String::new(),
            ca_path: None,
        };
        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        // No CA means empty root store, which is valid (client construction succeeds).
        let result = TlsTcpRemoteExecutor::new(peer_map, &tls);
        assert!(result.is_ok());
    }
}
