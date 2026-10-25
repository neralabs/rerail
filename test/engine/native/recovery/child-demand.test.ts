import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";

const required = process.env.RERAIL_REQUIRE_PRODUCER_SHAPE === "1";
const engine = new URL("../../../../src/engine/index.ts", import.meta.url).href;
const capabilities = new URL("../../../../native/src/code_rule_dsl/capabilities.json", import.meta.url).href;

it.skipIf(!required)("batches distinct child-field bindings across two rules and result packets", () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-child-demand-"));
  try {
    const program = `
      import {readFileSync} from 'node:fs';
      import {loadNativeBinding} from ${JSON.stringify(engine)};
      const native=loadNativeBinding(),caps=readFileSync(new URL(${JSON.stringify(capabilities)}),'utf8');
      const ws=new native.CodeRuleDslWorkspace();
      for(const [id,field,kind] of [['callee','function','identifier'],['args','arguments','argument_list']]) {
        const clauses=Array.from({length:65},(_,i)=>({id:'selected'+i,
          forbid:{ast:{language:'python',rule:{kind:'call',has:{field,kind}}}},
          violation:{code:'MATCH',message:'Match',anchor:'witness.span'}}));
        ws.addCodeRule(native.CodeRuleDslRuntime.compile(JSON.stringify({format:'rerail.policy/v1',id,revision:1,mode:'log',clauses}),'json',caps));
      }
      try {console.log(await ws.applyAsync(JSON.stringify({revision:1,removed:[],files:[{path:'a.py',language:'python',source:'target(value)\\n'}]})));}
      finally {ws.close();}
    `;
    const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
      encoding: "utf8", timeout: 30_000, maxBuffer: 8 * 1024 * 1024,
      env: { ...process.env, RERAIL_SQL_PROFILE_DIR: directory },
    });
    expect(child.error).toBeUndefined(); expect(child.status, child.stderr).toBe(0);
    const result = JSON.parse(child.stdout.trim()).evaluation;
    expect(result.failures).toEqual([]); expect(result.violations).toHaveLength(130);
    const candidates = readdirSync(directory).filter(f => /^candidate-.*\.json$/.test(f))
      .map(f => JSON.parse(readFileSync(join(directory, f), "utf8"))).filter(c => c.variant === "normal");
    expect(candidates).toHaveLength(1); expect(candidates[0].queries).toHaveLength(2);
    const manifests = readdirSync(directory).filter(f => /^producer-plan-.*\.json$/.test(f))
      .map(f => JSON.parse(readFileSync(join(directory, f), "utf8")));
    const bindings = manifests.flatMap(m => m.bindings).filter(b => b.contract === "syntax.child_fields.v1");
    expect(bindings).toHaveLength(2);
    expect(new Set(bindings.map(b => b.options.field.value)).size, "field options remain different semantic bindings").toBe(2);
    let fills = 0;
    for (const file of readdirSync(directory).filter(f => /^profile-.*\.json$/.test(f))) {
      const catalogue = JSON.parse(readFileSync(join(directory, file), "utf8"));
      const queries = new Map<number, string>(catalogue.queries.map((q: { id: number; sql: string }) => [q.id, q.sql]));
      const events = readFileSync(join(directory, file.replace(/\.json$/, ".jsonl")), "utf8").split("\n").filter(Boolean).map(l => JSON.parse(l));
      expect(events.filter(e => e.kind === "sql")).toHaveLength(catalogue.executionCoverage.completed);
      fills += events.filter(e => e.kind === "sql" && queries.get(e.query)?.includes("INSERT INTO decoded_children")).length;
    }
    expect(fills, "ready fields share one SQL submission without merging their completion keys").toBe(1);
  } finally { rmSync(directory, { recursive: true, force: true }); }
}, 45_000);

it.skipIf(!required)("invalidates completed child answers when a file changes and changes back", () => {
  const program = `
    import {readFileSync} from 'node:fs';
    import {loadNativeBinding} from ${JSON.stringify(engine)};
    const native=loadNativeBinding(),caps=readFileSync(new URL(${JSON.stringify(capabilities)}),'utf8');
    const ws=new native.CodeRuleDslWorkspace();
    for(const [id,field,kind] of [['callee','function','identifier'],['args','arguments','argument_list']]) {
      ws.addCodeRule(native.CodeRuleDslRuntime.compile(JSON.stringify({format:'rerail.policy/v1',id,revision:1,mode:'log',
        clauses:[{id:'selected',forbid:{ast:{language:'python',rule:{kind:'call',has:{field,kind}}}},
        violation:{code:'MATCH',message:'Match',anchor:'witness.span'}}]}),'json',caps));
    }
    const results=[];
    try {
      for(const [i,source] of ['target(value)\\n','owner.target()\\n','target(value)\\n'].entries())
        results.push(JSON.parse(await ws.applyAsync(JSON.stringify({revision:i+1,removed:[],files:[{path:'a.py',language:'python',source}]}))).evaluation);
      console.log(JSON.stringify(results));
    } finally {ws.close();}
  `;
  const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
    encoding: "utf8", timeout: 30_000, maxBuffer: 8 * 1024 * 1024,
  });
  expect(child.error).toBeUndefined(); expect(child.status, child.stderr).toBe(0);
  const results = JSON.parse(child.stdout.trim());
  expect(results.map((r: { failures: unknown[] }) => r.failures)).toEqual([[], [], []]);
  expect(results.map((r: { violations: unknown[] }) => r.violations.length)).toEqual([2, 1, 2]);
});
