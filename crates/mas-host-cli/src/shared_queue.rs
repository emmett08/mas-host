//! Bounded paged task selection through the existing PostgreSQL queue authority.
use mas_protocol_codec::postgres_repository::PostgresRepository;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// One explicitly identified scheduling turn. Resume its returned sequence
/// cursor in a later turn; resetting to zero starts a new bounded traversal.
pub(super) struct Turn<'a> {
    pub owner: &'a str,
    pub turn_id: &'a str,
    pub after_seq: i64,
    pub page_size: usize,
    pub max_pages: usize,
    pub max_claims: usize,
    pub workflow: &'a str,
    pub workflow_identity: &'a str,
}

/// Task settlement supplied by the separately admitted native dispatcher.
/// Unknown execution preserves the original task/run and never requests retry.
pub(super) enum DispatchOutcome {
    Completed(Value),
    Unknown,
}

/// Cap a single native worker by the authority-evaluated original business
/// expiry and remaining active budget. The status already includes authorised
/// deadline amendments; recomputing from the original policy would lose them.
pub(super) fn task_deadline_ms(task: &Value) -> Result<u64, String> {
    let clock = &task["clock"];
    let timing = &task["timing"];
    if clock["format"] != "mas-execution-clock/2"
        || clock["phase"] != "running"
        || clock["active_branches"] != 1
        || clock["worker_time_complete"] != true
        || timing["phase"] != "running"
        || timing["expired"]
            .as_array()
            .is_none_or(|expired| !expired.is_empty())
    {
        return Err("current single-worker task timing required".into());
    }
    let business = timing["business_expires_ms"]
        .as_u64()
        .ok_or("task business expiry unavailable")?;
    let observed = timing["observed_at_ms"]
        .as_u64()
        .ok_or("task timing observation unavailable")?;
    let remaining = timing["remaining_active_ms"]
        .as_u64()
        .filter(|value| *value > 0)
        .ok_or("task active budget unavailable")?;
    let active = observed
        .checked_add(remaining)
        .ok_or("task timing overflow")?;
    let deadline = business.min(active);
    if deadline <= observed {
        return Err("task deadline expired".into());
    }
    Ok(deadline)
}

trait Authority {
    fn query(&mut self, operation: &str, parameters: &Value) -> Result<Value, String>;
    fn command(
        &mut self,
        identity: &str,
        operation: &str,
        parameters: &Value,
    ) -> Result<Value, String>;
}

impl Authority for PostgresRepository {
    fn query(&mut self, operation: &str, parameters: &Value) -> Result<Value, String> {
        self.host_query(operation, parameters)
    }
    fn command(
        &mut self,
        identity: &str,
        operation: &str,
        parameters: &Value,
    ) -> Result<Value, String> {
        self.host_command(identity, operation, parameters)
    }
}

fn bounded_text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .filter(|text| {
            !text.is_empty() && text.len() <= 4096 && !text.chars().any(char::is_control)
        })
        .ok_or_else(|| format!("invalid queue {key}"))
}

fn command_id(turn: &Turn<'_>, task: &str, operation: &str) -> String {
    let source = json!([
        "mas-host-shared-turn/1",
        turn.owner,
        turn.turn_id,
        task,
        operation
    ]);
    let hash: String = Sha256::digest(source.to_string().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("hq-{hash}")
}

fn validate(turn: &Turn<'_>) -> Result<(), String> {
    if turn.after_seq < 0
        || !(1..=1000).contains(&turn.page_size)
        || !(1..=64).contains(&turn.max_pages)
        || !(1..=16).contains(&turn.max_claims)
        || [turn.owner, turn.turn_id, turn.workflow]
            .iter()
            .any(|text| text.is_empty() || text.len() > 1024 || text.chars().any(char::is_control))
        || turn.workflow_identity.len() != 64
        || !turn
            .workflow_identity
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("invalid bounded shared scheduling turn".into());
    }
    Ok(())
}

/// Examine bounded pages, then claim exact tasks under current authority.
///
/// A page-local `claimable: false` never means queue exhaustion. A replayed
/// claim or an uncertain command never invokes the callback. The callback must
/// independently obtain the native dispatch permit; task ownership alone grants
/// no worker permission. Database calls finish before native work begins.
pub(super) fn drain(
    repository: &mut PostgresRepository,
    turn: &Turn<'_>,
    dispatch: impl FnMut(&mut PostgresRepository, &Value) -> Result<DispatchOutcome, String>,
) -> Result<Value, String> {
    drain_using(repository, turn, dispatch)
}

fn drain_using<A: Authority>(
    authority: &mut A,
    turn: &Turn<'_>,
    mut dispatch: impl FnMut(&mut A, &Value) -> Result<DispatchOutcome, String>,
) -> Result<Value, String> {
    validate(turn)?;
    let mut after = turn.after_seq;
    let mut scanned = 0;
    let mut pages = 0;
    let mut claims = 0;
    let mut replayed_claims = 0;
    let mut tail_seen = false;
    let mut summaries = Vec::new();
    let mut seen = BTreeSet::new();
    'pages: while pages < turn.max_pages && claims < turn.max_claims {
        let page_start = after;
        let page = authority.query(
            "task.pending",
            &json!({"after":after,"limit":turn.page_size}),
        )?;
        let tasks = page["tasks"].as_array().ok_or("invalid queue page")?;
        if tasks.len() > turn.page_size {
            return Err("oversized queue page".into());
        }
        let continuation = page.get("next").ok_or("queue continuation unavailable")?;
        let next = if continuation.is_null() {
            None
        } else {
            Some(
                continuation
                    .as_i64()
                    .filter(|next| *next > after)
                    .ok_or("invalid queue continuation")?,
            )
        };
        if tasks.is_empty() && next.is_some() {
            return Err("empty nonterminal queue page".into());
        }
        let scheduler = authority.query("task.scheduler", &json!({}))?;
        let last = scheduler["last_principal"]
            .as_str()
            .filter(|text| text.len() <= 4096 && !text.chars().any(char::is_control))
            .ok_or("queue scheduler unavailable")?;
        let mut candidates = tasks.iter().collect::<Vec<_>>();
        for candidate in &candidates {
            bounded_text(candidate, "task_id")?;
            bounded_text(candidate, "principal")?;
            bounded_text(candidate, "workflow")?;
            bounded_text(candidate, "workflow_identity")?;
        }
        // Rotation applies within this observed page. Cursor traversal and the
        // native authority retain their separate concurrency/fairness contracts.
        candidates.sort_by(|left, right| {
            let left = left["principal"].as_str().unwrap();
            let right = right["principal"].as_str().unwrap();
            (left <= last, left).cmp(&(right <= last, right))
        });
        pages += 1;
        for (index, candidate) in candidates.iter().enumerate() {
            let task_id = bounded_text(candidate, "task_id")?;
            if !seen.insert(task_id.to_owned()) {
                return Err("repeated task in queue traversal".into());
            }
            scanned += 1;
            if candidate["workflow"] != turn.workflow
                || candidate["workflow_identity"] != turn.workflow_identity
            {
                continue;
            }
            let claimable = authority.query("task.claimable", &json!({"task":task_id}))?;
            match claimable.as_bool() {
                Some(false) => continue,
                Some(true) => (),
                None => return Err("invalid queue eligibility result".into()),
            }
            let admission = authority.command(
                &command_id(turn, task_id, "claim"),
                "task.claim",
                &json!({"owner":turn.owner,"task":task_id}),
            )?;
            if admission["replayed"] == true {
                replayed_claims += 1;
                continue;
            }
            if admission["replayed"] != false {
                return Err("invalid queue claim disposition".into());
            }
            let claimed = admission
                .get("receipt")
                .ok_or("queue claim receipt unavailable")?;
            if claimed.is_null() {
                continue;
            }
            if claimed["task_id"] != task_id
                || claimed["principal"] != candidate["principal"]
                || claimed["workflow"] != turn.workflow
                || claimed["workflow_identity"] != turn.workflow_identity
                || claimed["owner"] != turn.owner
                || !claimed["input"].is_object()
            {
                return Err("claimed task differs from frozen queue identity".into());
            }
            bounded_text(claimed, "run_id")?;
            claims += 1;
            let (state, result, error) = match dispatch(authority, claimed) {
                Ok(DispatchOutcome::Completed(result)) if result.is_object() => {
                    ("completed", result, Value::Null)
                }
                _ => (
                    "unknown",
                    Value::Null,
                    json!({"code":"native_dispatch_unestablished"}),
                ),
            };
            // Unknown settlement ends this scheduling attempt. Native invocation
            // identity, charges and reconciliation obligations remain durable.
            let settlement = authority.command(
                &command_id(turn, task_id, "complete"),
                "task.complete",
                &json!({"principal":claimed["principal"],"task":task_id,
                    "owner":turn.owner,"state":state,"result":result,"error":error}),
            )?;
            let settlement = settlement
                .get("receipt")
                .ok_or("queue settlement receipt unavailable")?;
            if settlement["task_id"] != task_id
                || settlement["principal"] != claimed["principal"]
                || !matches!(
                    settlement["state"].as_str(),
                    Some("completed" | "failed" | "cancelled" | "unknown")
                )
            {
                return Err("queue settlement differs from original task".into());
            }
            summaries.push(json!({"task_id":task_id,"run_id":claimed["run_id"],
                "requested_state":state,"state":settlement["state"]}));
            if claims == turn.max_claims && index + 1 < candidates.len() {
                // The public page does not expose each item's sequence. Revisit
                // this page rather than skipping any unexamined candidate.
                after = page_start;
                break 'pages;
            }
        }
        if let Some(next) = next {
            after = next;
        } else {
            tail_seen = true;
            after = 0;
            break;
        }
    }
    Ok(
        json!({"version":"mas-host-shared-turn/1","turn_id":turn.turn_id,
        "pages_examined":pages,"candidates_examined":scanned,"fresh_claims":claims,
        "replayed_claims_skipped":replayed_claims,"next_after_seq":after,
        "candidate_snapshot_tail_seen":tail_seen,"global_queue_exhaustion_established":false,
        "task_summaries":summaries}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn turn() -> Turn<'static> {
        Turn {
            owner: "scheduler",
            turn_id: "turn-one",
            after_seq: 0,
            page_size: 1000,
            max_pages: 2,
            max_claims: 1,
            workflow: "native",
            workflow_identity: IDENTITY,
        }
    }
    const IDENTITY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fn candidate(index: usize, principal: &str) -> Value {
        json!({"task_id":format!("task-{index}"),"principal":principal,
            "workflow":"native","workflow_identity":IDENTITY})
    }
    #[derive(Default)]
    struct Fixture {
        pages: BTreeMap<i64, Value>,
        eligible: BTreeSet<String>,
        commands: Vec<(String, Value)>,
        replay_claim: bool,
        fail_claim: bool,
        bad_identity: bool,
        expiry_at_completion: bool,
        dispatched: usize,
    }
    impl Authority for Fixture {
        fn query(&mut self, operation: &str, p: &Value) -> Result<Value, String> {
            match operation {
                "task.pending" => self
                    .pages
                    .get(&p["after"].as_i64().unwrap())
                    .cloned()
                    .ok_or("missing page".into()),
                "task.scheduler" => Ok(json!({"last_principal":"a"})),
                "task.claimable" => Ok(json!(self.eligible.contains(
                    p["task"]
                        .as_str()
                        .expect("specific task eligibility required")
                ))),
                _ => Err("unexpected query".into()),
            }
        }
        fn command(&mut self, _id: &str, operation: &str, p: &Value) -> Result<Value, String> {
            self.commands.push((operation.into(), p.clone()));
            match operation {
                "task.claim" if self.fail_claim => Err("lost acknowledgement".into()),
                "task.claim" => {
                    let id = p["task"].as_str().unwrap();
                    let task = self
                        .pages
                        .values()
                        .flat_map(|v| v["tasks"].as_array().unwrap())
                        .find(|task| task["task_id"] == id)
                        .unwrap();
                    let mut task = task.clone();
                    task["run_id"] = json!(format!("run-{id}"));
                    task["owner"] = p["owner"].clone();
                    task["input"] = json!({});
                    if self.bad_identity {
                        task["principal"] = json!("another-principal");
                    }
                    Ok(json!({"replayed":self.replay_claim,"receipt":task}))
                }
                "task.complete" => Ok(json!({"replayed":false,"receipt":{
                    "task_id":p["task"],"principal":p["principal"],
                    "state":if self.expiry_at_completion {json!("failed")} else {p["state"].clone()}}})),
                _ => Err("unexpected command".into()),
            }
        }
    }
    fn complete(fixture: &mut Fixture, _: &Value) -> Result<DispatchOutcome, String> {
        fixture.dispatched += 1;
        Ok(DispatchOutcome::Completed(json!({"checked":true})))
    }
    #[test]
    fn queue_pages_reach_eligible_task_beyond_one_thousand_blocked_candidates() {
        let mut f = Fixture::default();
        f.pages.insert(
            0,
            json!({"tasks":(0..1000).map(|i|candidate(i,"a")).collect::<Vec<_>>(),"next":1000}),
        );
        f.pages
            .insert(1000, json!({"tasks":[candidate(1000,"z")],"next":null}));
        f.eligible.insert("task-1000".into());
        let result = drain_using(&mut f, &turn(), complete).unwrap();
        assert_eq!(result["candidates_examined"], 1001);
        assert_eq!(result["fresh_claims"], 1);
        assert_eq!(f.dispatched, 1);
        assert_eq!(f.commands[0].1["task"], "task-1000");
        assert_eq!(result["global_queue_exhaustion_established"], false);
    }
    #[test]
    fn bounded_turn_returns_cursor_and_resume_reaches_later_page() {
        let mut f = Fixture::default();
        f.pages
            .insert(0, json!({"tasks":[candidate(0,"a")],"next":1}));
        f.pages
            .insert(1, json!({"tasks":[candidate(1,"z")],"next":null}));
        f.eligible.insert("task-1".into());
        let mut options = turn();
        options.max_pages = 1;
        let first = drain_using(&mut f, &options, complete).unwrap();
        assert_eq!(first["next_after_seq"], 1);
        assert_eq!(f.dispatched, 0);
        options.after_seq = 1;
        assert_eq!(
            drain_using(&mut f, &options, complete).unwrap()["fresh_claims"],
            1
        );
    }
    #[test]
    fn principal_rotation_and_partial_page_do_not_skip_unexamined_tasks() {
        let mut f = Fixture::default();
        f.pages.insert(
            0,
            json!({"tasks":[candidate(0,"a"),candidate(1,"z")],"next":null}),
        );
        f.eligible.extend(["task-0".into(), "task-1".into()]);
        let result = drain_using(&mut f, &turn(), complete).unwrap();
        assert_eq!(f.commands[0].1["task"], "task-1");
        assert_eq!(result["next_after_seq"], 0);
        assert_eq!(result["candidate_snapshot_tail_seen"], false);
    }
    #[test]
    fn replayed_or_uncertain_claim_never_dispatches() {
        for replay in [true, false] {
            let mut f = Fixture::default();
            f.pages
                .insert(0, json!({"tasks":[candidate(0,"a")],"next":null}));
            f.eligible.insert("task-0".into());
            f.replay_claim = replay;
            f.fail_claim = !replay;
            let result = drain_using(&mut f, &turn(), complete);
            assert_eq!(f.dispatched, 0);
            assert_eq!(f.commands.len(), 1);
            if replay {
                assert_eq!(result.unwrap()["replayed_claims_skipped"], 1);
            } else {
                assert!(result.is_err());
            }
        }
    }
    #[test]
    fn changed_claim_identity_never_dispatches() {
        let mut f = Fixture::default();
        f.pages
            .insert(0, json!({"tasks":[candidate(0,"a")],"next":null}));
        f.eligible.insert("task-0".into());
        f.bad_identity = true;
        assert!(drain_using(&mut f, &turn(), complete).is_err());
        assert_eq!(f.dispatched, 0);
    }
    #[test]
    fn failed_dispatch_is_unknown_and_never_implicitly_retried() {
        let mut f = Fixture::default();
        f.pages
            .insert(0, json!({"tasks":[candidate(0,"a")],"next":null}));
        f.eligible.insert("task-0".into());
        let result = drain_using(&mut f, &turn(), |f, _| {
            f.dispatched += 1;
            Err("private details".into())
        })
        .unwrap();
        assert_eq!(f.dispatched, 1);
        assert_eq!(f.commands[1].1["state"], "unknown");
        assert!(!result.to_string().contains("private details"));
        let mut second = Fixture::default();
        second
            .pages
            .insert(0, json!({"tasks":[candidate(0,"a")],"next":null}));
        second.eligible.insert("task-0".into());
        drain_using(&mut second, &turn(), |_, _| Ok(DispatchOutcome::Unknown)).unwrap();
        assert_eq!(second.commands[1].1["state"], "unknown");
    }
    #[test]
    fn malformed_or_repeated_continuations_stop_before_dispatch() {
        for next in [json!(0), json!("cursor")] {
            let mut f = Fixture::default();
            f.pages
                .insert(0, json!({"tasks":[candidate(0,"a")],"next":next}));
            assert!(drain_using(&mut f, &turn(), complete).is_err());
            assert_eq!(f.dispatched, 0);
        }
    }
    #[test]
    fn final_authority_deadline_failure_controls_the_task_summary() {
        let mut f = Fixture::default();
        f.pages
            .insert(0, json!({"tasks":[candidate(0,"a")],"next":null}));
        f.eligible.insert("task-0".into());
        f.expiry_at_completion = true;
        let result = drain_using(&mut f, &turn(), complete).unwrap();
        assert_eq!(result["task_summaries"][0]["requested_state"], "completed");
        assert_eq!(result["task_summaries"][0]["state"], "failed");
    }
    #[test]
    fn deadline_cap_uses_evaluated_authority_budgets_and_rejects_missing_data() {
        let mut task = json!({"clock":{"format":"mas-execution-clock/2","phase":"running","active_branches":1,"worker_time_complete":true},
            "timing":{"phase":"running","expired":[],"business_expires_ms":160000,"observed_at_ms":120000,"remaining_active_ms":30000}});
        assert_eq!(task_deadline_ms(&task).unwrap(), 150000);
        task["timing"]["business_expires_ms"] = json!(140000);
        assert_eq!(task_deadline_ms(&task).unwrap(), 140000);
        task["timing"]["remaining_active_ms"] = Value::Null;
        assert!(task_deadline_ms(&task).is_err());
        task["timing"]["remaining_active_ms"] = json!(30000);
        task["clock"]["active_branches"] = json!(2);
        assert!(task_deadline_ms(&task).is_err());
    }
}
