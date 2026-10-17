//! Opt-in agreement on one caller-bound candidate and review context.
use crate::{plan::checked_json, Result};
use mas_protocol_codec::assessment::{canonical_digest, DigestRef};
use serde_json::{json, Value};

pub(crate) const VERSION: &str = "mas-candidate-quorum/1";

#[derive(Clone)]
pub(crate) struct Quorum {
    pub minimum: usize,
    pub candidate: Value,
    pub context: Value,
}

pub(crate) struct Binding {
    minimum: usize,
    candidate: Value,
    context: Value,
    digest: String,
}

impl Binding {
    pub(crate) fn new(minimum: usize, candidate: Value, context: Value) -> Result<Self> {
        DigestRef::from_value(&candidate).map_err(|e| format!("invalid quorum candidate: {e}"))?;
        if !context.as_object().is_some_and(|o| !o.is_empty()) {
            return Err("quorum context must be a non-empty frozen object".into());
        }
        let bound =
            json!({"version":VERSION,"minimum":minimum,"candidate":candidate,"context":context});
        checked_json(&bound)?;
        Ok(Self {
            minimum,
            candidate,
            context,
            digest: canonical_digest(&bound),
        })
    }

    pub(crate) fn finish(&self, targets: &[String], outputs: Vec<Value>) -> Result<Value> {
        let mut qualifying = vec![];
        let mut observations = vec![];
        for (branch, (actor, output)) in targets.iter().zip(&outputs).enumerate() {
            let status = if DigestRef::from_value(&output["candidate"]).is_err()
                || !output["context"].as_object().is_some_and(|o| !o.is_empty())
                || !output["accepted"].is_boolean()
            {
                "invalid"
            } else if output["candidate"] != self.candidate || output["context"] != self.context {
                "mismatched"
            } else if output["accepted"] == true {
                qualifying.push(json!({"branch":branch,"actor":actor}));
                "accepted"
            } else {
                "rejected"
            };
            observations.push(json!({"branch":branch,"actor":actor,"status":status}));
        }
        let accepted = qualifying.len() >= self.minimum;
        let selected = if accepted {
            qualifying
                .into_iter()
                .take(self.minimum)
                .collect::<Vec<_>>()
        } else {
            vec![]
        };
        let result = json!({"results":outputs,"join":{"version":VERSION,"accepted":accepted,
            "minimum":self.minimum,"candidate":self.candidate,"context":self.context,
            "binding_sha256":self.digest,"selected":selected,"observations":observations}});
        checked_json(&result)?;
        Ok(result)
    }
}
