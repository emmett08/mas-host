//! Exercise retirement against actual governed journals and a restored backup.
use super::*;
use crate::governance::govern_namespace_lifecycle;

fn fixture() -> Fixture {
    let mut f = Fixture::new();
    f.config["version"] = json!("mas-governed-local/2");
    f.config["baseline_policy_refs"] = json!(["policy-a"]);
    f.config["lifecycle_file"] = json!(f.path.join("lifecycle.sqlite"));
    f
}

#[test]
fn retirement_preserves_effect_certainty_and_refuses_restored_payloads() {
    let f = fixture();
    let session = f.session();
    let engine = f.engine();
    let invocation = f.invocation(&engine);
    let admission = engine
        .inner
        .begin_effect(&invocation, "effect", &json!({"format":"fixture/1","n":1}))
        .unwrap();
    let effect_id = admission["invocation_id"].as_str().unwrap().to_owned();
    engine
        .inner
        .complete_effect(
            &invocation,
            "effect",
            &json!({"n":2}),
            &json!({"execution":"completed"}),
        )
        .unwrap();
    {
        let db = lock(&engine.inner.journal).unwrap();
        db.execute(
            "UPDATE steps SET input=? WHERE id=?",
            rusqlite::params![json!({"private":PRIVATE}).to_string(), invocation.id],
        )
        .unwrap();
    }
    drop(engine);
    let directory = f.path.join("governed-run");
    let before = fs::read(directory.join("execution.sqlite")).unwrap();
    let result = govern_namespace_lifecycle(&directory, &session, "retire", None).unwrap();
    assert_eq!(result["payload_disposition"], "deleted");
    assert!(!directory.join("protocol.sqlite").exists());
    let db = rusqlite::Connection::open(directory.join("execution.sqlite")).unwrap();
    let (state, certainty): (String, String) = db
        .query_row(
            "SELECT state,external_execution FROM governed_retired_effects WHERE id=?",
            [&effect_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (state.as_str(), certainty.as_str()),
        ("completed", "completed")
    );
    drop(db);
    assert!(!fs::read(directory.join("execution.sqlite"))
        .unwrap()
        .windows(PRIVATE.len())
        .any(|w| w == PRIVATE.as_bytes()));
    assert!(Engine::open_governed(&f.bundle, &f.path, "governed-run", session.clone()).is_err());
    // An old backup contains its payloads, but the independently retained
    // retirement record still prevents both read and execution admission.
    fs::write(directory.join("execution.sqlite"), before).unwrap();
    assert!(Engine::open_governed(&f.bundle, &f.path, "governed-run", session.clone()).is_err());
    assert!(
        crate::governance::read_governed_root_source(&directory, &session, "worker", "effect")
            .is_err()
    );
    // Removing the control ledger cannot silently recreate availability.
    fs::remove_file(f.path.join("lifecycle.sqlite")).unwrap();
    assert!(Engine::open_governed(&f.bundle, &f.path, "governed-run", session).is_err());
}

#[test]
fn hold_expiry_and_ownership_are_independent_of_execution_permission() {
    let f = fixture();
    let engine = f.engine();
    let directory = f.path.join("governed-run");
    assert!(govern_namespace_lifecycle(&directory, &f.session(), "retire", None).is_err());
    drop(engine);
    govern_namespace_lifecycle(&directory, &f.session(), "hold", None).unwrap();
    assert_eq!(
        govern_namespace_lifecycle(&directory, &f.session(), "retire", None).unwrap_err(),
        "governed-lifecycle-held"
    );
    govern_namespace_lifecycle(&directory, &f.session(), "expire", Some(1)).unwrap();
    assert!(Engine::open_governed(&f.bundle, &f.path, "governed-run", f.session()).is_err());
    assert!(
        govern_namespace_lifecycle(&directory, &f.session(), "expire", Some(u64::MAX)).is_err()
    );
    govern_namespace_lifecycle(&directory, &f.session(), "release-hold", None).unwrap();
    govern_namespace_lifecycle(&directory, &f.session(), "retire", None).unwrap();
    govern_namespace_lifecycle(&directory, &f.session(), "retire", None).unwrap();
}

#[test]
fn replacing_a_retired_namespace_cannot_register_a_fresh_identity_epoch() {
    let f = fixture();
    drop(f.engine());
    let directory = f.path.join("governed-run");
    govern_namespace_lifecycle(&directory, &f.session(), "retire", None).unwrap();
    fs::remove_dir_all(&directory).unwrap();
    assert!(Engine::open_governed(&f.bundle, &f.path, "governed-run", f.session()).is_err());
}

fn credential_file(f: &mut Fixture, epoch: u64, value: &str) {
    f.config["credentials_file"] = json!(f.path.join("credentials.json"));
    let record = json!({"tenant":"tenant-a","principal":"principal-a","grant_ref":"grant-a","grant_epoch":1,"actor":"worker","destination":"local-worker","reference":"PROVIDER_TOKEN","credential_epoch":epoch,"expires_at_ms":crate::now_ms().unwrap()+60_000,"value":value});
    fs::write(
        f.path.join("credentials.json"),
        json!({"version":"mas-governed-credentials/1","credentials":[record]}).to_string(),
    )
    .unwrap();
    fs::set_permissions(
        f.path.join("credentials.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
}

#[test]
fn current_credentials_rotate_without_changing_namespace_identity_and_reject_rollback() {
    let mut f = fixture();
    credential_file(&mut f, 1, "credential-first-private");
    let allowed = ["PROVIDER_TOKEN".to_owned()].into_iter().collect();
    let session = f.session();
    let fingerprint = session.fingerprint();
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let first = session
        .credential_environment("worker", &allowed)
        .unwrap()
        .unwrap();
    first.admit(&db, "worker").unwrap();
    assert_eq!(first.values["PROVIDER_TOKEN"], "credential-first-private");
    credential_file(&mut f, 2, "credential-second-private");
    assert_eq!(f.session().fingerprint(), fingerprint);
    let second = session
        .credential_environment("worker", &allowed)
        .unwrap()
        .unwrap();
    second.admit(&db, "worker").unwrap();
    assert_eq!(second.values["PROVIDER_TOKEN"], "credential-second-private");
    credential_file(&mut f, 1, "credential-first-private");
    assert!(session
        .credential_environment("worker", &allowed)
        .unwrap()
        .unwrap()
        .admit(&db, "worker")
        .is_err());
    credential_file(&mut f, 2, "same-epoch-changed-secret");
    assert!(session
        .credential_environment("worker", &allowed)
        .unwrap()
        .unwrap()
        .admit(&db, "worker")
        .is_err());
    let bindings: String = db
        .query_row("SELECT binding FROM governed_credential_epochs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(!bindings.contains("credential"));
}

#[test]
fn credential_scope_missing_secret_and_grant_epoch_revoke_launch() {
    let mut f = fixture();
    credential_file(&mut f, 1, "synthetic-private-secret");
    let allowed = ["PROVIDER_TOKEN".to_owned()].into_iter().collect();
    let session = f.session();
    assert!(session.credential_environment("other", &allowed).is_err());
    let extra = ["PROVIDER_TOKEN".to_owned(), "UNAPPROVED_TOKEN".to_owned()]
        .into_iter()
        .collect();
    assert!(session.credential_environment("worker", &extra).is_err());
    fs::set_permissions(
        f.path.join("credentials.json"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(session.credential_environment("worker", &allowed).is_err());
    fs::set_permissions(
        f.path.join("credentials.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    f.authority["grants"][0]["grant_epoch"] = json!(2);
    f.update(&f.authority);
    assert!(session.credential_environment("worker", &allowed).is_err());
    assert!(f
        .session()
        .credential_environment("worker", &allowed)
        .is_err());
}

#[test]
fn unresolved_external_obligation_stops_destructive_retirement() {
    let f = fixture();
    let session = f.session();
    let engine = f.engine();
    let parent = f.invocation(&engine);
    let step = engine
        .inner
        .prepare(
            &parent.id,
            "pending",
            "effect",
            "worker",
            "",
            &json!({"format":"fixture/1","n":1}),
        )
        .unwrap();
    lock(&engine.inner.journal)
        .unwrap()
        .execute("UPDATE steps SET state='unknown' WHERE id=?", [&step.id])
        .unwrap();
    drop(engine);
    let directory = f.path.join("governed-run");
    assert_eq!(
        govern_namespace_lifecycle(&directory, &session, "retire", None).unwrap_err(),
        "governed-lifecycle-reconciliation-required"
    );
    let db = rusqlite::Connection::open(directory.join("execution.sqlite")).unwrap();
    let retained: String = db
        .query_row("SELECT input FROM steps WHERE id=?", [&step.id], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(retained.contains("fixture/1"));
}
