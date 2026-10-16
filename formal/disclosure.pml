/* Two worlds share public evidence and invocation policy; only withheld peer
 * findings differ. Reviewer input assembly must agree in both worlds. This is
 * a finite self-composition of the host-mediated input operation, not a claim
 * about files, model-provider state or malicious workers outside the boundary.
 */
byte public_evidence = 1;
byte peer_a = 1;
byte peer_b = 2;
byte reviewer_input_a;
byte reviewer_input_b;
byte snapshot_a;
byte snapshot_b;
byte active_policy = 1; /* Independent reviewer: evidence yes, peer/feedback no */
byte frozen_policy;
byte author_input;
bool evidence_reader = true;
bool feedback_reader;
bool output_reader;
bool supplied_feedback;
bool rejected_rebind;
bool assembled;
bool completed;
bool delegate_permitted;
bool delegated;
bool inherited_reader;
bool parent_viewer;

init {
    /* Trusted configuration makes either policy choice; payload cannot do so. */
    if :: feedback_reader = false :: feedback_reader = true fi;
    atomic {
        frozen_policy = active_policy;
        reviewer_input_a = public_evidence;
        reviewer_input_b = public_evidence;
#ifdef BUG_HIDDEN_PEER
        reviewer_input_a = reviewer_input_a + peer_a;
        reviewer_input_b = reviewer_input_b + peer_b;
#endif
        assert(reviewer_input_a == reviewer_input_b);
        snapshot_a = reviewer_input_a;
        snapshot_b = reviewer_input_b;
        assembled = true
    };
    /* Later peer findings and policy are different. Replay uses stored input
     * and the policy attached to the original invocation identity. */
    peer_a = 2; peer_b = 1;
    public_evidence = 2;
#ifdef BUG_REASSEMBLE
    reviewer_input_a = public_evidence
#else
    reviewer_input_a = snapshot_a
#endif
    reviewer_input_b = snapshot_b;
    assert(reviewer_input_a == snapshot_a && reviewer_input_b == snapshot_b);
    /* Same invocation identity + changed policy must be rejected. */
#ifdef BUG_POLICY_REBIND
    active_policy = 2; rejected_rebind = false
#else
    rejected_rebind = true
#endif
    assert(rejected_rebind && active_policy == frozen_policy);
    /* A reviser's authorised feedback may exclude a playback reader. Every
     * derived output inherits the intersection of its supplied input readers. */
    supplied_feedback = true;
    author_input = peer_a;
#ifdef BUG_DERIVED_READER
    output_reader = evidence_reader
#else
    output_reader = evidence_reader && feedback_reader
#endif
    assert(!output_reader || (evidence_reader && feedback_reader));
    /* Parent policy alone cannot authorise disclosure to a child. */
    if :: delegate_permitted = false :: delegate_permitted = true fi;
    if
#ifdef BUG_DELEGATION_DISCLOSURE
    :: delegate_permitted -> delegated = true
#else
    :: delegate_permitted && output_reader -> delegated = true
#endif
    :: else -> delegated = false
    fi;
    assert(!delegated || (delegate_permitted && output_reader));
    if :: parent_viewer = false :: parent_viewer = true fi;
    if
    :: delegated ->
#ifdef BUG_DELEGATION_INHERITANCE
        inherited_reader = true
#else
        inherited_reader = parent_viewer
#endif
    :: else -> inherited_reader = false
    fi;
    assert(!inherited_reader || parent_viewer);
    completed = true;
#ifdef WITNESS_RESTRICTED
    assert(!supplied_feedback || feedback_reader || output_reader)
#endif
}

ltl context_finishes { [] (assembled -> <> completed) }
