import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
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
it("rejects unknown inputs and invalid values and still checks unreachable SQL", () => {
  for (const value of ["missing", "__hidden", "", 1]) {
    const p = policy(); Object.assign(p.clauses[0]!.relational, { requireNonemptyInput: value });
    expect(() => native.CodeRuleDslRuntime.compile(JSON.stringify(p), "json", capabilities)).toThrow();
  }
  const p = policy(); p.clauses[0]!.relational.query = "SELECT * FROM forbidden_table";
  expect(() => compile(p)).toThrow(/forbidden_table|no such table|not authorized/);
});
it("does not turn incomplete evidence into a successful empty result", () => {
  const result = evaluate("def (\n");
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
it.skipIf(process.env.RERAIL_REQUIRE_PRODUCER_SHAPE !== "1")("complete empty admission acquires no relational SQL connection", () => {
  const dir = mkdtempSync(join(tmpdir(), "rerail-input-admission-"));
  try {
    const program = `import {readFileSync} from 'node:fs';import {loadNativeBinding} from ${JSON.stringify(new URL("../../../../src/engine/index.ts", import.meta.url).href)};
      const n=loadNativeBinding(),p=${JSON.stringify(policy())};const r=n.CodeRuleDslRuntime.compile(JSON.stringify(p),'json',${JSON.stringify(capabilities)});
      const w=new n.CodeRuleDslWorkspace();w.addCodeRule(r);try {console.log(r.evaluate(JSON.stringify({format:'rerail.repository-snapshot/v1',revision:1,files:[{path:'a.py',language:'python',source:'pass\\n'}]}),'report')); console.log(await w.applyAsync(JSON.stringify({revision:1,files:[{path:'a.py',language:'python',source:'pass\\n'}],removed:[]})));}finally{w.close();}`;
    const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], { env: { ...process.env, RERAIL_SQL_PROFILE_DIR: dir }, encoding: "utf8", timeout: 20_000 });
    expect(child.error).toBeUndefined(); expect(child.status, child.stderr).toBe(0);
    let runtime = 0, connections = 0;
    for (const file of readdirSync(dir).filter(f => /^profile-.*\.json$/.test(f))) {
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
