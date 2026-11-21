// scripts/ship/waits-steps.ts: the step benchmarks' edits, made the same way on both sides. Python gets the step
// benchmarks' own edits; TypeScript the same edits in its syntax; the one-rule shape turns every other rule off; the
// rule step changes a rule's document and nothing it finds.

import { describe, expect, it } from "vitest";
import { editRuleDocument, editSource, oneRulePack, pickStepFile } from "../../../scripts/ship/waits-steps.js";

const python = "import os\n\n\nclass A:\n    def run(self, x):\n        return x\n";
const typescript = 'import { join } from "node:path";\n\nexport function run(x: number): number {\n  return x;\n}\n';

describe("file edits", () => {
  it("makes the step benchmarks' Python edits", () => {
    expect(editSource("append", python, "a.py")).toBe(`${python}\n\ndef _t31_added(x):\n    assert x, "edited"\n    return x\n`);
    expect(editSource("body", python, "a.py")).toContain("    def run(self, x):\n        _t31_local = 0\n        return x\n");
    expect(editSource("comment", python, "a.py")).toContain("\n    # t31: a comment\n    def run(self, x):\n");
    expect(editSource("import", python, "a.py").startsWith("import json as _t31_json\nimport os\n")).toBe(true);
  });

  it("makes the same edits in TypeScript", () => {
    expect(editSource("body", typescript, "a.ts")).toContain("export function run(x: number): number {\n  const _t31Local = 0;\n  return x;\n");
    expect(editSource("comment", typescript, "a.ts")).toContain("\n// t31: a comment\nexport function run(");
    expect(editSource("import", typescript, "a.ts").startsWith('import * as _t31Path from "node:path";\n')).toBe(true);
    expect(editSource("append", typescript, "a.ts")).toMatch(/export function _t31Added\(x: any\): any \{\n {2}if \(!x\) throw new Error\("edited"\);\n {2}return x;\n\}\n$/);
  });

  it("refuses a file the body edit cannot place", () => {
    expect(() => editSource("body", "export function run(\n  x: number,\n): number {\n  return x;\n}\n", "a.ts")).toThrow(/does not open its body/);
    expect(() => editSource("comment", "export const x = 1;\n", "a.ts")).toThrow(/no function/);
  });

  it("picks the first file in path order over 4 KiB that takes every edit", () => {
    const big = (source: string): string => source + `// ${"x".repeat(5000)}\n`;
    const files = [
      { path: "b.ts", size: 5000, source: () => big(typescript) },
      // Its first function's signature spans lines: the body edit cannot place its statement.
      { path: "a.ts", size: 5000, source: () => big("export function run(\n  x: number,\n): number {\n  return x;\n}\n") },
      { path: "0.ts", size: 100, source: () => typescript },
    ];
    expect(pickStepFile(files)).toBe("b.ts");
  });
});

describe("rule edits", () => {
  it("turns every rule but one off", () => {
    const pack = JSON.parse(oneRulePack(JSON.stringify({ id: "p", rules: [{ id: "a", on: true }, { id: "b", on: false }, { id: "c" }] }), "b")) as { rules: { id: string; on: boolean }[] };
    expect(pack.rules.map((r) => [r.id, r.on])).toEqual([["a", false], ["b", true], ["c", false]]);
    expect(() => oneRulePack(JSON.stringify({ rules: [] }), "b")).toThrow(/no rule b/);
  });

  it("changes only the rule's description", () => {
    const before = { id: "r", description: "Avoid any.", clauses: [{ id: "c" }] };
    expect(JSON.parse(editRuleDocument(JSON.stringify(before)))).toEqual({ ...before, description: "Avoid any. (edited)" });
  });
});
