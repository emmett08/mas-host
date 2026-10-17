/* Optional host delegation policy. One invocation, two permitted peers, three
 * inference turns, minimum 0..2, every required subset, and at most one replay.
 * Required coverage counts distinct successful retained peer replies. Provider
 * and delegate work returns or expires; payload usefulness is outside scope.
 */
#define MAX_TURNS 3
#define ALL_PEERS 3
#define COVERED ((covered & required) == required)
mtype = { Running, Completed, Invalid, Unavailable, Unknown,
          DelegateFailed, Exhausted, Stopped };
mtype terminal = Running;
byte minimum;
byte required;
byte turns;
byte successful;
byte retained_successful;
byte covered;
byte retained_covered;
byte provider_dispatches;
byte peer_dispatches;
byte saved_provider_dispatches;
byte saved_peer_dispatches;
byte permitted_requests;
byte selected;
byte first_peer;
bool complete_allowed;
bool request_allowed;
bool duplicate_rejected;
bool partial_completion_rejected;
bool early_completion_rejected;
bool replayed;
bool finished;

inline replay_history() {
    /* Re-entering execute reconstructs history from retained inference/peer
     * results. Each retained successful peer turn contributes exactly once.
     * The bitset abstracts identity-based union over those retained turns. */
    saved_provider_dispatches = provider_dispatches;
    saved_peer_dispatches = peer_dispatches;
    successful = 0;
    do
    :: successful < retained_successful -> successful++
    :: else -> break
    od;
    covered = retained_covered;
#ifdef BUG_REPLAY_COUNT
    successful++;
#endif
#ifdef BUG_REPLAY_COVERAGE
    covered = ALL_PEERS;
#endif
#ifdef BUG_REPLAY_DISPATCH
    peer_dispatches++;
#endif
    assert(successful == retained_successful);
    assert(covered == retained_covered);
    assert(provider_dispatches == saved_provider_dispatches);
    assert(peer_dispatches == saved_peer_dispatches);
    replayed = true
}

init {
    /* Valid configurations: minimum < max_turns and enough turns for every
     * distinct required peer plus completion. Both peers are always allowed;
     * required == 0 preserves the earlier optional-delegation policy. */
    if :: minimum = 0 :: minimum = 1 :: minimum = 2 fi;
    if :: required = 0 :: required = 1 :: required = 2 :: required = 3 fi;
    do
    :: terminal == Running ->
#ifdef BUG_STALL
        if :: skip :: break fi
#else
        if
        :: turns == MAX_TURNS -> terminal = Exhausted
        :: turns < MAX_TURNS ->
            complete_allowed = successful >= minimum && COVERED;
            request_allowed = required == 0 || turns + 1 < MAX_TURNS;
            if
            :: COVERED -> permitted_requests = ALL_PEERS
            :: else -> permitted_requests = required & (ALL_PEERS ^ covered)
            fi;
#ifdef BUG_SCHEMA
            complete_allowed = true;
#endif
#ifdef BUG_TARGET_SCHEMA
            permitted_requests = ALL_PEERS;
#endif
#ifdef BUG_DUPLICATE_TARGET_SCHEMA
            if :: !COVERED && covered != 0 -> permitted_requests = ALL_PEERS
               :: else -> skip fi;
#endif
#ifdef BUG_FINAL_REQUEST
            request_allowed = true;
#endif
            assert(complete_allowed == (successful >= minimum && COVERED));
            assert(COVERED || permitted_requests == (required & (ALL_PEERS ^ covered)));
            assert(request_allowed == (required == 0 || turns + 1 < MAX_TURNS));
            turns++;
            provider_dispatches++;
            /* The provider may ignore its schema. Local validation rejects
             * early completion and out-of-schema targets without a retry. */
            if
            :: /* complete decision */
#ifdef BUG_EARLY_COMPLETE
                if :: COVERED -> terminal = Completed
                   :: else -> terminal = Invalid fi
#else
#ifdef BUG_PARTIAL_COMPLETE
                if :: successful >= minimum -> terminal = Completed
                   :: else -> terminal = Invalid fi
#else
                if
                :: complete_allowed && successful >= minimum && COVERED -> terminal = Completed
                :: else ->
                    if :: successful < minimum -> early_completion_rejected = true
                       :: else -> skip fi;
                    if
                    :: successful >= minimum && !COVERED -> partial_completion_rejected = true
                    :: else -> skip
                    fi;
                    terminal = Invalid
                fi
#endif
#endif
            :: /* request decision */
                if :: selected = 1 :: selected = 2 fi;
                if
                :: !request_allowed -> terminal = Invalid
                :: request_allowed ->
                    if
                    :: (selected & permitted_requests) == 0 ->
                        if
                        :: (selected & covered) != 0 -> duplicate_rejected = true
                        :: else -> skip
                        fi;
                        terminal = Invalid
                    :: (selected & permitted_requests) != 0 ->
                        if
                        :: turns == MAX_TURNS -> terminal = Exhausted
                        :: turns < MAX_TURNS ->
                            peer_dispatches++;
                            if
                            :: successful++;
                                retained_successful++;
                                covered = covered | selected;
                                retained_covered = retained_covered | selected;
                                if :: first_peer == 0 -> first_peer = selected :: else -> skip fi;
                                if
                                :: !replayed -> replay_history()
                                :: skip
                                fi
                            :: terminal = DelegateFailed
                            fi
                        fi
                    fi
                fi
            :: terminal = Invalid
            :: terminal = Unavailable
            :: terminal = Unknown
            :: terminal = Stopped
            fi
        fi
#endif
    :: terminal != Running -> break
    od;
    assert(terminal != Completed || successful >= minimum);
    assert(terminal != Completed || COVERED);
    assert(successful == retained_successful);
    assert(covered == retained_covered);
    assert(provider_dispatches <= MAX_TURNS);
    assert(peer_dispatches < MAX_TURNS);
    assert(successful <= peer_dispatches);
    finished = true;
#ifdef WITNESS_COMPLETE
    assert(!(terminal == Completed && required == ALL_PEERS && successful == 2));
#endif
#ifdef WITNESS_ZERO
    assert(!(terminal == Completed && minimum == 0 && required == 0 && successful == 0));
#endif
#ifdef WITNESS_REPLAY
    assert(!(replayed && terminal == Completed && required == ALL_PEERS));
#endif
#ifdef WITNESS_EARLY_INVALID
    assert(!(terminal == Invalid && early_completion_rejected && minimum > 0 && turns == 1));
#endif
#ifdef WITNESS_PARTIAL_INVALID
    assert(!(terminal == Invalid && partial_completion_rejected && required == ALL_PEERS && successful == 1));
#endif
#ifdef WITNESS_DUPLICATE_INVALID
    assert(!(terminal == Invalid && duplicate_rejected && peer_dispatches == 1));
#endif
#ifdef WITNESS_REVERSE_ORDER
    assert(!(terminal == Completed && required == ALL_PEERS && first_peer == 2));
#endif
}

/* Returning/expiring external work is an environment premise. Success is
 * reachable, not universal: invalid, unknown and failed work terminates too. */
ltl delegation_finishes { <> finished }
