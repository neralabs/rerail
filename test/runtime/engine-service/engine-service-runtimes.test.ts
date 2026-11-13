import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm, writeFile, type FileHandle } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CheckoutCatalogs, codeRuleSchema, SqliteAuditControlStore, type AdapterEvent, type AuditRecorder, type CodeRuleAuditor,
  type RuleSteerDelivery,
} from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService, type EngineService } from "../../../src/runtime/engine-service.js";
import { testEmbedder } from "../../support/hook-events.js";
import { setUpTestRepository, type TestRepositoryRules } from "../../support/repository-rules.js";
import { startTestRepository, stopTestRepository } from "../../support/started-repository.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

type CodeRuleEditBatch = Parameters<CodeRuleAuditor["auditGeneratedEdits"]>[1];

/** The opens of a file a test holds (`holdOpen`), by its absolute path, each waiting until the test lets it go on. */
const heldOpens = vi.hoisted(() => ({ paths: new Map<string, Array<() => Promise<void>>>(), releases: [] as Array<() => void> }));
vi.mock("../../../src/engine/no-follow.js", async importOriginal => {
  const actual = await importOriginal<{ openNoFollow: (path: string, ...flags: number[]) => Promise<FileHandle> }>();
  return {
    ...actual,
    openNoFollow: async (path: string, ...flags: number[]) => {
      await heldOpens.paths.get(path)?.shift()?.();
      return await actual.openNoFollow(path, ...flags);
    },
  };
});

/** Holds the next open of the file at `path` until `release`; `entered` settles once that open has begun. */
function holdOpen(path: string) {
  const entered = Promise.withResolvers<void>();
  const released = Promise.withResolvers<void>();
  heldOpens.paths.set(path, [...(heldOpens.paths.get(path) ?? []), async () => {
    entered.resolve();
    await released.promise;
  }]);
  heldOpens.releases.push(released.resolve);
  return { entered: entered.promise, release: () => released.resolve() };
}

const roots: string[] = [];
const services: EngineService[] = [];
afterEach(async () => {
  for (const release of heldOpens.releases.splice(0)) release();
  heldOpens.paths.clear();
  vi.unstubAllEnvs();
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

/** An evaluator that checks nothing. */
function quietAuditor(): CodeRuleAuditor {
  return {
    openWorkspace: async () => undefined,
    acceptsPath: () => true,
    auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
    close: async () => undefined,
  };
}

/**
 * An evaluator that checks nothing and records every batch of edits it is given (`batches`) and the paths a walk asked
 * it about (`looked`); its scans (`scans`) complete with no finding once `auditing` settles, each recorded in the audit
 * store as a real scan is. Every evaluator after the first opens once `reopening` settles, as one compiling its rules
 * does, and `reopened` settles as the second begins to. The first evaluator to close does so once `closing` settles, and
 * fails to with `closeFails`; `closeBegun` settles as it begins to.
 */
function recordingAuditor(options: { reopening?: Promise<void>; closing?: Promise<void>; closeFails?: boolean; auditing?: Promise<void> } = {}) {
  const batches: CodeRuleEditBatch[] = [];
  const scans: string[] = [];
  const looked = new Set<string>();
  const reopened = Promise.withResolvers<void>();
  const closeBegun = Promise.withResolvers<void>();
  let opened = 0;
  let closed = 0;
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
      await options.auditing;
      const auditId = audits.start({
        sessionId, workspaceRoot, scope: input.scope, mode: input.mode, revision: 0, catalogRevision: "test", ruleRevisions: [], paths: [],
      });
      audits.finish(auditId, { status: "completed", violations: [], failures: [] });
      return [auditId];
    },
    close: async () => {
      closed += 1;
      if (closed > 1) return;
      closeBegun.resolve();
      await options.closing;
      if (options.closeFails === true) throw new Error("close failed");
    },
  });
  return { create, batches, scans, looked, reopened: reopened.promise, closeBegun: closeBegun.promise };
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
   * `gitFirst` makes `first` a Git repository before it starts, as a checkout's identity is taken then. Sessions come
   * from Claude Code unless `host` says otherwise.
   */
  async function idleService(
    options: Partial<Parameters<typeof startEngineService>[0]> = {},
    { gitFirst = false, host = "claude-code" }: { gitFirst?: boolean; host?: "claude-code" | "omp" } = {},
  ) {
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
      createAuditor: quietAuditor,
      ...options,
    });
    services.push(service);
    const session = (sessionId: string, cwd = first) => ({ host, sessionId, cwd });
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

  // The open session claims a queued scan at its control timer's next tick. One queued just before the sweep, which the
  // API found the session open for, was left queued by the suspension until the session's next event.
  it("keeps a quiet session open while a scan asked for it waits to be claimed, and runs the scan", async () => {
    // The control timer ticks only as the test moves it, so the sweep comes before the claim.
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    try {
      const { create, scans } = recordingAuditor();
      const { clock, service, start, databasePath } = await idleService({ createAuditor: create });
      await start("a");
      clock.now += 60_000;
      const controls = new SqliteAuditControlStore(databasePath);
      try {
        const request = controls.requestAudit("a", { scope: "repository" });
        await service.resumeSession("a");
        await service.closeIdleSessions();
        await vi.waitFor(() => {
          expect(controls.listAuditRequests("a").find(item => item.id === request.id)).toMatchObject({ status: "completed", error: null });
        }, { timeout: 10_000, interval: 50 });
        expect(scans).toEqual(["repository"]);
        expect(service.openRuntimes()).toBe(1);
      } finally {
        controls.close();
      }
    } finally {
      vi.useRealTimers();
    }
  }, 20_000);

  // A scan asked for while the session closed queued for it, but the API found no suspension to open again until the
  // close had finished.
  it("runs a scan asked for a quiet session while its close runs", async () => {
    const closing = Promise.withResolvers<void>();
    const { create, scans, looked, closeBegun } = recordingAuditor({ closing: closing.promise });
    const { first, clock, service, start, databasePath } = await idleService({ createAuditor: create });
    await writeFile(join(first, "app.py"), "x = 1\n");
    await start("a");
    // The evaluator is open, so the sweep's close waits for it.
    await vi.waitFor(() => expect(looked.has("app.py")).toBe(true));
    clock.now += 60_000;
    const sweeping = service.closeIdleSessions();
    await closeBegun;
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      const request = controls.requestAudit("a", { scope: "repository" });
      expect(request).toMatchObject({ status: "queued", error: null });
      const resuming = service.resumeSession("a");
      closing.resolve();
      await Promise.all([sweeping, resuming]);
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

  // The end opens the suspended session again to close it. An open that failed lost the end: the record stayed open and
  // owned, read as live until the daemon stopped, and the runtime made for the end stayed open as long.
  it("ends a quiet session's record when its end comes and it cannot be opened again, and closes the runtime made for it", async () => {
    let made = 0;
    const { clock, service, session, send, start, ended, live } = await idleService({
      createAuditor: () => {
        made += 1;
        if (made > 1) throw new Error("evaluator unavailable");
        return quietAuditor();
      },
    });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(0);
    await expect(send({ kind: "session-end", eventId: "a:end", sessionId: "a", session: session("a") })).rejects.toThrow(/evaluator unavailable/u);
    expect(made).toBe(2);
    expect(ended("a")).not.toBeNull();
    expect(live("a")).toBe(false);
    expect(service.openRuntimes()).toBe(0);
  });

  // A suspension whose close failed was kept by nobody: its next event could not open it, its owner still active, and
  // its record stayed open and owned after the daemon stopped.
  it("keeps a quiet session suspended when its close fails, so its next event opens it again", async () => {
    const { create, looked } = recordingAuditor({ closeFails: true });
    const { first, clock, service, start, prompt, ended, live } = await idleService({ createAuditor: create });
    await writeFile(join(first, "app.py"), "x = 1\n");
    await start("a");
    // The evaluator is open, so the sweep's close waits for it, and fails.
    await vi.waitFor(() => expect(looked.has("app.py")).toBe(true));
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(ended("a")).toBeNull();
    expect(live("a")).toBe(true);
    await prompt("a", "back");
    expect(live("a")).toBe(true);
  });

  it("ends a quiet session whose close failed when the service stops", async () => {
    const { create, looked } = recordingAuditor({ closeFails: true });
    const { first, clock, service, start, ended, live } = await idleService({ createAuditor: create });
    await writeFile(join(first, "app.py"), "x = 1\n");
    await start("a");
    await vi.waitFor(() => expect(looked.has("app.py")).toBe(true));
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(ended("a")).toBeNull();
    services.splice(services.indexOf(service), 1);
    await service.close();
    expect(ended("a")).not.toBeNull();
    expect(live("a")).toBe(false);
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
  // session's first walk has looked at the file it writes: in OMP and Pi, whose tool calls do not wait for a reopened
  // session's rules as Claude Code's, Codex's and Cursor's do (`REOPENED_RULES_WAIT_MS`), and in any of them once that
  // wait has run out.
  it("measures it so too when the write lands while the reopened session's rules compile", async () => {
    const reopening = Promise.withResolvers<void>();
    const { create, batches, looked, reopened } = recordingAuditor({ reopening: reopening.promise });
    const { first, clock, service, start, prompt, writes, turnEnd } = await idleService({ createAuditor: create }, { host: "omp" });
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

  // After a long think the agent's first event can be a command that rewrites a file it edited. The event was answered
  // before the reopened session had read the file's text back, so a command that ran first left nothing to measure from.
  it("answers the event that opens a quiet session again once the text of the files its agent edited is read back", async () => {
    const { create, batches, looked } = recordingAuditor();
    const { first, clock, service, session, send, start, writes, turnEnd } = await idleService({ createAuditor: create });
    const app = join(first, "app.py");
    await writeFile(app, "x = 1\n");
    await start("a");
    await vi.waitFor(() => expect(looked.has("app.py")).toBe(true));
    await writes("a", "t1", "Write", app, "x = 2\n");
    await turnEnd("a", "turn1");
    clock.now += 60_000;
    await service.closeIdleSessions();
    // The read back of app.py is slow: it goes on once the agent's command has run, or after 2 s.
    const readBack = holdOpen(await realpath(app));
    const tool = { sessionId: "a", session: session("a"), toolCallId: "t2", toolName: "Bash" } as const;
    const command = send({ kind: "call", eventId: "a:t2:call", ...tool, input: { command: "python generate.py" } })
      .then(() => writeFile(app, "x = 2\ny = 3\n"));
    await readBack.entered;
    await Promise.race([command, delay(2_000)]);
    readBack.release();
    await command;
    await send({ kind: "result", eventId: "a:t2:result", ...tool, output: "ok" });
    await turnEnd("a", "turn2");
    expect(editsOf(batches, "app.py")).toContainEqual({ generatedText: "x = 2\ny = 3\n", previousText: "x = 2\n", previousReadError: undefined });
  }, 20_000);

  // The reopened session's first walk had no `HEAD` from before the quiet, so it took `HEAD`'s last move as made since:
  // after a checkout, the agent's removal of a file it made and never committed read as Git's, recorded in Log only. The
  // command runs while the rules compile again, as in OMP (above).
  it("takes the agent's removal of a file it made, in its first command after the quiet, as its own, as the open session did", async () => {
    const reopening = Promise.withResolvers<void>();
    const { create, batches, looked, reopened } = recordingAuditor({ reopening: reopening.promise });
    const { first, clock, service, session, send, start, writes, turnEnd } = await idleService({ createAuditor: create }, { gitFirst: true, host: "omp" });
    const git = (...args: string[]) => execFileSync("git", ["-c", "user.name=Rerail Test", "-c", "user.email=test@example.invalid", ...args], { cwd: first });
    git("commit", "--quiet", "--allow-empty", "-m", "start");
    // `HEAD`'s last move is a checkout, which writes files, not a commit.
    git("checkout", "--quiet", "-b", "work");
    const [other, made] = [join(first, "other.py"), join(first, "made.py")];
    await writeFile(other, "z = 1\n");
    await start("a");
    await vi.waitFor(() => expect(looked.has("other.py")).toBe(true));
    await writes("a", "t1", "Write", made, "x = 1\n");
    await turnEnd("a", "turn1");
    clock.now += 60_000;
    await service.closeIdleSessions();
    const tool = { sessionId: "a", session: session("a"), toolCallId: "t2", toolName: "Bash" } as const;
    await send({ kind: "call", eventId: "a:t2:call", ...tool, input: { command: "rm made.py" } });
    await reopened;
    await rm(made);
    await send({ kind: "result", eventId: "a:t2:result", ...tool, output: "" });
    // The rules have compiled once the reopened session's first walk looks at the files, which finds the removal.
    looked.clear();
    reopening.resolve();
    await vi.waitFor(() => expect(looked.has("other.py")).toBe(true));
    await turnEnd("a", "turn2");
    // Checked as the session's own edit, in the session's mode, not recorded in Log as another writer's.
    const removals = batches.filter(batch => batch.edits.some(edit => edit.path === "made.py" && edit.deleted === true));
    expect(removals.map(batch => batch.mode ?? "session")).toEqual(["session"]);
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

  // Internal testing: the session's first checkout audit runs in its evaluator, which the suspension closes.
  it("lets a quiet session's checkout audit finish before it suspends the session", async () => {
    vi.stubEnv("RERAIL_EXPERIMENTAL_FIRST_REPOSITORY_AUDIT", "1");
    const auditing = Promise.withResolvers<void>();
    const { create, scans } = recordingAuditor({ auditing: auditing.promise });
    const { clock, service, start } = await idleService({ createAuditor: create });
    await start("a");
    await vi.waitFor(() => expect(scans).toEqual(["repository"]));
    clock.now += 60_000;
    const sweeping = service.closeIdleSessions();
    await Promise.race([sweeping, delay(2_000)]);
    auditing.resolve();
    await sweeping;
    expect(service.openRuntimes()).toBe(1);
    // Once the audit has finished, the next sweep suspends the session.
    await vi.waitFor(async () => {
      await service.closeIdleSessions();
      expect(service.openRuntimes()).toBe(0);
    });
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

  /** `idleService` whose sessions' rule checks steer only when the test calls `steer`, as a scan finishing late does. */
  async function steeringService() {
    let delivery: RuleSteerDelivery | undefined;
    const service = await idleService({
      createAuditor: ({ steerDelivery }) => {
        delivery = steerDelivery;
        return {
          openWorkspace: async () => undefined,
          acceptsPath: () => true,
          auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
          close: async () => undefined,
        };
      },
    });
    const steer = async (sessionId: string, content: string) => await delivery!.requestSteer(sessionId, "steer", content, {
      revision: 0, scope: [], ruleIds: ["test:owed"], rules: [{ id: "test:owed", state: "steered" }],
    });
    return { ...service, steer };
  }

  it("gives the agent the steer a session quiet for a day still owed, with the session's next event", async () => {
    const { clock, service, session, send, start, prompt, ended, steer } = await steeringService();
    await start("a");
    // Left at a permission prompt overnight: the session waits on a tool call, so only the day closes it.
    await send({ kind: "call", eventId: "a:call", sessionId: "a", session: session("a"), toolCallId: "t1", toolName: "Bash", input: { command: "rm -rf build" } });
    await steer("a", "Fix a.py.");
    clock.now += 24 * 60 * 60_000;
    await service.closeIdleSessions();
    // Suspended, not ended: its memory is freed and its record stays open.
    expect(service.openRuntimes()).toBe(0);
    expect(ended("a")).toBeNull();
    expect((await prompt("a", "next morning")).map(delivery => delivery.content)).toEqual(["Fix a.py."]);
  });

  // An end drops what the session owed. One from a repository stopped since dropped nothing: the ledger kept the steer,
  // and the session's next start under the same id took it back.
  it.each([
    ["still suspended", false],
    ["opened again by a scan", true],
  ])("drops the steer a session quiet for a day owed when its end comes from a repository stopped since, %s", async (_case, resumed) => {
    const { first, clock, service, session, send, start, prompt, steer, databasePath } = await steeringService();
    await start("a");
    await send({ kind: "call", eventId: "a:call", sessionId: "a", session: session("a"), toolCallId: "t1", toolName: "Bash", input: { command: "rm -rf build" } });
    await steer("a", "Fix a.py.");
    clock.now += 24 * 60 * 60_000;
    await service.closeIdleSessions();
    // A scan opens the session with no event that takes the steer, so it stays owed in memory.
    if (resumed) await service.resumeSession("a");
    stopTestRepository(first, databasePath);
    await send({ kind: "session-end", eventId: "a:end", sessionId: "a", session: session("a") });
    startTestRepository(first, databasePath);
    // Claude Code resumes a session under its id.
    expect(await prompt("a", "resumed")).toEqual([]);
  });

  // The end of a suspended session that cannot be opened again ended its record and left the steer in its ledger, so
  // the session's next start under the same id delivered a steer its end had dropped.
  it("drops the steer a session quiet for a day owed when its end comes and it cannot be opened again", async () => {
    let delivery: RuleSteerDelivery | undefined;
    let made = 0;
    const { clock, service, session, send, start, prompt, ended } = await idleService({
      createAuditor: ({ steerDelivery }) => {
        made += 1;
        if (made === 2) throw new Error("evaluator unavailable");
        delivery = steerDelivery;
        return quietAuditor();
      },
    });
    await start("a");
    await send({ kind: "call", eventId: "a:call", sessionId: "a", session: session("a"), toolCallId: "t1", toolName: "Bash", input: { command: "rm -rf build" } });
    await delivery!.requestSteer("a", "steer", "Fix a.py.", { revision: 0, scope: [], ruleIds: ["test:owed"], rules: [{ id: "test:owed", state: "steered" }] });
    clock.now += 24 * 60 * 60_000;
    await service.closeIdleSessions();
    await expect(send({ kind: "session-end", eventId: "a:end", sessionId: "a", session: session("a") })).rejects.toThrow(/evaluator unavailable/u);
    expect(made).toBe(2);
    expect(ended("a")).not.toBeNull();
    expect(await prompt("a", "resumed")).toEqual([]);
  });

  // After a daemon restart the session is neither open nor suspended in the new runtime, so an end from a repository
  // stopped since wrote nothing, and the ledger kept the steer for the session's next start under the same id.
  it.each([
    ["no end comes, so the steer waits for the session's next event", "none"],
    ["its end comes", "started"],
    ["its end comes from a repository stopped since", "stopped"],
  ])("gives or drops a steer owed across a daemon restart as %s", async (_case, end) => {
    const { first, service, session, send, start, steer, databasePath } = await steeringService();
    await start("a");
    await send({ kind: "call", eventId: "a:call", sessionId: "a", session: session("a"), toolCallId: "t1", toolName: "Bash", input: { command: "rm -rf build" } });
    await steer("a", "Fix a.py.");
    services.splice(services.indexOf(service), 1);
    await service.close();
    const root = dirname(databasePath);
    const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-idle-again-${process.pid}-${Date.now()}` : join(root, "engine-again.sock");
    const again = await startEngineService({
      socketPath, storageRoot: join(root, "state"), databasePath, embedder: testEmbedder, idleSessionMs: 60_000, idleRuntimeMs: 0,
      createAuditor: quietAuditor,
    });
    services.push(again);
    const sendAgain = (event: AdapterEvent) => ingestEngineEvent({ socketPath }, event);
    if (end === "stopped") stopTestRepository(first, databasePath);
    if (end !== "none") await sendAgain({ kind: "session-end", eventId: "a:end", sessionId: "a", session: session("a") });
    if (end === "stopped") startTestRepository(first, databasePath);
    const delivered = await sendAgain({ kind: "user-prompt", eventId: "a:resumed", sessionId: "a", session: session("a"), text: "resumed" });
    expect(delivered.map(delivery => delivery.content)).toEqual(end === "none" ? ["Fix a.py."] : []);
  });

  // Suspended and opened again by its next event, the session would check the agent's next edit only at the turn end.
  it("keeps a quiet session that owes its agent a steer in memory at the idle limit, and its next event carries the steer", async () => {
    const { clock, service, start, prompt, ended, steer } = await steeringService();
    await start("a");
    await steer("a", "Fix a.py.");
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
    expect(ended("a")).toBeNull();
    expect((await prompt("a", "back")).map(delivery => delivery.content)).toEqual(["Fix a.py."]);
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
