import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import type { Engine } from "../../conformance/languages/checks/engine.js";
import { loadsGate, currentContract, loadAsBundled, type RuleDocument } from "../../conformance/languages/checks/loads.js";

const RAW = {
  format: "rerail.policy/v1",
  id: "eslint:no-eval",
  revision: 1,
  mode: "steer",
  clauses: [{ id: "c", define: { options: { callee: { type: "string", default: "foo" } } }, forbid: { ast: { language: "typescript", rule: { pattern: { $option: "callee", template: "{}()" } } } } }],
};
const RESOLVED = { ...RAW, clauses: [{ id: "c", forbid: { ast: { language: "typescript", rule: { pattern: "foo()" } } } }] };

function fakeEngine(inScope: (path: string) => boolean = () => true): Engine {
  return {
    rerail: {
      findDuplicateJsonKeys: () => [],
      codeRuleSchema: { safeParse: () => ({ success: true, data: RESOLVED }) },
      stableJsonText: (value: unknown) => JSON.stringify(value),
      loadPackRuleDocuments: () => [{ kind: "code", rule: { id: "eslint:no-eval" } }],
    },
    compile: () => ({ canonicalJson: JSON.stringify({ id: "eslint:no-eval" }), inScope }),
  };
}

const document = (path: string, text: string): RuleDocument => ({ file: path.slice(path.lastIndexOf("/") + 1), path, text, bytes: text.length });

test("loads hands later gates the document as the engine evaluates it, with options resolved", async () => {
  const scratch = await mkdtemp(join(tmpdir(), "validation-compile-"));
  try {
    const result = loadsGate(fakeEngine(), document("packs/javascript/rules/no-eval.json", JSON.stringify(RAW)), "javascript-eslint", scratch, ["src/example.ts"]);
    assert.deepEqual(result.checks.filter((check) => !check.ok), []);
    assert.deepEqual(result.document, RAW);
    assert.deepEqual(result.resolved, RESOLVED);
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
});

test("loads fails a rule that does not apply to a file type gates.yaml declares upstream lints", async () => {
  const scratch = await mkdtemp(join(tmpdir(), "validation-compile-"));
  try {
    const engine = fakeEngine((path) => !path.endsWith(".pyi"));
    const result = loadsGate(engine, document("packs/python/rules/no-eval.json", JSON.stringify(RAW)), "python", scratch, ["src/example.py", "src/example.pyi"]);
    const inScope = result.checks.find((check) => check.id === "in-scope")!;
    assert.equal(inScope.ok, false);
    assert.match(inScope.detail, /^does not apply to src\/example\.pyi: gates\.yaml declares upstream lints that file type \(upstream\.lints\)/u);
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
});

test("a document rerail's bundled-rule loader rejects is named by its path in the pack, not in a temporary directory", async () => {
  const scratch = await mkdtemp(join(tmpdir(), "validation-compile-"));
  try {
    const engine = fakeEngine();
    // rerail's loader names the rejected file by absolute path.
    engine.rerail.loadPackRuleDocuments = (packs: readonly string[]) => {
      throw new Error(`${join(packs[0]!, "rules", "assert-used.json")} is neither a rerail.policy/v1 document nor a conversation rule: m`);
    };
    assert.throws(() => loadAsBundled(engine, scratch, "python-bandit", [document("packs/python/rules/assert-used.json", "{}")]), {
      message: "python-bandit/rules/assert-used.json is neither a rerail.policy/v1 document nor a conversation rule: m",
    });
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
});

const violation = (severity?: string) => ({ anchor: "witness.span", code: "C", message: "m", ...(severity === undefined ? {} : { severity }) });

test("only a document written for the current rule contract is current", () => {
  const onMain = { format: "rerail.policy/v1", id: "bandit:b101", revision: 2, mode: "log", clauses: [{ id: "c", forbid: {}, violation: violation() }] };
  assert.deepEqual(currentContract(onMain), { id: "current-contract", ok: true, detail: "written for the current rule contract (rerail.policy/v1 with clauses, each with a violation, and a rule mode)" });
  const before = { format: "tempo.policy/v1", id: "bandit:b101", revision: 2, checks: [{ id: "c", forbid: {}, finding: violation("warning") }] };
  const result = currentContract(before);
  assert.equal(result.ok, false);
  assert.match(result.detail, /format "tempo\.policy\/v1", checks instead of clauses, no rule mode$/u);
  assert.equal(currentContract({ ...onMain, clauses: [{ id: "c", forbid: {}, violation: violation("warning") }] }).ok, false);
});

test("loads fails a document written for an earlier contract and compiles it as written", async () => {
  const scratch = await mkdtemp(join(tmpdir(), "validation-compile-"));
  try {
    const { clauses, mode: _mode, ...rest } = RAW;
    const text = JSON.stringify({ ...rest, format: "tempo.policy/v1", checks: [{ ...clauses[0]!, finding: violation("warning") }] });
    const compiledTexts: string[] = [];
    const engine = fakeEngine();
    const compile = engine.compile;
    engine.compile = (source: string) => {
      compiledTexts.push(source);
      return compile(source);
    };
    const result = loadsGate(engine, document("packs/javascript/candidates/rules/no-eval.json", text), "javascript-eslint", scratch, ["src/example.ts"]);
    assert.deepEqual(result.checks.filter((check) => !check.ok).map((check) => check.id), ["current-contract"]);
    assert.equal(compiledTexts[0], text);
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
});
