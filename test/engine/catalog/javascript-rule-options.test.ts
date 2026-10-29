/**
 * The JavaScript pack's ESLint ports under non-default options. The conformance check scores every port at its default
 * options only (packs/javascript/upstream/generate.mjs ignores a case's own options), so what a repository's `config`
 * changes is held here: each case is ESLint's answer on the code under the same options.
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { evaluateExample } from "../../../conformance/pack-validation.js";
import { applyCodeRuleConfig } from "../../../src/engine/code-rule/code-rule-options.js";
import type { CodeRuleDslValue } from "../../../src/engine/code-rule/code-rule.js";
import { loadNativeBinding } from "../../../src/engine/native/native.js";
import { repositoryPath } from "../../support/oracles.js";

const capabilities = readFileSync(repositoryPath("native", "src", "code_rule_dsl", "capabilities.json"), "utf8");

/** How many findings the pack's rule `id`, configured with `config`, reports on `source` read as `path`. */
function findings(id: string, config: Record<string, CodeRuleDslValue>, source: string, path = "src/main.js"): number {
  const document: unknown = JSON.parse(readFileSync(repositoryPath("packs", "javascript", "rules", `${id}.json`), "utf8"));
  const compiled = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(applyCodeRuleConfig(document, config)), "json", capabilities);
  const result = evaluateExample(compiled, path, source);
  expect(result.failures).toEqual([]);
  return result.codes.length;
}

describe("eslint-dot-notation allowPattern", () => {
  it.each([
    { config: { allowPattern: "^foo" }, source: 'obj["foobar"];\n', expected: 0 },
    { config: { allowPattern: "^foo" }, source: 'obj["barfoo"];\n', expected: 1 },
    { config: { allowPattern: "^nu" }, source: "obj[null];\n", expected: 0 },
    { config: { allowKeywords: true, allowPattern: "^a" }, source: 'o["ab"];\n', expected: 0 },
    { config: { allowKeywords: true, allowPattern: "^a" }, source: 'o["b"];\n', expected: 1 },
    { config: { allowKeywords: false, allowPattern: "^a" }, source: "o[`ab`];\n", expected: 0 },
    { config: { allowKeywords: false, allowPattern: "^a" }, source: "o[`b`];\n", expected: 1 },
    { config: { allowPattern: "" }, source: 'obj["foo_bar"];\n', expected: 1 },
    { config: {}, source: 'obj["foo_bar"];\n', expected: 1 },
  ])("$config on $source: $expected finding(s), as ESLint", ({ config, source, expected }) => {
    expect(findings("eslint-dot-notation", config, source)).toBe(expected);
  });

  it("refuses a pattern in JavaScript-only regex syntax instead of ignoring it", () => {
    expect(() => findings("eslint-dot-notation", { allowPattern: "(?<=a)b" }, 'o["ab"];\n')).toThrow(/regex/u);
  });
});

describe("eslint-no-duplicate-imports allowSeparateTypeImports", () => {
  const separate = { allowSeparateTypeImports: true };
  it("counts a default import named `type` as a value import", () => {
    expect(findings("eslint-no-duplicate-imports", separate, "import type from 'm';\nimport { a } from 'm';\n")).toBe(1);
  });

  it("still lets a type-only import stand beside a value import", () => {
    expect(findings("eslint-no-duplicate-imports", separate, "import type { A } from 'm';\nimport { a } from 'm';\n", "src/main.ts")).toBe(0);
  });
});
