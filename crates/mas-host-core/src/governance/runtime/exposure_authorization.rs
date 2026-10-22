//! Conjoin original source policies over one complete host-owned closure.
use super::GovernedSession;
use crate::governance::runtime::exposure::Exposure;
use crate::governance::{
    decide_disclosure, CurrentAuthority, DataUsePolicy, DisclosureRequest, GovernedEvidence,
    InvocationScope,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, String>;

impl GovernedSession {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn authorize_exposure(
        &self,
        actor: &str,
        operation: &str,
        recipient: &str,
        destination: &str,
        original: &Value,
        projection: &Value,
        exposure: &Exposure,
    ) -> Result<Value> {
        self.authorize_exposure_sized(
            actor,
            operation,
            recipient,
            destination,
            original,
            projection,
            projection.to_string().len(),
            exposure,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn authorize_exposure_sized(
        &self,
        actor: &str,
        operation: &str,
        recipient: &str,
        destination: &str,
        original: &Value,
        projection: &Value,
        retained_bytes: usize,
        exposure: &Exposure,
    ) -> Result<Value> {
        if !self.is_v2() {
            return self.authorize_sized(
                actor,
                operation,
                recipient,
                destination,
                original,
                projection,
                retained_bytes,
            );
        }
        exposure.validate_for(self)?;
        let snapshot = self.current()?;
        let now = crate::now_ms()?;
        let binding = self.boundary_binding(original)?;
        let mut sources = Vec::new();
        for source in exposure.sources.values() {
            self.validate_source_refs(&source.refs("policy_refs"))?;
            sources.push(json!({"id":source.id(),"binding":source.body["payload_binding"],"policy_refs":source.body["policy_refs"],"parents":source.body["parents"],"acquired_at_ms":source.body["acquired_at_ms"],"expires_at_ms":source.body["expires_at_ms"]}));
        }
        if sources.is_empty() {
            sources.push(json!({"id":format!("source-{binding}"),"binding":binding,"policy_refs":self.baseline_refs(),"parents":[],"acquired_at_ms":now,"expires_at_ms":snapshot.grant["expires_at_ms"]}));
        }
        let referenced = if sources.len() > 64 {
            if !self.referenced_lineage_enabled() {
                return Err("governed-exposure-limit".into());
            }
            let ids = exposure.sources.keys().cloned().collect::<Vec<_>>();
            let closure_ref =
                self.identity_commitment(&json!(["mas-governed-lineage-reference/1", ids]))?;
            let mut refs = BTreeSet::new();
            let mut acquired = 0;
            let mut expiry = u64::MAX;
            for source in exposure.sources.values() {
                let time = source.body["acquired_at_ms"]
                    .as_u64()
                    .ok_or("governed-source-invalid")?;
                let end = source.body["expires_at_ms"]
                    .as_u64()
                    .ok_or("governed-source-invalid")?;
                if time > now
                    || end <= now
                    || source.body["coverage"] != "complete"
                    || source.body["classification"] != "restricted"
                {
                    return Err("governed-source-expired-or-incomplete".into());
                }
                acquired = acquired.max(time);
                expiry = expiry.min(end);
                refs.extend(source.refs("policy_refs"));
            }
            sources = vec![
                json!({"id":format!("closure-{closure_ref}"),"binding":closure_ref,"policy_refs":refs,"parents":[],"acquired_at_ms":acquired,"expires_at_ms":expiry}),
            ];
            Some(
                json!({"version":"mas-governed-lineage-reference/1","closure_ref":closure_ref,"source_count":exposure.sources.len(),"page_size":16,"policy_digest":snapshot.policy_digest,"representation":"complete-host-verified-policy-conjunction/1"}),
            )
        } else {
            None
        };
        let mut associated: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for source in &sources {
            for template in source["policy_refs"]
                .as_array()
                .ok_or("governed-source-invalid")?
            {
                associated
                    .entry(template.as_str().ok_or("governed-source-invalid")?.into())
                    .or_default()
                    .insert(source["id"].as_str().unwrap().into());
            }
        }
        let mut policies = Vec::new();
        let mut association_ids = BTreeMap::new();
        for (name, refs) in associated {
            let mut template = snapshot
                .policies
                .iter()
                .find(|p| p["id"] == name)
                .ok_or("governed-source-policy-unavailable")?
                .clone();
            let id = format!("policy-{}", self.commitment(&json!([template, refs]))?);
            association_ids.insert(name, id.clone());
            template["id"] = json!(id);
            template["evidence_refs"] = json!(refs);
            policies
                .push(DataUsePolicy::from_value(&template).map_err(|_| "governed-policy-invalid")?);
        }
        let ids: Vec<_> = sources.iter().map(|s| s["id"].as_str().unwrap()).collect();
        let scope=InvocationScope::from_value(&json!({"version":"mas-invocation-scope/1","tenant":self.0.tenant,"principal":self.0.principal,"actor":actor,"producer":"host","grant_ref":self.0.grant_ref,"grant_epoch":self.0.epoch,"purpose":self.0.purpose,"policy_version":self.0.policy_version,"permitted_operations":[operation],"evidence_refs":ids})).map_err(|_|"governed-scope-invalid")?;
        let mut evidence = Vec::new();
        for source in &sources {
            let refs: Vec<_> = source["policy_refs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| association_ids.get(r.as_str().unwrap()).unwrap())
                .collect();
            evidence.push(GovernedEvidence::from_value(&json!({"version":"mas-governed-evidence/1","id":source["id"],"payload_ref":format!("retained-{}",source["id"].as_str().unwrap()),"binding_ref":source["binding"],"source_ref":"trusted-local-structured","acquisition_ref":"host-boundary","coverage":"complete","classification":"restricted","policy_refs":refs,"parents":source["parents"],"transformation_ref":"closed-structured-allowlist/1","acquired_at_ms":source["acquired_at_ms"],"expires_at_ms":source["expires_at_ms"]})).map_err(|_|"governed-evidence-invalid")?);
        }
        let expires = snapshot.grant["expires_at_ms"]
            .as_u64()
            .unwrap()
            .min(exposure.expiry());
        let request=DisclosureRequest::from_value(&json!({"version":"mas-disclosure-request/1","invocation_id":"boundary","operation":operation,"recipient":recipient,"destination":destination,"location":self.0.location,"input_binding_ref":binding,"output_binding_ref":self.boundary_binding(projection)?,"evidence_refs":ids,"fulfilled_obligations":["inline-retention-approved"],"byte_count":retained_bytes,"now_ms":now,"expires_at_ms":expires})).map_err(|_|"governed-request-invalid")?;
        let authority =
            CurrentAuthority::from_value(&snapshot.grant).map_err(|_| "governed-grant-invalid")?;
        let mut decision = decide_disclosure(&scope, &request, &authority, &evidence, &policies);
        if decision["disposition"] != "permitted-by-contract"
            || decision["binding_complete"] != true
        {
            return Err("governed-policy-denied".into());
        }
        if let Some(reference) = referenced {
            decision["version"] = json!("mas-data-use-referenced-decision/1");
            decision["closure_reference"] = reference;
            super::super::encoding::canonical(&decision, 65_536)?;
        }
        Ok(decision)
    }
    pub(crate) fn authorize_resolution_with_exposure(
        &self,
        actor: &str,
        original: &Value,
        projection: &Value,
        inspect: bool,
        exposure: &Exposure,
    ) -> Result<()> {
        self.agent(actor)?;
        self.authorize_exposure(
            actor,
            "resolve",
            &self.0.principal,
            "client",
            original,
            projection,
            exposure,
        )?;
        let inspection = if inspect {
            self.inspect_boundary_with_exposure(actor, original, projection, exposure)?
        } else {
            None
        };
        self.authorize_inspected_with_exposure(
            actor,
            "resolve",
            &self.0.principal,
            "client",
            original,
            projection,
            inspection,
            exposure,
        )
        .map(|_| ())
    }
    pub(crate) fn authorize_resolution_payload_with_exposure(
        &self,
        actor: &str,
        view: &Value,
        payload: &Value,
        exposure: &Exposure,
    ) -> Result<()> {
        self.agent(actor)?;
        self.authorize_exposure(
            actor,
            "resolve",
            &self.0.principal,
            "client",
            view,
            view,
            exposure,
        )?;
        let inspection = self.inspect_boundary_with_exposure(actor, payload, payload, exposure)?;
        self.authorize_inspected_with_exposure(
            actor,
            "resolve",
            &self.0.principal,
            "client",
            view,
            view,
            inspection,
            exposure,
        )
        .map(|_| ())
    }
    pub(crate) fn authorize_status_with_exposure(
        &self,
        actor: &str,
        value: &Value,
        exposure: &Exposure,
    ) -> Result<()> {
        self.agent(actor)?;
        self.authorize_exposure(
            actor,
            "deliver",
            &self.0.principal,
            "client",
            value,
            value,
            exposure,
        )
        .map(|_| ())
    }
}

impl GovernedSession {
    pub(crate) fn authorize_assessment_resolution(
        &self,
        actor: &str,
        value: &Value,
        exposure: &Exposure,
    ) -> Result<()> {
        self.authorize_exposure(
            actor,
            "resolve-assessment",
            &self.0.principal,
            "client",
            value,
            value,
            exposure,
        )
        .map(|_| ())
    }
}

impl GovernedSession {
    pub(crate) fn authorize_assessment_adjudication(
        &self,
        actor: &str,
        value: &Value,
        exposure: &Exposure,
    ) -> Result<Value> {
        self.authorize_exposure(
            actor,
            "adjudicate-assessment",
            "host-assessment-journal",
            "host-journal",
            value,
            value,
            exposure,
        )
    }
}
