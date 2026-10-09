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
// Passed through, and watched: a run that must read no code never calls the recommender.
const recommendRulePacks = vi.hoisted(() => vi.fn<(root: string) => unknown>());
vi.mock("../../src/engine/catalog/rule-packs.js", async (importOriginal) => {
  const actual = await importOriginal<{ recommendRulePacks(root: string): unknown }>();
  recommendRulePacks.mockImplementation(actual.recommendRulePacks);
  return { ...actual, recommendRulePacks };
});
import { openRerailDatabase } from "../../src/engine/index.js";
import { repositoryIdentityLocation } from "../../src/engine/index.js";
import { lookupCheckout, recordCheckout } from "../../src/engine/index.js";
import { parseCodingAgents, runSetup, startRerail, startRerailStep, type SetupRequest } from "../../src/cli/repository-setup.js";
import { onRuleCount, stopRerail, workspaceStartState, type RepositorySetupIo } from "../../src/runtime/index.js";
import { setUpRepository } from "../../src/engine/index.js";
import { noBrowserReason, runSetupCommand, type SetupIo } from "../../src/cli/setup-cli.js";
import { ManagedUpdater } from "../../src/setup/index.js";
import { attachPacks, fakeSetupEnvironment, type FakeSetupEnvironment } from "../support/fake-setup-environment.js";
import { attachedPacks } from "../support/attached-packs.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  recommendRulePacks.mockClear();
  vi.unstubAllEnvs();
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

const TS_PACKS = ["typescript"];

interface Fixture {
  root: string;
  databasePath: string;
  /** A TypeScript repository: the TypeScript pack fits it. */
  repo: string;
}

async function fixture(): Promise<Fixture> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-setup-")));
  temporaryDirectories.push(root);
  return { root, databasePath: join(root, "home", "rerail.sqlite"), repo: gitRepository(join(root, "acme-api")) };
}

function gitRepository(path: string): string {
  mkdirSync(path, { recursive: true });
  writeFileSync(join(path, "app.ts"), "export const hi = 1;\n");
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: path });
  execFileSync("git", ["add", "."], { cwd: path });
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "-m", "base"], { cwd: path });
  return path;
}

type Recorded = RepositorySetupIo & { said: string[]; text(): string };

/** The coding agent's side: no terminal, so set-up stops at each question. */
function agentIo(): Recorded {
  const said: string[] = [];
  return { lane: "agent", said, say: text => { said.push(text); }, text: () => said.join("\n") };
}

/** A person at a terminal typing `answers` in order; input ends after the last one. */
function terminalIo(answers: readonly string[]): Recorded & { left: string[] } {
  const said: string[] = [];
  const left = [...answers];
  return {
    lane: "terminal", said, left,
    say: text => { said.push(text); },
    ask: async () => left.shift(),
    text: () => said.join("\n"),
  };
}

function request(fixture: Fixture, environment: FakeSetupEnvironment, extra: Partial<SetupRequest> = {}): SetupRequest {
  return { cwd: fixture.repo, databasePath: fixture.databasePath, environment, ...extra };
}

function selected(fixture: Fixture, repo = fixture.repo): readonly string[] {
  return attachedPacks(repo, fixture.databasePath);
}

function enabledRules(fixture: Fixture, repo = fixture.repo): number {
  const database = openRerailDatabase(fixture.databasePath, { readonly: true });
  try {
    return onRuleCount(database, lookupCheckout(database, repo)!);
  } finally {
    database.$client.close();
  }
}

/** Every start and set-up value Rerail holds, with when each last changed. */
function startRows(fixture: Fixture): unknown {
  const database = openRerailDatabase(fixture.databasePath, { readonly: true });
  try {
    return {
      repositories: database.$client.prepare("SELECT repository_key, started, configured_at, updated_at FROM repositories").all(),
      checkouts: database.$client.prepare("SELECT checkout_path, started FROM checkouts").all(),
    };
  } finally {
    database.$client.close();
  }
}

/** Nothing of Rerail's in the repository, its Git metadata, or the database. */
function expectUntouched(fixture: Fixture, repo = fixture.repo): void {
  expect(existsSync(repositoryIdentityLocation(repo).identityPath)).toBe(false);
  expect(existsSync(join(repo, ".rerail"))).toBe(false);
  expect(existsSync(join(repo, ".claude"))).toBe(false);
  expect(execFileSync("git", ["status", "--porcelain", "--ignored"], { cwd: repo, encoding: "utf8" })).toBe("");
}

/** Every start of a repository fails from now on, as a write that fails partway through set-up's would. */
function failStarts(databasePath: string): void {
  const database = openRerailDatabase(databasePath);
  try {
    database.$client.exec("CREATE TRIGGER fail_start BEFORE UPDATE OF started ON repositories WHEN NEW.started = 1 BEGIN SELECT RAISE(ABORT, 'disk full'); END");
  } finally {
    database.$client.close();
  }
}

/** Each test sets real Git repositories up against SQLite; a loaded full-suite run needs more than the default 5 s. */
const SETUP_TIMEOUT_MS = 30_000;

describe("set-up in the coding agent's lane", { timeout: SETUP_TIMEOUT_MS }, () => {
  it("asks before reading, changes nothing until yes, and repeats no side effect when an answer is run again", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();

    const asked = agentIo();
    expect(await runSetup(asked, request(setup, environment))).toEqual({ status: "waiting" });
    expect(asked.text()).toContain("You’re working in acme-api. Can Rerail read it");
    expect(asked.text()).toContain(`rerail setup --answer yes --repo ${setup.repo}`);
    expectUntouched(setup);
    expect(existsSync(setup.databasePath)).toBe(false);

    const recommended = agentIo();
    expect(await runSetup(recommended, request(setup, environment, { answer: "yes", repo: setup.repo }))).toEqual({ status: "waiting" });
    expect(recommended.text()).toContain("Of the rule packs that come with Rerail, 1 looks right for acme-api.");
    expect(recommended.text()).toContain("not found: OMP, Pi, Codex");
    expectUntouched(setup);
    expect(existsSync(setup.databasePath)).toBe(false);
    expect(environment.connections).toEqual([]);

    const started = agentIo();
    const outcome = await runSetup(started, request(setup, environment, { answer: "start", repo: setup.repo }));
    expect(outcome).toMatchObject({ status: "started", repositoryName: "acme-api" });
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
    expect(selected(setup)).toEqual(TS_PACKS);
    expect(environment.connections).toEqual([{ agents: ["claude-code", "cursor"], repoRoot: setup.repo }]);
    expect(environment.pages).toMatchObject([{ repoRoot: setup.repo, page: "rules", open: true }]);
    expect(started.text()).toContain("Rerail is on in acme-api. Connected to Claude Code and Cursor.");
    const firstStart = startRows(setup);

    const again = agentIo();
    expect(await runSetup(again, request(setup, environment, { answer: "start", repo: setup.repo }))).toMatchObject({ status: "started" });
    expect(again.text()).not.toContain("Connected to");
    expect(again.text()).toContain(`Rerail is on in acme-api. 1 rule pack is on, with ${enabledRules(setup)} rules.`);
    expect(again.text()).toContain("http://127.0.0.1:4737/#rules");
    expect(startRows(setup)).toEqual(firstStart);
    expect(selected(setup)).toEqual(TS_PACKS);
    expect(environment.connections).toHaveLength(1);
    expect(environment.pages.at(-1)).toMatchObject({ repoRoot: setup.repo, page: "rules", open: false });
  });

  it("reviews in the dashboard without waiting on it, then starts with the packs kept on there", async () => {
    const setup = await fixture();
    // The review page's first switch adds the packs kept on, which sets the repository up: TypeScript off, complexity on.
    const environment = fakeSetupEnvironment({
      onPage: (scope, page) => (page === "review" ? attachPacks(setup.databasePath, scope, ["complexity"]) : undefined),
    });

    const review = agentIo();
    expect(await runSetup(review, request(setup, environment, { answer: "review", repo: setup.repo }))).toEqual({ status: "waiting" });
    expect(review.text()).toContain("http://127.0.0.1:4737/#rule-packs?review=1");
    expect(review.text()).toContain(`rerail setup --answer start --repo ${setup.repo}`);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
    expect(environment.pages).toMatchObject([{ repoRoot: setup.repo, page: "review", open: true }]);
    expect(environment.connections).toEqual([]);

    // Set-up run again before the person answers: the review's packs stand, the link shows, no second browser opens.
    const again = agentIo();
    expect(await runSetup(again, request(setup, environment))).toEqual({ status: "waiting" });
    expect(again.text()).toContain("Start Rerail in acme-api with the rule packs you kept on?");
    expect(environment.pages.at(-1)).toMatchObject({ page: "review", open: false });
    expect(selected(setup)).toEqual(["complexity"]);

    const started = agentIo();
    await runSetup(started, request(setup, environment, { answer: "start", repo: setup.repo }));
    expect(selected(setup)).toEqual(["complexity"]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
    expect(started.text()).toContain("Rule packs     Cyclomatic complexity (limit 6)");
  });

  it("drops the packs the review added when the person then picks packs themselves", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment({
      onPage: (scope, page) => (page === "review" ? attachPacks(setup.databasePath, scope, ["typescript"]) : undefined),
    });
    await runSetup(agentIo(), request(setup, environment, { answer: "review", repo: setup.repo }));
    expect(selected(setup)).toEqual(["typescript"]);

    const picked = agentIo();
    expect(await runSetup(picked, request(setup, environment, { answer: "pick-myself", repo: setup.repo })))
      .toMatchObject({ status: "started", ruleCount: 0 });
    expect(selected(setup)).toEqual([]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
  });

  it("just starts a repository that `rerail rules add` set up, keeping its rule and asking nothing", async () => {
    const setup = await fixture();
    // What `rerail rules add` (and `/rerail-create`) does in a repository that is not set up: the rule sets it up.
    const writer = openRerailDatabase(setup.databasePath);
    try {
      const { repositoryKey } = recordCheckout(writer, setup.repo);
      setUpRepository(writer, repositoryKey, { packs: [], rules: [{ sourceKind: "library_rule", sourceId: "no-console-log", folder: "" }] });
    } finally {
      writer.$client.close();
    }

    const io = agentIo();
    const report = await startRerail(setup.repo, io, { databasePath: setup.databasePath, environment: fakeSetupEnvironment() });
    expect(report).toMatchObject({ previous: "stopped", setUpRan: false, status: "started", cancelled: false });
    expect(io.text()).toMatch(/^● Rerail is on in acme-api\. .*, as you left them\.$/u);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
    const reader = openRerailDatabase(setup.databasePath, { readonly: true });
    try {
      expect(reader.$client.prepare("SELECT source_kind, source_id FROM rule_attachments").all())
        .toEqual([{ source_kind: "library_rule", source_id: "no-console-log" }]);
    } finally {
      reader.$client.close();
    }
  });

  it("takes a repository a coding agent set up after the review was offered as set up, not as the review's", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    // Set-up offers the review; the person never changes anything there.
    await runSetup(agentIo(), request(setup, environment, { answer: "review", repo: setup.repo }));
    const { repositoryKey } = environment.pages[0]!;
    // Meanwhile the coding agent adds a pack over MCP, which sets the repository up.
    await attachPacks(setup.databasePath, { repositoryKey, checkoutPath: setup.repo }, ["complexity"], {
      kind: "agent", checkout: setup.repo, folder: "",
    });

    // "I'll pick rule packs myself" removes nothing the review did not add.
    expect(await runSetup(agentIo(), request(setup, environment, { answer: "pick-myself", repo: setup.repo }))).toMatchObject({ status: "stopped" });
    expect(selected(setup)).toEqual(["complexity"]);

    const io = agentIo();
    const report = await startRerail(setup.repo, io, { databasePath: setup.databasePath, environment });
    expect(report).toMatchObject({ setUpRan: false, status: "started" });
    expect(io.text()).not.toContain("kept on");
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
    expect(selected(setup)).toEqual(["complexity"]);
  });

  it("asks the review's confirm question on `rerail start` once the review set the repository up", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment({
      onPage: (scope, page) => (page === "review" ? attachPacks(setup.databasePath, scope, ["complexity"]) : undefined),
    });
    await runSetup(agentIo(), request(setup, environment, { answer: "review", repo: setup.repo }));

    const io = agentIo();
    const report = await startRerail(setup.repo, io, { databasePath: setup.databasePath, environment });
    expect(report).toMatchObject({ setUpRan: true, status: "waiting" });
    expect(io.text()).toContain("Start Rerail in acme-api with the rule packs you kept on?");
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");

    // Once started and stopped, it is plainly set up: starting again asks nothing.
    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: setup.repo }));
    stopRerail(setup.repo, agentIo(), { databasePath: setup.databasePath });
    expect(await startRerail(setup.repo, agentIo(), { databasePath: setup.databasePath, environment })).toMatchObject({ setUpRan: false, status: "started" });
    expect(selected(setup)).toEqual(["complexity"]);
  });

  it("offers no rule pack where none matches, and starts with none", async () => {
    const setup = await fixture();
    const docs = join(setup.root, "docs");
    mkdirSync(docs);
    writeFileSync(join(docs, "README.md"), "# docs\n");
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: docs });
    const asked = agentIo();
    await runSetup(asked, request(setup, fakeSetupEnvironment(), { answer: "yes", repo: docs }));
    expect(asked.text()).toContain("None of the rule packs that come with Rerail look right for docs.");
    expect(asked.text()).toContain("Start Rerail in docs with no rule packs on?");
    expect(asked.text()).not.toContain("starts in Steer");

    await runSetup(agentIo(), request(setup, fakeSetupEnvironment(), { answer: "start", repo: docs }));
    expect(workspaceStartState(docs, setup.databasePath)).toBe("started");
    expect(selected(setup, docs)).toEqual([]);
  });

  it("connects only the coding agents named with --agents, and a later plain run connects no others", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: setup.repo, agents: parseCodingAgents("Codex") }));
    expect(environment.connections).toEqual([{ agents: ["codex"], repoRoot: setup.repo }]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");

    const later = agentIo();
    await runSetup(later, request(setup, environment));
    expect(environment.connections).toHaveLength(1);
    expect(later.text()).toMatch(/Connected\s+Codex/u);
  });

  it("takes a named folder as the go-ahead to read it, and leaves the repo it ran in alone", async () => {
    const setup = await fixture();
    const other = gitRepository(join(setup.root, "landing-page"));
    const environment = fakeSetupEnvironment();

    const recommended = agentIo();
    expect(await runSetup(recommended, request(setup, environment, { repo: other }))).toEqual({ status: "waiting" });
    expect(recommended.text()).toContain("looks right for landing-page.");
    expect(recommended.text()).toContain(`rerail setup --answer start --repo ${other}`);
    expectUntouched(setup, other);

    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: other }));
    expect(workspaceStartState(other, setup.databasePath)).toBe("started");
    expectUntouched(setup);
  });

  it("starts with every pack off, reading no code, when the person picks packs themselves", async () => {
    const setup = await fixture();
    const picked = agentIo();
    expect(await runSetup(picked, request(setup, fakeSetupEnvironment(), { answer: "pick-myself", repo: setup.repo })))
      .toMatchObject({ status: "started", ruleCount: 0 });
    expect(picked.text()).toContain("Rerail is on in acme-api. No rule packs are on yet.");
    expect(selected(setup)).toEqual([]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
  });

  it("asks for a folder where there is no repo, and skipping connects the coding agents and starts nothing", async () => {
    const setup = await fixture();
    const plain = join(setup.root, "downloads");
    mkdirSync(plain);
    const environment = fakeSetupEnvironment();

    const asked = agentIo();
    expect(await runSetup(asked, request(setup, environment, { cwd: plain }))).toEqual({ status: "waiting" });
    expect(asked.text()).toContain("No repo found here.");
    expect(asked.text()).toContain("rerail setup --answer skip");

    const skipped = agentIo();
    expect(await runSetup(skipped, request(setup, environment, { cwd: plain, answer: "skip" }))).toEqual({ status: "skipped" });
    expect(skipped.text()).toContain("Rerail is installed. It is not on in any repo yet.");
    expect(environment.connections).toEqual([{ agents: ["claude-code", "cursor"], repoRoot: undefined }]);
    expect(existsSync(setup.databasePath)).toBe(false);
    expect(existsSync(join(plain, ".rerail"))).toBe(false);
  });

  it("shows the link and says why when no browser can open", async () => {
    const setup = await fixture();
    const summary = agentIo();
    await runSetup(summary, request(setup, fakeSetupEnvironment({ browser: false }), { answer: "start", repo: setup.repo }));
    expect(summary.text()).toContain("http://127.0.0.1:4737/#rules");
    expect(summary.text()).toContain("Couldn’t open a browser from here (this looks like a remote session).");
  });

  it("rejects an answer set-up does not know", async () => {
    const setup = await fixture();
    await expect(runSetup(agentIo(), request(setup, fakeSetupEnvironment(), { answer: "maybe" }))).rejects.toThrow("Unknown answer: maybe");
  });

  it("shows the review link again on a re-run before the review changed anything, opening the browser once", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    await runSetup(agentIo(), request(setup, environment, { answer: "review", repo: setup.repo }));

    const plain = agentIo();
    expect(await runSetup(plain, request(setup, environment))).toEqual({ status: "waiting" });
    expect(plain.text()).not.toContain("Can Rerail read it");
    expect(plain.text()).toContain("http://127.0.0.1:4737/#rule-packs?review=1");

    const reviewAgain = agentIo();
    expect(await runSetup(reviewAgain, request(setup, environment, { answer: "review", repo: setup.repo }))).toEqual({ status: "waiting" });
    expect(reviewAgain.text()).toContain("http://127.0.0.1:4737/#rule-packs?review=1");

    expect(environment.pages.map(({ page, open }) => ({ page, open }))).toEqual([
      { page: "review", open: true },
      { page: "review", open: false },
      { page: "review", open: false },
    ]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
  });

  it("asks the confirm question again, with the error, for an answer's coding agents it does not know", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    const asked = agentIo();
    expect(await runSetup(asked, request(setup, environment, { answer: "start", repo: setup.repo, agentsError: "Unknown coding agent: only Claude Code." })))
      .toEqual({ status: "waiting" });
    expect(asked.text()).toContain("Unknown coding agent: only Claude Code.");
    expect(asked.text()).toContain("Start Rerail in acme-api with this rule pack?");
    expect(environment.connections).toEqual([]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
  });

  it("asks the read-repo question again, reading nothing, when picking packs myself names coding agents it does not know", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    const asked = agentIo();
    expect(await runSetup(asked, request(setup, environment, { answer: "pick-myself", repo: setup.repo, agentsError: "Unknown coding agent: nope." })))
      .toEqual({ status: "waiting" });
    expect(asked.text()).toContain("Unknown coding agent: nope.");
    expect(asked.text()).toContain("You’re working in acme-api. Can Rerail read it");
    expect(recommendRulePacks).not.toHaveBeenCalled();
    expect(environment.connections).toEqual([]);
    expect(existsSync(setup.databasePath)).toBe(false);
    expectUntouched(setup);
  });

  it("refuses the home folder named with --repo, and asks which project instead", async () => {
    const setup = await fixture();
    const home = join(setup.root, "user");
    mkdirSync(home);
    vi.stubEnv("HOME", home);
    const asked = agentIo();
    expect(await runSetup(asked, request(setup, fakeSetupEnvironment(), { answer: "start", repo: "~" }))).toEqual({ status: "waiting" });
    expect(asked.text()).toContain("Start Rerail inside a repository, not your home folder.");
    expect(asked.text()).toContain("Which project do you want to start with?");
    expect(recommendRulePacks).not.toHaveBeenCalled();
    expect(existsSync(setup.databasePath)).toBe(false);
  });

  it("writes nothing when starting fails partway", async () => {
    const setup = await fixture();
    failStarts(setup.databasePath);
    const before = startRows(setup);

    // A fresh start: the checkout, the set-up and its packs are written before the start fails.
    await expect(runSetup(agentIo(), request(setup, fakeSetupEnvironment(), { answer: "start", repo: setup.repo }))).rejects.toThrow("disk full");
    expect(startRows(setup)).toEqual(before);
    const reader = openRerailDatabase(setup.databasePath, { readonly: true });
    try {
      expect(reader.$client.prepare("SELECT count(*) AS count FROM rule_attachments").get()).toEqual({ count: 0 });
    } finally {
      reader.$client.close();
    }
  });

  it("keeps the review's packs when picking packs myself fails at the start", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment({
      onPage: (scope, page) => (page === "review" ? attachPacks(setup.databasePath, scope, ["typescript", "complexity"]) : undefined),
    });
    await runSetup(agentIo(), request(setup, environment, { answer: "review", repo: setup.repo }));
    failStarts(setup.databasePath);

    // Each pack is removed before the start fails; none of it stands.
    await expect(runSetup(agentIo(), request(setup, environment, { answer: "pick-myself", repo: setup.repo }))).rejects.toThrow("disk full");
    expect(selected(setup)).toEqual(["complexity", "typescript"]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
  });
});

describe("set-up in a terminal", { timeout: SETUP_TIMEOUT_MS }, () => {
  it("says yes, starts with the recommended packs, and waits for Enter before the summary", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    const io = terminalIo(["", "1", ""]);
    expect(await runSetup(io, request(setup, environment))).toMatchObject({ status: "started" });
    expect(io.left).toEqual([]);
    expect(selected(setup)).toEqual(TS_PACKS);
    expect(io.text()).toContain("Enter to see the summary.");
    expect(io.text()).toContain("Good to know · optional");
  });

  it("reviews first, then confirms back in the terminal", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    const io = terminalIo(["1", "2", "1", ""]);
    await runSetup(io, request(setup, environment));
    expect(environment.pages.map(({ page }) => page)).toEqual(["review", "rules"]);
    expect(io.text()).toContain("Start Rerail in acme-api with the rule packs you kept on?");
    expect(selected(setup)).toEqual(TS_PACKS);
  });

  it("reads a different folder instead, leaving this one untouched", async () => {
    const setup = await fixture();
    const other = gitRepository(join(setup.root, "landing-page"));
    const io = terminalIo(["2", join(setup.root, "missing"), other, "1", ""]);
    await runSetup(io, request(setup, fakeSetupEnvironment()));
    expect(io.text()).toContain(`There is no folder at ${join(setup.root, "missing")}.`);
    expect(workspaceStartState(other, setup.databasePath)).toBe("started");
    expect(selected(setup, other)).toEqual(TS_PACKS);
    expectUntouched(setup);
  });

  it("starts with every pack off when the person picks packs themselves", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    const io = terminalIo(["3"]);
    expect(await runSetup(io, request(setup, environment))).toMatchObject({ status: "started", ruleCount: 0 });
    expect(io.text()).toContain("Rerail did not read any code.");
    expect(selected(setup)).toEqual([]);
    expect(enabledRules(setup)).toBe(0);
    expect(environment.connections).toEqual([{ agents: ["claude-code", "cursor"], repoRoot: setup.repo }]);
  });

  it("connects the coding agents the person types, after an unknown one", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    const io = terminalIo(["1", "3", "vim", "claude code", ""]);
    await runSetup(io, request(setup, environment));
    expect(io.text()).toContain("Unknown coding agent: vim.");
    expect(environment.connections).toEqual([{ agents: ["claude-code"], repoRoot: setup.repo }]);
  });

  it("takes a typed folder where there is no repo", async () => {
    const setup = await fixture();
    const plain = join(setup.root, "downloads");
    mkdirSync(plain);
    const io = terminalIo(["1", setup.repo, "1", ""]);
    await runSetup(io, request(setup, fakeSetupEnvironment(), { cwd: plain }));
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
    expect(existsSync(join(plain, ".rerail"))).toBe(false);
  });

  it("refuses the home folder typed at the folder prompt, from either question that offers one", async () => {
    const setup = await fixture();
    const home = join(setup.root, "user");
    mkdirSync(home);
    vi.stubEnv("HOME", home);
    const plain = join(setup.root, "downloads");
    mkdirSync(plain);
    const environment = fakeSetupEnvironment();

    // Where there is no repo ("Type the folder…"), and instead of the repo set-up ran in ("Use a different folder").
    const noRepo = terminalIo(["1", "~"]);
    expect(await runSetup(noRepo, request(setup, environment, { cwd: plain }))).toEqual({ status: "cancelled" });
    const otherFolder = terminalIo(["2", home]);
    expect(await runSetup(otherFolder, request(setup, environment))).toEqual({ status: "cancelled" });
    for (const io of [noRepo, otherFolder]) {
      expect(io.text()).toContain("Start Rerail inside a repository, not your home folder.");
      expect(io.text()).toContain("Which project do you want to start with?");
    }
    expect(recommendRulePacks).not.toHaveBeenCalled();
    expect(existsSync(setup.databasePath)).toBe(false);
  });

  it("skips for now where there is no repo: connected, on nowhere", async () => {
    const setup = await fixture();
    const plain = join(setup.root, "downloads");
    mkdirSync(plain);
    const environment = fakeSetupEnvironment();
    const io = terminalIo(["3"]);
    expect(await runSetup(io, request(setup, environment, { cwd: plain }))).toEqual({ status: "skipped" });
    expect(io.text()).toContain("Next: open a session in one of your repos and type /rerail-start.");
    expect(environment.connections).toEqual([{ agents: ["claude-code", "cursor"], repoRoot: undefined }]);
    expect(existsSync(setup.databasePath)).toBe(false);
  });

  it("still shows the summary when input ends at the pause after Rerail is on", async () => {
    const setup = await fixture();
    const io = terminalIo(["", "1"]);
    expect(await runSetup(io, request(setup, fakeSetupEnvironment()))).toMatchObject({ status: "started" });
    expect(io.text()).toContain("Manage rules");
  });

  it("reports a cancelled first start, and refuses a folder that is not there", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    const report = await startRerail(setup.repo, terminalIo([]), { databasePath: setup.databasePath, environment });
    expect(report).toMatchObject({ previous: "stopped", setUpRan: true, cancelled: true });
    expectUntouched(setup);

    const missing = join(setup.root, "acme-apu");
    await expect(startRerail(missing, terminalIo(["3"]), { databasePath: setup.databasePath, environment })).rejects.toThrow("There is no folder at");
    expect(existsSync(missing)).toBe(false);
  });

  it("changes nothing when input ends before an answer", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    expect(await runSetup(terminalIo(["1"]), request(setup, environment))).toEqual({ status: "cancelled" });
    expectUntouched(setup);
    expect(existsSync(setup.databasePath)).toBe(false);
    expect(environment.connections).toEqual([]);
  });

  it("does not set a stopped repository up again", async () => {
    const setup = await fixture();
    await runSetup(agentIo(), request(setup, fakeSetupEnvironment(), { answer: "start", repo: setup.repo }));
    stopRerail(setup.repo, agentIo(), { databasePath: setup.databasePath });
    const io = terminalIo([]);
    expect(await runSetup(io, request(setup, fakeSetupEnvironment()))).toMatchObject({ status: "stopped" });
    expect(io.text()).toContain("Rerail is set up in acme-api, and stopped there. Start it again with rerail start.");
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
  });
});

/** A database written before set-up kept its offer of the review: no `setup_reviews` table, opened read-only. */
function dropSetupReviews(databasePath: string): void {
  const database = openRerailDatabase(databasePath);
  try {
    database.$client.exec("DROP TABLE setup_reviews");
  } finally {
    database.$client.close();
  }
}

describe("a database from before set-up kept its offer of the review", { timeout: SETUP_TIMEOUT_MS }, () => {
  it("asks the read-repo question where the checkout is recorded but not set up", async () => {
    const setup = await fixture();
    const writer = openRerailDatabase(setup.databasePath);
    try {
      recordCheckout(writer, setup.repo);
    } finally {
      writer.$client.close();
    }
    dropSetupReviews(setup.databasePath);
    const environment = fakeSetupEnvironment();

    const step = await startRerailStep(setup.repo, { databasePath: setup.databasePath, environment });
    expect(step).toMatchObject({ step: "read-repo" });
    const started = agentIo();
    expect(await startRerail(setup.repo, started, { databasePath: setup.databasePath, environment })).toMatchObject({ setUpRan: true, status: "waiting" });
    expect(started.text()).toContain("Can Rerail read it");
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
  });

  it("just starts a repository that is set up and was never started", async () => {
    const setup = await fixture();
    await runSetup(agentIo(), request(setup, fakeSetupEnvironment(), { answer: "start", repo: setup.repo }));
    const writer = openRerailDatabase(setup.databasePath);
    try {
      // Set up some other way (`rerail rules add`) and never started.
      writer.$client.exec("UPDATE repositories SET started = NULL; UPDATE checkouts SET started = NULL");
    } finally {
      writer.$client.close();
    }
    dropSetupReviews(setup.databasePath);

    const started = agentIo();
    expect(await startRerail(setup.repo, started, { databasePath: setup.databasePath, environment: fakeSetupEnvironment() }))
      .toMatchObject({ setUpRan: false, status: "started" });
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
  });
});

describe("rerail setup", () => {
  it("stops before changing anything on a Node older than 24", async () => {
    const setup = await fixture();
    const home = join(setup.root, "rerail-home");
    const packageRoot = join(setup.root, "package");
    const said: string[] = [];
    const io: SetupIo = { lane: "terminal", say: text => { said.push(text); }, close: () => {} };
    const context = { home, packageRoot, updater: new ManagedUpdater({ home, packageRoot, packageVersion: "1.0.0" }) };
    expect(await runSetupCommand([], context, io, "v22.11.0")).toBe(1);
    expect(said.join("\n")).toContain("Rerail needs Node 24 or newer. You have Node 22.11.0.");
    expect(existsSync(home)).toBe(false);
  });

  it("opens no browser over SSH, without a display, or when BROWSER is none", () => {
    expect(noBrowserReason({ SSH_CONNECTION: "10.0.0.1 22 10.0.0.2 22" }, "darwin")).toBe("this looks like a remote session");
    expect(noBrowserReason({}, "linux")).toBe("there is no display here");
    expect(noBrowserReason({ BROWSER: "none", DISPLAY: ":0" }, "linux")).toBe("BROWSER is set to none");
    expect(noBrowserReason({ DISPLAY: ":0" }, "linux")).toBeUndefined();
    expect(noBrowserReason({}, "darwin")).toBeUndefined();
  });
});
