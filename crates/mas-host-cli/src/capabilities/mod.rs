//! Package approved local command capabilities through existing bundle identities.
mod approval;
mod binding;
mod contract;
mod files;
mod retained;
use crate::config::{authoring, fields, model::Agent};
use approval::Approval;
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

pub(crate) struct Packages {
    approval: Approval,
    declarations: BTreeMap<String, (Value, Value)>,
}

impl Packages {
    pub fn load(
        root: &Path,
        agents: &[Agent],
        sidecar: &Path,
        policy: &Path,
    ) -> Result<Self, String> {
        let sidecar = if sidecar.is_absolute() {
            sidecar.to_owned()
        } else {
            root.join(sidecar)
        };
        let policy = if policy.is_absolute() {
            policy.to_owned()
        } else {
            root.join(policy)
        };
        let sidecar = sidecar
            .canonicalize()
            .map_err(|_| "capability sidecar unavailable")?;
        if !sidecar.starts_with(root) {
            return Err("capability sidecar escapes project".into());
        }
        let value = authoring::read(&sidecar)?;
        fields(&value, &["version", "packages"], &[], "capability sidecar")?;
        if value["version"] != "mas-capabilities/1" {
            return Err("unsupported capability sidecar version".into());
        }
        let packages = value["packages"]
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= 31)
            .ok_or("capability sidecar requires 1..31 packages")?;
        let approval = Approval::read(&policy)?;
        let mut declarations = BTreeMap::new();
        for package in packages {
            contract::declaration(package)?;
            approval.check(package)?;
            let name = contract::text(package, "agent")?;
            let agent = agents
                .iter()
                .find(|agent| agent.name == name)
                .ok_or("capability names unknown agent")?;
            let source_spec = binding::validate_agent(root, package, agent)?;
            if declarations
                .insert(name.into(), (package.clone(), source_spec))
                .is_some()
            {
                return Err("duplicate capability agent binding".into());
            }
        }
        Ok(Self {
            approval,
            declarations,
        })
    }

    pub fn bind(&self, root: &Path, out: &Path, workers: &mut [Value]) -> Result<(), String> {
        let runner = files::digest_file(
            &std::env::current_exe().map_err(|_| "capability runner unavailable")?,
            1024 * 1024 * 1024,
        )?
        .0;
        for (name, (package, source_spec)) in &self.declarations {
            let worker = workers
                .iter_mut()
                .find(|worker| worker["name"] == *name)
                .ok_or("capability generated worker unavailable")?;
            binding::bind(root, out, worker, package, source_spec, &self.approval)?;
            retained::validate_worker(worker, &self.approval, &runner)?;
        }
        Ok(())
    }
}

pub(crate) fn check(path: &Path) -> Result<Value, String> {
    let package = authoring::read(path)?;
    contract::declaration(&package)?;
    Ok(
        json!({"version":"mas-capability-check/1","package_id":package["package_id"],
        "sha256":contract::digest(&package),"approval_granted":false}),
    )
}

pub(crate) fn validate_bundle(bundle: &Value, policy: Option<&Path>) -> Result<(), String> {
    let workers = bundle["agents"]
        .as_array()
        .ok_or("bundle agents required")?;
    let packaged: Vec<_> = workers
        .iter()
        .filter(|worker| worker["configuration"].get("capability").is_some())
        .collect();
    if packaged.is_empty() {
        if policy.is_some() {
            return Err("capability policy supplied without packaged capabilities".into());
        }
        return Ok(());
    }
    let approval =
        Approval::read(policy.ok_or("packaged capabilities require an explicit operator policy")?)?;
    let runner = files::digest_file(
        &std::env::current_exe().map_err(|_| "capability runner unavailable")?,
        1024 * 1024 * 1024,
    )?
    .0;
    for worker in packaged {
        retained::validate_worker(worker, &approval, &runner)?;
    }
    Ok(())
}
