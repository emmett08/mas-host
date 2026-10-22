//! Authenticated A2A ingress into the existing PostgreSQL native task authority.
//! This profile owns neither a second queue nor a worker dispatch path.
use super::{digest, fields, text, Caller, Result, RpcError, RpcResult, Service};
use crate::shared_connection::Authority;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub(super) struct SharedQueue {
    authority_file: PathBuf,
    authority_binding: Value,
    epoch: String,
    service: String,
    workflows: BTreeMap<String, (String, String)>,
}
impl SharedQueue {
    pub(super) fn parse(
        value: &Value,
        bundle: &Value,
        descriptor: &Value,
        exports: &BTreeMap<String, Value>,
        governed: bool,
    ) -> Result<Self> {
        if governed {
            return Err(
                "shared native A2A does not support managed privacy or governed-local execution"
                    .into(),
            );
        }
        fields(
            value,
            &["version", "authority_file", "service_epoch", "workflows"],
            &[],
        )?;
        if value["version"] != "mas-host-a2a-shared-native/1" {
            return Err("unsupported shared A2A profile".into());
        }
        let authority_file = PathBuf::from(text(value, "authority_file")?);
        if !authority_file.is_absolute() {
            return Err("shared A2A authority path must be absolute".into());
        }
        let epoch = text(value, "service_epoch")?;
        if epoch.len() > 256 || epoch.chars().any(char::is_control) {
            return Err("invalid shared A2A service epoch".into());
        }
        let authority = Authority::open(&BTreeMap::from([(
            "--authority".into(),
            authority_file.to_string_lossy().into_owned(),
        )]))?;
        let mappings = value["workflows"]
            .as_object()
            .filter(|m| !m.is_empty() && m.len() == exports.len())
            .ok_or("shared A2A requires one workflow per export")?;
        let mut workflows = BTreeMap::new();
        let mut identities = BTreeSet::new();
        for (actor, mapping) in mappings {
            if !exports.contains_key(actor) {
                return Err("shared A2A workflow names an unknown export".into());
            }
            fields(mapping, &["name", "identity"], &[])?;
            let name = text(mapping, "name")?;
            let identity = text(mapping, "identity")?;
            if identity.len() != 64
                || !identity
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || !identities.insert((name.clone(), identity.clone()))
            {
                return Err("shared A2A workflow binding must be unique and exact".into());
            }
            mas_host_core::shared_native::preflight(bundle, actor)?;
            let definition = authority.repository.host_query(
                "definition.inspect",
                &json!({"kind":"workflow","name":name,"identity":identity}),
            )?;
            if definition["retired"] != false
                || definition["implementation_provenance"] != "verified_source_package"
                || definition["descriptor"]["bundle_digest"] != descriptor["bundle_digest"]
            {
                return Err("shared A2A workflow differs from registered native bundle".into());
            }
            let manifest = authority.repository.host_query(
                "definition.manifest",
                &json!({"digest":definition["implementation_digest"]}),
            )?;
            if manifest["format"] != "mas-native-implementation/1"
                || manifest["bundle_digest"] != descriptor["bundle_digest"]
            {
                return Err("shared A2A native source package unavailable".into());
            }
            workflows.insert(actor.clone(), (name, identity));
        }
        // Handover and credential rotation cannot create a new request namespace.
        // The operator's service epoch identifies the logical service lifetime.
        let service = digest(
            json!([
                "mas-host-a2a-shared-native/1",
                authority.configuration["deployment"],
                authority.configuration["schema"],
                epoch
            ])
            .to_string()
            .as_bytes(),
        );
        Ok(Self {
            authority_file,
            authority_binding: authority.configuration,
            epoch,
            service,
            workflows,
        })
    }
    fn open(&self) -> RpcResult<Authority> {
        let current = crate::config::read_json(&self.authority_file).map_err(|_| unavailable())?;
        if current != self.authority_binding {
            return Err(unavailable());
        }
        Authority::open(&BTreeMap::from([(
            "--authority".into(),
            self.authority_file.to_string_lossy().into_owned(),
        )]))
        .map_err(|_| unavailable())
    }
    fn command(&self, principal: &Caller, original: &str, operation: &str) -> String {
        format!(
            "ha-{}",
            digest(
                json!([self.service, principal.owner, original, operation])
                    .to_string()
                    .as_bytes()
            )
        )
    }
    fn check(&self, principal: &Caller, task: &Value) -> RpcResult<String> {
        if task["principal"] != principal.owner
            || task["request_namespace"] != "mas-unified/1"
            || task["request_epoch"] != self.epoch
            || task["a2a_service"] != self.service
        {
            return Err(not_found());
        }
        let actor = task["a2a_agent"]
            .as_str()
            .filter(|a| principal.agents.contains(*a))
            .ok_or_else(not_found)?;
        let Some((workflow, identity)) = self.workflows.get(actor) else {
            return Err(not_found());
        };
        if task["workflow"] != *workflow || task["workflow_identity"] != *identity {
            return Err(not_found());
        }
        Ok(actor.into())
    }
    fn inspect(&self, principal: &Caller, id: &str) -> RpcResult<(Value, String)> {
        principal.current().map_err(|_| not_found())?;
        let authority = self.open()?;
        let task = authority
            .repository
            .host_query(
                "task.status",
                &json!({"principal":principal.owner,"task":id}),
            )
            .map_err(|_| not_found())?;
        let actor = self.check(principal, &task)?;
        Ok((task, actor))
    }
}
fn unavailable() -> RpcError {
    RpcError(
        -32603,
        "shared_authority_unavailable_repeat_original_request",
    )
}
fn not_found() -> RpcError {
    RpcError(-32001, "task_not_found")
}
fn wire(state: &str) -> &'static str {
    match state {
        "queued" => "TASK_STATE_SUBMITTED",
        "running" => "TASK_STATE_WORKING",
        "completed" => "TASK_STATE_COMPLETED",
        "failed" => "TASK_STATE_FAILED",
        "cancelled" => "TASK_STATE_CANCELED",
        _ => "TASK_STATE_UNSPECIFIED",
    }
}
impl Service {
    pub(super) fn shared_submit(
        &self,
        principal: &Caller,
        original: &str,
        agent: &str,
        input: &Value,
        message: &Value,
    ) -> RpcResult<Value> {
        let shared = self
            .configuration
            .shared_queue
            .as_ref()
            .ok_or_else(unavailable)?;
        if message.get("contextId").is_some() {
            return Err(RpcError(
                -32602,
                "shared_native_custom_context_not_supported",
            ));
        }
        principal.current().map_err(|_| unavailable())?;
        let (workflow, identity) = shared.workflows.get(agent).ok_or_else(not_found)?;
        let task = format!(
            "a2a-{}",
            &digest(
                json!([shared.service, principal.owner, original])
                    .to_string()
                    .as_bytes()
            )[..56]
        );
        // Only business content participates. JSON whitespace/object key order,
        // RPC ids and returnImmediately transport choices cannot fork the task.
        let content=digest(json!({"agent":agent,"input":input,"bundle_digest":self.descriptor["bundle_digest"],"workflow":workflow,"identity":identity}).to_string().as_bytes());
        let mut authority = shared.open()?;
        authority.repository.host_command(&shared.command(principal,original,"submit"),"task.submit",&json!({"principal":principal.owner,"namespace":"mas-unified/1","epoch":shared.epoch,"key":original,"digest":content,"task":task,"workflow":workflow,"identity":identity,"expected_identity":identity,"input":input,"metadata":{"a2a_service":shared.service,"a2a_agent":agent,"bundle_digest":self.descriptor["bundle_digest"]}})).map_err(|_|RpcError(-32603,"admission_outcome_unknown_repeat_original_message"))?;
        Ok(json!({"task":self.shared_task(principal,&task)?}))
    }
    pub(super) fn shared_task(&self, principal: &Caller, id: &str) -> RpcResult<Value> {
        let shared = self
            .configuration
            .shared_queue
            .as_ref()
            .ok_or_else(unavailable)?;
        let (status, actor) = shared.inspect(principal, id)?;
        let mut task = json!({"id":id,"contextId":format!("context-{id}"),"status":{"state":wire(status["state"].as_str().unwrap_or("unknown"))},"metadata":{"mas":{"state":status["state"],"cancelRequested":status["cancel_requested"],"bundleDigest":self.descriptor["bundle_digest"],"agent":actor,"backend":"shared-native"}}});
        if status["state"] == "completed" && status["output_retained"] == true {
            let authority = shared.open()?;
            let retained = authority
                .repository
                .host_query(
                    "task.inspect",
                    &json!({"principal":principal.owner,"task":id}),
                )
                .map_err(|_| not_found())?;
            if retained["state"] != "completed"
                || retained["result"].is_null()
                || retained["archived"] == true
                || retained["result_available"] == false
            {
                return Err(RpcError(-32603, "retained_output_unavailable"));
            }
            let contract = self.descriptor["agents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|value| value["name"] == actor)
                .ok_or_else(not_found)?;
            mas_host_schema::validate(&contract["output_schema"], &retained["result"])
                .map_err(|_| RpcError(-32603, "retained_output_invalid"))?;
            principal
                .delivery(&actor, &retained["result"])
                .map_err(|_| not_found())?;
            task["artifacts"] = json!([{"artifactId":format!("{id}-result"),"name":"result","parts":[{"data":retained["result"],"mediaType":"application/json"}]}]);
        }
        principal.current().map_err(|_| not_found())?;
        Ok(task)
    }
    pub(super) fn shared_status(&self, principal: &Caller, params: &Value) -> RpcResult<Value> {
        fields(params, &["id"], &[])?;
        let id = text(params, "id")?;
        let shared = self
            .configuration
            .shared_queue
            .as_ref()
            .ok_or_else(unavailable)?;
        let (task, actor) = shared.inspect(principal, &id)?;
        let view = json!({"version":"mas-host-task-status/1","id":id,"agent":actor,"bundleDigest":self.descriptor["bundle_digest"],"serviceState":task["state"],"cancelRequested":task["cancel_requested"],"hostSettlement":if task["state"]=="completed" {"completed"} else {"not-established"},"externalExecution":"not-assessed","payloadAvailability":if task["output_retained"]==true {"retained-requires-authorised-read"} else {"not-retained-in-service"},"evidenceCoverage":"not-assessed","applicationAssessment":"not-assessed","sourceEvidenceFreshness":"not-assessed","materialisation":{"revision":task["revision"],"updatedAtMs":task["updated_at"],"consistency":"committed-shared-service-snapshot","incomplete":false},"executionPermission":false});
        if view.to_string().len() > super::status::MAX_BYTES {
            return Err(unavailable());
        }
        principal.current().map_err(|_| not_found())?;
        Ok(view)
    }
    pub(super) fn shared_cancel(&self, principal: &Caller, params: &Value) -> RpcResult<Value> {
        fields(params, &["id"], &[])?;
        let id = text(params, "id")?;
        let shared = self
            .configuration
            .shared_queue
            .as_ref()
            .ok_or_else(unavailable)?;
        let (task, _) = shared.inspect(principal, &id)?;
        if matches!(
            task["state"].as_str(),
            Some("completed" | "failed" | "cancelled")
        ) {
            return Err(RpcError(-32002, "task_not_cancelable"));
        }
        let mut authority = shared.open()?;
        authority
            .repository
            .host_command(
                &shared.command(principal, &id, "cancel"),
                "task.cancel",
                &json!({"principal":principal.owner,"task":id}),
            )
            .map_err(|_| RpcError(-32002, "cancellation_outcome_unknown_poll_original_task"))?;
        Err(RpcError(
            -32002,
            "cancellation_intent_recorded_poll_original_task",
        ))
    }
}
