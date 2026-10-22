//! Require explicit operator approval and static prerequisites for local packages.
use super::contract::{declaration, digest, hash, names};
use crate::config::{authoring, fields};
use serde_json::{json, Value};
use std::path::Path;

pub(super) struct Approval(Value);

impl Approval {
    pub fn read(path: &Path) -> Result<Self, String> {
        let value = authoring::read(path)?;
        fields(
            &value,
            &[
                "version",
                "profile",
                "approved_packages",
                "requirements",
                "require_attestation",
            ],
            &[],
            "capability approval",
        )?;
        if value["version"] != "mas-capability-approvals/1" || value["profile"] != "trusted-local" {
            return Err("unsupported capability approval profile".into());
        }
        if value["require_attestation"] != false {
            return Err("capability attestation required: unsupported verifier".into());
        }
        let hashes = value["approved_packages"]
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= 64)
            .ok_or("capability approval needs bounded package digests")?;
        let mut unique = std::collections::BTreeSet::new();
        for value in hashes {
            hash(&json!({"sha256": value}), "sha256")?;
            if !unique.insert(value.as_str().unwrap()) {
                return Err("duplicate capability approval".into());
            }
        }
        fields(
            &value["requirements"],
            &[
                "grants",
                "credential_references",
                "destinations",
                "runtime_ids",
            ],
            &[],
            "operator requirements",
        )?;
        for key in [
            "grants",
            "credential_references",
            "destinations",
            "runtime_ids",
        ] {
            names(&value["requirements"][key])?;
        }
        Ok(Self(value))
    }

    pub fn check(&self, package: &Value) -> Result<(), String> {
        declaration(package)?;
        let expected = digest(package);
        if !self.0["approved_packages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hash| hash == &expected)
        {
            return Err("capability implementation lacks operator approval".into());
        }
        for key in ["grants", "credential_references", "destinations"] {
            if !names(&package["requires"][key])?.is_subset(&names(&self.0["requirements"][key])?) {
                return Err(format!("capability missing static requirement: {key}"));
            }
        }
        if !names(&self.0["requirements"]["runtime_ids"])?
            .contains(package["runtime"]["id"].as_str().unwrap())
        {
            return Err("capability incompatible runtime requirement".into());
        }
        Ok(())
    }

    pub fn digest(&self) -> String {
        digest(&self.0)
    }
}
