// One restricted Cursor SDK run per admitted host inference effect.
// No shell tools, ambient settings, MCP servers, subagents or hidden retries.
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const MAX_BYTES = 1_048_576;

function metadata(result, agent) {
  const evidence = { agent_id: agent.agentId, run_id: result.id, output_status: "invalid" };
  if (typeof result.requestId === "string") evidence.request_id = result.requestId;
  if (typeof result.model?.id === "string") evidence.model = result.model.id;
  if (result.usage && typeof result.usage === "object") evidence.usage = result.usage;
  return evidence;
}

/** The injectable SDK argument permits offline contract verification. */
export async function infer(request, sdk) {
  let workspace, agent, run, timer;
  let dispatched = false;
  let outcome = { version: "mas-provider/1", status: "not_sent", evidence: {} };
  try {
    const remaining = request.deadline_ms - Date.now();
    const keyName = request.api_key_env ?? "CURSOR_API_KEY";
    if (request.version !== "mas-provider/1" || !Number.isSafeInteger(remaining) || remaining <= 0 || remaining > 86_400_000 ||
        typeof request.model !== "string" || !request.model.trim() ||
        typeof request.instructions !== "string" || !request.output_schema || typeof request.output_schema !== "object" ||
        typeof keyName !== "string" || !/^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(keyName) || !process.env[keyName]) return outcome;
    // SDK loading and local setup occur before inference dispatch.
    sdk ??= await import("@cursor/sdk");
    workspace = await mkdtemp(join(tmpdir(), "mas-cursor-"));
    agent = await sdk.Agent.create({
      apiKey: process.env[keyName], model: { id: request.model },
      tools: [], disallowedTools: ["mcp", "task"], mcpServers: {}, agents: {},
      local: { cwd: workspace, dirs: [], settingSources: [], customTools: {}, enableAgentRetries: false },
    });
    // Do not reserialise domain integers through JavaScript's Number type.
    if (typeof request.input_json !== "string" || typeof request.output_schema_json !== "string") return outcome;
    const prompt = `${request.instructions}\n\nReturn only one JSON value matching this JSON Schema. Do not use Markdown.\n${request.output_schema_json}\n\nInput:\n${request.input_json}`;
    dispatched = true;
    run = await agent.send(prompt);
    // Leave a small cleanup window before the independently enforced parent
    // deadline. A timed out remote inference remains an unknown effect.
    const deadline = new Promise(resolve => { timer = setTimeout(() => resolve(null), Math.max(1, request.deadline_ms - Date.now() - 250)); });
    const result = await Promise.race([run.wait(), deadline]);
    clearTimeout(timer);
    if (result === null) {
      outcome = { version: "mas-provider/1", status: "unknown", evidence: { output_status: "unavailable" } };
    } else {
      const evidence = metadata(result, agent);
      outcome = { version: "mas-provider/1", status: "unusable", evidence };
      if (result.status !== "finished") {
        // An SDK transport error or cancellation does not prove the provider
        // stopped inference or performed no billable work.
        outcome.status = "unknown";
        evidence.output_status = "unavailable";
      } else if (typeof result.result !== "string" || Buffer.byteLength(result.result) > MAX_BYTES / 2) {
        evidence.output_status = "invalid";
      } else {
        // Rust's strict MAS JSON parser detects duplicates, decimal syntax and
        // arbitrary integers. JSON.parse here would erase that evidence.
        outcome.output_json = result.result;
        outcome.status = "completed";
        evidence.output_status = "unvalidated_json";
      }
    }
  } catch {
    // SDK error messages may include prompt fragments, headers or credentials.
    outcome = { version: "mas-provider/1", status: dispatched ? "unknown" : "not_sent", evidence: { output_status: "unavailable" } };
  } finally {
    clearTimeout(timer);
    let clean = true;
    try { if (run && outcome.status === "unknown") await run.cancel(); } catch { clean = false; }
    try { if (agent) await agent[Symbol.asyncDispose](); } catch { clean = false; }
    try { if (workspace) await rm(workspace, { recursive: true, force: true }); } catch { clean = false; }
    outcome.evidence.cleanup = clean ? "confirmed" : "incomplete";
  }
  return outcome;
}

async function main() {
  let bytes = 0;
  const chunks = [];
  try {
    for await (const chunk of process.stdin) {
      bytes += chunk.length;
      if (bytes > MAX_BYTES) throw new Error("request limit");
      chunks.push(chunk);
    }
    const request = JSON.parse(Buffer.concat(chunks).toString("utf8"));
    const outcome = await infer(request);
    process.stdout.write(JSON.stringify(outcome));
  } catch {
    process.stdout.write(JSON.stringify({ version: "mas-provider/1", status: "not_sent", evidence: {} }));
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
