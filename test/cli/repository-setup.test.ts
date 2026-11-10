/** Set-up and the repository's own lint config, a database from before set-up, `rerail setup` itself, and rule packs from npm. */
import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { rm } from "node:fs/promises";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

// Set-up fetches rule packs through the fake environment's store: this repository's TypeScript pack.
// Passed through, and watched: a run that must read no code never matches packs against the repository.
const readsRepository = vi.hoisted(() => vi.fn<(root: string) => void>());
vi.mock("../../src/engine/catalog/rule-packs.js", async (importOriginal) => {
  type Match = (root: string, ...rest: unknown[]) => unknown;
  const actual = await importOriginal<{ recommendRulePacks: Match; rulePacksThatFit: Match }>();
  const watched = (match: Match): Match => (root, ...rest) => {
    readsRepository(root);
    return match(root, ...rest);
  };
  return { ...actual, recommendRulePacks: watched(actual.recommendRulePacks), rulePacksThatFit: watched(actual.rulePacksThatFit) };
});
import { openRerailDatabase } from "../../src/engine/index.js";
import { listRepositoryRules, loadRuleCatalog, lookupCheckout, recordCheckout, ruleSettings } from "../../src/engine/index.js";
import { runSetup, setupPacks, startRerail, startRerailStep } from "../../src/cli/repository-setup.js";
import { workspaceStartState } from "../../src/runtime/index.js";
import { noBrowserReason, runSetupCommand, type SetupIo } from "../../src/cli/setup-cli.js";
import { ManagedUpdater } from "../../src/setup/index.js";
import { directoryPackStore, repositoryPackVersions } from "../support/directory-pack-store.js";
import { installPack, installPackDirectory, removeInstalledPacks } from "../support/installed-packs.js";
import { repositoryPath } from "../support/oracles.js";
import { fakeSetupEnvironment } from "../support/fake-setup-environment.js";
import { attachedPacks } from "../support/attached-packs.js";
import { removeFixtures, TS_PACKS, type Fixture, fixture, agentIo, terminalIo, request, selected, expectUntouched, SETUP_TIMEOUT_MS, TYPESCRIPT } from "../support/repository-setup.js";

afterEach(async () => {
  readsRepository.mockClear();
  removeInstalledPacks();
  vi.unstubAllEnvs();
  await removeFixtures();
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
/** A rule the TypeScript pack starts on, among the first the confirm screen lists (it shows six, then "…and N more"). */
const TS_COMMENT = "typescript-eslint-ban-ts-comment";

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

  it("starts the ESLint ports off for an eslint.config.ts it cannot read statically, and takes an .oxlintrc.json off", async () => {
    const setup = await fixture();
    writeFileSync(join(setup.repo, "eslint.config.ts"), "throw new Error(\"run\");\n");
    writeFileSync(join(setup.repo, ".oxlintrc.json"), "{ // ours\n  \"rules\": { \"typescript/ban-tslint-comment\": \"off\", },\n}\n");
    const environment = fakeSetupEnvironment();
    const review = agentIo();
    await runSetup(review, request(setup, environment, { answer: "yes", repo: setup.repo }));
    expect(review.text()).toContain("eslint.config.ts is not statically readable: it exports no config Rerail can find (export default or module.exports). Rerail never runs your config. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off.");
    expect(review.text()).toContain(`${TS_COMMENT} starts off  eslint.config.ts is not statically readable`);
    expect(review.text()).toContain(`${TSLINT_COMMENT} starts off  eslint.config.ts is not statically readable; .oxlintrc.json: "typescript/ban-tslint-comment": "off"`);

    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
    expect(selected(setup)).toEqual(["typescript"]);
    expect(rulesOf(setup, [TS_COMMENT, TSLINT_COMMENT])).toEqual({ [TS_COMMENT]: { on: false, config: null }, [TSLINT_COMMENT]: { on: false, config: null } });
  });

  // dolanmiu/docx (PR 3716) sets consistent-type-definitions to "type" in its eslint.config.ts, for its
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
    expect(await runSetup(asked, request(setup, fakeSetupEnvironment(), { answer: "start", repo: setup.repo, packs: [{ packageName: "@neralabs/rerail-pack-typescript", id: "typescript", version: "9.9.9" }] })))
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

  it("names a pack that fits but has no rules yet, and does not offer it", async () => {
    const setup = await fixture();
    // The TypeScript pack as it would be published before any of its rules is ready.
    const packsRoot = join(setup.root, "packs");
    cpSync(repositoryPath("packs", "typescript"), join(packsRoot, "typescript"), { recursive: true, filter: source => !source.includes("upstream") });
    const packJson = join(packsRoot, "typescript", "pack.json");
    writeFileSync(packJson, JSON.stringify({ ...JSON.parse(readFileSync(packJson, "utf8")) as Record<string, unknown>, rules: [] }));
    const asked = agentIo();
    await runSetup(asked, request(setup, fakeSetupEnvironment({ packs: directoryPackStore(packsRoot, ["typescript"]) }), { answer: "yes", repo: setup.repo }));
    expect(asked.text()).toContain("TypeScript would fit, but has no rules yet, so set-up does not offer it.");
  });

  it("explains why no pack is recommended, naming the closest match", async () => {
    const setup = await fixture();
    for (const path of ["tests/test_models.py", "tests/test_views.py", "scripts/release.py"]) {
      mkdirSync(join(setup.repo, dirname(path)), { recursive: true });
      writeFileSync(join(setup.repo, path), "x = 1\n");
    }
    const asked = agentIo();
    await runSetup(asked, request(setup, fakeSetupEnvironment({ packs: countingStore(["python"]) }), { answer: "yes", repo: setup.repo }));
    expect(asked.text()).toContain([
      "Scanned acme-api. No packs recommended.",
      "  Packs are recommended when their file or dependency checks match.",
      "",
      "  Scanned types   .py 3 · .ts 1",
      "  Closest match   Python — 3 source files found, all in folders",
      "                  the Python pack ignores: tests/ (2), scripts/ (1)",
      "",
      "  Available (1)   Python",
    ].join("\n"));
    expect(asked.text()).not.toContain("--packs typescript@");
  });

  it("says npm gave no pack and offers to try again, never that none fit or to start with none", async () => {
    const setup = await fixture();
    // Yes, read it; 1 tries again, which fetches again; then 2 is "Not now".
    const io = terminalIo(["1", "1", "2"]);
    const packs = countingStore([], ["typescript"]);
    const environment = fakeSetupEnvironment({ packs });
    expect(await runSetup(io, request(setup, environment))).toEqual({ status: "not-now" });
    expect(packs.fetches).toHaveLength(2);
    expect(io.text()).toContain("Couldn’t download Rerail’s rule packs, so set-up can’t tell which fit acme-api.");
    expect(io.text()).toContain("npm error code E401");
    expect(io.text()).toContain("Try downloading them again?");
    expect(io.text()).not.toContain("No packs recommended");
    expect(io.text()).not.toContain("Closest match");
    expect(io.text()).not.toContain("with no rule packs on?");
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
    expect(asked.text()).toContain(`complexity@${complexity!.version}  Cyclomatic complexity`);
    expect(asked.text()).not.toContain("(limit 6)");
    expect(asked.text()).toContain(`--answer pick-myself --repo ${setup.repo} --packs none`);
    expect(readsRepository).not.toHaveBeenCalled();
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
    expect(readsRepository).not.toHaveBeenCalled();
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
      expect(asked.text()).toContain("Scanned acme-api. No packs recommended.");
    } finally {
      await rm(current, { recursive: true, force: true });
    }
  });

  it("names a newer installed pack its core range leaves out, rather than offering npm's older one the downgrade guard would refuse", async () => {
    const setup = await fixture();
    // Installed from `next`, then left out: after setting up an older core again, say.
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
    expect(asked.text()).toContain("Name each rule pack as <id>@<version> or <package>@<version>, for example typescript@0.1.1, not typescript@01.2.3");
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
