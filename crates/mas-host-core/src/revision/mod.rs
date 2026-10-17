//! Durable, bounded assessment and revision above existing request/effect execution.
mod context;
mod evaluation;
mod plan;
mod projection;
use plan::Plan;
pub(crate) use projection::{audit, public_audit, step_public};
mod store;
#[cfg(test)]
mod tests;

use super::{checked_object, error, lock, Engine, Inner, Result};
use evaluation::bind_assessment;
use mas_protocol_codec::assessment::{
    canonical_digest, validate_candidate_revision, Assessment, AssessmentStatus, Candidate,
    Evidence, Verdict,
};
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};

impl Engine {
    /// Execute frozen draft/assess/revise stages under this run's original limits.
    /// Evaluators are registered workers; their identity is supplied by the host,
    /// never accepted from an evaluator's result. No remediation effect is allowed.
    pub fn revise(&self, plan: &Value, input: &Value) -> Result<Value> {
        if self.inner.running.swap(true, Ordering::SeqCst) {
            return Err("engine already running".into());
        }
        struct Running<'a>(&'a std::sync::atomic::AtomicBool);
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _running = Running(&self.inner.running);
        checked_object(input, "revision input")?;
        context::check_sources(input)?;
        let parsed = Plan::read(plan, &self.inner)?;
        // Reject invalid/forbidden host-managed references before any worker starts.
        context::assemble(
            &parsed.draft,
            &parsed.draft_inputs,
            input,
            None,
            &[],
            &parsed.delegates,
        )?;
        context::assemble(
            &parsed.revision,
            &parsed.revision_inputs,
            input,
            None,
            &[],
            &parsed.delegates,
        )?;
        for (actor, _, ids, _) in &parsed.evaluators {
            context::assemble(actor, ids, input, None, &[], &[])?;
        }
        self.inner.freeze_revision(plan, input)?;
        self.inner.check_live()?;
        self.inner.reconcile_completed_effects()?;
        if self.inner.has_unknown()? {
            return Err("uncertain effect requires evidence-backed resolution".into());
        }
        let retained = store::audit(&*lock(&self.inner.journal)?)?;
        if retained["selected"].is_object() {
            if let Some(candidate) = retained["candidates"]
                .as_array()
                .and_then(|a| a.iter().find(|c| c["id"] == retained["selected"]["id"]))
            {
                return self
                    .revision_result(Some(&Candidate::from_value(candidate).map_err(error)?));
            }
        }
        if retained["status"] == "exhausted" {
            return self.revision_result(None);
        }
        let context = context::assemble(
            &parsed.draft,
            &parsed.draft_inputs,
            input,
            None,
            &[],
            &parsed.delegates,
        )?;
        let output = match self.inner.invoke_revision("draft", &parsed.draft, &context) {
            Ok(output) => output,
            Err(failure) => {
                if self.inner.revision_exhausted("draft")? {
                    self.inner
                        .terminal_revision(&json!({"status":"exhausted","selected":null}))?;
                    return self.revision_result(None);
                }
                return Err(failure);
            }
        };
        let mut candidate=Candidate::from_value(&json!({"schema_version":1,"id":"candidate-0","output_sha256":canonical_digest(&output),"output":output,"parent":null,"feedback":[]})).map_err(error)?;
        let mut audience = context["readers"].clone();
        self.inner
            .record("candidate", &candidate.to_value(), &audience)?;
        self.inner
            .record("rubric", &parsed.rubric.to_value(), &json!(["*"]))?;
        for index in 0..=parsed.max_revisions {
            let mut assessed = Vec::new();
            for (number, (actor, evaluator, ids, rubric)) in parsed.evaluators.iter().enumerate() {
                let current = candidate.to_value();
                // Independent evaluators never receive earlier assessments.
                let mut context =
                    context::assemble(actor, ids, input, Some((&current, &audience)), &[], &[])?;
                let candidate_evidence=Evidence::from_value(&json!({"schema_version":1,"id":format!("{}-output",candidate.id),"content":candidate.output,"content_sha256":candidate.output_sha256})).map_err(error)?;
                context["input"]["evidence"]
                    .as_array_mut()
                    .unwrap()
                    .push(candidate_evidence.to_value());
                context["input"]["rubric"] = rubric.to_value();
                self.inner
                    .record("rubric", &rubric.to_value(), &json!(["*"]))?;
                let evidence = context["input"]["evidence"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(Evidence::from_value)
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(error)?;
                context["input"]["evidence_refs"] = json!(evidence
                    .iter()
                    .map(|e| e.reference().to_value())
                    .collect::<Vec<_>>());
                for item in &evidence {
                    let source_readers = if item.id == candidate_evidence.id {
                        &audience
                    } else {
                        &input["sources"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|s| s["id"] == item.id)
                            .ok_or("evidence has no frozen source")?["readers"]
                    };
                    self.inner
                        .record("evidence", &item.to_value(), source_readers)?;
                }
                let id = format!("assessment-{index}-{number}");
                let assessment = if let Some(assessment) = self
                    .inner
                    .retained_assessment(&id, &context, &candidate, rubric, evaluator, &evidence)?
                {
                    assessment
                } else {
                    let result = self.inner.invoke_revision(&id, actor, &context);
                    if self.inner.has_unknown()? {
                        let content = json!({"evaluation_attempt":id,"status":"unknown","candidate":candidate.reference().to_value(),"evaluator":evaluator.to_value()});
                        let attempt=Evidence::from_value(&json!({"schema_version":1,"id":format!("attempt-{id}"),"content_sha256":canonical_digest(&content),"content":content})).map_err(error)?;
                        self.inner
                            .record("evidence", &attempt.to_value(), &context["readers"])?;
                        return Err(
                            "evaluator effect outcome unknown; resolve with evidence before resuming"
                                .into(),
                        );
                    }
                    let assessment =
                        bind_assessment(&id, result, &candidate, rubric, evaluator, &evidence);
                    self.inner
                        .record("assessment", &assessment.to_value(), &context["readers"])?;
                    assessment
                };
                let exhausted = assessment.status == AssessmentStatus::Error
                    && self.inner.revision_exhausted(&id)?;
                if exhausted {
                    self.inner.terminal_revision(
                        &json!({"status":"exhausted","selected":candidate.reference().to_value()}),
                    )?;
                    return self.revision_result(Some(&candidate));
                }
                assessed.push((assessment, context["readers"].clone()));
                self.inner.check_live()?;
            }
            let passed = assessed.iter().all(|(a, _)| {
                a.status == AssessmentStatus::Completed
                    && a.findings.iter().all(|f| f.verdict == Verdict::Pass)
            });
            if passed || index == parsed.max_revisions {
                self.inner.terminal_revision(&json!({"status":if passed{"completed"}else{"exhausted"},"selected":candidate.reference().to_value()}))?;
                return self.revision_result(Some(&candidate));
            }
            let permitted = assessed
                .iter()
                .filter(|(a, _)| parsed.feedback.contains(&a.evaluator.id))
                .map(|(a, r)| (a.to_value(), r.clone()))
                .collect::<Vec<_>>();
            let feedback = permitted.iter().map(|(a, r)| (a, r)).collect::<Vec<_>>();
            let current = candidate.to_value();
            let context = context::assemble(
                &parsed.revision,
                &parsed.revision_inputs,
                input,
                Some((&current, &audience)),
                &feedback,
                &parsed.delegates,
            )?;
            let output = match self.inner.invoke_revision(
                &format!("revise-{}", index + 1),
                &parsed.revision,
                &context,
            ) {
                Ok(output) => output,
                Err(_)
                    if self
                        .inner
                        .revision_exhausted(&format!("revise-{}", index + 1))? =>
                {
                    self.inner.terminal_revision(
                        &json!({"status":"exhausted","selected":candidate.reference().to_value()}),
                    )?;
                    return self.revision_result(Some(&candidate));
                }
                Err(failure) => return Err(failure),
            };
            let supplied = context["input"]["feedback"]
                .as_array()
                .unwrap()
                .iter()
                .map(Assessment::from_value)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(error)?;
            let revised=Candidate::from_value(&json!({"schema_version":1,"id":format!("candidate-{}",index+1),"output_sha256":canonical_digest(&output),"output":output,"parent":candidate.reference().to_value(),"feedback":supplied.iter().map(|a|a.reference().to_value()).collect::<Vec<_>>()})).map_err(error)?;
            validate_candidate_revision(&revised, Some(&candidate), &supplied).map_err(error)?;
            audience = context["readers"].clone();
            candidate = revised;
            self.inner
                .record("candidate", &candidate.to_value(), &audience)?;
        }
        unreachable!("bounded revision terminates")
    }
    fn revision_result(&self, candidate: Option<&Candidate>) -> Result<Value> {
        let audit = store::audit(&*lock(&self.inner.journal)?)?;
        let visible = audit["records"].as_array().unwrap().iter().any(|r| {
            r["kind"] == "candidate"
                && candidate.is_some_and(|c| r["id"] == c.id)
                && r["readers"] == json!(["*"])
        });
        Ok(
            json!({"output":if visible {candidate.unwrap().output.clone()} else {Value::Null},"revision":public_audit(audit,true)}),
        )
    }
}

impl Inner {
    fn invoke_revision(
        self: &Arc<Self>,
        key: &str,
        target: &str,
        context: &Value,
    ) -> Result<Value> {
        self.check_live()?;
        let input = &context["input"];
        self.bind_context(&self.step_id("", key, "request"), context)?;
        self.request("", key, "client", target, input, 0)
    }
}
