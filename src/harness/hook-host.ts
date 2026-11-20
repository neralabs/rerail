import type { AdapterDelivery, AdapterEvent, AdapterSession, EventAnswerFacts, RerailEventLoop, RerailHost } from "../engine/adapter-contract.js";
import { resolveSessionPaths } from "../runtime/session-paths.js";
import { readRepositoryState } from "../presentation/session-presentation.js";
import { isRerailStarted } from "../runtime/repository-start.js";
import { RerailRuntime } from "../runtime/runtime.js";
import { engineLink, engineRequestBody, type EngineLink, type EngineRuntimeConfiguration } from "../runtime/engine-service.js";
import { RERAIL_EVENT_MAX_FRAME_BYTES } from "../runtime/event-protocol.js";
import { leaveUntaken, UntakenFolderFullError } from "../engine/storage/untaken-events.js";
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
  runningDaemonEngineEndpoint,
  type DaemonState,
  type HookDaemon,
} from "../runtime/daemon-control.js";
import { HOOK_DAEMON_WAIT_MS } from "../runtime/daemon/daemon-waits.js";
import { recordHookFailure } from "./hook-failures.js";
import {
  bindSteerReceipt,
  withSessionLock,
  type HookHost,
  type HookOutput,
  type SessionLock,
} from "./hook-adapter.js";
import type { SemanticEmbedder } from "../engine/native/semantic-types.js";
import { steerReceiptWrite } from "../runtime/session-store.js";
import type { SteerReceipt } from "../runtime/event-protocol.js";
import { openRecordWriter } from "../engine/storage/record-writes.js";
import { defaultRerailDatabasePath } from "../engine/storage/database.js";
import { rerailRunsIn } from "../engine/catalog/repository-model.js";

export * from "./hook-adapter.js";

/**
 * Records the receipt of a steer the hook's answer carried, once that answer is written, never holding the hook on
 * another connection's write lock longer than its own record did. While the lock is free the hook records it at once
 * (`RecordWriter.writeNow`), as before. While another connection holds it, the hook hands the receipt to the daemon
 * that answered the event, which keeps it until the lock is given back, and meanwhile tries the lock again as SQLite's
 * busy wait did (`RecordWriter.writeWhenFree`): the hook ends once the daemon takes the receipt, once a try records it,
 * or at the busy timeout. A daemon that does not take it (one from before receipts were handed over, one gone) leaves
 * the tries to run, as the hook's own wait did.
 */
async function recordReceipt(link: EngineLink, configuration: EngineRuntimeConfiguration, databasePath: string, receipt: SteerReceipt): Promise<void> {
  const write = steerReceiptWrite(databasePath, receipt);
  const writer = openRecordWriter(databasePath);
  const stop = new AbortController();
  try {
    if (writer.writeNow(write)) return;
    const written = writer.writeWhenFree(write, stop.signal);
    await Promise.race([written, link.handReceipt(receipt, configuration, stop.signal).then(taken => taken || written)]);
  } finally {
    stop.abort();
    writer.close();
  }
}

/**
 * Records the receipt of a steer the answer of a hook that runs its own runtime (tests, embedded hosts) carried, once
 * that answer is written: through the record writer, whose use's close makes it, waiting for another connection's write
 * lock no longer than the busy timeout, as the hook's own record always did.
 */
function recordOwnReceipt(databasePath: string, receipt: SteerReceipt): void {
  const writer = openRecordWriter(databasePath);
  try {
    writer.write(steerReceiptWrite(databasePath, receipt));
  } finally {
    writer.close();
  }
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
 * start check's read of the daemon (`hookRerailStarted`: up to 2 s, and on Windows up to 30 s more while every instance
 * of a busy engine pipe is taken) counts toward the one wait: a busy daemon held a hook 2 s for the start check and then
 * 12 s more for the endpoint lookup.
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
 * unanswered leaves the event in the home's untaken folder for the daemon to check when it reads again, and so does
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
        return bindSteerReceipt(output, (receiptId, ruleIds) => recordOwnReceipt(databasePath, { session, receiptId, ruleIds: [...ruleIds] }));
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
  if (daemon.status === "running") return await runWithDaemon(hook, engineLink(daemon.endpoint));
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

/** Runs the hook's handler under its session's lock with the daemon at `link` as its event loop. */
async function runWithDaemon(hook: HookRun, link: EngineLink): Promise<HookOutput> {
  const { session, storageRoot, databasePath, configuration } = hook;
  return withSessionLock(storageRoot, hook.host, session.sessionId, async lock => {
    // The daemon orders a session's events itself, so the lock guards only the session's files: a hook lets go of it
    // while the daemon checks its event, unless it keeps it (`SessionLock.kept`), and the hooks of a session's parallel
    // subagents, which all carry the parent's session id, do not queue behind each other's checks until one gives up.
    // Each answer's facts are kept for the adapter, which then asks the daemon for none of them again.
    let facts: EventAnswerFacts = {};
    const ingest = (adapterEvent: AdapterEvent) => lock.released(() => link.ingest(adapterEvent, configuration))
      .then(({ eventId: _eventId, deliveries, ...answered }) => {
        facts = { ...facts, ...answered };
        return deliveries;
      })
      .catch((error: unknown) => {
        throw unreachable(error);
      });
    const output = await hook.handle({ ingest, answerFacts: () => facts }, storageRoot, lock);
    return bindSteerReceipt(output, (receiptId, ruleIds) => recordReceipt(link, configuration, databasePath, { session, receiptId, ruleIds: [...ruleIds] }));
  });
}

/**
 * Runs the hook's handler with an `ingest` that leaves each event in the home's untaken folder for the daemon `daemon`
 * names, which runs but does not read (`untaken-events.ts`), and gives the coding agent the answer of a hook with
 * nothing to deliver: the handler touches none of the session's files then (`RerailEventLoop.untaken`) but one, which
 * it locks for that write alone once its event is left, so the hook holds no session lock through its run, waits for
 * another hook of its session only for that write, and loses no event to a lock it is not given. The daemon checks the
 * event as it reads again, before any event sent to it after, and what that check finds reaches the agent with the
 * session's next answer.
 */
async function leaveForDaemon(hook: HookRun, daemon: DaemonState): Promise<HookOutput> {
  return await hook.handle({ ingest: async adapterEvent => await leaveEvent(hook, daemon, adapterEvent), untaken: true }, hook.storageRoot, NO_LOCK);
}

/**
 * The lock of a hook that leaves its event: it holds nothing, as such a hook touches none of its session's files but
 * the one it locks for itself (`RerailEventLoop.untaken`).
 */
const NO_LOCK: SessionLock = {
  released: async work => await work(),
  kept: async work => await work(),
};

/**
 * Leaves `adapterEvent` for `daemon` as the daemon keeps what it records: its secrets scrubbed, and a tool's output
 * withheld, since the hook cannot tell whether the call named a secret-like path, whose output the daemon never writes
 * down. An event that cannot be left is lost, and the failure log says why.
 */
async function leaveEvent(hook: HookRun, daemon: DaemonState, adapterEvent: AdapterEvent): Promise<readonly AdapterDelivery[]> {
  const lost = await leftOrWhyNot(hook, daemon, adapterEvent);
  if (lost !== undefined) {
    recordHookFailure({ host: hook.host, event: hook.event, reason: `the Rerail daemon (pid ${daemon.pid}) is running but not reading, and ${lost}, so this event was not recorded` });
  }
  return [];
}

/**
 * Leaves the event, or says why it could not: one an engine request cannot carry (`engineRequestBody` refuses it, as
 * the daemon would), a folder at its bound, or a write that failed. Said of the event itself, since a hook that leaves
 * its event behind an earlier one waited for nothing.
 */
async function leftOrWhyNot(hook: HookRun, daemon: DaemonState, adapterEvent: AdapterEvent): Promise<string | undefined> {
  try {
    await leaveUntaken(defaultRerailHome(), daemon.pid, engineRequestBody(untakenForm(adapterEvent), hook.configuration), hook.arrivedAt);
    return undefined;
  } catch (error) {
    return whyNotLeft(error);
  }
}

/** Why an event could not be left: too large for one engine request, the folder at its bound, or the error itself. */
function whyNotLeft(error: unknown): string {
  if (error instanceof RangeError) return `the event is larger than the ${RERAIL_EVENT_MAX_FRAME_BYTES / (1024 * 1024)} MiB one event may hold`;
  if (error instanceof UntakenFolderFullError) return error.message;
  return `the event could not be left for it (${error instanceof Error ? error.message : String(error)})`;
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
