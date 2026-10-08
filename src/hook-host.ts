import { join } from "node:path";
import type { AdapterSession, RerailEventLoop } from "./adapter-contract.js";
import { bundledModelPath } from "./extension.js";
import { RerailRuntime } from "./runtime.js";
import { ingestEngineEvent } from "./engine-service.js";
import { ingestRerailDevEvent, recordRerailDevReceipt, resolveRerailWorkspaceRoute } from "./dev-routing.js";
import { daemonEngineSocket } from "./daemon-control.js";
import {
  bindSteerReceipt,
  withSessionLock,
  type HookHost,
  type HookOutput,
} from "./hook-adapter.js";
import type { SemanticEmbedder } from "./semantic-types.js";
import { SqliteSessionStore } from "./session-store.js";
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
    const store = new SqliteSessionStore(databasePath, { repositoryId });
    try {
      store.recordSteer(session, receiptId, ruleIds);
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

/** Runs a normal package hook against its selected dev container, engine service, or explicit local test runtime. */
export async function runHostHook(
  host: HookHost,
  workspace: string,
  sessionId: string,
  options: HookOptions,
  handle: (runtime: RerailEventLoop, storageRoot: string) => Promise<HookOutput>,
): Promise<HookOutput> {
  const route = await resolveRerailWorkspaceRoute(workspace);
  if (route.kind === "disabled") return {};
  const storageRoot = options.storageRoot ?? process.env.RERAIL_STORAGE_ROOT ?? join(workspace, ".rerail", "sessions");
  return withSessionLock(storageRoot, host, sessionId, async () => {
    const databasePath = options.databasePath ?? defaultRerailDatabasePath();
    const session: AdapterSession = { host, sessionId, cwd: workspace };
    const developmentInstance = route.kind === "workspace" ? route.instance : undefined;
    if (developmentInstance !== undefined) {
      // The container's engine service records into its own Rerail home; it never takes a host database path.
      const configuration = {
        storageRoot,
        modelPath: options.modelPath ?? process.env.RERAIL_MODEL_PATH ?? bundledModelPath(),
        workspaceRoot: developmentInstance.workspaceRoot,
      };
      const output = await handle({ ingest: event => ingestRerailDevEvent(developmentInstance, event, configuration) }, storageRoot);
      return bindSteerReceipt(output, (receiptId, ruleIds) => recordRerailDevReceipt(developmentInstance, session, receiptId, ruleIds));
    }
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
