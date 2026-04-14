use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use crate::errors::{Result, SomaError};
use crate::types::peer::{DistributedFailure, RemoteGoalRequest, RoutineTransfer, SchemaTransfer};

use super::remote::{
    RemoteExecutor, RemoteGoalResponse, RemoteResourceResponse, RemoteSkillResponse,
};
use super::transport::{IncomingHandler, TransportMessage, TransportResponse};

// ---------------------------------------------------------------------------
// WsRemoteExecutor — client side
// ---------------------------------------------------------------------------

/// Maps peer IDs to their WebSocket addresses so the executor knows where
/// to connect.
pub type WsPeerAddressMap = Arc<Mutex<HashMap<String, SocketAddr>>>;

/// Implements `RemoteExecutor` by opening a WebSocket connection to the target
/// peer, sending a JSON text message, and reading the response. Uses the same
/// `TransportMessage`/`TransportResponse` types as the TCP transport.
///
/// Each call opens a fresh connection. Connection pooling can be added later
/// without changing the trait interface.
pub struct WsRemoteExecutor {
    peer_addresses: WsPeerAddressMap,
}

impl WsRemoteExecutor {
    pub fn new(peer_addresses: WsPeerAddressMap) -> Self {
        Self { peer_addresses }
    }

    /// Send a request to the given peer and return the parsed response.
    /// Uses a blocking tokio runtime since RemoteExecutor is a sync trait.
    fn send_request(&self, peer_id: &str, msg: &TransportMessage) -> Result<TransportResponse> {
        let addr = {
            let map = self.peer_addresses.lock().unwrap();
            map.get(peer_id).copied().ok_or_else(|| SomaError::Distributed {
                failure: DistributedFailure::PeerUnreachable,
                details: format!("no WebSocket address registered for peer '{}'", peer_id),
            })?
        };

        let url = format!("ws://{}", addr);
        let payload = serde_json::to_string(msg).map_err(|e| SomaError::Distributed {
            failure: DistributedFailure::TransportFailure,
            details: format!("failed to serialize request: {}", e),
        })?;

        // Run the async WebSocket call in a blocking context.
        let rt = tokio::runtime::Runtime::new().map_err(|e| SomaError::Distributed {
            failure: DistributedFailure::TransportFailure,
            details: format!("failed to create tokio runtime: {}", e),
        })?;

        rt.block_on(async {
            let (mut ws_stream, _) =
                tokio_tungstenite::connect_async(&url)
                    .await
                    .map_err(|e| SomaError::Distributed {
                        failure: DistributedFailure::TransportFailure,
                        details: format!(
                            "WebSocket connect to {} (peer {}) failed: {}",
                            url, peer_id, e
                        ),
                    })?;

            ws_stream
                .send(Message::Text(payload))
                .await
                .map_err(|e| SomaError::Distributed {
                    failure: DistributedFailure::TransportFailure,
                    details: format!("failed to send WebSocket message to peer {}: {}", peer_id, e),
                })?;

            let resp_msg = ws_stream.next().await.ok_or_else(|| SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("WebSocket stream closed before response from peer {}", peer_id),
            })?.map_err(|e| SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!("failed to read WebSocket response from peer {}: {}", peer_id, e),
            })?;

            let text = match resp_msg {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => String::from_utf8(b.to_vec()).map_err(|e| {
                    SomaError::Distributed {
                        failure: DistributedFailure::TransportFailure,
                        details: format!("invalid UTF-8 in WebSocket response: {}", e),
                    }
                })?,
                other => {
                    return Err(SomaError::Distributed {
                        failure: DistributedFailure::TransportFailure,
                        details: format!("unexpected WebSocket message type: {:?}", other),
                    });
                }
            };

            let response: TransportResponse =
                serde_json::from_str(&text).map_err(|e| SomaError::Distributed {
                    failure: DistributedFailure::TransportFailure,
                    details: format!(
                        "failed to deserialize WebSocket response from peer {}: {}",
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
}

impl RemoteExecutor for WsRemoteExecutor {
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
// WsTransport — server side (WebSocket listener + request dispatch)
// ---------------------------------------------------------------------------

/// WebSocket listener that accepts incoming connections and dispatches requests
/// to an `IncomingHandler`. Runs as a background tokio task. Uses the same
/// `TransportMessage`/`TransportResponse` wire types as the TCP transport,
/// but frames them as WebSocket text messages instead of length-prefixed TCP.
pub struct WsTransport {
    bind_addr: SocketAddr,
}

impl WsTransport {
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self { bind_addr }
    }

    /// Start the WebSocket listener. Accepts connections in a loop, spawning a
    /// task per connection.
    pub async fn listen(
        self,
        handler: Arc<dyn IncomingHandler>,
    ) -> std::result::Result<(), std::io::Error> {
        let listener = TcpListener::bind(self.bind_addr).await?;
        info!(addr = %self.bind_addr, "WebSocket transport listening");

        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    warn!(error = %e, "failed to accept TCP connection for WebSocket");
                    continue;
                }
            };

            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                debug!(peer = %peer_addr, "accepted WebSocket connection");
                if let Err(e) = handle_ws_connection(stream, &*handler).await {
                    debug!(
                        peer = %peer_addr,
                        error = %e,
                        "WebSocket connection handler finished with error"
                    );
                }
            });
        }
    }
}

/// Handle a single WebSocket connection: perform the upgrade handshake,
/// then read messages and dispatch to the handler.
async fn handle_ws_connection(
    stream: tokio::net::TcpStream,
    handler: &dyn IncomingHandler,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut ws_stream = tokio_tungstenite::accept_async(stream).await?;

    // Process messages until the client disconnects.
    while let Some(msg_result) = ws_stream.next().await {
        let msg = match msg_result {
            Ok(m) => m,
            Err(e) => {
                debug!(error = %e, "WebSocket read error");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                let transport_msg: TransportMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        let error_resp = TransportResponse::Error {
                            details: format!("invalid JSON: {}", e),
                        };
                        let resp_text = serde_json::to_string(&error_resp)?;
                        ws_stream.send(Message::Text(resp_text)).await?;
                        continue;
                    }
                };

                debug!(
                    msg_type = ?std::mem::discriminant(&transport_msg),
                    "dispatching incoming WebSocket request"
                );
                let response = handler.handle(transport_msg);
                let resp_text = serde_json::to_string(&response)?;
                ws_stream.send(Message::Text(resp_text)).await?;
            }
            Message::Binary(data) => {
                // Accept binary frames as JSON too.
                let text = String::from_utf8(data.to_vec())?;
                let transport_msg: TransportMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        let error_resp = TransportResponse::Error {
                            details: format!("invalid JSON in binary frame: {}", e),
                        };
                        let resp_text = serde_json::to_string(&error_resp)?;
                        ws_stream.send(Message::Text(resp_text)).await?;
                        continue;
                    }
                };

                let response = handler.handle(transport_msg);
                let resp_text = serde_json::to_string(&response)?;
                ws_stream.send(Message::Text(resp_text)).await?;
            }
            Message::Ping(payload) => {
                ws_stream.send(Message::Pong(payload)).await?;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    Ok(())
}

/// Start the WebSocket listener on a new thread with a dedicated tokio runtime.
/// Returns a handle to the thread. The listener runs until the process exits.
pub fn start_ws_listener_background(
    bind_addr: SocketAddr,
    handler: Arc<dyn IncomingHandler>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("soma-ws-listener".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new()
                .expect("failed to create tokio runtime for WebSocket listener");
            rt.block_on(async {
                let transport = WsTransport::new(bind_addr);
                if let Err(e) = transport.listen(handler).await {
                    error!(error = %e, "WebSocket listener failed");
                }
            });
        })
        .expect("failed to spawn WebSocket listener thread")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;
    use crate::distributed::remote::{RemoteGoalStatus, ResourceDataMode};
    use crate::distributed::transport::{IncomingHandler, TransportMessage, TransportResponse};
    use crate::types::peer::{RemoteBudget, RemoteGoalRequest, RoutineTransfer, SchemaTransfer};
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
                        session_id: Some("ws-test-session".to_string()),
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
                        provenance: "ws-echo".to_string(),
                        freshness_ms: 0,
                        timestamp: Utc::now(),
                    },
                },
                TransportMessage::TransferSchema { .. } => TransportResponse::SchemaOk,
                TransportMessage::TransferRoutine { .. } => TransportResponse::RoutineOk,
                TransportMessage::Ping { nonce } => TransportResponse::Pong { nonce, load: 0.0 },
                _ => TransportResponse::Error {
                    details: "unsupported message type".to_string(),
                },
            }
        }
    }

    fn free_port() -> u16 {
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    #[test]
    fn ws_roundtrip_invoke_skill() {
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle = start_ws_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(200));

        let peer_map: WsPeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("ws-peer-1".to_string(), addr);

        let executor = WsRemoteExecutor::new(peer_map);

        let result =
            executor.invoke_skill("ws-peer-1", "file.list", serde_json::json!({"path": "/tmp"}));
        let resp = result.expect("invoke_skill should succeed over WebSocket");
        assert_eq!(resp.skill_id, "file.list");
        assert!(resp.success);
        assert_eq!(resp.peer_id, "echo");
    }

    #[test]
    fn ws_roundtrip_submit_goal() {
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle = start_ws_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(200));

        let peer_map: WsPeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("ws-peer-1".to_string(), addr);

        let executor = WsRemoteExecutor::new(peer_map);

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

        let result = executor.submit_goal("ws-peer-1", &request);
        let resp = result.expect("submit_goal should succeed over WebSocket");
        assert_eq!(resp.status, RemoteGoalStatus::Accepted);
        assert_eq!(resp.session_id, Some("ws-test-session".to_string()));
    }

    #[test]
    fn ws_roundtrip_query_resource() {
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle = start_ws_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(200));

        let peer_map: WsPeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("ws-peer-1".to_string(), addr);

        let executor = WsRemoteExecutor::new(peer_map);

        let result = executor.query_resource("ws-peer-1", "filesystem", "root");
        let resp = result.expect("query_resource should succeed over WebSocket");
        assert_eq!(resp.resource_type, "filesystem");
        assert_eq!(resp.resource_id, "root");
        assert_eq!(resp.provenance, "ws-echo");
    }

    #[test]
    fn ws_roundtrip_transfer_schema() {
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle = start_ws_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(200));

        let peer_map: WsPeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("ws-peer-1".to_string(), addr);

        let executor = WsRemoteExecutor::new(peer_map);

        let schema = SchemaTransfer {
            schema_id: "s1".to_string(),
            version: "1.0".to_string(),
            trigger_conditions: vec![],
            subgoal_structure: vec![],
            candidate_skill_ordering: vec![],
            stop_conditions: vec![],
            confidence: 0.9,
        };

        let result = executor.transfer_schema("ws-peer-1", &schema);
        assert!(result.is_ok());
    }

    #[test]
    fn ws_roundtrip_transfer_routine() {
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle = start_ws_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(200));

        let peer_map: WsPeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("ws-peer-1".to_string(), addr);

        let executor = WsRemoteExecutor::new(peer_map);

        let routine = RoutineTransfer {
            routine_id: "r1".to_string(),
            match_conditions: vec![],
            compiled_skill_path: vec![],
            guard_conditions: vec![],
            expected_cost: 1.0,
            expected_effect: vec![],
            confidence: 0.8,
            autonomous: false,
        };

        let result = executor.transfer_routine("ws-peer-1", &routine);
        assert!(result.is_ok());
    }

    #[test]
    fn ws_unknown_peer_returns_error() {
        let peer_map: WsPeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        let executor = WsRemoteExecutor::new(peer_map);

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
    fn ws_connection_refused_returns_transport_failure() {
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();

        let peer_map: WsPeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("ws-peer-1".to_string(), addr);

        let executor = WsRemoteExecutor::new(peer_map);

        let result = executor.invoke_skill("ws-peer-1", "file.list", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TransportFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn ws_multi_message_on_single_connection() {
        // Verify that the server handles multiple messages on a persistent
        // WebSocket connection (unlike TCP which is one-shot).
        let port = free_port();
        let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(EchoHandler);

        let _listener_handle = start_ws_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(200));

        let url = format!("ws://127.0.0.1:{}", port);

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

            // Send two messages on the same connection.
            let msg1 = TransportMessage::InvokeSkill {
                peer_id: "p1".to_string(),
                skill_id: "skill.a".to_string(),
                input: serde_json::json!({}),
            };
            let msg2 = TransportMessage::InvokeSkill {
                peer_id: "p1".to_string(),
                skill_id: "skill.b".to_string(),
                input: serde_json::json!({}),
            };

            ws.send(Message::Text(serde_json::to_string(&msg1).unwrap()))
                .await
                .unwrap();
            let resp1 = ws.next().await.unwrap().unwrap();
            let r1: TransportResponse = serde_json::from_str(resp1.to_text().unwrap()).unwrap();

            ws.send(Message::Text(serde_json::to_string(&msg2).unwrap()))
                .await
                .unwrap();
            let resp2 = ws.next().await.unwrap().unwrap();
            let r2: TransportResponse = serde_json::from_str(resp2.to_text().unwrap()).unwrap();

            match r1 {
                TransportResponse::SkillResult { response } => {
                    assert_eq!(response.skill_id, "skill.a");
                }
                other => panic!("unexpected: {:?}", other),
            }
            match r2 {
                TransportResponse::SkillResult { response } => {
                    assert_eq!(response.skill_id, "skill.b");
                }
                other => panic!("unexpected: {:?}", other),
            }
        });
    }
}
