use crate::active::ObservedTransition;
use crate::memory::{digest_bytes, digest_json};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceDescriptor {
    pub id: String,
    pub path: String,
    pub sha256: String,
    pub provenance_url: String,
    pub citation: String,
    pub role: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstrumentProtocol {
    Delimited {
        delimiter: String,
        sequence_fields: Vec<String>,
        experience_fields: Vec<String>,
        intervention_fields: Vec<String>,
        report_fields: Vec<String>,
        filters: Vec<RowFilter>,
    },
    MatrixJson {
        collections: Vec<String>,
        matrix_fields: Vec<String>,
        #[serde(default)]
        interaction: Option<MatrixInteractionSpec>,
    },
    TraceJson {
        events_pointer: String,
        sequence_field: String,
        ordinal_field: String,
        state_field: String,
        action_field: String,
        next_state_field: String,
        outcome_fields: Vec<String>,
        terminal_field: String,
        cost_field: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatrixInteractionSpec {
    pub demonstration_collection: String,
    pub query_collection: String,
    pub observation_field: String,
    pub action_field: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operator", rename_all = "snake_case")]
pub enum RowFilter {
    Equals {
        field: String,
        value: String,
    },
    NotIn {
        field: String,
        values: Vec<String>,
    },
    NumericRangeInclusive {
        field: String,
        minimum: f64,
        maximum: f64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InstrumentManifest {
    pub schema: String,
    pub source: SourceDescriptor,
    pub protocol: InstrumentProtocol,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeasuredEvent {
    pub sequence_id: String,
    pub ordinal: usize,
    pub experience: String,
    pub intervention: Option<String>,
    pub report_digest: String,
    pub reports: BTreeMap<String, String>,
    pub source_row: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObservedSequence {
    pub id: String,
    pub events: Vec<MeasuredEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IngestionAudit {
    pub rows_read: usize,
    pub rows_accepted: usize,
    pub rows_rejected_by_declared_filter: usize,
    pub sequence_count: usize,
    pub experience_count: usize,
    pub intervention_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeasurementBatch {
    pub source: SourceDescriptor,
    pub sequences: Vec<ObservedSequence>,
    pub audit: IngestionAudit,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MatrixObservation {
    pub source: SourceDescriptor,
    pub original: Value,
    pub events: Vec<MeasuredEvent>,
    pub cells: Vec<MatrixCell>,
    pub collections: Vec<String>,
    pub matrix_fields: Vec<String>,
    pub interaction: Option<MatrixInteractionSpec>,
    pub document_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MatrixCell {
    pub collection: String,
    pub case_index: usize,
    pub field: String,
    pub row: usize,
    pub column: usize,
    pub value: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceObservation {
    pub source: SourceDescriptor,
    pub events: Vec<ObservedTransition>,
    pub document_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "measurement", rename_all = "snake_case")]
pub enum InstrumentOutput {
    Sequences(MeasurementBatch),
    Matrix(MatrixObservation),
    Traces(TraceObservation),
}

pub fn load_manifest(path: &Path) -> Result<InstrumentManifest> {
    let bytes = fs::read(path)
        .with_context(|| format!("cannot read instrument manifest {}", path.display()))?;
    let manifest: InstrumentManifest = serde_json::from_slice(&bytes)?;
    if manifest.schema != "soma.rtl.instrument.v1" {
        bail!("unsupported instrument manifest schema");
    }
    Ok(manifest)
}

pub fn observe(manifest_path: &Path) -> Result<InstrumentOutput> {
    let manifest = load_manifest(manifest_path)?;
    let source_path = resolve_source(manifest_path, &manifest.source.path);
    let bytes = fs::read(&source_path)
        .with_context(|| format!("cannot read external source {}", source_path.display()))?;
    let actual = digest_bytes(&bytes);
    if actual != manifest.source.sha256 {
        bail!(
            "external source checksum mismatch for {}: expected {}, found {}",
            source_path.display(),
            manifest.source.sha256,
            actual
        );
    }
    match &manifest.protocol {
        InstrumentProtocol::Delimited {
            delimiter,
            sequence_fields,
            experience_fields,
            intervention_fields,
            report_fields,
            filters,
        } => Ok(InstrumentOutput::Sequences(observe_delimited(
            manifest.source.clone(),
            &bytes,
            delimiter,
            sequence_fields,
            experience_fields,
            intervention_fields,
            report_fields,
            filters,
        )?)),
        InstrumentProtocol::MatrixJson {
            collections,
            matrix_fields,
            interaction,
        } => Ok(InstrumentOutput::Matrix(observe_matrix(
            manifest.source.clone(),
            &bytes,
            collections,
            matrix_fields,
            interaction.clone(),
        )?)),
        InstrumentProtocol::TraceJson {
            events_pointer,
            sequence_field,
            ordinal_field,
            state_field,
            action_field,
            next_state_field,
            outcome_fields,
            terminal_field,
            cost_field,
        } => Ok(InstrumentOutput::Traces(TraceObservation {
            source: manifest.source.clone(),
            events: observe_trace_json(
                &bytes,
                &manifest.source.sha256,
                events_pointer,
                sequence_field,
                ordinal_field,
                state_field,
                action_field,
                next_state_field,
                outcome_fields,
                terminal_field,
                cost_field,
            )?,
            document_digest: actual,
        })),
    }
}

#[allow(clippy::too_many_arguments)]
fn observe_trace_json(
    bytes: &[u8],
    source_digest: &str,
    events_pointer: &str,
    sequence_field: &str,
    ordinal_field: &str,
    state_field: &str,
    action_field: &str,
    next_state_field: &str,
    outcome_fields: &[String],
    terminal_field: &str,
    cost_field: &str,
) -> Result<Vec<ObservedTransition>> {
    let document: Value = serde_json::from_slice(bytes)?;
    let events = document
        .pointer(events_pointer)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("trace events pointer does not select an array"))?;
    events
        .iter()
        .enumerate()
        .map(|(index, event)| {
            let object = event
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("trace event {index} is not an object"))?;
            let text = |field: &str| -> Result<String> {
                object
                    .get(field)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| {
                        anyhow::anyhow!("trace event {index} lacks string field {field}")
                    })
            };
            let mut outcomes = BTreeMap::new();
            for field in outcome_fields {
                let value = object.get(field).and_then(Value::as_i64).ok_or_else(|| {
                    anyhow::anyhow!("trace event {index} lacks integer outcome {field}")
                })?;
                outcomes.insert(field.clone(), value);
            }
            Ok(ObservedTransition {
                sequence_id: text(sequence_field)?,
                ordinal: object
                    .get(ordinal_field)
                    .and_then(Value::as_u64)
                    .ok_or_else(|| anyhow::anyhow!("trace event {index} lacks ordinal"))?
                    as usize,
                state: text(state_field)?,
                action: text(action_field)?,
                next_state: text(next_state_field)?,
                outcomes,
                terminal: object
                    .get(terminal_field)
                    .and_then(Value::as_bool)
                    .ok_or_else(|| anyhow::anyhow!("trace event {index} lacks terminal flag"))?,
                action_cost: object
                    .get(cost_field)
                    .and_then(Value::as_i64)
                    .ok_or_else(|| anyhow::anyhow!("trace event {index} lacks action cost"))?,
                evidence_digest: digest_json(&(source_digest, index, event))?,
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn observe_delimited(
    source: SourceDescriptor,
    bytes: &[u8],
    delimiter: &str,
    sequence_fields: &[String],
    experience_fields: &[String],
    intervention_fields: &[String],
    report_fields: &[String],
    filters: &[RowFilter],
) -> Result<MeasurementBatch> {
    let delimiter = delimiter.as_bytes();
    if delimiter.len() != 1 {
        bail!("delimited instruments require a one-byte delimiter");
    }
    if sequence_fields.is_empty() || experience_fields.is_empty() {
        bail!("delimited instruments require sequence and experience fields");
    }
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter[0])
        .from_reader(bytes);
    let headers = reader.headers()?.clone();
    let header_index = headers
        .iter()
        .enumerate()
        .map(|(index, name)| (name.to_string(), index))
        .collect::<BTreeMap<_, _>>();
    for field in sequence_fields
        .iter()
        .chain(experience_fields)
        .chain(intervention_fields)
        .chain(report_fields)
        .chain(filters.iter().map(RowFilter::field))
    {
        if !header_index.contains_key(field) {
            bail!("instrument field {field} is absent from source headers");
        }
    }
    let mut rows_read = 0usize;
    let mut rows_rejected = 0usize;
    let mut sequences: BTreeMap<String, Vec<MeasuredEvent>> = BTreeMap::new();
    for record in reader.records() {
        let record = record?;
        rows_read += 1;
        if !filters
            .iter()
            .all(|filter| filter.accepts(&record, &header_index))
        {
            rows_rejected += 1;
            continue;
        }
        let sequence_id = canonical_fields(&record, sequence_fields, &header_index)?;
        let experience = canonical_fields(&record, experience_fields, &header_index)?;
        let intervention = if intervention_fields.is_empty() {
            None
        } else {
            Some(canonical_fields(
                &record,
                intervention_fields,
                &header_index,
            )?)
        };
        let reports = field_map(&record, report_fields, &header_index)?;
        let events = sequences.entry(sequence_id.clone()).or_default();
        events.push(MeasuredEvent {
            sequence_id,
            ordinal: events.len(),
            experience,
            intervention,
            report_digest: digest_json(&reports)?,
            reports,
            source_row: rows_read + 1,
        });
    }
    let sequences = sequences
        .into_iter()
        .map(|(id, events)| ObservedSequence { id, events })
        .collect::<Vec<_>>();
    let experiences = sequences
        .iter()
        .flat_map(|sequence| sequence.events.iter().map(|event| &event.experience))
        .collect::<BTreeSet<_>>()
        .len();
    let interventions = sequences
        .iter()
        .flat_map(|sequence| {
            sequence
                .events
                .iter()
                .filter_map(|event| event.intervention.as_ref())
        })
        .collect::<BTreeSet<_>>()
        .len();
    let rows_accepted = sequences.iter().map(|sequence| sequence.events.len()).sum();
    Ok(MeasurementBatch {
        source,
        audit: IngestionAudit {
            rows_read,
            rows_accepted,
            rows_rejected_by_declared_filter: rows_rejected,
            sequence_count: sequences.len(),
            experience_count: experiences,
            intervention_count: interventions,
        },
        sequences,
    })
}

fn observe_matrix(
    source: SourceDescriptor,
    bytes: &[u8],
    collections: &[String],
    matrix_fields: &[String],
    interaction: Option<MatrixInteractionSpec>,
) -> Result<MatrixObservation> {
    if collections.is_empty() || matrix_fields.is_empty() {
        bail!("matrix protocol requires collection and matrix field names");
    }
    let original: Value = serde_json::from_slice(bytes)?;
    let root = original
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("matrix protocol root must be an object"))?;
    let mut events = Vec::new();
    let mut cells_flat = Vec::new();
    for collection in collections {
        let cases = root
            .get(collection)
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("matrix collection {collection} is absent"))?;
        for (case_index, case) in cases.iter().enumerate() {
            let case = case
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("matrix case must be an object"))?;
            for field in matrix_fields {
                let Some(matrix_value) = case.get(field) else {
                    continue;
                };
                let matrix = matrix_value
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("matrix field {field} must be an array"))?;
                let sequence_id = format!("{collection}:{case_index}:{field}");
                let mut ordinal = 0;
                for (row, row_value) in matrix.iter().enumerate() {
                    let cells = row_value
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("matrix row must be an array"))?;
                    for (column, cell) in cells.iter().enumerate() {
                        let context = BTreeMap::from([
                            ("collection", Value::String(collection.clone())),
                            ("case", Value::from(case_index)),
                            ("field", Value::String(field.clone())),
                            ("row", Value::from(row)),
                            ("column", Value::from(column)),
                        ]);
                        events.push(MeasuredEvent {
                            sequence_id: sequence_id.clone(),
                            ordinal,
                            experience: serde_json::to_string(cell)?,
                            intervention: None,
                            report_digest: digest_json(&context)?,
                            reports: context
                                .into_iter()
                                .map(|(key, value)| (key.to_string(), value.to_string()))
                                .collect(),
                            source_row: 0,
                        });
                        cells_flat.push(MatrixCell {
                            collection: collection.clone(),
                            case_index,
                            field: field.clone(),
                            row,
                            column,
                            value: cell.clone(),
                        });
                        ordinal += 1;
                    }
                }
            }
        }
    }
    Ok(MatrixObservation {
        source,
        document_digest: digest_json(&original)?,
        original,
        events,
        cells: cells_flat,
        collections: collections.to_vec(),
        matrix_fields: matrix_fields.to_vec(),
        interaction,
    })
}

impl RowFilter {
    fn field(&self) -> &String {
        match self {
            Self::Equals { field, .. }
            | Self::NotIn { field, .. }
            | Self::NumericRangeInclusive { field, .. } => field,
        }
    }

    fn accepts(&self, record: &csv::StringRecord, headers: &BTreeMap<String, usize>) -> bool {
        let value = |field: &str| {
            headers
                .get(field)
                .and_then(|index| record.get(*index))
                .unwrap_or_default()
        };
        match self {
            Self::Equals {
                field,
                value: expected,
            } => value(field) == expected,
            Self::NotIn { field, values } => !values.iter().any(|item| item == value(field)),
            Self::NumericRangeInclusive {
                field,
                minimum,
                maximum,
            } => value(field)
                .parse::<f64>()
                .map(|number| number >= *minimum && number <= *maximum)
                .unwrap_or(false),
        }
    }
}

fn canonical_fields(
    record: &csv::StringRecord,
    fields: &[String],
    headers: &BTreeMap<String, usize>,
) -> Result<String> {
    Ok(serde_json::to_string(&field_map(record, fields, headers)?)?)
}

fn field_map(
    record: &csv::StringRecord,
    fields: &[String],
    headers: &BTreeMap<String, usize>,
) -> Result<BTreeMap<String, String>> {
    fields
        .iter()
        .map(|field| {
            let index = headers
                .get(field)
                .ok_or_else(|| anyhow::anyhow!("field {field} is absent"))?;
            Ok((
                field.clone(),
                record.get(*index).unwrap_or_default().to_string(),
            ))
        })
        .collect()
}

fn resolve_source(manifest_path: &Path, source: &str) -> PathBuf {
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(source)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoundaryRoundTripCertificate {
    pub source_digest: String,
    pub decoded_document_digest: String,
    pub reencoded_document_digest: String,
    pub lossless_at_json_value_level: bool,
    pub cognitive_operators_in_adapter: usize,
}

pub fn certify_matrix_round_trip(
    observation: &MatrixObservation,
) -> Result<BoundaryRoundTripCertificate> {
    let reconstructed = reconstruct_matrix_document(observation)?;
    let encoded = serde_json::to_vec(&reconstructed)?;
    let decoded_again: Value = serde_json::from_slice(&encoded)?;
    let reencoded_document_digest = digest_json(&decoded_again)?;
    Ok(BoundaryRoundTripCertificate {
        source_digest: observation.source.sha256.clone(),
        decoded_document_digest: observation.document_digest.clone(),
        reencoded_document_digest: reencoded_document_digest.clone(),
        lossless_at_json_value_level: observation.original == decoded_again
            && observation.document_digest == reencoded_document_digest,
        cognitive_operators_in_adapter: 0,
    })
}

fn reconstruct_matrix_document(observation: &MatrixObservation) -> Result<Value> {
    let mut reconstructed = observation.original.clone();
    let root = reconstructed
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("matrix document root must be an object"))?;
    let mut grouped: BTreeMap<(String, usize, String), Vec<&MatrixCell>> = BTreeMap::new();
    for cell in &observation.cells {
        grouped
            .entry((cell.collection.clone(), cell.case_index, cell.field.clone()))
            .or_default()
            .push(cell);
    }
    for collection in &observation.collections {
        let cases = root
            .get_mut(collection)
            .and_then(Value::as_array_mut)
            .ok_or_else(|| anyhow::anyhow!("matrix collection {collection} disappeared"))?;
        for (case_index, case) in cases.iter_mut().enumerate() {
            let case = case
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("matrix case must remain an object"))?;
            for field in &observation.matrix_fields {
                let Some(cells) = grouped.get(&(collection.clone(), case_index, field.clone()))
                else {
                    continue;
                };
                let row_count = cells.iter().map(|cell| cell.row).max().unwrap_or(0) + 1;
                let mut rows = vec![Vec::<Value>::new(); row_count];
                let mut occupied = vec![Vec::<bool>::new(); row_count];
                for row in 0..row_count {
                    let column_count = cells
                        .iter()
                        .filter(|cell| cell.row == row)
                        .map(|cell| cell.column)
                        .max()
                        .map(|maximum| maximum + 1)
                        .ok_or_else(|| anyhow::anyhow!("matrix row {row} has no cells"))?;
                    rows[row] = vec![Value::Null; column_count];
                    occupied[row] = vec![false; column_count];
                }
                for cell in cells {
                    if occupied[cell.row][cell.column] {
                        bail!("duplicate matrix cell in universal event stream");
                    }
                    rows[cell.row][cell.column] = cell.value.clone();
                    occupied[cell.row][cell.column] = true;
                }
                if occupied.iter().flatten().any(|present| !present) {
                    bail!("matrix event stream has a coordinate gap");
                }
                case.insert(
                    field.clone(),
                    Value::Array(rows.into_iter().map(Value::Array).collect()),
                );
            }
        }
    }
    Ok(reconstructed)
}
