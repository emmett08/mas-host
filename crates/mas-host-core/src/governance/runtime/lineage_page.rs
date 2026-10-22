//! Resolve a complete immutable closure through bounded, currently authorised pages.
use super::{exposure::Exposure, resolver, GovernedSession};
use serde_json::{json, Value};
use std::path::Path;
type Result<T> = std::result::Result<T, String>;

pub(super) fn reference(session: &GovernedSession, exposure: &Exposure) -> Result<Value> {
    exposure.validate_for(session)?;
    let ids = exposure.sources.keys().cloned().collect::<Vec<_>>();
    Ok(
        json!({"version":"mas-governed-lineage-reference/1","closure_ref":session.identity_commitment(&json!(["mas-governed-lineage-reference/1",ids]))?,"source_count":ids.len(),"page_size":16}),
    )
}

/// The caller supplies an already authenticated owner and independent reader.
/// `owner == reader` is the owned-task case. A stale reference or incomplete
/// closure fails without returning a partial page or acquiring fresh evidence.
#[allow(clippy::too_many_arguments)]
pub fn read_governed_source_lineage_page(
    directory: &Path,
    owner: &GovernedSession,
    reader: &GovernedSession,
    actor: &str,
    key: &str,
    closure_ref: &str,
    offset: usize,
) -> Result<Value> {
    (||->Result<Value>{
        owner.compatible_source_reader(reader)?;
        if !owner.referenced_lineage_enabled(){return Err("governed-referenced-lineage-unsupported".into());}
        let query=json!({"version":"mas-governed-lineage-query/1","actor":actor,"key":key,"closure_ref":closure_ref,"offset":offset});
        owner.authorize_resolution(actor,&query,&query,true)?;
        let snapshot=resolver::snapshot(directory,owner,actor,None,key)?;
        let binding=snapshot.binding(None,key);
        let selected=&snapshot.source_exposure;
        let reference=reference(owner,selected)?;
        if reference["closure_ref"]!=closure_ref || offset>selected.sources.len(){return Err("governed-lineage-page-invalid".into());}
        let records=selected.sources.values().map(|s|s.metadata()).collect::<Vec<_>>();
        let view=bounded_page(&records,closure_ref,offset)?;
        owner.authorize_resolution_with_exposure(actor,&view,&view,false,&snapshot.exposure)?;
        reader.authorize_resolution_with_exposure(actor,&view,&view,false,&snapshot.exposure)?;
        reader.check_current()?;
        owner.check_current()?;
        binding.verify(directory,owner,actor)?;
        Ok(view)
    })().map_err(|_|"governed-lineage-page-unavailable".into())
}

fn bounded_page(records: &[Value], closure_ref: &str, offset: usize) -> Result<Value> {
    let count = records.len();
    let make = |start: usize, end: usize| json!({"version":"mas-governed-lineage-page/1","closure_ref":closure_ref,"source_count":count,"offset":start,"records":records[start..end],"next_offset":if end<count{Some(end)}else{None},"coverage":"complete-closure-resolved-page"});
    if count == 0 && offset == 0 {
        return Ok(make(0, 0));
    }
    let mut start = 0;
    while start < count {
        let mut end = start;
        while end < count && end - start < 16 {
            if super::encoding::canonical(&make(start, end + 1), 65_536).is_err() {
                break;
            }
            end += 1;
        }
        if end == start {
            return Err("governed-lineage-record-limit".into());
        }
        if start == offset {
            return Ok(make(start, end));
        }
        start = end;
    }
    Err("governed-lineage-page-offset".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dense_metadata_uses_exact_byte_bounded_page_boundaries() {
        let records = (0..8)
            .map(|i| json!({"id":i,"metadata":"x".repeat(30_000)}))
            .collect::<Vec<_>>();
        let mut offset = 0;
        let mut seen = Vec::new();
        loop {
            let page = bounded_page(&records, "closure-fixture", offset).unwrap();
            assert!(super::super::encoding::canonical(&page, 65_536).is_ok());
            assert_eq!(page["records"].as_array().unwrap().len(), 2);
            seen.extend(page["records"].as_array().unwrap().iter().cloned());
            match page["next_offset"].as_u64() {
                Some(next) => offset = next as usize,
                None => break,
            }
        }
        assert_eq!(seen, records);
        assert!(bounded_page(&records, "closure-fixture", 1).is_err());
    }
}
