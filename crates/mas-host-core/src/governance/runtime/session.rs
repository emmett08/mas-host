//! Bind trusted identities to one closed structured policy and commitment key.
#[path = "authorization.rs"]
mod authorization;
#[path = "exposure_authorization.rs"]
mod exposure_authorization;
use super::{
    authority::{self, Snapshot},
    exposure::Exposure,
    inspection::InspectionGate,
    schema,
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
type Result<T> = std::result::Result<T, String>;

#[derive(Clone)]
pub(crate) struct Effect {
    pub destination: String,
    pub descriptor_schema: Value,
    pub output_schema: Value,
    pub evidence_schema: Value,
    pub source_policy_refs: BTreeSet<String>,
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
    baseline_policy_refs: BTreeSet<String>,
    v2: bool,
    policy_template_ids: BTreeSet<String>,
    policy_template_expiry: BTreeMap<String, u64>,
}

/// Trusted-local structured execution session. Construction consumes identities
/// already authenticated by the caller; it never authenticates worker fields.
/// Every admission and delivery re-reads current authority. Policy changes
/// require an explicit retained-run migration; credential epochs remain separate.
#[derive(Clone)]
pub struct GovernedSession(
    Arc<Session>,
    Option<Arc<dyn Fn() -> Result<()> + Send + Sync>>,
    pub(super) Option<Arc<InspectionGate>>,
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

fn policy_refs(value: &Value, snapshot: &Snapshot) -> Result<BTreeSet<String>> {
    let values = value
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 64)
        .ok_or("governed-config-policy-refs")?;
    let mut refs = BTreeSet::new();
    for value in values {
        let name = value.as_str().ok_or("governed-config-policy-refs")?;
        id(&json!({"id":name}), "id")?;
        if !refs.insert(name.to_owned()) || !snapshot.policies.iter().any(|p| p["id"] == name) {
            return Err("governed-config-policy-refs".into());
        }
    }
    Ok(refs)
}

impl GovernedSession {
    pub fn from_value(
        config: &Value,
        tenant: &str,
        principal: &str,
        purpose: &str,
        grant_ref: &str,
    ) -> Result<Self> {
        let v2 = config["version"] == "mas-governed-local/2";
        super::super::record(
            config,
            if v2 {
                "mas-governed-local/2"
            } else {
                "mas-governed-local/1"
            },
            &[
                "version",
                "authority_file",
                "commitment_key_file",
                "commitment_key_id",
                "location",
                "agents",
            ],
            if v2 { &["baseline_policy_refs"] } else { &[] },
        )
        .map_err(|_| "governed-config-invalid")?;
        for value in [tenant, principal, purpose, grant_ref] {
            id(&json!({"id":value}), "id")?;
        }
        let authority_file = path(config, "authority_file")?;
        let snapshot = authority::read(&authority_file, tenant, principal, grant_ref)?;
        let baseline_policy_refs = if v2 {
            policy_refs(&config["baseline_policy_refs"], &snapshot)?
        } else {
            BTreeSet::new()
        };
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
                    if v2 { &["source_policy_refs"] } else { &[] },
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
                            source_policy_refs: if effect.get("source_policy_refs").is_some() {
                                policy_refs(&effect["source_policy_refs"], &snapshot)?
                            } else {
                                BTreeSet::new()
                            },
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
                policy_template_expiry: snapshot
                    .policies
                    .iter()
                    .map(|p| {
                        (
                            p["id"].as_str().unwrap().to_owned(),
                            p["expires_at_ms"].as_u64().unwrap(),
                        )
                    })
                    .collect(),
                policy_template_ids: snapshot
                    .policies
                    .iter()
                    .map(|p| p["id"].as_str().unwrap().to_owned())
                    .collect(),
                policy_digest: snapshot.policy_digest,
                location,
                fingerprint,
                agents,
                baseline_policy_refs,
                v2,
            }),
            None,
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
        match &self.2 {
            Some(gate) => crate::digest(
                json!([self.0.fingerprint, gate.contract()])
                    .to_string()
                    .as_bytes(),
            ),
            None => self.0.fingerprint.clone(),
        }
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

    pub fn is_v2(&self) -> bool {
        self.0.v2
    }

    pub(crate) fn baseline_refs(&self) -> &BTreeSet<String> {
        &self.0.baseline_policy_refs
    }

    pub(crate) fn validate_source_refs(&self, refs: &BTreeSet<String>) -> Result<()> {
        if refs.is_empty()
            || !self.0.baseline_policy_refs.is_subset(refs)
            || !refs.is_subset(&self.0.policy_template_ids)
        {
            return Err("governed-source-policy-unavailable".into());
        }
        Ok(())
    }

    pub(crate) fn source_template_expiry(&self, refs: &BTreeSet<String>) -> Result<u64> {
        self.validate_source_refs(refs)?;
        refs.iter()
            .map(|id| {
                self.0
                    .policy_template_expiry
                    .get(id)
                    .copied()
                    .ok_or_else(|| "governed-source-policy-unavailable".into())
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .min()
            .ok_or_else(|| "governed-source-policy-unavailable".into())
    }
    pub(crate) fn source_expiry(&self, refs: &BTreeSet<String>) -> Result<u64> {
        let templates = self.source_template_expiry(refs)?;
        let snapshot = self.current()?;
        Ok(snapshot.grant["expires_at_ms"]
            .as_u64()
            .ok_or("governed-grant-invalid")?
            .min(templates))
    }
    pub(crate) fn admitted_expiry(decision: &Value) -> Result<u64> {
        decision.get("disclosure").unwrap_or(decision)["expires_at_ms"]
            .as_u64()
            .ok_or_else(|| "governed-admission-expiry-unavailable".into())
    }

    pub(crate) fn authorize_delivery_without_inspection(
        &self,
        source_actor: &str,
        recipient_actor: &str,
        value: &Value,
        exposure: &Exposure,
    ) -> Result<()> {
        if recipient_actor == "client" {
            self.authorize_exposure(
                source_actor,
                "deliver",
                &self.0.principal,
                "client",
                value,
                value,
                exposure,
            )
            .map(|_| ())
        } else {
            self.authorize_exposure(
                source_actor,
                "invoke",
                &format!("worker:{recipient_actor}"),
                &self.agent(recipient_actor)?.destination,
                value,
                value,
                exposure,
            )
            .map(|_| ())
        }
    }

    pub(crate) fn authorize_request_admission(
        &self,
        actor: &str,
        value: &Value,
        exposure: &Exposure,
    ) -> Result<()> {
        let policy = self.agent(actor)?;
        self.authorize_exposure(
            actor,
            "invoke",
            &format!("worker:{actor}"),
            &policy.destination,
            value,
            value,
            exposure,
        )
        .map(|_| ())
    }

    pub fn check_current(&self) -> Result<()> {
        self.current().map(|_| ())
    }

    fn current(&self) -> Result<Snapshot> {
        if let Some(gate) = &self.2 {
            gate.check_implementation()?;
        }
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
        self.prepare_request_with_exposure(actor, value, &Exposure::default())
    }

    pub(crate) fn prepare_request_with_exposure(
        &self,
        actor: &str,
        value: &Value,
        exposure: &Exposure,
    ) -> Result<(Value, Value)> {
        self.commitment(value)?;
        let policy = self.agent(actor)?;
        let projected = schema::project(&policy.input_schema, value)?;
        let inspection = self.inspect_boundary_with_exposure(actor, value, &projected, exposure)?;
        let decision = self.authorize_inspected_with_exposure(
            actor,
            "invoke",
            &format!("worker:{actor}"),
            &policy.destination,
            value,
            &projected,
            inspection,
            exposure,
        )?;
        Ok((projected, decision))
    }

    pub(crate) fn prepare_effect_with_exposure(
        &self,
        actor: &str,
        value: &Value,
        exposure: &Exposure,
    ) -> Result<(Value, Value)> {
        self.commitment(value)?;
        let format = value["format"].as_str().ok_or("governed-effect-format")?;
        let policy = self.effect(actor, format)?;
        let augmented = if self.is_v2() && !policy.source_policy_refs.is_empty() {
            let refs = self
                .baseline_refs()
                .union(&policy.source_policy_refs)
                .cloned()
                .collect();
            let source = super::source::Source::create(
                self,
                "boundary-effect",
                "effect-input",
                actor,
                value,
                &exposure.roots,
                &refs,
                exposure.expiry(),
                true,
            )?;
            exposure.with_source(source)?
        } else {
            exposure.clone()
        };
        let exposure = &augmented;
        let projected = schema::project(&policy.descriptor_schema, value)?;
        if projected != *value {
            return Err("governed-effect-input-must-already-be-minimised".into());
        }
        let inspection = self.inspect_boundary_with_exposure(actor, value, &projected, exposure)?;
        let decision = self.authorize_inspected_with_exposure(
            actor,
            "effect",
            &format!("operation:{format}"),
            &policy.destination,
            value,
            &projected,
            inspection,
            exposure,
        )?;
        Ok((projected, decision))
    }

    pub(crate) fn effect(&self, actor: &str, format: &str) -> Result<&Effect> {
        self.agent(actor)?
            .effects
            .get(format)
            .ok_or_else(|| "governed-effect-not-declared".into())
    }

    pub fn check_delivery(&self, actor: &str, value: &Value) -> Result<()> {
        self.check_delivery_with_exposure(actor, value, &Exposure::default())
    }

    pub(crate) fn check_delivery_with_exposure(
        &self,
        actor: &str,
        value: &Value,
        exposure: &Exposure,
    ) -> Result<()> {
        if !is_unavailable(value) {
            let projected = schema::project(&self.agent(actor)?.output_schema, value)?;
            if projected != *value {
                return Err("governed-retained-output-invalid".into());
            }
        }
        let inspection = if is_unavailable(value) {
            None
        } else {
            self.inspect_boundary_with_exposure(actor, value, value, exposure)?
        };
        self.authorize_inspected_with_exposure(
            actor,
            "deliver",
            &self.0.principal,
            "client",
            value,
            value,
            inspection,
            exposure,
        )
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

    pub(crate) fn check_effect_delivery_with_exposure(
        &self,
        actor: &str,
        format: &str,
        value: &Value,
        exposure: &Exposure,
    ) -> Result<()> {
        if !is_unavailable(value) {
            let projected = schema::project(&self.effect(actor, format)?.output_schema, value)?;
            if projected != *value {
                return Err("governed-retained-output-invalid".into());
            }
        }
        let inspection = if is_unavailable(value) {
            None
        } else {
            self.inspect_boundary_with_exposure(actor, value, value, exposure)?
        };
        self.authorize_inspected_with_exposure(
            actor,
            "invoke",
            &format!("worker:{actor}"),
            &self.agent(actor)?.destination,
            value,
            value,
            inspection,
            exposure,
        )
        .map(|_| ())
    }

    pub(crate) fn project_output_with_exposure(
        &self,
        actor: &str,
        value: &Value,
        exposure: &Exposure,
    ) -> Result<Value> {
        if is_unavailable(value) {
            self.check_current()?;
            return Ok(unavailable("unknown"));
        }
        let output = schema::project(&self.agent(actor)?.output_schema, value)?;
        let inspection = self.inspect_boundary_with_exposure(actor, value, &output, exposure)?;
        self.authorize_inspected_with_exposure(
            actor,
            "invoke",
            &format!("worker:{actor}"),
            &self.agent(actor)?.destination,
            value,
            &output,
            inspection,
            exposure,
        )?;
        Ok(output)
    }

    pub(super) fn request_delivery(
        &self,
        source_actor: &str,
        recipient_actor: &str,
        value: &Value,
    ) -> Result<()> {
        self.request_delivery_with_exposure(
            source_actor,
            recipient_actor,
            value,
            &Exposure::default(),
        )
    }
    pub(crate) fn request_delivery_with_exposure(
        &self,
        source_actor: &str,
        recipient_actor: &str,
        value: &Value,
        exposure: &Exposure,
    ) -> Result<()> {
        if recipient_actor == "client" {
            return self.check_delivery_with_exposure(source_actor, value, exposure);
        }
        let inspection = if is_unavailable(value) {
            None
        } else {
            self.inspect_boundary_with_exposure(source_actor, value, value, exposure)?
        };
        self.authorize_inspected_with_exposure(
            source_actor,
            "invoke",
            &format!("worker:{recipient_actor}"),
            &self.agent(recipient_actor)?.destination,
            value,
            value,
            inspection,
            exposure,
        )
        .map(|_| ())
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
