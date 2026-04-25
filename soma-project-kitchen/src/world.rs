use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub items: Option<Vec<ScenarioItem>>,
    #[serde(default)]
    pub cabinet_open: Option<bool>,
    #[serde(default)]
    pub drawer_open: Option<bool>,
    #[serde(default)]
    pub window_open: Option<bool>,
    #[serde(default)]
    pub required_tasks: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioItem {
    pub id: String,
    pub kind: String,
    pub pos: [f64; 2],
}

impl ScenarioSpec {
    pub fn goal_fingerprint(&self) -> String {
        "organize_kitchen".to_string()
    }

    pub fn to_port_input(&self) -> serde_json::Value {
        let mut v = serde_json::json!({});
        if let Some(preset) = &self.preset {
            v["preset"] = serde_json::Value::String(preset.clone());
        }
        if self.seed != 0 {
            v["seed"] = serde_json::json!(self.seed);
        }
        if let Some(items) = &self.items {
            v["items"] = serde_json::json!(items);
        }
        if let Some(co) = self.cabinet_open {
            v["cabinet_open"] = serde_json::json!(co);
        }
        if let Some(dw) = self.drawer_open {
            v["drawer_open"] = serde_json::json!(dw);
        }
        if let Some(wo) = self.window_open {
            v["window_open"] = serde_json::json!(wo);
        }
        if let Some(tasks) = &self.required_tasks {
            v["required_tasks"] = serde_json::json!(tasks);
        }
        v
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        serde_json::from_str(&data)
            .map_err(|e| format!("parse {}: {e}", path.display()))
    }

    pub fn load_dir(dir: &Path) -> Result<Vec<Self>, String> {
        let mut scenarios = Vec::new();
        let entries = std::fs::read_dir(dir)
            .map_err(|e| format!("read dir {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                scenarios.push(Self::load(&path)?);
            }
        }
        scenarios.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(scenarios)
    }
}
