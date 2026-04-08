use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// ResourceRef — pointer to a typed resource.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ResourceRef {
    pub resource_type: String,
    pub resource_id: String,
    pub version: u64,
    pub origin: String,
}

/// ResourceSpec — full resource definition provided by a pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSpec {
    pub resource_id: String,
    pub namespace: String,
    pub type_name: String,
    pub schema: serde_json::Value,
    pub identity_rules: IdentityRules,
    pub versioning_rules: VersioningRules,
    pub mutability: Mutability,
    pub relationships: Vec<Relationship>,
    pub exposure: ResourceExposure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityRules {
    pub key_fields: Vec<String>,
    pub auto_generate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersioningRules {
    pub strategy: VersioningStrategy,
    pub conflict_policy: ConflictPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersioningStrategy {
    Monotonic,
    Timestamp,
    Hash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    LastWriterWins,
    Reject,
    Merge,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mutability {
    Immutable,
    Mutable,
    AppendOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub relation_type: String,
    pub target_type: String,
    pub cardinality: Cardinality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    OneToOne,
    OneToMany,
    ManyToOne,
    ManyToMany,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceExposure {
    pub local: bool,
    pub remote: bool,
    pub sync_mode: SyncMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    Snapshot,
    Delta,
    EventStream,
}

/// A concrete resource instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub resource_ref: ResourceRef,
    pub namespace: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Resource patch for updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcePatch {
    pub resource_ref: ResourceRef,
    pub operations: Vec<PatchOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchOperation {
    pub op: PatchOp,
    pub path: String,
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchOp {
    Add,
    Remove,
    Replace,
    Move,
}
