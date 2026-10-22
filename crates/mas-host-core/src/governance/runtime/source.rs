//! Host-sealed source metadata; payloads remain in the existing step journal.
use super::GovernedSession;
use serde_json::{json, Value};
use std::collections::BTreeSet;
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug)]
pub(crate) struct Source {
    pub(crate) body: Value,
}
impl Source {
    pub(crate) fn id(&self) -> &str {
        self.body["id"].as_str().unwrap()
    }
    pub(crate) fn step_id(&self) -> &str {
        self.body["step_id"].as_str().unwrap()
    }
    pub(crate) fn kind(&self) -> &str {
        self.body["kind"].as_str().unwrap()
    }
    pub(crate) fn metadata(&self) -> Value {
        self.body.clone()
    }
    pub(crate) fn refs(&self, field: &str) -> BTreeSet<String> {
        self.body[field]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    }
    pub(crate) fn parse(session: &GovernedSession, body: Value) -> Result<Self> {
        crate::fields(
            &body,
            &[
                "version",
                "id",
                "step_id",
                "kind",
                "actor",
                "payload_binding",
                "policy_refs",
                "parents",
                "acquired_at_ms",
                "expires_at_ms",
                "coverage",
                "classification",
                "payload_available",
            ],
            &[],
        )?;
        if body["version"] != "mas-governed-source/1"
            || body["coverage"] != "complete"
            || body["classification"] != "restricted"
            || !matches!(
                body["kind"].as_str(),
                Some("request-input" | "request-output" | "effect-output" | "effect-input")
            )
            || !body["payload_available"].is_boolean()
        {
            return Err("governed-source-invalid".into());
        }
        for field in ["id", "step_id", "actor", "payload_binding"] {
            super::super::identifier(&body, field).map_err(|_| "governed-source-invalid")?;
        }
        for field in ["policy_refs", "parents"] {
            let refs = body[field]
                .as_array()
                .filter(|r| r.len() <= 64)
                .ok_or("governed-source-invalid")?;
            let mut seen = BTreeSet::new();
            for value in refs {
                let id = value.as_str().ok_or("governed-source-invalid")?;
                super::super::identifier(&json!({"id":id}), "id")
                    .map_err(|_| "governed-source-invalid")?;
                if !seen.insert(id) {
                    return Err("governed-source-invalid".into());
                }
            }
            if field == "policy_refs" && refs.is_empty() {
                return Err("governed-source-invalid".into());
            }
        }
        let acquired = body["acquired_at_ms"]
            .as_u64()
            .ok_or("governed-source-invalid")?;
        let expiry = body["expires_at_ms"]
            .as_u64()
            .filter(|n| *n > acquired)
            .ok_or("governed-source-invalid")?;
        let _ = expiry;
        let mut unsigned = body.clone();
        unsigned.as_object_mut().unwrap().remove("id");
        if body["id"] != format!("source-{}", session.commitment(&unsigned)?) {
            return Err("governed-source-binding-mismatch".into());
        }
        session.validate_source_refs(&Self { body: body.clone() }.refs("policy_refs"))?;
        Ok(Self { body })
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create(
        session: &GovernedSession,
        step_id: &str,
        kind: &str,
        actor: &str,
        payload: &Value,
        parents: &BTreeSet<String>,
        refs: &BTreeSet<String>,
        parent_expiry: u64,
        payload_available: bool,
    ) -> Result<Self> {
        Self::create_at(
            session,
            step_id,
            kind,
            actor,
            payload,
            parents,
            refs,
            parent_expiry,
            payload_available,
            crate::now_ms()?,
            session.source_expiry(refs)?,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create_at(
        session: &GovernedSession,
        step_id: &str,
        kind: &str,
        actor: &str,
        payload: &Value,
        parents: &BTreeSet<String>,
        refs: &BTreeSet<String>,
        parent_expiry: u64,
        payload_available: bool,
        now: u64,
        frozen_expiry: u64,
    ) -> Result<Self> {
        let expiry = session
            .source_template_expiry(refs)?
            .min(parent_expiry)
            .min(frozen_expiry);
        if expiry <= now {
            return Err("governed-source-expired".into());
        }
        let mut body = json!({"version":"mas-governed-source/1","step_id":step_id,"kind":kind,"actor":actor,"payload_binding":session.commitment(payload)?,"policy_refs":refs,"parents":parents,"acquired_at_ms":now,"expires_at_ms":expiry,"coverage":"complete","classification":"restricted","payload_available":payload_available});
        body["id"] = json!(format!("source-{}", session.commitment(&body)?));
        Self::parse(session, body)
    }
}
