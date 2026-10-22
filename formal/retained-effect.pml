/* Read-only governed retained-effect resolver. One reader, one independently
 * scheduled live-owner commit, one authority change and two identity classes.
 * SQLite snapshot copies are atomic; callbacks occur after commit/connection
 * release. Authority checks remain file snapshots, not linearised revocation.
 * Representation classes abstract exact JSON/HMAC/schema/byte-limit checks.
 */
mtype = { Absent, Prepared, Executing, Known, Unknown,
          Available, Withheld, Unavailable, NoPayload,
          Running, Returned, Denied };
mtype db_state;
mtype db_payload;
mtype snap_state;
mtype snap_payload;
mtype returned_state;
mtype returned_payload = NoPayload;
mtype terminal = Running;
byte db_revision = 1;
byte db_certainty;
byte snap_revision;
byte snap_receipt_revision;
byte snap_certainty;
byte returned_certainty;
byte request_namespace;
byte request_parent;
byte request_actor;
byte request_key;
byte returned_namespace;
byte returned_parent;
byte returned_actor;
byte returned_key;
byte authorised_view;
byte returned_view = 1;
byte pc;
byte reader_writes;
byte reader_recoveries;
byte reader_dispatches;
bool db_receipt;
bool snap_receipt;
bool current = true;
bool metadata_allowed;
bool payload_allowed;
bool precheck;
bool final_check;
bool query_inspection_ok;
bool result_inspection_ok;
bool result_inspection_precheck;
bool payload_valid;
bool db_open;
bool lookup;
bool namespace_verified;
bool parent_verified;
bool actor_verified;
bool owner_committed;
bool owner_live = true;
bool inspected;
bool result_callback_completed;
bool post_inspection_revocation;
bool withheld_for_inspection;
bool binding_denied;
bool query_denied;
bool return_after_revocation;
bool observed_older_snapshot;
bool finished;
bool stall_bit;

inline invariants() {
    assert(reader_writes == 0);
    assert(reader_recoveries == 0);
    assert(reader_dispatches == 0);
    assert(!lookup || precheck);
    assert(!lookup || namespace_verified);
    assert(!lookup || parent_verified);
    assert(!lookup || actor_verified);
    assert(!inspected || !db_open);
    assert(!result_callback_completed || result_inspection_precheck);
    assert(terminal != Returned || final_check);
    assert(terminal != Returned || (returned_namespace == request_namespace && returned_parent == request_parent && returned_actor == request_actor && returned_key == request_key));
    assert(terminal != Returned || authorised_view == returned_view);
    assert(terminal != Returned || returned_state == snap_state);
    assert(terminal != Returned || returned_certainty == snap_certainty);
    assert(returned_payload != Available || (snap_state == Known && snap_payload == Available && payload_allowed && result_inspection_ok));
}

proctype Authority() {
    if
    :: skip
    :: atomic { !finished; current = false }
    fi
}

proctype Owner() {
    /* The live owner may commit before or after the read transaction. The
     * ordinary SQLite read lock excludes its commit while this snapshot is
     * held. No owner lock is acquired by the reader. */
    if
    :: skip
    :: atomic {
        !db_open;
        if
        :: db_state == Prepared -> db_state = Executing; db_revision = 2; owner_committed = true
        :: db_state == Executing ->
            if
            :: db_state = Known; db_receipt = true;
                if :: db_payload = Available :: db_payload = Withheld fi;
                if :: db_certainty = 0 :: db_certainty = 1 :: db_certainty = 2 :: db_certainty = 3 fi
            :: db_state = Unknown
            fi;
            db_revision = 2; owner_committed = true
        :: else -> skip
        fi
    }
    fi
}

inline read_receipt() {
    snap_receipt_revision = db_revision;
    snap_receipt = db_receipt; snap_payload = db_payload;
    snap_certainty = db_certainty
}

proctype Reader() {
    do
    :: finished -> break
    :: !finished -> atomic {
        if
        :: pc == 0 ->
            /* Authorise resolve plus inspect the exact query before opening
             * the database. A failed query inspection discloses no existence. */
            precheck = current && metadata_allowed;
#ifdef BUG_PREAUTH
            precheck = metadata_allowed;
#endif
            assert(!precheck || current);
            if
            :: !precheck -> terminal = Denied; finished = true
            :: precheck -> pc = 1
            fi
        :: pc == 1 ->
            if :: query_inspection_ok = true :: query_inspection_ok = false fi;
            if
            :: query_inspection_ok -> pc = 2
            :: !query_inspection_ok -> query_denied = true; terminal = Denied; finished = true
            fi
        :: pc == 2 ->
            /* This check represents current authorisation after query
             * inspection, before the SQLite open. */
            precheck = current && metadata_allowed;
            if
            :: !precheck -> terminal = Denied; finished = true
            :: precheck -> pc = 3
            fi
        :: pc == 3 ->
            /* Open one read transaction. Namespace/header and parent checks
             * precede any lookup of the effect-specific row or receipt. */
            db_open = true;
            pc = 4
        :: pc == 4 ->
            if
            :: request_namespace != 1 ->
#ifdef BUG_NAMESPACE
                pc = 5
#else
                binding_denied = true; terminal = Denied; finished = true
#endif
            :: request_namespace == 1 -> namespace_verified = true; pc = 5
            fi
        :: pc == 5 ->
            if
            :: request_parent != 1 ->
#ifdef BUG_PARENT
                pc = 6
#else
                binding_denied = true; terminal = Denied; finished = true
#endif
            :: request_parent == 1 -> parent_verified = true; pc = 6
            fi
        :: pc == 6 ->
            if
            :: request_actor != 1 ->
#ifdef BUG_ACTOR
                pc = 7
#else
                binding_denied = true; terminal = Denied; finished = true
#endif
            :: request_actor == 1 -> actor_verified = true; pc = 7
            fi
        :: pc == 7 ->
            lookup = true;
            snap_revision = db_revision; snap_state = db_state;
#ifdef BUG_SPLIT_SNAPSHOT
            /* Incorrect independent reads let a live-owner commit split the
             * step and its completion evidence into different snapshots. */
            db_open = false;
#else
            read_receipt();
#endif
            pc = 8
        :: pc == 8 ->
#ifdef BUG_SPLIT_SNAPSHOT
            db_open = true; read_receipt();
#endif
            assert(snap_revision == snap_receipt_revision);
            assert(snap_receipt == (snap_state == Known));
            /* End the read transaction and drop the connection before any
             * payload/schema inspection callback or authority decision. */
#ifndef BUG_CALLBACK_WITH_DB
            db_open = false;
#endif
            pc = 14
        :: pc == 14 ->
            returned_namespace = 1; returned_parent = 1; returned_actor = 1;
            returned_key = 1;
            if
            :: request_key == 2 ->
#ifdef BUG_KEY
                skip
#else
                /* Absence under an already verified parent is an authorised
                 * not-recorded view. It never starts a replacement action. */
                snap_state = Absent; snap_payload = Unavailable;
                snap_receipt = false; snap_certainty = 0;
                returned_key = request_key
#endif
            :: else -> skip
            fi;
            returned_state = snap_state; returned_certainty = snap_certainty;
            returned_payload = Unavailable;
            pc = 9
        :: pc == 9 ->
            assert(!db_open);
            inspected = true;
#ifdef BUG_READER_WRITE
            reader_writes++;
#endif
#ifdef BUG_READER_RECOVERY
            if
            :: snap_state == Executing -> reader_recoveries++; db_state = Unknown
            :: else -> skip
            fi;
#endif
#ifdef BUG_READER_DISPATCH
            if
            :: snap_state == Absent || snap_state == Unknown || snap_state == Executing -> reader_dispatches++
            :: else -> skip
            fi;
#endif
            if
            :: snap_state == Known && snap_payload == Available -> pc = 10
            :: snap_state == Known && snap_payload == Withheld -> returned_payload = Withheld; pc = 12
            :: else -> pc = 12
            fi
        :: pc == 10 ->
            result_inspection_precheck = current && payload_allowed;
            if
            :: !result_inspection_precheck -> returned_payload = Withheld; pc = 12
            :: result_inspection_precheck ->
#ifdef BUG_STALL
            if
            :: stall_bit = !stall_bit
            :: pc = 11
            fi
#else
            pc = 11
#endif
            fi
        :: pc == 11 ->
            /* One bounded callback abstraction: complete exact-source clean
             * schema-valid result versus partial, failed or prohibited data. */
            if :: result_inspection_ok = true :: result_inspection_ok = false fi;
            result_callback_completed = true;
            payload_valid = result_inspection_ok && payload_allowed;
#ifdef BUG_INSPECTION
            payload_valid = payload_allowed;
#endif
            if
            :: payload_valid -> returned_payload = Available
            :: !payload_valid -> returned_payload = Withheld;
                if :: !result_inspection_ok -> withheld_for_inspection = true :: else -> skip fi
            fi;
            pc = 12
        :: pc == 12 ->
            /* Authorise the actual whole returned envelope, including the
             * minimal metadata view when payload handling failed. */
            final_check = current && metadata_allowed;
#ifdef BUG_POSTAUTH
            final_check = precheck;
#endif
            assert(!final_check || current);
            if
            :: final_check ->
                authorised_view = returned_view;
#ifdef BUG_VIEW_BINDING
                authorised_view = 2;
#endif
#ifdef BUG_WITHHELD_SETTLEMENT
                if :: returned_payload == Withheld -> returned_state = Unknown :: else -> skip fi;
#endif
#ifdef BUG_CERTAINTY
                if :: returned_state == Known -> returned_certainty = 1 :: else -> skip fi;
#endif
                pc = 13
            :: !final_check ->
                if :: result_callback_completed && !current -> post_inspection_revocation = true :: else -> skip fi;
                terminal = Denied; finished = true
            fi
        :: pc == 13 ->
            if :: !current -> return_after_revocation = true :: else -> skip fi;
            if :: snap_revision == 1 && db_revision == 2 -> observed_older_snapshot = true :: else -> skip fi;
            terminal = Returned; finished = true
        fi;
        invariants()
    }
    od;
    db_open = false;
#ifdef WITNESS_AVAILABLE
    assert(!(terminal == Returned && returned_payload == Available));
#endif
#ifdef WITNESS_WITHHELD_KNOWN
    assert(!(terminal == Returned && snap_state == Known && withheld_for_inspection && returned_payload == Withheld));
#endif
#ifdef WITNESS_UNKNOWN
    assert(!(terminal == Returned && returned_state == Unknown));
#endif
#ifdef WITNESS_NOT_RECORDED
    assert(!(terminal == Returned && returned_state == Absent));
#endif
#ifdef WITNESS_LIVE_OWNER
    assert(!(terminal == Returned && owner_live && owner_committed));
#endif
#ifdef WITNESS_OLD_SNAPSHOT
    assert(!observed_older_snapshot);
#endif
#ifdef WITNESS_REVOKED
    assert(!post_inspection_revocation);
#endif
#ifdef WITNESS_BINDING_DENIED
    assert(!binding_denied);
#endif
#ifdef WITNESS_QUERY_DENIED
    assert(!query_denied);
#endif
#ifdef WITNESS_AUTHORITY_RACE
    assert(!return_after_revocation);
#endif
}

init {
    atomic {
        if
        :: db_state = Absent
        :: db_state = Prepared
        :: db_state = Executing
        :: db_state = Unknown
        :: db_state = Known; db_receipt = true
        fi;
        db_payload = Unavailable; db_certainty = 0;
        if
        :: db_state == Known ->
            if :: db_payload = Available :: db_payload = Withheld fi;
            if :: db_certainty = 0 :: db_certainty = 1 :: db_certainty = 2 :: db_certainty = 3 fi
        :: else -> skip
        fi;
        if :: request_namespace = 1 :: request_namespace = 2 fi;
        if :: request_parent = 1 :: request_parent = 2 fi;
        if :: request_actor = 1 :: request_actor = 2 fi;
        if :: request_key = 1 :: request_key = 2 fi;
        if :: metadata_allowed = true :: metadata_allowed = false fi;
        if :: payload_allowed = true :: payload_allowed = false fi;
        run Reader(); run Owner(); run Authority()
    }
}

ltl retained_effect_finishes { <> finished }
