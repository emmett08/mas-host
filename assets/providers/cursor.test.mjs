import test from "node:test";
import assert from "node:assert/strict";
import { access } from "node:fs/promises";
import { infer } from "./cursor.mjs";

const keyName = "MAS_CURSOR_CONTRACT_TEST_KEY";
function request() { return { version: "mas-provider/1", model: "test-model", instructions: "Review", input: { n: 3 }, input_json:'{"n":3}', output_schema: { type: "object" }, output_schema_json:'{"type":"object"}', deadline_ms: Date.now() + 5000, api_key_env: keyName }; }
function fakeSdk(result, options = {}) {
  const state = { creates: 0, sends: 0, closes: 0, cancels: 0 };
  const sdk = { Agent: { async create(config) {
    state.creates++; state.config = config;
    return { agentId: "agent-test", async send(prompt) {
      state.sends++; state.prompt = prompt;
      if (options.sendError) throw new Error("secret key included in original error");
      return { async wait() { return result; }, async cancel() { state.cancels++; } };
    }, async [Symbol.asyncDispose]() { state.closes++; if (options.closeError) throw new Error("secret cleanup details"); } };
  } } };
  return { sdk, state };
}

test("Cursor uses the documented text-only SDK contract in a clean temporary workspace", async () => {
  process.env[keyName] = "fake-key";
  const { sdk, state } = fakeSdk({ id: "run-1", requestId: "request-1", status: "finished", result: '{"ok":true}', model: { id: "resolved-model" }, usage: { inputTokens: 9, outputTokens: 4 } });
  const outcome = await infer(request(), sdk);
  assert.equal(outcome.status, "completed"); assert.equal(outcome.output_json, '{"ok":true}');
  assert.equal(outcome.evidence.model, "resolved-model"); assert.equal(outcome.evidence.cleanup, "confirmed");
  assert.equal(state.sends, 1); assert.equal(state.closes, 1);
  assert.deepEqual(state.config.tools, []); assert.deepEqual(state.config.disallowedTools, ["mcp", "task"]);
  assert.deepEqual(state.config.local.settingSources, []); assert.equal(state.config.local.enableAgentRetries, false);
  assert.deepEqual(state.config.mcpServers, {}); assert.deepEqual(state.config.agents, {});
  assert.ok(state.config.local.cwd.includes("mas-cursor-"));
  await assert.rejects(access(state.config.local.cwd));
  delete process.env[keyName];
});
test("Missing credentials fail before SDK acquisition", async () => {
  delete process.env[keyName]; const { sdk, state } = fakeSdk({});
  assert.equal((await infer(request(), sdk)).status, "not_sent"); assert.equal(state.creates, 0);
});
test("A failed send remains unknown and does not reveal or retry errors", async () => {
  process.env[keyName] = "fake-key";
  const { sdk, state } = fakeSdk({}, { sendError: true });
  const outcome = await infer(request(), sdk);
  assert.equal(outcome.status, "unknown"); assert.equal(state.sends, 1); assert.equal(state.closes, 1);
  assert.ok(!JSON.stringify(outcome).includes("secret"));
  delete process.env[keyName];
});
test("Raw output and failed disposal retain separate evidence for Rust validation", async () => {
  process.env[keyName] = "fake-key";
  const { sdk } = fakeSdk({ status: "finished", result: "not JSON" }, { closeError: true });
  const outcome = await infer(request(), sdk);
  assert.equal(outcome.status, "completed"); assert.equal(outcome.evidence.cleanup, "incomplete");
  assert.equal(outcome.output_json, "not JSON"); delete process.env[keyName];
});
test("Application integers and raw output reach Rust without JavaScript rounding", async () => {
  process.env[keyName] = "fake-key";
  const raw = '{"n":340282366920938463463374607431768211457}';
  const { sdk, state } = fakeSdk({ status: "finished", result: raw });
  const input = request(); input.input_json = raw;
  const outcome = await infer(input, sdk);
  assert.equal(outcome.output_json, raw); assert.ok(state.prompt.includes(raw));
  delete process.env[keyName];
});
