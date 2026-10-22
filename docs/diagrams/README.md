# Animated host architecture

The root README embeds a committed GIF so the animation plays within GitHub's
repository view. The diagram also has a static [SVG](mas-host-architecture.svg)
and [PNG](mas-host-architecture.png). Its first GIF frame contains the complete
overview. The README's `picture` element selects the PNG for a reduced-motion
preference. GitHub also provides [animated-image accessibility settings](https://docs.github.com/en/account-and-profile/how-tos/account-settings/managing-accessibility-settings).

The diagram represents main commit
`8b535be6d3de18b8288858fdf6ed79f67ce9155e`, which includes
[PR #19](https://github.com/emmett08/mas-host/pull/19), plus
[PR #20](https://github.com/emmett08/mas-host/pull/20) at source revision
`4fd4b536e6589d631f6a475f4ddd68b0d30ffade`.
Timing, coloured packets and the selected worker choreography are explanatory
animation. Each worker box represents an implementation kind that an application
can select. The effect sequence illustrates one provider operation.

## Follow the flow

1. Application source, agent bindings and topology become generated launchers
   and a frozen bundle. Executable, bundle and root-input identity bind a run.
2. The local CLI and optional A2A service enter the same Rust core. A2A adds
   authenticated exports, principal-scoped tasks and its own service journal.
3. The protocol library owns routes, messages, tasks and call reservations in
   `protocol.sqlite`. The host owns named steps and effect checkpoints in
   `execution.sqlite`. Source associations and invocation exposure also belong
   to that execution journal. These databases have separate transaction boundaries.
4. The guardian supervises worker subprocesses. Private `mas-native/1` IPC uses
   a Unix socketpair carrying length-prefixed JSON. The host derives the sender
   identity and mediates every logical cross-agent request.
5. Native handlers, command adapters, provider adapters and workflow workers
   share that execution contract. Workflows delegate back through the host.
6. An effect obtains durable admission before the worker calls its external
   system. Completion retains output and evidence. Lost completion responses
   preserve uncertainty and require evidence-backed reconciliation.
7. The optional governed A2A profile checks current authority and structured
   projections at its protected boundaries. Configured local inspection uses
   pinned neural assets and deterministic recognisers. Failed inspection stops
   admission or release according to that boundary's contract. Trusted worker
   execution and deployment enforcement retain their existing limits.
8. PR #20's opt-in `mas-governed-local/2` profile retains pending acquisition
   before returning effect permission. Pending acquisition blocks new outbound
   work. Completion seals the retained projection, source association and
   recipient exposure in the existing journal transaction. Acquisition time
   and expiry originate at admission. V2 applies the pending rule to every
   effect, including baseline-only source policies.
9. The complete original source ancestry, policy and expiry constrain later
   requests, effects, detector processing and delivery. A child inherits its
   parent's exposure before input persistence and dispatch. Parallel child
   requests freeze the parent's exposure, while subsequently admitted results
   constrain later commands. Missing, conflicting, cyclic or expired lineage
   blocks admission. The supported complete closure contains at most 64 sources.
   The source-restrictions card expands host-owned journal records in the
   drawing. Current grants and policy templates remain operator-owned authority.
10. `GetRetainedEffect` and `GetRetainedSource` derive scope from an authorised
    A2A task and require separate current `resolve` permission. In v2 namespaces,
    current complete task exposure joins the selected source's ancestry as an
    admission constraint. Pending acquisition can make reads unavailable. A
    bounded SQLite read snapshot binds the retained record. The transaction
    ends before detector callbacks. Query and returned data pass configured
    inspection gates. Source reads return the retained structured projection,
    with discarded original bytes marked `raw_source_disposition: not-retained`.
    The readers preserve execution state and external uncertainty.
11. The privacy adapter combines two separately authorised effect views to
   reconstruct case status. Host settlement, payload disposition and external
   execution certainty remain distinct. Recipient delivery remains unknown.
12. `mas-host observe` reads ordinary run state from the two stores using
    independent snapshots. Payload inclusion requires explicit opt-in. Governed
    namespaces retain their separate authorised read contract.

The Linux outline shows process and storage ownership in the trusted-local
profile. The guardian enforces the original deadline and cleans up the inherited
process group. Application code runs with the operator's filesystem permissions.
The diagram's read-only arrows carry observations and confer no execution
permission. Effect reconciliation remains a separate operator action.

## Contracts and source correspondence

| Diagram element | Contract | Implementation |
|---|---|---|
| Build and frozen bundle | [Architecture](../architecture.md), [authoring](../authoring.md) | `crates/mas-host-cli/src/build.rs`, `crates/mas-host-native-build/` |
| Core and protocol authority | [Native contract](../native-contract.md) | `crates/mas-host-core/src/execution.rs`, `protocol.rs`, pinned `mas-protocol-codec` dependency |
| Worker kinds and IPC | [LLM/workflows](../llm-workflows.md), [commands](../commands.md), [MCP tools](../mcp-tools.md) | `crates/mas-host-worker/`, `crates/mas-host-runtime/`, `crates/mas-host-workflow/` |
| Effect admission and settlement | [Architecture](../architecture.md) | `crates/mas-host-core/src/effects.rs`, `journal.rs` |
| A2A ingress and service journal | [A2A](../a2a.md) | `crates/mas-host-cli/src/a2a/` |
| Optional governance and inspection | [Governed local path](../governed-local.md), [privacy mediation](../governed-privacy.md) | `crates/mas-host-core/src/governance/runtime/`, `crates/mas-host-cli/src/privacy_inspect/` |
| Governed acquisition and inheritance | [Acquisition contract at PR #20](https://github.com/emmett08/mas-host/blob/4fd4b536e6589d631f6a475f4ddd68b0d30ffade/docs/governed-acquisition.md) | `crates/mas-host-core/src/governance/runtime/source.rs`, `lineage.rs`, `exposure.rs`, `exposure_authorization.rs` |
| Authorised effect/source readers | [Retained-effect contract](../governed-evidence-reader.md), [PR #20 source-read contract](https://github.com/emmett08/mas-host/blob/4fd4b536e6589d631f6a475f4ddd68b0d30ffade/docs/governed-acquisition.md#read-the-retained-projection) | `crates/mas-host-core/src/governance/runtime/resolver.rs`, `source_resolver.rs`, `crates/mas-host-cli/src/a2a/retained_effect.rs`, `retained_source.rs` |
| Privacy case projection | [Notification adapter](../../adapters/notifications/README.md) | `adapters/notifications/privacy_response/case_status.py` |
| Ordinary observation | [Observation](../observation.md) | `crates/mas-host-core/src/observation.rs` |

The retained-effect reader and case-status sources were introduced by PR #19.
PR #20 adds complete bounded source ancestry and invocation exposure. Its
qualification covers at most 64 sources and 65,536 representation bytes.
Version 1 retains its existing policy semantics. Shared ownership, arbitrary
free-text release, source-byte adjudication, shared subscribers, durable
notification retries and escalation retain the follow-on qualification gates recorded in
[the implementation sequence](../nehemiah-implementation.md).

## Regenerate

`render_architecture.py` contains the scene, labels, edges and animation stages.
Its shared drawing operations produce all three committed assets. The generator
uses Pillow for deterministic frame selection and GIF encoding. It runs entirely
offline once its font and rendering dependency have been installed.

On Linux with DejaVu Sans installed in `/usr/share/fonts/truetype/dejavu`:

```sh
python3 -m venv /tmp/mas-host-diagram-venv
/tmp/mas-host-diagram-venv/bin/pip install Pillow==12.3.0
/tmp/mas-host-diagram-venv/bin/python docs/diagrams/render_architecture.py
```

Use `--font-dir PATH` for another installation of the same font. Use
`--output-dir PATH` to render a review copy, or `--contact-sheet PATH` for a
compact visual review. Font versions can change rasterisation, so regenerate all
three assets together and inspect them before committing. The host runtime and
its build have no dependency on this documentation renderer.

The GIF loops through twelve stages, each with four seconds of motion and a
two-second reading hold, with a complete overview at the start and end.
A fixed palette and frame deltas limit its size. Asset review
should check the README-sized rendering, arrow directions, captions, first
frame, loop duration, static fallback and source correspondence.
