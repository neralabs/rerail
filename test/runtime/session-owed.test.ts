import { createHash } from "node:crypto";
import { mkdtemp, readFile, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderSteer, type CodeRuleViolation, type RuleSteerDelivery } from "../../src/engine/index.js";
import { RerailRuntime, type SessionSuspension, type SuspendedSessions } from "../../src/runtime/runtime.js";
import { removeFolder } from "../support/platform/remove-folder.js";

/**
 * The suspended sessions an engine service keeps for its runtimes (`suspendedSessions` in engine-service.ts): the idle
 * close keeps a quiet session's owner claim there, and the session's next open in a runtime takes it back.
 */
function keptSuspensions(): SuspendedSessions {
  const kept = new Map<string, SessionSuspension>();
  return {
    keep: (suspension) => {
      kept.set(suspension.descriptor.sessionId, suspension);
    },
    get: sessionId => kept.get(sessionId),
    take: sessionId => {
      const suspension = kept.get(sessionId);
      kept.delete(sessionId);
      return suspension;
    },
  };
}

const directories: string[] = [];
afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(directories.splice(0).map(async path => await removeFolder(path)));
});

function finding(path: string): CodeRuleViolation {
  return {
    id: `no-bad:${path}`, ruleId: "test:no-bad", clauseId: "clause", code: "NO_BAD", mode: "steer", message: "Do not write bad.",
    path, span: { startByte: 26, endByte: 29, startLine: 1, startColumn: 6, endLine: 1, endColumn: 9 }, captures: {}, variables: {},
  };
}
const STEER = renderSteer([finding("a.ts")]);
const DIGEST = renderSteer([finding("a.ts")], { kind: "late" });
// Assembled from parts, so no secret scanner takes this file for one holding a key.
const SECRET = ["glpat", "-x7Kq2mZ9vL4pR8tN1wB6"].join("");
const REDACTED = `[redacted:gitlab-token:${createHash("sha256").update(SECRET).digest("hex").slice(0, 6)}]`;

/** What `a.ts` holds once an edit writes `bad`, which its rule steers on. */
const BAD = "export const a = 1;\nconst bad = 2;\n";

/**
 * A Claude Code session in a checkout holding `a.ts`, whose rule check steers on an edit that writes `bad` into it, and
 * when the test asks (`steer`, `hold`), as a check that finishes after the boundary that waited for it does. `reopen`
 * makes the runtime a daemon would make after a restart, over the same records. An evaluator opened after the first
 * opens once `reopened` settles, as one whose rules compile again does.
 */
async function owingSession(options: { reopened?: Promise<void> } = {}) {
  // The real path: macOS's temporary folder is reached through a symlink, which would name the file twice.
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-session-owed-")));
  directories.push(root);
  await writeFile(join(root, "a.ts"), "export const a = 1;\n");
  let delivery: RuleSteerDelivery | undefined;
  const checked: string[] = [];
  /** Each edit a rule check read, with the mode it read it in: Log for another writer's change. */
  const reads: { path: string; mode: string; bad: boolean }[] = [];
  let evaluators = 0;
  // One for the session's whole life, as the engine service keeps one across the runtimes it makes.
  const suspendedSessions = keptSuspensions();
  const open = () => new RerailRuntime({
    autostart: false,
    suspendedSessions,
    workspaceRoot: root,
    storageRoot: join(root, "sessions"),
    databasePath: join(root, "rerail.sqlite"),
    embedder: {
      fingerprint: () => ({ modelId: "test", revision: "1", dimensions: 2, weightsSha256: "test", tokenizerSha256: "test", kernelFormatVersion: 1 }),
      embed: async texts => texts.map(() => new Float32Array([1, 0])),
    },
    createAuditor: ({ steerDelivery }) => {
      delivery = steerDelivery;
      evaluators += 1;
      const opening = evaluators > 1 ? options.reopened : undefined;
      return {
        openWorkspace: async () => await opening,
        acceptsPath: path => path.endsWith(".ts"),
        auditGeneratedEdits: async (sessionId, batch) => {
          const bad = batch.edits.filter(edit => edit.generatedText?.includes("bad")).map(edit => edit.path);
          if (bad.length > 0) {
            await steerDelivery.requestSteer(sessionId, "steer", STEER, {
              revision: batch.revision, scope: bad, ruleIds: ["test:no-bad"], rules: [{ id: "test:no-bad", state: "steered" }],
            });
          }
          // Once the steer is placed, so a test that waits for the check finds it pending.
          checked.push(...batch.edits.map(edit => edit.path));
          reads.push(...batch.edits.map(edit => ({ path: edit.path, mode: batch.mode ?? "session", bad: edit.generatedText?.includes("bad") === true })));
          return { revision: batch.revision, disposition: "continue", violations: [], failures: [] };
        },
        close: async () => undefined,
      };
    },
  });
  const session = { host: "claude-code" as const, sessionId: "owing", cwd: root };
  let runtime = open();
  await runtime.ingest({ kind: "session-start", eventId: "start", session });
  await runtime.codeRulesOpened(session.sessionId);
  const ledger = join(root, "sessions", createHash("sha256").update(session.sessionId).digest("hex"), "events.jsonl");
  return {
    root, session, ledger, checked, reads,
    get runtime() {
      return runtime;
    },
    /** How many evaluators the session has opened: one, unless it was closed and opened again. */
    get evaluators() {
      return evaluators;
    },
    /** A steer for `a.ts`, pending for the session's next answer. */
    steer: async (content = STEER) => await delivery!.requestSteer(session.sessionId, "steer", content, {
      revision: 0, scope: ["a.ts"], ruleIds: ["test:no-bad"], rules: [{ id: "test:no-bad", state: "steered" }],
    }),
    /** `path`'s finding, held for the yield's digest (with the experimental digest on). */
    hold: async (path = "a.ts") => await delivery!.requestSteer(session.sessionId, "steer", renderSteer([finding(path)]), {
      revision: 0, scope: [path], ruleIds: ["test:no-bad"], rules: [{ id: "test:no-bad", state: "steered" }],
      generatedEdit: { violations: [finding(path)], sourceOf: () => undefined },
    }),
    prompt: (eventId: string) => runtime.ingest({ kind: "user-prompt", eventId, sessionId: session.sessionId, session, text: eventId }),
    turnEnd: (eventId: string) => runtime.ingest({ kind: "turn-end", eventId, sessionId: session.sessionId, session, text: "" }),
    reopen: async () => {
      await runtime.dispose();
      runtime = open();
    },
  };
}

describe("what a session owes its agent while it is closed", () => {
  it("gives the agent the steer a session still owed when it was closed after a day, with its next event, once", async () => {
    const owing = await owingSession();
    try {
      await owing.steer();
      // The engine service closes a session quiet for a day whatever it owes, as one left at a permission prompt overnight.
      expect(await owing.runtime.closeIdleSession(owing.session.sessionId, { evenIfOwing: true })).toMatchObject({ kind: "suspended" });
      expect((await owing.prompt("next-morning")).map(delivery => delivery.content)).toEqual([STEER]);
      // Delivered: the session owes nothing now, closed again or not.
      expect(await owing.runtime.closeIdleSession(owing.session.sessionId)).toMatchObject({ kind: "suspended" });
      expect(await owing.prompt("later")).toEqual([]);
    } finally {
      await owing.runtime.dispose();
    }
  });

  it("gives the agent a steer owed when its runtime closed, as at a daemon restart, with the session's next event under the next one", async () => {
    const owing = await owingSession();
    try {
      await owing.steer();
      await owing.reopen();
      expect((await owing.prompt("after-restart")).map(delivery => delivery.content)).toEqual([STEER]);
    } finally {
      await owing.runtime.dispose();
    }
  });

  it("keeps a finding held for the digest across a close, and lists it in the digest at the next end of the agent's run", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST", "1");
    const owing = await owingSession();
    try {
      await owing.hold();
      expect(await owing.runtime.closeIdleSession(owing.session.sessionId, { evenIfOwing: true })).toMatchObject({ kind: "suspended" });
      expect(await owing.prompt("next-morning")).toEqual([]);
      await owing.runtime.codeRulesOpened(owing.session.sessionId);
      expect((await owing.turnEnd("turn")).map(delivery => delivery.content)).toEqual([DIGEST]);
      expect(await owing.turnEnd("turn-2")).toEqual([]);
    } finally {
      await owing.runtime.dispose();
    }
  });

  it("checks the edit of a call that waited across a close once the session's rules are open, though its result came while they opened", async () => {
    const owing = await owingSession();
    const { session } = owing;
    try {
      await owing.runtime.ingest({
        kind: "call", eventId: "call", sessionId: session.sessionId, session, toolCallId: "t1", toolName: "Write",
        input: { file_path: join(owing.root, "a.ts"), content: BAD },
      });
      // Left at the permission prompt past the day, then allowed: the write lands, and its result opens the session again.
      expect(await owing.runtime.closeIdleSession(session.sessionId, { evenIfOwing: true })).toMatchObject({ kind: "suspended" });
      await writeFile(join(owing.root, "a.ts"), BAD);
      const result = await owing.runtime.ingest({ kind: "result", eventId: "result", sessionId: session.sessionId, session, toolCallId: "t1", toolName: "Write", output: "ok" });
      expect(result).toEqual([]);
      await vi.waitFor(() => expect(owing.checked).toContain("a.ts"));
      expect((await owing.prompt("next")).map(delivery => delivery.content)).toEqual([STEER]);
    } finally {
      await owing.runtime.dispose();
    }
  });

  // The suspension keeps where the session's walks stood, and the waiting call's check reads the file once the rules are
  // open: the edit is the call's, read once in the session's mode, not again as another writer's change.
  it("checks once, as the call's own, the edit a call that waited across the day's close made to a file the session had edited", async () => {
    const owing = await owingSession();
    const { session } = owing;
    const tool = (toolCallId: string) => ({ sessionId: session.sessionId, session, toolCallId, toolName: "Write" }) as const;
    const call = (toolCallId: string) => owing.runtime.ingest({
      kind: "call", eventId: `${toolCallId}:call`, ...tool(toolCallId), input: { file_path: join(owing.root, "a.ts"), content: "" },
    });
    const result = (toolCallId: string) => owing.runtime.ingest({ kind: "result", eventId: `${toolCallId}:result`, ...tool(toolCallId), output: "ok" });
    try {
      await call("t0");
      await writeFile(join(owing.root, "a.ts"), "export const a = 1;\nconst ok = 1;\n");
      await result("t0");
      await owing.turnEnd("turn-1");
      await call("t1");
      // Left at the permission prompt past the day, then allowed: the write lands, and its result opens the session again.
      expect(await owing.runtime.closeIdleSession(session.sessionId, { evenIfOwing: true })).toMatchObject({ kind: "suspended" });
      await writeFile(join(owing.root, "a.ts"), BAD);
      expect(await result("t1")).toEqual([]);
      await vi.waitFor(() => expect(owing.reads.some(read => read.bad)).toBe(true));
      expect((await owing.prompt("next")).map(delivery => delivery.content)).toEqual([STEER]);
      expect(await owing.turnEnd("turn-2")).toEqual([]);
      expect(owing.reads.filter(read => read.bad)).toEqual([{ path: "a.ts", mode: "session", bad: true }]);
    } finally {
      await owing.runtime.dispose();
    }
  });

  // A steer reaches the agent with the next event either way; a session closed and opened again would check the agent's
  // next edit only at the turn end, while its rules compile again.
  it("keeps a quiet session that owes a steer open at the idle limit, so the agent's next edit is checked at its own result", async () => {
    const reopened = Promise.withResolvers<void>();
    const owing = await owingSession({ reopened: reopened.promise });
    const { session } = owing;
    try {
      await owing.steer("Fix a.py.");
      expect(await owing.runtime.closeIdleSession(session.sessionId)).toEqual({ kind: "owing" });
      expect((await owing.prompt("back")).map(delivery => delivery.content)).toEqual(["Fix a.py."]);
      await owing.runtime.ingest({
        kind: "call", eventId: "call", sessionId: session.sessionId, session, toolCallId: "t1", toolName: "Write",
        input: { file_path: join(owing.root, "a.ts"), content: BAD },
      });
      await writeFile(join(owing.root, "a.ts"), BAD);
      const result = await owing.runtime.ingest({ kind: "result", eventId: "result", sessionId: session.sessionId, session, toolCallId: "t1", toolName: "Write", output: "ok" });
      expect(owing.checked).toEqual(["a.ts"]);
      expect(result.map(delivery => delivery.content)).toEqual([STEER]);
      reopened.resolve();
      expect(await owing.turnEnd("turn")).toEqual([]);
      expect(owing.evaluators).toBe(1);
    } finally {
      reopened.resolve();
      await owing.runtime.dispose();
    }
  });

  // A scan opens a suspended session with no event, so the steer its ledger gave back stays owed. The session was then
  // suspended again at the next idle close, and the agent's next edit was checked only at the turn's end.
  it("keeps a session a scan opened again in memory at the idle limit while it owes the steer its ledger gave back", async () => {
    const owing = await owingSession();
    try {
      await owing.steer();
      expect(await owing.runtime.closeIdleSession(owing.session.sessionId, { evenIfOwing: true })).toMatchObject({ kind: "suspended" });
      await owing.runtime.resumeSession(owing.session);
      await owing.runtime.codeRulesOpened(owing.session.sessionId);
      expect(await owing.runtime.closeIdleSession(owing.session.sessionId)).toEqual({ kind: "owing" });
      expect((await owing.prompt("back")).map(delivery => delivery.content)).toEqual([STEER]);
      expect(owing.evaluators).toBe(2);
    } finally {
      await owing.runtime.dispose();
    }
  });

  it("does not give the agent a steer owed across a close once its file changed while the session was closed", async () => {
    const owing = await owingSession();
    try {
      await owing.steer();
      await owing.reopen();
      // The person edits the file before the agent's next event opens the session again.
      await writeFile(join(owing.root, "a.ts"), "export const a = 2;\n");
      expect(await owing.turnEnd("stop")).toEqual([]);
    } finally {
      await owing.runtime.dispose();
    }
  });

  it("leaves out of the digest a finding held across a close once its file changed while the session was closed", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST", "1");
    const owing = await owingSession();
    try {
      await owing.hold();
      expect(await owing.runtime.closeIdleSession(owing.session.sessionId, { evenIfOwing: true })).toMatchObject({ kind: "suspended" });
      await writeFile(join(owing.root, "a.ts"), "export const a = 2;\n");
      expect(await owing.prompt("next-morning")).toEqual([]);
      await owing.runtime.codeRulesOpened(owing.session.sessionId);
      expect(await owing.turnEnd("turn")).toEqual([]);
    } finally {
      await owing.runtime.dispose();
    }
  });

  it("drops what a session owed once it ends, as its end always has", async () => {
    const owing = await owingSession();
    try {
      await owing.steer();
      await owing.reopen();
      await owing.runtime.ingest({ kind: "session-end", eventId: "end", sessionId: owing.session.sessionId, session: owing.session });
      // Claude Code resumes a session under its id.
      expect(await owing.prompt("resumed")).toEqual([]);
    } finally {
      await owing.runtime.dispose();
    }
  });

  it("keeps an owed steer in the ledger scrubbed of secrets, and a finding in a secret-like file without its message", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST", "1");
    const owing = await owingSession();
    try {
      await owing.steer(`Move \`const key = "${SECRET}"\` to the environment.`);
      await owing.hold("secrets.yaml");
      await owing.reopen();
      const ledger = await readFile(owing.ledger, "utf8");
      expect(ledger).not.toContain(SECRET);
      expect(ledger).not.toContain("Do not write bad.");
      expect((await owing.prompt("after-restart")).map(delivery => delivery.content)).toEqual([`Move \`const key = "${REDACTED}"\` to the environment.`]);
    } finally {
      await owing.runtime.dispose();
    }
  });
});
