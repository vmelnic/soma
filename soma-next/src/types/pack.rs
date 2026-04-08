use serde::{Deserialize, Serialize};
use semver::{Version, VersionReq};

use super::common::CapabilityScope;
use super::policy::PolicySpec;
use super::port::PortSpec;
use super::resource::ResourceSpec;
use super::routine::Routine;
use super::schema::Schema;
use super::skill::SkillSpec;

/// PackLifecycleState from pack-spec.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackLifecycleState {
    Discovered,
    Validated,
    Staged,
    Active,
    Degraded,
    Quarantined,
    Suspended,
    Unloaded,
    Failed,
}

/// PackSpec — the canonical pack manifest.
/// Full compliance with pack-spec.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackSpec {
    pub id: String,
    pub name: String,
    pub version: Version,
    pub runtime_compatibility: VersionReq,
    pub namespace: String,
    pub capabilities: Vec<CapabilityGroup>,
    pub dependencies: Vec<DependencySpec>,
    pub resources: Vec<ResourceSpec>,
    pub skills: Vec<SkillSpec>,
    pub schemas: Vec<Schema>,
    pub routines: Vec<Routine>,
    pub policies: Vec<PolicySpec>,
    pub exposure: ExposureSpec,
    pub observability: ObservabilitySpec,

    // Recommended fields (pack-spec.md Section "Recommended Fields")
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub build: Option<BuildSpec>,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub entrypoints: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub deprecation: Option<DeprecationSpec>,
    #[serde(default)]
    pub ports: Vec<PortSpec>,
    /// Declared dependencies on specific port versions.
    /// If a port in this list is not registered at the required version, the
    /// port is considered unavailable for this pack's dependency path.
    #[serde(default)]
    pub port_dependencies: Vec<PortDependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityGroup {
    pub group_name: String,
    pub scope: CapabilityScope,
    pub capabilities: Vec<String>,
}

/// Declares a dependency on a specific port version.
///
/// If the declared port is not available at the required version, the pack
/// treats it as an unsatisfied dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortDependency {
    pub port_id: String,
    /// Semver requirement string (e.g., ">=1.0.0, <2.0.0").
    pub version_range: VersionReq,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencySpec {
    pub pack_id: String,
    pub version_range: String,
    pub required: bool,
    pub capabilities_needed: Vec<String>,
    #[serde(default)]
    pub feature_flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExposureSpec {
    pub local_skills: Vec<String>,
    pub remote_skills: Vec<RemoteExposureEntry>,
    pub local_resources: Vec<String>,
    pub remote_resources: Vec<RemoteExposureEntry>,
    /// Default deny for destructive, credential, secret, device-actuation, policy-mutation
    /// capabilities (pack-spec.md Section "Remote Safety").
    #[serde(default = "default_true")]
    pub default_deny_destructive: bool,
}

fn default_true() -> bool {
    true
}

/// An entry for a remotely-exposed capability with the 7 required fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteExposureEntry {
    pub capability_id: String,
    pub remote_scope: CapabilityScope,
    pub peer_trust_requirements: String,
    pub serialization_requirements: String,
    pub rate_limits: String,
    pub replay_protection: bool,
    pub observation_streaming: bool,
    pub delegation_support: bool,
}

/// ObservabilitySpec — all 9 required fields from pack-spec.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilitySpec {
    pub health_checks: Vec<String>,
    pub version_metadata: serde_json::Value,
    pub dependency_status: Vec<DependencyStatusEntry>,
    pub capability_inventory: Vec<String>,
    pub expected_latency_classes: Vec<String>,
    pub expected_failure_modes: Vec<String>,
    pub trace_categories: Vec<String>,
    pub metric_names: Vec<String>,
    pub pack_load_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyStatusEntry {
    pub pack_id: String,
    pub status: String,
}

/// BuildSpec — recommended field describing how a pack is built.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSpec {
    pub tool: String,
    pub args: Vec<String>,
}

/// DeprecationSpec — recommended field describing deprecation status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeprecationSpec {
    pub deprecated: bool,
    pub message: String,
    pub replacement: Option<String>,
}

/// Pack-level failure class (pack-spec.md Section "Failure Classes").
/// The runtime MUST distinguish at least these 9 failure classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackFailureClass {
    ManifestFailure,
    SchemaFailure,
    DependencyFailure,
    NamespaceCollision,
    PolicyFailure,
    PortFailure,
    SkillExecutionFailure,
    RemotePeerFailure,
    IntegrityFailure,
}
