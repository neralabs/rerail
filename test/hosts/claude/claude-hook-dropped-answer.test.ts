import { createHash, randomUUID } from "node:crypto";
import { mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { describe, expect, it } from "vitest";
import { languageFor, renderSteer, type CodeRuleViolation, type RerailEventLoop } from "../../../src/engine/index.js";
import { handleClaudeEvent } from "../../../src/hosts/claude/claude-hook-adapter.js";
import { bindSteerReceipt, type HookInput } from "../../../src/harness/index.js";
import { ingestEngineEvent, startEngineService } from "../../../src/runtime/index.js";
import { serveText, testEmbedder } from "../../support/hook-events.js";
import { startTestRepository } from "../../support/started-repository.js";

const BAD = "export const a = 1;\nconst bad = 2;\n";

function finding(path: string): CodeRuleViolation {
  return {
    id: `no-bad:${path}`, ruleId: "test:no-bad", clauseId: "clause", code: "NO_BAD", mode: "steer", message: "Do not write bad.",
    path, span: { startByte: 26, endByte: 29, startLine: 1, startColumn: 6, endLine: 1, endColumn: 9 }, captures: {}, variables: {},
  };
}

const STEER = renderSteer([finding("a.ts")]);

/**
 * A Claude Code session on a real engine service, whose edit checks finish only once the test opens `check`, each
 * steering its file's finding when the edit wrote `bad`. `hook` runs one hook as the hook process does (stdout and
 * stderr, with each steer receipt's rule ids in `receipts`), through the engine service unless another event loop is
 * given. `waitingForChecks` settles once the daemon next waits for the session's edit checks.
 */
async function engineSession() {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-dropped-answer-")));
  const databasePath = join(root, "rerail.sqlite");
  startTestRepository(root, databasePath);
  await writeFile(join(root, "a.ts"), "export const a = 1;\n");
  const check = Promise.withResolvers<void>();
  const checks: Promise<void>[] = [];
  let nextWait = Promise.withResolvers<void>();
  const waitingForChecks = (): Promise<void> => {
    nextWait = Promise.withResolvers<void>();
    return nextWait.promise;
  };
  // Windows serves the engine on a named pipe; a socket file there fails to listen.
  const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-test-${randomUUID()}` : join(root, "engine.sock");
  const service = await startEngineService({
    socketPath,
    storageRoot: join(root, "sessions"),
    databasePath,
    embedder: testEmbedder,
    createAuditor: ({ steerDelivery }) => ({
      openWorkspace: async () => undefined,
      // As rules without a text clause: the session's own files under `sessions/` are no sources.
      acceptsPath: path => languageFor(path) !== undefined,
      auditGeneratedEdits: async (sessionId, batch) => {
        const violations = batch.edits.filter(edit => edit.generatedText?.includes("bad")).map(edit => finding(edit.path));
        checks.push((async () => {
          await check.promise;
          if (violations.length === 0) return;
          await steerDelivery.requestSteer(sessionId, "steer", renderSteer(violations), {
            revision: batch.revision, scope: batch.edits.map(edit => edit.path), ruleIds: ["test:no-bad"],
            rules: [{ id: "test:no-bad", state: "steered" }], background: true,
            generatedEdit: { violations, sourceOf: () => undefined },
          });
        })());
        return { revision: batch.revision, disposition: "continue", violations: [], failures: [] };
      },
      settleGeneratedEdits: async () => {
        nextWait.resolve();
        await Promise.all(checks);
      },
      close: async () => undefined,
    }),
  });
  const engine: RerailEventLoop = { ingest: event => ingestEngineEvent({ socketPath }, event) };
  const receipts: Array<readonly string[]> = [];
  const common = { session_id: "claude-session", cwd: root };
  const hook = async (input: Record<string, unknown>, loop: RerailEventLoop = engine) => {
    const served = await serveText(
      (parsed: HookInput) => handleClaudeEvent(parsed, loop, join(root, "sessions"), { NO_COLOR: "1" })
        .then(output => bindSteerReceipt(output, (_receipt, ruleIds) => { receipts.push(ruleIds); })),
      JSON.stringify({ ...common, ...input }),
    );
    return { output: JSON.parse(served.stdout) as unknown, stderr: served.stderr };
  };
  /** Writes `bad` into `a.ts` with the Write tool; its check waits for `check`, so the tool result answers without it. */
  const badEdit = async () => {
    await hook({ hook_event_name: "SessionStart", source: "startup" });
    const input = { tool_name: "Write", tool_use_id: "tool-1", tool_input: { file_path: join(root, "a.ts"), content: BAD } };
    await hook({ hook_event_name: "PreToolUse", ...input });
    await writeFile(join(root, "a.ts"), BAD);
    expect((await hook({ hook_event_name: "PostToolUse", ...input, tool_response: { type: "update" } })).output).toEqual({});
  };
  /** The rule ids of each steer the session's ledger records as given to the agent, in order. */
  const recordedSteers = async () => {
    const ledger = await readFile(join(root, "sessions", createHash("sha256").update("claude-session").digest("hex"), "events.jsonl"), "utf8");
    return ledger.split("\n").filter(line => line !== "")
      .map(line => JSON.parse(line) as { type: string; delivery?: { ruleIds: string[] } })
      .flatMap(entry => (entry.type === "delivery" && entry.delivery !== undefined ? [entry.delivery.ruleIds] : []));
  };
  const close = async () => {
    check.resolve();
    await service.close();
    await rm(root, { recursive: true, force: true, maxRetries: 5 });
  };
  return { socketPath, check, waitingForChecks, receipts, recordedSteers, hook, badEdit, close };
}

/**
 * An event loop that sends its event to the engine service as a hook does and gives up on it once `waiting` settles,
 * as a hook does at its request deadline: the connection closes before the daemon answers.
 */
function givingUp(socketPath: string, waiting: Promise<void>): RerailEventLoop {
  return {
    ingest: event => new Promise((_, reject) => {
      const body = JSON.stringify({ event });
      const outgoing = request({
        socketPath, method: "POST", path: "/events", agent: false,
        headers: { "content-type": "application/json", "content-length": Buffer.byteLength(body) },
      });
      outgoing.on("error", reject);
      outgoing.end(body);
      void waiting.then(() => outgoing.destroy(new Error("Kernel request timed out")));
    }),
  };
}

const READ = { hook_event_name: "PreToolUse", tool_name: "Read", tool_input: {} };
const STEERED = {
  systemMessage: "◆ R E R A I L · test:no-bad  steered",
  hookSpecificOutput: { hookEventName: "PreToolUse", additionalContext: STEER },
};

describe("Claude Code hook whose answer from the daemon never arrived", () => {
  it("gives the steer that answer carried to the agent at the session's next hook, and at no hook after it", { timeout: 20_000 }, async () => {
    const session = await engineSession();
    try {
      await session.badEdit();
      // The turn end waits for the edit's check, and its hook gives up first, as one does at its deadline under load.
      const stop = session.hook(
        { hook_event_name: "Stop", prompt_id: "p1", stop_hook_active: false },
        givingUp(session.socketPath, session.waitingForChecks()),
      );
      expect(await stop).toEqual({ output: {}, stderr: "rerail hook: Kernel request timed out\n" });
      // Time for the daemon to see the connection close; then the check steers, and the turn end answers no one.
      await delay(200);
      session.check.resolve();

      expect((await session.hook({ ...READ, tool_use_id: "tool-2" })).output).toEqual(STEERED);
      expect(session.receipts).toEqual([["test:no-bad"]]);
      expect((await session.hook({ ...READ, tool_use_id: "tool-3" })).output).toEqual({});
      expect(session.receipts).toEqual([["test:no-bad"]]);
      // Recorded once, when it went out, not also for the answer nobody read.
      expect(await session.recordedSteers()).toEqual([["test:no-bad"]]);
    } finally {
      await session.close();
    }
  });

  it("never offers again a steer whose answer reached the hook", { timeout: 20_000 }, async () => {
    const session = await engineSession();
    try {
      await session.badEdit();
      session.check.resolve();
      expect((await session.hook({ hook_event_name: "Stop", prompt_id: "p1", stop_hook_active: false })).output).toEqual({
        ...STEERED, hookSpecificOutput: { hookEventName: "Stop", additionalContext: STEER },
      });
      expect(session.receipts).toEqual([["test:no-bad"]]);

      expect((await session.hook({ ...READ, tool_use_id: "tool-2" })).output).toEqual({});
      expect((await session.hook({ ...READ, tool_use_id: "tool-3" })).output).toEqual({});
      expect(session.receipts).toEqual([["test:no-bad"]]);
      expect(await session.recordedSteers()).toEqual([["test:no-bad"]]);
    } finally {
      await session.close();
    }
  });
});
