import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../src/engine/index.js";

const native = loadNativeBinding();
const caps = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const compile = (policy: unknown) => native.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", caps);
function document(deferred = true) {
  const stages: Record<string, unknown>[] = [
    { name: "nodes", query: "SELECT __path,__start,__end,kind FROM syntax", ...(deferred ? { deferred: { text: "source_text(__path,__start,__end)" } } : {}) },
    { name: "selected", query: "SELECT * FROM nodes WHERE kind='identifier'", indexes: [["__path", "__start"]], ...(deferred ? { deferred: { text: "source_text(__path,__start,__end)" } } : {}) },
    { name: "twice", query: "SELECT a.*,b.text AS other FROM selected a JOIN selected b ON b.__path=a.__path AND b.__start=a.__start" },
  ];
  if (!deferred) stages[0]!.query = "SELECT * FROM syntax";
  return { format: "rerail.policy/v1", id: "deferred", revision: 1, mode: "log", clauses: [{ id: "text", relational: {
    partitionBy: "file", inputs: { syntax: { evidence: { as: "n", query: { ast: { language: "python", rule: { pattern: "$NODE" } } }, project: { kind: "capture.NODE.kind", ...(!deferred ? { text: "capture.NODE" } : {}) } } } }, stages,
    query: "SELECT __path AS path,__start AS start_byte,__end AS end_byte,text,other FROM twice", key: ["text"],
  }, violation: { code: "TEXT", message: "Text", anchor: "witness.span" } }] };
}
const files = [{ path: "a.py", source: "π = '☃'\nanswer = π\n", language: "python" }, { path: "b.py", source: "answer = 2\n", language: "python" }];
const evaluate = (rule: ReturnType<typeof compile>, current = files) => {
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: current }), "report"));
  return { failures: result.failures, violations: result.violations };
};

it("matches eager captures across deferred views, indexed joins, Unicode, and file partitions", () => {
  const policy = document();
  expect(codeRuleSchema.safeParse(policy).success).toBe(true);
  const actual = evaluate(compile(policy));
  expect(actual.failures).toEqual([]);
  expect(actual.violations).toEqual(evaluate(compile(document(false))).violations);
  expect(actual.violations.some((v: { variables: { row: { text: string } } }) => v.variables.row.text === "π")).toBe(true);
});

it("returns NULL for missing sources and invalid byte slices", () => {
  const policy = document();
  policy.clauses[0]!.relational.query = "SELECT __path AS path,__start AS start_byte,__end AS end_byte,source_text('missing.py',0,1) AS missing,source_text(__path,-1,1) AS negative,source_text(__path,1,2) AS split,source_text(__path,3,2) AS reversed FROM syntax WHERE __path='a.py' AND kind='module'";
  policy.clauses[0]!.relational.key = ["missing"];
  const actual = evaluate(compile(policy));
  expect(actual.failures).toEqual([]);
  expect(actual.violations.length).toBeGreaterThan(0);
  expect(actual.violations[0].variables.row).toMatchObject({ missing: null, negative: null, split: null, reversed: null });
});

it("keeps source visibility within the rule scope in repository snapshots and workspaces", async () => {
  const policy = document();
  Object.assign(policy, { scope: { include: ["a.py"], exclude: [] } });
  policy.clauses[0]!.relational.partitionBy = "repository";
  policy.clauses[0]!.relational.query = "SELECT __path AS path,__start AS start_byte,__end AS end_byte,source_text('b.py',0,6) AS text FROM syntax WHERE kind='module'";
  const rule = compile(policy), expected = evaluate(rule);
  expect(expected.failures).toEqual([]);
  expect(expected.violations).toHaveLength(1);
  expect(expected.violations[0].variables.row.text).toBeNull();
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    const actual = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [] }))).evaluation;
    expect(actual).toMatchObject(expected);
  } finally { workspace.close(); }
});

it.each([
  { deferred: { text: "text" } },
  { deferred: { text: "random()" } },
  { deferred: { text: "(SELECT sql FROM sqlite_master)" } },
  { deferred: { text: "1) AS text FROM __stage_nodes; SELECT (1" } },
  { deferred: { text: "source_text(__path,__start,__end)" }, indexes: [["text"]] },
])("rejects deferred self-dependencies, unavailable functions, hidden tables and lazy indexes: %j", shape => {
  const policy = document();
  Object.assign(policy.clauses[0]!.relational.stages[0]!, shape);
  expect(() => compile(policy)).toThrow();
});

it("recomputes lazy source values after same-length workspace edits and removals", async () => {
  const rule = compile(document()), workspace = new native.CodeRuleDslWorkspace();
  workspace.addCodeRule(rule);
  try {
    const apply = async (revision: number, current: typeof files, removed: string[] = []) => JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: current, removed }))).evaluation;
    expect(await apply(1, files)).toMatchObject(evaluate(rule));
    const edited = { ...files[0]!, source: files[0]!.source.replace("answer", "result") };
    expect(await apply(2, [edited])).toMatchObject(evaluate(rule, [edited, files[1]!]));
    expect(await apply(3, [files[0]!])).toMatchObject(evaluate(rule));
    expect(await apply(4, [], ["a.py"])).toMatchObject(evaluate(rule, [files[1]!]));
  } finally { workspace.close(); }
});

it.each([false, true])("invalidates source reads without evidence, including eviction (stored=%s)", async stored => {
  const policy = document();
  policy.clauses[0]!.relational.partitionBy = "repository";
  policy.clauses[0]!.relational.inputs.syntax.evidence.query.ast.rule = { pattern: "$NODE", ...{ kind: "function_definition" } };
  policy.clauses[0]!.relational.query = "SELECT __path AS path,__start AS start_byte,__end AS end_byte,source_text('settings.py',0,4) AS text FROM syntax";
  const initial = [{ path: "a.py", source: "def read(): pass\n", language: "python" }, { path: "settings.py", source: "pass\n", language: "python" }];
  const rule = compile(policy), workspace = new native.CodeRuleDslWorkspace();
  let root: string | undefined;
  try {
    if (stored) {
      root = mkdtempSync(join(tmpdir(), "rerail-deferred-"));
      for (const file of initial) writeFileSync(join(root, file.path), file.source);
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "a.py", "settings.py"], { cwd: root });
      execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "source fixture"], { cwd: root });
      await workspace.enableStorageAsync(root, join(root, "results.sqlite"));
      workspace.setDocumentResidencyLimit(0);
    }
    workspace.addCodeRule(rule);
    await workspace.applyAsync(JSON.stringify({ revision: 1, files: initial, removed: [] }));
    if (stored) {
      await workspace.completeStorageRevisionAsync();
      expect(workspace.evictInactive()).toBe(true);
    }
    const edited = { ...initial[1]!, source: "None\n" };
    const actual = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 2, files: [edited], removed: [] }))).evaluation;
    expect(actual).toMatchObject(evaluate(rule, [initial[0]!, edited]));
    expect(actual.violations[0].variables.row.text).toBe("None");
    const removed = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 3, files: [], removed: ["settings.py"] }))).evaluation;
    expect(removed).toMatchObject(evaluate(rule, [initial[0]!]));
    expect(removed.violations[0].variables.row.text).toBeNull();
  } finally { workspace.close(); if (root) rmSync(root, { recursive: true, force: true }); }
});
