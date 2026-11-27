import { execFileSync, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, realpath, rename, rm, writeFile, type FileHandle } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CheckoutCatalogs, CodeRuleBridge, codeRuleSchema, RepositoryWorktreeWalks, SqliteAuditControlStore, type AdapterEvent, type AuditRecorder,
  type CodeRuleAuditor, type RuleSteerDelivery,
} from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService, type EngineService } from "../../../src/runtime/engine-service.js";
import { KeptRuntimes } from "../../../src/runtime/memory/kept-runtimes.js";
import { RerailRuntime } from "../../../src/runtime/runtime.js";
import { SqliteSessionStore } from "../../../src/runtime/session-store.js";
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

/** The folder looks a test holds (`holdLook`), by path: each runs before the look's stat and after it answers. */
const heldLooks = vi.hoisted(() => new Map<string, { before: () => Promise<void>; after: () => void }>());
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof import("node:fs/promises")>();
  return {
    ...actual,
    stat: async (...args: Parameters<typeof actual.stat>) => {
      const held = heldLooks.get(String(args[0]));
      await held?.before();
      try {
        return await actual.stat(...args);
      } finally {
        held?.after();
      }
    },
  };
});

/** Each wait for a write lock begun, with its signal, and whether a quiet session's open for a scan began it. */
const lockWaits = vi.hoisted(() => [] as Array<{ signal: AbortSignal | undefined; resume: boolean }>);
/** While `held` is set, a wait for the write lock begins only once it settles: one that has not seen the lock given back. */
const lockWaitGate = vi.hoisted(() => ({ held: undefined as Promise<void> | undefined }));
vi.mock("../../../src/engine/storage/write-lock-wait.js", async importOriginal => {
  const actual = await importOriginal<{ waitForWriteLock: (path: string, signal?: AbortSignal, since?: number) => Promise<boolean> }>();
  return {
    ...actual,
    waitForWriteLock: (path: string, signal?: AbortSignal, since?: number) => {
      lockWaits.push({ signal, resume: new Error().stack?.includes("retryResume") === true });
      const held = lockWaitGate.held;
      return held === undefined ? actual.waitForWriteLock(path, signal, since) : held.then(() => actual.waitForWriteLock(path, signal, since));
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

/**
 * Holds the next read of where a session's walks stand (`CodeRuleBridge.walkResume`), which the idle close makes before
 * it suspends the session, until `release`; `entered` settles once it has begun.
 */
function holdWalkResume() {
  const entered = Promise.withResolvers<void>();
  const released = Promise.withResolvers<void>();
  const walkResume = CodeRuleBridge.prototype.walkResume;
  const spy = vi.spyOn(CodeRuleBridge.prototype, "walkResume").mockImplementationOnce(async function (this: CodeRuleBridge) {
    entered.resolve();
    await released.promise;
    return await walkResume.call(this);
  });
  return { entered: entered.promise, release: () => released.resolve(), restore: () => spy.mockRestore() };
}

/** Resolves once a control store records the end of the scan request `id`: the test waits for that, not a clock. */
function requestFinished(id: string): Promise<void> {
  const finish = SqliteAuditControlStore.prototype.finish;
  return new Promise(resolve => {
    vi.spyOn(SqliteAuditControlStore.prototype, "finish").mockImplementation(function (this: SqliteAuditControlStore, ...args) {
      finish.apply(this, args);
      if (args[0] === id) resolve();
    });
  });
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
 * An engine service over two started checkouts, `first` (a Git repository adding `rules`) and `second`, whose daemon
 * needs the room a runtime no session uses takes whenever `needsRoom` says so, and whose time is `clock.now`.
 */
async function twoCheckouts(needsRoom: (() => boolean) | undefined, rules?: TestRepositoryRules) {
  const clock = { now: 1_000_000 };
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
    now: () => clock.now,
    ...(needsRoom === undefined ? {} : { needsRoom }),
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
  return { service, databasePath, first, second, start, end, send, session, clock };
}

const MINUTE = 60_000;

describe("the engine service's runtimes", () => {
  // A /clear in Claude Code ends a session and starts the next in the same checkout a moment later. While the daemon
  // needed the room, the runtime closed as the first session left, and the next one compiled the rules and walked the
  // checkout again: in a daemon limited to 640 MB its SessionStart took 580 to 610 ms where it took 220 to 250 ms with
  // the runtime kept.
  it("keeps the runtime a session left though the daemon needs the room, so the next session in the checkout finds it", async () => {
    const disposed = vi.spyOn(RerailRuntime.prototype, "dispose");
    try {
      const { service, first, start, end } = await twoCheckouts(() => true);
      await start("a", first);
      await end("a", first);
      expect(service.openRuntimes()).toBe(1);
      await service.closeIdleSessions();
      expect(service.openRuntimes()).toBe(1);
      await start("b", first);
      expect(service.openRuntimes()).toBe(1);
      expect(disposed).not.toHaveBeenCalled();
    } finally {
      disposed.mockRestore();
    }
  });

  // A runtime holds its checkout's compiled rules, 12 s of CPU to compile for the five packs a TypeScript repository
  // adds. It closed 15 minutes after its last session ended, room or not, and a session that came back later compiled
  // them all again while its first tool calls waited.
  it("keeps a checkout's runtime after its last session ended while the daemon has room, however long, so a session that comes back reuses it", async () => {
    // The idle clocks of a runtime would be timeouts; the runtime's own timers are intervals, and the HTTP client's are the socket's.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const disposed = vi.spyOn(RerailRuntime.prototype, "dispose");
    try {
      const { service, first, start, end, clock } = await twoCheckouts(() => false);
      await start("a", first);
      await end("a", first);
      vi.advanceTimersByTime(2 * 60 * MINUTE);
      clock.now += 2 * 60 * MINUTE;
      await service.closeIdleSessions();
      expect(service.openRuntimes()).toBe(1);
      await start("b", first);
      expect(service.openRuntimes()).toBe(1);
      expect(disposed).not.toHaveBeenCalled();
    } finally {
      disposed.mockRestore();
      vi.useRealTimers();
    }
  });

  // A daemon that kept runtimes while it had room and then ran short closed every kept one as the next runtime opened,
  // on that session's path, one full collection each, and freed the compiled rules it was about to compile again: its
  // SessionStart took 700 to 770 ms where it took 350 to 380 ms with the runtimes closed one by one, 15 minutes after
  // each one's last session, off any session's path.
  it("opens a runtime in another checkout with no close on its path while the daemon needs the room", async () => {
    const disposed = vi.spyOn(RerailRuntime.prototype, "dispose");
    try {
      const { service, first, second, start, end } = await twoCheckouts(() => true);
      await start("a", first);
      await end("a", first);
      await start("b", second);
      expect(service.openRuntimes()).toBe(2);
      expect(disposed).not.toHaveBeenCalled();
    } finally {
      disposed.mockRestore();
    }
  });

  // Short of room, every runtime no session uses was freed 15 minutes after its last session left, the one left last
  // too: a daemon serving many worktrees freed each one's records. It still is, and none sooner, so a /clear or a
  // session back within 15 minutes finds its rules compiled as it did.
  it("frees each kept runtime at the first sweep 15 minutes after its last session left while the daemon needs the room, the one left last too", async () => {
    const { service, first, second, start, end, clock } = await twoCheckouts(() => true);
    await start("a", first);
    await start("b", second);
    await end("a", first);
    clock.now += 5 * MINUTE;
    await end("b", second);
    clock.now += 10 * MINUTE - 1;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(2);
    clock.now += 1;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
    clock.now += 5 * MINUTE;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(0);
  });

  // Each runtime no session used closed on its own 15-minute timer, whatever else the daemon was doing. Closed at the end
  // of the minute's sweep instead, after every quiet session's idle close had settled, one idle close that hung (a read
  // of a share that stops answering) or ran long (a large checkout's audit) held every other checkout's kept runtime past
  // its time while the daemon was short of room.
  it("frees a kept runtime 15 minutes after its last session left while another session's idle close hangs", async () => {
    const { service, first, second, start, end, clock } = await twoCheckouts(() => true);
    await start("a", first);
    await start("b", second);
    await end("b", second);
    const entered = Promise.withResolvers<void>();
    const released = Promise.withResolvers<void>();
    const closeIdle = RerailRuntime.prototype.closeIdleSession;
    const hang = vi.spyOn(RerailRuntime.prototype, "closeIdleSession").mockImplementation(async function (this: RerailRuntime, ...args) {
      entered.resolve();
      await released.promise;
      return await closeIdle.apply(this, args);
    });
    try {
      clock.now += 31 * MINUTE;
      const sweeping = service.closeIdleSessions();
      await entered.promise;
      const openWhileHung = service.openRuntimes();
      released.resolve();
      await sweeping;
      expect({ openWhileHung, openAfter: service.openRuntimes() }).toEqual({ openWhileHung: 1, openAfter: 1 });
    } finally {
      released.resolve();
      hang.mockRestore();
    }
  });

  // A runtime's 15 minutes were a timer's, on the monotonic clock: a wall clock set forward by an hour meanwhile closed
  // nothing sooner. Read from the wall clock, the age jumped with it, and the runtime of a person back within 15 minutes
  // was gone.
  it("counts a kept runtime's 15 minutes on the monotonic clock, which the wall clock's jumps do not move", async () => {
    // The wall clock as the service reads it from its start, set forward by `jump` milliseconds.
    const wallNow = Date.now.bind(Date);
    const wall = { jump: 0 };
    const wallClock = vi.spyOn(Date, "now").mockImplementation(() => wallNow() + wall.jump);
    const monotonicNow = performance.now.bind(performance);
    const monotonic = vi.spyOn(performance, "now");
    try {
      const root = await mkdtemp(join(tmpdir(), "rerail-runtimes-"));
      roots.push(root);
      const databasePath = join(root, "rerail.sqlite");
      const checkout = join(root, "first");
      await mkdir(checkout);
      startTestRepository(checkout, databasePath);
      const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-clock-${process.pid}-${wallNow()}` : join(root, "engine.sock");
      const service = await startEngineService({ socketPath, storageRoot: join(root, "state"), databasePath, embedder: testEmbedder, needsRoom: () => true, createAuditor: quietAuditor });
      services.push(service);
      const session = { host: "claude-code" as const, sessionId: "a", cwd: checkout };
      await ingestEngineEvent({ socketPath }, { kind: "session-start", eventId: "a:start", session });
      await ingestEngineEvent({ socketPath }, { kind: "session-end", eventId: "a:end", sessionId: "a", session });
      wall.jump = 60 * MINUTE;
      await service.closeIdleSessions();
      expect(service.openRuntimes()).toBe(1);
      const at = monotonicNow();
      monotonic.mockImplementation(() => at + 15 * MINUTE);
      await service.closeIdleSessions();
      expect(service.openRuntimes()).toBe(0);
    } finally {
      monotonic.mockRestore();
      wallClock.mockRestore();
    }
  });

  // The heap watch restarts the daemon at its ceiling, which makes every hook wait through a start; what the daemon keeps
  // for its checkouts' next sessions goes first.
  it("closes every runtime no session uses when asked, the one left last too, and none a session uses", async () => {
    const { service, first, second, start, end } = await twoCheckouts(() => false);
    await start("a", first);
    await start("b", second);
    await end("b", second);
    await service.closeUnusedRuntimes();
    expect(service.openRuntimes()).toBe(1);
    await end("a", first);
    await service.closeUnusedRuntimes();
    expect(service.openRuntimes()).toBe(0);
  });

  // A look for a removed folder still under way as the service closed closed the runtime the close had closed already.
  it("closes no runtime a second time when a look for its removed checkout answers after the service closed", async () => {
    const { service, first, start, end, clock } = await twoCheckouts(() => false);
    await start("a", first);
    await end("a", first);
    const looking = Promise.withResolvers<void>();
    const answer = Promise.withResolvers<void>();
    const answered = Promise.withResolvers<void>();
    heldLooks.set(first, { before: async () => {
      looking.resolve();
      await answer.promise;
    }, after: () => answered.resolve() });
    const disposed = vi.spyOn(RerailRuntime.prototype, "dispose");
    try {
      clock.now += 15 * MINUTE;
      const sweeping = service.closeIdleSessions();
      await looking.promise;
      services.splice(services.indexOf(service), 1);
      await service.close();
      await removeFolder(first);
      answer.resolve();
      await answered.promise;
      await sweeping;
      expect(disposed).toHaveBeenCalledTimes(1);
    } finally {
      heldLooks.clear();
      disposed.mockRestore();
    }
  });

  // Every other case answers for the daemon's memory; the daemon itself reads it (`memoryRoom`), and a daemon whose own
  // reading was lost would keep every runtime whatever its size.
  it("reads the daemon's own memory when no room check is given, and closes the kept runtimes it has no room for", async () => {
    const { service, first, second, start, end, clock } = await twoCheckouts(undefined);
    await start("a", first);
    await start("b", second);
    await end("a", first);
    await end("b", second);
    clock.now += 15 * MINUTE;
    // The daemon's resident size past a quarter of the machine's memory.
    const resident = vi.spyOn(process.memoryUsage, "rss").mockReturnValue(Number.MAX_SAFE_INTEGER);
    try {
      await service.closeIdleSessions();
      expect(service.openRuntimes()).toBe(0);
    } finally {
      resident.mockRestore();
    }
  });

  // Worktrees come and go: a runtime kept for a removed checkout would hold its walk until the daemon needed the room. It
  // goes at the time every runtime no session used went, and not sooner: a folder gone for a moment may come back.
  it("closes the runtime no session uses of a checkout whose folder was removed once it has been kept 15 minutes, though the daemon has room", async () => {
    const { service, first, second, start, end, clock } = await twoCheckouts(() => false);
    await start("a", first);
    await start("b", second);
    await end("b", second);
    await removeFolder(second);
    clock.now += 15 * MINUTE - 1;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(2);
    clock.now += 1;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
    await end("a", first);
    expect(service.openRuntimes()).toBe(1);
  });

  // A checkout moved and moved back, or on a share that dropped for a moment, read as removed at one sweep: its runtime
  // closed though every runtime no session used had stayed 15 minutes, and a session back there compiled its rules again.
  it("keeps the runtime of a checkout whose folder was gone for a moment and came back, past 15 minutes while the daemon has room", async () => {
    const { service, second, start, end, clock } = await twoCheckouts(() => false);
    await start("b", second);
    await end("b", second);
    const start0 = clock.now;
    await rename(second, `${second}-moved`);
    for (const minute of [5, 6]) {
      clock.now = start0 + minute * MINUTE;
      await service.closeIdleSessions();
    }
    await rename(`${second}-moved`, second);
    clock.now = start0 + 15 * MINUTE;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
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
    const { service, databasePath, first, start, end } = await twoCheckouts(() => true, { library: [{ kind: "code", rule }] });

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
    await service.closeUnusedRuntimes();
    expect(service.openRuntimes()).toBe(0);
    // Nothing in this test collects: the service does, once the runtime closed.
    await vi.waitFor(() => expect(compiled.deref()).toBeUndefined(), { timeout: 5_000 });
  });

  it("lets go of the engine's file-system watch though a runtime fails to close", async () => {
    const { service, first, start } = await twoCheckouts(() => false);
    await start("a", first);
    const dispose = vi.spyOn(RerailRuntime.prototype, "dispose").mockRejectedValueOnce(new Error("the disk went away"));
    const close = vi.spyOn(RepositoryWorktreeWalks.prototype, "close");
    try {
      services.splice(services.indexOf(service), 1);
      await expect(service.close()).rejects.toThrow("the disk went away");
      expect(close).toHaveBeenCalledTimes(1);
    } finally {
      // The runtime whose close failed still runs its session's Git in the checkout.
      const failed = [...dispose.mock.contexts] as RerailRuntime[];
      dispose.mockRestore();
      close.mockRestore();
      await Promise.all(failed.map(runtime => runtime.dispose()));
    }
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
 * it about (`looked`); `lookedAt(path)` settles once a walk has asked about `path` since `looked` was last cleared, at
 * once if one already has. Its scans (`scans`) complete with no finding once `auditing` settles, each recorded in the
 * audit store as a real scan is; `scanBegun` settles as the first begins. Every evaluator after the first opens once
 * `reopening` settles, as one compiling its rules does, and `reopened` settles as the second begins to. The first
 * evaluator to close does so once `closing` settles, and fails to with `closeFails`; `closeBegun` settles as it begins to.
 */
function recordingAuditor(options: { reopening?: Promise<void>; closing?: Promise<void>; closeFails?: boolean; auditing?: Promise<void> } = {}) {
  const batches: CodeRuleEditBatch[] = [];
  const scans: string[] = [];
  const looked = new Set<string>();
  // The test's waits for a walk's look, by path: each settles at that look, so no clock decides whether the walk came.
  const lookWaits = new Map<string, PromiseWithResolvers<void>>();
  const lookedAt = (path: string): Promise<void> => {
    if (looked.has(path)) return Promise.resolve();
    const wait = lookWaits.get(path) ?? Promise.withResolvers<void>();
    lookWaits.set(path, wait);
    return wait.promise;
  };
  const scanBegun = Promise.withResolvers<void>();
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
      lookWaits.get(path)?.resolve();
      lookWaits.delete(path);
      return true;
    },
    auditGeneratedEdits: async (_sessionId, batch) => {
      batches.push(batch);
      return { revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] };
    },
    audit: async (sessionId, input) => {
      scans.push(input.scope);
      scanBegun.resolve();
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
  return { create, batches, scans, looked, lookedAt, scanBegun: scanBegun.promise, reopened: reopened.promise, closeBegun: closeBegun.promise };
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
   * from Claude Code unless `host` says otherwise. With `noLock` the service can take no owner lock, as where the file
   * system refuses one: a file sits where the owner locks' folder goes.
   */
  async function idleService(
    options: Partial<Parameters<typeof startEngineService>[0]> = {},
    { gitFirst = false, host = "claude-code", noLock = false }: { gitFirst?: boolean; host?: "claude-code" | "omp"; noLock?: boolean } = {},
  ) {
    // Resolved: on macOS the run folder is under `/tmp`, a link, and the service records the paths it resolves.
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-idle-")));
    roots.push(root);
    const databasePath = join(root, "rerail.sqlite");
    const [first, second] = [join(root, "first"), join(root, "second")];
    for (const checkout of [first, second]) await mkdir(checkout);
    if (gitFirst) execFileSync("git", ["init", "--quiet"], { cwd: first });
    for (const checkout of [first, second]) startTestRepository(checkout, databasePath);
    if (noLock) {
      await mkdir(join(root, "locks"), { recursive: true });
      await writeFile(join(root, "locks", "owners"), "");
    }
    const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-idle-${process.pid}-${Date.now()}` : join(root, "engine.sock");
    const clock = { now: 1_000_000 };
    // The daemon has no room for a runtime no session uses, so the closed session's runtime goes with it.
    const needsRoom = options.needsRoom ?? (() => true);
    const engine = await startEngineService({
      socketPath,
      storageRoot: join(root, "state"),
      databasePath,
      embedder: testEmbedder,
      idleSessionMs: 60_000,
      now: () => clock.now,
      createAuditor: quietAuditor,
      ...options,
      needsRoom,
    });
    services.push(engine);
    // A sweep of a daemon that needs the room closes what no session uses, the runtime a suspended session left last too,
    // as the heap watch's relief does: the minute's sweep alone keeps the one left last for its checkout's next session.
    const service: EngineService = {
      ...engine,
      closeIdleSessions: async () => {
        await engine.closeIdleSessions();
        if (needsRoom()) await engine.closeUnusedRuntimes();
      },
    };
    const session = (sessionId: string, cwd = first) => ({ host, sessionId, cwd });
    const send = (event: AdapterEvent) => ingestEngineEvent({ socketPath }, event);
    return {
      first, second, clock, service, session, send, databasePath,
      /**
       * The engine service of the next daemon, started on the same records with `now` as its clock, as one starts after
       * this one died.
       */
      nextService: async (now: () => number) => {
        const next = await startEngineService({
          socketPath: process.platform === "win32" ? `${socketPath}-next` : join(root, "next.sock"),
          storageRoot: join(root, "state"), databasePath, embedder: testEmbedder, idleSessionMs: 60_000, needsRoom: () => true, now,
          createAuditor: quietAuditor,
        });
        services.push(next);
        return next;
      },
      /** When `sessionId` was last heard from: the time its newest recorded event was observed. */
      heardAt: (sessionId: string) => lastObservedAt(databasePath, sessionId),
      /** Makes the owner row of `sessionId` name process `pid`, as the runtime of that process left it. */
      ownedBy: (sessionId: string, pid: number) => setOwnerProcess(databasePath, sessionId, pid),
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

  // Ended at the stop, the record said the session ran for the days between its last event and the daemon's stop.
  it("ends a suspended session's record when the service stops, at the time it went quiet, as stopping ends every open session", async () => {
    const { clock, service, start, ended, live } = await idleService();
    await start("a");
    clock.now += 60_000;
    const quietAt = clock.now;
    await service.closeIdleSessions();
    expect(ended("a")).toBeNull();
    clock.now += 3 * 24 * 60 * 60_000;
    services.splice(services.indexOf(service), 1);
    await service.close();
    expect(ended("a")).toBe(quietAt);
    expect(live("a")).toBe(false);
  });

  // A daemon that ends without its stop (a machine restart, the heap limit, a kill) ends none of the sessions it had
  // suspended, and no daemon after it holds them: each record read as open for good. The idle close of a daemon still
  // running ends each at the first sweep past the idle limit after its last event.
  it("ends a quiet session's record at the next daemon's start when its daemon died, at the idle limit after its last event", async () => {
    const { clock, service, start, prompt, ended, live, heardAt, ownedBy, nextService } = await idleService();
    await start("a");
    await prompt("a", "last");
    const answeredAt = Date.now();
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(ended("a")).toBeNull();
    // Nothing is recorded as the session goes quiet: it was last heard from at its last event.
    const lastHeard = heardAt("a");
    expect(lastHeard).toBeLessThanOrEqual(answeredAt);
    // The daemon dies: its stop never runs, and the process its owner row names is gone.
    ownedBy("a", goneProcessId());
    await nextService(() => lastHeard + 3 * 24 * 60 * 60_000);
    expect(ended("a")).toBe(lastHeard + 60_000);
    expect(live("a")).toBe(false);
  });

  // Main leaves the same gap for a session at work when its daemon dies. Ended at the next start, its record ends no
  // later than that start; a session this daemon's process owns, or one whose owner still runs, is left as it is.
  it("ends a session its daemon held open when it died at the next daemon's start at the latest, and none whose owner runs", async () => {
    const { second, clock, service, start, ended, live, ownedBy, nextService } = await idleService();
    await start("quiet-here", second);
    clock.now += 60_000;
    await start("at-work");
    await start("other-daemon");
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
    ownedBy("at-work", goneProcessId());
    ownedBy("other-daemon", process.ppid);
    const startedFrom = Date.now();
    await nextService(Date.now);
    const startedBy = Date.now();
    expect(ended("at-work")).toEqual(expect.any(Number));
    expect(ended("at-work")).toBeGreaterThanOrEqual(startedFrom);
    expect(ended("at-work")).toBeLessThanOrEqual(startedBy);
    expect([ended("quiet-here"), live("quiet-here")]).toEqual([null, true]);
    expect([ended("other-daemon"), live("other-daemon")]).toEqual([null, true]);
  });

  // The session's next event opens it and takes its suspension back. A suspension left in the service would end the
  // record again at the stop, at the time the session first went quiet, though the session was at work after it.
  it("ends a session that opened again after the quiet at the service's stop, not at the time it went quiet", async () => {
    const { clock, service, start, prompt, ended } = await idleService();
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    await prompt("a", "back");
    services.splice(services.indexOf(service), 1);
    const stoppedAt = Date.now();
    await service.close();
    expect(ended("a")).toBeGreaterThanOrEqual(stoppedAt);
  });

  // The open session's control timer can claim a queued scan while the idle close reads where the session's walks
  // stand. A close that went on then would cut the scan off: "Session closed during scan".
  it("keeps a quiet session open when it takes a scan while the idle close reads its walks, and runs the scan", async () => {
    const auditing = Promise.withResolvers<void>();
    const { create, scans, lookedAt } = recordingAuditor({ auditing: auditing.promise });
    const { first, clock, service, start, ended, databasePath } = await idleService({ createAuditor: create });
    await writeFile(join(first, "app.py"), "x = 1\n");
    await start("a");
    // The evaluator is open, so the idle close reads where the walks stand.
    await lookedAt("app.py");
    const reading = holdWalkResume();
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      clock.now += 60_000;
      const sweeping = service.closeIdleSessions();
      await reading.entered;
      const request = (await controls.requestAudit("a", { scope: "repository" }));
      await vi.waitFor(() => expect(scans).toEqual(["repository"]), { timeout: 10_000, interval: 20 });
      reading.release();
      // The scan runs on until the sweep is done: a close that waited for it would wait here.
      await Promise.race([sweeping, delay(2_000)]);
      auditing.resolve();
      await sweeping;
      expect(service.openRuntimes()).toBe(1);
      await vi.waitFor(() => {
        expect(controls.listAuditRequests("a").find(item => item.id === request.id)).toMatchObject({ status: "completed", error: null });
      }, { timeout: 10_000, interval: 50 });
      expect(ended("a")).toBeNull();
    } finally {
      reading.release();
      reading.restore();
      auditing.resolve();
      controls.close();
    }
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
      const request = (await controls.requestAudit("a", { scope: "repository" }));
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

  // Before, each write of the open waited for another process's write lock up to 5 s on the daemon's thread, then the
  // failed open's scans waited 5 s more to fail: health, the dashboard and every hook waited 10 s with them.
  it.each([
    ["closed", true],
    ["still open", false],
  ])("opens a quiet session for a scan only once another process's write lock is free, its runtime %s, without waiting for it", async (_, short) => {
    const { create, scans } = recordingAuditor();
    const { clock, service, start, databasePath, ended } = await idleService({ createAuditor: create, needsRoom: () => short });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    const controls = new SqliteAuditControlStore(databasePath);
    const writer = new Database(databasePath);
    try {
      const request = (await controls.requestAudit("a", { scope: "repository" }));
      writer.exec("BEGIN IMMEDIATE");
      const asked = performance.now();
      await service.resumeSession("a");
      expect(performance.now() - asked).toBeLessThan(1_000);
      await delay(500);
      expect(controls.listAuditRequests("a").find(item => item.id === request.id)).toMatchObject({ status: "queued" });
      expect(ended("a")).toBeNull();
      writer.exec("ROLLBACK");
      await vi.waitFor(() => {
        expect(controls.listAuditRequests("a").find(item => item.id === request.id)).toMatchObject({ status: "completed", error: null });
      }, { timeout: 10_000, interval: 50 });
      expect(scans).toEqual(["repository"]);
    } finally {
      if (writer.inTransaction) writer.exec("ROLLBACK");
      writer.close();
      controls.close();
    }
  }, 30_000);

  // Before, the open was tried again on a 250 ms timer, each try opening a writer, for as long as the lock was held. The
  // open meets the lock before it reads the session's record, which would first wait for this process's waiting writes.
  it("opens a quiet session for a scan when another process gives back the write lock, with no timer trying it again", async () => {
    const { create, scans } = recordingAuditor();
    const { clock, service, start, databasePath } = await idleService({ createAuditor: create });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    const controls = new SqliteAuditControlStore(databasePath);
    const sessions = new SqliteSessionStore(databasePath);
    const writer = new Database(databasePath);
    try {
      const request = (await controls.requestAudit("a", { scope: "repository" }));
      const finished = requestFinished(request.id);
      writer.exec("BEGIN IMMEDIATE");
      // Another session's record waits for the lock in this process.
      sessions.openSession({ sessionId: "other", host: "claude-code", cwd: databasePath }, 1_000);
      vi.useFakeTimers({ toFake: ["setTimeout", "setInterval"] });
      const resumed = process.hrtime.bigint();
      await service.resumeSession("a");
      expect(Number(process.hrtime.bigint() - resumed) / 1e6).toBeLessThan(1_000);
      expect(vi.getTimerCount()).toBe(0);
      vi.useRealTimers();
      writer.exec("ROLLBACK");
      await finished;
      expect(controls.listAuditRequests("a").find(item => item.id === request.id)).toMatchObject({ status: "completed", error: null });
      expect(scans).toEqual(["repository"]);
    } finally {
      vi.restoreAllMocks();
      vi.useRealTimers();
      if (writer.inTransaction) writer.exec("ROLLBACK");
      writer.close();
      sessions.close();
      controls.close();
    }
  }, 30_000);

  // In a runtime still open the service does not try the lock first: the runtime's open meets it before reading the
  // session's record, which would first wait, on the daemon's thread, for this process's writes waiting for the lock.
  it("tries a quiet session's open for a scan in a runtime still open without waiting for the held lock", async () => {
    const { create, scans } = recordingAuditor();
    const { clock, service, start, databasePath } = await idleService({ createAuditor: create, needsRoom: () => false });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    const controls = new SqliteAuditControlStore(databasePath);
    const sessions = new SqliteSessionStore(databasePath);
    const writer = new Database(databasePath);
    try {
      const request = (await controls.requestAudit("a", { scope: "repository" }));
      const finished = requestFinished(request.id);
      writer.exec("BEGIN IMMEDIATE");
      sessions.openSession({ sessionId: "other", host: "claude-code", cwd: databasePath }, 1_000);
      const resumed = process.hrtime.bigint();
      await service.resumeSession("a");
      expect(Number(process.hrtime.bigint() - resumed) / 1e6).toBeLessThan(1_000);
      writer.exec("ROLLBACK");
      await finished;
      expect(controls.listAuditRequests("a").find(item => item.id === request.id)).toMatchObject({ status: "completed", error: null });
      expect(scans).toEqual(["repository"]);
    } finally {
      vi.restoreAllMocks();
      if (writer.inTransaction) writer.exec("ROLLBACK");
      writer.close();
      sessions.close();
      controls.close();
    }
  }, 30_000);

  // Before, the open read the session's record only once it had made this process's waiting record writes, waiting on the
  // daemon's thread until the oldest was 5 s old, then met the lock again where it records the checkout, which waited
  // 5 s more and failed the open. It waits for neither now: the checkout's record, the session's row and its claim wait
  // in this process's record writer, behind the other session's row, until the lock is given back.
  it("opens a session at once while another process holds the write lock, behind another session's waiting records", async () => {
    const { start, turnEnd, live, databasePath } = await idleService({ needsRoom: () => false });
    await start("a");
    // The first session's walk and checks, which run on after its start, end before the lock is taken: a write of theirs
    // would wait for it on the same thread.
    await turnEnd("a", "turn");
    const sessions = new SqliteSessionStore(databasePath);
    const writer = new Database(databasePath);
    try {
      writer.exec("BEGIN IMMEDIATE");
      // Another session's record waits for the lock in this process.
      sessions.openSession({ sessionId: "other", host: "claude-code", cwd: databasePath }, 1_000);
      const asked = performance.now();
      await start("b");
      // This thread holds the lock's holder too: any wait for the lock here would last the 5 s busy timeout.
      expect(performance.now() - asked).toBeLessThan(5_000);
      writer.exec("ROLLBACK");
      // Once the lock is given back the session's record and claim are made, when the writer's lock wait finds it free,
      // which another connection learns only from the database.
      await vi.waitFor(() => expect(live("b")).toBe(true), { timeout: 10_000, interval: 20 });
    } finally {
      if (writer.inTransaction) writer.exec("ROLLBACK");
      writer.close();
      sessions.close();
    }
  }, 30_000);

  // Before, the open read the claim the suspension kept only from the database, which did not have it yet, and asked
  // for a second one behind it: the first was made once the lock was free and the second refused, so the runtime held a
  // claim the database lacked, never ran the session's scans, and refused every later open of the session.
  it("keeps the claim of a session suspended and opened again while the claim waits for the write lock, and runs its scans", async () => {
    const { create, scans } = recordingAuditor();
    const { clock, service, start, prompt, turnEnd, live, databasePath } = await idleService({ createAuditor: create, needsRoom: () => false });
    await start("a");
    await turnEnd("a", "turn");
    const controls = new SqliteAuditControlStore(databasePath);
    const writer = new Database(databasePath);
    const lockWait = Promise.withResolvers<void>();
    const scanned = async (): Promise<void> => {
      const request = await controls.requestAudit("b", { scope: "repository" });
      await vi.waitFor(() => {
        expect(controls.listAuditRequests("b").find(item => item.id === request.id)).toMatchObject({ status: "completed", error: null });
      }, { timeout: 10_000, interval: 50 });
    };
    try {
      writer.exec("BEGIN IMMEDIATE");
      lockWaitGate.held = lockWait.promise;
      await start("b");
      // The lock is free, and the record writer's wait has not seen it: the session's claim still waits.
      writer.exec("ROLLBACK");
      clock.now += 60_000;
      await service.closeIdleSessions();
      await prompt("b", "back");
      lockWaitGate.held = undefined;
      lockWait.resolve();
      await vi.waitFor(() => expect(live("b")).toBe(true), { timeout: 10_000, interval: 20 });
      await scanned();
      clock.now += 60_000;
      await service.closeIdleSessions();
      await prompt("b", "again");
      await scanned();
      expect(scans).toEqual(["repository", "repository"]);
    } finally {
      lockWaitGate.held = undefined;
      lockWait.resolve();
      if (writer.inTransaction) writer.exec("ROLLBACK");
      writer.close();
      controls.close();
    }
  }, 30_000);

  // Before, the suspension kept the claim its session asked for while another process held the write lock, and the
  // claim, refused once the lock was given back, stayed on it: the daemon's stop interrupted the owner's scans.
  it("leaves the owner's scans queued as it ends a session suspended while its claim waited, once that claim was refused", async () => {
    const { clock, service, start, turnEnd, databasePath } = await idleService({ needsRoom: () => false });
    await start("a");
    await turnEnd("a", "turn");
    const controls = new SqliteAuditControlStore(databasePath);
    const writer = new Database(databasePath);
    const lockWait = Promise.withResolvers<void>();
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      writer.exec("BEGIN IMMEDIATE");
      lockWaitGate.held = lockWait.promise;
      await start("b");
      claimFirst(writer, "b");
      writer.exec("COMMIT");
      // The lock is free, and the record writer's wait has not seen it: the session is suspended while its claim waits.
      clock.now += 60_000;
      await service.closeIdleSessions();
      lockWaitGate.held = undefined;
      lockWait.resolve();
      await vi.waitFor(() => expect(stderr.mock.calls.map(([text]) => String(text)))
        .toContain("rerail: the claim on session b was not made, so this process does not own it\n"), { timeout: 10_000, interval: 20 });
      const queued = await controls.requestAudit("b", { scope: "repository" });
      services.splice(services.indexOf(service), 1);
      await service.close();
      expect(controls.listAuditRequests("b").find(request => request.id === queued.id)?.status).toBe("queued");
    } finally {
      stderr.mockRestore();
      lockWaitGate.held = undefined;
      lockWait.resolve();
      if (writer.inTransaction) writer.exec("ROLLBACK");
      writer.close();
      controls.close();
    }
  }, 30_000);

  // Before, the runtime made again for a session suspended while its claim waited took that claim, and only the runtime
  // that asked for it was told once it was refused: the session's end interrupted the owner's scans.
  it("leaves the owner's scans queued as a session opened again in a new runtime ends, once the claim it took was refused", async () => {
    const { clock, service, start, prompt, turnEnd, send, session, databasePath } = await idleService();
    await start("a");
    await turnEnd("a", "turn");
    const controls = new SqliteAuditControlStore(databasePath);
    const writer = new Database(databasePath);
    // Another runtime's store over the database, whose use of this process's record writer keeps the claim waiting as
    // `b`'s runtime closes.
    const records = new SqliteSessionStore(databasePath);
    const lockWait = Promise.withResolvers<void>();
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      writer.exec("BEGIN IMMEDIATE");
      lockWaitGate.held = lockWait.promise;
      await start("b");
      claimFirst(writer, "b");
      writer.exec("COMMIT");
      clock.now += 60_000;
      await service.closeIdleSessions();
      // `b`'s runtime closed with `a` and `b` suspended: `b`'s next event makes it again, and it takes the waiting claim.
      expect(service.openRuntimes()).toBe(0);
      await prompt("b", "back");
      lockWaitGate.held = undefined;
      lockWait.resolve();
      await vi.waitFor(() => expect(stderr.mock.calls.map(([text]) => String(text)))
        .toContain("rerail: the claim on session b was not made, so this process does not own it\n"), { timeout: 10_000, interval: 20 });
      const queued = await controls.requestAudit("b", { scope: "repository" });
      await send({ kind: "session-end", eventId: "b:end", sessionId: "b", session: session("b") });
      expect(controls.listAuditRequests("b").find(request => request.id === queued.id)?.status).toBe("queued");
    } finally {
      stderr.mockRestore();
      lockWaitGate.held = undefined;
      lockWait.resolve();
      if (writer.inTransaction) writer.exec("ROLLBACK");
      writer.close();
      records.close();
      controls.close();
    }
  }, 30_000);

  // Before, the end interrupted the session's checkout audit and its scans on the audit controls' own connection, which
  // waited on the daemon's thread for the lock up to 5 s, every other session's hooks waiting too, then failed: the end
  // answered the lock's error, and the session's record stayed open, owned by a runtime that had let it go.
  it("ends a session at once while another process holds the write lock, and its record and owner once the lock is given back", async () => {
    const { start, turnEnd, send, session, ended, live, databasePath } = await idleService({ needsRoom: () => false });
    await start("a");
    await start("b");
    // The sessions' walks and checks, which run on after their starts, end before the lock is taken: a write of theirs
    // would wait for it on the same thread.
    await turnEnd("a", "turn");
    await turnEnd("b", "turn");
    const writer = new Database(databasePath);
    try {
      writer.exec("BEGIN IMMEDIATE");
      const asked = performance.now();
      await send({ kind: "session-end", eventId: "a:end", sessionId: "a", session: session("a") });
      // This thread holds the lock's holder too: any wait for the lock here would last the 5 s busy timeout, then fail.
      expect(performance.now() - asked).toBeLessThan(5_000);
      expect(ended("a")).toBeNull();
      writer.exec("ROLLBACK");
      // The end's writes wait in this process's record writer until its lock wait finds the lock given back, which
      // another connection learns only from the database.
      await vi.waitFor(() => {
        expect(ended("a")).not.toBeNull();
        expect(live("a")).toBe(false);
      }, { timeout: 10_000, interval: 20 });
      expect(live("b")).toBe(true);
    } finally {
      if (writer.inTransaction) writer.exec("ROLLBACK");
      writer.close();
    }
  }, 30_000);

  // Before, a session that ended while its claim still waited for the lock interrupted its scans on the audit controls'
  // own connection, at once once the lock was free: ahead of the claim, which was then refused, so the end interrupted
  // the scans of the runtime that owns the session.
  it("leaves the owner's scans queued as a session ends while its claim waits for the write lock, once that claim is refused", async () => {
    const { start, turnEnd, send, session, databasePath } = await idleService({ needsRoom: () => false });
    await start("a");
    await turnEnd("a", "turn");
    const controls = new SqliteAuditControlStore(databasePath);
    const writer = new Database(databasePath);
    const lockWait = Promise.withResolvers<void>();
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      writer.exec("BEGIN IMMEDIATE");
      lockWaitGate.held = lockWait.promise;
      await start("b");
      claimFirst(writer, "b");
      writer.prepare("INSERT INTO audit_requests (id, session_id, workspace_root, scope, mode, paths_json, status, created_at, audit_ids_json) VALUES ('owner-scan', 'b', ?, 'repository', 'log', '[]', 'queued', 0, '[]')")
        .run(dirname(databasePath));
      writer.exec("COMMIT");
      // The lock is free, and the record writer's wait has not seen it: `b`'s claim still waits as its end comes.
      await send({ kind: "session-end", eventId: "b:end", sessionId: "b", session: session("b") });
      lockWaitGate.held = undefined;
      lockWait.resolve();
      await vi.waitFor(() => expect(stderr.mock.calls.map(([text]) => String(text)))
        .toContain("rerail: the claim on session b was not made, so this process does not own it\n"), { timeout: 10_000, interval: 20 });
      expect(controls.listAuditRequests("b").find(request => request.id === "owner-scan")?.status).toBe("queued");
    } finally {
      stderr.mockRestore();
      lockWaitGate.held = undefined;
      lockWait.resolve();
      if (writer.inTransaction) writer.exec("ROLLBACK");
      writer.close();
      controls.close();
    }
  }, 30_000);

  // Before, the wait went on, a worker thread with its own connection watching the database's log, until the lock was
  // given back, though nothing was left for it to open. An event of the session cannot open it while the lock is held.
  it("stops a quiet session's wait for the write lock its open for a scan met once the session's end comes", async () => {
    const { clock, service, start, send, session, databasePath } = await idleService({ needsRoom: () => false });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    const controls = new SqliteAuditControlStore(databasePath);
    const writer = new Database(databasePath);
    try {
      await controls.requestAudit("a", { scope: "repository" });
      writer.exec("BEGIN IMMEDIATE");
      lockWaits.length = 0;
      await service.resumeSession("a");
      const [wait] = lockWaits.filter(begun => begun.resume);
      expect(wait?.signal?.aborted).toBe(false);
      // Its runtime cannot record the end while the lock is held; the service ends the suspended session's record anyway.
      await send({ kind: "session-end", eventId: "a:end", sessionId: "a", session: session("a") }).catch(() => undefined);
      expect(wait!.signal!.aborted).toBe(true);
    } finally {
      if (writer.inTransaction) writer.exec("ROLLBACK");
      writer.close();
      controls.close();
    }
  }, 30_000);

  it("runs a scan queued for a quiet session once its next event opens it, as the session's owner", async () => {
    const { create, scans } = recordingAuditor();
    const { clock, service, start, prompt, databasePath } = await idleService({ createAuditor: create });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      // Queued by a writer that does not open the session (a second connection, as `rerail scan` once wrote).
      const request = (await controls.requestAudit("a", { scope: "repository" }));
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

  // At base the idle close ended the session, so a scan asked for it once Rerail was stopped in its checkout failed at
  // once. Opened again for the scan, it recorded nothing there: the scan failed with "completed scan without a recorded
  // run", and the session stayed open and owned until the daemon stopped.
  it("ends a quiet session whose repository was stopped since, rather than open it again for a scan, and fails the scan", async () => {
    const { create, scans } = recordingAuditor();
    const { first, clock, service, start, databasePath, ended, live } = await idleService({ createAuditor: create });
    await start("a");
    clock.now += 60_000;
    const quietAt = clock.now;
    await service.closeIdleSessions();
    stopTestRepository(first, databasePath);
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      const request = (await controls.requestAudit("a", { scope: "repository" }));
      expect(request).toMatchObject({ status: "queued", error: null });
      clock.now += 60_000;
      await service.resumeSession("a");
      expect(controls.listAuditRequests("a").find(item => item.id === request.id)).toMatchObject({
        status: "failed", error: expect.stringContaining(`Rerail is stopped in ${first}`),
      });
      expect(ended("a")).toBe(quietAt);
      expect(live("a")).toBe(false);
      expect(scans).toEqual([]);
    } finally {
      controls.close();
    }
  });

  // At base the idle close ended a quiet session. Suspended instead, one whose repository was then stopped, or whose
  // folder was removed, got no event and could run no scan, yet read as live and owned until the daemon stopped.
  it.each([
    ["Rerail is stopped in its checkout", (first: string, databasePath: string) => stopTestRepository(first, databasePath)],
    ["its folder is removed", (first: string) => removeFolder(first)],
  ])("ends a quiet session at the next sweep once %s, at the time it went quiet", async (_, unopen) => {
    const { first, clock, service, start, databasePath, ended, live } = await idleService();
    await start("a");
    clock.now += 60_000;
    const quietAt = clock.now;
    await service.closeIdleSessions();
    expect(live("a")).toBe(true);
    // The runtime's close still reads the checkout a moment after the sweep.
    await vi.waitFor(() => expect(service.openRuntimes()).toBe(0), { timeout: 5_000, interval: 50 });
    await unopen(first, databasePath);
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(ended("a")).toBe(quietAt);
    expect(live("a")).toBe(false);
  });

  // At base the idle close ended the session, so a scan asked for it once its checkout was removed failed at once, and
  // the folder stayed gone. Opening the session again to run the scan made the folder anew with Rerail's records.
  it("ends a quiet session whose checkout was removed, rather than open it again for a scan, and fails the scan", async () => {
    const { create, scans } = recordingAuditor();
    // Its runtime outlives the session, as in a daemon with room, so it is still there when the scan is asked for.
    const { first, clock, service, start, databasePath, ended } = await idleService({ createAuditor: create, needsRoom: () => false });
    await start("a");
    clock.now += 60_000;
    const quietAt = clock.now;
    await service.closeIdleSessions();
    await removeFolder(first);
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      const request = (await controls.requestAudit("a", { scope: "repository" }));
      expect(request).toMatchObject({ status: "queued", error: null });
      clock.now += 60_000;
      await service.resumeSession("a");
      expect(existsSync(first), "the removed checkout").toBe(false);
      expect(controls.listAuditRequests("a").find(item => item.id === request.id)).toMatchObject({
        status: "failed", error: expect.stringContaining(`its folder ${first} was removed`),
      });
      expect(ended("a")).toBe(quietAt);
      expect(scans).toEqual([]);
      // The runtime the scan's ask kept is unused, and the minute's sweep closes it once it has been kept 15 minutes, as
      // every runtime no session used was, and its look finds the checkout gone.
      expect(service.openRuntimes()).toBe(1);
      await service.closeIdleSessions();
      expect(service.openRuntimes()).toBe(1);
      clock.now += 15 * MINUTE;
      await service.closeIdleSessions();
      expect(service.openRuntimes()).toBe(0);
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
        const request = (await controls.requestAudit("a", { scope: "repository" }));
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
    const { create, scans, lookedAt, closeBegun } = recordingAuditor({ closing: closing.promise });
    const { first, clock, service, start, databasePath } = await idleService({ createAuditor: create });
    await writeFile(join(first, "app.py"), "x = 1\n");
    await start("a");
    // The evaluator is open, so the sweep's close waits for it.
    await lookedAt("app.py");
    clock.now += 60_000;
    const sweeping = service.closeIdleSessions();
    await closeBegun;
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      const request = (await controls.requestAudit("a", { scope: "repository" }));
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

  // The close's release kept the session's runtime as one no session uses, and the open for the scan then used it
  // without taking it out: once the daemon needed the room, the next room check closed it under the reopened session.
  it("closes no runtime a scan opened its session in again during the session's close, once the daemon needs the room", async () => {
    const closing = Promise.withResolvers<void>();
    const room = { short: false };
    const { create, lookedAt, closeBegun, scans } = recordingAuditor({ closing: closing.promise });
    const { first, second, clock, service, start, databasePath } = await idleService({ createAuditor: create, needsRoom: () => room.short });
    await writeFile(join(first, "app.py"), "x = 1\n");
    await start("a");
    await lookedAt("app.py");
    clock.now += 60_000;
    const sweeping = service.closeIdleSessions();
    await closeBegun;
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      const request = (await controls.requestAudit("a", { scope: "repository" }));
      const resuming = service.resumeSession("a");
      closing.resolve();
      await Promise.all([sweeping, resuming]);
      room.short = true;
      await service.closeUnusedRuntimes();
      expect(service.openRuntimes()).toBe(1);
      // A runtime opening in another checkout closes nothing.
      await start("b", second);
      expect(service.openRuntimes()).toBe(2);
      await vi.waitFor(() => {
        expect(controls.listAuditRequests("a").find(item => item.id === request.id)).toMatchObject({ status: "completed", error: null });
      }, { timeout: 10_000, interval: 50 });
      expect(scans).toEqual(["repository"]);
    } finally {
      controls.close();
    }
  });

  // An open a scan asks for can fail (the session's folder gone, its evaluator failing). The scan stayed queued, with a
  // line in the daemon's log only, until the session's next event or the daemon's stop.
  it("fails a scan asked for a quiet session it cannot open again, saying why", async () => {
    let made = 0;
    const { clock, service, start, databasePath } = await idleService({
      createAuditor: () => {
        made += 1;
        if (made > 1) throw new Error("evaluator unavailable");
        return quietAuditor();
      },
    });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      const request = (await controls.requestAudit("a", { scope: "repository" }));
      expect(request).toMatchObject({ status: "queued", error: null });
      await service.resumeSession("a");
      expect(made).toBe(2);
      expect(controls.listAuditRequests("a").find(item => item.id === request.id))
        .toMatchObject({ status: "failed", error: expect.stringMatching(/evaluator unavailable/u) });
    } finally {
      controls.close();
    }
  });

  // The end opens the suspended session to close it. A scan asked for meanwhile found the session live, so the API asked
  // the service to open it again; that open waited behind the end, then opened the ended session as if it were quiet:
  // its record read as live and owned until the daemon stopped, and no end would come for it.
  it("leaves a quiet session ended when a scan is asked for it while its end opens it", async () => {
    const hold = { on: false, entered: Promise.withResolvers<void>(), released: Promise.withResolvers<void>() };
    const { clock, service, session, send, start, ended, live, databasePath } = await idleService({
      beforeSessionPublish: async () => {
        if (!hold.on) return;
        hold.on = false;
        hold.entered.resolve();
        await hold.released.promise;
      },
    });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    hold.on = true;
    const ending = send({ kind: "session-end", eventId: "a:end", sessionId: "a", session: session("a") });
    await hold.entered.promise;
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      expect((await controls.requestAudit("a", { scope: "repository" }))).toMatchObject({ status: "queued", error: null });
      const resuming = service.resumeSession("a");
      hold.released.resolve();
      await Promise.all([ending, resuming]);
    } finally {
      controls.close();
    }
    expect(ended("a")).not.toBeNull();
    expect(live("a")).toBe(false);
  });

  it("ends a quiet session's record when its end comes, also from a repository stopped since", async () => {
    const { first, second, clock, service, session, send, start, ended, databasePath } = await idleService();
    await start("a");
    await start("b", second);
    clock.now += 60_000;
    await service.closeIdleSessions();
    // `a`'s end opens it again and closes it; `b`'s runtime records nothing once its repository stopped.
    stopTestRepository(second, databasePath);
    const endsSentAt = Date.now();
    await send({ kind: "session-end", eventId: "a:end", sessionId: "a", session: session("a", first) });
    await send({ kind: "session-end", eventId: "b:end", sessionId: "b", session: session("b", second) });
    // Each at the time its end came, not at the time it went quiet.
    expect(ended("a")).toBeGreaterThanOrEqual(endsSentAt);
    expect(ended("b")).toBeGreaterThanOrEqual(endsSentAt);
  });

  // The end opens the suspended session again to close it. An open that failed lost the end: the record stayed open and
  // owned, read as live until the daemon stopped, and the runtime made for the end stayed open as long.
  it("ends a quiet session's record when its end comes and it cannot be opened again, and the runtime made for it goes when the daemon needs the room", async () => {
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
    // The runtime made for the end is kept as any runtime no session uses is, until the daemon needs the room.
    expect(service.openRuntimes()).toBe(1);
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(0);
  });

  // A quiet session's end opens it again in its checkout's runtime to close it, and notes no use of the runtime. The
  // end's hold on the runtime ended as its work began, so the runtime read as kept while the end still ran in it, and a
  // relief closed it under the end.
  it("writes a quiet session's end and closes nothing under it while the daemon is short of room", async () => {
    const hold = { on: false, entered: Promise.withResolvers<void>(), released: Promise.withResolvers<void>() };
    const { clock, service, session, send, start, ended, live } = await idleService({
      beforeSessionPublish: async () => {
        if (!hold.on) return;
        hold.on = false;
        hold.entered.resolve();
        await hold.released.promise;
      },
    });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(0);
    const disposed = vi.spyOn(RerailRuntime.prototype, "dispose");
    try {
      hold.on = true;
      const ending = send({ kind: "session-end", eventId: "a:end", sessionId: "a", session: session("a") });
      await hold.entered.promise;
      // The heap watch's relief while the end opens the session: it closes every runtime no session uses.
      const relief = service.closeUnusedRuntimes();
      expect(disposed).not.toHaveBeenCalled();
      hold.released.resolve();
      await Promise.all([ending, relief]);
      expect(disposed).not.toHaveBeenCalled();
      expect(ended("a")).not.toBeNull();
      expect(live("a")).toBe(false);
      // Once the end's work has ended, its runtime is kept as any runtime no session uses.
      expect(service.openRuntimes()).toBe(1);
      await service.closeUnusedRuntimes();
      expect(service.openRuntimes()).toBe(0);
    } finally {
      disposed.mockRestore();
    }
  });

  // The end of a quiet session whose runtime was kept, as the daemon had room at the idle close, comes once the daemon is
  // short of room. The end notes no use of the runtime: only its claim keeps the heap watch's relief from closing the
  // runtime under it, and without it the end was lost ("The database connection is not open").
  it("writes the end of a quiet session whose runtime was kept, and closes nothing under it once the daemon is short of room", async () => {
    const hold = { on: false, entered: Promise.withResolvers<void>(), released: Promise.withResolvers<void>() };
    const room = { short: false };
    const { clock, service, session, send, start, ended } = await idleService({
      needsRoom: () => room.short,
      beforeSessionPublish: async () => {
        if (!hold.on) return;
        hold.on = false;
        hold.entered.resolve();
        await hold.released.promise;
      },
    });
    await start("a");
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
    room.short = true;
    const disposed = vi.spyOn(RerailRuntime.prototype, "dispose");
    try {
      hold.on = true;
      const ending = send({ kind: "session-end", eventId: "a:end", sessionId: "a", session: session("a") });
      await hold.entered.promise;
      const relief = service.closeUnusedRuntimes();
      const disposedDuringEnd = disposed.mock.calls.length;
      hold.released.resolve();
      await Promise.all([ending, relief]);
      expect({ disposedDuringEnd, ended: ended("a") !== null }).toEqual({ disposedDuringEnd: 0, ended: true });
    } finally {
      hold.released.resolve();
      disposed.mockRestore();
    }
  });

  // A suspension whose close failed was kept by nobody: its next event could not open it, its owner still active, and
  // its record stayed open and owned after the daemon stopped.
  it("keeps a quiet session suspended when its close fails, so its next event opens it again", async () => {
    const { create, lookedAt } = recordingAuditor({ closeFails: true });
    const { first, clock, service, start, prompt, ended, live } = await idleService({ createAuditor: create });
    await writeFile(join(first, "app.py"), "x = 1\n");
    await start("a");
    // The evaluator is open, so the sweep's close waits for it, and fails.
    await lookedAt("app.py");
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(ended("a")).toBeNull();
    expect(live("a")).toBe(true);
    await prompt("a", "back");
    expect(live("a")).toBe(true);
  });

  it("ends a quiet session whose close failed when the service stops", async () => {
    const { create, lookedAt } = recordingAuditor({ closeFails: true });
    const { first, clock, service, start, ended, live } = await idleService({ createAuditor: create });
    await writeFile(join(first, "app.py"), "x = 1\n");
    await start("a");
    await lookedAt("app.py");
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(ended("a")).toBeNull();
    services.splice(services.indexOf(service), 1);
    await service.close();
    expect(ended("a")).not.toBeNull();
    expect(live("a")).toBe(false);
  });

  // The service's stop ended its quiet sessions one after another and stopped at the first that failed (a write of its
  // end that failed as it was made): each one after it stayed open and owned, with nothing left to end it.
  it("ends every other quiet session's record when the service stops and ending one fails", async () => {
    const { second, clock, service, start, ended } = await idleService();
    await start("a");
    await start("b", second);
    clock.now += 60_000;
    await service.closeIdleSessions();
    const ending = vi.spyOn(SqliteAuditControlStore.prototype, "endOwnership").mockImplementationOnce(() => {
      throw new Error("database is locked");
    });
    services.splice(services.indexOf(service), 1);
    try {
      await service.close();
    } finally {
      ending.mockRestore();
    }
    // Which of the two ends first depends on which close finished first.
    expect([ended("a"), ended("b")].filter(at => at !== null)).toHaveLength(1);
  });

  // At base an idle minute opened no database. The sweep that ends what a dead daemon left open then opened a writer
  // every minute, which waits for another process's write lock (`busy_timeout`, 5 s) on the daemon's main thread:
  // health, the dashboard and every hook waited with it.
  it("opens no database in a minute's sweep while no daemon has died, so another process's write lock holds nothing up", async () => {
    const { service, start, databasePath } = await idleService();
    await start("a");
    const writer = new Database(databasePath);
    writer.exec("BEGIN IMMEDIATE");
    try {
      const swept = performance.now();
      await service.closeIdleSessions();
      // The wait it made was the busy timeout, 5 s.
      expect(performance.now() - swept).toBeLessThan(3_000);
    } finally {
      writer.exec("ROLLBACK");
      writer.close();
    }
  }, 20_000);

  // At base the quiet session's next walk found another writer's change to a file the session had edited and recorded
  // it in Log; after the idle close the reopened session's first walk took it as its baseline and recorded nothing.
  it("records another writer's change to a file the session edited, made while the session was quiet, as its open walk did", async () => {
    const { create, batches, lookedAt } = recordingAuditor();
    const { first, clock, service, start, prompt, writes, turnEnd } = await idleService({ createAuditor: create });
    const [changed, removed, untouched] = [join(first, "app.py"), join(first, "gone.py"), join(first, "other.py")];
    await writeFile(changed, "x = 1\n");
    await writeFile(removed, "y = 1\n");
    await writeFile(untouched, "z = 1\n");
    await start("a");
    // The evaluator is open once the session's first walk has asked about the files, so the edits below are checked.
    await lookedAt("app.py");
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
    const { create, batches, lookedAt } = recordingAuditor();
    // A repository whose Git does not hold the session's text of the file, as with any edit not yet committed.
    const { first, clock, service, start, prompt, writes, turnEnd } = await idleService({ createAuditor: create }, { gitFirst: true });
    const app = join(first, "app.py");
    await writeFile(app, "x = 1\n");
    await start("a");
    await lookedAt("app.py");
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
    const { create, batches, lookedAt, reopened } = recordingAuditor({ reopening: reopening.promise });
    const { first, clock, service, start, prompt, writes, turnEnd } = await idleService({ createAuditor: create }, { host: "omp" });
    const app = join(first, "app.py");
    await writeFile(app, "x = 1\n");
    await start("a");
    await lookedAt("app.py");
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
  // In Claude Code the call also waits for the reopened session's rules, which open after the read back; OMP's does not.
  it.each(["claude-code", "omp"] as const)("answers the event that opens a quiet session again once the text of the files its agent edited is read back, in %s", async (host) => {
    const { create, batches, lookedAt } = recordingAuditor();
    const { first, clock, service, session, send, start, writes, turnEnd } = await idleService({ createAuditor: create }, { host });
    const app = join(first, "app.py");
    await writeFile(app, "x = 1\n");
    await start("a");
    await lookedAt("app.py");
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
    const { create, batches, looked, lookedAt, reopened } = recordingAuditor({ reopening: reopening.promise });
    const { first, clock, service, session, send, start, writes, turnEnd } = await idleService({ createAuditor: create }, { gitFirst: true, host: "omp" });
    const git = (...args: string[]) => execFileSync("git", ["-c", "user.name=Rerail Test", "-c", "user.email=test@example.invalid", ...args], { cwd: first });
    git("commit", "--quiet", "--allow-empty", "-m", "start");
    // `HEAD`'s last move is a checkout, which writes files, not a commit.
    git("checkout", "--quiet", "-b", "work");
    const [other, made] = [join(first, "other.py"), join(first, "made.py")];
    await writeFile(other, "z = 1\n");
    await start("a");
    await lookedAt("other.py");
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
    await lookedAt("other.py");
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
    const { create, scans, scanBegun } = recordingAuditor({ auditing: auditing.promise });
    const { clock, service, start } = await idleService({ createAuditor: create });
    await start("a");
    await scanBegun;
    expect(scans).toEqual(["repository"]);
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
  async function steeringService(setUp: { noLock?: boolean } = {}) {
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
    }, setUp);
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

  // A service that could take no owner lock writes owner rows that a later start can judge only by their pid, which
  // another process can hold after a kill: a quiet session it kept open read as live and owned for good, a scan for it
  // queued and never ran, and its next event was refused. It ends each quiet session at its idle close, as before, and
  // what a session owed still waits in its ledger for its next event.
  it("ends a quiet session at its idle close when the service could take no owner lock, and keeps what it owed for its next event", async () => {
    const { clock, service, start, prompt, ended, live, steer } = await steeringService({ noLock: true });
    await start("quiet");
    await start("owing");
    await steer("owing", "Fix owing.py.");
    clock.now += 60_000;
    await service.closeIdleSessions();
    // Each record ends at the idle close's time by the service's clock, as a suspended session's does.
    expect([ended("quiet"), live("quiet")]).toEqual([clock.now, false]);
    expect([ended("owing"), live("owing")]).toEqual([null, true]);
    await prompt("quiet", "back");
    expect([ended("quiet"), live("quiet")]).toEqual([null, true]);
    clock.now += 24 * 60 * 60_000;
    await service.closeIdleSessions();
    expect([ended("owing"), live("owing")]).toEqual([clock.now, false]);
    expect((await prompt("owing", "next morning")).map(delivery => delivery.content)).toEqual(["Fix owing.py."]);
    expect([ended("owing"), live("owing")]).toEqual([null, true]);
  });

  // A runtime that records its sessions in another database than the service's (an event's configuration names it)
  // wrote rows naming the service's lock, whose file is only beside the service's own database: a later start read
  // the lock as one that may still run, and no start sweeps that other database. Its quiet sessions stayed open after a
  // kill, live and owned for good while a process had the dead daemon's pid. Such a runtime holds no lock and ends a
  // quiet session at its idle close, as before.
  it("ends a quiet session at its idle close when its runtime records it in another database than the service's", async () => {
    const { clock, service, databasePath } = await idleService();
    const elsewhere = join(dirname(databasePath), "elsewhere");
    const checkout = join(elsewhere, "repo");
    await mkdir(checkout, { recursive: true });
    const otherDatabase = join(elsewhere, "rerail.sqlite");
    startTestRepository(checkout, otherDatabase);
    const configuration = { storageRoot: join(elsewhere, "state"), modelPath: resolve("model"), databasePath: otherDatabase, workspaceRoot: checkout };
    const session = { host: "claude-code" as const, sessionId: "o", cwd: checkout };
    await service.ingest({ kind: "session-start", eventId: "o:start", session }, configuration);
    // A token under a lock is the lock's id, a dot and a fresh id; one under none is a fresh id alone.
    expect(ownerTokenOf(otherDatabase, "o"), "its owner row's token").toMatch(/^[0-9a-f-]{36}$/u);
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect([endedAt(otherDatabase, "o"), ownedLive(otherDatabase, "o")]).toEqual([clock.now, false]);
    await service.ingest({ kind: "user-prompt", eventId: "o:back", sessionId: "o", session, text: "back" }, configuration);
    expect([endedAt(otherDatabase, "o"), ownedLive(otherDatabase, "o")]).toEqual([null, true]);
  });

  // A close whose runtime fails to close leaves that runtime's sessions open and owned. The close removed its lock's file
  // all the same, and a later start takes a lock with no file for one that may still run: those sessions read as live
  // and owned for good while a process had the daemon's pid. This process stands in for that one.
  it("ends at the next start the sessions a close that failed left open", async () => {
    const { service, start, ended, live, nextService } = await idleService();
    await start("a");
    const left: RerailRuntime[] = [];
    const dispose = vi.spyOn(RerailRuntime.prototype, "dispose").mockImplementationOnce(async function (this: RerailRuntime) {
      left.push(this);
      throw new Error("dispose failed");
    });
    services.splice(services.indexOf(service), 1);
    try {
      await expect(service.close()).rejects.toThrow("dispose failed");
      expect([ended("a"), live("a")]).toEqual([null, true]);
      await nextService(Date.now);
      expect([typeof ended("a"), live("a")]).toEqual(["number", false]);
    } finally {
      dispose.mockRestore();
      await Promise.all(left.map(runtime => runtime.dispose()));
    }
  });

  // One runtime's failed close cut the others short: the service's close settled, and let its lock go, while they were
  // still ending their sessions, which a daemon stopping then never finished.
  it("waits for every runtime to close before its own close settles, when one of them fails", async () => {
    const { service, second, start } = await idleService();
    await start("a");
    await start("b", second);
    const failed = Promise.withResolvers<void>();
    const gate = Promise.withResolvers<void>();
    const dispose = RerailRuntime.prototype.dispose;
    let calls = 0;
    const spy = vi.spyOn(RerailRuntime.prototype, "dispose").mockImplementation(async function (this: RerailRuntime) {
      calls += 1;
      if (calls === 1) {
        await dispose.call(this);
        failed.resolve();
        throw new Error("dispose failed");
      }
      await gate.promise;
      await dispose.call(this);
    });
    services.splice(services.indexOf(service), 1);
    try {
      let settled = false;
      const closing = service.close().finally(() => { settled = true; });
      void closing.catch(() => undefined);
      await failed.promise;
      // What the close does after a runtime's close fails runs within the turns that follow.
      for (let turn = 0; turn < 3; turn += 1) await new Promise(resolve => setImmediate(resolve));
      expect(settled, "the close settled while a runtime was still closing").toBe(false);
      gate.resolve();
      await expect(closing).rejects.toThrow("dispose failed");
      expect(calls).toBe(2);
    } finally {
      gate.resolve();
      spy.mockRestore();
    }
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
      socketPath, storageRoot: join(root, "state"), databasePath, embedder: testEmbedder, idleSessionMs: 60_000, needsRoom: () => true,
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

  // A stopped repository records nothing. The daemon's stop there wrote the steer its session owed to the ledger all the
  // same, and once the repository started again the session's next event gave the agent a steer from before the stop.
  it("keeps no steer a session owed when the daemon stops in a repository stopped before", async () => {
    const { first, service, session, start, steer, databasePath } = await steeringService();
    await start("a");
    await steer("a", "Fix a.py.");
    stopTestRepository(first, databasePath);
    services.splice(services.indexOf(service), 1);
    await service.close();
    startTestRepository(first, databasePath);
    const root = dirname(databasePath);
    const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-idle-again-${process.pid}-${Date.now()}` : join(root, "engine-again.sock");
    const again = await startEngineService({
      socketPath, storageRoot: join(root, "state"), databasePath, embedder: testEmbedder, idleSessionMs: 60_000, needsRoom: () => true,
      createAuditor: quietAuditor,
    });
    services.push(again);
    const delivered = await ingestEngineEvent({ socketPath }, { kind: "user-prompt", eventId: "a:resumed", sessionId: "a", session: session("a"), text: "resumed" });
    expect(delivered).toEqual([]);
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
    const left = vi.spyOn(KeptRuntimes.prototype, "left");
    try {
      const { second, clock, service, start, ended } = await idleService();
      await start("a");
      await start("b", second);
      left.mockClear();
      clock.now += 60_000;
      vi.advanceTimersByTime(60_000);
      // Both quiet sessions suspended: the runtimes they left are kept, under 15 minutes though the daemon needs the room.
      await vi.waitFor(() => expect(left).toHaveBeenCalledTimes(2));
      expect(service.openRuntimes()).toBe(2);
      expect(ended("a")).toBeNull();
      // 15 minutes on, the minute's sweep closes them.
      clock.now += 15 * 60_000;
      vi.advanceTimersByTime(60_000);
      await vi.waitFor(() => expect(service.openRuntimes()).toBe(0));
    } finally {
      left.mockRestore();
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

/** When `sessionId`'s newest recorded event was observed, read through a connection of its own. */
function lastObservedAt(databasePath: string, sessionId: string): number {
  const database = new Database(databasePath, { readonly: true });
  try {
    const row = database.prepare("SELECT observed_at AS at FROM events WHERE session_id = ? ORDER BY sequence DESC LIMIT 1").get(sessionId) as { at: number } | undefined;
    if (row === undefined) throw new Error(`no event of ${sessionId}`);
    return row.at;
  } finally {
    database.close();
  }
}

/** The token of `sessionId`'s owner row, read through a connection of its own. */
function ownerTokenOf(databasePath: string, sessionId: string): string {
  const database = new Database(databasePath, { readonly: true });
  try {
    const row = database.prepare("SELECT token FROM policy_session_owners WHERE session_id = ?").get(sessionId) as { token: string } | undefined;
    if (row === undefined) throw new Error(`no owner row of ${sessionId}`);
    return row.token;
  } finally {
    database.close();
  }
}

/**
 * Another runtime's claim on `sessionId`, written through `writer`, which holds the write lock: a claim of this process
 * waiting for that lock is refused once it is made.
 */
function claimFirst(writer: Database.Database, sessionId: string): void {
  writer.prepare("INSERT INTO policy_session_owners (session_id, delivery, active, process_id, token, updated_at) VALUES (?, 'background', 1, ?, 'another runtime', 0)")
    .run(sessionId, process.pid);
}

/** Makes the owner row of `sessionId` name process `pid`, through a connection of its own. */
function setOwnerProcess(databasePath: string, sessionId: string, pid: number): void {
  const database = new Database(databasePath);
  try {
    const { changes } = database.prepare("UPDATE policy_session_owners SET process_id = ? WHERE session_id = ?").run(pid, sessionId);
    if (changes !== 1) throw new Error(`no owner row of ${sessionId}`);
  } finally {
    database.close();
  }
}

/** The id of a process that has exited, as the owner row of a daemon that died names one. */
function goneProcessId(): number {
  const { pid, status } = spawnSync(process.execPath, ["-e", ""]);
  if (pid === undefined || status !== 0) throw new Error("the short-lived process did not run");
  return pid;
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
