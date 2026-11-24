import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, afterEach, describe, expect, it, vi } from "vitest";
import { runHeadlessScan, type ScanReport } from "../../src/cli/headless-scan.js";
import { runRuleApproveCommand } from "../../src/cli/rule-approval-cli.js";
import type { PackStore } from "../../src/setup/index.js";
import { withExperiments } from "../support/environment/experiments.js";
import { installPackDirectory, versionOf } from "../support/installed-packs.js";
import { repositoryPath } from "../support/oracles.js";
import { removeFolderSync } from "../support/platform/remove-folder.js";
import { setUpTestRepository } from "../support/repository-rules.js";

withExperiments("HEADLESS_SCAN");
const roots: string[] = [];
const fileHome = process.env.RERAIL_HOME;
afterEach(() => {
  vi.stubEnv("RERAIL_HOME", fileHome);
  for (const root of roots.splice(0)) removeFolderSync(root);
});

function folder(prefix: string): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), prefix)));
  roots.push(root);
  return root;
}

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.email=test@example.com", "-c", "user.name=Test", ...args], { cwd, encoding: "utf8" }).trim();
}

/** A rule that flags `code` in Python files. */
function needle(id: string, code: string): unknown {
  return {
    format: "rerail.policy/v1", id, revision: 1, mode: "steer", description: `Keep ${code} out of Python files.`,
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{ id: "needle", violation: { code, anchor: "witness.span", message: `No ${code}` }, forbid: { regex: { pattern: code, source: "file.content" } } }],
  };
}

/** Writes a pack into `directory`: package `packageName` at `version`, one needle rule per entry of `rules` (id → code). */
function writePack(directory: string, packageName: string, version: string, rules: Record<string, string>): string {
  const id = /rerail-pack-([a-z0-9-]+)$/u.exec(packageName)![1]!;
  mkdirSync(join(directory, "rules"), { recursive: true });
  writeFileSync(join(directory, "package.json"), JSON.stringify({
    name: packageName, version, license: "Apache-2.0", files: ["pack.json", "rules"], peerDependencies: { "@neralabs/rerail": ">=0.1.0-0" },
  }));
  writeFileSync(join(directory, "pack.json"), JSON.stringify({
    format: "rerail.rule-pack/v1", id, name: id, category: "language", covers: `Test pack ${id}.`, reason: "matches Python",
    match: { any: [{ files: ["**/*.py"], label: "*.py files" }] },
    rules: Object.keys(rules).map(rule => ({ id: rule, document: `rules/${rule}.json`, title: rule, group: id })),
  }));
  for (const [rule, code] of Object.entries(rules)) writeFileSync(join(directory, "rules", `${rule}.json`), JSON.stringify(needle(rule, code)));
  return directory;
}

/** A registry of published packs, `<package>@<version>` → folder: installs link the folder, as npm installs a version. */
function registry(versions: Record<string, string>): { store: (home: string) => PackStore; installs: string[][]; offline: boolean[] } {
  const installs: string[][] = [];
  const offline: boolean[] = [];
  const store = (home: string): PackStore => ({
    home,
    fetch: async () => ({ fetched: [], problems: [], release: async () => {} }),
    async install(specs, options) {
      installs.push([...specs]);
      offline.push(options?.preferOffline === true);
      for (const spec of specs) {
        const directory = versions[spec];
        if (directory === undefined) throw new Error(`npm error code E404\nnpm error 404 Not Found - ${spec}`);
        const at = spec.lastIndexOf("@");
        installPackDirectory(directory, spec.slice(0, at), home, spec.slice(at + 1));
      }
    },
    remove: async () => {},
  });
  return { store, installs, offline };
}

const published = realpathSync(mkdtempSync(join(tmpdir(), "rerail-published-packs-")));
afterAll(() => removeFolderSync(published));
const ONE = writePack(join(published, "one"), "@neralabs/rerail-pack-needles", "1.0.0", { "needles-alpha": "ALPHA" });
const TWO = writePack(join(published, "two"), "@neralabs/rerail-pack-needles", "2.0.0", { "needles-beta": "BETA" });
const COMPLEXITY = repositoryPath("packs", "complexity");
const COMPLEXITY_VERSION = versionOf(COMPLEXITY);
const npm = registry({
  "@neralabs/rerail-pack-needles@1.0.0": ONE,
  "@neralabs/rerail-pack-needles@2.0.0": TWO,
  [`@neralabs/rerail-pack-complexity@${COMPLEXITY_VERSION}`]: COMPLEXITY,
});

function writePacksFile(root: string, packs: unknown): void {
  mkdirSync(join(root, ".rerail"), { recursive: true });
  writeFileSync(join(root, ".rerail", "packs.json"), typeof packs === "string" ? packs : JSON.stringify({ schema: "rerail.packs/1", packs }));
}

/**
 * A committed checkout that lists `needles@1.0.0` and its own pack `packs/own` (named as if it were Rerail's), with
 * `a.py` breaking ALPHA, BETA and OWN.
 */
function checkout(): string {
  const root = folder("rerail-declared-packs-");
  git(root, "init", "--quiet", "--initial-branch=main");
  writePack(join(root, "packs", "own"), "@neralabs/rerail-pack-own", "0.1.0", { "own-rule": "OWN" });
  writePacksFile(root, [{ id: "needles", version: "1.0.0" }, { path: "packs/own" }]);
  writeFileSync(join(root, "a.py"), "a = ALPHA\nb = BETA\nc = OWN\n");
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  return root;
}

/** A Rerail home of its own, as a fresh machine or CI runner has: `RERAIL_HOME` for the scan, and its database. */
function freshHome(): { home: string; databasePath: string } {
  const home = folder("rerail-declared-home-");
  return { home, databasePath: join(home, "rerail.sqlite") };
}

async function scanIn(home: { home: string; databasePath: string }, repo: string, options: Partial<Parameters<typeof runHeadlessScan>[0]> = {}): Promise<ScanReport> {
  vi.stubEnv("RERAIL_HOME", home.home);
  return await runHeadlessScan({ repo, scope: "repository", packs: [], version: "0.0.0-test", databasePath: home.databasePath, packStore: npm.store, ...options });
}

/** What decides the rules a scan ran: each rule, with its pack, mode and content, and each finding. */
const ruleShape = (report: ScanReport): unknown => ({
  packs: report.packs,
  rules: report.rules,
  violations: report.violations.map(violation => ({ ruleId: violation.ruleId, mode: violation.mode, fingerprint: violation.fingerprint })),
});

describe("rerail scan with the packs a repository declares in .rerail/packs.json", () => {
  it("runs the same rules for the same commit in two fresh Rerail homes, whatever each home has installed or set up", async () => {
    const root = checkout();
    const empty = freshHome();
    const busy = freshHome();
    // The second machine has another version of the pack installed, and the repository set up with it there.
    installPackDirectory(TWO, "@neralabs/rerail-pack-needles", busy.home, "2.0.0");
    setUpTestRepository(root, { packs: ["needles"] }, busy.databasePath);

    const first = await scanIn(empty, root);
    const second = await scanIn(busy, root);

    expect(ruleShape(second)).toEqual(ruleShape(first));
    // A scan may run with no network, so a version npm has cached installs from there (the store asks the registry when not).
    expect(npm.offline.slice(-2)).toEqual([true, true]);
    expect(first.status).toBe("completed");
    expect(first.rulesSource).toEqual({ kind: "worktree" });
    expect(first.rules.map(rule => rule.ruleId)).toEqual(["needles-alpha", "own-rule"]);
  });

  it("names the pack of every rule, and runs a pack kept in the repository in Log until a person approves it", async () => {
    const root = checkout();
    const home = freshHome();
    const report = await scanIn(home, root);

    expect(report.packs).toEqual([
      { id: "needles", packageName: "@neralabs/rerail-pack-needles", version: "1.0.0", from: "npm" },
      { id: "own", packageName: "@neralabs/rerail-pack-own", version: "0.1.0", from: "repository", path: "packs/own" },
    ]);
    expect(report.rules.map(rule => [rule.ruleId, rule.source.packId, rule.mode])).toEqual([
      ["needles-alpha", "needles", "steer"],
      ["own-rule", "own", "log"],
    ]);
    expect(report.violations.map(violation => [violation.ruleId, violation.mode])).toEqual([["needles-alpha", "steer"], ["own-rule", "log"]]);

    let listed = "";
    expect(runRuleApproveCommand(["--repo", root], () => "usage", { stdout: { write: (text: string) => { listed += text; } } }, home.databasePath)).toBe(0);
    expect(listed).toContain(`pack @neralabs/rerail-pack-own@0.1.0  (from the folder ${join(root, "packs", "own")})`);
    expect(runRuleApproveCommand(["@neralabs/rerail-pack-own", "--repo", root], () => "usage", { stdout: { write: () => true } }, home.databasePath)).toBe(0);

    const approved = await scanIn(home, root);
    expect(approved.rules.map(rule => [rule.ruleId, rule.mode])).toEqual([["needles-alpha", "steer"], ["own-rule", "steer"]]);
  });

  it("runs the packs the file lists at the scan's commit: an edit changes the next scan, and --rules-ref reads the file at that ref", async () => {
    const root = checkout();
    const home = freshHome();
    expect((await scanIn(home, root)).rules.map(rule => rule.ruleId)).toEqual(["needles-alpha", "own-rule"]);

    writePacksFile(root, [{ id: "needles", version: "2.0.0" }]);
    const edited = await scanIn(home, root);
    expect(edited.rules.map(rule => rule.ruleId)).toEqual(["needles-beta"]);
    expect(edited.packs.map(pack => `${pack.id}@${pack.version}`)).toEqual(["needles@2.0.0"]);

    git(root, "commit", "--quiet", "-am", "needles 2");
    const atMain = await scanIn(home, root, { rulesRef: "main~1" });
    expect(atMain.rulesSource).toEqual({ kind: "ref", ref: "main~1" });
    expect(atMain.rules.map(rule => rule.ruleId)).toEqual(["needles-alpha", "own-rule"]);
    expect(atMain.packs.map(pack => pack.from)).toEqual(["npm", "repository"]);
  });

  it("writes nothing outside the scan's folder for a pack whose tree at --rules-ref climbs out with '..' entries", async () => {
    const root = checkout();
    const escaped = `rerail-escaped-${process.pid}-${Date.now()}.json`;
    const mktree = (lines: string[]): string => execFileSync("git", ["mktree"], { cwd: root, input: `${lines.join("\n")}\n`, encoding: "utf8" }).trim();
    const blob = execFileSync("git", ["hash-object", "-w", "--stdin"], { cwd: root, input: "{}", encoding: "utf8" }).trim();
    // rules/../../../../../<escaped>: from the pack's copy in the scan's folder, up to the system's temporary folder.
    let climb = mktree([`100644 blob ${blob}\t${escaped}`]);
    for (let level = 0; level < 5; level += 1) climb = mktree([`040000 tree ${climb}\t..`]);
    const own = git(root, "ls-tree", "HEAD:packs/own").split("\n").map(line => line.endsWith("\trules") ? line.replace(/tree [0-9a-f]+/u, `tree ${mktree([
      ...git(root, "ls-tree", "HEAD:packs/own/rules").split("\n"),
      ...git(root, "ls-tree", climb).split("\n"),
    ])}`) : line);
    const packs = mktree([`040000 tree ${mktree(own)}\town`]);
    const top = git(root, "ls-tree", "HEAD").split("\n").map(line => line.endsWith("\tpacks") ? `040000 tree ${packs}\tpacks` : line);
    const commit = git(root, "commit-tree", mktree(top), "-p", "HEAD", "-m", "climb");

    const report = await scanIn(freshHome(), root, { rulesRef: commit });
    expect(report.packs.map(pack => pack.id)).toEqual(["needles", "own"]);
    expect(existsSync(join(tmpdir(), escaped))).toBe(false);
  });

  it("refuses a file that cannot be parsed, saying what is wrong", async () => {
    const root = checkout();
    const home = freshHome();
    writePacksFile(root, "{ \"schema\": \"rerail.packs/1\", \"packs\": [");
    await expect(scanIn(home, root)).rejects.toThrow(".rerail/packs.json is not valid JSON");
    writePacksFile(root, [{ id: "needles", version: "^1.0.0" }]);
    await expect(scanIn(home, root)).rejects.toThrow(".rerail/packs.json packs[0].version must be one exact version, such as 1.4.0, not \"^1.0.0\"");
    writePacksFile(root, [{ path: "../elsewhere" }]);
    await expect(scanIn(home, root)).rejects.toThrow("packs[0].path \"../elsewhere\" must name a folder inside the repository");
    writePacksFile(root, JSON.stringify({ schema: "rerail.packs/2", packs: [] }));
    await expect(scanIn(home, root)).rejects.toThrow(".rerail/packs.json is not a rerail.packs/1 document: schema:");
  });

  it("runs a pack with folders only in those folders, in the working tree and at --rules-ref", async () => {
    const root = checkout();
    const home = freshHome();
    for (const at of ["web", "legacy"]) {
      mkdirSync(join(root, at));
      writeFileSync(join(root, at, "a.py"), "a = ALPHA\n");
    }
    writePacksFile(root, [{ id: "needles", version: "1.0.0", folders: ["web"] }]);
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "needles in web only");

    const scanned = await scanIn(home, root);
    expect(scanned.status).toBe("completed");
    expect(scanned.packs).toEqual([{ id: "needles", packageName: "@neralabs/rerail-pack-needles", version: "1.0.0", from: "npm", folders: ["web"] }]);
    expect(scanned.violations.map(violation => violation.path)).toEqual(["web/a.py"]);

    // The same commit judged with its own file at a ref, while the working tree widens the pack: the ref's folders win.
    writePacksFile(root, [{ id: "needles", version: "1.0.0" }]);
    const atRef = await scanIn(home, root, { rulesRef: "HEAD" });
    expect(atRef.violations.map(violation => violation.path)).toEqual(["web/a.py"]);
    const widened = await scanIn(home, root);
    expect(widened.violations.map(violation => violation.path).sort()).toEqual(["a.py", "legacy/a.py", "web/a.py"]);
  });

  it("refuses a folder that is not in the scan's tree rather than scanning a pack that judges nothing", async () => {
    const root = checkout();
    writePacksFile(root, [{ id: "needles", version: "1.0.0", folders: ["web"] }]);
    await expect(scanIn(freshHome(), root)).rejects.toThrow(".rerail/packs.json lists the folder web for @neralabs/rerail-pack-needles@1.0.0, which is not in the repository");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "web listed, not added");
    await expect(scanIn(freshHome(), root, { rulesRef: "HEAD" })).rejects.toThrow("HEAD:.rerail/packs.json lists the folder web for @neralabs/rerail-pack-needles@1.0.0, which is not a folder at that commit");
  });

  it("fails rather than scanning without a listed pack npm cannot give", async () => {
    const root = checkout();
    writePacksFile(root, [{ id: "needles", version: "9.9.9" }]);
    await expect(scanIn(freshHome(), root)).rejects.toThrow(".rerail/packs.json lists @neralabs/rerail-pack-needles@9.9.9, which npm could not install");
  });

  it("runs only the home's installed version of each --pack, beside the tree's own rules, in place of the packs the file lists", async () => {
    const root = checkout();
    const home = freshHome();
    installPackDirectory(TWO, "@neralabs/rerail-pack-needles", home.home, "2.0.0");
    mkdirSync(join(root, ".rerail", "rules"));
    writeFileSync(join(root, ".rerail", "rules", "tree-rule.json"), JSON.stringify(needle("tree-rule", "OWN")));
    const installs = npm.installs.length;

    const report = await scanIn(home, root, { packs: ["needles"] });

    expect(report.status).toBe("completed");
    expect(report.rulesSource).toEqual({ kind: "worktree" });
    expect(report.packs).toEqual([{ id: "needles", packageName: "@neralabs/rerail-pack-needles", version: "2.0.0", from: "home" }]);
    expect(report.rules.map(rule => [rule.ruleId, rule.mode])).toEqual([["needles-beta", "steer"], ["tree-rule", "log"]]);
    expect(report.violations.map(violation => violation.ruleId)).toEqual(["needles-beta", "tree-rule"]);
    expect(npm.installs.length).toBe(installs);

    // The file is not read at all, so one that cannot be parsed does not stop the scan.
    writePacksFile(root, "{ \"schema\": \"rerail.packs/1\", \"packs\": [");
    expect((await scanIn(home, root, { packs: ["needles"] })).rules.map(rule => rule.ruleId)).toEqual(["needles-beta", "tree-rule"]);
  });

  it("with --rules-ref, runs --pack from the home and leaves the ref's packs out; a pack installed from a folder runs in Log until approved", async () => {
    const root = checkout();
    const home = freshHome();
    installPackDirectory(writePack(folder("rerail-extra-pack-"), "@acme/rerail-pack-extra", "1.0.0", { "extra-own": "OWN" }), "@acme/rerail-pack-extra", home.home);
    // The change under review lists another pack; with --pack, neither its file nor the ref's chooses the packs.
    writePacksFile(root, [{ id: "needles", version: "2.0.0" }]);

    const report = await scanIn(home, root, { packs: ["@acme/rerail-pack-extra"], rulesRef: "HEAD" });

    expect(report.rulesSource).toEqual({ kind: "ref", ref: "HEAD" });
    expect(report.packs).toEqual([{ id: "extra", packageName: "@acme/rerail-pack-extra", version: "1.0.0", from: "home" }]);
    expect(report.rules.map(rule => [rule.ruleId, rule.mode])).toEqual([["extra-own", "log"]]);

    expect(runRuleApproveCommand(["@acme/rerail-pack-extra", "--repo", root], () => "usage", { stdout: { write: () => true } }, home.databasePath)).toBe(0);
    const approved = await scanIn(home, root, { packs: ["extra"], rulesRef: "HEAD" });
    expect(approved.rules.map(rule => [rule.ruleId, rule.mode])).toEqual([["extra-own", "steer"]]);
  });
});

/** `count` copies of `line(index)`, one per line. */
const lines = (count: number, line: (index: number) => string): string => Array.from({ length: count }, (_, index) => line(index + 1)).join("\n");

/**
 * A file per language of the complexity pack, holding one function per entry of `ccns` at that cyclomatic complexity:
 * `ccn - 1` if statements, each one path more than the function's one.
 */
const COMPLEXITY_FILES: Record<string, { path: string; text: (ccns: readonly number[]) => string }> = {
  python: { path: "src/ccn.py", text: ccns => ccns.map(ccn => `def f${ccn}(x):\n${lines(ccn - 1, at => `    if x == ${at}:\n        return ${at}`)}\n    return 0\n`).join("\n\n") },
  typescript: { path: "src/ccn.ts", text: ccns => ccns.map(ccn => `export function f${ccn}(x: number): number {\n${lines(ccn - 1, at => `  if (x === ${at}) return ${at};`)}\n  return 0;\n}\n`).join("\n") },
  javascript: { path: "src/ccn.js", text: ccns => ccns.map(ccn => `export function f${ccn}(x) {\n${lines(ccn - 1, at => `  if (x === ${at}) return ${at};`)}\n  return 0;\n}\n`).join("\n") },
  php: { path: "src/Ccn.php", text: ccns => `<?php\n${ccns.map(ccn => `function f${ccn}(int $x): int\n{\n${lines(ccn - 1, at => `    if ($x === ${at}) {\n        return ${at};\n    }`)}\n    return 0;\n}\n`).join("\n")}` },
  blade: {
    path: "resources/views/ccn.blade.php",
    text: ccns => `@php\n${ccns.map(ccn => `    $f${ccn} = function (int $x): int {\n${lines(ccn - 1, at => `        if ($x === ${at}) return ${at};`)}\n        return 0;\n    };`).join("\n")}\n@endphp\n<p>{{ $x }}</p>\n`,
  },
  rust: { path: "src/ccn.rs", text: ccns => ccns.map(ccn => `pub fn f${ccn}(x: u32) -> u32 {\n${lines(ccn - 1, at => `    if x == ${at} {\n        return ${at};\n    }`)}\n    0\n}\n`).join("\n") },
  vue: {
    path: "src/components/Ccn.vue",
    text: ccns => `<script setup lang="ts">\n${ccns.map(ccn => `function f${ccn}(x: number): number {\n${lines(ccn - 1, at => `  if (x === ${at}) return ${at};`)}\n  return 0;\n}\n`).join("\n")}</script>\n\n<template>\n  <p>ccn</p>\n</template>\n`,
  },
  java: { path: "src/main/java/com/example/Ccn.java", text: ccns => `class Ccn {\n${ccns.map(ccn => `    int f${ccn}(int x) {\n${lines(ccn - 1, at => `        if (x == ${at}) {\n            return ${at};\n        }`)}\n        return 0;\n    }\n`).join("\n")}}\n` },
};

/** A committed checkout with a function at cyclomatic complexity 6, 7, 10 and 11 in each language, listing `packs`. */
function complexityCheckout(packs: unknown): string {
  const root = folder("rerail-declared-ceiling-");
  git(root, "init", "--quiet", "--initial-branch=main");
  for (const { path, text } of Object.values(COMPLEXITY_FILES)) {
    mkdirSync(join(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), text([6, 7, 10, 11]));
  }
  writePacksFile(root, packs);
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  return root;
}

const complexityPack = (config?: unknown): unknown => ({ id: "complexity", version: COMPLEXITY_VERSION, ...(config === undefined ? {} : { config }) });

/** Per language, the complexity of each function the scan flagged, and every ceiling its messages name. */
function flagged(report: ScanReport): { ccns: Record<string, number[]>; ceilings: string[] } {
  expect(report.status).toBe("completed");
  const ccns = Object.fromEntries(Object.entries(COMPLEXITY_FILES).map(([language, { path }]) => [language, report.violations
    .filter(violation => violation.path === path)
    .map(violation => Number(/has cyclomatic complexity (\d+),/u.exec(violation.message)?.[1]))
    .sort((left, right) => left - right)]));
  const ceilings = [...new Set(report.violations.map(violation => /over the ceiling of (\d+)\./u.exec(violation.message)?.[1] ?? violation.message))];
  return { ccns, ceilings };
}

const everyLanguage = (ccns: number[]): Record<string, number[]> => Object.fromEntries(Object.keys(COMPLEXITY_FILES).map(language => [language, ccns]));

describe("the complexity pack's ceiling, set in .rerail/packs.json", () => {
  it("flags functions over 6 without config, and over the ceiling the file sets with it, in every language, naming the ceiling", async () => {
    const root = complexityCheckout([complexityPack()]);
    const home = freshHome();
    expect(flagged(await scanIn(home, root))).toEqual({ ccns: everyLanguage([7, 10, 11]), ceilings: ["6"] });

    writePacksFile(root, [complexityPack({ ceiling: 10 })]);
    const configured = await scanIn(home, root);
    expect(flagged(configured)).toEqual({ ccns: everyLanguage([11]), ceilings: ["10"] });
    expect(configured.packs).toEqual([{ id: "complexity", packageName: "@neralabs/rerail-pack-complexity", version: COMPLEXITY_VERSION, from: "npm", config: { ceiling: 10 } }]);
  });

  it.each([0, 1.5, "10"])("fails the scan for a ceiling of %j, naming the file, the pack and the option", async ceiling => {
    const root = complexityCheckout([complexityPack({ ceiling })]);
    await expect(scanIn(freshHome(), root)).rejects.toThrow(
      `.rerail/packs.json sets config.ceiling for @neralabs/rerail-pack-complexity@${COMPLEXITY_VERSION} to ${JSON.stringify(ceiling)}, which complexity-cyclomatic-`,
    );
  });

  it("fails the scan for an option no rule of the pack declares, naming the file, the pack and the option", async () => {
    const root = complexityCheckout([complexityPack({ ceiling: 10, max: 10 })]);
    await expect(scanIn(freshHome(), root)).rejects.toThrow(
      `.rerail/packs.json sets config.max for @neralabs/rerail-pack-complexity@${COMPLEXITY_VERSION}, which no rule of that pack declares (its rules declare ceiling)`,
    );
  });

  it("takes the ceiling from the file at --rules-ref, so a change cannot raise the ceiling it is judged by", async () => {
    const root = complexityCheckout([complexityPack({ ceiling: 10 })]);
    const home = freshHome();
    writePacksFile(root, [complexityPack({ ceiling: 20 })]);
    git(root, "commit", "--quiet", "-am", "raise the ceiling");

    expect(flagged(await scanIn(home, root))).toEqual({ ccns: everyLanguage([]), ceilings: [] });
    const atBase = await scanIn(home, root, { rulesRef: "main~1" });
    expect(flagged(atBase)).toEqual({ ccns: everyLanguage([11]), ceilings: ["10"] });
    expect(atBase.packs.map(pack => pack.config)).toEqual([{ ceiling: 10 }]);
  });

  it("runs a --pack that replaces the file's packs at its defaults, not at the file's ceiling", async () => {
    const root = complexityCheckout([complexityPack({ ceiling: 10 })]);
    const home = freshHome();
    installPackDirectory(COMPLEXITY, "@neralabs/rerail-pack-complexity", home.home, COMPLEXITY_VERSION);

    const report = await scanIn(home, root, { packs: ["complexity"] });
    expect(report.packs).toEqual([{ id: "complexity", packageName: "@neralabs/rerail-pack-complexity", version: COMPLEXITY_VERSION, from: "home" }]);
    expect(flagged(report)).toEqual({ ccns: everyLanguage([7, 10, 11]), ceilings: ["6"] });
  });
});
