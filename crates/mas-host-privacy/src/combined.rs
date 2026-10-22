//! Combine deterministic and neural findings without granting release qualification.

use crate::{
    handling::decide_findings, merge_findings, scan_synthetic, Completion, Decision, Error,
    Finding, HandlingContext, HandlingProfile, Limits, NeuralAssessment, Result, Source,
};
use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub struct CombinedAssessment {
    neural: NeuralAssessment,
    findings: Vec<Finding>,
}

impl CombinedAssessment {
    pub fn new(source: &Source, neural: NeuralAssessment, limits: Limits) -> Result<Self> {
        neural.validate(source, neural.detector(), limits)?;
        let deterministic = scan_synthetic(source, limits)?;
        let mut findings = deterministic.findings().to_vec();
        findings.extend(
            neural
                .findings()
                .iter()
                .map(|finding| finding.finding())
                .collect::<Result<Vec<_>>>()?,
        );
        // Each detector has already enforced its own finding bound. Their
        // overlapping contributions can total twice the final output bound.
        let merge_limits = Limits {
            max_findings: limits.max_findings.saturating_mul(2),
            ..limits
        };
        let result = Self {
            neural,
            findings: merge_findings(source, findings, merge_limits)?,
        };
        result.validate(source, limits)?;
        Ok(result)
    }

    pub fn completion(&self) -> Completion {
        self.neural.completion()
    }
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }
    pub fn neural(&self) -> &NeuralAssessment {
        &self.neural
    }

    /// Contains governed category/span metadata only, without source values.
    pub fn metadata(&self) -> Value {
        json!({"version":"mas-combined-privacy-assessment/1","taxonomy":"mas-privacy-taxonomy/2",
            "completion":self.completion().as_str(),"release_qualified":false,
            "deterministic":{"bundle":"mas-synthetic-patterns/1","coverage":"complete-pattern-pass"},
            "neural":self.neural.metadata(),
            "findings":self.findings.iter().map(|finding|json!({"start":finding.range.start,"end":finding.range.end,
                "categories":finding.categories.iter().map(|category|category.as_str()).collect::<Vec<_>>(),
                "recognisers":finding.recognisers.iter().collect::<Vec<_>>()})).collect::<Vec<_>>()})
    }

    /// A complete neural pass is not a quality evaluation. Every neural result
    /// is handled under the protected profile, even if the caller requests a fixture.
    pub fn decide(
        &self,
        source: &Source,
        mut context: HandlingContext,
        limits: Limits,
    ) -> Result<Decision> {
        self.validate(source, limits)?;
        context.profile = HandlingProfile::Protected;
        Ok(decide_findings(&self.findings, self.completion(), context))
    }

    fn validate(&self, source: &Source, limits: Limits) -> Result<()> {
        self.neural
            .validate(source, self.neural.detector(), limits)?;
        if self.findings.len() > limits.max_findings
            || self.metadata().to_string().len() > limits.max_record_bytes
        {
            return Err(Error::LimitExceeded);
        }
        Ok(())
    }
}
