use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    FormalInvariant,
    ExternalMeasurement,
    BoundaryRoundTrip,
    ArchitectureAudit,
    OpenConjecture,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckRecord {
    pub id: String,
    pub class: EvidenceClass,
    pub passed: Option<bool>,
    pub statement: String,
    pub evidence_digest: Option<String>,
    pub limitation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EmpiricalMetric {
    pub id: String,
    pub source_digest: String,
    pub sample_count: usize,
    pub value: Option<f64>,
    pub unit: String,
    pub criterion_fixed_before_evaluation: bool,
    pub interpretation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ValidationLedger {
    pub checks: Vec<CheckRecord>,
    pub empirical_metrics: Vec<EmpiricalMetric>,
}

impl ValidationLedger {
    pub fn new() -> Self {
        Self {
            checks: Vec::new(),
            empirical_metrics: Vec::new(),
        }
    }

    pub fn formal(
        &mut self,
        id: impl Into<String>,
        passed: bool,
        statement: impl Into<String>,
        evidence_digest: Option<String>,
        limitation: impl Into<String>,
    ) {
        self.checks.push(CheckRecord {
            id: id.into(),
            class: EvidenceClass::FormalInvariant,
            passed: Some(passed),
            statement: statement.into(),
            evidence_digest,
            limitation: limitation.into(),
        });
    }

    pub fn boundary(
        &mut self,
        id: impl Into<String>,
        passed: bool,
        statement: impl Into<String>,
        evidence_digest: Option<String>,
        limitation: impl Into<String>,
    ) {
        self.checks.push(CheckRecord {
            id: id.into(),
            class: EvidenceClass::BoundaryRoundTrip,
            passed: Some(passed),
            statement: statement.into(),
            evidence_digest,
            limitation: limitation.into(),
        });
    }

    pub fn architecture(
        &mut self,
        id: impl Into<String>,
        passed: bool,
        statement: impl Into<String>,
        limitation: impl Into<String>,
    ) {
        self.checks.push(CheckRecord {
            id: id.into(),
            class: EvidenceClass::ArchitectureAudit,
            passed: Some(passed),
            statement: statement.into(),
            evidence_digest: None,
            limitation: limitation.into(),
        });
    }

    pub fn conjecture(
        &mut self,
        id: impl Into<String>,
        statement: impl Into<String>,
        limitation: impl Into<String>,
    ) {
        self.checks.push(CheckRecord {
            id: id.into(),
            class: EvidenceClass::OpenConjecture,
            passed: None,
            statement: statement.into(),
            evidence_digest: None,
            limitation: limitation.into(),
        });
    }

    pub fn required_checks_pass(&self) -> bool {
        self.checks.iter().all(|check| match check.class {
            EvidenceClass::FormalInvariant
            | EvidenceClass::BoundaryRoundTrip
            | EvidenceClass::ArchitectureAudit => check.passed == Some(true),
            EvidenceClass::ExternalMeasurement | EvidenceClass::OpenConjecture => true,
        })
    }
}

impl Default for ValidationLedger {
    fn default() -> Self {
        Self::new()
    }
}
