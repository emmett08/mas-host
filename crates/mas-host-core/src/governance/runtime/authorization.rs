//! Construct current policy decisions with exact payload bindings and retention bounds.
use super::GovernedSession;
use crate::governance::{
    decide_disclosure, CurrentAuthority, DataUsePolicy, DisclosureRequest, GovernedEvidence,
    InvocationScope,
};
use serde_json::{json, Value};
type Result<T> = std::result::Result<T, String>;

impl GovernedSession {
    pub(in crate::governance::runtime) fn authorize_resolution(
        &self,
        actor: &str,
        original: &Value,
        projection: &Value,
        inspect: bool,
    ) -> Result<()> {
        self.agent(actor)?;
        let inspection = if inspect {
            self.authorize(
                actor,
                "resolve",
                &self.0.principal,
                "client",
                original,
                projection,
            )?;
            self.inspect_boundary(actor, original, projection)?
        } else {
            None
        };
        self.authorize_inspected(
            actor,
            "resolve",
            &self.0.principal,
            "client",
            original,
            projection,
            inspection,
        )
        .map(|_| ())
    }

    pub(in crate::governance::runtime) fn authorize(
        &self,
        actor: &str,
        operation: &str,
        recipient: &str,
        destination: &str,
        original: &Value,
        projection: &Value,
    ) -> Result<Value> {
        self.authorize_sized(
            actor,
            operation,
            recipient,
            destination,
            original,
            projection,
            projection.to_string().len(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::governance::runtime) fn authorize_sized(
        &self,
        actor: &str,
        operation: &str,
        recipient: &str,
        destination: &str,
        original: &Value,
        projection: &Value,
        retained_bytes: usize,
    ) -> Result<Value> {
        if self.is_v2() {
            return self.authorize_exposure_sized(
                actor,
                operation,
                recipient,
                destination,
                original,
                projection,
                retained_bytes,
                &crate::governance::runtime::exposure::Exposure::default(),
            );
        }
        let snapshot = self.current()?;
        let now = crate::now_ms().map_err(|_| "governed-clock-unavailable")?;
        let binding = self.boundary_binding(original)?;
        let source_id = format!("source-{binding}");
        let mut policies = Vec::new();
        let mut policy_refs = Vec::new();
        for template in snapshot.policies {
            let mut association = template;
            let association_id = format!(
                "policy-{}",
                self.commitment(&json!([association, source_id]))?
            );
            association["id"] = json!(association_id);
            association["evidence_refs"] = json!([source_id]);
            policy_refs.push(association_id);
            policies.push(
                DataUsePolicy::from_value(&association).map_err(|_| "governed-policy-invalid")?,
            );
        }
        let expires = snapshot.grant["expires_at_ms"].as_u64().unwrap();
        let scope=InvocationScope::from_value(&json!({
            "version":"mas-invocation-scope/1","tenant":self.0.tenant,"principal":self.0.principal,"actor":actor,"producer":"host",
            "grant_ref":self.0.grant_ref,"grant_epoch":self.0.epoch,"purpose":self.0.purpose,"policy_version":self.0.policy_version,
            "permitted_operations":[operation],"evidence_refs":[source_id],
        })).map_err(|_|"governed-scope-invalid")?;
        let evidence=GovernedEvidence::from_value(&json!({
            "version":"mas-governed-evidence/1","id":source_id,"payload_ref":format!("inline-{binding}"),"binding_ref":binding,
            "source_ref":if operation == "inspect" {"trusted-local-inspection"} else {"trusted-local-structured"},"acquisition_ref":"host-boundary","coverage":"complete","classification":"restricted",
            "policy_refs":policy_refs,"parents":[],"transformation_ref":if operation == "inspect" {"json-text-components/1"} else {"closed-structured-allowlist/1"},"acquired_at_ms":now,"expires_at_ms":expires,
        })).map_err(|_|"governed-evidence-invalid")?;
        let request=DisclosureRequest::from_value(&json!({
            "version":"mas-disclosure-request/1","invocation_id":"boundary","operation":operation,"recipient":recipient,
            "destination":destination,"location":self.0.location,"input_binding_ref":binding,"output_binding_ref":self.boundary_binding(projection)?,
            "evidence_refs":[source_id],"fulfilled_obligations":["inline-retention-approved"],"byte_count":retained_bytes,
            "now_ms":now,"expires_at_ms":expires,
        })).map_err(|_|"governed-request-invalid")?;
        let authority =
            CurrentAuthority::from_value(&snapshot.grant).map_err(|_| "governed-grant-invalid")?;
        let decision = decide_disclosure(&scope, &request, &authority, &[evidence], &policies);
        if decision["disposition"] != "permitted-by-contract"
            || decision["binding_complete"] != true
        {
            return Err("governed-policy-denied".into());
        }
        Ok(decision)
    }
}
