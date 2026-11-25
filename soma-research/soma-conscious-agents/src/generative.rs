use crate::active::{ActiveWorldModel, ObservedTransition};
use crate::exact::{sum, Exact};
use crate::kernel::{zeros, MarkovKernel};
use crate::memory::digest_json;
use crate::relational::{Atom, Database, Relation};
use crate::structure::{canonical_values, cartesian_indices, IndexedStructure};
use crate::universal::{
    description_bits, AggregateOperator, AggregateTerm, CarrierTerm, CoreTerm, CoreType,
    CorrespondenceHypothesis, EntityTraceMacro, HeadsetProgram, JointStructuralHypothesis,
    MacroDefinition, MorphismTerm, ProgramBinding, RoleHypothesis, StructuralPredicate,
    TypedCoreProgram, ValueTerm,
};
use crate::world::{LearningView, RelationalFrame};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GenerativeLimits {
    pub maximum_candidates: usize,
    /// Kept as a serialized resource field for compatibility. It bounds the
    /// number of structural models considered per expansion, not periodicity.
    pub maximum_period_models: usize,
    pub maximum_recursion_depth: usize,
}

impl GenerativeLimits {
    pub fn validate(&self) -> Result<()> {
        if self.maximum_candidates == 0
            || self.maximum_period_models == 0
            || self.maximum_recursion_depth == 0
        {
            bail!("generative resource limits must be positive");
        }
        Ok(())
    }

    fn structural_width(&self) -> usize {
        self.maximum_period_models
            .min(self.maximum_candidates.saturating_mul(8))
            .max(1)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GenerativeProgram {
    pub id: String,
    pub joint: JointStructuralHypothesis,
    pub maximum_iterations: usize,
    pub observation_schema_digest: String,
    pub action_schema_digest: String,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgramExecution {
    pub program_id: String,
    pub observation_digest: String,
    pub headset_digest: String,
    pub perception_digest: String,
    pub program_execution_digest: String,
    pub action_execution_digest: String,
    pub result: IndexedStructure,
    pub result_relations: Database,
    pub derived_cells: usize,
    pub unresolved_cells: usize,
    pub ambiguous_cells: usize,
    pub execution_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayScore {
    pub examples: usize,
    pub cells: usize,
    pub exact_cells: usize,
    pub wrong_cells: usize,
    pub unresolved_cells: usize,
    pub exact_examples: usize,
}

impl ReplayScore {
    pub fn all_exact(&self) -> bool {
        self.examples > 0
            && self.exact_examples == self.examples
            && self.wrong_cells == 0
            && self.unresolved_cells == 0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LatentWorldDescription {
    pub id: String,
    pub state_type: CoreType,
    pub transition_type: CoreType,
    pub generative_semantics_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeadsetDescription {
    pub id: String,
    pub executable_digest: String,
    pub observation_schema_digest: String,
    pub action_schema_digest: String,
    pub observer_semantics: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentDescription {
    pub id: String,
    pub qualia_composition: String,
    pub counter_update: String,
    pub network_join: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyDescription {
    pub id: String,
    pub policy_type: CoreType,
    pub action_selection: String,
    pub feedback_update: String,
    pub experiment_selection: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GenerativeHypothesis {
    pub id: String,
    pub program: GenerativeProgram,
    pub latent_world_model: LatentWorldDescription,
    pub headset_model: HeadsetDescription,
    pub agent_model: AgentDescription,
    pub policy_model: PolicyDescription,
    pub description_bits: usize,
    pub replay: ReplayScore,
    pub leave_one_out: ReplayScore,
    pub prior: Exact,
    pub likelihood: Exact,
    pub posterior: Exact,
    pub evidence_digests: Vec<String>,
    pub transferred_from_memory: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExperimentPriority {
    pub episode_id: String,
    pub executable_hypotheses: usize,
    pub distinct_predicted_actions: usize,
    pub posterior_disagreement: Exact,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GenerativeInduction {
    pub hypotheses: Vec<GenerativeHypothesis>,
    pub selected_hypothesis: String,
    pub proposal_kernel: MarkovKernel,
    pub posterior_normalized: bool,
    pub detailed_balance: bool,
    pub local_reversible_proposals: bool,
    pub experiment_priorities: Vec<ExperimentPriority>,
    pub trace_model: ActiveWorldModel,
    pub trace_predictive_likelihood: Exact,
    pub public_view_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeedbackRevision {
    pub selected_hypothesis: String,
    pub success: bool,
    pub prior_posterior: Vec<(String, Exact)>,
    pub revised_posterior: Vec<(String, Exact)>,
    pub prior_kernel_digest: String,
    pub revised_kernel: MarkovKernel,
    pub posterior_changed: bool,
    pub support_expanded: bool,
    pub proposed_programs: Vec<GenerativeProgram>,
    pub revised_world_model: ActiveWorldModel,
    pub revision_digest: String,
}

impl GenerativeProgram {
    pub fn create(
        joint: JointStructuralHypothesis,
        maximum_iterations: usize,
        observation_schema_digest: String,
        action_schema_digest: String,
        provenance: Vec<String>,
    ) -> Result<Self> {
        if maximum_iterations == 0 {
            bail!("program iteration bound must be positive")
        }
        let id = format!(
            "structural-program:{}",
            digest_json(&(
                &joint,
                maximum_iterations,
                observation_schema_digest.as_str(),
                action_schema_digest.as_str()
            ))?
        );
        Ok(Self {
            id,
            joint,
            maximum_iterations,
            observation_schema_digest,
            action_schema_digest,
            provenance,
        })
    }

    /// Causal execution: boundary -> candidate headset perception -> candidate
    /// program -> candidate action morphism. The selected program never sees a
    /// frame that has bypassed its own headset.
    pub fn execute(&self, observation: &RelationalFrame) -> Result<ProgramExecution> {
        let perceived = self.perceive(observation)?;
        self.execute_perceived(observation, perceived)
    }

    pub fn perceive(
        &self,
        observation: &RelationalFrame,
    ) -> Result<crate::universal::CoreExecution> {
        self.joint
            .headset
            .perceive(observation, self.maximum_iterations)
    }

    pub fn execute_perceived(
        &self,
        observation: &RelationalFrame,
        perceived: crate::universal::CoreExecution,
    ) -> Result<ProgramExecution> {
        let computed = self
            .joint
            .program
            .execute(&perceived.outputs, self.maximum_iterations)?;
        let emitted = self
            .joint
            .headset
            .emit(&computed.outputs, self.maximum_iterations)?;
        let result_frame = RelationalFrame::create(
            format!("{}:action", observation.id),
            emitted.outputs.clone(),
            self.provenance.clone(),
        )?;
        let result = IndexedStructure::infer(&result_frame)?;
        let input = IndexedStructure::infer(observation).ok();
        let mut derived_cells = result.cardinality();
        if let Some(input) = input {
            if input.shape() == result.shape() {
                derived_cells = cartesian_indices(&result.shape())
                    .into_iter()
                    .filter(|key| input.values.get(key) != result.values.get(key))
                    .count();
            }
        }
        let headset_digest = self.joint.headset.stable_digest()?;
        let execution_digest = digest_json(&(
            self.id.as_str(),
            observation.payload_digest.as_str(),
            &headset_digest,
            &perceived.execution_digest,
            &computed.execution_digest,
            &emitted.execution_digest,
            &result.domains,
            canonical_values(&result.values),
        ))?;
        Ok(ProgramExecution {
            program_id: self.id.clone(),
            observation_digest: observation.payload_digest.clone(),
            headset_digest,
            perception_digest: perceived.execution_digest,
            program_execution_digest: computed.execution_digest,
            action_execution_digest: emitted.execution_digest,
            result,
            result_relations: emitted.outputs,
            derived_cells,
            unresolved_cells: 0,
            ambiguous_cells: 0,
            execution_digest,
        })
    }

    pub fn prefix_description_bits(&self) -> Result<usize> {
        Ok(description_bits(&self.joint.headset.perception)?
            + description_bits(&self.joint.program)?
            + description_bits(&self.joint.headset.action)?
            + self
                .joint
                .reusable_abstractions
                .iter()
                .map(|item| description_bits(&item.body).unwrap_or(usize::MAX / 16))
                .sum::<usize>()
            + gamma_bits(self.maximum_iterations + 1))
    }
}

impl GenerativeInduction {
    pub fn selected(&self) -> Result<&GenerativeHypothesis> {
        self.hypotheses
            .iter()
            .find(|candidate| candidate.id == self.selected_hypothesis)
            .ok_or_else(|| anyhow::anyhow!("selected hypothesis is absent"))
    }

    pub fn apply_binary_feedback(
        &self,
        success: bool,
        observed: &[ObservedTransition],
    ) -> Result<FeedbackRevision> {
        let before = self
            .hypotheses
            .iter()
            .map(|item| (item.id.clone(), item.posterior.clone()))
            .collect::<Vec<_>>();
        let selected = self.selected()?;
        let mut weights = Vec::new();
        for candidate in &self.hypotheses {
            let factor = if candidate.id == selected.id {
                if success {
                    Exact::new(3, 2)?
                } else {
                    Exact::new(1, 4)?
                }
            } else if success {
                Exact::new(1, 2)?
            } else {
                Exact::one()
            };
            weights.push((candidate.id.clone(), candidate.posterior.mul(&factor)));
        }
        let total = sum(weights.iter().map(|item| item.1.clone()));
        let revised = weights
            .into_iter()
            .map(|(id, weight)| Ok((id, weight.div(&total)?)))
            .collect::<Result<Vec<_>>>()?;
        let proposed_programs = if success {
            Vec::new()
        } else {
            mutate_program(selected, self)?
        };
        let mut revised_world_model = self.trace_model.clone();
        for event in observed {
            revised_world_model = revised_world_model.update(event.clone())?;
        }
        let revised_hypotheses = self
            .hypotheses
            .iter()
            .map(|candidate| {
                let mut candidate = candidate.clone();
                candidate.posterior = revised
                    .iter()
                    .find(|item| item.0 == candidate.id)
                    .unwrap()
                    .1
                    .clone();
                candidate
            })
            .collect::<Vec<_>>();
        let revised_kernel = posterior_kernel(&revised_hypotheses)?;
        let revision_digest = digest_json(&(
            &self.selected_hypothesis,
            success,
            &before,
            &revised,
            &proposed_programs,
            &revised_world_model,
            &revised_kernel,
        ))?;
        Ok(FeedbackRevision {
            selected_hypothesis: self.selected_hypothesis.clone(),
            success,
            prior_posterior: before.clone(),
            revised_posterior: revised.clone(),
            prior_kernel_digest: self.proposal_kernel.stable_digest()?,
            revised_kernel,
            posterior_changed: before != revised,
            support_expanded: !proposed_programs.is_empty(),
            proposed_programs,
            revised_world_model,
            revision_digest,
        })
    }
}

pub fn induce(
    view: &LearningView,
    limits: &GenerativeLimits,
    observation_schema_digest: &str,
    action_schema_digest: &str,
    learned_programs: &[GenerativeProgram],
    learned_priors: &BTreeMap<String, Exact>,
) -> Result<GenerativeInduction> {
    limits.validate()?;
    if view.demonstrations.is_empty() {
        bail!("induction requires demonstrations")
    }
    let trace_events = one_step_traces(view)?;
    let trace_model = ActiveWorldModel::learn(&trace_events)?;
    let trace_predictive_likelihood = leave_one_trace_likelihood(&trace_events)?;
    let headsets = induce_headsets(view, limits)?;
    eprintln!("induction: {} headset hypotheses", headsets.len());
    let mut programs = Vec::new();
    for (headset_index, headset) in headsets.into_iter().enumerate() {
        let perceived = view
            .demonstrations
            .iter()
            .map(|example| {
                Ok((
                    headset
                        .perceive(&example.observation, limits.maximum_recursion_depth)?
                        .outputs,
                    semantic_action_database(&example.demonstrated_action)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        eprintln!(
            "induction: headset {headset_index} perceived {} demonstrations",
            perceived.len()
        );
        let synthesized = synthesize_programs(&perceived, limits, &view.public_digest)?;
        eprintln!(
            "induction: headset {headset_index} retained {} programs",
            synthesized.len()
        );
        let reusable_abstractions = discover_macros(&perceived, &synthesized, limits)?;
        for core in synthesized {
            let action = identity_for_schemas(
                core_output_schemas(&core, &perceived[0].0)?,
                action_output_schemas(&perceived[0].1),
                &view.public_digest,
            )?;
            let action_carriers = action_output_schemas(&perceived[0].1)
                .into_values()
                .map(|fields| CarrierTerm::Constructed { fields })
                .collect::<Vec<_>>();
            let correspondences =
                induce_correspondences(&headset, &action_carriers, &view.public_digest);
            let joint = JointStructuralHypothesis {
                headset: HeadsetProgram {
                    action_carriers,
                    action,
                    correspondences,
                    ..headset.clone()
                },
                program: core,
                latent_state_type: trace_state_type(&headset),
                transition_type: CoreType::Kernel {
                    source: Box::new(trace_state_type(&headset)),
                    target: Box::new(trace_state_type(&headset)),
                },
                goal_type: CoreType::Relation {
                    fields: vec!["outcome".into(), "utility".into()],
                },
                policy_type: CoreType::Kernel {
                    source: Box::new(trace_state_type(&headset)),
                    target: Box::new(CoreType::Structure {
                        relations: action_output_schemas(&perceived[0].1),
                    }),
                },
                latent_dynamics: trace_model.clone(),
                goal_hypotheses: trace_model.goal_posterior.clone(),
                reusable_abstractions: reusable_abstractions.clone(),
            };
            programs.push(GenerativeProgram::create(
                joint,
                limits.maximum_recursion_depth,
                observation_schema_digest.to_string(),
                action_schema_digest.to_string(),
                vec![view.public_digest.clone()],
            )?);
        }
    }
    programs.extend(
        learned_programs
            .iter()
            .filter(|program| {
                program.observation_schema_digest == observation_schema_digest
                    && program.action_schema_digest == action_schema_digest
            })
            .cloned(),
    );
    let mut unique = BTreeMap::new();
    for program in programs {
        unique.entry(program.id.clone()).or_insert(program);
    }
    let mut programs = unique.into_values().collect::<Vec<_>>();
    let demonstration_frames = view
        .demonstrations
        .iter()
        .map(|example| (&example.observation, &example.demonstrated_action))
        .collect::<Vec<_>>();
    let mut preliminary_replays = BTreeMap::new();
    for program in &programs {
        if let Ok(replay) = replay_frames(program, &demonstration_frames) {
            preliminary_replays.insert(program.id.clone(), replay);
        }
    }
    programs.retain(|program| preliminary_replays.contains_key(&program.id));
    programs.sort_by(|left, right| {
        let left_replay = &preliminary_replays[&left.id];
        let right_replay = &preliminary_replays[&right.id];
        left_replay
            .wrong_cells
            .saturating_add(left_replay.unresolved_cells)
            .cmp(
                &right_replay
                    .wrong_cells
                    .saturating_add(right_replay.unresolved_cells),
            )
            .then(right_replay.exact_examples.cmp(&left_replay.exact_examples))
            .then(
                left.prefix_description_bits()
                    .unwrap_or(usize::MAX)
                    .cmp(&right.prefix_description_bits().unwrap_or(usize::MAX)),
            )
            .then(left.id.cmp(&right.id))
    });
    programs.truncate(limits.maximum_candidates);
    eprintln!("induction: scoring {} joint programs", programs.len());

    let alphabet = view
        .demonstrations
        .iter()
        .flat_map(|example| {
            example
                .observation
                .relations
                .values()
                .chain(example.demonstrated_action.relations.values())
                .flat_map(|relation| relation.rows.iter().flatten().cloned())
        })
        .collect::<BTreeSet<_>>()
        .len()
        .max(2);
    let bits_per_error = usize::BITS as usize - (alphabet - 1).leading_zeros() as usize;
    let mut hypotheses = Vec::new();
    for program in programs {
        if view
            .queries
            .iter()
            .any(|query| program.execute(&query.observation).is_err())
        {
            continue;
        }
        let replay = preliminary_replays
            .remove(&program.id)
            .ok_or_else(|| anyhow::anyhow!("preliminary replay is absent"))?;
        if replay.cells == 0 {
            continue;
        }
        let leave_one_out = conservative_leave_one_out(&program, view)?;
        let description = program.prefix_description_bits()?;
        let code_prior = Exact::inverse_power_of_two(description);
        let prior = learned_priors
            .get(&program.id)
            .map(|old| code_prior.add(old).mul(&Exact::new(1, 2).unwrap()))
            .unwrap_or(code_prior);
        let predictive = if leave_one_out.examples > 0 {
            &leave_one_out
        } else {
            &replay
        };
        let error_bits = predictive
            .wrong_cells
            .saturating_add(predictive.unresolved_cells)
            .saturating_mul(bits_per_error.max(1));
        let likelihood = Exact::inverse_power_of_two(error_bits).mul(&trace_predictive_likelihood);
        let semantics = digest_json(&program.joint)?;
        let headset_digest = program.joint.headset.stable_digest()?;
        let id = format!(
            "joint-hypothesis:{}",
            digest_json(&(program.id.as_str(), &replay, &leave_one_out, &semantics))?
        );
        let transferred = program
            .provenance
            .iter()
            .any(|item| item != &view.public_digest);
        hypotheses.push(GenerativeHypothesis {
            id,
            latent_world_model: LatentWorldDescription {
                id: format!("latent-world:{semantics}"),
                state_type: program.joint.latent_state_type.clone(),
                transition_type: program.joint.transition_type.clone(),
                generative_semantics_digest: semantics,
            },
            headset_model: HeadsetDescription {
                id: format!("headset-program:{headset_digest}"),
                executable_digest: headset_digest,
                observation_schema_digest: observation_schema_digest.to_string(),
                action_schema_digest: action_schema_digest.to_string(),
                observer_semantics:
                    "candidate-relative-executable-perception-before-program-selection".into(),
            },
            agent_model: AgentDescription {
                id: "finite-conscious-agent.v2".into(),
                qualia_composition: "Q=DAP".into(),
                counter_update: "N:=N+1 after observed transition".into(),
                network_join: "checked-shared-world-product".into(),
            },
            policy_model: PolicyDescription {
                id: "posterior-trace-policy.v2".into(),
                policy_type: program.joint.policy_type.clone(),
                action_selection: "posterior-predictive-utility-information-cost".into(),
                feedback_update: "posterior-reweight-and-typed-local-support-expansion".into(),
                experiment_selection: "expected-information-gain-under-learned-transition".into(),
            },
            description_bits: description,
            replay,
            leave_one_out,
            prior: prior.clone(),
            likelihood: likelihood.clone(),
            posterior: prior.mul(&likelihood),
            evidence_digests: program.provenance.clone(),
            transferred_from_memory: transferred,
            program,
        });
    }
    eprintln!(
        "induction: retained {} executable joint hypotheses",
        hypotheses.len()
    );
    if hypotheses.is_empty() {
        bail!("typed structural grammar produced no executable hypothesis")
    }
    let mass = sum(hypotheses.iter().map(|item| item.posterior.clone()));
    if mass.is_zero() {
        bail!("joint posterior has zero mass")
    }
    for item in &mut hypotheses {
        item.posterior = item.posterior.div(&mass)?;
    }
    hypotheses.sort_by(hypothesis_order);
    let selected_hypothesis = hypotheses[0].id.clone();
    let proposal_kernel = posterior_kernel(&hypotheses)?;
    let experiment_priorities = rank_experiments(view, &hypotheses)?;
    let posterior_normalized = sum(hypotheses.iter().map(|item| item.posterior.clone())).is_one();
    let detailed_balance = detailed_balance(&hypotheses, &proposal_kernel);
    let local_reversible_proposals =
        proposal_kernel
            .transition
            .iter()
            .enumerate()
            .all(|(from, row)| {
                row.iter().enumerate().all(|(to, probability)| {
                    probability.is_zero()
                        || from == to
                        || joint_edit_distance(&hypotheses[from].program, &hypotheses[to].program)
                            <= 1
                })
            });
    Ok(GenerativeInduction {
        hypotheses,
        selected_hypothesis,
        proposal_kernel,
        posterior_normalized,
        detailed_balance,
        local_reversible_proposals,
        experiment_priorities,
        trace_model,
        trace_predictive_likelihood,
        public_view_digest: view.public_digest.clone(),
    })
}

fn one_step_traces(view: &LearningView) -> Result<Vec<ObservedTransition>> {
    let action_label = view
        .protocol
        .channels
        .iter()
        .find(|channel| channel.direction == crate::world::ChannelDirection::Action)
        .map(|channel| channel.id.clone())
        .ok_or_else(|| anyhow::anyhow!("world protocol has no action channel"))?;
    view.demonstrations
        .iter()
        .enumerate()
        .map(|(index, example)| {
            Ok(ObservedTransition {
                sequence_id: example.episode_id.clone(),
                ordinal: 0,
                state: example.observation.payload_digest.clone(),
                action: action_label.clone(),
                next_state: example.demonstrated_action.payload_digest.clone(),
                outcomes: BTreeMap::from([("demonstrated".into(), 1)]),
                terminal: true,
                action_cost: 1,
                evidence_digest: digest_json(&(
                    view.public_digest.as_str(),
                    index,
                    &example.observation,
                    &example.demonstrated_action,
                ))?,
            })
        })
        .collect()
}

fn leave_one_trace_likelihood(events: &[ObservedTransition]) -> Result<Exact> {
    if events.len() < 2 {
        return Ok(Exact::one());
    }
    let states = events
        .iter()
        .flat_map(|event| [event.state.clone(), event.next_state.clone()])
        .collect::<BTreeSet<_>>()
        .len()
        .max(1);
    let mut likelihood = Exact::one();
    for held_out in 0..events.len() {
        let training = events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| (index != held_out).then_some(event.clone()))
            .collect::<Vec<_>>();
        let model = ActiveWorldModel::learn(&training)?;
        let event = &events[held_out];
        let prediction = model.predict(&event.state, &event.action)?;
        let probability = prediction
            .next_state_distribution
            .get(&event.next_state)
            .cloned()
            .unwrap_or(Exact::new(1, states as i128)?);
        likelihood = likelihood.mul(&probability);
    }
    Ok(likelihood)
}

fn induce_headsets(view: &LearningView, limits: &GenerativeLimits) -> Result<Vec<HeadsetProgram>> {
    let first = &view.demonstrations[0].observation;
    let input_type = CoreType::Structure {
        relations: action_output_schemas(&first.relations),
    };
    let mut bindings = Vec::new();
    let mut outputs = first
        .relations
        .keys()
        .map(|name| {
            (
                name.clone(),
                CoreTerm::Source {
                    relation: name.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut roles = Vec::new();
    for (relation_name, relation) in &first.relations {
        for field in &relation.attributes {
            let role_name = format!("role:{}", digest_json(&(relation_name, field))?);
            let term_name = format!(
                "derived:{}",
                digest_json(&(relation_name, field, "quotient"))?
            );
            bindings.push(ProgramBinding {
                name: term_name.clone(),
                term: CoreTerm::Group {
                    input: Box::new(CoreTerm::Source {
                        relation: relation_name.clone(),
                    }),
                    keys: vec![field.clone()],
                    aggregates: vec![AggregateTerm {
                        output: "multiplicity".into(),
                        input: None,
                        operator: AggregateOperator::Count,
                    }],
                },
            });
            outputs.insert(
                term_name,
                CoreTerm::Source {
                    relation: bindings.last().unwrap().name.clone(),
                },
            );
            roles.push(RoleHypothesis {
                name: role_name,
                carrier: CarrierTerm::Quotient {
                    source: Box::new(CarrierTerm::Boundary {
                        relation: relation_name.clone(),
                    }),
                    keys: vec![field.clone()],
                },
                evidence: view
                    .demonstrations
                    .iter()
                    .map(|item| item.observation.payload_digest.clone())
                    .collect(),
            });
            let column = relation.column(field)?;
            for atom in relation
                .rows
                .iter()
                .map(|row| row[column].clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .take(2)
            {
                for (label, predicate) in [
                    (
                        "support",
                        StructuralPredicate::Equal {
                            left: ValueTerm::Field {
                                name: field.clone(),
                            },
                            right: ValueTerm::Literal {
                                value: atom.clone(),
                            },
                        },
                    ),
                    (
                        "complement",
                        StructuralPredicate::Not {
                            term: Box::new(StructuralPredicate::Equal {
                                left: ValueTerm::Field {
                                    name: field.clone(),
                                },
                                right: ValueTerm::Literal {
                                    value: atom.clone(),
                                },
                            }),
                        },
                    ),
                ] {
                    let name = format!(
                        "derived:{}",
                        digest_json(&(relation_name, field, &atom, label))?
                    );
                    bindings.push(ProgramBinding {
                        name: name.clone(),
                        term: CoreTerm::Select {
                            input: Box::new(CoreTerm::Source {
                                relation: relation_name.clone(),
                            }),
                            predicate: predicate.clone(),
                        },
                    });
                    outputs.insert(
                        name,
                        CoreTerm::Source {
                            relation: bindings.last().unwrap().name.clone(),
                        },
                    );
                    roles.push(RoleHypothesis {
                        name: format!(
                            "role:{}",
                            digest_json(&(relation_name, field, &atom, label))?
                        ),
                        carrier: CarrierTerm::Subobject {
                            source: Box::new(CarrierTerm::Boundary {
                                relation: relation_name.clone(),
                            }),
                            predicate,
                        },
                        evidence: view
                            .demonstrations
                            .iter()
                            .map(|item| item.observation.payload_digest.clone())
                            .collect(),
                    });
                }
            }
            if bindings.len() >= limits.structural_width() {
                break;
            }
        }
        for left in 0..relation.attributes.len() {
            for right in 0..relation.attributes.len() {
                if left == right
                    || bindings.len() >= limits.structural_width()
                    || relation.rows.len() > limits.structural_width()
                {
                    continue;
                }
                let from = relation.attributes[left].clone();
                let to = relation.attributes[right].clone();
                let name = format!(
                    "derived:{}",
                    digest_json(&(relation_name, &from, &to, "closure"))?
                );
                bindings.push(ProgramBinding {
                    name: name.clone(),
                    term: CoreTerm::TransitiveClosure {
                        input: Box::new(CoreTerm::Image {
                            input: Box::new(CoreTerm::Source {
                                relation: relation_name.clone(),
                            }),
                            morphism: MorphismTerm::Project {
                                fields: vec![from.clone(), to.clone()],
                            },
                        }),
                        from: from.clone(),
                        to: to.clone(),
                    },
                });
                outputs.insert(
                    name,
                    CoreTerm::Source {
                        relation: bindings.last().unwrap().name.clone(),
                    },
                );
                roles.push(RoleHypothesis {
                    name: format!(
                        "connectivity:{}",
                        digest_json(&(relation_name, &from, &to))?
                    ),
                    carrier: CarrierTerm::Partition {
                        source: Box::new(CarrierTerm::Boundary {
                            relation: relation_name.clone(),
                        }),
                        keys: vec![from, to],
                    },
                    evidence: view
                        .demonstrations
                        .iter()
                        .map(|item| item.observation.payload_digest.clone())
                        .collect(),
                });
            }
        }
        if bindings.len() >= limits.structural_width() {
            break;
        }
    }
    let relation_items = first.relations.iter().collect::<Vec<_>>();
    for (left_name, left) in &relation_items {
        for (right_name, right) in &relation_items {
            if left_name == right_name {
                continue;
            }
            let common = left
                .attributes
                .iter()
                .filter(|field| right.attributes.contains(*field))
                .cloned()
                .collect::<Vec<_>>();
            if common.is_empty() {
                continue;
            }
            roles.push(RoleHypothesis {
                name: format!("nested:{}", digest_json(&(left_name, right_name, &common))?),
                carrier: CarrierTerm::Subobject {
                    source: Box::new(CarrierTerm::Boundary {
                        relation: (*left_name).clone(),
                    }),
                    predicate: StructuralPredicate::InRelation {
                        relation: (*right_name).clone(),
                        fields: common
                            .iter()
                            .map(|field| (field.clone(), field.clone()))
                            .collect(),
                    },
                },
                evidence: view
                    .demonstrations
                    .iter()
                    .map(|item| item.observation.payload_digest.clone())
                    .collect(),
            });
        }
    }
    let perception = TypedCoreProgram {
        input_type: input_type.clone(),
        output_type: CoreType::Structure {
            relations: BTreeMap::new(),
        },
        bindings,
        outputs,
        provenance: vec![view.public_digest.clone()],
    };
    let identity = TypedCoreProgram::identity(first);
    let observation_carriers = first
        .relations
        .keys()
        .map(|relation| CarrierTerm::Boundary {
            relation: relation.clone(),
        })
        .collect::<Vec<_>>();
    let structural = HeadsetProgram {
        observation_carriers: observation_carriers.clone(),
        action_carriers: Vec::new(),
        perception,
        action: identity.clone(),
        roles,
        correspondences: Vec::new(),
        evidence_digest: view.public_digest.clone(),
    };
    let boundary = HeadsetProgram {
        observation_carriers,
        action_carriers: Vec::new(),
        perception: identity.clone(),
        action: identity,
        roles: Vec::new(),
        correspondences: Vec::new(),
        evidence_digest: view.public_digest.clone(),
    };
    Ok(vec![structural, boundary])
}

fn induce_correspondences(
    headset: &HeadsetProgram,
    actions: &[CarrierTerm],
    evidence: &str,
) -> Vec<CorrespondenceHypothesis> {
    let mut result = Vec::new();
    let source_schemas = match &headset.perception.input_type {
        CoreType::Structure { relations } => relations,
        _ => return result,
    };
    for observation in &headset.observation_carriers {
        for action in actions {
            let (CarrierTerm::Boundary { relation }, CarrierTerm::Constructed { fields: target }) =
                (observation, action)
            else {
                continue;
            };
            let Some(source) = source_schemas.get(relation) else {
                continue;
            };
            let span = if source == target {
                MorphismTerm::Identity
            } else if target.iter().all(|field| source.contains(field)) {
                MorphismTerm::Project {
                    fields: target.clone(),
                }
            } else if source.len() == target.len() {
                MorphismTerm::Derive {
                    fields: target
                        .iter()
                        .zip(source)
                        .map(|(to, from)| (to.clone(), ValueTerm::Field { name: from.clone() }))
                        .collect(),
                }
            } else {
                continue;
            };
            result.push(CorrespondenceHypothesis {
                observation: observation.clone(),
                action: action.clone(),
                span,
                evidence: vec![evidence.to_string()],
            });
        }
    }
    result
}

fn synthesize_programs(
    examples: &[(Database, Database)],
    limits: &GenerativeLimits,
    provenance: &str,
) -> Result<Vec<TypedCoreProgram>> {
    let output_schemas = action_output_schemas(&examples[0].1);
    if examples
        .iter()
        .any(|(_, target)| action_output_schemas(target) != output_schemas)
    {
        bail!("demonstrations disagree on action relation signatures");
    }
    let mut choices = BTreeMap::<String, Vec<CoreTerm>>::new();
    for (output_name, output_fields) in &output_schemas {
        eprintln!("induction: synthesizing relation {output_name}");
        let terms = enumerate_relation_terms(examples, output_name, output_fields, limits)?;
        if terms.is_empty() {
            bail!("no typed term can produce output relation {output_name}")
        }
        eprintln!(
            "induction: relation {output_name} retained {} observational classes",
            terms.len()
        );
        choices.insert(output_name.clone(), terms);
    }
    let input_type = CoreType::Structure {
        relations: action_output_schemas(&examples[0].0),
    };
    let output_type = CoreType::Structure {
        relations: output_schemas,
    };
    let mut beam = vec![(BTreeMap::<String, CoreTerm>::new(), 0usize)];
    for (name, terms) in choices {
        let scored_terms = terms
            .into_iter()
            .map(|term| {
                let error =
                    relation_term_error(&term, &name, examples, limits.maximum_recursion_depth);
                (term, error)
            })
            .collect::<Vec<_>>();
        let mut expanded = Vec::new();
        for (partial, partial_score) in &beam {
            for (term, term_score) in &scored_terms {
                let mut output = partial.clone();
                output.insert(name.clone(), term.clone());
                let score = partial_score.saturating_add(*term_score);
                expanded.push((output, score));
            }
        }
        expanded.sort_by_key(|(output, error)| (*error, digest_json(output).unwrap_or_default()));
        expanded.dedup_by(|left, right| left.0 == right.0);
        expanded.truncate(limits.maximum_candidates);
        beam = expanded;
    }
    let programs = beam
        .into_iter()
        .map(|(outputs, _)| TypedCoreProgram {
            input_type: input_type.clone(),
            output_type: output_type.clone(),
            bindings: Vec::new(),
            outputs,
            provenance: vec![provenance.to_string()],
        })
        .collect::<Vec<_>>();
    let mut scored = programs
        .into_iter()
        .map(|program| {
            let key = (
                program_error(&program, examples, limits.maximum_recursion_depth),
                description_bits(&program).unwrap_or(usize::MAX),
                program.stable_digest().unwrap_or_default(),
            );
            (key, program)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| left.0.cmp(&right.0));
    scored.dedup_by(|left, right| left.1 == right.1);
    Ok(scored
        .into_iter()
        .take(limits.maximum_candidates)
        .map(|(_, program)| program)
        .collect())
}

fn semantic_action_database(frame: &RelationalFrame) -> Result<Database> {
    let structure = IndexedStructure::infer(frame)?;
    let relation = frame
        .relations
        .get(&structure.schema.data_relation)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("inferred action data relation is absent"))?;
    Ok(Database::from([(structure.schema.data_relation, relation)]))
}

fn enumerate_relation_terms(
    examples: &[(Database, Database)],
    output_name: &str,
    output_fields: &[String],
    limits: &GenerativeLimits,
) -> Result<Vec<CoreTerm>> {
    let schemas = action_output_schemas(&examples[0].0);
    let mut terms = Vec::new();
    terms.extend(induce_entity_trace_terms(
        examples,
        output_name,
        output_fields,
    )?);
    let primitive_budget = limits.structural_width();
    for (source_name, source_fields) in &schemas {
        if source_fields == output_fields {
            terms.push(CoreTerm::Source {
                relation: source_name.clone(),
            });
        }
        if source_fields.len() == output_fields.len() {
            terms.push(CoreTerm::Image {
                input: Box::new(CoreTerm::Source {
                    relation: source_name.clone(),
                }),
                morphism: MorphismTerm::Derive {
                    fields: output_fields
                        .iter()
                        .zip(source_fields)
                        .map(|(target, source)| {
                            (
                                target.clone(),
                                ValueTerm::Field {
                                    name: source.clone(),
                                },
                            )
                        })
                        .collect(),
                },
            });
            for offset in common_integer_offsets(
                examples,
                source_name,
                output_name,
                source_fields,
                output_fields,
            ) {
                terms.push(CoreTerm::Image {
                    input: Box::new(CoreTerm::Source {
                        relation: source_name.clone(),
                    }),
                    morphism: MorphismTerm::Derive {
                        fields: output_fields
                            .iter()
                            .zip(source_fields)
                            .zip(offset)
                            .map(|((target, source), delta)| {
                                let base = ValueTerm::Field {
                                    name: source.clone(),
                                };
                                let expression = if delta == 0 {
                                    base
                                } else if delta > 0 {
                                    ValueTerm::Add {
                                        left: Box::new(base),
                                        right: Box::new(ValueTerm::Literal {
                                            value: Atom::Integer(delta),
                                        }),
                                    }
                                } else {
                                    ValueTerm::Subtract {
                                        left: Box::new(base),
                                        right: Box::new(ValueTerm::Literal {
                                            value: Atom::Integer(-delta),
                                        }),
                                    }
                                };
                                (target.clone(), expression)
                            })
                            .collect(),
                    },
                });
            }
        }
        if output_fields
            .iter()
            .all(|field| source_fields.contains(field))
        {
            terms.push(CoreTerm::Image {
                input: Box::new(CoreTerm::Source {
                    relation: source_name.clone(),
                }),
                morphism: MorphismTerm::Project {
                    fields: output_fields.to_vec(),
                },
            });
        }
        for (field, atom) in common_atoms(examples, source_name)
            .into_iter()
            .take(4.min(primitive_budget.saturating_sub(terms.len())))
        {
            let selected = CoreTerm::Select {
                input: Box::new(CoreTerm::Source {
                    relation: source_name.clone(),
                }),
                predicate: StructuralPredicate::Equal {
                    left: ValueTerm::Field { name: field },
                    right: ValueTerm::Literal { value: atom },
                },
            };
            if source_fields == output_fields {
                terms.push(selected.clone());
            }
            if output_fields
                .iter()
                .all(|name| source_fields.contains(name))
            {
                terms.push(CoreTerm::Image {
                    input: Box::new(selected),
                    morphism: MorphismTerm::Project {
                        fields: output_fields.to_vec(),
                    },
                });
            }
        }
        if source_fields.len() == 2 && source_fields == output_fields {
            terms.push(CoreTerm::TransitiveClosure {
                input: Box::new(CoreTerm::Source {
                    relation: source_name.clone(),
                }),
                from: source_fields[0].clone(),
                to: source_fields[1].clone(),
            });
        }
        if source_fields.contains(&"value".to_string())
            && output_fields.contains(&"value".to_string())
        {
            let keys = output_fields
                .iter()
                .filter(|field| field.as_str() != "value" && source_fields.contains(*field))
                .cloned()
                .collect::<Vec<_>>();
            if !keys.is_empty() && keys.len() + 1 == output_fields.len() {
                terms.push(CoreTerm::Fold {
                    input: Box::new(CoreTerm::Source {
                        relation: source_name.clone(),
                    }),
                    keys,
                    value: "value".into(),
                    consensus: true,
                });
            }
        }
        if source_fields == output_fields && output_fields.len() >= 2 {
            let value = output_fields.last().cloned().unwrap();
            let keys = output_fields[..output_fields.len() - 1].to_vec();
            terms.push(CoreTerm::QuotientRepair {
                input: Box::new(CoreTerm::Source {
                    relation: source_name.clone(),
                }),
                keys: keys.clone(),
                value: value.clone(),
            });
            for unavailable in common_field_atoms(examples, source_name, &value) {
                let term = CoreTerm::IsomorphicCompletion {
                    input: Box::new(CoreTerm::Source {
                        relation: source_name.clone(),
                    }),
                    keys: keys.clone(),
                    value: value.clone(),
                    unavailable,
                };
                if examples.iter().all(|(source, _)| {
                    TypedCoreProgram {
                        input_type: CoreType::Structure {
                            relations: action_output_schemas(source),
                        },
                        output_type: CoreType::Relation {
                            fields: output_fields.to_vec(),
                        },
                        bindings: Vec::new(),
                        outputs: BTreeMap::from([(output_name.to_string(), term.clone())]),
                        provenance: Vec::new(),
                    }
                    .execute(source, limits.maximum_recursion_depth)
                    .is_ok()
                }) {
                    terms.push(term);
                }
            }
        }
    }
    let schema_items = schemas.iter().collect::<Vec<_>>();
    'pairs: for (left_name, left_fields) in &schema_items {
        for (right_name, right_fields) in &schema_items {
            if terms.len() >= primitive_budget {
                break 'pairs;
            }
            if left_name >= right_name {
                continue;
            }
            if left_fields == right_fields && left_fields.as_slice() == output_fields {
                terms.push(CoreTerm::Union {
                    left: Box::new(CoreTerm::Source {
                        relation: (*left_name).clone(),
                    }),
                    right: Box::new(CoreTerm::Source {
                        relation: (*right_name).clone(),
                    }),
                });
                terms.push(CoreTerm::Difference {
                    left: Box::new(CoreTerm::Source {
                        relation: (*left_name).clone(),
                    }),
                    right: Box::new(CoreTerm::Source {
                        relation: (*right_name).clone(),
                    }),
                });
            }
            if left_fields.len() + right_fields.len() == output_fields.len() {
                let joined = left_fields
                    .iter()
                    .chain(right_fields.iter())
                    .cloned()
                    .collect::<Vec<_>>();
                if joined == output_fields {
                    terms.push(CoreTerm::Join {
                        left: Box::new(CoreTerm::Source {
                            relation: (*left_name).clone(),
                        }),
                        right: Box::new(CoreTerm::Source {
                            relation: (*right_name).clone(),
                        }),
                        on: Vec::new(),
                    });
                }
            }
        }
    }
    let targets = examples
        .iter()
        .map(|(_, action)| action.get(output_name))
        .collect::<Option<Vec<_>>>();
    if let Some(targets) = targets {
        if targets.windows(2).all(|pair| pair[0] == pair[1]) {
            terms.push(CoreTerm::Literal {
                relation: targets[0].clone(),
            });
        }
    }
    let schema_map = action_output_schemas(&examples[0].0);
    let mut best_error = usize::MAX;
    let mut stagnant = 0usize;
    for depth in 1..=limits.maximum_recursion_depth {
        terms.retain(|term| {
            crate::universal::infer_schema(term, &schema_map, &BTreeMap::new()).is_ok()
        });
        let mut unique = BTreeMap::new();
        for term in std::mem::take(&mut terms) {
            unique
                .entry(digest_json(&term).unwrap_or_default())
                .or_insert(term);
        }
        let mut observational_classes = BTreeMap::<Vec<String>, (usize, usize, CoreTerm)>::new();
        for term in unique.into_values() {
            let (error, signature) =
                relation_term_profile(&term, output_name, examples, limits.maximum_recursion_depth);
            let syntax_bytes = serde_json::to_vec(&term)
                .map(|bytes| bytes.len())
                .unwrap_or(usize::MAX);
            let candidate = (error, syntax_bytes, term);
            match observational_classes.get_mut(&signature) {
                Some(previous) if (candidate.0, candidate.1) < (previous.0, previous.1) => {
                    *previous = candidate;
                }
                None => {
                    observational_classes.insert(signature, candidate);
                }
                _ => {}
            }
        }
        let mut scored = observational_classes
            .into_values()
            .map(|(error, _, term)| (error, term))
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            left.0.cmp(&right.0).then_with(|| {
                digest_json(&left.1)
                    .unwrap_or_default()
                    .cmp(&digest_json(&right.1).unwrap_or_default())
            })
        });
        let current = scored.first().map(|item| item.0).unwrap_or(usize::MAX);
        terms = scored
            .into_iter()
            .take(limits.maximum_candidates.min(limits.structural_width()))
            .map(|(_, term)| term)
            .collect();
        if current == 0 || depth == limits.maximum_recursion_depth {
            break;
        }
        if current < best_error {
            best_error = current;
            stagnant = 0;
        } else {
            stagnant += 1;
        }
        if stagnant >= 1 {
            break;
        }
        let frontier = terms
            .iter()
            .filter(|term| !contains_terminal_inference(term))
            .cloned()
            .collect::<Vec<_>>();
        let mut expanded = Vec::new();
        for term in &frontier {
            for other in frontier.iter().take(limits.structural_width().min(8)) {
                if term == other {
                    continue;
                }
                expanded.push(CoreTerm::Union {
                    left: Box::new(term.clone()),
                    right: Box::new(other.clone()),
                });
                expanded.push(CoreTerm::Difference {
                    left: Box::new(term.clone()),
                    right: Box::new(other.clone()),
                });
                if !output_fields.is_empty() {
                    expanded.push(CoreTerm::Overlay {
                        base: Box::new(term.clone()),
                        layer: Box::new(other.clone()),
                        keys: vec![output_fields[0].clone()],
                        prefer_layer: true,
                    });
                }
            }
        }
        terms.extend(expanded);
    }
    Ok(terms)
}

fn identity_for_schemas(
    input: BTreeMap<String, Vec<String>>,
    output: BTreeMap<String, Vec<String>>,
    provenance: &str,
) -> Result<TypedCoreProgram> {
    if input != output {
        bail!("headset action boundary differs from program output")
    }
    Ok(TypedCoreProgram {
        input_type: CoreType::Structure {
            relations: input.clone(),
        },
        output_type: CoreType::Structure {
            relations: output.clone(),
        },
        bindings: Vec::new(),
        outputs: output
            .keys()
            .map(|name| {
                (
                    name.clone(),
                    CoreTerm::Source {
                        relation: name.clone(),
                    },
                )
            })
            .collect(),
        provenance: vec![provenance.to_string()],
    })
}

fn action_output_schemas(database: &Database) -> BTreeMap<String, Vec<String>> {
    database
        .iter()
        .map(|(name, relation)| (name.clone(), relation.attributes.clone()))
        .collect()
}

fn core_output_schemas(
    program: &TypedCoreProgram,
    source: &Database,
) -> Result<BTreeMap<String, Vec<String>>> {
    Ok(program
        .execute(source, 1)?
        .outputs
        .iter()
        .map(|(name, relation)| (name.clone(), relation.attributes.clone()))
        .collect())
}

fn trace_state_type(headset: &HeadsetProgram) -> CoreType {
    CoreType::Trace {
        state: Box::new(headset.perception.output_type.clone()),
        action: Box::new(headset.action.output_type.clone()),
        observation: Box::new(headset.perception.input_type.clone()),
    }
}

fn relation_term_error(
    term: &CoreTerm,
    output_name: &str,
    examples: &[(Database, Database)],
    iterations: usize,
) -> usize {
    relation_term_profile(term, output_name, examples, iterations).0
}

fn relation_term_profile(
    term: &CoreTerm,
    output_name: &str,
    examples: &[(Database, Database)],
    iterations: usize,
) -> (usize, Vec<String>) {
    let mut error = 0usize;
    let mut signature = Vec::new();
    for (source, target) in examples {
        let expected = target.get(output_name);
        let program = TypedCoreProgram {
            input_type: CoreType::Structure {
                relations: action_output_schemas(source),
            },
            output_type: CoreType::Relation {
                fields: expected
                    .map(|item| item.attributes.clone())
                    .unwrap_or_default(),
            },
            bindings: Vec::new(),
            outputs: BTreeMap::from([(output_name.to_string(), term.clone())]),
            provenance: Vec::new(),
        };
        match (program.execute(source, iterations), expected) {
            (Ok(execution), Some(expected)) => {
                let output = execution.outputs.get(output_name).unwrap();
                error = error.saturating_add(relation_distance(output, expected));
                signature
                    .push(digest_json(output).unwrap_or_else(|_| "serialization-error".into()));
            }
            (_, Some(expected)) => {
                error = error.saturating_add(expected.rows.len().saturating_add(1_000));
                signature.push("execution-error".into());
            }
            _ => {
                error = error.saturating_add(1_000);
                signature.push("missing-target".into());
            }
        }
    }
    (error, signature)
}

fn program_error(
    program: &TypedCoreProgram,
    examples: &[(Database, Database)],
    iterations: usize,
) -> usize {
    examples
        .iter()
        .map(
            |(source, target)| match program.execute(source, iterations) {
                Ok(execution) => {
                    let integrity_penalty = RelationalFrame::create(
                        "candidate-action",
                        execution.outputs.clone(),
                        Vec::new(),
                    )
                    .and_then(|frame| IndexedStructure::infer(&frame))
                    .map(|_| 0)
                    .unwrap_or(100_000);
                    database_distance(&execution.outputs, target).saturating_add(integrity_penalty)
                }
                Err(_) => target
                    .values()
                    .map(|relation| relation.rows.len())
                    .sum::<usize>()
                    .saturating_add(10_000),
            },
        )
        .sum()
}

fn relation_distance(left: &Relation, right: &Relation) -> usize {
    if left.attributes != right.attributes {
        return left
            .rows
            .len()
            .saturating_add(right.rows.len())
            .saturating_add(1_000);
    }
    left.rows.symmetric_difference(&right.rows).count()
}

fn database_distance(left: &Database, right: &Database) -> usize {
    let names = left.keys().chain(right.keys()).collect::<BTreeSet<_>>();
    names
        .into_iter()
        .map(|name| match (left.get(name), right.get(name)) {
            (Some(a), Some(b)) => relation_distance(a, b),
            (Some(a), None) => a.rows.len().saturating_add(1_000),
            (None, Some(b)) => b.rows.len().saturating_add(1_000),
            _ => 0,
        })
        .sum()
}

fn common_atoms(examples: &[(Database, Database)], relation_name: &str) -> Vec<(String, Atom)> {
    let mut common: Option<BTreeSet<(String, Atom)>> = None;
    for (source, _) in examples {
        let Some(relation) = source.get(relation_name) else {
            return Vec::new();
        };
        let current = relation
            .attributes
            .iter()
            .enumerate()
            .flat_map(|(column, field)| {
                relation
                    .rows
                    .iter()
                    .map(move |row| (field.clone(), row[column].clone()))
            })
            .collect::<BTreeSet<_>>();
        common = Some(match common {
            None => current,
            Some(previous) => previous.intersection(&current).cloned().collect(),
        });
    }
    common.unwrap_or_default().into_iter().collect()
}

fn common_field_atoms(
    examples: &[(Database, Database)],
    relation_name: &str,
    field: &str,
) -> Vec<Atom> {
    let mut common: Option<BTreeSet<Atom>> = None;
    for (source, _) in examples {
        let Some(relation) = source.get(relation_name) else {
            return Vec::new();
        };
        let Ok(column) = relation.column(field) else {
            return Vec::new();
        };
        let current = relation
            .rows
            .iter()
            .map(|row| row[column].clone())
            .collect::<BTreeSet<_>>();
        common = Some(match common {
            None => current,
            Some(previous) => previous.intersection(&current).cloned().collect(),
        });
    }
    common.unwrap_or_default().into_iter().collect()
}

#[derive(Clone)]
struct TraceTrainingCase {
    sequence: Vec<(Atom, Vec<Vec<usize>>)>,
    start: Vec<usize>,
    background: Atom,
    shape: Vec<usize>,
    target: BTreeMap<Vec<usize>, Atom>,
}

fn induce_entity_trace_terms(
    examples: &[(Database, Database)],
    output_name: &str,
    output_fields: &[String],
) -> Result<Vec<CoreTerm>> {
    if output_fields.len() < 2 {
        return Ok(Vec::new());
    }
    let value = output_fields.last().cloned().unwrap();
    let keys = output_fields[..output_fields.len() - 1].to_vec();
    let rank = keys.len();
    if rank == 0 || rank > 4 {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for source_name in action_output_schemas(&examples[0].0)
        .into_iter()
        .filter_map(|(name, fields)| (fields == output_fields).then_some(name))
    {
        for split_axis in 0..rank {
            for canvas_high in [false, true] {
                let extracted = examples
                    .iter()
                    .map(|(source, target)| {
                        extract_trace_training(
                            source.get(&source_name)?,
                            target.get(output_name)?,
                            &keys,
                            &value,
                            split_axis,
                            canvas_high,
                        )
                        .ok()
                    })
                    .collect::<Option<Vec<_>>>();
                let Some(unordered) = extracted else {
                    continue;
                };
                for order in axis_permutations(rank) {
                    let cases = unordered
                        .iter()
                        .map(|case| ordered_trace_case(case, &order))
                        .collect::<Vec<_>>();
                    let assignments = solve_trace_macros(&cases, 16);
                    for macros in assignments {
                        let term = CoreTerm::EntityTrace {
                            input: Box::new(CoreTerm::Source {
                                relation: source_name.clone(),
                            }),
                            keys: keys.clone(),
                            value: value.clone(),
                            split_axis,
                            canvas_high,
                            order: order.clone(),
                            macros,
                        };
                        if examples.iter().all(|(source, target)| {
                            let program = TypedCoreProgram {
                                input_type: CoreType::Structure {
                                    relations: action_output_schemas(source),
                                },
                                output_type: CoreType::Relation {
                                    fields: output_fields.to_vec(),
                                },
                                bindings: Vec::new(),
                                outputs: BTreeMap::from([(output_name.to_string(), term.clone())]),
                                provenance: Vec::new(),
                            };
                            program
                                .execute(source, 1)
                                .ok()
                                .and_then(|execution| execution.outputs.get(output_name).cloned())
                                == target.get(output_name).cloned()
                        }) {
                            result.push(term);
                        }
                    }
                }
            }
        }
    }
    result.sort_by_key(|term| digest_json(term).unwrap_or_default());
    result.dedup();
    Ok(result)
}

fn extract_trace_training(
    source: &Relation,
    target: &Relation,
    keys: &[String],
    value: &str,
    split_axis: usize,
    canvas_high: bool,
) -> Result<TraceTrainingCase> {
    let (source_domains, source_values) = crate::universal::indexed_relation(source, keys, value)?;
    let (target_domains, target_values) = crate::universal::indexed_relation(target, keys, value)?;
    let extent = source_domains[split_axis].len();
    let mut matches = Vec::new();
    for separator in 1..extent.saturating_sub(1) {
        let plane = source_values
            .iter()
            .filter(|(key, _)| key[split_axis] == separator)
            .map(|(_, atom)| atom)
            .collect::<BTreeSet<_>>();
        if plane.len() != 1 {
            continue;
        }
        let (source_interval, canvas_interval) = if canvas_high {
            ((0, separator), (separator + 1, extent))
        } else {
            ((separator + 1, extent), (0, separator))
        };
        let mut canvas_shape = source_domains.iter().map(Vec::len).collect::<Vec<_>>();
        canvas_shape[split_axis] = canvas_interval.1 - canvas_interval.0;
        if canvas_shape != target_domains.iter().map(Vec::len).collect::<Vec<_>>() {
            continue;
        }
        let canvas_atoms = source_values
            .iter()
            .filter(|(key, _)| {
                key[split_axis] >= canvas_interval.0 && key[split_axis] < canvas_interval.1
            })
            .map(|(_, atom)| atom.clone())
            .collect::<Vec<_>>();
        let background = crate::universal::mode(&canvas_atoms)?;
        let markers = source_values
            .iter()
            .filter(|(key, atom)| {
                key[split_axis] >= canvas_interval.0
                    && key[split_axis] < canvas_interval.1
                    && **atom != background
            })
            .map(|(key, atom)| {
                let mut local = key.clone();
                local[split_axis] -= canvas_interval.0;
                (local, atom.clone())
            })
            .collect::<Vec<_>>();
        if markers.len() != 1 || target_values.get(&markers[0].0) != Some(&markers[0].1) {
            continue;
        }
        let source_map = source_values
            .iter()
            .filter(|(key, _)| {
                key[split_axis] >= source_interval.0 && key[split_axis] < source_interval.1
            })
            .map(|(key, atom)| (key.clone(), atom.clone()))
            .collect::<BTreeMap<_, _>>();
        let source_background =
            crate::universal::mode(&source_map.values().cloned().collect::<Vec<_>>())?;
        let entities = crate::universal::connected_entities(&source_map, &source_background);
        if entities.is_empty() {
            continue;
        }
        let sequence = entities
            .into_iter()
            .map(|support| {
                (
                    source_map[&support[0]].clone(),
                    crate::universal::normalize_support(&support),
                    support,
                )
            })
            .collect::<Vec<_>>();
        matches.push((sequence, markers[0].clone(), background));
    }
    if matches.len() != 1 {
        bail!("trace training requires one compatible separator")
    }
    let (sequence, (start, _marker), background) = matches.remove(0);
    Ok(TraceTrainingCase {
        sequence: sequence
            .into_iter()
            .map(|(atom, normalized, support)| {
                let mut encoded = normalized;
                encoded.push(
                    (0..support[0].len())
                        .map(|axis| support.iter().map(|key| key[axis]).min().unwrap())
                        .collect(),
                );
                (atom, encoded)
            })
            .collect(),
        start,
        background,
        shape: target_domains.iter().map(Vec::len).collect(),
        target: target_values,
    })
}

fn ordered_trace_case(case: &TraceTrainingCase, order: &[usize]) -> TraceTrainingCase {
    let mut result = case.clone();
    result.sequence.sort_by(|left, right| {
        let left_origin = left.1.last().unwrap();
        let right_origin = right.1.last().unwrap();
        order
            .iter()
            .map(|axis| left_origin[*axis])
            .cmp(order.iter().map(|axis| right_origin[*axis]))
    });
    for (_, support) in &mut result.sequence {
        support.pop();
    }
    result
}

type TraceMacroMap = BTreeMap<(Atom, Vec<Vec<usize>>), EntityTraceMacro>;

fn solve_trace_macros(cases: &[TraceTrainingCase], limit: usize) -> Vec<Vec<EntityTraceMacro>> {
    let mut assignments = vec![TraceMacroMap::new()];
    for case in cases {
        let mut next = Vec::new();
        for assignment in assignments {
            solve_trace_case(
                case,
                0,
                case.start.clone(),
                assignment,
                BTreeSet::from([case.start.clone()]),
                &mut next,
                limit,
            );
            if next.len() >= limit {
                break;
            }
        }
        assignments = next;
        if assignments.is_empty() {
            break;
        }
    }
    assignments
        .into_iter()
        .map(|assignment| assignment.into_values().collect())
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn solve_trace_case(
    case: &TraceTrainingCase,
    index: usize,
    cursor: Vec<usize>,
    assignment: TraceMacroMap,
    painted: BTreeSet<Vec<usize>>,
    solutions: &mut Vec<TraceMacroMap>,
    limit: usize,
) {
    if solutions.len() >= limit {
        return;
    }
    if index == case.sequence.len() {
        let expected = case
            .target
            .iter()
            .filter(|(_, atom)| **atom != case.background)
            .map(|(key, _)| key.clone())
            .collect::<BTreeSet<_>>();
        if painted == expected {
            solutions.push(assignment);
        }
        return;
    }
    let signature = case.sequence[index].clone();
    let candidates = assignment
        .get(&signature)
        .cloned()
        .map(|item| vec![item])
        .unwrap_or_else(|| trace_macro_candidates(&signature, &case.shape));
    for candidate in candidates {
        let mut next_painted = painted.clone();
        let mut valid = true;
        for offset in &candidate.paint_offsets {
            let Some(target) = checked_shift(&cursor, offset, &case.shape) else {
                valid = false;
                break;
            };
            if case.target.get(&target) != Some(&candidate.value) || !next_painted.insert(target) {
                valid = false;
                break;
            }
        }
        let Some(next_cursor) = checked_shift(&cursor, &candidate.cursor_delta, &case.shape) else {
            continue;
        };
        if !valid {
            continue;
        }
        let mut next_assignment = assignment.clone();
        if let Some(previous) = next_assignment.get(&signature) {
            if previous != &candidate {
                continue;
            }
        } else {
            next_assignment.insert(signature.clone(), candidate);
        }
        solve_trace_case(
            case,
            index + 1,
            next_cursor,
            next_assignment,
            next_painted,
            solutions,
            limit,
        );
    }
}

fn trace_macro_candidates(
    signature: &(Atom, Vec<Vec<usize>>),
    shape: &[usize],
) -> Vec<EntityTraceMacro> {
    let rank = shape.len();
    let mut result = Vec::new();
    for progress_axis in 0..rank {
        for progress_sign in [-1i64, 1] {
            for paint_axis in 0..rank {
                for paint_sign in [-1i64, 1] {
                    for length in 1..=shape[paint_axis] {
                        let mut offsets = Vec::new();
                        let mut delta = vec![0i64; rank];
                        if progress_axis == paint_axis {
                            offsets.extend((1..=length).map(|step| {
                                let mut offset = vec![0i64; rank];
                                offset[progress_axis] = progress_sign * step as i64;
                                offset
                            }));
                            delta[progress_axis] = progress_sign * length as i64;
                        } else {
                            for step in 0..length {
                                let mut offset = vec![0i64; rank];
                                offset[progress_axis] = progress_sign;
                                offset[paint_axis] = if paint_sign > 0 {
                                    step as i64
                                } else {
                                    step as i64 - (length - 1) as i64
                                };
                                offsets.push(offset);
                            }
                            delta[progress_axis] = progress_sign;
                            delta[paint_axis] = paint_sign * (length - 1) as i64;
                        }
                        result.push(EntityTraceMacro {
                            value: signature.0.clone(),
                            normalized_support: signature.1.clone(),
                            paint_offsets: offsets,
                            cursor_delta: delta,
                        });
                    }
                }
            }
        }
    }
    result.sort_by_key(|item| digest_json(item).unwrap_or_default());
    result.dedup();
    result
}

fn checked_shift(origin: &[usize], offset: &[i64], shape: &[usize]) -> Option<Vec<usize>> {
    origin
        .iter()
        .zip(offset)
        .zip(shape)
        .map(|((coordinate, delta), extent)| {
            let target = *coordinate as i64 + delta;
            (target >= 0 && target < *extent as i64).then_some(target as usize)
        })
        .collect()
}

fn axis_permutations(rank: usize) -> Vec<Vec<usize>> {
    fn visit(prefix: &mut Vec<usize>, rest: &mut Vec<usize>, output: &mut Vec<Vec<usize>>) {
        if rest.is_empty() {
            output.push(prefix.clone());
            return;
        }
        for index in 0..rest.len() {
            let value = rest.remove(index);
            prefix.push(value);
            visit(prefix, rest, output);
            prefix.pop();
            rest.insert(index, value);
        }
    }
    let mut output = Vec::new();
    visit(&mut Vec::new(), &mut (0..rank).collect(), &mut output);
    output
}

fn common_integer_offsets(
    examples: &[(Database, Database)],
    source_name: &str,
    output_name: &str,
    source_fields: &[String],
    output_fields: &[String],
) -> Vec<Vec<i64>> {
    let mut offsets: Option<Vec<i64>> = None;
    for (source, target) in examples {
        let (Some(input), Some(output)) = (source.get(source_name), target.get(output_name)) else {
            return Vec::new();
        };
        if input.rows.len() != output.rows.len() {
            return Vec::new();
        }
        let mut candidate = Vec::new();
        for (source_field, output_field) in source_fields.iter().zip(output_fields) {
            let (Ok(a), Ok(b)) = (input.column(source_field), output.column(output_field)) else {
                return Vec::new();
            };
            let input_values = input.rows.iter().map(|row| &row[a]).collect::<Vec<_>>();
            let output_values = output.rows.iter().map(|row| &row[b]).collect::<Vec<_>>();
            let differences = input_values
                .into_iter()
                .zip(output_values)
                .map(|(left, right)| match (left, right) {
                    (Atom::Integer(x), Atom::Integer(y)) => Some(y - x),
                    _ => None,
                })
                .collect::<Option<BTreeSet<_>>>();
            let Some(differences) = differences else {
                return Vec::new();
            };
            if differences.len() != 1 {
                return Vec::new();
            }
            candidate.push(*differences.iter().next().unwrap());
        }
        if let Some(previous) = &offsets {
            if previous != &candidate {
                return Vec::new();
            }
        } else {
            offsets = Some(candidate);
        }
    }
    offsets.into_iter().collect()
}

fn contains_literal(term: &CoreTerm) -> bool {
    match term {
        CoreTerm::Literal { .. } => true,
        CoreTerm::Select { input, .. }
        | CoreTerm::Image { input, .. }
        | CoreTerm::Group { input, .. }
        | CoreTerm::TransitiveClosure { input, .. }
        | CoreTerm::Fold { input, .. }
        | CoreTerm::IsomorphicCompletion { input, .. }
        | CoreTerm::QuotientRepair { input, .. }
        | CoreTerm::EntityTrace { input, .. }
        | CoreTerm::Abstract { body: input, .. } => contains_literal(input),
        CoreTerm::Join { left, right, .. }
        | CoreTerm::Union { left, right }
        | CoreTerm::Difference { left, right }
        | CoreTerm::Overlay {
            base: left,
            layer: right,
            ..
        }
        | CoreTerm::Compose {
            first: left,
            second: right,
        } => contains_literal(left) || contains_literal(right),
        CoreTerm::Scan { seed, .. } => contains_literal(seed),
        CoreTerm::LeastFixedPoint { seed, step, .. } => {
            contains_literal(seed) || contains_literal(step)
        }
        CoreTerm::Source { .. } => false,
    }
}

fn contains_terminal_inference(term: &CoreTerm) -> bool {
    match term {
        CoreTerm::IsomorphicCompletion { .. }
        | CoreTerm::QuotientRepair { .. }
        | CoreTerm::EntityTrace { .. } => true,
        CoreTerm::Select { input, .. }
        | CoreTerm::Image { input, .. }
        | CoreTerm::Group { input, .. }
        | CoreTerm::TransitiveClosure { input, .. }
        | CoreTerm::Fold { input, .. }
        | CoreTerm::Abstract { body: input, .. } => contains_terminal_inference(input),
        CoreTerm::Join { left, right, .. }
        | CoreTerm::Union { left, right }
        | CoreTerm::Difference { left, right }
        | CoreTerm::Overlay {
            base: left,
            layer: right,
            ..
        }
        | CoreTerm::Compose {
            first: left,
            second: right,
        } => contains_terminal_inference(left) || contains_terminal_inference(right),
        CoreTerm::Scan { seed, .. } => contains_terminal_inference(seed),
        CoreTerm::LeastFixedPoint { seed, step, .. } => {
            contains_terminal_inference(seed) || contains_terminal_inference(step)
        }
        CoreTerm::Source { .. } | CoreTerm::Literal { .. } => false,
    }
}

fn discover_macros(
    examples: &[(Database, Database)],
    programs: &[TypedCoreProgram],
    limits: &GenerativeLimits,
) -> Result<Vec<MacroDefinition>> {
    if examples.len() < 2 {
        return Ok(Vec::new());
    }
    let mut macros = Vec::new();
    for program in programs
        .iter()
        .filter(|program| program_error(program, examples, limits.maximum_recursion_depth) == 0)
        .take(limits.maximum_candidates.min(8))
        .cloned()
    {
        if program.outputs.values().any(contains_literal) {
            continue;
        }
        let expanded = description_bits(&program)?;
        let references = gamma_bits(expanded.max(1)).saturating_mul(examples.len());
        let repeated = expanded.saturating_mul(examples.len());
        let gain = repeated.saturating_sub(expanded.saturating_add(references)) as i64;
        if gain <= 0 {
            continue;
        }
        let support_digests = examples
            .iter()
            .map(|(source, target)| digest_json(&(source, target)))
            .collect::<Result<Vec<_>>>()?;
        let definition = MacroDefinition {
            id: format!("structural-macro:{}", program.stable_digest()?),
            parameters: BTreeMap::from([("input".into(), program.input_type.clone())]),
            body: program,
            support_digests,
            held_out_gain_bits: gain,
        };
        definition.validate()?;
        macros.push(definition);
    }
    Ok(macros)
}

fn replay_frames(
    program: &GenerativeProgram,
    examples: &[(&RelationalFrame, &RelationalFrame)],
) -> Result<ReplayScore> {
    let mut score = ReplayScore {
        examples: examples.len(),
        cells: 0,
        exact_cells: 0,
        wrong_cells: 0,
        unresolved_cells: 0,
        exact_examples: 0,
    };
    for (observation, action) in examples {
        let expected = IndexedStructure::infer(action)?;
        score.cells += expected.cardinality();
        let execution = match program.execute(observation) {
            Ok(execution) => execution,
            Err(_) => {
                score.unresolved_cells += expected.cardinality();
                continue;
            }
        };
        if execution.result.shape() != expected.shape() {
            score.unresolved_cells += expected.cardinality();
            continue;
        }
        let mut exact = true;
        for key in cartesian_indices(&expected.shape()) {
            match execution.result.values.get(&key) {
                Some(value) if value == expected.value(&key)? => score.exact_cells += 1,
                Some(_) => {
                    score.wrong_cells += 1;
                    exact = false;
                }
                None => {
                    score.unresolved_cells += 1;
                    exact = false;
                }
            }
        }
        if exact {
            score.exact_examples += 1;
        }
    }
    Ok(score)
}

fn conservative_leave_one_out(
    program: &GenerativeProgram,
    view: &LearningView,
) -> Result<ReplayScore> {
    if view.demonstrations.len() < 2 {
        return Ok(ReplayScore {
            examples: 0,
            cells: 0,
            exact_cells: 0,
            wrong_cells: 0,
            unresolved_cells: 0,
            exact_examples: 0,
        });
    }
    if program.joint.program.outputs.values().any(contains_literal) {
        let cells = view
            .demonstrations
            .iter()
            .map(|item| {
                IndexedStructure::infer(&item.demonstrated_action)
                    .map(|structure| structure.cardinality())
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .sum();
        return Ok(ReplayScore {
            examples: view.demonstrations.len(),
            cells,
            exact_cells: 0,
            wrong_cells: 0,
            unresolved_cells: cells,
            exact_examples: 0,
        });
    }
    if program
        .joint
        .program
        .outputs
        .values()
        .any(contains_fitted_term)
    {
        return fitted_term_leave_one_out(program, view);
    }
    replay_frames(
        program,
        &view
            .demonstrations
            .iter()
            .map(|item| (&item.observation, &item.demonstrated_action))
            .collect::<Vec<_>>(),
    )
}

fn contains_fitted_term(term: &CoreTerm) -> bool {
    match term {
        CoreTerm::EntityTrace { .. } | CoreTerm::IsomorphicCompletion { .. } => true,
        CoreTerm::Select { input, .. }
        | CoreTerm::Image { input, .. }
        | CoreTerm::Group { input, .. }
        | CoreTerm::TransitiveClosure { input, .. }
        | CoreTerm::Overlay { base: input, .. }
        | CoreTerm::Fold { input, .. }
        | CoreTerm::QuotientRepair { input, .. }
        | CoreTerm::Abstract { body: input, .. } => contains_fitted_term(input),
        CoreTerm::Join { left, right, .. }
        | CoreTerm::Union { left, right }
        | CoreTerm::Difference { left, right } => {
            contains_fitted_term(left) || contains_fitted_term(right)
        }
        CoreTerm::Scan { seed, .. } => contains_fitted_term(seed),
        CoreTerm::LeastFixedPoint { seed, step, .. }
        | CoreTerm::Compose {
            first: seed,
            second: step,
        } => contains_fitted_term(seed) || contains_fitted_term(step),
        CoreTerm::Source { .. } | CoreTerm::Literal { .. } => false,
    }
}

fn same_entity_trace_family(left: &CoreTerm, right: &CoreTerm) -> bool {
    match (left, right) {
        (
            CoreTerm::EntityTrace {
                input: left_input,
                keys: left_keys,
                value: left_value,
                split_axis: left_split,
                canvas_high: left_side,
                order: left_order,
                ..
            },
            CoreTerm::EntityTrace {
                input: right_input,
                keys: right_keys,
                value: right_value,
                split_axis: right_split,
                canvas_high: right_side,
                order: right_order,
                ..
            },
        ) => {
            left_input == right_input
                && left_keys == right_keys
                && left_value == right_value
                && left_split == right_split
                && left_side == right_side
                && left_order == right_order
        }
        _ => false,
    }
}

fn fitted_term_leave_one_out(
    program: &GenerativeProgram,
    view: &LearningView,
) -> Result<ReplayScore> {
    let mut total = ReplayScore {
        examples: 0,
        cells: 0,
        exact_cells: 0,
        wrong_cells: 0,
        unresolved_cells: 0,
        exact_examples: 0,
    };
    for held_out in 0..view.demonstrations.len() {
        let training = view
            .demonstrations
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != held_out)
            .map(|(_, example)| {
                Ok((
                    program
                        .joint
                        .headset
                        .perceive(&example.observation, program.maximum_iterations)?
                        .outputs,
                    semantic_action_database(&example.demonstrated_action)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let expected = IndexedStructure::infer(&view.demonstrations[held_out].demonstrated_action)?;
        let mut fold_program = program.clone();
        let mut supported = true;
        for (output_name, original) in &program.joint.program.outputs {
            if !contains_fitted_term(original) {
                continue;
            }
            let Some(fields) = training[0]
                .1
                .get(output_name)
                .map(|relation| relation.attributes.clone())
            else {
                supported = false;
                break;
            };
            let mut candidates = match original {
                CoreTerm::EntityTrace { .. } => {
                    induce_entity_trace_terms(&training, output_name, &fields)?
                        .into_iter()
                        .filter(|candidate| same_entity_trace_family(original, candidate))
                        .collect::<Vec<_>>()
                }
                CoreTerm::IsomorphicCompletion {
                    input, keys, value, ..
                } => {
                    let CoreTerm::Source { relation } = input.as_ref() else {
                        supported = false;
                        break;
                    };
                    common_field_atoms(&training, relation, value)
                        .into_iter()
                        .map(|unavailable| CoreTerm::IsomorphicCompletion {
                            input: input.clone(),
                            keys: keys.clone(),
                            value: value.clone(),
                            unavailable,
                        })
                        .filter(|candidate| {
                            training.iter().all(|(source, target)| {
                                TypedCoreProgram {
                                    input_type: CoreType::Structure {
                                        relations: action_output_schemas(source),
                                    },
                                    output_type: CoreType::Relation {
                                        fields: fields.clone(),
                                    },
                                    bindings: Vec::new(),
                                    outputs: BTreeMap::from([(
                                        output_name.clone(),
                                        candidate.clone(),
                                    )]),
                                    provenance: Vec::new(),
                                }
                                .execute(source, program.maximum_iterations)
                                .ok()
                                .and_then(|execution| execution.outputs.get(output_name).cloned())
                                    == target.get(output_name).cloned()
                            })
                        })
                        .collect::<Vec<_>>()
                }
                _ => Vec::new(),
            };
            candidates.sort_by_key(|candidate| digest_json(candidate).unwrap_or_default());
            let Some(reinduced) = candidates.into_iter().next() else {
                supported = false;
                break;
            };
            fold_program
                .joint
                .program
                .outputs
                .insert(output_name.clone(), reinduced);
        }
        if !supported {
            total.examples += 1;
            total.cells += expected.cardinality();
            total.unresolved_cells += expected.cardinality();
            continue;
        }
        let fold_score = replay_frames(
            &fold_program,
            &[(
                &view.demonstrations[held_out].observation,
                &view.demonstrations[held_out].demonstrated_action,
            )],
        )?;
        total.examples += fold_score.examples;
        total.cells += fold_score.cells;
        total.exact_cells += fold_score.exact_cells;
        total.wrong_cells += fold_score.wrong_cells;
        total.unresolved_cells += fold_score.unresolved_cells;
        total.exact_examples += fold_score.exact_examples;
    }
    Ok(total)
}

fn rank_experiments(
    view: &LearningView,
    hypotheses: &[GenerativeHypothesis],
) -> Result<Vec<ExperimentPriority>> {
    let mut priorities = Vec::new();
    for query in &view.queries {
        let mut predictions = BTreeMap::<String, Exact>::new();
        let mut executable = 0usize;
        for hypothesis in hypotheses {
            let Ok(execution) = hypothesis.program.execute(&query.observation) else {
                continue;
            };
            executable += 1;
            let digest = digest_json(&execution.result_relations)?;
            let mass = predictions.entry(digest).or_insert_with(Exact::zero);
            *mass = mass.add(&hypothesis.posterior);
        }
        let mass = sum(predictions.values().cloned());
        let agreement = if mass.is_zero() {
            Exact::one()
        } else {
            predictions
                .values()
                .try_fold(Exact::zero(), |total, probability| {
                    let normalized = probability.div(&mass)?;
                    Ok::<Exact, anyhow::Error>(total.add(&normalized.mul(&normalized)))
                })?
        };
        priorities.push(ExperimentPriority {
            episode_id: query.episode_id.clone(),
            executable_hypotheses: executable,
            distinct_predicted_actions: predictions.len(),
            posterior_disagreement: Exact::one().sub(&agreement),
        });
    }
    priorities.sort_by(|left, right| {
        right
            .posterior_disagreement
            .cmp(&left.posterior_disagreement)
            .then(left.episode_id.cmp(&right.episode_id))
    });
    Ok(priorities)
}

fn hypothesis_order(
    left: &GenerativeHypothesis,
    right: &GenerativeHypothesis,
) -> std::cmp::Ordering {
    left.leave_one_out
        .wrong_cells
        .saturating_add(left.leave_one_out.unresolved_cells)
        .cmp(
            &right
                .leave_one_out
                .wrong_cells
                .saturating_add(right.leave_one_out.unresolved_cells),
        )
        .then(
            right
                .leave_one_out
                .exact_examples
                .cmp(&left.leave_one_out.exact_examples),
        )
        .then(
            left.replay
                .wrong_cells
                .saturating_add(left.replay.unresolved_cells)
                .cmp(
                    &right
                        .replay
                        .wrong_cells
                        .saturating_add(right.replay.unresolved_cells),
                ),
        )
        .then(left.description_bits.cmp(&right.description_bits))
        .then(right.posterior.cmp(&left.posterior))
        .then(left.id.cmp(&right.id))
}

fn joint_edit_distance(left: &GenerativeProgram, right: &GenerativeProgram) -> usize {
    let mut distance = 0;
    if left.joint.headset.perception != right.joint.headset.perception {
        distance += 1;
    }
    if left.joint.program != right.joint.program {
        distance += 1;
    }
    if left.joint.headset.action != right.joint.headset.action {
        distance += 1;
    }
    if left.joint.latent_state_type != right.joint.latent_state_type
        || left.joint.transition_type != right.joint.transition_type
    {
        distance += 1;
    }
    if left.joint.goal_type != right.joint.goal_type
        || left.joint.policy_type != right.joint.policy_type
    {
        distance += 1;
    }
    distance
}

fn posterior_kernel(hypotheses: &[GenerativeHypothesis]) -> Result<MarkovKernel> {
    if hypotheses.is_empty() {
        bail!("proposal kernel needs hypotheses")
    }
    let count = hypotheses.len();
    if count == 1 {
        return MarkovKernel::identity(vec![hypotheses[0].id.clone()]);
    }
    let mut adjacency = vec![BTreeSet::new(); count];
    for from in 0..count {
        for to in (from + 1)..count {
            if joint_edit_distance(&hypotheses[from].program, &hypotheses[to].program) == 1 {
                adjacency[from].insert(to);
                adjacency[to].insert(from);
            }
        }
    }
    let maximum_degree = adjacency
        .iter()
        .map(BTreeSet::len)
        .max()
        .unwrap_or(1)
        .max(1);
    let proposal_unit = Exact::new(1, maximum_degree as i128)?;
    let mut transition = zeros(count, count);
    for from in 0..count {
        let mut outgoing = Exact::zero();
        for to in &adjacency[from] {
            let ratio = hypotheses[*to].posterior.div(&hypotheses[from].posterior)?;
            let acceptance = ratio.min(Exact::one());
            let probability = proposal_unit.mul(&acceptance);
            transition[from][*to] = probability.clone();
            outgoing = outgoing.add(&probability);
        }
        transition[from][from] = Exact::one().sub(&outgoing);
    }
    MarkovKernel::new(
        hypotheses.iter().map(|item| item.id.clone()).collect(),
        transition,
    )
}

fn detailed_balance(hypotheses: &[GenerativeHypothesis], kernel: &MarkovKernel) -> bool {
    (0..hypotheses.len()).all(|from| {
        (0..hypotheses.len()).all(|to| {
            hypotheses[from].posterior.mul(&kernel.transition[from][to])
                == hypotheses[to].posterior.mul(&kernel.transition[to][from])
        })
    })
}

fn mutate_program(
    selected: &GenerativeHypothesis,
    induction: &GenerativeInduction,
) -> Result<Vec<GenerativeProgram>> {
    let mut mutations = Vec::new();
    let body = &selected.program.joint.program;
    for (name, term) in &body.outputs {
        let variants = match term {
            CoreTerm::Select { input, .. } => vec![(**input).clone()],
            CoreTerm::Image { input, .. } => vec![(**input).clone()],
            CoreTerm::Fold { input, .. } => vec![(**input).clone()],
            _ => vec![CoreTerm::Abstract {
                parameters: Vec::new(),
                body: Box::new(term.clone()),
            }],
        };
        for variant in variants {
            let mut next = selected.program.clone();
            next.joint.program.outputs.insert(name.clone(), variant);
            next.provenance.push(induction.public_view_digest.clone());
            next.provenance.sort();
            next.provenance.dedup();
            next.id = format!(
                "structural-program:{}",
                digest_json(&(
                    &next.joint,
                    next.maximum_iterations,
                    &next.observation_schema_digest,
                    &next.action_schema_digest
                ))?
            );
            if !induction
                .hypotheses
                .iter()
                .any(|item| item.program.id == next.id)
            {
                mutations.push(next);
            }
        }
    }
    Ok(mutations)
}

fn gamma_bits(value: usize) -> usize {
    let bits = usize::BITS as usize - value.max(1).leading_zeros() as usize;
    bits.saturating_mul(2).saturating_sub(1)
}
