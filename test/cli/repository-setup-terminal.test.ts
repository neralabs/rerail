/** Set-up in a terminal: the person answers each question there. */
import { existsSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

// Set-up fetches rule packs through the fake environment's store: this repository's TypeScript pack.
// The pack matcher's scan, passed through and watched: a run that must read no code never scans the repository.
const readsRepository = vi.hoisted(() => vi.fn<(root: string) => void>());
vi.mock("../../src/engine/catalog/rule-pack-matcher.js", async (importOriginal) => {
  const actual = await importOriginal<{ scanRepository: (root: string) => unknown }>();
  return { ...actual, scanRepository: (root: string) => (readsRepository(root), actual.scanRepository(root)) };
});
import { runSetup, startRerail } from "../../src/cli/repository-setup.js";
import { stopRerail, workspaceStartState } from "../../src/runtime/index.js";
import { installedPackVersion } from "../../src/setup/index.js";
import { removeInstalledPacks } from "../support/installed-packs.js";
import { fakeSetupEnvironment } from "../support/fake-setup-environment.js";
import { removeFixtures, TS_PACKS, fixture, gitRepository, stripAnsi, type Recorded, agentIo, terminalIo, request, selected, enabledRules, expectUntouched, SETUP_TIMEOUT_MS, TYPESCRIPT, offerReview } from "../support/repository-setup.js";
import { stubUserHome } from "../support/user-home.js";

afterEach(async () => {
  readsRepository.mockClear();
  removeInstalledPacks();
  vi.unstubAllEnvs();
  await removeFixtures();
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
    // A person at a terminal gets the dashboard in their browser.
    expect(environment.pages).toMatchObject([{ repoRoot: setup.repo, page: "rules", open: true }]);
  });

  it("shows the link and says why when no browser can open", async () => {
    const setup = await fixture();
    const io = terminalIo(["", "1"]);
    expect(await runSetup(io, request(setup, fakeSetupEnvironment({ browser: false })))).toMatchObject({ status: "started" });
    expect(stripAnsi(io.text())).toContain("http://127.0.0.1:4737/#rules");
    expect(stripAnsi(io.text())).toContain("Couldn’t open your browser. Copy the link instead.");
  });

  it("says which numbers it takes when a typed answer names none, then asks again", async () => {
    const setup = await fixture();
    const io = terminalIo(["9", "maybe", "", "1"]);
    expect(await runSetup(io, request(setup, fakeSetupEnvironment()))).toMatchObject({ status: "started" });
    expect(io.said.map(stripAnsi).filter(text => text === "Type a number from 1 to 3.")).toHaveLength(2);
  });

  it("asks through the terminal's menu where it has one, drawing the answers there and not in the question", async () => {
    const setup = await fixture();
    const frames: string[] = [];
    const picks = [0, 0];
    const io: Recorded = {
      ...terminalIo([]),
      choose: async menu => {
        frames.push(stripAnsi(menu.frame(1)));
        return picks.shift();
      },
    };
    expect(await runSetup(io, request(setup, fakeSetupEnvironment()))).toMatchObject({ status: "started" });
    expect(selected(setup)).toEqual(TS_PACKS);
    expect(frames).toHaveLength(2);
    expect(frames[0]).toMatch(/^  1\. Yes, scan acme-api.*\n❯ 2\. Scan a different repo…/u);
    expect(frames[0]).toMatch(/\n\n↑↓ to move, Enter to choose, or type a number: $/u);
    expect(stripAnsi(io.text())).toContain("Step 1 of 2 · Find rule packs for acme-api?");
    expect(stripAnsi(io.text())).not.toContain("1. Yes, scan acme-api");
  });

  it("stops, as when input ends, when the menu is cancelled", async () => {
    const setup = await fixture();
    const io: Recorded = { ...terminalIo([]), choose: async () => undefined };
    expect(await runSetup(io, request(setup, fakeSetupEnvironment()))).toMatchObject({ status: "cancelled" });
    expectUntouched(setup);
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
    expect(io.text()).toContain("Set-up stopped. No rule packs were installed and nothing was turned on.");
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
    expect(stripAnsi(io.text())).toContain("Rerail is on in acme-api · no rule packs on yet");
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

  it("numbers the no-repo question as the scan step, and a folder typed there goes straight to the last step", async () => {
    const setup = await fixture();
    const plain = join(setup.root, "downloads");
    mkdirSync(plain);
    // A folder that is not there is asked for again, without changing the count.
    const approved = terminalIo(["1", join(setup.root, "missing"), setup.repo, "1"]);
    await runSetup(approved, request(setup, fakeSetupEnvironment(), { cwd: plain }));
    const steps = (io: Recorded): string[] => stripAnsi(io.text()).match(/Step \d of \d · [^\n]+/gu) ?? [];
    expect(steps(approved)).toEqual(["Step 1 of 2 · Which project do you want to start with?", "Step 2 of 2 · Start Rerail in acme-api with this rule pack?"]);
    expect(approved.text()).toContain("There is no folder at");

    const fresh = await fixture();
    const elsewhere = join(fresh.root, "downloads");
    mkdirSync(elsewhere);
    const asked = terminalIo(["", "1", fresh.repo, "1"]);
    await runSetup(asked, request(fresh, fakeSetupEnvironment({ sessionsApproved: false }), { cwd: elsewhere }));
    expect(steps(asked)).toEqual([
      "Step 1 of 3 · Can Rerail see your coding agent sessions?",
      "Step 2 of 3 · Which project do you want to start with?",
      "Step 3 of 3 · Start Rerail in acme-api with this rule pack?",
    ]);

    // A `--repo` folder that is not there asks which project, counted the same way.
    const third = await fixture();
    const named = terminalIo(["1", third.repo, "1"]);
    await runSetup(named, request(third, fakeSetupEnvironment(), { cwd: third.root, repo: join(third.root, "missing") }));
    expect(steps(named)).toEqual(["Step 1 of 2 · Which project do you want to start with?", "Step 2 of 2 · Start Rerail in acme-api with this rule pack?"]);
  });

  it("counts only the questions it will ask: no scan step where an earlier set-up offered the review", async () => {
    const setup = await fixture();
    offerReview(setup);
    const io = terminalIo(["", "1"]);
    expect(await runSetup(io, request(setup, fakeSetupEnvironment({ sessionsApproved: false })))).toMatchObject({ status: "started" });
    expect(stripAnsi(io.text()).match(/Step \d of \d · [^\n]+/gu)).toEqual([
      "Step 1 of 2 · Can Rerail see your coding agent sessions?",
      "Step 2 of 2 · Start Rerail in acme-api with this rule pack?",
    ]);
  });

  it("numbers the no-repo question as the scan step when --repo names the home folder", async () => {
    const setup = await fixture();
    const home = join(setup.root, "user");
    mkdirSync(home);
    stubUserHome(home);
    const io = terminalIo(["1", setup.repo, "1"]);
    expect(await runSetup(io, request(setup, fakeSetupEnvironment(), { cwd: setup.root, repo: home }))).toMatchObject({ status: "started" });
    expect(io.text()).toContain("Start Rerail inside a repository, not your home folder.");
    expect(stripAnsi(io.text()).match(/Step \d of \d · [^\n]+/gu)).toEqual([
      "Step 1 of 2 · Which project do you want to start with?",
      "Step 2 of 2 · Start Rerail in acme-api with this rule pack?",
    ]);
  });

  it("refuses the home folder typed at the folder prompt, from either question that offers one", async () => {
    const setup = await fixture();
    const home = join(setup.root, "user");
    mkdirSync(home);
    stubUserHome(home);
    const plain = join(setup.root, "downloads");
    mkdirSync(plain);
    const environment = fakeSetupEnvironment();

    // Where there is no repo ("Type the folder…"), and instead of the repo set-up ran in ("Scan a different repo…").
    const noRepo = terminalIo(["1", "~"]);
    expect(await runSetup(noRepo, request(setup, environment, { cwd: plain }))).toEqual({ status: "cancelled" });
    const otherFolder = terminalIo(["2", home]);
    expect(await runSetup(otherFolder, request(setup, environment))).toEqual({ status: "cancelled" });
    for (const io of [noRepo, otherFolder]) {
      expect(io.text()).toContain("Start Rerail inside a repository, not your home folder.");
      expect(io.text()).toContain("Which project do you want to start with?");
    }
    expect(readsRepository).not.toHaveBeenCalled();
    expect(existsSync(setup.databasePath)).toBe(false);
  });

  it("skips for now where there is no repo: connected, on nowhere", async () => {
    const setup = await fixture();
    const plain = join(setup.root, "downloads");
    mkdirSync(plain);
    const environment = fakeSetupEnvironment();
    const io = terminalIo(["2"]);
    expect(await runSetup(io, request(setup, environment, { cwd: plain }))).toEqual({ status: "skipped" });
    expect(io.text()).toContain("Open a session in one of your repos and type /rerail-start.");
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
