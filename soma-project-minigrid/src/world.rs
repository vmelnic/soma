use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldSpec {
    pub name: String,
    #[serde(default = "default_size")]
    pub size: usize,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub env: Option<String>,
    #[serde(default)]
    pub cells: Option<Vec<Vec<String>>>,
    #[serde(default)]
    pub agent: Option<[usize; 2]>,
    #[serde(default = "default_view_radius")]
    pub view_radius: usize,
}

fn default_size() -> usize { 8 }
fn default_view_radius() -> usize { 3 }

impl WorldSpec {
    pub fn is_custom(&self) -> bool {
        self.cells.is_some()
    }

    pub fn env_type(&self) -> &str {
        self.env.as_deref().unwrap_or("doorkey")
    }

    pub fn goal_fingerprint(&self) -> String {
        "solve_gridworld".to_string()
    }

    pub fn to_port_input(&self) -> serde_json::Value {
        let mut v = if let Some(cells) = &self.cells {
            let agent = self.agent.unwrap_or([1, 1]);
            serde_json::json!({
                "cells": cells,
                "agent": agent,
            })
        } else {
            let mut v = serde_json::json!({
                "size": self.size,
                "seed": self.seed,
            });
            if let Some(env) = &self.env {
                v["env"] = serde_json::Value::String(env.clone());
            }
            v
        };
        v["view_radius"] = serde_json::json!(self.view_radius);
        v
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        serde_json::from_str(&data)
            .map_err(|e| format!("parse {}: {e}", path.display()))
    }

    pub fn load_dir(dir: &Path) -> Result<Vec<Self>, String> {
        let mut worlds = Vec::new();
        let entries = std::fs::read_dir(dir)
            .map_err(|e| format!("read dir {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                worlds.push(Self::load(&path)?);
            }
        }
        worlds.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(worlds)
    }
}
