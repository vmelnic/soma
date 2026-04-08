//! SOMA HTTP Bridge Plugin -- synchronous HTTP client conventions.
//!
//! Five conventions:
//!
//! | ID | Name      | Description                                  |
//! |----|-----------|----------------------------------------------|
//! | 0  | `get`     | HTTP GET request                             |
//! | 1  | `post`    | HTTP POST request with body                  |
//! | 2  | `put`     | HTTP PUT request with body                   |
//! | 3  | `delete`  | HTTP DELETE request                           |
//! | 4  | `request` | Generic HTTP request with any method + body  |
//!
//! Uses `reqwest` (async under the hood) with a blocking bridge via `tokio`.
//! The `SomaPlugin` trait is synchronous, so `execute()` either borrows the
//! current tokio runtime (if one is active) or spins up a temporary one.
//! This avoids requiring every SOMA host to be async-aware while still
//! benefiting from reqwest's connection pooling and TLS stack.
//!
//! The HTTP client is created lazily in `on_load` with a 30-second timeout
//! and a `soma-http-bridge/0.1.0` user-agent string.

use soma_plugin_sdk::prelude::*;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// The SOMA HTTP bridge plugin.
///
/// Wraps a `reqwest::Client` that is initialized on `on_load` and torn down on
/// `on_unload`.  The client is connection-pooled, so repeated requests to the
/// same host reuse TCP connections.
pub struct HttpBridgePlugin {
    client: Option<reqwest::Client>,
}

impl Default for HttpBridgePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpBridgePlugin {
    /// Create a new plugin instance with no HTTP client yet.
    ///
    /// The client is initialized in [`on_load`](SomaPlugin::on_load).
    pub const fn new() -> Self {
        Self { client: None }
    }

    /// Get a reference to the HTTP client, or return an error if not initialized.
    fn client(&self) -> Result<&reqwest::Client, PluginError> {
        self.client
            .as_ref()
            .ok_or_else(|| PluginError::Failed("HTTP client not initialized; call on_load first".into()))
    }

    /// Parse an optional JSON string into a header map.
    ///
    /// Returns an empty map when `headers_json` is `None` or empty.
    fn parse_headers(
        headers_json: Option<&str>,
    ) -> Result<HashMap<String, String>, PluginError> {
        match headers_json {
            Some(json) if !json.is_empty() => {
                let map: HashMap<String, String> = serde_json::from_str(json).map_err(|e| {
                    PluginError::InvalidArg(format!("invalid headers JSON: {e}"))
                })?;
                Ok(map)
            }
            _ => Ok(HashMap::new()),
        }
    }

    /// Build a `Value::Map` response from a `reqwest::Response`.
    ///
    /// The map contains three keys:
    /// - `status` -- HTTP status code as `Value::Int`
    /// - `headers` -- response headers as `Value::Map<String, Value::String>`
    /// - `body` -- response body as `Value::String`
    async fn build_response(resp: reqwest::Response) -> Result<Value, PluginError> {
        let status = i64::from(resp.status().as_u16());

        let mut resp_headers = HashMap::new();
        for (name, value) in resp.headers() {
            if let Ok(v) = value.to_str() {
                resp_headers.insert(name.as_str().to_string(), Value::String(v.to_string()));
            }
        }

        let body = resp
            .text()
            .await
            .map_err(|e| PluginError::Failed(format!("failed to read response body: {e}")))?;

        let mut result = HashMap::new();
        result.insert("status".to_string(), Value::Int(status));
        result.insert("headers".to_string(), Value::Map(resp_headers));
        result.insert("body".to_string(), Value::String(body));

        Ok(Value::Map(result))
    }

    /// Apply a map of header key-value pairs to a request builder.
    fn apply_headers(
        mut builder: reqwest::RequestBuilder,
        headers: &HashMap<String, String>,
    ) -> reqwest::RequestBuilder {
        for (key, value) in headers {
            builder = builder.header(key.as_str(), value.as_str());
        }
        builder
    }

    /// Extract an optional headers JSON string from a `Value` at the given index.
    fn extract_headers_json(args: &[Value], index: usize) -> Option<&str> {
        args.get(index).and_then(|v| match v {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        })
    }

    /// Execute an HTTP GET request.
    async fn do_get(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let client = self.client()?;

        let url = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: url".into()))?
            .as_str()?;

        let headers = Self::parse_headers(Self::extract_headers_json(&args, 1))?;

        let builder = Self::apply_headers(client.get(url), &headers);
        let resp = builder
            .send()
            .await
            .map_err(|e| PluginError::Failed(format!("HTTP GET failed: {e}")))?;

        Self::build_response(resp).await
    }

    /// Execute an HTTP POST request with a body.
    async fn do_post(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let client = self.client()?;

        let url = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: url".into()))?
            .as_str()?;

        let body = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: body".into()))?
            .as_str()?
            .to_string();

        let headers = Self::parse_headers(Self::extract_headers_json(&args, 2))?;

        let builder = Self::apply_headers(client.post(url).body(body), &headers);
        let resp = builder
            .send()
            .await
            .map_err(|e| PluginError::Failed(format!("HTTP POST failed: {e}")))?;

        Self::build_response(resp).await
    }

    /// Execute an HTTP PUT request with a body.
    async fn do_put(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let client = self.client()?;

        let url = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: url".into()))?
            .as_str()?;

        let body = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: body".into()))?
            .as_str()?
            .to_string();

        let headers = Self::parse_headers(Self::extract_headers_json(&args, 2))?;

        let builder = Self::apply_headers(client.put(url).body(body), &headers);
        let resp = builder
            .send()
            .await
            .map_err(|e| PluginError::Failed(format!("HTTP PUT failed: {e}")))?;

        Self::build_response(resp).await
    }

    /// Execute an HTTP DELETE request.
    async fn do_delete(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let client = self.client()?;

        let url = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: url".into()))?
            .as_str()?;

        let headers = Self::parse_headers(Self::extract_headers_json(&args, 1))?;

        let builder = Self::apply_headers(client.delete(url), &headers);
        let resp = builder
            .send()
            .await
            .map_err(|e| PluginError::Failed(format!("HTTP DELETE failed: {e}")))?;

        Self::build_response(resp).await
    }

    /// Execute a generic HTTP request with any method, optional headers and body.
    async fn do_request(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let client = self.client()?;

        let method_str = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: method".into()))?
            .as_str()?;

        let url = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: url".into()))?
            .as_str()?;

        let headers = Self::parse_headers(Self::extract_headers_json(&args, 2))?;

        let body = args.get(3).and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        });

        let method: reqwest::Method = method_str
            .parse()
            .map_err(|e| PluginError::InvalidArg(format!("invalid HTTP method '{method_str}': {e}")))?;

        let mut builder = client.request(method, url);
        builder = Self::apply_headers(builder, &headers);
        if let Some(b) = body {
            builder = builder.body(b);
        }

        let resp = builder
            .send()
            .await
            .map_err(|e| PluginError::Failed(format!("HTTP request failed: {e}")))?;

        Self::build_response(resp).await
    }
}

// ---------------------------------------------------------------------------
// Convention definitions helper
// ---------------------------------------------------------------------------

/// Shared side-effect declaration for all HTTP conventions.
fn network_side_effect() -> Vec<SideEffect> {
    vec![SideEffect("network".into())]
}

// ---------------------------------------------------------------------------
// SomaPlugin trait implementation
// ---------------------------------------------------------------------------

#[allow(clippy::unnecessary_literal_bound)]
impl SomaPlugin for HttpBridgePlugin {
    fn name(&self) -> &str {
        "http-bridge"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "HTTP client: GET, POST, PUT, DELETE for calling external APIs"
    }

    fn trust_level(&self) -> TrustLevel {
        TrustLevel::Community
    }

    fn permissions(&self) -> PluginPermissions {
        PluginPermissions {
            network: vec!["*".into()],
            ..Default::default()
        }
    }

    fn on_load(&mut self, _config: &PluginConfig) -> Result<(), PluginError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("soma-http-bridge/0.1.0")
            .build()
            .map_err(|e| PluginError::Failed(format!("failed to create HTTP client: {e}")))?;
        self.client = Some(client);
        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), PluginError> {
        self.client = None;
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn conventions(&self) -> Vec<Convention> {
        vec![
            // 0: get
            Convention {
                id: 0,
                name: "get".into(),
                description: "HTTP GET request".into(),
                call_pattern: "get(url, headers?)".into(),
                args: vec![
                    ArgSpec {
                        name: "url".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "URL to request".into(),
                    },
                    ArgSpec {
                        name: "headers".into(),
                        arg_type: ArgType::String,
                        required: false,
                        description: "Optional JSON object of headers".into(),
                    },
                ],
                returns: ReturnSpec::Value("Map".into()),
                is_deterministic: false,
                estimated_latency_ms: 1000,
                max_latency_ms: 30000,
                side_effects: network_side_effect(),
                cleanup: None,
            },
            // 1: post
            Convention {
                id: 1,
                name: "post".into(),
                description: "HTTP POST request".into(),
                call_pattern: "post(url, body, headers?)".into(),
                args: vec![
                    ArgSpec {
                        name: "url".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "URL to request".into(),
                    },
                    ArgSpec {
                        name: "body".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Request body".into(),
                    },
                    ArgSpec {
                        name: "headers".into(),
                        arg_type: ArgType::String,
                        required: false,
                        description: "Optional JSON object of headers".into(),
                    },
                ],
                returns: ReturnSpec::Value("Map".into()),
                is_deterministic: false,
                estimated_latency_ms: 1000,
                max_latency_ms: 30000,
                side_effects: network_side_effect(),
                cleanup: None,
            },
            // 2: put
            Convention {
                id: 2,
                name: "put".into(),
                description: "HTTP PUT request".into(),
                call_pattern: "put(url, body, headers?)".into(),
                args: vec![
                    ArgSpec {
                        name: "url".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "URL to request".into(),
                    },
                    ArgSpec {
                        name: "body".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Request body".into(),
                    },
                    ArgSpec {
                        name: "headers".into(),
                        arg_type: ArgType::String,
                        required: false,
                        description: "Optional JSON object of headers".into(),
                    },
                ],
                returns: ReturnSpec::Value("Map".into()),
                is_deterministic: false,
                estimated_latency_ms: 1000,
                max_latency_ms: 30000,
                side_effects: network_side_effect(),
                cleanup: None,
            },
            // 3: delete
            Convention {
                id: 3,
                name: "delete".into(),
                description: "HTTP DELETE request".into(),
                call_pattern: "delete(url, headers?)".into(),
                args: vec![
                    ArgSpec {
                        name: "url".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "URL to request".into(),
                    },
                    ArgSpec {
                        name: "headers".into(),
                        arg_type: ArgType::String,
                        required: false,
                        description: "Optional JSON object of headers".into(),
                    },
                ],
                returns: ReturnSpec::Value("Map".into()),
                is_deterministic: false,
                estimated_latency_ms: 1000,
                max_latency_ms: 30000,
                side_effects: network_side_effect(),
                cleanup: None,
            },
            // 4: request
            Convention {
                id: 4,
                name: "request".into(),
                description: "Generic HTTP request with any method".into(),
                call_pattern: "request(method, url, headers?, body?)".into(),
                args: vec![
                    ArgSpec {
                        name: "method".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "HTTP method (GET, POST, PUT, DELETE, PATCH, etc.)".into(),
                    },
                    ArgSpec {
                        name: "url".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "URL to request".into(),
                    },
                    ArgSpec {
                        name: "headers".into(),
                        arg_type: ArgType::String,
                        required: false,
                        description: "Optional JSON object of headers".into(),
                    },
                    ArgSpec {
                        name: "body".into(),
                        arg_type: ArgType::String,
                        required: false,
                        description: "Optional request body".into(),
                    },
                ],
                returns: ReturnSpec::Value("Map".into()),
                is_deterministic: false,
                estimated_latency_ms: 1000,
                max_latency_ms: 30000,
                side_effects: network_side_effect(),
                cleanup: None,
            },
        ]
    }

    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        // Use tokio's current runtime to block on the async implementation.
        // If no runtime is active, create a temporary one.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            // We are inside a tokio runtime -- use block_in_place + block_on
            // to avoid blocking an async worker thread.
            tokio::task::block_in_place(|| {
                handle.block_on(self.execute_async(convention_id, args))
            })
        } else {
            // No runtime -- spin up a temporary one.
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| PluginError::Failed(format!("failed to create tokio runtime: {e}")))?;
            rt.block_on(self.execute_async(convention_id, args))
        }
    }

    fn execute_async(
        &self,
        convention_id: u32,
        args: Vec<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<Value, PluginError>> + Send + '_>> {
        Box::pin(async move {
            match convention_id {
                0 => self.do_get(args).await,
                1 => self.do_post(args).await,
                2 => self.do_put(args).await,
                3 => self.do_delete(args).await,
                4 => self.do_request(args).await,
                _ => Err(PluginError::NotFound(format!(
                    "unknown convention_id: {convention_id}"
                ))),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

/// Create a heap-allocated `HttpBridgePlugin` and return a raw pointer for dynamic loading.
///
/// Called by the SOMA runtime's `libloading`-based plugin loader.  The runtime
/// takes ownership of the pointer and drops it on unload.
#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(HttpBridgePlugin::new()))
}
