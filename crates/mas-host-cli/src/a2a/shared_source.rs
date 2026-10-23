//! Operator-selected acquisition exports with independently authenticated readers.
use super::{fields, Caller, Result, RpcError, RpcResult, Service};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub(super) const PROFILE: &str = "urn:mas:host-governed-shared-source-read:1";

#[derive(Clone)]
pub(super) struct SourceExport {
    task_id: String,
    key: String,
    tenant: String,
    principal: String,
    purpose: String,
    grant_ref: String,
    actor: String,
}

fn identity(value: &Value, field: &str) -> Result<String> {
    value[field]
        .as_str()
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
        })
        .map(str::to_owned)
        .ok_or_else(|| "invalid shared source identity".into())
}

pub(super) fn parse(
    value: &Value,
    actors: &BTreeSet<String>,
    supported: bool,
) -> Result<BTreeMap<String, SourceExport>> {
    let Some(values) = value.get("source_exports") else {
        return Ok(BTreeMap::new());
    };
    if !supported {
        return Err("source exports require governed local version 2".into());
    }
    let values = values
        .as_array()
        .filter(|values| values.len() <= 64)
        .ok_or("shared source export limit")?;
    let mut exports = BTreeMap::new();
    for value in values {
        crate::config::fields(
            value,
            &[
                "name",
                "task_id",
                "key",
                "tenant",
                "principal",
                "purpose",
                "grant_ref",
                "actor",
            ],
            &[],
            "source export",
        )?;
        let name = identity(value, "name")?;
        // Reuse the exact owned-reader task/key grammar. No path can enter the
        // service from either this declaration or the public RPC.
        let request = json!({"id":value["task_id"],"key":value["key"]});
        let (task_id, key) = super::retained::request(&request, unavailable)
            .map_err(|_| "invalid shared source selection")?;
        let export = SourceExport {
            task_id: task_id.to_owned(),
            key: key.to_owned(),
            tenant: identity(value, "tenant")?,
            principal: identity(value, "principal")?,
            purpose: identity(value, "purpose")?,
            grant_ref: identity(value, "grant_ref")?,
            actor: identity(value, "actor")?,
        };
        if !actors.contains(&export.actor) || exports.insert(name, export).is_some() {
            return Err("unknown actor or repeated shared source export".into());
        }
    }
    Ok(exports)
}

pub(super) fn unavailable() -> RpcError {
    RpcError(-32001, "source_not_found")
}

impl Service {
    pub(super) fn shared_source(&self, reader: &Caller, params: &Value) -> RpcResult<Value> {
        fields(params, &["reference"], &[])?;
        let reference =
            identity(params, "reference").map_err(|_| RpcError(-32602, "invalid_parameters"))?;
        let (directory, owner, actor, key) = self.shared_source_selection(reader, &reference)?;
        let reader_session = reader.session.as_ref().ok_or_else(unavailable)?;
        let owner_session = owner.session.as_ref().ok_or_else(unavailable)?;
        mas_host_core::governance::read_governed_shared_source(
            &directory,
            owner_session,
            reader_session,
            &actor,
            &key,
        )
        .map_err(|_| unavailable())
    }

    pub(super) fn shared_source_selection(
        &self,
        reader: &Caller,
        reference: &str,
    ) -> RpcResult<(PathBuf, Caller, String, String)> {
        let (export, published_owner) = match self.configuration.source_exports.get(reference) {
            Some(export) => (export.clone(), None),
            None if self.configuration.source_publication => {
                let (export, owner) = self
                    .lookup_published_source(reference)?
                    .ok_or_else(unavailable)?;
                reader.current().map_err(|_| unavailable())?;
                (export, Some(owner))
            }
            None => return Err(unavailable()),
        };
        if !reader.agents.contains(&export.actor) {
            return Err(unavailable());
        }
        reader.session.as_ref().ok_or_else(unavailable)?;
        let ingress = self
            .configuration
            .governed
            .as_ref()
            .ok_or_else(unavailable)?;
        let owner = match published_owner {
            Some(owner) => owner,
            None => ingress
                .configured_caller(
                    &self.configuration,
                    &export.tenant,
                    &export.principal,
                    &export.purpose,
                    &export.grant_ref,
                )
                .ok_or_else(unavailable)?,
        };
        let (directory, actor) = self
            .owned_governed_task(&owner, &export.task_id)
            .filter(|(_, actor)| actor == &export.actor)
            .ok_or_else(unavailable)?;
        Ok((directory, owner, actor, export.key.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration() -> Value {
        json!({"name":"deployment-point","task_id":format!("a2a-{}","a".repeat(56)),
            "key":"collect","tenant":"tenant-a","principal":"alice",
            "purpose":"diagnosis","grant_ref":"grant-a","actor":"collector"})
    }

    #[test]
    fn exports_require_supported_profile_and_exact_bound_selection() {
        let actors = BTreeSet::from(["collector".into()]);
        assert!(parse(&json!({}), &actors, false).unwrap().is_empty());
        assert!(parse(&json!({"source_exports":[]}), &actors, false).is_err());
        assert!(parse(&json!({"source_exports":[declaration()]}), &actors, true).is_ok());
        for (field, invalid) in [
            ("task_id", "../forged"),
            ("actor", "unpublished"),
            ("key", "\n"),
            ("principal", ""),
        ] {
            let mut declaration = declaration();
            declaration[field] = json!(invalid);
            assert!(parse(&json!({"source_exports":[declaration]}), &actors, true).is_err());
        }
        let mut extra = declaration();
        extra["bearer"] = json!("never-allowed");
        assert!(parse(&json!({"source_exports":[extra]}), &actors, true).is_err());
        assert!(parse(
            &json!({"source_exports":[declaration(),declaration()]}),
            &actors,
            true
        )
        .is_err());
        assert!(parse(
            &json!({"source_exports":vec![declaration();65]}),
            &actors,
            true
        )
        .is_err());
    }
}
