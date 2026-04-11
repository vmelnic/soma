//! MCP-client port backend.
//!
//! Lets soma-next consume an MCP server as if it were a port. At load time
//! the bootstrap layer spawns a subprocess (stdio) or opens a blocking HTTP
//! client, performs the MCP `initialize` handshake, runs `tools/list` to
//! discover capabilities, and hands the runtime a ready `Box<dyn Port>`.
//! Every subsequent `invoke` call maps onto a JSON-RPC `tools/call`.
//!
//! This is how ports written in any language with an MCP SDK — Node, Python,
//! Bun, PHP, Go, Ruby, and so on — become loadable by SOMA: ship an MCP
//! server executable, declare its transport in the pack manifest.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::interfaces::mcp::{McpRequest, McpResponse, McpTool};
use crate::runtime::port::Port;
use crate::types::common::{
    CostClass, CostProfile, DeterminismClass, IdempotenceClass, LatencyProfile, PortFailureClass,
    RiskClass, RollbackSupport, SchemaRef, SideEffectClass,
};
use crate::types::observation::PortCallRecord;
use crate::types::port::{
    McpTransport, PortCapabilitySpec, PortLifecycleState, PortSpec,
};

/// Grace period after closing stdin before we SIGKILL a misbehaving child.
const STDIO_CHILD_KILL_GRACE_MS: u64 = 500;

/// MCP protocol version we advertise in `initialize`. Servers may negotiate
/// a different one in their response; we accept whatever comes back.
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Port adapter that talks JSON-RPC 2.0 to a remote MCP server.
///
/// `transport` holds either a spawned subprocess + its stdio pipes, or a
/// blocking HTTP client pointed at a remote URL. Both are serialized with a
/// `Mutex` because stdio reads/writes must not interleave and the blocking
/// HTTP client is cheap enough to lock per-call.
pub struct McpClientPort {
    spec: PortSpec,
    transport: Mutex<TransportImpl>,
    request_id: AtomicU64,
}

enum TransportImpl {
    Stdio(StdioTransport),
    Http(HttpTransport),
}

struct StdioTransport {
    /// Kept so `Drop` can reap the child process.
    child: Child,
    /// `Option` so `Drop` can take stdin out before waiting on the child,
    /// which closes the pipe and signals a well-behaved MCP server to exit.
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: Option<BufReader<ChildStdout>>,
}

struct HttpTransport {
    client: reqwest::blocking::Client,
    url: String,
    headers: HashMap<String, String>,
}

impl McpClientPort {
    /// Spawn the MCP server (or open the HTTP client), run the initialize +
    /// discovery handshake, merge discovered tools into the pack manifest's
    /// declared capabilities, and return a ready adapter.
    ///
    /// Returns `(port, effective_spec)` where `effective_spec` is what the
    /// caller should register with `PortRuntime::register_port` — it may
    /// contain capabilities that weren't in the original manifest.
    pub fn spawn_and_discover(
        spec: PortSpec,
        transport: McpTransport,
    ) -> Result<(Self, PortSpec)> {
        let mut transport_impl = build_transport(transport)?;
        let request_id = AtomicU64::new(1);

        initialize(&mut transport_impl, &request_id)?;
        let discovered = list_tools(&mut transport_impl, &request_id)?;

        let merged = merge_capabilities(spec, discovered);

        Ok((
            Self {
                spec: merged.clone(),
                transport: Mutex::new(transport_impl),
                request_id,
            },
            merged,
        ))
    }

    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }

    fn invoke_tool(&self, capability_id: &str, input: Value) -> Result<Value> {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": capability_id,
                "arguments": input,
            })),
            id: Value::from(self.next_id()),
        };
        let mut transport = self
            .transport
            .lock()
            .map_err(|_| SomaError::Port("MCP port transport lock poisoned".to_string()))?;
        let resp = send_request(&mut transport, &req)?;
        if let Some(err) = resp.error {
            return Err(SomaError::Port(format!(
                "MCP tools/call failed: {} (code {})",
                err.message, err.code
            )));
        }
        resp.result.ok_or_else(|| {
            SomaError::Port("MCP tools/call response missing result field".to_string())
        })
    }
}

// ---------------------------------------------------------------------------
// Transport setup
// ---------------------------------------------------------------------------

fn build_transport(transport: McpTransport) -> Result<TransportImpl> {
    match transport {
        McpTransport::Stdio {
            command,
            args,
            env,
            working_dir,
        } => {
            let mut cmd = Command::new(&command);
            cmd.args(&args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit());
            for (k, v) in &env {
                cmd.env(k, v);
            }
            if let Some(dir) = working_dir.as_deref() {
                cmd.current_dir(dir);
            }
            let mut child = cmd.spawn().map_err(|e| {
                SomaError::Port(format!(
                    "failed to spawn MCP server '{command}': {e}"
                ))
            })?;
            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| SomaError::Port("spawned MCP child has no stdin".to_string()))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| SomaError::Port("spawned MCP child has no stdout".to_string()))?;
            Ok(TransportImpl::Stdio(StdioTransport {
                child,
                stdin: Some(BufWriter::new(stdin)),
                stdout: Some(BufReader::new(stdout)),
            }))
        }
        McpTransport::Http { url, headers } => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|e| {
                    SomaError::Port(format!(
                        "failed to build HTTP client for MCP port: {e}"
                    ))
                })?;
            Ok(TransportImpl::Http(HttpTransport {
                client,
                url,
                headers,
            }))
        }
    }
}

// ---------------------------------------------------------------------------
// Handshake and discovery (operate on &mut TransportImpl directly so they
// can run before the `Mutex`-wrapped McpClientPort is constructed)
// ---------------------------------------------------------------------------

fn initialize(transport: &mut TransportImpl, counter: &AtomicU64) -> Result<()> {
    let id = counter.fetch_add(1, Ordering::Relaxed);
    let req = McpRequest {
        jsonrpc: "2.0".to_string(),
        method: "initialize".to_string(),
        params: Some(json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "soma-next",
                "version": env!("CARGO_PKG_VERSION"),
            }
        })),
        id: Value::from(id),
    };
    let resp = send_request(transport, &req)?;
    if let Some(err) = resp.error {
        return Err(SomaError::Port(format!(
            "MCP initialize failed: {} (code {})",
            err.message, err.code
        )));
    }

    // Per MCP spec, the client sends `notifications/initialized` after the
    // initialize reply lands. Stdio servers typically require it; HTTP MCP
    // tends not to. Best effort — if the transport rejects it we let the
    // error propagate so the caller can see what went wrong.
    if let TransportImpl::Stdio(stdio) = transport {
        let notified = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });
        let mut line = serde_json::to_string(&notified).map_err(|e| {
            SomaError::Port(format!("failed to serialize initialized notification: {e}"))
        })?;
        line.push('\n');
        let stdin = stdio
            .stdin
            .as_mut()
            .ok_or_else(|| SomaError::Port("MCP stdio transport has no stdin".to_string()))?;
        stdin.write_all(line.as_bytes()).map_err(|e| {
            SomaError::Port(format!("failed to write initialized notification: {e}"))
        })?;
        stdin.flush().map_err(|e| {
            SomaError::Port(format!("failed to flush initialized notification: {e}"))
        })?;
    }

    Ok(())
}

fn list_tools(transport: &mut TransportImpl, counter: &AtomicU64) -> Result<Vec<McpTool>> {
    let id = counter.fetch_add(1, Ordering::Relaxed);
    let req = McpRequest {
        jsonrpc: "2.0".to_string(),
        method: "tools/list".to_string(),
        params: None,
        id: Value::from(id),
    };
    let resp = send_request(transport, &req)?;
    if let Some(err) = resp.error {
        return Err(SomaError::Port(format!(
            "MCP tools/list failed: {} (code {})",
            err.message, err.code
        )));
    }
    let result = resp.result.ok_or_else(|| {
        SomaError::Port("MCP tools/list response missing result field".to_string())
    })?;
    let tools_val = result.get("tools").ok_or_else(|| {
        SomaError::Port("MCP tools/list result missing 'tools' field".to_string())
    })?;
    let tools: Vec<McpTool> = serde_json::from_value(tools_val.clone()).map_err(|e| {
        SomaError::Port(format!("MCP tools/list returned invalid tools array: {e}"))
    })?;
    Ok(tools)
}

// ---------------------------------------------------------------------------
// Transport send (used by both discovery and invoke_tool)
// ---------------------------------------------------------------------------

fn send_request(transport: &mut TransportImpl, req: &McpRequest) -> Result<McpResponse> {
    match transport {
        TransportImpl::Stdio(stdio) => stdio_send(stdio, req),
        TransportImpl::Http(http) => http_send(http, req),
    }
}

fn stdio_send(stdio: &mut StdioTransport, req: &McpRequest) -> Result<McpResponse> {
    let mut line = serde_json::to_string(req)
        .map_err(|e| SomaError::Port(format!("failed to serialize MCP request: {e}")))?;
    line.push('\n');

    let stdin = stdio
        .stdin
        .as_mut()
        .ok_or_else(|| SomaError::Port("MCP stdio transport has no stdin".to_string()))?;
    stdin
        .write_all(line.as_bytes())
        .map_err(|e| SomaError::Port(format!("failed to write MCP request: {e}")))?;
    stdin
        .flush()
        .map_err(|e| SomaError::Port(format!("failed to flush MCP request: {e}")))?;

    let stdout = stdio
        .stdout
        .as_mut()
        .ok_or_else(|| SomaError::Port("MCP stdio transport has no stdout".to_string()))?;

    // Read lines until we find a response whose id matches. MCP servers can
    // emit notifications, log lines, or JSON they expect the client to
    // ignore in between; we skip anything that doesn't parse as a response
    // to the request we just sent.
    let target_id = req.id.clone();
    loop {
        let mut buf = String::new();
        let bytes = stdout
            .read_line(&mut buf)
            .map_err(|e| SomaError::Port(format!("failed to read MCP response line: {e}")))?;
        if bytes == 0 {
            return Err(SomaError::Port(
                "MCP server closed stdout before responding".to_string(),
            ));
        }
        let trimmed = buf.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<McpResponse>(trimmed) {
            Ok(resp) if resp.id == target_id => return Ok(resp),
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
}

fn http_send(http: &mut HttpTransport, req: &McpRequest) -> Result<McpResponse> {
    let mut builder = http
        .client
        .post(&http.url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    for (k, v) in &http.headers {
        builder = builder.header(k, v);
    }
    let resp = builder
        .json(req)
        .send()
        .map_err(|e| SomaError::Port(format!("MCP HTTP request failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(SomaError::Port(format!(
            "MCP HTTP server returned status {}",
            resp.status()
        )));
    }
    resp.json::<McpResponse>()
        .map_err(|e| SomaError::Port(format!("MCP HTTP response not valid JSON-RPC: {e}")))
}

// ---------------------------------------------------------------------------
// Result extraction and capability merging
// ---------------------------------------------------------------------------

/// Extract the semantic result from an MCP `tools/call` envelope.
///
/// Preference order:
///   1. `structuredContent` — newer spec, already a JSON value.
///   2. `content[0].text` parsed as JSON if it looks parseable.
///   3. `content[0].text` as a literal string wrapped in `{"text": ...}`.
///   4. The entire result, unchanged.
fn extract_structured(result: &Value) -> Value {
    if let Some(structured) = result.get("structuredContent") {
        return structured.clone();
    }
    if let Some(content) = result.get("content").and_then(|c| c.as_array())
        && let Some(first) = content.first()
        && let Some(text) = first.get("text").and_then(|t| t.as_str())
    {
        if let Ok(parsed) = serde_json::from_str::<Value>(text) {
            return parsed;
        }
        return json!({ "text": text });
    }
    result.clone()
}

/// Return `Some(error_message)` if the MCP result has `isError: true`.
fn is_mcp_error_result(result: &Value) -> Option<String> {
    if !result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false) {
        return None;
    }
    if let Some(content) = result.get("content").and_then(|c| c.as_array())
        && let Some(first) = content.first()
        && let Some(text) = first.get("text").and_then(|t| t.as_str())
    {
        return Some(text.to_string());
    }
    Some("MCP tool reported isError=true".to_string())
}

/// Merge tools discovered via `tools/list` into the manifest-declared spec.
///
/// Authored capabilities always win: if the manifest declares a capability
/// with id `X`, it stays as-is even if the server also exposes `X`. Server
/// tools that the manifest did not declare are appended with safe defaults.
fn merge_capabilities(mut spec: PortSpec, discovered: Vec<McpTool>) -> PortSpec {
    let existing: std::collections::HashSet<String> = spec
        .capabilities
        .iter()
        .map(|c| c.capability_id.clone())
        .collect();
    for tool in discovered {
        if existing.contains(&tool.name) {
            continue;
        }
        spec.capabilities.push(discovered_capability(tool));
    }
    spec
}

fn discovered_capability(tool: McpTool) -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: tool.name.clone(),
        name: tool.name.clone(),
        purpose: if tool.description.is_empty() {
            format!("MCP tool {}", tool.name)
        } else {
            tool.description
        },
        input_schema: SchemaRef {
            schema: tool.input_schema,
        },
        output_schema: SchemaRef {
            schema: json!({ "description": "any" }),
        },
        // Safe defaults — the manifest author can override by declaring the
        // capability statically, since static always wins over discovered.
        effect_class: SideEffectClass::ReadOnly,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Deterministic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Low,
        latency_profile: LatencyProfile {
            expected_latency_ms: 100,
            p95_latency_ms: 1000,
            max_latency_ms: 30_000,
        },
        cost_profile: CostProfile {
            cpu_cost_class: CostClass::Low,
            memory_cost_class: CostClass::Low,
            io_cost_class: CostClass::Low,
            network_cost_class: CostClass::Low,
            energy_cost_class: CostClass::Low,
        },
        remote_exposable: false,
        auth_override: None,
    }
}

// ---------------------------------------------------------------------------
// Port trait impl
// ---------------------------------------------------------------------------

impl Port for McpClientPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(&self, capability_id: &str, input: Value) -> Result<PortCallRecord> {
        let start = Instant::now();
        match self.invoke_tool(capability_id, input) {
            Ok(raw) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                if let Some(err_msg) = is_mcp_error_result(&raw) {
                    return Ok(PortCallRecord {
                        observation_id: Uuid::new_v4(),
                        port_id: self.spec.port_id.clone(),
                        capability_id: capability_id.to_string(),
                        invocation_id: Uuid::new_v4(),
                        success: false,
                        failure_class: Some(PortFailureClass::ExternalError),
                        raw_result: raw,
                        structured_result: json!({ "error": err_msg }),
                        effect_patch: None,
                        side_effect_summary: Some("mcp_tool_error".to_string()),
                        latency_ms,
                        resource_cost: 0.0,
                        confidence: 0.0,
                        timestamp: Utc::now(),
                        retry_safe: false,
                        input_hash: None,
                        session_id: None,
                        goal_id: None,
                        caller_identity: None,
                        auth_result: None,
                        policy_result: None,
                        sandbox_result: None,
                    });
                }
                let structured = extract_structured(&raw);
                Ok(PortCallRecord {
                    observation_id: Uuid::new_v4(),
                    port_id: self.spec.port_id.clone(),
                    capability_id: capability_id.to_string(),
                    invocation_id: Uuid::new_v4(),
                    success: true,
                    failure_class: None,
                    raw_result: raw,
                    structured_result: structured,
                    effect_patch: None,
                    side_effect_summary: Some("mcp_tool_call".to_string()),
                    latency_ms,
                    resource_cost: 0.001,
                    confidence: 1.0,
                    timestamp: Utc::now(),
                    retry_safe: true,
                    input_hash: None,
                    session_id: None,
                    goal_id: None,
                    caller_identity: None,
                    auth_result: None,
                    policy_result: None,
                    sandbox_result: None,
                })
            }
            Err(e) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                Ok(PortCallRecord {
                    observation_id: Uuid::new_v4(),
                    port_id: self.spec.port_id.clone(),
                    capability_id: capability_id.to_string(),
                    invocation_id: Uuid::new_v4(),
                    success: false,
                    failure_class: Some(PortFailureClass::TransportError),
                    raw_result: Value::Null,
                    structured_result: json!({ "error": e.to_string() }),
                    effect_patch: None,
                    side_effect_summary: Some("none".to_string()),
                    latency_ms,
                    resource_cost: 0.0,
                    confidence: 0.0,
                    timestamp: Utc::now(),
                    retry_safe: false,
                    input_hash: None,
                    session_id: None,
                    goal_id: None,
                    caller_identity: None,
                    auth_result: None,
                    policy_result: None,
                    sandbox_result: None,
                })
            }
        }
    }

    fn validate_input(&self, capability_id: &str, _input: &Value) -> Result<()> {
        // The runtime already validates the input against input_schema from
        // the capability spec. We only check the tool name is known — the
        // MCP server will re-validate on its own side against its own schema.
        if !self
            .spec
            .capabilities
            .iter()
            .any(|c| c.capability_id == capability_id)
        {
            return Err(SomaError::Port(format!(
                "MCP port '{}' has no tool named '{}'",
                self.spec.port_id, capability_id
            )));
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

// ---------------------------------------------------------------------------
// Drop: reap the child so we don't leak zombies
// ---------------------------------------------------------------------------

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // Closing stdin first signals a well-behaved server to exit.
        drop(self.stdin.take());
        drop(self.stdout.take());
        let deadline = Instant::now() + Duration::from_millis(STDIO_CHILD_KILL_GRACE_MS);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::common::{
        AuthRequirements, PortFailureClass, SandboxRequirements, SchemaRef, TrustLevel,
    };
    use crate::types::port::PortKind;
    use semver::Version;

    fn empty_spec(id: &str) -> PortSpec {
        PortSpec {
            port_id: id.to_string(),
            name: id.to_string(),
            version: Version::new(0, 1, 0),
            kind: PortKind::Custom,
            description: "test".to_string(),
            namespace: format!("test.{id}"),
            trust_level: TrustLevel::Trusted,
            capabilities: vec![],
            input_schema: SchemaRef {
                schema: json!({"type": "object"}),
            },
            output_schema: SchemaRef {
                schema: json!({"description": "any"}),
            },
            failure_modes: vec![PortFailureClass::ExternalError, PortFailureClass::TransportError],
            side_effect_class: SideEffectClass::ReadOnly,
            latency_profile: LatencyProfile {
                expected_latency_ms: 100,
                p95_latency_ms: 1000,
                max_latency_ms: 30_000,
            },
            cost_profile: CostProfile {
                cpu_cost_class: CostClass::Low,
                memory_cost_class: CostClass::Low,
                io_cost_class: CostClass::Low,
                network_cost_class: CostClass::Low,
                energy_cost_class: CostClass::Low,
            },
            auth_requirements: AuthRequirements {
                methods: vec![],
                required: false,
            },
            sandbox_requirements: SandboxRequirements {
                filesystem_access: false,
                network_access: true,
                device_access: false,
                process_access: true,
                memory_limit_mb: None,
                cpu_limit_percent: None,
                time_limit_ms: None,
                syscall_limit: None,
            },
            observable_fields: vec![],
            validation_rules: vec![],
            remote_exposure: false,
            backend: crate::types::port::PortBackend::default(),
        }
    }

    fn tool(name: &str) -> McpTool {
        McpTool {
            name: name.to_string(),
            description: format!("{name} tool"),
            input_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn merge_appends_discovered_when_manifest_empty() {
        let spec = empty_spec("demo");
        let merged = merge_capabilities(spec, vec![tool("ping"), tool("echo")]);
        assert_eq!(merged.capabilities.len(), 2);
        assert_eq!(merged.capabilities[0].capability_id, "ping");
        assert_eq!(merged.capabilities[1].capability_id, "echo");
    }

    #[test]
    fn merge_keeps_authored_over_discovered() {
        let mut spec = empty_spec("demo");
        spec.capabilities.push(discovered_capability(tool("ping")));
        // Tag the authored ping so we can tell if it got overwritten.
        spec.capabilities[0].purpose = "authored".to_string();
        spec.capabilities[0].risk_class = RiskClass::High;

        let merged = merge_capabilities(spec, vec![tool("ping"), tool("echo")]);
        assert_eq!(merged.capabilities.len(), 2);
        let ping = merged
            .capabilities
            .iter()
            .find(|c| c.capability_id == "ping")
            .unwrap();
        assert_eq!(ping.purpose, "authored");
        assert_eq!(ping.risk_class, RiskClass::High);
    }

    #[test]
    fn extract_structured_prefers_structured_content() {
        let result = json!({
            "structuredContent": {"value": 42},
            "content": [{"type": "text", "text": "ignored"}],
            "isError": false
        });
        assert_eq!(extract_structured(&result), json!({"value": 42}));
    }

    #[test]
    fn extract_structured_parses_content_text_as_json() {
        let result = json!({
            "content": [{"type": "text", "text": "{\"temp_c\": 22.5}"}],
            "isError": false
        });
        assert_eq!(extract_structured(&result), json!({"temp_c": 22.5}));
    }

    #[test]
    fn extract_structured_wraps_non_json_text() {
        let result = json!({
            "content": [{"type": "text", "text": "hello marcu"}],
            "isError": false
        });
        assert_eq!(
            extract_structured(&result),
            json!({"text": "hello marcu"})
        );
    }

    #[test]
    fn is_error_detects_is_error_true() {
        let result = json!({
            "content": [{"type": "text", "text": "thing exploded"}],
            "isError": true
        });
        assert_eq!(is_mcp_error_result(&result), Some("thing exploded".to_string()));
    }

    #[test]
    fn is_error_returns_none_on_success() {
        let result = json!({
            "content": [{"type": "text", "text": "ok"}],
            "isError": false
        });
        assert_eq!(is_mcp_error_result(&result), None);
    }

    #[test]
    fn discovered_capability_uses_safe_defaults() {
        let cap = discovered_capability(tool("ping"));
        assert_eq!(cap.capability_id, "ping");
        assert_eq!(cap.effect_class, SideEffectClass::ReadOnly);
        assert_eq!(cap.risk_class, RiskClass::Low);
        assert_eq!(cap.idempotence_class, IdempotenceClass::Idempotent);
    }
}
