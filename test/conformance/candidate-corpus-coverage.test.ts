/**
 * Corpus coverage of candidates (`rowFor` and `runCorpus`, conformance/languages/runner.ts): a corpus recording covers
 * the shipped ports, so a candidate it does not cover is skipped on that corpus, with no row and no error; a shipped
 * port it does not cover is an error, and the fixtures recording must cover every port.
 */
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { loadPack } from "../../conformance/languages/pack.js";
import { reportFailures, runPack } from "../../conformance/languages/runner.js";
import { fixturesPath } from "../support/oracles.js";

const editJson = (path: string, edit: (value: Record<string, unknown>) => unknown): void =>
  writeFileSync(path, JSON.stringify(edit(JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>), null, 2));

describe("a corpus recording that does not cover a port", () => {
  let directory: string;
  let corpusRoot: string;
  const options = (): Parameters<typeof runPack>[1] => ({ fixtures: true, corpora: ["mini"], corpusRoot, date: "2026-01-01" });

  beforeEach(() => {
    // The `valid` pack ports examplelint's EX101 (every assert) as rules/assert-used.json. It gains a corpus, `mini`,
    // whose examplelint recording covers another rule only.
    directory = mkdtempSync(join(tmpdir(), "rerail-candidate-corpus-"));
    cpSync(fixturesPath("packs", "valid"), directory, { recursive: true });
    corpusRoot = join(directory, "corpora");
    mkdirSync(join(corpusRoot, "mini"), { recursive: true });
    writeFileSync(join(corpusRoot, "mini", "app.py"), "assert ready\n");
    editJson(join(directory, "conformance.json"), pack => ({
      ...pack,
      corpora: [{ name: "mini", repository: "https://example.invalid/mini", commit: "0".repeat(40), paths: ["."], exclude: [] }],
    }));
    writeFileSync(
      join(directory, "upstream", "examplelint", "corpus-mini.expected.json"),
      JSON.stringify({
        tool: "examplelint",
        version: "1.0.0",
        source: { kind: "corpus", name: "mini", repository: "https://example.invalid/mini", commit: "0".repeat(40), rules: ["EX999"] },
        findings: [],
      }),
    );
  });

  afterEach(() => rmSync(directory, { recursive: true, force: true }));

  const asCandidate = (): void => {
    mkdirSync(join(directory, "candidates", "rules"), { recursive: true });
    cpSync(join(directory, "rules", "assert-used.json"), join(directory, "candidates", "rules", "assert-used.json"));
    editJson(join(directory, "conformance.json"), pack => ({
      ...pack,
      rules: (pack.rules as Record<string, unknown>[]).map(rule => ({ ...rule, policy: "candidates/rules/assert-used.json" })),
    }));
  };

  it("is an error for a shipped port", async () => {
    const report = await runPack(loadPack(directory), options());
    expect(report.errors).toEqual(["assert-used (corpus:mini): the examplelint recording does not cover EX101"]);
    expect(report.pass).toBe(false);
  });

  it("skips a candidate on that corpus, and still scores it on the fixtures", async () => {
    asCandidate();
    const report = await runPack(loadPack(directory), options());
    expect(report.errors).toEqual([]);
    expect(report.rules.map(({ rule, source, candidate }) => ({ rule, source, candidate }))).toEqual([{ rule: "assert-used", source: "fixtures", candidate: true }]);
    expect(reportFailures(report)).toEqual([]);
  });

  it("skips a candidate whose tool has no recording of the corpus", async () => {
    asCandidate();
    rmSync(join(directory, "upstream", "examplelint", "corpus-mini.expected.json"));
    const report = await runPack(loadPack(directory), options());
    expect(report.errors).toEqual([]);
    expect(report.rules.map(({ source }) => source)).toEqual(["fixtures"]);
  });

  it("is still an error for a candidate the fixtures recording does not cover", async () => {
    asCandidate();
    editJson(join(directory, "upstream", "examplelint", "fixtures.expected.json"), recording => ({
      ...recording,
      source: { ...(recording.source as Record<string, unknown>), rules: ["EX999"] },
      findings: [],
    }));
    const report = await runPack(loadPack(directory), options());
    expect(report.errors).toContain("assert-used (fixtures): the examplelint recording does not cover EX101");
    expect(report.pass).toBe(false);
  });
});
