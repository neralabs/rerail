import { execFileSync } from "node:child_process";
import { mkdirSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdir, realpath, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import type { RuleCatalog } from "../../../src/engine/catalog/repository-model.js";
import { recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { loadCheckoutRuleSet, setRuleSetting } from "../../../src/engine/catalog/rule-configuration.js";
import { WORKTREE_PACK_ID, worktreeRuleCatalog } from "../../../src/engine/catalog/worktree-pack.js";
import { approve, currentSubject } from "../../../src/engine/catalog/rule-approval.js";
import { runRuleApproveCommand } from "../../../src/cli/index.js";
import type { AuditInput, AuditRecorder, AuditResult } from "../../../src/engine/audit/audit.js";
import { CheckoutCodeRuleRouter } from "../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CheckoutCatalogs, startingCheckout } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { setUpTestRepository } from "../../support/repository-rules.js";
import {
  addRepository,
  checkout,
  REPOSITORY,
  RUNNING,
  temporaryDatabase,
  temporaryDirectory,
  testBundle,
} from "../../support/rule-configuration-builders.js";

const cleanups: Array<() => Promise<void> | void> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/** A code rule forbidding `NEEDLE` in Python files, reporting `code`, as a person writes one into `.rerail/rules`. */
function needle(id: string, code: string, mode = "steer", message = `No ${code}`): unknown {
  return {
    format: "rerail.policy/v1", id, revision: 1, mode, description: `Keep ${code} out of Python files.`,
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{ id: "needle", violation: { code, anchor: "witness.span", message }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
  };
}

/** Writes `files` (name → JSON value, or raw text) into `<root>/.rerail/rules`; returns the folder. */
function writeRules(root: string, files: Record<string, unknown>): string {
  const directory = join(root, ".rerail", "rules");
  mkdirSync(directory, { recursive: true });
  for (const [name, value] of Object.entries(files)) writeFileSync(join(directory, name), typeof value === "string" ? value : JSON.stringify(value));
  return directory;
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

describe("a worktree's own rules", () => {
  it("load as the worktree pack beside the installed packs, each document checked on its own", async () => {
    const root = await temporaryDirectory();
    const installed = testBundle();
    const directory = writeRules(root, {
      "a.json": needle("worktree-needle", "NEEDLE"),
      "b.json": needle("worktree-needle", "AGAIN"),
      "c.json": "{ not json",
      "d.json": { format: "rerail.policy/v1", id: "no-clauses", revision: 1, mode: "steer", clauses: [] },
      "e.json": needle("no-console", "CLASH"),
    });
    symlinkSync(join(directory, "a.json"), join(directory, "f.json"));
    writeFileSync(join(directory, "notes.md"), "not a rule");

    const catalog = worktreeRuleCatalog(installed, root);

    expect(catalog.packs.get(WORKTREE_PACK_ID)).toMatchObject({
      publisher: "worktree", packageName: null,
      rules: [{ id: "worktree-needle", kind: "code", title: "Keep NEEDLE out of Python files.", mode: "steer", on: true }],
    });
    expect(catalog.packOf.get("worktree-needle")).toBe(WORKTREE_PACK_ID);
    // The installed packs are untouched.
    expect(catalog.packs.get("lint")).toBe(installed.packs.get("lint"));
    expect(catalog.documents.get("no-console")).toBe(installed.documents.get("no-console"));
    expect(catalog.digest).not.toBe(installed.digest);
    expect(catalog.skipped.map(({ path, reason }) => ({ file: path.slice(directory.length + 1), reason }))).toEqual([
      { file: "b.json", reason: "rule worktree-needle is already in a.json" },
      { file: "c.json", reason: expect.any(String) as string },
      { file: "d.json", reason: expect.stringMatching(/^clauses /u) as string },
      { file: "e.json", reason: "rule no-console is already in the installed pack lint" },
      { file: "f.json", reason: "it is not a regular file" },
    ]);
  });

  it("are read again when a file changes, and leave the installed catalog itself when there are none", async () => {
    const root = await temporaryDirectory();
    const installed = testBundle();
    expect(worktreeRuleCatalog(installed, root)).toBe(installed);

    writeRules(root, { "a.json": needle("worktree-needle", "NEEDLE") });
    const first = worktreeRuleCatalog(installed, root);
    expect(worktreeRuleCatalog(installed, root)).toBe(first);

    writeRules(root, { "a.json": needle("worktree-needle", "NEEDLE", "log"), "b.json": needle("second", "SECOND") });
    const second = worktreeRuleCatalog(installed, root);
    expect(second.digest).not.toBe(first.digest);
    expect(second.packs.get(WORKTREE_PACK_ID)?.rules.map(rule => [rule.id, rule.mode])).toEqual([["worktree-needle", "log"], ["second", "steer"]]);
  });

  it("are present in their checkout for the whole checkout without an attachment, follow its settings, and nowhere else", async () => {
    const db = await temporaryDatabase();
    addRepository(db, REPOSITORY, 1);
    const own = await temporaryDirectory();
    const other = await temporaryDirectory();
    writeRules(own, { "a.json": needle("worktree-needle", "NEEDLE") });
    const installed: RuleCatalog = testBundle();

    const rulesIn = (root: string, path: string) => loadCheckoutRuleSet(db, worktreeRuleCatalog(installed, root), checkout(root), RUNNING, [])
      .rulesFor(path).map(rule => ({ id: rule.document.rule.id, on: rule.on, mode: rule.mode, source: rule.source, approved: rule.approval?.approved }));
    const source = { kind: "pack", id: WORKTREE_PACK_ID, packId: WORKTREE_PACK_ID };

    // Its content waits for a person's approval, and runs in Log until then.
    expect(rulesIn(own, "deep/x.py")).toEqual([{ id: "worktree-needle", on: true, mode: "log", source, approved: false }]);
    expect(rulesIn(other, "deep/x.py")).toEqual([]);
    approve(db, currentSubject(db, worktreeRuleCatalog(installed, own), "worktree-needle")!);
    expect(rulesIn(own, "deep/x.py")).toEqual([{ id: "worktree-needle", on: true, mode: "steer", source, approved: true }]);

    setRuleSetting(db, { owner: { kind: "checkout", checkoutPath: own }, folder: "", targetKind: "pack", targetId: WORKTREE_PACK_ID, on: false, mode: null });
    expect(rulesIn(own, "deep/x.py")).toMatchObject([{ id: "worktree-needle", on: false }]);
  });

  it("steer edits in the linked worktree that holds them, and not in the repository's main checkout", async () => {
    const root = await realpath(await temporaryDirectory());
    const databasePath = join(root, "rerail.sqlite");
    const main = join(root, "main");
    await mkdir(join(main, "api"), { recursive: true });
    await writeFile(join(main, "api", "seed.py"), "seed\n");
    git(main, "init", "--quiet");
    git(main, "add", ".");
    git(main, "commit", "--quiet", "-m", "seed");
    git(main, "worktree", "add", "--quiet", "-b", "feature", join(root, "feature"));
    const feature = await realpath(join(root, "feature"));
    // Set up with nothing added: only the worktree's own rules can run.
    setUpTestRepository(main, {}, databasePath);
    const database = openRerailDatabase(databasePath);
    recordCheckout(database, feature);
    database.$client.close();
    writeRules(feature, { "needle.json": needle("worktree-needle", "NEEDLE") });

    const audits: Array<AuditInput & { result?: AuditResult }> = [];
    const recorder: AuditRecorder = {
      start(input) { audits.push({ ...input, paths: [...input.paths] }); return String(audits.length - 1); },
      finish(auditId, result) { audits[Number(auditId)]!.result = result; },
      progress(auditId, result) { audits[Number(auditId)]!.result = { status: "completed", ...result }; },
      interrupt() {},
    };
    const routerAt = (sessionRoot: string): CheckoutCodeRuleRouter => {
      const owners = new CheckoutOwners();
      const catalogs = new CheckoutCatalogs(databasePath);
      const router = new CheckoutCodeRuleRouter({
        sessionRoot, startingCheckout: startingCheckout(owners, sessionRoot).checkoutRoot, owners, catalogs,
        steerDelivery: { requestSteer: async () => undefined }, audits: recorder,
      });
      cleanups.push(async () => { await router.close(); catalogs.close(); });
      return router;
    };
    const edit = async (router: CheckoutCodeRuleRouter, sessionId: string, revision: number): Promise<void> => {
      await router.auditGeneratedEdits(sessionId, { revision, checkpoints: ["edit"], edits: [{ path: "api/x.py", language: "python", generatedText: "NEEDLE\n" }] });
      await router.settleGeneratedEdits();
    };
    const found = () => audits.flatMap(audit => (audit.result?.violations ?? []).map(violation => ({ checkout: audit.workspaceRoot, code: violation.code, mode: violation.mode })));

    const approveRule = (): string => {
      let output = "";
      runRuleApproveCommand(["worktree-needle", "--repo", feature], () => "usage", { stdout: { write: text => (output += text) } }, databasePath);
      return output;
    };

    const inFeature = routerAt(feature);
    await inFeature.openWorkspace("feature");
    // Until a person approves the file's content it is recorded, and steers no one.
    await edit(inFeature, "feature", 1);
    expect(found()).toEqual([{ checkout: feature, code: "NEEDLE", mode: "log" }]);
    expect(approveRule()).toBe("● Approved rule worktree-needle: it runs in the mode it is set to from the next edit.\n");
    audits.length = 0;
    await edit(inFeature, "feature", 2);
    const inMain = routerAt(await realpath(main));
    await inMain.openWorkspace("main");
    await edit(inMain, "main", 1);
    expect(found()).toEqual([{ checkout: feature, code: "NEEDLE", mode: "steer" }]);

    // An edit to the rule applies at the next edit, with nothing else touched; it is new content, and waits again.
    writeRules(feature, { "needle.json": needle("worktree-needle", "NEEDLE", "steer", "Remove NEEDLE; it is deprecated") });
    audits.length = 0;
    await edit(inFeature, "feature", 3);
    expect(found()).toEqual([{ checkout: feature, code: "NEEDLE", mode: "log" }]);
  });
});
