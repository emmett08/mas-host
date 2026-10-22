//! Administrator-owned publication and authority; never supplied by a card.
use super::governed::{Caller, GovernedIngress};
use super::{digest, Result, PROFILE};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
};

pub(super) struct Configuration {
    pub listen: SocketAddr,
    pub public_url: String,
    pub name: String,
    pub exports: BTreeMap<String, Value>,
    pub principals: BTreeMap<String, (String, BTreeSet<String>)>,
    pub max_active: usize,
    pub max_tasks: usize,
    pub governed: Option<GovernedIngress>,
    pub source_exports: BTreeMap<String, super::shared_source::SourceExport>,
    pub source_publication: bool,
}
fn string(value: &Value, key: &str, max: usize) -> Result<String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| format!("invalid {key}"))
}
fn bound(value: &Value, key: &str, default: u64, maximum: u64) -> Result<usize> {
    let v = value
        .get(key)
        .map(|v| v.as_u64().ok_or_else(|| format!("invalid {key}")))
        .transpose()?
        .unwrap_or(default);
    if v == 0 || v > maximum {
        return Err(format!("invalid {key}"));
    }
    Ok(v as usize)
}
impl Configuration {
    pub(super) fn source_reads_supported(&self) -> bool {
        self.governed
            .as_ref()
            .is_some_and(GovernedIngress::source_reads_supported)
    }

    pub(super) fn shared_reads_supported(&self) -> bool {
        self.source_reads_supported()
            && (!self.source_exports.is_empty() || self.source_publication)
    }

    pub(super) fn lineage_reads_supported(&self) -> bool {
        self.governed
            .as_ref()
            .is_some_and(GovernedIngress::referenced_lineage_enabled)
    }

    pub(super) fn connection_timeout(&self) -> std::time::Duration {
        connection_timeout(
            self.governed
                .as_ref()
                .and_then(|g| g.inspection_timeout_ms()),
        )
    }

    pub fn parse(value: &Value, descriptor: &Value, bundle: &Value) -> Result<Self> {
        let governed = match value["version"].as_str() {
            Some("mas-host-a2a/1") => None,
            Some("mas-host-a2a/2") => Some(GovernedIngress::load(value)?),
            _ => return Err("unsupported A2A configuration".into()),
        };
        let mut required = vec!["version", "listen", "public_url", "name", "exports"];
        if governed.is_some() {
            required.extend(["authentication_file", "governance_file"]);
        } else {
            required.push("principals");
        }
        let mut optional = vec!["max_active", "max_tasks"];
        if governed.is_some() {
            optional.push("privacy_inspector_file");
            optional.push("source_exports");
            optional.push("source_publication");
        }
        crate::config::fields(value, &required, &optional, "A2A configuration")?;
        let listen: SocketAddr = string(value, "listen", 100)?
            .parse()
            .map_err(|_| "invalid listen address")?;
        if !listen.ip().is_loopback() || listen.port() == 0 {
            return Err("A2A listen must be literal loopback with a nonzero port".into());
        }
        let public_url = string(value, "public_url", 2000)?;
        let uri: hyper::Uri = public_url.parse().map_err(|_| "invalid public_url")?;
        let authority = uri.authority().ok_or("public_url requires authority")?;
        if authority.as_str().contains('@')
            || uri.path() != "/a2a"
            || uri.query().is_some()
            || public_url.contains('#')
        {
            return Err(
                "public_url must identify /a2a without user information, query or fragment".into(),
            );
        }
        let loopback_http = uri.scheme_str() == Some("http")
            && authority
                .host()
                .trim_matches(&['[', ']'][..])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if uri.scheme_str() != Some("https") && !loopback_http {
            return Err("public_url requires HTTPS or literal loopback HTTP".into());
        }
        let mut exports = BTreeMap::new();
        for export in value["exports"]
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 31)
            .ok_or("expected 1..31 exports")?
        {
            crate::config::fields(
                export,
                &["agent", "name", "description", "public"],
                &[],
                "export",
            )?;
            let agent = string(export, "agent", 64)?;
            string(export, "name", 200)?;
            string(export, "description", 2000)?;
            if !export["public"].is_boolean() {
                return Err("export public must be boolean".into());
            }
            let contract = descriptor["agents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["name"] == agent)
                .ok_or("export names unknown agent")?;
            if contract["input_schema"].is_null() || contract["output_schema"].is_null() {
                return Err(
                    "exported operations require explicit input_schema and output_schema".into(),
                );
            }
            let edges = bundle["topology"]["edges"]
                .as_array()
                .ok_or("invalid topology")?;
            if !edges.contains(&json!(["client", agent]))
                || !edges.contains(&json!([agent, "client"]))
            {
                return Err("export requires client request and reply routes".into());
            }
            if exports.insert(agent, export.clone()).is_some() {
                return Err("duplicate export".into());
            }
        }
        let mut principals = BTreeMap::new();
        let mut tokens = BTreeSet::new();
        let empty = Vec::new();
        let records = if governed.is_some() {
            &empty
        } else {
            value["principals"]
                .as_array()
                .filter(|v| !v.is_empty() && v.len() <= 128)
                .ok_or("expected 1..128 principals")?
        };
        for principal in records {
            crate::config::fields(
                principal,
                &["name", "token_sha256", "agents"],
                &[],
                "principal",
            )?;
            let name = string(principal, "name", 128)?;
            let token = string(principal, "token_sha256", 64)?;
            if token.len() != 64
                || !token
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                || !tokens.insert(token.clone())
            {
                return Err("principal token hashes must be distinct SHA-256 hex digests".into());
            }
            let agents = principal["agents"]
                .as_array()
                .filter(|a| !a.is_empty() && a.len() <= 31)
                .ok_or("invalid principal grants")?;
            let mut grants = BTreeSet::new();
            for agent in agents {
                let agent = agent.as_str().ok_or("invalid principal grant")?;
                if !exports.contains_key(agent) || !grants.insert(agent.to_owned()) {
                    return Err("unknown or duplicate principal grant".into());
                }
            }
            if principals.insert(name, (token, grants)).is_some() {
                return Err("duplicate principal".into());
            }
        }
        let source_exports = super::shared_source::parse(
            value,
            &exports.keys().cloned().collect(),
            governed
                .as_ref()
                .is_some_and(GovernedIngress::source_reads_supported),
        )?;
        let source_publication = super::publication::enabled(
            value,
            governed
                .as_ref()
                .is_some_and(GovernedIngress::source_reads_supported),
        )?;
        let result = Self {
            listen,
            public_url,
            name: string(value, "name", 200)?,
            exports,
            principals,
            max_active: bound(value, "max_active", 4, 16)?,
            max_tasks: bound(value, "max_tasks", 1000, 10000)?,
            governed,
            source_exports,
            source_publication,
        };
        if let Some(governed) = &result.governed {
            governed.validate(&result)?;
        }
        Ok(result)
    }
    pub fn authenticate(&self, bearer: &str) -> Option<Caller> {
        if !(32..=512).contains(&bearer.len()) || !bearer.bytes().all(|b| b.is_ascii_graphic()) {
            return None;
        }
        if let Some(governed) = &self.governed {
            return governed.authenticate(self, bearer);
        }
        let hash = digest(bearer.as_bytes());
        self.principals
            .iter()
            .find(|(_, (expected, _))| {
                hash.bytes()
                    .zip(expected.bytes())
                    .fold(0u8, |d, (a, b)| d | (a ^ b))
                    == 0
            })
            .map(|(name, (_, agents))| Caller {
                owner: name.clone(),
                agents: agents.clone(),
                session: None,
                lease: None,
            })
    }
    pub fn card(&self, descriptor: &Value, principal: Option<&Caller>) -> Value {
        let allowed: BTreeSet<_> = match principal {
            Some(p) => p.agents.clone(),
            None => self
                .exports
                .iter()
                .filter(|(_, v)| v["public"] == true)
                .map(|(k, _)| k.clone())
                .collect(),
        };
        let mut projection = descriptor.clone();
        projection["agents"] = json!(descriptor["agents"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| allowed.contains(v["name"].as_str().unwrap()))
            .collect::<Vec<_>>());
        let skills:Vec<_>=allowed.iter().map(|agent| {let e=&self.exports[agent];json!({"id":agent,"name":e["name"],"description":e["description"],"tags":["nehemiah","bounded-workflow"],"inputModes":["application/json"],"outputModes":["application/json"]})}).collect();
        let mut card = json!({"name":self.name,"description":"Nehemiah bounded durable workflows through the Rust MAS host. Authorised exports only.","version":env!("CARGO_PKG_VERSION"),
            "supportedInterfaces":[{"url":self.public_url,"protocolBinding":"JSONRPC","protocolVersion":"1.0"}],
            "capabilities":{"streaming":false,"pushNotifications":false,"extendedAgentCard":true,
                "extensions":[{"uri":PROFILE,"required":true,"description":"Exact integer JSON, pinned operation contracts, durable request identity and explicit uncertain outcomes.",
                    "params":{"descriptor":projection,"methods":["SendMessage","GetTask","GetTaskStatus","CancelTask"],"cancellation":"intent-only; pending cancellation returns TaskNotCancelableError","restart":"unfinished accepted work becomes unknown; no automatic redispatch"}}]},
            "securitySchemes":{"bearer":{"httpAuthSecurityScheme":{"scheme":"Bearer"}}},"securityRequirements":[{"schemes":{"bearer":{"list":[]}}}],
            "defaultInputModes":["application/json"],"defaultOutputModes":["application/json"],"skills":skills});
        if self.governed.is_some() {
            card["capabilities"]["extensions"].as_array_mut().unwrap().push(json!({
                "uri":super::retained_effect::PROFILE,"required":false,
                "description":"Read an owned task's retained root effect under current resolve and inspection policy.",
                "params":{"requires":[PROFILE],"methods":["GetRetainedEffect"],
                    "requestExample":{"id":"task-id","key":"exact-effect-key"},
                    "requestFields":["id","key"],"additionalRequestFields":false,"maxKeyUtf8Bytes":128,
                    "resultVersion":"mas-governed-effect-view/1", "scope":"owned-task-root-workflow-effects",
                    "semantics":"read-only snapshot; no dispatch, settlement or reconciliation"}
            }));
        }
        if self.governed.is_some() {
            card["capabilities"]["extensions"].as_array_mut().unwrap().push(json!({
                "uri":super::retained_assessment::PROFILE,"required":false,
                "description":"Read and append a separately authorised exact component assessment review. Review grants no release or execution permission.",
                "params":{"requires":[PROFILE],"methods":["GetRetainedAssessment","AdjudicateAssessment"],
                    "resultVersion":"mas-governed-assessment-view/1","rawSource":"not-retained"}
            }));
        }
        if self.source_reads_supported() {
            card["capabilities"]["extensions"].as_array_mut().unwrap().push(json!({
                "uri":super::retained_source::PROFILE,"required":false,
                "description":"Resolve an owned task's exact retained source projection with its complete policy ancestry.",
                "params":{"requires":[PROFILE],"methods":["GetRetainedSource"],
                    "requestFields":["id","key"],"additionalRequestFields":false,"maxKeyUtf8Bytes":128,
                    "resultVersion":"mas-governed-source-view/1","scope":"owned-task-root-workflow-sources",
                    "semantics":"read-only snapshot; discarded raw originals are not retained"}
            }));
        }
        if self.shared_reads_supported() {
            card["capabilities"]["extensions"].as_array_mut().unwrap().push(json!({
                "uri":super::shared_source::PROFILE,"required":false,
                "description":"Read an operator-approved retained source under independent current reader authority.",
                "params":{"requires":[PROFILE],"methods":["GetSharedSource"],
                    "requestFields":["reference"],"additionalRequestFields":false,"maxReferenceUtf8Bytes":128,
                    "resultVersion":"mas-governed-source-view/1","scope":"operator-approved-shared-root-workflow-sources",
                    "semantics":"read-only snapshot; producer and recipient authority both required; no new acquisition"}
            }));
        }
        if self.source_publication {
            card["capabilities"]["extensions"].as_array_mut().unwrap().push(json!({
                "uri":super::publication::PROFILE,"required":false,
                "description":"Publish an immutable retained source selection under its owner's current publication authority.",
                "params":{"requires":[PROFILE],"methods":["PublishRetainedSource"],
                    "requestFields":["id","key","reference"],"additionalRequestFields":false,
                    "resultVersion":"mas-governed-source-publication/1","maxAliasUtf8Bytes":128,
                    "semantics":"retained completed source only; publication grants no recipient access or execution permission"}
            }));
        }
        if self.lineage_reads_supported() {
            let extensions = card["capabilities"]["extensions"].as_array_mut().unwrap();
            for extension in extensions.iter_mut().filter(|extension| {
                extension["uri"] == super::retained_source::PROFILE
                    || extension["uri"] == super::shared_source::PROFILE
            }) {
                extension["params"]
                    .as_object_mut()
                    .unwrap()
                    .remove("resultVersion");
                extension["params"]["resultVersions"] =
                    json!(["mas-governed-source-view/1", "mas-governed-source-view/2"]);
                extension["params"]["lineageExtension"] = json!(super::lineage::PROFILE);
            }
            let mut methods = vec!["GetRetainedSourceLineage"];
            if self.shared_reads_supported() {
                methods.push("GetSharedSourceLineage");
            }
            extensions.push(json!({
                "uri":super::lineage::PROFILE,"required":false,
                "description":"Resolve a complete retained source closure through exact currently authorised metadata pages.",
                "params":{"requires":[PROFILE],"methods":methods,
                    "resultVersion":"mas-governed-lineage-page/1","maxPageRecords":16,"maxPageBytes":65536,
                    "closureReferenceUtf8Bytes":64,"closureReferenceEncoding":"lowercase-hex",
                    "closureBinding":"exact reference from the selected source view",
                    "semantics":"complete closure admission repeated per page; no acquisition or execution recovery"}
            }));
        }
        card
    }
}

fn connection_timeout(inspection_timeout_ms: Option<u64>) -> std::time::Duration {
    // A completed SendMessage replay can inspect its input and its retained
    // result. The synchronous observation loop reaches at most one payload
    // inspection; earlier polls contain host-generated state only.
    std::time::Duration::from_millis(
        inspection_timeout_ms.map_or(10_000, |limit| limit * 2 + 10_000),
    )
}

#[cfg(test)]
mod timeout_tests {
    use super::connection_timeout;

    #[test]
    fn only_configured_inspection_adds_two_bounded_rpc_scan_budgets() {
        assert_eq!(connection_timeout(None).as_millis(), 10_000);
        assert_eq!(connection_timeout(Some(50)).as_millis(), 10_100);
        assert_eq!(connection_timeout(Some(120_000)).as_millis(), 250_000);
    }
}
