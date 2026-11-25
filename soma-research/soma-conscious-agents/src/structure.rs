use crate::memory::digest_json;
use crate::relational::{Atom, Database, Relation};
use crate::world::{RelationalFrame, WorldProtocol};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AtomSort {
    Integer,
    Text,
    Boolean,
    Null,
    Mixed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationSignature {
    pub name: String,
    pub attributes: Vec<String>,
    pub sorts: Vec<AtomSort>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FiniteRelationalStructure {
    pub relations: Vec<RelationSignature>,
    pub carrier: BTreeMap<AtomSort, BTreeSet<Atom>>,
    pub frame_digest: String,
    pub signature_digest: String,
    /// Isomorphism-level type identity that ignores relation and attribute
    /// names while retaining relation arities and atom sorts.
    pub structural_type_digest: String,
    pub provenance: Vec<String>,
}

impl FiniteRelationalStructure {
    /// Infer only typed carriers and relation signatures.  No coordinate,
    /// object, benchmark, or solution semantics enter at this layer.
    pub fn infer(frame: &RelationalFrame) -> Result<Self> {
        if frame.relations.is_empty() {
            bail!("a finite relational structure needs at least one relation");
        }
        let mut carrier = BTreeMap::<AtomSort, BTreeSet<Atom>>::new();
        let mut relations = Vec::new();
        for (name, relation) in &frame.relations {
            let mut sorts = Vec::new();
            for column in 0..relation.attributes.len() {
                let observed = relation
                    .rows
                    .iter()
                    .map(|row| atom_sort(&row[column]))
                    .collect::<BTreeSet<_>>();
                let sort = if observed.len() == 1 {
                    observed.iter().next().cloned().unwrap_or(AtomSort::Null)
                } else {
                    AtomSort::Mixed
                };
                sorts.push(sort);
                for row in &relation.rows {
                    carrier
                        .entry(atom_sort(&row[column]))
                        .or_default()
                        .insert(row[column].clone());
                }
            }
            relations.push(RelationSignature {
                name: name.clone(),
                attributes: relation.attributes.clone(),
                sorts,
            });
        }
        let signature_digest = digest_json(&relations)?;
        let mut structural_type = relations
            .iter()
            .map(|relation| relation.sorts.clone())
            .collect::<Vec<_>>();
        structural_type.sort();
        let structural_type_digest = digest_json(&structural_type)?;
        Ok(Self {
            relations,
            carrier,
            frame_digest: frame.payload_digest.clone(),
            signature_digest,
            structural_type_digest,
            provenance: frame.provenance.clone(),
        })
    }
}

fn atom_sort(atom: &Atom) -> AtomSort {
    match atom {
        Atom::Integer(_) => AtomSort::Integer,
        Atom::Text(_) => AtomSort::Text,
        Atom::Boolean(_) => AtomSort::Boolean,
        Atom::Null => AtomSort::Null,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexedSchema {
    pub data_relation: String,
    pub key_attributes: Vec<String>,
    pub value_attribute: String,
    pub relation_attributes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexedStructure {
    pub schema: IndexedSchema,
    pub domains: Vec<Vec<Atom>>,
    #[serde(with = "index_value_map")]
    pub values: BTreeMap<Vec<usize>, Atom>,
    pub frame_digest: String,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralCapabilityCertificate {
    pub capability: String,
    pub source_frame_digest: String,
    pub selected_relation: String,
    pub rank: usize,
    pub total_function: bool,
    pub structural_evidence_digest: String,
}

impl IndexedStructure {
    pub fn infer(frame: &RelationalFrame) -> Result<Self> {
        let mut candidates = Vec::new();
        for (relation_name, relation) in &frame.relations {
            if relation.attributes.len() < 2 || relation.rows.is_empty() {
                continue;
            }
            for value_column in 0..relation.attributes.len() {
                let key_columns = (0..relation.attributes.len())
                    .filter(|column| *column != value_column)
                    .collect::<Vec<_>>();
                let keys = relation
                    .rows
                    .iter()
                    .map(|row| {
                        key_columns
                            .iter()
                            .map(|column| row[*column].clone())
                            .collect::<Vec<_>>()
                    })
                    .collect::<BTreeSet<_>>();
                if keys.len() != relation.rows.len() {
                    continue;
                }
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
                let product = domains.iter().try_fold(1usize, |size, domain| {
                    size.checked_mul(domain.len())
                        .ok_or_else(|| anyhow::anyhow!("indexed domain cardinality overflow"))
                })?;
                if product != relation.rows.len() {
                    continue;
                }
                let distinct_values = relation
                    .rows
                    .iter()
                    .map(|row| row[value_column].clone())
                    .collect::<BTreeSet<_>>()
                    .len();
                let repeated = relation.rows.len().saturating_sub(distinct_values);
                candidates.push((
                    relation.rows.len(),
                    repeated,
                    relation_name.clone(),
                    value_column,
                    key_columns,
                    domains,
                ));
            }
        }
        candidates.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then(right.1.cmp(&left.1))
                .then(left.2.cmp(&right.2))
        });
        let (_, _, relation_name, value_column, key_columns, domains) = candidates
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("frame has no complete finite-function relation"))?;
        let relation = &frame.relations[&relation_name];
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
        let mut values = BTreeMap::new();
        for row in &relation.rows {
            let key = key_columns
                .iter()
                .enumerate()
                .map(|(axis, column)| domain_indices[axis][&row[*column]])
                .collect::<Vec<_>>();
            values.insert(key, row[value_column].clone());
        }
        let schema = IndexedSchema {
            data_relation: relation_name,
            key_attributes: key_columns
                .iter()
                .map(|column| relation.attributes[*column].clone())
                .collect(),
            value_attribute: relation.attributes[value_column].clone(),
            relation_attributes: relation.attributes.clone(),
        };
        Ok(Self {
            schema,
            domains,
            values,
            frame_digest: frame.payload_digest.clone(),
            provenance: frame.provenance.clone(),
        })
    }

    pub fn rank(&self) -> usize {
        self.domains.len()
    }

    pub fn capability_certificate(&self) -> Result<StructuralCapabilityCertificate> {
        Ok(StructuralCapabilityCertificate {
            capability: "finite-product-function.v1".to_string(),
            source_frame_digest: self.frame_digest.clone(),
            selected_relation: self.schema.data_relation.clone(),
            rank: self.rank(),
            total_function: self.cardinality()
                == self.domains.iter().map(Vec::len).product::<usize>(),
            structural_evidence_digest: digest_json(&(
                &self.schema,
                &self.domains,
                canonical_values(&self.values),
            ))?,
        })
    }

    pub fn cardinality(&self) -> usize {
        self.values.len()
    }

    pub fn shape(&self) -> Vec<usize> {
        self.domains.iter().map(Vec::len).collect()
    }

    pub fn value(&self, key: &[usize]) -> Result<&Atom> {
        self.values
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("indexed key lies outside the structure"))
    }

    pub fn support(&self, value: &Atom) -> Vec<Vec<usize>> {
        self.values
            .iter()
            .filter_map(|(key, candidate)| (candidate == value).then_some(key.clone()))
            .collect()
    }

    pub fn distinct_values(&self) -> BTreeSet<Atom> {
        self.values.values().cloned().collect()
    }

    pub fn with_values(
        &self,
        domains: Vec<Vec<Atom>>,
        values: BTreeMap<Vec<usize>, Atom>,
    ) -> Result<Self> {
        let expected = domains.iter().try_fold(1usize, |size, domain| {
            size.checked_mul(domain.len())
                .ok_or_else(|| anyhow::anyhow!("indexed domain cardinality overflow"))
        })?;
        if values.len() != expected {
            bail!("generated finite function is not total on its product domain");
        }
        Ok(Self {
            schema: self.schema.clone(),
            domains,
            values,
            frame_digest: self.frame_digest.clone(),
            provenance: self.provenance.clone(),
        })
    }
}

pub fn canonical_values(values: &BTreeMap<Vec<usize>, Atom>) -> Vec<(Vec<usize>, Atom)> {
    values
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

mod index_value_map {
    use super::Atom;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S>(
        values: &BTreeMap<Vec<usize>, Atom>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        values.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<Vec<usize>, Atom>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<(Vec<usize>, Atom)>::deserialize(deserializer)?;
        let mut values = BTreeMap::new();
        for (key, value) in entries {
            if values.insert(key, value).is_some() {
                return Err(serde::de::Error::custom(
                    "indexed value representation contains a duplicate key",
                ));
            }
        }
        Ok(values)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "generator", rename_all = "snake_case")]
pub enum AuxiliaryRelationGenerator {
    DomainCardinality {
        relation: String,
        attributes: Vec<String>,
        axis_column: usize,
        cardinality_column: usize,
    },
    DomainMembers {
        relation: String,
        attributes: Vec<String>,
        axis_column: usize,
        member_column: usize,
    },
    DomainSuccessors {
        relation: String,
        attributes: Vec<String>,
        axis_column: usize,
        left_column: usize,
        right_column: usize,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexedFrameSchema {
    pub indexed: IndexedSchema,
    pub auxiliary: Vec<AuxiliaryRelationGenerator>,
    pub domain_labels: Vec<DomainLabelGenerator>,
    pub action_channel_relations: Vec<String>,
    pub schema_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "labels", rename_all = "snake_case")]
pub enum DomainLabelGenerator {
    Preserve,
    OrdinalInteger { origin: i64 },
}

impl IndexedFrameSchema {
    pub fn infer(
        frame: &RelationalFrame,
        structure: &IndexedStructure,
        protocol: &WorldProtocol,
    ) -> Result<Self> {
        let mut auxiliary = Vec::new();
        for (name, relation) in &frame.relations {
            if name == &structure.schema.data_relation {
                continue;
            }
            if let Some(generator) = infer_auxiliary(name, relation, &structure.domains) {
                auxiliary.push(generator);
            }
        }
        let action_channel_relations = protocol
            .channels
            .iter()
            .find(|channel| channel.direction == crate::world::ChannelDirection::Action)
            .map(|channel| channel.payload_relations.clone())
            .unwrap_or_else(|| vec![structure.schema.data_relation.clone()]);
        let domain_labels = structure
            .domains
            .iter()
            .map(|domain| match domain.first() {
                Some(Atom::Integer(origin))
                    if domain
                        .iter()
                        .enumerate()
                        .all(|(index, atom)| atom == &Atom::Integer(*origin + index as i64)) =>
                {
                    DomainLabelGenerator::OrdinalInteger { origin: *origin }
                }
                _ => DomainLabelGenerator::Preserve,
            })
            .collect::<Vec<_>>();
        let schema_digest = digest_json(&(
            &structure.schema,
            &auxiliary,
            &domain_labels,
            &action_channel_relations,
        ))?;
        Ok(Self {
            indexed: structure.schema.clone(),
            auxiliary,
            domain_labels,
            action_channel_relations,
            schema_digest,
        })
    }

    pub fn encode(
        &self,
        id: impl Into<String>,
        structure: &IndexedStructure,
        provenance: Vec<String>,
    ) -> Result<RelationalFrame> {
        if self.indexed.relation_attributes.len() != self.indexed.key_attributes.len() + 1
            || self.indexed.key_attributes.len() != structure.domains.len()
            || self.domain_labels.len() != structure.domains.len()
        {
            bail!("indexed action schema has unsupported non-key attributes");
        }
        let domains = structure
            .domains
            .iter()
            .zip(&self.domain_labels)
            .map(|(domain, labels)| match labels {
                DomainLabelGenerator::Preserve => domain.clone(),
                DomainLabelGenerator::OrdinalInteger { origin } => (0..domain.len())
                    .map(|index| Atom::Integer(*origin + index as i64))
                    .collect(),
            })
            .collect::<Vec<Vec<Atom>>>();
        let mut rows = Vec::new();
        for (key, value) in &structure.values {
            let mut row = Vec::with_capacity(self.indexed.relation_attributes.len());
            for attribute in &self.indexed.relation_attributes {
                if attribute == &self.indexed.value_attribute {
                    row.push(value.clone());
                } else {
                    let axis = self
                        .indexed
                        .key_attributes
                        .iter()
                        .position(|candidate| candidate == attribute)
                        .ok_or_else(|| anyhow::anyhow!("unknown indexed action attribute"))?;
                    row.push(domains[axis][key[axis]].clone());
                }
            }
            rows.push(row);
        }
        let data = Relation::new(self.indexed.relation_attributes.clone(), rows)?;
        let mut relations = Database::from([(self.indexed.data_relation.clone(), data)]);
        for generator in &self.auxiliary {
            let (name, relation) = generate_auxiliary(generator, &domains)?;
            relations.insert(name, relation);
        }
        relations.retain(|name, _| {
            self.action_channel_relations.contains(name) || name == &self.indexed.data_relation
        });
        RelationalFrame::create(id, relations, provenance)
    }
}

fn infer_auxiliary(
    name: &str,
    relation: &Relation,
    domains: &[Vec<Atom>],
) -> Option<AuxiliaryRelationGenerator> {
    if relation.attributes.len() == 2 && relation.rows.len() == domains.len() {
        for axis_column in 0..2 {
            let cardinality_column = 1 - axis_column;
            let valid = relation.rows.iter().all(|row| {
                match (&row[axis_column], &row[cardinality_column]) {
                    (Atom::Integer(axis), Atom::Integer(size)) if *axis >= 0 => domains
                        .get(*axis as usize)
                        .is_some_and(|domain| domain.len() as i64 == *size),
                    _ => false,
                }
            });
            if valid {
                return Some(AuxiliaryRelationGenerator::DomainCardinality {
                    relation: name.to_string(),
                    attributes: relation.attributes.clone(),
                    axis_column,
                    cardinality_column,
                });
            }
        }
    }
    if relation.attributes.len() == 2
        && relation.rows.len() == domains.iter().map(Vec::len).sum::<usize>()
    {
        for axis_column in 0..2 {
            let member_column = 1 - axis_column;
            let expected = domains
                .iter()
                .enumerate()
                .flat_map(|(axis, domain)| {
                    domain
                        .iter()
                        .cloned()
                        .map(move |member| (Atom::Integer(axis as i64), member))
                })
                .collect::<BTreeSet<_>>();
            let actual = relation
                .rows
                .iter()
                .map(|row| (row[axis_column].clone(), row[member_column].clone()))
                .collect::<BTreeSet<_>>();
            if actual == expected {
                return Some(AuxiliaryRelationGenerator::DomainMembers {
                    relation: name.to_string(),
                    attributes: relation.attributes.clone(),
                    axis_column,
                    member_column,
                });
            }
        }
    }
    if relation.attributes.len() == 3
        && relation.rows.len()
            == domains
                .iter()
                .map(|domain| domain.len().saturating_sub(1))
                .sum::<usize>()
    {
        for axis_column in 0..3 {
            for left_column in 0..3 {
                if left_column == axis_column {
                    continue;
                }
                let right_column =
                    (0..3).find(|column| *column != axis_column && *column != left_column)?;
                let expected = domains
                    .iter()
                    .enumerate()
                    .flat_map(|(axis, domain)| {
                        domain.windows(2).map(move |pair| {
                            (Atom::Integer(axis as i64), pair[0].clone(), pair[1].clone())
                        })
                    })
                    .collect::<BTreeSet<_>>();
                let actual = relation
                    .rows
                    .iter()
                    .map(|row| {
                        (
                            row[axis_column].clone(),
                            row[left_column].clone(),
                            row[right_column].clone(),
                        )
                    })
                    .collect::<BTreeSet<_>>();
                if actual == expected {
                    return Some(AuxiliaryRelationGenerator::DomainSuccessors {
                        relation: name.to_string(),
                        attributes: relation.attributes.clone(),
                        axis_column,
                        left_column,
                        right_column,
                    });
                }
            }
        }
    }
    None
}

fn generate_auxiliary(
    generator: &AuxiliaryRelationGenerator,
    domains: &[Vec<Atom>],
) -> Result<(String, Relation)> {
    match generator {
        AuxiliaryRelationGenerator::DomainCardinality {
            relation,
            attributes,
            axis_column,
            cardinality_column,
        } => {
            let rows = domains
                .iter()
                .enumerate()
                .map(|(axis, domain)| {
                    let mut row = vec![Atom::Null; 2];
                    row[*axis_column] = Atom::Integer(axis as i64);
                    row[*cardinality_column] = Atom::Integer(domain.len() as i64);
                    row
                })
                .collect::<Vec<_>>();
            Ok((relation.clone(), Relation::new(attributes.clone(), rows)?))
        }
        AuxiliaryRelationGenerator::DomainMembers {
            relation,
            attributes,
            axis_column,
            member_column,
        } => {
            let rows = domains
                .iter()
                .enumerate()
                .flat_map(|(axis, domain)| {
                    domain.iter().cloned().map(move |member| {
                        let mut row = vec![Atom::Null; 2];
                        row[*axis_column] = Atom::Integer(axis as i64);
                        row[*member_column] = member;
                        row
                    })
                })
                .collect::<Vec<_>>();
            Ok((relation.clone(), Relation::new(attributes.clone(), rows)?))
        }
        AuxiliaryRelationGenerator::DomainSuccessors {
            relation,
            attributes,
            axis_column,
            left_column,
            right_column,
        } => {
            let rows = domains
                .iter()
                .enumerate()
                .flat_map(|(axis, domain)| {
                    domain.windows(2).map(move |pair| {
                        let mut row = vec![Atom::Null; 3];
                        row[*axis_column] = Atom::Integer(axis as i64);
                        row[*left_column] = pair[0].clone();
                        row[*right_column] = pair[1].clone();
                        row
                    })
                })
                .collect::<Vec<_>>();
            Ok((relation.clone(), Relation::new(attributes.clone(), rows)?))
        }
    }
}

pub fn cartesian_indices(shape: &[usize]) -> Vec<Vec<usize>> {
    let mut result = vec![Vec::new()];
    for size in shape {
        let mut next = Vec::new();
        for prefix in &result {
            for index in 0..*size {
                let mut key = prefix.clone();
                key.push(index);
                next.push(key);
            }
        }
        result = next;
    }
    result
}

pub fn product_support(structure: &IndexedStructure, value: &Atom) -> Option<Vec<Vec<usize>>> {
    let support = structure.support(value);
    if support.is_empty() {
        return None;
    }
    let projections = (0..structure.rank())
        .map(|axis| {
            support
                .iter()
                .map(|key| key[axis])
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let expected = projections.iter().map(Vec::len).product::<usize>();
    (expected == support.len()).then_some(projections)
}
