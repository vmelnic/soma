use crate::active::ObservedTransition;
use crate::ca::{
    deterministic_map, AgentCertificate, AgentNetwork, AgentStepCertificate, ConsciousAgent,
    FiniteMeasurableSpace, FusionCertificate, NetworkCertificate,
};
use crate::exact::Exact;
use crate::generative::{
    induce, FeedbackRevision, GenerativeHypothesis, GenerativeInduction, GenerativeLimits,
    ProgramExecution, ReplayScore,
};
use crate::headset::{HeadsetCertificate, LearnedActionAlgebra, LearnedHeadset, LearnedOperation};
use crate::kernel::{zeros, MarkovKernel, StochasticMap};
use crate::memory::{digest_json, load_verified, save_atomic, Episode, PersistentMemory};
use crate::rtl::{
    ConfigurationTransport, ExperienceModel, PolicyHierarchy, PolicyLevel,
    RecursiveTraceCertificate, RecursiveTraceDynamics, RtlConfiguration, SpaceMorphism,
    SpaceMorphismKind,
};
use crate::structure::{
    FiniteRelationalStructure, IndexedFrameSchema, IndexedStructure,
    StructuralCapabilityCertificate,
};
use crate::substrate::{certify_program_execution, ProgramSubstrateCertificate};
use crate::trace_logic::{MarkovChain, TraceWitness};
use crate::world::{
    load_sealed_interaction, ActionCommitment, EvaluationReport, LearningView, RelationalFrame,
    SealingCertificate,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteractionLimits {
    pub generative: GenerativeLimits,
    pub trace_duration_horizon: usize,
}

impl InteractionLimits {
    pub fn validate(&self) -> Result<()> {
        self.generative.validate()?;
        if self.trace_duration_horizon == 0 {
            bail!("interaction trace horizon must be positive");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueryAction {
    pub episode_id: String,
    pub observation_digest: String,
    pub execution: ProgramExecution,
    pub action: ActionCommitment,
    pub agent_step: AgentStepCertificate,
    pub substrate: ProgramSubstrateCertificate,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CausalEvent {
    pub sequence: usize,
    pub phase: String,
    pub episode_id: String,
    pub input_digest: String,
    pub executable_digest: String,
    pub output_digest: String,
    pub before_answer_reveal: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CausalNode {
    pub id: String,
    pub kind: String,
    pub payload_digest: String,
    pub before_answer_reveal: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CausalEdge {
    pub from: String,
    pub to: String,
    pub mechanism_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EpisodeCausalPath {
    pub episode_id: String,
    pub ordered_nodes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CausalProvenanceGraph {
    pub nodes: Vec<CausalNode>,
    pub edges: Vec<CausalEdge>,
    pub episode_paths: Vec<EpisodeCausalPath>,
}

impl CausalProvenanceGraph {
    pub fn validate(&self) -> Result<()> {
        let nodes = self
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect::<BTreeMap<_, _>>();
        if nodes.len() != self.nodes.len()
            || self.nodes.iter().any(|node| {
                node.id.is_empty() || node.kind.is_empty() || node.payload_digest.is_empty()
            })
        {
            bail!("causal provenance graph has duplicate or incomplete nodes");
        }
        let edges = self
            .edges
            .iter()
            .map(|edge| ((edge.from.as_str(), edge.to.as_str()), edge))
            .collect::<BTreeMap<_, _>>();
        if edges.len() != self.edges.len()
            || self.edges.iter().any(|edge| {
                !nodes.contains_key(edge.from.as_str())
                    || !nodes.contains_key(edge.to.as_str())
                    || edge.mechanism_digest.is_empty()
            })
        {
            bail!("causal provenance graph has invalid edges");
        }
        for path in &self.episode_paths {
            if path.ordered_nodes.len() < 8
                || path
                    .ordered_nodes
                    .windows(2)
                    .any(|pair| !edges.contains_key(&(pair[0].as_str(), pair[1].as_str())))
            {
                bail!("episode causal path is incomplete");
            }
            let reveal = path
                .ordered_nodes
                .iter()
                .position(|id| nodes[id.as_str()].kind == "feedback")
                .ok_or_else(|| anyhow::anyhow!("episode causal path has no feedback node"))?;
            if path.ordered_nodes[..reveal]
                .iter()
                .any(|id| !nodes[id.as_str()].before_answer_reveal)
                || path.ordered_nodes[reveal..]
                    .iter()
                    .any(|id| nodes[id.as_str()].before_answer_reveal)
            {
                bail!("causal path violates the answer-reveal boundary");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CausalArchitectureCertificate {
    pub sealing_before_action: SealingCertificate,
    pub causal_graph: CausalProvenanceGraph,
    pub selected_public_replay: ReplayScore,
    pub selected_leave_one_out: ReplayScore,
    pub proposal_kernel_digest: String,
    pub posterior_normalized: bool,
    pub detailed_balance: bool,
    pub local_reversible_proposals: bool,
    pub agent: AgentCertificate,
    pub network: NetworkCertificate,
    pub headset: HeadsetCertificate,
    pub recursive_trace: RecursiveTraceCertificate,
    pub substrate_executions: Vec<ProgramSubstrateCertificate>,
    pub feedback_revision: FeedbackRevision,
    pub pre_feedback_state_digest: String,
    pub post_feedback_state_digest: String,
    pub commitments_preceded_reveal: bool,
}

impl CausalArchitectureCertificate {
    pub fn complete(&self) -> bool {
        self.sealing_before_action.reveal_count == 0
            && self
                .sealing_before_action
                .answer_payloads_absent_from_learning_type
            && self.causal_graph.validate().is_ok()
            && self.selected_public_replay.all_exact()
            && self.selected_leave_one_out.all_exact()
            && !self.proposal_kernel_digest.is_empty()
            && self.posterior_normalized
            && self.detailed_balance
            && self.local_reversible_proposals
            && self.agent.q_equals_dap
            && self.network.member_count > 0
            && self.headset.observer_is_trace
            && self.headset.perception_closed
            && self.headset.action_closed
            && self.headset.posterior_normalized
            && self.headset.action_algebra_executable
            && self.headset.composition_semantics_executable
            && self.headset.perception_fibers_partition_world
            && self.headset.geometry_derived_from_actions
            && self.recursive_trace.distinct_experience_spaces > 1
            && self.recursive_trace.variable_space_transitions > 0
            && self.recursive_trace.all_space_changes_trace_valid
            && self.recursive_trace.policy_caused_configuration_dynamics
            && self.recursive_trace.one_markov_law
            && self.recursive_trace.policy_depth >= 2
            && !self.substrate_executions.is_empty()
            && self.substrate_executions.iter().all(|execution| {
                execution.open_trace_probability_one
                    && execution.execution_digest_matches
                    && execution.headset_preceded_program
                    && execution.causal_execution_replayed
                    && execution.ast_result_matches_committed_action
                    && execution.symbolic_search_trace.exact_mass_decomposition
                    && execution.universal_language.two_unbounded_registers
            })
            && self.feedback_revision.posterior_changed
            && self.pre_feedback_state_digest != self.post_feedback_state_digest
            && self.commitments_preceded_reveal
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteractionReport {
    pub schema: String,
    pub public_view_digest: String,
    pub sealing_before_action: SealingCertificate,
    pub induction: GenerativeInduction,
    pub observation_structure: FiniteRelationalStructure,
    pub action_structure: FiniteRelationalStructure,
    pub observation_capability: StructuralCapabilityCertificate,
    pub action_capability: StructuralCapabilityCertificate,
    pub action_schema: IndexedFrameSchema,
    pub pre_feedback_headset: LearnedHeadset,
    pub pre_feedback_headset_certificate: HeadsetCertificate,
    pub agent_network_certificate: NetworkCertificate,
    pub fusion_certificate: FusionCertificate,
    pub recursive_trace_dynamics: RecursiveTraceDynamics,
    pub query_actions: Vec<QueryAction>,
    pub causal_events: Vec<CausalEvent>,
    pub evaluation: EvaluationReport,
    pub feedback_revision: FeedbackRevision,
    pub post_feedback_headset: LearnedHeadset,
    pub post_feedback_headset_certificate: HeadsetCertificate,
    pub memory_path: PathBuf,
    pub memory_digest: String,
    pub architecture: CausalArchitectureCertificate,
    pub integration_complete: bool,
    pub external_task_solved: bool,
}

struct InterfaceBundle {
    headset: LearnedHeadset,
    headset_certificate: HeadsetCertificate,
    agent: ConsciousAgent,
    network_certificate: NetworkCertificate,
    fusion_certificate: FusionCertificate,
    recursive: RecursiveTraceDynamics,
    observation_experiences: Vec<String>,
    action_world_states: Vec<String>,
}

pub fn run_world_interaction(
    manifest_path: &Path,
    report_path: &Path,
    limits: InteractionLimits,
) -> Result<InteractionReport> {
    limits.validate()?;
    let mut sealed = load_sealed_interaction(manifest_path)?;
    let sealing_before_action = sealed.sealing_certificate();
    let view = sealed.learning_view().clone();
    let memory_path = memory_path_for(report_path);
    let prior_memory = if memory_path.exists() {
        Some(load_verified(&memory_path)?)
    } else {
        None
    };
    let learned_programs = prior_memory
        .as_ref()
        .map(|memory| {
            memory
                .generative_procedures
                .values()
                .chain(memory.failed_structural_programs.values())
                .filter_map(|value| serde_json::from_value(value.clone()).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let learned_priors = prior_memory
        .as_ref()
        .map(|memory| memory.generative_posteriors.clone())
        .unwrap_or_default();
    let (observation_structure, action_structure, action_schema) = infer_boundary_schemas(&view)?;
    let observation_capability =
        IndexedStructure::infer(&view.demonstrations[0].observation)?.capability_certificate()?;
    let action_capability = IndexedStructure::infer(&view.demonstrations[0].demonstrated_action)?
        .capability_certificate()?;
    let induction = induce(
        &view,
        &limits.generative,
        &observation_structure.structural_type_digest,
        &action_structure.structural_type_digest,
        &learned_programs,
        &learned_priors,
    )?;
    let selected = induction.selected()?.clone();

    let mut executions = Vec::new();
    let mut action_frames = Vec::new();
    for query in &view.queries {
        let execution = selected.program.execute(&query.observation)?;
        if execution.unresolved_cells > 0 || execution.ambiguous_cells > 0 {
            bail!("selected generative program did not produce a complete unique action");
        }
        let action = action_schema.encode(
            format!("{}:committed-action", query.episode_id),
            &execution.result,
            vec![
                view.public_digest.clone(),
                selected.id.clone(),
                execution.execution_digest.clone(),
            ],
        )?;
        executions.push(execution);
        action_frames.push(action);
    }
    // Compile the candidate's predicted finite interface before action. The
    // actual headset perception and program execution happen below, around the
    // agent decision, and must replay this prediction exactly.
    let pre_interface = build_interface(
        &view,
        &induction.hypotheses,
        &selected,
        &executions,
        &action_frames,
        "feedback:pending",
        0,
        limits.trace_duration_horizon,
    )?;
    let mut query_actions = Vec::new();
    let mut commitments = Vec::new();
    let mut causal_events = Vec::new();
    for (index, query) in view.queries.iter().enumerate() {
        let perceived = selected.program.perceive(&query.observation)?;
        if perceived.execution_digest != executions[index].perception_digest {
            bail!("causal headset perception differs from its compiled prediction");
        }
        let step = pre_interface
            .agent
            .step_certificate(&pre_interface.observation_experiences[index])?;
        let action_state = &pre_interface.action_world_states[index];
        let caused = step.paths.iter().any(|path| {
            &path.world == action_state
                && path.next_experience.starts_with("experience:commit:")
                && path.probability.is_one()
        });
        if !caused {
            bail!("the conscious-agent transition did not cause the committed action state");
        }
        let execution = selected
            .program
            .execute_perceived(&query.observation, perceived)?;
        let action_frame = action_schema.encode(
            format!("{}:committed-action", query.episode_id),
            &execution.result,
            vec![
                view.public_digest.clone(),
                selected.id.clone(),
                execution.execution_digest.clone(),
            ],
        )?;
        if action_frame.payload_digest != action_frames[index].payload_digest
            || execution.execution_digest != executions[index].execution_digest
        {
            bail!("agent-selected execution differs from its pre-action prediction");
        }
        let structure = IndexedStructure::infer(&query.observation)?;
        let substrate = certify_program_execution(
            &query.observation,
            &action_frame,
            &structure,
            &selected.program,
            &execution,
            &induction.proposal_kernel,
            &selected.id,
            view.demonstrations.len(),
            limits.trace_duration_horizon,
        )?;
        let commitment = ActionCommitment {
            episode_id: query.episode_id.clone(),
            action: action_frame,
            program_digest: digest_json(&selected.program)?,
            public_view_digest: view.public_digest.clone(),
        };
        let sequence = causal_events.len();
        causal_events.push(CausalEvent {
            sequence,
            phase: "headset_perceive".to_string(),
            episode_id: query.episode_id.clone(),
            input_digest: query.observation.payload_digest.clone(),
            executable_digest: execution.headset_digest.clone(),
            output_digest: execution.perception_digest.clone(),
            before_answer_reveal: true,
        });
        causal_events.push(CausalEvent {
            sequence: sequence + 1,
            phase: "agent_decide".to_string(),
            episode_id: query.episode_id.clone(),
            input_digest: execution.perception_digest.clone(),
            executable_digest: digest_json(&step)?,
            output_digest: selected.program.id.clone(),
            before_answer_reveal: true,
        });
        causal_events.push(CausalEvent {
            sequence: sequence + 2,
            phase: "program_execute_commit".to_string(),
            episode_id: query.episode_id.clone(),
            input_digest: selected.program.id.clone(),
            executable_digest: execution.execution_digest.clone(),
            output_digest: commitment.action.payload_digest.clone(),
            before_answer_reveal: true,
        });
        commitments.push(commitment.clone());
        query_actions.push(QueryAction {
            episode_id: query.episode_id.clone(),
            observation_digest: query.observation.payload_digest.clone(),
            execution,
            action: commitment,
            agent_step: step,
            substrate,
        });
    }

    // This is the only answer-vault access. All executable commitments exist.
    let evaluation = sealed.evaluate(commitments)?;
    let feedback_events = query_actions
        .iter()
        .enumerate()
        .map(|(index, action)| ObservedTransition {
            sequence_id: format!("feedback:{}", action.episode_id),
            ordinal: 0,
            state: action.observation_digest.clone(),
            action: action.execution.program_id.clone(),
            next_state: action.action.action.payload_digest.clone(),
            outcomes: BTreeMap::from([("exact".into(), i64::from(evaluation.cases[index].exact))]),
            terminal: true,
            action_cost: 1,
            evidence_digest: evaluation.feedback_digest.clone(),
        })
        .collect::<Vec<_>>();
    let feedback_revision =
        induction.apply_binary_feedback(evaluation.all_exact, &feedback_events)?;
    let revised_hypotheses = revised_hypotheses(&induction, &feedback_revision);
    let revised_selected = revised_hypotheses
        .iter()
        .find(|candidate| candidate.id == selected.id)
        .ok_or_else(|| anyhow::anyhow!("feedback revision lost selected hypothesis"))?;
    let feedback_label = if evaluation.all_exact {
        "feedback:success"
    } else {
        "feedback:failure"
    };
    let post_interface = build_interface(
        &view,
        &revised_hypotheses,
        revised_selected,
        &executions,
        &action_frames,
        feedback_label,
        1,
        limits.trace_duration_horizon,
    )?;
    causal_events.push(CausalEvent {
        sequence: causal_events.len(),
        phase: "feedback_revise".to_string(),
        episode_id: "interaction".to_string(),
        input_digest: evaluation.feedback_digest.clone(),
        executable_digest: feedback_revision.prior_kernel_digest.clone(),
        output_digest: feedback_revision.revision_digest.clone(),
        before_answer_reveal: false,
    });

    let memory_digest = persist_interaction_memory(
        &memory_path,
        prior_memory,
        &view,
        &query_actions,
        &evaluation,
        &feedback_revision,
        revised_selected,
        &induction,
        &post_interface.headset,
    )?;
    let pre_feedback_state_digest = digest_json(&(
        &pre_interface.headset,
        &pre_interface.recursive.complete_chain,
    ))?;
    let post_feedback_state_digest = digest_json(&(
        &post_interface.headset,
        &post_interface.recursive.complete_chain,
    ))?;
    let causal_graph = build_causal_graph(
        &view,
        &selected,
        &query_actions,
        &evaluation,
        &feedback_revision,
        &pre_interface.headset,
        &post_feedback_state_digest,
    )?;
    let architecture = CausalArchitectureCertificate {
        sealing_before_action: sealing_before_action.clone(),
        causal_graph,
        selected_public_replay: selected.replay.clone(),
        selected_leave_one_out: selected.leave_one_out.clone(),
        proposal_kernel_digest: induction.proposal_kernel.stable_digest()?,
        posterior_normalized: induction.posterior_normalized,
        detailed_balance: induction.detailed_balance,
        local_reversible_proposals: induction.local_reversible_proposals,
        agent: pre_interface.agent.validate()?,
        network: pre_interface.network_certificate.clone(),
        headset: pre_interface.headset_certificate.clone(),
        recursive_trace: pre_interface.recursive.certificate.clone(),
        substrate_executions: query_actions
            .iter()
            .map(|action| action.substrate.clone())
            .collect(),
        feedback_revision: feedback_revision.clone(),
        pre_feedback_state_digest,
        post_feedback_state_digest,
        commitments_preceded_reveal: evaluation.actions_committed_before_reveal,
    };
    let integration_complete = architecture.complete();
    let report = InteractionReport {
        schema: "soma.conscious-agent-interaction.v3".to_string(),
        public_view_digest: view.public_digest.clone(),
        sealing_before_action,
        induction,
        observation_structure,
        action_structure,
        observation_capability,
        action_capability,
        action_schema,
        pre_feedback_headset: pre_interface.headset,
        pre_feedback_headset_certificate: pre_interface.headset_certificate,
        agent_network_certificate: pre_interface.network_certificate,
        fusion_certificate: pre_interface.fusion_certificate,
        recursive_trace_dynamics: pre_interface.recursive,
        query_actions,
        causal_events,
        evaluation: evaluation.clone(),
        feedback_revision,
        post_feedback_headset: post_interface.headset,
        post_feedback_headset_certificate: post_interface.headset_certificate,
        memory_path,
        memory_digest,
        architecture,
        integration_complete,
        external_task_solved: evaluation.all_exact,
    };
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create report directory {}", parent.display()))?;
    }
    fs::write(report_path, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("cannot write interaction report {}", report_path.display()))?;
    Ok(report)
}

fn build_causal_graph(
    view: &LearningView,
    selected: &GenerativeHypothesis,
    actions: &[QueryAction],
    evaluation: &EvaluationReport,
    revision: &FeedbackRevision,
    pre_headset: &LearnedHeadset,
    post_state_digest: &str,
) -> Result<CausalProvenanceGraph> {
    if actions.len() != view.queries.len() || evaluation.cases.len() != actions.len() {
        bail!("causal graph requires aligned queries, actions, and feedback");
    }
    let hypothesis_node = "hypothesis:selected".to_string();
    let revision_node = "posterior:revised".to_string();
    let post_node = "headset:post-feedback".to_string();
    let mut nodes = vec![
        CausalNode {
            id: hypothesis_node.clone(),
            kind: "generative_hypothesis".to_string(),
            payload_digest: digest_json(selected)?,
            before_answer_reveal: true,
        },
        CausalNode {
            id: revision_node.clone(),
            kind: "posterior_revision".to_string(),
            payload_digest: revision.revision_digest.clone(),
            before_answer_reveal: false,
        },
        CausalNode {
            id: post_node.clone(),
            kind: "revised_headset".to_string(),
            payload_digest: post_state_digest.to_string(),
            before_answer_reveal: false,
        },
    ];
    let mut edges = Vec::new();
    let mut episode_paths = Vec::new();
    for (index, query) in view.queries.iter().enumerate() {
        let action = &actions[index];
        let case = &evaluation.cases[index];
        if query.episode_id != action.episode_id || query.episode_id != case.episode_id {
            bail!("causal graph episode alignment changed across the sealed boundary");
        }
        let prefix = format!("episode:{}", query.episode_id);
        let observation = format!("{prefix}:boundary-observation");
        let perception = format!("{prefix}:headset-perception");
        let experience = format!("{prefix}:experience");
        let execution = format!("{prefix}:program-execution");
        let world_action = format!("{prefix}:world-action");
        let commitment = format!("{prefix}:commitment");
        let feedback = format!("{prefix}:feedback");
        nodes.extend([
            CausalNode {
                id: observation.clone(),
                kind: "boundary_observation".to_string(),
                payload_digest: query.observation.payload_digest.clone(),
                before_answer_reveal: true,
            },
            CausalNode {
                id: perception.clone(),
                kind: "headset_perception".to_string(),
                payload_digest: action.execution.perception_digest.clone(),
                before_answer_reveal: true,
            },
            CausalNode {
                id: experience.clone(),
                kind: "primitive_experience".to_string(),
                payload_digest: action.agent_step.source_experience.clone(),
                before_answer_reveal: true,
            },
            CausalNode {
                id: execution.clone(),
                kind: "program_execution".to_string(),
                payload_digest: action.execution.execution_digest.clone(),
                before_answer_reveal: true,
            },
            CausalNode {
                id: world_action.clone(),
                kind: "world_action".to_string(),
                payload_digest: action.action.action.payload_digest.clone(),
                before_answer_reveal: true,
            },
            CausalNode {
                id: commitment.clone(),
                kind: "sealed_commitment".to_string(),
                payload_digest: digest_json(&action.action)?,
                before_answer_reveal: true,
            },
            CausalNode {
                id: feedback.clone(),
                kind: "feedback".to_string(),
                payload_digest: digest_json(case)?,
                before_answer_reveal: false,
            },
        ]);
        let substrate_digest = digest_json(&action.substrate)?;
        edges.extend([
            CausalEdge {
                from: observation.clone(),
                to: perception.clone(),
                mechanism_digest: action.execution.headset_digest.clone(),
            },
            CausalEdge {
                from: perception.clone(),
                to: experience.clone(),
                mechanism_digest: pre_headset.id.clone(),
            },
            CausalEdge {
                from: experience.clone(),
                to: hypothesis_node.clone(),
                mechanism_digest: action.agent_step.causal_digest.clone(),
            },
            CausalEdge {
                from: hypothesis_node.clone(),
                to: execution.clone(),
                mechanism_digest: selected.program.id.clone(),
            },
            CausalEdge {
                from: execution.clone(),
                to: world_action.clone(),
                mechanism_digest: substrate_digest,
            },
            CausalEdge {
                from: world_action.clone(),
                to: commitment.clone(),
                mechanism_digest: action.agent_step.causal_digest.clone(),
            },
            CausalEdge {
                from: commitment.clone(),
                to: feedback.clone(),
                mechanism_digest: evaluation.feedback_digest.clone(),
            },
            CausalEdge {
                from: feedback.clone(),
                to: revision_node.clone(),
                mechanism_digest: revision.prior_kernel_digest.clone(),
            },
        ]);
        episode_paths.push(EpisodeCausalPath {
            episode_id: query.episode_id.clone(),
            ordered_nodes: vec![
                observation,
                perception,
                experience,
                hypothesis_node.clone(),
                execution,
                world_action,
                commitment,
                feedback,
                revision_node.clone(),
                post_node.clone(),
            ],
        });
    }
    edges.push(CausalEdge {
        from: revision_node,
        to: post_node,
        mechanism_digest: revision.revision_digest.clone(),
    });
    let graph = CausalProvenanceGraph {
        nodes,
        edges,
        episode_paths,
    };
    graph.validate()?;
    Ok(graph)
}

fn infer_boundary_schemas(
    view: &LearningView,
) -> Result<(
    FiniteRelationalStructure,
    FiniteRelationalStructure,
    IndexedFrameSchema,
)> {
    let first = view
        .demonstrations
        .first()
        .ok_or_else(|| anyhow::anyhow!("interaction needs public demonstrations"))?;
    let observation = IndexedStructure::infer(&first.observation)?;
    let action = IndexedStructure::infer(&first.demonstrated_action)?;
    let observation_structure = FiniteRelationalStructure::infer(&first.observation)?;
    let action_structure = FiniteRelationalStructure::infer(&first.demonstrated_action)?;
    let action_schema =
        IndexedFrameSchema::infer(&first.demonstrated_action, &action, &view.protocol)?;
    for example in &view.demonstrations {
        let observed = IndexedStructure::infer(&example.observation)?;
        let demonstrated = IndexedStructure::infer(&example.demonstrated_action)?;
        let observed_structure = FiniteRelationalStructure::infer(&example.observation)?;
        let demonstrated_structure =
            FiniteRelationalStructure::infer(&example.demonstrated_action)?;
        if observed_structure.signature_digest != observation_structure.signature_digest
            || demonstrated_structure.signature_digest != action_structure.signature_digest
            || observed.schema != observation.schema
            || demonstrated.schema != action_schema.indexed
        {
            bail!("public demonstrations do not share one typed boundary schema");
        }
    }
    Ok((observation_structure, action_structure, action_schema))
}

#[allow(clippy::too_many_arguments)]
fn build_interface(
    view: &LearningView,
    hypotheses: &[GenerativeHypothesis],
    selected: &GenerativeHypothesis,
    executions: &[ProgramExecution],
    actions: &[RelationalFrame],
    feedback_label: &str,
    revision: u64,
    duration_horizon: usize,
) -> Result<InterfaceBundle> {
    if executions.len() != view.queries.len() || actions.len() != view.queries.len() {
        bail!("interface construction requires one execution and action per query");
    }
    let causal_headset_digest = selected.program.joint.headset.stable_digest()?;
    if executions
        .iter()
        .any(|execution| execution.headset_digest != causal_headset_digest)
    {
        bail!("interface certificate does not match the headset that caused perception");
    }
    let mut observed_states = Vec::new();
    let mut support = Vec::new();
    let mut observation_experiences = Vec::new();
    let mut commit_experiences = Vec::new();
    let mut feedback_experiences = Vec::new();
    let mut hidden_states = Vec::new();
    for (index, query) in view.queries.iter().enumerate() {
        let observe = format!("experience:observe:{}", query.observation.payload_digest);
        let hidden = format!("hidden:execute:{}", executions[index].execution_digest);
        let commit = format!("experience:commit:{}", actions[index].payload_digest);
        let feedback = format!("experience:{feedback_label}:{}", query.episode_id);
        observed_states.extend([
            observe.clone(),
            hidden.clone(),
            commit.clone(),
            feedback.clone(),
        ]);
        support.extend([observe.clone(), commit.clone(), feedback.clone()]);
        observation_experiences.push(observe);
        hidden_states.push(hidden);
        commit_experiences.push(commit);
        feedback_experiences.push(feedback);
    }
    let mut observed_transition = zeros(observed_states.len(), observed_states.len());
    for index in 0..view.queries.len() {
        let offset = 4 * index;
        observed_transition[offset][offset + 1] = Exact::one();
        observed_transition[offset + 1][offset + 2] = Exact::one();
        observed_transition[offset + 2][offset + 3] = Exact::one();
        observed_transition[offset + 3][4 * ((index + 1) % view.queries.len())] = Exact::one();
    }
    let mut initial = vec![Exact::zero(); observed_states.len()];
    initial[0] = Exact::one();
    let observed_process = MarkovChain {
        id: format!(
            "observed-interface-process:{}",
            digest_json(&(&observed_states, &observed_transition, revision))?
        ),
        initial,
        kernel: MarkovKernel::new(observed_states.clone(), observed_transition)?,
        provenance: vec![view.public_digest.clone()],
    };
    let observer_trace =
        TraceWitness::derive(&observed_process, support.clone(), duration_horizon)?;

    let mut world_states = Vec::new();
    let mut action_world_states = Vec::new();
    let mut choices = Vec::new();
    let mut perception_assignments = BTreeMap::new();
    let mut decision_assignments = BTreeMap::new();
    let mut action_assignments = BTreeMap::new();
    for (index, query) in view.queries.iter().enumerate() {
        let observation_world = format!("world:observation:{}", query.observation.payload_digest);
        let action_world = format!("world:action:{}", actions[index].payload_digest);
        let feedback_world = format!("world:{feedback_label}:{}", query.episode_id);
        let execute_choice = format!("choice:execute:{}:{}", selected.id, query.episode_id);
        let await_choice = format!("choice:await-feedback:{}", query.episode_id);
        let continue_choice = format!("choice:continue:{}", query.episode_id);
        world_states.extend([
            observation_world.clone(),
            action_world.clone(),
            feedback_world.clone(),
        ]);
        choices.extend([
            execute_choice.clone(),
            await_choice.clone(),
            continue_choice.clone(),
        ]);
        perception_assignments.insert(
            observation_world.clone(),
            observation_experiences[index].clone(),
        );
        perception_assignments.insert(action_world.clone(), commit_experiences[index].clone());
        perception_assignments.insert(feedback_world.clone(), feedback_experiences[index].clone());
        decision_assignments.insert(
            observation_experiences[index].clone(),
            execute_choice.clone(),
        );
        decision_assignments.insert(commit_experiences[index].clone(), await_choice.clone());
        decision_assignments.insert(feedback_experiences[index].clone(), continue_choice.clone());
        action_assignments.insert(execute_choice, action_world.clone());
        action_assignments.insert(await_choice, feedback_world);
        action_assignments.insert(
            continue_choice,
            format!(
                "world:observation:{}",
                view.queries[(index + 1) % view.queries.len()]
                    .observation
                    .payload_digest
            ),
        );
        action_world_states.push(action_world);
    }
    let world = FiniteMeasurableSpace::new("world:interaction", world_states.clone())?;
    let experience = FiniteMeasurableSpace::new("experience:interface", support.clone())?;
    let choice_space = FiniteMeasurableSpace::new("choice:program-actions", choices.clone())?;
    let perception = deterministic_map(
        world_states.clone(),
        support.clone(),
        &perception_assignments,
    )?;
    let decision = deterministic_map(support.clone(), choices.clone(), &decision_assignments)?;
    let action = deterministic_map(choices.clone(), world_states.clone(), &action_assignments)?;

    let program_digest = digest_json(&selected.program)?;
    let protocol_digest = digest_json(&view.protocol)?;
    let revision_digest = digest_json(&(feedback_label, revision))?;
    let operations = (0..view.queries.len())
        .flat_map(|index| {
            [
                LearnedOperation {
                    id: format!("operation:execute:{}", view.queries[index].episode_id),
                    input_type: "observation".to_string(),
                    output_type: "action".to_string(),
                    executable_digest: executions[index].execution_digest.clone(),
                    generators: vec![program_digest.clone()],
                    state_edges: vec![(
                        observation_experiences[index].clone(),
                        commit_experiences[index].clone(),
                    )],
                    provenance: vec![view.public_digest.clone()],
                },
                LearnedOperation {
                    id: format!("operation:feedback:{}", view.queries[index].episode_id),
                    input_type: "action".to_string(),
                    output_type: "feedback".to_string(),
                    executable_digest: protocol_digest.clone(),
                    generators: Vec::new(),
                    state_edges: vec![(
                        commit_experiences[index].clone(),
                        feedback_experiences[index].clone(),
                    )],
                    provenance: vec![view.public_digest.clone()],
                },
                LearnedOperation {
                    id: format!("operation:continue:{}", view.queries[index].episode_id),
                    input_type: "feedback".to_string(),
                    output_type: "observation".to_string(),
                    executable_digest: revision_digest.clone(),
                    generators: Vec::new(),
                    state_edges: vec![(
                        feedback_experiences[index].clone(),
                        observation_experiences[(index + 1) % view.queries.len()].clone(),
                    )],
                    provenance: vec![view.public_digest.clone()],
                },
            ]
        })
        .collect::<Vec<_>>();
    let action_algebra = LearnedActionAlgebra::derive(
        vec![
            "observation".to_string(),
            "action".to_string(),
            "feedback".to_string(),
        ],
        operations,
    )?;
    let headset = LearnedHeadset::create(
        perception.clone(),
        action.clone(),
        &observed_process,
        observer_trace.clone(),
        action_algebra,
        hypotheses,
        vec![view.public_digest.clone(), causal_headset_digest],
        revision,
        duration_horizon,
    )?;
    let headset_certificate = headset.validate()?;
    let agent = ConsciousAgent {
        id: format!("agent:learner:{}", headset.id),
        experience,
        choices: choice_space,
        world: world.clone(),
        perception,
        decision,
        action: action.clone(),
        experience_counter: revision,
        provenance: vec![view.public_digest.clone(), headset.id.clone()],
    };
    let joint_labels = AgentNetwork::canonical_joint_choices(std::slice::from_ref(&agent))?;
    let joint_action = StochasticMap::new(
        joint_labels,
        world.atoms.clone(),
        agent.action.transition.clone(),
    )?;
    let (network, _) = AgentNetwork::join(
        format!("network:interaction:{}", headset.id),
        vec![agent.clone()],
        joint_action,
        vec![view.public_digest.clone(), headset.id.clone()],
    )?;
    let network_certificate = network.validate()?;
    let fusion_certificate = network.fusion_certificate()?;
    if agent.qualia_kernel()? != observer_trace.trace.kernel {
        bail!("agent Q = DAP differs from the headset's observer trace");
    }
    let recursive = build_recursive_dynamics(
        &observed_process,
        &observer_trace,
        &headset,
        selected,
        &hidden_states,
        duration_horizon,
    )?;
    Ok(InterfaceBundle {
        headset,
        headset_certificate,
        agent,
        network_certificate,
        fusion_certificate,
        recursive,
        observation_experiences,
        action_world_states,
    })
}

fn build_recursive_dynamics(
    observed: &MarkovChain,
    trace: &TraceWitness,
    headset: &LearnedHeadset,
    selected: &GenerativeHypothesis,
    hidden_states: &[String],
    duration_horizon: usize,
) -> Result<RecursiveTraceDynamics> {
    let full_space = ExperienceModel {
        id: "space:full-computation".to_string(),
        chain: observed.clone(),
        description_digest: digest_json(&("full", &observed.kernel))?,
        provenance: observed.provenance.clone(),
    };
    let interface_space = ExperienceModel {
        id: "space:observer-interface".to_string(),
        chain: trace.trace.clone(),
        description_digest: digest_json(&("trace", &trace.trace.kernel))?,
        provenance: trace.trace.provenance.clone(),
    };
    let mut restrict_assignments = BTreeMap::new();
    for state in &observed.kernel.states {
        if trace.support.contains(state) {
            restrict_assignments.insert(state.clone(), state.clone());
        }
    }
    for hidden in hidden_states {
        let index = observed
            .kernel
            .index_of(hidden)
            .ok_or_else(|| anyhow::anyhow!("hidden program state is absent"))?;
        let destination = observed.kernel.transition[index]
            .iter()
            .enumerate()
            .find(|(_, probability)| probability.is_one())
            .map(|(to, _)| observed.kernel.states[to].clone())
            .ok_or_else(|| anyhow::anyhow!("hidden program state has no deterministic exit"))?;
        restrict_assignments.insert(hidden.clone(), destination);
    }
    let restriction = SpaceMorphism {
        id: "morphism:restrict-to-interface".to_string(),
        source_space: full_space.id.clone(),
        target_space: interface_space.id.clone(),
        kind: SpaceMorphismKind::Restriction,
        transport: deterministic_map(
            observed.kernel.states.clone(),
            trace.support.clone(),
            &restrict_assignments,
        )?,
        trace_witness: Some(trace.clone()),
        provenance: observed.provenance.clone(),
    };
    let expansion_assignments = trace
        .support
        .iter()
        .map(|state| (state.clone(), state.clone()))
        .collect::<BTreeMap<_, _>>();
    let expansion = SpaceMorphism {
        id: "morphism:expand-to-computation".to_string(),
        source_space: interface_space.id.clone(),
        target_space: full_space.id.clone(),
        kind: SpaceMorphismKind::Expansion,
        transport: deterministic_map(
            trace.support.clone(),
            observed.kernel.states.clone(),
            &expansion_assignments,
        )?,
        trace_witness: Some(trace.clone()),
        provenance: observed.provenance.clone(),
    };
    let base_states = vec!["restrict".to_string(), "expand".to_string()];
    let base_policy = PolicyLevel {
        id: "policy:state-space".to_string(),
        level: 0,
        states: base_states.clone(),
        kernel: MarkovKernel::new(
            base_states.clone(),
            vec![
                vec![Exact::zero(), Exact::one()],
                vec![Exact::one(), Exact::zero()],
            ],
        )?,
        controls: BTreeMap::from([
            ("restrict".to_string(), vec![restriction.id.clone()]),
            ("expand".to_string(), vec![expansion.id.clone()]),
        ]),
    };
    let meta_states = vec!["evidence-driven".to_string()];
    let meta_policy = PolicyLevel {
        id: "meta-policy:interface-revision".to_string(),
        level: 1,
        states: meta_states.clone(),
        kernel: MarkovKernel::identity(meta_states.clone())?,
        controls: BTreeMap::from([("evidence-driven".to_string(), base_states.clone())]),
    };
    let configurations = vec![
        RtlConfiguration {
            id: "configuration:full".to_string(),
            experience_space: full_space.id.clone(),
            policy_path: vec!["restrict".to_string(), "evidence-driven".to_string()],
            headset_id: headset.id.clone(),
            program_id: selected.program.id.clone(),
            memory_digest: selected.id.clone(),
            provenance: selected.evidence_digests.clone(),
        },
        RtlConfiguration {
            id: "configuration:interface".to_string(),
            experience_space: interface_space.id.clone(),
            policy_path: vec!["expand".to_string(), "evidence-driven".to_string()],
            headset_id: headset.id.clone(),
            program_id: selected.program.id.clone(),
            memory_digest: selected.id.clone(),
            provenance: selected.evidence_digests.clone(),
        },
    ];
    let configuration_kernel = MarkovKernel::new(
        configurations
            .iter()
            .map(|configuration| configuration.id.clone())
            .collect(),
        vec![
            vec![Exact::zero(), Exact::one()],
            vec![Exact::one(), Exact::zero()],
        ],
    )?;
    RecursiveTraceDynamics::compile(
        vec![full_space, interface_space],
        vec![restriction, expansion],
        PolicyHierarchy {
            levels: vec![base_policy, meta_policy],
        },
        configurations,
        configuration_kernel,
        vec![
            ConfigurationTransport {
                from_configuration: "configuration:full".to_string(),
                to_configuration: "configuration:interface".to_string(),
                morphism_id: "morphism:restrict-to-interface".to_string(),
            },
            ConfigurationTransport {
                from_configuration: "configuration:interface".to_string(),
                to_configuration: "configuration:full".to_string(),
                morphism_id: "morphism:expand-to-computation".to_string(),
            },
        ],
        "configuration:full",
        &trace.support[0],
        duration_horizon,
    )
}

fn revised_hypotheses(
    induction: &GenerativeInduction,
    revision: &FeedbackRevision,
) -> Vec<GenerativeHypothesis> {
    induction
        .hypotheses
        .iter()
        .map(|hypothesis| {
            let mut updated = hypothesis.clone();
            if let Some((_, probability)) = revision
                .revised_posterior
                .iter()
                .find(|(id, _)| id == &hypothesis.id)
            {
                updated.posterior = probability.clone();
            }
            updated.program.joint.latent_dynamics = revision.revised_world_model.clone();
            updated.program.joint.goal_hypotheses =
                revision.revised_world_model.goal_posterior.clone();
            updated.program.id = format!(
                "structural-program:{}",
                digest_json(&(
                    &updated.program.joint,
                    updated.program.maximum_iterations,
                    &updated.program.observation_schema_digest,
                    &updated.program.action_schema_digest,
                ))
                .unwrap_or_else(|_| revision.revision_digest.clone())
            );
            updated.latent_world_model.generative_semantics_digest =
                revision.revised_world_model.model_digest.clone();
            updated
        })
        .collect()
}

fn memory_path_for(report_path: &Path) -> PathBuf {
    report_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("soma-agent.memory.json")
}

#[allow(clippy::too_many_arguments)]
fn persist_interaction_memory(
    path: &Path,
    prior: Option<PersistentMemory>,
    view: &LearningView,
    actions: &[QueryAction],
    evaluation: &EvaluationReport,
    revision: &FeedbackRevision,
    selected: &GenerativeHypothesis,
    induction: &GenerativeInduction,
    post_headset: &LearnedHeadset,
) -> Result<String> {
    let mut memory = prior.unwrap_or(PersistentMemory::empty("soma-conscious-agent", 1)?);
    for action in actions {
        let base = memory.episodes.len();
        memory.episodes.extend([
            Episode {
                source_digest: view.source.boundary_identity_digest.clone(),
                sequence_id: format!("{}:{}", view.public_digest, action.episode_id),
                ordinal: base,
                experience: action.agent_step.source_experience.clone(),
                intervention: None,
                report_digest: action.observation_digest.clone(),
            },
            Episode {
                source_digest: view.source.boundary_identity_digest.clone(),
                sequence_id: format!("{}:{}", view.public_digest, action.episode_id),
                ordinal: base + 1,
                experience: action
                    .agent_step
                    .paths
                    .iter()
                    .find(|path| path.probability.is_one())
                    .map(|path| path.next_experience.clone())
                    .unwrap_or_else(|| "experience:unresolved".to_string()),
                intervention: Some(action.execution.program_id.clone()),
                report_digest: action.action.action.payload_digest.clone(),
            },
            Episode {
                source_digest: evaluation.source_digest.clone(),
                sequence_id: format!("{}:{}", view.public_digest, action.episode_id),
                ordinal: base + 2,
                experience: if evaluation.all_exact {
                    "feedback:success".to_string()
                } else {
                    "feedback:failure".to_string()
                },
                intervention: Some("posterior-revision".to_string()),
                report_digest: evaluation.feedback_digest.clone(),
            },
        ]);
    }
    memory.consolidate();
    memory.generative_procedures.insert(
        selected.program.id.clone(),
        serde_json::to_value(&selected.program)?,
    );
    for definition in &selected.program.joint.reusable_abstractions {
        definition.validate()?;
        memory
            .structural_macros
            .insert(definition.id.clone(), definition.clone());
    }
    memory.active_world_models.insert(
        revision.revised_world_model.model_digest.clone(),
        revision.revised_world_model.clone(),
    );
    for program in &revision.proposed_programs {
        memory
            .failed_structural_programs
            .insert(program.id.clone(), serde_json::to_value(program)?);
    }
    for hypothesis in &induction.hypotheses {
        let probability = revision
            .revised_posterior
            .iter()
            .find(|(id, _)| id == &hypothesis.id)
            .map(|(_, probability)| probability.clone())
            .ok_or_else(|| anyhow::anyhow!("persistent posterior lost a hypothesis"))?;
        memory
            .generative_posteriors
            .insert(hypothesis.program.id.clone(), probability);
    }
    memory.policy_states.insert(
        selected.policy_model.id.clone(),
        serde_json::to_value(&selected.policy_model)?,
    );
    memory
        .headset_states
        .insert(post_headset.id.clone(), serde_json::to_value(post_headset)?);
    memory.record_transition(
        "feedback revised generative posterior",
        &revision.revision_digest,
    )?;
    save_atomic(path, &memory)
}
