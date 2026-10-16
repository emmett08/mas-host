use super::*;

fn input() -> Value {
    json!({"sources":[
        {"id":"incident","kind":"evidence","content":{"log":"crash"},"readers":["*"]},
        {"id":"peer","kind":"peer_history","content":{"finding":"secret A"},"readers":["reviser"]}
    ]})
}
#[test]
fn withheld_peer_content_does_not_affect_independent_input_or_policy() {
    let first = input();
    let mut second = first.clone();
    second["sources"][1]["content"] = json!({"finding":"secret B"});
    let ids = vec!["incident".into()];
    assert_eq!(
        context::assemble("reviewer", &ids, &first, None, &[], &[]).unwrap(),
        context::assemble("reviewer", &ids, &second, None, &[], &[]).unwrap()
    );
    assert!(context::assemble("reviewer", &["peer".into()], &first, None, &[], &[]).is_err());
}
#[test]
fn derived_output_intersects_restrictions_and_feedback_is_filtered() {
    let value = json!({"id":"assessment"});
    let restricted = json!(["reviser"]);
    let permitted = context::assemble(
        "reviser",
        &["incident".into()],
        &input(),
        None,
        &[(&value, &restricted)],
        &[],
    )
    .unwrap();
    assert_eq!(permitted["readers"], json!(["reviser"]));
    assert_eq!(permitted["input"]["feedback"], json!([value]));
    let withheld = context::assemble(
        "reviewer",
        &["incident".into()],
        &input(),
        None,
        &[(&value, &restricted)],
        &[],
    )
    .unwrap();
    assert_eq!(withheld["input"]["feedback"], json!([]));
    assert_eq!(withheld["readers"], json!(["*"]));
}
#[test]
fn audit_records_and_contexts_cannot_be_rewritten_or_deleted() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    store::create(&db).unwrap();
    db.execute(
        "INSERT INTO revision_records VALUES('assessment','a','{}','[]')",
        [],
    )
    .unwrap();
    assert!(db
        .execute("UPDATE revision_records SET body='changed'", [])
        .is_err());
    assert!(db.execute("DELETE FROM revision_records", []).is_err());
    db.execute("INSERT INTO revision_contexts VALUES('i','{}')", [])
        .unwrap();
    assert!(db
        .execute("UPDATE revision_contexts SET body='changed'", [])
        .is_err());
    assert!(db.execute("DELETE FROM revision_contexts", []).is_err());
}
#[test]
fn evaluator_failure_and_forged_identity_cannot_pass() {
    let candidate=Candidate::from_value(&json!({"schema_version":1,"id":"candidate-0","output":{},"output_sha256":canonical_digest(&json!({})),"parent":null,"feedback":[]})).unwrap();
    let rubric=Rubric::from_value(&json!({"schema_version":1,"id":"r","criteria":[{"id":"safe","measurement":"deterministic","minimum":0,"maximum":1}]})).unwrap();
    let evaluator =
        Evaluator::from_value(&json!({"id":"checker","version":"1","kind":"deterministic"}))
            .unwrap();
    for result in [
        Err("worker failed".into()),
        Ok(json!({"status":"completed","findings":[],"evaluator":{"id":"forged"}})),
        Ok(json!({"status":"completed","findings":[]})),
    ] {
        let assessment = bind_assessment("a", result, &candidate, &rubric, &evaluator, &[]);
        assert_eq!(assessment.status, AssessmentStatus::Error);
        assert!(assessment.findings.is_empty());
        assert_eq!(assessment.evaluator, evaluator);
    }
}

fn engine_fixture() -> (std::path::PathBuf, Value, Engine) {
    let path = std::env::temp_dir().join(format!(
        "revision-unit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    let executable = std::fs::canonicalize("/usr/bin/true").unwrap();
    let bundle = json!({"version":"mas-bundle/3","agents":[{"name":"worker","executable":executable,"sha256":super::super::digest(&std::fs::read(&executable).unwrap()),"operation":"assess","environment":[],"working_directory":path,"configuration":{"kind":"llm"}}],"topology":{"agents":["client","worker"],"edges":[["client","worker"],["worker","client"]],"application":"agent-task/1","max_calls":2,"wall_seconds":60}});
    let engine = Engine::open(&bundle, &path, "test").unwrap();
    (path, bundle, engine)
}

#[test]
fn invocation_identity_is_reserved_once_and_audit_conflicts_are_rejected() {
    let (path, _, engine) = engine_fixture();
    let inner = &engine.inner;
    inner
        .freeze_revision(&json!({"max_invocations":1}), &input())
        .unwrap();
    let context =
        context::assemble("worker", &["incident".into()], &input(), None, &[], &[]).unwrap();
    inner.bind_context("i", &context).unwrap();
    inner.bind_context("i", &context).unwrap();
    let mut changed = context.clone();
    changed["input"]["new"] = json!(true);
    assert!(inner.bind_context("i", &changed).is_err());
    assert!(inner.bind_context("j", &context).is_err());
    let record = json!({"id":"a","score":0});
    inner.record("assessment", &record, &json!(["*"])).unwrap();
    inner.record("assessment", &record, &json!(["*"])).unwrap();
    assert!(inner
        .record("assessment", &json!({"id":"a","score":1}), &json!(["*"]))
        .is_err());
    let db = lock(&inner.journal).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM revision_contexts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    drop(db);
    assert!(engine
        .run("worker", &json!({}))
        .unwrap_err()
        .contains("revision plan"));
    drop(engine);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn scoped_inference_unknown_effect_remains_blocked_after_restart() {
    let (path, bundle, engine) = engine_fixture();
    let inner = &engine.inner;
    inner
        .freeze_revision(&json!({"max_invocations":2}), &input())
        .unwrap();
    let context =
        context::assemble("worker", &["incident".into()], &input(), None, &[], &[]).unwrap();
    let id = inner.step_id("", "draft", "request");
    inner.bind_context(&id, &context).unwrap();
    let invocation = inner
        .prepare(
            "",
            "draft",
            "request",
            "client",
            "worker",
            &context["input"],
        )
        .unwrap();
    assert!(inner
        .begin_effect(&invocation, "attack", &json!({"format":"remediation/1"}))
        .is_err());
    let descriptor = json!({"format":"mas-inference/2","configuration_sha256":"frozen","input":context["input"]});
    assert_eq!(
        inner
            .begin_effect(&invocation, "model", &descriptor)
            .unwrap()["state"],
        "execute"
    );
    drop(engine);
    let resumed = Engine::open(&bundle, &path, "test").unwrap();
    assert!(resumed.inner.has_unknown().unwrap());
    assert!(resumed
        .inner
        .begin_effect(&invocation, "model", &descriptor)
        .unwrap_err()
        .contains("uncertain"));
    drop(resumed);
    std::fs::remove_dir_all(path).unwrap();
}
