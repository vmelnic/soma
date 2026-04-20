//! WebSocket transport for the MCP server, with reverse-port routing.
//!
//! Two responsibilities on one WebSocket:
//!
//! 1. **Forward MCP**: client → server JSON-RPC 2.0 requests, dispatched
//!    into the shared `McpServer::handle_request`. Same as stdio.
//!
//! 2. **Reverse-port routing**: the server can call *back into the client*
//!    to invoke ports that live on the client's hardware (phone camera,
//!    browser geolocation, etc.). Clients register their local ports with
//!    `reverse/register_ports`; the server later issues outbound
//!    `reverse/invoke_port` requests correlated by `id`, and clients reply
//!    with normal JSON-RPC responses on the same socket.
//!
//! Envelope types on the wire:
//!
//! - Incoming request  (client → server): JSON-RPC request with `method`.
//!   - Standard MCP methods (`tools/call`, `initialize`, …) dispatch to `McpServer`.
//!   - `reverse/register_ports` / `reverse/list_ports` are handled locally
//!     by the listener; they never reach `McpServer`.
//! - Incoming response (client → server): JSON-RPC response to a prior
//!   server-initiated `reverse/invoke_port`. Routed to the pending
//!   oneshot by `id`.
//! - Outgoing request  (server → client): JSON-RPC request with
//!   `method: "reverse/invoke_port"`. Server expects a matching response.
//!
//! Known limitations (follow-ups):
//! - The implicit-session tracker inside `McpServer` is globally shared
//!   across connections.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use super::mcp::{McpRequest, McpServer};

pub type ConnId = u64;

/// One port advertised by an attached device. `capabilities` is opaque to
/// the registry — forwarded verbatim into the eventual outbound request.
#[derive(Clone, Debug)]
pub struct LocalPortEntry {
    pub port_id: String,
    pub device_id: String,
    pub conn_id: ConnId,
    pub capabilities: Value,
}

/// Per-connection state: outbound channel + pending correlated requests.
struct ConnState {
    device_id: Mutex<Option<String>>,
    outbound: mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    next_req: AtomicU64,
}

/// Shared registry of attached devices, their local ports, and the
/// outbound channel to each. Cloneable via `Arc`.
#[derive(Clone)]
pub struct LocalPortRegistry {
    ports: Arc<Mutex<HashMap<String, LocalPortEntry>>>, // port_id -> entry
    conns: Arc<Mutex<HashMap<ConnId, Arc<ConnState>>>>,
    next_conn: Arc<AtomicU64>,
    invoke_timeout: Duration,
}

impl Default for LocalPortRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalPortRegistry {
    pub fn new() -> Self {
        Self {
            ports: Arc::new(Mutex::new(HashMap::new())),
            conns: Arc::new(Mutex::new(HashMap::new())),
            next_conn: Arc::new(AtomicU64::new(1)),
            invoke_timeout: Duration::from_secs(30),
        }
    }

    pub fn set_invoke_timeout(&mut self, d: Duration) {
        self.invoke_timeout = d;
    }

    fn register_conn(&self, outbound: mpsc::UnboundedSender<String>) -> ConnId {
        let id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let state = Arc::new(ConnState {
            device_id: Mutex::new(None),
            outbound,
            pending: Mutex::new(HashMap::new()),
            next_req: AtomicU64::new(1),
        });
        self.conns.lock().unwrap().insert(id, state);
        id
    }

    fn drop_conn(&self, conn_id: ConnId) {
        self.conns.lock().unwrap().remove(&conn_id);
        let mut ports = self.ports.lock().unwrap();
        ports.retain(|_, e| e.conn_id != conn_id);
    }

    fn attach_ports(
        &self,
        conn_id: ConnId,
        device_id: String,
        manifests: &[Value],
    ) -> Result<usize, String> {
        let conn = match self.conns.lock().unwrap().get(&conn_id).cloned() {
            Some(c) => c,
            None => return Err(format!("unknown conn_id {}", conn_id)),
        };
        *conn.device_id.lock().unwrap() = Some(device_id.clone());

        let mut ports = self.ports.lock().unwrap();
        let mut added = 0;
        for m in manifests {
            let port_id = match m.get("port_id").and_then(Value::as_str) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            };
            let caps = m.get("capabilities").cloned().unwrap_or(Value::Null);
            ports.insert(
                port_id.clone(),
                LocalPortEntry {
                    port_id,
                    device_id: device_id.clone(),
                    conn_id,
                    capabilities: caps,
                },
            );
            added += 1;
        }
        Ok(added)
    }

    /// Snapshot of all currently-registered remote ports.
    pub fn list(&self) -> Vec<LocalPortEntry> {
        self.ports.lock().unwrap().values().cloned().collect()
    }

    /// Issue an outbound `reverse/invoke_port` request to the device that
    /// owns `port_id` and await the response. Fails if the port isn't
    /// registered, the conn was dropped, or the timeout elapses.
    pub async fn invoke_remote_port(
        &self,
        port_id: &str,
        capability_id: &str,
        input: Value,
    ) -> Result<Value, String> {
        let entry = match self.ports.lock().unwrap().get(port_id).cloned() {
            Some(e) => e,
            None => return Err(format!("no remote port registered for '{}'", port_id)),
        };
        let conn = match self.conns.lock().unwrap().get(&entry.conn_id).cloned() {
            Some(c) => c,
            None => return Err(format!("connection for port '{}' dropped", port_id)),
        };

        let req_id = format!(
            "rinv-{}-{}",
            entry.conn_id,
            conn.next_req.fetch_add(1, Ordering::Relaxed)
        );
        let (tx, rx) = oneshot::channel();
        conn.pending.lock().unwrap().insert(req_id.clone(), tx);

        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": "reverse/invoke_port",
            "params": {
                "port_id": port_id,
                "capability_id": capability_id,
                "input": input,
            }
        });
        let text = serde_json::to_string(&frame).map_err(|e| e.to_string())?;
        if conn.outbound.send(text).is_err() {
            conn.pending.lock().unwrap().remove(&req_id);
            return Err(format!("outbound channel closed for port '{}'", port_id));
        }

        match tokio::time::timeout(self.invoke_timeout, rx).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err(format!("response channel dropped for '{}'", port_id)),
            Err(_) => {
                conn.pending.lock().unwrap().remove(&req_id);
                Err(format!(
                    "remote invoke timed out after {:?}",
                    self.invoke_timeout
                ))
            }
        }
    }

    /// Send a JSON-RPC notification (no `id`) to every connected client.
    /// Used for pushing runtime events (observations, status changes) to PWAs.
    pub fn broadcast(&self, method: &str, params: Value) {
        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let text = match serde_json::to_string(&frame) {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "failed to serialize broadcast frame");
                return;
            }
        };
        let conns = self.conns.lock().unwrap();
        for conn in conns.values() {
            let _ = conn.outbound.send(text.clone());
        }
    }

    fn route_response(&self, conn_id: ConnId, id: &str, value: Value) -> bool {
        let conn = match self.conns.lock().unwrap().get(&conn_id).cloned() {
            Some(c) => c,
            None => return false,
        };
        let sender = conn.pending.lock().unwrap().remove(id);
        match sender {
            Some(s) => {
                let _ = s.send(value);
                true
            }
            None => false,
        }
    }
}

/// Spawn a dedicated thread that runs a tokio runtime and serves the MCP
/// WebSocket listener. The thread runs for the life of the process.
pub fn start_mcp_ws_listener_background(
    bind_addr: SocketAddr,
    server: Arc<McpServer>,
    auth_token: Option<String>,
) -> std::thread::JoinHandle<()> {
    start_mcp_ws_listener_background_with_registry(
        bind_addr,
        server,
        LocalPortRegistry::new(),
        auth_token,
    )
}

/// Same as above but lets the caller supply (and retain a clone of) the
/// `LocalPortRegistry` — useful when other subsystems need to invoke
/// remote ports via the shared registry.
pub fn start_mcp_ws_listener_background_with_registry(
    bind_addr: SocketAddr,
    server: Arc<McpServer>,
    registry: LocalPortRegistry,
    auth_token: Option<String>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("soma-mcp-ws".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new()
                .expect("failed to create tokio runtime for MCP WebSocket listener");
            rt.block_on(async {
                if let Err(e) = listen(bind_addr, server, registry, auth_token).await {
                    error!(error = %e, "MCP WebSocket listener exited with error");
                }
            });
        })
        .expect("failed to spawn MCP WebSocket listener thread")
}

async fn listen(
    bind_addr: SocketAddr,
    server: Arc<McpServer>,
    registry: LocalPortRegistry,
    auth_token: Option<String>,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(bind_addr).await?;
    let auth_token: Option<Arc<str>> = auth_token.map(|s| Arc::from(s.as_str()));
    info!(addr = %bind_addr, "MCP WebSocket transport listening");
    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "failed to accept MCP WebSocket connection");
                continue;
            }
        };
        let server = Arc::clone(&server);
        let registry = registry.clone();
        let auth_token = auth_token.clone();
        tokio::spawn(async move {
            debug!(peer = %peer_addr, "accepted MCP WebSocket connection");
            if let Err(e) = handle_connection(stream, server, registry, auth_token, peer_addr).await {
                debug!(peer = %peer_addr, error = %e, "MCP WebSocket connection finished with error");
            }
        });
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    server: Arc<McpServer>,
    registry: LocalPortRegistry,
    auth_token: Option<Arc<str>>,
    peer_addr: SocketAddr,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Use accept_hdr_async so we can intercept the handshake. Clients
    // (Node.js built-in WebSocket, browsers) commonly request the
    // permessage-deflate extension. Tungstenite doesn't support compression
    // extensions, so we strip Sec-WebSocket-Extensions from the response to
    // signal "no extensions negotiated". Per RFC 6455 the client then falls
    // back to uncompressed framing.
    use tokio_tungstenite::tungstenite::handshake::server::{
        ErrorResponse as WsErrorResponse, Request as WsRequest, Response as WsResponse,
    };
    #[allow(clippy::result_large_err)] // return type dictated by tungstenite Callback trait
    fn strip_extensions(
        _req: &WsRequest,
        mut response: WsResponse,
    ) -> Result<WsResponse, WsErrorResponse> {
        response.headers_mut().remove("Sec-WebSocket-Extensions");
        Ok(response)
    }
    let ws = tokio_tungstenite::accept_hdr_async(stream, strip_extensions).await?;
    let (mut write, mut read) = ws.split();

    // First-message auth: if the server has a token configured, the client
    // must send {"method":"auth","params":{"token":"..."}} as the very
    // first WebSocket text frame. Any mismatch drops the connection.
    if let Some(ref expected) = auth_token {
        let authed = match tokio::time::timeout(Duration::from_secs(10), read.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                match serde_json::from_str::<Value>(&text) {
                    Ok(v) => {
                        let method = v.get("method").and_then(Value::as_str).unwrap_or("");
                        let token = v
                            .get("params")
                            .and_then(|p| p.get("token"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        method == "auth" && token == expected.as_ref()
                    }
                    Err(_) => false,
                }
            }
            _ => false,
        };
        if !authed {
            warn!(peer = %peer_addr, "WebSocket auth failed — dropping connection");
            let reject = serde_json::json!({
                "jsonrpc": "2.0",
                "error": { "code": -32001, "message": "authentication failed" },
                "id": null
            });
            let _ = write
                .send(Message::Text(serde_json::to_string(&reject).unwrap()))
                .await;
            let _ = write.close().await;
            return Ok(());
        }
        debug!(peer = %peer_addr, "WebSocket auth succeeded");
        // Send auth-ok acknowledgement so the client knows it can proceed.
        let ack = serde_json::json!({
            "jsonrpc": "2.0",
            "result": { "status": "authenticated" },
            "id": null
        });
        let _ = write
            .send(Message::Text(serde_json::to_string(&ack).unwrap()))
            .await;
    }

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let conn_id = registry.register_conn(out_tx.clone());

    // Pump: outbound channel → WebSocket writer.
    let writer = tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if write.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    });

    // Read loop: inbound frames → dispatch.
    let loop_registry = registry.clone();
    let result: std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> = async {
        while let Some(msg_result) = read.next().await {
            let msg = match msg_result {
                Ok(m) => m,
                Err(e) => {
                    debug!(error = %e, "MCP WebSocket read error");
                    break;
                }
            };
            match msg {
                Message::Text(text) => {
                    handle_frame(&server, &loop_registry, conn_id, &out_tx, text.as_str())
                        .await;
                }
                Message::Binary(data) => {
                    let text = match String::from_utf8(data.to_vec()) {
                        Ok(t) => t,
                        Err(e) => {
                            let _ = out_tx.send(error_frame(
                                -32700,
                                &format!("invalid UTF-8: {}", e),
                            ));
                            continue;
                        }
                    };
                    handle_frame(&server, &loop_registry, conn_id, &out_tx, &text).await;
                }
                Message::Ping(p) => {
                    // Ping/pong bypass the outbound channel since tungstenite
                    // manages control frames on the write half.
                    let _ = out_tx.send(format!("__pong__{}__", p.len()));
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        Ok(())
    }
    .await;

    // Clean up connection state and stop the writer.
    registry.drop_conn(conn_id);
    drop(out_tx);
    writer.abort();
    result
}

/// Dispatch a single incoming frame. May produce a reply (queued to the
/// outbound channel), route a response to a pending correlation, or be a
/// no-op (e.g. malformed frame).
async fn handle_frame(
    server: &McpServer,
    registry: &LocalPortRegistry,
    conn_id: ConnId,
    out_tx: &mpsc::UnboundedSender<String>,
    text: &str,
) {
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            let _ = out_tx.send(error_frame(-32700, &format!("parse error: {}", e)));
            return;
        }
    };

    // Response (reply to a server-initiated request)?
    let id_str = value.get("id").and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    });
    let has_result_or_error = value.get("result").is_some() || value.get("error").is_some();
    let has_method = value.get("method").is_some();
    if has_result_or_error && !has_method {
        if let Some(id) = id_str.as_ref()
            && registry.route_response(conn_id, id, value)
        {
            return;
        }
        return;
    }

    // Request — standard MCP or reverse-only local methods.
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    match method.as_str() {
        "reverse/register_ports" => {
            let reply = handle_register_ports(registry, conn_id, &value);
            let _ = out_tx.send(reply);
        }
        "reverse/list_ports" => {
            let reply = handle_list_ports(registry, &value);
            let _ = out_tx.send(reply);
        }
        _ => {
            let reply = dispatch_mcp(server, &value, text);
            let _ = out_tx.send(reply);
        }
    }
}

fn handle_register_ports(registry: &LocalPortRegistry, conn_id: ConnId, req: &Value) -> String {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let device_id = match params.get("device_id").and_then(Value::as_str) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return err_reply(id, -32602, "device_id (string) required");
        }
    };
    let ports_arr = match params.get("ports").and_then(Value::as_array) {
        Some(a) => a.clone(),
        None => {
            return err_reply(id, -32602, "ports (array) required");
        }
    };
    match registry.attach_ports(conn_id, device_id, &ports_arr) {
        Ok(added) => ok_reply(id, serde_json::json!({ "registered": added })),
        Err(e) => err_reply(id, -32603, &e),
    }
}

fn handle_list_ports(registry: &LocalPortRegistry, req: &Value) -> String {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let entries: Vec<Value> = registry
        .list()
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "port_id": e.port_id,
                "device_id": e.device_id,
                "conn_id": e.conn_id,
                "capabilities": e.capabilities,
            })
        })
        .collect();
    ok_reply(id, serde_json::json!({ "ports": entries }))
}

fn dispatch_mcp(server: &McpServer, value: &Value, text: &str) -> String {
    // Re-parse as the typed McpRequest so id/method/params are validated.
    let _ = value; // only used for Value-typed inspection; actual dispatch uses text.
    let request: McpRequest = match serde_json::from_str(text) {
        Ok(r) => r,
        Err(e) => return error_frame(-32700, &format!("parse error: {}", e)),
    };
    match server.handle_request(request) {
        Ok(resp) => match serde_json::to_string(&resp) {
            Ok(s) => s,
            Err(e) => error_frame(-32603, &format!("serialize error: {}", e)),
        },
        Err(e) => error_frame(-32603, &format!("internal error: {}", e)),
    }
}

fn ok_reply(id: Value, result: Value) -> String {
    let v = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    serde_json::to_string(&v).unwrap_or_else(|_| "{}".to_string())
}

fn err_reply(id: Value, code: i32, msg: &str) -> String {
    let v = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": msg },
    });
    serde_json::to_string(&v).unwrap_or_else(|_| "{}".to_string())
}

fn error_frame(code: i32, msg: &str) -> String {
    let value = serde_json::json!({
        "jsonrpc": "2.0",
        "error": { "code": code, "message": msg },
        "id": null,
    });
    serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::connect_async;

    async fn spawn_server() -> (SocketAddr, LocalPortRegistry) {
        spawn_server_with_token(None).await
    }

    async fn spawn_server_with_token(
        token: Option<String>,
    ) -> (SocketAddr, LocalPortRegistry) {
        let server = Arc::new(McpServer::new_stub());
        let registry = LocalPortRegistry::new();
        let listener_registry = registry.clone();
        let auth_token: Option<Arc<str>> = token.map(|s| Arc::from(s.as_str()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (stream, peer_addr) = listener.accept().await.unwrap();
                let server = Arc::clone(&server);
                let registry = listener_registry.clone();
                let auth_token = auth_token.clone();
                tokio::spawn(async move {
                    let _ =
                        handle_connection(stream, server, registry, auth_token, peer_addr).await;
                });
            }
        });
        (addr, registry)
    }

    async fn send_text(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        v: &Value,
    ) {
        ws.send(Message::Text(v.to_string())).await.unwrap();
    }

    async fn recv_json(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Value {
        loop {
            let msg = ws.next().await.unwrap().unwrap();
            if let Message::Text(t) = msg {
                return serde_json::from_str(&t).unwrap();
            }
        }
    }

    #[test]
    fn ws_listener_handles_initialize_and_tools_list() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let (addr, _registry) = spawn_server().await;
            let url = format!("ws://{}", addr);
            let (mut ws, _) = connect_async(&url).await.unwrap();

            send_text(
                &mut ws,
                &serde_json::json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}
                }),
            )
            .await;
            let init_resp = recv_json(&mut ws).await;
            assert_eq!(init_resp["jsonrpc"], "2.0");
            assert!(init_resp["result"].is_object(), "{:?}", init_resp);

            send_text(
                &mut ws,
                &serde_json::json!({
                    "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}
                }),
            )
            .await;
            let tools_resp = recv_json(&mut ws).await;
            assert!(
                tools_resp["result"]["tools"].is_array(),
                "{:?}",
                tools_resp
            );
        });
    }

    #[test]
    fn reverse_register_then_list_then_invoke_roundtrip() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let (addr, registry) = spawn_server().await;
            let url = format!("ws://{}", addr);
            let (mut ws, _) = connect_async(&url).await.unwrap();

            // Register two ports against this connection.
            send_text(
                &mut ws,
                &serde_json::json!({
                    "jsonrpc": "2.0", "id": "r1", "method": "reverse/register_ports",
                    "params": {
                        "device_id": "phone-abc",
                        "ports": [
                            { "port_id": "camera", "capabilities": ["capture_image"] },
                            { "port_id": "geo", "capabilities": ["current_position"] }
                        ]
                    }
                }),
            )
            .await;
            let reg_resp = recv_json(&mut ws).await;
            assert_eq!(reg_resp["result"]["registered"], 2, "{:?}", reg_resp);

            // List: should see both.
            send_text(
                &mut ws,
                &serde_json::json!({
                    "jsonrpc": "2.0", "id": "r2", "method": "reverse/list_ports"
                }),
            )
            .await;
            let list_resp = recv_json(&mut ws).await;
            let ports = list_resp["result"]["ports"].as_array().unwrap();
            let mut ids: Vec<String> = ports
                .iter()
                .map(|p| p["port_id"].as_str().unwrap().to_string())
                .collect();
            ids.sort();
            assert_eq!(ids, vec!["camera".to_string(), "geo".to_string()]);
            assert_eq!(ports[0]["device_id"], "phone-abc");

            // Server initiates invoke_remote_port; client receives the
            // request on its read half, replies on write half.
            let invoker_registry = registry.clone();
            let invoke = tokio::spawn(async move {
                invoker_registry
                    .invoke_remote_port("camera", "capture_image", serde_json::json!({"fmt": "jpg"}))
                    .await
            });

            // Client reads the outbound invoke request.
            let req = recv_json(&mut ws).await;
            assert_eq!(req["method"], "reverse/invoke_port");
            assert_eq!(req["params"]["port_id"], "camera");
            assert_eq!(req["params"]["capability_id"], "capture_image");
            assert_eq!(req["params"]["input"]["fmt"], "jpg");
            let req_id = req["id"].clone();

            // Client responds.
            send_text(
                &mut ws,
                &serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": { "image_base64": "YWJj", "latency_ms": 42 }
                }),
            )
            .await;

            // Server-side invoke_remote_port resolves with that result.
            let result = invoke.await.unwrap().unwrap();
            assert_eq!(result["result"]["image_base64"], "YWJj");
            assert_eq!(result["result"]["latency_ms"], 42);
        });
    }

    #[test]
    fn invoke_remote_port_unknown_port_errors() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let (_addr, registry) = spawn_server().await;
            let err = registry
                .invoke_remote_port("nonexistent", "x", Value::Null)
                .await
                .unwrap_err();
            assert!(err.contains("no remote port"), "{}", err);
        });
    }

    #[test]
    fn auth_valid_token_allows_connection() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let (addr, _registry) =
                spawn_server_with_token(Some("secret-42".to_string())).await;
            let url = format!("ws://{}", addr);
            let (mut ws, _) = connect_async(&url).await.unwrap();

            // Send auth as first message.
            send_text(
                &mut ws,
                &serde_json::json!({ "method": "auth", "params": { "token": "secret-42" } }),
            )
            .await;
            let ack = recv_json(&mut ws).await;
            assert_eq!(ack["result"]["status"], "authenticated");

            // Normal request should work after auth.
            send_text(
                &mut ws,
                &serde_json::json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}
                }),
            )
            .await;
            let resp = recv_json(&mut ws).await;
            assert!(resp["result"].is_object(), "{:?}", resp);
        });
    }

    #[test]
    fn auth_invalid_token_drops_connection() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let (addr, _registry) =
                spawn_server_with_token(Some("secret-42".to_string())).await;
            let url = format!("ws://{}", addr);
            let (mut ws, _) = connect_async(&url).await.unwrap();

            // Send wrong token.
            send_text(
                &mut ws,
                &serde_json::json!({ "method": "auth", "params": { "token": "wrong" } }),
            )
            .await;
            let resp = recv_json(&mut ws).await;
            assert!(resp["error"].is_object(), "expected error: {:?}", resp);
            assert_eq!(resp["error"]["code"], -32001);

            // Connection should be closed — next read should fail or return close.
            let next = ws.next().await;
            assert!(
                next.is_none() || next.as_ref().unwrap().is_err()
                    || matches!(next.as_ref().unwrap().as_ref().unwrap(), Message::Close(_)),
                "expected connection to be closed, got {:?}",
                next
            );
        });
    }

    #[test]
    fn auth_no_token_configured_skips_auth() {
        // When no token is set on the server, clients connect without auth.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let (addr, _registry) = spawn_server_with_token(None).await;
            let url = format!("ws://{}", addr);
            let (mut ws, _) = connect_async(&url).await.unwrap();

            // Send a normal request directly (no auth frame).
            send_text(
                &mut ws,
                &serde_json::json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}
                }),
            )
            .await;
            let resp = recv_json(&mut ws).await;
            assert!(resp["result"].is_object(), "{:?}", resp);
        });
    }

    #[test]
    fn dropped_connection_removes_registered_ports() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let (addr, registry) = spawn_server().await;
            let url = format!("ws://{}", addr);
            {
                let (mut ws, _) = connect_async(&url).await.unwrap();
                send_text(
                    &mut ws,
                    &serde_json::json!({
                        "jsonrpc": "2.0", "id": "r1", "method": "reverse/register_ports",
                        "params": {
                            "device_id": "phone-xyz",
                            "ports": [{ "port_id": "camera", "capabilities": [] }]
                        }
                    }),
                )
                .await;
                let _ = recv_json(&mut ws).await;
                assert_eq!(registry.list().len(), 1);
                // drop ws at end of scope
            }
            // Give the server a beat to notice the close.
            for _ in 0..40 {
                if registry.list().is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            assert_eq!(registry.list().len(), 0, "port not cleaned up on close");
        });
    }
}
