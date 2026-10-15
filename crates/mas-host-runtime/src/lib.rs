//! A small facade selecting maintained worker implementations.
pub mod inference;
pub mod schema;
use mas_host_sdk::{Context, Json, Result};
use sha2::{Digest, Sha256};

/// Invoked only through the private worker socket by the host CLI.
pub fn serve() -> Result<()> {
    mas_host_sdk::serve_configured(execute)
}

fn execute(ctx: &mut Context, input: Json, configuration: Json) -> Result<Json> {
    if let Some(assets) = configuration["assets"].as_array() {
        for asset in assets {
            let bytes = std::fs::read(
                asset["path"]
                    .as_str()
                    .ok_or("invalid provider asset path")?,
            )
            .map_err(|_| "provider asset unavailable")?;
            let digest: String = Sha256::digest(bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            if asset["sha256"] != digest {
                return Err("provider asset digest differs from retained binding".into());
            }
        }
    }
    match configuration["kind"].as_str() {
        Some("llm") => {
            let provider = mas_host_providers::create(&configuration["connection"])?;
            inference::execute(ctx, input, &configuration, provider.as_ref())
        }
        Some("workflow") => mas_host_workflow::execute(ctx, &configuration["plan"], input),
        _ => Err("unsupported built-in implementation".into()),
    }
}
