import { join } from "node:path";
import type { AdapterSession, RerailEventLoop } from "./adapter-contract.js";
import { bundledModelPath } from "./extension.js";
import { RerailRuntime } from "./runtime.js";
import { ingestKernelEvent } from "./kernel-service.js";
import { daemonKernelSocket } from "./daemon-control.js";
import {
  bindSteeringReceipt,
  withSessionLock,
  type HookHost,
  type HookOutput,
} from "./hook-adapter.js";
import type { SemanticEmbedder } from "./semantic-types.js";
import { SqliteTranscriptStore } from "./transcript-store.js";
import { defaultRerailDatabasePath } from "./database.js";
import { readRepositoryId } from "./repository-identity.js";

export * from "./hook-adapter.js";

function recordLocalReceipt(session: AdapterSession, databasePath: string): (receiptId: string, ruleIds: readonly string[]) => void {
  return (receiptId, ruleIds) => {
    // Attribution matters only when a reset removed the session row and this receipt restores it.
    let repositoryId: string | undefined;
    try {
      repositoryId = readRepositoryId(session.cwd);
    } catch {
      repositoryId = undefined;
    }
    const store = new SqliteTranscriptStore(databasePath, { repositoryId });
    try {
      store.recordSteeringDelivery(session, receiptId, ruleIds);
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

/** Runs a normal package hook against the daemon kernel, or an explicit local test runtime. */
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
        ?? await daemonKernelSocket(true);
      const configuration = {
        storageRoot,
        databasePath,
        modelPath: options.modelPath ?? process.env.RERAIL_MODEL_PATH ?? bundledModelPath(),
        workspaceRoot: workspace,
      };
      const output = await handle({ ingest: event => ingestKernelEvent(socketPath, event, configuration) }, storageRoot);
      return bindSteeringReceipt(output, recordLocalReceipt(session, databasePath));
    }
    const runtime = new RerailRuntime({
      storageRoot,
      databasePath,
      workspaceRoot: workspace,
      embedder: options.embedder,
    });
    try {
      const output = await handle(runtime, storageRoot);
      return bindSteeringReceipt(output, recordLocalReceipt(session, databasePath));
    } finally {
      await runtime.dispose();
    }
  });
}
