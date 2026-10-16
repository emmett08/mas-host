//! Validate frozen evaluator bindings, criteria and execution limits before dispatch.
use super::super::{error, fields, text, Inner, Result};
use super::context;
use mas_protocol_codec::assessment::{Evaluator, Rubric};
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) struct Plan {
    pub(super) draft: String,
    pub(super) revision: String,
    pub(super) max_revisions: u64,
    pub(super) draft_inputs: Vec<String>,
    pub(super) revision_inputs: Vec<String>,
    pub(super) delegates: Vec<String>,
    pub(super) feedback: Vec<String>,
    pub(super) rubric: Rubric,
    pub(super) evaluators: Vec<(String, Evaluator, Vec<String>, Rubric)>,
}
impl Plan {
    pub(super) fn read(value: &Value, inner: &Inner) -> Result<Self> {
        fields(
            value,
            &[
                "version",
                "draft_agent",
                "revision_agent",
                "max_revisions",
                "max_invocations",
                "draft_inputs",
                "revision_inputs",
                "allowed_agents",
                "feedback_evaluators",
                "rubric",
                "evaluators",
            ],
            &[],
        )?;
        if value["version"] != "mas-host-revision/1" {
            return Err("unsupported revision plan".into());
        }
        let bounded = |key: &str, max: u64| {
            value[key]
                .as_u64()
                .filter(|n| *n <= max)
                .ok_or_else(|| format!("{key} exceeds supported bound"))
        };
        let max_revisions = bounded("max_revisions", 16)?;
        if bounded("max_invocations", 1024)? == 0 {
            return Err("max_invocations must be positive".into());
        }
        let draft = text(value, "draft_agent")?.to_owned();
        let revision = text(value, "revision_agent")?.to_owned();
        let delegates = context::names(&value["allowed_agents"])?;
        for actor in delegates.iter().chain([&draft, &revision]) {
            if !inner.agents.contains_key(actor) {
                return Err("revision names an unknown agent".into());
            }
        }
        let rubric = Rubric::from_value(&value["rubric"]).map_err(error)?;
        let mut rubric_identities =
            std::collections::BTreeMap::from([(rubric.id.clone(), rubric.to_value())]);
        let mut evaluators = Vec::new();
        let mut identities = BTreeSet::new();
        for v in value["evaluators"]
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= 16)
            .ok_or("provide 1..16 evaluators")?
        {
            fields(v, &["actor", "identity", "inputs"], &["rubric"])?;
            let actor = text(v, "actor")?.to_owned();
            if !inner.agents.contains_key(&actor) {
                return Err("unknown evaluator actor".into());
            }
            let identity = Evaluator::from_value(&v["identity"]).map_err(error)?;
            if identity.kind == mas_protocol_codec::assessment::EvaluatorKind::Deterministic
                && inner.agents[&actor].configuration["kind"] == "llm"
            {
                return Err("LLM evaluator cannot claim deterministic measurement".into());
            }
            if !identities.insert(identity.id.clone()) {
                return Err("duplicate evaluator identity".into());
            }
            let rubric =
                Rubric::from_value(v.get("rubric").unwrap_or(&value["rubric"])).map_err(error)?;
            if rubric
                .criteria
                .iter()
                .any(|c| c.measurement != identity.kind)
            {
                return Err("rubric measurement must match authorised evaluator kind".into());
            }
            if let Some(previous) = rubric_identities.insert(rubric.id.clone(), rubric.to_value()) {
                if previous != rubric.to_value() {
                    return Err("rubric identity reused with changed criteria".into());
                }
            }
            evaluators.push((actor, identity, context::names(&v["inputs"])?, rubric));
        }
        let feedback = context::names(&value["feedback_evaluators"])?;
        if feedback.iter().any(|id| !identities.contains(id)) {
            return Err("feedback names an unknown evaluator".into());
        }
        Ok(Self {
            draft,
            revision,
            max_revisions,
            draft_inputs: context::names(&value["draft_inputs"])?,
            revision_inputs: context::names(&value["revision_inputs"])?,
            delegates,
            feedback,
            rubric,
            evaluators,
        })
    }
}
