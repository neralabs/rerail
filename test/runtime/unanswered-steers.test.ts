import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AdapterEvent } from "../../src/engine/index.js";
import { RerailRuntime } from "../../src/runtime/runtime.js";
import { setUpTestRepository } from "../support/repository-rules.js";

const directories: string[] = [];
afterEach(async () => {
  vi.restoreAllMocks();
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

const FORCE_PUSH = {
  id: "acme:no-force-push",
  mode: "steer" as const,
  match: { field: "input.command" as const, regex: String.raw`\bgit\s+push\b[^\n]*--force\b` },
  sample: { source: "tool:Bash" as const, boundary: "tool-call" as const },
  action: { content: "Ask before force-pushing." },
  repeat: { mode: "always" as const },
};

const FORCE_PUSH_STEER = expect.objectContaining({
  requested: "steer",
  content: expect.stringContaining("- Ask before force-pushing. [acme:no-force-push]"),
  ruleIds: ["acme:no-force-push"],
});

/** The rule ids of each steer the session's ledger records as given to the agent, in order. */
async function recordedSteers(root: string, sessionId: string): Promise<string[][]> {
  const ledger = await readFile(join(root, "sessions", createHash("sha256").update(sessionId).digest("hex"), "events.jsonl"), "utf8");
  return ledger.split("\n").filter(line => line !== "")
    .map(line => JSON.parse(line) as { type: string; delivery?: { ruleIds: string[] } })
    .flatMap(entry => (entry.type === "delivery" && entry.delivery !== undefined ? [entry.delivery.ruleIds] : []));
}

/**
 * A started Claude Code session `s` whose evaluator never opens, so its code rules are off and only its conversation
 * rule steers: a forced `git push`. `call` makes a tool call event of it.
 */
async function conversationRuleSession() {
  const root = await mkdtemp(join(tmpdir(), "rerail-unanswered-"));
  directories.push(root);
  const databasePath = join(root, "rerail.sqlite");
  setUpTestRepository(root, { library: [{ kind: "conversation", rule: FORCE_PUSH }] }, databasePath);
  vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  const runtime = new RerailRuntime({
    autostart: false,
    workspaceRoot: root,
    storageRoot: join(root, "sessions"),
    modelPath: resolve("model"),
    databasePath,
    createAuditor: () => ({
      openWorkspace: async () => { throw new Error("no evaluator in this test"); },
      acceptsPath: () => false,
      auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
      close: async () => undefined,
    }),
  });
  await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "s", cwd: root } });
  const call = (id: string, toolName: string, input: unknown): AdapterEvent => ({
    kind: "call", eventId: id, sessionId: "s", toolCallId: id, toolName, input,
  });
  const read = (id: string): AdapterEvent => call(id, "Read", { file_path: join(root, "a.ts") });
  return { root, runtime, push: call("push", "Bash", { command: "git push --force origin main" }), read };
}

describe("an answer its host stopped waiting for", () => {
  it("gives a conversation rule's steer it held to the session's next answer, once, in a session without code rules", async () => {
    const { root, runtime, push, read } = await conversationRuleSession();
    try {
      // The hook gave up while its event still waited in the session's lane, as every hook did on a loaded laptop.
      expect(await runtime.ingest(push, { signal: AbortSignal.abort() })).toEqual([]);

      expect(await runtime.ingest(read("read-1"))).toEqual([FORCE_PUSH_STEER]);
      expect(await runtime.ingest(read("read-2"))).toEqual([]);
      expect(await recordedSteers(root, "s")).toEqual([["acme:no-force-push"]]);
    } finally {
      await runtime.dispose();
    }
  });

  it("gives the steers of an answer back to the session's next answer, once, when its host stops waiting after the answer is ready", async () => {
    const { root, runtime, push, read } = await conversationRuleSession();
    try {
      const unanswered = new AbortController();
      expect(await runtime.ingest(push, { signal: unanswered.signal })).toEqual([FORCE_PUSH_STEER]);
      // The hook gave up just as the answer was ready, so the engine service could not write it.
      unanswered.abort();

      expect(await runtime.ingest(read("read-1"))).toEqual([FORCE_PUSH_STEER]);
      expect(await runtime.ingest(read("read-2"))).toEqual([]);
      expect(await recordedSteers(root, "s")).toEqual([["acme:no-force-push"]]);
    } finally {
      await runtime.dispose();
    }
  });
});
