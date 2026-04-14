//! Minimal HTTP webhook listener.
//!
//! Accepts POST requests at `/<hook_name>` and patches the world state
//! with the JSON body as a fact. Emits structured JSON to stderr for
//! SSE pickup by the terminal.
//!
//! This is intentionally minimal — no framework, no async, just
//! std::net. Webhooks are simple: read body, parse JSON, patch state,
//! respond 200.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chrono::Utc;

use crate::errors::Result;
use crate::runtime::world_state::WorldStateStore;
use crate::types::belief::Fact;
use crate::types::common::FactProvenance;

/// Start a background thread running a minimal HTTP server that accepts
/// webhook POST requests and patches world state.
pub fn start_webhook_listener(
    bind_addr: SocketAddr,
    world_state: Arc<Mutex<dyn WorldStateStore + Send>>,
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
                thread::spawn(move || {
                    if let Err(e) = handle_webhook_request(stream, &ws) {
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
        if lower.starts_with("content-length:") {
            if let Ok(len) = lower["content-length:".len()..].trim().parse::<usize>() {
                content_length = len;
            }
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
        reader.read_exact(&mut buf).map_err(|e| crate::errors::SomaError::Io(e))?;
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

    // 8. Build a Fact and add to world state.
    let now = Utc::now();
    let timestamp_ms = now.timestamp_millis();
    let fact = Fact {
        fact_id: format!("webhook.{}.{}", hook_name, timestamp_ms),
        subject: "webhook".to_string(),
        predicate: hook_name.clone(),
        value: payload.clone(),
        confidence: 1.0,
        provenance: FactProvenance::Observed,
        timestamp: now,
    };

    {
        let mut ws = world_state.lock().unwrap();
        ws.add_fact(fact)?;
    }

    // 9. Emit structured event to stderr for SSE consumers.
    let event = serde_json::json!({
        "_webhook_event": true,
        "name": hook_name,
        "payload": payload,
        "received_at": now.to_rfc3339(),
    });
    eprintln!("{}", serde_json::to_string(&event).unwrap_or_default());

    // 10. Respond 200.
    let response_body = serde_json::json!({
        "status": "ok",
        "hook": hook_name,
    });
    send_response(
        &mut stream,
        200,
        "OK",
        &serde_json::to_string(&response_body).unwrap_or_default(),
    );

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
                    let _ = handle_webhook_request(stream, &ws);
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
