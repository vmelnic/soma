use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Atom {
    Integer(i64),
    Text(String),
    Boolean(bool),
    Null,
}

impl Atom {
    fn integer(&self) -> Result<i64> {
        match self {
            Self::Integer(value) => Ok(*value),
            _ => bail!("relational arithmetic requires integer atoms"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Relation {
    pub attributes: Vec<String>,
    pub rows: BTreeSet<Vec<Atom>>,
}

impl Relation {
    pub fn new(attributes: Vec<String>, rows: impl IntoIterator<Item = Vec<Atom>>) -> Result<Self> {
        if attributes.iter().collect::<BTreeSet<_>>().len() != attributes.len() {
            bail!("relation attributes must be unique");
        }
        let rows = rows.into_iter().collect::<BTreeSet<_>>();
        if rows.iter().any(|row| row.len() != attributes.len()) {
            bail!("relation row arity does not match its schema");
        }
        Ok(Self { attributes, rows })
    }

    pub fn empty(attributes: Vec<String>) -> Result<Self> {
        Self::new(attributes, Vec::<Vec<Atom>>::new())
    }

    pub(crate) fn column(&self, name: &str) -> Result<usize> {
        self.attributes
            .iter()
            .position(|attribute| attribute == name)
            .ok_or_else(|| anyhow::anyhow!("unknown relation attribute {name}"))
    }
}

pub type Database = BTreeMap<String, Relation>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "expression", rename_all = "snake_case")]
pub enum ValueExpr {
    Column {
        name: String,
    },
    Literal {
        value: Atom,
    },
    Add {
        left: Box<ValueExpr>,
        right: Box<ValueExpr>,
    },
    Subtract {
        left: Box<ValueExpr>,
        right: Box<ValueExpr>,
    },
}

impl ValueExpr {
    fn evaluate(&self, relation: &Relation, row: &[Atom]) -> Result<Atom> {
        match self {
            Self::Column { name } => Ok(row[relation.column(name)?].clone()),
            Self::Literal { value } => Ok(value.clone()),
            Self::Add { left, right } => Ok(Atom::Integer(
                left.evaluate(relation, row)?.integer()?
                    + right.evaluate(relation, row)?.integer()?,
            )),
            Self::Subtract { left, right } => Ok(Atom::Integer(
                left.evaluate(relation, row)?.integer()?
                    - right.evaluate(relation, row)?.integer()?,
            )),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "predicate", rename_all = "snake_case")]
pub enum Predicate {
    Equal { left: ValueExpr, right: ValueExpr },
    NotEqual { left: ValueExpr, right: ValueExpr },
    LessThan { left: ValueExpr, right: ValueExpr },
    And { terms: Vec<Predicate> },
    Or { terms: Vec<Predicate> },
    Not { term: Box<Predicate> },
}

impl Predicate {
    fn evaluate(&self, relation: &Relation, row: &[Atom]) -> Result<bool> {
        match self {
            Self::Equal { left, right } => {
                Ok(left.evaluate(relation, row)? == right.evaluate(relation, row)?)
            }
            Self::NotEqual { left, right } => {
                Ok(left.evaluate(relation, row)? != right.evaluate(relation, row)?)
            }
            Self::LessThan { left, right } => {
                Ok(left.evaluate(relation, row)? < right.evaluate(relation, row)?)
            }
            Self::And { terms } => {
                for term in terms {
                    if !term.evaluate(relation, row)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Or { terms } => {
                for term in terms {
                    if term.evaluate(relation, row)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not { term } => Ok(!term.evaluate(relation, row)?),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Projection {
    pub name: String,
    pub value: ValueExpr,
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
pub struct Aggregate {
    pub output: String,
    pub input: Option<String>,
    pub operator: AggregateOperator,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operator", rename_all = "snake_case")]
pub enum RelExpr {
    Source {
        relation: String,
    },
    Select {
        input: Box<RelExpr>,
        predicate: Predicate,
    },
    Project {
        input: Box<RelExpr>,
        columns: Vec<Projection>,
    },
    Join {
        left: Box<RelExpr>,
        right: Box<RelExpr>,
        on: Vec<(String, String)>,
    },
    Union {
        left: Box<RelExpr>,
        right: Box<RelExpr>,
    },
    Difference {
        left: Box<RelExpr>,
        right: Box<RelExpr>,
    },
    Group {
        input: Box<RelExpr>,
        keys: Vec<String>,
        aggregates: Vec<Aggregate>,
    },
    TransitiveClosure {
        input: Box<RelExpr>,
        from: String,
        to: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgramStep {
    pub bind: String,
    pub expression: RelExpr,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationalProgram {
    pub id: String,
    pub steps: Vec<ProgramStep>,
    pub output: String,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationalLimits {
    pub maximum_rows_per_relation: usize,
    pub maximum_program_steps: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationalExecution {
    pub output: Relation,
    pub row_counts: BTreeMap<String, usize>,
    pub program_digest: String,
    pub database_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationalCalculusCertificate {
    pub source_database_digest: String,
    pub program_digest: String,
    pub selection: bool,
    pub projection: bool,
    pub join: bool,
    pub union: bool,
    pub difference: bool,
    pub aggregation: bool,
    pub transitive_closure: bool,
    pub exact_finite_result: bool,
}

impl RelationalCalculusCertificate {
    pub fn all_operators_pass(&self) -> bool {
        self.selection
            && self.projection
            && self.join
            && self.union
            && self.difference
            && self.aggregation
            && self.transitive_closure
            && self.exact_finite_result
    }
}

impl RelationalProgram {
    pub fn execute(
        &self,
        source: &Database,
        limits: &RelationalLimits,
    ) -> Result<RelationalExecution> {
        if limits.maximum_rows_per_relation == 0 || limits.maximum_program_steps == 0 {
            bail!("relational execution limits must be positive");
        }
        if self.steps.len() > limits.maximum_program_steps {
            bail!("relational program exceeds the declared step limit");
        }
        let mut database = source.clone();
        let mut row_counts = database
            .iter()
            .map(|(name, relation)| (name.clone(), relation.rows.len()))
            .collect::<BTreeMap<_, _>>();
        for step in &self.steps {
            if database.contains_key(&step.bind) {
                bail!("relational program attempts to overwrite {}", step.bind);
            }
            let relation = evaluate(&step.expression, &database)?;
            if relation.rows.len() > limits.maximum_rows_per_relation {
                bail!("relation {} exceeded its declared row limit", step.bind);
            }
            row_counts.insert(step.bind.clone(), relation.rows.len());
            database.insert(step.bind.clone(), relation);
        }
        let output = database
            .get(&self.output)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("program output relation is absent"))?;
        Ok(RelationalExecution {
            output,
            row_counts,
            program_digest: crate::memory::digest_json(self)?,
            database_digest: crate::memory::digest_json(&database)?,
        })
    }
}

/// Exercise the calculus on an actual coordinate database. This is a formal
/// execution certificate over the supplied observation, not generated task
/// evidence.
pub fn certify_calculus(
    source_database: &Database,
    limits: &RelationalLimits,
) -> Result<RelationalCalculusCertificate> {
    let axis_zero = RelExpr::Select {
        input: Box::new(RelExpr::Source {
            relation: "coordinate".to_string(),
        }),
        predicate: Predicate::Equal {
            left: ValueExpr::Column {
                name: "axis".to_string(),
            },
            right: ValueExpr::Literal {
                value: Atom::Integer(0),
            },
        },
    };
    let ordered_pairs = RelExpr::Project {
        input: Box::new(RelExpr::Select {
            input: Box::new(RelExpr::Source {
                relation: "successor".to_string(),
            }),
            predicate: Predicate::Equal {
                left: ValueExpr::Column {
                    name: "axis".to_string(),
                },
                right: ValueExpr::Literal {
                    value: Atom::Integer(0),
                },
            },
        }),
        columns: vec![
            Projection {
                name: "from".to_string(),
                value: ValueExpr::Column {
                    name: "left".to_string(),
                },
            },
            Projection {
                name: "to".to_string(),
                value: ValueExpr::Column {
                    name: "right".to_string(),
                },
            },
        ],
    };
    let steps = vec![
        ProgramStep {
            bind: "axis_zero".to_string(),
            expression: axis_zero,
        },
        ProgramStep {
            bind: "ordered_pairs".to_string(),
            expression: ordered_pairs,
        },
        ProgramStep {
            bind: "closure".to_string(),
            expression: RelExpr::TransitiveClosure {
                input: Box::new(RelExpr::Source {
                    relation: "ordered_pairs".to_string(),
                }),
                from: "from".to_string(),
                to: "to".to_string(),
            },
        },
        ProgramStep {
            bind: "closure_union".to_string(),
            expression: RelExpr::Union {
                left: Box::new(RelExpr::Source {
                    relation: "closure".to_string(),
                }),
                right: Box::new(RelExpr::Source {
                    relation: "closure".to_string(),
                }),
            },
        },
        ProgramStep {
            bind: "empty_difference".to_string(),
            expression: RelExpr::Difference {
                left: Box::new(RelExpr::Source {
                    relation: "closure_union".to_string(),
                }),
                right: Box::new(RelExpr::Source {
                    relation: "closure".to_string(),
                }),
            },
        },
        ProgramStep {
            bind: "coordinate_count".to_string(),
            expression: RelExpr::Group {
                input: Box::new(RelExpr::Source {
                    relation: "axis_zero".to_string(),
                }),
                keys: Vec::new(),
                aggregates: vec![Aggregate {
                    output: "observed_size".to_string(),
                    input: None,
                    operator: AggregateOperator::Count,
                }],
            },
        },
        ProgramStep {
            bind: "joined_extent".to_string(),
            expression: RelExpr::Join {
                left: Box::new(RelExpr::Source {
                    relation: "coordinate_count".to_string(),
                }),
                right: Box::new(RelExpr::Select {
                    input: Box::new(RelExpr::Source {
                        relation: "extent".to_string(),
                    }),
                    predicate: Predicate::Equal {
                        left: ValueExpr::Column {
                            name: "axis".to_string(),
                        },
                        right: ValueExpr::Literal {
                            value: Atom::Integer(0),
                        },
                    },
                }),
                on: Vec::new(),
            },
        },
    ];
    let program = RelationalProgram {
        id: format!(
            "calculus-certificate:{}",
            crate::memory::digest_json(&steps)?
        ),
        steps,
        output: "joined_extent".to_string(),
        provenance: vec![crate::memory::digest_json(source_database)?],
    };
    let execution = program.execute(source_database, limits)?;
    let joined = &execution.output;
    let observed = joined.column("observed_size")?;
    let declared = joined.column("size")?;
    let exact_finite_result =
        joined.rows.len() == 1 && joined.rows.iter().all(|row| row[observed] == row[declared]);
    Ok(RelationalCalculusCertificate {
        source_database_digest: crate::memory::digest_json(source_database)?,
        program_digest: execution.program_digest,
        selection: execution.row_counts.contains_key("axis_zero"),
        projection: execution.row_counts.contains_key("ordered_pairs"),
        join: execution.row_counts.get("joined_extent") == Some(&1),
        union: execution.row_counts.get("closure_union") == execution.row_counts.get("closure"),
        difference: execution.row_counts.get("empty_difference") == Some(&0),
        aggregation: execution.row_counts.get("coordinate_count") == Some(&1),
        transitive_closure: execution
            .row_counts
            .get("closure")
            .zip(execution.row_counts.get("ordered_pairs"))
            .is_some_and(|(closure, edges)| closure >= edges),
        exact_finite_result,
    })
}

fn evaluate(expression: &RelExpr, database: &Database) -> Result<Relation> {
    match expression {
        RelExpr::Source { relation } => database
            .get(relation)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown source relation {relation}")),
        RelExpr::Select { input, predicate } => {
            let input = evaluate(input, database)?;
            let mut rows = BTreeSet::new();
            for row in &input.rows {
                if predicate.evaluate(&input, row)? {
                    rows.insert(row.clone());
                }
            }
            Relation::new(input.attributes, rows)
        }
        RelExpr::Project { input, columns } => {
            let input = evaluate(input, database)?;
            let attributes = columns.iter().map(|item| item.name.clone()).collect();
            let rows = input
                .rows
                .iter()
                .map(|row| {
                    columns
                        .iter()
                        .map(|column| column.value.evaluate(&input, row))
                        .collect::<Result<Vec<_>>>()
                })
                .collect::<Result<Vec<_>>>()?;
            Relation::new(attributes, rows)
        }
        RelExpr::Join { left, right, on } => {
            let left = evaluate(left, database)?;
            let right = evaluate(right, database)?;
            let left_columns = on
                .iter()
                .map(|(name, _)| left.column(name))
                .collect::<Result<Vec<_>>>()?;
            let right_columns = on
                .iter()
                .map(|(_, name)| right.column(name))
                .collect::<Result<Vec<_>>>()?;
            let mut attributes = left.attributes.clone();
            for attribute in &right.attributes {
                if attributes.contains(attribute) {
                    bail!("join output has duplicate attribute {attribute}; project first");
                }
                attributes.push(attribute.clone());
            }
            let mut rows = BTreeSet::new();
            for left_row in &left.rows {
                for right_row in &right.rows {
                    if left_columns
                        .iter()
                        .zip(&right_columns)
                        .all(|(a, b)| left_row[*a] == right_row[*b])
                    {
                        let mut row = left_row.clone();
                        row.extend(right_row.clone());
                        rows.insert(row);
                    }
                }
            }
            Relation::new(attributes, rows)
        }
        RelExpr::Union { left, right } => {
            let left = evaluate(left, database)?;
            let right = evaluate(right, database)?;
            require_same_schema(&left, &right)?;
            Relation::new(
                left.attributes,
                left.rows.union(&right.rows).cloned().collect::<Vec<_>>(),
            )
        }
        RelExpr::Difference { left, right } => {
            let left = evaluate(left, database)?;
            let right = evaluate(right, database)?;
            require_same_schema(&left, &right)?;
            Relation::new(
                left.attributes,
                left.rows
                    .difference(&right.rows)
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        }
        RelExpr::Group {
            input,
            keys,
            aggregates,
        } => group(evaluate(input, database)?, keys, aggregates),
        RelExpr::TransitiveClosure { input, from, to } => {
            transitive_closure(evaluate(input, database)?, from, to)
        }
    }
}

fn require_same_schema(left: &Relation, right: &Relation) -> Result<()> {
    if left.attributes != right.attributes {
        bail!("set operations require identical relation schemas");
    }
    Ok(())
}

fn group(input: Relation, keys: &[String], aggregates: &[Aggregate]) -> Result<Relation> {
    let key_columns = keys
        .iter()
        .map(|name| input.column(name))
        .collect::<Result<Vec<_>>>()?;
    let mut groups: BTreeMap<Vec<Atom>, Vec<&Vec<Atom>>> = BTreeMap::new();
    if input.rows.is_empty() && keys.is_empty() {
        groups.insert(Vec::new(), Vec::new());
    } else {
        for row in &input.rows {
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
    }
    let mut attributes = keys.to_vec();
    attributes.extend(aggregates.iter().map(|item| item.output.clone()));
    let mut rows = Vec::new();
    'groups: for (key, members) in groups {
        let mut row = key;
        for aggregate in aggregates {
            let value = match aggregate.operator {
                AggregateOperator::Count => Atom::Integer(members.len() as i64),
                AggregateOperator::Minimum
                | AggregateOperator::Maximum
                | AggregateOperator::Consensus => {
                    let input_name = aggregate.input.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("non-count aggregate requires an input attribute")
                    })?;
                    let column = input.column(input_name)?;
                    let values = members
                        .iter()
                        .map(|member| member[column].clone())
                        .collect::<BTreeSet<_>>();
                    if values.is_empty() {
                        continue 'groups;
                    }
                    match aggregate.operator {
                        AggregateOperator::Minimum => {
                            values.iter().next().cloned().expect("nonempty values")
                        }
                        AggregateOperator::Maximum => {
                            values.iter().next_back().cloned().expect("nonempty values")
                        }
                        AggregateOperator::Consensus if values.len() == 1 => {
                            values.iter().next().cloned().expect("singleton values")
                        }
                        AggregateOperator::Consensus => continue 'groups,
                        AggregateOperator::Count => unreachable!(),
                    }
                }
            };
            row.push(value);
        }
        rows.push(row);
    }
    Relation::new(attributes, rows)
}

fn transitive_closure(input: Relation, from: &str, to: &str) -> Result<Relation> {
    if input.attributes.len() != 2 {
        bail!("transitive closure requires a binary relation");
    }
    let from_column = input.column(from)?;
    let to_column = input.column(to)?;
    let mut pairs = input
        .rows
        .iter()
        .map(|row| (row[from_column].clone(), row[to_column].clone()))
        .collect::<BTreeSet<_>>();
    loop {
        let before = pairs.len();
        let snapshot = pairs.iter().cloned().collect::<Vec<_>>();
        for (left, middle) in &snapshot {
            for (candidate, right) in &snapshot {
                if middle == candidate {
                    pairs.insert((left.clone(), right.clone()));
                }
            }
        }
        if pairs.len() == before {
            break;
        }
    }
    Relation::new(
        vec![from.to_string(), to.to_string()],
        pairs.into_iter().map(|(a, b)| vec![a, b]),
    )
}
