import { join } from "node:path";
import type { AdapterSession, RerailEventLoop } from "./adapter-contract.js";
import { ingestRerailDevEvent, recordRerailDevReceipt, resolveRerailWorkspaceRoute } from "./dev-routing.js";
import {
  bindSteeringReceipt,
  withSessionLock,
  type HookHost,
  type HookOutput,
} from "./hook-adapter.js";

/** Routes an installed development bridge hook only to its selected checkout container. */
export async function runDevHostHook(
  host: HookHost,
  workspace: string,
  sessionId: string,
  handle: (runtime: RerailEventLoop, storageRoot: string) => Promise<HookOutput>,
): Promise<HookOutput> {
  const route = await resolveRerailWorkspaceRoute(workspace);
  if (route.kind === "disabled") return {};
  if (route.kind === "host") {
    throw new Error("Rerail development instance is unavailable; the development hook cannot use a host-local fallback");
  }
  const storageRoot = process.env.RERAIL_STORAGE_ROOT ?? join(workspace, ".rerail", "sessions");
  return withSessionLock(storageRoot, host, sessionId, async () => {
    const instance = route.instance;
    const session: AdapterSession = { host, sessionId, cwd: workspace };
    const configuration = {
      storageRoot,
      modelPath: process.env.RERAIL_MODEL_PATH ?? join(workspace, "rerail", "model"),
      workspaceRoot: instance.workspaceRoot,
    };
    const output = await handle({ ingest: event => ingestRerailDevEvent(instance, event, configuration) }, storageRoot);
    return bindSteeringReceipt(output, receiptId => recordRerailDevReceipt(instance, session, receiptId));
  });
}
