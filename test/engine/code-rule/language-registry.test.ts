import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  LANGUAGE_REGISTRY_MODULE_PATH,
  languageEntrySchema,
  languageRegistryProblems,
  readLanguageRegistry,
  renderLanguageRegistryModule,
  type LanguageEntry,
} from "../scripts/generate-language-registry.js";
import { languageFor } from "../src/code-rule-enforcer.js";
import { generatedEditPaths } from "../src/code-rule-bridge.js";
import { LANGUAGE_REGISTRY, CODE_RULE_DIALECTS, codeRuleDialectForPath } from "../src/code-rule-languages.js";

const capabilities = (support: "supported" | "refused" | "not-supported-yet") => ({ scope: support, value: support, arguments: support, calls: support, formatArgs: support });

function supported(id: string, suffix: string, tapeTag: number): LanguageEntry {
  return languageEntrySchema.parse({
    format: "rerail.language/v1", id, status: "supported", family: id,
    dialects: [{ label: id, variant: "One", suffixes: [suffix], parser: "tree-sitter", grammar: `vendored:${id}`, tapeTag }],
    capabilities: capabilities("supported"), suppressions: [],
  });
}

function planned(id: string, suffix: string): LanguageEntry {
  return languageEntrySchema.parse({
    format: "rerail.language/v1", id, status: "planned", family: id,
    dialects: [{ label: id, suffixes: [suffix] }],
    capabilities: capabilities("not-supported-yet"), suppressions: [],
  });
}

const problems = (...entries: LanguageEntry[]) => languageRegistryProblems(entries.map(entry => ({ file: `languages/${entry.id}.json`, entry })));
const issues = (entry: unknown) => languageEntrySchema.safeParse(entry).error?.issues.map(issue => `${issue.path.join("/")}: ${issue.message}`) ?? [];

describe("the language registry", () => {
  it("validates, and the host's generated module is current", () => {
    const entries = readLanguageRegistry();
    expect(readFileSync(LANGUAGE_REGISTRY_MODULE_PATH, "utf8")).toBe(renderLanguageRegistryModule(entries));
    expect(entries.map(entry => [entry.id, entry.status])).toEqual([
      ["python", "supported"], ["typescript", "supported"], ["javascript", "supported"], ["php", "supported"], ["blade", "supported"],
      ["rust", "planned"], ["sql", "planned"], ["vue", "planned"],
    ]);
  });

  it("refuses repeated ids, labels, suffixes and tape tags", () => {
    expect(problems(supported("one", ".one", 1), supported("one", ".two", 2))).toContain("languages/one.json: id one is also languages/one.json's");
    expect(problems(supported("one", ".one", 1), planned("two", ".one"))).toEqual(["languages/two.json: suffix .one is also languages/one.json's"]);
    expect(problems(supported("one", ".one", 1), supported("two", ".two", 1))).toEqual(["languages/two.json: tape tag 1 is also languages/one.json's"]);
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

  it("routes no file to a planned language", () => {
    for (const entry of LANGUAGE_REGISTRY.filter(entry => entry.status === "planned")) {
      for (const { label, suffixes } of entry.dialects) {
        expect(Object.keys(CODE_RULE_DIALECTS)).not.toContain(label);
        for (const suffix of suffixes) {
          const path = `src/file${suffix}`;
          expect(codeRuleDialectForPath(path)).toBeUndefined();
          expect(Object.keys(CODE_RULE_DIALECTS)).not.toContain(languageFor(path));
          expect(generatedEditPaths("write", { path }).length).toBe(languageFor(path) === undefined ? 0 : 1);
        }
      }
    }
  });
});
