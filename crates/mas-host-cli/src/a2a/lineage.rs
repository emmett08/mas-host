//! Resolve exact bounded metadata pages under the source's original authority.
use super::{fields, retained, retained_source, Caller, RpcError, RpcResult, Service};
use serde_json::{json, Value};

pub(super) const PROFILE: &str = "urn:mas:host-governed-lineage-read:1";

fn page_selection(params: &Value) -> RpcResult<(&str, usize)> {
    let closure = params["closure_ref"]
        .as_str()
        .filter(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or(RpcError(-32602, "invalid_parameters"))?;
    let offset = params["offset"]
        .as_u64()
        .filter(|offset| *offset <= 256)
        .ok_or(RpcError(-32602, "invalid_parameters"))?;
    Ok((closure, offset as usize))
}

impl Service {
    pub(super) fn retained_source_lineage(
        &self,
        reader: &Caller,
        params: &Value,
    ) -> RpcResult<Value> {
        fields(params, &["id", "key", "closure_ref", "offset"], &[])?;
        let selection = json!({"id":params["id"],"key":params["key"]});
        let (id, key) = retained::request(&selection, retained_source::unavailable)?;
        let (closure, offset) = page_selection(params)?;
        let session = reader
            .session
            .as_ref()
            .ok_or_else(retained_source::unavailable)?;
        let (directory, actor) = self
            .owned_governed_task(reader, id)
            .ok_or_else(retained_source::unavailable)?;
        mas_host_core::governance::read_governed_source_lineage_page(
            &directory, session, session, &actor, key, closure, offset,
        )
        .map_err(|_| retained_source::unavailable())
    }

    pub(super) fn shared_source_lineage(
        &self,
        reader: &Caller,
        params: &Value,
    ) -> RpcResult<Value> {
        fields(params, &["reference", "closure_ref", "offset"], &[])?;
        let reference = params["reference"]
            .as_str()
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 128
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
            })
            .ok_or(RpcError(-32602, "invalid_parameters"))?;
        let (closure, offset) = page_selection(params)?;
        let (directory, owner, actor, key) = self.shared_source_selection(reader, reference)?;
        let owner_session = owner
            .session
            .as_ref()
            .ok_or_else(retained_source::unavailable)?;
        let reader_session = reader
            .session
            .as_ref()
            .ok_or_else(retained_source::unavailable)?;
        mas_host_core::governance::read_governed_source_lineage_page(
            &directory,
            owner_session,
            reader_session,
            &actor,
            &key,
            closure,
            offset,
        )
        .map_err(|_| retained_source::unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lineage_selection_retains_exact_unsigned_offset_and_bounded_reference() {
        let closure = "a".repeat(64);
        assert!(page_selection(&json!({"closure_ref":closure,"offset":0})).is_ok());
        for invalid in [json!(-1), json!(257), json!(1.5), json!("16"), json!(null)] {
            assert!(page_selection(&json!({"closure_ref":closure,"offset":invalid})).is_err());
        }
        for invalid in [
            "".to_owned(),
            "\n".into(),
            "x".repeat(64),
            "a".repeat(65),
            "A".repeat(64),
        ] {
            assert!(page_selection(&json!({"closure_ref":invalid,"offset":0})).is_err());
        }
    }
}
