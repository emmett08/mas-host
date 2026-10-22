#!/usr/bin/env python3
"""Execute bounded SPIN controls, mutations and reachability witnesses."""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import shutil
import subprocess
import tempfile
import time
import zipfile

ROOT = Path(__file__).resolve().parent
DEPTH = 100_000
SECONDS = 30
MEMORY_BYTES = 512 * 1024 * 1024
CASES = []
for model, claim in (("revision", "settles"), ("disclosure", "context_finishes"),
                     ("delegation", "delegation_finishes"),
                     ("provider-results", "provider_results_settle"),
                     ("governed-local", "governed_local_settles"),
                     ("privacy-response", "privacy_response_settles"),
                     ("retained-effect", "retained_effect_finishes"),
                     ("exposure-lineage", "exposure_lineage_finishes"),
                     ("privacy-lifecycle", "privacy_lifecycle_finishes"),
                     ("shared-native", "shared_native_finishes")):
    for ltl in (False, True):
        for no_reduce in (False, True):
            CASES.append((model, f"{'progress' if ltl else 'safety'}{'-no-reduction' if no_reduce else ''}",
                          None, claim if ltl else None, no_reduce, None))
for name, macro, assertion in (
    ("unknown-retry", "BUG_UNKNOWN_RETRY", "unknown_seen"),
    ("replay", "BUG_REPLAY", "attempts<=2"),
    ("stale-authority", "BUG_STALE_AUTH", "rights"),
    ("early-retry", "BUG_EARLY_RETRY", "now>=due"),
    ("ack-certainty", "BUG_ACK_CERTAINTY", "acknowledgement"),
    ("escalation-identity", "BUG_ESCALATION_ID", "action!=retained_action"),
    ("review-binding", "BUG_REVIEW_BINDING", "exact_review"),
    ("review-release", "BUG_REVIEW_RELEASE", "release_permission"),
    ("accepted-witness", "WITNESS_ACCEPTED", "accepted"),
    ("escalated-witness", "WITNESS_ESCALATED", "escalated"),
    ("unknown-witness", "WITNESS_UNKNOWN", "unknown_seen"),
):
    CASES.append(("privacy-lifecycle", name, macro, None, False, assertion))
for name, macro, assertion in (
    ("unknown-retry", "BUG_UNKNOWN_RETRY", "effect_dispatches"),
    ("completed-replay", "BUG_REPLAY", "effect_dispatches"),
    ("assessment-replay", "BUG_ASSESSMENT_REPLAY", "decision"),
    ("error-pass", "BUG_ERROR_PASS", "terminal!=Completed"),
    ("revision-budget", "BUG_BUDGET", "revisions<=1"),
    ("permission-expansion", "BUG_PERMISSION", "permitted_effects==1"),
    ("deadline-expansion", "BUG_DEADLINE", "active_deadline==frozen_deadline"),
    ("remediation-effect", "BUG_REMEDIATION", "remediation_effects==0"),
    ("complete-witness", "WITNESS_COMPLETE", "terminal!=Completed"),
    ("replay-witness", "WITNESS_REPLAY", "!(replayed)"),
    ("unknown-witness", "WITNESS_UNKNOWN", "!(unknown_seen)"),
    ("exhausted-witness", "WITNESS_EXHAUSTED", "terminal!=Exhausted"),
    ("error-witness", "WITNESS_ERROR", "!(retained_error)"),
    ("assessment-replay-witness", "WITNESS_ASSESSMENT_REPLAY", "!(assessment_replayed)"),
    ("invocation-budget-witness", "WITNESS_INVOCATION_BUDGET", "!(budget_exhausted)"),
):
    CASES.append(("revision", name, macro, None, False, assertion))
CASES.append(("revision", "stalled-controller", "BUG_STALL", "settles", False, "acceptance cycle"))
for name, macro, assertion in (
    ("hidden-peer", "BUG_HIDDEN_PEER", "reviewer_input_a==reviewer_input_b"),
    ("reassemble-replay", "BUG_REASSEMBLE", "reviewer_input_a==snapshot_a"),
    ("policy-rebind", "BUG_POLICY_REBIND", "rejected_rebind"),
    ("derived-reader", "BUG_DERIVED_READER", "!(output_reader)"),
    ("restricted-witness", "WITNESS_RESTRICTED", "!(supplied_feedback)"),
    ("delegation-disclosure", "BUG_DELEGATION_DISCLOSURE", "!(delegated)"),
    ("delegation-inheritance", "BUG_DELEGATION_INHERITANCE", "!(inherited_reader)"),
    ("evidence-namespace", "BUG_EVIDENCE_NAMESPACE", "!(draft_dispatched)"),
    ("evidence-readers", "BUG_EVIDENCE_READERS", "!(candidate_evidence_reader)"),
):
    CASES.append(("disclosure", name, macro, None, False, assertion))
for name, macro, assertion in (
    ("early-complete", "BUG_EARLY_COMPLETE", "successful>=minimum"),
    ("early-complete-schema", "BUG_SCHEMA", "complete_allowed=="),
    ("partial-coverage-complete", "BUG_PARTIAL_COMPLETE", "covered&required"),
    ("required-target-schema", "BUG_TARGET_SCHEMA", "permitted_requests=="),
    ("duplicate-target-schema", "BUG_DUPLICATE_TARGET_SCHEMA", "permitted_requests=="),
    ("final-turn-request", "BUG_FINAL_REQUEST", "request_allowed=="),
    ("replay-count", "BUG_REPLAY_COUNT", "successful==retained_successful"),
    ("replay-coverage", "BUG_REPLAY_COVERAGE", "covered==retained_covered"),
    ("replay-dispatch", "BUG_REPLAY_DISPATCH", "peer_dispatches==saved_peer_dispatches"),
    ("complete-witness", "WITNESS_COMPLETE", "terminal==Completed"),
    ("zero-minimum-witness", "WITNESS_ZERO", "minimum==0"),
    ("replay-witness", "WITNESS_REPLAY", "replayed"),
    ("early-invalid-witness", "WITNESS_EARLY_INVALID", "terminal==Invalid"),
    ("partial-coverage-invalid-witness", "WITNESS_PARTIAL_INVALID", "successful==1"),
    ("duplicate-target-invalid-witness", "WITNESS_DUPLICATE_INVALID", "duplicate_rejected"),
    ("reverse-order-witness", "WITNESS_REVERSE_ORDER", "first_peer==2"),
):
    CASES.append(("delegation", name, macro, None, False, assertion))
CASES.append(("delegation", "stalled-inference", "BUG_STALL", "delegation_finishes",
              False, "acceptance cycle"))
for name, macro, assertion in (
    ("first-output", "BUG_FIRST_OUTPUT", "selected=="),
    ("ignore-malformed-sibling", "BUG_MALFORMED_SIBLING", "selected=="),
    ("commentary-promotion", "BUG_COMMENTARY", "selected=="),
    ("ignore-refusal", "BUG_REFUSAL", "selected=="),
    ("reject-equivalent-messages", "BUG_REJECT_REPEATED", "selected=="),
    ("skip-schema", "BUG_SCHEMA", "peer_calls==prior_peer_calls"),
    ("skip-request-admission", "BUG_REQUEST_ADMISSION", "peer_calls==prior_peer_calls"),
    ("known-invalid-becomes-unknown", "BUG_INVALID_UNKNOWN", "inference_state==Known"),
    ("inference-replay", "BUG_REPLAY", "inference_calls"),
    ("peer-replay", "BUG_PEER_REPLAY", "peer_calls"),
    ("invalid-retry", "BUG_INVALID_RETRY", "inference_calls"),
    ("unknown-retry", "BUG_UNKNOWN_RETRY", "inference_calls"),
    ("reclassify-retained-invalid", "BUG_RECLASSIFY", "retained_status==sealed_status"),
    ("descriptor-rebind", "BUG_DESCRIPTOR_REBIND", "frozen_descriptor"),
    ("single-witness", "WITNESS_SINGLE", "response_kind==Single"),
    ("repeated-witness", "WITNESS_REPEATED", "response_kind==Repeated"),
    ("triplicated-witness", "WITNESS_TRIPLICATED", "response_kind==Triplicated"),
    ("conflict-after-peer-witness", "WITNESS_CONFLICT_AFTER_PEER", "response_kind==Conflicting"),
    ("malformed-witness", "WITNESS_MALFORMED", "response_kind==MalformedSibling"),
    ("schema-invalid-witness", "WITNESS_SCHEMA_INVALID", "response_kind==RepeatedSchemaInvalid"),
    ("request-invalid-witness", "WITNESS_REQUEST_INVALID", "response_kind==RequestInvalid"),
    ("refusal-witness", "WITNESS_REFUSAL", "response_kind==Refused"),
    ("unknown-witness", "WITNESS_UNKNOWN", "terminal==Blocked"),
    ("lost-known-witness", "WITNESS_LOST_KNOWN", "transient_known_lost"),
    ("cached-inference-witness", "WITNESS_CACHED_INFERENCE", "cached_inference"),
    ("delegate-crash-witness", "WITNESS_DELEGATE_CRASH", "delegate_state==Uncertain"),
    ("descriptor-rebind-witness", "WITNESS_REBIND", "!(rebind_rejected)"),
):
    CASES.append(("provider-results", name, macro, None, False, assertion))
CASES.append(("provider-results", "stalled-provider", "BUG_STALL", "provider_results_settle",
              False, "acceptance cycle"))
for name, macro, assertion in (
    ("raw-input", "BUG_RAW_INPUT", "journal_input!=Raw"),
    ("raw-completion", "BUG_RAW_COMPLETION", "journal_output!=Raw"),
    ("raw-protocol", "BUG_RAW_PROTOCOL", "protocol_output!=Raw"),
    ("stale-effect-authority", "BUG_STALE_EFFECT_AUTHORITY", "!(effect_check)"),
    ("stale-delivery-authority", "BUG_STALE_DELIVERY_AUTHORITY", "!(delivery_check)"),
    ("certainty-promotion", "BUG_CERTAINTY_PROMOTION", "retained_certainty==observed_certainty"),
    ("erase-completion", "BUG_ERASE_COMPLETION", "receipt&&(effect_state==Completed)"),
    ("borrowed-receipt", "BUG_BORROWED_RECEIPT", "forwarded_certainty==0"),
    ("foreign-tenant", "BUG_FOREIGN_TENANT", "attempt_tenant==1"),
    ("original-rebind", "BUG_ORIGINAL_REBIND", "bound_original==1"),
    ("unknown-retry", "BUG_UNKNOWN_RETRY", "calls==sealed_calls"),
    ("completed-retry", "BUG_COMPLETED_RETRY", "calls<=1"),
    ("inspection-coverage", "BUG_INSPECTION_COVERAGE", "inspection_complete"),
    ("inspection-binding", "BUG_INSPECTION_BINDING", "inspection_exact"),
    ("inspection-finding", "BUG_INSPECTION_FINDING", "inspection_clean"),
    ("inspection-revocation", "BUG_INSPECTION_REVOCATION", "!(inspection_postcheck)"),
    ("outbound-inspection", "BUG_OUTBOUND_INSPECTION", "outbound_inspected"),
    ("available-witness", "WITNESS_AVAILABLE", "terminal==Delivered"),
    ("withheld-witness", "WITNESS_WITHHELD", "retained_certainty==1"),
    ("replay-witness", "WITNESS_REPLAY", "!(replayed)"),
    ("unknown-witness", "WITNESS_UNKNOWN", "!(unknown_seen)"),
    ("restart-repair-witness", "WITNESS_RESTART_REPAIR", "protocol_completed"),
    ("revoked-completion-witness", "WITNESS_REVOKED_COMPLETION", "!(completed_while_revoked)"),
    ("input-conflict-witness", "WITNESS_INPUT_CONFLICT", "!(input_conflict)"),
    ("foreign-denial-witness", "WITNESS_FOREIGN_DENIAL", "!(foreign_denied)"),
    ("snapshot-race-witness", "WITNESS_SNAPSHOT_RACE", "!(check_action_race)"),
    ("epoch-refresh-witness", "WITNESS_EPOCH_REFRESH", "epoch_refreshed"),
    ("positive-original-witness", "WITNESS_POSITIVE_ORIGINAL", "positive_original_projected"),
    ("inspection-denial-witness", "WITNESS_INSPECTION_DENIAL", "!(inspection_denied)"),
    ("inspection-withholding-witness", "WITNESS_INSPECTION_WITHHOLDING", "inspection_withheld"),
    ("unknown-retry-witness", "WITNESS_UNKNOWN_RETRY", "!(unknown_retry_blocked)"),
):
    CASES.append(("governed-local", name, macro, None, False, assertion))
CASES.append(("governed-local", "stalled-external", "BUG_STALL", "governed_local_settles",
              False, "acceptance cycle"))
for name, macro, assertion in (
    ("untrusted-route", "BUG_UNTRUSTED_ROUTE", "route==required_route"),
    ("intent-order", "BUG_INTENT_ORDER", "intent_receipt&&intent_exposed"),
    ("withheld-intent", "BUG_WITHHELD_INTENT", "intent_receipt&&intent_exposed"),
    ("revoked-notification", "BUG_REVOKED_NOTIFY", "!(admission_check)"),
    ("unknown-retry", "BUG_UNKNOWN_RETRY", "notification_calls==sealed_notification_calls"),
    ("completed-retry", "BUG_COMPLETED_RETRY", "notification_calls<=1"),
    ("withheld-retry", "BUG_WITHHELD_RETRY", "notification_calls<=1"),
    ("throttled-retry", "BUG_THROTTLE_RETRY", "notification_calls<=1"),
    ("throttled-success", "BUG_THROTTLED_SUCCESS", "provider_outcome==sealed_outcome"),
    ("recipient-certainty", "BUG_RECIPIENT_CERTAINTY", "!(recipient_delivery_known)"),
    ("event-rebind", "BUG_EVENT_REBIND", "frozen_event==1"),
    ("destination-rebind", "BUG_DESTINATION_REBIND", "frozen_destination==1"),
    ("replay-identity", "BUG_REPLAY_IDENTITY", "attempt_tenant==1"),
    ("accepted-witness", "WITNESS_ACCEPTED", "provider_outcome==Accepted"),
    ("refused-witness", "WITNESS_REFUSED", "provider_outcome==Refused"),
    ("throttled-witness", "WITNESS_THROTTLED", "provider_outcome==Throttled"),
    ("unknown-witness", "WITNESS_UNKNOWN", "notification_state!=Unknown"),
    ("between-effects-witness", "WITNESS_BETWEEN_EFFECTS", "crashed_between_effects"),
    ("withheld-completion-witness", "WITNESS_WITHHELD_COMPLETION", "payload_withheld"),
    ("withheld-intent-witness", "WITNESS_WITHHELD_INTENT", "notification_calls==0"),
    ("revocation-witness", "WITNESS_REVOCATION", "!(revoked_before_notification)"),
    ("replay-witness", "WITNESS_REPLAY", "terminal==Settled"),
    ("event-conflict-witness", "WITNESS_EVENT_CONFLICT", "!(conflict_rejected)"),
    ("foreign-denial-witness", "WITNESS_FOREIGN_DENIAL", "!(foreign_rejected)"),
    ("not-sent-witness", "WITNESS_NOT_SENT", "provider_outcome==NotSent"),
    ("destination-conflict-witness", "WITNESS_DESTINATION_CONFLICT", "!(destination_conflict)"),
    ("unknown-retry-witness", "WITNESS_UNKNOWN_RETRY", "!(unknown_retry_blocked)"),
):
    CASES.append(("privacy-response", name, macro, None, False, assertion))
CASES.append(("privacy-response", "stalled-endpoint", "BUG_STALL", "privacy_response_settles",
              False, "acceptance cycle"))
for name, macro, assertion in (
    ("namespace-binding", "BUG_NAMESPACE", "namespace_verified"),
    ("parent-binding", "BUG_PARENT", "parent_verified"),
    ("actor-binding", "BUG_ACTOR", "actor_verified"),
    ("key-binding", "BUG_KEY", "returned_key==request_key"),
    ("view-binding", "BUG_VIEW_BINDING", "authorised_view==returned_view"),
    ("pre-lookup-authority", "BUG_PREAUTH", "!(precheck)"),
    ("post-inspection-authority", "BUG_POSTAUTH", "!(final_check)"),
    ("split-snapshot", "BUG_SPLIT_SNAPSHOT", "snap_revision==snap_receipt_revision"),
    ("callback-with-database", "BUG_CALLBACK_WITH_DB", "!(db_open)"),
    ("reader-write", "BUG_READER_WRITE", "reader_writes==0"),
    ("reader-recovery", "BUG_READER_RECOVERY", "reader_recoveries==0"),
    ("reader-dispatch", "BUG_READER_DISPATCH", "reader_dispatches==0"),
    ("inspection-bypass", "BUG_INSPECTION", "result_inspection_ok"),
    ("withheld-settlement", "BUG_WITHHELD_SETTLEMENT", "returned_state==snap_state"),
    ("external-certainty", "BUG_CERTAINTY", "returned_certainty==snap_certainty"),
    ("available-witness", "WITNESS_AVAILABLE", "returned_payload==Available"),
    ("withheld-known-witness", "WITNESS_WITHHELD_KNOWN", "withheld_for_inspection"),
    ("unknown-witness", "WITNESS_UNKNOWN", "returned_state==Unknown"),
    ("not-recorded-witness", "WITNESS_NOT_RECORDED", "returned_state==Absent"),
    ("live-owner-witness", "WITNESS_LIVE_OWNER", "owner_committed"),
    ("old-snapshot-witness", "WITNESS_OLD_SNAPSHOT", "!(observed_older_snapshot)"),
    ("post-inspection-revocation-witness", "WITNESS_REVOKED", "!(post_inspection_revocation)"),
    ("binding-denied-witness", "WITNESS_BINDING_DENIED", "!(binding_denied)"),
    ("query-inspection-denied-witness", "WITNESS_QUERY_DENIED", "!(query_denied)"),
    ("authority-race-witness", "WITNESS_AUTHORITY_RACE", "!(return_after_revocation)"),
):
    CASES.append(("retained-effect", name, macro, None, False, assertion))
CASES.append(("retained-effect", "stalled-inspector", "BUG_STALL", "retained_effect_finishes",
              False, "acceptance cycle"))
for name, macro, assertion in (
    ("truncate-ancestry", "BUG_TRUNCATE_32", "resolved==expected_root"),
    ("invalid-closure", "BUG_INVALID_CLOSURE", "graph_valid"),
    ("live-parallel-context", "BUG_LIVE_PARALLEL_CONTEXT", "child2_context==frozen_batch"),
    ("child-target", "BUG_CHILD_TARGET", "frozen_batch"),
    ("dispatch-before-exposure", "BUG_DISPATCH_BEFORE_EXPOSURE", "child1_committed"),
    ("baseline-inspection", "BUG_BASELINE_INSPECTION", "source_processing_allowed"),
    ("result-processing", "BUG_RESULT_PROCESSING", "result_processing_allowed"),
    ("attach-only-at-completion", "BUG_ATTACH_ONLY_AT_COMPLETION", "nested_calls==0"),
    ("split-pending-commit", "BUG_SPLIT_PENDING_COMMIT", "acquisition!=Prepared"),
    ("reply-before-exposure", "BUG_REPLY_BEFORE_EXPOSURE", "parent_exposure&expected_parent"),
    ("overwrite-exposure", "BUG_OVERWRITE_EXPOSURE", "parent_exposure&expected_parent"),
    ("worker-narrowing", "BUG_WORKER_NARROWING", "expected_parent"),
    ("result-disclosure", "BUG_RESULT_DISCLOSURE", "expected_parent"),
    ("source-rebind", "BUG_SOURCE_REBIND", "returned_binding==attempted_binding"),
    ("unknown-retry", "BUG_UNKNOWN_RETRY", "acquisition_calls==sealed_acquisition_calls"),
    ("completed-retry", "BUG_COMPLETED_RETRY", "action_calls==sealed_action_calls"),
    ("replay-expiry", "BUG_REPLAY_EXPIRY", "now<sealed_expiry"),
    ("erase-completion", "BUG_ERASE_COMPLETION", "action_receipt"),
    ("approved-witness", "WITNESS_APPROVED", "action_receipt"),
    ("inherited-denial-witness", "WITNESS_INHERITED_DENIAL", "restriction_denied"),
    ("late33-witness", "WITNESS_LATE33", "late_ancestor_seen"),
    ("parallel-freeze-witness", "WITNESS_PARALLEL_FREEZE", "frozen_parallel_seen"),
    ("pending-block-witness", "WITNESS_PENDING_BLOCK", "nested_blocked"),
    ("known-withheld-witness", "WITNESS_KNOWN_WITHHELD", "known_withheld"),
    ("unknown-retry-witness", "WITNESS_UNKNOWN_RETRY", "unknown_retry_blocked"),
    ("exposure-crash-witness", "WITNESS_EXPOSURE_CRASH", "crash_between_exposure_reply"),
    ("expiry-witness", "WITNESS_EXPIRY", "expiry_denied"),
    ("invalid-graph-witness", "WITNESS_INVALID_GRAPH", "invalid_graph_denied"),
    ("processing-denied-witness", "WITNESS_PROCESSING_DENIED", "processing_denied"),
    ("current-race-witness", "WITNESS_CURRENT_RACE", "check_action_race"),
    ("replay-witness", "WITNESS_REPLAY", "replayed"),
    ("result-withheld-witness", "WITNESS_RESULT_WITHHELD", "child2_payload_withheld"),
    ("prepared-retry-witness", "WITNESS_PREPARED_RETRY", "prepared_retry_seen"),
    ("result-inspection-witness", "WITNESS_RESULT_INSPECTION", "result_inspection_withheld"),
    ("child-denial-witness", "WITNESS_CHILD_DENIAL", "child_target_denied"),
):
    CASES.append(("exposure-lineage", name, macro, None, False, assertion))
CASES.append(("exposure-lineage", "stalled-acquisition", "BUG_STALL", "exposure_lineage_finishes",
              False, "acceptance cycle"))


for name, macro, assertion in (
    ("replay-dispatch", "BUG_REPLAY_DISPATCH", "dispatches<=1"),
    ("unknown-retry", "BUG_UNKNOWN_RETRY", "allocations<=1"),
    ("stale-completion", "BUG_STALE_COMPLETE", "epoch==admitted_epoch"),
    ("erase-completion", "BUG_ERASE_COMPLETION", "state==Completed"),
    ("complete-witness", "WITNESS_COMPLETE", "!(completed_seen)"),
    ("unknown-witness", "WITNESS_UNKNOWN", "!(unknown_seen)"),
    ("lost-admission-witness", "WITNESS_LOST_ADMISSION", "!(reply_lost)"),
    ("fenced-witness", "WITNESS_FENCED", "!(stale_completion_blocked)"),
):
    CASES.append(("shared-native", name, macro, None, False, assertion))

def bounded() -> None:
    resource.setrlimit(resource.RLIMIT_AS, (MEMORY_BYTES, MEMORY_BYTES))


def command(argv: list[str], cwd: Path, logs: list[dict]) -> str:
    started = time.monotonic()
    entry = {"argv": argv}
    logs.append(entry)
    try:
        result = subprocess.run(argv, cwd=cwd, text=True, capture_output=True,
                                timeout=SECONDS, preexec_fn=bounded)
    except subprocess.TimeoutExpired as error:
        entry.update(exit_code=None, timed_out=True, elapsed_seconds=time.monotonic() - started,
                     output=str(error.stdout or "") + str(error.stderr or ""))
        raise
    output = result.stdout + result.stderr
    entry.update(exit_code=result.returncode, output=output,
                 elapsed_seconds=time.monotonic() - started)
    if result.returncode:
        raise RuntimeError(f"{argv[0]} exited {result.returncode}")
    return output


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    started = time.monotonic()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "evidence")
    parser.add_argument("--model", action="append", choices=sorted({case[0] for case in CASES}),
                        help="check only this model; repeat to select several")
    args = parser.parse_args()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=True)
    output_lock = (output / ".verification.lock").open("a")
    try:
        fcntl.flock(output_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        parser.error("another verifier owns this output directory; choose another --out")
    summary = {"schema": "mas-host-assessment-formal/1", "status": "RUNNING_OR_INTERRUPTED",
               "model_sha256": {p.name: sha(p) for p in sorted(ROOT.glob("*.pml"))},
               "runner_sha256": sha(Path(__file__)),
               "limits": {"seconds_per_command": SECONDS, "memory_bytes": MEMORY_BYTES,
                          "search_depth": DEPTH},
               "bounds": {"runs": 1, "candidates": 2, "invocations": 4,
                          "revisions": 1, "disruptions": 1, "disclosure_worlds": 2,
                          "delegation": {"inference_turns": 3, "permitted_peers": 2,
                                         "required_peer_subsets": 4,
                                         "minimum_calls": [0, 1, 2], "replays": 1},
                          "provider_results": {"inference_effects": 1, "messages": 3,
                                               "downstream_effects": 1,
                                               "retained_earlier_peers": [0, 1],
                                               "crashes": 1, "terminal_replays": 1},
                          "governed_local": {"invocations": 1, "mediated_effects": 1,
                                             "tenants": 2, "original_classes": 2,
                                             "request_channel_capacity": 1,
                                             "result_channel_capacity": 1,
                                             "authority_updates": 1, "crashes": 1,
                                             "explicit_core_retries": 1,
                                             "crash_recovery_cached_passes": 1,
                                             "maximum_cached_passes": 2},
                          "privacy_response": {"case_intents": 1, "notification_effects": 1,
                                               "tenants": 2, "event_classes": 2,
                                               "destination_classes": 2,
                                               "route_classes": 3,
                                               "endpoint_request_channel_capacity": 1,
                                               "endpoint_reply_channel_capacity": 1,
                                               "revocations": 1, "crashes": 1,
                                               "explicit_retries": 1,
                                               "maximum_cached_passes": 2},
                          "retained_effect": {"readers": 1, "live_owners": 1,
                                              "owner_commits": 1, "authority_updates": 1,
                                              "namespace_classes": 2, "parent_classes": 2,
                                              "actor_classes": 2, "logical_key_classes": 2,
                                              "effect_states": 5, "external_certainties": 4},
                          "exposure_lineage": {"parents": 1, "parallel_children": 2,
                                               "acquisition_effects": 1, "downstream_effects": 1,
                                               "nested_pending_attempts": 1,
                                               "source_policy_bits": 2,
                                               "child_target_classes": 2,
                                               "effect_target_classes": 2,
                                               "result_disclosure_tuple_classes": 2,
                                               "closure_classes": 6,
                                               "closure_size_classes": [1, 33, 65],
                                               "actual_closure_limit": 64,
                                               "traversed_late_ancestor": 33,
                                               "source_expiry_classes": [2, 3],
                                               "authority_or_time_updates": 1, "crashes": 1,
                                               "execute_transaction_rollbacks": 1,
                                               "prepared_admission_retries": 1,
                                               "explicit_cached_retries": 1}},
               "assumptions": [
                   "One host owner serialises this run and immutable journal writes are atomic",
                   "A provider invocation terminates or reaches its fixed external deadline",
                   "At most one crash, cancellation or expiry observation; storage survives",
                   "An interrupted started external effect is unresolved and not retried; pure handler computation may restart",
                   "Deadline values are abstract observations; no wall-clock or scheduling bound is proved",
                   "Two disclosure worlds differ only in withheld peer findings",
                   "Disclosure applies only to host-managed inputs under the trusted-worker boundary",
                   "Provider result values abstract exact parsed JSON; schema and request admissibility are separate inputs",
                   "Provider result admission covers one current logical decision, not semantic deduplication across turns",
                   "Governed local authority checks are file-snapshot checks, not linearised with later commits or dispatch",
                   "Governed replay is an explicit core retry; the A2A service does not automatically resume unknown tasks",
                   "Governed representation classes abstract JSON projection, exact HMAC/tenant comparisons and native framing",
                   "Privacy response routes and safe event classes abstract the bounded adapter parser and trusted configuration",
                   "Notification endpoint acceptance establishes submission only; recipient delivery remains unknown",
                   "Privacy response has no automated retry/reconciliation/escalation scheduler or shared case owner",
                   "Retained-effect reads use one SQLite snapshot and release it before inspection or authority callbacks",
                   "The retained-effect reader performs no recovery, storage writes or worker/effect dispatch",
                   "An authorised missing effect is not-recorded; foreign or missing parent and denied reads are unavailable",
                   "Exposure lineage represents opt-in configuration v2 with baseline plus registered source restrictions",
                   "A sealed pending acquisition blocks new outbound work before execute permission and survives uncertainty",
                   "Parallel child input contexts share one immutable parent snapshot; later recipient exposure only grows",
                   "Exposure graph classes abstract exact DAG/MAC/schema/byte validation; the late restriction is traversed at position 33",
                   "Source expiry is the original minimum and cached delivery cannot renew it; current authority checks remain snapshots",
                   "No unmediated files, provider memory, traffic timing, hash collision or storage adversary",
                   "No fairness options, approximate hashing or disabled bounds/assertion checks",
                   "This is finite control checking plus implementation correspondence, not a refinement proof",
               ], "checks": []}
    summary_path = output / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2) + "\n")
    spin = shutil.which("spin")
    compiler = shutil.which(os.environ.get("CC", "cc"))
    if not spin or not compiler:
        summary.update(status="NOT_EXECUTED", reason="SPIN and a C compiler are required")
        summary_path.write_text(json.dumps(summary, indent=2) + "\n")
        return 1
    logs = {"versions": []}
    summary["spin"] = command([spin, "-V"], ROOT, logs["versions"]).strip()
    summary["compiler"] = command([compiler, "--version"], ROOT, logs["versions"]).splitlines()[0]
    archive_workspace = tempfile.TemporaryDirectory(prefix="mas-host-spin-evidence-")
    archive_path = Path(archive_workspace.name) / "counterexamples.zip"
    with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED):
        pass
    selected_cases = [case for case in CASES if not args.model or case[0] in args.model]
    summary["selected_models"] = sorted({case[0] for case in selected_cases})
    for model, name, macro, claim, no_reduce, expected in selected_cases:
        key = f"{model}-{name}"
        entries = logs[key] = []
        check = {"name": key, "model": f"{model}.pml", "macro": macro,
                 "claim": claim, "partial_order_reduction": not no_reduce,
                 "expected": "counterexample" if expected else "zero_errors"}
        try:
            with tempfile.TemporaryDirectory(prefix="mas-host-spin-") as directory:
                work = Path(directory)
                source = f"{model}.pml"
                shutil.copyfile(ROOT / source, work / source)
                defines = [f"-D{macro}"] if macro else []
                if name == "safety":
                    for seed in (17, 29):
                        command([spin, f"-n{seed}", "-u1000", "-p", *defines, source], work, entries)
                command([spin, *defines, "-a", source], work, entries)
                flags = ["-O2", "-DMEMLIM=512"]
                if not claim:
                    flags += ["-DSAFETY", "-DNOCLAIM"]
                if no_reduce:
                    flags += ["-DNOREDUCE"]
                command([compiler, *flags, "-o", "pan", "pan.c"], work, entries)
                options = ["-a", "-N", claim] if claim else []
                options += ["-b", f"-m{DEPTH}", "-w20"]
                result = command([str(work / "pan"), *options], work, entries)
                errors = re.search(r"errors:\s*(\d+)", result)
                if not errors:
                    raise RuntimeError("verifier supplied no error count")
                if any(t in result for t in ("max search depth too small", "out of memory")):
                    raise RuntimeError("search depth or memory exhausted")
                if not expected and "Search not completed" in result:
                    raise RuntimeError("positive search was incomplete")
                if expected:
                    if int(errors[1]) == 0:
                        raise RuntimeError("mutation/witness did not produce a counterexample")
                    replay = command([spin, *defines, "-t", "-p", "-g", "-l", source], work, entries)
                    if claim:
                        if "acceptance cycle" not in result or "START OF CYCLE" not in replay:
                            raise RuntimeError("expected liveness cycle did not replay")
                    else:
                        failed = re.search(r"text of failed assertion: (.+)", replay)
                        if not failed or expected not in re.sub(r"\s+", "", failed[1]):
                            raise RuntimeError(f"counterexample missed intended assertion: {expected}")
                    trail = work / f"{source}.trail"
                    if not trail.exists():
                        raise RuntimeError("no replayable trail")
                    with zipfile.ZipFile(archive_path, "a", zipfile.ZIP_DEFLATED) as archive:
                        for file in [work / source, trail, *sorted(work.glob("pan.*"))]:
                            archive.write(file, f"{key}/{file.name}")
                        archive.writestr(f"{key}/configuration.json", json.dumps({
                            "defines": defines, "compiler_flags": flags,
                            "verifier_options": options,
                        }, indent=2) + "\n")
                    check["result"] = "EXPECTED_COUNTEREXAMPLE"
                else:
                    if int(errors[1]):
                        raise RuntimeError("positive model produced a counterexample")
                    check["result"] = "COMPLETE_ZERO_ERRORS"
                check["errors"] = int(errors[1])
                for label, pattern in (("stored_states", r"([\d.e+]+) states, stored"),
                                       ("transitions", r"([\d.e+]+) transitions"),
                                       ("depth_reached", r"depth reached (\d+)")):
                    match = re.search(pattern, result)
                    check[label] = match[1] if match else None
        except (RuntimeError, OSError, subprocess.TimeoutExpired) as error:
            check.update(result="FAILED_OR_INCOMPLETE", reason=str(error))
        summary["checks"].append(check)
        print(f"{key}: {check['result']}", flush=True)
    changed = [name for name, digest in summary["model_sha256"].items()
               if sha(ROOT / name) != digest]
    if sha(Path(__file__)) != summary["runner_sha256"]:
        changed.append("verify.py")
    summary["source_changed_during_verification"] = changed
    summary["elapsed_seconds"] = time.monotonic() - started
    summary["status"] = "PASS" if not changed and all(
        item["result"] != "FAILED_OR_INCOMPLETE" for item in summary["checks"]) else "FAIL"
    with zipfile.ZipFile(archive_path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            summary["status"] = "FAIL"
            summary["archive_error"] = "duplicate evidence names"
    shutil.copyfile(archive_path, output / "counterexamples.zip")
    archive_workspace.cleanup()
    (output / "commands.json").write_text(json.dumps(logs, indent=2) + "\n")
    summary_path.write_text(json.dumps(summary, indent=2) + "\n")
    return 0 if summary["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
