//! Authoring format adapters. Every adapter yields the same strict MAS value.
//! Runtime bundles, journals and protocol frames continue to use the codec directly.
use mas_protocol_codec::{MAX_BYTES, MAX_INTEGER_DIGITS};
use serde_json::Value;
use std::{io::Read, path::Path};
mod toml;
mod yaml;

#[derive(Clone, Copy, Debug)]
pub enum Format {
    Json,
    Toml,
    Yaml,
}

impl Format {
    pub fn named(name: &str) -> Result<Self, String> {
        match name {
            "json" => Ok(Self::Json),
            "toml" => Ok(Self::Toml),
            "yaml" | "yml" => Ok(Self::Yaml),
            _ => Err(format!(
                "unsupported authoring format {name:?}; expected json, toml or yaml"
            )),
        }
    }

    fn for_path(path: &Path) -> Result<Self, String> {
        Self::named(path.extension().and_then(|s| s.to_str()).unwrap_or(""))
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

pub fn read(path: &Path) -> Result<Value, String> {
    read_input(path, None)
}

pub fn read_input(path: &Path, explicit: Option<&str>) -> Result<Value, String> {
    let stdin = path == Path::new("-");
    let format = match explicit {
        Some(name) => Format::named(name)?,
        None if stdin => Format::Json,
        None => Format::for_path(path)?,
    };
    let reader: Box<dyn Read> = if stdin {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?)
    };
    let mut text = String::new();
    reader
        .take((MAX_BYTES + 1) as u64)
        .read_to_string(&mut text)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text, format).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn parse(text: &str, format: Format) -> Result<Value, String> {
    if text.len() > MAX_BYTES {
        return Err(format!(
            "SIZE: authoring document exceeds {MAX_BYTES} bytes"
        ));
    }
    let value = match format {
        Format::Json => {
            return mas_protocol_codec::parse_json(text)
                .map_err(|e| format!("{}: {}", e.code, e.message))
        }
        Format::Toml => toml::decode(text)?,
        Format::Yaml => yaml::decode(text)?,
    };
    // Conversion may increase encoded length (escaping/base conversion). Reapply
    // the single protocol contract after conversion, before any build or launch.
    let canonical = serde_json::to_string(&value).map_err(|e| e.to_string())?;
    mas_protocol_codec::parse_json(&canonical).map_err(|e| format!("{}: {}", e.code, e.message))
}

fn integer(text: &str, radix: u32) -> Result<Value, String> {
    let unsigned = text
        .strip_prefix('-')
        .or_else(|| text.strip_prefix('+'))
        .unwrap_or(text);
    let significant = unsigned.trim_start_matches('0');
    // Conservative bounds reject impossible protocol integers before BigInt
    // conversion; the codec enforces the exact decimal-digit limit afterwards.
    let source_limit = match radix {
        2 => MAX_INTEGER_DIGITS * 4,
        8 => MAX_INTEGER_DIGITS * 2,
        _ => MAX_INTEGER_DIGITS,
    };
    if significant.len() > source_limit {
        return Err(format!(
            "INTEGER: integer exceeds {MAX_INTEGER_DIGITS} decimal digits"
        ));
    }
    let digits = if significant.is_empty() {
        "0"
    } else {
        significant
    };
    let normalised = if text.starts_with('-') {
        format!("-{digits}")
    } else {
        digits.to_owned()
    };
    let number = num_bigint::BigInt::parse_bytes(normalised.as_bytes(), radix)
        .ok_or_else(|| "invalid integer".to_string())?;
    let decimal = number.to_string();
    if decimal.trim_start_matches('-').len() > MAX_INTEGER_DIGITS {
        return Err(format!(
            "INTEGER: integer exceeds {MAX_INTEGER_DIGITS} decimal digits"
        ));
    }
    serde_json::from_str(&decimal).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
