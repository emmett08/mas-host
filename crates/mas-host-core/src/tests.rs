use super::*;
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    path: PathBuf,
    bundle: Value,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mas-host-core-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let executable = std::fs::canonicalize("/usr/bin/true").unwrap();
        let hash = digest(&std::fs::read(&executable).unwrap());
        let bundle = json!({"version":"mas-bundle/3","agents":[
            {"name":"worker","executable":executable,"sha256":hash,"handler":"Assess","operation":"Assess","environment":[],"working_directory":path},
            {"name":"other","executable":executable,"sha256":hash,"handler":"Assess","operation":"Assess","environment":[],"working_directory":path}],
            "topology":{"agents":["client","worker","other"],"edges":[
                ["client","worker"],["worker","client"],["worker","other"],["other","worker"]],
                "application":"agent-task/1","max_calls":5,"wall_seconds":60}});
        Self { path, bundle }
    }
    fn engine(&self) -> Engine {
        Engine::open(&self.bundle, &self.path, "test-run").unwrap()
    }
    fn invocation(&self, engine: &Engine) -> Step {
        engine
            .inner
            .prepare("", "root", "request", "client", "worker", &json!({"n":1}))
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn live_observation_is_read_only_ordered_and_payloads_require_opt_in() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let secret = "sensitive-application-content";
    let request = engine
        .inner
        .prepare(
            "",
            "root",
            "request",
            "client",
            "worker",
            &json!({"text":secret}),
        )
        .unwrap();
    let message = engine.inner.message(
        format!("req-{}", request.id),
        "client",
        "worker",
        "Request",
        json!({"task_id":request.id,"operation":"Assess","input":{"text":secret}}),
    );
    {
        let mut protocol = lock(&engine.inner.protocol).unwrap();
        protocol.send(&message, "client").unwrap();
        protocol.receive("test-run", "worker", 1).unwrap();
    }
    let effect = engine
        .inner
        .begin_effect(&request, "inference", &json!({"prompt":secret}))
        .unwrap();
    let state = observe(&fixture.path, "test-run", false).unwrap();
    assert_eq!(state["version"], "mas-host-observation/1");
    assert_eq!(state["messages"][0]["message_id"], message.message_id);
    assert_eq!(state["messages"][0]["sequence"], 1);
    assert_eq!(state["messages"][0]["task_id"], request.id);
    assert_eq!(state["messages"][0]["operation"], "Assess");
    assert_eq!(state["messages"][0]["received"], true);
    assert_eq!(state["calls"][0]["id"], effect["invocation_id"]);
    assert_eq!(state["calls"][0]["state"], "reserved");
    assert_eq!(state["steps"][1]["state"], "executing");
    assert!(!state.to_string().contains(secret));
    assert!(state["messages"][0].get("payload").is_none());
    assert!(state["steps"][0].get("output").is_none());
    assert!(state["calls"][0].get("evidence").is_none());
    let full = observe(&fixture.path, "test-run", true).unwrap();
    assert_eq!(full["messages"][0]["payload"]["input"]["text"], secret);
    assert_eq!(full["steps"][1]["input"]["prompt"], secret);
    engine
        .inner
        .set_state(
            &request.id,
            "completed",
            Some(&json!({"text":secret})),
            None,
        )
        .unwrap();
    engine
        .inner
        .terminal(&request, Some(&json!({"text":secret})), None)
        .unwrap();
    let full = observe(&fixture.path, "test-run", true).unwrap();
    assert_eq!(full["steps"][0]["output"]["text"], secret);
    assert_eq!(full["messages"][1]["kind"], "Result");
    assert_eq!(full["messages"][1]["sequence"], 2);
    assert_eq!(full["messages"][1]["reply_to"], message.message_id);
    assert_eq!(full["messages"][0]["acked"], true);
    assert_eq!(full["tasks"][0]["state"], "completed");

    // Expired wall time must not make the visualisation an authority that
    // reconciles tasks. Database bytes remain identical across both modes.
    let directory = engine.state_directory();
    let protocol = Connection::open(directory.join("protocol.sqlite")).unwrap();
    protocol.execute("UPDATE runs SET deadline=0", []).unwrap();
    drop(protocol);
    let before = ["execution.sqlite", "protocol.sqlite"]
        .map(|file| std::fs::read(directory.join(file)).unwrap());
    let expired = observe(&fixture.path, "test-run", false).unwrap();
    assert_eq!(expired["run"]["expired"], false);
    observe(&fixture.path, "test-run", true).unwrap();
    for (file, bytes) in ["execution.sqlite", "protocol.sqlite"]
        .into_iter()
        .zip(before)
    {
        assert_eq!(std::fs::read(directory.join(file)).unwrap(), bytes);
    }
}

#[test]
fn observation_rejects_missing_and_incompatible_runs_without_creating_state() {
    let fixture = Fixture::new();
    assert!(observe(&fixture.path, "missing", false).is_err());
    assert!(!fixture.path.join("missing").exists());
    assert!(observe(&fixture.path, "../escape", false).is_err());
    let engine = fixture.engine();
    let db = Connection::open(engine.state_directory().join("protocol.sqlite")).unwrap();
    db.execute_batch("PRAGMA user_version=3").unwrap();
    assert!(observe(&fixture.path, "test-run", false)
        .unwrap_err()
        .contains("schema version 2"));
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn rejects_previous_bundle_version_and_missing_public_operation() {
    let mut fixture = Fixture::new();
    drop(fixture.engine());
    fixture.bundle["version"] = json!("mas-bundle/1");
    assert!(Engine::open(&fixture.bundle, &fixture.path, "test-run")
        .err()
        .unwrap()
        .contains("unsupported bundle version"));
    fixture.bundle["version"] = json!("mas-bundle/3");
    fixture.bundle["agents"][0]
        .as_object_mut()
        .unwrap()
        .remove("operation");
    assert!(Engine::open(&fixture.bundle, &fixture.path, "test-run")
        .err()
        .unwrap()
        .contains("missing or unknown field"));
}

#[test]
fn forbidden_route_uses_protocol_authority() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let failure = engine.run("other", &json!({})).unwrap_err();
    assert!(failure.contains("route forbidden"), "{failure}");
    let state = engine.status().unwrap();
    assert_eq!(state["protocol"]["usage"]["messages"], 0);
    assert_eq!(state["protocol"]["usage"]["calls"], 0);
}

#[test]
fn execution_permission_is_uncertain_after_restart_and_requires_resolution() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    let first = engine
        .inner
        .begin_effect(&invocation, "publish", &json!({"object":"release"}))
        .unwrap();
    assert_eq!(first["state"], "execute");
    let effect_id = first["invocation_id"].as_str().unwrap().to_owned();
    assert!(engine
        .inner
        .begin_effect(&invocation, "publish", &json!({"object":"release"}))
        .is_err());
    assert!(
        resolve(
            &fixture.path,
            "test-run",
            &effect_id,
            &json!({"published":true}),
            &json!({"receipt":"1"})
        )
        .is_err(),
        "active owner must exclude administrative resolution"
    );
    let deadline = engine.status().unwrap()["protocol"]["deadline"].clone();
    drop(engine);
    let engine = fixture.engine();
    assert!(engine.inner.has_unknown().unwrap());
    assert!(engine
        .run("worker", &json!({"n":1}))
        .unwrap_err()
        .contains("uncertain"));
    assert_eq!(engine.status().unwrap()["protocol"]["deadline"], deadline);
    drop(engine);
    resolve(
        &fixture.path,
        "test-run",
        &effect_id,
        &json!({"published":true}),
        &json!({"receipt":"1"}),
    )
    .unwrap();
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    let cached = engine
        .inner
        .begin_effect(&invocation, "publish", &json!({"object":"release"}))
        .unwrap();
    assert_eq!(cached["state"], "cached");
    assert_eq!(cached["output"], json!({"published":true}));
    let state = engine.status().unwrap();
    assert_eq!(state["protocol"]["usage"]["calls"], 1);
    assert_eq!(state["protocol"]["calls"][0]["state"], "recorded");
    assert!(!engine.inner.has_unsettled_effects().unwrap());
}

#[test]
fn prepared_reservation_recovers_without_a_second_charge() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    let prepared = engine
        .inner
        .prepare(
            &invocation.id,
            "publish",
            "effect",
            "worker",
            "",
            &json!({"object":"release"}),
        )
        .unwrap();
    lock(&engine.inner.protocol)
        .unwrap()
        .reserve_call("test-run", "worker", &prepared.id)
        .unwrap();
    drop(engine);
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    let permission = engine
        .inner
        .begin_effect(&invocation, "publish", &json!({"object":"release"}))
        .unwrap();
    assert_eq!(permission["state"], "execute");
    assert_eq!(engine.status().unwrap()["protocol"]["usage"]["calls"], 1);
    engine
        .inner
        .complete_effect(
            &invocation,
            "publish",
            &json!({"value":1}),
            &json!({"receipt":"a"}),
        )
        .unwrap();
    engine
        .inner
        .complete_effect(
            &invocation,
            "publish",
            &json!({"value":1}),
            &json!({"receipt":"a"}),
        )
        .unwrap();
    assert!(engine
        .inner
        .complete_effect(
            &invocation,
            "publish",
            &json!({"value":2}),
            &json!({"receipt":"a"})
        )
        .is_err());
    assert_eq!(
        engine
            .inner
            .begin_effect(&invocation, "publish", &json!({"object":"release"}))
            .unwrap()["output"],
        json!({"value":1})
    );
}

#[test]
fn effect_identity_and_evidence_fail_closed() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    engine
        .inner
        .begin_effect(&invocation, "publish", &json!({"object":"release"}))
        .unwrap();
    assert!(engine
        .inner
        .begin_effect(&invocation, "publish", &json!({"object":"another"}))
        .unwrap_err()
        .contains("reused"));
    assert!(engine
        .inner
        .prepare(
            &invocation.id,
            "publish",
            "request",
            "worker",
            "other",
            &json!({})
        )
        .is_err());
    assert!(engine
        .inner
        .complete_effect(&invocation, "publish", &json!({"value":1}), &json!({}))
        .is_err());
    assert!(engine
        .inner
        .complete_effect(
            &invocation,
            "publish",
            &json!({"value":1.5}),
            &json!({"receipt":"a"})
        )
        .is_err());
    assert_eq!(
        engine.status().unwrap()["protocol"]["calls"][0]["state"],
        "reserved"
    );
}

#[test]
fn parallel_sibling_effect_does_not_block_independent_branch_completion() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let root = fixture.invocation(&engine);
    let left = engine
        .inner
        .prepare(&root.id, "left", "request", "worker", "other", &json!({}))
        .unwrap();
    let right = engine
        .inner
        .prepare(&root.id, "right", "request", "worker", "other", &json!({}))
        .unwrap();
    engine
        .inner
        .begin_effect(&left, "publish", &json!({"object":"release"}))
        .unwrap();
    assert!(engine.inner.has_unsettled_below(&root.id).unwrap());
    assert!(engine.inner.has_unsettled_below(&left.id).unwrap());
    assert!(!engine.inner.has_unsettled_below(&right.id).unwrap());
    engine
        .inner
        .complete_effect(&left, "publish", &json!({}), &json!({"receipt":"left"}))
        .unwrap();
    assert!(!engine.inner.has_unsettled_below(&root.id).unwrap());
}

#[test]
fn completed_output_survives_a_missing_protocol_finish() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    let permission = engine
        .inner
        .begin_effect(&invocation, "publish", &json!({}))
        .unwrap();
    let id = permission["invocation_id"].as_str().unwrap();
    engine
        .inner
        .set_state(
            id,
            "completed",
            Some(&json!({"retained":true})),
            Some(&json!({"receipt":"durable"})),
        )
        .unwrap();
    drop(engine);
    let engine = fixture.engine();
    engine.inner.reconcile_completed_effects().unwrap();
    let invocation = fixture.invocation(&engine);
    assert_eq!(
        engine
            .inner
            .begin_effect(&invocation, "publish", &json!({}))
            .unwrap()["state"],
        "cached"
    );
    assert_eq!(engine.status().unwrap()["protocol"]["usage"]["calls"], 1);
}

#[test]
fn identity_drift_and_reused_root_input_are_rejected() {
    let mut fixture = Fixture::new();
    let engine = fixture.engine();
    let _ = fixture.invocation(&engine);
    assert!(engine
        .inner
        .prepare("", "root", "request", "client", "worker", &json!({"n":2}))
        .is_err());
    drop(engine);
    fixture.bundle["topology"]["max_calls"] = json!(6);
    assert!(Engine::open(&fixture.bundle, &fixture.path, "test-run")
        .err()
        .unwrap()
        .contains("identity differs"));
}

#[test]
fn cancellation_persists_without_resolving_an_effect() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    engine
        .inner
        .begin_effect(&invocation, "publish", &json!({}))
        .unwrap();
    cancel(&fixture.path, "test-run").unwrap();
    assert!(engine.inner.check_live().unwrap_err().contains("cancelled"));
    assert_eq!(
        engine.status().unwrap()["protocol"]["calls"][0]["state"],
        "reserved"
    );
}

#[test]
fn cancellation_prevents_durable_final_result_and_retains_known_effect_completion() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let invocation = fixture.invocation(&engine);
    engine
        .inner
        .begin_effect(&invocation, "inference-0", &json!({"model":"mock"}))
        .unwrap();
    cancel(&fixture.path, "test-run").unwrap();
    // A complete provider response observed by the host is still durable
    // evidence, even when cancellation prevents the containing task succeeding.
    engine
        .inner
        .complete_effect(
            &invocation,
            "inference-0",
            &json!({"accepted":true}),
            &json!({"provider_response_id":"response-1"}),
        )
        .unwrap();
    assert!(engine
        .inner
        .commit_completed(&invocation.id, &json!({"accepted":true}))
        .unwrap_err()
        .contains("cancelled"));
    let state = engine.status().unwrap();
    let root = state["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == invocation.id)
        .unwrap();
    assert_ne!(root["state"], "completed");
    assert_eq!(state["protocol"]["calls"][0]["state"], "recorded");
    assert!(!engine.inner.has_unsettled_effects().unwrap());
}

#[test]
fn bundle_two_binds_configuration_arguments_and_public_operation() {
    let mut fixture = Fixture::new();
    fixture.bundle["agents"][0]["configuration"] = json!({"kind":"llm","model":"mock", "connection":{
        "provider":"mock","api_key_env":"MAS_HOST_TEST_KEY","env":["MAS_HOST_TEST_EXTRA"]}});
    fixture.bundle["agents"][0]["environment"] =
        json!(["MAS_HOST_TEST_KEY", "MAS_HOST_TEST_EXTRA"]);
    fixture.bundle["agents"][0]["arguments"] = json!(["__builtin"]);
    fixture.bundle["agents"][0]["operation"] = "AssessProposal".into();
    let engine = fixture.engine();
    let agent = &engine.inner.agents["worker"];
    assert_eq!(agent.handler.as_deref(), Some("Assess"));
    assert_eq!(agent.operation, "AssessProposal");
    assert_eq!(agent.arguments, ["__builtin"]);
    assert_eq!(agent.configuration["model"], "mock");
    assert_eq!(
        agent.environment_names,
        BTreeSet::from(["MAS_HOST_TEST_EXTRA".into(), "MAS_HOST_TEST_KEY".into()])
    );
    assert!(engine.inner.agents["other"].environment_names.is_empty());
}

#[test]
fn executable_verification_cache_does_not_trust_a_different_digest() {
    let mut fixture = Fixture::new();
    fixture.bundle["agents"][1]["sha256"] = "0".repeat(64).into();
    assert!(Engine::open(&fixture.bundle, &fixture.path, "test-run")
        .err()
        .unwrap()
        .contains("digest differs"));
}

#[test]
fn environment_scope_is_independent_of_implementation_kind() {
    assert!(declared_environment(&json!([])).unwrap().is_empty());
    for value in [
        json!({}),
        json!("ALL"),
        json!(["PATH"]),
        json!(["MAS_WORKER_CHANNEL_FD"]),
        json!(["INVALID=NAME"]),
    ] {
        assert!(declared_environment(&value).is_err());
    }
    std::env::set_var("MAS_HOST_TEST_SCOPED_KEY", "temporary-test-value");
    std::env::set_var("MAS_HOST_TEST_UNDECLARED_KEY", "must-not-be-copied");
    let mut fixture = Fixture::new();
    fixture.bundle["agents"][0]["environment"] = json!(["MAS_HOST_TEST_SCOPED_KEY"]);
    let engine = fixture.engine();
    assert_eq!(
        engine.inner.agents["worker"].environment().unwrap(),
        BTreeMap::from([(
            "MAS_HOST_TEST_SCOPED_KEY".into(),
            "temporary-test-value".into()
        )])
    );
    assert!(engine.inner.agents["other"]
        .environment()
        .unwrap()
        .is_empty());
    let persisted = std::fs::read(fixture.path.join("test-run").join("execution.sqlite")).unwrap();
    assert!(!persisted
        .windows(b"temporary-test-value".len())
        .any(|bytes| bytes == b"temporary-test-value"));
    std::env::remove_var("MAS_HOST_TEST_SCOPED_KEY");
    std::env::remove_var("MAS_HOST_TEST_UNDECLARED_KEY");
}

#[test]
fn independent_run_namespaces_have_distinct_effect_identities() {
    let first = Fixture::new();
    let second = Fixture::new();
    let a = first.engine();
    let b = second.engine();
    let a_call = a
        .inner
        .begin_effect(&first.invocation(&a), "publish", &json!({}))
        .unwrap();
    let b_call = b
        .inner
        .begin_effect(&second.invocation(&b), "publish", &json!({}))
        .unwrap();
    assert_ne!(a_call["invocation_id"], b_call["invocation_id"]);
    let nonce = a.inner.nonce.clone();
    drop(a);
    assert_eq!(first.engine().inner.nonce, nonce);
}

#[test]
fn unfinished_child_prevents_parent_success_but_rejected_route_is_settled() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let root = fixture.invocation(&engine);
    let child = engine
        .inner
        .prepare(
            &root.id,
            "missing",
            "request",
            "worker",
            "other",
            &json!({}),
        )
        .unwrap();
    assert!(engine
        .inner
        .settle_children(&root.id)
        .unwrap_err()
        .contains("unfinished"));
    engine
        .inner
        .set_state(
            &child.id,
            "rejected",
            None,
            Some(&json!({"error":"not admitted"})),
        )
        .unwrap();
    engine.inner.settle_children(&root.id).unwrap();
}

#[test]
fn removed_binary_does_not_reclassify_an_admitted_child_as_rejected() {
    let mut fixture = Fixture::new();
    let executable = fixture.path.join("worker");
    std::fs::copy("/usr/bin/true", &executable).unwrap();
    fixture.bundle["agents"][1]["executable"] = json!(executable);
    let engine = fixture.engine();
    let root = fixture.invocation(&engine);
    let child = engine
        .inner
        .prepare(&root.id, "child", "request", "worker", "other", &json!({}))
        .unwrap();
    let request = engine.inner.message(
        format!("req-{}", child.id),
        "worker",
        "other",
        "Request",
        json!({"task_id":child.id,"operation":"Assess","input":{}}),
    );
    lock(&engine.inner.protocol)
        .unwrap()
        .send(&request, "worker")
        .unwrap();
    std::fs::remove_file(&executable).unwrap();
    assert!(engine
        .inner
        .request(&root.id, "child", "worker", "other", &json!({}), 1)
        .is_err());
    assert_eq!(
        read_step(&lock(&engine.inner.journal).unwrap(), &child.id)
            .unwrap()
            .unwrap()
            .state,
        "blocked"
    );
    assert!(engine.inner.settle_children(&root.id).is_err());
}

#[test]
fn parent_repairs_retained_child_output_before_success() {
    let fixture = Fixture::new();
    let engine = fixture.engine();
    let root = fixture.invocation(&engine);
    let child = engine
        .inner
        .prepare(&root.id, "child", "request", "worker", "other", &json!({}))
        .unwrap();
    let request = engine.inner.message(
        format!("req-{}", child.id),
        "worker",
        "other",
        "Request",
        json!({"task_id":child.id,"operation":"Assess","input":{}}),
    );
    lock(&engine.inner.protocol)
        .unwrap()
        .send(&request, "worker")
        .unwrap();
    lock(&engine.inner.protocol)
        .unwrap()
        .receive("test-run", "other", 10)
        .unwrap();
    engine
        .inner
        .set_state(&child.id, "completed", Some(&json!({"result":1})), None)
        .unwrap();
    engine.inner.settle_children(&root.id).unwrap();
    let snapshot = engine.status().unwrap();
    assert_eq!(snapshot["protocol"]["tasks"][0]["state"], "completed");
    assert!(snapshot["protocol"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|message| message["acked"] == true));
}
