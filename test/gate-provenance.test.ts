/**
 * Check 7 (provenance, conformance/gates/provenance.ts): V1 Gate 1's rule checks from the rule validation ladder, read
 * from a tool's source.json, and the tool-level pins that tie source.json to conformance.json and the recordings.
 */
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ruleProvenanceChecks, toolPins, type DeclaredTool } from "../conformance/gates/provenance.js";
import { packSchema, upstreamSourceSchema, type LoadedPack, type UpstreamRule, type UpstreamSource } from "../conformance/languages/pack.js";

const COMMIT = "2d0b675b04c80ae42277e10500db06a0a37bae17";
const REPOSITORY = "https://github.com/PyCQA/bandit";

function bandit(rules: Record<string, Partial<UpstreamRule>> = {}, overrides: Partial<UpstreamSource> = {}): UpstreamSource {
  const rule = (partial: Partial<UpstreamRule>): UpstreamRule => ({
    implementation: ["bandit/plugins/asserts.py"],
    tests: ["examples/assert.py"],
    deprecated: null,
    enabledBy: { configuration: "default", options: [] },
    names: { implementation: ["assert"], shared: [] },
    regexes: [],
    unscored: [],
    ...partial,
  });
  return upstreamSourceSchema.parse({
    tool: "bandit",
    version: "1.8.6",
    repository: REPOSITORY,
    commit: COMMIT,
    tag: "1.8.6",
    configurations: ["default"],
    extensions: [".py"],
    keywords: ["assert"],
    commentVocabulary: ["nosec"],
    testWords: ["assert"],
    configurationWords: [],
    sharedNames: [],
    rules: Object.fromEntries(Object.entries({ B101: {}, ...rules }).map(([name, partial]) => [name, rule(partial)])),
    ...overrides,
  });
}

function failed(checks: { id: string; ok: boolean }[]): string[] {
  return checks.filter(check => !check.ok).map(check => check.id);
}

describe("check 7: provenance of a port", () => {
  it("requires an upstream rule that is stable and on in an approved configuration", () => {
    expect(ruleProvenanceChecks("B101", bandit()).map(check => [check.id, check.ok])).toEqual([
      ["provenance:upstream-rule", true],
      ["provenance:stable", true],
      ["provenance:recommended", true],
    ]);
    expect(failed(ruleProvenanceChecks("B101", bandit({ B101: { tests: [] } })))).toEqual(["provenance:upstream-rule"]);
    expect(failed(ruleProvenanceChecks("B101", bandit({ B101: { implementation: [] } })))).toEqual(["provenance:upstream-rule"]);
    expect(failed(ruleProvenanceChecks("B101", bandit({ B101: { deprecated: "redundant" } })))).toEqual(["provenance:stable"]);
    expect(failed(ruleProvenanceChecks("B101", bandit({ B101: { enabledBy: null } })))).toEqual(["provenance:recommended"]);
    expect(ruleProvenanceChecks("B101", bandit({ B101: { enabledBy: null } }))[2]!.detail).toBe("not on in any approved upstream configuration");
  });

  it("fails every provenance sub-check for a rule source.json does not record, the later two as not run", () => {
    const checks = ruleProvenanceChecks("nera-only", bandit());
    expect(failed(checks)).toEqual(["provenance:upstream-rule", "provenance:stable", "provenance:recommended"]);
    expect(checks.map(check => check.detail)).toEqual([
      "nera-only is not a rule source.json records for bandit 1.8.6",
      "not run: nera-only is not a rule source.json records for bandit 1.8.6",
      "not run: nera-only is not a rule source.json records for bandit 1.8.6",
    ]);
  });

  it("names the configuration and the options it sets", () => {
    expect(ruleProvenanceChecks("B101", bandit())[2]!.detail).toBe("on in default with default options");
    const withOptions = ruleProvenanceChecks("B101", bandit({ B101: { enabledBy: { configuration: "default", options: [{ skips: [] }] } } }));
    expect(withOptions[2]!.detail).toBe('on in default with options [{"skips":[]}]');
  });
});

describe("check 7: provenance:pins", () => {
  let directory: string;
  let loaded: LoadedPack;
  const tool: DeclaredTool = { name: "bandit", version: "1.8.6", recommended: ["default"] };
  const write = (path: string, value: unknown): void => {
    mkdirSync(join(directory, "upstream", "bandit"), { recursive: true });
    writeFileSync(join(directory, "upstream", "bandit", path), typeof value === "string" ? value : `${JSON.stringify(value, null, 2)}\n`);
  };
  const fixtures = (origin = `${REPOSITORY}@${COMMIT}`, version = "1.8.6") => ({
    tool: "bandit",
    version,
    source: { kind: "fixtures", origin, rules: ["B101"], files: ["examples/assert.py"] },
    findings: [],
  });
  const corpus = (version = "1.8.6") => ({
    tool: "bandit",
    version,
    source: { kind: "corpus", name: "django", repository: "https://github.com/django/django", commit: "fd91518f17c8a84fa47fe0e5534ef46afa7a0f7a", rules: ["B101"] },
    findings: [],
  });

  beforeEach(() => {
    directory = mkdtempSync(join(tmpdir(), "rerail-pins-"));
    const pack = packSchema.parse({
      pack: "python",
      family: "python",
      capabilities: ["assert"],
      corpora: [{ name: "django", repository: "https://github.com/django/django", commit: "fd91518f17c8a84fa47fe0e5534ef46afa7a0f7a", paths: ["django"] }],
      tools: [tool],
      rules: [],
    });
    loaded = { pack, directory, oraclesDirectory: join(directory, "upstream"), divergences: [] };
    write("source.json", bandit());
    write("fixtures.expected.json", fixtures());
    write("corpus-django.expected.json", corpus());
  });
  afterEach(() => rmSync(directory, { recursive: true, force: true }));

  it("passes source facts of the pinned tool, version, configurations and commit", () => {
    const pins = toolPins(loaded, tool);
    expect(pins.failures).toEqual([]);
    expect(pins.source?.commit).toBe(COMMIT);
  });

  it("fails a missing or invalid source.json, which leaves the tool without facts", () => {
    rmSync(join(directory, "upstream", "bandit", "source.json"));
    expect(toolPins(loaded, tool)).toEqual({ source: undefined, failures: ["bandit: upstream/bandit/source.json is missing; the tool declares recommended configurations"] });

    write("source.json", JSON.stringify(bandit()).replace('"tool":"bandit"', '"tool":"bandit","tool":"bandit"'));
    const repeated = toolPins(loaded, tool);
    expect(repeated.source).toBeUndefined();
    expect(repeated.failures).toHaveLength(1);
    expect(repeated.failures[0]).toMatch(/^bandit: upstream\/bandit\/source\.json: .*tool/u);

    write("source.json", { ...bandit(), commit: "2d0b675" });
    const invalid = toolPins(loaded, tool);
    expect(invalid.source).toBeUndefined();
    expect(invalid.failures[0]).toMatch(/^bandit: upstream\/bandit\/source\.json: .*commit/su);
  });

  it("fails facts recorded for another tool, version or set of configurations than conformance.json declares", () => {
    write("source.json", { ...bandit(), tool: "ruff" });
    expect(toolPins(loaded, tool).failures).toEqual(["bandit: upstream/bandit/source.json records tool ruff, not the tool its directory names"]);
    write("source.json", bandit());
    expect(toolPins(loaded, { ...tool, version: "1.8.7" }).failures).toEqual(["bandit: upstream/bandit/source.json records version 1.8.6; conformance.json pins 1.8.7"]);
    expect(toolPins(loaded, { ...tool, recommended: ["default", "strict"] }).failures).toEqual([
      'bandit: upstream/bandit/source.json records configurations ["default"]; conformance.json recommends ["default","strict"]',
    ]);
  });

  it("fails facts of another commit or version than the recordings were made from", () => {
    write("fixtures.expected.json", fixtures(`${REPOSITORY}@${"1".repeat(40)}`));
    expect(toolPins(loaded, tool).failures).toEqual([`bandit: upstream/bandit/fixtures.expected.json was copied from ${REPOSITORY}@${"1".repeat(40)}; source.json records ${REPOSITORY}@${COMMIT}`]);
    write("fixtures.expected.json", fixtures());
    write("corpus-django.expected.json", corpus("1.8.5"));
    expect(toolPins(loaded, tool).failures).toEqual(["bandit: upstream/bandit/corpus-django.expected.json was recorded with version 1.8.5; source.json records 1.8.6"]);
    rmSync(join(directory, "upstream", "bandit", "fixtures.expected.json"));
    write("corpus-django.expected.json", corpus());
    expect(toolPins(loaded, tool).failures).toEqual(["bandit: upstream/bandit/fixtures.expected.json is missing, so nothing pins source.json's commit"]);
  });
});
