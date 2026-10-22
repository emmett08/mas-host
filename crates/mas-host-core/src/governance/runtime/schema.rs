//! Project only explicitly bounded structured values, with no free-text release.
use serde_json::{Map, Value};
type Result<T> = std::result::Result<T, String>;

pub(super) fn check(value: &Value, depth: usize) -> Result<()> {
    if depth > 8 {
        return Err("governed-schema-depth".into());
    }
    match value["type"].as_str() {
        Some("object") => {
            let properties = value["properties"]
                .as_object()
                .filter(|p| p.len() <= 32)
                .ok_or("governed-schema-properties")?;
            if value["additionalProperties"] != false {
                return Err("governed-schema-open-object".into());
            }
            for (key, child) in properties {
                if key.is_empty() || key.len() > 128 || key.chars().any(char::is_control) {
                    return Err("governed-schema-property-name".into());
                }
                check(child, depth + 1)?;
            }
        }
        Some("string") => {
            let choices = value["enum"]
                .as_array()
                .filter(|v| !v.is_empty() && v.len() <= 64)
                .ok_or("governed-schema-free-text")?;
            if choices.iter().any(|v| {
                v.as_str()
                    .is_none_or(|s| s.len() > 128 || s.chars().any(char::is_control))
            }) {
                return Err("governed-schema-string-enum".into());
            }
        }
        Some("integer") => {
            if !value["minimum"].is_number()
                || !value["maximum"].is_number()
                || value["minimum"].to_string().len() > 128
                || value["maximum"].to_string().len() > 128
                || mas_host_schema::validate(value, &value["minimum"]).is_err()
                || mas_host_schema::validate(value, &value["maximum"]).is_err()
            {
                return Err("governed-schema-unbounded-integer".into());
            }
        }
        Some("array") => {
            if value["maxItems"].as_u64().is_none_or(|n| n > 32) {
                return Err("governed-schema-unbounded-array".into());
            }
            check(&value["items"], depth + 1)?;
        }
        Some("boolean" | "null") => {}
        _ => return Err("governed-schema-unsupported".into()),
    }
    if value.get("anyOf").is_some() {
        return Err("governed-schema-alternatives-unsupported".into());
    }
    mas_host_schema::check(value).map_err(|_| "governed-schema-invalid")?;
    Ok(())
}

pub(super) fn project(schema: &Value, value: &Value) -> Result<Value> {
    let projected = match schema["type"].as_str() {
        Some("object") => {
            let original = value.as_object().ok_or("governed-structured-type")?;
            let mut projected = Map::new();
            for (name, child_schema) in schema["properties"].as_object().unwrap() {
                if let Some(child) = original.get(name) {
                    projected.insert(name.clone(), project(child_schema, child)?);
                }
            }
            Value::Object(projected)
        }
        Some("array") => {
            let original = value.as_array().ok_or("governed-structured-type")?;
            if original.len() > schema["maxItems"].as_u64().unwrap() as usize {
                return Err("governed-structured-limit".into());
            }
            Value::Array(
                original
                    .iter()
                    .map(|child| project(&schema["items"], child))
                    .collect::<Result<Vec<_>>>()?,
            )
        }
        _ => {
            mas_host_schema::validate(schema, value).map_err(|_| "governed-structured-refused")?;
            value.clone()
        }
    };
    mas_host_schema::validate(schema, &projected).map_err(|_| "governed-structured-refused")?;
    if projected.to_string().len() > 16_384 {
        return Err("governed-structured-limit".into());
    }
    Ok(projected)
}
