// soma-esp32-port-storage — persistent key-value storage primitives.
//
// Primitives:
//   storage.get   { key }            -> { key, value: str?, found: bool }
//   storage.set   { key, value }     -> { key, stored: bool }
//   storage.delete { key }           -> { key, deleted: bool }
//   storage.list  { prefix? }        -> { keys: [str] }
//   storage.clear { }                -> { ok: bool }    (clears the entire namespace)
//
// Type-erased: depends on KvStore trait the firmware implements with
// esp-storage or any other backing store. The port doesn't know how
// the data is persisted.

#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde_json::{json, Value};
use soma_esp32_leaf::{CapabilityDescriptor, Effect, SomaEspPort};

#[derive(Debug, Clone)]
pub enum StorageError {
    NotFound,
    OutOfSpace,
    BackendError(String),
}

/// Trait the firmware implements with the real storage backend.
pub trait KvStore {
    fn get(&self, key: &str) -> Result<Option<String>, StorageError>;
    fn set(&mut self, key: &str, value: &str) -> Result<(), StorageError>;
    fn delete(&mut self, key: &str) -> Result<bool, StorageError>;
    fn list(&self, prefix: Option<&str>) -> Result<Vec<String>, StorageError>;
    fn clear(&mut self) -> Result<(), StorageError>;
}

pub struct StoragePort {
    store: Box<dyn KvStore>,
}

impl StoragePort {
    pub fn new(store: Box<dyn KvStore>) -> Self {
        Self { store }
    }
}

impl SomaEspPort for StoragePort {
    fn port_id(&self) -> &'static str {
        "storage"
    }

    fn primitives(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor {
                skill_id: "storage.get".to_string(),
                description: "Read a string value by key from persistent KV storage"
                    .to_string(),
                input_schema: r#"{"key":"str"}"#.to_string(),
                output_schema: r#"{"key":"str","value":"str?","found":"bool"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "storage.set".to_string(),
                description: "Store a string value at the given key (overwrites if exists)"
                    .to_string(),
                input_schema: r#"{"key":"str","value":"str"}"#.to_string(),
                output_schema: r#"{"key":"str","stored":"bool"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "storage.delete".to_string(),
                description: "Remove a key from persistent storage".to_string(),
                input_schema: r#"{"key":"str"}"#.to_string(),
                output_schema: r#"{"key":"str","deleted":"bool"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "storage.list".to_string(),
                description: "List keys in persistent storage, optionally filtered by prefix"
                    .to_string(),
                input_schema: r#"{"prefix":"str?"}"#.to_string(),
                output_schema: r#"{"keys":"[str]"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "storage.clear".to_string(),
                description: "Erase the entire SOMA storage namespace".to_string(),
                input_schema: "{}".to_string(),
                output_schema: r#"{"ok":"bool"}"#.to_string(),
                effect: Effect::StateMutation,
            },
        ]
    }

    fn invoke(&mut self, skill_id: &str, input: &Value) -> Result<Value, String> {
        match skill_id {
            "storage.get" => {
                let key = input["key"]
                    .as_str()
                    .ok_or_else(|| "missing 'key'".to_string())?;
                let value = self
                    .store
                    .get(key)
                    .map_err(|e| alloc::format!("{:?}", e))?;
                Ok(json!({
                    "key": key,
                    "value": value,
                    "found": value.is_some(),
                }))
            }
            "storage.set" => {
                let key = input["key"]
                    .as_str()
                    .ok_or_else(|| "missing 'key'".to_string())?;
                let value = input["value"]
                    .as_str()
                    .ok_or_else(|| "missing 'value'".to_string())?;
                self.store
                    .set(key, value)
                    .map_err(|e| alloc::format!("{:?}", e))?;
                Ok(json!({ "key": key, "stored": true }))
            }
            "storage.delete" => {
                let key = input["key"]
                    .as_str()
                    .ok_or_else(|| "missing 'key'".to_string())?;
                let deleted = self
                    .store
                    .delete(key)
                    .map_err(|e| alloc::format!("{:?}", e))?;
                Ok(json!({ "key": key, "deleted": deleted }))
            }
            "storage.list" => {
                let prefix = input["prefix"].as_str();
                let keys = self
                    .store
                    .list(prefix)
                    .map_err(|e| alloc::format!("{:?}", e))?;
                Ok(json!({ "keys": keys }))
            }
            "storage.clear" => {
                self.store
                    .clear()
                    .map_err(|e| alloc::format!("{:?}", e))?;
                Ok(json!({ "ok": true }))
            }
            _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
        }
    }
}
