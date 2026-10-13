import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  LANGUAGES_DIRECTORY,
  LANGUAGE_REGISTRY_MODULE_PATH,
  languageEntrySchema,
  languageRegistryProblems,
  readLanguageRegistry,
  renderLanguageRegistryModule,
  type LanguageEntry,
} from "../../../scripts/generate/languages/registry.js";
import { languageFor } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import { generatedEditPaths } from "../../../src/engine/code-rule/code-rule-bridge.js";
import {
  LANGUAGE_REGISTRY,
  CODE_RULE_AST_LANGUAGES,
  CODE_RULE_DIALECTS,
  CODE_RULE_LANGUAGE_IDS,
  CODE_RULE_PLANNED_LANGUAGES,
  codeRuleDialectForPath,
} from "../../../src/engine/code-rule/code-rule-languages.js";
import { EXEMPLAR, exemplarLanguageRegistry, exemplarRegistryModule } from "../../support/exemplar-registry.js";

const capabilities = (support: "supported" | "refused" | "not-supported-yet") => ({ scope: support, value: support, arguments: support, calls: support, formatArgs: support });

function supported(id: string, suffix: string, tapeTag: number): LanguageEntry {
  return languageEntrySchema.parse({
    format: "rerail.language/v1", id, status: "supported", family: id,
    dialects: [{ label: id, variant: "One", suffixes: [suffix], parser: "tree-sitter", grammar: `vendored:${id}`, tapeTag }],
    capabilities: capabilities("supported"), suppressions: [],
  });
}

function planned(id: string, suffix: string, tapeTag?: number): LanguageEntry {
  return languageEntrySchema.parse({
    format: "rerail.language/v1", id, status: "planned", family: id,
    dialects: [{ label: id, suffixes: [suffix], ...(tapeTag === undefined ? {} : { tapeTag }) }],
    capabilities: capabilities("not-supported-yet"), suppressions: [],
  });
}

const problems = (...entries: LanguageEntry[]) => languageRegistryProblems(entries.map(entry => ({ file: `languages/${entry.id}.json`, entry })));
const issues = (entry: unknown) => languageEntrySchema.safeParse(entry).error?.issues.map(issue => `${issue.path.join("/")}: ${issue.message}`) ?? [];

describe("the language registry", () => {
  it("validates, and the host's generated module is current", () => {
    const entries = readLanguageRegistry();
    expect(readFileSync(LANGUAGE_REGISTRY_MODULE_PATH, "utf8")).toBe(renderLanguageRegistryModule(entries));
    expect(entries.slice(0, 5).map(entry => [entry.id, entry.status])).toEqual([
      ["python", "supported"], ["typescript", "supported"], ["javascript", "supported"], ["php", "supported"], ["blade", "supported"],
    ]);
    // Supported languages come first, in the order they were added; planned ones follow.
    const firstPlanned = entries.findIndex(entry => entry.status === "planned");
    expect(entries.slice(firstPlanned).every(entry => entry.status === "planned")).toBe(true);
  });

  it("keeps the test-only exemplar out of the product registry and the host module", () => {
    expect(readLanguageRegistry().map(entry => entry.id)).not.toContain(EXEMPLAR);
    expect(readdirSync(LANGUAGES_DIRECTORY)).not.toContain(`${EXEMPLAR}.json`);
    expect(readFileSync(LANGUAGE_REGISTRY_MODULE_PATH, "utf8")).not.toContain(EXEMPLAR);
    expect(CODE_RULE_LANGUAGE_IDS).not.toContain(EXEMPLAR);
    expect(CODE_RULE_PLANNED_LANGUAGES).not.toContain(EXEMPLAR);
  });

  it("reads the exemplar beside the product registry for tests: planned, with every capability not supported yet", () => {
    const product = readLanguageRegistry();
    const entries = exemplarLanguageRegistry();
    expect(entries.map(entry => entry.id).toSorted()).toEqual([...product.map(entry => entry.id), EXEMPLAR].toSorted());
    expect(entries.filter(entry => entry.status === "supported")).toEqual(product.filter(entry => entry.status === "supported"));
    const exemplar = entries.find(entry => entry.id === EXEMPLAR)!;
    expect(exemplar).toMatchObject({ status: "planned", family: EXEMPLAR, suppressions: [], dialects: [{ label: EXEMPLAR, suffixes: [".exemplar"] }] });
    expect(exemplar.dialects[0]).not.toHaveProperty("tapeTag");
    expect(Object.values(exemplar.capabilities).every(support => support === "not-supported-yet")).toBe(true);
    expect(exemplarRegistryModule().CODE_RULE_PLANNED_LANGUAGES).toContain(EXEMPLAR);
    expect(exemplarRegistryModule().CODE_RULE_AST_LANGUAGES).toEqual(CODE_RULE_AST_LANGUAGES);
  });

  it("checks test-only languages like any entry, and refuses one that collides with the product registry or is supported", () => {
    const product = mkdtempSync(join(tmpdir(), "rerail-product-languages-"));
    const directory = mkdtempSync(join(tmpdir(), "rerail-test-languages-"));
    try {
      const write = (where: string, entry: LanguageEntry) => writeFileSync(join(where, `${entry.id}.json`), JSON.stringify(entry));
      write(product, supported("one", ".one", 1));
      write(directory, planned("two", ".one"));
      expect(() => readLanguageRegistry(product, directory)).toThrow("native/test-languages/two.json: suffix .one is also languages/one.json's");
      rmSync(join(directory, "two.json"));
      write(directory, supported("three", ".three", 3));
      expect(() => readLanguageRegistry(product, directory)).toThrow("native/test-languages/three.json: a test-only language is planned");
      rmSync(join(directory, "three.json"));
      write(directory, planned("four", ".four"));
      expect(readLanguageRegistry(product, directory).map(entry => entry.id)).toEqual(["one", "four"]);
      expect(readLanguageRegistry(product).map(entry => entry.id)).toEqual(["one"]);
    } finally {
      rmSync(product, { recursive: true, force: true });
      rmSync(directory, { recursive: true, force: true });
    }
  });

  it("refuses repeated ids, labels, suffixes and tape tags", () => {
    expect(problems(supported("one", ".one", 1), supported("one", ".two", 2))).toContain("languages/one.json: id one is also languages/one.json's");
    expect(problems(supported("one", ".one", 1), planned("two", ".one"))).toEqual(["languages/two.json: suffix .one is also languages/one.json's"]);
    expect(problems(supported("one", ".one", 1), supported("two", ".two", 1))).toEqual(["languages/two.json: tape tag 1 is also languages/one.json's"]);
  });

  it("allows reserved tape tags on planned dialects", () => {
    const future = planned("future", ".future", 9);
    expect(future.dialects[0]!.tapeTag).toBe(9);
    expect(problems(future)).toEqual([]);
  });

  it("refuses a planned dialect reusing another entry's tape tag", () => {
    expect(problems(supported("one", ".one", 1), planned("future", ".future", 1))).toEqual([
      "languages/future.json: tape tag 1 is also languages/one.json's",
    ]);
    expect(problems(planned("one", ".one", 1), planned("future", ".future", 1))).toEqual([
      "languages/future.json: tape tag 1 is also languages/one.json's",
    ]);
  });

  it("requires a grammar, parser, variant and tape tag of every supported dialect, and none of a planned one", () => {
    const { grammar: _grammar, ...dialect } = supported("one", ".one", 1).dialects[0]!;
    expect(issues({ ...supported("one", ".one", 1), dialects: [dialect] })).toEqual([
      "dialects/0/grammar: A supported dialect needs a grammar; a supported language needs a grammar for every dialect.",
    ]);
    expect(issues({ ...planned("two", ".two"), capabilities: { ...capabilities("not-supported-yet"), scope: "supported" } })).toEqual([
      "capabilities: A planned language supports no capability yet.",
    ]);
    expect(issues({ ...planned("two", ".two"), parser: "tree-sitter" })).toEqual([expect.stringContaining("Unrecognized key")]);
    expect(issues({ ...planned("two", ".two"), format: "rerail.language/v2" })).toEqual([expect.stringMatching(/^format: /u)]);
  });

  it("requires one capability vocabulary, and runsOn and embeds that resolve", () => {
    const { calls: _calls, ...fewer } = capabilities("not-supported-yet");
    expect(problems(supported("one", ".one", 1), { ...planned("two", ".two"), capabilities: fewer })).toEqual([
      "languages/two.json: capabilities must name the same capabilities as languages/one.json",
    ]);
    expect(problems({ ...supported("one", ".one", 1), runsOn: ["one", "two"] }, supported("two", ".two", 2))).toEqual([
      "languages/one.json: runsOn dialect two is not a supported dialect of family one",
    ]);
    expect(problems({ ...planned("two", ".two"), embeds: [{ language: "three", via: "script-block" }] })).toEqual([
      "languages/two.json: embeds unknown language three",
    ]);
  });
  it("refuses supported languages embedding planned languages", () => {
    expect(
      problems(
        { ...supported("one", ".one", 1), embeds: [{ language: "two", via: "script-block" }] },
        planned("two", ".two"),
      ),
    ).toEqual([
      "languages/one.json: embeds planned language two; a supported language embeds only supported ones",
    ]);
  });

  it("routes a planned language's files to no dialect, reading them as text", () => {
    for (const entry of LANGUAGE_REGISTRY.filter(entry => entry.status === "planned")) {
      for (const { label, suffixes } of entry.dialects) {
        expect(Object.keys(CODE_RULE_DIALECTS)).not.toContain(label);
        for (const suffix of suffixes) {
          const path = `src/file${suffix}`;
          expect(codeRuleDialectForPath(path)).toBeUndefined();
          expect(languageFor(path)).toBe("text");
          expect(generatedEditPaths("write", { path })).toEqual([path]);
        }
      }
    }
  });
});
