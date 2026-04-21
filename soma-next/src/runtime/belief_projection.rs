use crate::types::belief::BeliefState;

pub struct BeliefProjector {
    skill_selection_expr: jmespath::Expression<'static>,
}

const SKILL_SELECTION_JMESPATH: &str = concat!(
    "{",
    "facts: facts[?confidence > `0.3`]",
    " | sort_by(@, &confidence)",
    " | reverse(@)",
    " | [:5]",
    " | [].{s: subject, p: predicate, v: value, c: confidence},",
    "bindings: active_bindings",
    " | sort_by(@, &confidence)",
    " | reverse(@)",
    " | [:10]",
    " | [].{n: name, v: value, s: source, c: confidence}",
    "}",
);

impl BeliefProjector {
    pub fn new() -> Self {
        let skill_selection_expr = jmespath::compile(SKILL_SELECTION_JMESPATH)
            .expect("built-in JMESPath expression must compile");
        Self { skill_selection_expr }
    }

    pub fn project_for_brain(&self, belief: &BeliefState) -> serde_json::Value {
        let belief_json = serde_json::to_value(belief).unwrap_or_default();
        let result = self.skill_selection_expr.search(&belief_json).unwrap_or_else(|_| {
            std::sync::Arc::new(jmespath::Variable::Null)
        });
        serde_json::to_value(&*result).unwrap_or_default()
    }

    pub fn project_to_toon(&self, belief: &BeliefState) -> String {
        let projected = self.project_for_brain(belief);
        toon_format::encode_default(&projected).unwrap_or_else(|_| {
            serde_json::to_string(&projected).unwrap_or_default()
        })
    }
}

impl Default for BeliefProjector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::belief::{Binding, Fact};
    use crate::types::common::FactProvenance;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_belief(facts: Vec<Fact>, bindings: Vec<Binding>) -> BeliefState {
        BeliefState {
            belief_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            resources: vec![],
            facts,
            uncertainties: vec![],
            provenance: vec![],
            active_bindings: bindings,
            world_hash: String::new(),
            updated_at: Utc::now(),
        }
    }

    fn make_fact(subject: &str, confidence: f64) -> Fact {
        Fact {
            fact_id: format!("fact-{subject}"),
            subject: subject.to_string(),
            predicate: "is".to_string(),
            value: serde_json::json!(true),
            confidence,
            provenance: FactProvenance::Observed,
            timestamp: Utc::now(),
            ttl_ms: None,
        }
    }

    fn make_binding(name: &str, confidence: f64) -> Binding {
        Binding {
            name: name.to_string(),
            value: serde_json::json!(name),
            source: "goal".to_string(),
            confidence,
        }
    }

    #[test]
    fn test_projection_filters_low_confidence() {
        let projector = BeliefProjector::new();
        let belief = make_belief(
            vec![make_fact("high", 0.9), make_fact("low", 0.1)],
            vec![],
        );
        let result = projector.project_for_brain(&belief);
        let facts = result["facts"].as_array().unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0]["s"], "high");
    }

    #[test]
    fn test_projection_limits_counts() {
        let projector = BeliefProjector::new();
        let facts: Vec<Fact> = (0..10)
            .map(|i| make_fact(&format!("f{i}"), 0.5 + i as f64 * 0.01))
            .collect();
        let bindings: Vec<Binding> = (0..20)
            .map(|i| make_binding(&format!("b{i}"), 0.5 + i as f64 * 0.01))
            .collect();
        let belief = make_belief(facts, bindings);
        let result = projector.project_for_brain(&belief);
        assert_eq!(result["facts"].as_array().unwrap().len(), 5);
        assert_eq!(result["bindings"].as_array().unwrap().len(), 10);
    }

    #[test]
    fn test_projection_sorted_by_confidence() {
        let projector = BeliefProjector::new();
        let belief = make_belief(
            vec![make_fact("low", 0.4), make_fact("high", 0.9), make_fact("mid", 0.6)],
            vec![],
        );
        let result = projector.project_for_brain(&belief);
        let facts = result["facts"].as_array().unwrap();
        let confidences: Vec<f64> = facts.iter().map(|f| f["c"].as_f64().unwrap()).collect();
        assert!(confidences.windows(2).all(|w| w[0] >= w[1]));
    }

    #[test]
    fn test_toon_output_parses_back() {
        let projector = BeliefProjector::new();
        let belief = make_belief(
            vec![make_fact("temp", 0.95)],
            vec![make_binding("table", 1.0)],
        );
        let toon_str = projector.project_to_toon(&belief);
        assert!(!toon_str.is_empty());
        let decoded: serde_json::Value = toon_format::decode_default(&toon_str)
            .expect("TOON output must decode back to valid data");
        assert!(decoded["facts"].is_array());
        assert!(decoded["bindings"].is_array());
    }

    #[test]
    fn test_empty_belief_produces_empty_projection() {
        let projector = BeliefProjector::new();
        let belief = make_belief(vec![], vec![]);
        let result = projector.project_for_brain(&belief);
        assert_eq!(result["facts"].as_array().unwrap().len(), 0);
        assert_eq!(result["bindings"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_projection_uses_short_keys() {
        let projector = BeliefProjector::new();
        let belief = make_belief(
            vec![make_fact("x", 0.9)],
            vec![make_binding("y", 0.8)],
        );
        let result = projector.project_for_brain(&belief);
        let fact = &result["facts"][0];
        assert!(fact.get("s").is_some());
        assert!(fact.get("p").is_some());
        assert!(fact.get("v").is_some());
        assert!(fact.get("c").is_some());
        assert!(fact.get("subject").is_none());

        let binding = &result["bindings"][0];
        assert!(binding.get("n").is_some());
        assert!(binding.get("v").is_some());
        assert!(binding.get("s").is_some());
        assert!(binding.get("c").is_some());
        assert!(binding.get("name").is_none());
    }

    #[test]
    fn test_realistic_mcp_session_projection() {
        let projector = BeliefProjector::new();

        let mut facts: Vec<Fact> = (0..50)
            .map(|i| Fact {
                fact_id: format!("obs:{}", Uuid::new_v4()),
                subject: "soma.ports.google_mail.list_labels".to_string(),
                predicate: "failed".to_string(),
                value: serde_json::json!({"error": "port not connected"}),
                confidence: 0.0,
                provenance: FactProvenance::Observed,
                timestamp: Utc::now(),
                ttl_ms: None,
            })
            .collect();

        facts.extend(vec![
            make_fact("cpu_load", 0.95),
            make_fact("memory_available", 0.88),
            make_fact("disk_usage", 0.72),
            make_fact("network_latency", 0.45),
            make_fact("process_count", 0.61),
            make_fact("io_throughput", 0.33),
            make_fact("swap_usage", 0.15),
        ]);

        let bindings: Vec<Binding> = vec![
            make_binding("target_host", 0.99),
            make_binding("check_interval", 0.85),
            make_binding("alert_threshold", 0.70),
            make_binding("output_format", 0.55),
        ];

        let belief = make_belief(facts, bindings);

        let full_json = serde_json::to_string(&belief).unwrap();
        let projected = projector.project_for_brain(&belief);
        let projected_json = serde_json::to_string(&projected).unwrap();
        let toon_str = projector.project_to_toon(&belief);

        let proj_facts = projected["facts"].as_array().unwrap();
        assert_eq!(proj_facts.len(), 5);
        let conf_values: Vec<f64> = proj_facts.iter().map(|f| f["c"].as_f64().unwrap()).collect();
        assert!(conf_values[0] >= conf_values[1]);
        assert!(conf_values.iter().all(|&c| c > 0.3));

        let proj_bindings = projected["bindings"].as_array().unwrap();
        assert_eq!(proj_bindings.len(), 4);

        let full_size = full_json.len();
        let proj_size = projected_json.len();
        let toon_size = toon_str.len();

        eprintln!("\n=== REALISTIC MCP SESSION PROJECTION ===");
        eprintln!("Input: 57 facts (50 failed @ 0.0, 7 real @ 0.15-0.95), 4 bindings");
        eprintln!("Full belief JSON:  {} bytes", full_size);
        eprintln!("Projected JSON:    {} bytes", proj_size);
        eprintln!("TOON encoded:      {} bytes", toon_size);
        eprintln!("JMESPath reduction: {:.1}%", (1.0 - proj_size as f64 / full_size as f64) * 100.0);
        eprintln!("TOON reduction:     {:.1}%", (1.0 - toon_size as f64 / full_size as f64) * 100.0);
        eprintln!("\nProjected facts (top 5 by confidence > 0.3):");
        for f in proj_facts {
            eprintln!("  {} / {} = {} (conf: {})", f["s"], f["p"], f["v"], f["c"]);
        }
        eprintln!("\nProjected bindings (top 4):");
        for b in proj_bindings {
            eprintln!("  {} = {} from {} (conf: {})", b["n"], b["v"], b["s"], b["c"]);
        }
        eprintln!("\nTOON output:\n{}", toon_str);

        assert!(toon_size < full_size / 5);

        let decoded: serde_json::Value = toon_format::decode_default(&toon_str)
            .expect("TOON must round-trip");
        assert_eq!(decoded["facts"].as_array().unwrap().len(), 5);
        assert_eq!(decoded["bindings"].as_array().unwrap().len(), 4);
    }
}
