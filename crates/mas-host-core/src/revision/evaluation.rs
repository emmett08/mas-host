//! Bind evaluator results to frozen authority and reuse exact terminal assessments.
use super::super::{error, fields, Inner, Result};
use mas_protocol_codec::assessment::{
    validate_assessment, Assessment, AssessmentStatus, Candidate, Evaluator, Evidence, Rubric,
};
use serde_json::{json, Value};

impl Inner {
    pub(super) fn retained_assessment(
        &self,
        id: &str,
        context: &Value,
        candidate: &Candidate,
        rubric: &Rubric,
        evaluator: &Evaluator,
        evidence: &[Evidence],
    ) -> Result<Option<Assessment>> {
        let Some(value) = self.retained_record("assessment", id, &context["readers"])? else {
            return Ok(None);
        };
        let assessment = Assessment::from_value(&value).map_err(error)?;
        validate_assessment(&assessment, candidate, rubric, evidence, evaluator).map_err(error)?;
        self.bind_context(&self.step_id("", id, "request"), context)?;
        Ok(Some(assessment))
    }
}

pub(super) fn bind_assessment(
    id: &str,
    result: Result<Value>,
    candidate: &Candidate,
    rubric: &Rubric,
    evaluator: &Evaluator,
    evidence: &[Evidence],
) -> Assessment {
    let valid = result.and_then(|v| {
        fields(&v, &["status", "findings"], &[])?;
        let assessment = Assessment::from_value(&json!({"schema_version":1,"id":id,"candidate":candidate.reference().to_value(),"rubric":rubric.reference().to_value(),"evaluator":evaluator.to_value(),"status":v["status"],"findings":v["findings"],"error":null})).map_err(error)?;
        validate_assessment(&assessment, candidate, rubric, evidence, evaluator).map_err(error)?;
        Ok(assessment)
    });
    valid.unwrap_or_else(|failure| Assessment {
        schema_version: 1,
        id: id.into(),
        candidate: candidate.reference(),
        evaluator: evaluator.clone(),
        rubric: rubric.reference(),
        status: AssessmentStatus::Error,
        findings: Vec::new(),
        error: Some(super::super::bounded_error(&failure)),
    })
}
