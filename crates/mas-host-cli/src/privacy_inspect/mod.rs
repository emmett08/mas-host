//! Expose local detector inspection without admitting or persisting a protected operation.
mod config;
mod runner;

use mas_host_privacy::{
    CombinedAssessment, ExecutionCertainty, HandlingContext, HandlingProfile, Limits,
    NeuralAssessment, Source,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Read, path::Path};

pub(super) fn execute(args: &BTreeMap<String, String>) -> Result<Value, String> {
    let config = config::Config::load(args)?;
    let limits = Limits::default();
    let input = config::required(args, "--input")?;
    let bytes = if input == "-" {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(limits.max_input_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "privacy input unreadable")?;
        bytes
    } else {
        config::read_bounded(Path::new(input), limits.max_input_bytes)?
    };
    if bytes.len() > limits.max_input_bytes {
        return Err("privacy input exceeds byte limit".into());
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(
            json!({"version":"mas-privacy-inspection/1", "status":"unsupported", "completion":"unsupported", "profile":"local-detector-inspection", "quality_qualified":false, "disposition":"hold", "release_permitted":false, "reason":"unsupported-input-encoding"}),
        );
    };
    let source = Source::text("local-input", text, limits.max_input_bytes)
        .map_err(|_| "privacy source unavailable")?;
    let neural = match runner::inspect(&config, &source, limits) {
        Ok(assessment) => assessment,
        Err(reason) => NeuralAssessment::failed(&source, &config.pins, reason, limits)
            .map_err(|_| "privacy assessment unavailable")?,
    };
    let combined = CombinedAssessment::new(&source, neural, limits)
        .map_err(|_| "privacy assessment invalid")?;
    let decision = combined
        .decide(
            &source,
            HandlingContext {
                profile: HandlingProfile::Protected,
                policy_available: true,
                authorised: true,
                findings_permitted: false,
                suspected_exposure: false,
                execution: ExecutionCertainty::NotSent,
            },
            limits,
        )
        .map_err(|_| "privacy handling invalid")?;
    let metadata = combined.metadata();
    let mut report = json!({
        "version":"mas-privacy-inspection/1", "status":"assessed",
        "source_bytes": bytes.len(), "detector": config.pins.value(),
        "completion": metadata["completion"], "finding_count": combined.findings().len(),
        "disposition": match decision.disposition {
            mas_host_privacy::Disposition::Withhold => "withhold", _ => "hold",
        },
        "release_permitted": false,
        "profile":"local-detector-inspection", "quality_qualified":false,
        "reason":format!("{:?}", decision.reason),
    });
    if config.details {
        report["restricted_assessment"] = metadata;
    }
    Ok(report)
}
