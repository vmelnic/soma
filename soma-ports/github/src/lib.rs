//! SOMA GitHub Port -- interact with the GitHub REST API.
//!
//! Capabilities:
//!
//! | ID | Name | Description |
//! |----|------|-------------|
//! | 0  | `issue.create` | Create an issue in a repository |
//! | 1  | `issue.list` | List issues in a repository |
//! | 2  | `issue.get` | Get a single issue by number |
//! | 3  | `issue.update` | Update an issue (close, label, title, body) |
//! | 4  | `issue.comment` | Add a comment to an issue |
//! | 5  | `pr.create` | Create a pull request |
//! | 6  | `pr.list` | List pull requests in a repository |
//! | 7  | `pr.get` | Get a single pull request by number |
//! | 8  | `pr.merge` | Merge a pull request |
//! | 9  | `repo.read_file` | Read file contents from a repository (no clone) |
//! | 10 | `repo.list_branches` | List branches in a repository |
//!
//! Uses `reqwest::blocking::Client` with Bearer auth (personal access token).
//! If no token is configured, the port loads but returns an error on invoke.

use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.github";
const GITHUB_API_BASE: &str = "https://api.github.com";

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct GitHubPort {
    spec: PortSpec,
    token: Option<String>,
    client: reqwest::blocking::Client,
}

impl GitHubPort {
    pub fn new() -> Self {
        let token = std::env::var("SOMA_GITHUB_TOKEN")
            .ok()
            .or_else(|| std::env::var("GITHUB_TOKEN").ok())
            .filter(|v| !v.is_empty());

        let client = reqwest::blocking::Client::builder()
            .user_agent("soma-port-github/0.1.0")
            .build()
            .expect("reqwest client build");

        Self {
            spec: build_spec(),
            token,
            client,
        }
    }

    fn require_token(&self) -> Option<&str> {
        self.token.as_deref().filter(|v| !v.is_empty())
    }

    fn request_builder(&self, method: reqwest::Method, url: &str) -> reqwest::blocking::RequestBuilder {
        let builder = self
            .client
            .request(method, url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = self.require_token() {
            builder.bearer_auth(token)
        } else {
            builder
        }
    }

    fn parse_github_error(&self, status: reqwest::StatusCode, body: &str) -> PortError {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
            let msg = val["message"]
                .as_str()
                .or_else(|| val["error"].as_str())
                .unwrap_or(body);
            match status.as_u16() {
                401 | 403 => PortError::AuthorizationDenied(msg.into()),
                404 => PortError::NotFound(msg.into()),
                422 => PortError::Validation(msg.into()),
                _ => PortError::ExternalError(msg.into()),
            }
        } else {
            PortError::ExternalError(format!("GitHub HTTP {status}: {body}"))
        }
    }

    fn send_request(
        &self,
        builder: reqwest::blocking::RequestBuilder,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let resp = builder
            .send()
            .map_err(|e| PortError::TransportError(format!("GitHub request failed: {e}")))?;

        let status = resp.status();
        let body = resp
            .text()
            .map_err(|e| PortError::ExternalError(format!("failed to read GitHub response: {e}")))?;

        if !status.is_success() {
            return Err(self.parse_github_error(status, &body));
        }

        if body.is_empty() || body == "null" {
            return Ok(serde_json::json!({}));
        }

        serde_json::from_str(&body)
            .map_err(|e| PortError::ExternalError(format!("failed to parse GitHub response: {e}")))
    }

    fn get(&self, url: &str) -> soma_port_sdk::Result<serde_json::Value> {
        self.send_request(self.request_builder(reqwest::Method::GET, url))
    }

    fn post(&self, url: &str, body: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        self.send_request(self.request_builder(reqwest::Method::POST, url).json(body))
    }

    fn patch(&self, url: &str, body: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        self.send_request(self.request_builder(reqwest::Method::PATCH, url).json(body))
    }

    fn put(&self, url: &str, body: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        self.send_request(self.request_builder(reqwest::Method::PUT, url).json(body))
    }
}

impl Default for GitHubPort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for GitHubPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> soma_port_sdk::Result<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "issue.create" => self.issue_create(&input),
            "issue.list" => self.issue_list(&input),
            "issue.get" => self.issue_get(&input),
            "issue.update" => self.issue_update(&input),
            "issue.comment" => self.issue_comment(&input),
            "pr.create" => self.pr_create(&input),
            "pr.list" => self.pr_list(&input),
            "pr.get" => self.pr_get(&input),
            "pr.merge" => self.pr_merge(&input),
            "repo.read_file" => self.repo_read_file(&input),
            "repo.list_branches" => self.repo_list_branches(&input),
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )));
            }
        };
        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(value) => Ok(PortCallRecord::success(
                PORT_ID,
                capability_id,
                value,
                latency_ms,
            )),
            Err(e) => Ok(PortCallRecord::failure(
                PORT_ID,
                capability_id,
                e.failure_class(),
                &e.to_string(),
                latency_ms,
            )),
        }
    }

    fn validate_input(
        &self,
        capability_id: &str,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<()> {
        match capability_id {
            "issue.create" => {
                require_field(input, "owner")?;
                require_field(input, "repo")?;
                require_field(input, "title")?;
            }
            "issue.list" => {
                require_field(input, "owner")?;
                require_field(input, "repo")?;
            }
            "issue.get" => {
                require_field(input, "owner")?;
                require_field(input, "repo")?;
                require_field(input, "issue_number")?;
            }
            "issue.update" => {
                require_field(input, "owner")?;
                require_field(input, "repo")?;
                require_field(input, "issue_number")?;
            }
            "issue.comment" => {
                require_field(input, "owner")?;
                require_field(input, "repo")?;
                require_field(input, "issue_number")?;
                require_field(input, "body")?;
            }
            "pr.create" => {
                require_field(input, "owner")?;
                require_field(input, "repo")?;
                require_field(input, "title")?;
                require_field(input, "head")?;
                require_field(input, "base")?;
            }
            "pr.list" => {
                require_field(input, "owner")?;
                require_field(input, "repo")?;
            }
            "pr.get" => {
                require_field(input, "owner")?;
                require_field(input, "repo")?;
                require_field(input, "pull_number")?;
            }
            "pr.merge" => {
                require_field(input, "owner")?;
                require_field(input, "repo")?;
                require_field(input, "pull_number")?;
            }
            "repo.read_file" => {
                require_field(input, "owner")?;
                require_field(input, "repo")?;
                require_field(input, "path")?;
            }
            "repo.list_branches" => {
                require_field(input, "owner")?;
                require_field(input, "repo")?;
            }
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )));
            }
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl GitHubPort {
    fn issue_create(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let owner = get_str(input, "owner")?;
        let repo = get_str(input, "repo")?;
        let title = get_str(input, "title")?;

        let mut body = serde_json::json!({"title": title});
        if let Some(text) = input.get("body").and_then(|v| v.as_str()) {
            body["body"] = serde_json::Value::String(text.into());
        }
        if let Some(labels) = input.get("labels").and_then(|v| v.as_array()) {
            body["labels"] = serde_json::Value::Array(labels.clone());
        }
        if let Some(assignees) = input.get("assignees").and_then(|v| v.as_array()) {
            body["assignees"] = serde_json::Value::Array(assignees.clone());
        }

        self.post(
            &format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues"),
            &body,
        )
    }

    fn issue_list(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let owner = get_str(input, "owner")?;
        let repo = get_str(input, "repo")?;

        let mut query = vec![];
        if let Some(state) = input.get("state").and_then(|v| v.as_str()) {
            query.push(format!("state={state}"));
        }
        if let Some(labels) = input.get("labels").and_then(|v| v.as_str()) {
            query.push(format!("labels={labels}"));
        }
        if let Some(assignee) = input.get("assignee").and_then(|v| v.as_str()) {
            query.push(format!("assignee={assignee}"));
        }
        if let Some(per_page) = input.get("per_page").and_then(|v| v.as_u64()) {
            query.push(format!("per_page={per_page}"));
        }

        let url = if query.is_empty() {
            format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues")
        } else {
            format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues?{}", query.join("&"))
        };

        self.get(&url)
    }

    fn issue_get(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let owner = get_str(input, "owner")?;
        let repo = get_str(input, "repo")?;
        let issue_number = get_u64(input, "issue_number")?;

        self.get(&format!(
            "{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{issue_number}"
        ))
    }

    fn issue_update(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let owner = get_str(input, "owner")?;
        let repo = get_str(input, "repo")?;
        let issue_number = get_u64(input, "issue_number")?;

        let mut body = serde_json::json!({});
        if let Some(title) = input.get("title").and_then(|v| v.as_str()) {
            body["title"] = serde_json::Value::String(title.into());
        }
        if let Some(text) = input.get("body").and_then(|v| v.as_str()) {
            body["body"] = serde_json::Value::String(text.into());
        }
        if let Some(state) = input.get("state").and_then(|v| v.as_str()) {
            body["state"] = serde_json::Value::String(state.into());
        }
        if let Some(labels) = input.get("labels").and_then(|v| v.as_array()) {
            body["labels"] = serde_json::Value::Array(labels.clone());
        }
        if let Some(assignees) = input.get("assignees").and_then(|v| v.as_array()) {
            body["assignees"] = serde_json::Value::Array(assignees.clone());
        }

        self.patch(
            &format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{issue_number}"),
            &body,
        )
    }

    fn issue_comment(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let owner = get_str(input, "owner")?;
        let repo = get_str(input, "repo")?;
        let issue_number = get_u64(input, "issue_number")?;
        let text = get_str(input, "body")?;

        let body = serde_json::json!({"body": text});
        self.post(
            &format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{issue_number}/comments"),
            &body,
        )
    }

    fn pr_create(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let owner = get_str(input, "owner")?;
        let repo = get_str(input, "repo")?;
        let title = get_str(input, "title")?;
        let head = get_str(input, "head")?;
        let base = get_str(input, "base")?;

        let mut body = serde_json::json!({
            "title": title,
            "head": head,
            "base": base,
        });
        if let Some(text) = input.get("body").and_then(|v| v.as_str()) {
            body["body"] = serde_json::Value::String(text.into());
        }
        if let Some(draft) = input.get("draft").and_then(|v| v.as_bool()) {
            body["draft"] = serde_json::Value::Bool(draft);
        }

        self.post(
            &format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls"),
            &body,
        )
    }

    fn pr_list(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let owner = get_str(input, "owner")?;
        let repo = get_str(input, "repo")?;

        let mut query = vec![];
        if let Some(state) = input.get("state").and_then(|v| v.as_str()) {
            query.push(format!("state={state}"));
        }
        if let Some(head) = input.get("head").and_then(|v| v.as_str()) {
            query.push(format!("head={head}"));
        }
        if let Some(base) = input.get("base").and_then(|v| v.as_str()) {
            query.push(format!("base={base}"));
        }
        if let Some(per_page) = input.get("per_page").and_then(|v| v.as_u64()) {
            query.push(format!("per_page={per_page}"));
        }

        let url = if query.is_empty() {
            format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls")
        } else {
            format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls?{}", query.join("&"))
        };

        self.get(&url)
    }

    fn pr_get(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let owner = get_str(input, "owner")?;
        let repo = get_str(input, "repo")?;
        let pull_number = get_u64(input, "pull_number")?;

        self.get(&format!(
            "{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls/{pull_number}"
        ))
    }

    fn pr_merge(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let owner = get_str(input, "owner")?;
        let repo = get_str(input, "repo")?;
        let pull_number = get_u64(input, "pull_number")?;

        let mut body = serde_json::json!({});
        if let Some(msg) = input.get("commit_title").and_then(|v| v.as_str()) {
            body["commit_title"] = serde_json::Value::String(msg.into());
        }
        if let Some(msg) = input.get("commit_message").and_then(|v| v.as_str()) {
            body["commit_message"] = serde_json::Value::String(msg.into());
        }
        if let Some(method) = input.get("merge_method").and_then(|v| v.as_str()) {
            body["merge_method"] = serde_json::Value::String(method.into());
        }

        self.put(
            &format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls/{pull_number}/merge"),
            &body,
        )
    }

    fn repo_read_file(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let owner = get_str(input, "owner")?;
        let repo = get_str(input, "repo")?;
        let path = get_str(input, "path")?;
        let ref_ = input.get("ref").and_then(|v| v.as_str()).unwrap_or("HEAD");

        let result = self.get(&format!(
            "{GITHUB_API_BASE}/repos/{owner}/{repo}/contents/{path}?ref={ref_}"
        ))?;

        // GitHub returns content as base64. Decode it for convenience.
        if let Some(content_b64) = result.get("content").and_then(|v| v.as_str()) {
            let cleaned = content_b64.replace('\n', "");
            match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &cleaned) {
                Ok(decoded) => {
                    let mut out = result.as_object().cloned().unwrap_or_default();
                    out.insert(
                        "decoded_content".into(),
                        serde_json::Value::String(
                            String::from_utf8_lossy(&decoded).into_owned(),
                        ),
                    );
                    return Ok(serde_json::Value::Object(out));
                }
                Err(_) => return Ok(result),
            }
        }

        Ok(result)
    }

    fn repo_list_branches(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let owner = get_str(input, "owner")?;
        let repo = get_str(input, "repo")?;

        let mut query = vec![];
        if let Some(per_page) = input.get("per_page").and_then(|v| v.as_u64()) {
            query.push(format!("per_page={per_page}"));
        }

        let url = if query.is_empty() {
            format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/branches")
        } else {
            format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/branches?{}", query.join("&"))
        };

        self.get(&url)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn require_field(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<()> {
    if input.get(field).is_none() {
        return Err(PortError::Validation(format!("missing field: {field}")));
    }
    Ok(())
}

fn get_str<'a>(input: &'a serde_json::Value, field: &str) -> soma_port_sdk::Result<&'a str> {
    input[field]
        .as_str()
        .ok_or_else(|| PortError::Validation(format!("{field} must be a string")))
}

fn get_u64(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<u64> {
    input[field]
        .as_u64()
        .or_else(|| input[field].as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(|| PortError::Validation(format!("{field} must be an integer")))
}

// ---------------------------------------------------------------------------
// Spec builder
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: PORT_ID.into(),
        name: "github".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "GitHub REST API: issues, pull requests, repository files, branches".into(),
        namespace: "soma.github".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "issue.create".into(),
                name: "issue.create".into(),
                purpose: "Create an issue in a GitHub repository".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "owner": {"type": "string", "description": "Repository owner (user or org)"},
                    "repo": {"type": "string", "description": "Repository name"},
                    "title": {"type": "string", "description": "Issue title"},
                    "body": {"type": "string", "description": "Issue body (markdown supported)"},
                    "labels": {"type": "array", "description": "Array of label name strings", "items": {"type": "string"}},
                    "assignees": {"type": "array", "description": "Array of GitHub usernames", "items": {"type": "string"}},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "number": {"type": "integer"},
                    "html_url": {"type": "string"},
                    "title": {"type": "string"},
                    "state": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 800,
                    p95_latency_ms: 3000,
                    max_latency_ms: 15000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "issue.list".into(),
                name: "issue.list".into(),
                purpose: "List issues in a GitHub repository".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "state": {"type": "string", "description": "open, closed, or all (default: open)"},
                    "labels": {"type": "string", "description": "Comma-separated label names"},
                    "assignee": {"type": "string", "description": "GitHub username or '*' for any, 'none' for unassigned"},
                    "per_page": {"type": "integer", "description": "Results per page (max 100)"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "items": {"type": "array"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 600,
                    p95_latency_ms: 3000,
                    max_latency_ms: 15000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "issue.get".into(),
                name: "issue.get".into(),
                purpose: "Get a single issue by number".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "issue_number": {"type": "integer", "description": "Issue number (not ID)"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "number": {"type": "integer"},
                    "title": {"type": "string"},
                    "body": {"type": "string"},
                    "state": {"type": "string"},
                    "html_url": {"type": "string"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 500,
                    p95_latency_ms: 2000,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "issue.update".into(),
                name: "issue.update".into(),
                purpose: "Update an issue: close, edit title/body, set labels or assignees".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "issue_number": {"type": "integer"},
                    "title": {"type": "string"},
                    "body": {"type": "string"},
                    "state": {"type": "string", "description": "open or closed"},
                    "labels": {"type": "array", "items": {"type": "string"}},
                    "assignees": {"type": "array", "items": {"type": "string"}},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "number": {"type": "integer"},
                    "state": {"type": "string"},
                    "html_url": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::ConditionallyIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 600,
                    p95_latency_ms: 3000,
                    max_latency_ms: 15000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "issue.comment".into(),
                name: "issue.comment".into(),
                purpose: "Add a comment to an issue or pull request".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "issue_number": {"type": "integer", "description": "Issue or PR number"},
                    "body": {"type": "string", "description": "Comment body (markdown supported)"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "id": {"type": "integer"},
                    "html_url": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 600,
                    p95_latency_ms: 3000,
                    max_latency_ms: 15000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "pr.create".into(),
                name: "pr.create".into(),
                purpose: "Create a pull request".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "title": {"type": "string"},
                    "head": {"type": "string", "description": "Branch containing changes"},
                    "base": {"type": "string", "description": "Branch to merge into"},
                    "body": {"type": "string", "description": "PR description"},
                    "draft": {"type": "boolean", "description": "Create as draft PR"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "number": {"type": "integer"},
                    "html_url": {"type": "string"},
                    "state": {"type": "string"},
                    "mergeable": {"type": "boolean"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Medium,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 800,
                    p95_latency_ms: 3000,
                    max_latency_ms: 15000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "pr.list".into(),
                name: "pr.list".into(),
                purpose: "List pull requests in a repository".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "state": {"type": "string", "description": "open, closed, or all (default: open)"},
                    "head": {"type": "string", "description": "Filter by head branch (format: user:branch)"},
                    "base": {"type": "string", "description": "Filter by base branch"},
                    "per_page": {"type": "integer"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "items": {"type": "array"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 600,
                    p95_latency_ms: 3000,
                    max_latency_ms: 15000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "pr.get".into(),
                name: "pr.get".into(),
                purpose: "Get a single pull request by number".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "pull_number": {"type": "integer"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "number": {"type": "integer"},
                    "title": {"type": "string"},
                    "state": {"type": "string"},
                    "mergeable": {"type": "boolean"},
                    "html_url": {"type": "string"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 500,
                    p95_latency_ms: 2000,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "pr.merge".into(),
                name: "pr.merge".into(),
                purpose: "Merge a pull request".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "pull_number": {"type": "integer"},
                    "commit_title": {"type": "string"},
                    "commit_message": {"type": "string"},
                    "merge_method": {"type": "string", "description": "merge, squash, or rebase"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "sha": {"type": "string"},
                    "merged": {"type": "boolean"},
                    "message": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::ConditionallyIdempotent,
                risk_class: RiskClass::Medium,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 800,
                    p95_latency_ms: 3000,
                    max_latency_ms: 15000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "repo.read_file".into(),
                name: "repo.read_file".into(),
                purpose: "Read a file from a repository without cloning".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "path": {"type": "string", "description": "File path within the repository"},
                    "ref": {"type": "string", "description": "Branch, tag, or commit SHA (default: HEAD)"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "name": {"type": "string"},
                    "path": {"type": "string"},
                    "sha": {"type": "string"},
                    "content": {"type": "string", "description": "Base64-encoded content"},
                    "decoded_content": {"type": "string", "description": "Decoded text content"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 500,
                    p95_latency_ms: 2000,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "repo.list_branches".into(),
                name: "repo.list_branches".into(),
                purpose: "List branches in a repository".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "per_page": {"type": "integer"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "items": {"type": "array"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 500,
                    p95_latency_ms: 2000,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![
            PortFailureClass::ValidationError,
            PortFailureClass::ExternalError,
            PortFailureClass::TransportError,
            PortFailureClass::Timeout,
            PortFailureClass::DependencyUnavailable,
            PortFailureClass::AuthorizationDenied,
        ],
        side_effect_class: SideEffectClass::ExternalStateMutation,
        latency_profile: LatencyProfile {
            expected_latency_ms: 600,
            p95_latency_ms: 5000,
            max_latency_ms: 30000,
        },
        cost_profile: CostProfile {
            network_cost_class: CostClass::Medium,
            ..CostProfile::default()
        },
        auth_requirements: AuthRequirements {
            methods: vec![AuthMethod::BearerToken],
            required: true,
        },
        sandbox_requirements: SandboxRequirements {
            network_access: true,
            ..SandboxRequirements::default()
        },
        observable_fields: vec![],
        validation_rules: vec![],
        remote_exposure: false,
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(GitHubPort::new()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = GitHubPort::new();
        assert_eq!(port.spec().port_id, "soma.github");
        assert_eq!(port.spec().capabilities.len(), 11);
    }

    #[test]
    fn test_lifecycle_active() {
        let port = GitHubPort::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Active);
    }

    #[test]
    fn test_validate_issue_create_missing_fields() {
        let port = GitHubPort::new();
        assert!(port
            .validate_input("issue.create", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_validate_issue_create_ok() {
        let port = GitHubPort::new();
        let input = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "title": "Bug report"
        });
        assert!(port.validate_input("issue.create", &input).is_ok());
    }

    #[test]
    fn test_validate_repo_read_file_missing_path() {
        let port = GitHubPort::new();
        assert!(port
            .validate_input("repo.read_file", &serde_json::json!({"owner": "x", "repo": "y"}))
            .is_err());
    }

    #[test]
    fn test_validate_pr_merge_ok() {
        let port = GitHubPort::new();
        let input = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "pull_number": 42
        });
        assert!(port.validate_input("pr.merge", &input).is_ok());
    }

    #[test]
    fn test_unknown_capability() {
        let port = GitHubPort::new();
        assert!(port.invoke("nonexistent", serde_json::json!({})).is_err());
    }

    #[test]
    fn test_invoke_without_token_allows_public_reads() {
        let port = GitHubPort::new();
        assert!(port.require_token().is_none());
        let input = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world",
            "path": "README.md"
        });
        // Validation should pass even without a token
        assert!(port.validate_input("repo.read_file", &input).is_ok());
    }
}
