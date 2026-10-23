use mas_host_sdk::{json, Context, Json, Result};
use std::{
    fs::OpenOptions,
    io::Write,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn source_event(path: &str, request: &str, effect: &str, kind: &str) -> Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    let event =
        json!({"request_id":request,"effect_id":effect,"kind":kind,"at_ms":now.as_millis() as u64});
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "{}", event).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

/// Bounded synthetic work with an independent source log and a lost-reply fault.
pub fn occupy(context: &mut Context, input: Json) -> Result<Json> {
    let request = input["request_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .ok_or("bounded request_id required")?;
    let path = input["source_ledger"]
        .as_str()
        .ok_or("source_ledger required")?;
    let occupancy = input["occupancy_ms"]
        .as_u64()
        .filter(|ms| *ms <= 30_000)
        .ok_or("occupancy_ms must be 0..30000")?;
    let mode = input["fault_mode"].as_str().ok_or("fault_mode required")?;
    if !["success", "refuse-before-effect", "lost-completion"].contains(&mode) {
        return Err("unsupported fixture fault".into());
    }
    if mode == "refuse-before-effect" {
        return Err("synthetic refusal before effect admission".into());
    }
    context.effect(
        "occupancy",
        json!({"request_id":request,"occupancy_ms":occupancy,
        "fault_mode":mode,"source_ledger":path}),
        |effect| {
            source_event(path, request, effect, "started")?;
            thread::sleep(Duration::from_millis(occupancy));
            source_event(path, request, effect, "completed")?;
            if mode == "lost-completion" {
                // The independent source committed; the host receives no completion.
                std::process::exit(71);
            }
            Ok((
                json!({"request_id":request,"effect_id":effect,"done":true}),
                json!({"source":"synthetic-local-ledger","request_id":request}),
            ))
        },
    )
}
