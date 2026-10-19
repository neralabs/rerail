import { invokeOperation } from "../../src/api/index.js";
import { operations } from "../../src/api/index.js";
import type { RulesDashboardScope } from "../../src/api/index.js";
import { DatabaseApiStores } from "../../src/api/index.js";
import type { Harness } from "../../src/setup/index.js";
import type { ApiCaller } from "../../src/engine/index.js";
import type { DashboardPage, SetupEnvironment } from "../../src/cli/index.js";
import type { DashboardLink } from "../../src/harness/index.js";
import type { PackStore } from "../../src/setup/index.js";
import { directoryPackStore } from "./directory-pack-store.js";

export interface FakeSetupEnvironment extends SetupEnvironment {
  /** Every `connect` call, in order. */
  readonly connections: Array<{ agents: readonly Harness[]; repoRoot: string | undefined }>;
  /** Every dashboard page set-up asked for, in order, with the checkout it is scoped to. */
  readonly pages: Array<{ repoRoot: string | null; repositoryKey: string; page: DashboardPage; open: boolean }>;
}

/** Records what set-up asks of the machine instead of registering coding agents or starting a daemon. */
export function fakeSetupEnvironment(options: {
  found?: readonly Harness[];
  browser?: boolean;
  /** Whether the person has let Rerail read their sessions already; true unless a test asks the question. */
  sessionsApproved?: boolean;
  /** Runs when a page is asked for, as the person's clicks in the dashboard would. */
  onPage?: (scope: RulesDashboardScope, page: DashboardPage) => void | Promise<void>;
  /** Where rule packs come from: this repository's TypeScript pack alone by default, as if no other were published. */
  packs?: PackStore;
} = {}): FakeSetupEnvironment {
  let approved = options.sessionsApproved ?? true;
  const connections: FakeSetupEnvironment["connections"] = [];
  const pages: FakeSetupEnvironment["pages"] = [];
  return {
    connections,
    pages,
    packs: options.packs ?? directoryPackStore(undefined, ["typescript"]),
    agentsFound: () => options.found ?? ["claude-code", "cursor"],
    async connect(agents, repoRoot) {
      connections.push({ agents, repoRoot });
    },
    async connectedAgents() {
      return connections.at(-1)?.agents ?? [];
    },
    sessionsApproved: () => approved,
    async setSessionsApproved(answer) {
      approved = answer;
    },
    async dashboard(scope, page, open): Promise<DashboardLink> {
      pages.push({ repoRoot: scope.checkoutPath, repositoryKey: scope.repositoryKey, page, open });
      await options.onPage?.(scope, page);
      const url = `http://127.0.0.1:4737/#${page === "rules" ? "rules" : "rule-packs?review=1"}`;
      if (!open) return { url, browserOpened: false, linkOnly: true };
      return options.browser === false
        ? { url, browserOpened: false, noBrowserReason: "this looks like a remote session" }
        : { url, browserOpened: true };
    },
  };
}

/**
 * Add `packIds` to the repository with `repository_attachments_set` and `configure: true`: by default as the
 * dashboard's review sends the packs kept on (a person through `/api`), or as `caller` (a coding agent over MCP).
 */
export async function attachPacks(
  databasePath: string,
  scope: RulesDashboardScope,
  packIds: readonly string[],
  caller: ApiCaller = { kind: "person" },
): Promise<void> {
  const stores = new DatabaseApiStores(databasePath, {
    health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
    origin: () => "http://127.0.0.1:4737",
  });
  try {
    const operation = operations.find(candidate => candidate.id === "repository_attachments_set")!;
    const add = packIds.map(sourceId => ({ sourceKind: "pack", sourceId, folder: "" }));
    await invokeOperation(operation, stores, { repository: scope.repositoryKey, add, configure: true }, caller);
  } finally {
    stores.close();
  }
}
