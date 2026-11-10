import type { AdapterEvent, AdapterSession, RerailEventLoop, RerailHost } from "../engine/adapter-contract.js";
import { resolveSessionPaths } from "../runtime/session-paths.js";
import { readRepositoryState } from "../presentation/session-presentation.js";
import { isRerailStarted } from "../runtime/repository-start.js";
import { RerailRuntime } from "../runtime/runtime.js";
import { ingestEngineEvent } from "../runtime/engine-service.js";
import { autostartDaemon, daemonEngineEndpoint, engineEndpointAt, runningDaemonEngineEndpoint, type EngineEndpoint } from "../runtime/daemon-control.js";
import { recordHookFailure } from "./hook-failures.js";
import {
  bindSteerReceipt,
  withSessionLock,
  type HookHost,
  type HookOutput,
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
  handle: (runtime: RerailEventLoop, storageRoot: string) => Promise<HookOutput>,
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
  handle: (runtime: RerailEventLoop, storageRoot: string) => Promise<HookOutput>,
  event: string,
): Promise<HookOutput> {
  const { storageRoot, modelPath } = resolveSessionPaths(workspace, options);
  const databasePath = options.databasePath ?? defaultRerailDatabasePath();
  const session: AdapterSession = { host, sessionId, cwd: workspace };
  const { embedder } = options;
  if (embedder !== undefined) {
    return withSessionLock(storageRoot, host, sessionId, async () => {
      const runtime = new RerailRuntime({ storageRoot, databasePath, workspaceRoot: workspace, embedder });
      try {
        const output = await handle(runtime, storageRoot);
        return bindSteerReceipt(output, recordLocalReceipt(session, databasePath));
      } finally {
        await runtime.dispose();
      }
    });
  }
  // Finding the daemon touches no session file, and can wait seconds for a busy one, so no other hook waits on it.
  const engine = await engineEndpoint(startsSession, options);
  if (engine === undefined) {
    // The daemon is gone (it crashed, or was stopped): this event is lost, so say so where `rerail status` looks,
    // and start one in the background for the next event. The start is handed to a detached process, so the
    // tool call never waits on it (design N11); one that finds a daemon alive does nothing.
    recordHookFailure({ host, event, reason: "no Rerail daemon answered, so this event was not recorded" });
    autostartDaemon();
    return {};
  }
  const configuration = { storageRoot, databasePath, modelPath, workspaceRoot: workspace };
  return withSessionLock(storageRoot, host, sessionId, async lock => {
    // The daemon orders a session's events itself, so the lock guards only the session's held steers: a hook lets go
    // of it while the daemon checks its event, and the hooks of a session's parallel subagents, which all carry the
    // parent's session id, do not queue behind each other's checks until one gives up on its event.
    const ingest = (adapterEvent: AdapterEvent) => lock.released(() => ingestEngineEvent(engine, adapterEvent, configuration))
      .catch((error: unknown) => {
        throw unreachable(error);
      });
    const output = await handle({ ingest }, storageRoot);
    return bindSteerReceipt(output, recordLocalReceipt(session, databasePath));
  });
}

/** The daemon engine a hook sends its event to, or undefined when none is running and this hook may not start one. */
async function engineEndpoint(startsSession: boolean, options: HookOptions): Promise<EngineEndpoint | undefined> {
  try {
    const named = options.kernelSocketPath ?? process.env.RERAIL_ENGINE_SOCKET;
    if (named !== undefined) return await engineEndpointAt(named);
    return startsSession ? await daemonEngineEndpoint(true) : await runningDaemonEngineEndpoint();
  } catch (error) {
    throw new DaemonUnreachableError(error instanceof Error ? error.message : String(error), { cause: error });
  }
}
