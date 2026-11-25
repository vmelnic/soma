use crate::exact::{sum, Exact};
use crate::kernel::{inverse, multiply, zeros, MarkovKernel, Matrix};
use crate::memory::digest_json;
use crate::trace::{first_passage, TraceCertificate};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MarkovChain {
    pub id: String,
    pub initial: Vec<Exact>,
    pub kernel: MarkovKernel,
    pub provenance: Vec<String>,
}

impl MarkovChain {
    pub fn validate(&self) -> Result<()> {
        self.kernel.validate()?;
        if self.initial.len() != self.kernel.states.len()
            || self.initial.iter().any(Exact::is_negative)
            || !sum(self.initial.clone()).is_one()
        {
            bail!("a Markov chain requires one normalized initial distribution");
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String> {
        digest_json(self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceWitness {
    pub id: String,
    pub source_chain_digest: String,
    pub support: Vec<String>,
    pub inclusion: BTreeMap<String, String>,
    pub trace: MarkovChain,
    pub return_certificate: TraceCertificate,
    pub exact_kernel: bool,
    pub initial_is_normalized_restriction: bool,
}

impl TraceWitness {
    pub fn derive(
        source: &MarkovChain,
        support: Vec<String>,
        duration_horizon: usize,
    ) -> Result<Self> {
        source.validate()?;
        if support.is_empty() || support.iter().collect::<BTreeSet<_>>().len() != support.len() {
            bail!("trace support must be nonempty and unique");
        }
        let indices = support
            .iter()
            .map(|state| {
                source
                    .kernel
                    .index_of(state)
                    .ok_or_else(|| anyhow::anyhow!("trace support is outside the source chain"))
            })
            .collect::<Result<Vec<_>>>()?;
        let certificate = first_passage(&source.kernel, &indices, &indices, duration_horizon)?;
        let retained_mass = sum(indices.iter().map(|index| source.initial[*index].clone()));
        if retained_mass.is_zero() {
            bail!("source initial measure assigns zero mass to the trace support");
        }
        let initial = indices
            .iter()
            .map(|index| source.initial[*index].div(&retained_mass))
            .collect::<Result<Vec<_>>>()?;
        let trace = MarkovChain {
            id: format!(
                "trace:{}",
                digest_json(&(source.id.as_str(), &support, duration_horizon))?
            ),
            initial,
            kernel: certificate.first_return.clone(),
            provenance: vec![source.digest()?],
        };
        trace.validate()?;
        let inclusion = support
            .iter()
            .map(|state| (state.clone(), state.clone()))
            .collect();
        let initial_is_normalized_restriction =
            indices.iter().enumerate().all(|(local, source_i)| {
                trace.initial[local].mul(&retained_mass) == source.initial[*source_i]
            });
        let id = format!("trace-witness:{}", digest_json(&(&trace, &inclusion))?);
        Ok(Self {
            id,
            source_chain_digest: source.digest()?,
            support,
            inclusion,
            exact_kernel: certificate.first_return == trace.kernel,
            initial_is_normalized_restriction,
            trace,
            return_certificate: certificate,
        })
    }

    pub fn validates(&self, source: &MarkovChain, duration_horizon: usize) -> Result<bool> {
        let derived = Self::derive(source, self.support.clone(), duration_horizon)?;
        Ok(self.source_chain_digest == source.digest()?
            && self.trace.initial == derived.trace.initial
            && self.trace.kernel == derived.trace.kernel
            && self.inclusion == derived.inclusion)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceOrderCertificate {
    pub observer_chain_digest: String,
    pub observed_chain_digest: String,
    pub witness_digest: String,
    pub observer_is_trace: bool,
    pub observer_is_internal: bool,
}

pub fn certify_trace_order(
    observer: &MarkovChain,
    observed: &MarkovChain,
    witness: &TraceWitness,
    duration_horizon: usize,
) -> Result<TraceOrderCertificate> {
    let observer_is_trace = witness.validates(observed, duration_horizon)?
        && observer.initial == witness.trace.initial
        && observer.kernel == witness.trace.kernel;
    let observer_is_internal = witness
        .support
        .iter()
        .all(|state| observed.kernel.index_of(state).is_some());
    Ok(TraceOrderCertificate {
        observer_chain_digest: observer.digest()?,
        observed_chain_digest: observed.digest()?,
        witness_digest: digest_json(witness)?,
        observer_is_trace,
        observer_is_internal,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceTransitivityCertificate {
    pub root_digest: String,
    pub intermediate_digest: String,
    pub inner_digest: String,
    pub direct_digest: String,
    pub trace_of_trace_equals_direct_trace: bool,
}

pub fn certify_trace_transitivity(
    root: &MarkovChain,
    outer: &TraceWitness,
    inner: &TraceWitness,
    duration_horizon: usize,
) -> Result<TraceTransitivityCertificate> {
    if !outer.validates(root, duration_horizon)?
        || !inner.validates(&outer.trace, duration_horizon)?
    {
        bail!("trace transitivity requires two composable valid witnesses");
    }
    let direct = TraceWitness::derive(root, inner.support.clone(), duration_horizon)?;
    Ok(TraceTransitivityCertificate {
        root_digest: root.digest()?,
        intermediate_digest: outer.trace.digest()?,
        inner_digest: inner.trace.digest()?,
        direct_digest: direct.trace.digest()?,
        trace_of_trace_equals_direct_trace: direct.trace.initial == inner.trace.initial
            && direct.trace.kernel == inner.trace.kernel,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalTraceOperation {
    Meet,
    Join,
    Complement,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalTraceLogicCertificate {
    pub parent_digest: String,
    pub operation: LocalTraceOperation,
    pub operand_supports: Vec<Vec<String>>,
    pub result: TraceWitness,
    pub result_traces_to_parent: bool,
    pub operands_recovered_when_required: bool,
}

pub fn local_meet(
    parent: &MarkovChain,
    left: &TraceWitness,
    right: &TraceWitness,
    duration_horizon: usize,
) -> Result<Option<LocalTraceLogicCertificate>> {
    require_common_parent(parent, left, right, duration_horizon)?;
    let right_support = right.support.iter().collect::<BTreeSet<_>>();
    let support = left
        .support
        .iter()
        .filter(|state| right_support.contains(state))
        .cloned()
        .collect::<Vec<_>>();
    if support.is_empty() {
        return Ok(None);
    }
    let result = TraceWitness::derive(parent, support, duration_horizon)?;
    Ok(Some(LocalTraceLogicCertificate {
        parent_digest: parent.digest()?,
        operation: LocalTraceOperation::Meet,
        operand_supports: vec![left.support.clone(), right.support.clone()],
        result_traces_to_parent: result.validates(parent, duration_horizon)?,
        operands_recovered_when_required: true,
        result,
    }))
}

pub fn local_join(
    parent: &MarkovChain,
    left: &TraceWitness,
    right: &TraceWitness,
    duration_horizon: usize,
) -> Result<LocalTraceLogicCertificate> {
    require_common_parent(parent, left, right, duration_horizon)?;
    let support = left
        .support
        .iter()
        .chain(&right.support)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let result = TraceWitness::derive(parent, support, duration_horizon)?;
    let left_from_join =
        TraceWitness::derive(&result.trace, left.support.clone(), duration_horizon)?;
    let right_from_join =
        TraceWitness::derive(&result.trace, right.support.clone(), duration_horizon)?;
    let recovered = left_from_join.trace.kernel == left.trace.kernel
        && right_from_join.trace.kernel == right.trace.kernel;
    Ok(LocalTraceLogicCertificate {
        parent_digest: parent.digest()?,
        operation: LocalTraceOperation::Join,
        operand_supports: vec![left.support.clone(), right.support.clone()],
        result_traces_to_parent: result.validates(parent, duration_horizon)?,
        operands_recovered_when_required: recovered,
        result,
    })
}

pub fn local_complement(
    parent: &MarkovChain,
    operand: &TraceWitness,
    duration_horizon: usize,
) -> Result<Option<LocalTraceLogicCertificate>> {
    if !operand.validates(parent, duration_horizon)? {
        bail!("local complement requires a trace of the declared parent");
    }
    let hidden = operand.support.iter().collect::<BTreeSet<_>>();
    let support = parent
        .kernel
        .states
        .iter()
        .filter(|state| !hidden.contains(state))
        .cloned()
        .collect::<Vec<_>>();
    if support.is_empty() {
        return Ok(None);
    }
    let result = TraceWitness::derive(parent, support, duration_horizon)?;
    Ok(Some(LocalTraceLogicCertificate {
        parent_digest: parent.digest()?,
        operation: LocalTraceOperation::Complement,
        operand_supports: vec![operand.support.clone()],
        result_traces_to_parent: result.validates(parent, duration_horizon)?,
        operands_recovered_when_required: true,
        result,
    }))
}

fn require_common_parent(
    parent: &MarkovChain,
    left: &TraceWitness,
    right: &TraceWitness,
    duration_horizon: usize,
) -> Result<()> {
    if !left.validates(parent, duration_horizon)? || !right.validates(parent, duration_horizon)? {
        bail!("local trace operations require traces of one common parent");
    }
    Ok(())
}

pub fn stationary_distribution(kernel: &MarkovKernel) -> Result<Vec<Exact>> {
    if kernel
        .transition
        .iter()
        .any(|row| !sum(row.clone()).is_one())
    {
        bail!("stationary distribution requires a closed Markov kernel");
    }
    let n = kernel.states.len();
    if n == 1 {
        return Ok(vec![Exact::one()]);
    }
    let mut system = zeros(n, n);
    for (equation, row) in system.iter_mut().enumerate().take(n - 1) {
        for (variable, value) in row.iter_mut().enumerate().take(n) {
            *value = kernel.transition[variable][equation].clone();
            if equation == variable {
                *value = value.sub(&Exact::one());
            }
        }
    }
    for value in &mut system[n - 1] {
        *value = Exact::one();
    }
    let inverse = inverse(&system)?;
    let mut rhs = vec![vec![Exact::zero()]; n];
    rhs[n - 1][0] = Exact::one();
    let solution: Matrix = multiply(&inverse, &rhs)?;
    let stationary = solution
        .into_iter()
        .map(|row| row[0].clone())
        .collect::<Vec<_>>();
    if stationary.iter().any(Exact::is_negative)
        || !sum(stationary.clone()).is_one()
        || (0..n).any(|destination| {
            sum((0..n)
                .map(|source| stationary[source].mul(&kernel.transition[source][destination])))
                != stationary[destination]
        })
    {
        bail!("kernel has no unique nonnegative stationary distribution in this solver");
    }
    Ok(stationary)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StationaryMapCertificate {
    pub parent_stationary: Vec<Exact>,
    pub trace_stationary: Vec<Exact>,
    pub normalized_parent_restriction: Vec<Exact>,
    pub homomorphism_holds: bool,
}

pub fn certify_stationary_map(
    parent: &MarkovChain,
    trace: &TraceWitness,
    duration_horizon: usize,
) -> Result<StationaryMapCertificate> {
    if !trace.validates(parent, duration_horizon)? {
        bail!("stationary map requires a valid trace witness");
    }
    let parent_stationary = stationary_distribution(&parent.kernel)?;
    let trace_stationary = stationary_distribution(&trace.trace.kernel)?;
    let indices = trace
        .support
        .iter()
        .map(|state| {
            parent
                .kernel
                .index_of(state)
                .ok_or_else(|| anyhow::anyhow!("trace support changed after validation"))
        })
        .collect::<Result<Vec<_>>>()?;
    let retained = sum(indices
        .iter()
        .map(|index| parent_stationary[*index].clone()));
    let normalized_parent_restriction = indices
        .iter()
        .map(|index| parent_stationary[*index].div(&retained))
        .collect::<Result<Vec<_>>>()?;
    Ok(StationaryMapCertificate {
        homomorphism_holds: trace_stationary == normalized_parent_restriction,
        parent_stationary,
        trace_stationary,
        normalized_parent_restriction,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FiniteTraceEvidence {
    pub support: Vec<String>,
    pub transition_counts: Vec<Vec<usize>>,
    pub observed_transitions: usize,
    pub empirical_kernel: MarkovKernel,
    pub row_sample_counts: Vec<usize>,
    /// Exact 95% Chebyshev certificate.  For each observed source state the
    /// squared absolute error of any transition-frequency estimate is at most
    /// the recorded radius except on probability mass at most 1/20.
    pub confidence: Exact,
    pub squared_error_radius_upper: Vec<Option<Exact>>,
    pub unobserved_source_states: Vec<String>,
    pub source_digest: String,
    pub ideal_trace_digest: Option<String>,
    pub empirical_not_ideal: bool,
}

impl FiniteTraceEvidence {
    pub fn observe(
        sequences: &[Vec<String>],
        support: Vec<String>,
        source_digest: impl Into<String>,
        ideal: Option<&TraceWitness>,
    ) -> Result<Self> {
        if support.is_empty() {
            bail!("finite trace sampling needs a nonempty declared support");
        }
        let index = support
            .iter()
            .enumerate()
            .map(|(i, state)| (state.as_str(), i))
            .collect::<BTreeMap<_, _>>();
        let mut counts = vec![vec![0usize; support.len()]; support.len()];
        for sequence in sequences {
            let visible = sequence
                .iter()
                .filter(|state| index.contains_key(state.as_str()))
                .collect::<Vec<_>>();
            for pair in visible.windows(2) {
                counts[index[pair[0].as_str()]][index[pair[1].as_str()]] += 1;
            }
        }
        let observed_transitions = counts.iter().flatten().sum();
        let mut transition = zeros(support.len(), support.len());
        let mut unobserved_source_states = Vec::new();
        let mut row_sample_counts = Vec::new();
        let mut squared_error_radius_upper = Vec::new();
        for (from, row) in counts.iter().enumerate() {
            let total = row.iter().sum::<usize>();
            row_sample_counts.push(total);
            if total == 0 {
                unobserved_source_states.push(support[from].clone());
                squared_error_radius_upper.push(None);
                continue;
            }
            let radius = Exact::from_counts(5, total)?;
            squared_error_radius_upper.push(Some(if radius > Exact::one() {
                Exact::one()
            } else {
                radius
            }));
            for (to, count) in row.iter().enumerate() {
                transition[from][to] = Exact::from_counts(*count, total)?;
            }
        }
        Ok(Self {
            support: support.clone(),
            transition_counts: counts,
            observed_transitions,
            empirical_kernel: MarkovKernel::new(support, transition)?,
            row_sample_counts,
            confidence: Exact::new(19, 20)?,
            squared_error_radius_upper,
            unobserved_source_states,
            source_digest: source_digest.into(),
            ideal_trace_digest: ideal.map(digest_json).transpose()?,
            empirical_not_ideal: true,
        })
    }
}
