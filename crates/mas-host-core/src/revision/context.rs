//! Assemble invocation inputs solely from the references permitted by frozen policy.
use super::super::{checked_object, fields, text, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(super) fn readers(value: &Value) -> Result<BTreeSet<String>> {
    let set = value
        .as_array()
        .ok_or("readers must be an array")?
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or("reader must be a nonempty string".to_owned())
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if set.contains("*") && set.len() != 1 {
        return Err("public reader marker cannot be combined with names".into());
    }
    Ok(set)
}
pub(super) fn permitted(readers: &BTreeSet<String>, actor: &str) -> bool {
    readers.contains("*") || readers.contains(actor)
}
pub(super) fn intersection(a: &BTreeSet<String>, b: &BTreeSet<String>) -> BTreeSet<String> {
    if a.contains("*") {
        b.clone()
    } else if b.contains("*") {
        a.clone()
    } else {
        a.intersection(b).cloned().collect()
    }
}
pub(super) fn names(value: &Value) -> Result<Vec<String>> {
    let values = value
        .as_array()
        .ok_or("reference list must be an array")?
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or("reference must be a nonempty string".to_owned())
        })
        .collect::<Result<Vec<_>>>()?;
    if values.len() > 64 || values.iter().collect::<BTreeSet<_>>().len() != values.len() {
        return Err("reference list must contain at most 64 unique references".into());
    }
    Ok(values)
}

pub(super) fn check_sources(input: &Value) -> Result<()> {
    fields(input, &["sources"], &[])?;
    let sources = input["sources"]
        .as_array()
        .filter(|s| s.len() <= 64)
        .ok_or("provide at most 64 sources")?;
    let mut ids = BTreeSet::new();
    for source in sources {
        fields(source, &["id", "kind", "content", "readers"], &[])?;
        let id = text(source, "id")?;
        if id.is_empty() || id.len() > 96 || !ids.insert(id) {
            return Err("source ids must be unique and bounded".into());
        }
        if !matches!(
            source["kind"].as_str(),
            Some("input" | "evidence" | "peer_history")
        ) {
            return Err("unsupported source kind".into());
        }
        checked_object(&source["content"], "source content")?;
        mas_protocol_codec::assessment::Evidence::from_value(&json!({"schema_version":1,"id":id,"content":source["content"],"content_sha256":super::canonical_digest(&source["content"])})).map_err(super::super::error)?;
        readers(&source["readers"])?;
    }
    Ok(())
}

/// A record's readers are never widened by deriving another record from it.
pub(super) fn assemble(
    actor: &str,
    ids: &[String],
    input: &Value,
    candidate: Option<(&Value, &Value)>,
    feedback: &[(&Value, &Value)],
    delegates: &[String],
) -> Result<Value> {
    let mut supplied = serde_json::Map::new();
    let mut evidence = Vec::new();
    let mut audience = BTreeSet::from(["*".to_owned()]);
    for id in ids {
        let source = input["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == *id)
            .ok_or("unknown source reference")?;
        let restriction = readers(&source["readers"])?;
        if !permitted(&restriction, actor) {
            return Err("invocation is not permitted to read requested source".into());
        }
        audience = intersection(&audience, &restriction);
        supplied.insert(id.clone(), source["content"].clone());
        evidence.push(json!({"schema_version":1,"id":id,"content":source["content"],"content_sha256":super::super::digest(source["content"].to_string().as_bytes())}));
    }
    if let Some((_, restriction)) = candidate {
        let restriction = readers(restriction)?;
        if !permitted(&restriction, actor) {
            return Err("invocation is not permitted to read candidate".into());
        }
        audience = intersection(&audience, &restriction);
    }
    let mut disclosed = Vec::new();
    for (assessment, restriction) in feedback {
        let restriction = readers(restriction)?;
        if !permitted(&restriction, actor) {
            continue;
        }
        audience = intersection(&audience, &restriction);
        disclosed.push((*assessment).clone());
    }
    let evidence_refs = evidence
        .iter()
        .map(|e| json!({"id":e["id"],"sha256":super::canonical_digest(e)}))
        .collect::<Vec<_>>();
    Ok(
        json!({"actor":actor,"input":{"sources":supplied,"evidence":evidence,"evidence_refs":evidence_refs,"candidate":candidate.map(|c|c.0),"feedback":disclosed},
        "policy":{"source_ids":ids,"allowed_agents":delegates,"allowed_effects":["mas-inference/2"]},"readers":audience}),
    )
}
