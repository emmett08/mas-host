# Cursor provider helper

Install the pinned SDK with Node.js 22.13 or later:

```sh
npm --prefix /path/to/mas-host/assets/providers ci
```

A Cursor connection uses `{"kind":"cursor","command":["node","@sdk/assets/providers/cursor.mjs"],"api_key_env":"CURSOR_API_KEY"}`. The build resolves `@sdk/` against the host source directory. The host passes only the configured credential environment and explicit connection environment names to its supervised worker. The helper reads a single `mas-provider/1` request from stdin and returns one outcome object on stdout.

Every call creates an empty temporary workspace. The official SDK runs with no built-in tools, MCP servers, subagents, custom tools or ambient settings, and with agent transport/stall retries disabled. The model receives the agent instructions, input and output schema as one prompt. The host validates the resulting JSON locally. The helper cancels an unfinished run, disposes its SDK agent and removes its workspace; the host guardian independently enforces the deadline and descendant cleanup.

Cursor is an agent SDK. Its documented API does not expose an output-token cap; `max_output_tokens` cannot be enforced by this adapter. Evidence therefore records `token_limit_enforced:false`. Time and returned byte limits apply, while an organisation's Cursor account controls billing limits. An unknown inference outcome must be reconciled rather than automatically retried.

Offline contract tests run with `npm --prefix /path/to/mas-host/assets/providers test` and inject an SDK double; they do not require an API key or install the SDK. A live test requires an explicit account key and model selection. No live call is made by the normal test suite.

Implementation references (checked 15 October 2026): [Cursor TypeScript SDK](https://cursor.com/docs/sdk/typescript), especially **Restricting the toolset**, **LocalAgentOptions**, **Run**, and **Resource management**. Dependency `@cursor/sdk` is pinned to `1.0.37`; `package-lock.json` fixes its transitive dependencies.
