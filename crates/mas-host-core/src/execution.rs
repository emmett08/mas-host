//! Supervised handler execution and bounded durable delegation.
use super::{
    bounded_error, checked_object, error, fields, lock, reply, step_key, text, verify_executable,
    Inner, Result, Step, MAX_ACTIVE, MAX_DEPTH,
};
use mas_host_worker::Worker;
use mas_protocol_codec::{admission::AdmissionDisposition, Message};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    os::fd::AsRawFd,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

// A bounded dispatch attempt may be resumed explicitly under the same durable
// identity and original deadline. No external effect is retried by this loop.
const ADMISSION_ATTEMPTS: usize = 8;

impl Inner {
    pub(crate) fn admit_request(&self, step: &Step, request: &Message) -> Result<()> {
        for attempt in 1..=ADMISSION_ATTEMPTS {
            self.check_live()?;
            let outcome = lock(&self.protocol)?.send_checked(request, &step.actor);
            let Err(failure) = outcome else {
                return Ok(());
            };
            let state = match failure.disposition {
                AdmissionDisposition::Rejected => "rejected",
                AdmissionDisposition::Retryable => "prepared",
                AdmissionDisposition::Unknown => "blocked",
            };
            self.set_state(
                &step.id,
                state,
                None,
                Some(&json!({"error":bounded_error(&failure.message),
                    "admission":failure.to_value(),"admission_attempts":attempt})),
            )?;
            if failure.disposition != AdmissionDisposition::Retryable
                || attempt == ADMISSION_ATTEMPTS
            {
                // An absent message in a later observation does not turn an
                // uncertain commit into permission to retry automatically.
                return Err(failure.message);
            }
            self.check_live()?;
            let delay_ms = (25u64 << (attempt - 1)).min(250);
            let remaining_ms = self.deadline_ms.saturating_sub(super::now_ms()?);
            // Neither database lock is held here. Other workers must be able
            // to finish and acknowledge messages while admission is waiting.
            std::thread::sleep(Duration::from_millis(delay_ms.min(remaining_ms)));
        }
        unreachable!("bounded admission loop always returns")
    }

    pub(crate) fn request(
        self: &Arc<Self>,
        parent: &str,
        key: &str,
        actor: &str,
        target: &str,
        input: &Value,
        depth: usize,
    ) -> Result<Value> {
        self.check_live()?;
        if depth > MAX_DEPTH {
            return Err("delegation depth limit exhausted".into());
        }
        let agent = self
            .agents
            .get(target)
            .ok_or("unknown application agent")?
            .clone();
        if let Some(schema) = &agent.input_schema {
            mas_host_schema::validate(schema, input)?;
        }
        let step = self.prepare(parent, key, "request", actor, target, input)?;
        if step.state == "completed" {
            self.settle_children(&step.id)?;
            let output = step
                .output
                .as_ref()
                .ok_or("completed checkpoint has no output")?;
            self.terminal(&step, Some(output), None)?;
            return Ok(output.clone());
        }
        if matches!(step.state.as_str(), "failed" | "rejected") {
            let failure = step
                .evidence
                .as_ref()
                .and_then(|v| v["error"].as_str())
                .unwrap_or("worker failed");
            if step.state == "failed" {
                self.terminal(&step, None, Some(failure))?;
            }
            return Err(failure.into());
        }
        if let Err(failure) = verify_executable(&agent) {
            // A retained prepared checkpoint may already have an admitted
            // Request after an interrupted send. Identity drift is a recovery
            // block, never proof that the child was not admitted.
            self.set_state(&step.id, "blocked", None, Some(&json!({"error":failure})))?;
            return Err(failure);
        }
        let request = self.message(
            format!("req-{}", step.id),
            actor,
            target,
            "Request",
            json!({"task_id":step.id,"operation":agent.operation,"input":input}),
        );
        self.admit_request(&step, &request)?;
        {
            let mut protocol = lock(&self.protocol)?;
            let deliveries = protocol.receive(&self.run_id, target, 100)?;
            let delivery = deliveries
                .as_array()
                .ok_or("invalid protocol delivery")?
                .iter()
                .find(|r| r["message"]["message_id"] == request.message_id)
                .ok_or("request not delivered")?;
            if delivery["actionable"] != true {
                return Err("protocol task is no longer actionable".into());
            }
        }
        let active = self.active.fetch_add(1, Ordering::SeqCst);
        if active >= MAX_ACTIVE {
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.known_failure(&step, "concurrent worker limit exhausted")?;
            return Err("concurrent worker limit exhausted".into());
        }
        struct Active<'a>(&'a AtomicUsize);
        impl Drop for Active<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let _active = Active(&self.active);
        self.set_state(&step.id, "running", None, None)?;
        let environment = match agent.environment() {
            Ok(environment) => environment,
            Err(failure) => {
                self.known_failure(&step, &failure)?;
                return Err(failure);
            }
        };
        let mut start = json!({"type":"start","version":"mas-native/1","actor":target,
            "input":input,"configuration":agent.configuration,"invocation_id":step.id,"deadline_ms":self.deadline_ms});
        if let Some(handler) = &agent.handler {
            start["handler"] = json!(handler);
        }
        let mut worker = match Worker::spawn_scoped(
            &agent.executable,
            &start,
            self.deadline_ms,
            self.owner.as_raw_fd(),
            &agent.arguments,
            &environment,
            Some(&agent.working_directory),
        ) {
            Ok(worker) => worker,
            Err(e) => {
                self.known_failure(&step, &e)?;
                return Err(e);
            }
        };
        let outcome = self.serve(&mut worker, &step, depth);
        let cleanup = worker.stop();
        if let Err(cleanup) = cleanup {
            self.mark_uncertain(&step.id)?;
            self.set_state(
                &step.id,
                "blocked",
                None,
                Some(&json!({"error":"worker cleanup not confirmed"})),
            )?;
            return Err(format!("worker cleanup not confirmed: {cleanup}"));
        }
        match outcome {
            Ok(output) => {
                if let Some(schema) = &agent.output_schema {
                    if let Err(failure) = mas_host_schema::validate(schema, &output) {
                        self.mark_uncertain(&step.id)?;
                        if self.has_unsettled_below(&step.id)? {
                            self.set_state(
                                &step.id,
                                "blocked",
                                None,
                                Some(&json!({"error":failure})),
                            )?;
                        } else {
                            self.known_failure(&step, &failure)?;
                        }
                        return Err(failure);
                    }
                }
                if self.has_unsettled_below(&step.id)? {
                    self.mark_uncertain(&step.id)?;
                    self.set_state(&step.id, "blocked", None, None)?;
                    return Err("unsettled effect prevents successful completion".into());
                }
                if let Err(failure) = self.settle_children(&step.id) {
                    self.set_state(&step.id, "blocked", None, Some(&json!({"error":failure})))?;
                    return Err(failure);
                }
                self.reconcile_completed_effects()?;
                // Validate terminal grammar/size before making success durable.
                if let Err(failure)=self.message(format!("end-{}",step.id),target,actor,"Result",
                    json!({"task_id":step.id,"reply_to":request.message_id,"status":"completed","output":output}))
                    .format().map_err(|e|e.message) {
                    self.known_failure(&step,&failure)?;
                    return Err(failure);
                }
                if let Err(failure) = self.commit_completed(&step.id, &output) {
                    self.known_failure(&step, &failure)?;
                    return Err(failure);
                }
                self.terminal(&step, Some(&output), None)?;
                Ok(output)
            }
            Err(failure) => {
                self.mark_uncertain(&step.id)?;
                if self.has_unsettled_below(&step.id)? {
                    self.set_state(&step.id, "blocked", None, Some(&json!({"error":failure})))?;
                } else {
                    self.known_failure(&step, &failure)?;
                }
                Err(failure)
            }
        }
    }
    fn mark_uncertain(&self, parent: &str) -> Result<()> {
        lock(&self.journal)?.execute("UPDATE steps SET state='unknown' WHERE parent=? AND kind='effect' AND state='executing'",[parent]).map_err(error)?;
        Ok(())
    }
    fn serve(self: &Arc<Self>, worker: &mut Worker, step: &Step, depth: usize) -> Result<Value> {
        loop {
            let frame = worker.read_interruptible(|| self.check_live())?;
            checked_object(&frame, "worker frame")?;
            let command = text(&frame, "type")?;
            // A fully observed effect completion remains useful evidence after
            // cancellation. Every command that advances work is checked again
            // after the blocking read, including the final result.
            if command != "effect_complete" {
                self.check_live()?;
            }
            match command {
                "complete" => {
                    fields(&frame, &["type", "output"], &[])?;
                    checked_object(&frame["output"], "output")?;
                    return Ok(frame["output"].clone());
                }
                "error" => {
                    fields(&frame, &["type", "message"], &[])?;
                    return Err(bounded_error(text(&frame, "message")?));
                }
                "request" => {
                    fields(&frame, &["type", "key", "target", "input"], &[])?;
                    let key = step_key(&frame)?;
                    let target = text(&frame, "target")?;
                    let outcome = self.request(
                        &step.id,
                        key,
                        &step.target,
                        target,
                        &frame["input"],
                        depth + 1,
                    );
                    worker.write(&reply(outcome))?;
                }
                "parallel" => {
                    fields(&frame, &["type", "requests"], &[])?;
                    let requests = frame["requests"]
                        .as_array()
                        .filter(|v| !v.is_empty() && v.len() <= 8)
                        .ok_or("parallel requires 1..8 requests")?;
                    let mut keys = BTreeSet::new();
                    for request in requests {
                        fields(request, &["key", "target", "input"], &[])?;
                        if !keys.insert(step_key(request)?) {
                            return Err("parallel request keys must be unique".into());
                        }
                        text(request, "target")?;
                        checked_object(&request["input"], "input")?;
                    }
                    let outcomes = std::thread::scope(|scope| {
                        let handles: Vec<_> = requests
                            .iter()
                            .map(|r| {
                                scope.spawn(move || {
                                    self.request(
                                        &step.id,
                                        r["key"].as_str().unwrap(),
                                        &step.target,
                                        r["target"].as_str().unwrap(),
                                        &r["input"],
                                        depth + 1,
                                    )
                                })
                            })
                            .collect();
                        handles
                            .into_iter()
                            .map(|h| {
                                h.join()
                                    .unwrap_or_else(|_| Err("child execution panicked".into()))
                            })
                            .collect::<Vec<_>>()
                    });
                    let result = outcomes.into_iter().collect::<Result<Vec<_>>>();
                    worker.write(&match result {
                        Ok(outputs) => json!({"type":"response","ok":true,"outputs":outputs}),
                        Err(e) => json!({"type":"response","ok":false,"error":bounded_error(&e)}),
                    })?;
                }
                "effect_begin" => {
                    fields(&frame, &["type", "key", "descriptor"], &[])?;
                    let response =
                        match self.begin_effect(step, step_key(&frame)?, &frame["descriptor"]) {
                            Ok(value) => value,
                            Err(e) => {
                                json!({"type":"response","ok":false,"error":bounded_error(&e)})
                            }
                        };
                    worker.write(&response)?;
                }
                "effect_complete" => {
                    fields(&frame, &["type", "key", "output", "evidence"], &[])?;
                    let result = self.complete_effect(
                        step,
                        step_key(&frame)?,
                        &frame["output"],
                        &frame["evidence"],
                    );
                    worker.write(&match result {
                        Ok(()) => json!({"type":"response","ok":true}),
                        Err(e) => json!({"type":"response","ok":false,"error":bounded_error(&e)}),
                    })?;
                }
                _ => return Err("unknown native worker command".into()),
            }
        }
    }
}
