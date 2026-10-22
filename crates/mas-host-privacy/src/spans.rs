//! Validate and merge overlapping byte spans independently of detector implementations.

use crate::source::identifier;
use crate::{ByteRange, Error, Finding, Limits, Result, Source};

/// Merge only overlapping ranges. Adjacent findings retain distinct byte spans.
/// Categories and recogniser identities accumulate without taking a maximum score.
pub fn merge_findings(
    source: &Source,
    mut findings: Vec<Finding>,
    limits: Limits,
) -> Result<Vec<Finding>> {
    if findings.len() > limits.max_findings {
        return Err(Error::LimitExceeded);
    }
    for finding in &findings {
        ByteRange::new(source, finding.range.start, finding.range.end)?;
        if finding.categories.is_empty()
            || finding.recognisers.is_empty()
            || finding.recognisers.len() > 8
        {
            return Err(Error::InvalidRecord);
        }
        for recogniser in &finding.recognisers {
            identifier(recogniser)?;
        }
    }
    findings.sort_by_key(|finding| finding.range);
    let mut merged: Vec<Finding> = Vec::new();
    for finding in findings {
        if let Some(last) = merged.last_mut() {
            if finding.range.start < last.range.end {
                last.range.end = last.range.end.max(finding.range.end);
                last.categories.extend(finding.categories);
                last.recognisers.extend(finding.recognisers);
                if last.recognisers.len() > 8 {
                    return Err(Error::LimitExceeded);
                }
                continue;
            }
        }
        merged.push(finding);
    }
    Ok(merged)
}
