use crate::exact::Exact;
use crate::generative::GenerativeHypothesis;
use crate::kernel::StochasticMap;
use crate::memory::digest_json;
use crate::trace_logic::{certify_trace_order, MarkovChain, TraceOrderCertificate, TraceWitness};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearnedOperation {
    pub id: String,
    pub input_type: String,
    pub output_type: String,
    pub executable_digest: String,
    pub generators: Vec<String>,
    pub state_edges: Vec<(String, String)>,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearnedActionAlgebra {
    pub types: Vec<String>,
    pub operations: Vec<LearnedOperation>,
    pub composition: BTreeMap<String, Vec<String>>,
    pub composition_laws: Vec<CompositionLaw>,
    pub identity_operations: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompositionLaw {
    pub first: String,
    pub second: String,
    pub input_type: String,
    pub output_type: String,
    pub composite_edges: Vec<(String, String)>,
    pub semantics_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PerceptionFiber {
    pub id: String,
    pub perceived_distribution: Vec<Exact>,
    pub world_states: Vec<String>,
}

impl LearnedActionAlgebra {
    pub fn derive(types: Vec<String>, operations: Vec<LearnedOperation>) -> Result<Self> {
        if types.is_empty()
            || types.iter().collect::<BTreeSet<_>>().len() != types.len()
            || operations
                .iter()
                .map(|operation| &operation.id)
                .collect::<BTreeSet<_>>()
                .len()
                != operations.len()
        {
            bail!("learned action algebra has duplicate or empty types/operations");
        }
        if operations.iter().any(|operation| {
            !types.contains(&operation.input_type) || !types.contains(&operation.output_type)
        }) {
            bail!("learned operation references an unknown interface type");
        }
        let mut composition = BTreeMap::new();
        let mut composition_laws = Vec::new();
        for left in &operations {
            let composable = operations
                .iter()
                .filter(|right| left.output_type == right.input_type)
                .map(|right| right.id.clone())
                .collect::<Vec<_>>();
            composition.insert(left.id.clone(), composable);
            for right in operations
                .iter()
                .filter(|right| left.output_type == right.input_type)
            {
                let composite_edges = left
                    .state_edges
                    .iter()
                    .flat_map(|(source, middle)| {
                        right
                            .state_edges
                            .iter()
                            .filter(move |(candidate, _)| candidate == middle)
                            .map(move |(_, target)| (source.clone(), target.clone()))
                    })
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                if !composite_edges.is_empty() {
                    composition_laws.push(CompositionLaw {
                        first: left.id.clone(),
                        second: right.id.clone(),
                        input_type: left.input_type.clone(),
                        output_type: right.output_type.clone(),
                        semantics_digest: digest_json(&(
                            left.id.as_str(),
                            right.id.as_str(),
                            &composite_edges,
                        ))?,
                        composite_edges,
                    });
                }
            }
        }
        let identity_operations = operations
            .iter()
            .filter(|operation| {
                operation.input_type == operation.output_type
                    && operation.state_edges.iter().all(|(from, to)| from == to)
            })
            .map(|operation| operation.id.clone())
            .collect();
        Ok(Self {
            types,
            operations,
            composition,
            composition_laws,
            identity_operations,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InterfaceGeometry {
    pub states: Vec<String>,
    pub operation_edges: BTreeMap<String, Vec<(usize, usize)>>,
    pub directed_distance: Vec<Vec<Option<usize>>>,
}

impl InterfaceGeometry {
    pub fn derive(algebra: &LearnedActionAlgebra) -> Result<Self> {
        let states = algebra
            .operations
            .iter()
            .flat_map(|operation| {
                operation
                    .state_edges
                    .iter()
                    .flat_map(|(from, to)| [from.clone(), to.clone()])
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if states.is_empty() {
            bail!("interface geometry needs observed action edges");
        }
        let index = states
            .iter()
            .enumerate()
            .map(|(i, state)| (state.as_str(), i))
            .collect::<BTreeMap<_, _>>();
        let operation_edges = algebra
            .operations
            .iter()
            .map(|operation| {
                (
                    operation.id.clone(),
                    operation
                        .state_edges
                        .iter()
                        .map(|(from, to)| (index[from.as_str()], index[to.as_str()]))
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut adjacency = vec![BTreeSet::new(); states.len()];
        for (from, to) in operation_edges.values().flatten() {
            adjacency[*from].insert(*to);
        }
        let directed_distance = (0..states.len())
            .map(|start| {
                let mut distance = vec![None; states.len()];
                distance[start] = Some(0);
                let mut queue = VecDeque::from([start]);
                while let Some(from) = queue.pop_front() {
                    let Some(current_distance) = distance[from] else {
                        bail!("interface geometry queue contained an unvisited state");
                    };
                    let next = current_distance + 1;
                    for to in &adjacency[from] {
                        if distance[*to].is_none() {
                            distance[*to] = Some(next);
                            queue.push_back(*to);
                        }
                    }
                }
                Ok(distance)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            states,
            operation_edges,
            directed_distance,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PosteriorEntry {
    pub hypothesis_id: String,
    pub probability: Exact,
    pub program_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearnedHeadset {
    pub id: String,
    pub perception_channel: StochasticMap,
    pub action_channel: StochasticMap,
    pub observer_trace: TraceWitness,
    pub trace_order: TraceOrderCertificate,
    pub action_algebra: LearnedActionAlgebra,
    pub perception_fibers: Vec<PerceptionFiber>,
    pub geometry: InterfaceGeometry,
    pub posterior: Vec<PosteriorEntry>,
    pub evidence_digests: Vec<String>,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeadsetCertificate {
    pub headset_digest: String,
    pub perception_closed: bool,
    pub action_closed: bool,
    pub observer_is_trace: bool,
    pub posterior_normalized: bool,
    pub action_algebra_executable: bool,
    pub composition_semantics_executable: bool,
    pub perception_fibers_partition_world: bool,
    pub geometry_derived_from_actions: bool,
    pub evidence_complete: bool,
}

impl LearnedHeadset {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        perception_channel: StochasticMap,
        action_channel: StochasticMap,
        observed_process: &MarkovChain,
        observer_trace: TraceWitness,
        action_algebra: LearnedActionAlgebra,
        hypotheses: &[GenerativeHypothesis],
        evidence_digests: Vec<String>,
        revision: u64,
        duration_horizon: usize,
    ) -> Result<Self> {
        let observer = observer_trace.trace.clone();
        let trace_order = certify_trace_order(
            &observer,
            observed_process,
            &observer_trace,
            duration_horizon,
        )?;
        let geometry = InterfaceGeometry::derive(&action_algebra)?;
        let perception_fibers = derive_perception_fibers(&perception_channel)?;
        let posterior = hypotheses
            .iter()
            .map(|hypothesis| {
                Ok(PosteriorEntry {
                    hypothesis_id: hypothesis.id.clone(),
                    probability: hypothesis.posterior.clone(),
                    program_digest: digest_json(&hypothesis.program)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let id = format!(
            "headset:{}",
            digest_json(&(
                &perception_channel,
                &action_channel,
                &observer_trace,
                &action_algebra,
                &perception_fibers,
                &posterior,
                revision,
            ))?
        );
        let headset = Self {
            id,
            perception_channel,
            action_channel,
            observer_trace,
            trace_order,
            action_algebra,
            perception_fibers,
            geometry,
            posterior,
            evidence_digests,
            revision,
        };
        headset.validate()?;
        Ok(headset)
    }

    pub fn validate(&self) -> Result<HeadsetCertificate> {
        let perception_closed = self
            .perception_channel
            .transition
            .iter()
            .all(|row| crate::exact::sum(row.clone()).is_one());
        let action_closed = self
            .action_channel
            .transition
            .iter()
            .all(|row| crate::exact::sum(row.clone()).is_one());
        let posterior_normalized =
            crate::exact::sum(self.posterior.iter().map(|entry| entry.probability.clone()))
                .is_one();
        let action_algebra_executable = self
            .action_algebra
            .operations
            .iter()
            .all(|operation| !operation.executable_digest.is_empty());
        let composition_semantics_executable = self
            .action_algebra
            .composition_laws
            .iter()
            .all(|law| !law.composite_edges.is_empty() && !law.semantics_digest.is_empty());
        let perception_fibers_partition_world = {
            let members = self
                .perception_fibers
                .iter()
                .flat_map(|fiber| fiber.world_states.iter())
                .collect::<Vec<_>>();
            members.len() == self.perception_channel.domain.len()
                && members.iter().collect::<BTreeSet<_>>().len() == members.len()
                && members
                    .iter()
                    .all(|state| self.perception_channel.domain.contains(state))
        };
        let geometry_derived_from_actions = self
            .geometry
            .operation_edges
            .keys()
            .collect::<BTreeSet<_>>()
            == self
                .action_algebra
                .operations
                .iter()
                .map(|operation| &operation.id)
                .collect::<BTreeSet<_>>();
        let evidence_complete = !self.evidence_digests.is_empty()
            && self
                .evidence_digests
                .iter()
                .all(|digest| !digest.is_empty());
        let certificate = HeadsetCertificate {
            headset_digest: digest_json(self)?,
            perception_closed,
            action_closed,
            observer_is_trace: self.trace_order.observer_is_trace,
            posterior_normalized,
            action_algebra_executable,
            composition_semantics_executable,
            perception_fibers_partition_world,
            geometry_derived_from_actions,
            evidence_complete,
        };
        if !certificate.perception_closed
            || !certificate.action_closed
            || !certificate.observer_is_trace
            || !certificate.posterior_normalized
            || !certificate.action_algebra_executable
            || !certificate.composition_semantics_executable
            || !certificate.perception_fibers_partition_world
            || !certificate.geometry_derived_from_actions
            || !certificate.evidence_complete
        {
            bail!("learned headset failed its executable interface certificate");
        }
        Ok(certificate)
    }
}

fn derive_perception_fibers(channel: &StochasticMap) -> Result<Vec<PerceptionFiber>> {
    let mut classes = BTreeMap::<Vec<Exact>, Vec<String>>::new();
    for (index, state) in channel.domain.iter().enumerate() {
        classes
            .entry(channel.transition[index].clone())
            .or_default()
            .push(state.clone());
    }
    classes
        .into_iter()
        .map(|(perceived_distribution, world_states)| {
            Ok(PerceptionFiber {
                id: format!(
                    "perception-fiber:{}",
                    digest_json(&(&perceived_distribution, &world_states))?
                ),
                perceived_distribution,
                world_states,
            })
        })
        .collect()
}
