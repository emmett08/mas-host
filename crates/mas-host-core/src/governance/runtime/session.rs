//! Bind trusted identities to one closed structured policy and commitment key.
use super::{
    authority::{self, Snapshot},
    schema,
};
use crate::governance::{
    decide_disclosure, CurrentAuthority, DataUsePolicy, DisclosureRequest, GovernedEvidence,
    InvocationScope,
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
type Result<T> = std::result::Result<T, String>;

#[derive(Clone)]
pub(super) struct Effect {
    pub destination: String,
    pub descriptor_schema: Value,
    pub output_schema: Value,
    pub evidence_schema: Value,
}

#[derive(Clone)]
pub(super) struct AgentPolicy {
    pub destination: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub effects: BTreeMap<String, Effect>,
}

struct Session {
    authority_file: PathBuf,
    key_file: PathBuf,
    key: [u8; 32],
    key_id: String,
    tenant: String,
    principal: String,
    purpose: String,
    grant_ref: String,
    epoch: u64,
    policy_version: String,
    policy_digest: String,
    location: String,
    fingerprint: String,
    agents: BTreeMap<String, AgentPolicy>,
}

/// Trusted-local structured execution session. Construction consumes identities
/// already authenticated by the caller; it never authenticates worker fields.
/// Every admission and delivery re-reads current authority. Policy changes
/// require an explicit retained-run migration; credential epochs remain separate.
#[derive(Clone)]
pub struct GovernedSession(
    Arc<Session>,
    Option<Arc<dyn Fn() -> Result<()> + Send + Sync>>,
);

fn id(value: &Value, name: &str) -> Result<String> {
    super::super::identifier(value, name).map_err(|_| "governed-config-identifier".into())
}

fn path(value: &Value, name: &str) -> Result<PathBuf> {
    let path = value[name]
        .as_str()
        .filter(|s| s.len() <= 4096 && !s.chars().any(char::is_control))
        .map(PathBuf::from)
        .ok_or("governed-config-path")?;
    if !path.is_absolute() {
        return Err("governed-config-path".into());
    }
    Ok(path)
}

fn key(path: &Path) -> Result<[u8; 32]> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "governed-key-unavailable")?;
    let metadata = file.metadata().map_err(|_| "governed-key-unavailable")?;
    if !metadata.is_file() || metadata.len() != 32 || metadata.permissions().mode() & 0o077 != 0 {
        return Err("governed-key-requires-private-32-byte-file".into());
    }
    let mut key = [0; 32];
    file.read_exact(&mut key)
        .map_err(|_| "governed-key-unavailable")?;
    Ok(key)
}

impl GovernedSession {
    pub fn from_value(
        config: &Value,
        tenant: &str,
        principal: &str,
        purpose: &str,
        grant_ref: &str,
    ) -> Result<Self> {
        super::super::record(
            config,
            "mas-governed-local/1",
            &[
                "version",
                "authority_file",
                "commitment_key_file",
                "commitment_key_id",
                "location",
                "agents",
            ],
            &[],
        )
        .map_err(|_| "governed-config-invalid")?;
        for value in [tenant, principal, purpose, grant_ref] {
            id(&json!({"id":value}), "id")?;
        }
        let authority_file = path(config, "authority_file")?;
        let snapshot = authority::read(&authority_file, tenant, principal, grant_ref)?;
        let key_file = path(config, "commitment_key_file")?;
        let key = key(&key_file)?;
        let key_id = id(config, "commitment_key_id")?;
        let location = id(config, "location")?;
        let mut agents = BTreeMap::new();
        let declarations = config["agents"]
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 31)
            .ok_or("governed-config-agents")?;
        for declaration in declarations {
            crate::fields(
                declaration,
                &[
                    "actor",
                    "destination",
                    "input_schema",
                    "output_schema",
                    "effects",
                ],
                &[],
            )
            .map_err(|_| "governed-config-agent")?;
            let actor = id(declaration, "actor")?;
            for name in ["input_schema", "output_schema"] {
                schema::check(&declaration[name], 0)?;
            }
            let mut effects = BTreeMap::new();
            for effect in declaration["effects"]
                .as_array()
                .filter(|v| v.len() <= 16)
                .ok_or("governed-config-effects")?
            {
                crate::fields(
                    effect,
                    &[
                        "format",
                        "destination",
                        "descriptor_schema",
                        "output_schema",
                        "evidence_schema",
                    ],
                    &[],
                )
                .map_err(|_| "governed-config-effect")?;
                for name in ["descriptor_schema", "output_schema", "evidence_schema"] {
                    schema::check(&effect[name], 0)?;
                }
                if effects
                    .insert(
                        id(effect, "format")?,
                        Effect {
                            destination: id(effect, "destination")?,
                            descriptor_schema: effect["descriptor_schema"].clone(),
                            output_schema: effect["output_schema"].clone(),
                            evidence_schema: effect["evidence_schema"].clone(),
                        },
                    )
                    .is_some()
                {
                    return Err("governed-config-duplicate-effect".into());
                }
            }
            if agents
                .insert(
                    actor,
                    AgentPolicy {
                        destination: id(declaration, "destination")?,
                        input_schema: declaration["input_schema"].clone(),
                        output_schema: declaration["output_schema"].clone(),
                        effects,
                    },
                )
                .is_some()
            {
                return Err("governed-config-duplicate-agent".into());
            }
        }
        let fingerprint = crate::digest(json!({"config":config,"tenant":tenant,"principal":principal,
            "purpose":purpose,"grant_ref":grant_ref,"policy_version":snapshot.grant["policy_version"],
            "policy_digest":snapshot.policy_digest,"key_sha256":crate::digest(&key)}).to_string().as_bytes());
        Ok(Self(
            Arc::new(Session {
                authority_file,
                key_file,
                key,
                key_id,
                tenant: tenant.into(),
                principal: principal.into(),
                purpose: purpose.into(),
                grant_ref: grant_ref.into(),
                epoch: snapshot.grant["grant_epoch"].as_u64().unwrap(),
                policy_version: snapshot.grant["policy_version"].as_str().unwrap().into(),
                policy_digest: snapshot.policy_digest,
                location,
                fingerprint,
                agents,
            }),
            None,
        ))
    }

    /// Attach credential revalidation owned by trusted ingress. The callback
    /// can restrict current authority and cannot widen the configured policy.
    pub fn with_authentication_check(
        mut self,
        check: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    ) -> Self {
        self.1 = Some(check);
        self
    }

    pub fn fingerprint(&self) -> String {
        self.0.fingerprint.clone()
    }

    /// Bind stable transport identities independently of mutable grants/purpose.
    pub fn identity_commitment(&self, value: &Value) -> Result<String> {
        self.mac(
            &json!([
                "mas-governed-identity/1",
                self.0.key_id,
                self.0.tenant,
                self.0.principal
            ]),
            value,
            true,
        )
    }

    pub fn key_fingerprint(&self) -> String {
        crate::digest(
            json!([
                "mas-governed-key/1",
                self.0.key_id,
                crate::digest(&self.0.key)
            ])
            .to_string()
            .as_bytes(),
        )
    }

    pub fn commitment(&self, value: &Value) -> Result<String> {
        let context = json!([
            "mas-governed-original/1",
            self.0.key_id,
            self.0.tenant,
            self.0.principal,
            self.0.purpose,
            self.0.grant_ref
        ]);
        self.mac(&context, value, true)
    }

    fn boundary_binding(&self, value: &Value) -> Result<String> {
        self.mac(
            &json!([
                "mas-governed-original/1",
                self.0.key_id,
                self.0.tenant,
                self.0.principal,
                self.0.purpose,
                self.0.grant_ref
            ]),
            value,
            false,
        )
    }

    fn mac(&self, context: &Value, value: &Value, strict: bool) -> Result<String> {
        let encoded = super::encoding::canonical(value, 65_536)?;
        if strict {
            mas_protocol_codec::parse_json(
                std::str::from_utf8(&encoded).map_err(|_| "governed-input-invalid")?,
            )
            .map_err(|_| "governed-input-invalid")?;
        }
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.0.key).map_err(|_| "governed-key-invalid")?;
        mac.update(context.to_string().as_bytes());
        mac.update(&[0]);
        mac.update(&encoded);
        Ok(mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect())
    }

    pub fn check_current(&self) -> Result<()> {
        self.current().map(|_| ())
    }

    fn current(&self) -> Result<Snapshot> {
        if let Some(check) = &self.1 {
            check().map_err(|_| "governed-authentication-revoked")?;
        }
        if key(&self.0.key_file)? != self.0.key {
            return Err("governed-key-changed".into());
        }
        let snapshot = authority::read(
            &self.0.authority_file,
            &self.0.tenant,
            &self.0.principal,
            &self.0.grant_ref,
        )?;
        if snapshot.grant["grant_epoch"] != self.0.epoch
            || snapshot.grant["policy_version"] != self.0.policy_version
            || snapshot.policy_digest != self.0.policy_digest
        {
            return Err("governed-authority-changed".into());
        }
        Ok(snapshot)
    }

    pub(super) fn agent(&self, actor: &str) -> Result<&AgentPolicy> {
        self.0
            .agents
            .get(actor)
            .ok_or_else(|| "governed-agent-not-declared".into())
    }

    pub fn project_input(&self, actor: &str, value: &Value) -> Result<Value> {
        self.prepare_request(actor, value).map(|v| v.0)
    }

    pub(super) fn prepare_request(&self, actor: &str, value: &Value) -> Result<(Value, Value)> {
        self.commitment(value)?;
        let policy = self.agent(actor)?;
        let projected = schema::project(&policy.input_schema, value)?;
        let decision = self.authorize(
            actor,
            "invoke",
            &format!("worker:{actor}"),
            &policy.destination,
            value,
            &projected,
        )?;
        Ok((projected, decision))
    }

    pub(super) fn prepare_effect(&self, actor: &str, value: &Value) -> Result<(Value, Value)> {
        self.commitment(value)?;
        let format = value["format"].as_str().ok_or("governed-effect-format")?;
        let policy = self.effect(actor, format)?;
        let projected = schema::project(&policy.descriptor_schema, value)?;
        if projected != *value {
            return Err("governed-effect-input-must-already-be-minimised".into());
        }
        let decision = self.authorize(
            actor,
            "effect",
            &format!("operation:{format}"),
            &policy.destination,
            value,
            &projected,
        )?;
        Ok((projected, decision))
    }

    pub(super) fn effect(&self, actor: &str, format: &str) -> Result<&Effect> {
        self.agent(actor)?
            .effects
            .get(format)
            .ok_or_else(|| "governed-effect-not-declared".into())
    }

    pub fn check_delivery(&self, actor: &str, value: &Value) -> Result<()> {
        if !is_unavailable(value) {
            let projected = schema::project(&self.agent(actor)?.output_schema, value)?;
            if projected != *value {
                return Err("governed-retained-output-invalid".into());
            }
        }
        self.authorize(actor, "deliver", &self.0.principal, "client", value, value)
            .map(|_| ())
    }

    pub fn check_status_value(&self, actor: &str, value: &Value) -> Result<()> {
        self.agent(actor)?;
        if !value.is_object() {
            return Err("governed-status-invalid".into());
        }
        self.authorize(actor, "deliver", &self.0.principal, "client", value, value)
            .map(|_| ())
    }

    pub(crate) fn check_effect_delivery(
        &self,
        actor: &str,
        format: &str,
        value: &Value,
    ) -> Result<()> {
        if !is_unavailable(value) {
            let projected = schema::project(&self.effect(actor, format)?.output_schema, value)?;
            if projected != *value {
                return Err("governed-retained-output-invalid".into());
            }
        }
        self.authorize(
            actor,
            "invoke",
            &format!("worker:{actor}"),
            &self.agent(actor)?.destination,
            value,
            value,
        )
        .map(|_| ())
    }

    pub(super) fn project_output(&self, actor: &str, value: &Value) -> Result<Value> {
        if is_unavailable(value) {
            self.check_current()?;
            return Ok(unavailable("unknown"));
        }
        let output = schema::project(&self.agent(actor)?.output_schema, value)?;
        self.authorize(
            actor,
            "invoke",
            &format!("worker:{actor}"),
            &self.agent(actor)?.destination,
            value,
            &output,
        )?;
        Ok(output)
    }

    pub(super) fn request_delivery(
        &self,
        source_actor: &str,
        recipient_actor: &str,
        value: &Value,
    ) -> Result<()> {
        if recipient_actor == "client" {
            return self.check_delivery(source_actor, value);
        }
        self.authorize(
            source_actor,
            "invoke",
            &format!("worker:{recipient_actor}"),
            &self.agent(recipient_actor)?.destination,
            value,
            value,
        )
        .map(|_| ())
    }

    pub(super) fn authorize(
        &self,
        actor: &str,
        operation: &str,
        recipient: &str,
        destination: &str,
        original: &Value,
        projection: &Value,
    ) -> Result<Value> {
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
            "source_ref":"trusted-local-structured","acquisition_ref":"host-boundary","coverage":"complete","classification":"restricted",
            "policy_refs":policy_refs,"parents":[],"transformation_ref":"closed-structured-allowlist/1","acquired_at_ms":now,"expires_at_ms":expires,
        })).map_err(|_|"governed-evidence-invalid")?;
        let request=DisclosureRequest::from_value(&json!({
            "version":"mas-disclosure-request/1","invocation_id":"boundary","operation":operation,"recipient":recipient,
            "destination":destination,"location":self.0.location,"input_binding_ref":binding,"output_binding_ref":self.boundary_binding(projection)?,
            "evidence_refs":[source_id],"fulfilled_obligations":["inline-retention-approved"],"byte_count":projection.to_string().len(),
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

pub(super) fn unavailable(certainty: &str) -> Value {
    json!({"version":"mas-governed-unavailable/1","external_execution":certainty,"payload_disposition":"withheld"})
}

pub(super) fn is_unavailable(value: &Value) -> bool {
    value["version"] == "mas-governed-unavailable/1"
        && value.as_object().is_some_and(|o| o.len() == 3)
        && value["payload_disposition"] == "withheld"
        && value["external_execution"]
            .as_str()
            .is_some_and(|s| ["completed", "not-sent", "refused", "unknown"].contains(&s))
}
