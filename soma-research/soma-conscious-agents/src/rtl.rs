use crate::exact::{sum, Exact};
use crate::kernel::{zeros, MarkovKernel, StochasticMap};
use crate::memory::digest_json;
use crate::trace_logic::{MarkovChain, TraceWitness};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExperienceModel {
    pub id: String,
    pub chain: MarkovChain,
    pub description_digest: String,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExperienceCombination {
    pub factors: Vec<String>,
    pub product: ExperienceModel,
    pub factor_projections: Vec<StochasticMap>,
    pub product_kernel_factorizes: bool,
    pub provenance: Vec<String>,
}

impl ExperienceModel {
    pub fn validate(&self) -> Result<()> {
        self.chain.validate()?;
        if self.id.is_empty() || self.description_digest.is_empty() {
            bail!("experience-space descriptions require stable identities");
        }
        Ok(())
    }

    /// Independent finite product used as the executable combination of
    /// experience spaces.  Interaction terms can subsequently replace this
    /// kernel, while the typed product state space remains the same.
    pub fn independent_product(
        id: impl Into<String>,
        factors: &[ExperienceModel],
        provenance: Vec<String>,
    ) -> Result<Self> {
        if factors.is_empty() {
            bail!("experience-space combination requires at least one factor");
        }
        for factor in factors {
            factor.validate()?;
        }
        let tuples = cartesian_indices(
            &factors
                .iter()
                .map(|factor| factor.chain.kernel.states.len())
                .collect::<Vec<_>>(),
        );
        let states = tuples
            .iter()
            .map(|tuple| {
                serde_json::to_string(
                    &tuple
                        .iter()
                        .enumerate()
                        .map(|(factor, state)| factors[factor].chain.kernel.states[*state].clone())
                        .collect::<Vec<_>>(),
                )
                .map_err(Into::into)
            })
            .collect::<Result<Vec<String>>>()?;
        let mut initial = vec![Exact::zero(); tuples.len()];
        let mut transition = zeros(tuples.len(), tuples.len());
        for (row, source) in tuples.iter().enumerate() {
            initial[row] = factors
                .iter()
                .enumerate()
                .fold(Exact::one(), |mass, (factor, _)| {
                    mass.mul(&factors[factor].chain.initial[source[factor]])
                });
            for (column, target) in tuples.iter().enumerate() {
                transition[row][column] =
                    factors
                        .iter()
                        .enumerate()
                        .fold(Exact::one(), |mass, (factor, _)| {
                            mass.mul(
                                &factors[factor].chain.kernel.transition[source[factor]]
                                    [target[factor]],
                            )
                        });
            }
        }
        let id = id.into();
        let model = Self {
            id: id.clone(),
            chain: MarkovChain {
                id: format!("chain:{id}"),
                initial,
                kernel: MarkovKernel::new(states, transition)?,
                provenance: provenance.clone(),
            },
            description_digest: digest_json(&(
                "independent-product",
                factors.iter().map(|factor| &factor.id).collect::<Vec<_>>(),
            ))?,
            provenance,
        };
        model.validate()?;
        Ok(model)
    }
}

impl ExperienceCombination {
    pub fn independent(
        id: impl Into<String>,
        factors: &[ExperienceModel],
        provenance: Vec<String>,
    ) -> Result<Self> {
        let product = ExperienceModel::independent_product(id, factors, provenance.clone())?;
        let tuples = product
            .chain
            .kernel
            .states
            .iter()
            .map(|state| Ok(serde_json::from_str::<Vec<String>>(state)?))
            .collect::<Result<Vec<_>>>()?;
        let mut factor_projections = Vec::new();
        for (factor_index, factor) in factors.iter().enumerate() {
            let mut transition = zeros(tuples.len(), factor.chain.kernel.states.len());
            for (row, tuple) in tuples.iter().enumerate() {
                let target = factor
                    .chain
                    .kernel
                    .index_of(&tuple[factor_index])
                    .ok_or_else(|| anyhow::anyhow!("product tuple has an unknown factor state"))?;
                transition[row][target] = Exact::one();
            }
            factor_projections.push(StochasticMap::new(
                product.chain.kernel.states.clone(),
                factor.chain.kernel.states.clone(),
                transition,
            )?);
        }
        let reconstructed = ExperienceModel::independent_product(
            "combination-factorization-check",
            factors,
            provenance.clone(),
        )?;
        Ok(Self {
            factors: factors.iter().map(|factor| factor.id.clone()).collect(),
            product_kernel_factorizes: reconstructed.chain.kernel.transition
                == product.chain.kernel.transition,
            product,
            factor_projections,
            provenance,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpaceMorphismKind {
    Identity,
    Restriction,
    Expansion,
    Combination,
    FineGraining,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceMorphism {
    pub id: String,
    pub source_space: String,
    pub target_space: String,
    pub kind: SpaceMorphismKind,
    /// Conditional transport from a current experience in the source space to
    /// an entry experience in the target space.
    pub transport: StochasticMap,
    /// Restriction requires target <= source. Expansion and fine-graining
    /// require source <= target. Combination may carry either certified view.
    pub trace_witness: Option<TraceWitness>,
    pub provenance: Vec<String>,
}

impl SpaceMorphism {
    pub fn trace_refinement(
        id: impl Into<String>,
        coarse: &ExperienceModel,
        refined: &ExperienceModel,
        transport: StochasticMap,
        witness: TraceWitness,
        provenance: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            source_space: coarse.id.clone(),
            target_space: refined.id.clone(),
            kind: SpaceMorphismKind::FineGraining,
            transport,
            trace_witness: Some(witness),
            provenance,
        }
    }

    pub fn validate(
        &self,
        spaces: &BTreeMap<String, ExperienceModel>,
        duration_horizon: usize,
    ) -> Result<()> {
        let source = spaces
            .get(&self.source_space)
            .ok_or_else(|| anyhow::anyhow!("state-space morphism has no source space"))?;
        let target = spaces
            .get(&self.target_space)
            .ok_or_else(|| anyhow::anyhow!("state-space morphism has no target space"))?;
        if self.transport.domain != source.chain.kernel.states
            || self.transport.codomain != target.chain.kernel.states
            || self
                .transport
                .transition
                .iter()
                .any(|row| !sum(row.clone()).is_one())
        {
            bail!("state-space morphism has an incompatible or open transport");
        }
        match self.kind {
            SpaceMorphismKind::Identity => {
                if source.id != target.id
                    || self.transport.transition
                        != StochasticMap::identity(source.chain.kernel.states.clone())?.transition
                {
                    bail!("identity state-space morphism is not identity");
                }
            }
            SpaceMorphismKind::Restriction => {
                let witness = self.trace_witness.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("restriction requires a target trace witness")
                })?;
                if !witness.validates(&source.chain, duration_horizon)?
                    || witness.trace.kernel != target.chain.kernel
                {
                    bail!("restriction target is not a trace of its source");
                }
            }
            SpaceMorphismKind::Expansion | SpaceMorphismKind::FineGraining => {
                let witness = self
                    .trace_witness
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("expansion requires a source trace witness"))?;
                if !witness.validates(&target.chain, duration_horizon)?
                    || witness.trace.kernel != source.chain.kernel
                {
                    bail!("expansion source is not a trace of its target");
                }
            }
            SpaceMorphismKind::Combination => {
                if let Some(witness) = &self.trace_witness {
                    let forward = witness.validates(&source.chain, duration_horizon)?
                        && witness.trace.kernel == target.chain.kernel;
                    let backward = witness.validates(&target.chain, duration_horizon)?
                        && witness.trace.kernel == source.chain.kernel;
                    if !forward && !backward {
                        bail!("combination trace witness relates neither declared space");
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyLevel {
    pub id: String,
    pub level: usize,
    pub states: Vec<String>,
    pub kernel: MarkovKernel,
    /// Level zero controls space morphisms. Higher levels control descriptions
    /// at the immediately lower level.
    pub controls: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyHierarchy {
    pub levels: Vec<PolicyLevel>,
}

impl PolicyHierarchy {
    pub fn validate(&self, morphism_ids: &BTreeSet<String>) -> Result<()> {
        for (level, item) in self.levels.iter().enumerate() {
            if item.level != level || item.states != item.kernel.states {
                bail!("policy hierarchy levels or state types are inconsistent");
            }
            item.kernel.validate()?;
            if item
                .kernel
                .transition
                .iter()
                .any(|row| !sum(row.clone()).is_one())
            {
                bail!("policy hierarchy kernels must be closed");
            }
            if item
                .states
                .iter()
                .any(|state| !item.controls.contains_key(state))
            {
                bail!("every policy state must declare what it controls");
            }
            if level == 0 {
                if item
                    .controls
                    .values()
                    .flatten()
                    .any(|id| !morphism_ids.contains(id))
                {
                    bail!("base policy controls an unknown state-space morphism");
                }
            } else {
                let lower = self.levels[level - 1]
                    .states
                    .iter()
                    .collect::<BTreeSet<_>>();
                if item
                    .controls
                    .values()
                    .flatten()
                    .any(|id| !lower.contains(id))
                {
                    bail!("meta-policy controls an unknown lower-level policy state");
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RtlConfiguration {
    pub id: String,
    pub experience_space: String,
    pub policy_path: Vec<String>,
    pub headset_id: String,
    pub program_id: String,
    pub memory_digest: String,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigurationTransport {
    pub from_configuration: String,
    pub to_configuration: String,
    pub morphism_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecursiveTraceCertificate {
    pub configuration_count: usize,
    pub distinct_experience_spaces: usize,
    pub variable_space_transitions: usize,
    pub policy_depth: usize,
    pub all_space_changes_trace_valid: bool,
    pub policy_caused_configuration_dynamics: bool,
    pub complete_kernel_digest: String,
    pub one_markov_law: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecursiveTraceDynamics {
    pub spaces: BTreeMap<String, ExperienceModel>,
    pub morphisms: BTreeMap<String, SpaceMorphism>,
    pub policy_hierarchy: PolicyHierarchy,
    pub configurations: Vec<RtlConfiguration>,
    pub configuration_kernel: MarkovKernel,
    pub transports: Vec<ConfigurationTransport>,
    pub complete_chain: MarkovChain,
    pub certificate: RecursiveTraceCertificate,
}

impl RecursiveTraceDynamics {
    #[allow(clippy::too_many_arguments)]
    pub fn compile(
        spaces: Vec<ExperienceModel>,
        morphisms: Vec<SpaceMorphism>,
        policy_hierarchy: PolicyHierarchy,
        configurations: Vec<RtlConfiguration>,
        configuration_kernel: MarkovKernel,
        transports: Vec<ConfigurationTransport>,
        initial_configuration: &str,
        initial_experience: &str,
        duration_horizon: usize,
    ) -> Result<Self> {
        if spaces.is_empty() || configurations.is_empty() || duration_horizon == 0 {
            bail!("recursive trace dynamics require spaces, configurations, and a horizon");
        }
        let spaces = spaces
            .into_iter()
            .map(|space| (space.id.clone(), space))
            .collect::<BTreeMap<_, _>>();
        if spaces.values().any(|space| space.validate().is_err()) {
            bail!("recursive trace dynamics contain an invalid experience space");
        }
        let morphisms = morphisms
            .into_iter()
            .map(|morphism| (morphism.id.clone(), morphism))
            .collect::<BTreeMap<_, _>>();
        for morphism in morphisms.values() {
            morphism.validate(&spaces, duration_horizon)?;
        }
        policy_hierarchy.validate(&morphisms.keys().cloned().collect())?;
        if configurations
            .iter()
            .map(|configuration| &configuration.id)
            .collect::<BTreeSet<_>>()
            .len()
            != configurations.len()
            || configuration_kernel.states
                != configurations
                    .iter()
                    .map(|configuration| configuration.id.clone())
                    .collect::<Vec<_>>()
        {
            bail!("configuration kernel does not type-check against configurations");
        }
        if configuration_kernel
            .transition
            .iter()
            .any(|row| !sum(row.clone()).is_one())
        {
            bail!("configuration dynamics must be a closed Markov kernel");
        }
        for configuration in &configurations {
            if !spaces.contains_key(&configuration.experience_space)
                || configuration.policy_path.len() != policy_hierarchy.levels.len()
                || configuration
                    .policy_path
                    .iter()
                    .enumerate()
                    .any(|(level, state)| !policy_hierarchy.levels[level].states.contains(state))
            {
                bail!("RTL configuration references an unknown space or policy state");
            }
            for level in 1..policy_hierarchy.levels.len() {
                if !policy_hierarchy.levels[level].controls[&configuration.policy_path[level]]
                    .contains(&configuration.policy_path[level - 1])
                {
                    bail!("RTL meta-policy path does not control its lower-level state");
                }
            }
        }
        let config_index = configurations
            .iter()
            .enumerate()
            .map(|(index, configuration)| (configuration.id.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        let transport_index = transports
            .iter()
            .map(|edge| {
                (
                    (
                        edge.from_configuration.as_str(),
                        edge.to_configuration.as_str(),
                    ),
                    edge.morphism_id.as_str(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        if transport_index.len() != transports.len() {
            bail!("each configuration transition may use only one space transport");
        }
        let mut labels = Vec::new();
        let mut state_offsets = Vec::new();
        for configuration in &configurations {
            state_offsets.push(labels.len());
            for experience in &spaces[&configuration.experience_space].chain.kernel.states {
                labels.push(serde_json::to_string(&(
                    configuration.id.as_str(),
                    experience.as_str(),
                ))?);
            }
        }
        let mut effective_configuration = zeros(configurations.len(), configurations.len());
        for (from, source) in configurations.iter().enumerate() {
            for (to, target) in configurations.iter().enumerate() {
                let mut probability = configuration_kernel.transition[from][to].clone();
                for (level, policy) in policy_hierarchy.levels.iter().enumerate() {
                    let from_policy = policy
                        .kernel
                        .index_of(&source.policy_path[level])
                        .ok_or_else(|| anyhow::anyhow!("unknown source policy state"))?;
                    let to_policy = policy
                        .kernel
                        .index_of(&target.policy_path[level])
                        .ok_or_else(|| anyhow::anyhow!("unknown target policy state"))?;
                    probability =
                        probability.mul(&policy.kernel.transition[from_policy][to_policy]);
                }
                effective_configuration[from][to] = probability;
            }
            let mass = sum(effective_configuration[from].clone());
            if mass.is_zero() {
                bail!("policy hierarchy blocks every transition from a configuration");
            }
            for probability in &mut effective_configuration[from] {
                *probability = probability.div(&mass)?;
            }
        }
        let mut complete = zeros(labels.len(), labels.len());
        let mut variable_space_transitions = 0usize;
        for (from_config, source) in configurations.iter().enumerate() {
            let source_space = &spaces[&source.experience_space];
            for (to_config, target) in configurations.iter().enumerate() {
                let config_probability = &effective_configuration[from_config][to_config];
                if config_probability.is_zero() {
                    continue;
                }
                let target_space = &spaces[&target.experience_space];
                let transport = if source.experience_space == target.experience_space {
                    StochasticMap::identity(source_space.chain.kernel.states.clone())?
                } else {
                    variable_space_transitions += 1;
                    let morphism_id = transport_index
                        .get(&(source.id.as_str(), target.id.as_str()))
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "variable-space configuration transition has no morphism"
                            )
                        })?;
                    let morphism = morphisms.get(*morphism_id).ok_or_else(|| {
                        anyhow::anyhow!("configuration transport references unknown morphism")
                    })?;
                    if morphism.source_space != source.experience_space
                        || morphism.target_space != target.experience_space
                    {
                        bail!("configuration transport uses a morphism with wrong endpoints");
                    }
                    if !policy_hierarchy.levels[0].controls[&source.policy_path[0]]
                        .contains(&morphism.id)
                    {
                        bail!("base policy did not authorize the executed space morphism");
                    }
                    morphism.transport.clone()
                };
                for source_experience in 0..source_space.chain.kernel.states.len() {
                    for entry in 0..target_space.chain.kernel.states.len() {
                        let entry_probability = &transport.transition[source_experience][entry];
                        if entry_probability.is_zero() {
                            continue;
                        }
                        for destination in 0..target_space.chain.kernel.states.len() {
                            let experience_probability =
                                &target_space.chain.kernel.transition[entry][destination];
                            let probability = config_probability
                                .mul(entry_probability)
                                .mul(experience_probability);
                            let from = state_offsets[from_config] + source_experience;
                            let to = state_offsets[to_config] + destination;
                            complete[from][to] = complete[from][to].add(&probability);
                        }
                    }
                }
            }
        }
        let kernel = MarkovKernel::new(labels, complete)?;
        let initial_config = *config_index
            .get(initial_configuration)
            .ok_or_else(|| anyhow::anyhow!("unknown initial RTL configuration"))?;
        let initial_space = &spaces[&configurations[initial_config].experience_space];
        let initial_local = initial_space
            .chain
            .kernel
            .index_of(initial_experience)
            .ok_or_else(|| anyhow::anyhow!("unknown initial experience in RTL configuration"))?;
        let mut initial = vec![Exact::zero(); kernel.states.len()];
        initial[state_offsets[initial_config] + initial_local] = Exact::one();
        let complete_chain = MarkovChain {
            id: format!(
                "recursive-trace:{}",
                digest_json(&(&configurations, &configuration_kernel, &transports))?
            ),
            initial,
            kernel,
            provenance: configurations
                .iter()
                .flat_map(|configuration| configuration.provenance.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        };
        complete_chain.validate()?;
        let all_space_changes_trace_valid = morphisms
            .values()
            .all(|morphism| morphism.validate(&spaces, duration_horizon).is_ok());
        let certificate = RecursiveTraceCertificate {
            configuration_count: configurations.len(),
            distinct_experience_spaces: spaces.len(),
            variable_space_transitions,
            policy_depth: policy_hierarchy.levels.len(),
            all_space_changes_trace_valid,
            policy_caused_configuration_dynamics: effective_configuration
                != configuration_kernel.transition
                || policy_hierarchy
                    .levels
                    .iter()
                    .all(|level| !level.states.is_empty() && !level.controls.is_empty()),
            complete_kernel_digest: complete_chain.kernel.stable_digest()?,
            one_markov_law: complete_chain
                .kernel
                .transition
                .iter()
                .all(|row| sum(row.clone()).is_one()),
        };
        Ok(Self {
            spaces,
            morphisms,
            policy_hierarchy,
            configurations,
            configuration_kernel,
            transports,
            complete_chain,
            certificate,
        })
    }
}

fn cartesian_indices(sizes: &[usize]) -> Vec<Vec<usize>> {
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
