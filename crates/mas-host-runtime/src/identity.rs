//! Compact references to the immutable configuration retained by the host journal.
use mas_host_sdk::{json, Json};
use sha2::{Digest, Sha256};

/// Bind resolved configuration without repeating its schemas and prompts in
/// every effect frame. The changing input remains directly inspectable.
pub(crate) fn effect_descriptor(format: &str, configuration: &Json, input: &Json) -> Json {
    let digest: String = Sha256::digest(configuration.to_string().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    json!({"format":format,"configuration_sha256":digest,"input":input})
}
