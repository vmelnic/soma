use crate::active::{
    evaluate_withheld_transitions, ActivePlan, ActiveWorldModel, TransitionEvaluation,
};
use crate::ca::{deterministic_map, AgentNetwork, ConsciousAgent, FiniteMeasurableSpace};
use crate::certificate::{EmpiricalMetric, ValidationLedger};
use crate::exact::{sum, Exact};
use crate::instrument::{
    certify_matrix_round_trip, observe, InstrumentOutput, MatrixObservation, MeasurementBatch,
    TraceObservation,
};
use crate::kernel::{MarkovKernel, StochasticMap};
use crate::memory::{digest_json, save_atomic, Episode, PersistentMemory};
use crate::projection::{build_projection_report, ProjectionReport};
use crate::rtl::{ExperienceCombination, ExperienceModel, SpaceMorphism};
use crate::trace_logic::{
    certify_stationary_map, certify_trace_order, FiniteTraceEvidence, MarkovChain,
    StationaryMapCertificate, TraceOrderCertificate, TraceWitness,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Deserialize)]
struct ValidationSuite {
    schema: String,
    id: String,
    criteria_frozen_before_evaluation: bool,
    instruments: Vec<String>,
    model_source_id: String,
    reported_experience_source_id: String,
    boundary_source_id: String,
    #[serde(default)]
    trace_source_id: Option<String>,
    duration_horizon: usize,
    memory_path: String,
    report_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IntegratedReport {
    pub schema: String,
    pub suite_id: String,
    pub criteria_frozen_before_evaluation: bool,
    pub source_audits: BTreeMap<String, crate::instrument::IngestionAudit>,
    pub learned_chain: MarkovChain,
    pub agent_certificate: crate::ca::AgentCertificate,
    pub network_certificate: crate::ca::NetworkCertificate,
    pub fusion_certificate: crate::ca::FusionCertificate,
    pub observer_trace: TraceWitness,
    pub trace_order: TraceOrderCertificate,
    pub stationary_map: StationaryMapCertificate,
    pub finite_trace_evidence: FiniteTraceEvidence,
    pub experience_combination: ExperienceCombination,
    pub fine_graining_morphism: SpaceMorphism,
    pub projection: ProjectionReport,
    pub active_world_model: Option<ActiveWorldModel>,
    pub active_plan: Option<ActivePlan>,
    pub withheld_transition_evaluation: Option<TransitionEvaluation>,
    pub ledger: ValidationLedger,
    pub memory_digest: String,
    pub complete: bool,
}

pub fn run_suite(suite_path: &Path) -> Result<IntegratedReport> {
    let suite_bytes = fs::read(suite_path)
        .with_context(|| format!("cannot read validation suite {}", suite_path.display()))?;
    let suite: ValidationSuite = serde_json::from_slice(&suite_bytes)?;
    if suite.schema != "soma.rtl.validation-suite.v1" {
        bail!("unsupported validation suite schema");
    }
    if !suite.criteria_frozen_before_evaluation {
        bail!("integrated validation criteria were not frozen before evaluation");
    }
    let root = suite_path.parent().unwrap_or_else(|| Path::new("."));
    let mut sequences = BTreeMap::<String, MeasurementBatch>::new();
    let mut matrices = BTreeMap::<String, MatrixObservation>::new();
    let mut traces = BTreeMap::<String, TraceObservation>::new();
    for relative in &suite.instruments {
        let path = root.join(relative);
        match observe(&path)? {
            InstrumentOutput::Sequences(batch) => {
                sequences.insert(batch.source.id.clone(), batch);
            }
            InstrumentOutput::Matrix(batch) => {
                matrices.insert(batch.source.id.clone(), batch);
            }
            InstrumentOutput::Traces(batch) => {
                traces.insert(batch.source.id.clone(), batch);
            }
        }
    }
    let model = sequences
        .get(&suite.model_source_id)
        .ok_or_else(|| anyhow::anyhow!("model observation source is absent"))?;
    let reports = sequences
        .get(&suite.reported_experience_source_id)
        .ok_or_else(|| anyhow::anyhow!("reported-experience source is absent"))?;
    let boundary = matrices
        .get(&suite.boundary_source_id)
        .ok_or_else(|| anyhow::anyhow!("boundary source is absent"))?;
    let observed_sequences = model
        .sequences
        .iter()
        .map(|sequence| {
            sequence
                .events
                .iter()
                .map(|event| event.experience.clone())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let kernel = MarkovKernel::from_sequences(&observed_sequences)?;
    if kernel
        .transition
        .iter()
        .any(|row| !sum(row.clone()).is_one())
    {
        bail!("real sequence evidence did not identify a closed experience law");
    }
    let initial = empirical_initial(&kernel, &observed_sequences)?;
    let learned_chain = MarkovChain {
        id: format!("empirical-chain:{}", model.source.sha256),
        initial,
        kernel: kernel.clone(),
        provenance: vec![model.source.sha256.clone()],
    };
    learned_chain.validate()?;

    // This canonical realization gives each possible next experience a choice
    // label and uses identity action/perception boundaries. It is an explicit
    // semantic model, not an edge-witness factorization claimed to be unique.
    let atoms = kernel.states.clone();
    let identity = StochasticMap::identity(atoms.clone())?;
    let agent = ConsciousAgent {
        id: "agent:real-observation-model".to_string(),
        experience: FiniteMeasurableSpace::new("X:observed-experience", atoms.clone())?,
        choices: FiniteMeasurableSpace::new("G:next-experience-choice", atoms.clone())?,
        world: FiniteMeasurableSpace::new("W:shared-observation-world", atoms.clone())?,
        perception: identity.clone(),
        decision: StochasticMap::new(atoms.clone(), atoms.clone(), kernel.transition.clone())?,
        action: identity.clone(),
        experience_counter: model.audit.rows_accepted as u64,
        provenance: vec![model.source.sha256.clone()],
    };
    let agent_certificate = agent.validate()?;
    let joint_labels = AgentNetwork::canonical_joint_choices(std::slice::from_ref(&agent))?;
    let (network, _) = AgentNetwork::join(
        "network:real-observation-model",
        vec![agent.clone()],
        StochasticMap::new(joint_labels, atoms.clone(), identity.transition.clone())?,
        vec![model.source.sha256.clone()],
    )?;
    let network_certificate = network.validate()?;
    let fusion_certificate = network.fusion_certificate()?;

    let support = most_observed_support(&kernel, &observed_sequences);
    let observer_trace =
        TraceWitness::derive(&learned_chain, support.clone(), suite.duration_horizon)?;
    let trace_order = certify_trace_order(
        &observer_trace.trace,
        &learned_chain,
        &observer_trace,
        suite.duration_horizon,
    )?;
    let stationary_map =
        certify_stationary_map(&learned_chain, &observer_trace, suite.duration_horizon)?;
    let finite_trace_evidence = FiniteTraceEvidence::observe(
        &observed_sequences,
        support,
        &model.source.sha256,
        Some(&observer_trace),
    )?;
    let full_experience = ExperienceModel {
        id: "experience-space:measured-full".to_string(),
        chain: learned_chain.clone(),
        description_digest: learned_chain.digest()?,
        provenance: learned_chain.provenance.clone(),
    };
    let traced_experience = ExperienceModel {
        id: "experience-space:measured-trace".to_string(),
        chain: observer_trace.trace.clone(),
        description_digest: observer_trace.trace.digest()?,
        provenance: observer_trace.trace.provenance.clone(),
    };
    let experience_combination = ExperienceCombination::independent(
        "experience-space:measured-product",
        &[full_experience.clone(), traced_experience.clone()],
        vec![model.source.sha256.clone()],
    )?;
    let refinement_assignments = observer_trace
        .support
        .iter()
        .map(|state| (state.clone(), state.clone()))
        .collect::<BTreeMap<_, _>>();
    let fine_graining_morphism = SpaceMorphism::trace_refinement(
        "morphism:measured-trace-refinement",
        &traced_experience,
        &full_experience,
        deterministic_map(
            observer_trace.support.clone(),
            learned_chain.kernel.states.clone(),
            &refinement_assignments,
        )?,
        observer_trace.clone(),
        vec![model.source.sha256.clone()],
    );
    fine_graining_morphism.validate(
        &BTreeMap::from([
            (full_experience.id.clone(), full_experience),
            (traced_experience.id.clone(), traced_experience),
        ]),
        suite.duration_horizon,
    )?;
    let projection = build_projection_report(
        &learned_chain.kernel,
        &observer_trace.return_certificate,
        Some(reports),
    )?;
    let boundary_certificate = certify_matrix_round_trip(boundary)?;
    let (active_world_model, active_plan, withheld_transition_evaluation) =
        if let Some(source_id) = &suite.trace_source_id {
            let trace = traces
                .get(source_id)
                .ok_or_else(|| anyhow::anyhow!("interactive trace source is absent"))?;
            let model = ActiveWorldModel::learn(&trace.events)?;
            let evaluation = evaluate_withheld_transitions(&trace.events)?;
            let initial = trace
                .events
                .first()
                .ok_or_else(|| anyhow::anyhow!("interactive trace source is empty"))?
                .state
                .clone();
            let plan = model.plan(&initial, suite.duration_horizon)?;
            (Some(model), Some(plan), Some(evaluation))
        } else {
            (None, None, None)
        };

    let memory_path = root.join(&suite.memory_path);
    let memory_digest = persist_measurements(&memory_path, model)?;
    let mut ledger = ValidationLedger::new();
    ledger.formal(
        "agent.q_equals_dap",
        agent_certificate.q_equals_dap,
        "The typed agent's qualia kernel is the exact D-A-P composite.",
        Some(agent_certificate.qualia_kernel_digest.clone()),
        "This validates the declared finite model; it does not prove the ontology.",
    );
    ledger.formal(
        "experience.combination_and_refinement",
        experience_combination.product_kernel_factorizes,
        "Measured experience spaces combine by an exact product and the observer expands through a trace-certified fine-graining.",
        Some(digest_json(&(&experience_combination, &fine_graining_morphism))?),
        "This is a finite formal construction over measured states, not evidence for a physical ontology.",
    );
    ledger.formal(
        "network.executable",
        network_certificate.compatibility.compatible(),
        "The real-observation agent participates in an executable shared-world network.",
        Some(network_certificate.joint_qualia_kernel_digest.clone()),
        "The measured suite instantiates one member; the implementation supports finite products.",
    );
    ledger.formal(
        "trace.order",
        trace_order.observer_is_trace && trace_order.observer_is_internal,
        "The observer chain is an exact first-return trace of the learned experience chain.",
        Some(trace_order.witness_digest.clone()),
        "The exact trace is a formal model; finite observations are recorded separately.",
    );
    ledger.formal(
        "trace.stationary_map",
        stationary_map.homomorphism_holds,
        "The trace stationary measure is the normalized parent restriction.",
        Some(digest_json(&stationary_map)?),
        "This finite result requires the identified closed ergodic component.",
    );
    ledger.boundary(
        "boundary.lossless",
        boundary_certificate.lossless_at_json_value_level
            && boundary_certificate.cognitive_operators_in_adapter == 0,
        "The external indexed document round-trips without cognitive operators.",
        Some(boundary_certificate.source_digest.clone()),
        "This certifies transport only, not task solution.",
    );
    ledger.empirical_metrics.push(EmpiricalMetric {
        id: "finite_trace.observed_transitions".to_string(),
        source_digest: model.source.sha256.clone(),
        sample_count: finite_trace_evidence.observed_transitions,
        value: Some(finite_trace_evidence.observed_transitions as f64),
        unit: "transitions".to_string(),
        criterion_fixed_before_evaluation: suite.criteria_frozen_before_evaluation,
        interpretation: "Recorded visible-to-visible transitions after skipping hidden states; no pseudocounts were added.".to_string(),
    });
    if let Some(evaluation) = &withheld_transition_evaluation {
        ledger.empirical_metrics.push(EmpiricalMetric {
            id: "interactive_trace.held_out_predictions".to_string(),
            source_digest: active_world_model.as_ref().unwrap().model_digest.clone(),
            sample_count: evaluation.held_out_events,
            value: Some(evaluation.top_one_correct as f64),
            unit: "top-one-correct-transitions".to_string(),
            criterion_fixed_before_evaluation: suite.criteria_frozen_before_evaluation,
            interpretation: "Predictions on the last measured edge of each multi-step external trace; unseen state-action pairs remain unscored.".to_string(),
        });
    }
    let complete = ledger.required_checks_pass();
    let report = IntegratedReport {
        schema: "soma.conscious-agent-validation.v3".to_string(),
        suite_id: suite.id,
        criteria_frozen_before_evaluation: suite.criteria_frozen_before_evaluation,
        source_audits: sequences
            .iter()
            .map(|(id, batch)| (id.clone(), batch.audit.clone()))
            .collect(),
        learned_chain,
        agent_certificate,
        network_certificate,
        fusion_certificate,
        observer_trace,
        trace_order,
        stationary_map,
        finite_trace_evidence,
        experience_combination,
        fine_graining_morphism,
        projection,
        active_world_model,
        active_plan,
        withheld_transition_evaluation,
        ledger,
        memory_digest,
        complete,
    };
    let report_path = root.join(&suite.report_path);
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    Ok(report)
}

fn empirical_initial(kernel: &MarkovKernel, sequences: &[Vec<String>]) -> Result<Vec<Exact>> {
    let mut counts = vec![0usize; kernel.states.len()];
    for sequence in sequences {
        if let Some(first) = sequence.first() {
            let index = kernel
                .index_of(first)
                .ok_or_else(|| anyhow::anyhow!("initial observation is outside learned kernel"))?;
            counts[index] += 1;
        }
    }
    let total = counts.iter().sum::<usize>();
    if total == 0 {
        bail!("real sequences have no initial observations");
    }
    counts
        .into_iter()
        .map(|count| Exact::from_counts(count, total))
        .collect()
}

fn most_observed_support(kernel: &MarkovKernel, sequences: &[Vec<String>]) -> Vec<String> {
    let mut counts = kernel
        .states
        .iter()
        .map(|state| {
            let count = sequences
                .iter()
                .flatten()
                .filter(|item| *item == state)
                .count();
            (state.clone(), count)
        })
        .collect::<Vec<_>>();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    counts
        .into_iter()
        .take(kernel.states.len().div_ceil(2).max(1))
        .map(|(state, _)| state)
        .collect()
}

fn persist_measurements(path: &Path, batch: &MeasurementBatch) -> Result<String> {
    let mut memory = PersistentMemory::empty("integrated-real-observation-agent", 1)?;
    for sequence in &batch.sequences {
        for event in &sequence.events {
            memory.episodes.push(Episode {
                source_digest: batch.source.sha256.clone(),
                sequence_id: sequence.id.clone(),
                ordinal: event.ordinal,
                experience: event.experience.clone(),
                intervention: event.intervention.clone(),
                report_digest: event.report_digest.clone(),
            });
        }
    }
    memory.consolidate();
    memory.record_transition("consolidated external observations", &batch.source.sha256)?;
    save_atomic(path, &memory)
}
