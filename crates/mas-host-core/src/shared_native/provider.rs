//! Bounded observations from an operator-registered credential-free GET worker.
use super::{command, PostgresRepository, Result};
use crate::fields;
use serde_json::{json, Value};
const INVALID: &str = "registered provider observation invalid";

fn disposition(policy: &Value, output: &Value) -> Result<(&'static str, Option<i64>)> {
    let observation = &output["provider_observation"];
    fields(
        observation,
        &[
            "version",
            "provider",
            "quota_scope",
            "operation",
            "transport",
            "http_status",
            "response_class",
            "mutation_disposition",
            "retry_not_before_ms",
            "retry_hint_state",
            "retry_after_not_before_ms",
            "rate_limit_reset_ms",
            "throttle_kind",
            "source_time_ms",
            "history_coverage",
            "watch_cursor_sha256",
            "relist_required",
            "body_retained",
        ],
        &[],
    )?;
    if observation["version"] != "mas-provider-observation/1"
        || observation["quota_scope"] != policy["resource_key"]
        || observation["mutation_disposition"] != "not-applicable"
        || observation["body_retained"] != false
        || !matches!(
            observation["provider"].as_str(),
            Some("github" | "kubernetes")
        )
        || !matches!(
            observation["operation"].as_str(),
            Some("read" | "list" | "watch")
        )
        || !matches!(
            observation["retry_hint_state"].as_str(),
            Some("absent" | "valid" | "invalid")
        )
        || !matches!(
            observation["throttle_kind"].as_str(),
            Some("none" | "provider" | "primary" | "secondary-or-unspecified")
        )
    {
        return Err(INVALID.into());
    }
    for name in [
        "source_time_ms",
        "retry_not_before_ms",
        "retry_after_not_before_ms",
        "rate_limit_reset_ms",
    ] {
        if !(name != "source_time_ms" && observation[name].is_null())
            && !observation[name]
                .as_i64()
                .is_some_and(|n| (0..=253_402_300_799_999).contains(&n))
        {
            return Err(INVALID.into());
        }
    }
    let cursor = &observation["watch_cursor_sha256"];
    if !cursor.is_null()
        && !cursor.as_str().is_some_and(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err(INVALID.into());
    }
    let status = observation["http_status"].as_i64();
    let transport = observation["transport"].as_str().ok_or(INVALID)?;
    if !matches!(transport, "response" | "not-dispatched" | "outcome-unknown")
        || (transport == "response") != status.is_some_and(|n| (100..=599).contains(&n))
        || (transport != "response" && !observation["http_status"].is_null())
    {
        return Err(INVALID.into());
    }
    let class = observation["response_class"].as_str().ok_or(INVALID)?;
    let valid = match class {
        "accepted" => transport == "response" && status.is_some_and(|n| (200..=299).contains(&n)),
        "not-modified" => status == Some(304),
        "rate-limited" => {
            status == Some(429) || observation["provider"] == "github" && status == Some(403)
        }
        "expired-history" => {
            observation["provider"] == "kubernetes"
                && status == Some(410)
                && observation["operation"] != "read"
        }
        "http-refusal" => status.is_some_and(|n| (400..=499).contains(&n)),
        "redirect-denied" => status.is_some_and(|n| (300..=399).contains(&n)),
        "server-error" => status.is_some_and(|n| (500..=599).contains(&n)),
        "transport-unknown" => transport == "outcome-unknown",
        "not-dispatched" => transport == "not-dispatched",
        _ => false,
    };
    let throttle_valid = if class == "rate-limited" {
        if observation["provider"] == "github" {
            matches!(
                observation["throttle_kind"].as_str(),
                Some("primary" | "secondary-or-unspecified")
            )
        } else {
            observation["throttle_kind"] == "provider"
        }
    } else {
        observation["throttle_kind"] == "none"
    };
    if !valid
        || !throttle_valid
        || observation["relist_required"] != (class == "expired-history")
        || observation["history_coverage"]
            != if class == "expired-history" {
                "lost-interval"
            } else {
                "point-response"
            }
    {
        return Err(INVALID.into());
    }
    let category = match class {
        "rate-limited" | "server-error" | "transport-unknown" => "provider_failure",
        "not-dispatched" => "aborted",
        _ => "success",
    };
    let delay = if category == "provider_failure" {
        observation["retry_not_before_ms"].as_i64().map(|hint| {
            hint.saturating_sub(observation["source_time_ms"].as_i64().unwrap())
                .max(0)
        })
    } else {
        None
    };
    if observation["retry_hint_state"] == "valid" && observation["retry_not_before_ms"].is_null() {
        return Err(INVALID.into());
    }
    Ok((category, delay))
}

fn cooldown_until(policy: &Value, output: &Value, now: i64) -> Result<Option<i64>> {
    let (category, delay) = disposition(policy, output)?;
    let observation = &output["provider_observation"];
    if category == "provider_failure" && observation["retry_hint_state"] == "invalid" {
        // Unknown provider waiting bounds require operator reconciliation. A
        // finite fallback could resume earlier than the unparseable response.
        return Ok(Some(i64::MAX));
    }
    let minimum = if observation["response_class"] == "rate-limited" {
        policy["open_seconds"]
            .as_i64()
            .filter(|n| (1..=3600).contains(n))
            .and_then(|n| n.checked_mul(1000))
            .ok_or(INVALID)?
    } else {
        0
    };
    if delay.is_none() && minimum == 0 {
        return Ok(None);
    }
    // Transfer a conservative duration to the authority clock. Absolute worker
    // timestamps cannot safely serve as PostgreSQL admission deadlines.
    let translated = now
        .checked_add(delay.unwrap_or(0).max(minimum))
        .ok_or(INVALID)?;
    // Provider epoch reset/date hints are additional lower bounds. Retaining
    // both avoids an early retry whichever side's clock is ahead.
    let absolute = [
        "retry_not_before_ms",
        "retry_after_not_before_ms",
        "rate_limit_reset_ms",
    ]
    .iter()
    .filter_map(|name| observation[*name].as_i64())
    .max()
    .unwrap_or(0);
    Ok(Some(translated.max(absolute)))
}

pub(super) fn observe(
    repository: &mut PostgresRepository,
    run: &str,
    owner: &str,
    call: &str,
    policy: &Value,
    output: &Value,
) -> Result<()> {
    let (category, _) = disposition(policy, output)?;
    let now = repository.authority_status()?["database_time_ms"]
        .as_i64()
        .ok_or("provider authority clock unavailable")?;
    let not_before = cooldown_until(policy, output, now)?;
    if let Some(not_before_ms) = not_before {
        repository.host_command(
            &command(run, owner, "provider-defer"),
            "circuit.defer",
            &json!({"policy":policy,"call_id":call,"not_before_ms":not_before_ms}),
        )?;
    }
    repository.host_command(
        &command(run, owner, "provider-observe"),
        "circuit.observe",
        &json!({"policy":policy,"call_id":call,"category":category}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation() -> Value {
        json!({"provider_observation":{"version":"mas-provider-observation/1","provider":"github","quota_scope":"registered-scope","operation":"read","transport":"response","http_status":429,"response_class":"rate-limited","mutation_disposition":"not-applicable","retry_not_before_ms":5000,"retry_after_not_before_ms":5000,"rate_limit_reset_ms":null,"retry_hint_state":"valid","throttle_kind":"secondary-or-unspecified","source_time_ms":1000,"history_coverage":"point-response","watch_cursor_sha256":null,"relist_required":false,"body_retained":false}})
    }
    #[test]
    fn provider_waits_use_authority_time_and_invalid_hints_remain_held() {
        let policy = json!({"resource_key":"registered-scope","open_seconds":5});
        let mut value = observation();
        assert_eq!(
            cooldown_until(&policy, &value, 900000).unwrap(),
            Some(905000)
        );
        value["provider_observation"]["retry_not_before_ms"] = json!(61000);
        value["provider_observation"]["retry_hint_state"] = json!("absent");
        assert_eq!(
            cooldown_until(&policy, &value, 900000).unwrap(),
            Some(960000)
        );
        value["provider_observation"]["rate_limit_reset_ms"] = json!(1000000);
        assert_eq!(
            cooldown_until(&policy, &value, 900000).unwrap(),
            Some(1000000)
        );
        value["provider_observation"]["retry_hint_state"] = json!("invalid");
        assert_eq!(
            cooldown_until(&policy, &value, 900000).unwrap(),
            Some(i64::MAX)
        );
    }
    #[test]
    fn only_exact_registered_read_observations_can_set_cooldown() {
        let policy = json!({"resource_key":"registered-scope"});
        assert_eq!(
            disposition(&policy, &observation()).unwrap(),
            ("provider_failure", Some(4000))
        );
        for (name, value) in [
            ("quota_scope", json!("other")),
            ("operation", json!("mutation")),
            ("body_retained", json!(true)),
            ("http_status", json!(200)),
            ("provider", json!("a2a")),
            ("history_coverage", json!("complete")),
            ("retry_not_before_ms", json!("secret")),
        ] {
            let mut output = observation();
            output["provider_observation"][name] = value;
            assert!(disposition(&policy, &output).is_err(), "{name}");
        }
    }
}
