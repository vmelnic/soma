use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::resource::*;

// ---------------------------------------------------------------------------
// ResourceRuntime trait
// ---------------------------------------------------------------------------

/// ResourceRuntime — typed resource definitions, indexing, versioning,
/// and patch-based updates.
///
/// Responsibilities:
/// - Register resource types (from packs) with schemas and identity/versioning rules
/// - Create, get, update, delete, list, and query resource instances
/// - Validate data against the registered schema before create/update
/// - Bump versions on every successful update
/// - Enforce mutability constraints (immutable, append-only)
pub trait ResourceRuntime: Send + Sync {
    /// Register a resource type definition (from a pack).
    /// The type is keyed by its `type_name`. Duplicate registrations are rejected.
    fn register_type(&self, spec: ResourceSpec) -> Result<()>;

    /// Create a new resource instance in `namespace` of `type_name`.
    /// The resource is validated against the registered schema, assigned a unique
    /// id (if `auto_generate` is set), and stored with version 1.
    fn create(
        &self,
        namespace: &str,
        type_name: &str,
        data: serde_json::Value,
    ) -> Result<Resource>;

    /// Get a resource by type and id. Returns `None` if not found.
    fn get(&self, resource_type: &str, resource_id: &str) -> Result<Option<Resource>>;

    /// Apply a patch to an existing resource, bumping its version.
    /// Returns the updated resource. Rejects patches on immutable resources.
    fn update(&self, resource_ref: &ResourceRef, patch: ResourcePatch) -> Result<Resource>;

    /// Delete a resource by ref. Returns an error if the resource does not exist
    /// or the type is immutable.
    fn delete(&self, resource_ref: &ResourceRef) -> Result<()>;

    /// List all resources of a given type, optionally filtered by namespace.
    fn list(&self, resource_type: &str, namespace: Option<&str>) -> Result<Vec<Resource>>;

    /// Query resources of a given type using a simple JSON filter.
    /// Each top-level key in `filter` is matched against the resource data with
    /// equality semantics.
    fn query(&self, resource_type: &str, filter: serde_json::Value) -> Result<Vec<Resource>>;
}

// ---------------------------------------------------------------------------
// DefaultResourceRuntime
// ---------------------------------------------------------------------------

/// In-memory implementation of `ResourceRuntime`.
///
/// Uses interior mutability (`std::sync::RwLock`) so the trait methods take `&self`.
pub struct DefaultResourceRuntime {
    /// Registered resource type specs, keyed by `type_name`.
    specs: std::sync::RwLock<HashMap<String, ResourceSpec>>,
    /// Resource instances, keyed by `(resource_type, resource_id)`.
    instances: std::sync::RwLock<HashMap<(String, String), Resource>>,
}

impl DefaultResourceRuntime {
    pub fn new() -> Self {
        Self {
            specs: std::sync::RwLock::new(HashMap::new()),
            instances: std::sync::RwLock::new(HashMap::new()),
        }
    }

    // --- helpers ---

    /// Look up a registered spec by type_name. Returns a clone.
    fn get_spec(&self, type_name: &str) -> Result<ResourceSpec> {
        let specs = self.specs.read().unwrap();
        specs.get(type_name).cloned().ok_or_else(|| {
            SomaError::Resource(format!("resource type not registered: {type_name}"))
        })
    }

    /// Validate `data` against the spec's schema.
    ///
    /// The schema is a JSON object whose top-level keys describe required fields.
    /// Each key maps to an object with a `"type"` string describing the expected
    /// JSON type (`"string"`, `"number"`, `"boolean"`, `"object"`, `"array"`).
    ///
    /// If the schema is `null`, `{}`, or not an object, validation is skipped.
    fn validate_against_schema(schema: &serde_json::Value, data: &serde_json::Value) -> Result<()> {
        let schema_obj = match schema.as_object() {
            Some(obj) if !obj.is_empty() => obj,
            _ => return Ok(()), // no schema or empty schema — allow anything
        };

        let data_obj = data.as_object().ok_or_else(|| {
            SomaError::Resource("resource data must be a JSON object".to_string())
        })?;

        for (field, field_schema) in schema_obj {
            // Determine the expected type string, if provided.
            let expected_type = field_schema
                .as_object()
                .and_then(|fs| fs.get("type"))
                .and_then(|t| t.as_str());

            let value = match data_obj.get(field) {
                Some(v) => v,
                None => {
                    // If the schema says this field is required (default: true),
                    // its absence is an error.
                    let required = field_schema
                        .as_object()
                        .and_then(|fs| fs.get("required"))
                        .and_then(|r| r.as_bool())
                        .unwrap_or(true);
                    if required {
                        return Err(SomaError::Resource(format!(
                            "missing required field: {field}"
                        )));
                    }
                    continue;
                }
            };

            if let Some(ty) = expected_type {
                let ok = match ty {
                    "string" => value.is_string(),
                    "number" => value.is_number(),
                    "boolean" => value.is_boolean(),
                    "object" => value.is_object(),
                    "array" => value.is_array(),
                    "null" => value.is_null(),
                    _ => true, // unknown type — pass through
                };
                if !ok {
                    return Err(SomaError::Resource(format!(
                        "field '{field}' expected type '{ty}', got: {}",
                        value_type_name(value),
                    )));
                }
            }
        }
        Ok(())
    }

    /// Derive a resource id from the identity rules and the data.
    /// If `auto_generate` is true, a UUID v4 is returned.
    /// Otherwise the key fields are extracted from the data and concatenated.
    fn derive_resource_id(identity: &IdentityRules, data: &serde_json::Value) -> Result<String> {
        if identity.auto_generate {
            return Ok(Uuid::new_v4().to_string());
        }

        if identity.key_fields.is_empty() {
            return Err(SomaError::Resource(
                "identity rules have no key_fields and auto_generate is false".to_string(),
            ));
        }

        let data_obj = data.as_object().ok_or_else(|| {
            SomaError::Resource("resource data must be a JSON object".to_string())
        })?;

        let mut parts: Vec<String> = Vec::with_capacity(identity.key_fields.len());
        for key in &identity.key_fields {
            let val = data_obj.get(key).ok_or_else(|| {
                SomaError::Resource(format!("identity key field missing in data: {key}"))
            })?;
            // Use the raw JSON string representation for non-string types.
            let part = match val.as_str() {
                Some(s) => s.to_string(),
                None => val.to_string(),
            };
            parts.push(part);
        }
        Ok(parts.join(":"))
    }

    /// Apply a single PatchOperation to a mutable JSON value.
    fn apply_operation(data: &mut serde_json::Value, op: &PatchOperation) -> Result<()> {
        match op.op {
            PatchOp::Add | PatchOp::Replace => {
                let new_value = op.value.clone().ok_or_else(|| {
                    SomaError::Resource(format!(
                        "patch operation {:?} at '{}' requires a value",
                        op.op, op.path
                    ))
                })?;
                set_path(data, &op.path, new_value)
            }
            PatchOp::Remove => remove_path(data, &op.path),
            PatchOp::Move => {
                // `value` holds the source path as a string.
                let source_path = op
                    .value
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        SomaError::Resource(
                            "move operation requires 'value' to be the source path string"
                                .to_string(),
                        )
                    })?;
                let extracted = remove_path_return(data, source_path)?;
                set_path(data, &op.path, extracted)
            }
        }
    }
}

impl Default for DefaultResourceRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceRuntime for DefaultResourceRuntime {
    fn register_type(&self, spec: ResourceSpec) -> Result<()> {
        let mut specs = self.specs.write().unwrap();
        if specs.contains_key(&spec.type_name) {
            return Err(SomaError::Resource(format!(
                "resource type already registered: {}",
                spec.type_name
            )));
        }
        specs.insert(spec.type_name.clone(), spec);
        Ok(())
    }

    fn create(
        &self,
        namespace: &str,
        type_name: &str,
        data: serde_json::Value,
    ) -> Result<Resource> {
        let spec = self.get_spec(type_name)?;

        // Validate against schema.
        Self::validate_against_schema(&spec.schema, &data)?;

        // Derive resource id.
        let resource_id = Self::derive_resource_id(&spec.identity_rules, &data)?;

        // Check for duplicates.
        let mut instances = self.instances.write().unwrap();
        let key = (type_name.to_string(), resource_id.clone());
        if instances.contains_key(&key) {
            return Err(SomaError::Resource(format!(
                "resource already exists: {type_name}/{resource_id}"
            )));
        }

        let now = Utc::now();
        let resource = Resource {
            resource_ref: ResourceRef {
                resource_type: type_name.to_string(),
                resource_id,
                version: 1,
                origin: namespace.to_string(),
            },
            namespace: namespace.to_string(),
            data,
            created_at: now,
            updated_at: now,
        };

        instances.insert(key, resource.clone());
        Ok(resource)
    }

    fn get(&self, resource_type: &str, resource_id: &str) -> Result<Option<Resource>> {
        let instances = self.instances.read().unwrap();
        let key = (resource_type.to_string(), resource_id.to_string());
        Ok(instances.get(&key).cloned())
    }

    fn update(&self, resource_ref: &ResourceRef, patch: ResourcePatch) -> Result<Resource> {
        let spec = self.get_spec(&resource_ref.resource_type)?;

        // Enforce mutability.
        if spec.mutability == Mutability::Immutable {
            return Err(SomaError::Resource(format!(
                "resource type '{}' is immutable",
                resource_ref.resource_type
            )));
        }

        let mut instances = self.instances.write().unwrap();
        let key = (
            resource_ref.resource_type.clone(),
            resource_ref.resource_id.clone(),
        );

        let resource = instances.get_mut(&key).ok_or_else(|| {
            SomaError::ResourceNotFound {
                resource_type: resource_ref.resource_type.clone(),
                resource_id: resource_ref.resource_id.clone(),
            }
        })?;

        // Optimistic concurrency: the caller's version must match.
        if resource_ref.version != resource.resource_ref.version {
            return Err(SomaError::ResourceVersionConflict {
                expected: resource.resource_ref.version,
                found: resource_ref.version,
            });
        }

        // For append-only, only Add operations are permitted.
        if spec.mutability == Mutability::AppendOnly {
            for op in &patch.operations {
                if op.op != PatchOp::Add {
                    return Err(SomaError::Resource(format!(
                        "resource type '{}' is append-only; only 'add' operations are permitted",
                        resource_ref.resource_type
                    )));
                }
            }
        }

        // Apply patch operations.
        let mut new_data = resource.data.clone();
        for op in &patch.operations {
            Self::apply_operation(&mut new_data, op)?;
        }

        // Validate the result against schema.
        Self::validate_against_schema(&spec.schema, &new_data)?;

        // Commit: bump version, update timestamps and data.
        resource.data = new_data;
        resource.resource_ref.version += 1;
        resource.updated_at = Utc::now();

        Ok(resource.clone())
    }

    fn delete(&self, resource_ref: &ResourceRef) -> Result<()> {
        let spec = self.get_spec(&resource_ref.resource_type)?;

        if spec.mutability == Mutability::Immutable {
            return Err(SomaError::Resource(format!(
                "cannot delete immutable resource type '{}'",
                resource_ref.resource_type
            )));
        }

        let mut instances = self.instances.write().unwrap();
        let key = (
            resource_ref.resource_type.clone(),
            resource_ref.resource_id.clone(),
        );

        if instances.remove(&key).is_none() {
            return Err(SomaError::ResourceNotFound {
                resource_type: resource_ref.resource_type.clone(),
                resource_id: resource_ref.resource_id.clone(),
            });
        }

        Ok(())
    }

    fn list(&self, resource_type: &str, namespace: Option<&str>) -> Result<Vec<Resource>> {
        let instances = self.instances.read().unwrap();
        let results: Vec<Resource> = instances
            .values()
            .filter(|r| {
                r.resource_ref.resource_type == resource_type
                    && namespace.is_none_or(|ns| r.namespace == ns)
            })
            .cloned()
            .collect();
        Ok(results)
    }

    fn query(&self, resource_type: &str, filter: serde_json::Value) -> Result<Vec<Resource>> {
        let filter_obj = match filter.as_object() {
            Some(obj) => obj.clone(),
            None if filter.is_null() => {
                // Null filter means "match all".
                return self.list(resource_type, None);
            }
            None => {
                return Err(SomaError::Resource(
                    "query filter must be a JSON object or null".to_string(),
                ));
            }
        };

        let instances = self.instances.read().unwrap();
        let results: Vec<Resource> = instances
            .values()
            .filter(|r| {
                if r.resource_ref.resource_type != resource_type {
                    return false;
                }
                let data_obj = match r.data.as_object() {
                    Some(obj) => obj,
                    None => return false,
                };
                // Every filter key must match via equality.
                filter_obj
                    .iter()
                    .all(|(k, v)| data_obj.get(k).is_some_and(|dv| dv == v))
            })
            .cloned()
            .collect();
        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// JSON path helpers
// ---------------------------------------------------------------------------

/// Return a human-readable name for a JSON value's type.
fn value_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Parse a simplified JSON path (`/a/b/c` or `a.b.c`) into segments.
fn parse_path(path: &str) -> Vec<&str> {
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.contains('/') {
        trimmed.split('/').collect()
    } else {
        trimmed.split('.').collect()
    }
}

/// Set a value at a path in a JSON tree, creating intermediate objects as needed.
fn set_path(root: &mut serde_json::Value, path: &str, value: serde_json::Value) -> Result<()> {
    let segments = parse_path(path);
    if segments.is_empty() {
        return Err(SomaError::Resource("empty path in patch operation".to_string()));
    }

    let mut current = root;
    for (i, seg) in segments.iter().enumerate() {
        if i == segments.len() - 1 {
            // Last segment: insert/replace.
            match current {
                serde_json::Value::Object(map) => {
                    map.insert(seg.to_string(), value);
                    return Ok(());
                }
                _ => {
                    return Err(SomaError::Resource(format!(
                        "cannot index into non-object at path segment '{seg}'"
                    )));
                }
            }
        } else {
            // Intermediate segment: descend, create object if missing.
            if !current.is_object() {
                return Err(SomaError::Resource(format!(
                    "cannot traverse non-object at path segment '{seg}'"
                )));
            }
            let map = current.as_object_mut().unwrap();
            current = map
                .entry(seg.to_string())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        }
    }
    Ok(())
}

/// Remove a value at a path, returning an error if the path doesn't exist.
fn remove_path(root: &mut serde_json::Value, path: &str) -> Result<()> {
    remove_path_return(root, path)?;
    Ok(())
}

/// Remove and return a value at a path.
fn remove_path_return(root: &mut serde_json::Value, path: &str) -> Result<serde_json::Value> {
    let segments = parse_path(path);
    if segments.is_empty() {
        return Err(SomaError::Resource("empty path in remove operation".to_string()));
    }

    let mut current = root;
    for (i, seg) in segments.iter().enumerate() {
        if i == segments.len() - 1 {
            match current {
                serde_json::Value::Object(map) => {
                    return map.remove(*seg).ok_or_else(|| {
                        SomaError::Resource(format!("path not found for remove: {path}"))
                    });
                }
                _ => {
                    return Err(SomaError::Resource(format!(
                        "cannot remove from non-object at path segment '{seg}'"
                    )));
                }
            }
        } else {
            match current {
                serde_json::Value::Object(map) => {
                    current = map.get_mut(*seg).ok_or_else(|| {
                        SomaError::Resource(format!(
                            "intermediate path segment '{seg}' not found"
                        ))
                    })?;
                }
                _ => {
                    return Err(SomaError::Resource(format!(
                        "cannot traverse non-object at path segment '{seg}'"
                    )));
                }
            }
        }
    }
    Err(SomaError::Resource("unreachable path traversal".to_string()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- helpers ---

    fn make_spec(type_name: &str) -> ResourceSpec {
        ResourceSpec {
            resource_id: Uuid::new_v4().to_string(),
            namespace: "test".to_string(),
            type_name: type_name.to_string(),
            schema: serde_json::json!({
                "name": { "type": "string" },
                "value": { "type": "number" }
            }),
            identity_rules: IdentityRules {
                key_fields: vec![],
                auto_generate: true,
            },
            versioning_rules: VersioningRules {
                strategy: VersioningStrategy::Monotonic,
                conflict_policy: ConflictPolicy::Reject,
            },
            mutability: Mutability::Mutable,
            relationships: vec![],
            exposure: ResourceExposure {
                local: true,
                remote: false,
                sync_mode: SyncMode::Snapshot,
            },
        }
    }

    fn make_spec_with_keys(type_name: &str, keys: Vec<&str>) -> ResourceSpec {
        let mut spec = make_spec(type_name);
        spec.identity_rules = IdentityRules {
            key_fields: keys.into_iter().map(String::from).collect(),
            auto_generate: false,
        };
        spec
    }

    fn make_immutable_spec(type_name: &str) -> ResourceSpec {
        let mut spec = make_spec(type_name);
        spec.mutability = Mutability::Immutable;
        spec
    }

    fn make_append_only_spec(type_name: &str) -> ResourceSpec {
        let mut spec = make_spec(type_name);
        spec.mutability = Mutability::AppendOnly;
        spec
    }

    fn sample_data() -> serde_json::Value {
        serde_json::json!({ "name": "alpha", "value": 42 })
    }

    // --- register_type ---

    #[test]
    fn register_type_succeeds() {
        let rt = DefaultResourceRuntime::new();
        assert!(rt.register_type(make_spec("widget")).is_ok());
    }

    #[test]
    fn register_duplicate_type_fails() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        assert!(rt.register_type(make_spec("widget")).is_err());
    }

    // --- create ---

    #[test]
    fn create_resource_succeeds() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        let resource = rt.create("default", "widget", sample_data()).unwrap();
        assert_eq!(resource.resource_ref.resource_type, "widget");
        assert_eq!(resource.resource_ref.version, 1);
        assert_eq!(resource.namespace, "default");
        assert_eq!(resource.data, sample_data());
    }

    #[test]
    fn create_with_unregistered_type_fails() {
        let rt = DefaultResourceRuntime::new();
        assert!(rt.create("default", "unknown", sample_data()).is_err());
    }

    #[test]
    fn create_with_invalid_schema_fails() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        // "name" should be string, not number.
        let bad_data = serde_json::json!({ "name": 123, "value": 42 });
        assert!(rt.create("default", "widget", bad_data).is_err());
    }

    #[test]
    fn create_with_missing_required_field_fails() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        let partial_data = serde_json::json!({ "name": "alpha" });
        assert!(rt.create("default", "widget", partial_data).is_err());
    }

    #[test]
    fn create_with_key_fields_derives_id() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec_with_keys("widget", vec!["name"]))
            .unwrap();
        let resource = rt.create("default", "widget", sample_data()).unwrap();
        assert_eq!(resource.resource_ref.resource_id, "alpha");
    }

    #[test]
    fn create_duplicate_resource_fails() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec_with_keys("widget", vec!["name"]))
            .unwrap();
        rt.create("default", "widget", sample_data()).unwrap();
        assert!(rt.create("default", "widget", sample_data()).is_err());
    }

    // --- get ---

    #[test]
    fn get_existing_resource() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        let created = rt.create("default", "widget", sample_data()).unwrap();
        let found = rt
            .get("widget", &created.resource_ref.resource_id)
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().resource_ref.version, 1);
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let rt = DefaultResourceRuntime::new();
        let found = rt.get("widget", "does-not-exist").unwrap();
        assert!(found.is_none());
    }

    // --- update ---

    #[test]
    fn update_bumps_version() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        let created = rt.create("default", "widget", sample_data()).unwrap();
        let patch = ResourcePatch {
            resource_ref: created.resource_ref.clone(),
            operations: vec![PatchOperation {
                op: PatchOp::Replace,
                path: "/value".to_string(),
                value: Some(serde_json::json!(99)),
            }],
        };
        let updated = rt.update(&created.resource_ref, patch).unwrap();
        assert_eq!(updated.resource_ref.version, 2);
        assert_eq!(updated.data["value"], 99);
    }

    #[test]
    fn update_nonexistent_fails() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        let fake_ref = ResourceRef {
            resource_type: "widget".to_string(),
            resource_id: "nonexistent".to_string(),
            version: 1,
            origin: "default".to_string(),
        };
        let patch = ResourcePatch {
            resource_ref: fake_ref.clone(),
            operations: vec![],
        };
        assert!(rt.update(&fake_ref, patch).is_err());
    }

    #[test]
    fn update_version_conflict_rejected() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        let created = rt.create("default", "widget", sample_data()).unwrap();
        let mut stale_ref = created.resource_ref.clone();
        stale_ref.version = 999;
        let patch = ResourcePatch {
            resource_ref: stale_ref.clone(),
            operations: vec![PatchOperation {
                op: PatchOp::Replace,
                path: "/value".to_string(),
                value: Some(serde_json::json!(1)),
            }],
        };
        assert!(rt.update(&stale_ref, patch).is_err());
    }

    #[test]
    fn update_immutable_resource_fails() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_immutable_spec("constant")).unwrap();
        let created = rt.create("default", "constant", sample_data()).unwrap();
        let patch = ResourcePatch {
            resource_ref: created.resource_ref.clone(),
            operations: vec![PatchOperation {
                op: PatchOp::Replace,
                path: "/value".to_string(),
                value: Some(serde_json::json!(1)),
            }],
        };
        assert!(rt.update(&created.resource_ref, patch).is_err());
    }

    #[test]
    fn update_append_only_allows_add_only() {
        let rt = DefaultResourceRuntime::new();
        let mut spec = make_append_only_spec("log");
        // Relax schema for this test.
        spec.schema = serde_json::json!({});
        rt.register_type(spec).unwrap();
        let created = rt
            .create("default", "log", serde_json::json!({ "entries": [] }))
            .unwrap();

        // Add should succeed.
        let add_patch = ResourcePatch {
            resource_ref: created.resource_ref.clone(),
            operations: vec![PatchOperation {
                op: PatchOp::Add,
                path: "/new_field".to_string(),
                value: Some(serde_json::json!("hello")),
            }],
        };
        assert!(rt.update(&created.resource_ref, add_patch).is_ok());

        // Fetch updated version for correct ref.
        let current = rt
            .get("log", &created.resource_ref.resource_id)
            .unwrap()
            .unwrap();

        // Replace should fail.
        let replace_patch = ResourcePatch {
            resource_ref: current.resource_ref.clone(),
            operations: vec![PatchOperation {
                op: PatchOp::Replace,
                path: "/new_field".to_string(),
                value: Some(serde_json::json!("changed")),
            }],
        };
        assert!(rt.update(&current.resource_ref, replace_patch).is_err());
    }

    #[test]
    fn update_with_add_operation() {
        let rt = DefaultResourceRuntime::new();
        let mut spec = make_spec("widget");
        spec.schema = serde_json::json!({}); // relaxed
        rt.register_type(spec).unwrap();
        let created = rt.create("default", "widget", sample_data()).unwrap();
        let patch = ResourcePatch {
            resource_ref: created.resource_ref.clone(),
            operations: vec![PatchOperation {
                op: PatchOp::Add,
                path: "/extra".to_string(),
                value: Some(serde_json::json!("new")),
            }],
        };
        let updated = rt.update(&created.resource_ref, patch).unwrap();
        assert_eq!(updated.data["extra"], "new");
        assert_eq!(updated.data["name"], "alpha"); // preserved
    }

    #[test]
    fn update_with_remove_operation() {
        let rt = DefaultResourceRuntime::new();
        let mut spec = make_spec("widget");
        spec.schema = serde_json::json!({}); // relaxed
        rt.register_type(spec).unwrap();
        let data = serde_json::json!({ "name": "alpha", "value": 42, "temp": true });
        let created = rt.create("default", "widget", data).unwrap();
        let patch = ResourcePatch {
            resource_ref: created.resource_ref.clone(),
            operations: vec![PatchOperation {
                op: PatchOp::Remove,
                path: "/temp".to_string(),
                value: None,
            }],
        };
        let updated = rt.update(&created.resource_ref, patch).unwrap();
        assert!(updated.data.get("temp").is_none());
    }

    // --- delete ---

    #[test]
    fn delete_resource_succeeds() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        let created = rt.create("default", "widget", sample_data()).unwrap();
        assert!(rt.delete(&created.resource_ref).is_ok());
        assert!(rt
            .get("widget", &created.resource_ref.resource_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn delete_nonexistent_fails() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        let fake_ref = ResourceRef {
            resource_type: "widget".to_string(),
            resource_id: "nonexistent".to_string(),
            version: 1,
            origin: "default".to_string(),
        };
        assert!(rt.delete(&fake_ref).is_err());
    }

    #[test]
    fn delete_immutable_fails() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_immutable_spec("constant")).unwrap();
        let created = rt.create("default", "constant", sample_data()).unwrap();
        assert!(rt.delete(&created.resource_ref).is_err());
    }

    // --- list ---

    #[test]
    fn list_resources_by_type() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        rt.create("ns1", "widget", serde_json::json!({ "name": "a", "value": 1 }))
            .unwrap();
        rt.create("ns2", "widget", serde_json::json!({ "name": "b", "value": 2 }))
            .unwrap();
        let all = rt.list("widget", None).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn list_resources_filtered_by_namespace() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        rt.create("ns1", "widget", serde_json::json!({ "name": "a", "value": 1 }))
            .unwrap();
        rt.create("ns2", "widget", serde_json::json!({ "name": "b", "value": 2 }))
            .unwrap();
        let ns1_only = rt.list("widget", Some("ns1")).unwrap();
        assert_eq!(ns1_only.len(), 1);
        assert_eq!(ns1_only[0].namespace, "ns1");
    }

    #[test]
    fn list_empty_type_returns_empty() {
        let rt = DefaultResourceRuntime::new();
        let result = rt.list("nonexistent", None).unwrap();
        assert!(result.is_empty());
    }

    // --- query ---

    #[test]
    fn query_with_filter() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        rt.create("default", "widget", serde_json::json!({ "name": "a", "value": 1 }))
            .unwrap();
        rt.create("default", "widget", serde_json::json!({ "name": "b", "value": 2 }))
            .unwrap();
        rt.create("default", "widget", serde_json::json!({ "name": "a", "value": 3 }))
            .unwrap();
        let results = rt
            .query("widget", serde_json::json!({ "name": "a" }))
            .unwrap();
        assert_eq!(results.len(), 2);
        for r in &results {
            assert_eq!(r.data["name"], "a");
        }
    }

    #[test]
    fn query_null_filter_returns_all() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        rt.create("default", "widget", serde_json::json!({ "name": "a", "value": 1 }))
            .unwrap();
        let results = rt.query("widget", serde_json::Value::Null).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn query_no_matches_returns_empty() {
        let rt = DefaultResourceRuntime::new();
        rt.register_type(make_spec("widget")).unwrap();
        rt.create("default", "widget", sample_data()).unwrap();
        let results = rt
            .query("widget", serde_json::json!({ "name": "nonexistent" }))
            .unwrap();
        assert!(results.is_empty());
    }

    // --- schema validation edge cases ---

    #[test]
    fn create_with_null_schema_allows_anything() {
        let rt = DefaultResourceRuntime::new();
        let mut spec = make_spec("flexible");
        spec.schema = serde_json::Value::Null;
        rt.register_type(spec).unwrap();
        let resource = rt
            .create("default", "flexible", serde_json::json!({ "anything": "goes" }))
            .unwrap();
        assert_eq!(resource.data["anything"], "goes");
    }

    #[test]
    fn create_with_empty_schema_allows_anything() {
        let rt = DefaultResourceRuntime::new();
        let mut spec = make_spec("flexible");
        spec.schema = serde_json::json!({});
        rt.register_type(spec).unwrap();
        let resource = rt
            .create("default", "flexible", serde_json::json!(42))
            .unwrap();
        assert_eq!(resource.data, serde_json::json!(42));
    }

    // --- nested patch paths ---

    #[test]
    fn update_nested_path() {
        let rt = DefaultResourceRuntime::new();
        let mut spec = make_spec("widget");
        spec.schema = serde_json::json!({}); // relaxed
        rt.register_type(spec).unwrap();
        let data = serde_json::json!({ "meta": { "color": "red" } });
        let created = rt.create("default", "widget", data).unwrap();
        let patch = ResourcePatch {
            resource_ref: created.resource_ref.clone(),
            operations: vec![PatchOperation {
                op: PatchOp::Replace,
                path: "/meta/color".to_string(),
                value: Some(serde_json::json!("blue")),
            }],
        };
        let updated = rt.update(&created.resource_ref, patch).unwrap();
        assert_eq!(updated.data["meta"]["color"], "blue");
    }

    // --- full lifecycle ---

    #[test]
    fn full_lifecycle_create_update_get_delete() {
        let rt = DefaultResourceRuntime::new();
        let mut spec = make_spec("widget");
        spec.schema = serde_json::json!({}); // relaxed
        rt.register_type(spec).unwrap();

        // Create
        let created = rt
            .create("prod", "widget", serde_json::json!({ "status": "active" }))
            .unwrap();
        assert_eq!(created.resource_ref.version, 1);

        // Update
        let patch = ResourcePatch {
            resource_ref: created.resource_ref.clone(),
            operations: vec![PatchOperation {
                op: PatchOp::Replace,
                path: "/status".to_string(),
                value: Some(serde_json::json!("inactive")),
            }],
        };
        let updated = rt.update(&created.resource_ref, patch).unwrap();
        assert_eq!(updated.resource_ref.version, 2);
        assert_eq!(updated.data["status"], "inactive");

        // Get
        let fetched = rt
            .get("widget", &created.resource_ref.resource_id)
            .unwrap()
            .unwrap();
        assert_eq!(fetched.resource_ref.version, 2);

        // Delete
        assert!(rt.delete(&updated.resource_ref).is_ok());
        assert!(rt
            .get("widget", &created.resource_ref.resource_id)
            .unwrap()
            .is_none());
    }
}
