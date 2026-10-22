//! Delivery failure does not change a host-settled effect's execution certainty.
use mas_host_sdk::{Json, Result};

pub(crate) fn require_available(output: &Json) -> Result<()> {
    if output["version"] == "mas-governed-unavailable/1" {
        return Err("effect completed; result unavailable under current handling policy".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mas_host_sdk::json;

    #[test]
    fn withheld_delivery_keeps_known_settlement_separate_from_external_execution() {
        for execution in ["completed", "not-sent", "refused", "unknown"] {
            let error = require_available(&json!({"version":"mas-governed-unavailable/1",
                "external_execution":execution,"payload_disposition":"withheld"}))
            .unwrap_err();
            assert_eq!(
                error,
                "effect completed; result unavailable under current handling policy"
            );
        }
        assert!(require_available(&json!({"status":"completed","output":{}})).is_ok());
    }
}
