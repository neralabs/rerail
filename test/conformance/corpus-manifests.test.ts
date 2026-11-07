/**
 * Corpus manifests (`runCorpus`, conformance/languages/runner.ts): a corpus's sources are scored with the facts a host
 * reads from their package.json files and project configurations, which ride along as fixture units' do, including one
 * above the paths the corpus walks and an extends base outside them.
 */
import { cpSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { loadPack } from "../../conformance/languages/pack.js";
import { runPack } from "../../conformance/languages/runner.js";
import { fixturesPath } from "../support/oracles.js";

const COMMIT = "0".repeat(40);

describe("a corpus's package.json files", () => {
  let directory: string;
  let corpusRoot: string;

  beforeEach(() => {
    // The `valid` pack, made an ECMA pack whose one port reports every file of a package that depends on next. Its
    // corpus `mini` walks `app` only; the manifest that says so is the checkout's root package.json.
    directory = mkdtempSync(join(tmpdir(), "rerail-corpus-manifests-"));
    cpSync(fixturesPath("packs", "valid"), directory, { recursive: true });
    writeFileSync(join(directory, "conformance.json"), JSON.stringify({
      pack: "valid",
      family: "ecma",
      capabilities: ["syntax"],
      corpora: [{ name: "mini", repository: "https://example.invalid/mini", commit: COMMIT, paths: ["app"], exclude: [] }],
      tools: [{ name: "examplelint", version: "1.0.0" }],
      rules: [{ id: "next-file", upstream: { tool: "examplelint", rule: "EX200" }, policy: "rules/next-file.json", capabilities: ["syntax"], match: "line+column" }],
    }));
    writeFileSync(join(directory, "divergences.json"), "[]\n");
    writeFileSync(join(directory, "rules", "next-file.json"), JSON.stringify({
      format: "rerail.policy/v1", id: "valid:next-file", revision: 1, mode: "steer",
      scope: { include: ["**/*"], exclude: [] },
      clauses: [{
        id: "next-file",
        forbid: { ast: { language: "typescript", file: { package: { dependsOn: ["next"] } }, rule: { kind: "program" } } },
        violation: { anchor: "witness.span", code: "EX200", message: "examplelint EX200" },
      }],
    }));
    writeFileSync(join(directory, "upstream", "examplelint", "corpus-mini.expected.json"), JSON.stringify({
      tool: "examplelint",
      version: "1.0.0",
      source: { kind: "corpus", name: "mini", repository: "https://example.invalid/mini", commit: COMMIT, rules: ["EX200"] },
      findings: [{ rule: "EX200", path: "app/page.js", line: 1, column: 1 }],
    }));
    corpusRoot = join(directory, "corpora");
    mkdirSync(join(corpusRoot, "mini", "app"), { recursive: true });
    writeFileSync(join(corpusRoot, "mini", "package.json"), '{ "dependencies": { "next": "15" } }\n');
    writeFileSync(join(corpusRoot, "mini", "app", "page.js"), "export {};\n");
  });

  afterEach(() => rmSync(directory, { recursive: true, force: true }));

  it("give its sources their manifest facts, a manifest above the walked paths included", async () => {
    const report = await runPack(loadPack(directory), { fixtures: false, corpora: ["mini"], corpusRoot, date: "2026-01-01" });
    expect(report.errors).toEqual([]);
    expect(report.rules.map(({ rule, source, tp, fp, fn }) => ({ rule, source, tp, fp, fn }))).toEqual([
      { rule: "next-file", source: "corpus:mini", tp: 1, fp: 0, fn: 0 },
    ]);
    expect(report.sources.map(({ source, files }) => ({ source, files }))).toEqual([{ source: "corpus:mini", files: 1 }]);
  });

  it("give its sources the jsxImportSource of a project configuration's base outside the walked paths", async () => {
    // React and next listed, so only the configuration's Preact runtime makes the file `other`.
    writeFileSync(join(directory, "rules", "next-file.json"), JSON.stringify({
      format: "rerail.policy/v1", id: "valid:next-file", revision: 1, mode: "steer",
      scope: { include: ["**/*"], exclude: [] },
      clauses: [{
        id: "next-file",
        forbid: { ast: { language: "typescript", file: { jsxRuntime: ["other"] }, rule: { kind: "program" } } },
        violation: { anchor: "witness.span", code: "EX200", message: "examplelint EX200" },
      }],
    }));
    writeFileSync(join(corpusRoot, "mini", "package.json"), '{ "dependencies": { "next": "15", "react": "19" } }\n');
    writeFileSync(join(corpusRoot, "mini", "tsconfig.json"), '{ "extends": "./config/tsconfig-base.json" }\n');
    mkdirSync(join(corpusRoot, "mini", "config"), { recursive: true });
    writeFileSync(join(corpusRoot, "mini", "config", "tsconfig-base.json"), '{ "compilerOptions": { "jsxImportSource": "preact", } }\n');
    const report = await runPack(loadPack(directory), { fixtures: false, corpora: ["mini"], corpusRoot, date: "2026-01-01" });
    expect(report.errors).toEqual([]);
    expect(report.rules.map(({ rule, source, tp, fp, fn }) => ({ rule, source, tp, fp, fn }))).toEqual([
      { rule: "next-file", source: "corpus:mini", tp: 1, fp: 0, fn: 0 },
    ]);
  });
});
