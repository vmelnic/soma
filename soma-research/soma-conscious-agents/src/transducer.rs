use crate::exact::Exact;
use crate::kernel::{zeros, MarkovKernel};
use crate::trace::{first_passage_map, PassageMap};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeKind {
    Input { port: String },
    Hidden,
    Output { port: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineNode {
    pub id: String,
    pub kind: NodeKind,
}

/// An open Markov process. Ports are boundary states and hidden nodes are the
/// implementation. Its observable behavior is its exact first-passage trace.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenTransducer {
    pub id: String,
    pub nodes: Vec<MachineNode>,
    pub kernel: MarkovKernel,
    pub provenance: Vec<String>,
}

impl OpenTransducer {
    pub fn new(
        id: impl Into<String>,
        nodes: Vec<MachineNode>,
        kernel: MarkovKernel,
        provenance: Vec<String>,
    ) -> Result<Self> {
        if nodes.len() != kernel.states.len() {
            bail!("transducer nodes and kernel states differ in size");
        }
        if nodes
            .iter()
            .zip(&kernel.states)
            .any(|(node, state)| node.id != *state)
        {
            bail!("transducer node order must match kernel state order");
        }
        let input_ports = nodes
            .iter()
            .filter_map(|node| match &node.kind {
                NodeKind::Input { port } => Some(port),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let output_ports = nodes
            .iter()
            .filter_map(|node| match &node.kind {
                NodeKind::Output { port } => Some(port),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let input_count = nodes
            .iter()
            .filter(|node| matches!(&node.kind, NodeKind::Input { .. }))
            .count();
        let output_count = nodes
            .iter()
            .filter(|node| matches!(&node.kind, NodeKind::Output { .. }))
            .count();
        if input_count == 0 || output_count == 0 {
            bail!("an open transducer needs input and output ports");
        }
        if input_ports.len() != input_count || output_ports.len() != output_count {
            bail!("transducer port names must be unique on each boundary");
        }
        for (index, node) in nodes.iter().enumerate() {
            if matches!(&node.kind, NodeKind::Output { .. }) {
                let absorbing =
                    kernel.transition[index]
                        .iter()
                        .enumerate()
                        .all(|(to, probability)| {
                            if to == index {
                                probability.is_one()
                            } else {
                                probability.is_zero()
                            }
                        });
                if !absorbing {
                    bail!("output nodes must be absorbing before composition");
                }
            }
        }
        Ok(Self {
            id: id.into(),
            nodes,
            kernel,
            provenance,
        })
    }

    pub fn input_ports(&self) -> BTreeMap<String, usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| match &node.kind {
                NodeKind::Input { port } => Some((port.clone(), index)),
                _ => None,
            })
            .collect()
    }

    pub fn output_ports(&self) -> BTreeMap<String, usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| match &node.kind {
                NodeKind::Output { port } => Some((port.clone(), index)),
                _ => None,
            })
            .collect()
    }

    pub fn semantics(&self, duration_horizon: usize) -> Result<PassageMap> {
        let inputs = self.input_ports().into_values().collect::<Vec<_>>();
        let outputs = self.output_ports().into_values().collect::<Vec<_>>();
        first_passage_map(&self.kernel, &inputs, &outputs, duration_horizon)
    }

    /// Embed an empirical one-step Markov law as an open computation. Each
    /// input names the present experience; each output names the next one.
    pub fn from_markov_one_step(id: impl Into<String>, source: &MarkovKernel) -> Result<Self> {
        let n = source.states.len();
        let mut nodes = Vec::with_capacity(3 * n);
        for state in &source.states {
            nodes.push(MachineNode {
                id: format!("input::{state}"),
                kind: NodeKind::Input {
                    port: state.clone(),
                },
            });
        }
        for state in &source.states {
            nodes.push(MachineNode {
                id: format!("hidden::{state}"),
                kind: NodeKind::Hidden,
            });
        }
        for state in &source.states {
            nodes.push(MachineNode {
                id: format!("output::{state}"),
                kind: NodeKind::Output {
                    port: state.clone(),
                },
            });
        }
        let mut transition = zeros(3 * n, 3 * n);
        for state in 0..n {
            transition[state][n + state] = Exact::one();
            for target in 0..n {
                transition[n + state][2 * n + target] = source.transition[state][target].clone();
            }
            transition[2 * n + state][2 * n + state] = Exact::one();
        }
        let kernel = MarkovKernel::new(
            nodes.iter().map(|node| node.id.clone()).collect(),
            transition,
        )?;
        Self::new(
            id,
            nodes,
            kernel,
            vec![format!("empirical-kernel:{}", source.stable_digest()?)],
        )
    }

    pub fn sequential(
        id: impl Into<String>,
        left: &Self,
        right: &Self,
        wiring: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let left_outputs = left.output_ports();
        let right_inputs = right.input_ports();
        if wiring.len() != left_outputs.len() || wiring.len() != right_inputs.len() {
            bail!("sequential composition must wire every intermediate port exactly once");
        }
        if wiring.keys().collect::<BTreeSet<_>>() != left_outputs.keys().collect::<BTreeSet<_>>()
            || wiring.values().collect::<BTreeSet<_>>()
                != right_inputs.keys().collect::<BTreeSet<_>>()
        {
            bail!("sequential wiring is not a boundary bijection");
        }
        let left_size = left.nodes.len();
        let mut nodes = left
            .nodes
            .iter()
            .map(|node| MachineNode {
                id: format!("left/{}", node.id),
                kind: match &node.kind {
                    NodeKind::Output { .. } => NodeKind::Hidden,
                    other => other.clone(),
                },
            })
            .collect::<Vec<_>>();
        nodes.extend(right.nodes.iter().map(|node| MachineNode {
            id: format!("right/{}", node.id),
            kind: match &node.kind {
                NodeKind::Input { .. } => NodeKind::Hidden,
                other => other.clone(),
            },
        }));
        let mut transition = zeros(nodes.len(), nodes.len());
        for (from, row) in left.kernel.transition.iter().enumerate() {
            transition[from][..left_size].clone_from_slice(row);
        }
        for (from, row) in right.kernel.transition.iter().enumerate() {
            transition[left_size + from][left_size..].clone_from_slice(row);
        }
        for (left_port, right_port) in wiring {
            let from = left_outputs[left_port];
            let to = left_size + right_inputs[right_port];
            transition[from].fill(Exact::zero());
            transition[from][to] = Exact::one();
        }
        let kernel = MarkovKernel::new(
            nodes.iter().map(|node| node.id.clone()).collect(),
            transition,
        )?;
        let mut provenance = left.provenance.clone();
        provenance.extend(right.provenance.clone());
        provenance.push("operator:sequential".to_string());
        Self::new(id, nodes, kernel, provenance)
    }

    pub fn parallel(id: impl Into<String>, left: &Self, right: &Self) -> Result<Self> {
        let right_size = right.nodes.len();
        let mut nodes = Vec::with_capacity(left.nodes.len() * right_size);
        for left_node in &left.nodes {
            for right_node in &right.nodes {
                let kind = match (&left_node.kind, &right_node.kind) {
                    (NodeKind::Input { port: a }, NodeKind::Input { port: b }) => NodeKind::Input {
                        port: serde_json::to_string(&(a, b))?,
                    },
                    (NodeKind::Output { port: a }, NodeKind::Output { port: b }) => {
                        NodeKind::Output {
                            port: serde_json::to_string(&(a, b))?,
                        }
                    }
                    _ => NodeKind::Hidden,
                };
                nodes.push(MachineNode {
                    id: serde_json::to_string(&(left_node.id.as_str(), right_node.id.as_str()))?,
                    kind,
                });
            }
        }
        let mut transition = zeros(nodes.len(), nodes.len());
        for left_from in 0..left.nodes.len() {
            for right_from in 0..right.nodes.len() {
                let from = left_from * right_size + right_from;
                for left_to in 0..left.nodes.len() {
                    for right_to in 0..right.nodes.len() {
                        let to = left_to * right_size + right_to;
                        transition[from][to] = left.kernel.transition[left_from][left_to]
                            .mul(&right.kernel.transition[right_from][right_to]);
                    }
                }
            }
        }
        let kernel = MarkovKernel::new(
            nodes.iter().map(|node| node.id.clone()).collect(),
            transition,
        )?;
        let mut provenance = left.provenance.clone();
        provenance.extend(right.provenance.clone());
        provenance.push("operator:parallel".to_string());
        Self::new(id, nodes, kernel, provenance)
    }

    /// Feed selected outputs back to selected inputs through an explicit
    /// one-step bridge. The bridge supplies the guard; zero-time algebraic
    /// feedback is unrepresentable.
    pub fn guarded_feedback(
        id: impl Into<String>,
        source: &Self,
        wiring: &BTreeMap<String, String>,
    ) -> Result<Self> {
        if wiring.is_empty() {
            bail!("guarded feedback needs at least one feedback wire");
        }
        let inputs = source.input_ports();
        let outputs = source.output_ports();
        if wiring.keys().any(|port| !outputs.contains_key(port))
            || wiring.values().any(|port| !inputs.contains_key(port))
        {
            bail!("feedback wiring references an unknown port");
        }
        if wiring.values().collect::<BTreeSet<_>>().len() != wiring.len() {
            bail!("feedback inputs cannot have multiple drivers");
        }
        let mut nodes = source.nodes.clone();
        let mut transition = source.kernel.transition.clone();
        for (output, input) in wiring {
            let from = outputs[output];
            let to = inputs[input];
            nodes[from].kind = NodeKind::Hidden;
            nodes[to].kind = NodeKind::Hidden;
            transition[from].fill(Exact::zero());
            transition[from][to] = Exact::one();
        }
        if !nodes
            .iter()
            .any(|node| matches!(&node.kind, NodeKind::Input { .. }))
            || !nodes
                .iter()
                .any(|node| matches!(&node.kind, NodeKind::Output { .. }))
        {
            bail!("feedback must leave at least one external input and output");
        }
        let kernel = MarkovKernel::new(
            nodes.iter().map(|node| node.id.clone()).collect(),
            transition,
        )?;
        let mut provenance = source.provenance.clone();
        provenance.push("operator:guarded_feedback".to_string());
        Self::new(id, nodes, kernel, provenance)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MacroCertificate {
    pub body_digest: String,
    pub semantics_digest: String,
    pub exact_trace_cached: bool,
    pub duration_trace_cached: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearnedMacro {
    pub id: String,
    pub body: OpenTransducer,
    pub semantics: PassageMap,
    pub description_bits_saved: usize,
    pub evidence_digests: Vec<String>,
    pub certificate: MacroCertificate,
}

impl LearnedMacro {
    pub fn certify(
        id: impl Into<String>,
        body: OpenTransducer,
        duration_horizon: usize,
        description_bits_saved: usize,
        evidence_digests: Vec<String>,
    ) -> Result<Self> {
        if description_bits_saved == 0 {
            bail!("a learned macro must compress its supporting programs");
        }
        let semantics = body.semantics(duration_horizon)?;
        let body_digest = crate::memory::digest_json(&body)?;
        let semantics_digest = crate::memory::digest_json(&semantics)?;
        Ok(Self {
            id: id.into(),
            body,
            semantics,
            description_bits_saved,
            evidence_digests,
            certificate: MacroCertificate {
                body_digest,
                semantics_digest,
                exact_trace_cached: true,
                duration_trace_cached: true,
            },
        })
    }

    pub fn verify(&self, duration_horizon: usize) -> Result<bool> {
        let recomputed = self.body.semantics(duration_horizon)?;
        Ok(recomputed == self.semantics
            && crate::memory::digest_json(&self.body)? == self.certificate.body_digest
            && crate::memory::digest_json(&recomputed)? == self.certificate.semantics_digest)
    }
}
