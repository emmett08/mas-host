//! Validate closed privacy assessment records against the exact in-memory source.

use crate::{source::identifier, Error, Result, Source};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_findings: usize,
    pub max_ranges: usize,
    pub max_record_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 16_384,
            max_findings: 128,
            max_ranges: 128,
            max_record_bytes: 65_536,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

impl ByteRange {
    pub fn new(source: &Source, start: usize, end: usize) -> Result<Self> {
        if start >= end
            || end > source.content().len()
            || !source.content().is_char_boundary(start)
            || !source.content().is_char_boundary(end)
        {
            return Err(Error::InvalidRange);
        }
        Ok(Self { start, end })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Completion {
    Complete,
    Partial,
    Unsupported,
    Failed,
}

impl Completion {
    fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Unsupported => "unsupported",
            Self::Failed => "failed",
        }
    }
}

/// Prototype taxonomy. Model labels require an explicit versioned mapping later.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Category {
    Person,
    Location,
    Organisation,
    EmailAddress,
    PhoneNumber,
    AccountIdentifier,
    Credential,
    SensitiveContext,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Location => "location",
            Self::Organisation => "organisation",
            Self::EmailAddress => "email-address",
            Self::PhoneNumber => "phone-number",
            Self::AccountIdentifier => "account-identifier",
            Self::Credential => "credential",
            Self::SensitiveContext => "sensitive-context",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "person" => Ok(Self::Person),
            "location" => Ok(Self::Location),
            "organisation" => Ok(Self::Organisation),
            "email-address" => Ok(Self::EmailAddress),
            "phone-number" => Ok(Self::PhoneNumber),
            "account-identifier" => Ok(Self::AccountIdentifier),
            "credential" => Ok(Self::Credential),
            "sensitive-context" => Ok(Self::SensitiveContext),
            _ => Err(Error::InvalidRecord),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finding {
    pub range: ByteRange,
    pub categories: BTreeSet<Category>,
    pub recognisers: BTreeSet<String>,
}

impl Finding {
    pub fn new(range: ByteRange, category: Category, recogniser: &str) -> Result<Self> {
        identifier(recogniser)?;
        Ok(Self {
            range,
            categories: BTreeSet::from([category]),
            recognisers: BTreeSet::from([recogniser.into()]),
        })
    }
}

/// N0's only implemented detector identity is a synthetic pattern fixture.
/// A successful record cannot impersonate a qualified neural classifier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetectorIdentity;

impl DetectorIdentity {
    pub fn synthetic() -> Self {
        Self
    }

    fn value(&self) -> Value {
        json!({
            "bundle": "mas-synthetic-patterns/1", "taxonomy": "mas-privacy-taxonomy/1",
            "extraction": "utf8-text/1", "offsets": "utf8-bytes/1",
            "domain": "synthetic-fixtures", "language": "ascii-patterns",
            "overlap_rule": "union-overlap/1", "neural_qualified": false
        })
    }
}

#[derive(Clone)]
pub struct Assessment {
    source_id: String,
    source_binding: [u8; 32],
    source_bytes: usize,
    source_token: String,
    detector: DetectorIdentity,
    completion: Completion,
    findings: Vec<Finding>,
    checked_ranges: Vec<ByteRange>,
    omissions: BTreeSet<String>,
}

impl Assessment {
    pub fn new(
        source: &Source,
        completion: Completion,
        findings: Vec<Finding>,
        checked_ranges: Vec<ByteRange>,
        omissions: BTreeSet<String>,
        limits: Limits,
    ) -> Result<Self> {
        let result = Self {
            source_id: source.id().into(),
            source_binding: source.binding(),
            source_bytes: source.content().len(),
            source_token: source.token().into(),
            detector: DetectorIdentity::synthetic(),
            completion,
            findings,
            checked_ranges,
            omissions,
        };
        result.validate(source, limits)?;
        Ok(result)
    }

    pub fn completion(&self) -> Completion {
        self.completion
    }

    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    pub fn omissions(&self) -> &BTreeSet<String> {
        &self.omissions
    }

    pub fn checked_ranges(&self) -> &[ByteRange] {
        &self.checked_ranges
    }

    /// Metadata may itself be sensitive and requires the caller's governed access.
    /// It intentionally contains no source values, excerpts or source fingerprints.
    pub fn metadata(&self) -> Value {
        let findings: Vec<_> = self
            .findings
            .iter()
            .map(|finding| {
                json!({
                    "start": finding.range.start, "end": finding.range.end,
                    "categories": finding.categories.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
                    "recognisers": finding.recognisers.iter().collect::<Vec<_>>()
                })
            })
            .collect();
        json!({
            "version": "mas-detection-assessment/1", "source_id": self.source_id,
            "source_bytes": self.source_bytes, "detector": self.detector.value(),
            "source_token": self.source_token,
            "completion": self.completion.as_str(), "findings": findings,
            "checked_ranges": self.checked_ranges.iter().map(|r| json!({"start":r.start,"end":r.end})).collect::<Vec<_>>(),
            "omissions": self.omissions.iter().collect::<Vec<_>>()
        })
    }

    /// Parse only at the trusted adapter boundary, using the source retained for
    /// that invocation. Opaque IDs alone never authenticate a source or worker.
    pub fn from_value(value: &Value, source: &Source, limits: Limits) -> Result<Self> {
        if serde_json::to_vec(value)
            .map_err(|_| Error::InvalidRecord)?
            .len()
            > limits.max_record_bytes
        {
            return Err(Error::LimitExceeded);
        }
        let record = object(
            value,
            &[
                "version",
                "source_id",
                "source_bytes",
                "source_token",
                "detector",
                "completion",
                "findings",
                "checked_ranges",
                "omissions",
            ],
        )?;
        if string(record, "version")? != "mas-detection-assessment/1" {
            return Err(Error::UnsupportedVersion);
        }
        if string(record, "source_id")? != source.id()
            || number(record, "source_bytes")? != source.content().len()
            || string(record, "source_token")? != source.token()
        {
            return Err(Error::SourceMismatch);
        }
        if record["detector"] != DetectorIdentity::synthetic().value() {
            return Err(Error::DetectorMismatch);
        }
        let completion = match string(record, "completion")? {
            "complete" => Completion::Complete,
            "partial" => Completion::Partial,
            "unsupported" => Completion::Unsupported,
            "failed" => Completion::Failed,
            _ => return Err(Error::InvalidRecord),
        };
        let raw_findings = array(record, "findings", limits.max_findings)?;
        let mut findings = Vec::with_capacity(raw_findings.len());
        for value in raw_findings {
            let item = object(value, &["start", "end", "categories", "recognisers"])?;
            let categories = string_set(item, "categories", 8)?
                .iter()
                .map(|v| Category::parse(v))
                .collect::<Result<BTreeSet<_>>>()?;
            findings.push(Finding {
                range: range(item, source)?,
                categories,
                recognisers: string_set(item, "recognisers", 8)?,
            });
        }
        let mut checked_ranges = Vec::new();
        for value in array(record, "checked_ranges", limits.max_ranges)? {
            checked_ranges.push(range(object(value, &["start", "end"])?, source)?);
        }
        Self::new(
            source,
            completion,
            findings,
            checked_ranges,
            string_set(record, "omissions", 16)?,
            limits,
        )
    }

    pub fn validate(&self, source: &Source, limits: Limits) -> Result<()> {
        if self.source_id != source.id()
            || self.source_binding != source.binding()
            || self.source_bytes != source.content().len()
            || self.source_token != source.token()
        {
            return Err(Error::SourceMismatch);
        }
        if source.content().len() > limits.max_input_bytes
            || self.findings.len() > limits.max_findings
            || self.checked_ranges.len() > limits.max_ranges
            || self.omissions.len() > 16
        {
            return Err(Error::LimitExceeded);
        }
        for omission in &self.omissions {
            identifier(omission)?;
        }
        let mut end = 0;
        for range in &self.checked_ranges {
            ByteRange::new(source, range.start, range.end)?;
            if range.start < end {
                return Err(Error::InvalidRange);
            }
            if self.completion == Completion::Complete && range.start != end {
                return Err(Error::IncompleteCoverage);
            }
            end = range.end;
        }
        if self.completion == Completion::Complete
            && (end != source.content().len() || !self.omissions.is_empty())
        {
            return Err(Error::IncompleteCoverage);
        }
        let mut prior_finding_end = 0;
        for finding in &self.findings {
            ByteRange::new(source, finding.range.start, finding.range.end)?;
            if finding.range.start < prior_finding_end
                || finding.categories.is_empty()
                || finding.recognisers.is_empty()
                || finding.recognisers.len() > 8
                || !covered(finding.range, &self.checked_ranges)
            {
                return Err(Error::InvalidRecord);
            }
            for recogniser in &finding.recognisers {
                identifier(recogniser)?;
            }
            prior_finding_end = finding.range.end;
        }
        if self.metadata().to_string().len() > limits.max_record_bytes {
            return Err(Error::LimitExceeded);
        }
        Ok(())
    }
}

impl std::fmt::Debug for Assessment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Assessment")
            .field("source_id", &self.source_id)
            .field("completion", &self.completion)
            .field("finding_count", &self.findings.len())
            .finish_non_exhaustive()
    }
}

fn covered(range: ByteRange, checks: &[ByteRange]) -> bool {
    let mut cursor = range.start;
    for check in checks {
        if check.start > cursor {
            break;
        }
        if check.end > cursor {
            cursor = check.end;
        }
        if cursor >= range.end {
            return true;
        }
    }
    false
}

fn object<'a>(value: &'a Value, fields: &[&str]) -> Result<&'a Map<String, Value>> {
    let record = value.as_object().ok_or(Error::InvalidRecord)?;
    if record.len() != fields.len() || fields.iter().any(|key| !record.contains_key(*key)) {
        return Err(Error::InvalidRecord);
    }
    Ok(record)
}

fn string<'a>(record: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    record[key].as_str().ok_or(Error::InvalidRecord)
}

fn number(record: &Map<String, Value>, key: &str) -> Result<usize> {
    record[key]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or(Error::InvalidRecord)
}

fn array<'a>(record: &'a Map<String, Value>, key: &str, maximum: usize) -> Result<&'a [Value]> {
    let values = record[key].as_array().ok_or(Error::InvalidRecord)?;
    if values.len() > maximum {
        return Err(Error::LimitExceeded);
    }
    Ok(values)
}

fn string_set(record: &Map<String, Value>, key: &str, maximum: usize) -> Result<BTreeSet<String>> {
    let values = array(record, key, maximum)?;
    let mut result = BTreeSet::new();
    for value in values {
        let value = value.as_str().ok_or(Error::InvalidRecord)?;
        identifier(value)?;
        if !result.insert(value.to_string()) {
            return Err(Error::InvalidRecord);
        }
    }
    Ok(result)
}

fn range(record: &Map<String, Value>, source: &Source) -> Result<ByteRange> {
    ByteRange::new(source, number(record, "start")?, number(record, "end")?)
}
