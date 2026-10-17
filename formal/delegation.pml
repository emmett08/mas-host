/* Optional host policy: completion requires min_delegate_calls successful peer
 * replies. One invocation, one permitted peer, three inference turns, minimum
 * 0..2, and at most one replay. Provider/delegate outcomes return or expire.
 * No payload, protocol delivery, wall-clock or provider behaviour is proved.
 */
#define MAX_TURNS 3
mtype = { Running, Completed, Invalid, Unavailable, Unknown,
          DelegateFailed, Exhausted, Stopped };
mtype terminal = Running;
byte minimum;
byte turns;
byte successful;
byte retained_successful;
byte provider_dispatches;
byte peer_dispatches;
byte saved_provider_dispatches;
byte saved_peer_dispatches;
bool complete_allowed;
bool replayed;
bool finished;

inline replay_history() {
    /* Re-entering execute reconstructs history from retained inference/peer
     * results. Each retained successful peer turn contributes exactly once.
     * This abstracts replay traversal; the active turn does not advance. */
    saved_provider_dispatches = provider_dispatches;
    saved_peer_dispatches = peer_dispatches;
    successful = 0;
    do
    :: successful < retained_successful -> successful++
    :: else -> break
    od;
#ifdef BUG_REPLAY_COUNT
    successful++;
#endif
#ifdef BUG_REPLAY_DISPATCH
    peer_dispatches++;
#endif
    assert(successful == retained_successful);
    assert(provider_dispatches == saved_provider_dispatches);
    assert(peer_dispatches == saved_peer_dispatches);
    replayed = true
}

init {
    /* The configuration validator admits 0 <= minimum < max_turns and
     * requires a permitted delegate for a positive minimum. All represented
     * configurations have one delegate, including the backwards-compatible 0. */
    if :: minimum = 0 :: minimum = 1 :: minimum = 2 fi;
    do
    :: terminal == Running ->
#ifdef BUG_STALL
        if :: skip :: break fi
#else
        if
        :: turns == MAX_TURNS -> terminal = Exhausted
        :: turns < MAX_TURNS ->
            complete_allowed = successful >= minimum;
#ifdef BUG_SCHEMA
            complete_allowed = true;
#endif
            assert(complete_allowed == (successful >= minimum));
            turns++;
            provider_dispatches++;
            /* A provider may ignore the advertised schema. Local validation
             * makes early completion invalid; it does not initiate a retry. */
            if
            :: /* complete decision */
#ifdef BUG_EARLY_COMPLETE
                terminal = Completed
#else
                if
                :: complete_allowed && successful >= minimum -> terminal = Completed
                :: else -> terminal = Invalid
                fi
#endif
            :: /* request decision */
                if
                :: turns == MAX_TURNS -> terminal = Exhausted
                :: turns < MAX_TURNS ->
                    peer_dispatches++;
                    if
                    :: successful++; retained_successful++;
                        if
                        :: !replayed -> replay_history()
                        :: skip
                        fi
                    :: terminal = DelegateFailed
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
    assert(successful == retained_successful);
    assert(provider_dispatches <= MAX_TURNS);
    assert(peer_dispatches < MAX_TURNS);
    assert(successful <= peer_dispatches);
    finished = true;
#ifdef WITNESS_COMPLETE
    assert(!(terminal == Completed && minimum == 2 && successful == 2));
#endif
#ifdef WITNESS_ZERO
    assert(!(terminal == Completed && minimum == 0 && successful == 0));
#endif
#ifdef WITNESS_REPLAY
    assert(!(replayed && terminal == Completed));
#endif
#ifdef WITNESS_EARLY_INVALID
    assert(!(terminal == Invalid && minimum > 0 && turns == 1));
#endif
}

/* Returning/expiring external work is an environment premise. Success is
 * reachable, not universal: invalid, unknown and failed work terminates too. */
ltl delegation_finishes { <> finished }
