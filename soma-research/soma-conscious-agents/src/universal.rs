use crate::active::{ActiveWorldModel, GoalHypothesis};
use crate::memory::digest_json;
use crate::relational::{Atom, Database, Relation};
use crate::world::RelationalFrame;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

type IndexedKey = Vec<usize>;
type IndexedValues = BTreeMap<IndexedKey, Atom>;
type QuotientRepairCandidate = (usize, usize, usize, Vec<usize>, IndexedValues);

/// Domain-neutral types shared by static transduction and interactive worlds.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreType {
    Atom,
    Carrier {
        fields: Vec<String>,
    },
    Relation {
        fields: Vec<String>,
    },
    Structure {
        relations: BTreeMap<String, Vec<String>>,
    },
    Morphism {
        source: Box<CoreType>,
        target: Box<CoreType>,
    },
    Kernel {
        source: Box<CoreType>,
        target: Box<CoreType>,
    },
    Trace {
        state: Box<CoreType>,
        action: Box<CoreType>,
        observation: Box<CoreType>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "expression", rename_all = "snake_case")]
pub enum ValueTerm {
    Field {
        name: String,
    },
    Literal {
        value: Atom,
    },
    Add {
        left: Box<ValueTerm>,
        right: Box<ValueTerm>,
    },
    Subtract {
        left: Box<ValueTerm>,
        right: Box<ValueTerm>,
    },
    Minimum {
        left: Box<ValueTerm>,
        right: Box<ValueTerm>,
    },
    Maximum {
        left: Box<ValueTerm>,
        right: Box<ValueTerm>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "predicate", rename_all = "snake_case")]
pub enum StructuralPredicate {
    True,
    Equal {
        left: ValueTerm,
        right: ValueTerm,
    },
    LessThan {
        left: ValueTerm,
        right: ValueTerm,
    },
    InRelation {
        relation: String,
        fields: Vec<(String, String)>,
    },
    And {
        terms: Vec<StructuralPredicate>,
    },
    Or {
        terms: Vec<StructuralPredicate>,
    },
    Not {
        term: Box<StructuralPredicate>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "carrier", rename_all = "snake_case")]
pub enum CarrierTerm {
    Boundary {
        relation: String,
    },
    Constructed {
        fields: Vec<String>,
    },
    Subobject {
        source: Box<CarrierTerm>,
        predicate: StructuralPredicate,
    },
    Partition {
        source: Box<CarrierTerm>,
        keys: Vec<String>,
    },
    Quotient {
        source: Box<CarrierTerm>,
        keys: Vec<String>,
    },
    Product {
        factors: Vec<CarrierTerm>,
    },
    Coproduct {
        summands: Vec<CarrierTerm>,
    },
    Image {
        source: Box<CarrierTerm>,
        morphism: Box<MorphismTerm>,
    },
    Preimage {
        target: Box<CarrierTerm>,
        morphism: Box<MorphismTerm>,
    },
    Pullback {
        left: Box<CarrierTerm>,
        right: Box<CarrierTerm>,
        on: Vec<(String, String)>,
    },
    Pushout {
        left: Box<CarrierTerm>,
        right: Box<CarrierTerm>,
        keys: Vec<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "morphism", rename_all = "snake_case")]
pub enum MorphismTerm {
    Identity,
    Project {
        fields: Vec<String>,
    },
    Rename {
        mapping: BTreeMap<String, String>,
    },
    Derive {
        fields: Vec<(String, ValueTerm)>,
    },
    Compose {
        first: Box<MorphismTerm>,
        second: Box<MorphismTerm>,
    },
    Product {
        factors: Vec<MorphismTerm>,
    },
    Coproduct {
        branches: Vec<MorphismTerm>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AggregateOperator {
    Count,
    Minimum,
    Maximum,
    Consensus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AggregateTerm {
    pub output: String,
    pub input: Option<String>,
    pub operator: AggregateOperator,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntityTraceMacro {
    pub value: Atom,
    pub normalized_support: Vec<Vec<usize>>,
    pub paint_offsets: Vec<Vec<i64>>,
    pub cursor_delta: Vec<i64>,
}

/// Guarded recursion uses `$current`; composition uses `$input`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "term", rename_all = "snake_case")]
pub enum CoreTerm {
    Source {
        relation: String,
    },
    Literal {
        relation: Relation,
    },
    Select {
        input: Box<CoreTerm>,
        predicate: StructuralPredicate,
    },
    Image {
        input: Box<CoreTerm>,
        morphism: MorphismTerm,
    },
    Join {
        left: Box<CoreTerm>,
        right: Box<CoreTerm>,
        on: Vec<(String, String)>,
    },
    Union {
        left: Box<CoreTerm>,
        right: Box<CoreTerm>,
    },
    Difference {
        left: Box<CoreTerm>,
        right: Box<CoreTerm>,
    },
    Group {
        input: Box<CoreTerm>,
        keys: Vec<String>,
        aggregates: Vec<AggregateTerm>,
    },
    TransitiveClosure {
        input: Box<CoreTerm>,
        from: String,
        to: String,
    },
    Overlay {
        base: Box<CoreTerm>,
        layer: Box<CoreTerm>,
        keys: Vec<String>,
        prefer_layer: bool,
    },
    Fold {
        input: Box<CoreTerm>,
        keys: Vec<String>,
        value: String,
        consensus: bool,
    },
    /// Complete a Cartesian subobject whose value is explicitly unavailable by
    /// transporting values along the best supported signed axis isomorphism.
    /// The operator is rank-generic and derives both the subobject and the
    /// correspondence from the supplied finite relation.
    IsomorphicCompletion {
        input: Box<CoreTerm>,
        keys: Vec<String>,
        value: String,
        unavailable: Atom,
    },
    /// Partition a finite product by homogeneous hyperplanes, peel supported
    /// homogeneous frames, and repair each remaining region by its minimum
    /// description-length translation quotient.
    QuotientRepair {
        input: Box<CoreTerm>,
        keys: Vec<String>,
        value: String,
    },
    /// Extract connected same-value entities on one side of a homogeneous
    /// separator, order them by an induced axis order, and execute learned
    /// entity-class trace macros from the marker in the opposite carrier.
    EntityTrace {
        input: Box<CoreTerm>,
        keys: Vec<String>,
        value: String,
        split_axis: usize,
        canvas_high: bool,
        order: Vec<usize>,
        macros: Vec<EntityTraceMacro>,
    },
    Scan {
        seed: Box<CoreTerm>,
        relation: String,
        from: String,
        to: String,
    },
    LeastFixedPoint {
        seed: Box<CoreTerm>,
        step: Box<CoreTerm>,
        maximum_iterations: usize,
    },
    Compose {
        first: Box<CoreTerm>,
        second: Box<CoreTerm>,
    },
    Abstract {
        parameters: Vec<String>,
        body: Box<CoreTerm>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgramBinding {
    pub name: String,
    pub term: CoreTerm,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TypedCoreProgram {
    pub input_type: CoreType,
    pub output_type: CoreType,
    pub bindings: Vec<ProgramBinding>,
    pub outputs: BTreeMap<String, CoreTerm>,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoleHypothesis {
    pub name: String,
    pub carrier: CarrierTerm,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorrespondenceHypothesis {
    pub observation: CarrierTerm,
    pub action: CarrierTerm,
    pub span: MorphismTerm,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeadsetProgram {
    pub observation_carriers: Vec<CarrierTerm>,
    pub action_carriers: Vec<CarrierTerm>,
    pub perception: TypedCoreProgram,
    pub action: TypedCoreProgram,
    pub roles: Vec<RoleHypothesis>,
    pub correspondences: Vec<CorrespondenceHypothesis>,
    pub evidence_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MacroDefinition {
    pub id: String,
    pub parameters: BTreeMap<String, CoreType>,
    pub body: TypedCoreProgram,
    pub support_digests: Vec<String>,
    pub held_out_gain_bits: i64,
}

impl MacroDefinition {
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() || self.support_digests.len() < 2 || self.held_out_gain_bits <= 0 {
            bail!("a macro requires independent reuse and positive held-out gain");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JointStructuralHypothesis {
    pub headset: HeadsetProgram,
    pub program: TypedCoreProgram,
    pub latent_state_type: CoreType,
    pub transition_type: CoreType,
    pub goal_type: CoreType,
    pub policy_type: CoreType,
    pub latent_dynamics: ActiveWorldModel,
    pub goal_hypotheses: Vec<GoalHypothesis>,
    pub reusable_abstractions: Vec<MacroDefinition>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreExecution {
    pub database: Database,
    pub outputs: Database,
    pub iterations: usize,
    pub trace_digests: Vec<String>,
    pub execution_digest: String,
}

impl TypedCoreProgram {
    pub fn identity(frame: &RelationalFrame) -> Self {
        let relations = schemas(&frame.relations);
        Self {
            input_type: CoreType::Structure {
                relations: relations.clone(),
            },
            output_type: CoreType::Structure {
                relations: relations.clone(),
            },
            bindings: Vec::new(),
            outputs: relations
                .keys()
                .map(|name| {
                    (
                        name.clone(),
                        CoreTerm::Source {
                            relation: name.clone(),
                        },
                    )
                })
                .collect(),
            provenance: frame.provenance.clone(),
        }
    }

    pub fn validate(&self, source: &Database) -> Result<()> {
        let mut known = schemas(source);
        for binding in &self.bindings {
            if known.contains_key(&binding.name) || is_bound_name(&binding.name) {
                bail!("program binding {} is reserved or duplicated", binding.name);
            }
            let schema = infer_schema(&binding.term, &known, &BTreeMap::new())?;
            known.insert(binding.name.clone(), schema);
        }
        if self.outputs.is_empty() {
            bail!("typed program has no output relations")
        }
        for (name, term) in &self.outputs {
            if name.is_empty() || is_bound_name(name) {
                bail!("invalid output relation name")
            }
            infer_schema(term, &known, &BTreeMap::new())?;
        }
        Ok(())
    }

    pub fn execute(&self, source: &Database, maximum_iterations: usize) -> Result<CoreExecution> {
        if maximum_iterations == 0 {
            bail!("execution iteration limit must be positive")
        }
        self.validate(source)?;
        let mut database = source.clone();
        let mut trace = Vec::new();
        let mut iterations = 0usize;
        for binding in &self.bindings {
            let (relation, used) = eval(&binding.term, &database, maximum_iterations, &mut trace)?;
            iterations = iterations.saturating_add(used);
            database.insert(binding.name.clone(), relation);
        }
        let mut outputs = Database::new();
        for (name, term) in &self.outputs {
            let (relation, used) = eval(term, &database, maximum_iterations, &mut trace)?;
            iterations = iterations.saturating_add(used);
            outputs.insert(name.clone(), relation);
        }
        let execution_digest = digest_json(&(self, source, &outputs, &trace))?;
        Ok(CoreExecution {
            database,
            outputs,
            iterations,
            trace_digests: trace,
            execution_digest,
        })
    }

    pub fn stable_digest(&self) -> Result<String> {
        digest_json(self)
    }
}

impl HeadsetProgram {
    pub fn boundary_identity(frame: &RelationalFrame) -> Result<Self> {
        if frame.relations.is_empty() {
            bail!("cannot construct a headset for an empty boundary")
        }
        let program = TypedCoreProgram::identity(frame);
        Ok(Self {
            observation_carriers: frame
                .relations
                .keys()
                .map(|relation| CarrierTerm::Boundary {
                    relation: relation.clone(),
                })
                .collect(),
            action_carriers: Vec::new(),
            perception: program.clone(),
            action: program,
            roles: Vec::new(),
            correspondences: Vec::new(),
            evidence_digest: frame.payload_digest.clone(),
        })
    }

    pub fn perceive(
        &self,
        frame: &RelationalFrame,
        maximum_iterations: usize,
    ) -> Result<CoreExecution> {
        for carrier in &self.observation_carriers {
            validate_carrier(carrier, &schemas(&frame.relations))?;
        }
        self.perception
            .execute(&frame.relations, maximum_iterations)
    }

    pub fn emit(&self, experience: &Database, maximum_iterations: usize) -> Result<CoreExecution> {
        self.action.execute(experience, maximum_iterations)
    }

    pub fn stable_digest(&self) -> Result<String> {
        digest_json(self)
    }
}

pub fn schemas(database: &Database) -> BTreeMap<String, Vec<String>> {
    database
        .iter()
        .map(|(name, relation)| (name.clone(), relation.attributes.clone()))
        .collect()
}

pub fn validate_carrier(
    term: &CarrierTerm,
    source: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>> {
    Ok(match term {
        CarrierTerm::Boundary { relation } => source
            .get(relation)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown boundary relation {relation}"))?,
        CarrierTerm::Constructed { fields } => unique_fields(fields)?,
        CarrierTerm::Subobject {
            source: inner,
            predicate,
        } => {
            let fields = validate_carrier(inner, source)?;
            validate_predicate(predicate, &fields, source)?;
            fields
        }
        CarrierTerm::Partition {
            source: inner,
            keys,
        }
        | CarrierTerm::Quotient {
            source: inner,
            keys,
        } => {
            let fields = validate_carrier(inner, source)?;
            require_fields(&fields, keys)?;
            unique_fields(keys)?
        }
        CarrierTerm::Product { factors } => product_carrier(factors, source)?,
        CarrierTerm::Coproduct { summands } => {
            let mut items = summands.iter();
            let first = items
                .next()
                .ok_or_else(|| anyhow::anyhow!("empty coproduct"))?;
            let schema = validate_carrier(first, source)?;
            for item in items {
                if validate_carrier(item, source)? != schema {
                    bail!("coproduct carrier schemas differ")
                }
            }
            schema
        }
        CarrierTerm::Image {
            source: inner,
            morphism,
        } => morphism_schema(&validate_carrier(inner, source)?, morphism)?,
        CarrierTerm::Preimage { target, morphism } => {
            morphism_schema(&validate_carrier(target, source)?, morphism)?
        }
        CarrierTerm::Pullback { left, right, on } => join_schema(
            &validate_carrier(left, source)?,
            &validate_carrier(right, source)?,
            on,
        )?,
        CarrierTerm::Pushout { left, right, keys } => {
            let a = validate_carrier(left, source)?;
            let b = validate_carrier(right, source)?;
            if a != b {
                bail!("pushout schemas differ")
            }
            require_fields(&a, keys)?;
            a
        }
    })
}

pub fn infer_schema(
    term: &CoreTerm,
    source: &BTreeMap<String, Vec<String>>,
    bound: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>> {
    Ok(match term {
        CoreTerm::Source { relation } => source
            .get(relation)
            .or_else(|| bound.get(relation))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown source relation {relation}"))?,
        CoreTerm::Literal { relation } => relation.attributes.clone(),
        CoreTerm::Select { input, predicate } => {
            let fields = infer_schema(input, source, bound)?;
            validate_predicate(predicate, &fields, source)?;
            fields
        }
        CoreTerm::Image { input, morphism } => {
            morphism_schema(&infer_schema(input, source, bound)?, morphism)?
        }
        CoreTerm::Join { left, right, on } => join_schema(
            &infer_schema(left, source, bound)?,
            &infer_schema(right, source, bound)?,
            on,
        )?,
        CoreTerm::Union { left, right } | CoreTerm::Difference { left, right } => {
            let a = infer_schema(left, source, bound)?;
            let b = infer_schema(right, source, bound)?;
            if a != b {
                bail!("set-operation schemas differ")
            }
            a
        }
        CoreTerm::Group {
            input,
            keys,
            aggregates,
        } => {
            let fields = infer_schema(input, source, bound)?;
            require_fields(&fields, keys)?;
            for aggregate in aggregates {
                if let Some(input) = &aggregate.input {
                    require_fields(&fields, std::slice::from_ref(input))?;
                }
            }
            let mut output = keys.clone();
            output.extend(aggregates.iter().map(|item| item.output.clone()));
            unique_fields(&output)?
        }
        CoreTerm::TransitiveClosure { input, from, to } => {
            let fields = infer_schema(input, source, bound)?;
            if fields.len() != 2 {
                bail!("transitive closure requires a binary relation")
            }
            require_fields(&fields, &[from.clone(), to.clone()])?;
            fields
        }
        CoreTerm::Overlay {
            base, layer, keys, ..
        } => {
            let a = infer_schema(base, source, bound)?;
            let b = infer_schema(layer, source, bound)?;
            if a != b {
                bail!("overlay schemas differ")
            }
            require_fields(&a, keys)?;
            a
        }
        CoreTerm::Fold {
            input, keys, value, ..
        } => {
            let fields = infer_schema(input, source, bound)?;
            require_fields(&fields, keys)?;
            require_fields(&fields, std::slice::from_ref(value))?;
            let mut output = keys.clone();
            output.push(value.clone());
            unique_fields(&output)?
        }
        CoreTerm::IsomorphicCompletion {
            input, keys, value, ..
        } => {
            let fields = infer_schema(input, source, bound)?;
            require_fields(&fields, keys)?;
            require_fields(&fields, std::slice::from_ref(value))?;
            if keys.len() + 1 != fields.len() || keys.is_empty() {
                bail!("isomorphic completion requires only key and value fields")
            }
            let mut output = keys.clone();
            output.push(value.clone());
            unique_fields(&output)?
        }
        CoreTerm::QuotientRepair { input, keys, value } => {
            let fields = infer_schema(input, source, bound)?;
            require_fields(&fields, keys)?;
            require_fields(&fields, std::slice::from_ref(value))?;
            if keys.len() + 1 != fields.len() || keys.is_empty() {
                bail!("quotient repair requires only key and value fields")
            }
            fields
        }
        CoreTerm::EntityTrace {
            input,
            keys,
            value,
            split_axis,
            order,
            macros,
            ..
        } => {
            let fields = infer_schema(input, source, bound)?;
            require_fields(&fields, keys)?;
            require_fields(&fields, std::slice::from_ref(value))?;
            if keys.len() + 1 != fields.len()
                || *split_axis >= keys.len()
                || order.len() != keys.len()
                || order.iter().copied().collect::<BTreeSet<_>>().len() != keys.len()
                || order.iter().any(|axis| *axis >= keys.len())
                || macros.is_empty()
            {
                bail!("entity trace has an invalid product signature")
            }
            for item in macros {
                if item.cursor_delta.len() != keys.len()
                    || item.paint_offsets.is_empty()
                    || item
                        .paint_offsets
                        .iter()
                        .any(|offset| offset.len() != keys.len())
                {
                    bail!("entity trace macro has an invalid rank")
                }
            }
            fields
        }
        CoreTerm::Scan {
            seed,
            relation,
            from,
            to,
        } => {
            let fields = infer_schema(seed, source, bound)?;
            if fields.len() != 1 {
                bail!("scan seed must be unary")
            }
            let edges = source
                .get(relation)
                .ok_or_else(|| anyhow::anyhow!("scan relation absent"))?;
            require_fields(edges, &[from.clone(), to.clone()])?;
            fields
        }
        CoreTerm::LeastFixedPoint {
            seed,
            step,
            maximum_iterations,
        } => {
            if *maximum_iterations == 0 {
                bail!("fixed point must be bounded")
            }
            let fields = infer_schema(seed, source, bound)?;
            let mut nested = bound.clone();
            nested.insert("$current".into(), fields.clone());
            let next = infer_schema(step, source, &nested)?;
            if fields != next {
                bail!("fixed-point schemas differ")
            }
            fields
        }
        CoreTerm::Compose { first, second } => {
            let fields = infer_schema(first, source, bound)?;
            let mut nested = bound.clone();
            nested.insert("$input".into(), fields);
            infer_schema(second, source, &nested)?
        }
        CoreTerm::Abstract { body, .. } => infer_schema(body, source, bound)?,
    })
}

fn eval(
    term: &CoreTerm,
    source: &Database,
    limit: usize,
    trace: &mut Vec<String>,
) -> Result<(Relation, usize)> {
    let result = match term {
        CoreTerm::Source { relation } => (
            source
                .get(relation)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unknown source relation {relation}"))?,
            0,
        ),
        CoreTerm::Literal { relation } => (relation.clone(), 0),
        CoreTerm::Select { input, predicate } => {
            let (relation, iterations) = eval(input, source, limit, trace)?;
            let rows = relation
                .rows
                .iter()
                .filter(|row| predicate_holds(predicate, &relation, row, source).unwrap_or(false))
                .cloned();
            (
                Relation::new(relation.attributes.clone(), rows)?,
                iterations,
            )
        }
        CoreTerm::Image { input, morphism } => {
            let (relation, iterations) = eval(input, source, limit, trace)?;
            (apply_morphism(&relation, morphism)?, iterations)
        }
        CoreTerm::Join { left, right, on } => {
            let (a, n) = eval(left, source, limit, trace)?;
            let (b, m) = eval(right, source, limit, trace)?;
            (join(&a, &b, on)?, n + m)
        }
        CoreTerm::Union { left, right } => {
            let (a, n) = eval(left, source, limit, trace)?;
            let (b, m) = eval(right, source, limit, trace)?;
            same_schema(&a, &b)?;
            (
                Relation::new(a.attributes.clone(), a.rows.union(&b.rows).cloned())?,
                n + m,
            )
        }
        CoreTerm::Difference { left, right } => {
            let (a, n) = eval(left, source, limit, trace)?;
            let (b, m) = eval(right, source, limit, trace)?;
            same_schema(&a, &b)?;
            (
                Relation::new(a.attributes.clone(), a.rows.difference(&b.rows).cloned())?,
                n + m,
            )
        }
        CoreTerm::Group {
            input,
            keys,
            aggregates,
        } => {
            let (relation, iterations) = eval(input, source, limit, trace)?;
            (group(&relation, keys, aggregates)?, iterations)
        }
        CoreTerm::TransitiveClosure { input, from, to } => {
            let (relation, iterations) = eval(input, source, limit, trace)?;
            (closure(&relation, from, to, limit)?, iterations + 1)
        }
        CoreTerm::Overlay {
            base,
            layer,
            keys,
            prefer_layer,
        } => {
            let (a, n) = eval(base, source, limit, trace)?;
            let (b, m) = eval(layer, source, limit, trace)?;
            (overlay(&a, &b, keys, *prefer_layer)?, n + m)
        }
        CoreTerm::Fold {
            input,
            keys,
            value,
            consensus,
        } => {
            let (relation, iterations) = eval(input, source, limit, trace)?;
            (fold(&relation, keys, value, *consensus)?, iterations)
        }
        CoreTerm::IsomorphicCompletion {
            input,
            keys,
            value,
            unavailable,
        } => {
            let (relation, iterations) = eval(input, source, limit, trace)?;
            (
                isomorphic_completion(&relation, keys, value, unavailable)?,
                iterations + 1,
            )
        }
        CoreTerm::QuotientRepair { input, keys, value } => {
            let (relation, iterations) = eval(input, source, limit, trace)?;
            (quotient_repair(&relation, keys, value)?, iterations + 1)
        }
        CoreTerm::EntityTrace {
            input,
            keys,
            value,
            split_axis,
            canvas_high,
            order,
            macros,
        } => {
            let (relation, iterations) = eval(input, source, limit, trace)?;
            (
                entity_trace(
                    &relation,
                    keys,
                    value,
                    *split_axis,
                    *canvas_high,
                    order,
                    macros,
                )?,
                iterations + 1,
            )
        }
        CoreTerm::Scan {
            seed,
            relation,
            from,
            to,
        } => {
            let (start, iterations) = eval(seed, source, limit, trace)?;
            (
                scan(&start, &source[relation], from, to, limit)?,
                iterations + 1,
            )
        }
        CoreTerm::LeastFixedPoint {
            seed,
            step,
            maximum_iterations,
        } => {
            let (mut current, mut iterations) = eval(seed, source, limit, trace)?;
            for _ in 0..(*maximum_iterations).min(limit) {
                let mut database = source.clone();
                database.insert("$current".into(), current.clone());
                let (next, used) = eval(step, &database, limit, trace)?;
                iterations += used + 1;
                let merged = Relation::new(
                    current.attributes.clone(),
                    current.rows.union(&next.rows).cloned(),
                )?;
                if merged == current {
                    break;
                }
                current = merged;
            }
            (current, iterations)
        }
        CoreTerm::Compose { first, second } => {
            let (middle, n) = eval(first, source, limit, trace)?;
            let mut database = source.clone();
            database.insert("$input".into(), middle);
            let (output, m) = eval(second, &database, limit, trace)?;
            (output, n + m)
        }
        CoreTerm::Abstract { body, .. } => eval(body, source, limit, trace)?,
    };
    trace.push(digest_json(&(term, &result.0))?);
    Ok(result)
}

fn value(term: &ValueTerm, relation: &Relation, row: &[Atom]) -> Result<Atom> {
    Ok(match term {
        ValueTerm::Field { name } => row[relation.column(name)?].clone(),
        ValueTerm::Literal { value } => value.clone(),
        ValueTerm::Add { left, right } => Atom::Integer(
            integer(&value(left, relation, row)?)? + integer(&value(right, relation, row)?)?,
        ),
        ValueTerm::Subtract { left, right } => Atom::Integer(
            integer(&value(left, relation, row)?)? - integer(&value(right, relation, row)?)?,
        ),
        ValueTerm::Minimum { left, right } => {
            value(left, relation, row)?.min(value(right, relation, row)?)
        }
        ValueTerm::Maximum { left, right } => {
            value(left, relation, row)?.max(value(right, relation, row)?)
        }
    })
}

fn integer(atom: &Atom) -> Result<i64> {
    if let Atom::Integer(value) = atom {
        Ok(*value)
    } else {
        bail!("arithmetic requires integers")
    }
}

fn predicate_holds(
    predicate: &StructuralPredicate,
    relation: &Relation,
    row: &[Atom],
    database: &Database,
) -> Result<bool> {
    Ok(match predicate {
        StructuralPredicate::True => true,
        StructuralPredicate::Equal { left, right } => {
            value(left, relation, row)? == value(right, relation, row)?
        }
        StructuralPredicate::LessThan { left, right } => {
            value(left, relation, row)? < value(right, relation, row)?
        }
        StructuralPredicate::InRelation {
            relation: target,
            fields,
        } => {
            let target = database
                .get(target)
                .ok_or_else(|| anyhow::anyhow!("membership relation absent"))?;
            let local = fields
                .iter()
                .map(|(from, _)| relation.column(from))
                .collect::<Result<Vec<_>>>()?;
            let remote = fields
                .iter()
                .map(|(_, to)| target.column(to))
                .collect::<Result<Vec<_>>>()?;
            target.rows.iter().any(|candidate| {
                local
                    .iter()
                    .zip(&remote)
                    .all(|(a, b)| row[*a] == candidate[*b])
            })
        }
        StructuralPredicate::And { terms } => {
            for term in terms {
                if !predicate_holds(term, relation, row, database)? {
                    return Ok(false);
                }
            }
            true
        }
        StructuralPredicate::Or { terms } => {
            for term in terms {
                if predicate_holds(term, relation, row, database)? {
                    return Ok(true);
                }
            }
            false
        }
        StructuralPredicate::Not { term } => !predicate_holds(term, relation, row, database)?,
    })
}

fn apply_morphism(relation: &Relation, morphism: &MorphismTerm) -> Result<Relation> {
    match morphism {
        MorphismTerm::Identity => Ok(relation.clone()),
        MorphismTerm::Project { fields } => {
            let columns = fields
                .iter()
                .map(|name| relation.column(name))
                .collect::<Result<Vec<_>>>()?;
            Relation::new(
                fields.clone(),
                relation
                    .rows
                    .iter()
                    .map(|row| columns.iter().map(|column| row[*column].clone()).collect()),
            )
        }
        MorphismTerm::Rename { mapping } => Relation::new(
            relation
                .attributes
                .iter()
                .map(|name| mapping.get(name).cloned().unwrap_or_else(|| name.clone()))
                .collect(),
            relation.rows.clone(),
        ),
        MorphismTerm::Derive { fields } => Relation::new(
            fields.iter().map(|item| item.0.clone()).collect(),
            relation
                .rows
                .iter()
                .map(|row| {
                    fields
                        .iter()
                        .map(|item| value(&item.1, relation, row))
                        .collect::<Result<Vec<_>>>()
                })
                .collect::<Result<Vec<_>>>()?,
        ),
        MorphismTerm::Compose { first, second } => {
            apply_morphism(&apply_morphism(relation, first)?, second)
        }
        MorphismTerm::Product { factors } => {
            let mut output = relation.clone();
            for factor in factors {
                output = apply_morphism(&output, factor)?;
            }
            Ok(output)
        }
        MorphismTerm::Coproduct { branches } => {
            let mut output: Option<Relation> = None;
            for branch in branches {
                let next = apply_morphism(relation, branch)?;
                output = Some(if let Some(current) = output {
                    same_schema(&current, &next)?;
                    Relation::new(
                        current.attributes.clone(),
                        current.rows.union(&next.rows).cloned(),
                    )?
                } else {
                    next
                });
            }
            output.ok_or_else(|| anyhow::anyhow!("empty morphism coproduct"))
        }
    }
}

fn validate_predicate(
    predicate: &StructuralPredicate,
    fields: &[String],
    source: &BTreeMap<String, Vec<String>>,
) -> Result<()> {
    match predicate {
        StructuralPredicate::True => {}
        StructuralPredicate::Equal { left, right }
        | StructuralPredicate::LessThan { left, right } => {
            validate_value(left, fields)?;
            validate_value(right, fields)?;
        }
        StructuralPredicate::InRelation {
            relation,
            fields: mapping,
        } => {
            let remote = source
                .get(relation)
                .ok_or_else(|| anyhow::anyhow!("membership relation absent"))?;
            for (local, target) in mapping {
                require_fields(fields, std::slice::from_ref(local))?;
                require_fields(remote, std::slice::from_ref(target))?;
            }
        }
        StructuralPredicate::And { terms } | StructuralPredicate::Or { terms } => {
            if terms.is_empty() {
                bail!("empty boolean predicate")
            }
            for term in terms {
                validate_predicate(term, fields, source)?;
            }
        }
        StructuralPredicate::Not { term } => validate_predicate(term, fields, source)?,
    }
    Ok(())
}

fn validate_value(term: &ValueTerm, fields: &[String]) -> Result<()> {
    match term {
        ValueTerm::Field { name } => require_fields(fields, std::slice::from_ref(name))?,
        ValueTerm::Literal { .. } => {}
        ValueTerm::Add { left, right }
        | ValueTerm::Subtract { left, right }
        | ValueTerm::Minimum { left, right }
        | ValueTerm::Maximum { left, right } => {
            validate_value(left, fields)?;
            validate_value(right, fields)?;
        }
    }
    Ok(())
}

fn morphism_schema(fields: &[String], morphism: &MorphismTerm) -> Result<Vec<String>> {
    Ok(match morphism {
        MorphismTerm::Identity => fields.to_vec(),
        MorphismTerm::Project { fields: output } => {
            require_fields(fields, output)?;
            unique_fields(output)?
        }
        MorphismTerm::Rename { mapping } => {
            for name in mapping.keys() {
                require_fields(fields, std::slice::from_ref(name))?;
            }
            unique_fields(
                &fields
                    .iter()
                    .map(|name| mapping.get(name).cloned().unwrap_or_else(|| name.clone()))
                    .collect::<Vec<_>>(),
            )?
        }
        MorphismTerm::Derive { fields: output } => {
            for (_, term) in output {
                validate_value(term, fields)?;
            }
            unique_fields(&output.iter().map(|item| item.0.clone()).collect::<Vec<_>>())?
        }
        MorphismTerm::Compose { first, second } => {
            morphism_schema(&morphism_schema(fields, first)?, second)?
        }
        MorphismTerm::Product { factors } => {
            let mut current = fields.to_vec();
            for factor in factors {
                current = morphism_schema(&current, factor)?;
            }
            current
        }
        MorphismTerm::Coproduct { branches } => {
            let mut candidates = branches
                .iter()
                .map(|branch| morphism_schema(fields, branch));
            let first = candidates
                .next()
                .ok_or_else(|| anyhow::anyhow!("empty morphism coproduct"))??;
            for candidate in candidates {
                if candidate? != first {
                    bail!("morphism coproduct schemas differ")
                }
            }
            first
        }
    })
}

fn product_carrier(
    items: &[CarrierTerm],
    source: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>> {
    if items.is_empty() {
        bail!("empty carrier product")
    }
    let mut output = Vec::new();
    for item in items {
        for field in validate_carrier(item, source)? {
            if output.contains(&field) {
                bail!("carrier product field collision {field}")
            }
            output.push(field);
        }
    }
    Ok(output)
}

fn join_schema(left: &[String], right: &[String], on: &[(String, String)]) -> Result<Vec<String>> {
    for (a, b) in on {
        require_fields(left, std::slice::from_ref(a))?;
        require_fields(right, std::slice::from_ref(b))?;
    }
    let mut output = left.to_vec();
    for field in right {
        if output.contains(field) {
            bail!("join attribute collision {field}")
        }
        output.push(field.clone());
    }
    Ok(output)
}

fn unique_fields(fields: &[String]) -> Result<Vec<String>> {
    if fields.is_empty() || fields.iter().collect::<BTreeSet<_>>().len() != fields.len() {
        bail!("carrier fields must be nonempty and unique")
    }
    Ok(fields.to_vec())
}

fn require_fields(schema: &[String], fields: &[String]) -> Result<()> {
    for field in fields {
        if !schema.contains(field) {
            bail!("unknown field {field}")
        }
    }
    Ok(())
}

fn is_bound_name(name: &str) -> bool {
    name == "$input" || name == "$current"
}
fn same_schema(left: &Relation, right: &Relation) -> Result<()> {
    if left.attributes != right.attributes {
        bail!("relation schemas differ")
    }
    Ok(())
}

fn join(left: &Relation, right: &Relation, on: &[(String, String)]) -> Result<Relation> {
    let left_columns = on
        .iter()
        .map(|item| left.column(&item.0))
        .collect::<Result<Vec<_>>>()?;
    let right_columns = on
        .iter()
        .map(|item| right.column(&item.1))
        .collect::<Result<Vec<_>>>()?;
    let attributes = join_schema(&left.attributes, &right.attributes, on)?;
    let mut rows = BTreeSet::new();
    for a in &left.rows {
        for b in &right.rows {
            if left_columns
                .iter()
                .zip(&right_columns)
                .all(|(x, y)| a[*x] == b[*y])
            {
                let mut row = a.clone();
                row.extend(b.clone());
                rows.insert(row);
            }
        }
    }
    Relation::new(attributes, rows)
}

fn fold(
    relation: &Relation,
    keys: &[String],
    value_name: &str,
    consensus: bool,
) -> Result<Relation> {
    let key_columns = keys
        .iter()
        .map(|name| relation.column(name))
        .collect::<Result<Vec<_>>>()?;
    let value_column = relation.column(value_name)?;
    let mut groups: BTreeMap<Vec<Atom>, BTreeSet<Atom>> = BTreeMap::new();
    for row in &relation.rows {
        groups
            .entry(
                key_columns
                    .iter()
                    .map(|column| row[*column].clone())
                    .collect(),
            )
            .or_default()
            .insert(row[value_column].clone());
    }
    let rows = groups.into_iter().filter_map(|(mut key, values)| {
        if consensus && values.len() != 1 {
            return None;
        }
        key.push(values.into_iter().next().unwrap_or(Atom::Null));
        Some(key)
    });
    let mut attributes = keys.to_vec();
    attributes.push(value_name.to_string());
    Relation::new(attributes, rows)
}

fn isomorphic_completion(
    relation: &Relation,
    keys: &[String],
    value: &str,
    unavailable: &Atom,
) -> Result<Relation> {
    if keys.is_empty() || keys.len() > 4 || relation.attributes.len() != keys.len() + 1 {
        bail!("isomorphic completion supports product ranks one through four")
    }
    let key_columns = keys
        .iter()
        .map(|key| relation.column(key))
        .collect::<Result<Vec<_>>>()?;
    let value_column = relation.column(value)?;
    let domains = key_columns
        .iter()
        .map(|column| {
            relation
                .rows
                .iter()
                .map(|row| row[*column].clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let cardinality = domains.iter().try_fold(1usize, |product, domain| {
        product
            .checked_mul(domain.len())
            .ok_or_else(|| anyhow::anyhow!("product carrier cardinality overflow"))
    })?;
    if cardinality != relation.rows.len() {
        bail!("isomorphic completion requires a total finite function")
    }
    let domain_indices = domains
        .iter()
        .map(|domain| {
            domain
                .iter()
                .enumerate()
                .map(|(index, atom)| (atom.clone(), index))
                .collect::<BTreeMap<_, _>>()
        })
        .collect::<Vec<_>>();
    let mut values = BTreeMap::<Vec<usize>, Atom>::new();
    let mut missing = Vec::new();
    for row in &relation.rows {
        let index = key_columns
            .iter()
            .enumerate()
            .map(|(axis, column)| domain_indices[axis].get(&row[*column]).copied().unwrap())
            .collect::<Vec<_>>();
        if &row[value_column] == unavailable {
            missing.push(index.clone());
        }
        values.insert(index, row[value_column].clone());
    }
    if missing.is_empty() || missing.len() == relation.rows.len() {
        bail!("unavailable support must be a proper nonempty subobject")
    }
    let support_axes = (0..keys.len())
        .map(|axis| {
            missing
                .iter()
                .map(|index| index[axis])
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let support_size = support_axes.iter().map(Vec::len).product::<usize>();
    if support_size != missing.len()
        || support_axes
            .iter()
            .any(|axis| axis.windows(2).any(|pair| pair[1] != pair[0] + 1))
    {
        bail!("unavailable support is not a contiguous Cartesian subobject")
    }
    let support_origin = support_axes.iter().map(|axis| axis[0]).collect::<Vec<_>>();
    let support_shape = support_axes.iter().map(Vec::len).collect::<Vec<_>>();
    let residual_prefixes = (0..keys.len())
        .map(|axis| infer_reflection_residual(axis, &domains, &values, unavailable))
        .collect::<Vec<_>>();
    let permutations = permutations(keys.len());
    let mut best_score: Option<(usize, usize, std::cmp::Reverse<usize>)> = None;
    let mut best_predictions = Vec::<BTreeMap<Vec<usize>, Atom>>::new();
    for permutation in permutations {
        let mut source_shape = vec![0usize; keys.len()];
        for (target_axis, source_axis) in permutation.iter().enumerate() {
            source_shape[*source_axis] = support_shape[target_axis];
        }
        if source_shape
            .iter()
            .zip(&domains)
            .any(|(length, domain)| *length > domain.len())
        {
            continue;
        }
        let origins = source_shape
            .iter()
            .zip(&domains)
            .map(|(length, domain)| (0..=domain.len() - length).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        for signs in 0usize..(1usize << keys.len()) {
            for source_origin in cartesian_usize(&origins) {
                let transport = |target: &[usize]| -> Option<Vec<usize>> {
                    let mut source = vec![0usize; keys.len()];
                    for target_axis in 0..keys.len() {
                        let local =
                            target[target_axis] as isize - support_origin[target_axis] as isize;
                        let local = if signs & (1 << target_axis) == 0 {
                            local
                        } else {
                            support_shape[target_axis] as isize - 1 - local
                        };
                        let coordinate = source_origin[permutation[target_axis]] as isize + local;
                        if coordinate < 0
                            || coordinate >= domains[permutation[target_axis]].len() as isize
                        {
                            return None;
                        }
                        source[permutation[target_axis]] = coordinate as usize;
                    }
                    Some(source)
                };
                let mut prediction = BTreeMap::new();
                let mut valid = true;
                for target in &missing {
                    let Some(source_index) = transport(target) else {
                        valid = false;
                        break;
                    };
                    let Some(source_value) = values.get(&source_index) else {
                        valid = false;
                        break;
                    };
                    if source_value == unavailable {
                        valid = false;
                        break;
                    }
                    prediction.insert(target.clone(), source_value.clone());
                }
                if !valid {
                    continue;
                }
                let mut agreements = 0usize;
                let mut disagreements = 0usize;
                for (target, target_value) in &values {
                    let Some(source_index) = transport(target) else {
                        continue;
                    };
                    let Some(source_value) = values.get(&source_index) else {
                        continue;
                    };
                    if target_value == unavailable || source_value == unavailable {
                        continue;
                    }
                    if target_value == source_value {
                        agreements += 1;
                    } else {
                        disagreements += 1;
                    }
                }
                if agreements == 0 {
                    continue;
                }
                let role_mismatches = missing
                    .iter()
                    .map(|target| {
                        let source = transport(target).unwrap();
                        (0..keys.len())
                            .filter(|target_axis| {
                                let source_axis = permutation[*target_axis];
                                (target[*target_axis] < residual_prefixes[*target_axis])
                                    != (source[source_axis] < residual_prefixes[source_axis])
                            })
                            .count()
                    })
                    .sum::<usize>();
                let score = (
                    role_mismatches,
                    disagreements,
                    std::cmp::Reverse(agreements),
                );
                match &best_score {
                    None => {
                        best_score = Some(score);
                        best_predictions = vec![prediction];
                    }
                    Some(best) if score < *best => {
                        best_score = Some(score);
                        best_predictions = vec![prediction];
                    }
                    Some(best) if score == *best && !best_predictions.contains(&prediction) => {
                        best_predictions.push(prediction);
                    }
                    _ => {}
                }
            }
        }
    }
    if best_predictions.is_empty() {
        bail!("no supported partial isomorphism completes the subobject")
    }
    let mut rows = Vec::with_capacity(missing.len());
    for target in missing {
        let candidates = best_predictions
            .iter()
            .filter_map(|prediction| prediction.get(&target).cloned())
            .collect::<BTreeSet<_>>();
        if candidates.len() != 1 {
            bail!("best partial isomorphisms disagree on the completion")
        }
        let mut row = target
            .iter()
            .enumerate()
            .map(|(axis, coordinate)| Atom::Integer((coordinate - support_origin[axis]) as i64))
            .collect::<Vec<_>>();
        row.push(candidates.into_iter().next().unwrap());
        rows.push(row);
    }
    let mut attributes = keys.to_vec();
    attributes.push(value.to_string());
    Relation::new(attributes, rows)
}

fn infer_reflection_residual(
    axis: usize,
    domains: &[Vec<Atom>],
    values: &BTreeMap<Vec<usize>, Atom>,
    unavailable: &Atom,
) -> usize {
    let extent = domains[axis].len();
    (0..extent)
        .map(|prefix| {
            let mut agreements = 0usize;
            let mut disagreements = 0usize;
            for (index, observed) in values {
                if index[axis] < prefix || observed == unavailable {
                    continue;
                }
                let mut reflected = index.clone();
                reflected[axis] = prefix + extent - 1 - index[axis];
                let Some(other) = values.get(&reflected) else {
                    continue;
                };
                if other == unavailable {
                    continue;
                }
                if observed == other {
                    agreements += 1;
                } else {
                    disagreements += 1;
                }
            }
            (
                (disagreements, std::cmp::Reverse(agreements), prefix),
                prefix,
            )
        })
        .min_by_key(|item| item.0)
        .map(|item| item.1)
        .unwrap_or(0)
}

fn quotient_repair(relation: &Relation, keys: &[String], value: &str) -> Result<Relation> {
    if keys.is_empty() || keys.len() > 4 || relation.attributes.len() != keys.len() + 1 {
        bail!("quotient repair supports product ranks one through four")
    }
    let key_columns = keys
        .iter()
        .map(|key| relation.column(key))
        .collect::<Result<Vec<_>>>()?;
    let value_column = relation.column(value)?;
    let domains = key_columns
        .iter()
        .map(|column| {
            relation
                .rows
                .iter()
                .map(|row| row[*column].clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let cardinality = domains.iter().try_fold(1usize, |product, domain| {
        product
            .checked_mul(domain.len())
            .ok_or_else(|| anyhow::anyhow!("product carrier cardinality overflow"))
    })?;
    if cardinality != relation.rows.len() {
        bail!("quotient repair requires a total finite function")
    }
    let indices = domains
        .iter()
        .map(|domain| {
            domain
                .iter()
                .enumerate()
                .map(|(index, atom)| (atom.clone(), index))
                .collect::<BTreeMap<_, _>>()
        })
        .collect::<Vec<_>>();
    let mut values = BTreeMap::<Vec<usize>, Atom>::new();
    for row in &relation.rows {
        let key = key_columns
            .iter()
            .enumerate()
            .map(|(axis, column)| indices[axis][&row[*column]])
            .collect::<Vec<_>>();
        values.insert(key, row[value_column].clone());
    }
    let homogeneous = (0..keys.len())
        .map(|axis| {
            (0..domains[axis].len())
                .filter(|coordinate| {
                    values
                        .iter()
                        .filter(|(key, _)| key[axis] == *coordinate)
                        .map(|(_, value)| value)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == 1
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let intervals = homogeneous
        .iter()
        .enumerate()
        .map(|(axis, planes)| {
            let bounded = planes
                .windows(2)
                .filter_map(|pair| (pair[0] + 1 < pair[1]).then_some((pair[0] + 1, pair[1])))
                .collect::<Vec<_>>();
            if bounded.is_empty() {
                vec![(0usize, domains[axis].len())]
            } else {
                bounded
            }
        })
        .collect::<Vec<_>>();
    let region_indices = intervals
        .iter()
        .map(|axis| (0..axis.len()).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let regions = cartesian_usize(&region_indices);
    let alphabet = values
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .len()
        .max(2);
    let error_bits = (usize::BITS as usize - (alphabet - 1).leading_zeros() as usize).max(1);
    let mut repaired = values.clone();
    for region in regions {
        let mut bounds = region
            .iter()
            .enumerate()
            .map(|(axis, choice)| intervals[axis][*choice])
            .collect::<Vec<_>>();
        loop {
            if bounds
                .iter()
                .any(|(start, end)| end.saturating_sub(*start) < 3)
            {
                break;
            }
            let border = values
                .iter()
                .filter(|(key, _)| inside(key, &bounds) && on_boundary(key, &bounds))
                .map(|(_, atom)| atom)
                .collect::<BTreeSet<_>>();
            if border.len() != 1 {
                break;
            }
            for (start, end) in &mut bounds {
                *start += 1;
                *end -= 1;
            }
        }
        if bounds.iter().any(|(start, end)| start >= end) {
            continue;
        }
        let extents = bounds
            .iter()
            .map(|(start, end)| end - start)
            .collect::<Vec<_>>();
        let period_domains = extents
            .iter()
            .map(|extent| (1..=*extent).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let mut best: Option<QuotientRepairCandidate> = None;
        for periods in cartesian_usize(&period_domains) {
            let mut groups = BTreeMap::<Vec<usize>, Vec<Atom>>::new();
            for (key, atom) in &values {
                if !inside(key, &bounds) {
                    continue;
                }
                let residue = key
                    .iter()
                    .enumerate()
                    .map(|(axis, coordinate)| (coordinate - bounds[axis].0) % periods[axis])
                    .collect::<Vec<_>>();
                groups.entry(residue).or_default().push(atom.clone());
            }
            let modes = groups
                .into_iter()
                .map(|(residue, atoms)| {
                    let mut counts = BTreeMap::<Atom, usize>::new();
                    for atom in atoms {
                        *counts.entry(atom).or_default() += 1;
                    }
                    let mode = counts
                        .into_iter()
                        .max_by(|left, right| {
                            left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0))
                        })
                        .unwrap()
                        .0;
                    (residue, mode)
                })
                .collect::<BTreeMap<_, _>>();
            let errors = values
                .iter()
                .filter(|(key, atom)| {
                    if !inside(key, &bounds) {
                        return false;
                    }
                    let residue = key
                        .iter()
                        .enumerate()
                        .map(|(axis, coordinate)| (coordinate - bounds[axis].0) % periods[axis])
                        .collect::<Vec<_>>();
                    modes[&residue] != **atom
                })
                .count();
            let model_cells = periods.iter().product::<usize>();
            let description = model_cells.saturating_add(errors.saturating_mul(error_bits));
            let candidate = (description, errors, model_cells, periods, modes);
            let better = best.as_ref().is_none_or(|previous| {
                (candidate.0, candidate.1, candidate.2, &candidate.3)
                    < (previous.0, previous.1, previous.2, &previous.3)
            });
            if better {
                best = Some(candidate);
            }
        }
        let Some((_, _, _, periods, modes)) = best else {
            continue;
        };
        for (key, atom) in &mut repaired {
            if inside(key, &bounds) {
                let residue = key
                    .iter()
                    .enumerate()
                    .map(|(axis, coordinate)| (coordinate - bounds[axis].0) % periods[axis])
                    .collect::<Vec<_>>();
                *atom = modes[&residue].clone();
            }
        }
    }
    let rows = repaired.into_iter().map(|(key, atom)| {
        let mut row = Vec::with_capacity(relation.attributes.len());
        for attribute in &relation.attributes {
            if attribute == value {
                row.push(atom.clone());
            } else {
                let axis = keys
                    .iter()
                    .position(|key_name| key_name == attribute)
                    .unwrap();
                row.push(domains[axis][key[axis]].clone());
            }
        }
        row
    });
    Relation::new(relation.attributes.clone(), rows)
}

fn inside(key: &[usize], bounds: &[(usize, usize)]) -> bool {
    key.iter()
        .zip(bounds)
        .all(|(coordinate, (start, end))| coordinate >= start && coordinate < end)
}

fn on_boundary(key: &[usize], bounds: &[(usize, usize)]) -> bool {
    key.iter()
        .zip(bounds)
        .any(|(coordinate, (start, end))| coordinate == start || coordinate + 1 == *end)
}

fn entity_trace(
    relation: &Relation,
    keys: &[String],
    value: &str,
    split_axis: usize,
    canvas_high: bool,
    order: &[usize],
    macros: &[EntityTraceMacro],
) -> Result<Relation> {
    let (domains, values) = indexed_relation(relation, keys, value)?;
    let extent = domains[split_axis].len();
    let mut candidates = Vec::new();
    for separator in 1..extent.saturating_sub(1) {
        let plane = values
            .iter()
            .filter(|(key, _)| key[split_axis] == separator)
            .map(|(_, atom)| atom)
            .collect::<BTreeSet<_>>();
        if plane.len() != 1 {
            continue;
        }
        let (source_interval, canvas_interval) = if canvas_high {
            ((0, separator), (separator + 1, extent))
        } else {
            ((separator + 1, extent), (0, separator))
        };
        if source_interval.0 >= source_interval.1 || canvas_interval.0 >= canvas_interval.1 {
            continue;
        }
        let canvas_values = values
            .iter()
            .filter(|(key, _)| {
                key[split_axis] >= canvas_interval.0 && key[split_axis] < canvas_interval.1
            })
            .map(|(_, atom)| atom.clone())
            .collect::<Vec<_>>();
        let background = mode(&canvas_values)?;
        let markers = values
            .iter()
            .filter(|(key, atom)| {
                key[split_axis] >= canvas_interval.0
                    && key[split_axis] < canvas_interval.1
                    && **atom != background
            })
            .map(|(key, atom)| (key.clone(), atom.clone()))
            .collect::<Vec<_>>();
        if markers.len() != 1 {
            continue;
        }
        let source = values
            .iter()
            .filter(|(key, _)| {
                key[split_axis] >= source_interval.0 && key[split_axis] < source_interval.1
            })
            .map(|(key, atom)| (key.clone(), atom.clone()))
            .collect::<BTreeMap<_, _>>();
        let source_background = mode(&source.values().cloned().collect::<Vec<_>>())?;
        let mut entities = connected_entities(&source, &source_background);
        if entities.is_empty() {
            continue;
        }
        entities.sort_by(|left, right| {
            order
                .iter()
                .map(|axis| left.iter().map(|key| key[*axis]).min().unwrap())
                .cmp(
                    order
                        .iter()
                        .map(|axis| right.iter().map(|key| key[*axis]).min().unwrap()),
                )
        });
        let macro_map = macros
            .iter()
            .map(|item| ((item.value.clone(), item.normalized_support.clone()), item))
            .collect::<BTreeMap<_, _>>();
        let mut sequence = Vec::new();
        let mut supported = true;
        for entity in entities {
            let atom = source[&entity[0]].clone();
            let signature = normalize_support(&entity);
            let Some(item) = macro_map.get(&(atom, signature)) else {
                supported = false;
                break;
            };
            sequence.push(*item);
        }
        if !supported {
            continue;
        }
        candidates.push((
            separator,
            canvas_interval,
            background,
            markers[0].clone(),
            sequence,
        ));
    }
    if candidates.len() != 1 {
        bail!("entity trace requires one structurally supported separator")
    }
    let (_, canvas_interval, background, (marker_key, marker), sequence) = candidates.remove(0);
    let mut canvas_shape = domains.iter().map(Vec::len).collect::<Vec<_>>();
    canvas_shape[split_axis] = canvas_interval.1 - canvas_interval.0;
    let mut cursor = marker_key;
    cursor[split_axis] -= canvas_interval.0;
    let mut output = cartesian_shape(&canvas_shape)
        .into_iter()
        .map(|key| (key, background.clone()))
        .collect::<BTreeMap<_, _>>();
    output.insert(cursor.clone(), marker);
    for item in sequence {
        for offset in &item.paint_offsets {
            let target = shifted(&cursor, offset, &canvas_shape)?;
            if output[&target] != background {
                bail!("entity trace macros overlap")
            }
            output.insert(target, item.value.clone());
        }
        cursor = shifted(&cursor, &item.cursor_delta, &canvas_shape)?;
    }
    relation_from_indexed(relation, keys, value, &canvas_shape, output)
}

pub(crate) fn indexed_relation(
    relation: &Relation,
    keys: &[String],
    value: &str,
) -> Result<(Vec<Vec<Atom>>, IndexedValues)> {
    let key_columns = keys
        .iter()
        .map(|key| relation.column(key))
        .collect::<Result<Vec<_>>>()?;
    let value_column = relation.column(value)?;
    let domains = key_columns
        .iter()
        .map(|column| {
            relation
                .rows
                .iter()
                .map(|row| row[*column].clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let indices = domains
        .iter()
        .map(|domain| {
            domain
                .iter()
                .enumerate()
                .map(|(index, atom)| (atom.clone(), index))
                .collect::<BTreeMap<_, _>>()
        })
        .collect::<Vec<_>>();
    let values = relation
        .rows
        .iter()
        .map(|row| {
            let key = key_columns
                .iter()
                .enumerate()
                .map(|(axis, column)| indices[axis][&row[*column]])
                .collect::<Vec<_>>();
            (key, row[value_column].clone())
        })
        .collect::<BTreeMap<_, _>>();
    if values.len() != domains.iter().map(Vec::len).product::<usize>() {
        bail!("entity trace requires a total finite function")
    }
    Ok((domains, values))
}

pub(crate) fn mode(atoms: &[Atom]) -> Result<Atom> {
    let mut counts = BTreeMap::<Atom, usize>::new();
    for atom in atoms {
        *counts.entry(atom.clone()).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
        .map(|item| item.0)
        .ok_or_else(|| anyhow::anyhow!("cannot infer a mode from an empty carrier"))
}

pub(crate) fn connected_entities(
    values: &BTreeMap<Vec<usize>, Atom>,
    background: &Atom,
) -> Vec<Vec<Vec<usize>>> {
    let mut unvisited = values
        .iter()
        .filter(|(_, atom)| *atom != background)
        .map(|(key, _)| key.clone())
        .collect::<BTreeSet<_>>();
    let mut result = Vec::new();
    while let Some(seed) = unvisited.iter().next().cloned() {
        let atom = values[&seed].clone();
        let mut stack = vec![seed];
        let mut component = Vec::new();
        while let Some(key) = stack.pop() {
            if !unvisited.remove(&key) {
                continue;
            }
            component.push(key.clone());
            for neighbor in unvisited.iter().filter(|neighbor| {
                neighbor
                    .iter()
                    .zip(&key)
                    .all(|(left, right)| left.abs_diff(*right) <= 1)
                    && values.get(*neighbor) == Some(&atom)
            }) {
                stack.push(neighbor.clone());
            }
        }
        component.sort();
        result.push(component);
    }
    result
}

pub(crate) fn normalize_support(support: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let origin = (0..support[0].len())
        .map(|axis| support.iter().map(|key| key[axis]).min().unwrap())
        .collect::<Vec<_>>();
    support
        .iter()
        .map(|key| {
            key.iter()
                .enumerate()
                .map(|(axis, coordinate)| coordinate - origin[axis])
                .collect()
        })
        .collect()
}

fn shifted(origin: &[usize], offset: &[i64], shape: &[usize]) -> Result<Vec<usize>> {
    origin
        .iter()
        .zip(offset)
        .zip(shape)
        .map(|((coordinate, delta), extent)| {
            let target = *coordinate as i64 + delta;
            if target < 0 || target >= *extent as i64 {
                bail!("entity trace left its constructed carrier")
            }
            Ok(target as usize)
        })
        .collect()
}

fn cartesian_shape(shape: &[usize]) -> Vec<Vec<usize>> {
    cartesian_usize(
        &shape
            .iter()
            .map(|extent| (0..*extent).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
    )
}

fn relation_from_indexed(
    relation: &Relation,
    keys: &[String],
    value: &str,
    shape: &[usize],
    values: BTreeMap<Vec<usize>, Atom>,
) -> Result<Relation> {
    let rows = values.into_iter().map(|(key, atom)| {
        relation
            .attributes
            .iter()
            .map(|attribute| {
                if attribute == value {
                    atom.clone()
                } else {
                    let axis = keys.iter().position(|name| name == attribute).unwrap();
                    debug_assert!(key[axis] < shape[axis]);
                    Atom::Integer(key[axis] as i64)
                }
            })
            .collect::<Vec<_>>()
    });
    Relation::new(relation.attributes.clone(), rows)
}

fn permutations(rank: usize) -> Vec<Vec<usize>> {
    fn visit(prefix: &mut Vec<usize>, unused: &mut Vec<usize>, output: &mut Vec<Vec<usize>>) {
        if unused.is_empty() {
            output.push(prefix.clone());
            return;
        }
        for index in 0..unused.len() {
            let value = unused.remove(index);
            prefix.push(value);
            visit(prefix, unused, output);
            prefix.pop();
            unused.insert(index, value);
        }
    }
    let mut output = Vec::new();
    visit(&mut Vec::new(), &mut (0..rank).collect(), &mut output);
    output
}

fn cartesian_usize(domains: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut rows = vec![Vec::new()];
    for domain in domains {
        rows = rows
            .into_iter()
            .flat_map(|prefix| {
                domain.iter().map(move |value| {
                    let mut row = prefix.clone();
                    row.push(*value);
                    row
                })
            })
            .collect();
    }
    rows
}

fn group(relation: &Relation, keys: &[String], aggregates: &[AggregateTerm]) -> Result<Relation> {
    let key_columns = keys
        .iter()
        .map(|name| relation.column(name))
        .collect::<Result<Vec<_>>>()?;
    let mut groups: BTreeMap<Vec<Atom>, Vec<&Vec<Atom>>> = BTreeMap::new();
    for row in &relation.rows {
        groups
            .entry(
                key_columns
                    .iter()
                    .map(|column| row[*column].clone())
                    .collect(),
            )
            .or_default()
            .push(row);
    }
    if keys.is_empty() && groups.is_empty() {
        groups.insert(Vec::new(), Vec::new());
    }
    let mut attributes = keys.to_vec();
    attributes.extend(aggregates.iter().map(|item| item.output.clone()));
    let mut rows = Vec::new();
    for (mut key, members) in groups {
        let mut valid = true;
        for aggregate in aggregates {
            let values = if let Some(input) = &aggregate.input {
                let column = relation.column(input)?;
                members
                    .iter()
                    .map(|row| row[column].clone())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let result = match aggregate.operator {
                AggregateOperator::Count => Atom::Integer(members.len() as i64),
                AggregateOperator::Minimum => values.into_iter().min().unwrap_or(Atom::Null),
                AggregateOperator::Maximum => values.into_iter().max().unwrap_or(Atom::Null),
                AggregateOperator::Consensus => {
                    let unique = values.into_iter().collect::<BTreeSet<_>>();
                    if unique.len() != 1 {
                        valid = false;
                        Atom::Null
                    } else {
                        unique.into_iter().next().unwrap()
                    }
                }
            };
            key.push(result);
        }
        if valid {
            rows.push(key);
        }
    }
    Relation::new(attributes, rows)
}

fn overlay(
    base: &Relation,
    layer: &Relation,
    keys: &[String],
    prefer_layer: bool,
) -> Result<Relation> {
    same_schema(base, layer)?;
    let columns = keys
        .iter()
        .map(|name| base.column(name))
        .collect::<Result<Vec<_>>>()?;
    let mut indexed = BTreeMap::new();
    for row in &base.rows {
        indexed.insert(
            columns
                .iter()
                .map(|column| row[*column].clone())
                .collect::<Vec<_>>(),
            row.clone(),
        );
    }
    for row in &layer.rows {
        let key = columns
            .iter()
            .map(|column| row[*column].clone())
            .collect::<Vec<_>>();
        if prefer_layer || !indexed.contains_key(&key) {
            indexed.insert(key, row.clone());
        }
    }
    Relation::new(base.attributes.clone(), indexed.into_values())
}

fn closure(relation: &Relation, from: &str, to: &str, limit: usize) -> Result<Relation> {
    if relation.attributes.len() != 2 {
        bail!("transitive closure requires a binary relation")
    }
    let from_column = relation.column(from)?;
    let to_column = relation.column(to)?;
    let adjacency =
        relation
            .rows
            .iter()
            .fold(BTreeMap::<Atom, BTreeSet<Atom>>::new(), |mut graph, row| {
                graph
                    .entry(row[from_column].clone())
                    .or_default()
                    .insert(row[to_column].clone());
                graph
            });
    let mut rows = BTreeSet::new();
    for start in adjacency.keys() {
        let mut seen = BTreeSet::new();
        let mut frontier = adjacency.get(start).cloned().unwrap_or_default();
        for _ in 0..limit {
            if frontier.is_empty() {
                break;
            }
            let current = frontier.clone();
            frontier.clear();
            for node in current {
                if !seen.insert(node.clone()) {
                    continue;
                }
                let mut row = vec![Atom::Null; 2];
                row[from_column] = start.clone();
                row[to_column] = node.clone();
                rows.insert(row);
                if let Some(next) = adjacency.get(&node) {
                    frontier.extend(next.iter().filter(|item| !seen.contains(*item)).cloned());
                }
            }
        }
    }
    Relation::new(relation.attributes.clone(), rows)
}

fn scan(seed: &Relation, edges: &Relation, from: &str, to: &str, limit: usize) -> Result<Relation> {
    if seed.attributes.len() != 1 {
        bail!("scan seed must be unary")
    }
    let from_column = edges.column(from)?;
    let to_column = edges.column(to)?;
    let mut values = seed
        .rows
        .iter()
        .map(|row| row[0].clone())
        .collect::<BTreeSet<_>>();
    for _ in 0..limit {
        let before = values.len();
        for edge in &edges.rows {
            if values.contains(&edge[from_column]) {
                values.insert(edge[to_column].clone());
            }
        }
        if values.len() == before {
            break;
        }
    }
    Relation::new(
        seed.attributes.clone(),
        values.into_iter().map(|value| vec![value]),
    )
}

/// Prefix cost of the executable syntax, independent of Rust/JSON byte layout.
pub fn description_bits(program: &TypedCoreProgram) -> Result<usize> {
    fn value_bits(value: &ValueTerm) -> usize {
        match value {
            ValueTerm::Field { name } => 2 + gamma(name.len() + 1),
            ValueTerm::Literal { .. } => 3 + 64,
            ValueTerm::Add { left, right }
            | ValueTerm::Subtract { left, right }
            | ValueTerm::Minimum { left, right }
            | ValueTerm::Maximum { left, right } => 3 + value_bits(left) + value_bits(right),
        }
    }
    fn predicate_bits(predicate: &StructuralPredicate) -> usize {
        match predicate {
            StructuralPredicate::True => 1,
            StructuralPredicate::Equal { left, right }
            | StructuralPredicate::LessThan { left, right } => {
                3 + value_bits(left) + value_bits(right)
            }
            StructuralPredicate::InRelation { relation, fields } => {
                3 + gamma(relation.len() + 1)
                    + fields
                        .iter()
                        .map(|(a, b)| gamma(a.len() + 1) + gamma(b.len() + 1))
                        .sum::<usize>()
            }
            StructuralPredicate::And { terms } | StructuralPredicate::Or { terms } => {
                3 + gamma(terms.len() + 1) + terms.iter().map(predicate_bits).sum::<usize>()
            }
            StructuralPredicate::Not { term } => 2 + predicate_bits(term),
        }
    }
    fn morphism_bits(morphism: &MorphismTerm) -> usize {
        match morphism {
            MorphismTerm::Identity => 1,
            MorphismTerm::Project { fields } => {
                3 + fields.iter().map(|x| gamma(x.len() + 1)).sum::<usize>()
            }
            MorphismTerm::Rename { mapping } => {
                3 + mapping
                    .iter()
                    .map(|(a, b)| gamma(a.len() + 1) + gamma(b.len() + 1))
                    .sum::<usize>()
            }
            MorphismTerm::Derive { fields } => {
                3 + fields
                    .iter()
                    .map(|(name, value)| gamma(name.len() + 1) + value_bits(value))
                    .sum::<usize>()
            }
            MorphismTerm::Compose { first, second } => {
                3 + morphism_bits(first) + morphism_bits(second)
            }
            MorphismTerm::Product { factors } => {
                3 + factors.iter().map(morphism_bits).sum::<usize>()
            }
            MorphismTerm::Coproduct { branches } => {
                3 + branches.iter().map(morphism_bits).sum::<usize>()
            }
        }
    }
    fn term_bits(term: &CoreTerm) -> usize {
        match term {
            CoreTerm::Source { relation } => 3 + gamma(relation.len() + 1),
            CoreTerm::Literal { relation } => {
                4 + relation
                    .rows
                    .len()
                    .saturating_mul(relation.attributes.len())
                    .saturating_mul(65)
            }
            CoreTerm::Select { input, predicate } => {
                4 + term_bits(input) + predicate_bits(predicate)
            }
            CoreTerm::Image { input, morphism } => 4 + term_bits(input) + morphism_bits(morphism),
            CoreTerm::Join { left, right, on } => {
                4 + term_bits(left) + term_bits(right) + gamma(on.len() + 1)
            }
            CoreTerm::Union { left, right } | CoreTerm::Difference { left, right } => {
                4 + term_bits(left) + term_bits(right)
            }
            CoreTerm::Group {
                input,
                keys,
                aggregates,
            } => 5 + term_bits(input) + gamma(keys.len() + 1) + aggregates.len().saturating_mul(8),
            CoreTerm::TransitiveClosure { input, .. } => 5 + term_bits(input),
            CoreTerm::Overlay {
                base, layer, keys, ..
            } => 5 + term_bits(base) + term_bits(layer) + gamma(keys.len() + 1),
            CoreTerm::Fold { input, keys, .. } => 5 + term_bits(input) + gamma(keys.len() + 1),
            CoreTerm::IsomorphicCompletion { input, keys, .. } => {
                7 + term_bits(input) + gamma(keys.len() + 1) + 65
            }
            CoreTerm::QuotientRepair { input, keys, .. } => {
                6 + term_bits(input) + gamma(keys.len() + 1)
            }
            CoreTerm::EntityTrace {
                input,
                keys,
                macros,
                ..
            } => {
                8 + term_bits(input)
                    + gamma(keys.len() + 1)
                    + macros
                        .iter()
                        .map(|item| {
                            65 + item.normalized_support.len().saturating_mul(keys.len())
                                + item.paint_offsets.len().saturating_mul(keys.len() * 4)
                        })
                        .sum::<usize>()
            }
            CoreTerm::Scan { seed, .. } => 5 + term_bits(seed),
            CoreTerm::LeastFixedPoint {
                seed,
                step,
                maximum_iterations,
            } => 5 + term_bits(seed) + term_bits(step) + gamma(*maximum_iterations + 1),
            CoreTerm::Compose { first, second } => 4 + term_bits(first) + term_bits(second),
            CoreTerm::Abstract { parameters, body } => {
                5 + parameters.iter().map(|x| gamma(x.len() + 1)).sum::<usize>() + term_bits(body)
            }
        }
    }
    Ok(8 + program
        .bindings
        .iter()
        .map(|item| gamma(item.name.len() + 1) + term_bits(&item.term))
        .sum::<usize>()
        + program
            .outputs
            .iter()
            .map(|(name, term)| gamma(name.len() + 1) + term_bits(term))
            .sum::<usize>())
}

fn gamma(value: usize) -> usize {
    let bits = usize::BITS as usize - value.max(1).leading_zeros() as usize;
    bits.saturating_mul(2).saturating_sub(1)
}
