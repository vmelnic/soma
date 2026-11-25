use crate::exact::{sum, Exact};
use crate::kernel::MarkovKernel;
use crate::transducer::OpenTransducer;
use anyhow::{bail, Result};
use num_bigint::BigUint;
use num_traits::{One, Zero};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct SymbolicState {
    pub control: String,
    pub registers: Vec<BigUint>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChoiceBranch {
    pub probability: Exact,
    pub next: String,
}

/// A local Markov rule over a countably infinite configuration space. Two
/// unbounded counters with increment and conditional decrement are a minimal
/// universal register-machine basis; stochastic choice embeds Markov laws.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "instruction", rename_all = "snake_case")]
pub enum SymbolicInstruction {
    Increment {
        register: usize,
        next: String,
    },
    DecrementOrBranch {
        register: usize,
        nonzero: String,
        zero: String,
    },
    MarkovChoice {
        branches: Vec<ChoiceBranch>,
    },
    Halt {
        output: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolicMarkovProgram {
    pub id: String,
    pub register_count: usize,
    pub inputs: BTreeMap<String, SymbolicState>,
    pub instructions: BTreeMap<String, SymbolicInstruction>,
    pub provenance: Vec<String>,
    pub lifted_kernel: Option<MarkovKernel>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolicTrace {
    pub input: String,
    pub horizon: usize,
    pub output_mass: BTreeMap<String, Exact>,
    pub remaining_mass: Exact,
    pub terminated_without_output_mass: Exact,
    pub explored_configurations: usize,
    pub exact_mass_decomposition: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UniversalSubstrateCertificate {
    pub finite_control: bool,
    pub two_unbounded_registers: bool,
    pub increment: bool,
    pub conditional_decrement_and_zero_branch: bool,
    pub exact_markov_choice: bool,
    pub lazy_finite_observer_trace: bool,
    pub scope: String,
}

impl SymbolicMarkovProgram {
    pub fn validate(&self) -> Result<()> {
        if self.register_count < 2 {
            bail!("universal symbolic programs require at least two registers");
        }
        if self.inputs.is_empty() || self.instructions.is_empty() {
            bail!("symbolic program requires input ports and instructions");
        }
        for state in self.inputs.values() {
            if state.registers.len() != self.register_count {
                bail!("symbolic input has the wrong register arity");
            }
            if !self.instructions.contains_key(&state.control) {
                bail!("symbolic input references unknown control state");
            }
        }
        for instruction in self.instructions.values() {
            match instruction {
                SymbolicInstruction::Increment { register, next } => {
                    self.validate_register_and_target(*register, next)?;
                }
                SymbolicInstruction::DecrementOrBranch {
                    register,
                    nonzero,
                    zero,
                } => {
                    self.validate_register_and_target(*register, nonzero)?;
                    if !self.instructions.contains_key(zero) {
                        bail!("zero branch references unknown control state");
                    }
                }
                SymbolicInstruction::MarkovChoice { branches } => {
                    if branches.iter().any(|branch| {
                        branch.probability.is_negative()
                            || !self.instructions.contains_key(&branch.next)
                    }) || sum(branches.iter().map(|branch| branch.probability.clone()))
                        > Exact::one()
                    {
                        bail!("invalid symbolic Markov choice");
                    }
                }
                SymbolicInstruction::Halt { .. } => {}
            }
        }
        Ok(())
    }

    fn validate_register_and_target(&self, register: usize, target: &str) -> Result<()> {
        if register >= self.register_count {
            bail!("symbolic instruction references unknown register");
        }
        if !self.instructions.contains_key(target) {
            bail!("symbolic instruction references unknown control state");
        }
        Ok(())
    }

    pub fn lift_one_step(kernel: &MarkovKernel) -> Result<Self> {
        let mut inputs = BTreeMap::new();
        let mut instructions = BTreeMap::new();
        for (from, state) in kernel.states.iter().enumerate() {
            let control = format!("choose:{from}");
            inputs.insert(
                state.clone(),
                SymbolicState {
                    control: control.clone(),
                    registers: vec![BigUint::zero(), BigUint::zero()],
                },
            );
            let branches = kernel.transition[from]
                .iter()
                .enumerate()
                .filter(|(_, probability)| !probability.is_zero())
                .map(|(to, probability)| ChoiceBranch {
                    probability: probability.clone(),
                    next: format!("halt:{to}"),
                })
                .collect();
            instructions.insert(control, SymbolicInstruction::MarkovChoice { branches });
        }
        for (to, state) in kernel.states.iter().enumerate() {
            instructions.insert(
                format!("halt:{to}"),
                SymbolicInstruction::Halt {
                    output: state.clone(),
                },
            );
        }
        let program = Self {
            id: format!("symbolic:one-step:{}", kernel.stable_digest()?),
            register_count: 2,
            inputs,
            instructions,
            provenance: vec![kernel.stable_digest()?],
            lifted_kernel: Some(kernel.clone()),
        };
        program.validate()?;
        Ok(program)
    }

    pub fn step(&self, state: &SymbolicState) -> Result<Vec<(SymbolicState, Exact)>> {
        let instruction = self
            .instructions
            .get(&state.control)
            .ok_or_else(|| anyhow::anyhow!("unknown symbolic control state"))?;
        match instruction {
            SymbolicInstruction::Increment { register, next } => {
                let mut destination = state.clone();
                destination.control = next.clone();
                destination.registers[*register] += BigUint::one();
                Ok(vec![(destination, Exact::one())])
            }
            SymbolicInstruction::DecrementOrBranch {
                register,
                nonzero,
                zero,
            } => {
                let mut destination = state.clone();
                if destination.registers[*register].is_zero() {
                    destination.control = zero.clone();
                } else {
                    destination.registers[*register] -= BigUint::one();
                    destination.control = nonzero.clone();
                }
                Ok(vec![(destination, Exact::one())])
            }
            SymbolicInstruction::MarkovChoice { branches } => Ok(branches
                .iter()
                .map(|branch| {
                    let mut destination = state.clone();
                    destination.control = branch.next.clone();
                    (destination, branch.probability.clone())
                })
                .collect()),
            SymbolicInstruction::Halt { .. } => Ok(Vec::new()),
        }
    }

    /// Explore only configurations reachable within an observer's finite
    /// horizon. The countably infinite state space is never materialized.
    pub fn finite_trace(
        &self,
        input: &str,
        horizon: usize,
        maximum_frontier_states: usize,
    ) -> Result<SymbolicTrace> {
        self.validate()?;
        if horizon == 0 || maximum_frontier_states == 0 {
            bail!("symbolic observation limits must be positive");
        }
        let initial = self
            .inputs
            .get(input)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown symbolic input port"))?;
        let mut frontier = BTreeMap::from([(initial, Exact::one())]);
        let mut output_mass: BTreeMap<String, Exact> = BTreeMap::new();
        let mut terminated_without_output_mass = Exact::zero();
        let mut explored = BTreeSet::new();
        for _ in 0..horizon {
            let mut next_frontier: BTreeMap<SymbolicState, Exact> = BTreeMap::new();
            for (state, mass) in frontier {
                explored.insert(state.clone());
                match &self.instructions[&state.control] {
                    SymbolicInstruction::Halt { output } => {
                        let accumulated = output_mass.entry(output.clone()).or_default();
                        *accumulated = accumulated.add(&mass);
                    }
                    _ => {
                        let destinations = self.step(&state)?;
                        let emitted = sum(destinations
                            .iter()
                            .map(|(_, probability)| probability.clone()));
                        terminated_without_output_mass = terminated_without_output_mass
                            .add(&mass.mul(&Exact::one().sub(&emitted)));
                        for (destination, probability) in destinations {
                            let accumulated = next_frontier.entry(destination).or_default();
                            *accumulated = accumulated.add(&mass.mul(&probability));
                        }
                    }
                }
            }
            if next_frontier.len() > maximum_frontier_states {
                bail!("symbolic observer frontier exceeded its declared resource limit");
            }
            frontier = next_frontier;
        }
        let mut remaining_mass = Exact::zero();
        for (state, mass) in frontier {
            explored.insert(state.clone());
            if let SymbolicInstruction::Halt { output } = &self.instructions[&state.control] {
                let accumulated = output_mass.entry(output.clone()).or_default();
                *accumulated = accumulated.add(&mass);
            } else {
                remaining_mass = remaining_mass.add(&mass);
            }
        }
        let total = sum(output_mass.values().cloned())
            .add(&remaining_mass)
            .add(&terminated_without_output_mass);
        Ok(SymbolicTrace {
            input: input.to_string(),
            horizon,
            output_mass,
            remaining_mass,
            terminated_without_output_mass,
            explored_configurations: explored.len(),
            exact_mass_decomposition: total.is_one(),
        })
    }

    pub fn finite_observer_transducer(&self) -> Result<OpenTransducer> {
        let kernel = self.lifted_kernel.as_ref().ok_or_else(|| {
            anyhow::anyhow!("this symbolic program has no finite lifted boundary")
        })?;
        OpenTransducer::from_markov_one_step(format!("finite-view:{}", self.id), kernel)
    }

    pub fn universal_substrate_certificate(&self) -> UniversalSubstrateCertificate {
        UniversalSubstrateCertificate {
            finite_control: true,
            two_unbounded_registers: self.register_count >= 2,
            increment: true,
            conditional_decrement_and_zero_branch: true,
            exact_markov_choice: true,
            lazy_finite_observer_trace: true,
            scope: "The instruction basis is equivalent to a two-counter register machine; universality is a property of the language. A finite run certifies only the observed trace of its particular program.".to_string(),
        }
    }
}
