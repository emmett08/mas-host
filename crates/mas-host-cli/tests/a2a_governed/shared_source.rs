//! Shared source reads through real authenticated HTTP and retained journals.
use super::retained_source::{
    change_authority, completed, configured_source, exchange, journal, submitted,
};
use super::*;

const SHARED_EXTENSION: &str = "urn:mas:host-governed-shared-source-read:1";
const LINEAGE_EXTENSION: &str = "urn:mas:host-governed-lineage-read:1";
const PUBLICATION_EXTENSION: &str = "urn:mas:host-governed-source-publication:1";
const REFERENCE: &str = "approved-deployment-point";

fn reader_token(number: usize) -> String {
    format!("shared-reader-{number:02}-synthetic-token-0123456789")
}

fn configured(project: &Project, readers: usize) -> String {
    configured_source(project);
    let config: Value =
        serde_json::from_slice(&fs::read(project.root.join("governance.json")).unwrap()).unwrap();
    let owner = mas_host_core::governance::GovernedSession::from_value(
        &config, "tenant-a", "alice", "test", "grant-a",
    )
    .unwrap();
    let owner_id = owner
        .identity_commitment(&json!(["a2a-owner", "tenant-a", "alice"]))
        .unwrap();
    let message_id = owner
        .identity_commitment(&json!(["a2a-message", "fixed-original-id"]))
        .unwrap();
    let task_id = format!(
        "a2a-{}",
        &digest(&json!([owner_id, message_id]).to_string())[..56]
    );
    // Exports are operator-pinned service identity. Predeclare the exact
    // collection's task reference; publication does not mutate a retained
    // service configuration after acquisition.
    let path = project.root.join("gateway.json");
    let mut gateway: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    gateway["source_exports"] = json!([{"name":REFERENCE,"task_id":task_id,"key":"collect",
        "tenant":"tenant-a","principal":"alice","purpose":"test","grant_ref":"grant-a","actor":"collector"}]);
    write(path, &gateway);
    let path = project.root.join("authentication.json");
    let mut authentication: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for number in 0..readers {
        authentication["principals"].as_array_mut().unwrap().push(json!({
            "tenant":"tenant-a","principal":format!("reader-{number:02}"),"purpose":"test",
            "grant_ref":format!("reader-grant-{number:02}"),"token_sha256":digest(&reader_token(number)),"agents":["collector"]}));
    }
    write(path, &authentication);
    change_authority(project, |authority| {
        let original = authority["grants"][0].clone();
        for number in 0..readers {
            let mut grant = original.clone();
            grant["principal"] = json!(format!("reader-{number:02}"));
            grant["grant_ref"] = json!(format!("reader-grant-{number:02}"));
            authority["grants"].as_array_mut().unwrap().push(grant);
        }
        for policy in authority["policies"].as_array_mut().unwrap() {
            if policy["tenant"] != "tenant-a" {
                continue;
            }
            let resolve = policy["rules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rule| rule["operation"] == "resolve")
                .unwrap()
                .clone();
            for number in 0..readers {
                let mut rule = resolve.clone();
                rule["principal"] = json!(format!("reader-{number:02}"));
                rule["recipient"] = rule["principal"].clone();
                policy["rules"].as_array_mut().unwrap().push(rule);
            }
        }
    });
    task_id
}

fn shared(server: &Server, token: &str, reference: &str) -> (u16, String, Value) {
    exchange(
        server,
        token,
        Some(&format!("{EXTENSION}, {SHARED_EXTENSION}")),
        "GetSharedSource",
        json!({"reference":reference}),
    )
}

fn source_denied(response: &(u16, String, Value)) {
    assert_eq!(response.0, 200);
    assert_eq!(
        response.2["error"],
        json!({"code":-32001,"message":"source_not_found"})
    );
    assert!(response.2.get("result").is_none());
    assert!(!response.2.to_string().contains(PRIVATE));
}

#[test]
fn shared_reader_negotiates_independent_extension_and_hides_owner_selection() {
    let project = Project::new();
    let id = configured(&project, 1);
    let server = project.start();
    let token = reader_token(0);
    source_denied(&shared(&server, &token, REFERENCE));
    assert_eq!(project.calls(), 0);
    let card = exchange(&server, &token, None, "GetExtendedAgentCard", json!({})).2;
    let extension = card["result"]["capabilities"]["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["uri"] == SHARED_EXTENSION)
        .unwrap();
    assert_eq!(extension["required"], false);
    assert_eq!(extension["params"]["requestFields"], json!(["reference"]));
    assert!(!card.to_string().contains(REFERENCE));
    assert!(!card.to_string().contains(&id));
    assert!(!card.to_string().contains("grant-a"));
    assert!(!card.to_string().contains(LINEAGE_EXTENSION));
    for (extensions, expected) in [
        (None, "required_extension_missing"),
        (Some(SHARED_EXTENSION), "required_extension_missing"),
        (Some(EXTENSION), "required_shared_source_extension_missing"),
    ] {
        let response = exchange(
            &server,
            &token,
            extensions,
            "GetSharedSource",
            json!({"reference":REFERENCE}),
        );
        assert_eq!(response.2["error"]["message"], expected);
    }
    assert_eq!(submitted(&project, &server, "available"), id);
    completed(&server, &id);
    let response = shared(&server, &token, REFERENCE);
    assert!(response.2.get("error").is_none(), "{}", response.2);
    assert_eq!(response.2["result"]["projection"], json!({"approved":true}));
    assert!(response.1.contains(SHARED_EXTENSION));
    source_denied(&shared(&server, OTHER, REFERENCE));
    source_denied(&shared(&server, &token, "unpublished"));
    let (_, own_task) = server.rpc(&token, "GetTask", json!({"id":id}));
    assert_eq!(own_task["error"]["message"], "task_not_found");
    for extra in [
        "id",
        "key",
        "tenant",
        "principal",
        "actor",
        "directory",
        "source_id",
    ] {
        let mut params = json!({"reference":REFERENCE});
        params[extra] = json!(PRIVATE);
        let response = exchange(
            &server,
            &token,
            Some(&format!("{EXTENSION}, {SHARED_EXTENSION}")),
            "GetSharedSource",
            params,
        );
        assert_eq!(response.2["error"]["code"], -32602);
        assert!(!response.2.to_string().contains(PRIVATE));
    }
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

#[test]
fn shared_reader_revalidates_producer_and_subscriber_across_restart_and_rotation() {
    let project = Project::new();
    let id = configured(&project, 2);
    let server = project.start();
    assert_eq!(submitted(&project, &server, "available"), id);
    completed(&server, &id);
    let original = shared(&server, &reader_token(0), REFERENCE).2;
    assert!(original.get("error").is_none(), "{original}");
    let before = journal(&project, &id);
    drop(server);
    let server = project.start();
    assert_eq!(shared(&server, &reader_token(0), REFERENCE).2, original);
    assert_eq!(journal(&project, &id), before);
    let path = project.root.join("authentication.json");
    let mut authentication: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    authentication["principals"][0]["token_sha256"] =
        json!(digest("rotated-owner-synthetic-token-0123456789"));
    write(&path, &authentication);
    assert_eq!(shared(&server, &reader_token(0), REFERENCE).2, original);
    authentication["principals"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    write(&path, &authentication);
    source_denied(&shared(&server, &reader_token(0), REFERENCE));
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

#[test]
fn shared_reader_serves_more_than_thirty_two_current_subscribers_without_reacquisition() {
    let project = Project::new();
    let id = configured(&project, 33);
    let server = project.start();
    assert_eq!(submitted(&project, &server, "available"), id);
    completed(&server, &id);
    let before = journal(&project, &id);
    for number in 0..33 {
        let response = shared(&server, &reader_token(number), REFERENCE);
        assert!(
            response.2.get("error").is_none(),
            "reader {number}: {}",
            response.2
        );
        assert_eq!(response.2["result"]["projection"], json!({"approved":true}));
    }
    change_authority(&project, |authority| {
        authority["grants"]
            .as_array_mut()
            .unwrap()
            .retain(|grant| grant["grant_ref"] != "reader-grant-32");
    });
    let revoked = shared(&server, &reader_token(32), REFERENCE);
    assert_eq!(revoked.0, 401);
    assert!(revoked.2.get("result").is_none());
    assert_eq!(
        shared(&server, &reader_token(0), REFERENCE).2["result"]["projection"],
        json!({"approved":true})
    );
    assert_eq!(journal(&project, &id), before);
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

#[test]
fn referenced_lineage_http_pages_bind_the_complete_source_for_owner_and_recipient() {
    let project = Project::new();
    let id = configured(&project, 1);
    let path = project.root.join("governance.json");
    let mut governance: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    governance["lineage"] = json!({"version":"mas-governed-referenced-lineage/1",
        "max_sources":128,"max_metadata_bytes":262_144});
    write(path, &governance);
    let server = project.start();
    assert_eq!(submitted(&project, &server, "available"), id);
    completed(&server, &id);
    let token = reader_token(0);
    let before = journal(&project, &id);
    let source = shared(&server, &token, REFERENCE).2;
    assert!(source.get("error").is_none(), "{source}");
    let view = &source["result"];
    assert_eq!(view["version"], "mas-governed-source-view/2");
    assert_eq!(view["lineage_disposition"], "referenced");
    assert_eq!(view["lineage"], json!([]));
    let reference = &view["lineage_reference"];
    assert_eq!(reference["version"], "mas-governed-lineage-reference/1");
    assert_eq!(reference["page_size"], 16);
    let closure = reference["closure_ref"].as_str().unwrap();
    let count = reference["source_count"].as_u64().unwrap();
    assert!(count > 0);
    let extensions = format!("{EXTENSION}, {LINEAGE_EXTENSION}");
    let mut offset = 0;
    let mut records = Vec::new();
    loop {
        let shared = exchange(
            &server,
            &token,
            Some(&extensions),
            "GetSharedSourceLineage",
            json!({"reference":REFERENCE,"closure_ref":closure,"offset":offset}),
        );
        assert!(shared.2.get("error").is_none(), "{}", shared.2);
        assert!(shared.1.contains(LINEAGE_EXTENSION));
        let page = &shared.2["result"];
        assert_eq!(page["version"], "mas-governed-lineage-page/1");
        assert_eq!(page["closure_ref"], closure);
        assert_eq!(page["source_count"], count);
        assert_eq!(page["offset"], offset);
        assert_eq!(page["coverage"], "complete-closure-resolved-page");
        let selected = page["records"].as_array().unwrap();
        assert!(selected.len() <= 16);
        records.extend(selected.iter().cloned());
        let own = exchange(
            &server,
            TOKEN,
            Some(&extensions),
            "GetRetainedSourceLineage",
            json!({"id":id,"key":"collect","closure_ref":closure,"offset":offset}),
        );
        assert_eq!(own.2["result"], *page);
        let Some(next) = page["next_offset"].as_u64() else {
            break;
        };
        assert_eq!(next, offset + selected.len() as u64);
        offset = next;
    }
    assert_eq!(records.len() as u64, count);
    assert!(records
        .iter()
        .any(|record| record["id"] == view["source_id"]));
    let card = exchange(&server, &token, None, "GetExtendedAgentCard", json!({})).2;
    assert!(card.to_string().contains(LINEAGE_EXTENSION));
    let shared_contract = card["result"]["capabilities"]["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|extension| extension["uri"] == SHARED_EXTENSION)
        .unwrap();
    assert_eq!(
        shared_contract["params"]["resultVersions"],
        json!(["mas-governed-source-view/1", "mas-governed-source-view/2"])
    );
    let methods = card["result"]["capabilities"]["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|extension| extension["uri"] == LINEAGE_EXTENSION)
        .unwrap()["params"]["methods"]
        .clone();
    assert_eq!(
        methods,
        json!(["GetRetainedSourceLineage", "GetSharedSourceLineage"])
    );
    let missing = exchange(
        &server,
        &token,
        Some(EXTENSION),
        "GetSharedSourceLineage",
        json!({"reference":REFERENCE,"closure_ref":closure,"offset":0}),
    );
    assert_eq!(
        missing.2["error"]["message"],
        "required_lineage_extension_missing"
    );
    for (bad_closure, bad_offset) in [("f".repeat(64), 0), (closure.to_owned(), 1)] {
        source_denied(&exchange(
            &server,
            &token,
            Some(&extensions),
            "GetSharedSourceLineage",
            json!({"reference":REFERENCE,"closure_ref":bad_closure,"offset":bad_offset}),
        ));
    }
    source_denied(&exchange(
        &server,
        &token,
        Some(&extensions),
        "GetRetainedSourceLineage",
        json!({"id":id,"key":"collect","closure_ref":closure,"offset":0}),
    ));
    assert_eq!(journal(&project, &id), before);
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}

fn publication_configured(project: &Project, permit: bool) -> String {
    let id = configured(project, 1);
    let path = project.root.join("gateway.json");
    let mut gateway: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    gateway.as_object_mut().unwrap().remove("source_exports");
    gateway["source_publication"] = json!(true);
    write(path, &gateway);
    let path = project.root.join("governance.json");
    let mut governance: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    governance["lifecycle_file"] = json!(project.root.join("lifecycle.sqlite"));
    write(path, &governance);
    if permit {
        change_authority(project, |authority| {
            for policy in authority["policies"].as_array_mut().unwrap() {
                if policy["tenant"] != "tenant-a" {
                    continue;
                }
                let mut rule = policy["rules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|rule| rule["operation"] == "resolve" && rule["principal"] == "alice")
                    .unwrap()
                    .clone();
                rule["operation"] = json!("publish-source");
                policy["rules"].as_array_mut().unwrap().push(rule);
            }
        });
    }
    id
}

fn publish(server: &Server, token: &str, id: &str, alias: &str) -> (u16, String, Value) {
    exchange(
        server,
        token,
        Some(&format!("{EXTENSION}, {PUBLICATION_EXTENSION}")),
        "PublishRetainedSource",
        json!({"id":id,"key":"collect","reference":alias}),
    )
}

#[test]
fn dynamic_source_publication_is_owned_immutable_and_privately_named() {
    let project = Project::new();
    let id = publication_configured(&project, true);
    let server = project.start();
    source_denied(&publish(&server, TOKEN, &id, PRIVATE));
    assert_eq!(submitted(&project, &server, "available"), id);
    completed(&server, &id);
    let missing = exchange(
        &server,
        TOKEN,
        Some(EXTENSION),
        "PublishRetainedSource",
        json!({"id":id,"key":"collect","reference":PRIVATE}),
    );
    assert_eq!(
        missing.2["error"]["message"],
        "required_source_publication_extension_missing"
    );
    source_denied(&publish(&server, &reader_token(0), &id, PRIVATE));
    source_denied(&publish(&server, OTHER, &id, PRIVATE));
    let before = journal(&project, &id);
    let first = publish(&server, TOKEN, &id, PRIVATE);
    assert!(first.2.get("error").is_none(), "{}", first.2);
    assert_eq!(
        first.2["result"]["version"],
        "mas-governed-source-publication/1"
    );
    assert_eq!(first.2["result"]["status"], "published");
    let reference = first.2["result"]["reference"].as_str().unwrap();
    assert!(reference.starts_with("shared-"));
    assert_eq!(reference.len(), 71);
    assert!(!reference.contains(PRIVATE));
    assert_eq!(publish(&server, TOKEN, &id, PRIVATE).2, first.2);
    let read = shared(&server, &reader_token(0), reference);
    assert_eq!(read.2["result"]["projection"], json!({"approved":true}));
    assert_eq!(journal(&project, &id), before);
    assert_eq!(project.calls(), 1);
    let db = rusqlite::Connection::open(project.root.join("state/service.sqlite")).unwrap();
    let rows: i64 = db
        .query_row("SELECT count(*) FROM published_source_keys", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 1);
    let body: String = db
        .query_row("SELECT body FROM published_sources", [], |row| row.get(0))
        .unwrap();
    assert!(!body.contains(PRIVATE));
    let mut second = project.send("available", PRIVATE);
    second["message"]["messageId"] = json!("second-retained-source");
    let (_, submitted) = server.rpc(TOKEN, "SendMessage", second);
    assert!(submitted.get("error").is_none(), "{submitted}");
    let second_id = submitted["result"]["task"]["id"].as_str().unwrap();
    completed(&server, second_id);
    let conflicting = publish(&server, TOKEN, second_id, PRIVATE);
    assert_eq!(
        conflicting.2["error"]["message"],
        "source_publication_conflict"
    );
    assert_eq!(shared(&server, &reader_token(0), reference).2, read.2);
    assert_eq!(project.calls(), 2);
    drop(server);
    let retirement = Command::new(env!("CARGO_BIN_EXE_mas-host"))
        .arg("govern-lifecycle")
        .arg("--state")
        .arg(project.root.join("state/runs"))
        .arg("--run")
        .arg(&id)
        .arg("--governance")
        .arg(project.root.join("governance.json"))
        .args([
            "--tenant",
            "tenant-a",
            "--principal",
            "alice",
            "--purpose",
            "test",
            "--grant",
            "grant-a",
            "--action",
            "retire",
        ])
        .arg("--service-state")
        .arg(project.root.join("state"))
        .output()
        .unwrap();
    assert!(
        retirement.status.success(),
        "{}",
        String::from_utf8_lossy(&retirement.stderr)
    );
    let active: i64 = db
        .query_row("SELECT count(*) FROM published_sources", [], |row| {
            row.get(0)
        })
        .unwrap();
    let reserved: i64 = db
        .query_row("SELECT count(*) FROM published_source_keys", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(active, 0);
    assert_eq!(reserved, 1);
    let server = project.start();
    source_denied(&shared(&server, &reader_token(0), reference));
    assert_eq!(
        publish(&server, TOKEN, second_id, PRIVATE).2["error"]["message"],
        "source_publication_conflict"
    );
    assert_eq!(project.calls(), 2);
    project.assert_private_absent();
}

#[test]
fn dynamic_source_publication_requires_current_permission_for_every_original_source() {
    for baseline_only in [false, true] {
        let project = Project::new();
        let id = publication_configured(&project, baseline_only);
        if baseline_only {
            change_authority(&project, |authority| {
                for policy in authority["policies"].as_array_mut().unwrap() {
                    if policy["tenant"] == "tenant-a" && policy["id"] == "collected" {
                        policy["rules"]
                            .as_array_mut()
                            .unwrap()
                            .retain(|rule| rule["operation"] != "publish-source");
                    }
                }
            });
        }
        let server = project.start();
        assert_eq!(submitted(&project, &server, "available"), id);
        completed(&server, &id);
        source_denied(&publish(&server, TOKEN, &id, "not-authorised"));
        let db = rusqlite::Connection::open(project.root.join("state/service.sqlite")).unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM published_source_keys", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(project.calls(), 1);
    }
}

#[test]
fn dynamic_source_publication_survives_restart_but_rechecks_both_current_parties() {
    let project = Project::new();
    let id = publication_configured(&project, true);
    let server = project.start();
    assert_eq!(submitted(&project, &server, "available"), id);
    completed(&server, &id);
    let published = publish(&server, TOKEN, &id, "stable-acquisition").2;
    assert!(published.get("error").is_none(), "{published}");
    let reference = published["result"]["reference"].as_str().unwrap();
    let original = shared(&server, &reader_token(0), reference).2;
    assert!(original.get("error").is_none(), "{original}");
    drop(server);
    let server = project.start();
    assert_eq!(shared(&server, &reader_token(0), reference).2, original);
    let path = project.root.join("authentication.json");
    let mut authentication: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let rotated = "dynamic-producer-rotated-synthetic-token-0123456789";
    authentication["principals"][0]["token_sha256"] = json!(digest(rotated));
    write(&path, &authentication);
    assert_eq!(
        publish(&server, rotated, &id, "stable-acquisition").2,
        published
    );
    assert_eq!(shared(&server, &reader_token(0), reference).2, original);
    let authority_path = project.root.join("authority.json");
    let authority: Value = serde_json::from_slice(&fs::read(&authority_path).unwrap()).unwrap();
    change_authority(&project, |value| {
        value["grants"]
            .as_array_mut()
            .unwrap()
            .retain(|grant| grant["grant_ref"] != "reader-grant-00")
    });
    assert_eq!(shared(&server, &reader_token(0), reference).0, 401);
    write(authority_path, &authority);
    assert_eq!(shared(&server, &reader_token(0), reference).2, original);
    authentication["principals"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    write(path, &authentication);
    source_denied(&shared(&server, &reader_token(0), reference));
    assert_eq!(project.calls(), 1);
    project.assert_private_absent();
}
