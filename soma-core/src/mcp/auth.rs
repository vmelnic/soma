//! MCP authentication — role-based access control (Whitepaper Section 8.3).
//!
//! Three levels: admin (full access), builder (read + execute), viewer (read-only).
//! Destructive actions require two-step confirmation.

use serde::Serialize;
use std::collections::HashMap;

/// Auth level for MCP connections (Section 8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AuthLevel {
    /// Full access: read, execute, admin operations
    Admin,
    /// Read + execute: can query state and run intents
    Builder,
    /// Read-only: can only query state
    Viewer,
}

impl AuthLevel {
    /// Check if this level can perform the given action category.
    pub fn can_execute(&self) -> bool {
        matches!(self, AuthLevel::Admin | AuthLevel::Builder)
    }

    pub fn can_admin(&self) -> bool {
        matches!(self, AuthLevel::Admin)
    }

    pub fn can_read(&self) -> bool {
        true // all levels can read
    }
}

impl std::fmt::Display for AuthLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthLevel::Admin => write!(f, "admin"),
            AuthLevel::Builder => write!(f, "builder"),
            AuthLevel::Viewer => write!(f, "viewer"),
        }
    }
}

/// A registered auth token.
#[derive(Debug, Clone)]
pub struct AuthToken {
    pub token: String,
    pub level: AuthLevel,
    pub session_id: String,
    pub created_at: u64,
}

/// Pending action awaiting two-step confirmation (Section 8.3).
#[derive(Debug)]
pub struct PendingConfirmation {
    pub action_id: String,
    pub description: String,
    pub created_at: std::time::Instant,
    pub token: String,
}

/// Auth manager for MCP connections.
pub struct AuthManager {
    tokens: HashMap<String, AuthToken>,
    pending_confirmations: HashMap<String, PendingConfirmation>,
    require_auth: bool,
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

    /// Create a new auth token for the given level.
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

    /// Register a pre-configured admin token (from config).
    pub fn register_admin_token(&mut self, token: String) {
        let session_id = "config".to_string();
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.tokens.insert(token.clone(), AuthToken {
            token,
            level: AuthLevel::Admin,
            session_id,
            created_at,
        });
    }

    /// Validate a token and return the auth info.
    pub fn validate(&self, token: &str) -> Option<&AuthToken> {
        if !self.require_auth {
            return None; // auth disabled, all requests pass
        }
        self.tokens.get(token)
    }

    /// Check if a request is authorized. Returns the session_id if authorized.
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

    /// Create a pending confirmation for a destructive action.
    pub fn create_confirmation(&mut self, description: String, token: &str) -> String {
        let action_id = format!("confirm-{}", self.next_action_id);
        self.next_action_id += 1;

        self.pending_confirmations.insert(action_id.clone(), PendingConfirmation {
            action_id: action_id.clone(),
            description,
            created_at: std::time::Instant::now(),
            token: token.to_string(),
        });

        action_id
    }

    /// Confirm a pending action. Returns true if confirmed.
    pub fn confirm(&mut self, action_id: &str) -> Option<PendingConfirmation> {
        let pending = self.pending_confirmations.remove(action_id)?;
        // Expire after 5 minutes
        if pending.created_at.elapsed().as_secs() > 300 {
            return None;
        }
        Some(pending)
    }

    /// Clean up expired confirmations.
    pub fn cleanup_expired(&mut self) {
        self.pending_confirmations.retain(|_, p| {
            p.created_at.elapsed().as_secs() < 300
        });
    }
}
