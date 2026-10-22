//! Read closed, bounded privacy wire objects without converting errors to content.

use crate::{source::identifier, Error, Result};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub(crate) fn object<'a>(value: &'a Value, fields: &[&str]) -> Result<&'a Map<String, Value>> {
    let record = value.as_object().ok_or(Error::InvalidRecord)?;
    if record.len() != fields.len() || fields.iter().any(|key| !record.contains_key(*key)) {
        return Err(Error::InvalidRecord);
    }
    Ok(record)
}

pub(crate) fn string<'a>(record: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    record[key].as_str().ok_or(Error::InvalidRecord)
}

pub(crate) fn number(record: &Map<String, Value>, key: &str) -> Result<usize> {
    record[key]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or(Error::InvalidRecord)
}

pub(crate) fn array<'a>(
    record: &'a Map<String, Value>,
    key: &str,
    maximum: usize,
) -> Result<&'a [Value]> {
    let values = record[key].as_array().ok_or(Error::InvalidRecord)?;
    if values.len() > maximum {
        return Err(Error::LimitExceeded);
    }
    Ok(values)
}

pub(crate) fn string_set(
    record: &Map<String, Value>,
    key: &str,
    maximum: usize,
) -> Result<BTreeSet<String>> {
    let values = array(record, key, maximum)?;
    let mut result = BTreeSet::new();
    for value in values {
        let value = value.as_str().ok_or(Error::InvalidRecord)?;
        identifier(value)?;
        if !result.insert(value.to_string()) {
            return Err(Error::InvalidRecord);
        }
    }
    Ok(result)
}
