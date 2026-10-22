//! One PostgreSQL-authorised source-function invocation through the maintained guardian.
//!
//! This opt-in ordinary-data slice has no child requests, application effects,
//! managed privacy, queued service or automatic takeover. Its durable authority
//! and checkpoints live in the existing native shared repository.
use crate::{checked_object, digest, fields, text, validated_agents, verify_executable, Result};
use mas_host_worker::Worker;
use mas_protocol_codec::{postgres_repository::PostgresRepository, shared_native::NativeAdmission};
use serde_json::{json, Value};
use std::{fs::File, os::fd::AsRawFd};
mod provider;

/// Trusted controller identities. They are never accepted from a worker frame.
pub struct NativeRun<'a> {
    pub run_id: &'a str,
    pub principal: &'a str,
    pub owner: &'a str,
    pub actor: &'a str,
    pub reservation_microusd: i64,
    /// An existing service task can impose a stricter original database-clock deadline.
    pub max_deadline_ms: Option<u64>,
}

fn command(run: &str, owner: &str, operation: &str) -> String {
    format!(
        "hn-{}",
        digest(
            json!(["mas-host-shared-native/1", run, owner, operation])
                .to_string()
                .as_bytes()
        )
    )
}

/// Check the static shared native profile before a service task is claimed.
pub fn preflight(bundle: &Value, actor: &str) -> Result<()> {
    profile(bundle, actor).map(|_| ())
}

fn profile(bundle: &Value, actor: &str) -> Result<(crate::Agent, Value)> {
    let agents = validated_agents(bundle)?;
    let agent = agents.get(actor).ok_or("unknown native actor")?;
    if agents.len() != 1
        || agent.handler.is_none()
        || !agent
            .configuration
            .as_object()
            .is_some_and(|v| v.is_empty())
    {
        return Err(
            "shared native slice requires one source-function actor with empty configuration"
                .into(),
        );
    }
    if !agent.environment_names.is_empty() {
        return Err("shared native slice requires a credential-free source function".into());
    }
    if agent.input_schema.is_none() || agent.output_schema.is_none() {
        return Err("shared native object schemas required".into());
    }
    let configuration =
        mas_protocol_codec::shared_native::normalise_configuration(&bundle["topology"])?;
    let edges = configuration["edges"]
        .as_array()
        .ok_or("shared native routes required")?;
    if edges.len() != 2
        || !edges.contains(&json!(["client", actor]))
        || !edges.contains(&json!([actor, "client"]))
    {
        return Err("shared native slice requires exactly its ingress and reply routes".into());
    }
    if configuration["wall_seconds"]
        .as_u64()
        .is_none_or(|seconds| seconds > 86_400)
    {
        return Err("shared native wall allowance exceeds guardian profile".into());
    }
    Ok((agent.clone(), configuration))
}

/// Execute only a newly returned, consuming native dispatch permit. A retained
/// admission, failed database exchange or uncertain completion never launches.
pub fn execute(
    repository: &mut PostgresRepository,
    bundle: &Value,
    input: &Value,
    identities: NativeRun<'_>,
    local_lifetime: &File,
) -> Result<Value> {
    checked_object(input, "shared native input")?;
    let (agent, configuration) = profile(bundle, identities.actor)?;
    mas_host_schema::validate(agent.input_schema.as_ref().unwrap(), input)?;
    let output_schema = agent.output_schema.as_ref().unwrap();
    if identities.reservation_microusd < 0 {
        return Err("invalid shared reservation".into());
    }
    let host = digest(
        &std::fs::read(std::env::current_exe().map_err(crate::error)?).map_err(crate::error)?,
    );
    let binding = digest(
        json!([
            bundle,
            input,
            host,
            identities.actor,
            identities.reservation_microusd,
            "shared-native/1"
        ])
        .to_string()
        .as_bytes(),
    );
    let run = identities.run_id;
    let owner = identities.owner;
    let actor = identities.actor;
    // Allocation scopes are provisioned separately. The host never raises
    // quotas, creates database schemas or resets outstanding identities.
    repository.host_command(&command(run,"","create"),"create_run",&json!({
        "run":run,"principal":identities.principal,"ancestors":[],"configuration":configuration,
        "metadata":{"run_id":run,"workflow_version":format!("shared-native/1:{binding}"),
            "topology":configuration,"max_depth":1,"secret_reference":"none","principal":identities.principal}
    }))?;
    repository.host_command(
        &command(run, owner, "claim"),
        "claim_run",
        &json!({"run":run,"owner":owner}),
    )?;
    repository.host_command(
        &command(run, owner, "step"),
        "step",
        &json!({
            "run":run,"owner":owner,"key":"root","kind":"invoke","input":input,"initial":{}
        }),
    )?;
    let call_id = format!("native-{}", digest(run.as_bytes()));
    let prepared = repository.host_command(
        &command(run, owner, "prepare"),
        "prepare_invocation",
        &json!({
            "run":run,"owner":owner,"step_key":"root","actor":actor,"call_id":call_id,
            "reservation_microusd":identities.reservation_microusd
        }),
    )?;
    if prepared["receipt"]["call_id"] != call_id
        || prepared["receipt"]["actor"] != actor
        || prepared["receipt"]["step_key"] != "root"
        || prepared["receipt"]["reservation_microusd"] != identities.reservation_microusd
    {
        return Err("retained native identity differs".into());
    }
    let permit = match repository.admit_native_invocation(
        &command(run, owner, "admit"),
        run,
        owner,
        &call_id,
    ) {
        NativeAdmission::FreshDispatch(permit) => permit,
        NativeAdmission::Retained { .. } => return retained(repository, run, &call_id),
        NativeAdmission::Rejected { code } => {
            let not_before_ms = match &code {
                mas_protocol_codec::shared_native::NativeRejection::ProviderDeferred {
                    not_before_ms,
                } => Some(*not_before_ms),
                _ => None,
            };
            return Ok(
                json!({"version":"mas-host-shared-native/1","run_id":run,"call_id":call_id,"admission":if not_before_ms.is_some(){"deferred"}else{"rejected"},"reason":code.as_str(),"not_before_ms":not_before_ms,"dispatched":false}),
            );
        }
        NativeAdmission::Uncertain { command_id } => {
            return Ok(
                json!({"version":"mas-host-shared-native/1","run_id":run,"call_id":call_id,"admission":"unknown","command_id":command_id,"dispatched":false}),
            )
        }
    };
    if permit.run() != run
        || permit.call_id() != call_id
        || permit.command_id() != command(run, owner, "admit")
    {
        return Err("native permit binding differs".into());
    }
    let database_deadline = u64::try_from(permit.deadline_ms())
        .map_err(|_| "invalid authoritative native deadline")?
        .min(identities.max_deadline_ms.unwrap_or(u64::MAX));
    let provider_policy = permit.provider_policy().cloned();
    let admitted_call = permit.into_call_id();
    if admitted_call != call_id {
        return Err("native permit binding differs".into());
    }
    // The consuming permit carries the admission transaction's original run
    // and attached service-task deadline. Caller options can only narrow it.
    // Translate a remaining database-clock allowance into the guardian's local
    // wall-clock frame. Starting before the round trip conservatively includes
    // connection latency and avoids assuming synchronised database/host clocks.
    let local_started = crate::now_ms()?;
    let authority = repository.authority_status()?;
    let database_now = authority["database_time_ms"]
        .as_u64()
        .ok_or("database clock unavailable")?;
    let deadline_ms = translate_deadline(database_deadline, database_now, local_started)?;
    verify_executable(&agent)?;
    let mut start = json!({"type":"start","version":"mas-native/1","actor":actor,"input":input,
        "configuration":{},"handler":agent.handler,"invocation_id":call_id,"deadline_ms":deadline_ms});
    // Keep the original binding in the start frame only through invocation_id.
    start.as_object_mut().unwrap().retain(|_, v| !v.is_null());
    let outcome = (|| -> Result<Value> {
        repository.authority_status()?;
        let mut worker = Worker::spawn_scoped(
            &agent.executable,
            &start,
            deadline_ms,
            local_lifetime.as_raw_fd(),
            &agent.arguments,
            &agent.environment()?,
            Some(&agent.working_directory),
        )?;
        let result = (|| -> Result<Value> {
            let frame = worker.read_interruptible(|| repository.authority_status().map(|_| ()))?;
            fields(&frame, &["type", "output"], &[])?;
            if text(&frame, "type")? != "complete" {
                return Err("shared native slice rejects child/effect requests".into());
            }
            checked_object(&frame["output"], "shared native result")?;
            mas_host_schema::validate(output_schema, &frame["output"])?;
            if let Some(policy) = &provider_policy {
                provider::observe(repository, run, owner, &call_id, policy, &frame["output"])?;
            }
            Ok(frame["output"].clone())
        })();
        worker.stop()?;
        result
    })();
    let (state, output, evidence) = match outcome {
        Ok(output) => (
            "completed",
            output,
            json!({"native":"observed-complete","cleanup":"confirmed","binding":binding}),
        ),
        Err(_) => (
            "unknown",
            Value::Null,
            json!({"native":"outcome-unestablished","binding":binding}),
        ),
    };
    let observed=repository.host_command(&command(run,owner,"observe"),"observe_invocation",&json!({
        "run":run,"owner":owner,"call_id":call_id,"state":state,"output":output,"evidence":evidence
    }));
    if observed.is_err() {
        return Ok(
            json!({"version":"mas-host-shared-native/1","run_id":run,"call_id":call_id,
            "admission":"fresh","dispatched":true,"hostSettlement":"unknown","resultAvailability":"withheld","reconciliation":"read-original-invocation"}),
        );
    }
    let mut view = retained(repository, run, &call_id)?;
    view["dispatched"] = json!(true);
    view["admission"] = json!("fresh");
    Ok(view)
}

fn retained(repository: &PostgresRepository, run: &str, call_id: &str) -> Result<Value> {
    let invocation = repository.host_query("invocation", &json!({"run":run,"call_id":call_id}))?;
    Ok(
        json!({"version":"mas-host-shared-native/1","run_id":run,"call_id":call_id,
        "admission":"retained","dispatched":false,"invocation":invocation}),
    )
}

fn translate_deadline(
    database_deadline: u64,
    database_now: u64,
    local_started: u64,
) -> Result<u64> {
    let remaining = database_deadline
        .checked_sub(database_now)
        .and_then(|remaining| remaining.checked_sub(1))
        .filter(|v| *v > 0)
        .ok_or("original shared deadline expired")?;
    local_started
        .checked_add(remaining)
        .ok_or_else(|| "shared deadline overflow".into())
}

#[cfg(test)]
mod tests {
    use super::translate_deadline;
    fn native_bundle() -> serde_json::Value {
        let executable = std::fs::canonicalize("/usr/bin/true").unwrap();
        serde_json::json!({"version":"mas-bundle/3","agents":[{"name":"native","executable":executable,"sha256":crate::digest(&std::fs::read(&executable).unwrap()),"handler":"run","operation":"native/1","environment":[],"working_directory":std::env::current_dir().unwrap(),"input_schema":{"type":"object","properties":{},"required":[],"additionalProperties":false},"output_schema":{"type":"object","properties":{},"required":[],"additionalProperties":false}}],"topology":{"agents":["client","native"],"application":"agent-task/1","edges":[["client","native"],["native","client"]],"wall_seconds":60,"max_calls":1}})
    }
    #[test]
    fn unsupported_static_profiles_fail_before_any_queue_claim() {
        let bundle = native_bundle();
        assert!(super::preflight(&bundle, "native").is_ok());
        assert!(super::preflight(&bundle, "missing").is_err());
        let mut changed = bundle.clone();
        changed["agents"][0]["environment"] = serde_json::json!(["PRIVATE_KEY"]);
        assert!(super::preflight(&changed, "native").is_err());
        changed = bundle.clone();
        changed["agents"][0]["configuration"] = serde_json::json!({"arbitrary":true});
        assert!(super::preflight(&changed, "native").is_err());
        changed = bundle.clone();
        changed["agents"][0]
            .as_object_mut()
            .unwrap()
            .remove("output_schema");
        assert!(super::preflight(&changed, "native").is_err());
        changed = bundle;
        changed["topology"]["wall_seconds"] = serde_json::json!(86_401);
        assert!(super::preflight(&changed, "native").is_err());
    }
    #[test]
    fn database_allowance_is_translated_without_clock_synchronisation() {
        assert_eq!(translate_deadline(10_000, 8_000, 100).unwrap(), 2_099);
        assert!(translate_deadline(8_000, 8_000, 100).is_err());
        assert!(translate_deadline(7_999, 8_000, 100).is_err());
        assert!(translate_deadline(10_000, 8_000, u64::MAX).is_err());
    }
}
