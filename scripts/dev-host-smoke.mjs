import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
import { resolve } from "node:path";
import { z } from "zod";

// This driver exercises the installed bridge's real extension factories against
// the real container. The minimal harness port supplies registrations/session entries;
// native kernels, SQLite, policy execution and semantic operations are not mocked.
const [bridge, host] = process.argv.slice(2);
assert.ok(host === "omp" || host === "pi");
const extension = await import(pathToFileURL(resolve(bridge, "dist", host === "omp" ? "default-extension.js" : "pi-extension.js")).href);
const handlers = new Map();
const tools = new Map();
const commands = new Map();
let entries = [
  { type: "message", id: "user-message", timestamp: "2026-01-01T00:00:00Z", message: { role: "user", content: "Build an isolated development container." } },
  { type: "message", id: "assistant-message", timestamp: "2026-01-01T00:00:01Z", message: { role: "assistant", content: "Use a separate container for each worktree." } },
];
const deliveries = [];
const api = {
  zod: z, setLabel() {},
  on: (name, handler) => handlers.set(name, handler),
  registerTool: tool => tools.set(tool.name, tool),
  registerCommand: (name, command) => commands.set(name, command),
  appendEntry: (customType, data) => entries.push({ type: "custom", id: `custom-${entries.length}`, timestamp: "2026-01-01T00:00:02Z", customType, data }),
  sendMessage: message => deliveries.push(message),
};
await extension.default(api);
const sessionId = `native-${host}`;
const contextFor = branch => ({ cwd: process.cwd(), sessionManager: { getSessionId: () => sessionId, getBranch: () => branch }, ui: { notify() {} } });
let context = contextFor(entries);
const invoke = async (name, input) => {
  assert.ok(tools.has(name), `${host} did not register ${name}`);
  const response = await tools.get(name).execute("smoke-call", input);
  return response.details;
};
await handlers.get("session_start")({}, context);
try {
  const rule = {
    id: `native-${host}-rule`, reference: "Build an isolated development container.",
    match: { operator: "gte", threshold: 0.99 },
    sample: { source: "user-input", boundary: "message" },
    action: { delivery: "nextTurn", content: `Only native ${host}` }, repeat: { mode: "once" },
  };
  await invoke("cosine_rules", { action: "add", rule });
  const listed = await invoke("cosine_rules", { action: "list" });
  assert.ok(listed.rules.some(entry => entry.rule.id === rule.id));
  const tested = await invoke("cosine_rules", { action: "test", sample: { source: "user-input", boundary: "message", text: rule.reference } });
  assert.ok(tested.matches.some(match => match.ruleId === rule.id));
  // Reload replaces the native semantic session, so create cache-backed IDs only after it completes.
  await commands.get("cosine-rules").handler("reload", context);
  const selected = await invoke("rerail_select", { spec: { order: "source" } });
  assert.deepEqual(selected.items.map(item => item.ref.id), ["user-message", "assistant-message"]);
  const projection = await invoke("rerail_project", { selectionId: selected.selectionId, spec: { unit: { kind: "item" }, render: { kind: "text-only" } } });
  assert.deepEqual(projection.vectors.map(vector => vector.renderedText), entries.slice(0, 2).map(entry => entry.message.content));
  const relations = await invoke("rerail_relate", { leftProjectionId: projection.projectionId, rightProjectionId: projection.projectionId, spec: { pairing: { kind: "all-pairs" } } });
  assert.ok(relations.pairs.some(relation => relation.similarity > 0.999));
  await invoke("rerail_mark", { change: { kind: "attach", targets: [selected.items[0].ref], watermarks: [{ namespace: "smoke", key: "selected", owner: "smoke", visibility: "private", createdAt: 1 }] } });
  const marked = await invoke("rerail_select", { spec: { where: { mark: { namespace: "smoke", key: "selected" } } } });
  assert.deepEqual(marked.items.map(item => item.ref.id), ["user-message"]);
  entries = [...entries, { type: "message", id: "branch-message", timestamp: "2026-01-01T00:00:03Z", message: { role: "user", content: "Use this branch's native session context." } }];
  context = contextFor(entries);
  await handlers.get("session_tree")({}, context);
  const retainedProjection = await invoke("rerail_project", { selectionId: selected.selectionId, spec: { unit: { kind: "item" }, render: { kind: "text-only" } } });
  assert.deepEqual(retainedProjection.vectors.map(vector => vector.renderedText), projection.vectors.map(vector => vector.renderedText));
  const branchSelection = await invoke("rerail_select", { spec: { order: "source" } });
  assert.deepEqual(branchSelection.items.map(item => item.ref.id), ["user-message", "assistant-message", "branch-message"]);
  const prompt = { prompt: rule.reference, text: rule.reference };
  await handlers.get(host === "omp" ? "before_agent_start" : "input")(prompt, context);
  assert.ok(deliveries.some(message => JSON.stringify(message).includes(`Only native ${host}`)));
  assert.ok(entries.some(entry => entry.customType === "rerail.mutation"));
  await invoke("cosine_rules", { action: "remove", ids: [rule.id] });
  const removed = await invoke("cosine_rules", { action: "list" });
  assert.ok(!removed.rules.some(entry => entry.rule.id === rule.id));
  console.log(`PASS ${host}: installed bridge, events, cosine add/list/test/remove, select/project/relate/mark; host native addon unavailable`);
} finally {
  await handlers.get("session_shutdown")({}, context);
}
