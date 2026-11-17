import type { AdapterEvent, AdapterSession, RerailEventLoop, RerailHost } from "../engine/adapter-contract.js";
import type { AdapterDelivery } from "../engine/adapter-contract.js";
import { resolveSessionPaths } from "../runtime/session-paths.js";
import { readRepositoryState } from "../presentation/session-presentation.js";
import { isRerailStarted } from "../runtime/repository-start.js";
import { RerailRuntime } from "../runtime/runtime.js";
import { engineRequestBody, ingestEngineEvent, type EngineRuntimeConfiguration } from "../runtime/engine-service.js";
import { leaveUntaken } from "../engine/untaken-events.js";
import { scrubValue } from "../engine/secret-scrub.js";
import { WITHHELD_OUTPUT, type Withheld } from "../engine/transcript-boundary.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import {
  autostartDaemon,
  autostartOff,
  AutostartOffError,
  daemonEngineEndpoint,
  daemonLeftEventUntaken,
  daemonNotRunningMessage,
  engineEndpointAt,
  HOOK_DAEMON_WAIT_MS,
  runningDaemonEngineEndpoint,
  type DaemonState,
  type HookDaemon,
} from "../runtime/daemon-control.js";
import { recordHookFailure } from "./hook-failures.js";
import {
  bindSteerReceipt,
  withSessionLock,
  type HookHost,
  type HookOutput,
  type SessionLock,
} from "./hook-adapter.js";
import type { SemanticEmbedder } from "../engine/native/semantic-types.js";
import { SqliteSessionStore } from "../runtime/session-store.js";
import { defaultRerailDatabasePath, openRerailDatabase } from "../engine/storage/database.js";
import { rerailRunsIn } from "../engine/catalog/repository-model.js";
import { lookupCheckout } from "../engine/catalog/repository-registry.js";

export * from "./hook-adapter.js";

/** The repository key of the checkout `cwd` is in, read without recording anything; undefined when unknown. */
function repositoryKeyOf(cwd: string, databasePath: string): string | undefined {
  let database;
  try {
    database = openRerailDatabase(databasePath, { readonly: true });
    return lookupCheckout(database, cwd)?.repositoryKey;
  } catch {
    return undefined;
  } finally {
    database?.$client.close();
  }
}

function recordLocalReceipt(session: AdapterSession, databasePath: string): (receiptId: string, ruleIds: readonly string[]) => void {
  return (receiptId, ruleIds) => {
    // Attribution matters only when a reset removed the session row and this receipt restores it.
    const repositoryKey = repositoryKeyOf(session.cwd, databasePath);
    const store = new SqliteSessionStore(databasePath);
    try {
      store.recordSteer(session, receiptId, ruleIds, repositoryKey === undefined ? {} : { repositoryKey });
    } finally {
      store.close();
    }
  };
}

/**
 * The check one-shot hooks make. A running daemon answers first, from its own environment and open
 * database; without one, the hook reads the database itself, which never starts a daemon. An
 * explicit database path (tests, embedded hosts) is always read directly, and so is the database while the daemon has
 * left an earlier hook's event untaken (`daemonLeftEventUntaken`): it is not reading, and the read would wait out its
 * limit for nothing.
 */
export async function hookRerailStarted(host: RerailHost, workspace: string, databasePath?: string): Promise<boolean> {
  if (databasePath === undefined && await daemonLeftEventUntaken() === undefined) {
    const answered = await readRepositoryState(workspace, host);
    if (answered !== null) return rerailRunsIn(answered);
  }
  return isRerailStarted(workspace, databasePath);
}

export interface HookOptions {
  storageRoot?: string;
  modelPath?: string;
  databasePath?: string;
  kernelSocketPath?: string;
  /** Test seam: replaces the native model so hooks can run where the addon is absent. */
  embedder?: SemanticEmbedder;
  /**
   * Epoch ms at which the hook stops waiting for a busy daemon (`withDaemonDeadline`); without one it waits the 2 s probe
   * and 10 s more from the endpoint lookup on.
   */
  daemonDeadline?: number;
  /** Epoch ms the hook's event arrived (`withDaemonDeadline`): an event left untaken is ordered by it. */
  arrivedAt?: number;
}

/**
 * `options` with the daemon deadline `HOOK_DAEMON_WAIT_MS` from now. A host's hook takes it as the event arrives, so the
 * start check's read of the daemon (`hookRerailStarted`, up to 2 s) counts toward the one wait: a busy daemon held a
 * hook 2 s for the start check and then 12 s more for the endpoint lookup.
 */
export function withDaemonDeadline(options: HookOptions): HookOptions {
  const arrivedAt = Date.now();
  return { daemonDeadline: arrivedAt + HOOK_DAEMON_WAIT_MS, arrivedAt, ...options };
}

/** No Rerail daemon could be reached: none answered and none could start, or its socket refused the hook. */
export class DaemonUnreachableError extends Error {
  override readonly name = "DaemonUnreachableError";
}

/** Socket errors that mean no daemon listens there, as against a daemon that answered with a failure. */
const NO_LISTENER = new Set(["ENOENT", "ECONNREFUSED"]);

function unreachable(error: unknown): unknown {
  const code = error instanceof Error && "code" in error ? error.code : undefined;
  return typeof code === "string" && NO_LISTENER.has(code)
    ? new DaemonUnreachableError(error instanceof Error ? error.message : String(error), { cause: error })
    : error;
}

/**
 * The hook found no daemon because `RERAIL_AUTOSTART=0` kept it from starting one. That is what the person asked for,
 * so it is not a failure; `rerail status` says the daemon is not running and why (`daemonNotRunningMessage`), and so
 * does the session's start (`answerAutostartOff`).
 */
function autostartTurnedOff(error: unknown): boolean {
  return error instanceof DaemonUnreachableError && error.cause instanceof AutostartOffError;
}

/**
 * The session-start hook's answer when `RERAIL_AUTOSTART` kept it from starting a daemon and none runs: `answer` given
 * the line `rerail status` prints (`daemonNotRunningMessage`), so the person learns as the session starts that nothing
 * is recorded or checked until the daemon runs, and how to start it. The answer rests on one lookup, and a daemon can
 * come up later in the same session (a start in flight, a restart at the heap ceiling, the person following the line)
 * and record its later events, so it says nothing of the rest of the session. A later hook can meet the same error,
 * when the daemon of another package it went to replace is gone before the replacement looks again; that hook
 * (`startsSession` false), and any other failure, is rethrown, so it answers as any failed hook does.
 */
export function answerAutostartOff<Output>(error: unknown, startsSession: boolean, answer: (line: string) => Output): Output {
  if (!startsSession || !autostartTurnedOff(error)) throw error;
  return answer(daemonNotRunningMessage());
}

/**
 * Runs a normal package hook against the engine service, or an explicit local test runtime. Only the hook that starts
 * a session (`startsSession`) waits for a daemon to start; any other hook that finds none running answers `{}` at
 * once, so a daemon that is down never stalls a tool call or shows as a hook error (design N11), records the lost event
 * in the hook failure log and starts a daemon in the background. One whose daemon runs but left its whole bounded wait
 * unanswered leaves the event in the home's untaken folder for the daemon to record when it reads again, and so does
 * every later hook while that event is still there, without asking the daemon (`leaveForDaemon`). A daemon that cannot
 * be reached throws
 * `DaemonUnreachableError`, so a coding agent without a footer can say so. Every failure, `event` named as the coding
 * agent names it, goes to the hook failure log (`hook-failures.ts`), which `rerail status` reads. Under
 * `RERAIL_AUTOSTART=0` a hook that finds no daemon logs nothing and starts none, since the person runs it themselves;
 * the session-start hook then throws, and its host answers with the line that says so (`answerAutostartOff`).
 */
export async function runHostHook(
  host: HookHost,
  workspace: string,
  sessionId: string,
  startsSession: boolean,
  options: HookOptions,
  handle: (runtime: RerailEventLoop, storageRoot: string, lock: SessionLock) => Promise<HookOutput>,
  event = "unknown",
): Promise<HookOutput> {
  try {
    return await runHostHookOnce(host, workspace, sessionId, startsSession, options, handle, event);
  } catch (error) {
    // The coding agent keeps a hook's stderr out of the session; the failure log is where `rerail status` finds it.
    if (!autostartTurnedOff(error)) recordHookFailure({ host, event, reason: error instanceof Error ? error.message : String(error) });
    throw error;
  }
}

async function runHostHookOnce(
  host: HookHost,
  workspace: string,
  sessionId: string,
  startsSession: boolean,
  options: HookOptions,
  handle: (runtime: RerailEventLoop, storageRoot: string, lock: SessionLock) => Promise<HookOutput>,
  event: string,
): Promise<HookOutput> {
  const { storageRoot, modelPath } = resolveSessionPaths(workspace, options);
  const databasePath = options.databasePath ?? defaultRerailDatabasePath();
  const session: AdapterSession = { host, sessionId, cwd: workspace };
  const { embedder } = options;
  if (embedder !== undefined) {
    return withSessionLock(storageRoot, host, sessionId, async lock => {
      const runtime = new RerailRuntime({ storageRoot, databasePath, workspaceRoot: workspace, embedder });
      try {
        const output = await handle(runtime, storageRoot, lock);
        return bindSteerReceipt(output, recordLocalReceipt(session, databasePath));
      } finally {
        await runtime.dispose();
      }
    });
  }
  const hook: HookRun = {
    host, session, event, storageRoot, databasePath, handle,
    configuration: { storageRoot, databasePath, modelPath, workspaceRoot: workspace },
    arrivedAt: options.arrivedAt ?? Date.now(),
  };
  // A daemon that left an earlier hook's event untaken has not read since that hook waited its whole wait for it: this
  // hook leaves its event behind that one at once, and waits for nothing.
  const untaken = await untakenDaemon(options, startsSession);
  if (untaken !== undefined) return await leaveForDaemon(hook, untaken);
  // Found before the session's lock is taken: a session's hooks that arrive together (parallel tool calls, subagents)
  // wait out a busy daemon side by side. Under the lock, each hook would begin its wait only when the one before it
  // gave up.
  return await answerWith(await hookDaemon(options, startsSession), hook);
}

/** What one hook run needs to hand its event on and answer its coding agent. */
interface HookRun {
  host: HookHost;
  session: AdapterSession;
  /** The hook's event as the coding agent names it, for the hook failure log. */
  event: string;
  storageRoot: string;
  databasePath: string;
  configuration: EngineRuntimeConfiguration;
  /** Epoch ms the event arrived ({@link HookOptions.arrivedAt}). */
  arrivedAt: number;
  handle: (runtime: RerailEventLoop, storageRoot: string, lock: SessionLock) => Promise<HookOutput>;
}

/**
 * The daemon a hook leaves its event for without asking it (`daemonLeftEventUntaken`). A session's start still asks:
 * it is the hook that starts a daemon and tells the person when none runs. A socket named outright is asked as named.
 */
async function untakenDaemon(options: HookOptions, startsSession: boolean): Promise<DaemonState | undefined> {
  if (startsSession || namedEngineSocket(options) !== undefined) return undefined;
  return await daemonLeftEventUntaken();
}

function namedEngineSocket(options: HookOptions): string | undefined {
  return options.kernelSocketPath ?? process.env.RERAIL_ENGINE_SOCKET;
}

/** The hook's answer once it has looked for its daemon: its event goes to the daemon, is left for it, or is lost. */
async function answerWith(daemon: HookDaemon, hook: HookRun): Promise<HookOutput> {
  if (daemon.status === "running") {
    const { endpoint } = daemon;
    // The daemon orders a session's events itself, so the lock guards only the session's files: a hook lets go of it
    // while the daemon checks its event, unless it keeps it (`SessionLock.kept`), and the hooks of a session's parallel
    // subagents, which all carry the parent's session id, do not queue behind each other's checks until one gives up.
    return await runWithLoop(hook, lock => ({
      ingest: async adapterEvent => await lock.released(() => ingestEngineEvent(endpoint, adapterEvent, hook.configuration))
        .catch((error: unknown) => {
          throw unreachable(error);
        }),
    }));
  }
  // A daemon that runs but left the hook's whole wait unanswered takes the event when it reads again.
  if (daemon.status === "unresponsive" && daemon.state.untakenFolder !== undefined) return await leaveForDaemon(hook, daemon.state);
  return lostEvent(daemon, hook);
}

/**
 * This event is lost, so say so where `rerail status` looks. A daemon that is gone (it crashed, or was stopped) is
 * started in the background for the next event; the start is handed to a detached process, so the tool call never
 * waits on it (design N11). A daemon that is alive is left alone: a start would find it and refuse to replace it, after
 * costing a Rerail process and, on Windows, loading the native engine to launch it. Under RERAIL_AUTOSTART=0 the
 * person runs the daemon themselves, so finding none is no failure and starts none.
 */
function lostEvent(daemon: Exclude<HookDaemon, { status: "running" }>, hook: HookRun): HookOutput {
  if (daemon.status === "absent" && autostartOff()) return {};
  recordHookFailure({ host: hook.host, event: hook.event, reason: lostEventReason(daemon) });
  if (daemon.status === "absent") autostartDaemon();
  return {};
}

/** Runs the hook's handler under its session's lock with the event loop that `loopFor` makes for that lock. */
async function runWithLoop(hook: HookRun, loopFor: (lock: SessionLock) => RerailEventLoop): Promise<HookOutput> {
  return withSessionLock(hook.storageRoot, hook.host, hook.session.sessionId, async lock => {
    const output = await hook.handle(loopFor(lock), hook.storageRoot, lock);
    return bindSteerReceipt(output, recordLocalReceipt(hook.session, hook.databasePath));
  });
}

/**
 * Runs the hook's handler with an `ingest` that leaves each event in the home's untaken folder for the daemon `daemon`
 * names, which runs but does not read (`untaken-events.ts`), and gives the coding agent what a daemon that holds
 * nothing for the event gives: no delivery. The daemon records the event as it reads again, before any event sent to
 * it after; it does not check it then, since the moment its finding could reach the agent has passed.
 */
async function leaveForDaemon(hook: HookRun, daemon: DaemonState): Promise<HookOutput> {
  return await runWithLoop(hook, () => ({ ingest: async adapterEvent => await leaveEvent(hook, daemon, adapterEvent), untaken: true }));
}

/**
 * Leaves `adapterEvent` for `daemon` as the daemon keeps what it records: its secrets scrubbed, and a tool's output
 * withheld, since the hook cannot tell whether the call named a secret-like path, whose output the daemon never writes
 * down. An event that cannot be left is lost, and the failure log says so.
 */
async function leaveEvent(hook: HookRun, daemon: DaemonState, adapterEvent: AdapterEvent): Promise<readonly AdapterDelivery[]> {
  try {
    await leaveUntaken(defaultRerailHome(), daemon.pid, engineRequestBody(untakenForm(adapterEvent), hook.configuration), hook.arrivedAt);
  } catch {
    recordHookFailure({ host: hook.host, event: hook.event, reason: lostEventReason({ status: "unresponsive", state: daemon }) });
  }
  return [];
}

/** What a tool's result keeps of its output in the untaken folder: none. */
const UNTAKEN_OUTPUT: Readonly<Withheld> = {
  ...WITHHELD_OUTPUT,
  reason: "the daemon was not reading when the tool's result came, and Rerail writes no tool output to disk before the daemon has seen the call",
};

/** `event` as the untaken folder keeps it: scrubbed of secrets (`scrubValue`), and a result without its output. */
function untakenForm(event: AdapterEvent): AdapterEvent {
  const scrubbed = scrubValue(event);
  return scrubbed.kind === "result" ? { ...scrubbed, output: UNTAKEN_OUTPUT } : scrubbed;
}

/** The engine a hook sends its event to: the socket named outright, else the daemon's, or why no daemon takes it. */
async function hookDaemon(options: HookOptions, startsSession: boolean): Promise<HookDaemon> {
  try {
    const named = namedEngineSocket(options);
    if (named !== undefined) return { status: "running", endpoint: await engineEndpointAt(named) };
    const { daemonDeadline } = options;
    if (startsSession) return { status: "running", endpoint: await daemonEngineEndpoint(true, daemonDeadline) };
    return await runningDaemonEngineEndpoint(daemonDeadline);
  } catch (error) {
    throw new DaemonUnreachableError(error instanceof Error ? error.message : String(error), { cause: error });
  }
}

/** What the hook failure log says of an event no daemon took. */
function lostEventReason(daemon: Exclude<HookDaemon, { status: "running" }>): string {
  const lost = "so this event was not recorded";
  if (daemon.status === "absent") return `no Rerail daemon answered, ${lost}`;
  if (daemon.status === "refused") return `the Rerail daemon (pid ${daemon.state.pid}) refused the access capability in daemon.json, ${lost}`;
  return `the Rerail daemon (pid ${daemon.state.pid}) is running but did not answer within ${HOOK_DAEMON_WAIT_MS / 1_000} s, ${lost}`;
}
