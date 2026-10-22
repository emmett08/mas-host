//! Validate exact-source neural results and convert original codepoint ranges to bytes.

use crate::{
    record::{array, number, object, string, string_set},
    spans::{covered, validate_coverage},
    ByteRange, Category, Completion, DetectorPins, Error, Finding, Limits, Result, Source,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

const WORKER_OMISSIONS: [&str; 10] = [
    "input-token-limit",
    "unsupported-language",
    "unsupported-domain",
    "model-unavailable",
    "model-inference-failed",
    "incomplete-token-coverage",
    "decoder-incomplete",
    "source-reconstruction-failed",
    "invalid-model-output",
    "detector-limit",
];

const HOST_FAILURES: [&str; 7] = [
    "worker-unavailable",
    "worker-cleanup-failed",
    "worker-incomplete",
    "worker-timeout",
    "invalid-worker-response",
    "request-limit",
    "invalid-request",
];

/// Exact official labels. Unknown labels fail rather than inheriting a broader class.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum OpenAiLabel {
    AccountNumber,
    PrivateAddress,
    PrivateEmail,
    PrivatePerson,
    PrivatePhone,
    PrivateUrl,
    PrivateDate,
    Secret,
}

impl OpenAiLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AccountNumber => "account_number",
            Self::PrivateAddress => "private_address",
            Self::PrivateEmail => "private_email",
            Self::PrivatePerson => "private_person",
            Self::PrivatePhone => "private_phone",
            Self::PrivateUrl => "private_url",
            Self::PrivateDate => "private_date",
            Self::Secret => "secret",
        }
    }

    pub fn category(self) -> Category {
        match self {
            Self::AccountNumber => Category::AccountIdentifier,
            Self::PrivateAddress => Category::PostalAddress,
            Self::PrivateEmail => Category::EmailAddress,
            Self::PrivatePerson => Category::Person,
            Self::PrivatePhone => Category::PhoneNumber,
            Self::PrivateUrl => Category::Url,
            Self::PrivateDate => Category::Date,
            Self::Secret => Category::Credential,
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "account_number" => Ok(Self::AccountNumber),
            "private_address" => Ok(Self::PrivateAddress),
            "private_email" => Ok(Self::PrivateEmail),
            "private_person" => Ok(Self::PrivatePerson),
            "private_phone" => Ok(Self::PrivatePhone),
            "private_url" => Ok(Self::PrivateUrl),
            "private_date" => Ok(Self::PrivateDate),
            "secret" => Ok(Self::Secret),
            _ => Err(Error::InvalidRecord),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NeuralFinding {
    pub range: ByteRange,
    pub label: OpenAiLabel,
}

impl NeuralFinding {
    pub(crate) fn finding(&self) -> Result<Finding> {
        Finding::new(self.range, self.label.category(), "mas-privacy-opf/1")
    }
}

/// Completion describes inspection coverage. Release quality remains unqualified
/// for all results in this increment, including complete neural responses.
#[derive(Clone)]
pub struct NeuralAssessment {
    source_id: String,
    source_token: String,
    source_bytes: usize,
    binding: [u8; 32],
    detector: DetectorPins,
    completion: Completion,
    checked_ranges: Vec<ByteRange>,
    findings: Vec<NeuralFinding>,
    omissions: BTreeSet<String>,
}

impl NeuralAssessment {
    pub fn from_value(
        value: &Value,
        source: &Source,
        detector: &DetectorPins,
        limits: Limits,
    ) -> Result<Self> {
        if source.content().len() > limits.max_input_bytes {
            return Err(Error::InputTooLarge);
        }
        if value.to_string().len() > limits.max_record_bytes {
            return Err(Error::LimitExceeded);
        }
        let record = object(
            value,
            &[
                "version",
                "source_id",
                "source_token",
                "input_bytes",
                "detector",
                "offset_unit",
                "completion",
                "checked_ranges",
                "omissions",
                "findings",
            ],
        )?;
        if string(record, "version")? != "mas-privacy-worker-response/1" {
            return Err(Error::UnsupportedVersion);
        }
        if string(record, "source_id")? != source.id()
            || string(record, "source_token")? != source.token()
            || number(record, "input_bytes")? != source.content().len()
        {
            return Err(Error::SourceMismatch);
        }
        if DetectorPins::from_value(&record["detector"])? != *detector {
            return Err(Error::DetectorMismatch);
        }
        if string(record, "offset_unit")? != "unicode-codepoints" {
            return Err(Error::InvalidRange);
        }
        let completion = match string(record, "completion")? {
            "complete" => Completion::Complete,
            "partial" => Completion::Partial,
            "unsupported" => Completion::Unsupported,
            "failed" => Completion::Failed,
            _ => return Err(Error::InvalidRecord),
        };
        // The table includes the terminal byte offset. No normalisation is applied.
        let mut boundaries: Vec<_> = source
            .content()
            .char_indices()
            .map(|(offset, _)| offset)
            .collect();
        boundaries.push(source.content().len());
        let mut checked_ranges = Vec::new();
        for value in array(record, "checked_ranges", limits.max_ranges)? {
            checked_ranges.push(codepoint_range(
                object(value, &["start", "end"])?,
                source,
                &boundaries,
            )?);
        }
        let mut findings = Vec::new();
        for value in array(record, "findings", limits.max_findings)? {
            let finding = object(value, &["start", "end", "label"])?;
            findings.push(NeuralFinding {
                range: codepoint_range(finding, source, &boundaries)?,
                label: OpenAiLabel::parse(string(finding, "label")?)?,
            });
        }
        let omissions = string_set(record, "omissions", 16)?;
        if omissions
            .iter()
            .any(|code| !WORKER_OMISSIONS.contains(&code.as_str()))
            || (completion != Completion::Complete && omissions.is_empty())
        {
            return Err(Error::InvalidRecord);
        }
        let assessment = Self {
            source_id: source.id().into(),
            source_token: source.token().into(),
            source_bytes: source.content().len(),
            binding: source.binding(),
            detector: detector.clone(),
            completion,
            checked_ranges,
            findings,
            omissions,
        };
        assessment.validate(source, detector, limits)?;
        Ok(assessment)
    }

    /// Fail a worker run safely without discarding a separate deterministic scan.
    pub fn failed(
        source: &Source,
        detector: &DetectorPins,
        omission: &str,
        limits: Limits,
    ) -> Result<Self> {
        if !HOST_FAILURES.contains(&omission) {
            return Err(Error::InvalidRecord);
        }
        let assessment = Self {
            source_id: source.id().into(),
            source_token: source.token().into(),
            source_bytes: source.content().len(),
            binding: source.binding(),
            detector: detector.clone(),
            completion: Completion::Failed,
            checked_ranges: vec![],
            findings: vec![],
            omissions: BTreeSet::from([omission.into()]),
        };
        assessment.validate(source, detector, limits)?;
        Ok(assessment)
    }

    pub fn completion(&self) -> Completion {
        self.completion
    }
    pub fn findings(&self) -> &[NeuralFinding] {
        &self.findings
    }
    pub fn checked_ranges(&self) -> &[ByteRange] {
        &self.checked_ranges
    }
    pub fn omissions(&self) -> &BTreeSet<String> {
        &self.omissions
    }
    pub fn detector(&self) -> &DetectorPins {
        &self.detector
    }

    /// Governed metadata only, with byte offsets after validation. Its format is
    /// distinct from the worker response's original codepoint coordinates.
    pub fn metadata(&self) -> Value {
        json!({"version":"mas-neural-assessment/1","source_id":self.source_id,
            "source_token":self.source_token,"input_bytes":self.source_bytes,
            "detector":self.detector.value(),"offset_unit":"utf8-bytes",
            "extraction":"utf8-text/1","normalisation":"none/1","taxonomy":"opf-labels/1","label_mapping":"opf-to-mas/1",
            "completion":self.completion.as_str(),"release_qualified":false,"score_semantics":"not-exposed",
            "checked_ranges":self.checked_ranges.iter().map(|range|json!({"start":range.start,"end":range.end})).collect::<Vec<_>>(),
            "omissions":self.omissions.iter().collect::<Vec<_>>(),
            "findings":self.findings.iter().map(|finding|json!({"start":finding.range.start,"end":finding.range.end,"label":finding.label.as_str()})).collect::<Vec<_>>()})
    }

    pub fn validate(&self, source: &Source, detector: &DetectorPins, limits: Limits) -> Result<()> {
        if self.source_id != source.id()
            || self.source_token != source.token()
            || self.source_bytes != source.content().len()
            || self.binding != source.binding()
        {
            return Err(Error::SourceMismatch);
        }
        if self.detector != *detector {
            return Err(Error::DetectorMismatch);
        }
        if source.content().len() > limits.max_input_bytes
            || self.findings.len() > limits.max_findings
            || self.checked_ranges.len() > limits.max_ranges
        {
            return Err(Error::LimitExceeded);
        }
        validate_coverage(
            source,
            &self.checked_ranges,
            self.completion,
            !self.omissions.is_empty(),
        )?;
        for finding in &self.findings {
            ByteRange::new(source, finding.range.start, finding.range.end)?;
            if !covered(finding.range, &self.checked_ranges) {
                return Err(Error::IncompleteCoverage);
            }
        }
        if self.metadata().to_string().len() > limits.max_record_bytes {
            return Err(Error::LimitExceeded);
        }
        Ok(())
    }
}

impl std::fmt::Debug for NeuralAssessment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NeuralAssessment")
            .field("source_id", &self.source_id)
            .field("completion", &self.completion)
            .field("finding_count", &self.findings.len())
            .finish_non_exhaustive()
    }
}

fn codepoint_range(
    record: &Map<String, Value>,
    source: &Source,
    boundaries: &[usize],
) -> Result<ByteRange> {
    let start = *boundaries
        .get(number(record, "start")?)
        .ok_or(Error::InvalidRange)?;
    let end = *boundaries
        .get(number(record, "end")?)
        .ok_or(Error::InvalidRange)?;
    ByteRange::new(source, start, end)
}
