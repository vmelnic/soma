use crate::exact::{sum, Exact};
use crate::kernel::{identity, inverse, multiply, select, subtract, zeros, MarkovKernel, Matrix};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DurationTrace {
    /// `by_step[k - 1]` contains first-return probabilities at exactly step k.
    pub by_step: Vec<Matrix>,
    pub tail_after_horizon: Vec<Exact>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCertificate {
    pub source_kernel_digest: String,
    pub visible_states: Vec<String>,
    pub first_return: MarkovKernel,
    pub duration: DurationTrace,
    pub nonreturn_probability: Vec<Exact>,
    pub exact_row_decomposition: bool,
}

pub fn first_return_trace(
    kernel: &MarkovKernel,
    support: &[String],
    duration_horizon: usize,
) -> Result<TraceCertificate> {
    if duration_horizon == 0 {
        bail!("duration horizon must be positive");
    }
    let visible = support
        .iter()
        .map(|state| {
            kernel
                .index_of(state)
                .ok_or_else(|| anyhow::anyhow!("trace support state is absent from the kernel"))
        })
        .collect::<Result<Vec<_>>>()?;
    if visible.is_empty() {
        bail!("first-return trace support cannot be empty");
    }
    first_passage(kernel, &visible, &visible, duration_horizon)
}

/// Exact first-passage semantics from a set of starts to a set of targets.
/// Passage always requires at least one transition, so this also implements
/// first return when starts and targets coincide.
pub fn first_passage(
    kernel: &MarkovKernel,
    starts: &[usize],
    targets: &[usize],
    duration_horizon: usize,
) -> Result<TraceCertificate> {
    if starts.is_empty() || targets.is_empty() || duration_horizon == 0 {
        bail!("first passage requires starts, targets, and a positive horizon");
    }
    let n = kernel.states.len();
    if starts.iter().chain(targets).any(|index| *index >= n) {
        bail!("first-passage support is outside the source kernel");
    }
    let target_set = targets.iter().copied().collect::<BTreeSet<_>>();
    let hidden_candidates = (0..n)
        .filter(|index| !target_set.contains(index))
        .collect::<Vec<_>>();
    let productive = hidden_that_can_reach_targets(kernel, &hidden_candidates, targets);

    let direct = select(&kernel.transition, starts, targets);
    let start_hidden = select(&kernel.transition, starts, &productive);
    let hidden_hidden = select(&kernel.transition, &productive, &productive);
    let hidden_target = select(&kernel.transition, &productive, targets);

    let indirect = if productive.is_empty() {
        zeros(starts.len(), targets.len())
    } else {
        let fundamental = inverse(&subtract(&identity(productive.len()), &hidden_hidden)?)?;
        multiply(&multiply(&start_hidden, &fundamental)?, &hidden_target)?
    };
    let mut total = direct.clone();
    for row in 0..total.len() {
        for column in 0..total[row].len() {
            total[row][column] = total[row][column].add(&indirect[row][column]);
        }
    }

    let mut by_step = Vec::with_capacity(duration_horizon);
    let mut hidden_power = identity(productive.len());
    for step in 1..=duration_horizon {
        let matrix = if step == 1 {
            direct.clone()
        } else if productive.is_empty() {
            zeros(starts.len(), targets.len())
        } else {
            let result = multiply(&multiply(&start_hidden, &hidden_power)?, &hidden_target)?;
            hidden_power = multiply(&hidden_power, &hidden_hidden)?;
            result
        };
        by_step.push(matrix);
    }

    let observed_through_horizon = (0..starts.len())
        .map(|row| {
            sum(by_step
                .iter()
                .flat_map(|matrix| matrix[row].iter().cloned()))
        })
        .collect::<Vec<_>>();
    let total_return = total.iter().map(|row| sum(row.clone())).collect::<Vec<_>>();
    let tail_after_horizon = total_return
        .iter()
        .zip(&observed_through_horizon)
        .map(|(all, finite)| all.sub(finite))
        .collect::<Vec<_>>();
    let nonreturn_probability = total_return
        .iter()
        .map(|returned| Exact::one().sub(returned))
        .collect::<Vec<_>>();
    let exact_row_decomposition = (0..starts.len()).all(|row| {
        !tail_after_horizon[row].is_negative()
            && !nonreturn_probability[row].is_negative()
            && observed_through_horizon[row]
                .add(&tail_after_horizon[row])
                .add(&nonreturn_probability[row])
                .is_one()
    });

    let first_return = MarkovKernel::new(
        starts
            .iter()
            .map(|index| kernel.states[*index].clone())
            .collect(),
        if starts == targets {
            total
        } else {
            // A first-passage map can be rectangular. For trace certificates
            // used by open machines, embed starts and targets in a square
            // boundary only when their ordered labels match.
            if starts.len() != targets.len()
                || starts
                    .iter()
                    .zip(targets)
                    .any(|(a, b)| kernel.states[*a] != kernel.states[*b])
            {
                bail!("rectangular first passage must use first_passage_map");
            }
            total
        },
    )?;
    Ok(TraceCertificate {
        source_kernel_digest: kernel.stable_digest()?,
        visible_states: first_return.states.clone(),
        first_return,
        duration: DurationTrace {
            by_step,
            tail_after_horizon,
        },
        nonreturn_probability,
        exact_row_decomposition,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PassageMap {
    pub starts: Vec<String>,
    pub targets: Vec<String>,
    pub probability: Matrix,
    pub by_step: Vec<Matrix>,
    pub tail_after_horizon: Vec<Exact>,
    pub nonreturn_probability: Vec<Exact>,
    pub exact_row_decomposition: bool,
}

pub fn first_passage_map(
    kernel: &MarkovKernel,
    starts: &[usize],
    targets: &[usize],
    duration_horizon: usize,
) -> Result<PassageMap> {
    if starts
        .iter()
        .any(|start| targets.iter().any(|target| target == start))
    {
        bail!("open first-passage boundaries must be disjoint");
    }
    let target_set = targets.iter().copied().collect::<BTreeSet<_>>();
    let hidden_candidates = (0..kernel.states.len())
        .filter(|index| !target_set.contains(index))
        .collect::<Vec<_>>();
    let productive = hidden_that_can_reach_targets(kernel, &hidden_candidates, targets);
    let direct = select(&kernel.transition, starts, targets);
    let start_hidden = select(&kernel.transition, starts, &productive);
    let hidden_hidden = select(&kernel.transition, &productive, &productive);
    let hidden_target = select(&kernel.transition, &productive, targets);
    let indirect = if productive.is_empty() {
        zeros(starts.len(), targets.len())
    } else {
        multiply(
            &multiply(
                &start_hidden,
                &inverse(&subtract(&identity(productive.len()), &hidden_hidden)?)?,
            )?,
            &hidden_target,
        )?
    };
    let probability = direct
        .iter()
        .zip(&indirect)
        .map(|(left, right)| {
            left.iter()
                .zip(right)
                .map(|(a, b)| a.add(b))
                .collect::<Vec<_>>()
        })
        .collect::<Matrix>();
    let mut by_step = vec![direct];
    let mut power = identity(productive.len());
    for _step in 2..=duration_horizon {
        if productive.is_empty() {
            by_step.push(zeros(starts.len(), targets.len()));
        } else {
            by_step.push(multiply(&multiply(&start_hidden, &power)?, &hidden_target)?);
            power = multiply(&power, &hidden_hidden)?;
        }
    }
    let finite = (0..starts.len())
        .map(|row| {
            sum(by_step
                .iter()
                .flat_map(|matrix| matrix[row].iter().cloned()))
        })
        .collect::<Vec<_>>();
    let returned = probability
        .iter()
        .map(|row| sum(row.clone()))
        .collect::<Vec<_>>();
    let tail_after_horizon = returned
        .iter()
        .zip(&finite)
        .map(|(all, observed)| all.sub(observed))
        .collect::<Vec<_>>();
    let nonreturn_probability = returned
        .iter()
        .map(|value| Exact::one().sub(value))
        .collect::<Vec<_>>();
    let exact_row_decomposition = (0..starts.len()).all(|row| {
        !tail_after_horizon[row].is_negative()
            && !nonreturn_probability[row].is_negative()
            && finite[row]
                .add(&tail_after_horizon[row])
                .add(&nonreturn_probability[row])
                .is_one()
    });
    Ok(PassageMap {
        starts: starts
            .iter()
            .map(|index| kernel.states[*index].clone())
            .collect(),
        targets: targets
            .iter()
            .map(|index| kernel.states[*index].clone())
            .collect(),
        probability,
        by_step,
        tail_after_horizon,
        nonreturn_probability,
        exact_row_decomposition,
    })
}

fn hidden_that_can_reach_targets(
    kernel: &MarkovKernel,
    hidden: &[usize],
    targets: &[usize],
) -> Vec<usize> {
    let hidden_set = hidden.iter().copied().collect::<BTreeSet<_>>();
    let mut productive = targets.iter().copied().collect::<BTreeSet<_>>();
    let mut queue = targets.iter().copied().collect::<VecDeque<_>>();
    while let Some(destination) = queue.pop_front() {
        for source in 0..kernel.states.len() {
            if !kernel.transition[source][destination].is_zero() && productive.insert(source) {
                queue.push_back(source);
            }
        }
    }
    hidden
        .iter()
        .filter(|state| hidden_set.contains(state) && productive.contains(state))
        .copied()
        .collect()
}
