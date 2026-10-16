//! TOML syntax adapter with source spans and exact integer conversion.
use super::integer;
use ::toml::de::{DeTable, DeValue};
use mas_protocol_codec::MAX_DEPTH;
use serde_json::Value;

pub(super) fn decode(text: &str) -> Result<Value, String> {
    // The maintained parser applies its own bounded recursion guard before
    // construction. Conversion additionally enforces MAS's tighter limit.
    let document = DeTable::parse(text).map_err(|e| e.to_string())?;
    toml_value(DeValue::Table(document.into_inner()), 0, text)
}

fn location(text: &str, offset: usize) -> String {
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
    let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    format!("line {line}, column {column}")
}

fn toml_value(value: DeValue<'_>, depth: usize, source: &str) -> Result<Value, String> {
    if matches!(value, DeValue::Table(_) | DeValue::Array(_)) && depth >= MAX_DEPTH {
        return Err(format!(
            "DEPTH: document exceeds {MAX_DEPTH} nesting levels"
        ));
    }
    Ok(match value {
        DeValue::String(s) => Value::String(s.into_owned()),
        DeValue::Integer(n) => integer(n.as_str(), n.radix())?,
        DeValue::Boolean(b) => Value::Bool(b),
        DeValue::Float(_) => {
            return Err("floating-point values are outside the MAS integer-only contract".into())
        }
        DeValue::Datetime(_) => {
            return Err("TOML dates/times require an explicitly quoted string".into())
        }
        DeValue::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|v| {
                    let at = location(source, v.span().start);
                    toml_value(v.into_inner(), depth + 1, source).map_err(|e| format!("{at}: {e}"))
                })
                .collect::<Result<_, _>>()?,
        ),
        DeValue::Table(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let at = location(source, value.span().start);
                    toml_value(value.into_inner(), depth + 1, source)
                        .map(|v| (key.into_inner().into_owned(), v))
                        .map_err(|e| format!("{at}: {e}"))
                })
                .collect::<Result<_, _>>()?,
        ),
    })
}
