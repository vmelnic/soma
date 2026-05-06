use soma_port_sdk::{Port, PortCallRecord};
use soma_port_glm::GlmPort;
use soma_port_kimi::KimiPort;
use soma_port_mercury::MercuryPort;

fn main() {
    println!("==================================================");
    println!("SOMA Multi-Brain Proof — Three LLMs as Ports");
    println!("==================================================\n");

    load_dotenv();

    let phases: &[(&str, fn() -> Result<String, String>)] = &[
        ("Phase 1: all ports load", phase1_specs),
        ("Phase 2: Mercury generate", phase2_mercury),
        ("Phase 3: Kimi generate", phase3_kimi),
        ("Phase 4: GLM generate", phase4_glm),
        ("Phase 5: three-brain race", phase5_race),
        ("Phase 6: observables comparison", phase6_observables),
    ];

    let mut passed = 0;
    let mut failed = 0;
    for (name, f) in phases {
        println!("--- {name} ---");
        match f() {
            Ok(detail) => {
                println!("  PASS: {detail}\n");
                passed += 1;
            }
            Err(e) => {
                println!("  FAIL: {e}\n");
                failed += 1;
            }
        }
    }

    println!("==================================================");
    println!("{passed} passed, {failed} failed");
    println!("==================================================");

    if failed > 0 {
        std::process::exit(1);
    }
}

fn load_dotenv() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    if let Ok(contents) = std::fs::read_to_string(&path) {
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                unsafe { std::env::set_var(key.trim(), value.trim()) };
            }
        }
    }
}

fn invoke_generate(port: &dyn Port, prompt: &str, max_tokens: u64) -> Result<PortCallRecord, String> {
    let input = serde_json::json!({
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": max_tokens,
    });
    let record = port.invoke("generate", input).map_err(|e| e.to_string())?;
    if !record.success {
        return Err(format!(
            "{}: {}",
            port.spec().port_id,
            record.raw_result["error"].as_str().unwrap_or("unknown error")
        ));
    }
    Ok(record)
}

fn fmt_record(record: &PortCallRecord) -> String {
    let content = record.structured_result["content"]
        .as_str()
        .unwrap_or("");
    let truncated = if content.len() > 60 {
        format!("{}...", &content[..60])
    } else {
        content.to_string()
    };
    let tokens = record.structured_result["usage"]["total_tokens"]
        .as_u64()
        .unwrap_or(0);
    let model = record.structured_result["model"]
        .as_str()
        .unwrap_or("?");
    format!(
        "{}ms | {}tok | {} | {:?}",
        record.latency_ms, tokens, model, truncated
    )
}

fn phase1_specs() -> Result<String, String> {
    let brains: Vec<Box<dyn Port>> = vec![
        Box::new(MercuryPort::new()),
        Box::new(KimiPort::new()),
        Box::new(GlmPort::new()),
    ];

    let mut ids = Vec::new();
    for brain in &brains {
        let spec = brain.spec();
        if spec.capabilities.is_empty() {
            return Err(format!("{}: no capabilities", spec.port_id));
        }
        if !spec.sandbox_requirements.network_access {
            return Err(format!("{}: network_access=false", spec.port_id));
        }
        if !spec.auth_requirements.required {
            return Err(format!("{}: auth not required", spec.port_id));
        }
        ids.push(spec.port_id.clone());
    }

    Ok(format!("loaded: {ids:?}"))
}

fn phase2_mercury() -> Result<String, String> {
    let port = MercuryPort::new();
    let record = invoke_generate(&port, "What is 2+2? One word.", 64)?;
    Ok(fmt_record(&record))
}

fn phase3_kimi() -> Result<String, String> {
    let port = KimiPort::new();
    let record = invoke_generate(&port, "What is 2+2? One word.", 64)?;
    Ok(fmt_record(&record))
}

fn phase4_glm() -> Result<String, String> {
    let port = GlmPort::new();
    let record = invoke_generate(&port, "What is 2+2? One word.", 64)?;
    Ok(fmt_record(&record))
}

fn phase5_race() -> Result<String, String> {
    let brains: Vec<(&str, Box<dyn Port>)> = vec![
        ("mercury", Box::new(MercuryPort::new())),
        ("kimi", Box::new(KimiPort::new())),
        ("glm", Box::new(GlmPort::new())),
    ];

    let prompt = "Explain why the sky is blue in exactly one sentence.";
    let mut results: Vec<(String, u64, u64, bool)> = Vec::new();

    for (name, brain) in &brains {
        let record = invoke_generate(brain.as_ref(), prompt, 256)?;
        let tokens = record.structured_result["usage"]["total_tokens"]
            .as_u64()
            .unwrap_or(0);
        let content = record.structured_result["content"]
            .as_str()
            .unwrap_or("");
        results.push((name.to_string(), record.latency_ms, tokens, !content.is_empty()));
    }

    let mut lines = Vec::new();
    for (name, latency, tokens, ok) in &results {
        lines.push(format!("{name}: {latency}ms/{tokens}tok ok={ok}"));
    }

    let fastest = results
        .iter()
        .min_by_key(|r| r.1)
        .map(|r| r.0.clone())
        .unwrap_or_default();

    Ok(format!("{} | winner={fastest}", lines.join(" | ")))
}

fn phase6_observables() -> Result<String, String> {
    let brains: Vec<Box<dyn Port>> = vec![
        Box::new(MercuryPort::new()),
        Box::new(KimiPort::new()),
        Box::new(GlmPort::new()),
    ];

    let prompts = [
        ("factual", "What is the capital of France? One word."),
        ("creative", "Invent a name for a color that doesn't exist. One word."),
        ("reasoning", "If A>B and B>C, is A>C? One word."),
    ];

    println!();
    println!("  {:>12} | {:>10} {:>10} {:>10}", "", "mercury", "kimi", "glm");
    println!("  {:-<12}-+-{:-<10}-{:-<10}-{:-<10}", "", "", "", "");

    let mut total_latency = [0u64; 3];
    let mut total_tokens = [0u64; 3];
    let mut all_ok = true;

    for (label, prompt) in &prompts {
        let mut row_latency = Vec::new();
        let mut row_tokens = Vec::new();

        for (i, brain) in brains.iter().enumerate() {
            match invoke_generate(brain.as_ref(), prompt, 64) {
                Ok(record) => {
                    let tokens = record.structured_result["usage"]["total_tokens"]
                        .as_u64()
                        .unwrap_or(0);
                    row_latency.push(format!("{}ms", record.latency_ms));
                    row_tokens.push(format!("{}tok", tokens));
                    total_latency[i] += record.latency_ms;
                    total_tokens[i] += tokens;
                }
                Err(e) => {
                    row_latency.push("ERR".to_string());
                    row_tokens.push(e.clone());
                    all_ok = false;
                }
            }
        }

        println!(
            "  {:>12} | {:>10} {:>10} {:>10}",
            label, row_latency[0], row_latency[1], row_latency[2]
        );
    }

    println!("  {:-<12}-+-{:-<10}-{:-<10}-{:-<10}", "", "", "", "");
    println!(
        "  {:>12} | {:>10} {:>10} {:>10}",
        "total ms",
        total_latency[0],
        total_latency[1],
        total_latency[2]
    );
    println!(
        "  {:>12} | {:>10} {:>10} {:>10}",
        "total tok",
        total_tokens[0],
        total_tokens[1],
        total_tokens[2]
    );
    println!();

    let fastest_idx = total_latency
        .iter()
        .enumerate()
        .min_by_key(|(_, v)| **v)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let names = ["mercury", "kimi", "glm"];

    if !all_ok {
        return Err("one or more invocations failed".into());
    }

    Ok(format!(
        "fastest={}, latency={}ms/{}tok across {} prompts",
        names[fastest_idx],
        total_latency[fastest_idx],
        total_tokens[fastest_idx],
        prompts.len()
    ))
}
