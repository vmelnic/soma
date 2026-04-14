//! Browser / WebAssembly entry point for soma-next.
//!
//! Compiled only on `target_arch = "wasm32"`. Exposes JavaScript-visible
//! functions through wasm-bindgen, installs a panic hook that forwards
//! Rust panics to the browser console, and boots a real
//! `soma_next::bootstrap::Runtime` inside the browser tab with the
//! in-tab `dom` / `audio` / `voice` ports registered on its port runtime.
//!
//! Phase 1d: every `invoke_port` call now dispatches through the full
//! `DefaultPortRuntime` pipeline — lifecycle gate, sandbox check, policy
//! check, auth check, input schema validation, and the observation
//! record construction in `runtime/port.rs` — exactly the same code path
//! every native proof project uses. The thread-local `HashMap` port
//! registry from phase 1a/b/c is gone.

use std::cell::RefCell;

use wasm_bindgen::prelude::*;

pub mod audio_port;
pub mod dom_port;
pub mod voice_port;

use audio_port::AudioPort;
use dom_port::DomPort;
use voice_port::VoicePort;

use crate::bootstrap::{bootstrap_from_specs, Runtime};
use crate::config::SomaConfig;
use crate::runtime::goal::{GoalInput, GoalRuntime};
use crate::runtime::port::{Port, PortRuntime};
use crate::runtime::session::{SessionRuntime, StepResult};
use crate::runtime::skill::SkillRuntime;
use crate::types::goal::{GoalSource, GoalSourceType};
use crate::types::pack::PackSpec;
use crate::types::port::InvocationContext;
use crate::types::session::{BindingSource, WorkingBinding};

// A full `Runtime` lives for the lifetime of the tab. `thread_local!` is
// the idiomatic single-threaded wasm container; Runtime is not `Send`
// (it holds Box<dyn Trait> adapters with no Send bound), which is fine
// because wasm32-unknown-unknown has no real threads anyway.
thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

/// Called from JavaScript exactly once, before any other soma-next
/// function. Installs the panic hook so Rust panics end up as
/// `console.error` entries with full stack traces, and logs a boot
/// banner so the page can confirm the wasm module actually loaded.
#[wasm_bindgen(start)]
pub fn soma_start() {
    console_error_panic_hook::set_once();
    web_sys::console::log_1(&JsValue::from_str(
        "[soma-next wasm] boot — soma body loaded in the browser",
    ));
}

/// Boot a SOMA Runtime from a pack manifest JSON string.
///
/// The manifest can be empty (`""`) or a full `PackSpec` JSON object.
/// An empty manifest still produces a valid Runtime — useful for
/// proving the pipeline works before any brain-side code is written.
///
/// Regardless of what the manifest declares, the three in-tab browser
/// ports (`dom`, `audio`, `voice`) are always registered on top of it,
/// because a browser SOMA without them cannot produce any observable
/// side effects.
///
/// Safe to call multiple times — each call replaces the previously
/// booted Runtime. Returns a JSON summary of the booted state.
#[wasm_bindgen]
pub fn soma_boot_runtime(manifest_json: &str) -> Result<String, JsValue> {
    let pack_specs: Vec<PackSpec> = if manifest_json.trim().is_empty() {
        Vec::new()
    } else {
        let spec: PackSpec = serde_json::from_str(manifest_json)
            .map_err(|e| JsValue::from_str(&format!("manifest parse failed: {e}")))?;
        vec![spec]
    };

    let config = SomaConfig::default();
    let runtime = bootstrap_from_specs(&config, pack_specs)
        .map_err(|e| JsValue::from_str(&format!("bootstrap_from_specs: {e}")))?;

    // Register the built-in browser ports on top of whatever the
    // manifest declared. The `native` meta-feature is off on wasm, so
    // Dylib- and McpClient-backed ports from the manifest (if any)
    // would have been skipped with a warning by `create_port_adapter`.
    {
        let mut port_runtime = runtime
            .port_runtime
            .lock()
            .map_err(|_| JsValue::from_str("port_runtime mutex poisoned"))?;

        register_wasm_port(&mut *port_runtime, "dom", Box::new(DomPort::new()))?;
        register_wasm_port(&mut *port_runtime, "audio", Box::new(AudioPort::new()))?;
        register_wasm_port(&mut *port_runtime, "voice", Box::new(VoicePort::new()))?;
    }

    let summary = make_summary(&runtime)?;
    RUNTIME.with(|r| *r.borrow_mut() = Some(runtime));

    web_sys::console::log_1(&JsValue::from_str(
        "[soma-next wasm] Runtime booted with ports: dom, audio, voice",
    ));

    Ok(summary)
}

fn register_wasm_port(
    port_runtime: &mut crate::runtime::port::DefaultPortRuntime,
    name: &str,
    port: Box<dyn Port>,
) -> Result<(), JsValue> {
    let spec = port.spec().clone();
    port_runtime
        .register_port(spec, port)
        .map_err(|e| JsValue::from_str(&format!("register {name}: {e}")))?;
    port_runtime
        .activate(name)
        .map_err(|e| JsValue::from_str(&format!("activate {name}: {e}")))?;
    Ok(())
}

/// Return a JSON summary of the currently-booted Runtime: pack count,
/// registered ports, and each port's capabilities.
#[wasm_bindgen]
pub fn soma_runtime_summary() -> Result<String, JsValue> {
    RUNTIME.with(|r| {
        let slot = r.borrow();
        let runtime = slot.as_ref().ok_or_else(|| {
            JsValue::from_str("runtime not booted — call soma_boot_runtime first")
        })?;
        make_summary(runtime)
    })
}

fn make_summary(runtime: &Runtime) -> Result<String, JsValue> {
    let port_runtime = runtime
        .port_runtime
        .lock()
        .map_err(|_| JsValue::from_str("port_runtime mutex poisoned"))?;

    let ports: Vec<serde_json::Value> = port_runtime
        .list_ports(None)
        .iter()
        .map(|p| {
            serde_json::json!({
                "port_id": p.port_id,
                "namespace": p.namespace,
                "kind": format!("{:?}", p.kind),
                "capabilities": p
                    .capabilities
                    .iter()
                    .map(|c| c.capability_id.clone())
                    .collect::<Vec<_>>(),
            })
        })
        .collect();

    let summary = serde_json::json!({
        "booted": true,
        "pack_count": runtime.pack_specs.len(),
        "pack_ids": runtime.pack_specs.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
        "ports": ports,
    });

    serde_json::to_string(&summary)
        .map_err(|e| JsValue::from_str(&format!("summary serialize: {e}")))
}

/// Return a JSON array of all registered port IDs. Convenience wrapper
/// for the browser dev console.
#[wasm_bindgen]
pub fn soma_list_ports() -> Result<String, JsValue> {
    ensure_booted()?;
    RUNTIME.with(|r| {
        let slot = r.borrow();
        let runtime = slot.as_ref().expect("ensured booted above");
        let port_runtime = runtime
            .port_runtime
            .lock()
            .map_err(|_| JsValue::from_str("port_runtime mutex poisoned"))?;
        let ids: Vec<String> = port_runtime
            .list_ports(None)
            .iter()
            .map(|p| p.port_id.clone())
            .collect();
        serde_json::to_string(&ids)
            .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
    })
}

/// Invoke a capability on one of the registered ports.
///
/// Dispatches through `DefaultPortRuntime::invoke`, the same code path
/// every native proof project uses. Every gate runs — lifecycle,
/// remote-exposure, policy, auth, input-schema validation, sandbox —
/// before the adapter's `invoke` is called. Returns the resulting
/// `PortCallRecord` as a JSON string.
#[wasm_bindgen]
pub fn soma_invoke_port(
    port_id: &str,
    capability_id: &str,
    input_json: &str,
) -> Result<String, JsValue> {
    ensure_booted()?;

    let input: serde_json::Value = serde_json::from_str(input_json)
        .map_err(|e| JsValue::from_str(&format!("input_json is not valid JSON: {e}")))?;

    RUNTIME.with(|r| {
        let slot = r.borrow();
        let runtime = slot.as_ref().expect("ensured booted above");
        let port_runtime = runtime
            .port_runtime
            .lock()
            .map_err(|_| JsValue::from_str("port_runtime mutex poisoned"))?;

        let ctx = InvocationContext {
            caller_identity: Some("wasm".to_string()),
            ..Default::default()
        };

        let record = port_runtime
            .invoke(port_id, capability_id, input, &ctx)
            .map_err(|e| JsValue::from_str(&format!("invoke: {e}")))?;

        serde_json::to_string(&record)
            .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
    })
}

/// Ensure a Runtime has been booted. If not, auto-boot with an empty
/// manifest — this keeps the phase 1a/b/c harnesses working unchanged
/// even if they never call `soma_boot_runtime` explicitly.
fn ensure_booted() -> Result<(), JsValue> {
    let already = RUNTIME.with(|r| r.borrow().is_some());
    if !already {
        soma_boot_runtime("")?;
    }
    Ok(())
}

/// Phase 1a compatibility shim. The new generic path is
/// `soma_invoke_port("dom", "append_heading", "{\"text\":\"...\"}")` —
/// this function wraps that so the original proof harness keeps working.
#[wasm_bindgen]
pub fn soma_demo_render_heading(text: &str) -> Result<JsValue, JsValue> {
    let input = serde_json::json!({ "text": text }).to_string();
    let json = soma_invoke_port("dom", "append_heading", &input)?;
    Ok(JsValue::from_str(&json))
}

/// Run a natural-language goal through the booted `SessionController`.
///
/// Parses the objective via `DefaultGoalRuntime`, creates a session,
/// injects a `"text"` binding into working memory (set to the full
/// objective string so any skill that needs text-to-render picks it
/// up), then loops `run_step` until the session reaches a terminal
/// state. On termination the episode is stored and the multistep
/// learning pipeline is triggered so repeated runs of the same goal
/// eventually compile a routine and start walking it without the
/// selector.
///
/// Returns a JSON object describing the final session state including
/// step count, elapsed milliseconds, whether plan-following was
/// active, and the total number of schemas/routines in memory — the
/// JS harness uses these to demonstrate that repeat invocations get
/// faster as the runtime learns.
#[wasm_bindgen]
pub fn soma_run_goal(objective: &str) -> Result<String, JsValue> {
    ensure_booted()?;

    RUNTIME.with(|r| {
        let mut slot = r.borrow_mut();
        let runtime = slot.as_mut().expect("ensured booted above");

        // Parse, normalize, validate.
        let source = GoalSource {
            source_type: GoalSourceType::Api,
            identity: Some("wasm".to_string()),
            session_id: None,
            peer_id: None,
        };
        let input = GoalInput::NaturalLanguage {
            text: objective.to_string(),
            source,
        };
        let mut goal = runtime
            .goal_runtime
            .parse_goal(input)
            .map_err(|e| JsValue::from_str(&format!("goal parse: {e}")))?;
        runtime.goal_runtime.normalize_goal(&mut goal);
        runtime
            .goal_runtime
            .validate_goal(&goal)
            .map_err(|e| JsValue::from_str(&format!("goal validation: {e}")))?;

        let goal_id = goal.goal_id;

        // Create the session via the real SessionController.
        let mut session = runtime
            .session_controller
            .create_session(goal)
            .map_err(|e| JsValue::from_str(&format!("create_session: {e}")))?;
        let session_id = session.session_id;

        // `SimpleBeliefSource::build_initial_belief` ignores the goal
        // entirely (same as every native proof project that isn't
        // soma-project-multistep). Inject the objective string as a
        // working-memory binding named "text" so the say_hello skill's
        // input schema finds it during bind_inputs. This is the same
        // workaround `soma-project-multistep` uses.
        session.working_memory.active_bindings.push(WorkingBinding {
            name: "text".to_string(),
            value: serde_json::json!(objective),
            source: BindingSource::GoalField,
        });

        // Run the control loop until terminal.
        let start_ms = js_sys::Date::now();
        let status: &'static str;
        let mut error_detail: Option<String> = None;
        loop {
            match runtime.session_controller.run_step(&mut session) {
                Ok(StepResult::Continue) => continue,
                Ok(StepResult::Completed) => {
                    status = "completed";
                    break;
                }
                Ok(StepResult::Failed(reason)) => {
                    status = "failed";
                    error_detail = Some(reason);
                    break;
                }
                Ok(StepResult::Aborted) => {
                    status = "aborted";
                    break;
                }
                Ok(StepResult::WaitingForInput(msg)) => {
                    status = "waiting_for_input";
                    error_detail = Some(msg);
                    break;
                }
                Ok(StepResult::WaitingForRemote(msg)) => {
                    status = "waiting_for_remote";
                    error_detail = Some(msg);
                    break;
                }
                Err(e) => {
                    return Err(JsValue::from_str(&format!("run_step: {e}")));
                }
            }
        }
        let elapsed_ms = js_sys::Date::now() - start_ms;

        // Inspect the session trace for evidence plan-following activated.
        // `WorkingMemory.active_plan` is cleared at the end of the plan's
        // last step, so polling it after `run_step` always sees `None`
        // even for sessions that ran entirely in plan-following mode.
        // The trustworthy signal is `TraceStep.retrieved_routines` — the
        // list of routine IDs that `RoutineMemoryAdapter::retrieve_matching`
        // returned during step 5. If any step retrieved a routine, the
        // session controller's step 6b loaded that routine's compiled
        // skill path into `active_plan` and the session walked the plan.
        let plan_following_active = session
            .trace
            .steps
            .iter()
            .any(|s| !s.retrieved_routines.is_empty());

        // On terminal, store episode + fire the learning pipeline.
        // Structural copy of the native MCP handler's terminal block.
        let is_terminal = matches!(status, "completed" | "failed" | "aborted");

        if is_terminal {
            let mut episode = crate::interfaces::cli::build_episode_from_session(
                &session,
                Some(&*runtime.embedder),
            );
            episode.world_state_context = runtime.world_state.lock().ok()
                .map(|ws| ws.snapshot())
                .unwrap_or(serde_json::json!({}));
            let fingerprint = episode.goal_fingerprint.clone();
            let adapter = crate::adapters::EpisodeMemoryAdapter::new(
                std::sync::Arc::clone(&runtime.episode_store),
                std::sync::Arc::clone(&runtime.embedder),
            );
            let _ = adapter.store(episode);
            crate::interfaces::cli::attempt_learning(
                &runtime.episode_store,
                &runtime.schema_store,
                &runtime.routine_store,
                &fingerprint,
                &*runtime.embedder,
            );
        }

        // Query memory-store sizes so the JS side can watch the pipeline
        // learn over repeated invocations.
        let episode_count = runtime
            .episode_store
            .lock()
            .map(|s| s.count())
            .unwrap_or(0);
        let schema_count = runtime
            .schema_store
            .lock()
            .map(|s| s.list_all().len())
            .unwrap_or(0);
        let routine_count = runtime
            .routine_store
            .lock()
            .map(|s| s.list_all().len())
            .unwrap_or(0);

        let last_skill = session
            .trace
            .steps
            .last()
            .map(|s| s.selected_skill.clone());

        let summary = serde_json::json!({
            "session_id": session_id.to_string(),
            "goal_id": goal_id.to_string(),
            "objective": objective,
            "status": status,
            "error_detail": error_detail,
            "steps": session.trace.steps.len(),
            "last_skill": last_skill,
            "elapsed_ms": elapsed_ms,
            "plan_following": plan_following_active,
            "episode_count": episode_count,
            "schema_count": schema_count,
            "routine_count": routine_count,
        });

        serde_json::to_string(&summary)
            .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
    })
}

/// Inject a compiled `Routine` directly into the routine store.
///
/// Takes a JSON-encoded `Routine` (see `soma_next::types::routine::Routine`)
/// and registers it via `RoutineStore::register`. Subsequent `soma_run_goal`
/// invocations whose objective matches the routine's `match_conditions`
/// will enter plan-following mode — the `SessionController` loads the
/// routine's `compiled_skill_path` into `WorkingMemory.active_plan` and
/// walks it step-by-step without re-running the selector.
///
/// This is the same code path `soma-project-multistep` uses to prove
/// plan-following works on native. Phase 1f uses it to demonstrate
/// plan-following end-to-end in the browser without waiting for organic
/// schema induction (which needs multi-step episodes the single-skill
/// hello pack doesn't naturally produce).
#[wasm_bindgen]
pub fn soma_inject_routine(routine_json: &str) -> Result<String, JsValue> {
    use crate::types::routine::Routine;

    ensure_booted()?;

    let routine: Routine = serde_json::from_str(routine_json)
        .map_err(|e| JsValue::from_str(&format!("routine parse failed: {e}")))?;

    let routine_id = routine.routine_id.clone();
    let skill_path = routine.compiled_skill_path.clone();

    RUNTIME.with(|r| {
        let slot = r.borrow();
        let runtime = slot.as_ref().expect("ensured booted above");
        let mut store = runtime
            .routine_store
            .lock()
            .map_err(|_| JsValue::from_str("routine_store mutex poisoned"))?;
        store
            .register(routine)
            .map_err(|e| JsValue::from_str(&format!("register routine: {e}")))?;
        Ok::<_, JsValue>(())
    })?;

    web_sys::console::log_1(&JsValue::from_str(&format!(
        "[soma-next wasm] injected routine {routine_id} → {skill_path:?}"
    )));

    let summary = serde_json::json!({
        "injected": true,
        "routine_id": routine_id,
        "compiled_skill_path": skill_path,
    });
    serde_json::to_string(&summary)
        .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
}

/// Return the count of registered skills on the currently-booted runtime.
#[wasm_bindgen]
pub fn soma_list_skills() -> Result<String, JsValue> {
    ensure_booted()?;
    RUNTIME.with(|r| {
        let slot = r.borrow();
        let runtime = slot.as_ref().expect("ensured booted above");
        let skills: Vec<serde_json::Value> = runtime
            .skill_runtime
            .list_skills(None)
            .iter()
            .map(|s| {
                serde_json::json!({
                    "skill_id": s.skill_id,
                    "namespace": s.namespace,
                    "description": s.description,
                    "capability_requirements": s.capability_requirements,
                })
            })
            .collect();
        serde_json::to_string(&skills)
            .map_err(|e| JsValue::from_str(&format!("serialize: {e}")))
    })
}
