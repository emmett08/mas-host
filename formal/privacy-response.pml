/* First N3 native adapter: durable safe case intent, then a separately admitted
 * notification effect. This composes the adapter's ordering and decision rules;
 * governed-local.pml covers the separate host receipt/protocol commit mechanics.
 * One logical event/action identity, one optional crash, one optional rights
 * revocation and one explicit retry. No scheduler, reconciliation API or
 * automatic retry is implemented or promised by this model.
 */
mtype = { Empty, Executing, Completed, Unknown,
          Privacy, Secret, Coverage,
          Accepted, Refused, Throttled, NotSent, Lost, Unset,
          Running, Settled, Held, Denied, Blocked };
mtype intent_state = Empty;
mtype notification_state = Empty;
mtype reason;
mtype provider_outcome = Unset;
mtype observed_outcome = Unset;
mtype sealed_outcome = Unset;
mtype terminal = Running;
chan submissions = [1] of { byte };
chan replies = [1] of { mtype };

byte pc;
byte route;
byte required_route;
byte frozen_route;
byte frozen_event;
byte attempt_event = 1;
byte attempt_tenant = 1;
byte attempt_destination = 1;
byte frozen_destination;
byte intent_calls;
byte notification_calls;
byte sealed_intent_calls;
byte sealed_notification_calls;
bool rights = true;
bool admission_check;
bool intent_exposed;
bool intent_receipt;
bool notification_receipt;
bool payload_withheld;
bool recipient_delivery_known;
bool crash;
bool crashed_once;
bool restarted;
bool crashed_between_effects;
bool unknown_retry_blocked;
bool destination_conflict;
bool replayed;
bool conflict_rejected;
bool foreign_rejected;
bool revoked_before_notification;
bool finished;
bool stall_bit;

inline invariants() {
    assert(intent_calls <= 1 && notification_calls <= 1);
    assert(route == required_route);
    assert(frozen_route == 0 || frozen_route == required_route);
    assert(frozen_event == 0 || frozen_event == 1);
    assert(frozen_destination == 0 || frozen_destination == 1);
    assert(!intent_receipt || intent_state == Completed);
    assert(!notification_receipt || notification_state == Completed);
    assert(notification_calls == 0 || (intent_receipt && intent_exposed));
    assert(!notification_receipt || provider_outcome == sealed_outcome);
    assert(!notification_receipt || provider_outcome == observed_outcome);
    assert(!recipient_delivery_known);
    assert(!replayed || (intent_calls == sealed_intent_calls && notification_calls == sealed_notification_calls));
}

proctype Revoke() {
    if
    :: skip
    :: atomic { !finished; rights = false }
    fi
}

proctype Crash() {
    if
    :: skip
    :: atomic { !finished; crash = true; crashed_once = true }
    fi
}

proctype Endpoint() {
    byte action_id;
    if
    :: submissions?action_id ->
        atomic {
            if
            :: replies!Accepted
            :: replies!Refused
            :: replies!Throttled
            :: replies!NotSent
            :: replies!Lost
            fi
        }
    :: finished -> skip
    fi
}

inline choose_replay() {
    if
    :: replayed -> finished = true
    :: !replayed ->
        if :: attempt_event = 1 :: attempt_event = 2 fi;
        if :: attempt_tenant = 1 :: attempt_tenant = 2 fi;
        if :: attempt_destination = 1 :: attempt_destination = 2 fi;
        sealed_intent_calls = intent_calls;
        sealed_notification_calls = notification_calls;
        replayed = true; terminal = Running; pc = 20
    fi
}

proctype Handler() {
    atomic {
        if
        :: reason = Privacy; required_route = 1
        :: reason = Secret; required_route = 2
        :: reason = Coverage; required_route = 3
        fi;
        route = required_route;
#ifdef BUG_UNTRUSTED_ROUTE
        route = 4;
#endif
        invariants()
    }
    do
    :: finished -> break
    :: !finished -> atomic {
        if
        :: crash ->
            crash = false; restarted = true;
            if
            :: intent_state == Executing ->
                intent_state = Unknown; terminal = Blocked; choose_replay()
            :: notification_state == Executing ->
                notification_state = Unknown;
                terminal = Blocked; choose_replay()
            :: intent_state == Unknown || notification_state == Unknown ->
                terminal = Blocked; finished = true
            :: else ->
                if
                :: intent_state == Completed && notification_state == Empty -> crashed_between_effects = true
                :: else -> skip
                fi;
#ifdef BUG_REPLAY_IDENTITY
                pc = 0
#else
                if :: replayed -> pc = 20 :: !replayed -> pc = 0 fi
#endif
            fi
        :: !crash ->
            if
            :: pc == 0 ->
                admission_check = rights;
                if
                :: !admission_check -> terminal = Denied; finished = true
                :: admission_check ->
                    if
                    :: intent_state == Completed -> pc = 4
                    :: intent_state == Empty ->
#ifdef BUG_INTENT_ORDER
                        pc = 5
#else
                        pc = 1
#endif
                    fi
                fi
            :: pc == 1 ->
                assert(admission_check);
                frozen_event = 1; frozen_route = route;
                frozen_destination = 1;
                intent_state = Executing; pc = 2
            :: pc == 2 ->
                intent_calls++; pc = 3
            :: pc == 3 ->
                /* The native callback returns only fixed safe metadata.
                 * Host filtering can withhold its approved return object. */
                intent_state = Completed; intent_receipt = true;
                if :: intent_exposed = true :: intent_exposed = false fi;
                pc = 4
            :: pc == 4 ->
                if
                :: intent_exposed -> pc = 5
                :: !intent_exposed ->
#ifdef BUG_WITHHELD_INTENT
                    pc = 5
#else
                    terminal = Held; choose_replay()
#endif
                fi
            :: pc == 5 ->
                /* Fresh admission is mandatory after case-intent settlement.
                 * A previous intent check grants no notification authority. */
                admission_check = rights;
#ifdef BUG_REVOKED_NOTIFY
                admission_check = true;
#endif
                assert(!admission_check || rights);
                if
                :: admission_check -> pc = 6
                :: !admission_check ->
                    revoked_before_notification = true; terminal = Denied; finished = true
                fi
            :: pc == 6 ->
                assert(intent_receipt && intent_exposed);
                assert(admission_check);
                if
                :: notification_state == Completed ->
#ifdef BUG_COMPLETED_RETRY
                    notification_calls++;
#endif
                    pc = 10
                :: notification_state == Empty -> notification_state = Executing; pc = 7
                fi
            :: pc == 7 ->
                notification_calls++; submissions!1; pc = 8
            :: pc == 8 ->
#ifdef BUG_STALL
                if
                :: stall_bit = !stall_bit
                :: replies?observed_outcome -> pc = 9
                fi
#else
                replies?observed_outcome; pc = 9
#endif
            :: pc == 9 ->
                if
                :: observed_outcome == Lost ->
                    notification_state = Unknown;
                    terminal = Blocked; choose_replay()
                :: observed_outcome != Lost ->
                    provider_outcome = observed_outcome; sealed_outcome = observed_outcome;
#ifdef BUG_THROTTLED_SUCCESS
                    if :: provider_outcome == Throttled -> provider_outcome = Accepted :: else -> skip fi;
#endif
                    notification_state = Completed; notification_receipt = true;
#ifdef BUG_THROTTLE_RETRY
                    if :: provider_outcome == Throttled -> notification_calls++ :: else -> skip fi;
#endif
                    if :: payload_withheld = true :: payload_withheld = false fi;
#ifdef BUG_WITHHELD_RETRY
                    if :: payload_withheld -> notification_calls++ :: else -> skip fi;
#endif
                    pc = 10
                fi
            :: pc == 10 ->
                assert(attempt_tenant == 1 && attempt_event == frozen_event && attempt_destination == frozen_destination);
#ifdef BUG_RECIPIENT_CERTAINTY
                if :: provider_outcome == Accepted -> recipient_delivery_known = true :: else -> skip fi;
#endif
                terminal = Settled; choose_replay()
            :: pc == 20 ->
                if
                :: attempt_tenant != 1 -> foreign_rejected = true; terminal = Denied; finished = true
                :: attempt_tenant == 1 ->
                    if
                    :: attempt_event != frozen_event ->
#ifdef BUG_EVENT_REBIND
                        frozen_event = attempt_event;
#else
                        conflict_rejected = true;
#endif
                        terminal = Denied; finished = true
                    :: attempt_event == frozen_event -> pc = 21
                    fi
                fi
            :: pc == 21 ->
                if
                :: attempt_destination != frozen_destination ->
#ifdef BUG_DESTINATION_REBIND
                    frozen_destination = attempt_destination;
#else
                    destination_conflict = true;
#endif
                    terminal = Denied; finished = true
                :: attempt_destination == frozen_destination ->
                        if
                        :: intent_state == Unknown || notification_state == Unknown ->
#ifdef BUG_UNKNOWN_RETRY
                            notification_calls++;
#endif
                            assert(notification_calls == sealed_notification_calls);
                            unknown_retry_blocked = true; terminal = Blocked; finished = true
                        :: else -> pc = 0
                        fi
                fi
            fi
        fi;
        invariants()
    }
    od;
#ifdef WITNESS_ACCEPTED
    assert(!(notification_receipt && provider_outcome == Accepted));
#endif
#ifdef WITNESS_REFUSED
    assert(!(notification_receipt && provider_outcome == Refused));
#endif
#ifdef WITNESS_THROTTLED
    assert(!(notification_receipt && provider_outcome == Throttled));
#endif
#ifdef WITNESS_UNKNOWN
    assert(notification_state != Unknown);
#endif
#ifdef WITNESS_BETWEEN_EFFECTS
    assert(!(crashed_between_effects && intent_receipt && notification_receipt));
#endif
#ifdef WITNESS_WITHHELD_COMPLETION
    assert(!(notification_receipt && payload_withheld && provider_outcome == Accepted));
#endif
#ifdef WITNESS_WITHHELD_INTENT
    assert(!(intent_receipt && !intent_exposed && notification_calls == 0));
#endif
#ifdef WITNESS_REVOCATION
    assert(!revoked_before_notification);
#endif
#ifdef WITNESS_REPLAY
    assert(!(replayed && notification_receipt && terminal == Settled));
#endif
#ifdef WITNESS_EVENT_CONFLICT
    assert(!conflict_rejected);
#endif
#ifdef WITNESS_FOREIGN_DENIAL
    assert(!foreign_rejected);
#endif
#ifdef WITNESS_NOT_SENT
    assert(!(notification_receipt && provider_outcome == NotSent));
#endif
#ifdef WITNESS_DESTINATION_CONFLICT
    assert(!destination_conflict);
#endif
#ifdef WITNESS_UNKNOWN_RETRY
    assert(!unknown_retry_blocked);
#endif
}

init {
    atomic { run Handler(); run Endpoint(); run Revoke(); run Crash() }
}

ltl privacy_response_settles { <> finished }
