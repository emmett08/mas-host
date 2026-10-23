# Authoring configuration and inputs

Choose JSON, TOML or restricted YAML for agent definitions, provider profiles, topology,
workflow plans and invocation inputs. All three formats express the same versioned
documents and pass through the same semantic validators. TOML suits settings, while YAML
makes nested workflows easier to inspect. JSON serves generated documents, schemas,
stdin and interchange.

Agent documents use `mas-agents/3` with nested implementation bindings. The build emits
`mas-bundle/3`, including an explicit public operation for every worker. Choose the
contract version explicitly in the document, independently of its file format.

## Discovery and explicit selection

The CLI discovers `agents` and `topology` with `.json`, `.toml`, `.yaml` or `.yml`
extensions in the project directory. Exactly one candidate for each document must exist.
Multiple candidates produce an error, requiring the author to resolve the ambiguity
before a potentially stale definition can be used. `--agents FILE` and `--topology FILE`
select explicit files. `--connections FILE` selects an optional provider profile. These
paths are relative to the project unless absolute.

```sh
mas-host run --project . --agent planner --input input.yaml \
  --state .state --run review-001

mas-host build --project . --agents config/agents.toml \
  --topology config/routes.yaml --connections profiles/openai.toml

printf '%s\n' '{"units":12}' | mas-host run --project . --agent planner \
  --input - --state .state --run review-002
```

File input uses its extension. `--input-format json|toml|yaml` explicitly selects
the format, including for stdin. Stdin defaults to JSON. Invocation input must
resolve to an object. Paths referenced inside agent definitions, including
source, prompt, schema and workflow files, remain relative to the project root.

## One meaning across formats

Readers convert authoring syntax into the MAS value domain: objects with unique string
keys, arrays, strings, exact integers, booleans and null. The protocol codec checks the
resulting representation before configuration validation or execution. Documents remain
bounded to 65536 bytes and the protocol's nesting and integer limits. TOML has no null
literal. Where a schema distinguishes omission from explicit null, preserve that
distinction when choosing the authoring format. The assembled bundle has the same total
limit, including embedded files and delegate contracts. The builder validates its
compact JSON representation before publishing it and preserves the previous published
bundle if the new one is too large or structurally invalid.

Floating-point values and native TOML date/time values are rejected. Use a
schema-defined string or scaled integer for quantities requiring those types. YAML
permits one document, string keys and ordinary scalars and collections. Duplicate keys,
explicit tags, anchors, aliases and merge keys are rejected. Quote a scalar when its
intended type is a string. Use sequences to order workflow steps. Object keys identify
fields independently of their order. Plain `true`, `True`, `TRUE` and the corresponding
false spellings are booleans. `null`, `Null`, `NULL`, `~` and an empty scalar are null.
`yes`, `no` and ISO date spellings remain strings under this YAML profile.

```yaml
type: sequence
steps:
  - type: parallel
    steps:
      - type: request
        key: capacity
        target: capacity
        input: {$input: ""}
      - type: request
        key: security
        target: security
        input: {$input: ""}
  - type: request
    key: acceptance
    target: acceptance
    input:
      reviews: {$previous: /results}
```

The workflow interpreter and MAS route checks are independent of source format.
Nested groups, branches, bounded loops and subworkflows compose as described in
the [workflow contract](../crates/mas-host-workflow/README.md).

## Native Python APIs

The [Python SDK](../sdk/python/README.md) adapts dataclasses and dictionaries to the
same object contract. Its client supplies input through stdin, so an application can
invoke a host using its own types without writing input files. Explicit Python
configuration builders export documents for the CLI. Inspection and recovery consume
those documents through the host's central validators, keeping Python configuration
execution an explicit application action.

## Identity and recovery

The build embeds resolved prompt, schema and workflow contents in the immutable
bundle. It hashes the execution representation and retained worker artefacts.
Equivalent authoring documents resolve to the same bundle at the same project
location, using the same host and build environment. Comments and formatting in
configuration files therefore do not change execution identity. Changes to
model settings, prompt text, schemas, workflow semantics or executable source
do. Array order remains significant.

Python application modules and SDK code are frozen into generated executable archives.
Existing archives remain runnable after application source changes. Rebuilding from
changed sources creates a different archive identity. Pin the interpreter and installed
third-party dependencies for retained runs. The archive freezes source bytes.
Reproducing its runtime also requires the separately pinned interpreter and installed
dependencies.

Authoring readers and language adapters terminate at the same validation
boundary. Runtime messages, generated bundles and journals retain their
strict JSON formats and contract versions.
