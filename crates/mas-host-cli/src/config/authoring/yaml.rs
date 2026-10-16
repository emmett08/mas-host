//! YAML event adapter using the restricted Core scalar profile.
use super::integer;
use mas_protocol_codec::MAX_DEPTH;
use serde_json::{Map, Value};
use yaml_rust2::{
    parser::{Event, Parser},
    scanner::TScalarStyle,
};

enum Container {
    Array(Vec<Value>),
    Object(Map<String, Value>, Option<String>),
}

fn append(value: Value, stack: &mut [Container], root: &mut Option<Value>) -> Result<(), String> {
    match stack.last_mut() {
        Some(Container::Array(values)) => values.push(value),
        Some(Container::Object(values, key)) => {
            if let Some(key) = key.take() {
                values.insert(key, value);
            } else {
                let key_value = value.as_str().ok_or("YAML mapping keys must be strings")?;
                if key_value == "<<" {
                    return Err("YAML merge keys are not supported".into());
                }
                if values.contains_key(key_value) {
                    return Err(format!("duplicate YAML key {key_value:?}"));
                }
                *key = Some(key_value.to_owned());
            }
        }
        None => {
            if root.is_some() {
                return Err("expected one YAML document".into());
            }
            *root = Some(value);
        }
    }
    Ok(())
}

fn yaml_scalar(text: String, style: TScalarStyle) -> Result<Value, String> {
    if style != TScalarStyle::Plain {
        return Ok(Value::String(text));
    }
    match text.as_str() {
        "" | "~" | "null" | "Null" | "NULL" => return Ok(Value::Null),
        "true" | "True" | "TRUE" => return Ok(Value::Bool(true)),
        "false" | "False" | "FALSE" => return Ok(Value::Bool(false)),
        _ => {}
    }
    let unsigned = text
        .strip_prefix('-')
        .or_else(|| text.strip_prefix('+'))
        .unwrap_or(&text);
    if !unsigned.is_empty() && unsigned.bytes().all(|c| c.is_ascii_digit()) {
        return integer(&text, 10);
    }
    for (prefix, radix) in [("0x", 16), ("0o", 8)] {
        if let Some(digits) = text.strip_prefix(prefix) {
            if !digits.is_empty() && digits.chars().all(|c| c.is_digit(radix)) {
                return integer(digits, radix);
            }
        }
    }
    // YAML 1.2 Core floats are deliberately rejected, never rounded or silently
    // turned into strings. Strings with numeric spelling must be quoted.
    if matches!(
        unsigned,
        ".inf" | ".Inf" | ".INF" | ".nan" | ".NaN" | ".NAN"
    ) || ((unsigned.starts_with('.')
        || unsigned.as_bytes().first().is_some_and(u8::is_ascii_digit))
        && text.parse::<f64>().is_ok())
    {
        return Err(
            "floating-point values are outside the MAS integer-only contract; quote strings".into(),
        );
    }
    Ok(Value::String(text))
}

pub(super) fn decode(text: &str) -> Result<Value, String> {
    let mut parser = Parser::new_from_str(text);
    let mut stack = Vec::new();
    let mut root = None;
    let mut documents = 0;
    loop {
        // Pull events rather than using Parser::load's recursive receiver. This
        // allows rejection at the depth boundary before constructing children.
        let (event, mark) = parser.next_token().map_err(|e| e.to_string())?;
        let at = format!("line {}, column {}", mark.line(), mark.col() + 1);
        let sequence = matches!(event, Event::SequenceStart(..));
        let result = match event {
            Event::StreamEnd => break,
            Event::StreamStart | Event::DocumentEnd | Event::Nothing => Ok(()),
            Event::DocumentStart => {
                documents += 1;
                if documents > 1 {
                    Err("expected one YAML document".into())
                } else {
                    Ok(())
                }
            }
            Event::Alias(_) => Err("YAML aliases are not supported".into()),
            Event::Scalar(text, style, anchor, tag) => {
                if anchor != 0 || tag.is_some() {
                    Err("YAML anchors and tags are not supported".into())
                } else {
                    yaml_scalar(text, style).and_then(|v| append(v, &mut stack, &mut root))
                }
            }
            Event::SequenceStart(anchor, tag) | Event::MappingStart(anchor, tag) => {
                if anchor != 0 || tag.is_some() {
                    Err("YAML anchors and tags are not supported".into())
                } else if stack.len() >= MAX_DEPTH {
                    Err(format!(
                        "DEPTH: document exceeds {MAX_DEPTH} nesting levels"
                    ))
                } else if matches!(stack.last(), Some(Container::Object(_, None))) {
                    Err("YAML mapping keys must be strings".into())
                } else {
                    stack.push(if sequence {
                        Container::Array(Vec::new())
                    } else {
                        Container::Object(Map::new(), None)
                    });
                    Ok(())
                }
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let value = match stack.pop().ok_or("unbalanced YAML collection")? {
                    Container::Array(values) => Value::Array(values),
                    Container::Object(values, None) => Value::Object(values),
                    Container::Object(_, Some(_)) => {
                        return Err(format!("{at}: YAML key without value"))
                    }
                };
                append(value, &mut stack, &mut root)
            }
        };
        result.map_err(|e| format!("{at}: {e}"))?;
    }
    root.ok_or_else(|| "expected one YAML document".into())
}
