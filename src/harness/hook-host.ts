import type { AdapterEvent, AdapterSession, RerailEventLoop, RerailHost } from "../engine/adapter-contract.js";
import { resolveSessionPaths } from "../runtime/session-paths.js";
import { readRepositoryState } from "../presentation/session-presentation.js";
import { isRerailStarted, readOnlyStartState } from "../runtime/repository-start.js";
import { RerailRuntime } from "../runtime/runtime.js";
import { ingestEngineEvent } from "../runtime/engine-service.js";
import { daemonEngineEndpoint, engineEndpointAt, runningDaemonEngineEndpoint, type EngineEndpoint } from "../runtime/daemon-control.js";
import {
  bindSteerReceipt,
  withSessionLock,
  type HookHost,
  type HookOutput,
} from "./hook-adapter.js";
import type { SemanticEmbedder } from "../engine/native/semantic-types.js";
import { SqliteSessionStore } from "../runtime/session-store.js";
import { defaultRerailDatabasePath, openRerailDatabase } from "../engine/storage/database.js";
import { rerailRunsIn, type RepositoryStartState } from "../engine/catalog/repository-model.js";
import { lookupCheckout } from "../engine/catalog/repository-registry.js";
import { gitExecutable } from "../engine/git.js";

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
 * The start state of the checkout `workspace` is in, as this process reads it from the database; undefined where it
 * cannot: no Git here to find the checkout with (the daemon may have one), a SQLite binding this Node does not load, a
 * database it cannot open or read, or one whose creator has not committed the schema yet.
 */
function localStartState(workspace: string): RepositoryStartState | undefined {
  try {
    gitExecutable();
    return readOnlyStartState(workspace);
  } catch {
    return undefined;
  }
}

/**
 * The check one-shot hooks make before anything else. The hook reads the start state from the database itself, so in
 * a checkout where Rerail is stopped or was never started it answers without a request to the daemon, however busy the
 * daemon is, and never starts one. Only where this process cannot read the state does a live daemon answer, from its
 * own environment and open database; without one, the database is read again and its failure is the hook's. An
 * explicit database path (tests, embedded hosts) is always read directly.
 */
export async function hookRerailStarted(host: RerailHost, workspace: string, databasePath?: string): Promise<boolean> {
  if (databasePath !== undefined) return isRerailStarted(workspace, databasePath);
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
 * a session (`startsSession`) may start the daemon; any other hook finds none running and answers `{}` at once, so a
 * daemon that is down never stalls a tool call or shows as a hook error (design N11). A daemon that cannot be reached
 * throws `DaemonUnreachableError`, so a coding agent without a footer can say so.
 */
export async function runHostHook(
  host: HookHost,
  workspace: string,
  sessionId: string,
  startsSession: boolean,
  options: HookOptions,
  handle: (runtime: RerailEventLoop, storageRoot: string) => Promise<HookOutput>,
): Promise<HookOutput> {
  const { storageRoot, modelPath } = resolveSessionPaths(workspace, options);
  return withSessionLock(storageRoot, host, sessionId, async () => {
    const databasePath = options.databasePath ?? defaultRerailDatabasePath();
    const session: AdapterSession = { host, sessionId, cwd: workspace };

    if (options.embedder === undefined) {
      let endpoint: EngineEndpoint | undefined;
      try {
        const named = options.kernelSocketPath ?? process.env.RERAIL_ENGINE_SOCKET;
        endpoint = named !== undefined ? await engineEndpointAt(named)
          : startsSession ? await daemonEngineEndpoint(true) : await runningDaemonEngineEndpoint();
      } catch (error) {
        throw new DaemonUnreachableError(error instanceof Error ? error.message : String(error), { cause: error });
      }
      if (endpoint === undefined) return {};
      const configuration = {
        storageRoot,
        databasePath,
        modelPath,
        workspaceRoot: workspace,
      };
      const engine = endpoint;
      const ingest = (event: AdapterEvent) => ingestEngineEvent(engine, event, configuration).catch((error: unknown) => {
        throw unreachable(error);
      });
      const output = await handle({ ingest }, storageRoot);
      return bindSteerReceipt(output, recordLocalReceipt(session, databasePath));
    }
    const runtime = new RerailRuntime({
      storageRoot,
      databasePath,
      workspaceRoot: workspace,
      embedder: options.embedder,
    });
    try {
      const output = await handle(runtime, storageRoot);
      return bindSteerReceipt(output, recordLocalReceipt(session, databasePath));
    } finally {
      await runtime.dispose();
    }
  });
}
