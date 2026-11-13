/** Set-up in the coding agent's lane: it stops at each question, and the agent relays the person's answers. */
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

// Set-up fetches rule packs through the fake environment's store: this repository's TypeScript pack.
// The pack matcher's scan, passed through and watched: a run that must read no code never scans the repository.
const readsRepository = vi.hoisted(() => vi.fn<(root: string) => void>());
vi.mock("../../src/engine/catalog/rule-pack-matcher.js", async (importOriginal) => {
  const actual = await importOriginal<{ scanRepository: (root: string) => unknown }>();
  return { ...actual, scanRepository: (root: string) => (readsRepository(root), actual.scanRepository(root)) };
});
import { openRerailDatabase } from "../../src/engine/index.js";
import { recordCheckout } from "../../src/engine/index.js";
import { parseCodingAgents, runSetup, startRerail } from "../../src/cli/repository-setup.js";
import { workspaceStartState } from "../../src/runtime/index.js";
import { setUpRepository } from "../../src/engine/index.js";
import { installedPackVersion } from "../../src/setup/index.js";
import { directoryPackStore } from "../support/directory-pack-store.js";
import { installPack, removeInstalledPacks } from "../support/installed-packs.js";
import { attachPacks, fakeSetupEnvironment } from "../support/fake-setup-environment.js";
import { removeFixtures, TS_PACKS, fixture, gitRepository, agentIo, request, selected, enabledRules, startRows, expectUntouched, failStarts, SETUP_TIMEOUT_MS, TYPESCRIPT, offerReview } from "../support/repository-setup.js";
import { shellWord } from "../../src/engine/index.js";
import { stubUserHome } from "../support/user-home.js";

afterEach(async () => {
  readsRepository.mockClear();
  removeInstalledPacks();
  vi.unstubAllEnvs();
  await removeFixtures();
});

describe("set-up in the coding agent's lane", { timeout: SETUP_TIMEOUT_MS }, () => {
  it("asks before reading, changes nothing until yes, and repeats no side effect when an answer is run again", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();

    const asked = agentIo();
    expect(await runSetup(asked, request(setup, environment))).toEqual({ status: "waiting" });
    expect(asked.text()).toContain("Find rule packs for acme-api?");
    // The agent runs answers in a POSIX shell, Git Bash on Windows, which needs a Windows path in quotes.
    expect(asked.text()).toContain(`rerail setup --answer yes --repo ${shellWord("posix", setup.repo)}`);
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
    expect(environment.pages).toMatchObject([{ repoRoot: setup.repo, page: "rules", open: false }]);
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

  it("opens no browser when it starts Rerail, and shows the dashboard's link instead", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment();
    const started = agentIo();
    // A coding agent may run set-up where no one watches the screen, or in a sandbox.
    expect(await runSetup(started, request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }))).toMatchObject({ status: "started" });
    expect(environment.pages).toEqual([expect.objectContaining({ page: "rules", open: false })]);
    expect(started.text()).toContain("http://127.0.0.1:4737/#rules");
    expect(started.text()).not.toContain("Opening in your browser.");
  });

  it("goes straight to the confirm question where an earlier set-up offered the review and nothing changed there", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment({ packs: directoryPackStore(undefined, ["typescript", "complexity"]) });
    offerReview(setup);

    // Nothing changed in the dashboard: the confirm question, never the read-repo question, and no review link.
    const plain = agentIo();
    expect(await runSetup(plain, request(setup, environment))).toEqual({ status: "waiting" });
    expect(plain.text()).not.toContain("Find rule packs for");
    expect(plain.text()).toContain("Start Rerail in acme-api with");
    expect(plain.text()).not.toContain("review=1");
    expect(environment.pages).toEqual([]);

    // "Yes" starts it with what that question showed.
    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
    expect(selected(setup)).toEqual(TS_PACKS);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
  });

  it("takes a repository an earlier review set up as set up: starting turns on the packs kept on there, asking nothing", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment({ packs: directoryPackStore(undefined, ["typescript", "complexity"]) });
    const scope = offerReview(setup);
    // The person's first switch in the dashboard set it up with complexity, not the TypeScript pack set-up recommends.
    installPack("complexity");
    await attachPacks(setup.databasePath, scope, ["complexity"]);

    // `rerail setup` offers no packs there: none of the recommended ones is installed or added.
    const again = agentIo();
    expect(await runSetup(again, request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }))).toMatchObject({ status: "stopped" });
    expect(again.text()).not.toContain("Start Rerail in acme-api with");
    expect(installedPackVersion("typescript", environment.packs.home)).toBeNull();
    expect(selected(setup)).toEqual(["complexity"]);
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("stopped");

    // `rerail start` starts it with what the dashboard shows, and asks nothing.
    const started = agentIo();
    expect(await startRerail(setup.repo, started, { databasePath: setup.databasePath, environment })).toMatchObject({ setUpRan: false, status: "started" });
    expect(started.text()).not.toContain("Start Rerail in acme-api with");
    expect(started.text()).toMatch(/^● Rerail is on in acme-api\. .*, as you left them\.$/u);
    expect(selected(setup)).toEqual(["complexity"]);
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

  it("connects on a later plain run only the coding agents found since, never one an earlier run saw", async () => {
    const setup = await fixture();
    const first = fakeSetupEnvironment({ found: ["claude-code", "cursor"] });
    await runSetup(agentIo(), request(setup, first, { answer: "start", repo: setup.repo, packs: TYPESCRIPT, agents: parseCodingAgents("claude-code") }));

    // Codex installed since; Cursor, left out by `--agents`, stays out.
    const later = fakeSetupEnvironment({ found: ["claude-code", "codex", "cursor"], seen: first.agentsSeen() });
    const io = agentIo();
    expect(await runSetup(io, request(setup, later))).toMatchObject({ status: "started" });
    expect(later.connections).toEqual([{ agents: ["codex"], repoRoot: setup.repo }]);
    expect(io.text()).toContain("Now also connected: Codex.");
    expect(io.text()).toContain("Open Codex and run /hooks once to trust Rerail’s hooks.");
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
  });

  it("connects no coding agent on a later plain run with no record of what an earlier one saw, and starts the record", async () => {
    const setup = await fixture();
    const first = fakeSetupEnvironment({ found: ["claude-code", "cursor"] });
    await runSetup(agentIo(), request(setup, first, { answer: "start", repo: setup.repo, packs: TYPESCRIPT, agents: parseCodingAgents("claude-code") }));

    // As after updating from a version that kept no record: Cursor, left out then, is found but must stay out.
    const later = fakeSetupEnvironment({ found: ["claude-code", "cursor"] });
    expect(await runSetup(agentIo(), request(setup, later))).toMatchObject({ status: "started" });
    expect(later.connections).toEqual([]);
    expect(later.agentsSeen()).toEqual(["claude-code", "cursor"]);
  });

  it("tries again on a later plain run an agent it could not connect, even one recorded as seen before", async () => {
    const setup = await fixture();
    const first = fakeSetupEnvironment({ found: ["claude-code", "codex"], seen: ["claude-code", "codex"], unreadable: ["codex"] });
    await runSetup(agentIo(), request(setup, first, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
    expect(first.agentsSeen()).toEqual(["claude-code"]);

    const later = fakeSetupEnvironment({ found: ["claude-code", "codex"], seen: first.agentsSeen() });
    await runSetup(agentIo(), request(setup, later));
    expect(later.connections).toEqual([{ agents: ["codex"], repoRoot: setup.repo }]);
  });

  it("starts Rerail and connects the other coding agents when one's config is not plain JSON, naming it", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment({ unreadable: ["claude-code"] });
    const io = agentIo();
    expect(await runSetup(io, request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }))).toMatchObject({ status: "started" });
    expect(workspaceStartState(setup.repo, setup.databasePath)).toBe("started");
    expect(io.text()).toContain("Couldn’t connect Claude Code: /home/me/.claude-code/settings.json is not plain JSON");
    expect(io.text()).toContain("then run rerail setup again to connect Claude Code.");
    // Not seen, so set-up run again tries it once more.
    expect(environment.agentsSeen()).toEqual(["cursor"]);
  });

  it("takes a named folder as the go-ahead to read it, and leaves the repo it ran in alone", async () => {
    const setup = await fixture();
    const other = gitRepository(join(setup.root, "landing-page"));
    const environment = fakeSetupEnvironment();

    const recommended = agentIo();
    expect(await runSetup(recommended, request(setup, environment, { repo: other }))).toEqual({ status: "waiting" });
    expect(recommended.text()).toContain("Scanned landing-page.");
    expect(recommended.text()).toContain(`rerail setup --answer start --repo ${shellWord("posix", other)}`);
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
    expect(picked.text()).toContain("Rerail is on in acme-api · no rule packs on yet");
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
    expect(notNow.text()).toContain("Set-up stopped. No rule packs were installed and nothing was turned on.");
    expect(notNow.text()).toContain("/rerail-start");
    expect(environment.connections).toEqual([]);
    expect(existsSync(setup.databasePath)).toBe(false);
    expect(installedPackVersion("typescript", environment.packs.home)).toBeNull();
    expectUntouched(setup);
  });

  it("tells a Codex user to start later with $rerail-start", async () => {
    const setup = await fixture();
    const environment = fakeSetupEnvironment({ found: ["codex"] });
    const notNow = agentIo();
    expect(await runSetup(notNow, request(setup, environment, { answer: "not-now", repo: setup.repo }))).toEqual({ status: "not-now" });
    expect(notNow.text()).toContain("whenever you’re ready with $rerail-start.");
    expect(notNow.text()).not.toContain("/rerail-start");
    expect(environment.connections).toEqual([]);
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
    expect(skipped.text()).toContain("Rerail is installed · not on in any repo yet");
    expect(environment.connections).toEqual([{ agents: ["claude-code", "cursor"], repoRoot: undefined }]);
    expect(existsSync(setup.databasePath)).toBe(false);
    expect(existsSync(join(plain, ".rerail"))).toBe(false);
  });

  it("asks which project again for a relayed pick-myself answer where there is no repo, connecting nothing", async () => {
    const setup = await fixture();
    const plain = join(setup.root, "downloads");
    mkdirSync(plain);
    const environment = fakeSetupEnvironment();
    const asked = agentIo();
    expect(await runSetup(asked, request(setup, environment, { cwd: plain, answer: "pick-myself" }))).toEqual({ status: "waiting" });
    expect(asked.text()).toContain("Which project do you want to start with?");
    expect(asked.text()).not.toContain("pick-myself");
    expect(environment.connections).toEqual([]);
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
    expect(readsRepository).not.toHaveBeenCalled();
    expect(environment.connections).toEqual([]);
    expect(existsSync(setup.databasePath)).toBe(false);
    expectUntouched(setup);
  });

  it("refuses the home folder named with --repo, and asks which project instead", async () => {
    const setup = await fixture();
    const home = join(setup.root, "user");
    mkdirSync(home);
    stubUserHome(home);
    const asked = agentIo();
    expect(await runSetup(asked, request(setup, fakeSetupEnvironment(), { answer: "start", repo: "~" }))).toEqual({ status: "waiting" });
    expect(asked.text()).toContain("Start Rerail inside a repository, not your home folder.");
    expect(asked.text()).toContain("Which project do you want to start with?");
    expect(readsRepository).not.toHaveBeenCalled();
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
});
