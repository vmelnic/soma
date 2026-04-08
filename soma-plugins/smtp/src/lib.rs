//! SOMA SMTP Plugin -- email delivery via the `lettre` crate.
//!
//! # Overview
//!
//! Provides 3 conventions for sending email through an SMTP server:
//!
//! | ID | Name | Description |
//! |----|------|-------------|
//! | 0 | `send` | Send a plain-text email |
//! | 1 | `send_html` | Send an HTML email |
//! | 2 | `send_with_attachment` | Send an email with a binary attachment |
//!
//! # Configuration
//!
//! Configured via the `[plugins.smtp]` section in `soma.toml`:
//!
//! ```toml
//! [plugins.smtp]
//! host = "smtp.gmail.com"
//! port = 587
//! username_env = "SMTP_USERNAME"
//! password_env = "SMTP_PASSWORD"
//! from = "noreply@helperbook.app"
//! ```
//!
//! Credentials are resolved via environment variables named in `username_env`
//! and `password_env` to avoid storing secrets in config files.
//!
//! # Why `tokio::runtime::Runtime::block_on()`?
//!
//! The `lettre` SMTP transport with `tokio1-rustls-tls` is async. Since the
//! `SomaPlugin` trait requires synchronous `execute()`, we create a dedicated
//! tokio runtime and use `block_on()` to bridge async to sync. This is the
//! same pattern used by other SOMA plugins that wrap async libraries.

#![allow(clippy::unnecessary_wraps)] // Convention methods must return Result per trait contract

use lettre::message::{Attachment, MultiPart, SinglePart, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use soma_plugin_sdk::prelude::*;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

/// The SOMA SMTP plugin.
///
/// Holds SMTP configuration set during [`SomaPlugin::on_load`]. Each `execute`
/// call builds a [`lettre::Message`] and sends it via an async SMTP transport,
/// bridged to sync with a dedicated tokio runtime.
pub struct SmtpPlugin {
    /// SMTP server hostname (e.g., `"smtp.gmail.com"`).
    host: OnceLock<String>,
    /// SMTP server port (e.g., 587 for STARTTLS).
    port: OnceLock<u16>,
    /// SMTP authentication credentials.
    credentials: OnceLock<Credentials>,
    /// Sender address for the `From` header.
    from: OnceLock<String>,
    /// Dedicated tokio runtime for async SMTP operations.
    runtime: OnceLock<tokio::runtime::Runtime>,
}

impl Default for SmtpPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl SmtpPlugin {
    /// Create a new unconfigured plugin instance.
    ///
    /// Configuration is deferred to [`SomaPlugin::on_load`], which is called
    /// by the plugin manager with the `[plugins.smtp]` settings from `soma.toml`.
    pub const fn new() -> Self {
        Self {
            host: OnceLock::new(),
            port: OnceLock::new(),
            credentials: OnceLock::new(),
            from: OnceLock::new(),
            runtime: OnceLock::new(),
        }
    }

    /// Build an async SMTP transport from the stored configuration.
    ///
    /// Creates a new transport per call. SMTP connections are short-lived
    /// (send + quit), so pooling adds complexity without meaningful benefit
    /// for SOMA's one-intent-at-a-time execution model.
    fn build_transport(
        &self,
    ) -> Result<AsyncSmtpTransport<Tokio1Executor>, PluginError> {
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::Failed("smtp not configured -- call on_load first".into()))?;
        let port = self
            .port
            .get()
            .ok_or_else(|| PluginError::Failed("smtp port not configured".into()))?;
        let creds = self
            .credentials
            .get()
            .ok_or_else(|| PluginError::Failed("smtp credentials not configured".into()))?;

        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(|e| PluginError::ConnectionRefused(format!("SMTP relay error: {e}")))?
            .port(*port)
            .credentials(creds.clone())
            .build();

        Ok(transport)
    }

    /// Get the tokio runtime, or return an error if not initialized.
    fn rt(&self) -> Result<&tokio::runtime::Runtime, PluginError> {
        self.runtime
            .get()
            .ok_or_else(|| PluginError::Failed("smtp runtime not initialized".into()))
    }

    /// Get the configured sender address for the `From` header.
    fn sender_address(&self) -> Result<&str, PluginError> {
        self.from
            .get()
            .map(|s| s.as_str())
            .ok_or_else(|| PluginError::Failed("smtp from address not configured".into()))
    }

    // -----------------------------------------------------------------------
    // Convention implementations
    // -----------------------------------------------------------------------

    /// Convention 0 -- Send a plain-text email.
    ///
    /// Args: `[to: String, subject: String, body: String]`
    /// Returns: `Value::Bool(true)` on success.
    fn send(&self, args: &[Value]) -> Result<Value, PluginError> {
        let to = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: to".into()))?
            .as_str()?;
        let subject = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: subject".into()))?
            .as_str()?;
        let body = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: body".into()))?
            .as_str()?;

        let from = self.sender_address()?;

        let message = Message::builder()
            .from(
                from.parse()
                    .map_err(|e| PluginError::InvalidArg(format!("invalid from address: {e}")))?,
            )
            .to(to
                .parse()
                .map_err(|e| PluginError::InvalidArg(format!("invalid to address: {e}")))?)
            .subject(subject)
            .body(body.to_string())
            .map_err(|e| PluginError::Failed(format!("failed to build email: {e}")))?;

        let transport = self.build_transport()?;
        let rt = self.rt()?;

        rt.block_on(async {
            transport
                .send(message)
                .await
                .map_err(|e| PluginError::Failed(format!("SMTP send failed: {e}")))
        })?;

        Ok(Value::Bool(true))
    }

    /// Convention 1 -- Send an HTML email.
    ///
    /// Args: `[to: String, subject: String, html: String]`
    /// Returns: `Value::Bool(true)` on success.
    fn send_html(&self, args: &[Value]) -> Result<Value, PluginError> {
        let to = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: to".into()))?
            .as_str()?;
        let subject = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: subject".into()))?
            .as_str()?;
        let html = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: html".into()))?
            .as_str()?;

        let from = self.sender_address()?;

        let message = Message::builder()
            .from(
                from.parse()
                    .map_err(|e| PluginError::InvalidArg(format!("invalid from address: {e}")))?,
            )
            .to(to
                .parse()
                .map_err(|e| PluginError::InvalidArg(format!("invalid to address: {e}")))?)
            .subject(subject)
            .singlepart(
                SinglePart::builder()
                    .header(ContentType::TEXT_HTML)
                    .body(html.to_string()),
            )
            .map_err(|e| PluginError::Failed(format!("failed to build email: {e}")))?;

        let transport = self.build_transport()?;
        let rt = self.rt()?;

        rt.block_on(async {
            transport
                .send(message)
                .await
                .map_err(|e| PluginError::Failed(format!("SMTP send failed: {e}")))
        })?;

        Ok(Value::Bool(true))
    }

    /// Convention 2 -- Send an email with a binary attachment.
    ///
    /// Args: `[to: String, subject: String, body: String, attachment_name: String, attachment_data: Bytes]`
    /// Returns: `Value::Bool(true)` on success.
    fn send_with_attachment(&self, args: &[Value]) -> Result<Value, PluginError> {
        let to = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: to".into()))?
            .as_str()?;
        let subject = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: subject".into()))?
            .as_str()?;
        let body = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: body".into()))?
            .as_str()?;
        let attachment_name = args
            .get(3)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: attachment_name".into()))?
            .as_str()?;
        let attachment_data = args
            .get(4)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: attachment_data".into()))?
            .as_bytes()?;

        let from = self.sender_address()?;

        let attachment = Attachment::new(attachment_name.to_string())
            .body(attachment_data.to_vec(), ContentType::parse("application/octet-stream").unwrap());

        let message = Message::builder()
            .from(
                from.parse()
                    .map_err(|e| PluginError::InvalidArg(format!("invalid from address: {e}")))?,
            )
            .to(to
                .parse()
                .map_err(|e| PluginError::InvalidArg(format!("invalid to address: {e}")))?)
            .subject(subject)
            .multipart(
                MultiPart::mixed()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(body.to_string()),
                    )
                    .singlepart(attachment),
            )
            .map_err(|e| PluginError::Failed(format!("failed to build email: {e}")))?;

        let transport = self.build_transport()?;
        let rt = self.rt()?;

        rt.block_on(async {
            transport
                .send(message)
                .await
                .map_err(|e| PluginError::Failed(format!("SMTP send failed: {e}")))
        })?;

        Ok(Value::Bool(true))
    }
}

// ---------------------------------------------------------------------------
// SomaPlugin trait implementation
// ---------------------------------------------------------------------------

impl SomaPlugin for SmtpPlugin {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "smtp"
    }

    #[allow(clippy::unnecessary_literal_bound)]
    fn version(&self) -> &str {
        "0.1.0"
    }

    #[allow(clippy::unnecessary_literal_bound)]
    fn description(&self) -> &str {
        "SMTP email delivery: plain text, HTML, and attachments via lettre"
    }

    fn trust_level(&self) -> TrustLevel {
        TrustLevel::Community
    }

    fn conventions(&self) -> Vec<Convention> {
        vec![
            // 0: send
            Convention {
                id: 0,
                name: "send".into(),
                description: "Send a plain-text email to a recipient".into(),
                call_pattern: "send(to, subject, body)".into(),
                args: vec![
                    ArgSpec {
                        name: "to".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Recipient email address".into(),
                    },
                    ArgSpec {
                        name: "subject".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Email subject line".into(),
                    },
                    ArgSpec {
                        name: "body".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Plain-text email body".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 500,
                max_latency_ms: 10000,
                side_effects: vec![SideEffect("sends network".into())],
                cleanup: None,
            },
            // 1: send_html
            Convention {
                id: 1,
                name: "send_html".into(),
                description: "Send an HTML email to a recipient".into(),
                call_pattern: "send_html(to, subject, html)".into(),
                args: vec![
                    ArgSpec {
                        name: "to".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Recipient email address".into(),
                    },
                    ArgSpec {
                        name: "subject".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Email subject line".into(),
                    },
                    ArgSpec {
                        name: "html".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "HTML email body".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 500,
                max_latency_ms: 10000,
                side_effects: vec![SideEffect("sends network".into())],
                cleanup: None,
            },
            // 2: send_with_attachment
            Convention {
                id: 2,
                name: "send_with_attachment".into(),
                description: "Send an email with a binary attachment".into(),
                call_pattern: "send_with_attachment(to, subject, body, attachment_name, attachment_data)".into(),
                args: vec![
                    ArgSpec {
                        name: "to".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Recipient email address".into(),
                    },
                    ArgSpec {
                        name: "subject".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Email subject line".into(),
                    },
                    ArgSpec {
                        name: "body".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Plain-text email body".into(),
                    },
                    ArgSpec {
                        name: "attachment_name".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Filename for the attachment (e.g., invoice.pdf)".into(),
                    },
                    ArgSpec {
                        name: "attachment_data".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Binary content of the attachment".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 1000,
                max_latency_ms: 30000,
                side_effects: vec![SideEffect("sends network".into())],
                cleanup: None,
            },
        ]
    }

    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match convention_id {
            0 => self.send(&args),
            1 => self.send_html(&args),
            2 => self.send_with_attachment(&args),
            _ => Err(PluginError::NotFound(format!(
                "unknown convention_id: {convention_id}"
            ))),
        }
    }

    fn on_load(&mut self, config: &PluginConfig) -> Result<(), PluginError> {
        let host = config
            .get_str("host")
            .unwrap_or("smtp.gmail.com")
            .to_string();

        // Port config is i64 from JSON; truncation to u16 is safe for valid port numbers.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let port = config.get_int("port").unwrap_or(587) as u16;

        // Read credentials from env vars named in the config.
        let username = config.get_str("username_env").map_or_else(
            || std::env::var("SMTP_USERNAME").ok(),
            |env_name| std::env::var(env_name).ok(),
        );
        let password = config.get_str("password_env").map_or_else(
            || std::env::var("SMTP_PASSWORD").ok(),
            |env_name| std::env::var(env_name).ok(),
        );

        let from = config
            .get_str("from")
            .unwrap_or("noreply@helperbook.app")
            .to_string();

        // Validate that we have credentials before proceeding.
        let username = username.ok_or_else(|| {
            PluginError::Failed(
                "SMTP username not configured: set username_env in soma.toml or SMTP_USERNAME env var"
                    .into(),
            )
        })?;
        let password = password.ok_or_else(|| {
            PluginError::Failed(
                "SMTP password not configured: set password_env in soma.toml or SMTP_PASSWORD env var"
                    .into(),
            )
        })?;

        let creds = Credentials::new(username, password);

        // Build a dedicated tokio runtime for async SMTP operations.
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PluginError::Failed(format!("failed to create tokio runtime: {e}")))?;

        // OnceLock::set returns Err if already set; ignore since on_load is called once.
        let _ = self.host.set(host);
        let _ = self.port.set(port);
        let _ = self.credentials.set(creds);
        let _ = self.from.set(from);
        let _ = self.runtime.set(runtime);

        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), PluginError> {
        // The tokio runtime will be dropped when the plugin is dropped,
        // which gracefully shuts down any pending tasks.
        Ok(())
    }

    fn permissions(&self) -> PluginPermissions {
        PluginPermissions {
            filesystem: vec![],
            network: vec!["tcp:*:587".into(), "tcp:*:465".into(), "tcp:*:25".into()],
            env_vars: vec!["SMTP_*".into()],
            process_spawn: false,
        }
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "host":         {"type": "string", "default": "smtp.gmail.com"},
                "port":         {"type": "integer", "default": 587},
                "username_env": {"type": "string", "description": "Env var name holding SMTP username"},
                "password_env": {"type": "string", "description": "Env var name holding SMTP password"},
                "from":         {"type": "string", "default": "noreply@helperbook.app"}
            },
            "required": ["host", "from"]
        }))
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

/// FFI entry point called by the SOMA plugin loader (`plugin/dynamic.rs`).
///
/// Returns a heap-allocated `SmtpPlugin` as a trait object pointer.
/// The caller takes ownership and is responsible for eventually dropping it.
#[allow(improper_ctypes_definitions)] // Trait objects have no C equivalent; SOMA uses a known ABI.
#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(SmtpPlugin::new()))
}
