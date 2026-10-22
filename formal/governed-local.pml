/* Trusted-local structured governance: one invocation and one mediated effect.
 * The file authority can change BETWEEN every host transition. In particular,
 * reading authority and subsequently committing/dispatching/delivering are not
 * one atomic operation: this model promises snapshot checks, not distributed
 * revocation linearisation. One atomic block is one local computation or one
 * SQLite transaction. A crash can interrupt all inter-transaction boundaries.
 *
 * Safe / Raw / Withheld are representation classes, not JSON/schema/HMAC
 * implementations. Two originals have the same safe projection but distinct
 * commitments. Tenant/actor/parent/key identities denote exact comparisons.
 */
#define OWNER 1
#define ORIGINAL 1
#define CURRENT (authority_available && grant_active && grant_epoch == session_grant_epoch && policy_epoch == 1 && key_epoch == 1 && lease_epoch == credential_epoch)

mtype = { Empty, Prepared, Executing, Completed, Unknown,
          Absent, Safe, Raw, Withheld, TransportLost,
          Running, Delivered, Denied, Blocked };
mtype effect_state = Empty;
mtype journal_input = Absent;
mtype protocol_input = Absent;
mtype journal_output = Absent;
mtype protocol_output = Absent;
mtype receipt_payload = Absent;
mtype local_output = Absent;
mtype observed = Absent;
mtype terminal = Running;
chan requests = [1] of { byte };
chan results = [1] of { mtype, byte };

byte pc;
byte bound_original;
byte attempt_original;
byte attempt_tenant;
byte credential_epoch = 1;
byte lease_epoch = 1;
byte grant_epoch = 1;
byte session_grant_epoch = 1;
byte policy_epoch = 1;
byte key_epoch = 1;
byte calls;
byte sealed_calls;
byte observed_certainty;
byte retained_certainty;
byte sealed_certainty;
byte forwarded_parent = 1;
byte forwarded_actor = 1;
byte forwarded_certainty;
bool authority_available = true;
bool grant_active = true;
bool authority_changed;
bool request_check;
bool effect_check;
bool protocol_check;
bool delivery_check;
bool reservation;
bool receipt;
bool protocol_completed;
bool envelope_valid;
bool result_projected;
bool local_withheld;
bool retained_withheld;
bool sealed_withheld;
bool crash;
bool crashed_once;
bool restarted;
bool replayed;
bool input_conflict;
bool foreign_denied;
bool unknown_seen;
bool unknown_retry_blocked;
bool completion_gap_crash;
bool completed_while_revoked;
bool check_action_race;
bool delivered_once;
bool epoch_refreshed;
bool inspection_complete;
bool inspection_exact;
bool inspection_clean;
bool inspection_original_positive;
bool inspection_precheck;
bool inspection_postcheck;
bool inspection_accepted;
bool input_inspected;
bool outbound_inspected;
bool positive_original_projected;
bool inspection_denied;
bool inspection_withheld;
bool finished;
bool stall_bit;

inline invariants() {
    assert(calls <= 1);
    assert(journal_input != Raw && protocol_input != Raw);
    assert(journal_output != Raw && protocol_output != Raw && receipt_payload != Raw);
    assert(bound_original == 0 || bound_original == ORIGINAL);
    assert(receipt == (effect_state == Completed));
    assert(!receipt || retained_certainty == sealed_certainty);
    assert(!receipt || retained_withheld == sealed_withheld);
    assert(!receipt || retained_certainty == observed_certainty);
    assert(!protocol_completed || (receipt && protocol_output == journal_output));
    assert(!replayed || calls == sealed_calls);
    assert(terminal != Delivered || (delivery_check && attempt_tenant == OWNER));
}

proctype Authority() {
    /* One optional update, independently scheduled. Rotation can preserve the
     * tenant/principal identity while invalidating a previously issued lease.
     * Changed grant or key identities require an explicit migration. */
    if
    :: skip
    :: atomic {
        !finished;
        if
        :: grant_active = false
        :: authority_available = false
        :: grant_epoch = 2
        :: policy_epoch = 2
        :: key_epoch = 2
        :: credential_epoch = 2
        fi;
        authority_changed = true
    }
    fi
}

proctype Crash() {
    if
    :: skip
    :: atomic { !finished; crash = true; crashed_once = true }
    fi
}

proctype External() {
    byte request_id;
    mtype output;
    byte certainty;
    if
    :: requests?request_id ->
        /* A returned host completion is separate from the adapter's external
         * certainty: 0 unknown, 1 completed, 2 not-sent, 3 refused. */
        atomic {
            if :: output = Safe :: output = Raw :: output = TransportLost fi;
            if :: certainty = 0 :: certainty = 1 :: certainty = 2 :: certainty = 3 fi;
            results!output,certainty
        }
    :: finished -> skip
    fi
}

inline choose_replay() {
    terminal = Running;
    if :: attempt_tenant = OWNER :: attempt_tenant = 2 fi;
    if :: attempt_original = ORIGINAL :: attempt_original = 2 fi;
    /* Re-authentication after token rotation preserves owner and input key.
     * The alternative is an old lease. Neither changes the frozen grant/key. */
    if
    :: skip
    :: lease_epoch = credential_epoch; session_grant_epoch = grant_epoch;
        if :: grant_epoch == 2 -> epoch_refreshed = true :: else -> skip fi
    fi;
    sealed_calls = calls;
    pc = 20
}

inline inspect_projection() {
    /* Representative callback classes: complete/exact clean projection;
     * removed private original; partial/failed coverage; wrong source/pins;
     * or a finding in the retained projection. The callback is trusted and
     * bounded by its registered unit/byte/deadline contract in Rust. */
    inspection_complete = true; inspection_exact = true;
    inspection_clean = true; inspection_original_positive = false;
    if
    :: skip
    :: inspection_original_positive = true
    :: inspection_complete = false
    :: inspection_exact = false
    :: inspection_clean = false
    fi;
    inspection_accepted = inspection_complete && inspection_exact && inspection_clean;
#ifdef BUG_INSPECTION_COVERAGE
    inspection_accepted = inspection_exact && inspection_clean;
#endif
#ifdef BUG_INSPECTION_BINDING
    inspection_accepted = inspection_complete && inspection_clean;
#endif
#ifdef BUG_INSPECTION_FINDING
    inspection_accepted = inspection_complete && inspection_exact;
#endif
    assert(!inspection_accepted || (inspection_complete && inspection_exact && inspection_clean))
}

inline post_inspection() {
    inspection_postcheck = CURRENT;
#ifdef BUG_INSPECTION_REVOCATION
    inspection_postcheck = inspection_precheck;
#endif
    assert(!inspection_postcheck || CURRENT)
}

inline finish_delivery_attempt() {
    if
    :: replayed -> finished = true
    :: !replayed -> choose_replay()
    fi
}

proctype Host() {
    attempt_tenant = OWNER;
    attempt_original = ORIGINAL;
    do
    :: finished -> break
    :: !finished -> atomic {
        if
        :: crash ->
            crash = false; restarted = true;
            if
            :: effect_state == Executing ->
                effect_state = Unknown; unknown_seen = true;
                terminal = Blocked; choose_replay()
            :: effect_state == Completed ->
                if :: !protocol_completed -> completion_gap_crash = true :: else -> skip fi;
                choose_replay()
            :: effect_state == Unknown -> terminal = Blocked; choose_replay()
            :: else -> pc = 0
            fi
        :: !crash ->
            if
            :: pc == 0 ->
                request_check = CURRENT;
                if
                :: request_check -> pc = 30
                :: !request_check -> terminal = Denied; finished = true
                fi
            :: pc == 1 ->
                /* One journal transaction binds exact original commitment
                 * and persists only the closed-schema projection. */
                assert(request_check);
                assert(input_inspected);
                bound_original = ORIGINAL; journal_input = Safe;
#ifdef BUG_RAW_INPUT
                journal_input = Raw;
#endif
                pc = 2
            :: pc == 2 ->
                protocol_check = CURRENT;
                if
                :: protocol_check -> pc = 23
                :: !protocol_check -> terminal = Denied; finished = true
                fi
            :: pc == 23 ->
                assert(protocol_check); protocol_input = journal_input; pc = 3
            :: pc == 3 ->
                effect_check = CURRENT;
#ifdef BUG_STALE_EFFECT_AUTHORITY
                effect_check = request_check;
#endif
                assert(!effect_check || CURRENT);
                if
                :: effect_check ->
#ifdef BUG_OUTBOUND_INSPECTION
                    pc = 4
#else
                    pc = 40
#endif
                :: !effect_check -> terminal = Denied; finished = true
                fi
            :: pc == 4 ->
                assert(outbound_inspected);
                effect_state = Prepared; reservation = true; pc = 5
            :: pc == 5 ->
                effect_check = CURRENT;
                if
                :: effect_check -> pc = 6
                :: !effect_check -> terminal = Denied; finished = true
                fi
            :: pc == 6 ->
                assert(effect_check && reservation);
                if :: !CURRENT -> check_action_race = true :: else -> skip fi;
                effect_state = Executing; pc = 7
            :: pc == 7 ->
                calls++; requests!1; pc = 8
            :: pc == 8 ->
#ifdef BUG_STALL
                if
                :: stall_bit = !stall_bit
                :: results?observed,observed_certainty -> pc = 9
                fi
#else
                results?observed,observed_certainty; pc = 9
#endif
            :: pc == 9 ->
                if
                :: observed == TransportLost ->
                    effect_state = Unknown; unknown_seen = true;
                    terminal = Blocked; choose_replay()
                :: observed != TransportLost -> pc = 33
                fi
            :: pc == 33 ->
                inspection_precheck = CURRENT;
                if
                :: inspection_precheck -> pc = 34
                :: !inspection_precheck ->
                    local_withheld = true; local_output = Withheld; pc = 10
                fi
            :: pc == 34 -> inspect_projection(); pc = 35
            :: pc == 35 ->
                    post_inspection();
                    if :: envelope_valid = true :: envelope_valid = false fi;
                    result_projected = inspection_accepted && inspection_postcheck && observed == Safe && envelope_valid;
                    local_withheld = !result_projected;
                    if
                    :: result_projected -> local_output = Safe
                    :: !result_projected -> local_output = Withheld
                    fi;
                    if
                    :: !inspection_accepted -> inspection_withheld = true
                    :: else -> skip
                    fi;
#ifdef BUG_RAW_COMPLETION
                    if :: observed == Raw -> local_output = Raw :: else -> skip fi;
#endif
                    pc = 10
            :: pc == 10 ->
                /* Receipt + step completion share this SQLite transaction.
                 * Revoked authority withholds payload but preserves completion. */
                receipt = true; effect_state = Completed;
                journal_output = local_output; receipt_payload = local_output;
                retained_withheld = local_withheld; sealed_withheld = local_withheld;
                retained_certainty = observed_certainty; sealed_certainty = observed_certainty;
#ifdef BUG_CERTAINTY_PROMOTION
                retained_certainty = 1; sealed_certainty = 1;
#endif
#ifdef BUG_ERASE_COMPLETION
                if
                :: local_withheld -> receipt = false; effect_state = Unknown
                :: else -> skip
                fi;
#endif
                assert(receipt && effect_state == Completed);
                if
                :: !CURRENT && local_withheld -> completed_while_revoked = true
                :: else -> skip
                fi;
                pc = 11
            :: pc == 11 ->
                /* Separate protocol database commit; crash recovery repairs
                 * this from the completed execution receipt, never vice versa. */
                protocol_output = journal_output; protocol_completed = true;
#ifdef BUG_RAW_PROTOCOL
                if :: observed == Raw -> protocol_output = Raw :: else -> skip fi;
#endif
                pc = 12
            :: pc == 12 ->
                /* The SDK can forward the exact host marker. An unrelated
                 * parent/actor may only forward an unknown withheld marker. */
                if :: forwarded_parent = 1 :: forwarded_parent = 2 fi;
                if :: forwarded_actor = 1 :: forwarded_actor = 2 fi;
                if
                :: local_withheld && (forwarded_parent != 1 || forwarded_actor != 1) ->
#ifdef BUG_BORROWED_RECEIPT
                    forwarded_certainty = retained_certainty
#else
                    forwarded_certainty = 0
#endif
                :: else -> forwarded_certainty = retained_certainty
                fi;
                assert(!local_withheld || (forwarded_parent == 1 && forwarded_actor == 1) || forwarded_certainty == 0);
                pc = 13
            :: pc == 13 ->
                delivery_check = CURRENT;
#ifdef BUG_STALE_DELIVERY_AUTHORITY
                delivery_check = request_check;
#endif
                assert(!delivery_check || CURRENT);
                if
                :: delivery_check ->
                    if
                    :: retained_withheld -> pc = 14
                    :: !retained_withheld -> inspection_precheck = delivery_check; pc = 36
                    fi
                :: !delivery_check -> terminal = Denied; finish_delivery_attempt()
                fi
            :: pc == 36 -> inspect_projection(); pc = 37
            :: pc == 37 ->
                post_inspection();
                delivery_check = inspection_accepted && inspection_postcheck;
                if
                :: delivery_check -> pc = 14
                :: !delivery_check -> terminal = Denied; finish_delivery_attempt()
                fi
            :: pc == 14 ->
                assert(delivery_check && attempt_tenant == OWNER);
                delivered_once = true; terminal = Delivered; finish_delivery_attempt()
            :: pc == 30 ->
                inspection_precheck = CURRENT;
                if
                :: inspection_precheck -> pc = 31
                :: !inspection_precheck -> terminal = Denied; finished = true
                fi
            :: pc == 31 -> inspect_projection(); pc = 32
            :: pc == 32 ->
                post_inspection();
                if
                :: inspection_accepted && inspection_postcheck ->
                    input_inspected = true;
                    if
                    :: inspection_original_positive -> positive_original_projected = true
                    :: else -> skip
                    fi;
                    pc = 1
                :: !inspection_accepted || !inspection_postcheck ->
                    inspection_denied = true; terminal = Denied; finished = true
                fi
            :: pc == 40 ->
                inspection_precheck = CURRENT;
                if
                :: inspection_precheck -> pc = 41
                :: !inspection_precheck -> terminal = Denied; finished = true
                fi
            :: pc == 41 -> inspect_projection(); pc = 42
            :: pc == 42 ->
                post_inspection();
                if
                :: inspection_accepted && inspection_postcheck -> outbound_inspected = true; pc = 4
                :: !inspection_accepted || !inspection_postcheck ->
                    inspection_denied = true; terminal = Denied; finished = true
                fi
            :: pc == 20 ->
                /* A2A ownership rejects another tenant before status/output.
                 * Transport identity excludes purpose/token/grant versions,
                 * while exact original commitment still rejects changed input. */
                if
                :: attempt_tenant != OWNER ->
#ifdef BUG_FOREIGN_TENANT
                    delivery_check = true; terminal = Delivered; finished = true
#else
                    foreign_denied = true; terminal = Denied; finished = true
#endif
                :: attempt_tenant == OWNER ->
                    if
                    :: attempt_original != bound_original ->
#ifdef BUG_ORIGINAL_REBIND
                        bound_original = attempt_original;
#else
                        input_conflict = true;
#endif
                        terminal = Denied; finished = true
                    :: attempt_original == bound_original -> pc = 21
                    fi
                fi
            :: pc == 21 ->
                if
                :: effect_state == Unknown ->
#ifdef BUG_UNKNOWN_RETRY
                    calls++;
#endif
                    assert(calls == sealed_calls);
                    unknown_retry_blocked = true; terminal = Blocked; finished = true
                :: effect_state == Completed ->
                    assert(effect_state == Completed);
                    replayed = true;
#ifdef BUG_COMPLETED_RETRY
                    calls++;
#endif
                    protocol_output = journal_output; protocol_completed = true;
                    pc = 13
                fi
            fi
        fi;
        invariants()
    }
    od;
#ifdef WITNESS_AVAILABLE
    assert(!(receipt && !retained_withheld && terminal == Delivered));
#endif
#ifdef WITNESS_WITHHELD
    assert(!(receipt && retained_withheld && retained_certainty == 1));
#endif
#ifdef WITNESS_REPLAY
    assert(!replayed);
#endif
#ifdef WITNESS_UNKNOWN
    assert(!unknown_seen);
#endif
#ifdef WITNESS_RESTART_REPAIR
    assert(!(completion_gap_crash && receipt && protocol_completed && replayed));
#endif
#ifdef WITNESS_REVOKED_COMPLETION
    assert(!completed_while_revoked);
#endif
#ifdef WITNESS_INPUT_CONFLICT
    assert(!input_conflict);
#endif
#ifdef WITNESS_FOREIGN_DENIAL
    assert(!foreign_denied);
#endif
#ifdef WITNESS_SNAPSHOT_RACE
    assert(!check_action_race);
#endif
#ifdef WITNESS_EPOCH_REFRESH
    assert(!(epoch_refreshed && replayed && terminal == Delivered));
#endif
#ifdef WITNESS_POSITIVE_ORIGINAL
    assert(!(positive_original_projected && receipt));
#endif
#ifdef WITNESS_INSPECTION_DENIAL
    assert(!inspection_denied);
#endif
#ifdef WITNESS_INSPECTION_WITHHOLDING
    assert(!(inspection_withheld && receipt && retained_withheld));
#endif
#ifdef WITNESS_UNKNOWN_RETRY
    assert(!unknown_retry_blocked);
#endif
}

init {
    atomic { run Host(); run External(); run Authority(); run Crash() }
}

ltl governed_local_settles { <> finished }
