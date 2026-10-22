//! Adapt an ordinary CLI to one durable MAS operation without shell evaluation.
mod spec;
use mas_host_process as process;
use mas_host_sdk::{json, Context, Json, Result};
use sha2::{Digest, Sha256};
pub use spec::{Argument, Specification};
use std::process::Command;

pub fn execute(ctx: &mut Context, input: Json, configuration: &Json) -> Result<Json> {
    let spec = Specification::parse(configuration)?;
    // Reject missing, composite or invalid argument values before effect admission.
    spec.render(&input, "pending")?;
    let stdin = if spec.json_stdin {
        input.to_string().into_bytes()
    } else {
        Vec::new()
    };
    let original_deadline = ctx.deadline_ms;
    let descriptor = super::identity::effect_descriptor("mas-command/2", configuration, &input);
    let outcome = ctx.effect("command", descriptor, |invocation_id| {
        let mut command = Command::new(&spec.executable);
        command.args(spec.render(&input, invocation_id)?);
        // The outer worker already has only the environment granted in its bundle.
        // Remove its transport capability name as well as the non-inheritable FD.
        command.env_remove("MAS_WORKER_CHANNEL_FD");
        let deadline = original_deadline.min(process::unix_ms().saturating_add(spec.timeout_seconds * 1000));
        let result = match process::run_bounded(command, stdin, deadline, spec.max_output_bytes) {
            Ok(result) => result,
            Err(process::Failure::NotStarted) => return Ok((
                json!({"status":"not_started"}),
                json!({"adapter":"mas-command/1","execution":"not_started","execution_scope":"command-process"}),
            )),
            Err(process::Failure::Incomplete) => return Err("command interrupted or exceeded output/deadline limits; external outcome uncertain".into()),
        };
        let code = result.status.code();
        if !code.is_some_and(|code| spec.success_exit_codes.contains(&code)) {
            return Err(format!("command process ended with {}; external outcome uncertain", result.status));
        }
        let digest: String = Sha256::digest(&result.stdout).iter().map(|b| format!("{b:02x}")).collect();
        // This adapter observed the command process exit successfully. It does
        // not establish completion of downstream operations started by the tool.
        // Preserve that process certainty even when stdout cannot be delivered.
        let evidence = json!({"adapter":"mas-command/1","invocation_id":invocation_id,
            "execution":"completed","execution_scope":"command-process",
            "exit_code":code,"stdout_sha256":digest,"stdout_bytes":result.stdout.len()});
        let output = std::str::from_utf8(&result.stdout).ok().and_then(|text| {
            if spec.text_stdout { Some(json!({"stdout":text})) }
            else { mas_protocol_codec::parse_json(text).ok() }
        });
        let Some(output) = output.filter(|output| output.is_object()
            && super::schema::validate(&configuration["output_schema"], output).is_ok()) else {
            return Ok((json!({"status":"invalid_output"}), evidence));
        };
        let outcome = json!({"status":"completed","output":output});
        // Include framing and evidence overhead before committing an outcome.
        let frame = json!({"type":"effect_complete","key":"command","output":outcome,"evidence":evidence});
        if mas_protocol_codec::parse_json(&frame.to_string()).is_err() {
            return Ok((json!({"status":"invalid_output"}), evidence));
        }
        Ok((outcome, evidence))
    })?;
    super::effect_result::require_available(&outcome)?;
    match outcome["status"].as_str() {
        Some("completed") => {
            super::schema::validate(&configuration["output_schema"], &outcome["output"])?;
            Ok(outcome["output"].clone())
        }
        Some("not_started") => Err("command was not started".into()),
        Some("invalid_output") => Err("command completed with invalid operation output".into()),
        _ => Err("invalid retained command outcome".into()),
    }
}
