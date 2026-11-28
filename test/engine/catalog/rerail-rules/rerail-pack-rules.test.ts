/**
 * A pack's rules written in `.rerail` (docs/pack-format.md, "Rules written in .rerail"), as the catalog reads them from
 * the fixture pack `fixtures/packs/rr-sample`: what the engine reads from each file, the digests that follow its text
 * and its includes, the readings kept between processes, and the checkout catalog's placed `.rerail` rules.
 */
import { createHash } from "node:crypto";
import { appendFileSync, cpSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as RrRunner from "../../../../src/engine/rr/rr-runner.js";
import { keptRerailDescriber } from "../../../../src/engine/catalog/rerail-rules/kept-rerail-descriptions.js";
import { packCompileChecks, PACK_COMPILE_CHECKS_FILE } from "../../../../src/engine/catalog/pack-compile-checks.js";
import type { RerailDescription } from "../../../../src/engine/catalog/rerail-rules/rerail-rule.js";
import { packApprovalDigest } from "../../../../src/engine/catalog/rule-approval.js";
import { catalogDigest, loadRuleCatalog, readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { setRuleSetting } from "../../../../src/engine/catalog/rule-configuration.js";
import { readRulePack } from "../../../../src/engine/catalog/rule-packs.js";
import { CheckoutCatalogs } from "../../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { openRerailDatabase } from "../../../../src/engine/storage/database.js";
import { ruleDocumentDigest } from "../../../../src/engine/stable-json.js";
import type { RrDescription } from "../../../../src/engine/rr/rr-runner.js";
import { installPackDirectory } from "../../../support/installed-packs.js";
import { fixturesPath } from "../../../support/oracles.js";
import { removeFolderSync } from "../../../support/platform/remove-folder.js";
import { setUpTestRepository } from "../../../support/repository-rules.js";

const SAMPLE = fixturesPath("packs", "rr-sample");
const LIBRARY = join("rules", "lib", "sample-calls.rerail");

/** Runs once the engine has described rules and before the catalog's read ends: a test's edit landing in between. */
let afterDescribe: (() => void) | undefined;
vi.mock("../../../../src/engine/rr/rr-runner.js", async importOriginal => {
  const actual = await importOriginal<typeof RrRunner>();
  return {
    ...actual,
    describeRerailPoliciesSync: (paths: string[]) => {
      const rows = actual.describeRerailPoliciesSync(paths);
      afterDescribe?.();
      return rows;
    },
  };
});

const temporary: string[] = [];
afterEach(() => {
  for (const directory of temporary.splice(0)) removeFolderSync(directory);
});

/** A copy of the fixture pack, in a directory named for it, with `change` applied. */
function samplePack(change: (directory: string) => void = () => undefined): string {
  const parent = realpathSync(mkdtempSync(join(tmpdir(), "rerail-rr-pack-")));
  temporary.push(parent);
  const directory = join(parent, "rr-sample");
  cpSync(SAMPLE, directory, { recursive: true });
  change(directory);
  return directory;
}

/**
 * Reading a `.rerail` rule starts the runner's child, which reads core's library with no kept store; on a loaded
 * machine at two threads that takes seconds, past vitest's default.
 */
const CHILD_TIMEOUT = 120_000;

describe("a pack's rules written in .rerail", { timeout: CHILD_TIMEOUT }, () => {
  it("takes each rule's facts from its file, as the engine reads it", () => {
    const directory = samplePack();
    const catalog = readRuleCatalog([directory]);
    expect(catalog.skipped).toEqual([]);
    expect(catalog.packs.get("rr-sample")?.rules.map(({ id, kind, mode, on }) => ({ id, kind, mode, on }))).toEqual([
      { id: "rr-sample-assert", kind: "code", mode: "steer", on: true },
      { id: "rr-sample-print-call", kind: "code", mode: "steer", on: true },
      { id: "rr-sample-deprecated-call", kind: "code", mode: "steer", on: true },
    ]);
    const printCall = catalog.documents.get("rr-sample-print-call");
    expect(printCall).toMatchObject({
      kind: "rerail",
      rule: {
        id: "rr-sample-print-call",
        revision: 1,
        mode: "steer",
        description: "Use logging instead of print calls, so output goes where the application's logs go.",
        language: "python",
        scope: { include: ["**/*.py"], exclude: [] },
        codes: ["P001"],
        source: readFileSync(join(directory, "rules", "rr-sample-print-call.rerail"), "utf8"),
      },
    });
    expect(printCall?.kind === "rerail" && printCall.rule.includes.map(file => file.path)).toContain(join(directory, LIBRARY));
    expect(catalog.sources.has("rr-sample-print-call")).toBe(false);
  });

  it("moves the rule's, the pack's and the approval's digests with a file the rule includes, and only that rule's", () => {
    const directory = samplePack();
    const before = readRuleCatalog([directory]);
    appendFileSync(join(directory, LIBRARY), "// an edit\n");
    const after = readRuleCatalog([directory]);
    const digest = (catalog: typeof before, id: string): string => ruleDocumentDigest(catalog.documents.get(id)!);
    expect(digest(after, "rr-sample-print-call")).not.toBe(digest(before, "rr-sample-print-call"));
    expect(digest(after, "rr-sample-assert")).toBe(digest(before, "rr-sample-assert"));
    expect(after.digest).not.toBe(before.digest);
    expect(packApprovalDigest(after, after.packs.get("rr-sample")!)).not.toBe(packApprovalDigest(before, before.packs.get("rr-sample")!));
  });

  it("keeps the rule's digest wherever the pack is installed", () => {
    const here = readRulePack(samplePack());
    const there = readRulePack(samplePack());
    expect(catalogDigest([there])).toBe(catalogDigest([here]));
  });

  it("reads an installed pack again when a file its .rerail rule includes changes, so the edit waits for approval", () => {
    // A folder install runs in Log until approved; the runner reads the files from disk, so the catalog must see an edit.
    const installed = installPackDirectory(samplePack(), "rr-sample");
    const before = loadRuleCatalog();
    appendFileSync(join(installed, LIBRARY), "// an edit\n");
    const after = loadRuleCatalog();
    expect(after).not.toBe(before);
    expect(ruleDocumentDigest(after.documents.get("rr-sample-print-call")!)).not.toBe(ruleDocumentDigest(before.documents.get("rr-sample-print-call")!));
    expect(packApprovalDigest(after, after.packs.get("rr-sample")!)).not.toBe(packApprovalDigest(before, before.packs.get("rr-sample")!));
    expect(loadRuleCatalog()).toBe(after);
  });

  it("reads an installed pack again when one of its .rerail files changes while the catalog reads it", () => {
    const installed = installPackDirectory(samplePack(), "rr-sample");
    const library = join(installed, LIBRARY);
    afterDescribe = () => {
      afterDescribe = undefined;
      appendFileSync(library, "// an edit while the catalog reads\n");
    };
    const first = loadRuleCatalog();
    const second = loadRuleCatalog();
    expect(second).not.toBe(first);
    const edited = { path: realpathSync(library), sha256: createHash("sha256").update(readFileSync(library)).digest("hex") };
    expect(second.documents.get("rr-sample-print-call")).toMatchObject({ kind: "rerail", rule: { includes: expect.arrayContaining([edited]) } });
    expect(loadRuleCatalog()).toBe(second);
  });

  it("leaves out a pack whose .rerail rule the engine refuses, in the engine's words", () => {
    const directory = samplePack(pack => {
      const path = join(pack, "rules", "rr-sample-assert.rerail");
      writeFileSync(path, readFileSync(path, "utf8").replace('kind(a, "assert_statement")', "no_such_relation(a)"));
    });
    const { packs, skipped } = readRuleCatalog([directory]);
    expect(packs.size).toBe(0);
    expect(skipped).toEqual([expect.objectContaining({
      path: directory,
      reason: expect.stringMatching(/^Rule pack rr-sample lists rr-sample-assert, whose document rules\/rr-sample-assert\.rerail the engine refuses: \S/u),
    })]);
  });

  it("leaves out a pack whose .rerail rule is on block mode, which is not supported yet", () => {
    const directory = samplePack(pack => {
      const path = join(pack, "rules", "rr-sample-assert.rerail");
      writeFileSync(path, readFileSync(path, "utf8").replace(".mode steer", ".mode block"));
    });
    expect(readRuleCatalog([directory]).skipped.map(entry => entry.reason)).toEqual([
      "Rule pack rr-sample lists rr-sample-assert: block mode is not supported yet for a .rerail rule; use steer or log",
    ]);
  });

  it("tells a core that cannot read the pack that it needs a newer Rerail", () => {
    // A core older than RERAIL_DOCUMENTS_SINCE refuses `rules/<id>.rerail`, as this one refuses a field it does not know.
    const directory = samplePack(pack => {
      const packJson = join(pack, "pack.json");
      writeFileSync(packJson, JSON.stringify({ ...JSON.parse(readFileSync(packJson, "utf8")), later: true }));
      const packageJson = join(pack, "package.json");
      const manifest = JSON.parse(readFileSync(packageJson, "utf8")) as { peerDependencies: Record<string, string> };
      writeFileSync(packageJson, JSON.stringify({ ...manifest, peerDependencies: { "@neralabs/rerail": ">=9.0.0-0" } }));
    });
    expect(readRuleCatalog([directory], { coreVersion: "0.1.5" }).skipped.map(entry => entry.reason)).toEqual([
      "it needs a newer Rerail (>=9.0.0-0) than this one (0.1.5); `rerail update` installs the newest",
    ]);
  });
});

describe("the engine's readings kept between processes", () => {
  const sha256 = (path: string): string => createHash("sha256").update(readFileSync(path)).digest("hex");
  /** A reading of `path` as the engine gives one, with the rule file and the library file it includes. */
  const reading = (directory: string, path: string): RerailDescription => ({
    path, ok: true, id: "rr-sample-print-call", revision: 1, description: null, mode: "steer", language: "python",
    scope: { include: ["**/*.py"], exclude: [], text: [], toolCalls: false }, codes: ["P001"], tests: 1, suppresses: false, fixes: false,
    files: [path, join(directory, LIBRARY)].map(file => ({ path: file, sha256: sha256(file) })),
  });

  it("asks the engine only for a rule whose file, or a file it includes, changed", () => {
    const directory = samplePack();
    const rule = join(directory, "rules", "rr-sample-print-call.rerail");
    const asked: string[][] = [];
    const describe = (paths: readonly string[]): RrDescription[] => {
      asked.push([...paths]);
      return paths.map(path => reading(directory, path));
    };
    const kept = new Map<string, RerailDescription>();
    const describer = keptRerailDescriber(kept, (path, description) => kept.set(path, description), describe);
    describer([rule]);
    describer([rule]);
    appendFileSync(join(directory, LIBRARY), "// an edit\n");
    describer([rule]);
    expect(asked).toEqual([[rule], [rule]]);
  });

  it("writes each reading beside the compile verdicts, for the next process", { timeout: CHILD_TIMEOUT }, () => {
    const home = realpathSync(mkdtempSync(join(tmpdir(), "rerail-rr-home-")));
    temporary.push(home);
    const directory = samplePack();
    const checks = packCompileChecks(home);
    readRuleCatalog([directory], { compile: true, compileChecks: checks });
    checks.save();
    const kept = JSON.parse(readFileSync(join(home, PACK_COMPILE_CHECKS_FILE), "utf8")) as { descriptions?: Record<string, { id: string }> }[];
    expect(Object.values(kept[0]?.descriptions ?? {}).map(description => description.id).sort()).toEqual(["rr-sample-assert", "rr-sample-deprecated-call", "rr-sample-print-call"]);
  });
});

describe("the checkout catalog's .rerail rules", { timeout: CHILD_TIMEOUT }, () => {
  it("places each active .rerail rule per folder for the .rerail runner, and refuses options for one", () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-rr-checkout-")));
    temporary.push(root);
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    const databasePath = join(root, ".rerail", "rerail.sqlite");
    // The runner reads each rule where the pack is installed, in the Rerail home.
    const installed = installPackDirectory(samplePack(), "rr-sample", undefined, "0.1.0");
    const key = setUpTestRepository(root, { packs: ["rr-sample"] }, databasePath);
    const catalogs = new CheckoutCatalogs(databasePath);
    const database = openRerailDatabase(databasePath);
    try {
      const catalog = catalogs.catalog(root);
      expect(catalog.snapshot().codeRules).toEqual([]);
      expect(catalog.snapshot().rerailRules.map(({ id, path, scope, language, placement }) => ({ id, path, scope, language, placement }))).toEqual([
        { id: "rr-sample-assert", path: join(installed, "rules", "rr-sample-assert.rerail"), scope: { include: ["**/*.py"], exclude: [], text: [] }, language: "python", placement: [{ folder: "", mode: "steer" }] },
        { id: "rr-sample-deprecated-call", path: join(installed, "rules", "rr-sample-deprecated-call.rerail"), scope: { include: ["**/*.py"], exclude: [], text: [] }, language: "python", placement: [{ folder: "", mode: "steer" }] },
        { id: "rr-sample-print-call", path: join(installed, "rules", "rr-sample-print-call.rerail"), scope: { include: ["**/*.py"], exclude: [], text: [] }, language: "python", placement: [{ folder: "", mode: "steer" }] },
      ]);
      expect(catalog.status.ruleCount).toBe(3);
      const printCall = catalog.snapshot().rerailRules.find(entry => entry.id === "rr-sample-print-call")!;
      const printCallFile = join(installed, "rules", "rr-sample-print-call.rerail");
      expect(printCall.files[0]).toEqual({ path: printCall.path, sha256: createHash("sha256").update(readFileSync(printCallFile)).digest("hex") });
      expect(printCall.files.map(file => file.path)).toContain(realpathSync(join(installed, LIBRARY)));

      setRuleSetting(database, { owner: { kind: "repository", repositoryKey: key }, folder: "legacy", targetKind: "rule", targetId: "rr-sample-assert", on: null, mode: "log" });
      setRuleSetting(database, { owner: { kind: "repository", repositoryKey: key }, folder: "", targetKind: "rule", targetId: "rr-sample-print-call", on: null, mode: null, config: { ceiling: 3 } });
      expect(catalog.refresh()).toBe(true);
      expect(catalog.snapshot().rerailRules.map(({ id, placement }) => ({ id, placement }))).toEqual([
        { id: "rr-sample-assert", placement: [{ folder: "", mode: "steer" }, { folder: "legacy", mode: "log" }] },
        { id: "rr-sample-deprecated-call", placement: [{ folder: "", mode: "steer" }] },
      ]);
      expect(catalog.quarantined()).toEqual([expect.objectContaining({
        ruleId: "rr-sample-print-call",
        reason: "The repository's config for this rule does not apply: config: rr-sample-print-call is written in .rerail, and options for a rule written in .rerail are not supported yet",
      })]);
    } finally {
      catalogs.close();
      database.$client.close();
    }
  });
});
