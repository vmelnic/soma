use crate::exact::{sum, Exact};
use crate::kernel::MarkovKernel;
use crate::memory::digest_json;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One measured intervention. State identifiers are observer-relative digests;
/// the learner does not assume a matrix, game, or benchmark ontology.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObservedTransition {
    pub sequence_id: String,
    pub ordinal: usize,
    pub state: String,
    pub action: String,
    pub next_state: String,
    pub outcomes: BTreeMap<String, i64>,
    pub terminal: bool,
    pub action_cost: i64,
    pub evidence_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StateAction {
    pub state: String,
    pub action: String,
}

impl Serialize for StateAction {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let key = serde_json::to_string(&(self.state.as_str(), self.action.as_str()))
            .map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&key)
    }
}

impl<'de> Deserialize<'de> for StateAction {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let key = String::deserialize(deserializer)?;
        let (state, action): (String, String) =
            serde_json::from_str(&key).map_err(serde::de::Error::custom)?;
        Ok(Self { state, action })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransitionPrediction {
    pub state: String,
    pub action: String,
    pub next_state_distribution: BTreeMap<String, Exact>,
    pub samples: usize,
    pub unseen: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalHypothesis {
    pub id: String,
    pub outcome_weights: BTreeMap<String, i64>,
    pub terminal_weight: i64,
    pub posterior: Exact,
    pub evidence_digests: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionAssessment {
    pub action: String,
    pub expected_utility: Exact,
    pub information_gain: Exact,
    pub expected_cost: Exact,
    pub reversibility: Exact,
    pub score: Exact,
    pub prediction: TransitionPrediction,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivePlan {
    pub current_state: String,
    pub horizon: usize,
    pub actions: Vec<String>,
    pub assessments: Vec<ActionAssessment>,
    pub policy_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransitionEvaluation {
    pub training_events: usize,
    pub held_out_events: usize,
    pub predicted_events: usize,
    pub top_one_correct: usize,
    pub assigned_probability: Exact,
    pub evidence_digests: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveWorldModel {
    pub observed_events: Vec<ObservedTransition>,
    pub states: Vec<String>,
    pub actions: Vec<String>,
    pub transition_counts: BTreeMap<StateAction, BTreeMap<String, usize>>,
    pub outcome_sums: BTreeMap<StateAction, BTreeMap<String, i64>>,
    pub terminal_counts: BTreeMap<StateAction, usize>,
    pub cost_sums: BTreeMap<StateAction, i64>,
    pub goal_posterior: Vec<GoalHypothesis>,
    pub evidence_digests: Vec<String>,
    pub model_digest: String,
}

/// Withhold the last transition of every multi-step measured sequence.  This
/// is predictive evidence over real traces; one-step probes remain training
/// observations and never become fabricated test continuations.
pub fn evaluate_withheld_transitions(
    events: &[ObservedTransition],
) -> Result<TransitionEvaluation> {
    validate_sequences(events)?;
    let mut by_sequence: BTreeMap<&str, Vec<&ObservedTransition>> = BTreeMap::new();
    for event in events {
        by_sequence
            .entry(&event.sequence_id)
            .or_default()
            .push(event);
    }
    let mut withheld = BTreeSet::new();
    for sequence in by_sequence.values_mut() {
        sequence.sort_by_key(|event| event.ordinal);
        if sequence.len() > 1 {
            withheld.insert(sequence.last().unwrap().evidence_digest.clone());
        }
    }
    let training = events
        .iter()
        .filter(|event| !withheld.contains(&event.evidence_digest))
        .cloned()
        .collect::<Vec<_>>();
    let testing = events
        .iter()
        .filter(|event| withheld.contains(&event.evidence_digest))
        .collect::<Vec<_>>();
    if training.is_empty() {
        bail!("withheld transition evaluation has no training observations")
    }
    let model = ActiveWorldModel::learn(&training)?;
    let mut predicted = 0usize;
    let mut correct = 0usize;
    let mut assigned = Exact::zero();
    for event in &testing {
        let prediction = model.predict(&event.state, &event.action)?;
        if prediction.unseen {
            continue;
        }
        predicted += 1;
        let top = prediction
            .next_state_distribution
            .iter()
            .max_by(|left, right| left.1.cmp(right.1).then(left.0.cmp(right.0)))
            .map(|item| item.0);
        correct += usize::from(top == Some(&event.next_state));
        assigned = assigned.add(
            &prediction
                .next_state_distribution
                .get(&event.next_state)
                .cloned()
                .unwrap_or_else(Exact::zero),
        );
    }
    Ok(TransitionEvaluation {
        training_events: training.len(),
        held_out_events: testing.len(),
        predicted_events: predicted,
        top_one_correct: correct,
        assigned_probability: assigned,
        evidence_digests: testing
            .iter()
            .map(|event| event.evidence_digest.clone())
            .collect(),
    })
}

impl ActiveWorldModel {
    pub fn learn(events: &[ObservedTransition]) -> Result<Self> {
        if events.is_empty() {
            bail!("interactive world learning needs observed transitions")
        }
        validate_sequences(events)?;
        let mut states = BTreeSet::new();
        let mut actions = BTreeSet::new();
        let mut transition_counts: BTreeMap<StateAction, BTreeMap<String, usize>> = BTreeMap::new();
        let mut outcome_sums: BTreeMap<StateAction, BTreeMap<String, i64>> = BTreeMap::new();
        let mut terminal_counts = BTreeMap::new();
        let mut cost_sums = BTreeMap::new();
        for event in events {
            states.extend([event.state.clone(), event.next_state.clone()]);
            actions.insert(event.action.clone());
            let key = StateAction {
                state: event.state.clone(),
                action: event.action.clone(),
            };
            *transition_counts
                .entry(key.clone())
                .or_default()
                .entry(event.next_state.clone())
                .or_insert(0) += 1;
            for (name, value) in &event.outcomes {
                *outcome_sums
                    .entry(key.clone())
                    .or_default()
                    .entry(name.clone())
                    .or_insert(0) += value;
            }
            *terminal_counts.entry(key.clone()).or_insert(0) += usize::from(event.terminal);
            *cost_sums.entry(key).or_insert(0) += event.action_cost;
        }
        let evidence_digests = events
            .iter()
            .map(|event| event.evidence_digest.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let goal_posterior = infer_goals(events)?;
        let material = (
            &states,
            &actions,
            &transition_counts,
            &outcome_sums,
            &terminal_counts,
            &cost_sums,
            &goal_posterior,
            &evidence_digests,
        );
        let model_digest = digest_json(&material)?;
        Ok(Self {
            observed_events: events.to_vec(),
            states: states.into_iter().collect(),
            actions: actions.into_iter().collect(),
            transition_counts,
            outcome_sums,
            terminal_counts,
            cost_sums,
            goal_posterior,
            evidence_digests,
            model_digest,
        })
    }

    pub fn update(&self, event: ObservedTransition) -> Result<Self> {
        let mut events = self.observed_events.clone();
        events.push(event);
        Self::learn(&events)
    }

    pub fn predict(&self, state: &str, action: &str) -> Result<TransitionPrediction> {
        let key = StateAction {
            state: state.to_string(),
            action: action.to_string(),
        };
        let Some(counts) = self.transition_counts.get(&key) else {
            return Ok(TransitionPrediction {
                state: state.into(),
                action: action.into(),
                next_state_distribution: BTreeMap::new(),
                samples: 0,
                unseen: true,
            });
        };
        let samples = counts.values().sum::<usize>();
        let distribution = counts
            .iter()
            .map(|(next, count)| Ok((next.clone(), Exact::new(*count as i128, samples as i128)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok(TransitionPrediction {
            state: state.into(),
            action: action.into(),
            next_state_distribution: distribution,
            samples,
            unseen: false,
        })
    }

    pub fn transition_kernel(&self, action: &str) -> Result<MarkovKernel> {
        let mut rows = Vec::new();
        for state in &self.states {
            let prediction = self.predict(state, action)?;
            if prediction.unseen {
                rows.push(
                    self.states
                        .iter()
                        .map(|candidate| {
                            if candidate == state {
                                Exact::one()
                            } else {
                                Exact::zero()
                            }
                        })
                        .collect(),
                );
            } else {
                rows.push(
                    self.states
                        .iter()
                        .map(|candidate| {
                            prediction
                                .next_state_distribution
                                .get(candidate)
                                .cloned()
                                .unwrap_or_else(Exact::zero)
                        })
                        .collect(),
                );
            }
        }
        MarkovKernel::new(self.states.clone(), rows)
    }

    pub fn assess_actions(&self, state: &str) -> Result<Vec<ActionAssessment>> {
        let mut assessments = Vec::new();
        for action in &self.actions {
            let prediction = self.predict(state, action)?;
            let key = StateAction {
                state: state.into(),
                action: action.clone(),
            };
            let samples = prediction.samples.max(1) as i128;
            let expected_utility =
                self.goal_posterior
                    .iter()
                    .try_fold(Exact::zero(), |total, goal| {
                        let outcomes = self.outcome_sums.get(&key);
                        let raw = goal
                            .outcome_weights
                            .iter()
                            .map(|(name, weight)| {
                                outcomes
                                    .and_then(|items| items.get(name))
                                    .copied()
                                    .unwrap_or(0)
                                    .saturating_mul(*weight)
                            })
                            .sum::<i64>()
                            + self.terminal_counts.get(&key).copied().unwrap_or(0) as i64
                                * goal.terminal_weight;
                        Ok::<Exact, anyhow::Error>(
                            total.add(&goal.posterior.mul(&Exact::new(raw as i128, samples)?)),
                        )
                    })?;
            let expected_cost = Exact::new(
                self.cost_sums.get(&key).copied().unwrap_or(0) as i128,
                samples,
            )?;
            let information_gain = if prediction.unseen {
                Exact::one()
            } else {
                let concentration = prediction
                    .next_state_distribution
                    .values()
                    .fold(Exact::zero(), |total, probability| {
                        total.add(&probability.mul(probability))
                    });
                Exact::one().sub(&concentration)
            };
            let reversibility = self.reversibility(&prediction)?;
            let score = expected_utility
                .add(&information_gain)
                .add(&reversibility)
                .sub(&expected_cost);
            assessments.push(ActionAssessment {
                action: action.clone(),
                expected_utility,
                information_gain,
                expected_cost,
                reversibility,
                score,
                prediction,
            });
        }
        assessments.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then(left.action.cmp(&right.action))
        });
        Ok(assessments)
    }

    pub fn plan(&self, initial_state: &str, maximum_horizon: usize) -> Result<ActivePlan> {
        if maximum_horizon == 0 || !self.states.contains(&initial_state.to_string()) {
            bail!("invalid planning boundary")
        }
        let mut current = initial_state.to_string();
        let mut actions = Vec::new();
        let mut assessments = Vec::new();
        for _ in 0..maximum_horizon {
            let ranked = self.assess_actions(&current)?;
            let Some(best) = ranked.first().cloned() else {
                break;
            };
            let next = best
                .prediction
                .next_state_distribution
                .iter()
                .max_by(|left, right| left.1.cmp(right.1).then(left.0.cmp(right.0)))
                .map(|item| item.0.clone());
            actions.push(best.action.clone());
            assessments.push(best);
            let Some(next) = next else {
                break;
            };
            current = next;
        }
        let policy_digest = digest_json(&(
            initial_state,
            maximum_horizon,
            &actions,
            &assessments,
            &self.model_digest,
        ))?;
        Ok(ActivePlan {
            current_state: initial_state.into(),
            horizon: actions.len(),
            actions,
            assessments,
            policy_digest,
        })
    }

    fn reversibility(&self, prediction: &TransitionPrediction) -> Result<Exact> {
        if prediction.unseen {
            return Ok(Exact::zero());
        }
        let mut reversible = Exact::zero();
        for (next, probability) in &prediction.next_state_distribution {
            let can_return = self.actions.iter().any(|action| {
                self.transition_counts
                    .get(&StateAction {
                        state: next.clone(),
                        action: action.clone(),
                    })
                    .is_some_and(|targets| targets.contains_key(&prediction.state))
            });
            if can_return {
                reversible = reversible.add(probability);
            }
        }
        Ok(reversible)
    }
}

fn validate_sequences(events: &[ObservedTransition]) -> Result<()> {
    let mut sequences: BTreeMap<&str, Vec<&ObservedTransition>> = BTreeMap::new();
    for event in events {
        if event.sequence_id.is_empty()
            || event.state.is_empty()
            || event.action.is_empty()
            || event.next_state.is_empty()
            || event.evidence_digest.is_empty()
        {
            bail!("observed transition is incomplete")
        }
        sequences.entry(&event.sequence_id).or_default().push(event);
    }
    for events in sequences.values_mut() {
        events.sort_by_key(|event| event.ordinal);
        if events
            .windows(2)
            .any(|pair| pair[0].ordinal == pair[1].ordinal || pair[0].next_state != pair[1].state)
        {
            bail!("interactive trace is not a contiguous ordered observation")
        }
    }
    Ok(())
}

fn infer_goals(events: &[ObservedTransition]) -> Result<Vec<GoalHypothesis>> {
    let outcome_names = events
        .iter()
        .flat_map(|event| event.outcomes.keys().cloned())
        .collect::<BTreeSet<_>>();
    let mut candidates = Vec::new();
    for outcome in outcome_names {
        candidates.push((BTreeMap::from([(outcome, 1)]), 0));
    }
    candidates.push((BTreeMap::new(), 1));
    let weights = candidates
        .iter()
        .map(|(outcomes, terminal)| {
            let supporting = events
                .iter()
                .filter(|event| {
                    *terminal > 0 && event.terminal
                        || outcomes.iter().any(|(name, sign)| {
                            event
                                .outcomes
                                .get(name)
                                .is_some_and(|value| value.signum() == *sign)
                        })
                })
                .count();
            Exact::new(
                (supporting + 1) as i128,
                (events.len() + candidates.len()) as i128,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let mass = sum(weights.clone());
    candidates
        .into_iter()
        .zip(weights)
        .enumerate()
        .map(|(index, ((outcome_weights, terminal_weight), weight))| {
            Ok(GoalHypothesis {
                id: format!(
                    "goal:{}",
                    digest_json(&(index, &outcome_weights, terminal_weight))?
                ),
                outcome_weights,
                terminal_weight,
                posterior: weight.div(&mass)?,
                evidence_digests: events
                    .iter()
                    .map(|event| event.evidence_digest.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            })
        })
        .collect()
}
