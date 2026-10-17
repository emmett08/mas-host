use super::*;
use mas_protocol_codec::assessment::{Evaluator, Rubric};

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
    for table in ["revision_plan", "revision_terminal"] {
        db.execute(&format!("INSERT INTO {table} VALUES(1,'{{}}')"), [])
            .unwrap();
        assert!(db
            .execute(&format!("UPDATE {table} SET body='changed'"), [])
            .is_err());
        assert!(db.execute(&format!("DELETE FROM {table}"), []).is_err());
    }
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

#[test]
fn host_evidence_names_are_rejected_before_any_draft_dispatch() {
    let (path, _, engine) = engine_fixture();
    for name in [
        "candidate-0-output",
        "attempt-assessment-0-0",
        "budget-task-x",
    ] {
        let value = json!({"sources":[{"id":name,"kind":"input","content":{"private":true},"readers":["*"]}]});
        assert!(engine
            .revise(&json!({}), &value)
            .unwrap_err()
            .contains("host-reserved evidence prefix"));
    }
    let db = lock(&engine.inner.journal).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM steps", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    drop(engine);
    std::fs::remove_dir_all(path).unwrap();
}

fn assessment_fixture() -> (Candidate, Rubric, Evaluator, Evidence, Value) {
    let candidate = Candidate::from_value(&json!({"schema_version":1,"id":"candidate-0","output":{"ready":true},"output_sha256":canonical_digest(&json!({"ready":true})),"parent":null,"feedback":[]})).unwrap();
    let rubric = Rubric::from_value(&json!({"schema_version":1,"id":"r","criteria":[{"id":"safe","measurement":"model","minimum":0,"maximum":1}]})).unwrap();
    let evaluator =
        Evaluator::from_value(&json!({"id":"checker","version":"1","kind":"model"})).unwrap();
    let evidence = Evidence::from_value(&json!({"schema_version":1,"id":"candidate-0-output","content":candidate.output,"content_sha256":candidate.output_sha256})).unwrap();
    let result = json!({"status":"completed","findings":[{"criterion_id":"safe","verdict":"pass","score":1,"evidence":[evidence.reference().to_value()],"explanation":"checked exact output"}]});
    (candidate, rubric, evaluator, evidence, result)
}

#[test]
fn exact_evidence_and_frozen_authority_are_required_and_inconclusive_is_preserved() {
    let (candidate, rubric, evaluator, evidence, result) = assessment_fixture();
    assert_eq!(
        bind_assessment(
            "a",
            Ok(result.clone()),
            &candidate,
            &rubric,
            &evaluator,
            std::slice::from_ref(&evidence)
        )
        .status,
        AssessmentStatus::Completed
    );
    let mut changed = result.clone();
    changed["findings"][0]["evidence"][0]["sha256"] = json!("0".repeat(64));
    let mut authority = result.clone();
    authority["evaluator"] = evaluator.to_value();
    let mut out_of_range = result.clone();
    out_of_range["findings"][0]["score"] = json!(2);
    for invalid in [changed, authority, out_of_range] {
        let assessment = bind_assessment(
            "a",
            Ok(invalid),
            &candidate,
            &rubric,
            &evaluator,
            std::slice::from_ref(&evidence),
        );
        assert_eq!(assessment.status, AssessmentStatus::Error);
        assert!(assessment.findings.is_empty());
    }
    let mut inconclusive = result;
    inconclusive["status"] = json!("inconclusive");
    inconclusive["findings"][0]["verdict"] = json!("inconclusive");
    inconclusive["findings"][0]["score"] = Value::Null;
    let assessment = bind_assessment(
        "a",
        Ok(inconclusive),
        &candidate,
        &rubric,
        &evaluator,
        &[evidence],
    );
    assert_eq!(assessment.status, AssessmentStatus::Inconclusive);
    assert_eq!(assessment.findings[0].verdict, Verdict::Inconclusive);
}

fn precomplete(inner: &Inner, key: &str, context: &Value, output: &Value) {
    let id = inner.step_id("", key, "request");
    inner.bind_context(&id, context).unwrap();
    let step = inner
        .prepare("", key, "request", "client", "worker", &context["input"])
        .unwrap();
    let request = inner.message(
        format!("req-{id}"),
        "client",
        "worker",
        "Request",
        json!({"task_id":id,"operation":"assess","input":context["input"]}),
    );
    inner.admit_request(&step, &request).unwrap();
    lock(&inner.protocol)
        .unwrap()
        .receive(&inner.run_id, "worker", 100)
        .unwrap();
    inner
        .set_state(&id, "completed", Some(output), None)
        .unwrap();
}

#[test]
fn restart_reuses_terminal_error_assessment_without_reclassifying_completed_worker_output() {
    let (path, bundle, engine) = engine_fixture();
    let (candidate, rubric, evaluator, evidence, passing_output) = assessment_fixture();
    let plan = json!({"version":"mas-host-revision/1","draft_agent":"worker","revision_agent":"worker","max_revisions":0,"max_invocations":2,"draft_inputs":["incident"],"revision_inputs":["incident"],"allowed_agents":[],"feedback_evaluators":[],"rubric":rubric.to_value(),"evaluators":[{"actor":"worker","identity":evaluator.to_value(),"inputs":["incident"]}]});
    let original_input = input();
    let inner = &engine.inner;
    inner.freeze_revision(&plan, &original_input).unwrap();
    let draft = context::assemble(
        "worker",
        &["incident".into()],
        &original_input,
        None,
        &[],
        &[],
    )
    .unwrap();
    precomplete(inner, "draft", &draft, &candidate.output);
    let mut evaluation = context::assemble(
        "worker",
        &["incident".into()],
        &original_input,
        Some((&candidate.to_value(), &json!(["*"]))),
        &[],
        &[],
    )
    .unwrap();
    evaluation["input"]["evidence"]
        .as_array_mut()
        .unwrap()
        .push(evidence.to_value());
    let available = evaluation["input"]["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Evidence::from_value(v).unwrap())
        .collect::<Vec<_>>();
    evaluation["input"]["evidence_refs"] = json!(available
        .iter()
        .map(|e| e.reference().to_value())
        .collect::<Vec<_>>());
    evaluation["input"]["rubric"] = rubric.to_value();
    precomplete(inner, "assessment-0-0", &evaluation, &passing_output);
    // A transport failure after the worker checkpoint can produce a known error
    // assessment. An interrupted controller must retain that outcome on replay.
    let assessment = bind_assessment(
        "assessment-0-0",
        Err("terminal delivery refused".into()),
        &candidate,
        &rubric,
        &evaluator,
        &available,
    );
    inner
        .record("assessment", &assessment.to_value(), &json!(["*"]))
        .unwrap();
    drop(engine);
    let resumed = Engine::open(&bundle, &path, "test").unwrap();
    let result = resumed.revise(&plan, &original_input).unwrap();
    assert_eq!(result["revision"]["status"], "exhausted");
    assert_eq!(
        result["revision"]["assessments"].as_array().unwrap().len(),
        1
    );
    assert_eq!(result["revision"]["assessments"][0]["status"], "error");
    assert_eq!(result["revision"]["budget"]["used_invocations"], 2);
    assert_eq!(resumed.revise(&plan, &original_input).unwrap(), result);
    drop(resumed);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn retained_assessment_revalidates_every_binding_and_original_context() {
    let (path, _, engine) = engine_fixture();
    let (candidate, rubric, evaluator, evidence, result) = assessment_fixture();
    let available = vec![evidence.clone()];
    let context = context::assemble("worker", &[], &input(), None, &[], &[]).unwrap();
    let inner = &engine.inner;
    inner
        .freeze_revision(&json!({"max_invocations":1}), &input())
        .unwrap();
    inner
        .bind_context(&inner.step_id("", "a", "request"), &context)
        .unwrap();
    let assessment = bind_assessment("a", Ok(result), &candidate, &rubric, &evaluator, &available);
    inner
        .record("assessment", &assessment.to_value(), &context["readers"])
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            inner
                .retained_assessment("a", &context, &candidate, &rubric, &evaluator, &available)
                .unwrap(),
            Some(assessment.clone())
        );
    }
    let mut changed_candidate = candidate.clone();
    changed_candidate.output = json!({"ready":false});
    changed_candidate.output_sha256 = canonical_digest(&changed_candidate.output);
    assert!(inner
        .retained_assessment(
            "a",
            &context,
            &changed_candidate,
            &rubric,
            &evaluator,
            &available
        )
        .is_err());
    let mut changed_rubric = rubric.clone();
    changed_rubric.criteria[0].maximum = 2;
    assert!(inner
        .retained_assessment(
            "a",
            &context,
            &candidate,
            &changed_rubric,
            &evaluator,
            &available
        )
        .is_err());
    let mut changed_evaluator = evaluator.clone();
    changed_evaluator.version = "2".into();
    assert!(inner
        .retained_assessment(
            "a",
            &context,
            &candidate,
            &rubric,
            &changed_evaluator,
            &available
        )
        .is_err());
    let mut changed_evidence = evidence;
    changed_evidence.content = json!({"ready":false});
    changed_evidence.content_sha256 = canonical_digest(&changed_evidence.content);
    assert!(inner
        .retained_assessment(
            "a",
            &context,
            &candidate,
            &rubric,
            &evaluator,
            &[changed_evidence]
        )
        .is_err());
    let mut changed_context = context.clone();
    changed_context["policy"]["allowed_agents"] = json!(["worker"]);
    assert!(inner
        .retained_assessment(
            "a",
            &changed_context,
            &candidate,
            &rubric,
            &evaluator,
            &available
        )
        .is_err());
    changed_context = context.clone();
    changed_context["readers"] = json!(["worker"]);
    assert!(inner
        .retained_assessment(
            "a",
            &changed_context,
            &candidate,
            &rubric,
            &evaluator,
            &available
        )
        .is_err());
    let db = lock(&inner.journal).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM revision_contexts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    drop(engine);
    std::fs::remove_dir_all(path).unwrap();
}
