//! SOMA HTTP Bridge Plugin — 5 HTTP client conventions.
//!
//! Provides: GET, POST, PUT, DELETE, and generic HTTP request capabilities
//! for calling external APIs. Uses reqwest with async execution.

use soma_plugin_sdk::prelude::*;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// The SOMA HTTP bridge plugin.
pub struct HttpBridgePlugin {
    client: Option<reqwest::Client>,
}

impl HttpBridgePlugin {
    pub fn new() -> Self {
        Self { client: None }
    }

    /// Get a reference to the HTTP client, or return an error if not initialized.
    fn client(&self) -> Result<&reqwest::Client, PluginError> {
        self.client
            .as_ref()
            .ok_or_else(|| PluginError::Failed("HTTP client not initialized; call on_load first".into()))
    }

    /// Parse optional headers JSON string into a header map.
    fn parse_headers(
        headers_json: Option<&str>,
    ) -> Result<HashMap<String, String>, PluginError> {
        match headers_json {
            Some(json) if !json.is_empty() => {
                let map: HashMap<String, String> = serde_json::from_str(json).map_err(|e| {
                    PluginError::InvalidArg(format!("invalid headers JSON: {}", e))
                })?;
                Ok(map)
            }
            _ => Ok(HashMap::new()),
        }
    }

    /// Build response Value from reqwest Response.
    async fn build_response(resp: reqwest::Response) -> Result<Value, PluginError> {
        let status = resp.status().as_u16() as i64;

        let mut resp_headers = HashMap::new();
        for (name, value) in resp.headers().iter() {
            if let Ok(v) = value.to_str() {
                resp_headers.insert(name.as_str().to_string(), Value::String(v.to_string()));
            }
        }

        let body = resp
            .text()
            .await
            .map_err(|e| PluginError::Failed(format!("failed to read response body: {}", e)))?;

        let mut result = HashMap::new();
        result.insert("status".to_string(), Value::Int(status));
        result.insert("headers".to_string(), Value::Map(resp_headers));
        result.insert("body".to_string(), Value::String(body));

        Ok(Value::Map(result))
    }

    /// Apply headers to a request builder.
    fn apply_headers(
        mut builder: reqwest::RequestBuilder,
        headers: &HashMap<String, String>,
    ) -> reqwest::RequestBuilder {
        for (key, value) in headers {
            builder = builder.header(key.as_str(), value.as_str());
        }
        builder
    }

    /// Execute an HTTP GET request (async).
    async fn do_get(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let client = self.client()?;

        let url = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: url".into()))?
            .as_str()?;

        let headers_json = args.get(1).and_then(|v| match v {
            Value::String(s) => Some(s.as_str()),
            Value::Null => None,
            _ => None,
        });
        let headers = Self::parse_headers(headers_json)?;

        let builder = Self::apply_headers(client.get(url), &headers);
        let resp = builder
            .send()
            .await
            .map_err(|e| PluginError::Failed(format!("HTTP GET failed: {}", e)))?;

        Self::build_response(resp).await
    }

    /// Execute an HTTP POST request (async).
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

        let headers_json = args.get(2).and_then(|v| match v {
            Value::String(s) => Some(s.as_str()),
            Value::Null => None,
            _ => None,
        });
        let headers = Self::parse_headers(headers_json)?;

        let builder = Self::apply_headers(client.post(url).body(body), &headers);
        let resp = builder
            .send()
            .await
            .map_err(|e| PluginError::Failed(format!("HTTP POST failed: {}", e)))?;

        Self::build_response(resp).await
    }

    /// Execute an HTTP PUT request (async).
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

        let headers_json = args.get(2).and_then(|v| match v {
            Value::String(s) => Some(s.as_str()),
            Value::Null => None,
            _ => None,
        });
        let headers = Self::parse_headers(headers_json)?;

        let builder = Self::apply_headers(client.put(url).body(body), &headers);
        let resp = builder
            .send()
            .await
            .map_err(|e| PluginError::Failed(format!("HTTP PUT failed: {}", e)))?;

        Self::build_response(resp).await
    }

    /// Execute an HTTP DELETE request (async).
    async fn do_delete(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let client = self.client()?;

        let url = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: url".into()))?
            .as_str()?;

        let headers_json = args.get(1).and_then(|v| match v {
            Value::String(s) => Some(s.as_str()),
            Value::Null => None,
            _ => None,
        });
        let headers = Self::parse_headers(headers_json)?;

        let builder = Self::apply_headers(client.delete(url), &headers);
        let resp = builder
            .send()
            .await
            .map_err(|e| PluginError::Failed(format!("HTTP DELETE failed: {}", e)))?;

        Self::build_response(resp).await
    }

    /// Execute a generic HTTP request (async).
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

        let headers_json = args.get(2).and_then(|v| match v {
            Value::String(s) => Some(s.as_str()),
            Value::Null => None,
            _ => None,
        });
        let headers = Self::parse_headers(headers_json)?;

        let body = args.get(3).and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Null => None,
            _ => None,
        });

        let method: reqwest::Method = method_str
            .parse()
            .map_err(|e| PluginError::InvalidArg(format!("invalid HTTP method '{}': {}", method_str, e)))?;

        let mut builder = client.request(method, url);
        builder = Self::apply_headers(builder, &headers);
        if let Some(b) = body {
            builder = builder.body(b);
        }

        let resp = builder
            .send()
            .await
            .map_err(|e| PluginError::Failed(format!("HTTP request failed: {}", e)))?;

        Self::build_response(resp).await
    }
}

// ---------------------------------------------------------------------------
// Convention definitions helper
// ---------------------------------------------------------------------------

fn network_side_effect() -> Vec<SideEffect> {
    vec![SideEffect("network".into())]
}

// ---------------------------------------------------------------------------
// SomaPlugin trait implementation
// ---------------------------------------------------------------------------

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
            .map_err(|e| PluginError::Failed(format!("failed to create HTTP client: {}", e)))?;
        self.client = Some(client);
        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), PluginError> {
        self.client = None;
        Ok(())
    }

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
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                // We are inside a tokio runtime — use block_in_place + block_on
                // to avoid blocking an async worker thread.
                tokio::task::block_in_place(|| {
                    handle.block_on(self.execute_async(convention_id, args))
                })
            }
            Err(_) => {
                // No runtime — spin up a temporary one.
                let rt = tokio::runtime::Runtime::new()
                    .map_err(|e| PluginError::Failed(format!("failed to create tokio runtime: {}", e)))?;
                rt.block_on(self.execute_async(convention_id, args))
            }
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
                    "unknown convention_id: {}",
                    convention_id
                ))),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(HttpBridgePlugin::new()))
}
