use crate::exact::{sum, Exact};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type Matrix = Vec<Vec<Exact>>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KernelCertificate {
    pub state_count: usize,
    pub nonnegative: bool,
    pub row_substochastic: bool,
    pub closed_rows: usize,
    pub open_rows: usize,
}

/// A finite exact Markov or sub-Markov kernel. Missing row mass means termination
/// or escape from the represented support; it is never silently renormalized.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MarkovKernel {
    pub states: Vec<String>,
    pub transition: Matrix,
}

impl MarkovKernel {
    pub fn new(states: Vec<String>, transition: Matrix) -> Result<Self> {
        let kernel = Self { states, transition };
        kernel.validate()?;
        Ok(kernel)
    }

    pub fn validate(&self) -> Result<KernelCertificate> {
        let n = self.states.len();
        if n == 0 {
            bail!("a Markov kernel needs at least one state");
        }
        if self.states.iter().collect::<BTreeSet<_>>().len() != n {
            bail!("Markov state labels must be unique");
        }
        if self.transition.len() != n || self.transition.iter().any(|row| row.len() != n) {
            bail!("Markov transition matrix must be square and match its state labels");
        }
        let mut closed_rows = 0;
        for (index, row) in self.transition.iter().enumerate() {
            if row.iter().any(Exact::is_negative) {
                bail!("row {index} contains a negative probability");
            }
            let row_sum = sum(row.clone());
            if row_sum > Exact::one() {
                bail!("row {index} has mass {row_sum}, exceeding one");
            }
            if row_sum.is_one() {
                closed_rows += 1;
            }
        }
        Ok(KernelCertificate {
            state_count: n,
            nonnegative: true,
            row_substochastic: true,
            closed_rows,
            open_rows: n - closed_rows,
        })
    }

    pub fn index_of(&self, state: &str) -> Option<usize> {
        self.states.iter().position(|candidate| candidate == state)
    }

    pub fn row_mass(&self, row: usize) -> Exact {
        sum(self.transition[row].clone())
    }

    pub fn identity(states: Vec<String>) -> Result<Self> {
        let mut matrix = zeros(states.len(), states.len());
        for (index, row) in matrix.iter_mut().enumerate() {
            row[index] = Exact::one();
        }
        Self::new(states, matrix)
    }

    /// Maximum-likelihood kernel from observed consecutive experience tokens.
    /// States without an observed outgoing transition retain zero mass; no
    /// pseudocount or fabricated self-loop is introduced.
    pub fn from_sequences(sequences: &[Vec<String>]) -> Result<Self> {
        let states = sequences
            .iter()
            .flat_map(|sequence| sequence.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if states.is_empty() {
            bail!("cannot learn a kernel from an empty observation set");
        }
        let index = states
            .iter()
            .enumerate()
            .map(|(position, state)| (state.as_str(), position))
            .collect::<BTreeMap<_, _>>();
        let mut counts = vec![vec![0usize; states.len()]; states.len()];
        for sequence in sequences {
            for pair in sequence.windows(2) {
                let from = index[pair[0].as_str()];
                let to = index[pair[1].as_str()];
                counts[from][to] += 1;
            }
        }
        let mut transition = zeros(states.len(), states.len());
        for (from, row) in counts.iter().enumerate() {
            let total = row.iter().sum::<usize>();
            if total == 0 {
                continue;
            }
            for (to, count) in row.iter().enumerate() {
                transition[from][to] = Exact::from_counts(*count, total)?;
            }
        }
        Self::new(states, transition)
    }

    pub fn restrict_rows(&self, indices: &[usize]) -> Result<Matrix> {
        indices
            .iter()
            .map(|row| {
                if *row >= self.states.len() {
                    bail!("state index {row} is outside the kernel");
                }
                Ok(indices
                    .iter()
                    .map(|column| self.transition[*row][*column].clone())
                    .collect())
            })
            .collect()
    }

    pub fn stable_digest(&self) -> Result<String> {
        crate::memory::digest_json(self)
    }
}

/// A rectangular conditional-probability map. It is used for derived
/// perception-decision-action factorizations without promoting those factors
/// to primitives.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StochasticMap {
    pub domain: Vec<String>,
    pub codomain: Vec<String>,
    pub transition: Matrix,
}

impl StochasticMap {
    pub fn new(domain: Vec<String>, codomain: Vec<String>, transition: Matrix) -> Result<Self> {
        if domain.is_empty() || codomain.is_empty() {
            bail!("a stochastic map needs nonempty domain and codomain");
        }
        if transition.len() != domain.len()
            || transition.iter().any(|row| row.len() != codomain.len())
        {
            bail!("stochastic map dimensions do not match its labels");
        }
        if domain.iter().collect::<BTreeSet<_>>().len() != domain.len()
            || codomain.iter().collect::<BTreeSet<_>>().len() != codomain.len()
        {
            bail!("stochastic map labels must be unique on each side");
        }
        for row in &transition {
            if row.iter().any(Exact::is_negative) || sum(row.clone()) > Exact::one() {
                bail!("stochastic map rows must be nonnegative with mass at most one");
            }
        }
        Ok(Self {
            domain,
            codomain,
            transition,
        })
    }

    pub fn identity(labels: Vec<String>) -> Result<Self> {
        let mut transition = zeros(labels.len(), labels.len());
        for (index, row) in transition.iter_mut().enumerate() {
            row[index] = Exact::one();
        }
        Self::new(labels.clone(), labels, transition)
    }

    pub fn compose(&self, next: &Self) -> Result<Self> {
        if self.codomain != next.domain {
            bail!("stochastic-map composition has incompatible boundary labels");
        }
        Self::new(
            self.domain.clone(),
            next.codomain.clone(),
            multiply(&self.transition, &next.transition)?,
        )
    }
}

pub fn zeros(rows: usize, columns: usize) -> Matrix {
    vec![vec![Exact::zero(); columns]; rows]
}

pub fn identity(size: usize) -> Matrix {
    let mut result = zeros(size, size);
    for (index, row) in result.iter_mut().enumerate() {
        row[index] = Exact::one();
    }
    result
}

pub fn multiply(left: &Matrix, right: &Matrix) -> Result<Matrix> {
    if left.is_empty() || right.is_empty() {
        bail!("matrix multiplication needs nonempty matrices");
    }
    let shared = left[0].len();
    if left.iter().any(|row| row.len() != shared)
        || right.len() != shared
        || right.iter().any(|row| row.len() != right[0].len())
    {
        bail!("matrix multiplication dimensions are incompatible");
    }
    let mut product = zeros(left.len(), right[0].len());
    for row in 0..left.len() {
        for column in 0..right[0].len() {
            product[row][column] =
                sum((0..shared).map(|middle| left[row][middle].mul(&right[middle][column])));
        }
    }
    Ok(product)
}

pub fn subtract(left: &Matrix, right: &Matrix) -> Result<Matrix> {
    if left.len() != right.len() || left.iter().zip(right).any(|(a, b)| a.len() != b.len()) {
        bail!("matrix subtraction dimensions differ");
    }
    Ok(left
        .iter()
        .zip(right)
        .map(|(a, b)| a.iter().zip(b).map(|(x, y)| x.sub(y)).collect())
        .collect())
}

pub fn inverse(matrix: &Matrix) -> Result<Matrix> {
    let size = matrix.len();
    if size == 0 || matrix.iter().any(|row| row.len() != size) {
        bail!("matrix inverse requires a nonempty square matrix");
    }
    let mut augmented = matrix
        .iter()
        .enumerate()
        .map(|(row, values)| {
            let mut result = values.clone();
            result.extend((0..size).map(|column| {
                if row == column {
                    Exact::one()
                } else {
                    Exact::zero()
                }
            }));
            result
        })
        .collect::<Matrix>();

    for pivot in 0..size {
        let nonzero = (pivot..size)
            .find(|row| !augmented[*row][pivot].is_zero())
            .with_context(|| "matrix is singular")?;
        augmented.swap(pivot, nonzero);
        let divisor = augmented[pivot][pivot].clone();
        for value in &mut augmented[pivot] {
            *value = value.div(&divisor)?;
        }
        for row in 0..size {
            if row == pivot {
                continue;
            }
            let factor = augmented[row][pivot].clone();
            if factor.is_zero() {
                continue;
            }
            let pivot_row = augmented[pivot].clone();
            for (value, pivot_value) in augmented[row].iter_mut().zip(pivot_row) {
                *value = value.sub(&factor.mul(&pivot_value));
            }
        }
    }
    Ok(augmented
        .into_iter()
        .map(|row| row[size..].to_vec())
        .collect())
}

pub fn select(matrix: &Matrix, rows: &[usize], columns: &[usize]) -> Matrix {
    rows.iter()
        .map(|row| {
            columns
                .iter()
                .map(|column| matrix[*row][*column].clone())
                .collect()
        })
        .collect()
}
