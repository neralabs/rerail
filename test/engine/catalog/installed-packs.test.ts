import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  addAttachment,
  loadCheckoutRuleSet,
  ruleConfigurationSignature,
  setRuleSetting,
} from "../../../src/engine/catalog/rule-configuration.js";
import type { RuleCatalog } from "../../../src/engine/catalog/repository-model.js";
import { exportRulesDump, importRulesDump } from "../../../src/engine/catalog/rule-bootstrap.js";
import { activeCoreVersion, installedPackDirectories, loadRuleCatalog, readRuleCatalog } from "../../../src/engine/catalog/rule-catalog.js";
import { copyRerailRuleToLibrary, getLibraryRule } from "../../../src/engine/catalog/rule-library.js";
import type { RuleMode } from "../../../src/engine/catalog/rule-mode.js";
import type { RerailDatabase } from "../../../src/engine/storage/database.js";
import { ruleSettings } from "../../../src/engine/storage/schema.js";
import { installPackDirectory, removeInstalledPacks, thirdPartyPack, uninstallPack } from "../../support/installed-packs.js";
import { fixturesPath, repositoryPath } from "../../support/oracles.js";
import {
  addRepository,
  checkout,
  codeRule,
  REPOSITORY,
  RUNNING,
  temporaryDatabase,
  temporaryDirectory,
} from "../../support/rule-configuration-builders.js";

// The exemplar is a language only tests plan: the registry module lists it for this file, and the engine refuses it as a
// planned language (test/engine/support/exemplar-registry.ts, exemplar-engine.ts). Every other rule compiles in the
// real engine.
vi.mock("../../../src/engine/code-rule/code-rule-language-registry.generated.js", async () =>
  (await import("../support/exemplar-registry.js")).exemplarRegistryModule());
vi.mock("../../../src/engine/native/native.js", async importOriginal =>
  (await import("../support/exemplar-engine.js")).nativeRefusingExemplar(await importOriginal()));

type Json = Record<string, unknown>;

const readJson = (path: string): Json => JSON.parse(readFileSync(path, "utf8")) as Json;
const writeJson = (path: string, value: unknown): void => writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);

/** A copy of `source` (pack.json, package.json and its rules) in a temporary directory, for a case to edit. */
async function copyPack(source: string): Promise<string> {
  const directory = join(await temporaryDirectory(), "pack");
  for (const entry of ["pack.json", "package.json", "rules"]) cpSync(join(source, entry), join(directory, entry), { recursive: true });
  return directory;
}

/** Rewrites the JSON file at `directory/file` with `edit`. */
function editJson(directory: string, file: string, edit: (value: Json) => Json): void {
  const path = join(directory, file);
  writeJson(path, edit(readJson(path)));
}

const VALID = fixturesPath("packs", "valid");
const COMPLEXITY = repositoryPath("packs", "complexity");
/** This repository's complexity pack's version, which each pack release bumps. */
const COMPLEXITY_VERSION = (JSON.parse(readFileSync(join(COMPLEXITY, "package.json"), "utf8")) as { version: string }).version;
const PYTHON_RULE = "complexity-cyclomatic-python";

/** The ids of what `catalog` loaded, and each skipped pack's directory and reason. */
function summary(catalog: RuleCatalog) {
  return {
    packs: [...catalog.packs.keys()].sort(),
    skipped: catalog.skipped.map(({ path, reason }) => ({ path, reason })),
  };
}

beforeEach(() => {
  removeInstalledPacks();
});

describe("the installed-pack catalog", () => {
  it("is empty in a fresh home: no packs, no rules, nothing skipped", () => {
    const catalog = loadRuleCatalog();

    expect(catalog.packs.size).toBe(0);
    expect(catalog.documents.size).toBe(0);
    expect(catalog.sources.size).toBe(0);
    expect(catalog.skipped).toEqual([]);
  });

  it("holds an installed pack with its version, reading only the documents pack.json lists", async () => {
    const directory = await copyPack(VALID);
    // A document in rules/ that pack.json does not list is not part of the pack.
    writeJson(join(directory, "rules", "unlisted.json"), codeRule("unlisted"));
    installPackDirectory(directory, "valid");

    const catalog = loadRuleCatalog();

    expect(catalog.packs.get("valid")).toMatchObject({ id: "valid", version: "0.1.0" });
    expect([...catalog.documents.keys()].sort()).toEqual(["assert-used", "pass-statement"]);
    expect([...catalog.sources.keys()].sort()).toEqual(["assert-used", "pass-statement"]);
    expect(catalog.sources.get("pass-statement")).toEqual(readJson(join(directory, "rules", "pass-statement.json")));
    expect(Object.fromEntries(catalog.packOf)).toEqual({ "assert-used": "valid", "pass-statement": "valid" });
    expect(catalog.skipped).toEqual([]);
  });

  describe("skips a broken pack with its reason and loads the others", () => {
    /** Installs `valid` and a copy of `complexity` broken by `breakPack`; returns the copy's installed path. */
    async function installBroken(breakPack: (directory: string) => void): Promise<string> {
      installPackDirectory(VALID, "valid");
      const directory = await copyPack(COMPLEXITY);
      breakPack(directory);
      return installPackDirectory(directory, "complexity");
    }

    const cases: Array<{ name: string; breakPack: (directory: string) => void; reason: RegExp }> = [
      {
        name: "a core range that excludes the running core",
        breakPack: directory => editJson(directory, "package.json", manifest => ({ ...manifest, peerDependencies: { "@neralabs/rerail": ">=9.0.0" } })),
        reason: new RegExp(`^it needs a newer Rerail \\(>=9\\.0\\.0\\) than this one \\(${activeCoreVersion().replace(/\./gu, "\\.")}\\); \`rerail update\` installs the newest$`, "u"),
      },
      {
        name: "a pack.json the schema refuses",
        breakPack: directory => editJson(directory, "pack.json", pack => ({ ...pack, category: "toolkit" })),
        reason: /pack\.json: category /u,
      },
      {
        name: "a pack.json without match",
        breakPack: directory => editJson(directory, "pack.json", ({ match: _match, ...pack }) => pack),
        reason: /pack\.json: match /u,
      },
      {
        name: "a package.json named for another pack",
        breakPack: directory => editJson(directory, "package.json", manifest => ({ ...manifest, name: "@neralabs/rerail-pack-other" })),
        reason: /^Rule pack complexity must be the package rerail-pack-complexity or @<scope>\/rerail-pack-complexity, not @neralabs\/rerail-pack-other$/u,
      },
      {
        name: "a listed document that does not exist",
        breakPack: directory => rmSync(join(directory, "rules", `${PYTHON_RULE}.json`)),
        reason: new RegExp(`^Rule pack complexity lists ${PYTHON_RULE}, whose document rules/${PYTHON_RULE}\\.json does not exist$`, "u"),
      },
      {
        name: "a document whose id differs from the listing",
        breakPack: directory => editJson(directory, `rules/${PYTHON_RULE}.json`, rule => ({ ...rule, id: "something-else" })),
        reason: new RegExp(`^Rule pack complexity lists ${PYTHON_RULE}, but rules/${PYTHON_RULE}\\.json has id something-else$`, "u"),
      },
      {
        name: "a code rule the engine rejects",
        breakPack: directory => writeJson(join(directory, "rules", `${PYTHON_RULE}.json`), {
          ...codeRule(PYTHON_RULE),
          clauses: [{
            id: "broken",
            violation: { anchor: "witness.span", code: "BROKEN", message: "never" },
            forbid: { regex: { pattern: "(", source: "file.content" } },
          }],
        }),
        reason: new RegExp(`^this Rerail \\(${activeCoreVersion().replace(/\./gu, "\\.")}\\) cannot run its rule ${PYTHON_RULE}, so the pack may need a newer Rerail \\(\`rerail update\` installs the newest\\)\\. The engine says: `, "u"),
      },
    ];

    for (const { name, breakPack, reason } of cases) {
      it(`for ${name}`, async () => {
        const installed = await installBroken(breakPack);

        const catalog = loadRuleCatalog();

        expect(summary(catalog)).toEqual({ packs: ["valid"], skipped: [{ path: installed, reason: expect.stringMatching(reason) }] });
        expect(catalog.skipped[0]).toMatchObject({ packageName: expect.stringMatching(/^@neralabs\/rerail-pack-/u), version: COMPLEXITY_VERSION });
        expect([...catalog.documents.keys()].sort()).toEqual(["assert-used", "pass-statement"]);
      });
    }

    it("for a rule id another installed pack already holds", async () => {
      installPackDirectory(VALID, "valid");
      const directory = await copyPack(VALID);
      editJson(directory, "pack.json", pack => ({ ...pack, id: "valid-copy" }));
      editJson(directory, "package.json", manifest => ({ ...manifest, name: "@neralabs/rerail-pack-valid-copy" }));
      const installed = installPackDirectory(directory, "valid-copy");

      const catalog = loadRuleCatalog();

      expect(summary(catalog)).toEqual({ packs: ["valid"], skipped: [{ path: installed, reason: "rule assert-used is already in @neralabs/rerail-pack-valid" }] });
      expect(catalog.skipped[0]).toMatchObject({ packageName: "@neralabs/rerail-pack-valid-copy", version: "0.1.0" });
    });

    it("for a package.json named for no pack at all", async () => {
      installPackDirectory(VALID, "valid");
      const directory = await copyPack(COMPLEXITY);
      editJson(directory, "package.json", manifest => ({ ...manifest, name: "@acme/complexity" }));
      const installed = installPackDirectory(directory, "@acme/rerail-pack-complexity");

      expect(summary(loadRuleCatalog()).skipped).toEqual([
        { path: installed, reason: "Rule pack complexity: package.json name must be rerail-pack-<id> or @<scope>/rerail-pack-<id>" },
      ]);
    });
  });

  describe("third parties' packs", () => {
    it("loads a scoped and an unscoped one, each with its package name and its publisher", async () => {
      const copies = await temporaryDirectory();
      installPackDirectory(VALID, "valid");
      installPackDirectory(thirdPartyPack(copies, "@acme/rerail-pack-acme-valid", "acme-valid"), "@acme/rerail-pack-acme-valid");
      installPackDirectory(thirdPartyPack(copies, "rerail-pack-lint", "lint"), "rerail-pack-lint");

      const catalog = loadRuleCatalog();

      expect([...catalog.packs.values()].map(({ id, packageName, publisher }) => ({ id, packageName, publisher })).sort((a, b) => a.id.localeCompare(b.id))).toEqual([
        { id: "acme-valid", packageName: "@acme/rerail-pack-acme-valid", publisher: "@acme" },
        { id: "lint", packageName: "rerail-pack-lint", publisher: "rerail-pack-lint" },
        { id: "valid", packageName: "@neralabs/rerail-pack-valid", publisher: "rerail" },
      ]);
      expect(catalog.packOf.get("lint-assert-used")).toBe("lint");
      expect(catalog.skipped).toEqual([]);
    });

    it("leaves one out whose pack id Rerail's own pack holds, whichever is found first", async () => {
      const copies = await temporaryDirectory();
      // `@aaa` sorts before `@neralabs`, and `rerail-pack-valid` after it: the scan meets one before Rerail's and one after.
      const before = installPackDirectory(thirdPartyPack(copies, "@aaa/rerail-pack-valid", "valid"), "@aaa/rerail-pack-valid");
      const rerail = installPackDirectory(VALID, "valid");
      const after = installPackDirectory(thirdPartyPack(copies, "rerail-pack-valid", "valid"), "rerail-pack-valid");

      const catalog = loadRuleCatalog();

      expect(catalog.packs.get("valid")?.packageName).toBe("@neralabs/rerail-pack-valid");
      expect(summary(catalog).skipped).toEqual([
        { path: before, reason: "pack id valid is taken by @neralabs/rerail-pack-valid" },
        { path: after, reason: "pack id valid is taken by @neralabs/rerail-pack-valid" },
      ]);
      for (const order of [[before, rerail, after], [after, before, rerail]]) {
        expect(readRuleCatalog(order).packs.get("valid")?.packageName).toBe("@neralabs/rerail-pack-valid");
      }
    });

    it("leaves one out that ships a rule id Rerail's own pack holds, whichever is found first", async () => {
      // A third party's copy of valid under another pack id: the same rule ids.
      const directory = await copyPack(VALID);
      editJson(directory, "pack.json", pack => ({ ...pack, id: "copied" }));
      editJson(directory, "package.json", manifest => ({ ...manifest, name: "@aaa/rerail-pack-copied" }));
      const copied = installPackDirectory(directory, "@aaa/rerail-pack-copied");
      const rerail = installPackDirectory(VALID, "valid");

      expect(summary(loadRuleCatalog())).toEqual({ packs: ["valid"], skipped: [{ path: copied, reason: "rule assert-used is already in @neralabs/rerail-pack-valid" }] });
      expect(summary(readRuleCatalog([rerail, copied])).packs).toEqual(["valid"]);
    });

    it("loads the first by package name of two others that hold one pack id, whichever is found first", async () => {
      const copies = await temporaryDirectory();
      const acme = thirdPartyPack(copies, "@acme/rerail-pack-sql", "sql");
      const unscoped = thirdPartyPack(copies, "rerail-pack-sql", "sql");

      for (const order of [[acme, unscoped], [unscoped, acme]]) {
        const catalog = readRuleCatalog(order);
        expect(catalog.packs.get("sql")).toMatchObject({ packageName: "@acme/rerail-pack-sql", publisher: "@acme" });
        expect(catalog.skipped).toEqual([expect.objectContaining({ path: unscoped, packageName: "rerail-pack-sql", reason: "pack id sql is taken by @acme/rerail-pack-sql" })]);
      }
    });
  });

  it("loads a pack whose rule is refused only because its language is not supported yet", () => {
    installPackDirectory(fixturesPath("packs", "exemplar"), "exemplar");

    const catalog = loadRuleCatalog();

    expect(summary(catalog)).toEqual({ packs: ["exemplar"], skipped: [] });
    expect(catalog.documents.has("no-identifier")).toBe(true);
  });

  it("follows installs: the same catalog while nothing changes, a new one when a pack is added or removed", () => {
    installPackDirectory(VALID, "valid");
    const first = loadRuleCatalog();
    expect(loadRuleCatalog()).toBe(first);

    installPackDirectory(fixturesPath("packs", "exemplar"), "exemplar");
    const added = loadRuleCatalog();
    expect(added).not.toBe(first);
    expect([...added.packs.keys()].sort()).toEqual(["exemplar", "valid"]);
    expect(loadRuleCatalog()).toBe(added);

    uninstallPack("exemplar");
    expect([...loadRuleCatalog().packs.keys()]).toEqual(["valid"]);
  });

  it("reads the packs again when the active core changes, checking their range against the new one", () => {
    installPackDirectory(VALID, "valid");
    expect([...loadRuleCatalog().packs.keys()]).toEqual(["valid"]);
    // `rerail update` makes a core active whose version is outside the pack's range; the packs themselves are unchanged.
    const current = join(process.env.RERAIL_HOME!, "current");
    mkdirSync(current, { recursive: true });
    writeFileSync(join(current, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "9.0.0" }));
    try {
      const catalog = loadRuleCatalog();
      expect(catalog.packs.size).toBe(0);
      expect(catalog.skipped.map(skipped => skipped.packageName)).toEqual(["@neralabs/rerail-pack-valid"]);
    } finally {
      rmSync(current, { recursive: true, force: true });
    }
  });

  it("reads the packs it is given without the core range unless asked", async () => {
    const directory = await copyPack(VALID);
    editJson(directory, "package.json", manifest => ({ ...manifest, peerDependencies: { "@neralabs/rerail": ">=9.0.0" } }));

    expect(summary(readRuleCatalog([directory]))).toEqual({ packs: ["valid"], skipped: [] });
    expect(summary(readRuleCatalog([directory], { coreVersion: "0.1.0" })).skipped).toEqual([
      { path: directory, reason: "it needs a newer Rerail (>=9.0.0) than this one (0.1.0); `rerail update` installs the newest" },
    ]);
  });
});

interface TinyRule {
  readonly id: string;
  readonly message: string;
  readonly revision?: number;
  readonly on?: boolean;
  readonly mode?: RuleMode;
}

/** Pack `tiny` at `version` with `rules`, each a regex code rule, written to a new temporary directory. */
async function tinyPack(version: string, rules: readonly TinyRule[]): Promise<string> {
  const directory = join(await temporaryDirectory(), `tiny-${version}`);
  mkdirSync(join(directory, "rules"), { recursive: true });
  writeJson(join(directory, "package.json"), {
    name: "@neralabs/rerail-pack-tiny",
    version,
    license: "Apache-2.0",
    files: ["pack.json", "rules"],
    peerDependencies: { "@neralabs/rerail": ">=0.1.0-0 <0.2.0" },
  });
  writeJson(join(directory, "pack.json"), {
    format: "rerail.rule-pack/v1",
    id: "tiny",
    name: "Tiny",
    category: "language",
    covers: "A few regex rules over TypeScript files.",
    reason: "matches TypeScript",
    match: { any: [{ files: ["**/*.ts"], label: "*.ts files" }] },
    rules: rules.map(rule => ({
      id: rule.id,
      document: `rules/${rule.id}.json`,
      title: rule.id,
      group: "Tiny",
      ...(rule.on === undefined ? {} : { on: rule.on }),
      ...(rule.mode === undefined ? {} : { mode: rule.mode }),
    })),
  });
  for (const rule of rules) writeJson(join(directory, "rules", `${rule.id}.json`), codeRule(rule.id, { revision: rule.revision ?? 1, message: rule.message }));
  return directory;
}

const TINY_V1: readonly TinyRule[] = [
  { id: "tiny-a", message: "A v1" },
  { id: "tiny-b", message: "B v1" },
  { id: "tiny-d", message: "D v1" },
];
// v2 changes A and B, adds C (off, in Log) and removes D.
const TINY_V2: readonly TinyRule[] = [
  { id: "tiny-a", message: "A v2", revision: 2 },
  { id: "tiny-b", message: "B v2", revision: 2 },
  { id: "tiny-c", message: "C v2", on: false, mode: "log" },
];

const CHECKOUT = "/work/app";

/** Each rule present at `src/a.ts`, by id: its message, whether it is on, its mode and where it comes from. */
function effectiveRules(db: RerailDatabase, catalog: RuleCatalog) {
  const rules = loadCheckoutRuleSet(db, catalog, checkout(CHECKOUT), RUNNING, []);
  return {
    rules: Object.fromEntries(rules.rulesFor("src/a.ts").map(rule => [rule.document.rule.id, {
      message: rule.document.kind === "code" ? rule.document.rule.clauses[0]?.violation?.message : undefined,
      on: rule.on,
      mode: rule.mode,
      source: rule.source.kind,
    }])),
    unavailable: rules.unavailable,
  };
}

/** A repository with pack `tiny` attached, A turned off and B copied into the library, under the installed `tiny`. */
async function configuredRepository(): Promise<{ db: RerailDatabase; copyId: string }> {
  const db = await temporaryDatabase();
  addRepository(db, REPOSITORY);
  addAttachment(db, { repositoryKey: REPOSITORY, folder: "", sourceKind: "pack", sourceId: "tiny" });
  setRuleSetting(db, { owner: { kind: "repository", repositoryKey: REPOSITORY }, folder: "", targetKind: "rule", targetId: "tiny-a", on: false, mode: null });
  const catalog = loadRuleCatalog();
  const edited = codeRule("tiny-b", { message: "B, my way" });
  const copy = copyRerailRuleToLibrary(db, catalog, REPOSITORY, "tiny-b", edited);
  return { db, copyId: copy.rule.document.rule.id };
}

describe("a repository's rule configuration across installs", () => {
  it("keeps applying across an upgrade and a downgrade of an attached pack", async () => {
    const v1 = await tinyPack("0.1.0", TINY_V1);
    const v2 = await tinyPack("0.2.0", TINY_V2);
    installPackDirectory(v1, "tiny");
    const { db, copyId } = await configuredRepository();
    const copy = getLibraryRule(db, copyId);
    const atV1 = loadRuleCatalog();
    const signatureV1 = ruleConfigurationSignature(db, atV1, REPOSITORY, []);
    const rulesV1 = effectiveRules(db, atV1);
    expect(rulesV1).toEqual({
      rules: {
        "tiny-a": { message: "A v1", on: false, mode: "block", source: "pack" },
        "tiny-b": { message: "B v1", on: false, mode: "block", source: "pack" },
        "tiny-d": { message: "D v1", on: true, mode: "block", source: "pack" },
        [copyId]: { message: "B, my way", on: true, mode: "block", source: "library_rule" },
      },
      unavailable: [],
    });

    installPackDirectory(v2, "tiny");
    const atV2 = loadRuleCatalog();

    expect(atV2.packs.get("tiny")?.version).toBe("0.2.0");
    expect(effectiveRules(db, atV2)).toEqual({
      rules: {
        "tiny-a": { message: "A v2", on: false, mode: "block", source: "pack" },
        "tiny-b": { message: "B v2", on: false, mode: "block", source: "pack" },
        "tiny-c": { message: "C v2", on: false, mode: "log", source: "pack" },
        [copyId]: { message: "B, my way", on: true, mode: "block", source: "library_rule" },
      },
      unavailable: [],
    });
    expect(getLibraryRule(db, copyId)).toEqual(copy);
    expect(ruleConfigurationSignature(db, atV2, REPOSITORY, [])).not.toBe(signatureV1);

    installPackDirectory(v1, "tiny");
    const backAtV1 = loadRuleCatalog();

    expect(backAtV1.packs.get("tiny")?.version).toBe("0.1.0");
    expect(effectiveRules(db, backAtV1)).toEqual(rulesV1);
    expect(getLibraryRule(db, copyId)).toEqual(copy);
    expect(ruleConfigurationSignature(db, backAtV1, REPOSITORY, [])).toBe(signatureV1);
  });

  it("reports an uninstalled pack's attachment as missing, keeps its settings, and restores them on reinstall", async () => {
    const v1 = await tinyPack("0.1.0", TINY_V1);
    installPackDirectory(v1, "tiny");
    const { db } = await configuredRepository();
    const installed = effectiveRules(db, loadRuleCatalog());
    const settings = db.select().from(ruleSettings).all();

    uninstallPack("tiny");
    const without = effectiveRules(db, loadRuleCatalog());

    expect(installedPackDirectories()).toEqual([]);
    expect(without.unavailable).toEqual([{ sourceKind: "pack", sourceId: "tiny", folder: "", reason: "missing" }]);
    expect(Object.values(without.rules).map(rule => rule.source)).toEqual(["library_rule"]);
    expect(db.select().from(ruleSettings).all()).toEqual(settings);

    installPackDirectory(v1, "tiny");
    expect(effectiveRules(db, loadRuleCatalog())).toEqual(installed);
  });

  it("imports a rules dump on a home without the pack, keeping its attachment missing until the pack is installed", async () => {
    const v1 = await tinyPack("0.1.0", TINY_V1);
    installPackDirectory(v1, "tiny");
    const { db } = await configuredRepository();
    const exported = effectiveRules(db, loadRuleCatalog());
    const dump = exportRulesDump(db);

    uninstallPack("tiny");
    const imported = await temporaryDatabase();
    importRulesDump(imported, loadRuleCatalog(), dump);

    expect(exportRulesDump(imported).repositories).toEqual(dump.repositories);
    expect(effectiveRules(imported, loadRuleCatalog()).unavailable).toEqual([{ sourceKind: "pack", sourceId: "tiny", folder: "", reason: "missing" }]);

    installPackDirectory(v1, "tiny");
    expect(effectiveRules(imported, loadRuleCatalog())).toEqual(exported);
  });
});
