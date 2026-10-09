import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { runSetup, runSetupStep, startRerailStep, type SetupRequest } from "../src/repository-setup.js";
import { workspaceStartState, type RepositorySetupIo } from "../src/repository-start.js";
import { runSetupCommand, type SetupIo } from "../src/setup-cli.js";
import { CONFIRM_PACKS_ANSWERS, NO_REPO_ANSWERS, READ_REPO_ANSWERS, shellQuote } from "../src/setup-screens.js";
import type { SetupQuestion, SetupStep } from "../src/setup-step.js";
import { ManagedUpdater } from "../src/updater.js";
import { fakeSetupEnvironment, type FakeSetupEnvironment } from "./fake-setup-environment.js";
import { attachedPacks } from "./support/attached-packs.js";

const SETUP_TIMEOUT_MS = 30_000;
const temporaryDirectories: string[] = [];
afterEach(async () => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

interface Fixture {
  root: string;
  databasePath: string;
  repo: string;
}

async function fixture(): Promise<Fixture> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-json-")));
  temporaryDirectories.push(root);
  return { root, databasePath: join(root, "home", "rerail.sqlite"), repo: gitRepository(join(root, "acme api")) };
}

function gitRepository(path: string): string {
  mkdirSync(path, { recursive: true });
  writeFileSync(join(path, "app.ts"), "export const hi = 1;\n");
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: path });
  execFileSync("git", ["add", "."], { cwd: path });
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "-m", "base"], { cwd: path });
  return path;
}

function agentText(run: (io: RepositorySetupIo) => Promise<unknown>): Promise<string> {
  const said: string[] = [];
  return run({ lane: "agent", say: text => { said.push(text); } }).then(() => said.join("\n"));
}

function request(setup: Fixture, environment: FakeSetupEnvironment, extra: Partial<SetupRequest> = {}): SetupRequest {
  return { cwd: setup.repo, databasePath: setup.databasePath, environment, ...extra };
}

/** The agent lane's numbered answers and, after the outro, its numbered commands. */
function agentQuestion(text: string): { answers: string[]; commands: string[] } {
  const lines = text.split("\n");
  const outro = lines.findIndex(line => line.startsWith("Do not choose for them."));
  const commands = lines.slice(outro + 1).filter(line => /^\d+\. /u.test(line));
  const answers = lines.slice(outro - 1 - commands.length, outro - 1);
  return { answers, commands };
}

/** A `--json` answer's arguments back into the request set-up would parse from them. */
function requestFrom(args: readonly string[], setup: Fixture, environment: FakeSetupEnvironment, value?: string): SetupRequest {
  expect(args.slice(0, 2)).toEqual(["setup", "--json"]);
  const flags = new Map<string, string>();
  for (let index = 2; index < args.length; index += 2) flags.set(args[index]!, args[index + 1]!);
  const answer = flags.get("--answer");
  const repo = flags.get("--repo");
  return request(setup, environment, {
    ...(answer === undefined ? {} : { answer }),
    ...(repo === undefined ? {} : { repo }),
    ...(value === undefined ? {} : { repo: value }),
  });
}

/** The same question in both lanes: the same answers in the same order, and each JSON answer runs the agent lane's command. */
function expectParity(question: SetupQuestion, agent: string, keys: readonly string[]): void {
  const { answers, commands } = agentQuestion(agent);
  expect(question.options.map(option => option.answer)).toEqual([...keys]);
  expect(question.options.map((option, index) => `${index + 1}. ${option.label}${option.detail === null ? "" : `  ${option.detail}`}`)).toEqual(answers);
  question.options.forEach((option, index) => {
    const command = ["rerail setup", ...option.next.args.slice(2).map(shellQuote)].join(" ");
    expect(commands[index]).toContain(command);
    if (option.input !== null) expect(commands[index]).toContain(`${command} ${option.input.flag} <`);
  });
  // The person's text in the JSON lane is the agent lane's, without the relay instructions or the question line.
  expect(agent).toContain(question.prompt);
}

function expectPersonText(step: SetupStep): void {
  expect(step.screen).not.toContain("Ask them this");
  expect(step.screen).not.toContain("run set-up again");
  expect(step.screen).not.toContain("Show the person");
  if (step.question !== null) expect(step.screen).not.toContain(step.question.prompt);
}

describe("rerail setup --json", { timeout: SETUP_TIMEOUT_MS }, () => {
  it("asks every question with the agent lane's answers, in its order, each running the agent lane's command", async () => {
    const setup = await fixture();
    const plain = join(setup.root, "downloads");
    mkdirSync(plain);
    const scenarios: Array<[Partial<SetupRequest>, SetupStep["step"], readonly string[]]> = [
      [{}, "read-repo", READ_REPO_ANSWERS],
      [{ answer: "yes", repo: setup.repo }, "confirm-packs", CONFIRM_PACKS_ANSWERS],
      [{ answer: "review", repo: setup.repo }, "confirm-packs", CONFIRM_PACKS_ANSWERS],
      [{ cwd: plain }, "no-repo", NO_REPO_ANSWERS],
    ];
    for (const [extra, step, keys] of scenarios) {
      const agent = await agentText(io => runSetup(io, request(setup, fakeSetupEnvironment(), extra)));
      const json = await runSetupStep(request(setup, fakeSetupEnvironment(), extra));
      expect(json.step).toBe(step);
      expect(json.question?.id).toBe(step);
      expectParity(json.question!, agent, keys);
      expectPersonText(json);
    }
    // The review recorded the checkout, so its dashboard can add packs; nothing is set up or started.
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
    expect(attachedPacks(setup.repo, setup.databasePath)).toEqual([]);
  });

  it("follows its own next.args from the first question to Rerail on, with the agent lane's side effects", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();

    const asked = await runSetupStep(request(setup, environment));
    expect(asked.screen).toContain("It reads on this computer only.");
    const yes = asked.question!.options.find(option => option.answer === "yes")!;
    expect(yes.next.args).toEqual(["setup", "--json", "--answer", "yes", "--repo", setup.repo]);

    const packs = await runSetupStep(requestFrom(yes.next.args, setup, environment));
    expect(packs.step).toBe("confirm-packs");
    expect(packs.screen).toContain("Of the rule packs that come with Rerail, 1 looks right for acme api.");
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
    const other = packs.question!.options.find(option => option.answer === "other")!;
    expect(other.input).toEqual({ flag: "--agents", prompt: "Which coding agents should Rerail connect to?", hint: "comma-separated ids from: claude-code (Claude Code), cursor (Cursor)" });

    const start = packs.question!.options.find(option => option.answer === "start")!;
    const done = await runSetupStep(requestFrom(start.next.args, setup, environment));
    expect(done).toMatchObject({ step: "done", question: null });
    expect(done.screen).toContain("● Rerail is on in acme api. 1 rule pack is on");
    expect(done.screen).toContain("── Good to know · optional");
    expectPersonText(done);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
    expect(attachedPacks(setup.repo, setup.databasePath)).toEqual(["tslint"]);
    expect(environment.connections).toEqual([{ agents: ["claude-code", "cursor"], repoRoot: setup.repo }]);
  });

  it("takes a folder for E3's first answer, and reports a bad answer as an error step", async () => {
    const setup = await fixture();
    const plain = join(setup.root, "downloads");
    mkdirSync(plain);
    const environment = fakeSetupEnvironment();
    const where = await runSetupStep(request(setup, environment, { cwd: plain }));
    const folder = where.question!.options[0]!;
    expect(folder).toMatchObject({ answer: "folder", next: { args: ["setup", "--json"] }, input: { flag: "--repo", prompt: "Which folder?" } });
    const read = await runSetupStep({ ...requestFrom(folder.next.args, setup, environment, setup.repo), cwd: plain });
    expect(read.step).toBe("confirm-packs");

    expect(await runSetupStep(request(setup, environment, { answer: "maybe" }))).toEqual({
      step: "error",
      screen: expect.stringContaining("Unknown answer: maybe"),
      question: null,
    });
  });

  it("starts each answer's arguments with the base it is given, such as the port", async () => {
    const setup = await fixture();
    const step = await runSetupStep(request(setup, fakeSetupEnvironment(), { nextArgs: ["setup", "--port", "4999", "--json"] }));
    expect(step.question!.options[0]!.next.args).toEqual(["setup", "--port", "4999", "--json", "--answer", "yes", "--repo", setup.repo]);
  });
});

describe("rerail start --json", { timeout: SETUP_TIMEOUT_MS }, () => {
  it("asks the set-up's first question where the repository is not set up, and says it is on where it is", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    const first = await startRerailStep(setup.repo, { databasePath: setup.databasePath, environment });
    expect(first.step).toBe("read-repo");
    expect(first.question!.options.map(option => option.next.args[0])).toEqual(["setup", "setup", "setup"]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");

    await runSetup({ lane: "agent", say: () => {} }, request(setup, environment, { answer: "start", repo: setup.repo }));
    const again = await startRerailStep(setup.repo, { databasePath: setup.databasePath, environment });
    expect(again).toMatchObject({ step: "done", question: null });
    expect(again.screen).toMatch(/^● Rerail is already on in acme api\. \d+ rules are on\.$/u);
  });
});

describe("rerail setup, the command", { timeout: SETUP_TIMEOUT_MS }, () => {
  async function commandFixture(): Promise<{ setup: Fixture; context: { home: string; packageRoot: string; updater: ManagedUpdater }; said: string[]; io: SetupIo }> {
    const setup = await fixture();
    const home = join(setup.root, "home");
    const packageRoot = join(setup.root, "package");
    mkdirSync(join(packageRoot, "dist"), { recursive: true });
    writeFileSync(join(packageRoot, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "1.0.0" }));
    vi.stubEnv("RERAIL_HOME", home);
    vi.stubEnv("HOME", join(setup.root, "user"));
    vi.spyOn(process, "cwd").mockReturnValue(setup.repo);
    const said: string[] = [];
    const io: SetupIo = { lane: "agent", say: text => { said.push(text); }, close: () => {} };
    return { setup, context: { home, packageRoot, updater: new ManagedUpdater({ home, packageRoot, packageVersion: "1.0.0" }) }, said, io };
  }

  it("says Rerail is installed only on the run that installed it", async () => {
    const installing = await commandFixture();
    const install = vi.spyOn(installing.context.updater, "install").mockResolvedValue(undefined as never);
    expect(await runSetupCommand([], installing.context, installing.io)).toBe(0);
    expect(install).toHaveBeenCalledOnce();
    expect(installing.said[0]).toBe("Rerail is installed. Nothing is on yet.");

    const installed = await commandFixture();
    await installed.context.updater.activateRoot(installed.context.packageRoot);
    const reinstall = vi.spyOn(installed.context.updater, "install");
    expect(await runSetupCommand([], installed.context, installed.io)).toBe(0);
    expect(reinstall).not.toHaveBeenCalled();
    expect(installed.said.join("\n")).not.toContain("Rerail is installed");
    expect(installed.said.join("\n")).toContain("Can Rerail read it");
  });

  it("prints exactly one JSON step and nothing else, the install line inside it", async () => {
    const { context, said, io, setup } = await commandFixture();
    vi.spyOn(context.updater, "install").mockResolvedValue(undefined as never);
    expect(await runSetupCommand(["--json", "--port", "4999"], context, io)).toBe(0);
    expect(said).toHaveLength(1);
    const step = JSON.parse(said[0]!) as SetupStep;
    expect(step.step).toBe("read-repo");
    expect(step.screen.split("\n")[0]).toBe("Rerail is installed. Nothing is on yet.");
    expect(step.question!.options[0]!.next.args).toEqual(["setup", "--port", "4999", "--json", "--answer", "yes", "--repo", setup.repo]);

    said.length = 0;
    expect(await runSetupCommand(["--json", "--answer", "maybe"], context, io)).toBe(1);
    expect(said).toHaveLength(1);
    expect(JSON.parse(said[0]!)).toMatchObject({ step: "error", question: null });

    said.length = 0;
    expect(await runSetupCommand(["--json"], context, io, "v22.11.0")).toBe(1);
    expect(JSON.parse(said[0]!)).toMatchObject({ step: "error", screen: expect.stringContaining("Rerail needs Node 24 or newer") });
  });
});
