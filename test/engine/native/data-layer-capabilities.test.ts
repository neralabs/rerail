import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/native/native.js";

const native = loadNativeBinding();
const capabilities = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const compile = (forbid: unknown, anchor = "witness.span") => native.CodeRuleDslRuntime.compile(JSON.stringify({
  format: "rerail.policy/v1", id: "port-capability", revision: 1, mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "check", forbid, violation: { code: "CHECK", anchor, message: "check" } }],
}), "json", capabilities);

it("scans independent Python splitlines with filters, first matches, and byte-correct captures", async () => {
  const rule = compile({ regex: { source: "file.content", pattern: "(?<WORD>needle)",
    paths: { include: ["impl/**"], exclude: [] }, include: ["need"], exclude: ["absent"],
    lines: { include: ["needle"], exclude: ["^#"], first: true } } }, "capture.WORD");
  const source = "# needle\né needle needle\r\nneedle\u2028needle\vneedle\n";
  const files = [{ path: "impl/a.py", source, language: "python" }, { path: "tests/a.py", source, language: "python" }];
  const stateless = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] }), "report"));
  expect(stateless.failures).toEqual([]);
  expect(stateless.violations.map((v: { span: { startLine: number; startColumn: number; startByte: number } }) => [v.span.startLine, v.span.startColumn, v.span.startByte]))
    .toEqual([[1, 3, 12], [2, 0, 27], [3, 0, 36], [4, 0, 43]]);
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const result = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [] })));
    expect(result.evaluation.violations).toEqual(stateless.violations);
    expect(result.evaluation.failures).toEqual([]);
  } finally { workspace.close(); }
});

it("gates on whole-file syntax and updates syntax findings after repair", async () => {
  const invalid = compile({ ast: { language: "python", syntax: "invalid", rule: { kind: "module" } } }, "file.start");
  const valid = compile({ ast: { language: "python", syntax: "valid", rule: { kind: "raise_statement" } } });
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(invalid);
    const files = [{ path: "a.py", source: "def broken(:\n", language: "python" }];
    const first = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [] })));
    expect(first.evaluation.failures).toEqual([]);
    expect(first.evaluation.violations).toHaveLength(1);
    expect(first.evaluation.violations[0].span.startByte).toBe(0);
    const snapshot = { format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] };
    expect(JSON.parse(valid.evaluate(JSON.stringify(snapshot), "report"))).toMatchObject({ failures: [], violations: [] });
    files[0]!.source = "raise ValueError()\n";
    const repaired = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 2, files, removed: [] })));
    expect(repaired.evaluation.failures).toEqual([]);
    expect(repaired.evaluation.violations).toEqual([]);
  } finally { workspace.close(); }
});

it("reads suppression text on the whole current or previous line, including the first line", () => {
  const rule = compile({ ast: { language: "python", syntax: "valid", rule: {
    kind: "raise_statement", not: { any: [{ layout: { line: "allow-marker" } }, { layout: { previousLine: "allow-marker" } }] },
  } } });
  const source = "raise ValueError()\n# allow-marker\nraise ValueError()\nraise ValueError() # allow-marker\nraise ValueError()\nraise ValueError()\n";
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: [{ path: "a.py", language: "python", source }] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((v: { span: { startLine: number } }) => v.span.startLine)).toEqual([0, 5]);
});


it("rejects unsupported regex and syntax options before evaluation", () => {
  expect(() => compile({ regex: { source: "file.path", pattern: "x", lines: {} } })).toThrow("requires file.content");
  expect(() => compile({ regex: { source: "file.content", pattern: "x", include: ["["] } })).toThrow();
  expect(() => compile({ regex: { source: "file.content", pattern: "x", lines: { first: 1 } } })).toThrow();
  expect(() => compile({ ast: { language: "python", syntax: "partial", rule: { kind: "module" } } })).toThrow("valid or invalid");
  expect(() => compile({ ast: { language: "python", rule: { kind: "identifier", layout: { line: 2 } } } })).toThrow("regex string");
});

it("keeps explicitly gated syntax evidence complete in cross-file selection", async () => {
  const rule = compile({ select_when: {
    select: { ast: { language: "python", syntax: "invalid", rule: { kind: "module" } } },
    exists: { regex: { source: "file.content", pattern: "broken" } },
  } }, "file.start");
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const result = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files: [{ path: "a.py", language: "python", source: "def broken(:\n" }], removed: [] })));
    expect(result.evaluation.failures).toEqual([]);
    expect(result.evaluation.violations).toHaveLength(1);
  } finally { workspace.close(); }
});
