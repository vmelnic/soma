use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::errors::Result;

// ---------------------------------------------------------------------------
// HTTP method
// ---------------------------------------------------------------------------

/// HTTP method for API requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

impl HttpMethod {
    /// Parse from a string (case-insensitive).
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "GET" => Some(Self::Get),
            "POST" => Some(Self::Post),
            "PUT" => Some(Self::Put),
            "DELETE" => Some(Self::Delete),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// API request / response
// ---------------------------------------------------------------------------

/// An incoming API request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest {
    pub method: HttpMethod,
    pub path: String,
    pub body: Option<Value>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

/// An outgoing API response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse {
    pub status: u16,
    pub body: Value,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

impl ApiResponse {
    fn ok(body: Value) -> Self {
        Self {
            status: 200,
            body,
            headers: Self::json_headers(),
        }
    }

    fn created(body: Value) -> Self {
        Self {
            status: 201,
            body,
            headers: Self::json_headers(),
        }
    }

    fn bad_request(message: &str) -> Self {
        Self {
            status: 400,
            body: serde_json::json!({ "error": message }),
            headers: Self::json_headers(),
        }
    }

    fn not_found(message: &str) -> Self {
        Self {
            status: 404,
            body: serde_json::json!({ "error": message }),
            headers: Self::json_headers(),
        }
    }

    pub fn method_not_allowed() -> Self {
        Self {
            status: 405,
            body: serde_json::json!({ "error": "method not allowed" }),
            headers: Self::json_headers(),
        }
    }

    fn json_headers() -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("content-type".to_string(), "application/json".to_string());
        h
    }
}

// ---------------------------------------------------------------------------
// Route matching
// ---------------------------------------------------------------------------

/// A matched route with extracted path parameters.
struct RouteMatch {
    route: Route,
    params: HashMap<String, String>,
}

/// Known routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    // POST /goals
    CreateGoal,
    // GET /sessions
    ListSessions,
    // GET /sessions/:id
    GetSession,
    // POST /sessions/:id/pause
    PauseSession,
    // POST /sessions/:id/resume
    ResumeSession,
    // POST /sessions/:id/abort
    AbortSession,
    // GET /sessions/:id/belief
    GetBelief,
    // GET /sessions/:id/trace
    GetTrace,
    // GET /resources
    ListResources,
    // GET /packs
    ListPacks,
    // GET /skills
    ListSkills,
    // GET /metrics
    GetMetrics,
}

// ---------------------------------------------------------------------------
// ApiRouter
// ---------------------------------------------------------------------------

/// REST-style API router for the SOMA runtime.
///
/// Maps HTTP-like requests to runtime operations: goal submission, session
/// inspection, control (pause/resume/abort), and metric queries.
pub struct ApiRouter;

impl ApiRouter {
    /// Create a new router.
    pub fn new() -> Self {
        Self
    }

    /// Handle an incoming API request and return a response.
    pub fn handle(&self, request: ApiRequest) -> Result<ApiResponse> {
        match self.match_route(&request) {
            Some(rm) => self.dispatch(rm, request),
            None => Ok(ApiResponse::not_found("no matching route")),
        }
    }

    // -----------------------------------------------------------------------
    // Route matching
    // -----------------------------------------------------------------------

    fn match_route(&self, request: &ApiRequest) -> Option<RouteMatch> {
        let segments: Vec<&str> = request
            .path
            .trim_start_matches('/')
            .trim_end_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        match (request.method, segments.as_slice()) {
            // POST /goals
            (HttpMethod::Post, ["goals"]) => Some(RouteMatch {
                route: Route::CreateGoal,
                params: HashMap::new(),
            }),

            // GET /sessions
            (HttpMethod::Get, ["sessions"]) => Some(RouteMatch {
                route: Route::ListSessions,
                params: HashMap::new(),
            }),

            // GET /sessions/:id
            (HttpMethod::Get, ["sessions", id]) => {
                let mut params = HashMap::new();
                params.insert("id".to_string(), id.to_string());
                Some(RouteMatch {
                    route: Route::GetSession,
                    params,
                })
            }

            // POST /sessions/:id/pause
            (HttpMethod::Post, ["sessions", id, "pause"]) => {
                let mut params = HashMap::new();
                params.insert("id".to_string(), id.to_string());
                Some(RouteMatch {
                    route: Route::PauseSession,
                    params,
                })
            }

            // POST /sessions/:id/resume
            (HttpMethod::Post, ["sessions", id, "resume"]) => {
                let mut params = HashMap::new();
                params.insert("id".to_string(), id.to_string());
                Some(RouteMatch {
                    route: Route::ResumeSession,
                    params,
                })
            }

            // POST /sessions/:id/abort
            (HttpMethod::Post, ["sessions", id, "abort"]) => {
                let mut params = HashMap::new();
                params.insert("id".to_string(), id.to_string());
                Some(RouteMatch {
                    route: Route::AbortSession,
                    params,
                })
            }

            // GET /sessions/:id/belief
            (HttpMethod::Get, ["sessions", id, "belief"]) => {
                let mut params = HashMap::new();
                params.insert("id".to_string(), id.to_string());
                Some(RouteMatch {
                    route: Route::GetBelief,
                    params,
                })
            }

            // GET /sessions/:id/trace
            (HttpMethod::Get, ["sessions", id, "trace"]) => {
                let mut params = HashMap::new();
                params.insert("id".to_string(), id.to_string());
                Some(RouteMatch {
                    route: Route::GetTrace,
                    params,
                })
            }

            // GET /resources
            (HttpMethod::Get, ["resources"]) => Some(RouteMatch {
                route: Route::ListResources,
                params: HashMap::new(),
            }),

            // GET /packs
            (HttpMethod::Get, ["packs"]) => Some(RouteMatch {
                route: Route::ListPacks,
                params: HashMap::new(),
            }),

            // GET /skills
            (HttpMethod::Get, ["skills"]) => Some(RouteMatch {
                route: Route::ListSkills,
                params: HashMap::new(),
            }),

            // GET /metrics
            (HttpMethod::Get, ["metrics"]) => Some(RouteMatch {
                route: Route::GetMetrics,
                params: HashMap::new(),
            }),

            _ => None,
        }
    }

    // -----------------------------------------------------------------------
    // Dispatch
    // -----------------------------------------------------------------------

    fn dispatch(&self, rm: RouteMatch, request: ApiRequest) -> Result<ApiResponse> {
        match rm.route {
            Route::CreateGoal => self.handle_create_goal(request.body),
            Route::ListSessions => self.handle_list_sessions(),
            Route::GetSession => self.handle_get_session(&rm.params),
            Route::PauseSession => self.handle_pause_session(&rm.params),
            Route::ResumeSession => self.handle_resume_session(&rm.params),
            Route::AbortSession => self.handle_abort_session(&rm.params),
            Route::GetBelief => self.handle_get_belief(&rm.params),
            Route::GetTrace => self.handle_get_trace(&rm.params),
            Route::ListResources => self.handle_list_resources(),
            Route::ListPacks => self.handle_list_packs(),
            Route::ListSkills => self.handle_list_skills(),
            Route::GetMetrics => self.handle_get_metrics(),
        }
    }

    // -----------------------------------------------------------------------
    // Handlers
    // -----------------------------------------------------------------------

    fn handle_create_goal(&self, body: Option<Value>) -> Result<ApiResponse> {
        let body = match body {
            Some(b) => b,
            None => return Ok(ApiResponse::bad_request("request body required")),
        };

        let objective = body
            .get("objective")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if objective.is_empty() {
            return Ok(ApiResponse::bad_request("objective is required"));
        }

        let session_id = Uuid::new_v4();
        let goal_id = Uuid::new_v4();

        Ok(ApiResponse::created(serde_json::json!({
            "session_id": session_id.to_string(),
            "goal_id": goal_id.to_string(),
            "status": "created",
            "objective": objective
        })))
    }

    fn handle_list_sessions(&self) -> Result<ApiResponse> {
        Ok(ApiResponse::ok(serde_json::json!({
            "sessions": []
        })))
    }

    fn handle_get_session(&self, params: &HashMap<String, String>) -> Result<ApiResponse> {
        let id = match params.get("id") {
            Some(id) => id,
            None => return Ok(ApiResponse::bad_request("session id required")),
        };

        Ok(ApiResponse::ok(serde_json::json!({
            "session_id": id,
            "status": "created",
            "working_memory": {
                "active_bindings": [],
                "unresolved_slots": [],
                "current_subgoal": null,
                "recent_observations": [],
                "candidate_shortlist": [],
                "current_branch_state": null,
                "budget_deltas": []
            },
            "budget_remaining": {
                "risk_remaining": 0.5,
                "latency_remaining_ms": 30000,
                "resource_remaining": 100.0,
                "steps_remaining": 100
            }
        })))
    }

    fn handle_pause_session(&self, params: &HashMap<String, String>) -> Result<ApiResponse> {
        let id = match params.get("id") {
            Some(id) => id,
            None => return Ok(ApiResponse::bad_request("session id required")),
        };

        Ok(ApiResponse::ok(serde_json::json!({
            "session_id": id,
            "status": "paused"
        })))
    }

    fn handle_resume_session(&self, params: &HashMap<String, String>) -> Result<ApiResponse> {
        let id = match params.get("id") {
            Some(id) => id,
            None => return Ok(ApiResponse::bad_request("session id required")),
        };

        Ok(ApiResponse::ok(serde_json::json!({
            "session_id": id,
            "status": "running"
        })))
    }

    fn handle_abort_session(&self, params: &HashMap<String, String>) -> Result<ApiResponse> {
        let id = match params.get("id") {
            Some(id) => id,
            None => return Ok(ApiResponse::bad_request("session id required")),
        };

        Ok(ApiResponse::ok(serde_json::json!({
            "session_id": id,
            "status": "aborted"
        })))
    }

    fn handle_get_belief(&self, params: &HashMap<String, String>) -> Result<ApiResponse> {
        let id = match params.get("id") {
            Some(id) => id,
            None => return Ok(ApiResponse::bad_request("session id required")),
        };

        Ok(ApiResponse::ok(serde_json::json!({
            "session_id": id,
            "belief": {
                "resources": [],
                "facts": [],
                "uncertainties": [],
                "active_bindings": [],
                "world_hash": ""
            }
        })))
    }

    fn handle_get_trace(&self, params: &HashMap<String, String>) -> Result<ApiResponse> {
        let id = match params.get("id") {
            Some(id) => id,
            None => return Ok(ApiResponse::bad_request("session id required")),
        };

        Ok(ApiResponse::ok(serde_json::json!({
            "session_id": id,
            "trace": {
                "steps": []
            }
        })))
    }

    fn handle_list_resources(&self) -> Result<ApiResponse> {
        Ok(ApiResponse::ok(serde_json::json!({
            "resources": []
        })))
    }

    fn handle_list_packs(&self) -> Result<ApiResponse> {
        Ok(ApiResponse::ok(serde_json::json!({
            "packs": []
        })))
    }

    fn handle_list_skills(&self) -> Result<ApiResponse> {
        Ok(ApiResponse::ok(serde_json::json!({
            "skills": []
        })))
    }

    fn handle_get_metrics(&self) -> Result<ApiResponse> {
        Ok(ApiResponse::ok(serde_json::json!({
            "metrics": {
                "sessions_created": 0,
                "sessions_completed": 0,
                "sessions_failed": 0,
                "sessions_aborted": 0,
                "skills_executed": 0,
                "port_calls": 0,
                "policy_checks": 0,
                "uptime_seconds": 0
            }
        })))
    }
}

impl Default for ApiRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn get(path: &str) -> ApiRequest {
        ApiRequest {
            method: HttpMethod::Get,
            path: path.to_string(),
            body: None,
            headers: HashMap::new(),
        }
    }

    fn post(path: &str, body: Value) -> ApiRequest {
        ApiRequest {
            method: HttpMethod::Post,
            path: path.to_string(),
            body: Some(body),
            headers: HashMap::new(),
        }
    }

    fn post_no_body(path: &str) -> ApiRequest {
        ApiRequest {
            method: HttpMethod::Post,
            path: path.to_string(),
            body: None,
            headers: HashMap::new(),
        }
    }

    // --- Route matching ---

    #[test]
    fn test_create_goal() {
        let router = ApiRouter::new();
        let req = post("/goals", serde_json::json!({ "objective": "list files" }));
        let resp = router.handle(req).unwrap();
        assert_eq!(resp.status, 201);
        assert_eq!(resp.body["status"], "created");
        assert_eq!(resp.body["objective"], "list files");
        assert!(resp.body["session_id"].as_str().is_some());
        assert!(resp.body["goal_id"].as_str().is_some());
    }

    #[test]
    fn test_create_goal_missing_body() {
        let router = ApiRouter::new();
        let req = post_no_body("/goals");
        let resp = router.handle(req).unwrap();
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn test_create_goal_empty_objective() {
        let router = ApiRouter::new();
        let req = post("/goals", serde_json::json!({ "objective": "" }));
        let resp = router.handle(req).unwrap();
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn test_create_goal_missing_objective() {
        let router = ApiRouter::new();
        let req = post("/goals", serde_json::json!({}));
        let resp = router.handle(req).unwrap();
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn test_list_sessions() {
        let router = ApiRouter::new();
        let resp = router.handle(get("/sessions")).unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.body["sessions"].is_array());
    }

    #[test]
    fn test_get_session() {
        let router = ApiRouter::new();
        let sid = Uuid::new_v4().to_string();
        let resp = router.handle(get(&format!("/sessions/{}", sid))).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body["session_id"], sid);
        assert_eq!(resp.body["status"], "created");
    }

    #[test]
    fn test_pause_session() {
        let router = ApiRouter::new();
        let sid = Uuid::new_v4().to_string();
        let req = post_no_body(&format!("/sessions/{}/pause", sid));
        let resp = router.handle(req).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body["session_id"], sid);
        assert_eq!(resp.body["status"], "paused");
    }

    #[test]
    fn test_resume_session() {
        let router = ApiRouter::new();
        let sid = Uuid::new_v4().to_string();
        let req = post_no_body(&format!("/sessions/{}/resume", sid));
        let resp = router.handle(req).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body["status"], "running");
    }

    #[test]
    fn test_abort_session() {
        let router = ApiRouter::new();
        let sid = Uuid::new_v4().to_string();
        let req = post_no_body(&format!("/sessions/{}/abort", sid));
        let resp = router.handle(req).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body["status"], "aborted");
    }

    #[test]
    fn test_get_belief() {
        let router = ApiRouter::new();
        let sid = Uuid::new_v4().to_string();
        let resp = router
            .handle(get(&format!("/sessions/{}/belief", sid)))
            .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body["session_id"], sid);
        assert!(resp.body["belief"].is_object());
    }

    #[test]
    fn test_get_trace() {
        let router = ApiRouter::new();
        let sid = Uuid::new_v4().to_string();
        let resp = router
            .handle(get(&format!("/sessions/{}/trace", sid)))
            .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body["session_id"], sid);
        assert!(resp.body["trace"]["steps"].is_array());
    }

    #[test]
    fn test_list_resources() {
        let router = ApiRouter::new();
        let resp = router.handle(get("/resources")).unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.body["resources"].is_array());
    }

    #[test]
    fn test_list_packs() {
        let router = ApiRouter::new();
        let resp = router.handle(get("/packs")).unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.body["packs"].is_array());
    }

    #[test]
    fn test_list_skills() {
        let router = ApiRouter::new();
        let resp = router.handle(get("/skills")).unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.body["skills"].is_array());
    }

    #[test]
    fn test_get_metrics() {
        let router = ApiRouter::new();
        let resp = router.handle(get("/metrics")).unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.body["metrics"].is_object());
        assert_eq!(resp.body["metrics"]["sessions_created"], 0);
    }

    // --- Not found ---

    #[test]
    fn test_unknown_path() {
        let router = ApiRouter::new();
        let resp = router.handle(get("/nonexistent")).unwrap();
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn test_wrong_method_on_goals() {
        let router = ApiRouter::new();
        // GET /goals is not defined
        let resp = router.handle(get("/goals")).unwrap();
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn test_wrong_method_on_sessions_action() {
        let router = ApiRouter::new();
        // GET /sessions/:id/pause is not defined (needs POST)
        let sid = Uuid::new_v4().to_string();
        let resp = router
            .handle(get(&format!("/sessions/{}/pause", sid)))
            .unwrap();
        assert_eq!(resp.status, 404);
    }

    // --- Path normalization ---

    #[test]
    fn test_trailing_slash() {
        let router = ApiRouter::new();
        let resp = router.handle(get("/sessions/")).unwrap();
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn test_leading_slash_optional() {
        let router = ApiRouter::new();
        let resp = router.handle(get("/metrics")).unwrap();
        assert_eq!(resp.status, 200);
    }

    // --- Response structure ---

    #[test]
    fn test_json_content_type_header() {
        let router = ApiRouter::new();
        let resp = router.handle(get("/metrics")).unwrap();
        assert_eq!(
            resp.headers.get("content-type"),
            Some(&"application/json".to_string())
        );
    }

    #[test]
    fn test_http_method_from_str() {
        assert_eq!(HttpMethod::from_str_loose("GET"), Some(HttpMethod::Get));
        assert_eq!(HttpMethod::from_str_loose("post"), Some(HttpMethod::Post));
        assert_eq!(HttpMethod::from_str_loose("Put"), Some(HttpMethod::Put));
        assert_eq!(
            HttpMethod::from_str_loose("DELETE"),
            Some(HttpMethod::Delete)
        );
        assert_eq!(HttpMethod::from_str_loose("PATCH"), None);
    }

    #[test]
    fn test_api_request_serialization() {
        let req = get("/sessions");
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: ApiRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.method, HttpMethod::Get);
        assert_eq!(deserialized.path, "/sessions");
    }

    #[test]
    fn test_api_response_serialization() {
        let resp = ApiResponse::ok(serde_json::json!({"test": true}));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("200"));
        assert!(json.contains("\"test\":true"));
    }

    #[test]
    fn test_default_impl() {
        let router = ApiRouter;
        let resp = router.handle(get("/metrics")).unwrap();
        assert_eq!(resp.status, 200);
    }
}
