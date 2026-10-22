import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

// Set-up fetches rule packs through the fake environment's store: this repository's TypeScript pack.
// Passed through, and watched: a run that must read no code never calls the recommender.
const recommendRulePacks = vi.hoisted(() => vi.fn<(root: string) => unknown>());
vi.mock("../../src/engine/catalog/rule-packs.js", async (importOriginal) => {
  const actual = await importOriginal<{ recommendRulePacks(root: string): unknown }>();
  recommendRulePacks.mockImplementation(actual.recommendRulePacks);
  return { ...actual, recommendRulePacks };
});
import { openRerailDatabase } from "../../src/engine/index.js";
import { repositoryIdentityLocation } from "../../src/engine/index.js";
import { listRepositoryRules, loadRuleCatalog, lookupCheckout, recordCheckout, ruleSettings } from "../../src/engine/index.js";
import { parseCodingAgents, runSetup, setupPacks, startRerail, startRerailStep, type SetupRequest } from "../../src/cli/repository-setup.js";
import { onRuleCount, stopRerail, workspaceStartState, type RepositorySetupIo } from "../../src/runtime/index.js";
import { offerSetupReview, setUpRepository } from "../../src/engine/index.js";
import { noBrowserReason, runSetupCommand, type SetupIo } from "../../src/cli/setup-cli.js";
import { installedPackVersion, ManagedUpdater } from "../../src/setup/index.js";
import { directoryPackStore, repositoryPackVersions } from "../support/directory-pack-store.js";
import { installPack, installPackDirectory, removeInstalledPacks } from "../support/installed-packs.js";
import { repositoryPath } from "../support/oracles.js";
import { attachPacks, fakeSetupEnvironment, type FakeSetupEnvironment } from "../support/fake-setup-environment.js";
import { attachedPacks } from "../support/attached-packs.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  recommendRulePacks.mockClear();
  removeInstalledPacks();
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
/** The pack set-up offers the TypeScript fixture, at the version its "Yes" answer carries (`--packs`). */
const TYPESCRIPT = repositoryPackVersions("typescript");

/**
 * A repository an earlier version's set-up offered the dashboard's review in: its checkout recorded and the offer kept,
 * as `offerSetupReview` left it. Set-up no longer offers the review, but goes straight to its confirm question there.
 */
function offerReview(fixture: Fixture, repo = fixture.repo): { repositoryKey: string; checkoutPath: string } {
  const database = openRerailDatabase(fixture.databasePath);
  try {
    const { repositoryKey, checkoutPath } = recordCheckout(database, repo);
    offerSetupReview(database, repositoryKey);
    return { repositoryKey, checkoutPath };
  } finally {
    database.$client.close();
  }
}

describe("set-up in the coding agent's lane", { timeout: SETUP_TIMEOUT_MS }, () => {
  it("asks before reading, changes nothing until yes, and repeats no side effect when an answer is run again", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();

    const asked = agentIo();
    expect(await runSetup(asked, request(setup, environment))).toEqual({ status: "waiting" });
    expect(asked.text()).toContain("Find rule packs for acme-api?");
    expect(asked.text()).toContain(`rerail setup --answer yes --repo ${setup.repo}`);
    expectUntouched(setup);
    expect(existsSync(setup.databasePath)).toBe(false);

    const recommended = agentIo();
    expect(await runSetup(recommended, request(setup, environment, { answer: "yes", repo: setup.repo }))).toEqual({ status: "waiting" });
    expect(recommended.text()).toContain("Scanned acme-api. 1 rule pack fits:");
    expect(recommended.text()).toContain("Will connect to: Claude Code, Cursor");
    expectUntouched(setup);
    expect(existsSync(setup.databasePath)).toBe(false);
    expect(environment.connections).toEqual([]);

    const started = agentIo();
    const outcome = await runSetup(started, request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
    expect(outcome).toMatchObject({ status: "started", repositoryName: "acme-api" });
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
    expect(selected(setup)).toEqual(TS_PACKS);
    expect(environment.connections).toEqual([{ agents: ["claude-code", "cursor"], repoRoot: setup.repo }]);
    expect(environment.pages).toMatchObject([{ repoRoot: setup.repo, page: "rules", open: true }]);
    expect(started.text()).toContain(`Rerail is on in acme-api · 1 rule pack, ${enabledRules(setup)} rules`);
    const firstStart = startRows(setup);

    const again = agentIo();
    expect(await runSetup(again, request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }))).toMatchObject({ status: "started" });
    expect(again.text()).toContain(`Rerail is on in acme-api · 1 rule pack, ${enabledRules(setup)} rules`);
    expect(again.text()).toContain("http://127.0.0.1:4737/#rules");
    expect(startRows(setup)).toEqual(firstStart);
    expect(selected(setup)).toEqual(TS_PACKS);
    expect(environment.connections).toHaveLength(1);
    expect(environment.pages.at(-1)).toMatchObject({ repoRoot: setup.repo, page: "rules", open: false });
  });

  it("goes straight to the confirm question where an earlier set-up offered the review, keeping the packs kept on there", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment({ packs: directoryPackStore(undefined, ["typescript", "complexity"]) });
    const scope = offerReview(setup);

    // Nothing changed in the dashboard yet: the confirm question, never the read-repo question, and no review link.
    const plain = agentIo();
    expect(await runSetup(plain, request(setup, environment))).toEqual({ status: "waiting" });
    expect(plain.text()).not.toContain("Find rule packs for");
    expect(plain.text()).toContain("Start Rerail in acme-api with");
    expect(plain.text()).not.toContain("review=1");
    expect(environment.pages).toEqual([]);

    // The person's first switch in the dashboard set it up: TypeScript off, complexity on. `rerail start` asks again.
    installPack("complexity");
    await attachPacks(setup.databasePath, scope, ["complexity"]);
    const report = await startRerail(setup.repo, agentIo(), { databasePath: setup.databasePath, environment });
    expect(report).toMatchObject({ setUpRan: true, status: "waiting" });
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");

    // "Yes" starts it with the packs kept on there.
    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
    expect(selected(setup)).toEqual(["complexity"]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");

    // Once started and stopped, it is plainly set up: starting again asks nothing.
    stopRerail(setup.repo, agentIo(), { databasePath: setup.databasePath });
    expect(await startRerail(setup.repo, agentIo(), { databasePath: setup.databasePath, environment })).toMatchObject({ setUpRan: false, status: "started" });
    expect(selected(setup)).toEqual(["complexity"]);
  });

  it("replaces the packs kept on in an earlier review with the ones the person chooses", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment({ packs: directoryPackStore(undefined, ["typescript", "complexity"]) });
    const scope = offerReview(setup);
    installPack("typescript");
    installPack("complexity");
    await attachPacks(setup.databasePath, scope, ["typescript", "complexity"]);
    expect(selected(setup)).toEqual(["complexity", "typescript"]);

    expect(await runSetup(agentIo(), request(setup, environment, { answer: "choose", repo: setup.repo, packs: repositoryPackVersions("complexity") })))
      .toMatchObject({ status: "started" });
    expect(selected(setup)).toEqual(["complexity"]);
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
    // An earlier set-up offered the review; the person never changes anything there.
    const { repositoryKey } = offerReview(setup);
    // Meanwhile the coding agent installs a pack and adds it over MCP, which sets the repository up.
    installPack("complexity");
    await attachPacks(setup.databasePath, { repositoryKey, checkoutPath: setup.repo }, ["complexity"], {
      kind: "agent", checkout: setup.repo, folder: "",
    });

    // "I'll pick rule packs myself" removes nothing the review did not add.
    expect(await runSetup(agentIo(), request(setup, environment, { answer: "pick-myself", repo: setup.repo, packs: [] }))).toMatchObject({ status: "stopped" });
    expect(selected(setup)).toEqual(["complexity"]);

    const io = agentIo();
    const report = await startRerail(setup.repo, io, { databasePath: setup.databasePath, environment });
    expect(report).toMatchObject({ setUpRan: false, status: "started" });
    expect(io.text()).not.toContain("kept on");
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
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
    expect(asked.text()).toContain("Scanned docs. None of Rerail’s rule packs fit it.");
    expect(asked.text()).toContain("Start Rerail in docs with no rule packs on?");

    await runSetup(agentIo(), request(setup, fakeSetupEnvironment(), { answer: "start", repo: docs }));
    expect(workspaceStartState(docs, setup.databasePath)).toBe("started");
    expect(selected(setup, docs)).toEqual([]);
  });

  it("connects only the coding agents named with --agents, and a later plain run connects no others", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT, agents: parseCodingAgents("Codex") }));
    expect(environment.connections).toEqual([{ agents: ["codex"], repoRoot: setup.repo }]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");

    const later = agentIo();
    await runSetup(later, request(setup, environment));
    expect(environment.connections).toHaveLength(1);
    expect(later.text()).toContain("Rerail is on in acme-api");
  });

  it("takes a named folder as the go-ahead to read it, and leaves the repo it ran in alone", async () => {
    const setup = await fixture();
    const other = gitRepository(join(setup.root, "landing-page"));
    const environment = fakeSetupEnvironment();

    const recommended = agentIo();
    expect(await runSetup(recommended, request(setup, environment, { repo: other }))).toEqual({ status: "waiting" });
    expect(recommended.text()).toContain("Scanned landing-page.");
    expect(recommended.text()).toContain(`rerail setup --answer start --repo ${other}`);
    expectUntouched(setup, other);

    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: other }));
    expect(workspaceStartState(other, setup.databasePath)).toBe("started");
    expectUntouched(setup);
  });

  it("starts with every pack off, reading no code, when the person picks packs themselves", async () => {
    const setup = await fixture();
    const picked = agentIo();
    expect(await runSetup(picked, request(setup, fakeSetupEnvironment(), { answer: "pick-myself", repo: setup.repo, packs: [] })))
      .toMatchObject({ status: "started", ruleCount: 0 });
    expect(picked.text()).toContain("Rerail is on in acme-api. No rule packs are on yet.");
    expect(selected(setup)).toEqual([]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
  });

  it("installs, connects and turns on nothing when the person answers not now", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    const asked = agentIo();
    await runSetup(asked, request(setup, environment, { answer: "yes", repo: setup.repo }));
    expect(asked.text()).toContain("rerail setup --answer not-now");

    const notNow = agentIo();
    expect(await runSetup(notNow, request(setup, environment, { answer: "not-now", repo: setup.repo }))).toEqual({ status: "not-now" });
    expect(notNow.text()).toContain("Set-up stopped. Nothing was installed or turned on.");
    expect(notNow.text()).toContain("/rerail-start");
    expect(environment.connections).toEqual([]);
    expect(existsSync(setup.databasePath)).toBe(false);
    expect(installedPackVersion("typescript", environment.packs.home)).toBeNull();
    expectUntouched(setup);
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
    await runSetup(summary, request(setup, fakeSetupEnvironment({ browser: false }), { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
    expect(summary.text()).toContain("http://127.0.0.1:4737/#rules");
    expect(summary.text()).toContain("Couldn’t open your browser. Copy the link instead.");
  });

  it("rejects an answer set-up does not know", async () => {
    const setup = await fixture();
    await expect(runSetup(agentIo(), request(setup, fakeSetupEnvironment(), { answer: "maybe" }))).rejects.toThrow("Unknown answer: maybe");
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
    expect(asked.text()).toContain("Find rule packs for acme-api?");
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
    await expect(runSetup(agentIo(), request(setup, fakeSetupEnvironment(), { answer: "start", repo: setup.repo, packs: TYPESCRIPT }))).rejects.toThrow("disk full");
    expect(startRows(setup)).toEqual(before);
    const reader = openRerailDatabase(setup.databasePath, { readonly: true });
    try {
      expect(reader.$client.prepare("SELECT count(*) AS count FROM rule_attachments").get()).toEqual({ count: 0 });
    } finally {
      reader.$client.close();
    }
  });

  it("keeps an earlier review's packs when picking packs myself fails at the start", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment({ packs: directoryPackStore(undefined, ["typescript", "complexity"]) });
    installPack("typescript");
    installPack("complexity");
    await attachPacks(setup.databasePath, offerReview(setup), ["typescript", "complexity"]);
    failStarts(setup.databasePath);

    // Each pack is removed before the start fails; none of it stands.
    await expect(runSetup(agentIo(), request(setup, environment, { answer: "pick-myself", repo: setup.repo, packs: [] }))).rejects.toThrow("disk full");
    expect(selected(setup)).toEqual(["complexity", "typescript"]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
  });
});

/** Each rule of `ruleIds` as the repository's settings leave it: on, and its option values. */
function rulesOf(fixture: Fixture, ruleIds: readonly string[]): Record<string, { on: boolean; config: unknown }> {
  const database = openRerailDatabase(fixture.databasePath, { readonly: true });
  try {
    const checkout = lookupCheckout(database, fixture.repo)!;
    const rules = listRepositoryRules(database, loadRuleCatalog(), checkout.repositoryKey).rules;
    return Object.fromEntries(ruleIds.map(id => {
      const rule = rules.find(candidate => candidate.document.rule.id === id)!;
      return [id, { on: rule.on, config: rule.config }];
    }));
  } finally {
    database.$client.close();
  }
}

const TYPE_DEFINITIONS = "typescript-eslint-consistent-type-definitions";
const TSLINT_COMMENT = "typescript-eslint-ban-tslint-comment";

describe("set-up and the repository's own lint config", { timeout: SETUP_TIMEOUT_MS }, () => {
  it("names the settings it takes from .eslintrc.json on the confirm screen, and writes them on start", async () => {
    const setup = await fixture();
    writeFileSync(join(setup.repo, ".eslintrc.json"), JSON.stringify({
      rules: { "@typescript-eslint/consistent-type-definitions": ["error", "type"], "@typescript-eslint/ban-tslint-comment": "off" },
    }));
    const environment = fakeSetupEnvironment();
    const asked = agentIo();
    await runSetup(asked, request(setup, environment, { answer: "yes", repo: setup.repo }));
    expect(asked.text()).toContain("From your lint config (read as data, never run):");
    expect(asked.text()).toContain(`${TYPE_DEFINITIONS} starts with option = "type"  .eslintrc.json: "@typescript-eslint/consistent-type-definitions": ["error","type"]`);
    expect(asked.text()).toContain(`${TSLINT_COMMENT} starts off  .eslintrc.json: "@typescript-eslint/ban-tslint-comment": "off"`);

    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
    expect(rulesOf(setup, [TYPE_DEFINITIONS, TSLINT_COMMENT])).toEqual({
      // The pack keeps consistent-type-definitions off; the repository's option is there for when it is turned on.
      [TYPE_DEFINITIONS]: { on: false, config: { option: "type" } },
      [TSLINT_COMMENT]: { on: false, config: null },
    });
  });

  it("starts the ESLint ports off for an eslint.config.ts it cannot read statically, and takes an .oxlintrc.json off where an earlier review set the repository up", async () => {
    const setup = await fixture();
    writeFileSync(join(setup.repo, "eslint.config.ts"), "throw new Error(\"run\");\n");
    writeFileSync(join(setup.repo, ".oxlintrc.json"), "{ // ours\n  \"rules\": { \"typescript/ban-tslint-comment\": \"off\", },\n}\n");
    const environment = fakeSetupEnvironment();
    installPack("typescript");
    await attachPacks(setup.databasePath, offerReview(setup), ["typescript"]);
    const review = agentIo();
    await runSetup(review, request(setup, environment));
    expect(review.text()).toContain("eslint.config.ts is not statically readable: it exports no config Rerail can find (export default or module.exports). Rerail never runs your config. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off.");
    expect(review.text()).toContain(`${TYPE_DEFINITIONS} starts off  eslint.config.ts is not statically readable`);
    expect(review.text()).toContain(`${TSLINT_COMMENT} starts off  eslint.config.ts is not statically readable; .oxlintrc.json: "typescript/ban-tslint-comment": "off"`);

    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
    expect(selected(setup)).toEqual(["typescript"]);
    expect(rulesOf(setup, [TSLINT_COMMENT])).toEqual({ [TSLINT_COMMENT]: { on: false, config: null } });
  });

  // NER-1789: dolanmiu/docx (PR 3716) sets consistent-type-definitions to "type" in its eslint.config.ts, for its
  // **/src/**/*.ts files, after shared configs; set-up used to leave the option for the person to set by hand.
  it("takes options and folder offs from a flat eslint.config.ts, read as data, and writes them on start", async () => {
    const setup = await fixture();
    writeFileSync(join(setup.repo, "eslint.config.ts"), [
      "import type { Linter } from \"eslint\";",
      "import tsEslint from \"typescript-eslint\";",
      "const config: Linter.Config[] = [",
      "  ...tsEslint.configs.stylistic,",
      "  { files: [\"**/src/**/*.ts\"], rules: { \"@typescript-eslint/consistent-type-definitions\": [\"error\", \"type\"] } },",
      "  { files: [\"scripts/**\"], rules: { \"@typescript-eslint/ban-tslint-comment\": \"off\" } },",
      "];",
      "export default config;",
    ].join("\n"));
    const environment = fakeSetupEnvironment();
    const asked = agentIo();
    await runSetup(asked, request(setup, environment, { answer: "yes", repo: setup.repo }));
    expect(asked.text()).toContain(`${TYPE_DEFINITIONS} starts with option = "type"  eslint.config.ts: "@typescript-eslint/consistent-type-definitions": ["error","type"] for ["**/src/**/*.ts"]`);
    expect(asked.text()).toContain(`${TSLINT_COMMENT} starts off in scripts/  eslint.config.ts: "@typescript-eslint/ban-tslint-comment": "off" for ["scripts/**"]`);

    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
    expect(rulesOf(setup, [TYPE_DEFINITIONS])).toEqual({ [TYPE_DEFINITIONS]: { on: false, config: { option: "type" } } });
    const database = openRerailDatabase(setup.databasePath, { readonly: true });
    try {
      expect(database.select().from(ruleSettings).all().map(({ folder, targetId, on, config }) => ({ folder, targetId, on, config }))).toEqual([
        { folder: "", targetId: TYPE_DEFINITIONS, on: null, config: { option: "type" } },
        { folder: "scripts", targetId: TSLINT_COMMENT, on: false, config: null },
      ]);
    } finally {
      database.$client.close();
    }
  });

  it("takes nothing from the lint config when the person picks packs themselves, even for a pack they pick", async () => {
    const setup = await fixture();
    writeFileSync(join(setup.repo, ".eslintrc.json"), JSON.stringify({ rules: { "@typescript-eslint/ban-tslint-comment": "off" } }));
    await runSetup(agentIo(), request(setup, fakeSetupEnvironment(), { answer: "pick-myself", repo: setup.repo, packs: TYPESCRIPT }));
    expect(selected(setup)).toEqual(["typescript"]);
    const database = openRerailDatabase(setup.databasePath, { readonly: true });
    try {
      expect(database.$client.prepare("SELECT COUNT(*) AS count FROM rule_settings").get()).toEqual({ count: 0 });
    } finally {
      database.$client.close();
    }
  });

  it("takes nothing from the lint config when the person picks packs themselves after an earlier review added packs", async () => {
    const setup = await fixture();
    writeFileSync(join(setup.repo, ".eslintrc.json"), JSON.stringify({ rules: { "@typescript-eslint/ban-tslint-comment": "off" } }));
    const environment = fakeSetupEnvironment();
    installPack("typescript");
    await attachPacks(setup.databasePath, offerReview(setup), ["typescript"]);
    await runSetup(agentIo(), request(setup, environment, { answer: "pick-myself", repo: setup.repo, packs: TYPESCRIPT }));
    expect(selected(setup)).toEqual(["typescript"]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
    const database = openRerailDatabase(setup.databasePath, { readonly: true });
    try {
      expect(database.$client.prepare("SELECT COUNT(*) AS count FROM rule_settings").get()).toEqual({ count: 0 });
    } finally {
      database.$client.close();
    }
  });
});
describe("set-up in a terminal", { timeout: SETUP_TIMEOUT_MS }, () => {
  it("says yes, starts with the recommended packs, and shows the summary straight away", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    const io = terminalIo(["", "1"]);
    expect(await runSetup(io, request(setup, environment))).toMatchObject({ status: "started" });
    expect(io.left).toEqual([]);
    expect(selected(setup)).toEqual(TS_PACKS);
    expect(io.text()).not.toContain("Enter to see the summary.");
    expect(io.text()).toContain("Try it");
    expect(io.text()).toContain("Good to know");
    expect(io.text()).not.toContain("optional");
  });

  it("numbers the main flow's questions, counting the sessions question only when it is asked", async () => {
    const setup = await fixture();
    const approved = terminalIo(["", "1"]);
    await runSetup(approved, request(setup, fakeSetupEnvironment()));
    expect(approved.text()).toContain("Step 1 of 2 · Find rule packs for acme-api?");
    expect(approved.text()).toContain("Step 2 of 2 · Start Rerail in acme-api with this rule pack?");

    const fresh = await fixture();
    const asked = terminalIo(["", "", "1"]);
    await runSetup(asked, request(fresh, fakeSetupEnvironment({ sessionsApproved: false })));
    expect(asked.text()).toContain("Step 1 of 3 · Can Rerail see your coding agent sessions?");
    expect(asked.text()).toContain("Step 2 of 3 · Find rule packs for acme-api?");
    expect(asked.text()).toContain("Step 3 of 3 · Start Rerail in acme-api with this rule pack?");

    // A folder named with --repo is the go-ahead to read it: no read-repo question to count.
    const named = await fixture();
    const other = gitRepository(join(named.root, "landing-page"));
    const repo = terminalIo(["", "1"]);
    await runSetup(repo, request(named, fakeSetupEnvironment({ sessionsApproved: false }), { repo: other }));
    expect(repo.text()).toContain("Step 1 of 2 · Can Rerail see your coding agent sessions?");
    expect(repo.text()).toContain("Step 2 of 2 · Start Rerail in landing-page with this rule pack?");
    expect(repo.text()).not.toContain("Find rule packs for");
  });

  it("installs, connects and turns on nothing when the person answers not now", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    const io = terminalIo(["", "3"]);
    expect(await runSetup(io, request(setup, environment))).toEqual({ status: "not-now" });
    expect(io.left).toEqual([]);
    expect(io.text()).toContain("Set-up stopped. Nothing was installed or turned on.");
    expect(io.text()).toContain("rerail start");
    expect(environment.connections).toEqual([]);
    expect(installedPackVersion("typescript", environment.packs.home)).toBeNull();
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
    expectUntouched(setup);
  });

  it("reads a different folder instead, leaving this one untouched", async () => {
    const setup = await fixture();
    const other = gitRepository(join(setup.root, "landing-page"));
    const io = terminalIo(["2", join(setup.root, "missing"), other, "1"]);
    await runSetup(io, request(setup, fakeSetupEnvironment()));
    expect(io.text()).toContain(`There is no folder at ${join(setup.root, "missing")}.`);
    expect(workspaceStartState(other, setup.databasePath)).toBe("started");
    expect(selected(setup, other)).toEqual(TS_PACKS);
    expectUntouched(setup);
  });

  it("starts with every pack off when the person picks packs themselves", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    // "I'll pick rule packs myself", then "None for now" from every pack set-up fetched.
    const io = terminalIo(["3", "2"]);
    expect(await runSetup(io, request(setup, environment))).toMatchObject({ status: "started", ruleCount: 0 });
    expect(io.text()).toContain("Rerail did not read any code.");
    expect(selected(setup)).toEqual([]);
    expect(enabledRules(setup)).toBe(0);
    expect(environment.connections).toEqual([{ agents: ["claude-code", "cursor"], repoRoot: setup.repo }]);
  });

  it("takes a typed folder where there is no repo", async () => {
    const setup = await fixture();
    const plain = join(setup.root, "downloads");
    mkdirSync(plain);
    const io = terminalIo(["1", setup.repo, "1"]);
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

  it("shows the summary when input ends as soon as Rerail is on", async () => {
    const setup = await fixture();
    const io = terminalIo(["", "1"]);
    expect(await runSetup(io, request(setup, fakeSetupEnvironment()))).toMatchObject({ status: "started" });
    expect(io.text()).toContain("Manage    http://127.0.0.1:4737/");
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
    await runSetup(agentIo(), request(setup, fakeSetupEnvironment(), { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
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
    expect(started.text()).toContain("Find rule packs for acme-api?");
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
  });

  it("just starts a repository that is set up and was never started", async () => {
    const setup = await fixture();
    await runSetup(agentIo(), request(setup, fakeSetupEnvironment(), { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
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

describe("set-up and rule packs from npm", { timeout: SETUP_TIMEOUT_MS }, () => {
  /** This repository's packs `only` as npm would give them, counting each fetch. */
  function countingStore(only: readonly string[], failing: readonly string[] = []) {
    const store = directoryPackStore(undefined, only);
    const fetches: string[][] = [];
    return {
      ...store,
      fetches,
      async fetch(ids: readonly string[], tag: string) {
        fetches.push([...ids]);
        const result = await store.fetch(ids.filter(id => !failing.includes(id)), tag);
        return { ...result, problems: [...result.problems, ...failing.map(id => ({ packageName: `@neralabs/rerail-pack-${id}`, reason: "npm error code E401" }))] };
      },
    };
  }

  it("fetches packs only after the person allows reading, installs nothing until an installing answer, then exactly the versions it carries", async () => {
    const setup = await fixture();
    const packs = countingStore(["typescript"]);
    const environment = fakeSetupEnvironment({ packs });

    await runSetup(agentIo(), request(setup, environment));
    expect(packs.fetches).toEqual([]);

    const asked = agentIo();
    await runSetup(asked, request(setup, environment, { answer: "yes", repo: setup.repo }));
    expect(packs.fetches).toHaveLength(1);
    expect(asked.text()).toContain(`--packs typescript@${TYPESCRIPT[0]!.version}`);
    expect(packs.installs).toEqual([]);
    expect(loadRuleCatalog().packs.size).toBe(0);
    expectUntouched(setup);

    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
    expect(packs.installs).toEqual([[`@neralabs/rerail-pack-typescript@${TYPESCRIPT[0]!.version}`]]);
    expect([...loadRuleCatalog().packs.keys()]).toEqual(["typescript"]);
    expect(selected(setup)).toEqual(TS_PACKS);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
  });

  it("asks the confirm question again, saying why and changing nothing, when npm has no such version of a pack its answer names", async () => {
    const setup = await fixture();
    const asked = agentIo();
    expect(await runSetup(asked, request(setup, fakeSetupEnvironment(), { answer: "start", repo: setup.repo, packs: [{ id: "typescript", version: "9.9.9" }] })))
      .toEqual({ status: "waiting" });
    expect(asked.text()).toContain("No matching version found for @neralabs/rerail-pack-typescript@9.9.9");
    expect(asked.text()).toContain("Start Rerail in acme-api with this rule pack?");
    expect(loadRuleCatalog().packs.size).toBe(0);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
  });

  /** This repository's typescript pack installed at `version`, as `rerail packs add typescript@<version>` would; `range`: its core range. */
  function installTypescriptAt(version: string, range?: string): void {
    const copy = join(process.env.RERAIL_HOME!, "copies", version);
    cpSync(repositoryPath("packs", "typescript"), copy, { recursive: true, filter: source => !source.includes("upstream") });
    const manifest = JSON.parse(readFileSync(join(copy, "package.json"), "utf8")) as Record<string, unknown>;
    writeFileSync(join(copy, "package.json"), JSON.stringify({ ...manifest, version, ...(range === undefined ? {} : { peerDependencies: { "@neralabs/rerail": range } }) }));
    installPackDirectory(copy, "typescript");
  }

  it("refuses an answer that would replace an installed pack with an older version, and asks again", async () => {
    const setup = await fixture();
    installTypescriptAt("0.9.0");
    const packs = countingStore(["typescript"]);
    const asked = agentIo();
    expect(await runSetup(asked, request(setup, fakeSetupEnvironment({ packs }), { answer: "start", repo: setup.repo, packs: TYPESCRIPT })))
      .toEqual({ status: "waiting" });
    expect(asked.text()).toContain(`typescript@${TYPESCRIPT[0]!.version} is older than the installed 0.9.0`);
    expect(packs.installs).toEqual([]);
    expect(loadRuleCatalog().packs.get("typescript")?.version).toBe("0.9.0");
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
  });

  it("asks again, saying why, when the installed copy of the version an answer names is left out", async () => {
    const setup = await fixture();
    installTypescriptAt(TYPESCRIPT[0]!.version, ">=9.0.0");
    const packs = countingStore(["typescript"]);
    const asked = agentIo();
    expect(await runSetup(asked, request(setup, fakeSetupEnvironment({ packs }), { answer: "start", repo: setup.repo, packs: TYPESCRIPT })))
      .toEqual({ status: "waiting" });
    expect(asked.text()).toContain(`@neralabs/rerail-pack-typescript@${TYPESCRIPT[0]!.version} cannot be used:`);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
  });

  it("installs nothing when the confirm question is asked, or asked again", async () => {
    const setup = await fixture();
    const packs = countingStore(["typescript"]);
    const environment = fakeSetupEnvironment({ packs });
    expect(await runSetup(agentIo(), request(setup, environment, { answer: "yes", repo: setup.repo }))).toEqual({ status: "waiting" });
    expect(await runSetup(agentIo(), request(setup, environment, { answer: "yes", repo: setup.repo }))).toEqual({ status: "waiting" });
    expect(packs.installs).toEqual([]);
    expect(loadRuleCatalog().packs.size).toBe(0);
  });

  it("asks, offering only None for now, when the person picks packs themselves and npm gives none", async () => {
    const setup = await fixture();
    const asked = agentIo();
    expect(await runSetup(asked, request(setup, fakeSetupEnvironment({ packs: countingStore([], ["typescript"]) }), { answer: "pick-myself", repo: setup.repo })))
      .toEqual({ status: "waiting" });
    expect(asked.text()).toContain("Set-up could not get any of Rerail’s rule packs.");
    expect(asked.text()).toContain("@neralabs/rerail-pack-typescript: npm error code E401");
    expect(asked.text()).toContain(`--answer pick-myself --repo ${setup.repo} --packs none`);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");
  });

  it("offers a pack installed at a newer version than npm's at that version, and never replaces it with an older one", async () => {
    const setup = await fixture();
    installTypescriptAt("0.9.0");
    const asked = agentIo();
    await runSetup(asked, request(setup, fakeSetupEnvironment(), { answer: "yes", repo: setup.repo }));
    expect(asked.text()).toContain("--packs typescript@0.9.0");
    expect(asked.text()).not.toContain("replaces");
  });

  it("says a pack a yes installs replaces an older installed version every repository uses", async () => {
    const setup = await fixture();
    installTypescriptAt("0.0.9");
    const asked = agentIo();
    await runSetup(asked, request(setup, fakeSetupEnvironment(), { answer: "yes", repo: setup.repo }));
    expect(asked.text()).toContain(`△ @neralabs/rerail-pack-typescript@${TYPESCRIPT[0]!.version} replaces 0.0.9, which every repo on this computer uses.`);
  });

  it("installs only the packs the person chooses at a terminal, by number or id, after one it does not know", async () => {
    const setup = await fixture();
    const packs = countingStore(["typescript", "complexity"]);
    const io = terminalIo(["1", "2", "eslint", "2"]);
    await runSetup(io, request(setup, fakeSetupEnvironment({ packs })));
    expect(io.text()).toContain("eslint is not one of the rule packs listed.");
    // Both fit the TypeScript repository; the person chose the second listed.
    expect(packs.installs).toHaveLength(1);
    expect(selected(setup)).toHaveLength(1);
    expect(loadRuleCatalog().packs.size).toBe(1);
  });

  it("asks the confirm question again, with the error, for an answer's rule packs it cannot read", async () => {
    const setup = await fixture();
    const asked = agentIo();
    expect(await runSetup(asked, request(setup, fakeSetupEnvironment(), { answer: "start", repo: setup.repo, packsError: "Name each rule pack as <id>@<version>, for example typescript@0.1.1, not typescript" })))
      .toEqual({ status: "waiting" });
    expect(asked.text()).toContain("Name each rule pack as <id>@<version>, for example typescript@0.1.1, not typescript");
    expect(asked.text()).toContain("Start Rerail in acme-api with this rule pack?");
    expect(loadRuleCatalog().packs.size).toBe(0);
  });

  it("names a pack npm would not give, and offers the others", async () => {
    const setup = await fixture();
    const asked = agentIo();
    await runSetup(asked, request(setup, fakeSetupEnvironment({ packs: countingStore(["typescript"], ["complexity"]) }), { answer: "yes", repo: setup.repo }));
    expect(asked.text()).toContain("Rule packs set-up could not use:\n    @neralabs/rerail-pack-complexity: npm error code E401");
    expect(asked.text()).toContain("Start Rerail in acme-api with this rule pack?");
  });

  it("offers no Choose rule packs… when npm gives no pack, so the person is never asked for one they cannot name", async () => {
    const setup = await fixture();
    // Yes, read it; `choose` is no answer here and is asked again; then 2 is "Not now", not Change.
    const io = terminalIo(["1", "choose", "2"]);
    const environment = fakeSetupEnvironment({ packs: countingStore([], ["typescript"]) });
    expect(await runSetup(io, request(setup, environment))).toEqual({ status: "not-now" });
    expect(io.text()).toContain("Start Rerail in acme-api with no rule packs on?");
    expect(io.text()).not.toContain("Change rule packs…");
    expect(environment.connections).toEqual([]);
  });

  it("offers an installed pack npm cannot give now, at its installed version", async () => {
    const setup = await fixture();
    installPack("typescript");
    const asked = agentIo();
    await runSetup(asked, request(setup, fakeSetupEnvironment({ packs: countingStore([], ["typescript"]) }), { answer: "yes", repo: setup.repo }));
    expect(asked.text()).toContain(`--packs typescript@${TYPESCRIPT[0]!.version}`);
  });

  it("offers every fetched pack, reading nothing in the repository, when the person picks packs themselves, and installs none until they pick", async () => {
    const setup = await fixture();
    const packs = countingStore(["typescript", "complexity"]);
    const asked = agentIo();
    expect(await runSetup(asked, request(setup, fakeSetupEnvironment({ packs }), { answer: "pick-myself", repo: setup.repo }))).toEqual({ status: "waiting" });
    const [complexity] = repositoryPackVersions("complexity");
    expect(asked.text()).toContain("Which rule packs should Rerail start with in acme-api?");
    expect(asked.text()).toContain(`complexity@${complexity!.version}  Cyclomatic complexity (limit 6)`);
    expect(asked.text()).toContain(`--answer pick-myself --repo ${setup.repo} --packs none`);
    expect(recommendRulePacks).not.toHaveBeenCalled();
    expect(packs.installs).toEqual([]);
    expectUntouched(setup);
  });

  it("installs the packs a person picked themselves and adds each for the whole repository", async () => {
    const setup = await fixture();
    const packs = countingStore(["typescript", "complexity"]);
    const complexity = repositoryPackVersions("complexity");
    expect(await runSetup(agentIo(), request(setup, fakeSetupEnvironment({ packs }), { answer: "pick-myself", repo: setup.repo, packs: complexity })))
      .toMatchObject({ status: "started" });
    expect(packs.installs).toEqual([[`@neralabs/rerail-pack-complexity@${complexity[0]!.version}`]]);
    // Not `complexity@src`, where its match would put it: the person did not let Rerail read the repository.
    expect(attachedPacks(setup.repo, setup.databasePath)).toEqual(["complexity"]);
    expect(recommendRulePacks).not.toHaveBeenCalled();
  });

  it("checks each pack's core range against the active Rerail, not the package set-up runs from", async () => {
    const setup = await fixture();
    // `rerail update` has made 0.2.0 active; this run is the older package a coding agent's session loaded.
    const current = join(process.env.RERAIL_HOME!, "current");
    mkdirSync(current, { recursive: true });
    writeFileSync(join(current, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "0.2.0" }));
    try {
      const asked = agentIo();
      await runSetup(asked, request(setup, fakeSetupEnvironment(), { answer: "yes", repo: setup.repo }));
      expect(asked.text()).toMatch(/@neralabs\/rerail-pack-typescript@\S+: .*0\.2\.0/u);
      expect(asked.text()).toContain("Scanned acme-api. None of Rerail’s rule packs fit it.");
    } finally {
      await rm(current, { recursive: true, force: true });
    }
  });

  it("names a newer installed pack its core range leaves out, rather than offering npm's older one the downgrade guard would refuse", async () => {
    const setup = await fixture();
    // Installed from `next`, then left out: after `rerail update rollback`, say.
    installTypescriptAt("0.9.0", ">=9.0.0");
    const packs = countingStore(["typescript"]);
    const environment = fakeSetupEnvironment({ packs });

    const asked = agentIo();
    await runSetup(asked, request(setup, environment, { answer: "yes", repo: setup.repo }));
    expect(asked.text()).toMatch(/@neralabs\/rerail-pack-typescript@0\.9\.0: .*>=9\.0\.0/u);
    expect(asked.text()).not.toContain("--packs typescript@");

    // The answer the question prints starts Rerail, installing nothing and keeping 0.9.0.
    expect(await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: setup.repo }))).toMatchObject({ status: "started" });
    expect(packs.installs).toEqual([]);
    const manifest = join(process.env.RERAIL_HOME!, "packs", "node_modules", "@neralabs", "rerail-pack-typescript", "package.json");
    expect((JSON.parse(readFileSync(manifest, "utf8")) as { version: string }).version).toBe("0.9.0");
  });

  it("asks again, saying why, for an answer's version semver cannot read, with that pack installed", async () => {
    const setup = await fixture();
    installTypescriptAt("0.9.0");
    const asked = agentIo();
    expect(await runSetup(asked, request(setup, fakeSetupEnvironment(), { answer: "start", repo: setup.repo, ...setupPacks("typescript@01.2.3", "start") })))
      .toEqual({ status: "waiting" });
    expect(asked.text()).toContain("Name each rule pack as <id>@<version>, for example typescript@0.1.1, not typescript@01.2.3");
  });

  it("installs into its pack store's home and checks the packs there, whatever RERAIL_HOME says", async () => {
    const setup = await fixture();
    const storeHome = join(setup.root, "store-home");
    const packs = directoryPackStore(undefined, ["typescript"], storeHome);

    expect(await runSetup(agentIo(), request(setup, fakeSetupEnvironment({ packs }), { answer: "start", repo: setup.repo, packs: TYPESCRIPT })))
      .toMatchObject({ status: "started" });
    expect(loadRuleCatalog(storeHome).packs.get("typescript")?.version).toBe(TYPESCRIPT[0]!.version);
    expect(loadRuleCatalog().packs.size).toBe(0);
    expect(selected(setup)).toEqual(TS_PACKS);
  });
});
