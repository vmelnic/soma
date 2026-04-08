//! MCP authentication and authorization.
//!
//! Implements role-based access control per Whitepaper Section 8.3. Three token-based
//! roles (admin, builder, viewer) gate access to MCP tools. Destructive actions
//! (checkpoint restore, plugin uninstall) require two-step confirmation with a 60-second
//! expiry window. Tokens are registered from environment variables or config at startup.

use serde::Serialize;
use std::collections::HashMap;

/// Authorization tier for MCP connections, checked on every tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AuthLevel {
    /// Full access: state queries, intent execution, plugin management, shutdown.
    Admin,
    /// Read + execute: state queries and intent execution, no admin operations.
    Builder,
    /// Read-only: state queries only, no side effects.
    Viewer,
}

impl AuthLevel {
    /// Returns true if this level permits side-effecting operations (intents, plugin calls).
    pub const fn can_execute(self) -> bool {
        matches!(self, Self::Admin | Self::Builder)
    }

    pub const fn can_admin(self) -> bool {
        matches!(self, Self::Admin)
    }

    #[allow(dead_code, clippy::unused_self)] // Spec feature: Section 8.3 role checks
    pub const fn can_read(self) -> bool {
        true // all levels can read
    }
}

impl std::fmt::Display for AuthLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Admin => write!(f, "admin"),
            Self::Builder => write!(f, "builder"),
            Self::Viewer => write!(f, "viewer"),
        }
    }
}

/// A registered authentication token bound to a session and role.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Spec feature: Section 8.3 auth token fields
pub struct AuthToken {
    pub token: String,
    pub level: AuthLevel,
    /// Identifies the session; `"config"` for tokens registered from env/config at startup.
    pub session_id: String,
    /// Unix timestamp (seconds) of token creation.
    pub created_at: u64,
}

/// A destructive action awaiting explicit user confirmation before execution.
///
/// Stores the original tool name and arguments so that `soma.confirm` can
/// re-dispatch the call without the caller having to resend the full request.
/// Expires after 60 seconds (Section 8.3).
#[derive(Debug)]
#[allow(dead_code)] // Spec feature: Section 8.3 confirmation fields
pub struct PendingConfirmation {
    pub action_id: String,
    pub description: String,
    pub created_at: std::time::Instant,
    pub token: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Manages token validation and two-step confirmation for MCP connections.
///
/// When `require_auth` is false (the default for local stdio), all requests
/// pass without a token. When enabled, every tool call must present a valid
/// token with sufficient privileges.
pub struct AuthManager {
    tokens: HashMap<String, AuthToken>,
    pending_confirmations: HashMap<String, PendingConfirmation>,
    require_auth: bool,
    /// Monotonic counter for generating unique `confirm-N` action IDs.
    next_action_id: u64,
}

impl AuthManager {
    pub fn new(require_auth: bool) -> Self {
        Self {
            tokens: HashMap::new(),
            pending_confirmations: HashMap::new(),
            require_auth,
            next_action_id: 1,
        }
    }

    /// Generate a new UUID-based auth token for the given role level.
    #[allow(dead_code)] // Spec feature: Section 8.3 dynamic token creation
    pub fn create_token(&mut self, level: AuthLevel) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        let session_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.tokens.insert(token.clone(), AuthToken {
            token: token.clone(),
            level,
            session_id,
            created_at,
        });

        token
    }

    /// Register a pre-configured admin token (typically from `SOMA_MCP_ADMIN_TOKEN`).
    pub fn register_admin_token(&mut self, token: String) {
        self.register_token(token, AuthLevel::Admin);
    }

    /// Register a pre-configured builder token (typically from `SOMA_MCP_BUILDER_TOKEN`).
    pub fn register_builder_token(&mut self, token: String) {
        self.register_token(token, AuthLevel::Builder);
    }

    /// Register a pre-configured viewer token (typically from `SOMA_MCP_VIEWER_TOKEN`).
    pub fn register_viewer_token(&mut self, token: String) {
        self.register_token(token, AuthLevel::Viewer);
    }

    /// Internal: insert a token with `session_id = "config"` (pre-configured, not dynamic).
    fn register_token(&mut self, token: String, level: AuthLevel) {
        let session_id = "config".to_string();
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.tokens.insert(token.clone(), AuthToken {
            token,
            level,
            session_id,
            created_at,
        });
    }

    /// Look up a token and return its metadata, or `None` if auth is disabled.
    #[allow(dead_code)] // Spec feature: Section 8.3 token validation
    pub fn validate(&self, token: &str) -> Option<&AuthToken> {
        if !self.require_auth {
            return None;
        }
        self.tokens.get(token)
    }

    /// Authorize a request. Returns the `session_id` on success, or a human-readable
    /// error describing the missing privilege. When auth is disabled, returns `"anonymous"`.
    pub fn check_request(&self, token: Option<&str>, needs_execute: bool, needs_admin: bool) -> Result<String, String> {
        if !self.require_auth {
            return Ok("anonymous".to_string());
        }

        let token_str = token.ok_or_else(|| "auth token required".to_string())?;
        let auth = self.tokens.get(token_str)
            .ok_or_else(|| "invalid auth token".to_string())?;

        if needs_admin && !auth.level.can_admin() {
            return Err(format!("admin access required (have: {})", auth.level));
        }
        if needs_execute && !auth.level.can_execute() {
            return Err(format!("execute access required (have: {})", auth.level));
        }

        Ok(auth.session_id.clone())
    }

    /// Register a destructive action for two-step confirmation. Returns the `action_id`
    /// that the caller must pass to `soma.confirm` within 60 seconds.
    pub fn create_confirmation(
        &mut self,
        description: String,
        token: &str,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> String {
        let action_id = format!("confirm-{}", self.next_action_id);
        self.next_action_id += 1;

        self.pending_confirmations.insert(action_id.clone(), PendingConfirmation {
            action_id: action_id.clone(),
            description,
            created_at: std::time::Instant::now(),
            token: token.to_string(),
            tool_name,
            arguments,
        });

        action_id
    }

    /// Consume and return a pending confirmation if it exists and has not expired (60s TTL).
    pub fn confirm(&mut self, action_id: &str) -> Option<PendingConfirmation> {
        let pending = self.pending_confirmations.remove(action_id)?;
        if pending.created_at.elapsed().as_secs() > 60 {
            return None;
        }
        Some(pending)
    }

    /// Evict all confirmations older than 60 seconds.
    #[allow(dead_code)] // Spec feature: Section 8.3 confirmation cleanup
    pub fn cleanup_expired(&mut self) {
        self.pending_confirmations.retain(|_, p| {
            p.created_at.elapsed().as_secs() < 60
        });
    }
}
