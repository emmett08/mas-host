/* Nehemiah host inference-result profile: one current inference effect, up to
 * three provider messages, one downstream external effect, one optional retained
 * earlier peer, one independently scheduled crash and one terminal replay.
 * ValueA/ValueB denote exact parsed JSON values; parsing/normalisation itself is
 * covered by implementation tests. In particular nested input_json strings are
 * opaque values: different strings can map to A/B even if their JSON meanings
 * would be equivalent. This is not a proof of the wire codec or Rust refinement.
 */
#define ABSENT 0
#define NULL_PHASE 1
#define FINAL 2
#define COMMENTARY 3
#define VALUE_A 1
#define VALUE_B 2
#define MALFORMED 3
#define MAX_MESSAGES 3
#define EXPECTED_SELECTION (response_kind == Single || response_kind == Repeated || response_kind == Triplicated || response_kind == CommentaryFinal || response_kind == SchemaInvalid || response_kind == RepeatedSchemaInvalid || response_kind == RequestInvalid)
#define EXPECTED_ADMISSION (EXPECTED_SELECTION && schema_valid && request_valid)
#define EXPECTED_UNKNOWN (response_kind == TransportUnknown || response_kind == Nonterminal)

mtype = { Single, Repeated, Triplicated, Conflicting, OpaqueStringConflict,
          MalformedSibling, CommentaryOnly, CommentaryFinal, Refused, Incomplete,
          TransportUnknown, Nonterminal, SchemaInvalid, RepeatedSchemaInvalid,
          RequestInvalid, SplitText, EmptyMessage, NonstringText,
          Free, Prepared, Executing, Known, Uncertain,
          Unset, Valid, Invalid, Running, Completed, Failed, Blocked };
mtype response_kind;
mtype inference_state = Free;
mtype delegate_state = Free;
mtype retained_status = Unset;
mtype sealed_status = Unset;
mtype local_status = Unset;
mtype terminal = Running;
mtype pending_terminal = Running;
mtype saved_terminal;
byte message_count;
byte phase[MAX_MESSAGES];
byte text_blocks[MAX_MESSAGES];
byte value[MAX_MESSAGES];
bool string_text[MAX_MESSAGES];
bool schema_valid = true;
bool request_valid = true;
bool refusal;
bool response_ready;
bool delegate_returns;
byte pc;
byte i;
byte eligible;
byte first_value;
bool conflicting;
bool malformed;
bool bad_shape;
bool selected;
bool admitted;
byte inference_calls;
byte prior_peer_calls;
byte peer_calls;
byte saved_inference_calls;
byte saved_peer_calls;
byte frozen_descriptor;
byte requested_descriptor = 1;
bool replay_requested;
bool replayed;
bool rebind_rejected;
bool cached_inference;
bool crash;
bool crashed_once;
bool unknown_seen;
bool transient_known_lost;
bool inference_observed;
bool finished;
bool stall_bit;

inline invariants() {
    assert(inference_calls <= 1);
    assert(peer_calls <= prior_peer_calls + 1);
    assert(frozen_descriptor == 0 || frozen_descriptor == 1);
    assert(inference_state != Known || retained_status == sealed_status);
    assert(peer_calls == prior_peer_calls ||
        (inference_state == Known && retained_status == Valid && EXPECTED_ADMISSION));
    assert(inference_state != Known || retained_status != Invalid ||
        peer_calls == prior_peer_calls);
    assert(terminal != Completed ||
        (inference_state == Known && retained_status == Valid &&
         delegate_state == Known && EXPECTED_ADMISSION));
    assert(!replay_requested || inference_calls == saved_inference_calls);
    assert(!replay_requested || peer_calls == saved_peer_calls);
}

proctype Provider() {
    byte last;
    byte saved_phase;
    byte saved_blocks;
    byte saved_value;
    bool saved_string;
    /* One provider participant. It returns an observation, or the owner has
     * already stopped before any dispatch. No fairness premise is needed: the
     * positive processes have no unbounded internal work or idle self-loop. */
    if
    :: inference_calls == 1 ->
        atomic {
            message_count = 1;
            phase[0] = FINAL; text_blocks[0] = 1;
            value[0] = VALUE_A; string_text[0] = true;
            phase[1] = NULL_PHASE; text_blocks[1] = 1;
            value[1] = VALUE_A; string_text[1] = true;
            phase[2] = ABSENT; text_blocks[2] = 1;
            value[2] = VALUE_A; string_text[2] = true;
            if
            :: response_kind = Single
            :: response_kind = Repeated; message_count = 2
            :: response_kind = Triplicated; message_count = 3
            :: response_kind = Conflicting; message_count = 2; value[1] = VALUE_B
            :: response_kind = OpaqueStringConflict; message_count = 2; value[1] = VALUE_B
            :: response_kind = MalformedSibling; message_count = 2; value[1] = MALFORMED
            :: response_kind = CommentaryOnly; phase[0] = COMMENTARY
            :: response_kind = CommentaryFinal; message_count = 2;
                phase[0] = COMMENTARY; value[0] = MALFORMED; phase[1] = FINAL
            :: response_kind = Refused; message_count = 2;
                phase[1] = COMMENTARY; refusal = true
            :: response_kind = Incomplete
            :: response_kind = TransportUnknown
            :: response_kind = Nonterminal
            :: response_kind = SchemaInvalid; schema_valid = false
            :: response_kind = RepeatedSchemaInvalid; message_count = 2; schema_valid = false
            :: response_kind = RequestInvalid; request_valid = false
            :: response_kind = SplitText; text_blocks[0] = 2
            :: response_kind = EmptyMessage; text_blocks[0] = 0
            :: response_kind = NonstringText; string_text[0] = false
            fi;
            /* Either order: a malformed/conflicting sibling is not assumed
             * to follow the value which a first-output policy would choose. */
            if
            :: skip
            :: message_count > 1 ->
                last = message_count - 1;
                saved_phase = phase[0]; saved_blocks = text_blocks[0];
                saved_value = value[0]; saved_string = string_text[0];
                phase[0] = phase[last]; text_blocks[0] = text_blocks[last];
                value[0] = value[last]; string_text[0] = string_text[last];
                phase[last] = saved_phase; text_blocks[last] = saved_blocks;
                value[last] = saved_value; string_text[last] = saved_string
            fi;
            response_ready = true
        }
    :: terminal != Running -> skip
    fi
}

proctype Crash() {
    if
    :: skip
    :: atomic { !finished; crash = true; crashed_once = true }
    fi
}

inline select_output() {
    eligible = 0; first_value = 0;
    conflicting = false; malformed = false; bad_shape = false;
    i = 0;
    do
    :: i < message_count ->
        if
        :: phase[i] == ABSENT || phase[i] == NULL_PHASE || phase[i] == FINAL ->
            eligible++;
            if
            :: text_blocks[i] != 1 || !string_text[i] -> bad_shape = true
            :: else ->
                if
                :: value[i] == MALFORMED -> malformed = true
                :: value[i] != MALFORMED ->
                    if
                    :: first_value == 0 -> first_value = value[i]
                    :: first_value != 0 && value[i] != first_value -> conflicting = true
                    :: else -> skip
                    fi
                fi
            fi
        :: else ->
#ifdef BUG_COMMENTARY
            /* Incorrectly promote excluded content to final output. */
            eligible++; first_value = value[i]
#else
            skip
#endif
        fi;
        i++
    :: else -> break
    od;
#ifdef BUG_FIRST_OUTPUT
    conflicting = false;
#endif
#ifdef BUG_MALFORMED_SIBLING
    if :: first_value != 0 -> malformed = false :: else -> skip fi;
#endif
#ifdef BUG_REFUSAL
    refusal = false;
#endif
    selected = eligible > 0 && !bad_shape && !malformed && !conflicting &&
        !refusal && response_kind != Incomplete;
#ifdef BUG_REJECT_REPEATED
    if :: eligible > 1 -> selected = false :: else -> skip fi;
#endif
    /* Selection is compared with an independent scenario specification. This
     * equality also asserts positive service for equivalent repeated messages. */
    assert(selected == EXPECTED_SELECTION)
}

inline retain_result() {
    retained_status = local_status; sealed_status = local_status;
    inference_state = Known; pc = 7
}

proctype Host() {
    do
    :: finished -> break
    :: !finished ->
        /* One iteration is one local step or one durable journal transition.
         * In particular executing, external dispatch, response selection,
         * schema validation and result retention are different iterations.
         * The crash process can interleave between every such boundary. */
        atomic {
            if
            :: crash ->
                crash = false;
                if
                :: inference_state == Executing && inference_observed ->
                    transient_known_lost = !EXPECTED_UNKNOWN
                :: else -> skip
                fi;
                pc = 0
            :: !crash && (pc != 3 || response_ready) ->
                if
                :: pc == 0 ->
                    if
                    :: terminal != Running -> pc = 13
                    :: terminal == Running ->
                        if
                        :: inference_state == Free ->
                            frozen_descriptor = 1; inference_state = Prepared; pc = 1
                        :: inference_state == Prepared -> pc = 1
                        :: inference_state == Executing || inference_state == Uncertain ->
                            inference_state = Uncertain; unknown_seen = true;
                            pending_terminal = Blocked; pc = 12
                        :: inference_state == Known -> cached_inference = true; pc = 7
                        fi
                    fi
                :: pc == 1 -> inference_state = Executing; pc = 2
                :: pc == 2 -> inference_calls++; pc = 3
                :: pc == 3 ->
                    inference_observed = true;
                    if
                    :: EXPECTED_UNKNOWN ->
                        inference_state = Uncertain; unknown_seen = true;
                        pending_terminal = Blocked; pc = 12
                    :: else -> pc = 4
                    fi
                :: pc == 4 -> select_output(); pc = 5
                :: pc == 5 ->
#ifdef BUG_SCHEMA
                    if :: selected -> local_status = Valid :: else -> local_status = Invalid fi;
#else
                    if :: selected && schema_valid -> local_status = Valid
                       :: else -> local_status = Invalid fi;
#endif
                    pc = 6
                :: pc == 6 ->
#ifdef BUG_INVALID_UNKNOWN
                    if
                    :: local_status == Invalid -> inference_state = Uncertain;
                        pending_terminal = Blocked; pc = 12
                    :: else -> retain_result()
                    fi
#else
                    retain_result()
#endif
                :: pc == 7 ->
                    admitted = retained_status == Valid && request_valid;
#ifdef BUG_REQUEST_ADMISSION
                    admitted = retained_status == Valid;
#endif
                    if
                    :: !admitted ->
                        pending_terminal = Failed; pc = 12
                    :: admitted ->
                        if
                        :: delegate_state == Free -> delegate_state = Prepared; pc = 8
                        :: delegate_state == Prepared -> pc = 8
                        :: delegate_state == Executing || delegate_state == Uncertain ->
                            delegate_state = Uncertain; unknown_seen = true;
                            pending_terminal = Blocked; pc = 12
                        :: delegate_state == Known -> pending_terminal = Completed; pc = 12
                        fi
                    fi
                :: pc == 8 -> delegate_state = Executing; pc = 9
                :: pc == 9 -> peer_calls++; pc = 10
                :: pc == 10 ->
                    if
                    :: delegate_returns -> pc = 11
                    :: !delegate_returns ->
                        delegate_state = Uncertain; unknown_seen = true;
                        pending_terminal = Blocked; pc = 12
                    fi
                :: pc == 11 -> delegate_state = Known;
                    pending_terminal = Completed; pc = 12
                :: pc == 12 -> terminal = pending_terminal; pc = 13
                :: pc == 13 ->
                    saved_inference_calls = inference_calls;
                    saved_peer_calls = peer_calls; saved_terminal = terminal;
                    replay_requested = true;
                    if :: requested_descriptor = 1 :: requested_descriptor = 2 fi;
                    pc = 14
                :: pc == 14 ->
                    if
                    :: requested_descriptor != frozen_descriptor ->
#ifdef BUG_DESCRIPTOR_REBIND
                        frozen_descriptor = requested_descriptor
#else
                        rebind_rejected = true
#endif
                    :: requested_descriptor == frozen_descriptor ->
#ifdef BUG_REPLAY
                        inference_calls++;
#endif
#ifdef BUG_PEER_REPLAY
                        if :: delegate_state == Known -> peer_calls++ :: else -> skip fi;
#endif
#ifdef BUG_INVALID_RETRY
                        if :: inference_state == Known && retained_status == Invalid -> inference_calls++
                           :: else -> skip fi;
#endif
#ifdef BUG_UNKNOWN_RETRY
                        if :: inference_state == Uncertain -> inference_calls++
                           :: else -> skip fi;
#endif
#ifdef BUG_RECLASSIFY
                        if :: retained_status == Invalid -> retained_status = Valid
                           :: else -> skip fi;
#endif
                        replayed = true
                    fi;
                    assert(requested_descriptor == frozen_descriptor || rebind_rejected);
                    assert(terminal == saved_terminal);
                    finished = true
                fi
            fi;
            invariants()
        }
#ifdef BUG_STALL
        if
        :: inference_calls > 0 && !finished -> do :: stall_bit = !stall_bit od
        :: else -> skip
        fi;
#endif
    od;
    /* Success is conditional on useful, permitted work, known delegate return
     * and no crash. The unconditional LTL property only requires settlement. */
    assert(crashed_once || !EXPECTED_ADMISSION || !delegate_returns || terminal == Completed);
    assert(crashed_once || EXPECTED_UNKNOWN || inference_state == Known);
    assert(crashed_once || EXPECTED_UNKNOWN || EXPECTED_ADMISSION || terminal == Failed);
#ifdef WITNESS_SINGLE
    assert(!(response_kind == Single && terminal == Completed && replayed));
#endif
#ifdef WITNESS_REPEATED
    assert(!(response_kind == Repeated && terminal == Completed && replayed));
#endif
#ifdef WITNESS_TRIPLICATED
    assert(!(response_kind == Triplicated && terminal == Completed && replayed));
#endif
#ifdef WITNESS_CONFLICT_AFTER_PEER
    assert(!(response_kind == Conflicting && prior_peer_calls == 1 &&
        inference_state == Known && retained_status == Invalid && replayed));
#endif
#ifdef WITNESS_MALFORMED
    assert(!(response_kind == MalformedSibling && inference_state == Known &&
        retained_status == Invalid && replayed));
#endif
#ifdef WITNESS_SCHEMA_INVALID
    assert(!(response_kind == RepeatedSchemaInvalid && inference_state == Known &&
        retained_status == Invalid && replayed));
#endif
#ifdef WITNESS_REQUEST_INVALID
    assert(!(response_kind == RequestInvalid && inference_state == Known &&
        retained_status == Valid && terminal == Failed && replayed));
#endif
#ifdef WITNESS_REFUSAL
    assert(!(response_kind == Refused && inference_state == Known &&
        retained_status == Invalid && terminal == Failed && replayed));
#endif
#ifdef WITNESS_UNKNOWN
    assert(!(EXPECTED_UNKNOWN && terminal == Blocked && replayed));
#endif
#ifdef WITNESS_LOST_KNOWN
    assert(!(transient_known_lost && inference_state == Uncertain &&
        terminal == Blocked && replayed));
#endif
#ifdef WITNESS_CACHED_INFERENCE
    assert(!(cached_inference && terminal == Completed && replayed));
#endif
#ifdef WITNESS_DELEGATE_CRASH
    assert(!(crashed_once && delegate_state == Uncertain && terminal == Blocked && replayed));
#endif
#ifdef WITNESS_REBIND
    assert(!rebind_rejected);
#endif
}

init {
    atomic {
        if :: prior_peer_calls = 0 :: prior_peer_calls = 1 fi;
        peer_calls = prior_peer_calls;
        if :: delegate_returns = false :: delegate_returns = true fi;
        run Provider(); run Crash(); run Host()
    }
}

/* All represented work returns or reaches its fixed external deadline. This
 * is terminal settlement (including failed/blocked), not guaranteed success,
 * response usefulness, eventual reconciliation or a quantitative time bound. */
ltl provider_results_settle { <> finished }
