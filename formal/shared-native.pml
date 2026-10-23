/* One host invocation, one native admission and one authority handover.
 * Exact JSON, PostgreSQL commit/refinement and guardian behaviour are separate
 * implementation obligations. No automatic recovery or retry is modelled. */
mtype={Prepared,Executing,Completed,Unknown};
mtype state=Prepared;
byte epoch=1;
byte admitted_epoch;
byte allocations;
byte dispatches;
bool committed;
bool fresh_permit;
bool reply_lost;
bool completed_seen;
bool unknown_seen;
bool stale_completion_blocked;
bool finished;

active proctype Authority(){ if :: epoch=2 :: skip fi }
active proctype Host(){
    if
    :: atomic { committed=true; allocations++; admitted_epoch=epoch; state=Executing }
       if :: fresh_permit=true :: reply_lost=true fi
    :: skip
    fi;
    if
    :: fresh_permit ->
       atomic { fresh_permit=false; dispatches++; assert(dispatches<=1) };
       if
       :: atomic {
             if
             :: epoch==admitted_epoch -> state=Completed; completed_seen=true
             :: else ->
#ifdef BUG_STALE_COMPLETE
                state=Completed; assert(epoch==admitted_epoch)
#else
                state=Unknown; stale_completion_blocked=true; unknown_seen=true
#endif
             fi
          }
       :: state=Unknown; unknown_seen=true
       fi
    :: else -> if :: committed -> unknown_seen=true :: else -> skip fi
    fi;
    /* An identical command can yield only its retained receipt/state. */
#ifdef BUG_REPLAY_DISPATCH
    if :: committed -> dispatches++; assert(dispatches<=1) :: else -> skip fi;
#endif
#ifdef BUG_UNKNOWN_RETRY
    if :: reply_lost -> allocations++; assert(allocations<=1) :: else -> skip fi;
#endif
#ifdef BUG_ERASE_COMPLETION
    if :: completed_seen -> state=Unknown :: else -> skip fi;
#endif
    assert(allocations<=1);
    assert(dispatches<=allocations);
    assert(!completed_seen || state==Completed);
    assert(!fresh_permit);
#ifdef WITNESS_COMPLETE
    assert(!completed_seen);
#endif
#ifdef WITNESS_UNKNOWN
    assert(!unknown_seen);
#endif
#ifdef WITNESS_LOST_ADMISSION
    assert(!reply_lost);
#endif
#ifdef WITNESS_FENCED
    assert(!stale_completion_blocked);
#endif
    finished=true
}
ltl shared_native_finishes { <> finished }
