//! Publish a retained source selection without granting its recipients authority.
use super::{fields, retained, shared_source, Caller, RpcError, RpcResult, Service};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

pub(super) const PROFILE: &str = "urn:mas:host-governed-source-publication:1";

pub(super) fn enabled(configuration: &Value, supported: bool) -> super::Result<bool> {
    match configuration.get("source_publication") {
        None => Ok(false),
        Some(value) if supported => value
            .as_bool()
            .ok_or_else(|| "source_publication must be boolean".into()),
        Some(_) => Err("source publication requires governed local version 2".into()),
    }
}

fn unavailable() -> RpcError {
    RpcError(-32001, "source_not_found")
}

fn conflict() -> RpcError {
    RpcError(-32602, "source_publication_conflict")
}

impl Service {
    pub(super) fn publish_source(&self, publisher: &Caller, params: &Value) -> RpcResult<Value> {
        fields(params, &["id", "key", "reference"], &[])?;
        let selection = json!({"id":params["id"],"key":params["key"]});
        let (id, key) = retained::request(&selection, unavailable)?;
        let alias = params["reference"]
            .as_str()
            .filter(|alias| {
                !alias.is_empty() && alias.len() <= 128 && !alias.chars().any(char::is_control)
            })
            .ok_or(RpcError(-32602, "invalid_parameters"))?;
        let session = publisher.session.as_ref().ok_or_else(unavailable)?;
        let (directory, actor) = self
            .owned_governed_task(publisher, id)
            .ok_or_else(unavailable)?;
        let alias_key = session
            .identity_commitment(&json!(["source-publication-alias/1", alias]))
            .map_err(|_| unavailable())?;
        let reference = format!(
            "shared-{}",
            session
                .identity_commitment(&json!(["published-source/1", alias, id, key, actor]))
                .map_err(|_| unavailable())?
        );
        let descriptor = json!({"reference":reference,"id":id,"key":key,"actor":actor});
        mas_host_core::governance::authorize_governed_source_publication(
            &directory,
            session,
            &actor,
            key,
            &descriptor,
        )
        .map_err(|_| unavailable())?;
        if self.configuration.source_exports.contains_key(&reference) {
            return Err(conflict());
        }
        let identity = publisher
            .source_export_identity()
            .map_err(|_| unavailable())?;
        let body = json!({"name":reference,"task_id":id,"key":key,"actor":actor,
            "tenant":identity["tenant"],"principal":identity["principal"],
            "purpose":identity["purpose"],"grant_ref":identity["grant_ref"]})
        .to_string();
        let binding = session
            .identity_commitment(&json!(["source-publication-binding/1", body]))
            .map_err(|_| unavailable())?;
        let mut db = self.db.lock().map_err(|_| unavailable())?;
        let tx = db.transaction().map_err(|_| unavailable())?;
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT owner,binding_digest FROM published_source_keys WHERE alias=?",
                [&alias_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| unavailable())?;
        if let Some((owner, digest)) = prior {
            if owner != publisher.owner || digest != binding {
                return Err(conflict());
            }
        } else {
            let count: i64 = tx
                .query_row("SELECT count(*) FROM published_source_keys", [], |row| {
                    row.get(0)
                })
                .map_err(|_| unavailable())?;
            if count >= self.configuration.max_tasks as i64 {
                return Err(RpcError(-32603, "source_publication_capacity_exhausted"));
            }
            tx.execute(
                "INSERT INTO published_source_keys(alias,owner,binding_digest) VALUES(?,?,?)",
                params![alias_key, publisher.owner, binding],
            )
            .map_err(|_| unavailable())?;
        }
        let prior: Option<(String, String, String)> = tx
            .query_row(
                "SELECT owner,alias,body FROM published_sources WHERE reference=?",
                [&reference],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| unavailable())?;
        if let Some((owner, alias, existing)) = prior {
            if owner != publisher.owner || alias != alias_key || existing != body {
                return Err(conflict());
            }
        } else {
            tx.execute(
                "INSERT INTO published_sources(reference,owner,alias,body) VALUES(?,?,?,?)",
                params![reference, publisher.owner, alias_key, body],
            )
            .map_err(|_| unavailable())?;
        }
        // Recheck the current authentication lease and grant immediately before
        // publication commit. A retained export still grants no disclosure.
        publisher.current().map_err(|_| unavailable())?;
        tx.commit().map_err(|_| unavailable())?;
        Ok(
            json!({"version":"mas-governed-source-publication/1","reference":reference,"status":"published"}),
        )
    }

    pub(super) fn lookup_published_source(
        &self,
        reference: &str,
    ) -> RpcResult<Option<(shared_source::SourceExport, Caller)>> {
        let selected: Option<(String, String, String, String)> = {
            let db = self.db.lock().map_err(|_| shared_source::unavailable())?;
            db.query_row("SELECT s.owner,s.body,k.owner,k.binding_digest FROM published_sources s JOIN published_source_keys k ON k.alias=s.alias WHERE s.reference=?",
                [reference],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))
                .optional().map_err(|_| shared_source::unavailable())?
        };
        let Some((owner, raw_body, key_owner, binding)) = selected else {
            return Ok(None);
        };
        if owner != key_owner || raw_body.len() > 65_536 {
            return Err(shared_source::unavailable());
        }
        let body =
            mas_protocol_codec::parse_json(&raw_body).map_err(|_| shared_source::unavailable())?;
        let mut records = shared_source::parse(
            &json!({"source_exports":[body]}),
            &self.configuration.exports.keys().cloned().collect(),
            true,
        )
        .map_err(|_| shared_source::unavailable())?;
        let export = records
            .remove(reference)
            .ok_or_else(shared_source::unavailable)?;
        let ingress = self
            .configuration
            .governed
            .as_ref()
            .ok_or_else(shared_source::unavailable)?;
        let producer = ingress
            .configured_caller(
                &self.configuration,
                body["tenant"]
                    .as_str()
                    .ok_or_else(shared_source::unavailable)?,
                body["principal"]
                    .as_str()
                    .ok_or_else(shared_source::unavailable)?,
                body["purpose"]
                    .as_str()
                    .ok_or_else(shared_source::unavailable)?,
                body["grant_ref"]
                    .as_str()
                    .ok_or_else(shared_source::unavailable)?,
            )
            .ok_or_else(shared_source::unavailable)?;
        let expected = producer
            .session
            .as_ref()
            .ok_or_else(shared_source::unavailable)?
            .identity_commitment(&json!(["source-publication-binding/1", raw_body]))
            .map_err(|_| shared_source::unavailable())?;
        if producer.owner != owner || expected != binding {
            return Err(shared_source::unavailable());
        }
        Ok(Some((export, producer)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_requires_explicit_boolean_in_supported_profile() {
        assert!(!enabled(&json!({}), false).unwrap());
        assert!(enabled(&json!({"source_publication":true}), true).unwrap());
        assert!(!enabled(&json!({"source_publication":false}), true).unwrap());
        assert!(enabled(&json!({"source_publication":true}), false).is_err());
        assert!(enabled(&json!({"source_publication":false}), false).is_err());
        for invalid in [json!("true"), json!(1), json!(null), json!([])] {
            assert!(enabled(&json!({"source_publication":invalid}), true).is_err());
        }
    }
}
