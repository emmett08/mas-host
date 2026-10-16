//! Durable effect admission, completion and cached-result reuse.
use super::{
    checked_object, digest, effect_evidence, finish_effect, lock, read_step, Inner, Result, Step,
};
use serde_json::{json, Value};

impl Inner {
    pub(crate) fn begin_effect(
        &self,
        invocation: &Step,
        key: &str,
        descriptor: &Value,
    ) -> Result<Value> {
        self.check_live()?;
        self.check_effect_policy(&invocation.id, descriptor)?;
        let step = self.prepare(
            &invocation.id,
            key,
            "effect",
            &invocation.target,
            "",
            descriptor,
        )?;
        if matches!(step.state.as_str(), "completed" | "resolved") {
            finish_effect(&mut *lock(&self.protocol)?, &self.run_id, &step)?;
            return Ok(json!({"type":"effect","state":"cached","output":step.output}));
        }
        if matches!(step.state.as_str(), "executing" | "unknown") {
            return Err("effect outcome uncertain; resolve before resuming".into());
        }
        if step.state != "prepared" {
            return Err("effect is not admissible".into());
        }
        let mut protocol = lock(&self.protocol)?;
        let snapshot = protocol.inspect(&self.run_id)?;
        let reservation = snapshot["calls"]
            .as_array()
            .ok_or("invalid protocol calls")?
            .iter()
            .find(|v| v["id"] == step.id);
        if let Some(call) = reservation {
            if call["actor"] != step.actor || call["state"] != "reserved" {
                return Err("prepared effect reservation differs".into());
            }
        } else {
            if let Err(failure) = protocol.reserve_call(&self.run_id, &step.actor, &step.id) {
                // The public call API returns strings. Bind exhaustion to the
                // authoritative counters, never to worker-supplied wording.
                if snapshot["usage"]["calls"]
                    .as_i64()
                    .zip(snapshot["config"]["max_calls"].as_i64())
                    .is_some_and(|(used, limit)| used >= limit)
                {
                    self.set_state(
                        &step.id,
                        "rejected",
                        None,
                        Some(
                            &json!({"error":failure,"host_budget":{"code":"call_budget_exceeded"}}),
                        ),
                    )?;
                }
                return Err(failure);
            }
        }
        drop(protocol);
        self.set_state(&step.id, "executing", None, None)?;
        Ok(json!({"type":"effect","state":"execute","invocation_id":step.id}))
    }
    pub(crate) fn complete_effect(
        &self,
        invocation: &Step,
        key: &str,
        output: &Value,
        evidence: &Value,
    ) -> Result<()> {
        checked_object(output, "effect output")?;
        checked_object(evidence, "effect evidence")?;
        if evidence.as_object().unwrap().is_empty() {
            return Err("effect completion requires non-empty evidence".into());
        }
        let id = format!(
            "eff-{}",
            digest(
                json!([self.nonce, self.run_id, invocation.id, key])
                    .to_string()
                    .as_bytes()
            )
        );
        let mut step = read_step(&*lock(&self.journal)?, &id)?.ok_or("unknown effect")?;
        if step.kind != "effect" || step.actor != invocation.target {
            return Err("effect belongs to another invocation".into());
        }
        if step.state == "completed" {
            if step.output.as_ref() != Some(output) || step.evidence.as_ref() != Some(evidence) {
                return Err("completed effect cannot be overwritten".into());
            }
            return finish_effect(&mut *lock(&self.protocol)?, &self.run_id, &step);
        }
        if step.state != "executing" {
            return Err("effect has no live execute permission".into());
        }
        step.state = "completed".into();
        step.output = Some(output.clone());
        step.evidence = Some(evidence.clone());
        effect_evidence(&step)?;
        self.set_state(&step.id, "completed", Some(output), Some(evidence))?;
        finish_effect(&mut *lock(&self.protocol)?, &self.run_id, &step)
    }
}
