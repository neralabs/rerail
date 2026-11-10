import type { AdapterEvent, AdapterSession, RerailEventLoop, RerailHost } from "../engine/adapter-contract.js";
import { resolveSessionPaths } from "../runtime/session-paths.js";
import { readRepositoryState } from "../presentation/session-presentation.js";
import { isRerailStarted } from "../runtime/repository-start.js";
import { RerailRuntime } from "../runtime/runtime.js";
import { ingestEngineEvent } from "../runtime/engine-service.js";
import {
  autostartDaemon,
  daemonEngineEndpoint,
  engineEndpointAt,
  HOOK_DAEMON_WAIT_MS,
  runningDaemonEngineEndpoint,
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
 * explicit database path (tests, embedded hosts) is always read directly.
 */
export async function hookRerailStarted(host: RerailHost, workspace: string, databasePath?: string): Promise<boolean> {
  if (databasePath === undefined) {
    const answered = await readRepositoryState(workspace, host);
    if (answered !== null) return answered !== "stopped";
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
}

/**
 * `options` with the daemon deadline `HOOK_DAEMON_WAIT_MS` from now. A host's hook takes it as the event arrives, so the
 * start check's read of the daemon (`hookRerailStarted`, up to 2 s) counts toward the one wait: a busy daemon held a
 * hook 2 s for the start check and then 12 s more for the endpoint lookup.
 */
export function withDaemonDeadline(options: HookOptions): HookOptions {
  return { daemonDeadline: Date.now() + HOOK_DAEMON_WAIT_MS, ...options };
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
 * Runs a normal package hook against the engine service, or an explicit local test runtime. Only the hook that starts
 * a session (`startsSession`) waits for a daemon to start; any other hook that finds none running answers `{}` at
 * once, so a daemon that is down never stalls a tool call or shows as a hook error (design N11), records the lost event
 * in the hook failure log and starts a daemon in the background. A daemon that cannot be reached throws
 * `DaemonUnreachableError`, so a coding agent without a footer can say so. Every failure, `event` named as the coding
 * agent names it, goes to the hook failure log (`hook-failures.ts`), which `rerail status` reads.
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
    recordHookFailure({ host, event, reason: error instanceof Error ? error.message : String(error) });
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
  // Found before the session's lock is taken: a session's hooks that arrive together (parallel tool calls, subagents)
  // wait out a busy daemon side by side. Under the lock, each hook would begin its wait only when the one before it
  // gave up.
  const daemon = await hookDaemon(options, startsSession);
  if (daemon.status !== "running") {
    // This event is lost, so say so where `rerail status` looks. A daemon that is gone (it crashed, or was stopped)
    // is started in the background for the next event; the start is handed to a detached process, so the tool call
    // never waits on it (design N11). A daemon that is alive is left alone: a start would find it and refuse to
    // replace it, after costing a Rerail process and, on Windows, loading the native engine to launch it.
    recordHookFailure({ host, event, reason: lostEventReason(daemon) });
    if (daemon.status === "absent") autostartDaemon();
    return {};
  }
  const { endpoint } = daemon;
  const configuration = { storageRoot, databasePath, modelPath, workspaceRoot: workspace };
  return withSessionLock(storageRoot, host, sessionId, async lock => {
    // The daemon orders a session's events itself, so the lock guards only the session's files: a hook lets go of it
    // while the daemon checks its event, unless it keeps it (`SessionLock.kept`), and the hooks of a session's parallel
    // subagents, which all carry the parent's session id, do not queue behind each other's checks until one gives up.
    const ingest = (adapterEvent: AdapterEvent) => lock.released(() => ingestEngineEvent(endpoint, adapterEvent, configuration))
      .catch((error: unknown) => {
        throw unreachable(error);
      });
    const output = await handle({ ingest }, storageRoot, lock);
    return bindSteerReceipt(output, recordLocalReceipt(session, databasePath));
  });
}

/** The engine a hook sends its event to: the socket named outright, else the daemon's, or why no daemon takes it. */
async function hookDaemon(options: HookOptions, startsSession: boolean): Promise<HookDaemon> {
  try {
    const named = options.kernelSocketPath ?? process.env.RERAIL_ENGINE_SOCKET;
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
