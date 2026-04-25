//! Minimal HTTP webhook listener.
//!
//! Accepts POST requests at `/<hook_name>` and patches the world state
//! with the JSON body as a fact. Emits structured JSON to stderr for
//! SSE pickup by the terminal.
//!
//! This is intentionally minimal — no framework, no async, just
//! std::net. Webhooks are simple: read body, parse JSON, patch state,
//! respond 200.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chrono::Utc;
use uuid::Uuid;

use crate::errors::Result;
use crate::runtime::world_state::WorldStateStore;
use crate::types::belief::Fact;
use crate::types::common::FactProvenance;

/// What the listener should do when a given webhook fires.
#[derive(Clone, Debug)]
pub enum WebhookAction {
    /// Deposit the incoming payload as a world-state fact (the original
    /// behavior). Reactive routines can still fire off that fact.
    DepositFact,
    /// Launch an async goal whose objective is built from the payload.
    TriggerGoal {
        objective_template: String,
        max_steps: Option<u32>,
    },
}

/// Registry mapping webhook name → action. Lookups default to
/// `DepositFact` when a hook is not registered, preserving backward
/// compatibility with callers that don't configure the registry.
#[derive(Default)]
pub struct WebhookRegistry {
    hooks: Mutex<HashMap<String, WebhookAction>>,
}

impl WebhookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, hook_name: impl Into<String>, action: WebhookAction) {
        self.hooks.lock().unwrap().insert(hook_name.into(), action);
    }

    pub fn lookup(&self, hook_name: &str) -> WebhookAction {
        self.hooks
            .lock()
            .unwrap()
            .get(hook_name)
            .cloned()
            .unwrap_or(WebhookAction::DepositFact)
    }

    pub fn list_registered(&self) -> Vec<String> {
        self.hooks.lock().unwrap().keys().cloned().collect()
    }
}

/// Called by the listener to launch an async goal. Returning the goal_id
/// lets the HTTP response echo it back to the caller so webhook-driven
/// orchestration can correlate requests with goals.
pub type WebhookGoalLauncher =
    Arc<dyn Fn(String, Option<u32>) -> Result<Uuid> + Send + Sync>;

/// Render `template` by substituting `{{path.to.field}}` placeholders
/// against `payload`. Missing paths render as empty strings. This is
/// deliberately minimal — it exists so the runtime can quote payload
/// fields into a goal objective without pulling in a templating crate.
pub fn render_template(template: &str, payload: &serde_json::Value) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len()
            && bytes[i] == b'{'
            && bytes[i + 1] == b'{'
            && let Some(end) = template[i + 2..].find("}}")
        {
            let key = template[i + 2..i + 2 + end].trim();
            out.push_str(&lookup_payload_value(payload, key));
            i += 2 + end + 2;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn lookup_payload_value(payload: &serde_json::Value, path: &str) -> String {
    let mut current = payload;
    for segment in path.split('.') {
        current = match current.get(segment) {
            Some(v) => v,
            None => return String::new(),
        };
    }
    match current {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Start a background thread running a minimal HTTP server that accepts
/// webhook POST requests and either patches world state or launches an
/// async goal. Supplying `None` for `registry` and `launcher` gives the
/// original fact-only behavior.
pub fn start_webhook_listener(
    bind_addr: SocketAddr,
    world_state: Arc<Mutex<dyn WorldStateStore + Send>>,
) -> JoinHandle<()> {
    start_webhook_listener_with_actions(bind_addr, world_state, None, None)
}

/// Same as `start_webhook_listener` but lets the caller supply a
/// per-hook action registry and a launcher for TriggerGoal actions.
pub fn start_webhook_listener_with_actions(
    bind_addr: SocketAddr,
    world_state: Arc<Mutex<dyn WorldStateStore + Send>>,
    registry: Option<Arc<WebhookRegistry>>,
    launcher: Option<WebhookGoalLauncher>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("soma-webhook-http".to_string())
        .spawn(move || {
            let listener = match TcpListener::bind(bind_addr) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("webhook listener failed to bind {}: {}", bind_addr, e);
                    return;
                }
            };
            eprintln!("webhook listener on http://{}", bind_addr);

            for stream in listener.incoming() {
                let stream = match stream {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let ws = Arc::clone(&world_state);
                let reg = registry.clone();
                let launch = launcher.clone();
                thread::spawn(move || {
                    if let Err(e) = handle_webhook_request(stream, &ws, reg.as_deref(), launch.as_ref()) {
                        eprintln!("[webhook-http] error: {}", e);
                    }
                });
            }
        })
        .expect("failed to spawn webhook listener thread")
}

/// Parse method and path from the HTTP request line (e.g. "POST /hook_name HTTP/1.1").
fn parse_request_line(line: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

/// Send an HTTP response with the given status code and JSON body.
fn send_response(stream: &mut TcpStream, status_code: u16, status_text: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status_code, status_text, body.len(), body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn handle_webhook_request(
    mut stream: TcpStream,
    world_state: &Arc<Mutex<dyn WorldStateStore + Send>>,
    registry: Option<&WebhookRegistry>,
    launcher: Option<&WebhookGoalLauncher>,
) -> Result<()> {
    // Set a read timeout so a slow/stalled client doesn't block the thread forever.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));

    let mut reader = BufReader::new(stream.try_clone().map_err(|e| {
        crate::errors::SomaError::Io(e)
    })?);

    // 1. Read request line.
    let mut request_line = String::new();
    reader.read_line(&mut request_line).map_err(|e| {
        crate::errors::SomaError::Io(e)
    })?;
    let request_line = request_line.trim_end().to_string();

    let (method, path) = match parse_request_line(&request_line) {
        Some(pair) => pair,
        None => {
            send_response(&mut stream, 400, "Bad Request", r#"{"error":"bad request line"}"#);
            return Ok(());
        }
    };

    // 2. Read headers until empty line.
    let mut content_length: usize = 0;
    loop {
        let mut header_line = String::new();
        reader.read_line(&mut header_line).map_err(|e| {
            crate::errors::SomaError::Io(e)
        })?;
        let trimmed = header_line.trim();
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_lowercase();
        if let Some(val) = lower.strip_prefix("content-length:")
            && let Ok(len) = val.trim().parse::<usize>()
        {
            content_length = len;
        }
    }

    // 3. Handle GET / as a health check.
    if method == "GET" && path == "/" {
        send_response(&mut stream, 200, "OK", r#"{"status":"ok","service":"soma-webhook"}"#);
        return Ok(());
    }

    // 4. Only POST is accepted for webhooks.
    if method != "POST" {
        send_response(&mut stream, 405, "Method Not Allowed", r#"{"error":"method not allowed"}"#);
        return Ok(());
    }

    // 5. Path must have a hook name.
    let hook_name = path.trim_start_matches('/');
    if hook_name.is_empty() {
        send_response(&mut stream, 404, "Not Found", r#"{"error":"no hook name in path"}"#);
        return Ok(());
    }
    let hook_name = hook_name.to_string();

    // 6. Read body. If Content-Length is known, read exactly that many bytes.
    // Otherwise read whatever is available (handles chunked or missing header).
    let body_bytes = if content_length > 0 {
        let mut buf = vec![0u8; content_length];
        reader.read_exact(&mut buf).map_err(crate::errors::SomaError::Io)?;
        buf
    } else {
        // No Content-Length — read what's available (with timeout protecting us).
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        buf
    };

    // 7. Parse body as JSON.
    let payload: serde_json::Value = if !body_bytes.is_empty() {
        serde_json::from_slice(&body_bytes).unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::Null
    };

    // 8. Route by configured action. Default action = DepositFact.
    let action = registry
        .map(|r| r.lookup(&hook_name))
        .unwrap_or(WebhookAction::DepositFact);
    let now = Utc::now();

    match action {
        WebhookAction::DepositFact => {
            let timestamp_ms = now.timestamp_millis();
            let fact = Fact {
                fact_id: format!("webhook.{}.{}", hook_name, timestamp_ms),
                subject: "webhook".to_string(),
                predicate: hook_name.clone(),
                value: payload.clone(),
                confidence: 1.0,
                provenance: FactProvenance::Observed,
                timestamp: now,
                            ttl_ms: None, prior_confidence: None, prediction_error: None,
            };
            {
                let mut ws = world_state.lock().unwrap();
                ws.add_fact(fact)?;
            }
            let event = serde_json::json!({
                "_webhook_event": true,
                "name": hook_name,
                "payload": payload,
                "received_at": now.to_rfc3339(),
            });
            eprintln!("{}", serde_json::to_string(&event).unwrap_or_default());

            let response_body = serde_json::json!({
                "status": "ok",
                "hook": hook_name,
                "action": "deposit_fact",
            });
            send_response(
                &mut stream,
                200,
                "OK",
                &serde_json::to_string(&response_body).unwrap_or_default(),
            );
        }
        WebhookAction::TriggerGoal {
            objective_template,
            max_steps,
        } => {
            let objective = render_template(&objective_template, &payload);
            let event = serde_json::json!({
                "_webhook_event": true,
                "name": hook_name,
                "action": "trigger_goal",
                "objective": objective,
                "payload": payload,
                "received_at": now.to_rfc3339(),
            });
            eprintln!("{}", serde_json::to_string(&event).unwrap_or_default());

            let response_body = match launcher {
                Some(launch) => match launch(objective.clone(), max_steps) {
                    Ok(goal_id) => serde_json::json!({
                        "status": "ok",
                        "hook": hook_name,
                        "action": "trigger_goal",
                        "goal_id": goal_id.to_string(),
                        "objective": objective,
                    }),
                    Err(e) => {
                        let body = serde_json::json!({
                            "status": "error",
                            "hook": hook_name,
                            "action": "trigger_goal",
                            "error": e.to_string(),
                        });
                        send_response(
                            &mut stream,
                            500,
                            "Internal Server Error",
                            &serde_json::to_string(&body).unwrap_or_default(),
                        );
                        return Ok(());
                    }
                },
                None => serde_json::json!({
                    "status": "error",
                    "hook": hook_name,
                    "action": "trigger_goal",
                    "error": "no goal launcher configured",
                }),
            };
            let code = if response_body["status"] == "ok" { 200 } else { 503 };
            send_response(
                &mut stream,
                code,
                if code == 200 { "OK" } else { "Service Unavailable" },
                &serde_json::to_string(&response_body).unwrap_or_default(),
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::world_state::DefaultWorldStateStore;
    use std::net::TcpStream;

    /// Start a test listener that accepts exactly one connection, handles it,
    /// and exits. Returns the bound address and a join handle.
    fn start_test_listener(
        ws: Arc<Mutex<dyn WorldStateStore + Send>>,
    ) -> (SocketAddr, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = thread::Builder::new()
            .name("test-webhook".to_string())
            .spawn(move || {
                // Accept exactly one connection, then stop.
                if let Some(Ok(stream)) = listener.incoming().next() {
                    let _ = handle_webhook_request(stream, &ws, None, None);
                }
            })
            .unwrap();

        (addr, handle)
    }

    fn send_raw_request(addr: SocketAddr, request: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        stream.flush().unwrap();
        // Shut down write side so the server sees EOF after the request.
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        response
    }

    #[test]
    fn test_valid_post_webhook() {
        let ws: Arc<Mutex<dyn WorldStateStore + Send>> =
            Arc::new(Mutex::new(DefaultWorldStateStore::new()));
        let (addr, handle) = start_test_listener(Arc::clone(&ws));

        let body = r#"{"event":"deploy","version":"1.2.3"}"#;
        let request = format!(
            "POST /ci-deploy HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let response = send_raw_request(addr, &request);

        assert!(response.contains("200 OK"), "expected 200 OK, got: {}", response);
        assert!(response.contains(r#""status":"ok"#), "response body missing status:ok");
        assert!(response.contains(r#""hook":"ci-deploy"#), "response body missing hook name");

        // Verify the fact was added to world state.
        let facts = ws.lock().unwrap().list_facts().iter().map(|f| f.fact_id.clone()).collect::<Vec<_>>();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].starts_with("webhook.ci-deploy."));

        handle.join().unwrap();
    }

    #[test]
    fn test_get_returns_405() {
        let ws: Arc<Mutex<dyn WorldStateStore + Send>> =
            Arc::new(Mutex::new(DefaultWorldStateStore::new()));
        let (addr, handle) = start_test_listener(Arc::clone(&ws));

        let request = "GET /some-hook HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let response = send_raw_request(addr, request);

        assert!(response.contains("405"), "expected 405, got: {}", response);

        handle.join().unwrap();
    }

    #[test]
    fn test_get_root_health_check() {
        let ws: Arc<Mutex<dyn WorldStateStore + Send>> =
            Arc::new(Mutex::new(DefaultWorldStateStore::new()));
        let (addr, handle) = start_test_listener(Arc::clone(&ws));

        let request = "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let response = send_raw_request(addr, request);

        assert!(response.contains("200 OK"), "expected 200 OK, got: {}", response);
        assert!(response.contains("soma-webhook"), "expected health check body");

        handle.join().unwrap();
    }

    #[test]
    fn test_render_template_substitutes_keys() {
        let payload = serde_json::json!({
            "order_id": "abc-123",
            "customer": { "email": "a@b.com" },
        });
        let out = render_template(
            "fulfill order {{order_id}} for {{customer.email}}",
            &payload,
        );
        assert_eq!(out, "fulfill order abc-123 for a@b.com");
    }

    #[test]
    fn test_render_template_missing_keys_empty() {
        let payload = serde_json::json!({ "a": 1 });
        let out = render_template("x={{missing}} y={{a}}", &payload);
        assert_eq!(out, "x= y=1");
    }

    #[test]
    fn test_registry_defaults_to_deposit_fact() {
        let reg = WebhookRegistry::new();
        assert!(matches!(
            reg.lookup("unknown"),
            WebhookAction::DepositFact
        ));
        reg.register(
            "orders",
            WebhookAction::TriggerGoal {
                objective_template: "process {{order_id}}".to_string(),
                max_steps: Some(20),
            },
        );
        assert!(matches!(
            reg.lookup("orders"),
            WebhookAction::TriggerGoal { .. }
        ));
    }

    #[test]
    fn test_trigger_goal_calls_launcher_and_returns_goal_id() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let ws: Arc<Mutex<dyn WorldStateStore + Send>> =
            Arc::new(Mutex::new(DefaultWorldStateStore::new()));
        let reg = Arc::new(WebhookRegistry::new());
        reg.register(
            "orders",
            WebhookAction::TriggerGoal {
                objective_template: "process order {{order_id}}".to_string(),
                max_steps: Some(7),
            },
        );
        let call_count = Arc::new(AtomicUsize::new(0));
        let fixed_goal_id = Uuid::new_v4();
        let count_for_closure = Arc::clone(&call_count);
        let launcher: WebhookGoalLauncher = Arc::new(move |objective, max_steps| {
            count_for_closure.fetch_add(1, Ordering::Relaxed);
            assert_eq!(objective, "process order abc-123");
            assert_eq!(max_steps, Some(7));
            Ok(fixed_goal_id)
        });

        // Run a single-request listener that uses registry + launcher.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let ws_clone = Arc::clone(&ws);
        let reg_clone = Arc::clone(&reg);
        let launcher_clone = Arc::clone(&launcher);
        let handle = thread::Builder::new()
            .name("test-webhook-trigger".into())
            .spawn(move || {
                if let Some(Ok(stream)) = listener.incoming().next() {
                    let _ = handle_webhook_request(
                        stream,
                        &ws_clone,
                        Some(reg_clone.as_ref()),
                        Some(&launcher_clone),
                    );
                }
            })
            .unwrap();

        let body = r#"{"order_id":"abc-123"}"#;
        let request = format!(
            "POST /orders HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let response = send_raw_request(addr, &request);
        assert!(response.contains("200 OK"));
        assert!(response.contains(&fixed_goal_id.to_string()));
        assert_eq!(call_count.load(Ordering::Relaxed), 1);
        // No fact deposited — we took the trigger_goal branch.
        assert!(ws.lock().unwrap().list_facts().is_empty());
        handle.join().unwrap();
    }

    #[test]
    fn test_hook_name_extraction() {
        // Direct unit test for parse_request_line.
        let (method, path) = parse_request_line("POST /my-hook HTTP/1.1").unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/my-hook");

        let hook_name = path.trim_start_matches('/');
        assert_eq!(hook_name, "my-hook");

        // Nested path
        let (_, path) = parse_request_line("POST /hooks/github/push HTTP/1.1").unwrap();
        let hook_name = path.trim_start_matches('/');
        assert_eq!(hook_name, "hooks/github/push");

        // Bad request line
        assert!(parse_request_line("").is_none());
        assert!(parse_request_line("GARBAGE").is_none());
    }
}
