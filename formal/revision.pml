/* One serialised run, two candidate identities and at most four invocations.
 * reserve/start/effect-checkpoint/invocation-checkpoint are separate boundaries.
 * Faults may interrupt any boundary once. A retained completed invocation is
 * reused; an interrupted external effect blocks instead of being repeated. Native
 * handler computation may restart and reuse a completed external effect.
 * Deadlines are abstract observations, not Promela timeout or clock proofs.
 */
#define LIMIT 4
mtype = { Running, Completed, Exhausted, Cancelled, Expired, Blocked };
mtype terminal = Running;
byte stage;
byte pc;
byte spent;
byte frozen_limit;
bool budget_exhausted;
bool stall_bit;
byte effect_dispatches[4];
bool reserved[4];
bool complete[4];
bool effect_complete[4];
bool inflight[4];
byte verdict[4]; /* 0 unset, 1 pass, 2 finding, 3 evaluator error */
byte candidate;
byte selected;
byte revisions;
byte remediation_effects;
byte permitted_effects = 1; /* 1 = inference only; 2 would include remediation */
byte frozen_deadline = 3;
byte active_deadline = 3;
bool cancelled;
bool expired;
bool crashed;
bool restarted;
bool replayed;
bool ever_started;
bool unknown_seen;
bool retained_error;

inline invariants() {
    assert(spent <= frozen_limit && frozen_limit <= LIMIT);
    assert(revisions <= 1);
    assert(permitted_effects == 1);
    assert(active_deadline == frozen_deadline);
    assert(remediation_effects == 0);
    assert(effect_dispatches[0] <= 1 && effect_dispatches[1] <= 1 &&
           effect_dispatches[2] <= 1 && effect_dispatches[3] <= 1);
    assert(terminal != Completed ||
           ((selected == 1 && complete[1] && verdict[1] == 1) ||
            (selected == 2 && complete[3] && verdict[3] == 1)));
}

proctype Disruption() {
    if
    :: skip
    :: cancelled = true
    :: expired = true
    :: crashed = true
    fi
}

proctype Controller() {
    do
    :: terminal != Running -> break
    :: terminal == Running ->
        atomic {
            if
            :: cancelled -> terminal = Cancelled
            :: !cancelled && expired -> terminal = Expired
            :: !cancelled && !expired && crashed ->
                restarted = true;
                crashed = false;
                if
                :: inflight[stage] && !effect_complete[stage] ->
                    unknown_seen = true;
#ifdef BUG_UNKNOWN_RETRY
                    inflight[stage] = false; pc = 1
#else
                    terminal = Blocked
#endif
                :: else -> pc = 0
                fi
            :: !cancelled && !expired && !crashed ->
                if
                :: pc == 0 ->
                    if
                    :: complete[stage] ->
                        replayed = true;
#ifdef BUG_REPLAY
                        effect_complete[stage] = false; pc = 1
#else
                        pc = 4
#endif
                    :: !complete[stage] ->
                        if
                        :: !reserved[stage] ->
                            if
                            :: spent < frozen_limit -> reserved[stage] = true; spent++
                            :: else -> budget_exhausted = true; terminal = Exhausted
                            fi
                        :: else -> skip
                        fi;
                        pc = 1
                    fi
                :: pc == 1 ->
                    assert(reserved[stage]);
                    if
                    :: effect_complete[stage] -> replayed = true; pc = 3
                    :: else -> effect_dispatches[stage]++;
                        inflight[stage] = true;
                        ever_started = true;
                        pc = 2
                    fi
                :: pc == 2 ->
                    if
                    :: stage == 0 || stage == 2 ->
                        candidate = (stage / 2) + 1
                    :: else ->
                        if
                        :: verdict[stage] = 1
                        :: verdict[stage] = 2
                        :: verdict[stage] = 3; retained_error = true
                        fi
                    fi;
                    effect_complete[stage] = true;
                    inflight[stage] = false;
                    pc = 3
                :: pc == 3 -> complete[stage] = true; pc = 4
                :: pc == 4 ->
                    if
                    :: stage == 0 || stage == 2 -> stage++; pc = 0
                    :: stage == 1 || stage == 3 ->
                        if
                        :: verdict[stage] == 1 -> selected = candidate; terminal = Completed
                        :: verdict[stage] != 1 ->
#ifdef BUG_ERROR_PASS
                            if
                            :: verdict[stage] == 3 -> selected = candidate; terminal = Completed
                            :: else -> skip
                            fi;
#endif
                            if
                            :: stage == 1 && terminal == Running ->
                                revisions++;
#ifdef BUG_BUDGET
                                revisions++;
#endif
#ifdef BUG_PERMISSION
                                permitted_effects = 2;
#endif
#ifdef BUG_DEADLINE
                                active_deadline++;
#endif
#ifdef BUG_REMEDIATION
                                remediation_effects++;
#endif
                                stage = 2; pc = 0
                            :: stage == 3 && terminal == Running ->
                                selected = candidate; terminal = Exhausted
                            :: else -> skip
                            fi
                        fi
                    fi
                fi
            fi;
            invariants()
        }
#ifdef BUG_STALL
        if
        :: ever_started && terminal == Running -> do :: stall_bit = !stall_bit od
        :: else -> skip
        fi;
#endif
    od;
#ifdef WITNESS_COMPLETE
    assert(terminal != Completed)
#endif
#ifdef WITNESS_REPLAY
    assert(!replayed)
#endif
#ifdef WITNESS_UNKNOWN
    assert(!unknown_seen)
#endif
#ifdef WITNESS_EXHAUSTED
    assert(terminal != Exhausted)
#endif
#ifdef WITNESS_INVOCATION_BUDGET
    assert(!budget_exhausted)
#endif
#ifdef WITNESS_ERROR
    assert(!retained_error)
#endif
}

init {
    atomic {
        if :: frozen_limit = 1 :: frozen_limit = 2
           :: frozen_limit = 3 :: frozen_limit = 4 fi;
        run Disruption(); run Controller();
    }
}

ltl settles { <> (terminal != Running) }
