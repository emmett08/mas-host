//! Bind host checkpoints to upstream messages and call evidence.
use super::{bounded_error, checked_object, error, lock, read_step, Inner, Result, Step};
use mas_protocol_codec::{store::Store, Message};
use serde_json::{json, Value};

pub(crate) fn effect_evidence(step: &Step) -> Result<Value> {
    checked_object(
        step.output
            .as_ref()
            .ok_or("completed effect has no output")?,
        "effect output",
    )?;
    let supporting = step
        .evidence
        .as_ref()
        .ok_or("completed effect has no evidence")?;
    checked_object(supporting, "effect evidence")?;
    if supporting.as_object().unwrap().is_empty() {
        return Err("completed effect evidence is empty".into());
    }
    let evidence = json!({"outcome":"completed", "output":step.output,
        "evidence":step.evidence, "resolution":step.state == "resolved"});
    checked_object(&evidence, "combined effect result")?;
    Ok(evidence)
}
pub(crate) fn finish_effect(protocol: &mut Store, run_id: &str, step: &Step) -> Result<()> {
    let evidence = effect_evidence(step)?;
    let snapshot = protocol.inspect(run_id)?;
    let call = snapshot["calls"]
        .as_array()
        .ok_or("invalid protocol calls")?
        .iter()
        .find(|v| v["id"] == step.id)
        .ok_or("effect has no protocol reservation")?;
    if call["actor"] != step.actor {
        return Err("effect reservation actor differs".into());
    }
    if call["state"] == "recorded" {
        if call["evidence"] != evidence {
            return Err("protocol effect evidence differs from journal".into());
        }
    } else if call["state"] == "reserved" {
        protocol.finish_call(run_id, &step.actor, &step.id, &evidence)?;
    } else {
        return Err("invalid protocol call state".into());
    }
    Ok(())
}

impl Inner {
    pub(crate) fn message(
        &self,
        id: String,
        actor: &str,
        target: &str,
        kind: &str,
        payload: Value,
    ) -> Message {
        Message {
            message_id: id,
            run_id: self.run_id.clone(),
            sender: actor.into(),
            recipient: target.into(),
            version: "1".into(),
            kind: kind.into(),
            payload: payload.as_object().unwrap().clone(),
        }
    }
    pub(crate) fn terminal(
        &self,
        step: &Step,
        output: Option<&Value>,
        failure: Option<&str>,
    ) -> Result<()> {
        let request_id = format!("req-{}", step.id);
        let payload = if let Some(output) = output {
            json!({"task_id":step.id,"reply_to":request_id,"status":"completed","output":output})
        } else {
            json!({"task_id":step.id,"reply_to":request_id,"reason":failure.unwrap_or("worker failed")})
        };
        let message = self.message(
            format!("end-{}", step.id),
            &step.target,
            &step.actor,
            if output.is_some() {
                "Result"
            } else {
                "Failure"
            },
            payload,
        );
        let mut protocol = lock(&self.protocol)?;
        protocol.send(&message, &step.target)?;
        protocol.ack(&self.run_id, &request_id, &step.target)?;
        let received = protocol.receive(&self.run_id, &step.actor, 100)?;
        if received.as_array().is_some_and(|rows| {
            rows.iter()
                .any(|r| r["message"]["message_id"] == message.message_id)
        }) {
            protocol.ack(&self.run_id, &message.message_id, &step.actor)?;
        }
        Ok(())
    }
    pub(crate) fn known_failure(&self, step: &Step, failure: &str) -> Result<()> {
        let failure = bounded_error(failure);
        self.set_state(&step.id, "failed", None, Some(&json!({"error":failure})))?;
        self.terminal(step, None, Some(&failure))
    }
    pub(crate) fn settle_children(&self, id: &str) -> Result<()> {
        let descendants = {
            let db = lock(&self.journal)?;
            let mut statement = db
                .prepare(
                    "WITH RECURSIVE descendants(id,depth) AS (
                SELECT id,1 FROM steps WHERE parent=? UNION ALL
                SELECT s.id,d.depth+1 FROM steps s JOIN descendants d ON s.parent=d.id)
                SELECT s.id FROM steps s JOIN descendants d ON s.id=d.id
                WHERE s.kind='request' ORDER BY d.depth DESC",
                )
                .map_err(error)?;
            let ids = statement
                .query_map([id], |r| r.get::<_, String>(0))
                .map_err(error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(error)?;
            ids.iter()
                .map(|id| read_step(&db, id)?.ok_or("missing child checkpoint".into()))
                .collect::<Result<Vec<_>>>()?
        };
        for child in descendants {
            match child.state.as_str() {
                "completed"=>self.terminal(&child,Some(child.output.as_ref().ok_or("completed child has no output")?),None)?,
                "failed"=>self.terminal(&child,None,Some(child.evidence.as_ref()
                    .and_then(|v|v["error"].as_str()).unwrap_or("worker failed")))?,
                "rejected"=>{},
                _=>return Err("unfinished child request prevents successful completion; resume the original run".into()),
            }
        }
        Ok(())
    }
}
