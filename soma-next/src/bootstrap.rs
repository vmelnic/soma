//! Bootstrap: assembles a functioning Runtime from config and pack manifests.
//!
//! Loads pack manifests from JSON files, instantiates port adapters based on
//! declared PortKind, registers skills, and wires everything into a
//! SessionController via the adapter layer.

#[cfg(feature = "dylib-ports")]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use web_time::Instant;

use crate::adapters::{
    EpisodeMemoryAdapter, PolicyEngineAdapter,
    PortBackedSkillExecutor, RoutineMemoryAdapter, SchemaMemoryAdapter,
    SimpleCandidatePredictor, SimpleBeliefSource, SimpleSessionCritic,
    SkillRegistryAdapter,
};
use crate::runtime::remote::RemoteExecutor;
use crate::config::SomaConfig;
use crate::errors::{Result, SomaError};
use crate::memory::episodes::{DefaultEpisodeStore, EpisodeStore};
use crate::memory::persistence::{resolve_data_dir, DiskEpisodeStore, DiskSchemaStore, DiskRoutineStore};
use crate::memory::routines::{DefaultRoutineStore, RoutineStore};
use crate::memory::schemas::{DefaultSchemaStore, SchemaStore};
#[cfg(feature = "dylib-ports")]
use crate::runtime::dynamic_port::DynamicPortLoader;
use crate::runtime::goal::DefaultGoalRuntime;
use crate::runtime::policy::{DefaultPolicyRuntime, PolicyRuntime};
use crate::runtime::port::{DefaultPortRuntime, Port, PortRuntime};
use crate::runtime::metrics::RuntimeMetrics;
use crate::runtime::session::{SessionController, SessionControllerDeps};
use crate::runtime::skill::{DefaultSkillRuntime, SkillRuntime};

type SharedEpisodeStore = Arc<Mutex<dyn EpisodeStore + Send>>;
type SharedSchemaStore = Arc<Mutex<dyn SchemaStore + Send>>;
type SharedRoutineStore = Arc<Mutex<dyn RoutineStore + Send>>;
#[cfg(feature = "native-filesystem")]
use crate::ports::filesystem::FilesystemPort;
#[cfg(feature = "native-http")]
use crate::ports::http::HttpPort;
use crate::types::pack::PackSpec;
use crate::types::policy::{
    PolicyCondition, PolicyEffect, PolicyRule, PolicyRuleType, PolicySpec, PolicyTarget,
    PolicyTargetType,
};
#[cfg(feature = "native-http")]
use crate::runtime::mcp_client_port::McpClientPort;
#[cfg(any(feature = "native-filesystem", feature = "native-http"))]
use crate::types::port::PortKind;
use crate::types::port::{PortBackend, PortSpec};

/// The assembled runtime: session controller plus the goal runtime needed to
/// parse user input into GoalSpecs.
pub struct Runtime {
    pub session_controller: SessionController,
    pub goal_runtime: DefaultGoalRuntime,
    pub skill_runtime: DefaultSkillRuntime,
    pub port_runtime: Arc<Mutex<DefaultPortRuntime>>,
    pub episode_store: Arc<Mutex<dyn EpisodeStore + Send>>,
    pub schema_store: Arc<Mutex<dyn SchemaStore + Send>>,
    pub routine_store: Arc<Mutex<dyn RoutineStore + Send>>,
    pub pack_specs: Vec<PackSpec>,
    /// Shared metrics collector, threaded through to subsystems.
    pub metrics: Arc<RuntimeMetrics>,
    /// Goal embedder used for embedding-based episode retrieval and schema induction.
    pub embedder: Arc<dyn crate::memory::embedder::GoalEmbedder + Send + Sync>,
    /// Instant when the runtime was created, used for uptime and CPU tracking.
    pub start_time: Instant,
}

impl Runtime {
    /// Take a proprioception snapshot of this runtime's current state.
    ///
    /// Gathers resource usage (RSS, CPU), session counts, loaded capability
    /// counts, uptime, and peer connection count into a single `SelfModel`.
    pub fn self_model(&self) -> crate::runtime::proprioception::SelfModel {
        use crate::runtime::proprioception;
        use std::sync::atomic::Ordering;

        let counts = proprioception::RuntimeCounts {
            active_sessions: self.metrics.active_sessions.load(Ordering::Relaxed),
            loaded_packs: self.pack_specs.len() as u64,
            registered_skills: self.skill_runtime.list_skills(None).len() as u64,
            registered_ports: self
                .port_runtime
                .lock()
                .map(|pr| pr.list_ports(None).len() as u64)
                .unwrap_or(0),
            peer_connections: 0, // updated by distributed layer when available
        };

        proprioception::snapshot(self.start_time, &counts)
    }
}

/// Build a fully wired Runtime from config and pack paths.
///
/// For each pack path, reads the manifest JSON, validates the pack,
/// instantiates port adapters by kind, registers skills, then assembles
/// the SessionController with all adapter dependencies.
pub fn bootstrap(config: &SomaConfig, pack_paths: &[String]) -> Result<Runtime> {
    // Configure the port runtime with a sandbox profile that permits
    // filesystem and network access, matching the capabilities that packs
    // may declare. The sandbox profile is the runtime's view of what the
    // host environment supports.
    let sandbox = crate::runtime::port::RuntimeSandboxProfile {
        filesystem_access: true,
        network_access: true,
        device_access: false,
        process_access: false,
        memory_limit_mb: None,
        cpu_limit_percent: None,
        time_limit_ms: None,
        syscall_limit: None,
    };
    let mut port_runtime = DefaultPortRuntime::with_sandbox_profile(sandbox);
    let mut skill_runtime = DefaultSkillRuntime::new();
    let mut pack_specs: Vec<PackSpec> = Vec::new();

    #[cfg(feature = "dylib-ports")]
    let mut dynamic_loader = {
        let search_paths: Vec<PathBuf> = config
            .ports
            .plugin_path
            .iter()
            .map(PathBuf::from)
            .collect();
        DynamicPortLoader::with_signature_policy(
            search_paths,
            config.ports.require_signatures,
        )
    };

    for path in pack_paths {
        let manifest_content = std::fs::read_to_string(path).map_err(|e| {
            SomaError::Pack(format!("failed to read pack manifest '{}': {}", path, e))
        })?;

        let pack_spec: PackSpec = serde_json::from_str(&manifest_content).map_err(|e| {
            SomaError::Pack(format!("failed to parse pack manifest '{}': {}", path, e))
        })?;

        // Register ports declared in the pack.
        for port_spec in &pack_spec.ports {
            let (adapter, effective_spec) = match create_port_adapter(
                port_spec,
                #[cfg(feature = "dylib-ports")]
                &mut dynamic_loader,
            ) {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!(
                        port_id = %port_spec.port_id,
                        kind = ?port_spec.kind,
                        error = %e,
                        "failed to create port adapter, skipping"
                    );
                    continue;
                }
            };

            let spec_to_register = effective_spec.unwrap_or_else(|| port_spec.clone());
            let port_id = spec_to_register.port_id.clone();
            port_runtime.register_port(spec_to_register, adapter)?;
            port_runtime.activate(&port_id)?;
        }

        // Register skills declared in the pack.
        for skill_spec in &pack_spec.skills {
            skill_runtime.register_skill(skill_spec.clone())?;
        }

        pack_specs.push(pack_spec);
    }

    let port_runtime = Arc::new(Mutex::new(port_runtime));

    let data_dir = resolve_data_dir(&config.soma.data_dir);
    let (episode_store, schema_store, routine_store): (
        SharedEpisodeStore,
        SharedSchemaStore,
        SharedRoutineStore,
    ) = if data_dir.as_os_str().is_empty() {
        (
            Arc::new(Mutex::new(DefaultEpisodeStore::new())),
            Arc::new(Mutex::new(DefaultSchemaStore::new())),
            Arc::new(Mutex::new(DefaultRoutineStore::new())),
        )
    } else {
        tracing::info!(data_dir = %data_dir.display(), "using disk-backed memory stores");
        (
            Arc::new(Mutex::new(DiskEpisodeStore::new(&data_dir)?)),
            Arc::new(Mutex::new(DiskSchemaStore::new(&data_dir)?)),
            Arc::new(Mutex::new(DiskRoutineStore::new(&data_dir)?)),
        )
    };

    let embedder: Arc<dyn crate::memory::embedder::GoalEmbedder + Send + Sync> = Arc::new(
        crate::memory::embedder::HashEmbedder::new(),
    );

    let skill_registry = SkillRegistryAdapter::new(&skill_runtime);
    let skill_executor = PortBackedSkillExecutor::new(Arc::clone(&port_runtime));
    let episode_memory = EpisodeMemoryAdapter::new(Arc::clone(&episode_store), Arc::clone(&embedder));
    let schema_memory = SchemaMemoryAdapter::new(Arc::clone(&schema_store));
    let routine_memory = RoutineMemoryAdapter::new(Arc::clone(&routine_store));

    // Build the policy engine with default safety rules from host config.
    let policy_runtime = DefaultPolicyRuntime::new();
    register_default_safety_policies(&policy_runtime, config)?;

    // Also register any policies declared in pack manifests.
    for pack_spec in &pack_specs {
        for policy_spec in &pack_spec.policies {
            if let Err(e) = policy_runtime.register_policy(policy_spec.clone()) {
                tracing::warn!(
                    policy_id = %policy_spec.policy_id,
                    pack = %pack_spec.id,
                    error = %e,
                    "pack policy rejected (may conflict with host policy), skipping"
                );
            }
        }
    }

    let policy_engine = PolicyEngineAdapter::new(policy_runtime, config.runtime.max_steps);

    let deps = SessionControllerDeps {
        belief_source: Box::new(SimpleBeliefSource::new()),
        episode_memory: Box::new(episode_memory),
        schema_memory: Box::new(schema_memory),
        routine_memory: Box::new(routine_memory),
        skill_registry: Box::new(skill_registry),
        skill_executor: Box::new(skill_executor),
        predictor: Box::new(SimpleCandidatePredictor::new()),
        critic: Box::new(SimpleSessionCritic::new()),
        policy_engine: Box::new(policy_engine),
        remote_executor: None,
        capability_scope_checker: None,
    };

    let metrics = Arc::new(RuntimeMetrics::new());

    let session_controller = SessionController::new(deps, Arc::clone(&metrics));
    let goal_runtime = DefaultGoalRuntime::new();

    Ok(Runtime {
        session_controller,
        goal_runtime,
        skill_runtime,
        port_runtime,
        episode_store,
        schema_store,
        routine_store,
        pack_specs,
        metrics,
        embedder,
        start_time: Instant::now(),
    })
}

/// Same as `bootstrap` but wires a remote executor into the session controller.
/// Used when `--peer` is specified on the command line.
pub fn bootstrap_with_remote(
    config: &SomaConfig,
    pack_paths: &[String],
    remote_executor: Box<dyn RemoteExecutor>,
) -> Result<Runtime> {
    let sandbox = crate::runtime::port::RuntimeSandboxProfile {
        filesystem_access: true,
        network_access: true,
        device_access: false,
        process_access: false,
        memory_limit_mb: None,
        cpu_limit_percent: None,
        time_limit_ms: None,
        syscall_limit: None,
    };
    let mut port_runtime = DefaultPortRuntime::with_sandbox_profile(sandbox);
    let mut skill_runtime = DefaultSkillRuntime::new();
    let mut pack_specs: Vec<PackSpec> = Vec::new();

    #[cfg(feature = "dylib-ports")]
    let mut dynamic_loader = {
        let search_paths: Vec<PathBuf> = config
            .ports
            .plugin_path
            .iter()
            .map(PathBuf::from)
            .collect();
        DynamicPortLoader::with_signature_policy(
            search_paths,
            config.ports.require_signatures,
        )
    };

    for path in pack_paths {
        let manifest_content = std::fs::read_to_string(path).map_err(|e| {
            SomaError::Pack(format!("failed to read pack manifest '{}': {}", path, e))
        })?;

        let pack_spec: PackSpec = serde_json::from_str(&manifest_content).map_err(|e| {
            SomaError::Pack(format!("failed to parse pack manifest '{}': {}", path, e))
        })?;

        for port_spec in &pack_spec.ports {
            let (adapter, effective_spec) = match create_port_adapter(
                port_spec,
                #[cfg(feature = "dylib-ports")]
                &mut dynamic_loader,
            ) {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!(
                        port_id = %port_spec.port_id,
                        kind = ?port_spec.kind,
                        error = %e,
                        "failed to create port adapter, skipping"
                    );
                    continue;
                }
            };

            let spec_to_register = effective_spec.unwrap_or_else(|| port_spec.clone());
            let port_id = spec_to_register.port_id.clone();
            port_runtime.register_port(spec_to_register, adapter)?;
            port_runtime.activate(&port_id)?;
        }

        for skill_spec in &pack_spec.skills {
            skill_runtime.register_skill(skill_spec.clone())?;
        }

        pack_specs.push(pack_spec);
    }

    let port_runtime = Arc::new(Mutex::new(port_runtime));

    let data_dir = resolve_data_dir(&config.soma.data_dir);
    let (episode_store, schema_store, routine_store): (
        SharedEpisodeStore,
        SharedSchemaStore,
        SharedRoutineStore,
    ) = if data_dir.as_os_str().is_empty() {
        (
            Arc::new(Mutex::new(DefaultEpisodeStore::new())),
            Arc::new(Mutex::new(DefaultSchemaStore::new())),
            Arc::new(Mutex::new(DefaultRoutineStore::new())),
        )
    } else {
        tracing::info!(data_dir = %data_dir.display(), "using disk-backed memory stores (remote bootstrap)");
        (
            Arc::new(Mutex::new(DiskEpisodeStore::new(&data_dir)?)),
            Arc::new(Mutex::new(DiskSchemaStore::new(&data_dir)?)),
            Arc::new(Mutex::new(DiskRoutineStore::new(&data_dir)?)),
        )
    };

    let embedder: Arc<dyn crate::memory::embedder::GoalEmbedder + Send + Sync> = Arc::new(
        crate::memory::embedder::HashEmbedder::new(),
    );

    let skill_registry = SkillRegistryAdapter::new(&skill_runtime);
    let skill_executor = PortBackedSkillExecutor::new(Arc::clone(&port_runtime));
    let episode_memory = EpisodeMemoryAdapter::new(Arc::clone(&episode_store), Arc::clone(&embedder));
    let schema_memory = SchemaMemoryAdapter::new(Arc::clone(&schema_store));
    let routine_memory = RoutineMemoryAdapter::new(Arc::clone(&routine_store));

    let policy_runtime = DefaultPolicyRuntime::new();
    register_default_safety_policies(&policy_runtime, config)?;

    for pack_spec in &pack_specs {
        for policy_spec in &pack_spec.policies {
            if let Err(e) = policy_runtime.register_policy(policy_spec.clone()) {
                tracing::warn!(
                    policy_id = %policy_spec.policy_id,
                    pack = %pack_spec.id,
                    error = %e,
                    "pack policy rejected (may conflict with host policy), skipping"
                );
            }
        }
    }

    let policy_engine = PolicyEngineAdapter::new(policy_runtime, config.runtime.max_steps);

    let deps = SessionControllerDeps {
        belief_source: Box::new(SimpleBeliefSource::new()),
        episode_memory: Box::new(episode_memory),
        schema_memory: Box::new(schema_memory),
        routine_memory: Box::new(routine_memory),
        skill_registry: Box::new(skill_registry),
        skill_executor: Box::new(skill_executor),
        predictor: Box::new(SimpleCandidatePredictor::new()),
        critic: Box::new(SimpleSessionCritic::new()),
        policy_engine: Box::new(policy_engine),
        remote_executor: Some(remote_executor),
        capability_scope_checker: None,
    };

    let metrics = Arc::new(RuntimeMetrics::new());

    let session_controller = SessionController::new(deps, Arc::clone(&metrics));
    let goal_runtime = DefaultGoalRuntime::new();

    Ok(Runtime {
        session_controller,
        goal_runtime,
        skill_runtime,
        port_runtime,
        episode_store,
        schema_store,
        routine_store,
        pack_specs,
        metrics,
        embedder,
        start_time: Instant::now(),
    })
}

/// Build a Runtime from already-parsed pack specs, skipping all filesystem
/// I/O. Used by the wasm entry point — the browser has no filesystem to
/// read manifests from, so the JS harness supplies the pack JSON directly
/// and we parse it before calling this function.
///
/// Structurally equivalent to `bootstrap()` but starts from a `Vec<PackSpec>`
/// instead of a `&[String]` of file paths. `disk-persistence`-gated code
/// paths (DiskEpisodeStore / DiskSchemaStore / DiskRoutineStore) cannot run
/// on wasm, so this function only ever uses the in-memory defaults.
pub fn bootstrap_from_specs(
    config: &SomaConfig,
    pack_specs_input: Vec<PackSpec>,
) -> Result<Runtime> {
    let sandbox = crate::runtime::port::RuntimeSandboxProfile {
        filesystem_access: false,
        network_access: false,
        device_access: false,
        process_access: false,
        memory_limit_mb: None,
        cpu_limit_percent: None,
        time_limit_ms: None,
        syscall_limit: None,
    };
    let mut port_runtime = DefaultPortRuntime::with_sandbox_profile(sandbox);
    let mut skill_runtime = DefaultSkillRuntime::new();
    let mut pack_specs: Vec<PackSpec> = Vec::new();

    #[cfg(feature = "dylib-ports")]
    let mut dynamic_loader = {
        let search_paths: Vec<PathBuf> = config
            .ports
            .plugin_path
            .iter()
            .map(PathBuf::from)
            .collect();
        DynamicPortLoader::with_signature_policy(
            search_paths,
            config.ports.require_signatures,
        )
    };

    for pack_spec in pack_specs_input.into_iter() {
        for port_spec in &pack_spec.ports {
            let (adapter, effective_spec) = match create_port_adapter(
                port_spec,
                #[cfg(feature = "dylib-ports")]
                &mut dynamic_loader,
            ) {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!(
                        port_id = %port_spec.port_id,
                        kind = ?port_spec.kind,
                        error = %e,
                        "failed to create port adapter, skipping"
                    );
                    continue;
                }
            };

            let spec_to_register = effective_spec.unwrap_or_else(|| port_spec.clone());
            let port_id = spec_to_register.port_id.clone();
            port_runtime.register_port(spec_to_register, adapter)?;
            port_runtime.activate(&port_id)?;
        }

        for skill_spec in &pack_spec.skills {
            skill_runtime.register_skill(skill_spec.clone())?;
        }

        pack_specs.push(pack_spec);
    }

    let port_runtime = Arc::new(Mutex::new(port_runtime));

    // In-memory stores only: disk-persistence is unavailable on wasm
    // and pointless for bootstrap-from-specs usage anyway (the caller
    // doesn't want side effects in ~/.soma/data from a browser tab).
    let episode_store: SharedEpisodeStore = Arc::new(Mutex::new(DefaultEpisodeStore::new()));
    let schema_store: SharedSchemaStore = Arc::new(Mutex::new(DefaultSchemaStore::new()));
    let routine_store: SharedRoutineStore = Arc::new(Mutex::new(DefaultRoutineStore::new()));

    let embedder: Arc<dyn crate::memory::embedder::GoalEmbedder + Send + Sync> = Arc::new(
        crate::memory::embedder::HashEmbedder::new(),
    );

    let skill_registry = SkillRegistryAdapter::new(&skill_runtime);
    let skill_executor = PortBackedSkillExecutor::new(Arc::clone(&port_runtime));
    let episode_memory = EpisodeMemoryAdapter::new(Arc::clone(&episode_store), Arc::clone(&embedder));
    let schema_memory = SchemaMemoryAdapter::new(Arc::clone(&schema_store));
    let routine_memory = RoutineMemoryAdapter::new(Arc::clone(&routine_store));

    let policy_runtime = DefaultPolicyRuntime::new();
    register_default_safety_policies(&policy_runtime, config)?;

    for pack_spec in &pack_specs {
        for policy_spec in &pack_spec.policies {
            if let Err(e) = policy_runtime.register_policy(policy_spec.clone()) {
                tracing::warn!(
                    policy_id = %policy_spec.policy_id,
                    pack = %pack_spec.id,
                    error = %e,
                    "pack policy rejected (may conflict with host policy), skipping"
                );
            }
        }
    }

    let policy_engine = PolicyEngineAdapter::new(policy_runtime, config.runtime.max_steps);

    let deps = SessionControllerDeps {
        belief_source: Box::new(SimpleBeliefSource::new()),
        episode_memory: Box::new(episode_memory),
        schema_memory: Box::new(schema_memory),
        routine_memory: Box::new(routine_memory),
        skill_registry: Box::new(skill_registry),
        skill_executor: Box::new(skill_executor),
        predictor: Box::new(SimpleCandidatePredictor::new()),
        critic: Box::new(SimpleSessionCritic::new()),
        policy_engine: Box::new(policy_engine),
        remote_executor: None,
        capability_scope_checker: None,
    };

    let metrics = Arc::new(RuntimeMetrics::new());

    let session_controller = SessionController::new(deps, Arc::clone(&metrics));
    let goal_runtime = DefaultGoalRuntime::new();

    Ok(Runtime {
        session_controller,
        goal_runtime,
        skill_runtime,
        port_runtime,
        episode_store,
        schema_store,
        routine_store,
        pack_specs,
        metrics,
        embedder,
        start_time: Instant::now(),
    })
}

/// Register host-level safety policies that enforce fundamental runtime constraints.
///
/// These rules are registered under the "host" namespace, giving them the highest
/// precedence. Pack policies cannot widen what these rules restrict.
///
/// Default safety rules:
/// 1. Budget enforcement: deny when budget is exhausted (budget_min = 0).
/// 2. Destructive operations: require confirmation for destructive/irreversible skills.
/// 3. Bounded loops: deny when step count exceeds max_steps.
/// 4. Write operations: constrain write/delete operations (logged at runtime by the adapter).
fn register_default_safety_policies(
    runtime: &DefaultPolicyRuntime,
    config: &SomaConfig,
) -> Result<()> {
    let host_policy = PolicySpec {
        policy_id: "host.default_safety".to_string(),
        namespace: "host".to_string(),
        rules: vec![
            // Budget enforcement is handled directly by PolicyEngineAdapter::check_budget(),
            // which correctly checks step count and resource depletion. No inner rule needed.

            // Rule 1: Require confirmation for destructive operations.
            // This adds a RequireConfirmation rule for all skills. At runtime, the
            // adapter checks the skill's SideEffectClass and only triggers the
            // confirmation gate when the class is Destructive or Irreversible.
            // For the policy engine's rule matching, this acts as a broad signal
            // that destructive ops within untrusted namespaces need confirmation.
            PolicyRule {
                rule_id: "host.destructive_confirmation".to_string(),
                rule_type: PolicyRuleType::RequireConfirmation,
                target: PolicyTarget {
                    target_type: PolicyTargetType::Skill,
                    identifiers: vec![], // wildcard — the adapter filters by side-effect class
                    scope: None,
                    trust_level: None,
                },
                effect: PolicyEffect::RequireConfirmation,
                conditions: vec![PolicyCondition {
                    condition_type: "trust_level_max".to_string(),
                    expression: serde_json::json!("verified"),
                }],
                priority: 500,
            },
            // Rule 3: Allow read-only operations from any trust level.
            PolicyRule {
                rule_id: "host.allow_readonly".to_string(),
                rule_type: PolicyRuleType::Allow,
                target: PolicyTarget {
                    target_type: PolicyTargetType::Skill,
                    identifiers: vec![], // wildcard — applies to all skills
                    scope: None,
                    trust_level: None,
                },
                effect: PolicyEffect::Allow,
                conditions: vec![], // unconditional — the adapter only invokes this for read-only ops
                priority: 1000,
            },
        ],
        allowed_capabilities: vec![],
        denied_capabilities: vec![],
        scope_limits: None,
        trust_classification: None,
        confirmation_requirements: vec!["destructive".to_string(), "irreversible".to_string()],
        destructive_action_constraints: vec![
            "require_confirmation".to_string(),
            "log_warning".to_string(),
        ],
        remote_exposure_limits: vec![],
    };

    runtime.register_policy(host_policy)?;

    tracing::info!(
        max_steps = config.runtime.max_steps,
        "registered host safety policies (budget enforcement, destructive operation gates, bounded loops)"
    );

    Ok(())
}

/// Create a port adapter for a given spec.
///
/// Built-in port kinds (Filesystem, Http) are instantiated directly when the
/// corresponding feature is enabled. Everything else dispatches on
/// `spec.backend`:
///   * `Dylib` — load a native `cdylib` through `DynamicPortLoader` (requires
///     the `dylib-ports` feature).
///   * `McpClient` — spawn/connect to an MCP server, run initialize +
///     tools/list discovery, and wrap it in an `McpClientPort` (requires the
///     `native-http` feature for its blocking HTTP + subprocess paths).
///
/// Returns the adapter and, when discovery added capabilities to the spec,
/// a replacement `PortSpec` the caller should register instead of the
/// original manifest version.
///
/// When a backend-specific feature is disabled (e.g. on wasm, where
/// `dylib-ports` and `native-http` are off), declarations of that kind are
/// returned as a `SomaError::Port` and the caller logs + skips them. This
/// keeps pack loading going for any remaining ports that *can* be loaded.
#[allow(unused_variables)]
fn create_port_adapter(
    spec: &PortSpec,
    #[cfg(feature = "dylib-ports")] loader: &mut DynamicPortLoader,
) -> Result<(Box<dyn Port>, Option<PortSpec>)> {
    match spec.kind {
        #[cfg(feature = "native-filesystem")]
        PortKind::Filesystem => return Ok((Box::new(FilesystemPort::new()), None)),
        #[cfg(feature = "native-http")]
        PortKind::Http => return Ok((Box::new(HttpPort::new()), None)),
        _ => {}
    }

    match spec.backend.clone() {
        PortBackend::Dylib { library_name } => {
            #[cfg(feature = "dylib-ports")]
            {
                let lib_name = library_name
                    .unwrap_or_else(|| format!("soma_port_{}", spec.port_id));
                let port = loader.load_port(&lib_name)?;
                Ok((port, None))
            }
            #[cfg(not(feature = "dylib-ports"))]
            {
                let _ = library_name;
                Err(SomaError::Port(format!(
                    "port '{}' requests Dylib backend but the runtime was built without the `dylib-ports` feature",
                    spec.port_id
                )))
            }
        }
        PortBackend::McpClient { transport } => {
            #[cfg(feature = "native-http")]
            {
                let (port, effective_spec) =
                    McpClientPort::spawn_and_discover(spec.clone(), transport)?;
                Ok((Box::new(port), Some(effective_spec)))
            }
            #[cfg(not(feature = "native-http"))]
            {
                let _ = transport;
                Err(SomaError::Port(format!(
                    "port '{}' requests McpClient backend but the runtime was built without the `native-http` feature",
                    spec.port_id
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::port::Port;

    #[test]
    fn bootstrap_with_no_packs() {
        let config = SomaConfig::default();
        let rt = bootstrap(&config, &[]).unwrap();
        // Should succeed with zero packs — just no skills available.
        let _ = rt.goal_runtime;
    }

    #[test]
    fn bootstrap_with_nonexistent_pack_fails() {
        let config = SomaConfig::default();
        let result = bootstrap(&config, &["/tmp/no_such_pack_manifest.json".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn bootstrap_with_valid_manifest() {
        let dir = std::env::temp_dir().join("soma_bootstrap_test");
        let _ = std::fs::create_dir_all(&dir);
        let manifest_path = dir.join("manifest.json");

        let manifest = make_test_manifest();
        std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).unwrap())
            .unwrap();

        let config = SomaConfig::default();
        let result = bootstrap(
            &config,
            &[manifest_path.to_string_lossy().to_string()],
        );

        // Clean up before asserting so it always runs.
        let _ = std::fs::remove_dir_all(&dir);

        let rt = result.unwrap();
        let _ = rt.session_controller;
    }

    fn make_test_manifest() -> PackSpec {
        use crate::types::common::*;
        use crate::types::pack::*;
        
        use crate::types::skill::*;
        use semver::{Version, VersionReq};

        let fs_port = crate::ports::filesystem::FilesystemPort::new();
        let port_spec = fs_port.spec().clone();

        let readdir_skill = SkillSpec {
            skill_id: "reference.readdir".to_string(),
            namespace: "reference".to_string(),
            pack: "reference".to_string(),
            kind: SkillKind::Primitive,
            name: "Read Directory".to_string(),
            description: "List entries in a directory via the filesystem port".to_string(),
            version: "0.1.0".to_string(),
            inputs: SchemaRef { schema: serde_json::json!({"type": "object", "required": ["path"], "properties": {"path": {"type": "string"}}}) },
            outputs: SchemaRef { schema: serde_json::json!({"type": "object", "properties": {"entries": {"type": "array"}, "count": {"type": "integer"}}}) },
            required_resources: vec![],
            preconditions: vec![],
            expected_effects: vec![],
            observables: vec![ObservableDecl { field: "entries".to_string(), role: ObservableRole::ConfirmSuccess }],
            termination_conditions: vec![
                TerminationCondition { condition_type: TerminationType::Success, expression: serde_json::json!({"entries": "non_empty"}), description: "directory listing returned".to_string() },
                TerminationCondition { condition_type: TerminationType::Failure, expression: serde_json::json!({"error": "any"}), description: "filesystem error".to_string() },
            ],
            rollback_or_compensation: RollbackSpec { support: RollbackSupport::Irreversible, compensation_skill: None, description: "read-only, no rollback needed".to_string() },
            cost_prior: CostPrior { latency: LatencyProfile { expected_latency_ms: 1, p95_latency_ms: 10, max_latency_ms: 1000 }, resource_cost: CostProfile { cpu_cost_class: CostClass::Negligible, memory_cost_class: CostClass::Negligible, io_cost_class: CostClass::Low, network_cost_class: CostClass::Negligible, energy_cost_class: CostClass::Negligible } },
            risk_class: RiskClass::Negligible,
            determinism: DeterminismClass::Deterministic,
            remote_exposure: RemoteExposureDecl { remote_scope: CapabilityScope::Local, peer_trust_requirements: "none".to_string(), serialization_requirements: "json".to_string(), rate_limits: "none".to_string(), replay_protection: false, observation_streaming: false, delegation_support: false, enabled: false },
            tags: vec!["filesystem".to_string()],
            aliases: vec![],
            capability_requirements: vec!["port:filesystem/readdir".to_string()],
            subskills: vec![],
            guard_conditions: vec![],
            match_conditions: vec![],
            telemetry_fields: vec![],
            policy_overrides: vec![],
            confidence_threshold: None,
            locality: None,
            remote_endpoint: None,
            remote_trust_requirement: None,
            remote_capability_contract: None,
            fallback_skill: None,
            invalidation_conditions: vec![],
            nondeterminism_sources: vec![],
            partial_success_behavior: None,
        };

        PackSpec {
            id: "reference".to_string(),
            name: "Reference Pack".to_string(),
            version: Version::new(0, 1, 0),
            runtime_compatibility: VersionReq::parse(">=0.1.0").unwrap(),
            namespace: "reference".to_string(),
            capabilities: vec![CapabilityGroup {
                group_name: "filesystem".to_string(),
                scope: CapabilityScope::Local,
                capabilities: vec!["readdir".to_string()],
            }],
            dependencies: vec![],
            resources: vec![],
            skills: vec![readdir_skill],
            schemas: vec![],
            routines: vec![],
            policies: vec![],
            exposure: ExposureSpec {
                local_skills: vec!["reference.readdir".to_string()],
                remote_skills: vec![],
                local_resources: vec![],
                remote_resources: vec![],
                default_deny_destructive: true,
            },
            observability: ObservabilitySpec {
                health_checks: vec!["filesystem_accessible".to_string()],
                version_metadata: serde_json::json!({"version": "0.1.0"}),
                dependency_status: vec![],
                capability_inventory: vec!["readdir".to_string()],
                expected_latency_classes: vec!["fast".to_string()],
                expected_failure_modes: vec!["permission_denied".to_string(), "not_found".to_string()],
                trace_categories: vec!["filesystem".to_string()],
                metric_names: vec!["fs_readdir_count".to_string()],
                pack_load_state: "active".to_string(),
            },
            description: Some("Reference pack with filesystem operations".to_string()),
            authors: vec![],
            license: None,
            homepage: None,
            repository: None,
            targets: vec![],
            build: None,
            checksum: None,
            signature: None,
            entrypoints: vec![],
            tags: vec!["filesystem".to_string()],
            deprecation: None,
            ports: vec![port_spec],
            port_dependencies: vec![],
        }
    }
}
