import { join } from "node:path";
import type { AdapterSession, RerailEventLoop } from "./adapter-contract.js";
import { bundledModelPath } from "./extension.js";
import { RerailRuntime } from "./runtime.js";
import { ingestEngineEvent } from "./engine-service.js";
import { daemonEngineSocket } from "./daemon-control.js";
import {
  bindSteerReceipt,
  withSessionLock,
  type HookHost,
  type HookOutput,
} from "./hook-adapter.js";
import type { SemanticEmbedder } from "./semantic-types.js";
import { SqliteSessionStore } from "./session-store.js";
import { defaultRerailDatabasePath, openRerailDatabase } from "./database.js";
import { lookupCheckout } from "./repository-registry.js";

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

export interface HookOptions {
  storageRoot?: string;
  modelPath?: string;
  databasePath?: string;
  kernelSocketPath?: string;
  /** Test seam: replaces the native model so hooks can run where the addon is absent. */
  embedder?: SemanticEmbedder;
}

/** Runs a normal package hook against the engine service, or an explicit local test runtime. */
export async function runHostHook(
  host: HookHost,
  workspace: string,
  sessionId: string,
  options: HookOptions,
  handle: (runtime: RerailEventLoop, storageRoot: string) => Promise<HookOutput>,
): Promise<HookOutput> {
  const storageRoot = options.storageRoot ?? process.env.RERAIL_STORAGE_ROOT ?? join(workspace, ".rerail", "sessions");
  return withSessionLock(storageRoot, host, sessionId, async () => {
    const databasePath = options.databasePath ?? defaultRerailDatabasePath();
    const session: AdapterSession = { host, sessionId, cwd: workspace };

    if (options.embedder === undefined) {
      const socketPath = options.kernelSocketPath ?? process.env.RERAIL_KERNEL_SOCKET
        ?? await daemonEngineSocket(true);
      const configuration = {
        storageRoot,
        databasePath,
        modelPath: options.modelPath ?? process.env.RERAIL_MODEL_PATH ?? bundledModelPath(),
        workspaceRoot: workspace,
      };
      const output = await handle({ ingest: event => ingestEngineEvent(socketPath, event, configuration) }, storageRoot);
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
