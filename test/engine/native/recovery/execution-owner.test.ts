import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../../src/engine/index.js";

const native = loadNativeBinding();
const capabilities = readFileSync("native/src/code_rule_dsl/capabilities.json", "utf8");
const rule = () => native.CodeRuleDslRuntime.compile(JSON.stringify({
  format: "rerail.policy/v1", id: "owner", revision: 1, mode: "log", clauses: [{
    id: "calls", forbid: { ast: { language: "python", syntax: "valid", rule: {
      all: [{ kind: "call" }, { has: { kind: "identifier", regex: "^target$", stopBy: "end" } }],
    } } }, violation: { code: "TARGET", message: "Target", anchor: "witness.span" },
  }],
}), "json", capabilities);
const file = (path: string, source: string) => ({ path, source, language: "python" });
const snapshot = (files: ReturnType<typeof file>[]) => JSON.stringify({
  format: "rerail.repository-snapshot/v1", revision: 1, files,
});

it("shares immutable facts while snapshots and workspaces keep separate membership and versions", async () => {
  const owner = new native.CodeRuleDslExecution();
  const compiled = rule();
  const a = owner.createWorkspace(), b = owner.createWorkspace();
  const results = async (workspace: typeof a) => JSON.parse(await workspace.resultPageAsync(0, 256 * 1024));
  try {
    a.addCodeRule(compiled); b.addCodeRule(compiled);
    const positive = file("same.py", "target()\n"), clean = file("same.py", "other()\n");
    const run = (files: ReturnType<typeof file>[]) => JSON.parse(owner.evaluate(compiled, snapshot(files), "report"));
    expect(run([positive])).toMatchObject({ failures: [], violations: [expect.anything()] });
    await a.applyAsync(JSON.stringify({ revision: 1, files: [positive], removed: [] }));
    await b.applyAsync(JSON.stringify({ revision: 1, files: [clean], removed: [] }));
    expect((await results(a)).violations).toHaveLength(1);
    expect((await results(b)).violations).toHaveLength(0);
    expect(run([clean])).toMatchObject({ failures: [], violations: [] });
    expect(run([])).toMatchObject({ failures: [], violations: [] });
    await b.applyAsync(JSON.stringify({ revision: 2, files: [], removed: ["same.py"] }));
    expect((await results(a)).violations).toHaveLength(1);
    expect(run([positive]).violations).toHaveLength(1);
  } finally { a.close(); b.close(); owner.close(); }
  expect(() => owner.createWorkspace()).toThrow(/closed/);
});

it.skipIf(process.env.RERAIL_REQUIRE_PRODUCER_SHAPE !== "1")("constructs a shared SQL producer once per source version while executing each consumer", async () => {
  const { spawnSync } = await import("node:child_process");
  const { mkdtempSync, readdirSync, rmSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const directory = mkdtempSync(join(tmpdir(), "rerail-shared-facts-"));
  const engine = new URL("../../../../src/engine/index.ts", import.meta.url).href;
  const caps = new URL("../../../../native/src/code_rule_dsl/capabilities.json", import.meta.url).href;
  try {
    const program = `
      import {readFileSync} from 'node:fs';
      import {loadNativeBinding} from ${JSON.stringify(engine)};
      const n=loadNativeBinding(), owner=new n.CodeRuleDslExecution();
      const compile=(id,filter,kind="function_definition",excludeKinds=[])=>n.CodeRuleDslRuntime.compile(JSON.stringify({format:'rerail.policy/v1',id,revision:1,mode:'log',clauses:[{
        id:'test', relational:{inputs:{gate:{evidence:{as:'g',query:{ast:{language:'python',rule:{kind:'module'}}},project:{path:'file.path'}}}},
          syntax:{name:'nodes',gate:'gate',excludeKinds},stages:[
            {name:'common',query:"SELECT __path,id,__start,__end FROM nodes WHERE kind='"+kind+"'"},
            {name:'descendants',reachability:{syntax:'nodes',roots:'SELECT __path,id FROM common'}}
          ],query:"SELECT f.__path path,f.__start start_byte,f.__end end_byte FROM common f JOIN descendants d ON d.__path=f.__path AND d.root=f.id AND d.node=f.id WHERE "+filter},
        violation:{code:'FOUND',message:'Found',anchor:'witness.span'}}]}),'json',readFileSync(new URL(${JSON.stringify(caps)}),'utf8'));
      const a=compile('all','1'),b=compile('filtered','f.id < 0'),c=compile('different-definition','1','class_definition'),d=compile('different-source','1','function_definition',['function_definition']);
      const run=(r,source)=>JSON.parse(owner.evaluate(r,JSON.stringify({format:'rerail.repository-snapshot/v1',revision:1,files:[{path:'a.py',language:'python',source}]}),'report'));
      try {console.log(JSON.stringify([run(a,'def a(): pass\\n'),run(b,'def a(): pass\\n'),run(a,'def a(): pass\\ndef b(): pass\\n'),run(b,'def a(): pass\\ndef b(): pass\\n'),run(a,'def a(): pass\\n'),run(c,'def a(): pass\\n'),run(d,'def a(): pass\\n')]));}finally{owner.close();}
    `;
    const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
      encoding: "utf8", timeout: 30_000, maxBuffer: 16 * 1024 * 1024,
      env: { ...process.env, RERAIL_SQL_PROFILE_DIR: directory },
    });
    expect(child.error).toBeUndefined(); expect(child.status, child.stderr).toBe(0);
    const results = JSON.parse(child.stdout);
    expect(results.map((r: { failures: unknown[] }) => r.failures)).toEqual([[], [], [], [], [], [], []]);
    expect(results.map((r: { violations: unknown[] }) => r.violations.length)).toEqual([1, 0, 2, 0, 1, 0, 0]);
    let builds = 0;
    for (const file of readdirSync(directory).filter(f => /^profile-.*\.json$/.test(f))) {
      const catalogue = JSON.parse(readFileSync(join(directory, file), "utf8"));
      const queries = new Map<number, string>(catalogue.queries.map((q: { id: number; sql: string }) => [q.id, q.sql]));
      const events = readFileSync(join(directory, file.replace(/\.json$/, ".jsonl")), "utf8").split("\n").filter(Boolean).map(l => JSON.parse(l));
      for (const e of events.filter(e => e.kind === "sql" && e.origin === "engine")) {
        const sql = queries.get(e.query) ?? "";
        if (/^INSERT INTO/.test(sql) && /kind='(?:function_definition|class_definition)'/.test(sql)) builds++;
      }
    }
    expect(builds, "two source versions share across consumers; different definitions and source filters each construct separately").toBe(4);
  } finally { rmSync(directory, { recursive: true, force: true }); }
}, 35_000);

it("uses document versions when workspaces release source strings before shared SQL preparation", async () => {
  const { codeRuleSchema } = await import("../../../../src/engine/index.js");
  const policy = JSON.parse(readFileSync("packs/data-layer/candidates/rules/data-layer-internal-dead-code.json", "utf8"));
  policy.scope = { include: ["**/*.py"], exclude: [] };
  policy.clauses[0].define.config = { oracleAnalyzer: true };
  const compiled = native.CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(policy)), "json", capabilities);
  const owner = new native.CodeRuleDslExecution();
  try {
    for (const exported of [false, true, false]) {
      const files = [file("impl/common/query.py", "def helper(x): return x\n"),
        file("data_layer/__init__.py", exported ? "from impl.common.query import helper\n" : "")];
      const expected = exported ? 0 : 1;
      const fresh = JSON.parse(owner.evaluate(compiled, snapshot(files), "report"));
      expect(fresh.failures).toEqual([]); expect(fresh.violations).toHaveLength(expected);
      const workspace = owner.createWorkspace();
      try {
        workspace.addCodeRule(compiled);
        const result = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [] })));
        expect(result.evaluation.failures).toEqual([]);
        expect(result.evaluation.violations).toHaveLength(expected);
      } finally { workspace.close(); }
    }
  } finally { owner.close(); }
});
