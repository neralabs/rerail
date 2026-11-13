import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CheckoutCatalogs, codeRuleSchema, SqliteAuditControlStore, type AdapterEvent, type AuditRecorder, type CodeRuleAuditor,
} from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService, type EngineService } from "../../../src/runtime/engine-service.js";
import { testEmbedder } from "../../support/hook-events.js";
import { setUpTestRepository, type TestRepositoryRules } from "../../support/repository-rules.js";
import { startTestRepository, stopTestRepository } from "../../support/started-repository.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

type CodeRuleEditBatch = Parameters<CodeRuleAuditor["auditGeneratedEdits"]>[1];

const roots: string[] = [];
const services: EngineService[] = [];
afterEach(async () => {
  await Promise.all(services.splice(0).map(service => service.close()));
  // A session's source walk can still hold a checkout folder open for a moment after its runtime closed, which fails an
  // immediate remove on Windows (EBUSY).
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

/**
 * An engine service over two started checkouts, `first` (a Git repository adding `rules`) and `second`, whose
 * runtimes stay `idleRuntimeMs` unused.
 */
async function twoCheckouts(idleRuntimeMs: number, rules?: TestRepositoryRules) {
  const root = await mkdtemp(join(tmpdir(), "rerail-runtimes-"));
  roots.push(root);
  const databasePath = join(root, "rerail.sqlite");
  const [first, second] = [join(root, "first"), join(root, "second")];
  for (const checkout of [first, second]) await mkdir(checkout);
  if (rules === undefined) startTestRepository(first, databasePath);
  else {
    execFileSync("git", ["init", "--quiet"], { cwd: first });
    setUpTestRepository(first, rules, databasePath);
  }
  startTestRepository(second, databasePath);
  const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-runtimes-${process.pid}-${Date.now()}` : join(root, "engine.sock");
  const service = await startEngineService({
    socketPath,
    storageRoot: join(root, "state"),
    databasePath,
    embedder: testEmbedder,
    idleRuntimeMs,
    createAuditor: () => ({
      openWorkspace: async () => undefined,
      acceptsPath: () => true,
      auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
      close: async () => undefined,
    }),
  });
  services.push(service);
  const send = (event: AdapterEvent) => ingestEngineEvent({ socketPath }, event);
  const session = (sessionId: string, cwd: string) => ({ host: "claude-code" as const, sessionId, cwd });
  const start = (sessionId: string, cwd: string) => send({ kind: "session-start", eventId: `${sessionId}:start`, session: session(sessionId, cwd) });
  const end = (sessionId: string, cwd: string) => send({ kind: "session-end", eventId: `${sessionId}:end`, sessionId, session: session(sessionId, cwd) });
  return { service, databasePath, first, second, start, end, send, session };
}

describe("the engine service's runtimes", () => {
  it("closes a checkout's runtime when its last session ends, so a daemon serving many worktrees does not keep them all", async () => {
    const { service, first, second, start, end, send, session } = await twoCheckouts(0);

    await start("a", first);
    await start("b", second);
    await start("c", second);
    expect(service.openRuntimes()).toBe(2);
    await end("a", first);
    expect(service.openRuntimes()).toBe(1);
    await end("b", second);
    expect(service.openRuntimes()).toBe(1);
    await end("c", second);
    expect(service.openRuntimes()).toBe(0);
    // The checkout's next session gets a runtime of its own, as its first did.
    await start("d", first);
    await send({ kind: "user-prompt", eventId: "d:prompt", sessionId: "d", session: session("d", first), text: "hi" });
    expect(service.openRuntimes()).toBe(1);
  });

  // A runtime holds its checkout's compiled rules, 12 s of CPU to compile for the five packs a TypeScript repository
  // adds: a session started soon after the last one ended reuses it, and the runtime closes once it was unused a while.
  it("keeps a checkout's runtime until no session has used it for the idle time, so a session that comes back soon reuses it", async () => {
    // Only the idle timer is faked: the runtime's own timers are intervals, and the HTTP client's are the socket's.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const { service, first, start, end } = await twoCheckouts(60_000);

      await start("a", first);
      await end("a", first);
      vi.advanceTimersByTime(59_999);
      expect(service.openRuntimes()).toBe(1);
      await start("b", first);
      vi.advanceTimersByTime(5 * 60_000);
      expect(service.openRuntimes()).toBe(1);
      // The idle time counts from the last session's end.
      await end("b", first);
      vi.advanceTimersByTime(59_999);
      expect(service.openRuntimes()).toBe(1);
      vi.advanceTimersByTime(1);
      expect(service.openRuntimes()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  // A compiled rule's program is native memory V8 does not count, 512 MB for the Node pack's builtins rule, and a daemon
  // with no session open allocates too little to start a collection: it kept a closed runtime's rules until the next
  // session came.
  it("frees a closed runtime's compiled rules without waiting for the daemon to allocate", async () => {
    const rule = codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:freed", revision: 1, mode: "block",
      scope: { include: ["**/*.py"], exclude: [] },
      clauses: [{ id: "test:freed:check", violation: { anchor: "witness.span", code: "NO_PRINT", message: "No print" },
        forbid: { ast: { language: "python", rule: { pattern: "print($ARG)" } } } }],
    });
    const { service, databasePath, first, start, end } = await twoCheckouts(0, { library: [{ kind: "code", rule }] });

    await start("a", first);
    // The checkout as the registry recorded it: a temporary folder's physical path differs on macOS (/private/var).
    const recorded = await realpath(first);
    // The rule as this checkout's catalogs compile it: the open runtime's catalog shares the executable.
    const compiled = ((): WeakRef<object> => {
      const catalogs = new CheckoutCatalogs(databasePath);
      try {
        const [entry] = catalogs.catalog(recorded).snapshot().codeRules;
        if (entry === undefined) throw new Error("the library rule did not compile");
        return new WeakRef(entry.executable);
      } finally {
        catalogs.close();
      }
    })();

    await end("a", first);
    expect(service.openRuntimes()).toBe(0);
    // Nothing in this test collects: the service does, once the runtime closed.
    await vi.waitFor(() => expect(compiled.deref()).toBeUndefined(), { timeout: 5_000 });
  });
});

/**
 * An evaluator that checks nothing and records every batch of edits it is given (`batches`) and the paths a walk asked
 * it about (`looked`); its scans (`scans`) complete with no finding, each recorded in the audit store as a real scan is.
 * Every evaluator after the first opens once `reopening` settles, as one compiling its rules does, and `reopened`
 * settles as the second begins to.
 */
function recordingAuditor(options: { reopening?: Promise<void> } = {}) {
  const batches: CodeRuleEditBatch[] = [];
  const scans: string[] = [];
  const looked = new Set<string>();
  const reopened = Promise.withResolvers<void>();
  let opened = 0;
  const create = ({ workspaceRoot, audits }: { workspaceRoot: string; audits: AuditRecorder }): CodeRuleAuditor => ({
    openWorkspace: async () => {
      opened += 1;
      if (opened === 1) return;
      reopened.resolve();
      await options.reopening;
    },
    acceptsPath: (path) => {
      looked.add(path);
      return true;
    },
    auditGeneratedEdits: async (_sessionId, batch) => {
      batches.push(batch);
      return { revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] };
    },
    audit: async (sessionId, input) => {
      scans.push(input.scope);
      const auditId = audits.start({
        sessionId, workspaceRoot, scope: input.scope, mode: input.mode, revision: 0, catalogRevision: "test", ruleRevisions: [], paths: [],
      });
      audits.finish(auditId, { status: "completed", violations: [], failures: [] });
      return [auditId];
    },
    close: async () => undefined,
  });
  return { create, batches, scans, looked, reopened: reopened.promise };
}

/** Each audited edit of `path`: the text it left, and the text it was measured from, or why there was none. */
function editsOf(batches: readonly CodeRuleEditBatch[], path: string) {
  return batches.flatMap(batch => batch.edits)
    .filter(edit => edit.path === path)
    .map(({ generatedText, previousText, previousReadError }) => ({ generatedText, previousText, previousReadError }));
}

describe("the engine service's idle close", () => {
  /**
   * A service over two started checkouts whose sessions are suspended after a minute of `clock` without an event;
   * `gitFirst` makes `first` a Git repository before it starts, as a checkout's identity is taken then.
   */
  async function idleService(options: Partial<Parameters<typeof startEngineService>[0]> = {}, { gitFirst = false } = {}) {
    const root = await mkdtemp(join(tmpdir(), "rerail-idle-"));
    roots.push(root);
    const databasePath = join(root, "rerail.sqlite");
    const [first, second] = [join(root, "first"), join(root, "second")];
    for (const checkout of [first, second]) await mkdir(checkout);
    if (gitFirst) execFileSync("git", ["init", "--quiet"], { cwd: first });
    for (const checkout of [first, second]) startTestRepository(checkout, databasePath);
    const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-idle-${process.pid}-${Date.now()}` : join(root, "engine.sock");
    const clock = { now: 1_000_000 };
    const service = await startEngineService({
      socketPath,
      storageRoot: join(root, "state"),
      databasePath,
      embedder: testEmbedder,
      idleSessionMs: 60_000,
      // The runtime closes when its last session does, so the closed session's runtime goes with it.
      idleRuntimeMs: 0,
      now: () => clock.now,
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
        close: async () => undefined,
      }),
      ...options,
    });
    services.push(service);
    const session = (sessionId: string, cwd = first) => ({ host: "claude-code" as const, sessionId, cwd });
    const send = (event: AdapterEvent) => ingestEngineEvent({ socketPath }, event);
    return {
      first, second, clock, service, session, send, databasePath,
      start: (sessionId: string, cwd = first) => send({ kind: "session-start", eventId: `${sessionId}:start`, session: session(sessionId, cwd) }),
      prompt: (sessionId: string, text: string, cwd = first) =>
        send({ kind: "user-prompt", eventId: `${sessionId}:${text}`, sessionId, session: session(sessionId, cwd), text }),
      ended: (sessionId: string) => endedAt(databasePath, sessionId),
      live: (sessionId: string) => ownedLive(databasePath, sessionId),
      /**
       * The agent of `sessionId` writes `text` to `path` in tool call `toolCallId`: with its Write tool, or with a shell
       * command whose input names no file.
       */
      writes: async (sessionId: string, toolCallId: string, toolName: "Write" | "Bash", path: string, text: string) => {
        const tool = { sessionId, session: session(sessionId), toolCallId, toolName } as const;
        const input = toolName === "Write" ? { file_path: path, content: text } : { command: "python generate.py" };
        await send({ kind: "call", eventId: `${sessionId}:${toolCallId}:call`, ...tool, input });
        await writeFile(path, text);
        await send({ kind: "result", eventId: `${sessionId}:${toolCallId}:result`, ...tool, output: "ok" });
      },
      /** The end of a turn of `sessionId`, which waits for the session's walks and their checks. */
      turnEnd: (sessionId: string, turn: string) =>
        send({ kind: "turn-end", eventId: `${sessionId}:${turn}`, sessionId, session: session(sessionId), text: "done" }),
    };
  }

  // A quiet session's agent may still be open: a person away, a long think. Before the idle close it read as live.
  it("frees a quiet session's memory and keeps its record open and owned, so a killed agent's session holds nothing and a live one still reads as live", async () => {
    const { second, clock, service, start, prompt, ended, live } = await idleService();
    await start("a");
    await start("b", second);
    clock.now += 50_000;
    await prompt("b", "still here", second);
    clock.now += 20_000;
    // `a` has been quiet for 70 s, `b` for 20 s: only `a` lets go of its memory, and its checkout's runtime with it.
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
    expect(ended("a")).toBeNull();
    expect(live("a")).toBe(true);
    expect(ended("b")).toBeNull();
    // A suspended session's next event opens it again from its ledger.
    await prompt("a", "back");
    expect(service.openRuntimes()).toBe(2);
    expect(ended("a")).toBeNull();
    expect(live("a")).toBe(true);
  });

  it("ends a suspended session's record when the service stops, as stopping ends every open session", async () => {
    const { clock, service, start, ended, live } = await idleService();
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(ended("a")).toBeNull();
    services.splice(services.indexOf(service), 1);
    await service.close();
    expect(ended("a")).not.toBeNull();
    expect(live("a")).toBe(false);
  });

  // At base a scan asked for an open session was claimed within 250 ms and ran; the idle close failed it with
  // "Session is inactive" though the agent was still open.
  it("takes a scan asked for a quiet session and runs it now, as it did while the session was open", async () => {
    const { create, scans } = recordingAuditor();
    const { clock, service, start, databasePath } = await idleService({ createAuditor: create });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(0);
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      const request = controls.requestAudit("a", { scope: "repository" });
      expect(request).toMatchObject({ status: "queued", error: null });
      // The daemon's API asks the engine service to open the session again (`audit_requests_create`).
      await service.resumeSession("a");
      await vi.waitFor(() => {
        expect(controls.listAuditRequests("a").find(item => item.id === request.id)).toMatchObject({ status: "completed", error: null });
      }, { timeout: 10_000, interval: 50 });
      expect(scans).toEqual(["repository"]);
    } finally {
      controls.close();
    }
  });

  it("runs a scan queued for a quiet session once its next event opens it, as the session's owner", async () => {
    const { create, scans } = recordingAuditor();
    const { clock, service, start, prompt, databasePath } = await idleService({ createAuditor: create });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      // Queued by a writer that does not open the session (a second connection, as `rerail scan` once wrote).
      const request = controls.requestAudit("a", { scope: "repository" });
      expect(request).toMatchObject({ status: "queued", error: null });
      await prompt("a", "back");
      await vi.waitFor(() => {
        expect(controls.listAuditRequests("a").find(item => item.id === request.id)).toMatchObject({ status: "completed", error: null });
      }, { timeout: 10_000, interval: 50 });
      expect(scans).toEqual(["repository"]);
    } finally {
      controls.close();
    }
  });

  it("runs a scan asked for a quiet session whose repository was stopped since, as the open session ran it", async () => {
    const { create, scans } = recordingAuditor();
    const { first, clock, service, start, databasePath } = await idleService({ createAuditor: create });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    stopTestRepository(first, databasePath);
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      const request = controls.requestAudit("a", { scope: "repository" });
      expect(request).toMatchObject({ status: "queued", error: null });
      await service.resumeSession("a");
      await vi.waitFor(() => {
        expect(controls.listAuditRequests("a").find(item => item.id === request.id)).toMatchObject({ status: "completed", error: null });
      }, { timeout: 10_000, interval: 50 });
      expect(scans).toEqual(["repository"]);
    } finally {
      controls.close();
    }
  });

  it("ends a quiet session's record when its end comes, also from a repository stopped since", async () => {
    const { first, second, clock, service, session, send, start, ended, databasePath } = await idleService();
    await start("a");
    await start("b", second);
    clock.now += 60_000;
    await service.closeIdleSessions();
    // `a`'s end opens it again and closes it; `b`'s runtime records nothing once its repository stopped.
    stopTestRepository(second, databasePath);
    await send({ kind: "session-end", eventId: "a:end", sessionId: "a", session: session("a", first) });
    await send({ kind: "session-end", eventId: "b:end", sessionId: "b", session: session("b", second) });
    expect(ended("a")).not.toBeNull();
    expect(ended("b")).not.toBeNull();
  });

  // At base the quiet session's next walk found another writer's change to a file the session had edited and recorded
  // it in Log; after the idle close the reopened session's first walk took it as its baseline and recorded nothing.
  it("records another writer's change to a file the session edited, made while the session was quiet, as its open walk did", async () => {
    const { create, batches, looked } = recordingAuditor();
    const { first, clock, service, start, prompt, writes, turnEnd } = await idleService({ createAuditor: create });
    const [changed, removed, untouched] = [join(first, "app.py"), join(first, "gone.py"), join(first, "other.py")];
    await writeFile(changed, "x = 1\n");
    await writeFile(removed, "y = 1\n");
    await writeFile(untouched, "z = 1\n");
    await start("a");
    // The evaluator is open once the session's first walk has asked about the files, so the edits below are checked.
    await vi.waitFor(() => expect(looked.has("app.py")).toBe(true));
    // The agent edits two files with its own tools, and its turn ends.
    await writes("a", "t1", "Write", changed, "x = 2\n");
    await writes("a", "t2", "Write", removed, "y = 2\n");
    await turnEnd("a", "turn");
    expect(batches.flatMap(batch => batch.edits.map(item => item.path)).sort()).toEqual(["app.py", "gone.py"]);
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(0);
    // Another writer (a person's editor, a background process) changes one of them and removes the other meanwhile,
    // and changes a file the session never edited, which the open session's walk did not record either.
    await writeFile(changed, "x = 3\n");
    await rm(removed);
    await writeFile(untouched, "z = 3\n");
    await prompt("a", "back");
    await vi.waitFor(() => {
      const logged = batches.filter(batch => batch.mode === "log").flatMap(batch => batch.edits);
      expect(logged).toEqual(expect.arrayContaining([
        expect.objectContaining({ path: "app.py", generatedText: "x = 3\n", previousText: "x = 3\n" }),
        expect.objectContaining({ path: "gone.py", deleted: true }),
      ]));
    }, { timeout: 10_000, interval: 50 });
    await turnEnd("a", "turn2");
    expect(batches.flatMap(batch => batch.edits.map(item => item.path))).not.toContain("other.py");
  }, 20_000);

  // At base the open session kept the text of each file it edited, so its next shell write to one was measured from
  // that text and only the lines the write changed were the agent's. The reopened session had lost the text, so every
  // line of the file read as the agent's (`previousReadError`), and a rule judging the agent's lines judged them all.
  it("measures a shell write to a file the session edited before it went quiet from the text the session left there", async () => {
    const { create, batches, looked } = recordingAuditor();
    // A repository whose Git does not hold the session's text of the file, as with any edit not yet committed.
    const { first, clock, service, start, prompt, writes, turnEnd } = await idleService({ createAuditor: create }, { gitFirst: true });
    const app = join(first, "app.py");
    await writeFile(app, "x = 1\n");
    await start("a");
    await vi.waitFor(() => expect(looked.has("app.py")).toBe(true));
    await writes("a", "t1", "Write", app, "x = 2\n");
    await turnEnd("a", "turn1");
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(0);
    await prompt("a", "back");
    await turnEnd("a", "turn2");
    await prompt("a", "next");
    await writes("a", "t2", "Bash", app, "x = 2\ny = 3\n");
    await turnEnd("a", "turn3");
    expect(editsOf(batches, "app.py")).toContainEqual({ generatedText: "x = 2\ny = 3\n", previousText: "x = 2\n", previousReadError: undefined });
  }, 20_000);

  // The agent's first command after the quiet can run while the session's rules compile again, before the reopened
  // session's first walk has looked at the file it writes.
  it("measures it so too when the write lands while the reopened session's rules compile", async () => {
    const reopening = Promise.withResolvers<void>();
    const { create, batches, looked, reopened } = recordingAuditor({ reopening: reopening.promise });
    const { first, clock, service, start, prompt, writes, turnEnd } = await idleService({ createAuditor: create });
    const app = join(first, "app.py");
    await writeFile(app, "x = 1\n");
    await start("a");
    await vi.waitFor(() => expect(looked.has("app.py")).toBe(true));
    await writes("a", "t1", "Write", app, "x = 2\n");
    await turnEnd("a", "turn1");
    clock.now += 60_000;
    await service.closeIdleSessions();
    await prompt("a", "back");
    await reopened;
    await writes("a", "t2", "Bash", app, "x = 2\ny = 3\n");
    reopening.resolve();
    await turnEnd("a", "turn2");
    expect(editsOf(batches, "app.py")).toContainEqual({ generatedText: "x = 2\ny = 3\n", previousText: "x = 2\n", previousReadError: undefined });
  }, 20_000);

  it("keeps a quiet session in memory while it waits on a tool call, and suspends it once the call is done", async () => {
    const { clock, service, session, send, start, ended } = await idleService();
    const tool = { sessionId: "a", session: session("a"), toolCallId: "t1", toolName: "Bash" } as const;
    await start("a");
    // A long command, or a permission prompt the user has not answered: the session sends nothing meanwhile.
    await send({ kind: "call", eventId: "a:call", ...tool, input: { command: "npm test" } });
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
    await send({ kind: "result", eventId: "a:result", ...tool, output: "ok" });
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(0);
    expect(ended("a")).toBeNull();
  });

  it("suspends a session that waits on a tool call once it has been quiet a day, so an agent killed at a permission prompt does not keep its memory forever", async () => {
    const { clock, service, session, send, start, ended } = await idleService();
    const hour = 60 * 60_000;
    await start("a");
    // Claude Code closed while the call waited for the user's permission: its result and its end never come.
    await send({ kind: "call", eventId: "a:call", sessionId: "a", session: session("a"), toolCallId: "t1", toolName: "Bash", input: { command: "rm -rf build" } });
    clock.now += 23 * hour;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
    clock.now += hour;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(0);
    expect(ended("a")).toBeNull();
  });

  it("keeps a session open whose event came in while the sweep waited behind the one before it", async () => {
    const published = Promise.withResolvers<void>();
    const publishing = Promise.withResolvers<void>();
    const { clock, service, session, ended } = await idleService({
      beforeSessionPublish: async () => {
        published.resolve();
        await publishing.promise;
      },
    });
    // The start holds the session's lane, and its prompt waits behind it.
    const starting = service.ingest({ kind: "session-start", eventId: "a:start", session: session("a") });
    await published.promise;
    const prompting = service.ingest({ kind: "user-prompt", eventId: "a:prompt", sessionId: "a", session: session("a"), text: "hi" });
    clock.now += 60_000;
    // `a` has been quiet since its start as the sweep looks, so its close waits behind the prompt in the same lane.
    const sweeping = service.closeIdleSessions();
    publishing.resolve();
    await Promise.all([starting, prompting, sweeping]);
    expect(service.openRuntimes()).toBe(1);
    expect(ended("a")).toBeNull();
  });

  it("looks for quiet sessions every minute on its own", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    try {
      const { clock, service, start, ended } = await idleService();
      await start("a");
      clock.now += 60_000;
      vi.advanceTimersByTime(60_000);
      await vi.waitFor(() => expect(service.openRuntimes()).toBe(0));
      expect(ended("a")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("leaves no timer behind once it has closed, or when it could not listen", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    try {
      const root = await mkdtemp(join(tmpdir(), "rerail-idle-timer-"));
      roots.push(root);
      const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-idle-timer-${process.pid}-${Date.now()}` : join(root, "engine.sock");
      const options = { socketPath, storageRoot: join(root, "state"), databasePath: join(root, "rerail.sqlite"), embedder: testEmbedder };
      const service = await startEngineService(options);
      expect(vi.getTimerCount()).toBe(1);
      // A second service cannot listen on the socket the first holds.
      await expect(startEngineService(options)).rejects.toThrow();
      expect(vi.getTimerCount()).toBe(1);
      await service.close();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});

/** Whether `sessionId` reads as live (open and owned by a live runtime), as `rerail settings` reports it. */
function ownedLive(databasePath: string, sessionId: string): boolean {
  const controls = new SqliteAuditControlStore(databasePath);
  try {
    return controls.getSettings(sessionId).active;
  } finally {
    controls.close();
  }
}

/** When the database says `sessionId` ended, read through a connection of its own; null while it runs. */
function endedAt(databasePath: string, sessionId: string): number | null {
  const database = new Database(databasePath, { readonly: true });
  try {
    const row = database.prepare("SELECT ended_at AS endedAt FROM sessions WHERE session_id = ?").get(sessionId) as { endedAt: number | null } | undefined;
    if (row === undefined) throw new Error(`no session ${sessionId}`);
    return row.endedAt;
  } finally {
    database.close();
  }
}
