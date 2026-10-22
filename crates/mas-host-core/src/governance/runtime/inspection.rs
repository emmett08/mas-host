//! Validate bounded exact-source inspection before the existing structured gate.
use super::GovernedSession;
use mas_host_privacy::{
    CombinedAssessment, Completion, DetectorPins, Limits, NeuralAssessment, Source,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;
const MAX_TEXT_UNITS: usize = 128;
const MAX_BOUNDARY_BYTES: usize = 16_384;

/// Trusted host integration for the already provisioned local N1 detector.
/// Implementations must enforce the shared Unix-millisecond deadline, bound
/// worker resources, validate their approved launch closure and suppress logs.
/// Returning an assessment supplies coverage evidence, never release authority.
pub trait PrivacyInspector: Send + Sync {
    fn detector(&self) -> &DetectorPins;
    fn configuration_binding(&self) -> String;
    fn timeout_ms(&self) -> u64;
    fn inspect(
        &self,
        source: &Source,
        limits: Limits,
        deadline_ms: u64,
    ) -> Result<NeuralAssessment>;
}

pub(super) struct InspectionGate {
    inspector: Arc<dyn PrivacyInspector>,
    detector: DetectorPins,
    configuration_binding: String,
    timeout_ms: u64,
    destination: String,
}

impl InspectionGate {
    pub(super) fn contract(&self) -> Value {
        json!({"version":"mas-governed-inspection/1","detector":self.detector.value(),
            "configuration_binding":self.configuration_binding,"timeout_ms":self.timeout_ms,
            "destination":self.destination,"recipient":"privacy-inspector",
            "extraction":"json-text-components/1","max_boundary_bytes":MAX_BOUNDARY_BYTES,
            "max_text_units":MAX_TEXT_UNITS,"release_mode":"structured-projection-only"})
    }

    pub(super) fn check_implementation(&self) -> Result<()> {
        if *self.inspector.detector() != self.detector
            || self.inspector.configuration_binding() != self.configuration_binding
            || self.inspector.timeout_ms() != self.timeout_ms
        {
            return Err("governed-inspector-identity-changed".into());
        }
        Ok(())
    }
}

fn text_units<'a>(value: &'a Value, units: &mut Vec<&'a str>) -> Result<()> {
    match value {
        Value::String(text) => units.push(text),
        Value::Array(values) => {
            for value in values {
                text_units(value, units)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                units.push(key);
                if units.len() > MAX_TEXT_UNITS {
                    return Err("governed-inspection-limit".into());
                }
                text_units(value, units)?;
            }
        }
        _ => {}
    }
    if units.len() > MAX_TEXT_UNITS {
        return Err("governed-inspection-limit".into());
    }
    Ok(())
}

impl GovernedSession {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn authorize_inspected(
        &self,
        actor: &str,
        operation: &str,
        recipient: &str,
        destination: &str,
        original: &Value,
        projection: &Value,
        inspection: Option<Value>,
    ) -> Result<Value> {
        let Some(inspection) = inspection else {
            return self.authorize(
                actor,
                operation,
                recipient,
                destination,
                original,
                projection,
            );
        };
        let mut retained_bytes = projection.to_string().len();
        for _ in 0..8 {
            let disclosure = self.authorize_sized(
                actor,
                operation,
                recipient,
                destination,
                original,
                projection,
                retained_bytes,
            )?;
            let record = json!({"version":"mas-governed-boundary-decision/1","disclosure":disclosure,"inspection":inspection});
            let size = super::encoding::canonical(&json!([projection, record]), 65_536)?.len();
            if size <= retained_bytes {
                return Ok(record);
            }
            retained_bytes = size;
        }
        Err("governed-inspection-admission-limit".into())
    }

    /// Attach a trusted inspector before opening a fresh governed namespace.
    /// Its complete versioned contract becomes part of retained run identity.
    pub fn with_privacy_inspector(
        mut self,
        inspector: Arc<dyn PrivacyInspector>,
        destination: &str,
    ) -> Result<Self> {
        if self.2.is_some() {
            return Err("governed-inspector-already-configured".into());
        }
        let destination = super::super::identifier(&json!({"id":destination}), "id")
            .map_err(|_| "governed-inspector-destination")?;
        let configuration_binding = inspector.configuration_binding();
        if configuration_binding.len() != 64
            || !configuration_binding
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("governed-inspector-configuration-binding".into());
        }
        let timeout_ms = inspector.timeout_ms();
        if !(1..=120_000).contains(&timeout_ms) {
            return Err("governed-inspector-timeout".into());
        }
        self.2 = Some(Arc::new(InspectionGate {
            detector: inspector.detector().clone(),
            inspector,
            configuration_binding,
            timeout_ms,
            destination,
        }));
        self.check_current()?;
        Ok(self)
    }

    pub(super) fn inspect_boundary(
        &self,
        actor: &str,
        original: &Value,
        projection: &Value,
    ) -> Result<Option<Value>> {
        let Some(gate) = &self.2 else {
            return Ok(None);
        };
        let started = Instant::now();
        let deadline_ms = crate::now_ms()?
            .checked_add(gate.timeout_ms)
            .ok_or("governed-inspection-deadline")?;
        let duration = Duration::from_millis(gate.timeout_ms);
        gate.check_implementation()?;
        let combined = json!([original, projection]);
        let bytes = super::encoding::canonical(&combined, MAX_BOUNDARY_BYTES)?;
        let mut originals = Vec::new();
        text_units(original, &mut originals)?;
        let original_count = originals.len();
        let mut units = originals;
        text_units(projection, &mut units)?;
        // Processing raw text is a separate correlated policy tuple. The
        // scanner receives no source before this fresh authority admission.
        self.authorize(
            actor,
            "inspect",
            "privacy-inspector",
            &gate.destination,
            &combined,
            &combined,
        )?;
        let mut original_findings = 0usize;
        let mut projection_findings = 0usize;
        let mut inspected = BTreeMap::new();
        let limits = Limits::default();
        for (index, text) in units.iter().enumerate() {
            if started.elapsed() >= duration || crate::now_ms()? >= deadline_ms {
                return Err("governed-inspection-timeout".into());
            }
            gate.check_implementation()?;
            self.check_current()?;
            if let Some(count) = inspected.get(text) {
                if index < original_count {
                    original_findings += count;
                } else {
                    projection_findings += count;
                }
                continue;
            }
            let source = Source::text(
                &format!("boundary-unit-{index}"),
                text,
                limits.max_input_bytes,
            )
            .map_err(|_| "governed-inspection-source")?;
            let neural = gate
                .inspector
                .inspect(&source, limits, deadline_ms)
                .map_err(|_| "governed-inspection-unavailable")?;
            if started.elapsed() >= duration || crate::now_ms()? >= deadline_ms {
                return Err("governed-inspection-timeout".into());
            }
            gate.check_implementation()?;
            neural
                .validate(&source, &gate.detector, limits)
                .map_err(|_| "governed-inspection-invalid")?;
            let assessment = CombinedAssessment::new(&source, neural, limits)
                .map_err(|_| "governed-inspection-invalid")?;
            if assessment.completion() != Completion::Complete {
                return Err("governed-inspection-incomplete".into());
            }
            inspected.insert(*text, assessment.findings().len());
            if index < original_count {
                original_findings += assessment.findings().len();
            } else {
                projection_findings += assessment.findings().len();
            }
        }
        gate.check_implementation()?;
        self.check_current()?;
        if started.elapsed() >= duration || crate::now_ms()? >= deadline_ms {
            return Err("governed-inspection-timeout".into());
        }
        if projection_findings != 0 {
            return Err("governed-inspection-prohibited-projection".into());
        }
        Ok(Some(json!({"version":"mas-governed-inspection-receipt/1",
            "contract":gate.contract(),"original_binding":self.commitment(original)?,
            "projection_binding":self.commitment(projection)?,"completion":"complete",
            "coverage":"complete-within-json-text-components/1",
            "original_findings_count":original_findings,"projected_findings_count":projection_findings,
            "text_units":units.len(),"inspected_sources":inspected.len(),"boundary_bytes":bytes.len()})))
    }
}

#[cfg(test)]
#[path = "inspection_tests.rs"]
mod tests;
