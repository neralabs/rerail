import type { RulesDashboardScope } from "../src/api/session-contracts.js";
import type { Harness } from "../src/harness-registration.js";
import type { DashboardPage, SetupEnvironment } from "../src/repository-setup.js";
import type { DashboardLink } from "../src/setup-screens.js";

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
  /** Runs when a page is asked for, as the person's clicks in the dashboard would. */
  onPage?: (scope: RulesDashboardScope, page: DashboardPage) => void;
} = {}): FakeSetupEnvironment {
  const connections: FakeSetupEnvironment["connections"] = [];
  const pages: FakeSetupEnvironment["pages"] = [];
  return {
    connections,
    pages,
    agentsFound: () => options.found ?? ["claude-code", "cursor"],
    async connect(agents, repoRoot) {
      connections.push({ agents, repoRoot });
    },
    async connectedAgents() {
      return connections.at(-1)?.agents ?? [];
    },
    async dashboard(scope, page, open): Promise<DashboardLink> {
      pages.push({ repoRoot: scope.checkoutPath, repositoryKey: scope.repositoryKey, page, open });
      options.onPage?.(scope, page);
      const url = `http://127.0.0.1:4737/#${page === "rules" ? "rules" : "rule-packs?review=1"}`;
      if (!open) return { url, browserOpened: false, linkOnly: true };
      return options.browser === false
        ? { url, browserOpened: false, noBrowserReason: "this looks like a remote session" }
        : { url, browserOpened: true };
    },
  };
}
