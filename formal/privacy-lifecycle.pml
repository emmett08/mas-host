/* Trusted-local N3 lifecycle: bounded durable timers/attempts, current admission,
 * exact-action lookup, separate acknowledgement/escalation and metadata review.
 * Abstracts clock into ordered ticks, the journal into atomic retained records,
 * and endpoint lookup into its explicitly qualified authoritative contract.
 */
mtype = { Pending, Accepted, Throttled, NotSent, Uncertain, Absent,
          Acked, AckPending, AckUnknown, Finished };
mtype status = Pending;
mtype acknowledgement = AckUnknown;
byte attempts;
byte action = 1;
byte retained_action;
byte now;
byte due;
byte expiry = 4;
bool intent;
bool rights = true;
bool absent_proof;
bool unknown_seen;
bool accepted;
bool escalated;
bool reviewed;
bool release_permission;
bool exact_review = true;
bool replayed;
bool finished;

inline submit() {
#ifdef BUG_STALE_AUTH
    rights = false;
    assert(rights);
#endif
    assert(intent && rights);
#ifdef BUG_EARLY_RETRY
    now = 0; due = 1;
#endif
    assert(now >= due);
    assert(now < expiry);
    assert(!unknown_seen || absent_proof);
    attempts++;
    assert(attempts <= 2);
    if
    :: status = Accepted; accepted = true
    :: status = Throttled
    :: status = NotSent
    :: status = Uncertain; unknown_seen = true
    fi;
    retained_action = action
}

init {
    intent = true;
    if
    :: rights = false
    :: skip
    fi;
    if
    :: rights ->
        submit();
        if
        :: status == Uncertain ->
            if
            :: status = Accepted; accepted = true
            :: status = Absent; absent_proof = true
            :: skip
            fi
        :: else -> skip
        fi;
#ifdef BUG_UNKNOWN_RETRY
        if
        :: status == Uncertain -> absent_proof = false; assert(!unknown_seen || absent_proof)
        :: else -> skip
        fi;
#endif
        if
        :: status == Throttled || status == NotSent || status == Absent ->
            due = 1;
            now = due;
            if
            :: rights = false
            :: skip
            fi;
            if
            :: rights -> submit()
            :: else -> skip
            fi
        :: else -> skip
        fi
    :: else -> skip
    fi;
    replayed = true;
#ifdef BUG_REPLAY
    attempts = 3;
#endif
    assert(attempts <= 2);
    assert(retained_action == 0 || retained_action == 1);
    /* Recovery after expiry preserves a completed accepted observation. */
    now = expiry;
    assert(!accepted || status == Accepted);
    if
    :: accepted ->
        if
        :: acknowledgement = Acked
        :: acknowledgement = AckPending
        :: acknowledgement = AckUnknown
        fi;
#ifdef BUG_ACK_CERTAINTY
        acknowledgement = Acked; assert(acknowledgement != Acked);
#endif
        if
        :: acknowledgement != Acked && rights ->
            escalated = true;
            action = 2;
#ifdef BUG_ESCALATION_ID
            action = 1;
#endif
            assert(action != retained_action)
        :: else -> skip
        fi
    :: else -> skip
    fi;
    if
    :: rights ->
        reviewed = true;
#ifdef BUG_REVIEW_BINDING
        exact_review = false;
#endif
        assert(exact_review);
#ifdef BUG_REVIEW_RELEASE
        release_permission = true;
#endif
        assert(!release_permission)
    :: else -> skip
    fi;
    finished = true;
#ifdef WITNESS_ACCEPTED
    assert(!accepted);
#endif
#ifdef WITNESS_ESCALATED
    assert(!escalated);
#endif
#ifdef WITNESS_UNKNOWN
    assert(!unknown_seen);
#endif
}
ltl privacy_lifecycle_finishes { <> finished }
