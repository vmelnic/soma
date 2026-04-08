use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::errors::{Result, SomaError};
use crate::types::common::TrustLevel;
use crate::types::peer::DistributedFailure;

// --- PeerCredentials ---

/// Credentials presented by a remote peer during authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerCredentials {
    /// Authentication method (e.g. "token", "mtls", "signed_capability").
    pub method: String,
    /// Bearer token or equivalent secret, when applicable.
    pub token: Option<String>,
}

// --- AuthResult ---

/// Outcome of an authentication attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthResult {
    /// Peer was authenticated successfully. An optional elevated trust level
    /// can be granted based on the credentials presented.
    Authenticated {
        elevated_trust: Option<TrustLevel>,
    },
    /// Authentication was rejected.
    Rejected {
        reason: String,
    },
}

// --- PeerAuthenticator trait ---

/// Authenticates remote peers before allowing distributed operations.
/// All peers are untrusted by default until they present valid credentials.
pub trait PeerAuthenticator: Send + Sync {
    /// Attempt to authenticate a peer with the given credentials.
    /// On success, the peer is recorded as authenticated and may receive
    /// an elevated trust level. On failure, returns `AuthResult::Rejected`.
    fn authenticate(&mut self, peer_id: &str, credentials: &PeerCredentials) -> Result<AuthResult>;

    /// Check whether a peer has been successfully authenticated.
    fn is_authenticated(&self, peer_id: &str) -> bool;

    /// Revoke authentication for a peer, returning it to unauthenticated state.
    fn revoke(&mut self, peer_id: &str);
}

// --- DefaultPeerAuthenticator ---

/// Default implementation that tracks authenticated peers in memory.
/// Accepts any credential with a non-empty token as valid. Production
/// implementations would verify signatures, certificates, or shared secrets.
pub struct DefaultPeerAuthenticator {
    /// Map from peer_id to the elevated trust level granted at authentication.
    authenticated_peers: HashMap<String, Option<TrustLevel>>,
}

impl DefaultPeerAuthenticator {
    pub fn new() -> Self {
        Self {
            authenticated_peers: HashMap::new(),
        }
    }
}

impl Default for DefaultPeerAuthenticator {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerAuthenticator for DefaultPeerAuthenticator {
    fn authenticate(&mut self, peer_id: &str, credentials: &PeerCredentials) -> Result<AuthResult> {
        // Reject empty tokens — a real implementation would verify against
        // a credential store, PKI, or shared secret.
        match &credentials.token {
            Some(token) if !token.is_empty() => {
                let elevated = Some(TrustLevel::Verified);
                self.authenticated_peers
                    .insert(peer_id.to_string(), elevated);
                Ok(AuthResult::Authenticated {
                    elevated_trust: elevated,
                })
            }
            _ => Ok(AuthResult::Rejected {
                reason: "missing or empty authentication token".to_string(),
            }),
        }
    }

    fn is_authenticated(&self, peer_id: &str) -> bool {
        self.authenticated_peers.contains_key(peer_id)
    }

    fn revoke(&mut self, peer_id: &str) {
        self.authenticated_peers.remove(peer_id);
    }
}

// --- Helper: require_authenticated ---

/// Check that a peer is authenticated, returning a structured error if not.
pub fn require_authenticated(
    authenticator: &dyn PeerAuthenticator,
    peer_id: &str,
) -> Result<()> {
    if !authenticator.is_authenticated(peer_id) {
        return Err(SomaError::Distributed {
            failure: DistributedFailure::AuthenticationFailure,
            details: format!("peer {} is not authenticated", peer_id),
        });
    }
    Ok(())
}

// --- Helper: require_trust ---

/// Check that a peer's trust level meets the required minimum.
pub fn require_trust(
    peer_trust: TrustLevel,
    required: TrustLevel,
    peer_id: &str,
) -> Result<()> {
    if peer_trust < required {
        return Err(SomaError::Distributed {
            failure: DistributedFailure::TrustValidationFailure,
            details: format!(
                "peer {} has trust {:?}, but {:?} is required",
                peer_id, peer_trust, required
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_creds(token: &str) -> PeerCredentials {
        PeerCredentials {
            method: "token".to_string(),
            token: Some(token.to_string()),
        }
    }

    fn empty_creds() -> PeerCredentials {
        PeerCredentials {
            method: "token".to_string(),
            token: None,
        }
    }

    #[test]
    fn authenticate_with_valid_token_succeeds() {
        let mut auth = DefaultPeerAuthenticator::new();
        let result = auth.authenticate("peer-1", &token_creds("secret-123")).unwrap();
        match result {
            AuthResult::Authenticated { elevated_trust } => {
                assert_eq!(elevated_trust, Some(TrustLevel::Verified));
            }
            AuthResult::Rejected { .. } => panic!("expected Authenticated"),
        }
        assert!(auth.is_authenticated("peer-1"));
    }

    #[test]
    fn authenticate_with_empty_token_rejected() {
        let mut auth = DefaultPeerAuthenticator::new();
        let result = auth
            .authenticate(
                "peer-1",
                &PeerCredentials {
                    method: "token".to_string(),
                    token: Some(String::new()),
                },
            )
            .unwrap();
        match result {
            AuthResult::Rejected { reason } => {
                assert!(reason.contains("empty"));
            }
            AuthResult::Authenticated { .. } => panic!("expected Rejected"),
        }
        assert!(!auth.is_authenticated("peer-1"));
    }

    #[test]
    fn authenticate_with_no_token_rejected() {
        let mut auth = DefaultPeerAuthenticator::new();
        let result = auth.authenticate("peer-1", &empty_creds()).unwrap();
        match result {
            AuthResult::Rejected { reason } => {
                assert!(reason.contains("missing"));
            }
            AuthResult::Authenticated { .. } => panic!("expected Rejected"),
        }
        assert!(!auth.is_authenticated("peer-1"));
    }

    #[test]
    fn is_authenticated_false_by_default() {
        let auth = DefaultPeerAuthenticator::new();
        assert!(!auth.is_authenticated("peer-1"));
        assert!(!auth.is_authenticated("unknown"));
    }

    #[test]
    fn revoke_removes_authentication() {
        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate("peer-1", &token_creds("secret")).unwrap();
        assert!(auth.is_authenticated("peer-1"));
        auth.revoke("peer-1");
        assert!(!auth.is_authenticated("peer-1"));
    }

    #[test]
    fn revoke_nonexistent_peer_is_harmless() {
        let mut auth = DefaultPeerAuthenticator::new();
        auth.revoke("ghost"); // no panic
        assert!(!auth.is_authenticated("ghost"));
    }

    #[test]
    fn require_authenticated_passes_for_authenticated_peer() {
        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate("peer-1", &token_creds("secret")).unwrap();
        assert!(require_authenticated(&auth, "peer-1").is_ok());
    }

    #[test]
    fn require_authenticated_fails_for_unauthenticated_peer() {
        let auth = DefaultPeerAuthenticator::new();
        let result = require_authenticated(&auth, "peer-1");
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::AuthenticationFailure);
                assert!(details.contains("peer-1"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn require_trust_passes_when_sufficient() {
        assert!(require_trust(TrustLevel::Trusted, TrustLevel::Verified, "peer-1").is_ok());
        assert!(require_trust(TrustLevel::Verified, TrustLevel::Verified, "peer-1").is_ok());
        assert!(require_trust(TrustLevel::BuiltIn, TrustLevel::Trusted, "peer-1").is_ok());
    }

    #[test]
    fn require_trust_fails_when_insufficient() {
        let result = require_trust(TrustLevel::Restricted, TrustLevel::Verified, "peer-1");
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::TrustValidationFailure);
                assert!(details.contains("peer-1"));
                assert!(details.contains("Restricted"));
                assert!(details.contains("Verified"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn require_trust_untrusted_fails_for_any_requirement() {
        assert!(require_trust(TrustLevel::Untrusted, TrustLevel::Restricted, "p").is_err());
        assert!(require_trust(TrustLevel::Untrusted, TrustLevel::Verified, "p").is_err());
        assert!(require_trust(TrustLevel::Untrusted, TrustLevel::Trusted, "p").is_err());
        assert!(require_trust(TrustLevel::Untrusted, TrustLevel::BuiltIn, "p").is_err());
    }

    #[test]
    fn require_trust_untrusted_passes_for_untrusted_requirement() {
        assert!(require_trust(TrustLevel::Untrusted, TrustLevel::Untrusted, "p").is_ok());
    }

    #[test]
    fn auth_result_serialization() {
        let authenticated = AuthResult::Authenticated {
            elevated_trust: Some(TrustLevel::Verified),
        };
        let json = serde_json::to_value(&authenticated).unwrap();
        assert_eq!(json["Authenticated"]["elevated_trust"], "verified");

        let rejected = AuthResult::Rejected {
            reason: "bad token".to_string(),
        };
        let json = serde_json::to_value(&rejected).unwrap();
        assert_eq!(json["Rejected"]["reason"], "bad token");
    }

    #[test]
    fn peer_credentials_serialization() {
        let creds = PeerCredentials {
            method: "token".to_string(),
            token: Some("secret-123".to_string()),
        };
        let json = serde_json::to_value(&creds).unwrap();
        assert_eq!(json["method"], "token");
        assert_eq!(json["token"], "secret-123");
    }

    #[test]
    fn re_authenticate_updates_state() {
        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate("peer-1", &token_creds("token-a")).unwrap();
        assert!(auth.is_authenticated("peer-1"));
        // Re-authenticate with a different token — should still work.
        auth.authenticate("peer-1", &token_creds("token-b")).unwrap();
        assert!(auth.is_authenticated("peer-1"));
    }
}
