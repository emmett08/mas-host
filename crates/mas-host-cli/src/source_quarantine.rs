//! Trusted local custody administration and exact raw-source adjudication.
use crate::require;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};
pub(crate) fn execute(args: &BTreeMap<String, String>) -> Result<Value, String> {
    let config = crate::config::read_json(Path::new(require(args, "--governance")?))?;
    let mut session = mas_host_core::governance::GovernedSession::from_value(
        &config,
        require(args, "--tenant")?,
        require(args, "--principal")?,
        require(args, "--purpose")?,
        require(args, "--grant")?,
    )?;
    if let Some(path) = args.get("--privacy-inspector") {
        session =
            crate::privacy_inspect::GovernedInspector::load(Path::new(path))?.attach(session)?;
    }
    let actor = require(args, "--actor")?;
    use mas_host_core::governance::*;
    match require(args, "--action")? {
        "initialize" => initialize_governed_source_quarantine(&session, actor),
        "list" => list_governed_source_quarantine_page(
            &session,
            actor,
            args.get("--after").map(String::as_str),
        ),
        "read" => read_governed_source_quarantine(&session, actor, require(args, "--reference")?),
        "adjudicate" => adjudicate_governed_source_quarantine(
            &session,
            actor,
            require(args, "--reference")?,
            &crate::config::read_json(Path::new(require(args, "--review")?))?,
        ),
        action @ ("hold" | "release-hold" | "expire" | "retire") => govern_source_quarantine(
            &session,
            actor,
            require(args, "--reference")?,
            action,
            args.get("--expires-at-ms")
                .map(|n| {
                    n.parse::<u64>()
                        .map_err(|_| "invalid custody expiry".to_owned())
                })
                .transpose()?,
        ),
        _ => Err("invalid source quarantine action".into()),
    }
}
