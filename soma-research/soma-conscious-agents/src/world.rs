use crate::instrument::{
    load_manifest, InstrumentProtocol, MatrixInteractionSpec, SourceDescriptor,
};
use crate::memory::{digest_bytes, digest_json};
use crate::relational::{Atom, Database, Relation};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelDirection {
    Observation,
    Action,
    Feedback,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelSpec {
    pub id: String,
    pub direction: ChannelDirection,
    pub payload_relations: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldProtocol {
    pub schema: String,
    pub id: String,
    pub channels: Vec<ChannelSpec>,
    pub lifecycle: Vec<String>,
    pub source_digest: String,
    pub transport_codec: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicSourceDescriptor {
    pub id: String,
    pub boundary_identity_digest: String,
    pub role: String,
}

impl PublicSourceDescriptor {
    fn from_source(source: &SourceDescriptor) -> Result<Self> {
        Ok(Self {
            id: source.id.clone(),
            boundary_identity_digest: digest_json(&(source.id.as_str(), source.role.as_str()))?,
            role: source.role.clone(),
        })
    }
}

impl WorldProtocol {
    pub fn validate(&self) -> Result<()> {
        if self.schema != "soma.rtl.world-protocol.v1" {
            bail!("unsupported world protocol schema");
        }
        if self.channels.is_empty() || self.lifecycle.is_empty() {
            bail!("world protocol requires channels and a lifecycle");
        }
        if self
            .channels
            .iter()
            .map(|item| &item.id)
            .collect::<BTreeSet<_>>()
            .len()
            != self.channels.len()
        {
            bail!("world protocol channel identifiers must be unique");
        }
        for required in [
            ChannelDirection::Observation,
            ChannelDirection::Action,
            ChannelDirection::Feedback,
        ] {
            if !self.channels.iter().any(|item| item.direction == required) {
                bail!("world protocol is missing a required channel direction");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationalFrame {
    pub id: String,
    pub relations: Database,
    pub payload_digest: String,
    pub source_value_digest: Option<String>,
    pub provenance: Vec<String>,
}

impl RelationalFrame {
    pub fn create(
        id: impl Into<String>,
        relations: Database,
        provenance: Vec<String>,
    ) -> Result<Self> {
        let id = id.into();
        let payload_digest = digest_json(&(id.as_str(), &relations))?;
        Ok(Self {
            id,
            relations,
            payload_digest,
            source_value_digest: None,
            provenance,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Demonstration {
    pub episode_id: String,
    pub observation: RelationalFrame,
    pub demonstrated_action: RelationalFrame,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Query {
    pub episode_id: String,
    pub observation: RelationalFrame,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearningView {
    pub protocol: WorldProtocol,
    pub demonstrations: Vec<Demonstration>,
    pub queries: Vec<Query>,
    pub source: PublicSourceDescriptor,
    pub public_digest: String,
}

impl LearningView {
    fn create(
        protocol: WorldProtocol,
        demonstrations: Vec<Demonstration>,
        queries: Vec<Query>,
        source: PublicSourceDescriptor,
    ) -> Result<Self> {
        protocol.validate()?;
        if demonstrations.is_empty() || queries.is_empty() {
            bail!("an interaction needs demonstrations and queries");
        }
        #[derive(Serialize)]
        struct PublicIdentity<'a> {
            protocol: &'a WorldProtocol,
            demonstrations: &'a [Demonstration],
            queries: &'a [Query],
            source: &'a PublicSourceDescriptor,
        }
        let public_digest = digest_json(&PublicIdentity {
            protocol: &protocol,
            demonstrations: &demonstrations,
            queries: &queries,
            source: &source,
        })?;
        Ok(Self {
            protocol,
            demonstrations,
            queries,
            source,
            public_digest,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SealingCertificate {
    pub public_view_digest: String,
    pub answer_commitments: Vec<String>,
    pub answer_payloads_absent_from_learning_type: bool,
    pub actions_committed_before_reveal: bool,
    pub reveal_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionCommitment {
    pub episode_id: String,
    pub action: RelationalFrame,
    pub program_digest: String,
    pub public_view_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaseEvaluation {
    pub episode_id: String,
    pub predicted_digest: String,
    pub expected_commitment: String,
    pub exact: bool,
    pub predicted_shape: Vec<usize>,
    pub expected_shape: Vec<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvaluationReport {
    pub source_digest: String,
    pub public_view_digest: String,
    pub cases: Vec<CaseEvaluation>,
    pub all_exact: bool,
    pub actions_committed_before_reveal: bool,
    pub feedback_digest: String,
}

struct SealedAnswer {
    episode_id: String,
    frame: RelationalFrame,
    commitment: String,
}

/// The answer payload is private to this module. Induction accepts only a
/// `LearningView`, whose type has no answer field. Evaluation is the sole
/// operation that reads the vault, and it first validates complete action
/// commitments against the public-view digest.
pub struct SealedEvaluation {
    public: LearningView,
    answers: Vec<SealedAnswer>,
    reveal_count: usize,
    source_digest: String,
}

impl SealedEvaluation {
    pub fn learning_view(&self) -> &LearningView {
        &self.public
    }

    pub fn sealing_certificate(&self) -> SealingCertificate {
        SealingCertificate {
            public_view_digest: self.public.public_digest.clone(),
            answer_commitments: self
                .answers
                .iter()
                .map(|item| item.commitment.clone())
                .collect(),
            answer_payloads_absent_from_learning_type: true,
            actions_committed_before_reveal: self.reveal_count > 0,
            reveal_count: self.reveal_count,
        }
    }

    pub fn evaluate(&mut self, actions: Vec<ActionCommitment>) -> Result<EvaluationReport> {
        if self.reveal_count > 0 {
            bail!("sealed answers may be revealed only once");
        }
        if actions.len() != self.answers.len() {
            bail!("every query needs exactly one committed action");
        }
        let actions_by_episode = actions
            .into_iter()
            .map(|action| (action.episode_id.clone(), action))
            .collect::<BTreeMap<_, _>>();
        if actions_by_episode.len() != self.answers.len() {
            bail!("action episode identifiers must be unique");
        }
        for answer in &self.answers {
            let action = actions_by_episode
                .get(&answer.episode_id)
                .ok_or_else(|| anyhow::anyhow!("missing committed action for a query"))?;
            if action.public_view_digest != self.public.public_digest {
                bail!("action was not committed against this public learning view");
            }
            if action.program_digest.is_empty() {
                bail!("action commitment is missing its executable program digest");
            }
        }

        // Crossing this line is the reveal. All action payloads and their
        // program/public-view commitments have already been validated.
        self.reveal_count = 1;
        let mut cases = Vec::new();
        for answer in &self.answers {
            let action = &actions_by_episode[&answer.episode_id];
            let predicted = MatrixCodec::decode(&action.action)?;
            let expected = MatrixCodec::decode(&answer.frame)?;
            cases.push(CaseEvaluation {
                episode_id: answer.episode_id.clone(),
                predicted_digest: action.action.payload_digest.clone(),
                expected_commitment: answer.commitment.clone(),
                exact: predicted == expected,
                predicted_shape: matrix_shape(&predicted)?,
                expected_shape: matrix_shape(&expected)?,
            });
        }
        let all_exact = cases.iter().all(|case| case.exact);
        let feedback_digest = digest_json(&cases)?;
        Ok(EvaluationReport {
            source_digest: self.source_digest.clone(),
            public_view_digest: self.public.public_digest.clone(),
            cases,
            all_exact,
            actions_committed_before_reveal: true,
            feedback_digest,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodecCertificate {
    pub frame_digest: String,
    pub source_value_digest: String,
    pub decoded_value_digest: String,
    pub lossless: bool,
    pub cognitive_operators: usize,
}

pub trait LosslessTransportCodec {
    type Payload;

    fn codec_id(&self) -> &'static str;
    fn encode_frame(
        &self,
        id: &str,
        payload: &Self::Payload,
        provenance: Vec<String>,
    ) -> Result<RelationalFrame>;
    fn decode_frame(&self, frame: &RelationalFrame) -> Result<Self::Payload>;
}

pub struct MatrixCodec;

impl LosslessTransportCodec for MatrixCodec {
    type Payload = Value;

    fn codec_id(&self) -> &'static str {
        "finite-rank-coordinate-incidence.v1"
    }

    fn encode_frame(
        &self,
        id: &str,
        payload: &Self::Payload,
        provenance: Vec<String>,
    ) -> Result<RelationalFrame> {
        Self::encode(id, payload, provenance)
    }

    fn decode_frame(&self, frame: &RelationalFrame) -> Result<Self::Payload> {
        Self::decode(frame)
    }
}

impl MatrixCodec {
    pub fn encode(
        id: impl Into<String>,
        matrix: &Value,
        provenance: Vec<String>,
    ) -> Result<RelationalFrame> {
        let rows = matrix
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("matrix payload must be an array"))?;
        if rows.is_empty() {
            bail!("matrix payload cannot be empty");
        }
        let width = rows[0]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("matrix row must be an array"))?
            .len();
        if width == 0
            || rows
                .iter()
                .any(|row| row.as_array().map(Vec::len) != Some(width))
        {
            bail!("matrix payload must be nonempty and rectangular");
        }
        let extent = Relation::new(
            vec!["axis".to_string(), "size".to_string()],
            vec![
                vec![Atom::Integer(0), Atom::Integer(rows.len() as i64)],
                vec![Atom::Integer(1), Atom::Integer(width as i64)],
            ],
        )?;
        let mut coordinate_rows = Vec::new();
        let mut successor_rows = Vec::new();
        for (axis, length) in [(0i64, rows.len()), (1i64, width)] {
            for index in 0..length {
                coordinate_rows.push(vec![Atom::Integer(axis), Atom::Integer(index as i64)]);
                if index + 1 < length {
                    successor_rows.push(vec![
                        Atom::Integer(axis),
                        Atom::Integer(index as i64),
                        Atom::Integer((index + 1) as i64),
                    ]);
                }
            }
        }
        let coordinate = Relation::new(
            vec!["axis".to_string(), "index".to_string()],
            coordinate_rows,
        )?;
        let successor = Relation::new(
            vec!["axis".to_string(), "left".to_string(), "right".to_string()],
            successor_rows,
        )?;
        let mut incidence_rows = Vec::with_capacity(rows.len() * width);
        for (row, values) in rows.iter().enumerate() {
            let values = values
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("matrix row changed during encoding"))?;
            for (column, value) in values.iter().enumerate() {
                incidence_rows.push(vec![
                    Atom::Integer(row as i64),
                    Atom::Integer(column as i64),
                    atom_from_json(value)?,
                ]);
            }
        }
        let incidence = Relation::new(
            vec!["row".to_string(), "column".to_string(), "value".to_string()],
            incidence_rows,
        )?;
        let mut frame = RelationalFrame::create(
            id,
            BTreeMap::from([
                ("extent".to_string(), extent),
                ("coordinate".to_string(), coordinate),
                ("successor".to_string(), successor),
                ("incidence".to_string(), incidence),
            ]),
            provenance,
        )?;
        frame.source_value_digest = Some(digest_json(matrix)?);
        Ok(frame)
    }

    pub fn decode(frame: &RelationalFrame) -> Result<Value> {
        let extent = frame
            .relations
            .get("extent")
            .ok_or_else(|| anyhow::anyhow!("frame has no extent relation"))?;
        let incidence = frame
            .relations
            .get("incidence")
            .or_else(|| frame.relations.get("action"))
            .ok_or_else(|| anyhow::anyhow!("frame has no incidence/action relation"))?;
        let mut dimensions = BTreeMap::new();
        for row in &extent.rows {
            let axis = atom_integer(&row[extent.column("axis")?])?;
            let size = atom_integer(&row[extent.column("size")?])?;
            if axis < 0 || size <= 0 {
                bail!("matrix extents must be positive");
            }
            dimensions.insert(axis as usize, size as usize);
        }
        let height = *dimensions
            .get(&0)
            .ok_or_else(|| anyhow::anyhow!("matrix row extent is absent"))?;
        let width = *dimensions
            .get(&1)
            .ok_or_else(|| anyhow::anyhow!("matrix column extent is absent"))?;
        let row_column = incidence.column("row")?;
        let column_column = incidence.column("column")?;
        let value_column = incidence.column("value")?;
        let mut cells = vec![vec![None; width]; height];
        for tuple in &incidence.rows {
            let row = atom_integer(&tuple[row_column])?;
            let column = atom_integer(&tuple[column_column])?;
            if row < 0 || column < 0 || row as usize >= height || column as usize >= width {
                bail!("matrix incidence lies outside declared extents");
            }
            let slot = &mut cells[row as usize][column as usize];
            if slot.is_some() {
                bail!("matrix incidence contains a duplicate coordinate");
            }
            *slot = Some(json_from_atom(&tuple[value_column]));
        }
        if cells.iter().flatten().any(Option::is_none) {
            bail!("matrix incidence has coordinate gaps");
        }
        let rows = cells
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|cell| {
                        cell.ok_or_else(|| anyhow::anyhow!("matrix incidence gap after validation"))
                    })
                    .collect::<Result<Vec<_>>>()
                    .map(Value::Array)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Value::Array(rows))
    }

    pub fn certify(frame: &RelationalFrame, source: &Value) -> Result<CodecCertificate> {
        let decoded = Self::decode(frame)?;
        Ok(CodecCertificate {
            frame_digest: frame.payload_digest.clone(),
            source_value_digest: digest_json(source)?,
            decoded_value_digest: digest_json(&decoded)?,
            lossless: decoded == *source
                && frame.source_value_digest.as_ref() == Some(&digest_json(&decoded)?),
            cognitive_operators: 0,
        })
    }
}

pub fn load_sealed_interaction(manifest_path: &Path) -> Result<SealedEvaluation> {
    let manifest = load_manifest(manifest_path)?;
    let (collections, matrix_fields, interaction) = match &manifest.protocol {
        InstrumentProtocol::MatrixJson {
            collections,
            matrix_fields,
            interaction: Some(interaction),
        } => (collections, matrix_fields, interaction),
        InstrumentProtocol::MatrixJson {
            interaction: None, ..
        } => bail!("matrix manifest has no operational interaction declaration"),
        _ => bail!("this world loader requires a matrix transport protocol"),
    };
    validate_interaction_spec(collections, matrix_fields, interaction)?;
    let source_path = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&manifest.source.path);
    let bytes = fs::read(&source_path)
        .with_context(|| format!("cannot read external source {}", source_path.display()))?;
    let actual_digest = digest_bytes(&bytes);
    if actual_digest != manifest.source.sha256 {
        bail!("external interaction source checksum mismatch");
    }
    let document: Value = serde_json::from_slice(&bytes)?;
    let root = document
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("interaction document root must be an object"))?;
    let protocol = matrix_world_protocol(&manifest.source, interaction)?;
    let public_source = PublicSourceDescriptor::from_source(&manifest.source)?;
    let provenance = vec![public_source.boundary_identity_digest.clone()];
    let demonstrations = cases(root, &interaction.demonstration_collection)?
        .iter()
        .enumerate()
        .map(|(index, case)| {
            let episode_id = format!("demonstration:{index}");
            Ok(Demonstration {
                episode_id: episode_id.clone(),
                observation: MatrixCodec::encode(
                    format!("{episode_id}:observation"),
                    field(case, &interaction.observation_field)?,
                    provenance.clone(),
                )?,
                demonstrated_action: MatrixCodec::encode(
                    format!("{episode_id}:action"),
                    field(case, &interaction.action_field)?,
                    provenance.clone(),
                )?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let query_cases = cases(root, &interaction.query_collection)?;
    let mut queries = Vec::new();
    let mut answers = Vec::new();
    for (index, case) in query_cases.iter().enumerate() {
        let episode_id = format!("query:{index}");
        queries.push(Query {
            episode_id: episode_id.clone(),
            observation: MatrixCodec::encode(
                format!("{episode_id}:observation"),
                field(case, &interaction.observation_field)?,
                provenance.clone(),
            )?,
        });
        let frame = MatrixCodec::encode(
            format!("{episode_id}:sealed-action"),
            field(case, &interaction.action_field)?,
            provenance.clone(),
        )?;
        answers.push(SealedAnswer {
            episode_id,
            commitment: digest_json(&(frame.payload_digest.as_str(), "sealed-answer-v1"))?,
            frame,
        });
    }
    let public = LearningView::create(protocol, demonstrations, queries, public_source)?;
    Ok(SealedEvaluation {
        public,
        answers,
        reveal_count: 0,
        source_digest: manifest.source.sha256,
    })
}

fn validate_interaction_spec(
    collections: &[String],
    matrix_fields: &[String],
    interaction: &MatrixInteractionSpec,
) -> Result<()> {
    for collection in [
        &interaction.demonstration_collection,
        &interaction.query_collection,
    ] {
        if !collections.contains(collection) {
            bail!("interaction collection is not declared by the transport codec");
        }
    }
    for field in [&interaction.observation_field, &interaction.action_field] {
        if !matrix_fields.contains(field) {
            bail!("interaction field is not declared by the transport codec");
        }
    }
    if interaction.demonstration_collection == interaction.query_collection
        || interaction.observation_field == interaction.action_field
    {
        bail!("demonstration/query and observation/action roles must be distinct");
    }
    Ok(())
}

fn matrix_world_protocol(
    source: &SourceDescriptor,
    interaction: &MatrixInteractionSpec,
) -> Result<WorldProtocol> {
    let protocol = WorldProtocol {
        schema: "soma.rtl.world-protocol.v1".to_string(),
        id: format!("protocol:{}", source.id),
        channels: vec![
            ChannelSpec {
                id: interaction.observation_field.clone(),
                direction: ChannelDirection::Observation,
                payload_relations: vec![
                    "extent".to_string(),
                    "coordinate".to_string(),
                    "successor".to_string(),
                    "incidence".to_string(),
                ],
            },
            ChannelSpec {
                id: interaction.action_field.clone(),
                direction: ChannelDirection::Action,
                payload_relations: vec!["extent".to_string(), "incidence".to_string()],
            },
            ChannelSpec {
                id: "evaluation".to_string(),
                direction: ChannelDirection::Feedback,
                payload_relations: vec!["exact_match".to_string()],
            },
        ],
        lifecycle: vec![
            "receive".to_string(),
            "perceive".to_string(),
            "decide".to_string(),
            "commit".to_string(),
            "feedback".to_string(),
        ],
        source_digest: digest_json(&(source.id.as_str(), source.role.as_str()))?,
        transport_codec: "finite-rank-coordinate-incidence.v1".to_string(),
    };
    protocol.validate()?;
    Ok(protocol)
}

fn cases<'a>(root: &'a Map<String, Value>, name: &str) -> Result<&'a Vec<Value>> {
    root.get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("declared interaction collection is absent"))
}

fn field<'a>(case: &'a Value, name: &str) -> Result<&'a Value> {
    case.as_object()
        .and_then(|object| object.get(name))
        .ok_or_else(|| anyhow::anyhow!("declared interaction field is absent"))
}

fn atom_from_json(value: &Value) -> Result<Atom> {
    match value {
        Value::Null => Ok(Atom::Null),
        Value::Bool(value) => Ok(Atom::Boolean(*value)),
        Value::Number(value) => value
            .as_i64()
            .map(Atom::Integer)
            .ok_or_else(|| anyhow::anyhow!("matrix numbers must be exact signed integers")),
        Value::String(value) => Ok(Atom::Text(value.clone())),
        Value::Array(_) | Value::Object(_) => {
            bail!("matrix cells must be scalar JSON values")
        }
    }
}

fn json_from_atom(value: &Atom) -> Value {
    match value {
        Atom::Null => Value::Null,
        Atom::Boolean(value) => Value::Bool(*value),
        Atom::Integer(value) => Value::from(*value),
        Atom::Text(value) => Value::String(value.clone()),
    }
}

fn atom_integer(value: &Atom) -> Result<i64> {
    match value {
        Atom::Integer(value) => Ok(*value),
        _ => bail!("matrix coordinate must be an integer"),
    }
}

fn matrix_shape(value: &Value) -> Result<Vec<usize>> {
    let rows = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("matrix must be an array"))?;
    let width = rows
        .first()
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    Ok(vec![rows.len(), width])
}
