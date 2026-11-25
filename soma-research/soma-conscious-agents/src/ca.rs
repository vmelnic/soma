use crate::exact::{sum, Exact};
use crate::kernel::{multiply, zeros, MarkovKernel, Matrix, StochasticMap};
use crate::memory::digest_json;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FiniteMeasurableSpace {
    pub id: String,
    pub atoms: Vec<String>,
}

impl FiniteMeasurableSpace {
    pub fn new(id: impl Into<String>, atoms: Vec<String>) -> Result<Self> {
        if atoms.is_empty() {
            bail!("a finite measurable space cannot be empty");
        }
        if atoms.iter().collect::<BTreeSet<_>>().len() != atoms.len() {
            bail!("finite measurable-space atoms must be unique");
        }
        Ok(Self {
            id: id.into(),
            atoms,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsciousAgent {
    pub id: String,
    pub experience: FiniteMeasurableSpace,
    pub choices: FiniteMeasurableSpace,
    pub world: FiniteMeasurableSpace,
    pub perception: StochasticMap,
    pub decision: StochasticMap,
    pub action: StochasticMap,
    pub experience_counter: u64,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCertificate {
    pub agent_id: String,
    pub typed_pda: bool,
    pub closed_pda: bool,
    pub qualia_kernel_digest: String,
    pub q_equals_dap: bool,
    pub provenance_complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CausalPath {
    pub experience: String,
    pub choice: String,
    pub world: String,
    pub next_experience: String,
    pub probability: Exact,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentStepCertificate {
    pub agent_id: String,
    pub source_experience: String,
    pub paths: Vec<CausalPath>,
    pub next_experience_distribution: Vec<Exact>,
    pub equals_qualia_kernel_row: bool,
    pub causal_digest: String,
}

impl ConsciousAgent {
    pub fn validate(&self) -> Result<AgentCertificate> {
        let typed = self.perception.domain == self.world.atoms
            && self.perception.codomain == self.experience.atoms
            && self.decision.domain == self.experience.atoms
            && self.decision.codomain == self.choices.atoms
            && self.action.domain == self.choices.atoms
            && self.action.codomain == self.world.atoms;
        if !typed {
            bail!("conscious-agent P, D, and A maps do not match X, G, and W");
        }
        let closed = [&self.perception, &self.decision, &self.action]
            .into_iter()
            .all(|map| map.transition.iter().all(|row| sum(row.clone()).is_one()));
        if !closed {
            bail!("conscious-agent P, D, and A maps must be Markov, not sub-Markov");
        }
        let q = self.qualia_kernel()?;
        let direct = self
            .decision
            .compose(&self.action)?
            .compose(&self.perception)?;
        let q_equals_dap = direct.domain == q.states
            && direct.codomain == q.states
            && direct.transition == q.transition;
        Ok(AgentCertificate {
            agent_id: self.id.clone(),
            typed_pda: typed,
            closed_pda: closed,
            qualia_kernel_digest: q.stable_digest()?,
            q_equals_dap,
            provenance_complete: self.provenance.iter().all(|item| !item.is_empty()),
        })
    }

    pub fn qualia_kernel(&self) -> Result<MarkovKernel> {
        if self.decision.domain != self.experience.atoms
            || self.decision.codomain != self.action.domain
            || self.action.codomain != self.perception.domain
            || self.perception.codomain != self.experience.atoms
        {
            bail!("cannot compose Q = DAP across incompatible agent spaces");
        }
        let dap = self
            .decision
            .compose(&self.action)?
            .compose(&self.perception)?;
        MarkovKernel::new(self.experience.atoms.clone(), dap.transition)
    }

    pub fn step_certificate(&self, experience: &str) -> Result<AgentStepCertificate> {
        self.validate()?;
        let x = self
            .experience
            .atoms
            .iter()
            .position(|item| item == experience)
            .ok_or_else(|| anyhow::anyhow!("unknown agent experience {experience}"))?;
        let mut paths = Vec::new();
        let mut next = vec![Exact::zero(); self.experience.atoms.len()];
        for (g, p_d) in self.decision.transition[x].iter().enumerate() {
            if p_d.is_zero() {
                continue;
            }
            for (w, p_a) in self.action.transition[g].iter().enumerate() {
                if p_a.is_zero() {
                    continue;
                }
                for (xp, p_p) in self.perception.transition[w].iter().enumerate() {
                    if p_p.is_zero() {
                        continue;
                    }
                    let probability = p_d.mul(p_a).mul(p_p);
                    next[xp] = next[xp].add(&probability);
                    paths.push(CausalPath {
                        experience: self.experience.atoms[x].clone(),
                        choice: self.choices.atoms[g].clone(),
                        world: self.world.atoms[w].clone(),
                        next_experience: self.experience.atoms[xp].clone(),
                        probability,
                    });
                }
            }
        }
        let qualia = self.qualia_kernel()?;
        let equals = next == qualia.transition[x];
        let causal_digest = digest_json(&(
            self.id.as_str(),
            experience,
            &paths,
            &next,
            self.experience_counter,
        ))?;
        Ok(AgentStepCertificate {
            agent_id: self.id.clone(),
            source_experience: experience.to_string(),
            paths,
            next_experience_distribution: next,
            equals_qualia_kernel_row: equals,
            causal_digest,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCompatibility {
    pub shared_world: bool,
    pub synchronized_counter: bool,
    pub joint_action_domain_complete: bool,
    pub joint_action_supported_by_members: bool,
}

impl AgentCompatibility {
    pub fn compatible(&self) -> bool {
        self.shared_world
            && self.synchronized_counter
            && self.joint_action_domain_complete
            && self.joint_action_supported_by_members
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentNetwork {
    pub id: String,
    pub agents: Vec<ConsciousAgent>,
    pub world: FiniteMeasurableSpace,
    /// A network action kernel from the Cartesian product of member choices to
    /// the shared world. Its labels are canonical serialized choice tuples.
    pub joint_action: StochasticMap,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkCertificate {
    pub network_id: String,
    pub member_certificates: Vec<AgentCertificate>,
    pub compatibility: AgentCompatibility,
    pub joint_qualia_kernel_digest: String,
    pub member_count: usize,
    pub provenance_complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FusionCertificate {
    pub network_digest: String,
    pub joint_state_count: usize,
    pub communicating_classes: Vec<Vec<String>>,
    pub algebraic_rank: usize,
    pub idempotent: bool,
}

impl AgentNetwork {
    pub fn join(
        id: impl Into<String>,
        agents: Vec<ConsciousAgent>,
        joint_action: StochasticMap,
        provenance: Vec<String>,
    ) -> Result<(Self, AgentCompatibility)> {
        let world = agents
            .first()
            .map(|agent| agent.world.clone())
            .ok_or_else(|| anyhow::anyhow!("cannot join an empty agent family"))?;
        let network = Self {
            id: id.into(),
            agents,
            world,
            joint_action,
            provenance,
        };
        let certificate = network.validate()?;
        Ok((network, certificate.compatibility))
    }

    pub fn canonical_joint_choices(agents: &[ConsciousAgent]) -> Result<Vec<String>> {
        let choices = cartesian_strings(
            &agents
                .iter()
                .map(|agent| agent.choices.atoms.clone())
                .collect::<Vec<_>>(),
        );
        choices
            .iter()
            .map(|tuple| Ok(serde_json::to_string(tuple)?))
            .collect()
    }

    pub fn canonical_joint_experiences(&self) -> Result<Vec<String>> {
        cartesian_strings(
            &self
                .agents
                .iter()
                .map(|agent| agent.experience.atoms.clone())
                .collect::<Vec<_>>(),
        )
        .iter()
        .map(|tuple| Ok(serde_json::to_string(tuple)?))
        .collect()
    }

    pub fn validate(&self) -> Result<NetworkCertificate> {
        if self.agents.is_empty() {
            bail!("an agent network needs at least one conscious agent");
        }
        let member_certificates = self
            .agents
            .iter()
            .map(ConsciousAgent::validate)
            .collect::<Result<Vec<_>>>()?;
        let shared_world = self.agents.iter().all(|agent| agent.world == self.world);
        let synchronized_counter = self
            .agents
            .iter()
            .map(|agent| agent.experience_counter)
            .collect::<BTreeSet<_>>()
            .len()
            == 1;
        let expected_domain = Self::canonical_joint_choices(&self.agents)?;
        let joint_action_domain_complete = self.joint_action.domain == expected_domain
            && self.joint_action.codomain == self.world.atoms
            && self
                .joint_action
                .transition
                .iter()
                .all(|row| sum(row.clone()).is_one());
        let tuples = cartesian_indices(
            &self
                .agents
                .iter()
                .map(|agent| agent.choices.atoms.len())
                .collect::<Vec<_>>(),
        );
        let joint_action_supported_by_members = joint_action_domain_complete
            && tuples.iter().enumerate().all(|(joint, choices)| {
                self.joint_action.transition[joint].iter().enumerate().all(
                    |(world, probability)| {
                        probability.is_zero()
                            || self.agents.iter().enumerate().any(|(agent, member)| {
                                !member.action.transition[choices[agent]][world].is_zero()
                            })
                    },
                )
            });
        let compatibility = AgentCompatibility {
            shared_world,
            synchronized_counter,
            joint_action_domain_complete,
            joint_action_supported_by_members,
        };
        if !compatibility.compatible() {
            bail!("conscious agents do not satisfy the network join conditions");
        }
        let joint = self.qualia_kernel()?;
        Ok(NetworkCertificate {
            network_id: self.id.clone(),
            member_certificates,
            compatibility,
            joint_qualia_kernel_digest: joint.stable_digest()?,
            member_count: self.agents.len(),
            provenance_complete: self.provenance.iter().all(|item| !item.is_empty()),
        })
    }

    pub fn qualia_kernel(&self) -> Result<MarkovKernel> {
        if self.agents.is_empty() {
            bail!("cannot derive dynamics of an empty agent network");
        }
        let experience_sizes = self
            .agents
            .iter()
            .map(|agent| agent.experience.atoms.len())
            .collect::<Vec<_>>();
        let choice_sizes = self
            .agents
            .iter()
            .map(|agent| agent.choices.atoms.len())
            .collect::<Vec<_>>();
        let experiences = cartesian_indices(&experience_sizes);
        let choices = cartesian_indices(&choice_sizes);
        let labels = self.canonical_joint_experiences()?;
        let mut transition = zeros(experiences.len(), experiences.len());
        for (source_index, source) in experiences.iter().enumerate() {
            for (choice_index, choice) in choices.iter().enumerate() {
                let decision_probability =
                    self.agents
                        .iter()
                        .enumerate()
                        .fold(Exact::one(), |mass, (agent, member)| {
                            mass.mul(&member.decision.transition[source[agent]][choice[agent]])
                        });
                if decision_probability.is_zero() {
                    continue;
                }
                for (world, world_probability) in self.joint_action.transition[choice_index]
                    .iter()
                    .enumerate()
                {
                    if world_probability.is_zero() {
                        continue;
                    }
                    for (destination_index, destination) in experiences.iter().enumerate() {
                        let perception_probability = self.agents.iter().enumerate().fold(
                            Exact::one(),
                            |mass, (agent, member)| {
                                mass.mul(&member.perception.transition[world][destination[agent]])
                            },
                        );
                        let path = decision_probability
                            .mul(world_probability)
                            .mul(&perception_probability);
                        transition[source_index][destination_index] =
                            transition[source_index][destination_index].add(&path);
                    }
                }
            }
        }
        MarkovKernel::new(labels, transition)
    }

    pub fn fusion_certificate(&self) -> Result<FusionCertificate> {
        self.validate()?;
        let kernel = self.qualia_kernel()?;
        let squared = multiply(&kernel.transition, &kernel.transition)?;
        Ok(FusionCertificate {
            network_digest: digest_json(self)?,
            joint_state_count: kernel.states.len(),
            communicating_classes: communicating_classes(&kernel),
            algebraic_rank: exact_rank(&kernel.transition)?,
            idempotent: squared == kernel.transition,
        })
    }
}

fn cartesian_strings(components: &[Vec<String>]) -> Vec<Vec<String>> {
    if components.is_empty() {
        return vec![Vec::new()];
    }
    let mut result = vec![Vec::new()];
    for component in components {
        let mut next = Vec::new();
        for prefix in &result {
            for value in component {
                let mut tuple = prefix.clone();
                tuple.push(value.clone());
                next.push(tuple);
            }
        }
        result = next;
    }
    result
}

fn cartesian_indices(sizes: &[usize]) -> Vec<Vec<usize>> {
    if sizes.is_empty() {
        return vec![Vec::new()];
    }
    let mut result = vec![Vec::new()];
    for size in sizes {
        let mut next = Vec::new();
        for prefix in &result {
            for value in 0..*size {
                let mut tuple = prefix.clone();
                tuple.push(value);
                next.push(tuple);
            }
        }
        result = next;
    }
    result
}

fn communicating_classes(kernel: &MarkovKernel) -> Vec<Vec<String>> {
    let n = kernel.states.len();
    let mut reachable = vec![vec![false; n]; n];
    for (start, reachable_from_start) in reachable.iter_mut().enumerate() {
        let mut queue = VecDeque::from([start]);
        reachable_from_start[start] = true;
        while let Some(from) = queue.pop_front() {
            for (to, probability) in kernel.transition[from].iter().enumerate() {
                if !probability.is_zero() && !reachable_from_start[to] {
                    reachable_from_start[to] = true;
                    queue.push_back(to);
                }
            }
        }
    }
    let mut remaining = (0..n).collect::<BTreeSet<_>>();
    let mut classes = Vec::new();
    while let Some(seed) = remaining.iter().next().copied() {
        let class = remaining
            .iter()
            .filter(|state| reachable[seed][**state] && reachable[**state][seed])
            .copied()
            .collect::<Vec<_>>();
        for state in &class {
            remaining.remove(state);
        }
        classes.push(
            class
                .into_iter()
                .map(|state| kernel.states[state].clone())
                .collect(),
        );
    }
    classes
}

fn exact_rank(matrix: &Matrix) -> Result<usize> {
    if matrix.is_empty() {
        return Ok(0);
    }
    let mut reduced = matrix.clone();
    let rows = reduced.len();
    let columns = reduced[0].len();
    let mut pivot_row = 0;
    for column in 0..columns {
        let Some(nonzero) = (pivot_row..rows).find(|row| !reduced[*row][column].is_zero()) else {
            continue;
        };
        reduced.swap(pivot_row, nonzero);
        let divisor = reduced[pivot_row][column].clone();
        for value in &mut reduced[pivot_row][column..] {
            *value = value.div(&divisor)?;
        }
        let pivot_values = reduced[pivot_row].clone();
        for (row_index, row) in reduced.iter_mut().enumerate().take(rows) {
            if row_index == pivot_row || row[column].is_zero() {
                continue;
            }
            let factor = row[column].clone();
            for c in column..columns {
                row[c] = row[c].sub(&factor.mul(&pivot_values[c]));
            }
        }
        pivot_row += 1;
        if pivot_row == rows {
            break;
        }
    }
    Ok(pivot_row)
}

pub fn deterministic_map(
    domain: Vec<String>,
    codomain: Vec<String>,
    assignments: &BTreeMap<String, String>,
) -> Result<StochasticMap> {
    let target = codomain
        .iter()
        .enumerate()
        .map(|(index, label)| (label.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut transition = zeros(domain.len(), codomain.len());
    for (row, source) in domain.iter().enumerate() {
        let destination = assignments
            .get(source)
            .ok_or_else(|| anyhow::anyhow!("deterministic map has no image for {source}"))?;
        transition[row][*target
            .get(destination.as_str())
            .ok_or_else(|| anyhow::anyhow!("deterministic map targets an unknown atom"))?] =
            Exact::one();
    }
    StochasticMap::new(domain, codomain, transition)
}
