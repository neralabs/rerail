import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ presentation: vi.fn(), rulesDashboard: vi.fn(), openDashboard: vi.fn() }));
// `readSummaryState` stays real: with an empty Rerail home no daemon answers, which is the case under test.
vi.mock("../../src/presentation/session-presentation.js", async () => ({
  readSummaryState: (await vi.importActual<typeof import("../../src/presentation/session-presentation.js")>("../../src/presentation/session-presentation.js")).readSummaryState,
  readPresentation: mocks.presentation, readRulesDashboard: mocks.rulesDashboard,
}));
vi.mock("../../src/runtime/daemon-control.js", async () => ({
  ...await vi.importActual<Record<string, unknown>>("../../src/runtime/daemon-control.js"),
  openDashboard: mocks.openDashboard,
}));
import { RERAIL_COMMANDS } from "../../src/harness/rerail-commands.js";
import {
  registerRerailCommands,
  type RerailCommandHost,
  type RerailCommandContext,
  type RerailCommandOptions,
} from "../../src/harness/rerail-command.js";
import { SETUP_CANCELLED } from "../../src/harness/setup-picker.js";

const temporaryDirectories: string[] = [];

beforeEach(() => {
  mocks.openDashboard.mockResolvedValue("system-browser");
});

afterEach(async () => {
  vi.unstubAllEnvs();
  vi.clearAllMocks();
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

function commandHarness(options: Partial<RerailCommandOptions> = {}, choice?: string) {
  const commands = new Map<string, Parameters<RerailCommandHost["registerCommand"]>[1]>();
  const sent: string[] = [];
  registerRerailCommands({
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
  const ctx: RerailCommandContext = {
    ui: {
      notify(message, level) { notices.push({ message, level }); },
      select: async (_title, labels) => { selections.push(labels); return choice; },
      input: async () => undefined,
      confirm: async () => false,
    },
  };
  return { commands, run: (name: string, args = "") => commands.get(name)!.handler(args, ctx), notices, sent, selections };
}

/** A package root whose launcher, `dist/cli/cli.js <args>`, runs `script` as `sh` with those arguments. */
async function launcherPackage(script: string): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-command-launcher-"));
  temporaryDirectories.push(root);
  await mkdir(join(root, "dist", "cli"), { recursive: true });
  await writeFile(join(root, "dist", "cli", "cli.js"), [
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

  it("says a started repository that is not set up checks no rules, and how to set it up", async () => {
    mocks.presentation.mockResolvedValue({
      status: "not-set-up", text: "", url: null, parts: [], presence: "compact", repositoryState: "not-set-up", counts: null,
    });
    mocks.rulesDashboard.mockResolvedValue({ url: null, reason: "daemon unavailable" });
    const { run, notices } = commandHarness();
    await run("rerail");
    expect(notices[0]?.message.split("\n").slice(0, 2)).toEqual([
      "○ Rerail is on in acme-web, which is not set up yet: no rules are checked.",
      "  Set it up with /rerail-start (rerail start).",
    ]);
  });

  it("hands the session read the launcher's start state for when no daemon answers", async () => {
    const root = await launcherPackage(`[ "$1" = state ]\nprintf 'stopped\\n'`);
    mocks.presentation.mockResolvedValue({
      status: "not-on-here", text: "", url: null, parts: [], presence: "compact", repositoryState: "stopped", counts: null,
    });
    mocks.rulesDashboard.mockResolvedValue({ url: null, reason: "daemon unavailable" });
    const { run } = commandHarness({ packageRoot: root, cwd: () => root });
    await run("rerail");
    const [request] = mocks.presentation.mock.calls[0] as [{ startState: () => Promise<string> }];
    expect(await request.startState()).toBe("stopped");
  });
});

describe("/rerail-ui", () => {
  const scoped = "http://127.0.0.1:4737/?repository=remote%3Agithub.com%2Facme%2Fapp&checkout=%2Fworkspace#rules";

  it("opens the rules page scoped to this checkout in the browser", async () => {
    mocks.rulesDashboard.mockResolvedValue({ url: scoped, reason: null });
    const { run, notices } = commandHarness();
    await run("rerail-ui");
    expect(mocks.rulesDashboard).toHaveBeenCalledWith({ cwd: "/workspace/acme-web" });
    expect(mocks.openDashboard).toHaveBeenCalledWith(scoped);
    expect(notices).toEqual([{ message: `Opening ${scoped}`, level: "info" }]);
  });

  it("says so when the page opened as a tab in the Orca window", async () => {
    mocks.rulesDashboard.mockResolvedValue({ url: scoped, reason: null });
    mocks.openDashboard.mockResolvedValue("orca-tab");
    const { run, notices } = commandHarness();
    await run("rerail-ui");
    expect(notices).toEqual([{ message: `Opening ${scoped} in the Orca window`, level: "info" }]);
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
    expect(sent[0]).toContain(`${join(resolve("."), "dist", "cli", "cli.js")}' rules test <draft.json> --json`);
    expect(sent[0]).toContain("Add this rule?");
    expect(sent[0]).toContain(`${join(resolve("."), "dist", "cli", "cli.js")}' rules add <draft.json>`);
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

describe("/rerail-start", () => {
  /** A launcher that answers `start --json` with `step` and echoes every other command's arguments. */
  const launcher = (step: object) => launcherPackage(
    `if [ "$1 \${2:-}" = "start --json" ]; then printf '%s\\n' '${JSON.stringify(step)}'; else printf 'ran %s\\n' "$*"; fi`,
  );
  const question = {
    step: "read-repo",
    screen: "You’re working in acme-web.",
    question: {
      id: "read-repo",
      prompt: "Can Rerail read it?",
      options: [{ answer: "yes", label: "Yes, read acme-web", detail: null, next: { args: ["setup", "--json", "--answer", "yes"] }, input: null }],
    },
  };

  it("runs set-up's steps through the launcher and re-attaches once one ends with Rerail on", async () => {
    const root = await launcher({ step: "done", screen: "● Rerail is on in acme-web. 3 rules are on, as you left them.", question: null });
    const reattach = vi.fn();
    const { run, notices, selections } = commandHarness({ packageRoot: root, cwd: () => root, reattach });
    await run("rerail-start");
    expect(selections).toEqual([]);
    expect(notices).toEqual([{ message: "● Rerail is on in acme-web. 3 rules are on, as you left them.", level: "info" }]);
    expect(reattach).toHaveBeenCalledOnce();
  });

  it("changes nothing and does not re-attach when the person closes set-up's first question", async () => {
    const root = await launcher(question);
    const reattach = vi.fn();
    const { run, notices, selections } = commandHarness({ packageRoot: root, cwd: () => root, reattach });
    await run("rerail-start");
    expect(selections).toEqual([["Yes, read acme-web"]]);
    expect(notices.at(-1)).toEqual({ message: SETUP_CANCELLED, level: "info" });
    expect(reattach).not.toHaveBeenCalled();
  });

  it("hands over to the launcher's text without a UI (print mode)", async () => {
    const root = await launcher(question);
    const reattach = vi.fn();
    const { commands } = commandHarness({ packageRoot: root, cwd: () => root, reattach });
    const notices: string[] = [];
    const ui = { notify: (message: string) => { notices.push(message); }, select: vi.fn(), input: vi.fn(), confirm: vi.fn() };
    await commands.get("rerail-start")!.handler("", { hasUI: false, ui });
    expect(notices).toEqual(["ran start"]);
    expect(ui.select).not.toHaveBeenCalled();
    expect(reattach).toHaveBeenCalledOnce();
  });
});
