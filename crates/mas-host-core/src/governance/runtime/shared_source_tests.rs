//! Check independent subscriber authority against one retained source identity.
use super::{exposure_adversarial_tests::Fixture, read_governed_shared_source, GovernedSession};
use serde_json::json;
use std::fs;

fn readers(f: &mut Fixture, count: usize) {
    let expiry = f.authority["grants"][0]["expires_at_ms"].clone();
    for index in 0..count {
        let principal = format!("reader-{index}");
        f.authority["grants"].as_array_mut().unwrap().push(json!({"version":"mas-current-authority/1","status":"active","tenant":"tenant-a","principal":principal,"grant_ref":format!("grant-reader-{index}"),"grant_epoch":1,"policy_version":"policy-v1","valid_from_ms":0,"expires_at_ms":expiry}));
        for policy in f.authority["policies"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .filter(|p| p["tenant"] == "tenant-a")
        {
            policy["rules"].as_array_mut().unwrap().push(json!({"operation":"resolve","principal":principal,"actor":"worker","producer":"host","recipient":principal,"purpose":"diagnosis","destination":"client","location":"uk","classifications":["restricted"],"obligations":["inline-retention-approved"],"max_bytes":65536}));
        }
    }
    fs::write(f.path.join("authority.json"), f.authority.to_string()).unwrap();
}
fn reader(f: &Fixture, index: usize) -> GovernedSession {
    GovernedSession::from_value(
        &f.config,
        "tenant-a",
        &format!("reader-{index}"),
        "diagnosis",
        &format!("grant-reader-{index}"),
    )
    .unwrap()
}

#[test]
fn forty_independent_subscribers_share_one_retained_projection_without_new_acquisition() {
    let mut f = Fixture::new();
    readers(&mut f, 40);
    let owner = f.session();
    let engine = f.engine(owner.clone());
    let invocation = f.invocation(&engine);
    let effect = f.complete(&engine, &invocation, "source", "fixture-read/1");
    for index in 0..40 {
        let view = read_governed_shared_source(
            &f.directory(),
            &owner,
            &reader(&f, index),
            "worker",
            "source",
        )
        .unwrap();
        assert_eq!(view["effect_id"], effect);
        assert_eq!(view["projection"], json!({"n":2}));
    }
    drop(engine);
    // Restart retains the original effect; sharing performs no dispatch.
    let view = read_governed_shared_source(
        &f.directory(),
        &f.session(),
        &reader(&f, 39),
        "worker",
        "source",
    )
    .unwrap();
    assert_eq!(view["effect_id"], effect);
    f.authority["grants"][2]["status"] = json!("revoked");
    fs::write(f.path.join("authority.json"), f.authority.to_string()).unwrap();
    // Only revoked reader-0 is denied. The producer and reader-39 remain valid.
    assert!(read_governed_shared_source(
        &f.directory(),
        &owner,
        &reader(&f, 39),
        "worker",
        "source"
    )
    .is_ok());
}

#[test]
fn original_source_policy_and_current_subscriber_revocation_both_constrain_delivery() {
    let mut f = Fixture::new();
    readers(&mut f, 2);
    let owner = f.session();
    let first = reader(&f, 0);
    let second = reader(&f, 1);
    let engine = f.engine(owner.clone());
    let invocation = f.invocation(&engine);
    f.complete(&engine, &invocation, "source", "fixture-read/1");
    assert!(
        read_governed_shared_source(&f.directory(), &owner, &first, "worker", "source").is_ok()
    );
    f.authority["grants"][2]["status"] = json!("revoked");
    fs::write(f.path.join("authority.json"), f.authority.to_string()).unwrap();
    assert!(
        read_governed_shared_source(&f.directory(), &owner, &first, "worker", "source").is_err()
    );
    assert!(
        read_governed_shared_source(&f.directory(), &owner, &second, "worker", "source").is_ok()
    );
    // Source-specific policy is conjunctive: keeping a baseline permit cannot
    // grant access when the source association no longer allows this reader.
    f.authority["policies"][1]["rules"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["principal"] != "reader-1");
    fs::write(f.path.join("authority.json"), f.authority.to_string()).unwrap();
    assert!(
        read_governed_shared_source(&f.directory(), &owner, &second, "worker", "source").is_err()
    );
}
