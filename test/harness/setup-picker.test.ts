import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

// Core ships no pack with a match definition; set-up's recommendation flow is exercised with the TypeScript pack added.
vi.mock("../../src/engine/catalog/bundled-rule-documents.js", async (importOriginal) => {
  const actual = await importOriginal<{ corePackDirectories(): string[] }>();
  const typescriptPack = decodeURIComponent(new URL("../../packs/typescript", import.meta.url).pathname);
  return { ...actual, corePackDirectories: () => [...actual.corePackDirectories(), typescriptPack] };
});
import { repositoryIdentityLocation } from "../../src/engine/index.js";
import { workspaceStartState } from "../../src/runtime/index.js";
import { runSetupCommand, runSetupStepArgs, type SetupContext } from "../../src/cli/index.js";
import { pickSetup, SETUP_CANCELLED, type SetupPickerUi } from "../../src/harness/setup-picker.js";
import { ManagedUpdater } from "../../src/setup/index.js";
import { attachPacks, fakeSetupEnvironment, type FakeSetupEnvironment } from "../support/fake-setup-environment.js";
import { attachedPacks } from "../support/attached-packs.js";

const SETUP_TIMEOUT_MS = 30_000;
const TS_PACKS = ["typescript"];
const temporaryDirectories: string[] = [];
afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

interface Machine {
  root: string;
  repo: string;
  databasePath: string;
  context: SetupContext;
}

function gitRepository(path: string): string {
  mkdirSync(path, { recursive: true });
  writeFileSync(join(path, "app.ts"), "export const hi = 1;\n");
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: path });
  execFileSync("git", ["add", "."], { cwd: path });
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "-m", "base"], { cwd: path });
  return path;
}

/** A Rerail home whose active install is a package here, so no run installs anything. */
async function machine(): Promise<Machine> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-picker-")));
  temporaryDirectories.push(root);
  const home = join(root, "home");
  const packageRoot = join(root, "package");
  mkdirSync(join(packageRoot, "dist"), { recursive: true });
  writeFileSync(join(packageRoot, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "1.0.0" }));
  vi.stubEnv("RERAIL_HOME", home);
  // Past set-up's one approval that Rerail may read sessions; `setup-json.test.ts` asks it.
  mkdirSync(home, { recursive: true });
  writeFileSync(join(home, "consent.json"), JSON.stringify({ sessions: { approvedAt: "2026-10-16T00:00:00.000Z" } }));
  const updater = new ManagedUpdater({ home, packageRoot, packageVersion: "1.0.0" });
  await updater.activateRoot(packageRoot);
  return { root, repo: gitRepository(join(root, "acme-api")), databasePath: join(home, "rerail.sqlite"), context: { home, packageRoot, updater } };
}

/** The steps as OMP's and Pi's `/rerail-start` gets them from the launcher, run in-process from `cwd` against a fake machine. */
function steps(setup: Machine, environment: FakeSetupEnvironment, cwd = setup.repo) {
  return (args: readonly string[]) => runSetupStepArgs(args, setup.context, { cwd, environment: () => environment });
}

interface FakeUi extends SetupPickerUi {
  notices: Array<{ message: string; level: string }>;
  selects: Array<{ title: string; options: string[] }>;
  inputs: Array<{ title: string; placeholder: string | undefined }>;
}

/** A person picking the option that starts with each of `picks` in turn, and typing each of `typed`; `undefined` closes the dialog. */
function fakeUi(picks: Array<string | undefined>, typed: Array<string | undefined> = []): FakeUi {
  const ui: FakeUi = {
    notices: [], selects: [], inputs: [],
    notify(message, level) { ui.notices.push({ message, level }); },
    async select(title, options) {
      ui.selects.push({ title, options });
      const pick = picks.shift();
      if (pick === undefined) return undefined;
      const option = options.find(label => label.startsWith(pick));
      if (option === undefined) throw new Error(`“${pick}” was not offered: ${options.join(" | ")}`);
      return option;
    },
    async input(title, placeholder) {
      ui.inputs.push({ title, placeholder });
      return typed.shift();
    },
  };
  return ui;
}

function selected(setup: Machine, repo = setup.repo): readonly string[] {
  return attachedPacks(repo, setup.databasePath);
}

describe("/rerail-start in OMP and Pi", { timeout: SETUP_TIMEOUT_MS }, () => {
  it("asks before reading, shows the packs, and starts on yes", async () => {
    const setup = await machine();
    const environment = fakeSetupEnvironment();
    const ui = fakeUi(["Yes, read acme-api", "Yes, start Rerail"]);

    const last = await pickSetup(ui, steps(setup, environment));

    expect(ui.selects.map(({ title }) => title)).toEqual([
      "You’re working in acme-api. Can Rerail read it to pick the rule packs that look right?",
      "Start Rerail in acme-api with this rule pack?",
    ]);
    expect(ui.selects[0]!.options).toEqual([
      "Yes, read acme-api",
      "Use a different folder… — for example “~/code/landing-page”",
      "I’ll pick rule packs myself — Rerail won’t read the repo",
    ]);
    expect(ui.notices[1]!.message).toContain("Of the rule packs that come with Rerail, 1 looks right for acme-api.");
    expect(last).toMatchObject({ step: "done", question: null });
    expect(ui.notices.at(-1)).toEqual({ message: last!.screen, level: "info" });
    expect(last!.screen).toContain("● Rerail is on in acme-api. 1 rule pack is on");
    // Nothing for the coding agent to relay: the person reads every line.
    expect(ui.notices.map(({ message }) => message).join("\n")).not.toMatch(/Ask them this|run set-up again|Show the person/u);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
    expect(selected(setup)).toEqual(TS_PACKS);
    expect(environment.connections).toEqual([{ agents: ["claude-code", "cursor"], repoRoot: setup.repo }]);
  });

  it("starts with every pack off, reading nothing, when the person picks packs themselves", async () => {
    const setup = await machine();
    const ui = fakeUi(["I’ll pick rule packs myself"]);
    const last = await pickSetup(ui, steps(setup, fakeSetupEnvironment()));
    expect(last).toMatchObject({ step: "done" });
    expect(last!.screen).toContain("Rerail is on in acme-api. No rule packs are on yet.");
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
    expect(selected(setup)).toEqual([]);
  });

  it("opens the review, asks again, and starts with the packs kept on there", async () => {
    const setup = await machine();
    // The person switches complexity on in the review, which adds the packs kept on and sets the repository up.
    const environment = fakeSetupEnvironment({
      onPage: (scope, page) => (page === "review" ? attachPacks(setup.databasePath, scope, ["typescript", "complexity"]) : undefined),
    });
    const ui = fakeUi(["Yes, read acme-api", "Review them first", "Yes, start Rerail"]);

    const last = await pickSetup(ui, steps(setup, environment));

    expect(ui.selects[2]!.title).toBe("Start Rerail in acme-api with the rule packs you kept on?");
    expect(ui.notices.some(({ message }) => message.includes("http://127.0.0.1:4737/#rule-packs?review=1"))).toBe(true);
    expect(environment.pages.map(({ page }) => page)).toEqual(["review", "rules"]);
    expect(last).toMatchObject({ step: "done" });
    expect(selected(setup)).toEqual(["complexity", "typescript"]);
  });

  it("asks which project in a session opened in the home folder, then sets the folder named up", async () => {
    const setup = await machine();
    const home = join(setup.root, "user");
    mkdirSync(home);
    vi.stubEnv("HOME", home);
    const ui = fakeUi(["Type the folder…", "Yes, start Rerail"], [`  ${setup.repo}  `]);

    const last = await pickSetup(ui, steps(setup, fakeSetupEnvironment(), home));

    expect(ui.selects[0]!.title).toBe("Which project do you want to start with?");
    expect(ui.notices[0]!.message).toContain("No repo found here.");
    expect(ui.inputs).toEqual([{ title: "Which folder?", placeholder: "for example ~/code/acme-web" }]);
    expect(last).toMatchObject({ step: "done" });
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
    expect(existsSync(join(home, ".rerail"))).toBe(false);
  });

  it("changes nothing when the person closes a question or leaves a value empty", async () => {
    const setup = await machine();
    const closed = fakeUi([undefined]);
    expect(await pickSetup(closed, steps(setup, fakeSetupEnvironment()))).toBeUndefined();
    expect(closed.notices.at(-1)).toEqual({ message: SETUP_CANCELLED, level: "info" });

    const empty = fakeUi(["Use a different folder…"], [""]);
    expect(await pickSetup(empty, steps(setup, fakeSetupEnvironment()))).toBeUndefined();
    expect(empty.notices.at(-1)).toEqual({ message: SETUP_CANCELLED, level: "info" });

    const agents = fakeUi(["Yes, read acme-api", "Type something else…"], [undefined]);
    expect(await pickSetup(agents, steps(setup, fakeSetupEnvironment()))).toBeUndefined();
    expect(agents.inputs[0]).toEqual({ title: "Which coding agents should Rerail connect to?", placeholder: "comma-separated ids from: claude-code (Claude Code), cursor (Cursor)" });

    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
    expect(existsSync(repositoryIdentityLocation(setup.repo).identityPath)).toBe(false);
  });

  it("connects only the coding agents the person names under “Type something else…”", async () => {
    const setup = await machine();
    const environment = fakeSetupEnvironment();
    const ui = fakeUi(["Yes, read acme-api", "Type something else…"], ["cursor"]);
    expect(await pickSetup(ui, steps(setup, environment))).toMatchObject({ step: "done" });
    expect(environment.connections).toEqual([{ agents: ["cursor"], repoRoot: setup.repo }]);
  });

  it("shows a failed step as an error and stops", async () => {
    const setup = await machine();
    const ui = fakeUi([]);
    const last = await pickSetup(ui, steps(setup, fakeSetupEnvironment()), ["setup", "--json", "--answer", "maybe"]);
    expect(last).toMatchObject({ step: "error" });
    expect(ui.notices).toEqual([{ message: expect.stringContaining("Unknown answer: maybe"), level: "error" }]);
    expect(ui.selects).toEqual([]);
  });

  it("sets up in the database the extension was configured with, not the home's", async () => {
    const setup = await machine();
    const configured = join(setup.root, "configured", "rerail.sqlite");
    const ui = fakeUi(["Yes, read acme-api", "Yes, start Rerail"]);
    const run = (args: readonly string[]) =>
      runSetupStepArgs(args, setup.context, { cwd: setup.repo, databasePath: configured, environment: () => fakeSetupEnvironment() });
    expect((await pickSetup(ui, run))?.step).toBe("done");
    expect(workspaceStartState(setup.repo, configured)).toBe("started");
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
  });

  it("carries `start --repository --database` through every answer, so set-up starts every checkout in that database", async () => {
    const setup = await machine();
    const worktree = join(setup.root, "acme-api-feature");
    execFileSync("git", ["worktree", "add", "-q", "-b", "feature", worktree], { cwd: setup.repo });
    const configured = join(setup.root, "configured", "rerail.sqlite");
    const ui = fakeUi(["Yes, read acme-api", "Yes, start Rerail"]);

    const last = await pickSetup(ui, steps(setup, fakeSetupEnvironment()), ["start", "--json", "--repository", "--database", configured]);

    expect(last?.step).toBe("done");
    expect([setup.repo, worktree].map(path => workspaceStartState(path, configured))).toEqual(["started", "started"]);
    expect(existsSync(setup.databasePath)).toBe(false);

    // Without a terminal, `rerail setup` prints the same answers as commands, carrying both flags.
    const other = join(setup.root, "other", "rerail.sqlite");
    const said: string[] = [];
    vi.stubEnv("HOME", join(setup.root, "user"));
    const io = { lane: "agent" as const, say: (text: string) => { said.push(text); }, close: () => {} };
    expect(await runSetupCommand(["--repository", "--database", other, "--repo", worktree], setup.context, io)).toBe(0);
    expect(said.join("\n")).toContain(`setup --relay --database ${other} --repository --answer start --repo ${worktree}`);
  });

  it("carries `start --agents` through every answer, so set-up connects only the coding agents named", async () => {
    const setup = await machine();
    const environment = fakeSetupEnvironment();
    const asked: Array<readonly string[]> = [];
    const run = steps(setup, environment);
    const ui = fakeUi(["Yes, read acme-api", "Yes, start Rerail"]);

    const last = await pickSetup(ui, args => { asked.push(args); return run(args); }, ["start", "--json", "--agents", "cursor"]);

    expect(last?.step).toBe("done");
    expect(asked.slice(1)).toEqual([
      ["setup", "--agents", "cursor", "--json", "--answer", "yes", "--repo", setup.repo],
      ["setup", "--agents", "cursor", "--json", "--answer", "start", "--repo", setup.repo],
    ]);
    expect(environment.connections).toEqual([{ agents: ["cursor"], repoRoot: setup.repo }]);
    // The confirm screen names only what the answer connects, not every coding agent found.
    const connectedLine = ui.notices[1]!.message.split("\n").find(line => line.trimStart().startsWith("Connected to"));
    expect(connectedLine).toMatch(/^ {2}Connected to +Cursor\b/u);
    expect(connectedLine).not.toContain("Claude Code");
  });

  it("never installs the session's own package over the active one, so answering after /rerail-update keeps the update", async () => {
    const setup = await machine();
    // The session loaded 0.9.0; `/rerail-update` has since made 1.0.0 the active install.
    const sessionPackage = join(setup.root, "session-package");
    mkdirSync(join(sessionPackage, "dist"), { recursive: true });
    writeFileSync(join(sessionPackage, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "0.9.0" }));
    const updater = new ManagedUpdater({ home: setup.context.home, packageRoot: sessionPackage, packageVersion: "0.9.0" });
    const install = vi.spyOn(updater, "install").mockResolvedValue(undefined as never);
    const session: Machine = { ...setup, context: { home: setup.context.home, packageRoot: sessionPackage, updater } };

    const inRepo = await pickSetup(fakeUi(["Yes, read acme-api", "Yes, start Rerail"]), steps(session, fakeSetupEnvironment()));
    // In the home folder, `start --json` hands over to set-up itself before any answer.
    const home = join(setup.root, "user");
    mkdirSync(home);
    vi.stubEnv("HOME", home);
    const other = gitRepository(join(setup.root, "acme-web"));
    const fromHome = await pickSetup(fakeUi(["Type the folder…", "Yes, start Rerail"], [other]), steps(session, fakeSetupEnvironment(), home));

    expect([inRepo?.step, fromHome?.step]).toEqual(["done", "done"]);
    expect(install).not.toHaveBeenCalled();
    expect(realpathSync(updater.currentLink)).toBe(setup.context.packageRoot);
  });
});
