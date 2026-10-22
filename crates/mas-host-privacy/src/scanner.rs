//! Scan bounded synthetic ASCII patterns while reporting neural coverage as unqualified.

use crate::spans::merge_findings;
use crate::{Assessment, ByteRange, Category, Completion, Error, Finding, Limits, Result, Source};
use std::collections::BTreeSet;

/// Deterministic synthetic recognisers provide no neural or population-quality claim.
/// Even with full pattern coverage, the combined privacy assessment stays partial.
pub fn scan_synthetic(source: &Source, limits: Limits) -> Result<Assessment> {
    if source.content().len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge);
    }
    let mut findings = Vec::new();
    let bytes = source.content().as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if is_email_byte(bytes[cursor]) {
            let start = cursor;
            while cursor < bytes.len() && is_email_byte(bytes[cursor]) {
                cursor += 1;
            }
            let candidate = &source.content()[start..cursor];
            if email(candidate) {
                push(
                    &mut findings,
                    Finding::new(
                        ByteRange::new(source, start, cursor)?,
                        Category::EmailAddress,
                        "synthetic-email/1",
                    )?,
                    limits,
                )?;
            }
        } else {
            cursor += 1;
        }
    }
    for prefix in ["sk_test_", "synthetic_secret_"] {
        let mut cursor = 0;
        while let Some(relative) = source.content()[cursor..].find(prefix) {
            let start = cursor + relative;
            let mut end = start + prefix.len();
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || b"_-".contains(&bytes[end]))
            {
                end += 1;
            }
            if end > start + prefix.len() {
                push(
                    &mut findings,
                    Finding::new(
                        ByteRange::new(source, start, end)?,
                        Category::Credential,
                        "synthetic-credential/1",
                    )?,
                    limits,
                )?;
            }
            cursor = end;
        }
    }
    let checked = if bytes.is_empty() {
        vec![]
    } else {
        vec![ByteRange::new(source, 0, bytes.len())?]
    };
    Assessment::new(
        source,
        Completion::Partial,
        merge_findings(source, findings, limits)?,
        checked,
        BTreeSet::from(["neural-unqualified".into()]),
        limits,
    )
}

fn push(findings: &mut Vec<Finding>, finding: Finding, limits: Limits) -> Result<()> {
    if findings.len() == limits.max_findings {
        return Err(Error::LimitExceeded);
    }
    findings.push(finding);
    Ok(())
}

fn is_email_byte(value: u8) -> bool {
    value.is_ascii_alphanumeric() || b"._%+-@".contains(&value)
}

fn email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && domain.split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}
