import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/native/native.js";

const native = loadNativeBinding();
const capabilities = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const compile = (forbid: unknown) => native.CodeRuleDslRuntime.compile(JSON.stringify({
  format: "rerail.policy/v1", id: "line-state", revision: 1, mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "check", forbid, violation: { code: "CHECK", anchor: "witness.span", message: "check" } }],
}), "json", capabilities);
const state = { initial: "code", emit: ["code"], transitions: [
  { from: "code", to: "hidden", pattern: "MARK", count: { parity: "odd" }, skip: true },
  { from: "hidden", to: "code", pattern: "MARK", count: { parity: "odd" }, skip: true },
  { from: "code", to: "code", pattern: "MARK", skip: true },
] };
const regex = (lines: unknown) => ({ regex: { source: "file.content", pattern: "needle", lines } });

it("executes ordered line state on invalid syntax and recomputes each file after edits", async () => {
  const rule = compile(regex({ state, first: true }));
  const files = [
    { path: "a.py", language: "python", source: "MARK MARK\nneedle\nMARK\nneedle ( invalid\n" },
    { path: "b.py", language: "python", source: "é needle\n" },
  ];
  const input = { format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files };
  const original = JSON.parse(rule.evaluate(JSON.stringify(input), "report"));
  expect(original.failures).toEqual([]);
  expect(original.violations.map((v: { path: string; span: { startLine: number; startByte: number } }) => [v.path, v.span.startLine, v.span.startByte]))
    .toEqual([["a.py", 1, 10], ["b.py", 0, 3]]);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const first = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [] })));
    expect(first.evaluation.violations).toEqual(original.violations);
    files[0]!.source = "MARK\nneedle\nMARK\nneedle\n";
    const changed = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 2, files: [files[0]], removed: [] })));
    expect(changed.evaluation.failures).toEqual([]);
    expect(changed.evaluation.violations.map((v: { path: string; span: { startLine: number } }) => [v.path, v.span.startLine]))
      .toEqual([["a.py", 3], ["b.py", 0]]);
    const deleted = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 3, files: [], removed: ["a.py"] })));
    expect(deleted.evaluation.violations.map((v: { path: string }) => v.path)).toEqual(["b.py"]);
  } finally { workspace.close(); }
});

it("updates state before excluded lines and suppresses indentation until its reset", () => {
  const rule = compile(regex({ first: true, exclude: ["^#"], state,
    indentation: { excludeUnder: "TYPES", reset: "^\\S", ignore: ["^\\s*#", "^\\s*$"] } }));
  const source = "# MARK\nneedle\n# MARK\nTYPES\n  needle\n# unrelated\nroot\n  needle\n";
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: [{ path: "a.py", language: "python", source }] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((v: { span: { startLine: number } }) => v.span.startLine)).toEqual([7]);
});

it("supports LF-only scanning separately from Python splitlines", () => {
  const source = "needle\vneedle\nneedle\n";
  const evaluate = (terminators: string) => {
    const rule = compile(regex({ first: true, terminators }));
    return JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: [{ path: "a.py", language: "python", source }] }), "report"));
  };
  expect(evaluate("python").violations).toHaveLength(3);
  expect(evaluate("lf").violations).toHaveLength(2);
});

it("gates each file on its own complete content and updates that gate after an edit", async () => {
  const rule = compile({ regex: { source: "file.content", pattern: "needle", content: { include: ["TRIGGER"] }, lines: { first: true } } });
  const files = [{ path: "a.py", language: "python", source: "needle\n" }, { path: "b.py", language: "python", source: "TRIGGER\nneedle\n" }];
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const original = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files }), "report"));
    const first = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [] })));
    expect(first.evaluation.violations).toEqual(original.violations);
    expect(original.violations.map((v: { path: string }) => v.path)).toEqual(["b.py"]);
    files[0]!.source = "# TRIGGER\nneedle\n";
    const changed = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 2, files: [files[0]], removed: ["b.py"] })));
    expect(changed.evaluation.failures).toEqual([]);
    expect(changed.evaluation.violations.map((v: { path: string }) => v.path)).toEqual(["a.py"]);
  } finally { workspace.close(); }
});

it("bounds an AST line window and ends it at the first matching line inclusively", () => {
  const rule = compile({ ast: { language: "python", syntax: "valid", rule: { kind: "function_definition",
    layout: { context: { after: 3, through: "\\):", regex: "column: str" } } } } });
  const source = "def clean():\n    column: str = 'body'\n\ndef bad(\n    column: str\n):\n    pass\n";
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: [{ path: "a.py", language: "python", source }] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((v: { span: { startLine: number } }) => v.span.startLine)).toEqual([3]);
});

it("rejects invalid state, count, indentation and window declarations before evaluation", () => {
  expect(() => compile(regex({ state: { ...state, emit: ["unknown"] } }))).toThrow("unknown state");
  expect(() => compile(regex({ state: { ...state, transitions: [{ from: "code", to: "code", pattern: "[" }] } }))).toThrow();
  expect(() => compile(regex({ state: { ...state, transitions: [{ from: "code", to: "code", pattern: "x", count: { min: 3, max: 1 } }] } }))).toThrow("min exceeds max");
  expect(() => compile(regex({ indentation: { excludeUnder: "x", reset: "x", ignore: "x" } }))).toThrow("must be an array");
  expect(() => compile(regex({ terminators: "other" }))).toThrow("python or lf");
  expect(() => compile({ ast: { language: "python", rule: { kind: "module", layout: { context: { after: 65, regex: "." } } } } })).toThrow("0 to 64");
});
