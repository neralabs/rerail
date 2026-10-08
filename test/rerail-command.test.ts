import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ presentation: vi.fn(), rulesDashboard: vi.fn(), openDashboard: vi.fn() }));
vi.mock("../src/session-presentation.js", () => ({ readPresentation: mocks.presentation, readRulesDashboard: mocks.rulesDashboard }));
vi.mock("../src/daemon-control.js", async () => ({
  ...await vi.importActual<Record<string, unknown>>("../src/daemon-control.js"),
  openDashboard: mocks.openDashboard,
}));
import { registerRerailCommands, type RerailCommandHost, type RerailCommandContext } from "../src/rerail-command.js";

beforeEach(() => {
  mocks.openDashboard.mockResolvedValue("system-browser");
});

afterEach(() => {
  vi.clearAllMocks();
});

function commandHarness() {
  const commands = new Map<string, Parameters<RerailCommandHost["registerCommand"]>[1]>();
  registerRerailCommands({ registerCommand(name, command) { commands.set(name, command); } }, {
    packageRoot: resolve("."),
    cwd: () => "/workspace",
    sessionId: () => "session-1",
  });
  const notices: Array<{ message: string; level: string }> = [];
  const ctx: RerailCommandContext = {
    ui: {
      notify(message, level) { notices.push({ message, level }); },
      select: async () => undefined,
      input: async () => undefined,
      confirm: async () => false,
    },
  };
  return { run: (name: string) => commands.get(name)!.handler("", ctx), notices };
}

describe("/rerail-ui", () => {
  it("opens the rules page in the browser", async () => {
    mocks.rulesDashboard.mockResolvedValue({ url: "http://127.0.0.1:4737/#rules", reason: null });
    const { run, notices } = commandHarness();
    await run("rerail-ui");
    expect(mocks.rulesDashboard).toHaveBeenCalledWith({ cwd: "/workspace" });
    expect(mocks.openDashboard).toHaveBeenCalledWith("http://127.0.0.1:4737/#rules");
    expect(notices).toEqual([{ message: "Opening http://127.0.0.1:4737/#rules", level: "info" }]);
  });

  it("says so when the page opened as a tab in the Orca window", async () => {
    mocks.rulesDashboard.mockResolvedValue({ url: "http://127.0.0.1:4737/#rules", reason: null });
    mocks.openDashboard.mockResolvedValue("orca-tab");
    const { run, notices } = commandHarness();
    await run("rerail-ui");
    expect(notices).toEqual([{ message: "Opening http://127.0.0.1:4737/#rules in the Orca window", level: "info" }]);
  });

  it("reports why there is no page instead of opening nothing", async () => {
    mocks.rulesDashboard.mockResolvedValue({ url: null, reason: "daemon unavailable" });
    const { run, notices } = commandHarness();
    await run("rerail-ui");
    expect(mocks.openDashboard).not.toHaveBeenCalled();
    expect(notices).toEqual([{ message: "/rerail-ui: no Rerail page to open: daemon unavailable", level: "error" }]);
  });
});

describe("/rerail-session", () => {
  it("opens the session's page in the browser", async () => {
    mocks.presentation.mockResolvedValue({ text: "Rerail | 3 rules", url: "http://127.0.0.1:4737/?sessionId=session-1&host=omp#sessions", parts: [], presence: "resident" });
    const { run, notices } = commandHarness();
    await run("rerail-session");
    expect(mocks.openDashboard).toHaveBeenCalledWith("http://127.0.0.1:4737/?sessionId=session-1&host=omp#sessions");
    expect(notices).toEqual([{ message: "Opening http://127.0.0.1:4737/?sessionId=session-1&host=omp#sessions", level: "info" }]);
  });

  it("reports why there is no page instead of opening nothing", async () => {
    mocks.presentation.mockResolvedValue({ text: "Rerail | daemon unavailable", url: null, parts: [], presence: "absent" });
    const { run, notices } = commandHarness();
    await run("rerail-session");
    expect(mocks.openDashboard).not.toHaveBeenCalled();
    expect(notices).toEqual([{ message: "/rerail-session: no Rerail page to open: Rerail | daemon unavailable", level: "error" }]);
  });
});
