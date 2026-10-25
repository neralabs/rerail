import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync, execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";
import { loadNativeBinding, codeRuleSchema } from "../../../../src/engine/index.js";
const native = loadNativeBinding();
const capabilities = readFileSync(new URL("../../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
function policy(guarded = true, partitionBy: "file" | "repository" = "repository") {
  return { format: "rerail.policy/v1", id: "admission", revision: 1, mode: "log", clauses: [{ id: "guard", relational: {
    ...(guarded ? { requireNonemptyInput: "gate" } : {}), partitionBy,
    ...(partitionBy === "repository" ? { locations: ["a.py"] } : {}),
    inputs: { gate: { evidence: { as: "g", query: { ast: { language: "python", syntax: "valid", rule: { all: [{ kind: "identifier" }, { regex: "^admit$" }] } } }, project: { path: "file.path" } } } },
    query: partitionBy === "repository" ? "SELECT 'a.py' path,0 start_byte,0 end_byte,count(*) AS count FROM gate" : "SELECT __path path,__start start_byte,__end end_byte FROM gate",
  }, violation: { code: "FOUND", message: "Found", anchor: "witness.span" } }] };
}
function compile(p = policy()) { return native.CodeRuleDslRuntime.compile(JSON.stringify(p), "json", capabilities); }
function evaluate(source: string, guarded = true) {
  return JSON.parse(compile(policy(guarded)).evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 1, files: [{ path: "a.py", language: "python", source }] }), "report"));
}
it("skips only an explicit complete empty precondition, preserving unguarded aggregates", () => {
  expect(codeRuleSchema.parse(policy()).clauses[0]!.relational).toHaveProperty("requireNonemptyInput", "gate");
  expect(evaluate("pass\n", false).violations).toHaveLength(1);
  expect(evaluate("pass\n")).toMatchObject({ violations: [], failures: [] });
  const nonempty = evaluate("admit = 1\n");
  expect(nonempty.failures).toEqual([]); expect(nonempty.violations).toHaveLength(1);
  expect(nonempty.violations[0].variables.row.count).toBe(1);
});
it("transfers nonempty projected rows without losing source values or multiplicity", () => {
  const p = policy();
  Object.assign(p.clauses[0]!.relational.inputs.gate.evidence.query.ast.rule.all[0]!, { pattern: "$NAME" });
  Object.assign(p.clauses[0]!.relational.inputs.gate.evidence.project, { name: "capture.NAME" });
  p.clauses[0]!.relational.query = "SELECT __path path,__start start_byte,__end end_byte,name FROM gate";
  const result = JSON.parse(compile(p).evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 1, files: [{ path: "a.py", language: "python", source: "admit; admit()\n" }] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((v: { variables: { row: { name: string } } }) => v.variables.row.name)).toEqual(["admit", "admit"]);
  Reflect.deleteProperty(p.clauses[0]!.relational, "requireNonemptyInput");
  const control = JSON.parse(compile(p).evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 1, files: [{ path: "a.py", language: "python", source: "admit; admit()\n" }] }), "report"));
  // Indexed evidence can use zero native node visits. Admission must not add a root walk.
  expect(result.metrics.visitedNodes).toBe(control.metrics.visitedNodes);
});
it("rejects unknown inputs and invalid values and still checks unreachable SQL", () => {
  for (const value of ["missing", "__hidden", "", 1]) {
    const p = policy(); Object.assign(p.clauses[0]!.relational, { requireNonemptyInput: value });
    expect(() => native.CodeRuleDslRuntime.compile(JSON.stringify(p), "json", capabilities)).toThrow();
  }
  const p = policy(); p.clauses[0]!.relational.query = "SELECT * FROM forbidden_table";
  expect(() => compile(p)).toThrow(/forbidden_table|no such table|not authorized/);
});
it("does not turn incomplete evidence into a successful empty result", () => {
  const p = policy();
  // Explicit syntax:valid selects only valid files. Omit it to exercise unknown evidence.
  Reflect.deleteProperty(p.clauses[0]!.relational.inputs.gate.evidence.query.ast, "syntax");
  const result = JSON.parse(compile(p).evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 1, files: [{ path: "a.py", language: "python", source: "def (\n" }] }), "report"));
  expect(result.violations).toEqual([]); expect(result.failures.length).toBeGreaterThan(0);
});
it("invalidates empty results across edits and removals in both partition scopes", async () => {
  for (const scope of ["file", "repository"] as const) {
    const ws = new native.CodeRuleDslWorkspace(); ws.addCodeRule(compile(policy(true, scope)));
    try {
      for (const [revision, source, expected] of [[1, "pass\n", 0], [2, "admit = 1\n", 1], [3, "pass\n", 0]] as const) {
        const result = JSON.parse(await ws.applyAsync(JSON.stringify({ revision, files: [{ path: "a.py", language: "python", source }], removed: [] }))).evaluation;
        expect(result.failures).toEqual([]); expect(result.violations).toHaveLength(expected);
        const first = await ws.resultPageAsync(0, 1_000_000); expect(await ws.resultPageAsync(0, 1_000_000)).toBe(first);
      }
      const result = JSON.parse(await ws.applyAsync(JSON.stringify({ revision: 4, files: [], removed: ["a.py"] }))).evaluation;
      expect(result.failures).toEqual([]); expect(result.violations).toEqual([]);
    } finally { ws.close(); }
  }
});
it("keeps repository gates complete across applies and restores only eligible evicted documents", async () => {
  const root = mkdtempSync(join(tmpdir(), "rerail-gate-repository-"));
  writeFileSync(join(root, "a.py"), "admit = 1\n"); writeFileSync(join(root, "b.py"), "pass\n");
  execFileSync("git", ["init", "--quiet"], { cwd: root }); execFileSync("git", ["add", "a.py", "b.py"], { cwd: root });
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "gate fixture"], { cwd: root });
  const p = policy();
  Object.assign(p.clauses[0]!.relational, { syntax: { name: "nodes", gate: "gate" } });
  p.clauses[0]!.relational.query = "SELECT __path path,__start start_byte,__end end_byte,(SELECT count(DISTINCT __path) FROM nodes) AS files FROM nodes WHERE kind='module'";
  const ws = new native.CodeRuleDslWorkspace();
  try {
    await ws.enableStorageAsync(root, join(root, "results.sqlite")); ws.setDocumentResidencyLimit(0); ws.addCodeRule(compile(p));
    const apply = async (revision: number, path: string, source: string) => {
      const changed = JSON.parse(await ws.applyAsync(JSON.stringify({ revision, files: [{ path, language: "python", source }], removed: [] }))).evaluation;
      expect(changed.failures).toEqual([]);
      return JSON.parse(await ws.resultPageAsync(0, 1_000_000));
    };
    const first = await apply(1, "a.py", "admit = 1\n"); expect(first.failures).toEqual([]); expect(first.violations).toHaveLength(1);
    await ws.completeStorageRevisionAsync(); expect(ws.evictInactive()).toBe(true);
    const second = await apply(2, "b.py", "pass\n"); expect(second.failures).toEqual([]); expect(second.violations).toHaveLength(1);
    await ws.completeStorageRevisionAsync(); expect(ws.evictInactive()).toBe(true);
    const third = await apply(3, "b.py", "admit = 2\n"); expect(third.failures).toEqual([]); expect(third.violations).toHaveLength(2);
    expect(third.violations.map((v: { variables: { row: { files: number } } }) => v.variables.row.files)).toEqual([2, 2]);
  } finally { ws.close(); rmSync(root, { recursive: true, force: true }); }
});
it.skipIf(process.env.RERAIL_REQUIRE_PRODUCER_SHAPE !== "1")("imports prepared positive gate rows without another matcher stream", () => {
  const program = `import {loadNativeBinding} from ${JSON.stringify(new URL("../../../../src/engine/index.ts", import.meta.url).href)};
    const n=loadNativeBinding();for(const [name,p] of [['guarded',${JSON.stringify(policy())}],['plain',${JSON.stringify(policy(false))}]]) {
      p.id=name; const r=n.CodeRuleDslRuntime.compile(JSON.stringify(p),'json',${JSON.stringify(capabilities)});
      console.log(r.evaluate(JSON.stringify({format:'rerail.repository-snapshot/v1',revision:1,files:[{path:'a.py',language:'python',source:'admit = 1\\n'}]}),'report'));
    }`;
  const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], { env: { ...process.env, RERAIL_PROFILE: "1" }, encoding: "utf8", timeout: 20_000 });
  expect(child.error).toBeUndefined(); expect(child.status, child.stderr).toBe(0);
  const results = child.stdout.trim().split("\n").map(line => JSON.parse(line));
  expect(results.map(r => [r.violations.length, r.failures.length])).toEqual([[1, 0], [1, 0]]);
  const trace = child.stderr.split("\n").filter(line => line.startsWith("RERAIL_TRACE ")).map(line => JSON.parse(line.slice(13)));
  expect(trace.filter(t => t.phase === "input-stream" && t.name === "gate")).toHaveLength(1);
  expect(trace.filter(t => t.phase === "input-admission" && t.detail.admitted)).toHaveLength(1);
});
it.skipIf(process.env.RERAIL_REQUIRE_PRODUCER_SHAPE !== "1")("complete empty admission acquires no relational SQL connection", () => {
  const dir = mkdtempSync(join(tmpdir(), "rerail-input-admission-"));
  try {
    const program = `import {readFileSync} from 'node:fs';import {loadNativeBinding} from ${JSON.stringify(new URL("../../../../src/engine/index.ts", import.meta.url).href)};
      const n=loadNativeBinding(),p=${JSON.stringify(policy())};
      p.clauses[0].relational.syntax={name:'nodes',gate:'gate'};
      p.clauses[0].relational.stages=[{name:'descendants',reachability:{syntax:'nodes',roots:'SELECT __path,id FROM nodes'}}];
      const r=n.CodeRuleDslRuntime.compile(JSON.stringify(p),'json',${JSON.stringify(capabilities)});
      const w=new n.CodeRuleDslWorkspace();w.addCodeRule(r);try {console.log(r.evaluate(JSON.stringify({format:'rerail.repository-snapshot/v1',revision:1,files:[{path:'a.py',language:'python',source:'pass\\n'}]}),'report')); console.log(await w.applyAsync(JSON.stringify({revision:1,files:[{path:'a.py',language:'python',source:'pass\\n'}],removed:[]})));}finally{w.close();}`;
    const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], { env: { ...process.env, RERAIL_SQL_PROFILE_DIR: dir, RERAIL_PROFILE: "1" }, encoding: "utf8", timeout: 20_000 });
    expect(child.error).toBeUndefined(); expect(child.status, child.stderr).toBe(0);
    const traces = child.stderr.split("\n").filter(line => line.startsWith("RERAIL_TRACE ")).map(line => JSON.parse(line.slice(13)));
    expect(traces.filter(t => ["syntax-prepare", "producer-wave"].includes(t.phase))).toEqual([]);
    let runtime = 0, connections = 0;
    const files = readdirSync(dir).filter(f => /^profile-.*\.json$/.test(f)); expect(files.length).toBeGreaterThan(0);
    for (const file of files) {
      const catalog = JSON.parse(readFileSync(join(dir, file), "utf8"));
      const queries = new Map<number, string>(catalog.queries.map((q: { id: number; sql: string }) => [q.id, q.sql]));
      for (const line of readFileSync(join(dir, file.replace(/\.json$/, ".jsonl")), "utf8").split("\n").filter(Boolean)) {
        const e = JSON.parse(line); if (e.kind !== "sql" || e.origin !== "engine") continue;
        runtime += Number(e.context?.scope === "relational"); connections += Number(queries.get(e.query)?.includes("PRAGMA analysis_limit=1000"));
      }
    }
    expect([runtime, connections]).toEqual([0, 0]);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
