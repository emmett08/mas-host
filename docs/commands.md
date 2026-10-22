# Ordinary CLI operations

`mas-agents/3` separates an operation from its implementation. A CLI tool is a
`command` implementation, independent of the LLM provider `command` connection.
The host runs a maintained adapter on its private worker channel; that adapter
launches the ordinary tool with argv and stdin, then translates stdout into an
operation result. The tool imports no MAS SDK. The existing Linux guardian
supervises foreground tools and descendants that keep the inherited process
group; there is one supervision implementation.

Tools must run in the foreground and must not detach themselves or their
descendants into another session or process group. The guardian terminates the
inherited process group and waits to reap adopted descendants. A detached
descendant can therefore delay cleanup and ownership-lock release beyond the
configured deadline. Use a tool's foreground option where available; daemonising
tools require a separate service integration.

```toml
version = "mas-agents/3"

[[agents]]
name = "policy_check"
operation = "check_policy/1"
input_schema = "schemas/input.json"
output_schema = "schemas/output.json"
environment = ["POLICY_TOKEN"]
working_directory = "."

[agents.implementation]
kind = "command"
executable = "tools/policy-check"
arguments = [
    { literal = "--format=json" },
    { literal = "--service" },
    { input = "/service" },
]
stdin = "none"
stdout = "json-object"
success_exit_codes = [0]
timeout_seconds = 30
max_output_bytes = 32768
```

An operation output schema is required for commands and LLMs. Input schemas are
optional; any declared input/output schema is enforced at the host boundary for
every implementation kind. Schema files must describe objects and use the host's
supported integer-JSON subset. They are embedded in the bundle.

## Arguments and I/O

Each of at most 64 arguments is an object with exactly one binding:

| Binding | Meaning |
| --- | --- |
| `literal` | Pass the configured string unchanged, including an empty string. |
| `input` | Select a string, exact integer or boolean by JSON Pointer. Missing, null and composite values fail before effect admission. |
| `file` | Retain a project-relative file and pass its immutable absolute path. Useful for interpreter scripts or configuration files. |
| `invocation_id = true` | Pass the admitted effect identity, for idempotency or receipts supported by the tool. |

Values become individual arguments without shell evaluation, splitting or
interpolation. Input validation must also respect the selected tool's option
semantics; direct execution does not turn a flag-shaped value into a positional
argument. Use fixed flags, an appropriate schema and `--` where the tool supports
it. Credentials belong in explicitly named environment variables, not argv.

`stdin` is `none` (the default, immediately closed) or `json` (the task object).
`stdout` is `json-object` (the default) or `text`, which produces
`{"stdout":"..."}`. JSON is parsed with MAS's exact codec: decimal numbers,
duplicate keys and invalid Unicode are rejected. Empty output is valid in text
mode. Stderr is drained separately and is not copied into results or journals.

Each stream is bounded by `max_output_bytes`, default 32768 and maximum 49152.
The final framed result must also fit the worker limit; escaping can expand text.
`timeout_seconds` is 1..3600, default 60, and never extends the run's original
deadline. `success_exit_codes` defaults to `[0]` and admits 1..32 distinct codes
in 0..255. Declare another successful code only when the tool's contract assigns
it a known completed outcome, such as a scanner reporting findings.

## Identity and environment

A project-relative executable is copied into the retained artefact store under
its configured basename, preserving filename-based dispatch. File arguments
likewise preserve their basename and suffix. Content digests isolate different
versions. Executables are owner-readable/executable and file arguments are
owner-readable; copied assets inherit no writable or set-id permission bits.
The published snapshot contains exactly the bytes that were hashed.
An absolute executable is digest-bound in place, preserving interpreter and
virtual-environment invocation paths. Every `file` argument is a project-relative
regular file retained by content digest. These digests are checked before tool
execution. Rebuilding changed tool bytes changes the bundle and cannot resume
an existing run; an old bundle can still use its retained project files.

`working_directory` is project-relative, defaults to `.`, and must identify an
existing directory inside the project. Its resolved absolute path is bound in
the bundle. Its contents are external state, not an automatic source snapshot:
use `file` bindings for resources that must remain immutable. Scripts importing
neighbouring modules need an explicit packaged executable or declared resources;
retaining one script does not capture its dependency graph.

The worker receives the fixed minimal PATH and the agent's named `environment`
references. Secret values are excluded from configuration and journals. The CLI
child receives no MAS channel descriptor or channel environment name. Shared
libraries, interpreters' package environments and remote account configuration
remain pinned deployment dependencies. Trusted application code retains the
operator's filesystem permissions; this is not a hostile-code sandbox.

## Effect outcomes and recovery

Every command invocation requests admission under the stable effect key
`command`. Completed effects reuse their recorded result. A command reading
changing external state is therefore replayed from the same recorded observation.

| Observation | Durable handling |
| --- | --- |
| Local launch fails before a process starts | Record `status: not_started`; fail the operation without retry. |
| Accepted exit code and valid output | Record `status: completed` and `output`; return the operation result. |
| Accepted exit code with invalid output | Record `status: invalid_output`; fail without repeating the tool. |
| Other exit code, signal, interrupted execution, stream limit or lost outcome | Keep the effect uncertain; block successful completion and automatic repetition. |

For an observed accepted exit code, the maintained adapter records
`execution: completed` with `execution_scope: command-process` in its evidence,
including when stdout is invalid. A failed launch records `execution: not_started`
with the same scope. These fields describe the local command process. They make
no assertion about completion of downstream operations performed by the tool.
Governed output withholding preserves that known process completion and returns
the unavailable marker through the [native completion contract](native-contract.md#external-effects-and-recovery).
Fresh delivery and cached replay fail with the same static result-unavailable
error and do not execute the command again.

The configured accepted exit codes are assertions about the tool's completion
contract. Process termination alone does not establish whether an external write
or remote action happened. Failure diagnostics record the observed exit status
when available, while the effect remains uncertain. Killing descendants does
not prove that a remote operation stopped.

For an uncertain command, verify external evidence against its admitted effect
ID. To resolve a successful outcome, supply this object as `resolve --output`:

```json
{"status":"completed","output":{"approved":true}}
```

Supply nonempty evidence separately. The wrapper is the effect's retained outcome;
the nested `output` is the public operation result. Resolution is an operator
assertion and does not extend the original deadline. The adapter checks the
resolved operation output against its schema before returning it.

An LLM can request this actor through its declared `delegates` and permitted
topology routes. Workflow actors can request it in any supported composition.
Neither use gives the tool authority to select a sender or bypass MAS admission.
