# Cursor provider helper

Install the pinned SDK with Node.js 22.13 or later:

```sh
npm --prefix /path/to/mas-host/assets/providers ci
```

Configure Cursor with `{"kind":"cursor","command":["node","@sdk/assets/providers
/cursor.mjs"],"api_key_env":"CURSOR_API_KEY"}`. The build resolves `@sdk/` against the
host source directory. The helper maps one admitted inference effect to one SDK run: it
reads a `mas-provider/1` request from stdin and returns one outcome on stdout. Its
supervised worker receives only the configured credential reference and explicit
connection environment names.

Each call creates an empty temporary workspace. The helper disables built-in tools, MCP
servers, subagents, custom tools and ambient settings, keeping external tool effects on
the host's declared delegate path. It also disables agent transport/stall retries. The
model receives the instructions, input and output schema in one prompt, then the host
validates the returned JSON locally.

The helper cancels an unfinished run, disposes its SDK agent and removes the workspace.
The host guardian independently enforces the deadline and descendant cleanup.

The adapter enforces time and returned-byte limits. An output-token cap remains
unavailable in the documented SDK API, so evidence records `token_limit_enforced:false`.
An organisation's Cursor account controls billing limits. An unknown inference outcome
requires reconciliation under the original effect identity.

Run offline contract tests with `npm --prefix /path/to/mas-host/assets/providers test`.
They inject an SDK double and run without an API key or SDK installation. Live testing
is a separate action requiring an explicit account key and model selection.

Implementation references (checked 15 October 2026): [Cursor TypeScript
SDK](https://cursor.com/docs/sdk/typescript), especially **Restricting the toolset**,
**LocalAgentOptions**, **Run**, and **Resource management**. Dependency `@cursor/sdk` is
pinned to `1.0.37`. `package-lock.json` fixes its transitive dependencies.
