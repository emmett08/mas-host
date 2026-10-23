//! CLI entrypoints for the bounded native shared operation and scheduling turn.
use crate::shared_connection::{lifetime, Authority};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

fn reservation(args: &BTreeMap<String, String>) -> Result<i64, String> {
    let amount = args
        .get("--reservation-microusd")
        .map(|v| v.parse::<i64>())
        .transpose()
        .map_err(|_| "invalid reservation")?
        .unwrap_or(0);
    if amount < 0 {
        return Err("invalid negative reservation".into());
    }
    Ok(amount)
}

fn bundle(args: &BTreeMap<String, String>) -> Result<Value, String> {
    let bundle = crate::config::read_json(Path::new(crate::require(args, "--bundle")?))?;
    crate::capabilities::validate_bundle(&bundle, None)?;
    Ok(bundle)
}

pub(super) fn execute(args: &BTreeMap<String, String>) -> Result<Value, String> {
    let mut authority = Authority::open(args)?;
    let bundle = bundle(args)?;
    let input = crate::config::read_json(Path::new(crate::require(args, "--input")?))?;
    let run = crate::require(args, "--run")?;
    let lock = lifetime(args, run)?;
    let principal = authority.text("principal")?.to_owned();
    let owner = authority.text("owner")?.to_owned();
    mas_host_core::shared_native::execute(
        &mut authority.repository,
        &bundle,
        &input,
        mas_host_core::shared_native::NativeRun {
            run_id: run,
            principal: &principal,
            owner: &owner,
            actor: crate::require(args, "--agent")?,
            reservation_microusd: reservation(args)?,
            max_deadline_ms: None,
        },
        &lock,
    )
}

pub(super) fn drain(args: &BTreeMap<String, String>) -> Result<Value, String> {
    let mut authority = Authority::open(args)?;
    let bundle = bundle(args)?;
    let owner = authority.text("owner")?.to_owned();
    let workflow = crate::require(args, "--workflow")?;
    let identity = crate::require(args, "--workflow-identity")?;
    let descriptor = mas_host_core::describe(&bundle)?;
    let definition = authority.repository.host_query(
        "definition.inspect",
        &json!({"kind":"workflow","name":workflow,"identity":identity}),
    )?;
    if definition["retired"] != false
        || definition["implementation_provenance"] != "verified_source_package"
        || definition["descriptor"]["bundle_digest"] != descriptor["bundle_digest"]
    {
        return Err("shared workflow differs from registered native bundle".into());
    }
    let manifest = authority.repository.host_query(
        "definition.manifest",
        &json!({"digest":definition["implementation_digest"]}),
    )?;
    if manifest["format"] != "mas-native-implementation/1"
        || manifest["bundle_digest"] != descriptor["bundle_digest"]
    {
        return Err("shared workflow native source package unavailable".into());
    }
    let lock = lifetime(args, &owner)?;
    let number = |name: &str, default: i64| {
        args.get(name)
            .map(|v| v.parse::<i64>())
            .transpose()
            .map_err(|_| format!("invalid {name}"))
            .map(|v| v.unwrap_or(default))
    };
    let turn = crate::shared_queue::Turn {
        owner: &owner,
        turn_id: crate::require(args, "--turn")?,
        after_seq: number("--after", 0)?,
        page_size: usize::try_from(number("--page-size", 1000)?)
            .map_err(|_| "invalid page size")?,
        max_pages: usize::try_from(number("--max-pages", 4)?).map_err(|_| "invalid page count")?,
        max_claims: usize::try_from(number("--max-claims", 1)?)
            .map_err(|_| "invalid claim count")?,
        workflow,
        workflow_identity: identity,
    };
    let actor = crate::require(args, "--agent")?;
    mas_host_core::shared_native::preflight(&bundle, actor)?;
    let amount = reservation(args)?;
    crate::shared_queue::drain(&mut authority.repository, &turn, |repository, task| {
        let value = mas_host_core::shared_native::execute(
            repository,
            &bundle,
            &task["input"],
            mas_host_core::shared_native::NativeRun {
                run_id: task["run_id"].as_str().ok_or("claimed run unavailable")?,
                principal: task["principal"]
                    .as_str()
                    .ok_or("claimed principal unavailable")?,
                owner: &owner,
                actor,
                reservation_microusd: amount,
                max_deadline_ms: Some(crate::shared_queue::task_deadline_ms(task)?),
            },
            &lock,
        )?;
        if value["admission"] == "deferred" && value["dispatched"] == false {
            return Ok(crate::shared_queue::DispatchOutcome::Deferred {
                not_before_ms: crate::shared_queue::provider_wake_ms(
                    task,
                    &value["not_before_ms"],
                )?,
            });
        }
        if matches!(
            value["invocation"]["state"].as_str(),
            Some("completed" | "resolved")
        ) && value["invocation"]["output"].is_object()
        {
            Ok(crate::shared_queue::DispatchOutcome::Completed(
                value["invocation"]["output"].clone(),
            ))
        } else {
            Ok(crate::shared_queue::DispatchOutcome::Unknown)
        }
    })
}
