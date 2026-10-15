//! Explicit integer-JSON schema subset, checked before provider execution.
use serde_json::Value;
type Result<T> = std::result::Result<T, String>;

/// The subset is deliberately closed: unsupported keywords fail at build time.
pub fn check(schema: &Value) -> Result<()> {
    let object = schema.as_object().ok_or("schema must be an object")?;
    let supported = [
        "type",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "enum",
        "minimum",
        "maximum",
        "minItems",
        "maxItems",
        "minLength",
        "maxLength",
        "anyOf",
        "description",
        "title",
    ];
    for key in object.keys() {
        if !supported.contains(&key.as_str()) {
            return Err(format!("unsupported schema keyword: {key}"));
        }
    }
    if let Some(kind) = schema.get("type") {
        if !matches!(
            kind.as_str(),
            Some("object" | "array" | "string" | "integer" | "boolean" | "null")
        ) {
            return Err("schema type must be one supported integer-JSON type".into());
        }
    }
    if let Some(properties) = schema.get("properties") {
        for child in properties
            .as_object()
            .ok_or("schema properties must be an object")?
            .values()
        {
            check(child)?;
        }
    }
    if let Some(required) = schema.get("required") {
        let items = required
            .as_array()
            .ok_or("schema required must be an array")?;
        let mut names = std::collections::BTreeSet::new();
        for item in items {
            if !names.insert(item.as_str().ok_or("required names must be strings")?) {
                return Err("duplicate required name".into());
            }
        }
    }
    if let Some(additional) = schema.get("additionalProperties") {
        if !additional.is_boolean() {
            check(additional)?;
        }
    }
    if let Some(items) = schema.get("items") {
        check(items)?;
    }
    if let Some(values) = schema.get("enum") {
        if values.as_array().is_none_or(|v| v.is_empty()) {
            return Err("schema enum must be a nonempty array".into());
        }
    }
    for name in ["minItems", "maxItems", "minLength", "maxLength"] {
        if schema.get(name).is_some_and(|v| v.as_u64().is_none()) {
            return Err(format!("{name} must be a nonnegative integer"));
        }
    }
    for name in ["minimum", "maximum"] {
        if let Some(v) = schema.get(name) {
            integer(v)?;
        }
    }
    if let Some(variants) = schema.get("anyOf") {
        let variants = variants
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or("anyOf must be a nonempty array")?;
        for variant in variants {
            check(variant)?;
        }
    }
    Ok(())
}

fn integer(value: &Value) -> Result<&str> {
    let text = value
        .as_number()
        .ok_or("schema bound must be an integer")?
        .as_str();
    if !text
        .strip_prefix('-')
        .unwrap_or(text)
        .bytes()
        .all(|b| b.is_ascii_digit())
    {
        return Err("schema requires exact integers".into());
    }
    Ok(text)
}

fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let an = a.starts_with('-') && a != "-0";
    let bn = b.starts_with('-') && b != "-0";
    if an != bn {
        return if an {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let a = a.trim_start_matches('-').trim_start_matches('0');
    let b = b.trim_start_matches('-').trim_start_matches('0');
    let order = a.len().cmp(&b.len()).then_with(|| a.cmp(b));
    if an {
        order.reverse()
    } else {
        order
    }
}

pub fn validate(schema: &Value, value: &Value) -> Result<()> {
    if let Some(variants) = schema.get("anyOf").and_then(Value::as_array) {
        if !variants.iter().any(|s| validate(s, value).is_ok()) {
            return Err("value satisfies no anyOf alternative".into());
        }
    }
    let correct_type = match schema["type"].as_str() {
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("integer") => integer(value).is_ok(),
        Some("boolean") => value.is_boolean(),
        Some("null") => value.is_null(),
        None => true,
        _ => false,
    };
    if !correct_type {
        return Err("value has wrong schema type".into());
    }
    if let Some(choices) = schema.get("enum").and_then(Value::as_array) {
        if !choices.contains(value) {
            return Err("value is outside schema enum".into());
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema["required"].as_array() {
            for name in required {
                if !object.contains_key(name.as_str().ok_or("invalid required property")?) {
                    return Err("required property is missing".into());
                }
            }
        }
        for (name, child) in object {
            if let Some(child_schema) = schema["properties"].get(name) {
                validate(child_schema, child)?;
            } else if schema["additionalProperties"] == false {
                return Err("additional property is forbidden".into());
            } else if schema["additionalProperties"].is_object() {
                validate(&schema["additionalProperties"], child)?;
            }
        }
    }
    if let Some(items) = value.as_array() {
        length(schema, items.len(), "minItems", "maxItems")?;
        if let Some(child_schema) = schema.get("items") {
            for item in items {
                validate(child_schema, item)?;
            }
        }
    }
    if let Some(s) = value.as_str() {
        length(schema, s.chars().count(), "minLength", "maxLength")?;
    }
    if let Ok(number) = integer(value) {
        if let Some(min) = schema.get("minimum") {
            if compare(number, integer(min)?).is_lt() {
                return Err("integer below minimum".into());
            }
        }
        if let Some(max) = schema.get("maximum") {
            if compare(number, integer(max)?).is_gt() {
                return Err("integer above maximum".into());
            }
        }
    }
    Ok(())
}

fn length(schema: &Value, len: usize, minimum: &str, maximum: &str) -> Result<()> {
    if schema[minimum].as_u64().is_some_and(|n| (len as u64) < n)
        || schema[maximum].as_u64().is_some_and(|n| (len as u64) > n)
    {
        return Err("schema length bound exceeded".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn schema_rejects_unsupported_rules_and_preserves_large_integer_bounds() {
        assert!(check(&json!({"type":"number"})).is_err());
        assert!(check(&json!({"type":"string","pattern":".*"})).is_err());
        let schema = json!({"type":"object","properties":{"n":{"type":"integer","minimum":-2,"maximum":4}},"required":["n"],"additionalProperties":false});
        check(&schema).unwrap();
        assert!(validate(&schema, &json!({"n":4})).is_ok());
        for value in [
            json!({"n":5}),
            json!({"n":-3}),
            json!({"n":1,"x":0}),
            json!({}),
        ] {
            assert!(validate(&schema, &value).is_err());
        }
        assert!(compare(
            "999999999999999999999999999999",
            "1000000000000000000000000000000"
        )
        .is_lt());
        assert!(compare(
            "-1000000000000000000000000000000",
            "-999999999999999999999999999999"
        )
        .is_lt());
    }
}
