//! Qualify policy conjunction and exact pages beyond the inline source budget.
use super::{
    exposure_adversarial_tests::Fixture, read_governed_root_source,
    read_governed_source_lineage_page,
};
use crate::lock;
use serde_json::json;

fn exercise(count: usize) {
    let mut f = Fixture::new();
    f.config["lineage"] = json!({"version":"mas-governed-referenced-lineage/1","max_sources":128,"max_metadata_bytes":262_144});
    let session = f.session();
    let engine = f.engine(session.clone());
    let parent = f.invocation(&engine);
    // Populate identified retained source fixtures through the real source
    // owner, then perform the final protected acquisition through effect APIs.
    // This isolates large-closure qualification from 126 unrelated executions.
    {
        let db = lock(&engine.inner.journal).unwrap();
        let mut context = super::exposure::load_invocation(&db, &session, &parent.id).unwrap();
        for index in 0..count - 2 {
            let id = format!(
                "task-{}",
                crate::digest(format!("retained-seed-{index}").as_bytes())
            );
            db.execute("INSERT INTO steps VALUES(?, '', ?, 'request', 'client','worker','{}','completed',?,NULL)",rusqlite::params![id,format!("seed-{index}"),json!({"n":1}).to_string()]).unwrap();
            context = super::exposure::create_source(
                &db,
                &session,
                &id,
                "request-output",
                "worker",
                &json!({"n":1}),
                &context,
                &std::collections::BTreeSet::new(),
                true,
                f.authority["grants"][0]["expires_at_ms"].as_u64().unwrap(),
            )
            .unwrap();
        }
        super::exposure::save_invocation(&db, &session, &parent.id, &context).unwrap();
    }
    f.complete(&engine, &parent, "restricted-final", "fixture-read/1");
    let view =
        read_governed_root_source(&f.directory(), &session, "worker", "restricted-final").unwrap();
    assert_eq!(view["version"], "mas-governed-source-view/2");
    assert_eq!(view["lineage_disposition"], "referenced");
    assert_eq!(view["lineage_reference"]["source_count"], count);
    assert_eq!(view["lineage"], json!([]));
    assert!(serde_json::to_vec(&view).unwrap().len() <= 65_536);
    let closure = view["lineage_reference"]["closure_ref"].as_str().unwrap();
    let mut records = Vec::new();
    let mut offset = 0;
    loop {
        let page = read_governed_source_lineage_page(
            &f.directory(),
            &session,
            &session,
            "worker",
            "restricted-final",
            closure,
            offset,
        )
        .unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() <= 65_536);
        records.extend(page["records"].as_array().unwrap().iter().cloned());
        match page["next_offset"].as_u64() {
            Some(next) => offset = next as usize,
            None => break,
        }
    }
    assert_eq!(records.len(), count);
    assert_eq!(
        records
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        count
    );
    assert!(read_governed_source_lineage_page(
        &f.directory(),
        &session,
        &session,
        "worker",
        "restricted-final",
        &"0".repeat(64),
        0
    )
    .is_err());
    assert!(engine
        .inner
        .begin_effect(
            &parent,
            "denied-action",
            &json!({"format":"fixture-action/1","n":1})
        )
        .is_err());
    // Corrupting an intermediate ancestor prevents an entire page; no partial
    // prefix is returned as a completely checked closure.
    let id = records[count / 2]["id"].as_str().unwrap();
    let old: String = lock(&engine.inner.journal)
        .unwrap()
        .query_row("SELECT body FROM governed_sources WHERE id=?", [id], |r| {
            r.get(0)
        })
        .unwrap();
    lock(&engine.inner.journal)
        .unwrap()
        .execute("DELETE FROM governed_sources WHERE id=?", [id])
        .unwrap();
    assert!(read_governed_source_lineage_page(
        &f.directory(),
        &session,
        &session,
        "worker",
        "restricted-final",
        closure,
        0
    )
    .is_err());
    lock(&engine.inner.journal)
        .unwrap()
        .execute(
            "INSERT INTO governed_sources VALUES(?,?)",
            rusqlite::params![id, old],
        )
        .unwrap();
    let mut revoked = f.authority.clone();
    revoked["grants"][0]["status"] = json!("revoked");
    f.write_authority(&revoked);
    assert!(read_governed_source_lineage_page(
        &f.directory(),
        &session,
        &session,
        "worker",
        "restricted-final",
        closure,
        0
    )
    .is_err());
    drop(engine);
}
#[test]
fn sixty_five_sources_preserve_last_source_restriction_and_exact_ancestry() {
    exercise(65);
}
#[test]
fn one_hundred_twenty_eight_sources_preserve_last_source_restriction_and_exact_ancestry() {
    exercise(128);
}

fn context_fixture(f: &Fixture, count: usize) -> (crate::Engine, super::exposure::Exposure) {
    let session = f.session();
    let engine = f.engine(session.clone());
    let parent = f.invocation(&engine);
    let context = {
        let db = lock(&engine.inner.journal).unwrap();
        let mut context = super::exposure::load_invocation(&db, &session, &parent.id).unwrap();
        for index in 0..count - 1 {
            let id = format!(
                "task-{}",
                crate::digest(format!("byte-seed-{index}").as_bytes())
            );
            db.execute("INSERT INTO steps VALUES(?,'',?,'request','client','worker','{}','completed',?,NULL)",rusqlite::params![id,format!("seed-{index}"),json!({"n":1}).to_string()]).unwrap();
            context = super::exposure::create_source(
                &db,
                &session,
                &id,
                "request-output",
                "worker",
                &json!({"n":1}),
                &context,
                &std::collections::BTreeSet::new(),
                true,
                f.authority["grants"][0]["expires_at_ms"].as_u64().unwrap(),
            )
            .unwrap();
        }
        context
    };
    (engine, context)
}
fn config_lineage(f: &mut Fixture) {
    f.config["lineage"] = json!({"version":"mas-governed-referenced-lineage/1","max_sources":128,"max_metadata_bytes":262144});
}
fn receipt(f: &Fixture, context: &super::exposure::Exposure) -> Result<serde_json::Value, String> {
    // This fixture isolates representation byte accounting. It supplies a
    // fixed synthetic inspection receipt, not a detector-quality assertion.
    f.session().authorize_inspected_with_exposure(
        "worker",
        "invoke",
        "worker:worker",
        "local-worker",
        &json!({"n":1}),
        &json!({"n":1}),
        Some(json!({"version":"fixture-inspection/1"})),
        context,
    )
}

#[test]
fn compact_receipt_counts_actual_encoded_bytes_at_exact_policy_boundary() {
    let mut f = Fixture::new();
    config_lineage(&mut f);
    let (_engine, context) = context_fixture(&f, 65);
    let record = receipt(&f, &context).unwrap();
    let bytes = serde_json::to_vec(&json!([{"n":1},record])).unwrap().len();
    assert_eq!(record["disclosure"]["max_bytes"], bytes);
    assert_eq!(
        record["disclosure"]["closure_reference"]["source_count"],
        65
    );
    assert_eq!(
        record["disclosure"]["policy_bindings"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for maximum in [bytes - 1, bytes] {
        let mut bounded = Fixture::new();
        config_lineage(&mut bounded);
        for policy in bounded.authority["policies"].as_array_mut().unwrap() {
            for rule in policy["rules"].as_array_mut().unwrap() {
                rule["max_bytes"] = json!(maximum);
            }
        }
        bounded.write_authority(&bounded.authority);
        let (_engine, context) = context_fixture(&bounded, 65);
        let result = receipt(&bounded, &context);
        assert_eq!(
            result.is_ok(),
            maximum == bytes,
            "exact retained representation bound {maximum}"
        );
        if let Ok(record) = result {
            assert_eq!(
                serde_json::to_vec(&json!([{"n":1},record])).unwrap().len(),
                maximum
            );
        }
    }
}

#[test]
fn repeated_policy_conjunction_keeps_all_matching_obligations_and_source_intervals() {
    let mut f = Fixture::new();
    config_lineage(&mut f);
    let (_engine, context) = context_fixture(&f, 65);
    assert!(receipt(&f, &context).is_ok());
    let now = crate::now_ms().unwrap();
    for (field, value) in [("expires_at_ms", now - 1), ("acquired_at_ms", now + 60_000)] {
        let mut invalid = context.clone();
        invalid.sources.values_mut().next().unwrap().body[field] = json!(value);
        assert!(
            receipt(&f, &invalid).is_err(),
            "every source {field} checked before projection"
        );
    }
    let mut obligations = Fixture::new();
    config_lineage(&mut obligations);
    for policy in obligations.authority["policies"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .filter(|p| p["tenant"] == "tenant-a")
    {
        let mut duplicate = policy["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| {
                r["operation"] == "invoke"
                    && r["actor"] == "worker"
                    && r["destination"] == "local-worker"
            })
            .unwrap()
            .clone();
        duplicate["obligations"] =
            json!(["inline-retention-approved", "extra-required-obligation"]);
        policy["rules"].as_array_mut().unwrap().push(duplicate);
    }
    obligations.write_authority(&obligations.authority);
    let session = obligations.session();
    // Reuse a complete already verified in-memory closure with the same sealed
    // source policy IDs. The exact query varies only its matched obligations.
    assert!(session
        .authorize_inspected_with_exposure(
            "worker",
            "invoke",
            "worker:worker",
            "local-worker",
            &json!({"n":1}),
            &json!({"n":1}),
            Some(json!({"version":"fixture-inspection/1"})),
            &context
        )
        .is_err());
}
