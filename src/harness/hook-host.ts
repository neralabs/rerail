import { existsSync } from "node:fs";
import { join } from "node:path";
import Database from "better-sqlite3";
import type { AdapterEvent, AdapterSession, RerailEventLoop, RerailHost } from "../engine/adapter-contract.js";
import { GitUnavailableError } from "../engine/git.js";
import { GitDidNotAnswerError } from "../engine/catalog/repository-identity.js";
import { resolveSessionPaths } from "../runtime/session-paths.js";
import { readRepositoryState } from "../presentation/session-presentation.js";
import { answeredStartState, isRerailStarted } from "../runtime/repository-start.js";
import { RerailRuntime } from "../runtime/runtime.js";
import { ingestEngineEvent } from "../runtime/engine-service.js";
import {
  autostartDaemon,
  autostartOff,
  AutostartOffError,
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
import { DAEMON_STATE_FILE, defaultRerailHome } from "../engine/storage/rerail-home.js";
import { rerailRunsIn, type RepositoryStartState } from "../engine/catalog/repository-model.js";
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
 * The `GIT_*` variables that cannot change where Git finds a checkout or which remote it names: the two every Git
 * process Rerail starts sets itself, and those that only choose how Git deals with a person (prompts, editors, pagers,
 * SSH) or who a commit names. Coding agents and editors set several of them for the commands they run.
 */
const LOOKUP_NEUTRAL_GIT_VARIABLES = new Set([
  "GIT_OPTIONAL_LOCKS", "GIT_NO_LAZY_FETCH", "GIT_TERMINAL_PROMPT", "GIT_ASKPASS", "GIT_EDITOR", "GIT_SEQUENCE_EDITOR",
  "GIT_PAGER", "GIT_SSH", "GIT_SSH_COMMAND", "GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL", "GIT_AUTHOR_DATE",
  "GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL", "GIT_COMMITTER_DATE",
]);

/**
 * Whether the coding agent's environment, which Git started here inherits, holds a `GIT_*` variable that can move Git's
 * answer away from the daemon's: `GIT_DIR`, `GIT_WORK_TREE`, `GIT_CEILING_DIRECTORIES`, a config file of its own or
 * none (`GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`, `GIT_CONFIG_NOSYSTEM`; those files hold `safe.directory` and
 * `url.*.insteadOf`), or one a later Git adds. The daemon's Git has the environment of whatever started it, which need
 * not be this agent's. Names are compared in upper case, as Windows reads them.
 */
function gitEnvironmentMovesLookup(environment: NodeJS.ProcessEnv): boolean {
  return Object.keys(environment).some(name => {
    const upper = name.toUpperCase();
    return upper.startsWith("GIT_") && !LOOKUP_NEUTRAL_GIT_VARIABLES.has(upper);
  });
}

/**
 * Whether `error`, from this process's own read of the start state, is one a live daemon answers past: no Git here, a
 * Git that gave no answer, or a database SQLite could not read (SQLite's error, or Drizzle's query error carrying it).
 */
function daemonAnswersPast(error: unknown): boolean {
  const cause = error instanceof Error && error.cause !== undefined ? error.cause : error;
  return error instanceof GitUnavailableError || error instanceof GitDidNotAnswerError || cause instanceof Database.SqliteError;
}

/**
 * The start state of the checkout `workspace` is in, as this process reads it from the database; undefined where it
 * cannot be sure to read what the daemon reads: the agent's environment can move Git's lookup, Git is not installed
 * here or gave no answer (the daemon keeps the locations its Git found), a SQLite binding this Node does not load, a
 * database it cannot open or read, or one whose creator has not committed the schema yet. Any other failure is a fault
 * of this read and is thrown, so it shows rather than sending every hook back to the daemon.
 */
function localStartState(workspace: string): RepositoryStartState | undefined {
  if (gitEnvironmentMovesLookup(process.env)) return undefined;
  try {
    return answeredStartState(workspace);
  } catch (error) {
    if (daemonAnswersPast(error)) return undefined;
    throw error;
  }
}

/** Whether a daemon has recorded itself in the Rerail home (`daemon.json`), whether or not it still runs. */
function daemonRecorded(): boolean {
  return existsSync(join(defaultRerailHome(), DAEMON_STATE_FILE));
}

/**
 * The check one-shot hooks make before anything else. With no daemon recorded, or an explicit database path (tests,
 * embedded hosts), the hook reads the database directly (`isRerailStarted`): there is no daemon to wait for or to ask,
 * so its Git runs once. Otherwise the hook reads the start state from the database itself, so in a checkout where
 * Rerail is stopped or was never started it answers without a request to the daemon, however busy the daemon is, and
 * never starts one. Only where this process cannot read the state as the daemon would does a live daemon answer, from
 * its own environment, the Git locations it keeps and its open database; when none answers, the database is read again
 * (`isRerailStarted`), and its failure is the hook's.
 */
export async function hookRerailStarted(host: RerailHost, workspace: string, databasePath?: string): Promise<boolean> {
  if (databasePath !== undefined || !daemonRecorded()) return isRerailStarted(workspace, databasePath);
  const local = localStartState(workspace);
  if (local !== undefined) return rerailRunsIn(local);
  const answered = await readRepositoryState(workspace, host);
  return answered === null ? isRerailStarted(workspace) : rerailRunsIn(answered);
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
 * The hook found no daemon because `RERAIL_AUTOSTART=0` kept it from starting one. That is what the person asked for,
 * so it is not a failure; `rerail status` says the daemon is not running and why (`daemonNotRunningMessage`).
 */
function autostartTurnedOff(error: unknown): boolean {
  return error instanceof DaemonUnreachableError && error.cause instanceof AutostartOffError;
}

/**
 * Runs a normal package hook against the engine service, or an explicit local test runtime. Only the hook that starts
 * a session (`startsSession`) waits for a daemon to start; any other hook that finds none running answers `{}` at
 * once, so a daemon that is down never stalls a tool call or shows as a hook error (design N11), records the lost event
 * in the hook failure log and starts a daemon in the background. A daemon that cannot be reached throws
 * `DaemonUnreachableError`, so a coding agent without a footer can say so. Every failure, `event` named as the coding
 * agent names it, goes to the hook failure log (`hook-failures.ts`), which `rerail status` reads. Under
 * `RERAIL_AUTOSTART=0` a hook that finds no daemon logs nothing and starts none, since the person runs it themselves.
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
  // Found before the session's lock is taken: a session's hooks that arrive together (parallel tool calls, subagents)
  // wait out a busy daemon side by side. Under the lock, each hook would begin its wait only when the one before it
  // gave up.
  const daemon = await hookDaemon(options, startsSession);
  if (daemon.status !== "running") {
    // This event is lost, so say so where `rerail status` looks. A daemon that is gone (it crashed, or was stopped)
    // is started in the background for the next event; the start is handed to a detached process, so the tool call
    // never waits on it (design N11). A daemon that is alive is left alone: a start would find it and refuse to
    // replace it, after costing a Rerail process and, on Windows, loading the native engine to launch it.
    // Under RERAIL_AUTOSTART=0 the person runs the daemon themselves, so finding none is no failure and starts none.
    if (daemon.status === "absent" && autostartOff()) return {};
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
