/* Opt-in structured exposure: a parent, two parallel child inputs, one
 * acquisition and one later external action. Source graph classes abstract
 * sealed journal metadata. The late restrictive ancestor is visited at 33.
 * Journal commits are atomic; callbacks, current reads, execute permission,
 * external observation and recipient delivery are distinct transitions.
 */
#define B 1
#define R 2
#define ALLOWS(mask,target) (((mask & R) == 0) || target == 1)
#define LIVE(expiry) (current && now < expiry)
mtype = { Basic, Late33, Missing, Cycle, Conflict, Oversize,
          Absent, Prepared, Executing, Known, Unknown };
mtype graph;
mtype acquisition = Absent;
mtype action = Absent;
byte parent_pc;
byte c1_pc;
byte c2_pc;
byte action_pc;
byte ancestry_count;
byte cursor;
byte resolved = B;
byte expected_root = B;
byte frozen_batch;
byte child1_context;
byte child2_context;
byte parent_exposure;
byte expected_parent;
byte completed_source_mask;
byte completed_source_id;
byte pending_source_id;
byte pending_expiry;
byte completed_expiry;
byte sealed_expiry = 2;
byte parent_expiry = 2;
byte policy_expiry = 3;
byte retained_expiry = 2;
byte attempted_binding = 1;
byte returned_binding;
byte now;
byte target;
byte result_target;
byte child_target;
byte worker_claim = B;
byte admitted_mask;
byte reply_mask;
byte reply_context;
byte accepted_reply_context;
byte acquisition_calls;
byte sealed_acquisition_calls;
byte action_calls;
byte sealed_action_calls;
byte nested_calls;
bool graph_valid;
bool batch_frozen;
bool child1_committed;
bool child2_committed;
bool child1_dispatched;
bool child2_dispatched;
bool child1_check;
bool child2_check;
bool child_target_denied;
bool pending;
bool pending_manifest;
bool start_rolled_back;
bool prepared_retry_seen;
bool source_processing_allowed;
bool processing_check;
bool result_processing_allowed;
bool result_processing_check;
bool result_inspector_called;
bool source_available;
bool inspector_called;
bool inspection_ok;
bool result_inspection_ok;
bool acquisition_postcheck;
bool execute_check;
bool action_check;
bool delivery_check;
bool current = true;
bool acquisition_receipt;
bool known_observed;
bool source_storage_ok;
bool completed_source;
bool child1_exposure_committed;
bool child1_delivered;
bool child2_exposure_committed;
bool child2_delivered;
bool child2_payload_withheld;
bool reply_admission;
bool child1_reply_check;
bool action_receipt;
bool action_known_observed;
bool replayed;
bool changed_binding;
bool binding_denied;
bool restriction_denied;
bool nested_blocked;
bool frozen_parallel_seen;
bool late_ancestor_seen;
bool invalid_graph_denied;
bool known_withheld;
bool result_restriction_withheld;
bool result_inspection_withheld;
bool cached_returned;
bool unknown_retry_blocked;
bool expiry_denied;
bool processing_denied;
bool crashed;
bool crash_between_exposure_reply;
bool check_action_race;
bool finished;
bool stall_bit;

inline invariants() {
    assert(!batch_frozen || graph_valid);
    assert(!batch_frozen || resolved == expected_root);
    assert(!child1_dispatched || child1_committed);
    assert(!child2_dispatched || child2_committed);
    assert(!child1_committed || ALLOWS(frozen_batch,child_target));
    assert(!child2_committed || ALLOWS(frozen_batch,child_target));
    assert(!child1_dispatched || child1_check);
    assert(!child2_dispatched || child2_check);
    assert(!child1_committed || child1_context == frozen_batch);
    assert(!child2_committed || child2_context == frozen_batch);
    assert(!inspector_called || processing_check);
    assert(!result_inspector_called || result_processing_check);
    assert(!pending || pending_manifest);
    assert(!pending || acquisition != Prepared);
    assert(pending_source_id == 0 || pending_source_id == 1);
    assert(!completed_source || completed_source_id == 2);
    assert(!completed_source || completed_source_mask == (child1_context | R));
    assert(!completed_source || completed_expiry == sealed_expiry);
    assert(!completed_source || acquisition_receipt);
    assert(!known_observed || acquisition_receipt);
    assert(!action_known_observed || action_receipt);
    assert(acquisition_calls <= 1);
    assert(action_calls <= 1);
    assert(nested_calls == 0);
    assert((parent_exposure & expected_parent) == expected_parent);
    assert(!child1_delivered || child1_exposure_committed);
    assert(!child2_delivered || child2_exposure_committed);
}

proctype Authority() {
    if
    :: skip
    :: atomic { !finished; current = false }
    :: atomic { !finished; now = 2 }
    fi
}

proctype Crash() {
    if
    :: skip
    :: do
        :: finished -> break
        :: atomic {
            !finished && !crashed &&
                ((acquisition == Executing && ((c1_pc >= 6 && c1_pc <= 8) || c1_pc >= 13)) ||
                 (acquisition == Known && c1_pc == 9) ||
                 (child1_exposure_committed && !child1_delivered && (c1_pc == 10 || c1_pc == 18)) ||
                 (action == Executing && action_pc == 2) ||
                 (action == Known && action_pc == 3));
            crashed = true;
            if
            :: acquisition == Executing ->
                acquisition = Unknown; sealed_acquisition_calls = acquisition_calls; c1_pc = 11
            :: acquisition == Known && c1_pc == 9 -> skip
            :: child1_exposure_committed && !child1_delivered && (c1_pc == 10 || c1_pc == 18) ->
                crash_between_exposure_reply = true; c1_pc = 9
            :: else -> skip
            fi;
            if
            :: action == Executing -> action = Unknown; action_pc = 4
            :: else -> skip
            fi;
            invariants()
        } -> break
       od
    fi
}

proctype Host() {
    do
    :: finished -> break
    :: !finished -> atomic {
        if
        :: parent_pc == 0 ->
            /* The finite prefix of 32 unrestricted ancestors is compressed
             * in metadata, but traversal still reaches the mandatory 33rd. */
            if
            :: graph == Basic -> ancestry_count = 1; graph_valid = true
            :: graph == Late33 -> ancestry_count = 33; graph_valid = true; expected_root = B | R
            :: else -> graph_valid = false
            fi;
            if
            :: graph_valid ->
                cursor = 1;
                do
#ifdef BUG_TRUNCATE_32
                :: cursor <= ancestry_count && cursor <= 32 ->
#else
                :: cursor <= ancestry_count ->
#endif
                    if
                    :: graph == Late33 && cursor == 33 -> resolved = resolved | R; late_ancestor_seen = true
                    :: else -> skip
                    fi;
                    cursor++
                :: else -> break
                od;
                parent_pc = 1
            :: else ->
#ifdef BUG_INVALID_CLOSURE
                parent_pc = 1
#else
                invalid_graph_denied = true; finished = true
#endif
            fi
        :: parent_pc == 1 ->
            /* All batch requests freeze one manifest before either thread. */
            frozen_batch = resolved; parent_exposure = resolved; expected_parent = expected_root;
            batch_frozen = true; parent_pc = 2
        :: parent_pc == 2 && c1_pc == 0 ->
            child1_check = LIVE(2) && ALLOWS(frozen_batch,child_target);
#ifdef BUG_CHILD_TARGET
            child1_check = LIVE(2);
#endif
            if
            :: child1_check -> child1_context = frozen_batch; child1_committed = true; c1_pc = 1
            :: else -> child_target_denied = true; c1_pc = 12
            fi
        :: parent_pc == 2 && c2_pc == 0 ->
            child2_check = LIVE(2) && ALLOWS(frozen_batch,child_target);
            if
            :: child2_check ->
#ifdef BUG_LIVE_PARALLEL_CONTEXT
                child2_context = parent_exposure
#else
                child2_context = frozen_batch
#endif
                ; child2_committed = true; c2_pc = 1
            :: else -> child_target_denied = true; c2_pc = 4
            fi
        :: parent_pc == 2 && c1_pc == 1 ->
#ifdef BUG_DISPATCH_BEFORE_EXPOSURE
            child1_committed = false;
#endif
            child1_dispatched = true; c1_pc = 2
        :: parent_pc == 2 && c1_pc == 2 ->
            /* Registered extra source policies govern REAL descriptor
             * processing before the detector receives any descriptor bytes. */
            processing_check = LIVE(2) && source_processing_allowed;
#ifdef BUG_BASELINE_INSPECTION
            processing_check = LIVE(2);
#endif
            if
            :: processing_check -> c1_pc = 3
            :: else -> processing_denied = true; c1_pc = 12
            fi
        :: parent_pc == 2 && c1_pc == 3 ->
            inspector_called = true;
            assert(source_processing_allowed);
            c1_pc = 4
        :: parent_pc == 2 && c1_pc == 4 ->
            acquisition_postcheck = LIVE(2) && source_processing_allowed;
            if
            :: acquisition_postcheck && inspection_ok ->
                /* Pending source has its own immutable identity/manifest.
                 * It is not fabricated completed output. Expiry is the
                 * minimum original grant/policies/parent expiry, here 2. */
                acquisition = Prepared;
#ifdef BUG_SPLIT_PENDING_COMMIT
                pending = true; pending_manifest = true;
#endif
                c1_pc = 5
            :: else -> c1_pc = 12
            fi
        :: parent_pc == 2 && c1_pc == 5 ->
            execute_check = LIVE(2);
            if
            :: execute_check -> c1_pc = 17
            :: else -> expiry_denied = true; c1_pc = 12
            fi
        :: parent_pc == 2 && c1_pc == 17 ->
                /* The file-authority read is separate from this journal
                 * transaction. Updates may interleave before its commit. */
                assert(execute_check);
                if
                :: !start_rolled_back ->
                    /* Failed transaction leaves prepared with no pending
                     * record or manifest change. Same-effect retry rereads
                     * current authority; it does not reset a source expiry. */
                    start_rolled_back = true; c1_pc = 5
                :: pending_source_id = 1; pending_expiry = policy_expiry;
                    if :: parent_expiry < pending_expiry -> pending_expiry = parent_expiry :: else -> skip fi;
#ifndef BUG_ATTACH_ONLY_AT_COMPLETION
                    pending = true; pending_manifest = true;
#endif
                    acquisition = Executing;
                    if :: start_rolled_back -> prepared_retry_seen = true :: else -> skip fi;
                    c1_pc = 6
                fi
        :: parent_pc == 2 && c1_pc == 6 ->
            assert(execute_check);
            if :: !current -> check_action_race = true :: else -> skip fi;
            acquisition_calls++; c1_pc = 7
        :: parent_pc == 2 && c1_pc == 7 ->
            /* Native callback attempts a nested action before completion.
             * An unresolved pending source blocks ALL outbound work. */
            if
            :: pending && pending_manifest -> nested_blocked = true
            :: else -> nested_calls++
            fi;
            c1_pc = 8
        :: parent_pc == 2 && c1_pc == 8 ->
#ifdef BUG_STALL
            stall_bit = !stall_bit
#else
            if
            :: acquisition = Unknown; sealed_acquisition_calls = acquisition_calls; c1_pc = 11
            :: c1_pc = 13
            fi
#endif
        :: parent_pc == 2 && c1_pc == 13 ->
            /* Native result is observed but not yet durably settled. Source
             * processing rights precede its separate detector callback. */
            result_processing_check = LIVE(pending_expiry) && result_processing_allowed;
#ifdef BUG_RESULT_PROCESSING
            result_processing_check = LIVE(pending_expiry);
#endif
            if
            :: result_processing_check -> c1_pc = 14
            :: else -> source_available = false; c1_pc = 16
            fi
        :: parent_pc == 2 && c1_pc == 14 ->
            result_inspector_called = true;
            assert(result_processing_allowed);
            c1_pc = 15
        :: parent_pc == 2 && c1_pc == 15 ->
            source_available = LIVE(pending_expiry) && result_processing_allowed && result_inspection_ok;
            if :: !result_inspection_ok -> result_inspection_withheld = true :: else -> skip fi;
            c1_pc = 16
        :: parent_pc == 2 && c1_pc == 16 ->
                acquisition = Known; acquisition_receipt = true; known_observed = true;
                if
                :: source_storage_ok ->
                    /* One journal transaction settles the step, creates a
                     * distinct complete source and removes pending state. */
                    completed_source = true; completed_source_id = 2;
                    completed_source_mask = child1_context | R;
                    completed_expiry = pending_expiry;
                    pending = false; pending_manifest = false; c1_pc = 9
                :: else -> known_withheld = true; c1_pc = 12
                fi
        :: parent_pc == 2 && c1_pc == 9 ->
            delivery_check = LIVE(completed_expiry) && completed_source && source_available && !pending && ALLOWS(parent_exposure | completed_source_mask,1);
            if
            :: delivery_check ->
                /* Recipient union is committed BEFORE payload/reply. */
                expected_parent = expected_parent | completed_source_mask;
#ifndef BUG_REPLY_BEFORE_EXPOSURE
                parent_exposure = parent_exposure | completed_source_mask;
                child1_exposure_committed = true;
#endif
                c1_pc = 10
            :: else -> known_withheld = true; c1_pc = 12
            fi
        :: parent_pc == 2 && c1_pc == 10 ->
            assert(child1_exposure_committed);
            /* This represented source/recipient tuple is class1. Recheck the
             * full recipient union at the final actual-payload boundary. */
            child1_reply_check = LIVE(completed_expiry) && ALLOWS(parent_exposure,1);
            if
            :: child1_reply_check -> c1_pc = 18
            :: else -> known_withheld = true; c1_pc = 12
            fi
        :: parent_pc == 2 && c1_pc == 18 ->
            assert(child1_reply_check && child1_exposure_committed);
            child1_delivered = true; c1_pc = 12
        :: parent_pc == 2 && c1_pc == 11 ->
            /* Exact recovery/retry consults pending state and never grants
             * fresh execute permission for unresolved external work. */
#ifdef BUG_UNKNOWN_RETRY
            acquisition_calls++;
#else
            unknown_retry_blocked = true;
#endif
            assert(acquisition_calls == sealed_acquisition_calls);
            c1_pc = 12
        :: parent_pc == 2 && c2_pc == 1 ->
            child2_dispatched = true; c2_pc = 2
        :: parent_pc == 2 && c2_pc == 2 ->
            if
            :: child1_delivered && !(frozen_batch & R) -> frozen_parallel_seen = true
            :: else -> skip
            fi;
            /* Opaque result inherits frozen input, ignoring worker refs. */
            expected_parent = expected_parent | child2_context;
#ifdef BUG_OVERWRITE_EXPOSURE
            parent_exposure = child2_context;
#else
            parent_exposure = parent_exposure | child2_context;
#endif
            child2_exposure_committed = true; c2_pc = 3
        :: parent_pc == 2 && c2_pc == 3 ->
            assert(child2_exposure_committed);
            /* Initial full-recipient admission precedes the inspector. The
             * other sibling may deliver R during this callback window. */
            delivery_check = LIVE(2) && ALLOWS(parent_exposure,result_target);
            if
            :: delivery_check -> c2_pc = 5
            :: else -> child2_payload_withheld = true; c2_pc = 4
            fi
        :: parent_pc == 2 && c2_pc == 5 ->
            /* Final actual-payload admission reloads ALL recipient exposure,
             * even though this sibling's input/source remains frozen at B. */
            reply_context = parent_exposure; reply_mask = reply_context;
#ifdef BUG_RESULT_DISCLOSURE
            reply_mask = child2_context;
#endif
            delivery_check = LIVE(2) && ALLOWS(reply_mask,result_target);
            if
            :: delivery_check -> c2_pc = 6
            :: else -> child2_payload_withheld = true; c2_pc = 4
            fi
        :: parent_pc == 2 && c2_pc == 6 ->
            /* A final pure comparison follows policy callbacks. No callback
             * occurs after this cut; changed recipient exposure withholds. */
            if
            :: reply_context == parent_exposure && now < sealed_expiry ->
                assert(ALLOWS(expected_parent,result_target));
                accepted_reply_context = reply_context; reply_admission = true; c2_pc = 7
            :: else -> child2_payload_withheld = true; c2_pc = 4
            fi
        :: parent_pc == 2 && c2_pc == 7 ->
            /* Reply I/O is later than the final pure snapshot. Parent growth
             * or authority changes in this window do not rewrite that cut. */
            assert(reply_admission && ALLOWS(accepted_reply_context,result_target));
            child2_delivered = true; c2_pc = 4
        :: parent_pc == 2 && c1_pc == 12 && c2_pc == 4 ->
            parent_pc = 3
        :: parent_pc == 3 && action_pc == 0 ->
            admitted_mask = parent_exposure;
#ifdef BUG_WORKER_NARROWING
            admitted_mask = worker_claim;
#endif
            action_check = LIVE(2) && ALLOWS(admitted_mask, target);
            if
            :: action_check ->
                assert(ALLOWS(expected_parent, target));
                action = Prepared; action_pc = 1
            :: else ->
                if :: !ALLOWS(expected_parent,target) -> restriction_denied = true :: else -> expiry_denied = true fi;
                action_pc = 5
            fi
        :: parent_pc == 3 && action_pc == 1 ->
            /* Effect arguments and target consume the current inherited
             * closure, then execution permission is durably retained. */
            action_check = LIVE(2) && ALLOWS(parent_exposure,target);
            if :: action_check -> action = Executing; action_pc = 2 :: else -> action_pc = 5 fi
        :: parent_pc == 3 && action_pc == 2 ->
            assert(action_check);
            action_calls++;
            if
            :: action = Known; action_receipt = true; action_known_observed = true
            :: action = Unknown
            fi;
            action_pc = 3
        :: parent_pc == 3 && action_pc == 3 ->
            /* Known completion is retained independently of current output
             * delivery, source expiry and detector availability. */
#ifdef BUG_ERASE_COMPLETION
            if :: !current -> action_receipt = false :: else -> skip fi;
#endif
            delivery_check = LIVE(2) && ALLOWS(parent_exposure,result_target);
            if
            :: action == Known && delivery_check -> assert(ALLOWS(expected_parent,result_target))
            :: action == Known && !delivery_check -> result_restriction_withheld = true
            :: else -> skip
            fi;
            action_pc = 4
        :: parent_pc == 3 && action_pc == 4 ->
            if
            :: !replayed ->
                replayed = true; sealed_action_calls = action_calls;
                if :: changed_binding = true; attempted_binding = 2 :: changed_binding = false fi;
                action_pc = 6
            :: else -> action_pc = 5
            fi
        :: parent_pc == 3 && action_pc == 6 ->
            if
#ifndef BUG_SOURCE_REBIND
            :: changed_binding -> binding_denied = true; action_pc = 5
            :: !changed_binding ->
#else
            :: true ->
#endif
#ifdef BUG_REPLAY_EXPIRY
                retained_expiry = 3;
#endif
#ifdef BUG_COMPLETED_RETRY
                if :: action == Known -> action_calls++ :: else -> skip fi;
#endif
                assert(action_calls == sealed_action_calls);
                if
                :: action == Unknown -> unknown_retry_blocked = true
                :: action == Known ->
                    delivery_check = LIVE(retained_expiry) && ALLOWS(parent_exposure,result_target);
                    if
                    :: !delivery_check -> known_withheld = true
                    :: delivery_check ->
                        /* Lookup remains bound to the original output/source
                         * identity and original minimum expiry on replay. */
                        returned_binding = 1; cached_returned = true;
                        assert(returned_binding == attempted_binding);
                        assert(now < sealed_expiry)
                    fi
                :: else -> skip
                fi;
                action_pc = 5
            fi
        :: parent_pc == 3 && action_pc == 5 -> finished = true
        fi;
        invariants();
#ifdef WITNESS_APPROVED
        assert(!(action_receipt && action_calls == 1));
#endif
#ifdef WITNESS_INHERITED_DENIAL
        assert(!restriction_denied);
#endif
#ifdef WITNESS_LATE33
        assert(!(late_ancestor_seen && restriction_denied));
#endif
#ifdef WITNESS_PARALLEL_FREEZE
        assert(!(frozen_parallel_seen && child2_delivered && result_target == 1));
#endif
#ifdef WITNESS_PENDING_BLOCK
        assert(!nested_blocked);
#endif
#ifdef WITNESS_KNOWN_WITHHELD
        assert(!(known_withheld && acquisition_receipt));
#endif
#ifdef WITNESS_UNKNOWN_RETRY
        assert(!unknown_retry_blocked);
#endif
#ifdef WITNESS_EXPOSURE_CRASH
        assert(!crash_between_exposure_reply);
#endif
#ifdef WITNESS_EXPIRY
        assert(!(expiry_denied && now >= sealed_expiry));
#endif
#ifdef WITNESS_INVALID_GRAPH
        assert(!invalid_graph_denied);
#endif
#ifdef WITNESS_PROCESSING_DENIED
        assert(!(processing_denied && !source_processing_allowed && LIVE(sealed_expiry)));
#endif
#ifdef WITNESS_CURRENT_RACE
        assert(!check_action_race);
#endif
#ifdef WITNESS_REPLAY
        assert(!(cached_returned && replayed && action_receipt && !changed_binding));
#endif
#ifdef WITNESS_RESULT_WITHHELD
        assert(!(child2_payload_withheld && child2_dispatched && child2_context == B && (expected_parent & R) && result_target == 2 && LIVE(sealed_expiry)));
#endif
#ifdef WITNESS_PREPARED_RETRY
        assert(!prepared_retry_seen);
#endif
#ifdef WITNESS_RESULT_INSPECTION
        assert(!(result_inspection_withheld && acquisition_receipt));
#endif
#ifdef WITNESS_CHILD_DENIAL
        assert(!(child_target_denied && (frozen_batch & R) && child_target == 2 && !child1_committed && !child2_committed));
#endif
    }
    od
}

init {
    atomic {
        if :: graph = Basic :: graph = Late33 :: graph = Missing :: graph = Cycle :: graph = Conflict :: graph = Oversize fi;
        if :: source_processing_allowed = true :: source_processing_allowed = false fi;
        if :: result_processing_allowed = true :: result_processing_allowed = false fi;
        if :: inspection_ok = true :: inspection_ok = false fi;
        if :: result_inspection_ok = true :: result_inspection_ok = false fi;
        if :: source_storage_ok = true :: source_storage_ok = false fi;
        if :: target = 1 :: target = 2 fi;
        if :: result_target = 1 :: result_target = 2 fi;
        if :: child_target = 1 :: child_target = 2 fi;
        run Host(); run Authority(); run Crash()
    }
}

ltl exposure_lineage_finishes { <> finished }
