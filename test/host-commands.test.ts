import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as SessionPresentation from "../src/session-presentation.js";
const mocks = vi.hoisted(() => ({ presentation: vi.fn(), rulesDashboard: vi.fn(), openDashboard: vi.fn() }));
// `readSummaryState` stays real: with an empty Rerail home no daemon answers, which is the case under test.
vi.mock("../src/session-presentation.js", async () => ({
  readSummaryState: (await vi.importActual<typeof SessionPresentation>("../src/session-presentation.js")).readSummaryState,
  readPresentation: mocks.presentation, readRulesDashboard: mocks.rulesDashboard,
}));
vi.mock("../src/daemon-control.js", async () => ({
  ...await vi.importActual<Record<string, unknown>>("../src/daemon-control.js"),
  openDashboard: mocks.openDashboard,
}));
import { RERAIL_COMMANDS } from "../src/rerail-commands.js";
import { registerHostCommands, type HostCommandHost, type HostCommandContext, type HostCommandOptions } from "../src/host-commands.js";

const temporaryDirectories: string[] = [];

beforeEach(() => {
  mocks.openDashboard.mockResolvedValue("system-browser");
});

afterEach(async () => {
  vi.unstubAllEnvs();
  vi.clearAllMocks();
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

function commandHarness(options: Partial<HostCommandOptions> = {}, choice?: string) {
  const commands = new Map<string, Parameters<HostCommandHost["registerCommand"]>[1]>();
  const sent: string[] = [];
  registerHostCommands({
    registerCommand(name, command) { commands.set(name, command); },
    sendUserMessage(content) { sent.push(content); },
  }, {
    packageRoot: resolve("."),
    cwd: () => "/workspace/acme-web",
    sessionId: () => "session-1",
    ...options,
  });
  const notices: Array<{ message: string; level: string }> = [];
  const selections: string[][] = [];
  const ctx: HostCommandContext = {
    ui: {
      notify(message, level) { notices.push({ message, level }); },
      select: async (_title, labels) => { selections.push(labels); return choice; },
      input: async () => undefined,
      confirm: async () => false,
    },
  };
  return { commands, run: (name: string, args = "") => commands.get(name)!.handler(args, ctx), notices, sent, selections };
}

/** A package root whose launcher, `dist/cli.js <args>`, runs `script` as `sh` with those arguments. */
async function launcherPackage(script: string): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-command-launcher-"));
  temporaryDirectories.push(root);
  await mkdir(join(root, "dist"));
  await writeFile(join(root, "dist", "cli.js"), [
    "const { spawnSync } = require(\"node:child_process\");",
    `const result = spawnSync("/bin/sh", ["-c", ${JSON.stringify(`set -eu\n${script}\n`)}, "sh", ...process.argv.slice(2)], { stdio: "inherit" });`,
    "process.exit(result.status ?? 1);",
    "",
  ].join("\n"));
  return root;
}

describe("Rerail commands in OMP and Pi", () => {
  it("registers exactly the Rerail commands, with the catalog's descriptions", () => {
    const { commands } = commandHarness();
    expect([...commands.keys()].sort()).toEqual(RERAIL_COMMANDS.map(command => command.name).sort());
    for (const { name, description } of RERAIL_COMMANDS) expect(commands.get(name)?.description).toBe(description);
  });
});

describe("/rerail", () => {
  it("says what Rerail is doing in this session, then lists every command", async () => {
    mocks.presentation.mockResolvedValue({
      status: "on", text: "", url: "http://127.0.0.1:4737/?sessionId=session-1&host=omp#sessions", parts: [],
      presence: "compact", repositoryState: "started", counts: { rules: 26, steered: 1 },
    });
    mocks.rulesDashboard.mockResolvedValue({ url: "http://127.0.0.1:4737/#rules", reason: null });
    const { run, notices } = commandHarness();
    await run("rerail");
    expect(mocks.presentation).toHaveBeenCalledWith({
      sessionId: "session-1", host: "omp", cwd: "/workspace/acme-web", onDemand: true, startState: expect.any(Function),
    });
    const [notice] = notices;
    expect(notice?.level).toBe("info");
    expect(notice?.message).toContain([
      "● Rerail is on in this repo · acme-web",
      "  26 rules · steered 1× in this session",
      "  See what it logged → http://127.0.0.1:4737/?sessionId=session-1&host=omp#sessions",
    ].join("\n"));
    for (const { name } of RERAIL_COMMANDS.filter(command => command.name !== "rerail")) expect(notice?.message).toContain(`/${name} `);
    expect(notice?.message).not.toContain("rerail-test");
  });

  it("says Rerail is not on here, and how to start it, where it is stopped", async () => {
    mocks.presentation.mockResolvedValue({
      status: "not-on-here", text: "", url: null, parts: [], presence: "compact", repositoryState: "stopped", counts: null,
    });
    mocks.rulesDashboard.mockResolvedValue({ url: null, reason: "not on here" });
    const { run, notices } = commandHarness();
    await run("rerail");
    expect(notices[0]?.message.split("\n").slice(0, 2)).toEqual(["○ Rerail is not on in acme-web.", "  Start it with /rerail-start."]);
  });

  it.each([
    { state: "never", running: false },
    { state: "stopped", running: false },
    { state: "started", running: true },
  ])("with no daemon and no session, asks the launcher whether Rerail is on here ($state)", async ({ state, running }) => {
    const home = await mkdtemp(join(tmpdir(), "rerail-command-home-"));
    temporaryDirectories.push(home);
    vi.stubEnv("RERAIL_HOME", home);
    const root = await launcherPackage(`[ "$1" = state ]\nprintf '${state}\\n'`);
    mocks.rulesDashboard.mockResolvedValue({ url: null, reason: "daemon unavailable" });
    const { run, notices } = commandHarness({ packageRoot: root, cwd: () => root, sessionId: () => undefined });
    await run("rerail");
    expect(notices[0]?.message.split("\n").slice(0, 2)).toEqual(running
      ? ["✕ Rerail is not running.", "  Start it with `rerail daemon start` in a terminal."]
      : [`○ Rerail is not on in ${basename(root)}.`, "  Start it with /rerail-start."]);
  });

  it("hands the session read the launcher's start state for when no daemon answers", async () => {
    const root = await launcherPackage(`[ "$1" = state ]\nprintf 'never\\n'`);
    mocks.presentation.mockResolvedValue({
      status: "not-on-here", text: "", url: null, parts: [], presence: "compact", repositoryState: "never", counts: null,
    });
    mocks.rulesDashboard.mockResolvedValue({ url: null, reason: "daemon unavailable" });
    const { run } = commandHarness({ packageRoot: root, cwd: () => root });
    await run("rerail");
    const [request] = mocks.presentation.mock.calls[0] as [{ startState: () => Promise<string> }];
    expect(await request.startState()).toBe("never");
  });
});

describe("/rerail-ui", () => {
  it("opens the dashboard in the browser", async () => {
    mocks.rulesDashboard.mockResolvedValue({ url: "http://127.0.0.1:4737/#rules", reason: null });
    const { run, notices } = commandHarness();
    await run("rerail-ui");
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

  it("reports why there is no dashboard instead of opening nothing", async () => {
    mocks.rulesDashboard.mockResolvedValue({ url: null, reason: "daemon unavailable" });
    const { run, notices } = commandHarness();
    await run("rerail-ui");
    expect(mocks.openDashboard).not.toHaveBeenCalled();
    expect(notices).toEqual([{ message: "/rerail-ui: no dashboard to open: daemon unavailable", level: "error" }]);
  });
});

describe("/rerail-create", () => {
  it("hands the person's words to the coding agent with the draft, test, read-back and add steps", async () => {
    const { run, sent, notices } = commandHarness();
    await run("rerail-create", "use design tokens, not hex colours");
    expect(notices).toEqual([]);
    expect(sent).toHaveLength(1);
    expect(sent[0]).toContain("\"use design tokens, not hex colours\"");
    expect(sent[0]).toContain(`${join(resolve("."), "dist", "cli.js")}' rules test <draft.json> --json`);
    expect(sent[0]).toContain("Add this rule?");
    expect(sent[0]).toContain(`${join(resolve("."), "dist", "cli.js")}' rules add <draft.json>`);
  });
});

describe("/rerail-rage-quit", () => {
  it("asks once and changes nothing unless the person says yes", async () => {
    const detach = vi.fn();
    const { run, notices, selections } = commandHarness({ detach }, "Cancel");
    await run("rerail-rage-quit");
    expect(selections).toEqual([["Yes, remove Rerail", "Just stop it in this repo (/rerail-stop)", "Cancel"]]);
    expect(detach).not.toHaveBeenCalled();
    expect(notices).toEqual([]);
  });

  it("removes Rerail from this computer after a yes, releasing the session's handles first", async () => {
    const root = await launcherPackage("[ \"$*\" = 'install --remove' ]\nprintf 'Backup: /home/me/.rerail-clean-20261005\\n'");
    const detach = vi.fn();
    const reattach = vi.fn();
    const { run, notices } = commandHarness({ packageRoot: root, cwd: () => root, detach, reattach }, "Yes, remove Rerail");
    await run("rerail-rage-quit");
    expect(detach).toHaveBeenCalledOnce();
    expect(reattach).not.toHaveBeenCalled();
    expect(notices).toEqual([{ message: "Backup: /home/me/.rerail-clean-20261005", level: "info" }]);
  });

  it("gives the session its runtime back when the removal fails, as Rerail is still installed", async () => {
    const root = await launcherPackage("printf 'could not write ~/.claude/settings.json\\n' >&2\nexit 1");
    const detach = vi.fn();
    const reattach = vi.fn();
    const { run, notices } = commandHarness({ packageRoot: root, cwd: () => root, detach, reattach }, "Yes, remove Rerail");
    await run("rerail-rage-quit");
    expect(reattach).toHaveBeenCalledOnce();
    expect(reattach.mock.invocationCallOrder[0]).toBeGreaterThan(detach.mock.invocationCallOrder[0]!);
    expect(notices).toEqual([{ message: "could not write ~/.claude/settings.json", level: "error" }]);
  });

  it("gives the session its runtime back when the removal cannot even be launched", async () => {
    // A NUL byte makes the launch itself throw, before any process exists.
    const unlaunchable = join(tmpdir(), "rerail\0checkout");
    const detach = vi.fn();
    const reattach = vi.fn();
    const { run, notices } = commandHarness({ cwd: () => unlaunchable, detach, reattach }, "Yes, remove Rerail");
    await run("rerail-rage-quit");
    expect(notices).toEqual([expect.objectContaining({ message: expect.stringMatching(/^\/rerail-rage-quit: /u), level: "error" })]);
    expect(detach).toHaveBeenCalledOnce();
    expect(reattach).toHaveBeenCalledOnce();
  });
});
