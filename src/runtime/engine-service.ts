import { existsSync } from "node:fs";
import { lstat, mkdir, unlink } from "node:fs/promises";
import { createServer, request, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { connect, type Socket } from "node:net";
import { dirname, resolve as resolvePath } from "node:path";
import { setFlagsFromString } from "node:v8";
import { runInNewContext } from "node:vm";
import { z } from "zod/v4";
import type { AdapterDelivery, AdapterEvent } from "../engine/adapter-contract.js";
import { SqliteAuditControlStore } from "../engine/audit/audit-control.js";
import { forgetFreedOwnerLocks, holdOwnerLock, someOwnerLockFreed } from "../engine/audit/owner-lock.js";
import {
  AdapterEventSchema,
  decodeRerailEventResult,
  encodeRerailEventResult,
  RERAIL_EVENT_MAX_FRAME_BYTES,
} from "./event-protocol.js";
import { defaultRerailDatabasePath, failWhileWriteLocked, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { isBusy } from "../engine/storage/sqlite-wal.js";
import { DAEMON_ACCESS_HEADER, sameSecret, type EngineEndpoint } from "./daemon-control.js";
import { ENGINE_CHALLENGE_HEADER, ENGINE_PROOF_PATH, engineHoldsCapability, engineProof, isEngineChallenge } from "./daemon/engine-proof.js";
import { isNamedPipe } from "./daemon/engine-pipe.js";
import { checkoutStatus } from "./repository-start.js";
import { nativeTreeWatch, type TreeWatch } from "../engine/code-rule/tree-watch.js";
import { RepositoryWorktreeWalks } from "../engine/code-rule/worktree-walks.js";
import {
  endSuspendedSessions, failQueuedScans, RerailRuntime, type RerailRuntimeOptions, type SessionSuspension, type SuspendedSessions,
} from "./runtime.js";
import { SessionLanes } from "./session-lanes.js";

export interface EngineService {
  /** Where it listens: the socket path it was given, or its `pipeFallback` when another process held that pipe. */
  readonly socketPath: string;
  /**
   * Stops taking connections, answers every request whose body has arrived (running each one's event, queued ones too),
   * then closes the runtimes. A request that stalls before it has all arrived is cut off ({@link cutStalledRequests}).
   */
  close(): Promise<void>;
  ingest(
    event: AdapterEvent,
    configuration?: EngineRuntimeConfiguration,
  ): Promise<readonly AdapterDelivery[]>;
  /** How many runtimes are open: one per checkout some live session uses. */
  openRuntimes(): number;
  /**
   * Suspends every session that has sent no event for the idle limit and owes its agent nothing, and every one quiet for
   * a day whatever it owes (`RerailRuntime.closeIdleSession`), ends each suspended one that can no longer open again
   * (its folder removed, or Rerail stopped in its checkout), then, once a daemon on the same records that held its
   * owner lock has ended, what it left open (`endAbandonedSessions`); the service also does this every minute.
   */
  closeIdleSessions(): Promise<void>;
  /**
   * Opens again the session the idle close suspended, so a scan just queued for it runs now, as it would have while the
   * session was open; nothing for a session that is not suspended. Never rejects: it reports its own failure, fails the
   * scans that waited for the open with why, and the service's close waits for it.
   */
  resumeSession(sessionId: string): Promise<void>;
}

/**
 * How long a session may send no event before the engine service suspends it: frees what it holds in memory, as its end
 * would. Its end is not certain to arrive: a coding agent that is killed sends none, and Claude Code gives its SessionEnd
 * hook 1.5 s, which a hook on a busy Windows machine outlasts. A session whose end was lost kept its state (its own copy
 * of its checkout's source walk, its conversation history, its evaluator) until the daemon stopped. A quiet session's
 * agent may as well still be open (a person away, a long think), so a suspended session stays open in the database and
 * keeps its owner claim until its daemon stops, the next daemon starts after one that died (`endAbandonedSessions`), or
 * a sweep finds its folder removed or Rerail stopped in its checkout (`endUnopenableSessions`): it reads as live, a scan
 * asked for it opens it again to run (`resumeSession`), and its next event opens it again from
 * its ledger, where its source walks stood (`RerailRuntime.closeIdleSession`). A session waiting on a tool call, holding
 * a steer or a finding for its agent, or running a check stays as it is for up to a day (`OWING_SESSION_IDLE_MS`). A
 * reopened session's code-rule boundaries wait for its rules to open again (`REOPENED_RULES_WAIT_MS` in runtime.ts), so
 * its first edit is still steered at its own result.
 */
export const IDLE_SESSION_MS = 30 * 60_000;
/**
 * How long a session that still owes its agent something (a tool call's result, a steer, a held finding, a check, a
 * scan) may send no event before the engine service suspends it anyway. An agent killed while its session owed it
 * something, as Claude Code closed at a permission prompt is, sends no end and no result: without a bound its session
 * kept its source walk, its evaluator and its checkout's runtime until the daemon stopped. The bound is a day, not
 * `IDLE_SESSION_MS`, because an agent waiting on a person at a permission prompt can be left for hours while they are
 * away, and the session its next event opened again would hold the agent's next tool call while its rules compile again
 * (`REOPENED_RULES_WAIT_MS` in runtime.ts), or check its edit a turn later in a host that does not wait. Past the bound,
 * the steers and findings the session holds wait in its ledger for its next event, and the edit of the call it waited
 * on is checked once its rules are open again (`#takeOwed`, `#checkWhenOpen` in runtime.ts).
 */
const OWING_SESSION_IDLE_MS = 24 * 60 * 60_000;
/** How often the engine service looks for idle sessions. */
const IDLE_SWEEP_MS = 60_000;
/**
 * How soon a quiet session's open for a scan is tried again after it met another process's write lock: as often as an
 * open session's control timer looks for its scans.
 */
const RESUME_RETRY_MS = 250;

const ConfigurationSchema = z
  .object({
    storageRoot: z.string().min(1),
    modelPath: z.string().min(1),
    /** Absent selects the engine service's own database. */
    databasePath: z.string().min(1).optional(),
    workspaceRoot: z.string().min(1).optional(),
  })
  .strict();
export type EngineRuntimeConfiguration = z.infer<typeof ConfigurationSchema>;
const RequestSchema = z
  .object({
    event: AdapterEventSchema,
    configuration: ConfigurationSchema.optional(),
  })
  .strict();

interface EngineRequest {
  event: AdapterEvent;
  configuration: EngineRuntimeConfiguration | undefined;
}

function parseEngineRequest(value: unknown): EngineRequest {
  const { event, configuration } = RequestSchema.parse(value);
  return { event, configuration };
}


/** Why `directory` cannot hold the engine socket, and what the person can do about it. */
interface DirectoryRefusal {
  problem: string;
  remedy: string;
}

/**
 * Makes `directory` (mode 0700, with any missing parents) and answers why it is refused, or undefined when it is this
 * user's alone: a real directory, not a link, owned by `uid`, with no group or other permission bits. Windows has no
 * POSIX owner or mode to check (`uid` undefined).
 */
async function privateDirectoryRefusal(directory: string, uid: number | undefined): Promise<DirectoryRefusal | undefined> {
  try {
    await mkdir(directory, { recursive: true, mode: 0o700 });
  } catch (error) {
    // Something that is not a directory already holds the name; `lstat` below says what and whose.
    if ((error as NodeJS.ErrnoException).code !== "EEXIST") {
      return { problem: `could not be made (${(error as Error).message})`, remedy: `Check that this user may create ${directory}.` };
    }
  }
  if (uid === undefined) return undefined;
  const info = await lstat(directory);
  const problem = !info.isDirectory() ? "is a link or not a directory"
    : info.uid !== uid ? `is owned by uid ${info.uid}, not ${uid}`
      : (info.mode & 0o077) !== 0 ? `has mode ${(info.mode & 0o777).toString(8)}, not 700`
        : undefined;
  if (problem === undefined) return undefined;
  // In a shared temp directory (sticky bit) only its owner or an administrator can remove what another user made.
  const remedy = info.uid !== uid
    ? `Another user (uid ${info.uid}) created it; they or an administrator must remove ${directory}.`
    : `Remove ${directory} and Rerail makes it again, for this user alone.`;
  return { problem, remedy };
}

/**
 * Makes `directory` (mode 0700, with any missing parents) and refuses it unless it is this user's alone: a real
 * directory, not a link, owned by `uid`, with no group or other permission bits. The socket inside then takes no
 * permission change after `listen`: nobody else can reach, replace or pre-create it. A directory another user made
 * first at a predictable path in a shared temp directory is refused rather than used. Windows has no POSIX owner or
 * mode to check (`uid` undefined).
 */
export async function preparePrivateDirectory(directory: string, uid: number | undefined = process.getuid?.()): Promise<void> {
  const refusal = await privateDirectoryRefusal(directory, uid);
  if (refusal !== undefined) {
    throw new Error(`Rerail will not put its engine socket in ${directory}: it ${refusal.problem}. The socket needs a directory only this user can open (mode 700). ${refusal.remedy}`);
  }
}

/**
 * The first of `candidates` that can carry the engine socket: a Windows named pipe as it is, a Unix socket path that
 * fits `sun_path` and whose directory `preparePrivateDirectory` accepts (and has made). Anyone can make a directory at a
 * predictable temp path before this user does; a later candidate keeps Rerail working instead of every hook failing
 * open. When none can, the error names each one refused, why, and what to do.
 */
export async function chooseEngineSocketPath(
  candidates: readonly string[],
  uid: number | undefined = process.getuid?.(),
  platform: NodeJS.Platform = process.platform,
): Promise<string> {
  const refused: string[] = [];
  // The longest Unix socket path in bytes, the terminating NUL aside: `sun_path` holds 108 on Linux, 104 on macOS and
  // the BSDs. A longer path fails to bind, or on some systems binds a truncated name no client finds.
  const limit = platform === "linux" || platform === "android" ? 107 : 103;
  for (const candidate of candidates) {
    if (isNamedPipe(candidate)) return candidate;
    const bytes = Buffer.byteLength(candidate);
    if (bytes > limit) {
      refused.push(`${candidate} is ${bytes} bytes, longer than the ${limit} a socket path may be here. Pointing RERAIL_HOME (or TMPDIR) at a shorter folder shortens it.`);
      continue;
    }
    const directory = dirname(candidate);
    const refusal = await privateDirectoryRefusal(directory, uid);
    if (refusal === undefined) return candidate;
    refused.push(`${directory}: it ${refusal.problem}. ${refusal.remedy}`);
  }
  throw new Error(`Rerail found nowhere to put its engine socket, which needs a directory only this user can open (mode 700):\n${refused.map(line => `- ${line}`).join("\n")}`);
}

type EngineServiceOptions = Omit<RerailRuntimeOptions, "workspaceRoot"> & {
  socketPath: string;
  workspaceRoot?: string;
  /**
   * The capability every request must carry as `x-rerail-access`: the daemon's own. A request decides where its
   * runtime writes (`storageRoot`, `databasePath`), and the daemon runs outside any coding agent's sandbox, so a
   * process that can connect to the socket but cannot read the Rerail home must not be able to use it. Only a
   * standalone service (tests, benchmarks) omits it.
   */
  accessToken?: string;
  /**
   * A pipe name to listen on instead when `socketPath`, a Windows named pipe, is held by a process that cannot prove it
   * is this home's engine service (`engineHoldsCapability`). Any local user can create a pipe of any name, and
   * `socketPath` is derived from the database path, which anyone can work out, so without one a pipe made first stops
   * every daemon start. Clients find the pipe through the state file, never by computing it. A socket path the caller
   * named outright gets none.
   */
  pipeFallback?: () => string;
  /** Defaults to `IDLE_SESSION_MS`; tests pass their own. */
  idleSessionMs?: number;
  /** The clock idle sessions are measured by; defaults to `Date.now`. */
  now?: () => number;
  /** How long a checkout's runtime stays open once no session uses it; {@link IDLE_RUNTIME_MS} by default. */
  idleRuntimeMs?: number;
};

/**
 * How long a checkout's runtime stays open after its last session ends. A session that starts there again within it
 * finds its rules compiled and its checkout walked: the five packs a TypeScript repository adds take 12 s of CPU to
 * compile on a Windows laptop. After it the runtime closes, and the engine frees what its rules held.
 */
export const IDLE_RUNTIME_MS = 15 * 60_000;

interface EngineRoutingState {
  runtimes: Map<string, RerailRuntime>;
  runtimeKeysBySession: Map<string, Set<string>>;
  runtimeKeysByConfiguration: Map<string, string>;
  /** When each open session last sent an event, by its lane (`laneOf`). */
  lastEventAt: Map<string, number>;
  lanes: SessionLanes;
  pending: Set<Promise<readonly AdapterDelivery[]>>;
  /** The runtimes no session uses, each with the timer that closes it. */
  idle: Map<string, NodeJS.Timeout>;
  /** How long a runtime no session uses stays open: `idleRuntimeMs`, or `IDLE_RUNTIME_MS`. */
  idleRuntimeMs: number;
  /** What each open runtime was made with, by its key: a suspended session's runtime is made again from it. */
  runtimeOptions: Map<string, RerailRuntimeOptions>;
  /** The sessions the idle close suspended, by session id, until a runtime opens one again or the service closes. */
  suspended: Map<string, SuspendedEntry>;
  /** The timer of each suspended session whose open for a scan met another process's write lock (`retryResume`). */
  resumeRetries: Map<string, NodeJS.Timeout>;
  /**
   * The lock the service holds for its life beside its own database (`holdOwnerLock`), which the owner rows of its
   * runtimes over that database name (`ownerLockFor`); none where it could take none.
   */
  ownerLockId: string | undefined;
  /** The service's own database: the one its start and its idle sweeps end a dead daemon's sessions in. */
  databasePath: string;
  /**
   * The walk of each repository's other worktrees, shared by the sessions of every runtime: hooks name the session's
   * folder, so sessions in different worktrees of one repository run in different runtimes, and each walking every
   * worktree for itself held every file of every worktree once per session.
   */
  worktreeWalks: RepositoryWorktreeWalks;
}

/**
 * The lock the owner rows of a runtime over `databasePath` name: the service's, for its own database, beside which its
 * file is and which its start and sweeps judge; none for another database, which no start of this service sweeps, so
 * the runtime ends its quiet sessions at the idle close (`closeIdleSessions`) as a service with no lock does.
 */
function ownerLockFor(state: EngineRoutingState, databasePath: string | undefined): string | undefined {
  return databasePath === state.databasePath ? state.ownerLockId : undefined;
}

/** Whether the runtime at `runtimeKey` ends a quiet session rather than suspending it: its owner rows name no lock. */
function endsQuietSessions(state: EngineRoutingState, runtimeKey: string): boolean {
  return ownerLockFor(state, state.runtimeOptions.get(runtimeKey)?.databasePath) === undefined;
}

/** A suspended session, with the runtime it was open in and what that runtime is made with. */
interface SuspendedEntry {
  readonly runtimeKey: string;
  readonly runtimeOptions: RerailRuntimeOptions | undefined;
  readonly suspension: SessionSuspension;
}

/**
 * The service's suspended sessions as the runtime at `runtimeKey` sees them: it keeps the sessions it suspends there, and
 * one opening a session takes its suspension back.
 */
function suspendedSessions(state: EngineRoutingState, runtimeKey: string): SuspendedSessions {
  return {
    keep: suspension => keepSuspended(state, runtimeKey, suspension),
    get: sessionId => state.suspended.get(sessionId)?.suspension,
    take: (sessionId) => {
      const entry = state.suspended.get(sessionId);
      state.suspended.delete(sessionId);
      return entry?.suspension;
    },
  };
}

/** The lane one session's work in one runtime runs in, one at a time. */
function laneOf(runtimeKey: string, sessionId: string): string {
  return JSON.stringify([runtimeKey, sessionId]);
}

interface RuntimeRoute {
  runtimeKey: string;
  runtimeOptions?: RerailRuntimeOptions;
}

function resolveRuntimeRoute(
  event: AdapterEvent,
  configuration: EngineRuntimeConfiguration | undefined,
  options: EngineServiceOptions,
  state: EngineRoutingState,
): RuntimeRoute {
  const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
  const eventWorkspace = event.kind === "session-start" ? event.session.cwd : event.session?.cwd;
  const workspaceRoot = configuration?.workspaceRoot ?? options.workspaceRoot ?? eventWorkspace;
  const configurationKey = configuration === undefined
    ? undefined
    : JSON.stringify([
        resolvePath(configuration.storageRoot),
        resolvePath(configuration.modelPath),
        resolvePath(configuration.databasePath ?? options.databasePath ?? defaultRerailDatabasePath()),
      ]);
  if (workspaceRoot === undefined) {
    const configuredRuntimeKey = configurationKey === undefined
      ? undefined
      : state.runtimeKeysByConfiguration.get(configurationKey);
    const candidates = state.runtimeKeysBySession.get(sessionId);
    if (configuredRuntimeKey === undefined && candidates?.size !== 1) {
      throw new Error("Kernel runtime requires workspaceRoot or unambiguous session context");
    }
    return { runtimeKey: configuredRuntimeKey ?? candidates!.values().next().value! };
  }
  const serviceDatabasePath = resolvePath(options.databasePath ?? defaultRerailDatabasePath());
  const { accessToken: _accessToken, ...serviceDefaults } = options;
  const runtimeOptions: RerailRuntimeOptions = configuration === undefined
    ? {
        ...serviceDefaults,
        storageRoot: resolvePath(options.storageRoot),
        databasePath: serviceDatabasePath,
        workspaceRoot: resolvePath(workspaceRoot),
      }
    : {
        storageRoot: resolvePath(configuration.storageRoot),
        modelPath: resolvePath(configuration.modelPath),
        databasePath: resolvePath(configuration.databasePath ?? serviceDatabasePath),
        workspaceRoot: resolvePath(workspaceRoot),
      };
  // The service's Merge Assist is over its own database; a runtime over another makes its own.
  if (configuration !== undefined && options.mergeAssist !== undefined && runtimeOptions.databasePath === serviceDatabasePath) {
    runtimeOptions.mergeAssist = options.mergeAssist;
  }
  const runtimeKey = JSON.stringify([
    runtimeOptions.storageRoot,
    runtimeOptions.modelPath,
    runtimeOptions.databasePath,
    runtimeOptions.workspaceRoot,
  ]);
  if (configurationKey !== undefined) state.runtimeKeysByConfiguration.set(configurationKey, runtimeKey);
  return { runtimeKey, runtimeOptions };
}

/**
 * Writes an event's answer and says whether it reaches a client that still waits for it (`answered`). The service
 * writes it in the session's lane, so what an answer its write already showed undelivered took is the session's again
 * before its next event runs.
 */
type EngineAnswer = (deliveries: readonly AdapterDelivery[]) => AnswerFate;

/**
 * What became of an answer: `refused` when the write itself showed it reached no client that still waited (its
 * connection closed or ended before it, or the write failed); else `heard` resolves once the connection shows it:
 * true once the answer was all handed to it, false when it closed first.
 */
interface AnswerFate {
  readonly refused: boolean;
  readonly heard: Promise<boolean>;
}

async function routeEngineEvent(
  event: AdapterEvent,
  configuration: EngineRuntimeConfiguration | undefined,
  options: EngineServiceOptions,
  state: EngineRoutingState,
  answer?: EngineAnswer,
): Promise<readonly AdapterDelivery[]> {
  const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
  const { runtimeKey, runtimeOptions } = resolveRuntimeRoute(event, configuration, options, state);
  const lane = laneOf(runtimeKey, sessionId);
  keepRuntime(state, runtimeKey);
  const execution = state.lanes.run(lane, async () => {
    const runtime = runtimeFor(state, runtimeKey, runtimeOptions, sessionId);
    // Every event, not only a start: a session whose start was lost still uses this runtime until it ends.
    if (event.kind !== "session-end") noteSessionUse(state, sessionId, runtimeKey, (options.now ?? Date.now)());
    const deliveries = await runtime.ingest(event);
    // A hook past its own limit has closed its connection: the steers its answer took go to the session's next answer.
    if (answer !== undefined) await keepUnheard(answer(deliveries), () => runtime.returnUndelivered(sessionId, deliveries));
    return deliveries;
  });
  track(state, execution);
  try {
    return await execution;
  } finally {
    if (event.kind === "session-end") afterSessionEnd(state, sessionId, runtimeKey);
  }
}

/**
 * Gives the session back what an answer took once no client heard it (`giveBack`): before the session's next event when
 * the write showed it, else once the connection shows it, for whichever of the session's answers comes next. The lane
 * never waits on the write itself, which a client alive that has stopped reading never lets complete once the answer
 * is bigger than its connection holds. A service's close waits for every connection to close, so it has each verdict.
 */
async function keepUnheard(fate: AnswerFate, giveBack: () => Promise<void>): Promise<void> {
  if (fate.refused) return await giveBack();
  void fate.heard.then(async heard => (heard ? undefined : await giveBack()));
}

/**
 * The session has ended, whether its runtime recorded the end or not: it is no longer quiet, its suspension, when its
 * runtime did not open it again (a stopped repository, or an open that failed), ends its record here, and the runtime it
 * used retires once no other session uses it.
 */
function afterSessionEnd(state: EngineRoutingState, sessionId: string, runtimeKey: string): void {
  state.lastEventAt.delete(laneOf(runtimeKey, sessionId));
  endIfSuspended(state, sessionId);
  releaseSessionRuntime(state, sessionId, runtimeKey);
}

/** The runtime at `runtimeKey` is about to be used: it is no longer one to close for want of use. */
function keepRuntime(state: EngineRoutingState, runtimeKey: string): void {
  clearTimeout(state.idle.get(runtimeKey));
  state.idle.delete(runtimeKey);
}

/** The runtime open at `runtimeKey`, else one made with `runtimeOptions`, which takes back the sessions the service suspended. */
function runtimeFor(
  state: EngineRoutingState,
  runtimeKey: string,
  runtimeOptions: RerailRuntimeOptions | undefined,
  sessionId: string,
): RerailRuntime {
  const open = state.runtimes.get(runtimeKey);
  if (open !== undefined) return open;
  if (runtimeOptions === undefined) throw new Error(`Kernel runtime is unavailable for session ${sessionId}`);
  const runtime = new RerailRuntime({
    ...runtimeOptions, autostart: false, startGate: true, worktreeWalks: state.worktreeWalks,
    suspendedSessions: suspendedSessions(state, runtimeKey), ownerLockId: ownerLockFor(state, runtimeOptions.databasePath),
  });
  state.runtimes.set(runtimeKey, runtime);
  state.runtimeOptions.set(runtimeKey, runtimeOptions);
  return runtime;
}

/** `sessionId` uses the runtime at `runtimeKey` from now on, and was last heard from `at`. */
function noteSessionUse(state: EngineRoutingState, sessionId: string, runtimeKey: string, at: number): void {
  const keys = state.runtimeKeysBySession.get(sessionId) ?? new Set<string>();
  keys.add(runtimeKey);
  state.runtimeKeysBySession.set(sessionId, keys);
  state.lastEventAt.set(laneOf(runtimeKey, sessionId), at);
}

/** Keeps `work` among what `close` waits for until it settles. */
function track(state: EngineRoutingState, work: Promise<unknown>): void {
  const settled = work.then(() => [] as const, () => [] as const);
  state.pending.add(settled);
  void settled.then(() => state.pending.delete(settled));
}

/** `sessionId` no longer uses the runtime at `runtimeKey`, which closes once no session has used it a while. */
function releaseSessionRuntime(state: EngineRoutingState, sessionId: string, runtimeKey: string): void {
  const keys = state.runtimeKeysBySession.get(sessionId);
  keys?.delete(runtimeKey);
  if (keys?.size === 0) state.runtimeKeysBySession.delete(sessionId);
  retireUnusedRuntime(state, runtimeKey);
}

/**
 * Suspends each session that has sent no event for `idleMs` (`IDLE_SESSION_MS`): its runtime frees what it holds and
 * keeps it open in the database (`RerailRuntime.closeIdleSession`), and the service keeps its suspension for the
 * session's next open; then its runtime retires as after an end. The check runs again in the session's lane, so an
 * event that came in meanwhile keeps the session open. A session that still owes its agent a tool call's check, a
 * steer, a held finding or a scan stays as it is until it has been quiet for `OWING_SESSION_IDLE_MS`, and so does one
 * whose suspension fails, which is reported; the next sweep looks again. A runtime whose owner rows name no lock
 * (`ownerLockFor`: the service could take none, or the runtime records in another database) ends each quiet session
 * instead: after a kill, its rows could be judged only by their pid, and a session it kept open would read as live and
 * owned for good.
 */
async function closeIdleSessions(state: EngineRoutingState, idleMs: number, now: () => number): Promise<void> {
  /** How long the session at `lane` has sent nothing; minus infinity once it has closed, so it is never idle. */
  const quietFor = (lane: string): number => {
    const at = state.lastEventAt.get(lane);
    return at === undefined ? Number.NEGATIVE_INFINITY : now() - at;
  };
  await Promise.all([...state.lastEventAt.keys()].filter(lane => quietFor(lane) >= idleMs).map(async (lane) => {
    const [runtimeKey, sessionId] = JSON.parse(lane) as [string, string];
    const closing = state.lanes.run(lane, async () => {
      const quiet = quietFor(lane);
      if (quiet < idleMs) return false;
      const evenIfOwing = quiet >= OWING_SESSION_IDLE_MS;
      // A suspended session's runtime has kept its suspension (`SuspendedSessions.keep`) before closing it, with the time
      // its record ends at if no runtime opens it again.
      const end = endsQuietSessions(state, runtimeKey);
      const idle = await state.runtimes.get(runtimeKey)?.closeIdleSession(sessionId, { evenIfOwing, at: now(), end });
      if (idle?.kind === "owing") return false;
      state.lastEventAt.delete(lane);
      return true;
    });
    track(state, closing);
    try {
      if (await closing) releaseSessionRuntime(state, sessionId, runtimeKey);
    } catch (error) {
      process.stderr.write(`rerail: closing idle session ${sessionId} failed: ${error instanceof Error ? error.message : String(error)}\n`);
    }
  }));
}

/**
 * One idle sweep: suspends this service's quiet sessions (`closeIdleSessions`), ends those that can no longer open again
 * (`endUnopenableSessions`), then ends what a daemon that died left open (`endAbandonedSessions`), as a start does, for
 * when the start's own pass failed, which it reports: with one daemon per Rerail home (`holdHomeDaemonLock`), no other
 * start comes while this one runs. Only once an owner lock beside the database is free (`someOwnerLockFreed`), which a
 * service that ended leaves: opening a writer waits for another process's write lock, up to `busy_timeout`, on the
 * daemon's main thread, so a minute with no such lock opens no writer. Rows that name no lock are judged at the next
 * start then, or by a sweep a free lock brings.
 */
async function sweepIdle(state: EngineRoutingState, options: EngineServiceOptions, databasePath: string): Promise<void> {
  const idleMs = options.idleSessionMs ?? IDLE_SESSION_MS;
  const now = options.now ?? Date.now;
  await closeIdleSessions(state, idleMs, now);
  await endUnopenableSessions(state);
  if (someOwnerLockFreed(databasePath)) endAbandonedSessions(databasePath, idleMs, now);
}

/**
 * Ends, at the time each went quiet, every suspended session that can no longer open again (`unopenable`), as the idle
 * close ended it before sessions were suspended: kept, it would read as live and owned until its daemon stops. Each
 * ends in its lane, once an open already under way there has taken it or not. The start states are read over one
 * read-only connection per database, only while some session is suspended; another process's write lock holds up no
 * reader.
 */
async function endUnopenableSessions(state: EngineRoutingState): Promise<void> {
  const ending: Array<{ entry: SuspendedEntry; reason: string }> = [];
  for (const [databasePath, entries] of byDatabase([...state.suspended.values()])) {
    readingStartStates(databasePath, (database) => {
      for (const entry of entries) {
        const reason = unopenable(entry, database);
        if (reason !== undefined) ending.push({ entry, reason });
      }
    });
  }
  await Promise.all(ending.map(({ entry, reason }) => {
    const { runtimeKey, suspension: { descriptor: { sessionId } } } = entry;
    const work = state.lanes.run(laneOf(runtimeKey, sessionId), async () => {
      if (state.suspended.get(sessionId) === entry) endUnopened(state, entry, reason);
    });
    track(state, work);
    return work;
  }));
}

/**
 * Why the suspended session of `entry` can no longer open again, or undefined while it can: its folder was removed
 * (opening it would make the folder anew with Rerail's records), or Rerail was stopped in its checkout (a stopped
 * repository drops the session's events and records no scan). `database` is a read-only connection to its records.
 */
function unopenable(entry: SuspendedEntry, database: RerailDatabase): string | undefined {
  const { descriptor: { cwd }, startGate } = entry.suspension;
  if (!existsSync(cwd)) return `its folder ${cwd} was removed`;
  if (startGate === undefined || checkoutStatus(database, startGate).started) return undefined;
  return `Rerail is stopped in ${startGate.checkoutPath}`;
}

/** `read` over a read-only connection to `databasePath`; a failure is reported and reads as undefined. */
function readingStartStates<T>(databasePath: string, read: (database: RerailDatabase) => T): T | undefined {
  let database: RerailDatabase | undefined;
  try {
    database = openRerailDatabase(databasePath, { readonly: true });
    return read(database);
  } catch (error) {
    process.stderr.write(`rerail: reading where quiet sessions' checkouts are started failed: ${error instanceof Error ? error.message : String(error)}\n`);
    return undefined;
  } finally {
    database?.$client.close();
  }
}

/**
 * Keeps a suspended session, with how to make its runtime again, for its next open. Its runtime keeps it as the close
 * begins (`RerailRuntime.closeIdleSession`), so a scan asked for it meanwhile opens it again behind the close, in its
 * lane, and a close that fails leaves it here to end at the service's close.
 */
function keepSuspended(state: EngineRoutingState, runtimeKey: string, suspension: SessionSuspension): void {
  state.suspended.set(suspension.descriptor.sessionId, { runtimeKey, runtimeOptions: state.runtimeOptions.get(runtimeKey), suspension });
}

/**
 * Opens again, in its lane, the session the idle close suspended (`RerailRuntime.resumeSession`), making its runtime
 * again when that has closed, so a scan just queued for it runs now. A session that is not suspended is left alone, and
 * so is one whose end, or an event that opened it, took its suspension while this waited for the lane; one that can no
 * longer open again (`unopenable`: its folder removed, or Rerail stopped in its checkout) is ended instead
 * (`endUnopened`). While another process holds the database's write lock the open stops before its first write, which
 * would wait for it up to 5 s on the daemon's thread, and is tried again (`retryResume`), its scans still queued, as an
 * open session's claim waits for the lock. Another failure is reported, and the scans queued for a session still
 * suspended fail with it (`failQueuedScans`), as an open session's do when its evaluator cannot open.
 */
async function resumeSuspended(state: EngineRoutingState, sessionId: string, now: () => number): Promise<void> {
  const entry = state.suspended.get(sessionId);
  if (entry === undefined) return;
  const { runtimeKey, runtimeOptions, suspension } = entry;
  keepRuntime(state, runtimeKey);
  const resuming = state.lanes.run(laneOf(runtimeKey, sessionId), async () => {
    // Asked again in the lane: an end that ran first has closed the session's record, which must stay closed.
    if (state.suspended.get(sessionId) !== entry) return;
    const reason = readingStartStates(suspension.databasePath, database => unopenable(entry, database));
    if (reason !== undefined) {
      endUnopened(state, entry, reason);
      retireUnusedRuntime(state, runtimeKey);
      return;
    }
    try {
      // A runtime made again opens its stores, each of which writes.
      if (!state.runtimes.has(runtimeKey)) failWhileWriteLocked(suspension.databasePath);
      const runtime = runtimeFor(state, runtimeKey, runtimeOptions, sessionId);
      noteSessionUse(state, sessionId, runtimeKey, now());
      await runtime.resumeSession(suspension.descriptor);
    } catch (error) {
      if (!isBusy(error) || state.suspended.get(sessionId) !== entry) throw error;
      state.lastEventAt.delete(laneOf(runtimeKey, sessionId));
      releaseSessionRuntime(state, sessionId, runtimeKey);
      retryResume(state, sessionId, now);
    }
  });
  track(state, resuming);
  try {
    await resuming;
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    process.stderr.write(`rerail: opening quiet session ${sessionId} again failed: ${reason}\n`);
    if (state.suspended.get(sessionId) === entry) failScans(suspension, reason);
  }
}

/**
 * Opens `sessionId` again (`resumeSuspended`) in `RESUME_RETRY_MS`, once however often it is asked meanwhile: its open
 * met another process's write lock. A session no longer suspended by then is left alone; the service's close drops the
 * timer.
 */
function retryResume(state: EngineRoutingState, sessionId: string, now: () => number): void {
  if (state.resumeRetries.has(sessionId)) return;
  const timer = setTimeout(() => {
    state.resumeRetries.delete(sessionId);
    void resumeSuspended(state, sessionId, now);
  }, RESUME_RETRY_MS);
  timer.unref();
  state.resumeRetries.set(sessionId, timer);
}

/**
 * Ends the suspended session of `entry`, which can no longer open again for `reason` (`unopenable`), at the time it went
 * quiet, as an idle close that ends it would have, once its queued scans have failed, saying why. Its next event, should
 * one come, opens it again.
 */
function endUnopened(state: EngineRoutingState, entry: SuspendedEntry, reason: string): void {
  const { suspension } = entry;
  state.suspended.delete(suspension.descriptor.sessionId);
  failScans(suspension, reason);
  endEntries([entry]);
}

/** Fails the scans queued for a suspended session that could not be opened again (`failQueuedScans`); a failure is reported. */
function failScans(suspension: SessionSuspension, reason: string): void {
  try {
    failQueuedScans(suspension, reason);
  } catch (error) {
    process.stderr.write(`rerail: failing the scans of quiet session ${suspension.descriptor.sessionId} failed: ${error instanceof Error ? error.message : String(error)}\n`);
  }
}

/**
 * Ends every session still suspended as the service closes, as closing a runtime ends each session open in it: a
 * suspended session is live only while its daemon runs. Each record ends when its session was suspended, the last the
 * daemon heard of it.
 */
function endSuspended(state: EngineRoutingState): void {
  const entries = [...state.suspended.values()];
  state.suspended.clear();
  endEntries(entries);
}

/**
 * Ends `sessionId`'s record when it is still suspended after its end came, which its runtime did not open it for: at
 * the time the end came, as the runtime ends an open session's.
 */
function endIfSuspended(state: EngineRoutingState, sessionId: string): void {
  const entry = state.suspended.get(sessionId);
  if (entry === undefined) return;
  state.suspended.delete(sessionId);
  endEntries([entry], Date.now());
}

/**
 * Ends the suspended sessions of `entries` (`endSuspendedSessions`), each in the database that records it, at `endedAt`
 * or else when each was suspended; a failure is reported.
 */
function endEntries(entries: readonly SuspendedEntry[], endedAt?: number): void {
  for (const [databasePath, grouped] of byDatabase(entries)) {
    try {
      endSuspendedSessions(databasePath, grouped.map(entry => entry.suspension), endedAt);
    } catch (error) {
      process.stderr.write(`rerail: ending quiet sessions failed: ${error instanceof Error ? error.message : String(error)}\n`);
    }
  }
}

/** `entries` by the database that records each one's session. */
function byDatabase(entries: readonly SuspendedEntry[]): Map<string, SuspendedEntry[]> {
  const grouped = new Map<string, SuspendedEntry[]>();
  for (const entry of entries) {
    const { databasePath } = entry.suspension;
    grouped.set(databasePath, [...(grouped.get(databasePath) ?? []), entry]);
  }
  return grouped;
}

function runtimeInUse(state: EngineRoutingState, runtimeKey: string): boolean {
  for (const keys of state.runtimeKeysBySession.values()) if (keys.has(runtimeKey)) return true;
  return false;
}

/**
 * Disposes the runtime at `runtimeKey` once no session has used it for `state.idleRuntimeMs`. A runtime holds its
 * checkout's whole source walk (one observation for every file of the working tree, ignored ones included, about
 * 100,000 in a large repository) and its compiled rules, so a daemon that kept the runtime of every checkout and
 * worktree it ever served grew until its heap ran out. The next event for that checkout makes a new runtime, as the
 * first one did.
 */
function retireUnusedRuntime(state: EngineRoutingState, runtimeKey: string): void {
  if (runtimeInUse(state, runtimeKey) || !state.runtimes.has(runtimeKey) || state.idle.has(runtimeKey)) return;
  if (state.idleRuntimeMs <= 0) {
    disposeRuntime(state, runtimeKey);
    return;
  }
  const timer = setTimeout(() => {
    state.idle.delete(runtimeKey);
    if (!runtimeInUse(state, runtimeKey)) disposeRuntime(state, runtimeKey);
  }, state.idleRuntimeMs);
  timer.unref();
  state.idle.set(runtimeKey, timer);
}

/**
 * A full collection, exposing `gc` for the call. Once a runtime closed it lets the engine free the programs of the rules
 * it compiled. They are native memory V8 does not count (512 MB for the Node pack's builtins rule alone), and a daemon
 * with no session open allocates too little to start a collection: the closed runtime's rules stayed in memory until the
 * next session came. The heap watch uses it to tell garbage from a live heap.
 */
export function collectGarbage(): void {
  setFlagsFromString("--expose-gc");
  const gc: unknown = runInNewContext("gc");
  if (typeof gc === "function") gc();
}

function disposeRuntime(state: EngineRoutingState, runtimeKey: string): void {
  const runtime = state.runtimes.get(runtimeKey);
  if (runtime === undefined) return;
  state.runtimes.delete(runtimeKey);
  state.runtimeOptions.delete(runtimeKey);
  for (const [configuration, key] of state.runtimeKeysByConfiguration) if (key === runtimeKey) state.runtimeKeysByConfiguration.delete(configuration);
  const disposal = runtime.dispose().then(() => {
    collectGarbage();
    return [] as const;
  }, (error: unknown) => {
    process.stderr.write(`rerail: closing an idle runtime failed: ${error instanceof Error ? error.message : String(error)}\n`);
    return [] as const;
  });
  state.pending.add(disposal);
  void disposal.then(() => state.pending.delete(disposal));
}

/**
 * Closes every runtime, each whatever another's close does, and once all have settled rejects with the first failure:
 * one failure no longer settles the service's close while the others still close their sessions.
 */
async function disposeAll(runtimes: Iterable<RerailRuntime>): Promise<void> {
  const closed = await Promise.allSettled([...runtimes].map(runtime => runtime.dispose()));
  const failed = closed.find((result): result is PromiseRejectedResult => result.status === "rejected");
  if (failed !== undefined) throw failed.reason;
}

/**
 * How far apart the close's looks for stalled requests are: a request that has not all arrived is cut off once it has
 * sent nothing from one look to the next, so after one to two of these with nothing sent.
 */
const STALLED_REQUEST_MS = 1_000;

/**
 * While the service closes, cuts off each connection still sending a request ({@link awaitsRequest}) once two looks
 * `STALLED_REQUEST_MS` apart find it has sent nothing between them. A hook writes its whole request at once,
 * so one still arriving has stalled, and waiting for it would hold the close with no limit: `server.close()` stops the
 * check that enforces the server's request and headers timeouts. A connection that owes an answer to a request whose
 * body has arrived is never cut, whatever came in after that request: its event runs and its hook gets the whole
 * answer; a next request that stalls is cut after that answer. Idle connections are left to `server.close()`, which
 * ends them once as the close begins: Node counts a connection whose answer is ended but still being written as idle,
 * so ending idle ones at each look would cut an answer larger than the socket's buffer. Each look runs after the event
 * loop has read what arrived meanwhile (`setImmediate`), so a loop kept busy past the wait cuts off nothing that came
 * in time. Returns the function that stops the looks.
 */
function cutStalledRequests(connections: ReadonlyMap<Socket, ConnectionState>): () => void {
  let seen = new Map<Socket, number>();
  const look = (): void => {
    const now = new Map<Socket, number>();
    for (const [socket, state] of connections) {
      if (!awaitsRequest(socket, state)) continue;
      if (seen.get(socket) === socket.bytesRead) socket.destroy();
      else now.set(socket, socket.bytesRead);
    }
    seen = now;
  };
  look();
  const looks = setInterval(() => setImmediate(look), STALLED_REQUEST_MS);
  return () => clearInterval(looks);
}

/**
 * What the close knows of an open connection: `undefined` until a request's headers have arrived; then the oldest
 * request whose answer is not all written, while there is one (a client can send its next request before it has the
 * answer to the one ahead); then the last request, or, once that request had all arrived when its answer was written,
 * how many bytes the connection had read by then.
 */
export type ConnectionState = IncomingMessage | number | undefined;

/**
 * Whether `socket` is still sending a request: it has sent none yet, or one whose body has not all arrived, or, after
 * the answer to a request that had all arrived, on a connection its client keeps open, bytes of a next request whose
 * headers have not all arrived. So a next request that stalls in its headers is cut off as a first one is. A kept
 * connection that sends nothing after such an answer is not: Node's keep-alive timeout ends it about 6 s after that
 * answer, as when the close made no looks, and a whole request its client sends there meanwhile is answered. So such a
 * connection answered more than 4 s into the close holds it past the 10 s `rerail daemon stop` gives before its kill.
 * Part of a next request's headers that came before the answer was written (a client that sends its next request
 * without waiting for the answer) counts with the answered request, so that connection is left to the same timeout.
 * A request answered on its headers alone (a wrong path, a refused access token, the capability proof) keeps its
 * entry, since the rest of its body is not a next request: it is cut while that body stalls, and once the body has
 * come it is left to the same timeout. A connection that owes an answer to a request whose body has arrived counts as
 * not sending one, whatever came in after that request: its entry is that request until the answer is written.
 */
function awaitsRequest(socket: Socket, state: ConnectionState): boolean {
  if (typeof state === "number") return socket.bytesRead > state;
  return state?.complete !== true;
}

/**
 * Each open connection of `server`, with its `ConnectionState`, which `cutStalledRequests` reads. A connection leaves it
 * when it closes, so a daemon does not keep every hook's socket for its life.
 */
export function trackConnections(server: Server): ReadonlyMap<Socket, ConnectionState> {
  const connections = new Map<Socket, ConnectionState>();
  // Each connection's requests whose answers are not all written yet, oldest first: Node writes a connection's answers
  // in the order its requests came.
  const unanswered = new WeakMap<Socket, IncomingMessage[]>();
  server.on("connection", (socket: Socket) => {
    connections.set(socket, undefined);
    socket.once("close", () => connections.delete(socket));
  });
  server.on("request", (incoming: IncomingMessage, response: ServerResponse) => {
    const { socket } = incoming;
    const waiting = unanswered.get(socket) ?? [];
    unanswered.set(socket, waiting);
    waiting.push(incoming);
    // A request that comes in while an answer is still owed on its connection waits for its turn.
    if (waiting.length === 1) connections.set(socket, incoming);
    response.once("finish", () => {
      if (!connections.has(socket)) return;
      waiting.shift();
      const next = waiting[0];
      if (next !== undefined) connections.set(socket, next);
      // Left as it is when this request's body has not all arrived (an answer on its headers alone), whose rest would
      // read as a next request.
      else if (incoming.complete) connections.set(socket, socket.bytesRead);
    });
  });
  return connections;
}

function listen(server: Server, socketPath: string): Promise<void> {
  const listening = Promise.withResolvers<void>();
  server.once("error", listening.reject);
  server.listen(socketPath, () => {
    server.off("error", listening.reject);
    listening.resolve();
  });
  return listening.promise;
}

/** Whether a process accepts connections on the Unix socket at `socketPath`; false when the file is a dead daemon's. */
async function socketAnswers(socketPath: string): Promise<boolean> {
  const probed = Promise.withResolvers<boolean>();
  const probe = connect(socketPath);
  probe.once("connect", () => {
    probe.destroy();
    probed.resolve(true);
  });
  probe.once("error", (error: NodeJS.ErrnoException) => {
    if (error.code === "ECONNREFUSED" || error.code === "ENOENT") probed.resolve(false);
    else probed.reject(error);
  });
  return probed.promise;
}

/**
 * Whether this home's engine service answers on the pipe `socketPath`: it proves it holds `accessToken` by answering a
 * challenge (`engineHoldsCapability`), which never shows the holder the capability. Any local user can create the pipe
 * name first, and a probe that presented the capability would hand it to them. Without a capability to check, any
 * holder counts as this home's.
 */
async function ownEngineAnswers(socketPath: string, accessToken: string | undefined): Promise<boolean> {
  return accessToken === undefined || await engineHoldsCapability(socketPath, accessToken);
}

/** Answers a challenge sent to {@link ENGINE_PROOF_PATH} with the proof that this engine holds `accessToken`. */
function answerEngineChallenge(challenge: unknown, accessToken: string, response: ServerResponse): void {
  if (!isEngineChallenge(challenge)) {
    response.writeHead(400).end(JSON.stringify({ error: "engine-challenge-required" }));
    return;
  }
  response.end(JSON.stringify({ proof: engineProof(accessToken, challenge) }));
}

/** What listening beside a held pipe needs; tests pass their own. */
export interface HeldPipeListening {
  listen: (socketPath: string) => Promise<void>;
  /** Whether this home's engine service answers on the held pipe. */
  ownEngineAnswers: (socketPath: string) => Promise<boolean>;
  fallback: (() => string) | undefined;
}

/**
 * The codes a listen on a Windows pipe name fails with when another process holds the name. libuv creates a pipe's first
 * instance alone, and reports a name whose holder may make more instances as EACCES or EADDRINUSE, and one whose holder
 * capped its instances and has made them all as EBUSY.
 */
const HELD_PIPE_CODES: ReadonlySet<string | undefined> = new Set(["EACCES", "EADDRINUSE", "EBUSY"]);

/** The code of the error a listen failed with. */
function listenCode(cause: unknown): string | undefined {
  return (cause as NodeJS.ErrnoException | undefined)?.code;
}

/**
 * Where to listen when the listen on `pipe` failed with `cause`: on the fallback pipe when another process holds the name
 * ({@link HELD_PIPE_CODES}) and is not this home's engine service, which a second daemon must never start beside. Any
 * other failure, and a held name with no fallback, is the start's own. Returns the pipe it listens on. A holder that
 * capped the name's instances (EBUSY) is not asked for the proof: a Node pipe server, as every Rerail engine is, never
 * caps them, and the ask would take the holder's one free instance or wait for one it never frees.
 */
export async function listenBesideHeldPipe(pipe: string, cause: unknown, listening: HeldPipeListening): Promise<string> {
  if (listening.fallback === undefined || !HELD_PIPE_CODES.has(listenCode(cause))) throw cause;
  if (listenCode(cause) !== "EBUSY" && await listening.ownEngineAnswers(pipe)) {
    throw new Error(`Rerail's engine pipe ${pipe} is in use by a running daemon; stop it with \`rerail daemon stop\` before starting another.`, { cause });
  }
  const fallback = listening.fallback();
  await listening.listen(fallback);
  return fallback;
}

/**
 * Listens on `socketPath`, and returns where it listens. A daemon killed without cleanup (`kill -9`, a crash) leaves
 * its Unix socket file behind, and binding over it fails with EADDRINUSE: when no process answers on that socket, the
 * file is removed and the bind tried once more; when one does, the error says a live daemon owns it. Windows named
 * pipes leave no file to clear; a held one moves to `pipeFallback` (`listenBesideHeldPipe`).
 */
async function listenOnSocket(
  server: Server,
  socketPath: string,
  options: Pick<EngineServiceOptions, "accessToken" | "pipeFallback">,
): Promise<string> {
  try {
    await listen(server, socketPath);
    return socketPath;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (isNamedPipe(socketPath)) {
      return await listenBesideHeldPipe(socketPath, error, {
        listen: async path => await listen(server, path),
        ownEngineAnswers: async path => await ownEngineAnswers(path, options.accessToken),
        fallback: options.pipeFallback,
      });
    }
    if (code !== "EADDRINUSE") throw error;
    // Only a socket is a dead daemon's to clear: anything else at the path is left alone, whatever a probe would say.
    if (!(await lstat(socketPath)).isSocket()) {
      throw new Error(`Rerail's engine socket path ${socketPath} holds something that is not a socket; move it aside before starting the daemon.`, { cause: error });
    }
    if (await socketAnswers(socketPath)) {
      throw new Error(`Rerail's engine socket ${socketPath} is in use by a running daemon; stop it with \`rerail daemon stop\` before starting another.`, { cause: error });
    }
    await unlink(socketPath).catch((unlinkError: NodeJS.ErrnoException) => {
      if (unlinkError.code !== "ENOENT") throw unlinkError;
    });
    await listen(server, socketPath);
    return socketPath;
  }
}

/**
 * The walks of the repositories' other worktrees, which read only those the engine's file-system watch says may have
 * changed. An engine built before that watch has none: every walk then reads every worktree, and the log says so once.
 * `close` releases the watch.
 */
export function engineWorktreeWalks(): RepositoryWorktreeWalks {
  let treeWatch: TreeWatch | undefined;
  try {
    treeWatch = nativeTreeWatch();
  } catch (error) {
    process.stderr.write(`rerail: the engine's file-system watch failed to start: ${error instanceof Error ? error.message : String(error)}\n`);
    return new RepositoryWorktreeWalks();
  }
  if (treeWatch === undefined) {
    process.stderr.write("rerail: the engine is older than Rerail and has no file-system watch: every walk reads every other worktree\n");
  }
  return new RepositoryWorktreeWalks({ treeWatch });
}

/**
 * Routes a hook's event and writes its answer to `response`. Any event but a session's end is answered in its session's
 * lane, so the steers an answer its hook no longer waited for took are the session's again before its next event runs
 * (`routeEngineEvent`). A session's end is answered once the service has let go of the session (`afterSessionEnd`), as
 * the hook that ends it may read what that left: there is no later event of the session to carry a steer.
 */
async function answerEngineEvent(
  routed: { event: AdapterEvent; configuration: EngineRuntimeConfiguration | undefined; options: EngineServiceOptions; routing: EngineRoutingState },
  response: ServerResponse,
): Promise<void> {
  const { event, configuration, options, routing } = routed;
  const answer: EngineAnswer = deliveries => answered(response, encodeRerailEventResult({ eventId: event.eventId, deliveries: [...deliveries] }));
  if (event.kind !== "session-end") {
    await routeEngineEvent(event, configuration, options, routing, answer);
    return;
  }
  answer(await routeEngineEvent(event, configuration, options, routing));
}

/** Answers the challenge a hook sends to learn the engine holds the daemon's token (`ENGINE_PROOF_PATH`); false for any other request. */
function answeredProof(incoming: IncomingMessage, response: ServerResponse, accessToken: string | undefined): boolean {
  if (incoming.method !== "POST" || incoming.url !== ENGINE_PROOF_PATH || accessToken === undefined) return false;
  answerEngineChallenge(incoming.headers[ENGINE_CHALLENGE_HEADER], accessToken, response);
  incoming.resume();
  return true;
}

/**
 * Refuses, before its body is read, a request that is no event (404) or an event without the daemon's access token
 * (401), so nothing of it reaches a runtime; false for an event the service takes.
 */
function refusedEventRequest(incoming: IncomingMessage, response: ServerResponse, accessToken: string | undefined): boolean {
  if (incoming.method !== "POST" || incoming.url !== "/events") {
    response.writeHead(404).end();
    return true;
  }
  const presented = incoming.headers[DAEMON_ACCESS_HEADER];
  if (accessToken === undefined || sameSecret(typeof presented === "string" ? presented : undefined, accessToken)) return false;
  response.writeHead(401).end(JSON.stringify({ error: "engine-access-required" }));
  return true;
}

/** An event request's body as text; it fails past the frame limit (`RERAIL_EVENT_MAX_FRAME_BYTES`). */
async function engineRequestBody(incoming: IncomingMessage): Promise<string> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of incoming) {
    const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    size += bytes.length;
    if (size > RERAIL_EVENT_MAX_FRAME_BYTES) throw new RangeError("Kernel event exceeds frame limit");
    chunks.push(bytes);
  }
  return Buffer.concat(chunks).toString("utf8");
}

export async function startEngineService(options: EngineServiceOptions): Promise<EngineService> {
  if (!isNamedPipe(options.socketPath)) await preparePrivateDirectory(dirname(options.socketPath));
  const databasePath = resolvePath(options.databasePath ?? defaultRerailDatabasePath());
  // Before any runtime registers an owner: every row this service's runtimes write names it.
  const ownerLock = holdOwnerLock(databasePath);
  const routing: EngineRoutingState = {
    runtimes: new Map(),
    runtimeKeysBySession: new Map(),
    runtimeKeysByConfiguration: new Map(),
    lastEventAt: new Map(),
    lanes: new SessionLanes(),
    pending: new Set(),
    idle: new Map(),
    idleRuntimeMs: options.idleRuntimeMs ?? IDLE_RUNTIME_MS,
    runtimeOptions: new Map(),
    suspended: new Map(),
    resumeRetries: new Map(),
    ownerLockId: ownerLock.id,
    databasePath,
    worktreeWalks: options.worktreeWalks ?? engineWorktreeWalks(),
  };
  // Walks a caller gave are the caller's to close.
  const closeWalks = options.worktreeWalks === undefined ? () => routing.worktreeWalks.close() : () => undefined;
  const sweepIdleSessions = (): Promise<void> => sweepIdle(routing, options, databasePath);
  const server = createServer(async (incoming, response) => {
    response.setHeader("Content-Type", "application/json");
    if (answeredProof(incoming, response, options.accessToken) || refusedEventRequest(incoming, response, options.accessToken)) return;
    try {
      const { event, configuration } = parseEngineRequest(JSON.parse(await engineRequestBody(incoming)));
      await answerEngineEvent({ event, configuration, options, routing }, response);
    } catch (error) {
      response.writeHead(400).end(JSON.stringify({ error: error instanceof Error ? error.message : String(error) }));
    }
  });
  let socketPath = options.socketPath;
  const connections = trackConnections(server);
  const engineService: EngineService = {
    get socketPath() {
      return socketPath;
    },
    ingest: (event, configuration) => routeEngineEvent(event, configuration, options, routing),
    openRuntimes: () => routing.runtimes.size,
    closeIdleSessions: sweepIdleSessions,
    resumeSession: sessionId => resumeSuspended(routing, sessionId, options.now ?? Date.now),
    async close() {
      clearInterval(idleSweep);
      for (const timer of routing.resumeRetries.values()) clearTimeout(timer);
      routing.resumeRetries.clear();
      try {
        // Every hook whose request has arrived gets its answer, with the block or steer in it, and its event is recorded,
        // the ones waiting behind a running event too. The close ends once the last of those connections has: its server
        // takes no new ones, and a request that stalls is cut off.
        const closed = Promise.withResolvers<void>();
        server.close((error) => (error ? closed.reject(error) : closed.resolve()));
        const stopLooking = cutStalledRequests(connections);
        try {
          await closed.promise;
          for (const timer of routing.idle.values()) clearTimeout(timer);
          routing.idle.clear();
          await Promise.allSettled(routing.pending);
        } finally {
          stopLooking();
        }
        await disposeAll(routing.runtimes.values());
      } finally {
        // Also when the server's or a runtime's close failed: a suspended session left open would read as live with no
        // daemon. Its lock goes once its sessions have ended, and its file stays, so a later sweep ends any session a
        // failed close left open (`OwnerLock.release`). The engine's watch thread and its handles go however the sessions'
        // close went.
        try {
          endSuspended(routing);
        } finally {
          try {
            closeWalks();
          } finally {
            ownerLock.release();
          }
        }
      }
    },
  };
  server.requestTimeout = 30_000;
  // Never keeps the process alive; `closeIdleSessions` reports its own failures.
  const idleSweep = setInterval(() => void sweepIdleSessions(), IDLE_SWEEP_MS);
  idleSweep.unref();
  try {
    socketPath = await listenOnSocket(server, options.socketPath, options);
  } catch (error) {
    clearInterval(idleSweep);
    server.close();
    try {
      await Promise.all([...routing.runtimes.values()].map(runtime => runtime.dispose()));
    } finally {
      closeWalks();
      ownerLock.release();
    }
    throw error;
  }
  // In the same turn as the listen, before any request is read: a session this service opens is its own.
  endAbandonedSessions(databasePath, options.idleSessionMs ?? IDLE_SESSION_MS, options.now ?? Date.now);
  return engineService;
}

/**
 * Ends the sessions a daemon that ended without its stop left open in the service's database: one stopped by a machine
 * restart, its heap limit or a kill ends none of the sessions it held or had suspended while quiet, and no later daemon
 * holds them, so each read as open for good, and as owned while another process held its pid. Each ends at the idle
 * limit after it was last heard from, as the idle close would have ended it, and at the latest now
 * (`SqliteAuditControlStore.endSessionsOfEndedOwners`); then the lock files of the services that ended are removed,
 * each once no open session's row names it (`forgetFreedOwnerLocks`). A database not made yet holds none. A failure is
 * reported, and the service runs all the same.
 */
function endAbandonedSessions(databasePath: string, idleMs: number, now: () => number): void {
  if (!existsSync(databasePath)) return;
  let store: SqliteAuditControlStore | undefined;
  try {
    const controls = new SqliteAuditControlStore(databasePath);
    store = controls;
    controls.endSessionsOfEndedOwners(idleMs, now());
    forgetFreedOwnerLocks(databasePath, lockId => controls.namesOwnerLock(lockId));
  } catch (error) {
    process.stderr.write(`rerail: ending the sessions a stopped daemon left open failed: ${error instanceof Error ? error.message : String(error)}\n`);
  } finally {
    store?.close();
  }
}

/**
 * Writes `body` as the answer and says whether it reaches a client that still waits for it, decided by what the
 * connection does, never by a time limit. Refused when the connection had already closed or ended (a hook past its own
 * limit closes it), or the write failed at once (EPIPE: the daemon wrote before it read the hook's close). Else heard
 * once the answer is all handed to the connection (`finish`), and not when the connection closes first. An answer queued
 * behind an earlier one on its connection is written on it once that one is.
 */
function answered(response: ServerResponse, body: string): AnswerFate {
  const connection = response.req.socket;
  // Closed or ended before the write: a hook past its own limit closed it.
  const gone = !connection.writable;
  const heard = new Promise<boolean>(done => {
    const onClose = (): void => done(false);
    connection.once("close", onClose);
    response.once("finish", () => {
      // A connection carries many answers: each leaves nothing on it once written.
      connection.off("close", onClose);
      done(true);
    });
  });
  response.end(body);
  // A write the closed connection refused leaves its error on the connection in this turn, and no finish follows.
  return { refused: gone || connection.errored !== null, heard };
}

export async function ingestEngineEvent(
  endpoint: EngineEndpoint,
  event: AdapterEvent,
  configuration?: EngineRuntimeConfiguration,
): Promise<readonly AdapterDelivery[]> {
  const body = JSON.stringify(RequestSchema.parse({ event, configuration }));
  if (Buffer.byteLength(body) > RERAIL_EVENT_MAX_FRAME_BYTES) throw new RangeError("Kernel event exceeds frame limit");
  const { promise, resolve, reject } = Promise.withResolvers<readonly AdapterDelivery[]>();
  const outgoing = request(
    {
      socketPath: endpoint.socketPath,
      method: "POST",
      path: "/events",
      agent: false,
      headers: {
        "content-type": "application/json",
        "content-length": Buffer.byteLength(body),
        ...(endpoint.accessToken === undefined ? {} : { [DAEMON_ACCESS_HEADER]: endpoint.accessToken }),
      },
    },
    (response) => {
      const chunks: Buffer[] = [];
      response.on("data", (chunk: Buffer) => chunks.push(chunk));
      response.on("error", reject);
      response.on("end", () => {
        try {
          const text = Buffer.concat(chunks).toString("utf8");
          if (response.statusCode !== 200) throw new Error(`Kernel request failed (${response.statusCode}): ${text}`);
          const result = decodeRerailEventResult(text);
          if (result.eventId !== event.eventId) throw new Error("Kernel response event identity mismatch");
          resolve(result.deliveries);
        } catch (error) {
          reject(error);
        }
      });
    },
  );
  outgoing.setTimeout(30_000, () => outgoing.destroy(new Error("Kernel request timed out")));
  outgoing.on("error", reject);
  outgoing.end(body);
  return promise;
}
