use crate::instrument::MeasurementBatch;
use crate::kernel::MarkovKernel;
use crate::memory::digest_json;
use crate::trace::TraceCertificate;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    FormalResult,
    EmpiricalMeasurement,
    OpenConjecture,
    Refuted,
    NotEvaluated,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Claim {
    pub id: String,
    pub title: String,
    pub status: ClaimStatus,
    pub statement: String,
    pub required_bridge: String,
    pub evidence_digests: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecurrentClass {
    pub states: Vec<String>,
    pub closed: bool,
    pub period: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NumericReportSummary {
    pub experience: String,
    pub field: String,
    pub count: usize,
    pub mean: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub source_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProjectionReport {
    pub source_kernel_digest: String,
    pub source_trace_digest: String,
    pub recurrent_classes: Vec<RecurrentClass>,
    pub report_summaries: Vec<NumericReportSummary>,
    pub claims: Vec<Claim>,
    pub feedback_into_induction: bool,
}

/// The workbench consumes certified dynamics but cannot mutate the inducer,
/// its priors, or its evidence. Formal graph invariants, empirical summaries,
/// and physical/consciousness conjectures therefore cannot be conflated.
pub fn build_projection_report(
    kernel: &MarkovKernel,
    trace: &TraceCertificate,
    reported_experience: Option<&MeasurementBatch>,
) -> Result<ProjectionReport> {
    let recurrent_classes = recurrent_classes(kernel);
    let report_summaries = reported_experience
        .map(numeric_report_summaries)
        .unwrap_or_default();
    let source_digest = reported_experience
        .map(|batch| vec![batch.source.sha256.clone()])
        .unwrap_or_default();
    let mut claims = physics_conjectures();
    claims.push(Claim {
        id: "consciousness.experience_primitive".to_string(),
        title: "Experience as primitive".to_string(),
        status: ClaimStatus::FormalResult,
        statement: "The runtime takes experience identities and their sequencing as primitives; physical labels are optional instrument annotations.".to_string(),
        required_bridge: "This is an axiom of the model, not an empirical proof that consciousness is ontologically fundamental.".to_string(),
        evidence_digests: Vec::new(),
    });
    claims.push(Claim {
        id: "consciousness.mint_interface".to_string(),
        title: "Mint experience and headset representations".to_string(),
        status: if report_summaries.is_empty() {
            ClaimStatus::NotEvaluated
        } else {
            ClaimStatus::OpenConjecture
        },
        statement: "A mint experience is primitive while odor names, molecules, receptors, neural states, and objects are candidate headset representations connected by discrimination, report, and intervention.".to_string(),
        required_bridge: "Paired interventions must predict first-person discriminations and reports across unseen participants; the present descriptive report summaries do not identify the experience with any physical icon.".to_string(),
        evidence_digests: source_digest,
    });
    Ok(ProjectionReport {
        source_kernel_digest: kernel.stable_digest()?,
        source_trace_digest: digest_json(trace)?,
        recurrent_classes,
        report_summaries,
        claims,
        feedback_into_induction: false,
    })
}

fn physics_conjectures() -> Vec<Claim> {
    [
        (
            "physics.special_relativity",
            "Special relativity",
            "Minkowski structure and Lorentz symmetry arise in a controlled limit of trace chains.",
            "Specify a convergent family, derive invariant speed and Lorentz transformations, and predict held-out operational measurements.",
        ),
        (
            "physics.general_relativity",
            "General relativity",
            "Curved metric dynamics arise from non-cyclic reflective Markov families.",
            "Derive a metric, curvature, and an energy-momentum coupling with an Einstein-equation limit from one law.",
        ),
        (
            "physics.cosmology",
            "Cosmology and Big Bang",
            "Long trace samples yield expansion histories and cosmological observables.",
            "Declare boundary conditions and derive scale factor, causal history, and unseen observables without inserting Friedmann dynamics.",
        ),
        (
            "physics.planck_breakdown",
            "Planck-scale failure of spacetime",
            "The spacetime projection fails at a derived resolution or energy threshold.",
            "Connect trace state count to calibrated energy and predict a measurable departure from spacetime physics.",
        ),
        (
            "physics.quantum_born",
            "Quantum amplitudes and Born rule",
            "Complex amplitudes, interference, composition, and the Born rule arise from enhanced trace asymptotics.",
            "Construct the amplitude map and derive squared-modulus probabilities rather than encoding them in a projection rule.",
        ),
        (
            "physics.qft_particles",
            "Quantum fields and elementary particles",
            "Classes of Markov dynamics project to local fields, vacuum, particles, and interactions.",
            "Derive locality, field algebra, vacuum, excitation spectrum, interactions, and a controlled continuum limit.",
        ),
        (
            "physics.scattering_quantum_gravity",
            "Scattering amplitudes and quantum gravity",
            "A single dynamics yields positive geometries, scattering amplitudes, and quantum geometry.",
            "Map recurrent classes to decorated permutations and positive geometries, then match new scattering data while coupling geometry and quantum degrees in one law.",
        ),
        (
            "physics.entanglement",
            "Entanglement from disjoint traces",
            "Disjoint observer traces of one ergodic process yield quantum correlations.",
            "Derive a non-signalling correlation family, violate an appropriate Bell inequality, and predict held-out experimental counts.",
        ),
    ]
    .into_iter()
    .map(|(id, title, statement, required_bridge)| Claim {
        id: id.to_string(),
        title: title.to_string(),
        status: ClaimStatus::OpenConjecture,
        statement: statement.to_string(),
        required_bridge: required_bridge.to_string(),
        evidence_digests: Vec::new(),
    })
    .collect()
}

fn recurrent_classes(kernel: &MarkovKernel) -> Vec<RecurrentClass> {
    let n = kernel.states.len();
    let adjacency = kernel
        .transition
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .filter_map(|(to, probability)| (!probability.is_zero()).then_some(to))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut reachable = vec![vec![false; n]; n];
    for (state, state_reachable) in reachable.iter_mut().enumerate() {
        let mut queue = VecDeque::from([state]);
        state_reachable[state] = true;
        while let Some(from) = queue.pop_front() {
            for to in &adjacency[from] {
                if !state_reachable[*to] {
                    state_reachable[*to] = true;
                    queue.push_back(*to);
                }
            }
        }
    }
    let mut unused = (0..n).collect::<BTreeSet<_>>();
    let mut classes = Vec::new();
    while let Some(seed) = unused.iter().next().copied() {
        let members = unused
            .iter()
            .filter(|state| reachable[seed][**state] && reachable[**state][seed])
            .copied()
            .collect::<Vec<_>>();
        for member in &members {
            unused.remove(member);
        }
        let member_set = members.iter().copied().collect::<BTreeSet<_>>();
        let closed = members.iter().all(|from| {
            kernel.row_mass(*from).is_one()
                && adjacency[*from].iter().all(|to| member_set.contains(to))
        });
        if closed {
            classes.push(RecurrentClass {
                states: members
                    .iter()
                    .map(|state| kernel.states[*state].clone())
                    .collect(),
                closed,
                period: graph_period(&members, &adjacency),
            });
        }
    }
    classes
}

fn graph_period(members: &[usize], adjacency: &[Vec<usize>]) -> usize {
    if members.is_empty() {
        return 0;
    }
    let member_set = members.iter().copied().collect::<BTreeSet<_>>();
    let root = members[0];
    let mut depth = BTreeMap::from([(root, 0isize)]);
    let mut queue = VecDeque::from([root]);
    while let Some(from) = queue.pop_front() {
        for to in adjacency[from].iter().filter(|to| member_set.contains(to)) {
            if !depth.contains_key(to) {
                depth.insert(*to, depth[&from] + 1);
                queue.push_back(*to);
            }
        }
    }
    let mut period = 0usize;
    for from in members {
        for to in adjacency[*from].iter().filter(|to| member_set.contains(to)) {
            let delta = (depth[from] + 1 - depth[to]).unsigned_abs();
            period = gcd(period, delta);
        }
    }
    period.max(1)
}

fn gcd(mut left: usize, mut right: usize) -> usize {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn numeric_report_summaries(batch: &MeasurementBatch) -> Vec<NumericReportSummary> {
    let mut values: BTreeMap<(String, String), Vec<f64>> = BTreeMap::new();
    for event in batch.sequences.iter().flat_map(|sequence| &sequence.events) {
        for (field, value) in &event.reports {
            if let Ok(number) = value.parse::<f64>() {
                if number.is_finite() {
                    values
                        .entry((event.experience.clone(), field.clone()))
                        .or_default()
                        .push(number);
                }
            }
        }
    }
    values
        .into_iter()
        .filter_map(|((experience, field), values)| {
            if values.is_empty() {
                return None;
            }
            Some(NumericReportSummary {
                experience,
                field,
                count: values.len(),
                mean: values.iter().sum::<f64>() / values.len() as f64,
                minimum: values.iter().copied().fold(f64::INFINITY, f64::min),
                maximum: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                source_digest: batch.source.sha256.clone(),
            })
        })
        .collect()
}
