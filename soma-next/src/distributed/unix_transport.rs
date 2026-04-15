use std::collections::HashMap;
use std::io::{Read as IoRead, Write as IoWrite};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, error, info, warn};

use crate::errors::{Result, SomaError};
use crate::types::peer::DistributedFailure;

use super::remote::{
    RemoteExecutor, RemoteGoalResponse, RemoteResourceResponse, RemoteSkillResponse,
};
use super::transport::{IncomingHandler, TransportMessage, TransportResponse};
use crate::types::peer::{RemoteGoalRequest, RoutineTransfer, SchemaTransfer};

// ---------------------------------------------------------------------------
// Framing helpers: 4-byte big-endian length prefix + JSON payload
// Same wire format as TCP so the protocol is transport-agnostic.
// ---------------------------------------------------------------------------

async fn write_frame(stream: &mut UnixStream, payload: &[u8]) -> std::io::Result<()> {
    let len = payload.len() as u32;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame(stream: &mut UnixStream) -> std::io::Result<Vec<u8>> {
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

/// Blocking framing for the sync RemoteExecutor trait methods.
fn write_frame_sync(stream: &mut StdUnixStream, payload: &[u8]) -> std::io::Result<()> {
    let len = payload.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(payload)?;
    stream.flush()?;
    Ok(())
}

fn read_frame_sync(stream: &mut StdUnixStream) -> std::io::Result<Vec<u8>> {
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
// UnixRemoteExecutor — client side
// ---------------------------------------------------------------------------

/// Maps peer IDs to their Unix socket paths so the executor knows where to connect.
pub type UnixPeerPathMap = Arc<Mutex<HashMap<String, PathBuf>>>;

/// Implements `RemoteExecutor` by connecting to a Unix domain socket, sending a
/// length-prefixed JSON message, and reading the response. Each call opens a
/// fresh connection, matching the TCP executor's behavior.
pub struct UnixRemoteExecutor {
    peer_paths: UnixPeerPathMap,
}

impl UnixRemoteExecutor {
    pub fn new(peer_paths: UnixPeerPathMap) -> Self {
        Self { peer_paths }
    }

    /// Send a request to the given peer and return the parsed response.
    fn send_request(
        &self,
        peer_id: &str,
        msg: &TransportMessage,
    ) -> Result<TransportResponse> {
        let path = {
            let map = self.peer_paths.lock().unwrap();
            map.get(peer_id).cloned().ok_or_else(|| SomaError::Distributed {
                failure: DistributedFailure::PeerUnreachable,
                details: format!("no Unix socket path registered for peer '{}'", peer_id),
            })?
        };

        let mut stream = StdUnixStream::connect(&path).map_err(|e| SomaError::Distributed {
            failure: DistributedFailure::TransportFailure,
            details: format!(
                "Unix connect to {} (peer {}) failed: {}",
                path.display(),
                peer_id,
                e
            ),
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
}

impl RemoteExecutor for UnixRemoteExecutor {
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
            TransportResponse::SkillResult { response } => Ok(response),
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
        let msg = TransportMessage::TransferRoutine {
            peer_id: peer_id.to_string(),
            routine: routine.clone(),
        };
        let response = self.send_request(peer_id, &msg)?;
        Self::check_error(&response)?;
        match response {
            TransportResponse::RoutineOk => Ok(()),
            other => Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("unexpected response type for transfer_routine: {:?}", other),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// UnixTransport — server side (listener + request dispatch)
// ---------------------------------------------------------------------------

/// Unix domain socket listener that accepts incoming connections and dispatches
/// requests to an `IncomingHandler`. Runs as a background tokio task.
pub struct UnixTransport {
    socket_path: PathBuf,
}

impl UnixTransport {
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Start the Unix listener. Returns only on fatal error. The listener
    /// accepts connections in a loop, spawning a task per connection. Any
    /// existing socket file at the path is removed before binding.
    pub async fn listen(
        self,
        handler: Arc<dyn IncomingHandler>,
    ) -> std::result::Result<(), std::io::Error> {
        // Remove stale socket file if it exists, so bind doesn't fail.
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        info!(path = %self.socket_path.display(), "Unix transport listening");

        loop {
            let (mut stream, _peer_addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    warn!(error = %e, "failed to accept Unix connection");
                    continue;
                }
            };

            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                debug!("accepted Unix connection");
                if let Err(e) = handle_unix_connection(&mut stream, &*handler).await {
                    debug!(error = %e, "Unix connection handler finished with error");
                }
            });
        }
    }
}

/// Handle a single Unix connection: read one request, dispatch, write response.
async fn handle_unix_connection(
    stream: &mut UnixStream,
    handler: &dyn IncomingHandler,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let req_bytes = read_frame(stream).await?;
    let msg: TransportMessage = serde_json::from_slice(&req_bytes)?;

    debug!(msg_type = ?std::mem::discriminant(&msg), "dispatching incoming Unix request");
    let response = handler.handle(msg);

    let resp_bytes = serde_json::to_vec(&response)?;
    write_frame(stream, &resp_bytes).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Convenience: start listener in a background thread with its own tokio runtime
// ---------------------------------------------------------------------------

/// Start the Unix socket listener on a new thread with a dedicated tokio runtime.
/// Returns a handle to the thread. The listener runs until the process exits.
/// Cleans up stale socket files before binding.
pub fn start_unix_listener_background(
    socket_path: PathBuf,
    handler: Arc<dyn IncomingHandler>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("soma-unix-listener".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new()
                .expect("failed to create tokio runtime for Unix listener");
            rt.block_on(async {
                let transport = UnixTransport::new(socket_path);
                if let Err(e) = transport.listen(handler).await {
                    error!(error = %e, "Unix listener failed");
                }
            });
        })
        .expect("failed to spawn Unix listener thread")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributed::remote::{RemoteGoalStatus, ResourceDataMode};
    use crate::distributed::transport::TransportResponse;
    use crate::types::peer::{RemoteBudget, RemoteGoalRequest, RoutineTransfer, SchemaTransfer};
    use chrono::Utc;
    use uuid::Uuid;

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
                _ => TransportResponse::Error {
                    details: "unsupported message type".to_string(),
                },
            }
        }
    }

    /// Create a unique temp socket path for each test.
    fn temp_socket_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!("soma-test-{}-{}.sock", name, std::process::id()))
    }

    #[test]
    fn roundtrip_invoke_skill_unix() {
        let sock_path = temp_socket_path("invoke-skill");
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle =
            start_unix_listener_background(sock_path.clone(), handler);

        // Give the listener a moment to bind.
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: UnixPeerPathMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map
            .lock()
            .unwrap()
            .insert("peer-1".to_string(), sock_path.clone());

        let executor = UnixRemoteExecutor::new(peer_map);

        let result =
            executor.invoke_skill("peer-1", "file.list", serde_json::json!({"path": "/tmp"}));
        let resp = result.expect("invoke_skill should succeed");
        assert_eq!(resp.skill_id, "file.list");
        assert!(resp.success);
        assert_eq!(resp.peer_id, "echo");

        // Cleanup
        let _ = std::fs::remove_file(&sock_path);
    }

    #[test]
    fn roundtrip_submit_goal_unix() {
        let sock_path = temp_socket_path("submit-goal");
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle =
            start_unix_listener_background(sock_path.clone(), handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: UnixPeerPathMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map
            .lock()
            .unwrap()
            .insert("peer-1".to_string(), sock_path.clone());

        let executor = UnixRemoteExecutor::new(peer_map);

        let request = RemoteGoalRequest {
            goal: serde_json::json!("list files"),
            constraints: vec![],
            budgets: RemoteBudget {
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

        let _ = std::fs::remove_file(&sock_path);
    }

    #[test]
    fn roundtrip_query_resource_unix() {
        let sock_path = temp_socket_path("query-resource");
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle =
            start_unix_listener_background(sock_path.clone(), handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: UnixPeerPathMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map
            .lock()
            .unwrap()
            .insert("peer-1".to_string(), sock_path.clone());

        let executor = UnixRemoteExecutor::new(peer_map);

        let result = executor.query_resource("peer-1", "filesystem", "root");
        let resp = result.expect("query_resource should succeed");
        assert_eq!(resp.resource_type, "filesystem");
        assert_eq!(resp.resource_id, "root");
        assert_eq!(resp.provenance, "echo");

        let _ = std::fs::remove_file(&sock_path);
    }

    #[test]
    fn roundtrip_transfer_schema_unix() {
        let sock_path = temp_socket_path("transfer-schema");
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle =
            start_unix_listener_background(sock_path.clone(), handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: UnixPeerPathMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map
            .lock()
            .unwrap()
            .insert("peer-1".to_string(), sock_path.clone());

        let executor = UnixRemoteExecutor::new(peer_map);

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

        let _ = std::fs::remove_file(&sock_path);
    }

    #[test]
    fn roundtrip_transfer_routine_unix() {
        let sock_path = temp_socket_path("transfer-routine");
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle =
            start_unix_listener_background(sock_path.clone(), handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: UnixPeerPathMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map
            .lock()
            .unwrap()
            .insert("peer-1".to_string(), sock_path.clone());

        let executor = UnixRemoteExecutor::new(peer_map);

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

        let _ = std::fs::remove_file(&sock_path);
    }

    #[test]
    fn unknown_peer_returns_error_unix() {
        let peer_map: UnixPeerPathMap = Arc::new(Mutex::new(HashMap::new()));
        let executor = UnixRemoteExecutor::new(peer_map);

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
    fn connection_refused_returns_transport_failure_unix() {
        // Use a path that nothing is listening on.
        let sock_path = temp_socket_path("no-listener");

        let peer_map: UnixPeerPathMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map
            .lock()
            .unwrap()
            .insert("peer-1".to_string(), sock_path);

        let executor = UnixRemoteExecutor::new(peer_map);

        let result = executor.invoke_skill("peer-1", "file.list", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TransportFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }
}
