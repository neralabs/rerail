/**
 * n-no-path-concat (eslint-plugin-n n/no-path-concat) traces Node's `__dirname` and `__filename` with eslint-utils'
 * ReferenceTracker, which skips a global the file writes anywhere (isModifiedGlobal): after `__dirname = '.'` the name
 * no longer holds the module's directory, so `__dirname + '/x'` is not reported. A write to one name leaves the other
 * traced, and a write to a local variable of the same name leaves the global traced.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../../src/engine/native/native.js";
import { loadPackRuleDocuments } from "../../../../../src/engine/catalog/rule-documents.js";
import { repositoryPath } from "../../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;

const evaluationSchema = z.object({
  violations: z.array(z.object({ span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function compile(): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = loadPackRuleDocuments([repositoryPath("packs", "node")]).find(candidate => candidate.rule.id === "n-no-path-concat");
  if (document === undefined) throw new Error("n-no-path-concat is not in packs/node/rules");
  return runtime.compile(JSON.stringify(document.rule), "json", capabilities);
}

const rule = compile();

/** The 1-based lines the rule reports in `source`, a `.js` file of a CommonJS package, where Node defines `__dirname`. */
function reportedLines(source: string): number[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [
      { language: "json", path: "package.json", source: '{ "type": "commonjs" }\n' },
      { language: "typescript", path: "test/test1276.js", source },
    ],
  });
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.span.startLine + 1).sort((a, b) => a - b);
}

const js = (...lines: string[]) => [...lines, ""].join("\n");

describe("n-no-path-concat on a file that writes __dirname or __filename", () => {
  it.each([
    ["assigned before the read (AlaSQL test/test1276.js)", js("__dirname = '.';", "const csv = __dirname + '/test134.csv';")],
    ["assigned after the read", js("const csv = __dirname + '/test134.csv';", "__dirname = '.';")],
    ["assigned inside a function", js("function reset() { __dirname = '.'; }", "const csv = `${__dirname}/test134.csv`;")],
    ["augmented", js("__dirname += '/sub';", "const csv = __dirname + '/test134.csv';")],
    ["updated", js("__filename++;", "const f = __filename + '/x';")],
    ["destructured into", js("[__dirname] = ['.'];", "const csv = __dirname + '/test134.csv';")],
    ["destructured into by shorthand", js("({ __dirname } = config);", "const csv = __dirname + '/test134.csv';")],
    ["a for-in target", js("for (__dirname in roots) {}", "const csv = __dirname + '/test134.csv';")],
    ["a for-of target", js("for (__dirname of roots) {}", "const csv = __dirname + '/test134.csv';")],
  ])("stays silent when the global is %s", (_name, source) => {
    expect(reportedLines(source)).toEqual([]);
  });

  it("still reports a global the file only reads", () => {
    expect(reportedLines(js("const csv = __dirname + '/test134.csv';", "const tpl = `${__filename}/x`;"))).toEqual([1, 2]);
  });

  it("still reports the other name when only one is written", () => {
    expect(reportedLines(js("__dirname = '.';", "const a = __dirname + '/a';", "const b = __filename + '/b';"))).toEqual([3]);
  });

  it("still reports the global when the write is to a local of the same name", () => {
    expect(reportedLines(js("function f() { let __dirname; __dirname = '.'; return __dirname + '/a'; }", "const csv = __dirname + '/test134.csv';"))).toEqual([2]);
  });

  it("stays silent on a local declaration of the name, as before", () => {
    expect(reportedLines(js("let __dirname = '.';", "const csv = __dirname + '/test134.csv';"))).toEqual([]);
  });
});
