import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { expect, it } from "vitest";

const required = process.env.RERAIL_REQUIRE_PRODUCER_SHAPE === "1";
const engine = new URL("../../../src/engine/index.ts", import.meta.url).href;
const capabilities = new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url).href;

it.skipIf(!required)("publishes language catalogs and value postings in batches rather than row executions", () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-publication-"));
  try {
    const program = `
      import { readFileSync } from 'node:fs';
      import { loadNativeBinding } from ${JSON.stringify(engine)};
      const native=loadNativeBinding();
      const rule=native.CodeRuleDslRuntime.compile(JSON.stringify({format:'rerail.policy/v1',id:'publish',revision:1,mode:'log',
        clauses:[{id:'identifiers',forbid:{ast:{language:'python',rule:{kind:'identifier',regex:'^v[0-9]+$'}}},
        violation:{code:'VALUE',message:'Value',anchor:'witness.span'}}]}),'json',readFileSync(new URL(${JSON.stringify(capabilities)}),'utf8'));
      const source=Array.from({length:100},(_,i)=>'v'+i+'(v'+(i+100)+')\\n').join('');
      const ws=new native.CodeRuleDslWorkspace();ws.addCodeRule(rule);
      try {
        const result=JSON.parse(await ws.applyAsync(JSON.stringify({revision:1,removed:[],files:Array.from({length:4},(_,i)=>({path:'f'+i+'.py',language:'python',source}))}))).evaluation;
        console.log(JSON.stringify({failures:result.failures,findings:result.violations.length}));
      } finally {ws.close();}
    `;
    const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
      encoding: "utf8", timeout: 45_000, maxBuffer: 2 * 1024 * 1024,
      env: { ...process.env, RERAIL_SQL_PROFILE_DIR: directory },
    });
    expect(child.error).toBeUndefined(); expect(child.status, child.stderr).toBe(0);
    expect(JSON.parse(child.stdout.trim().split("\n").at(-1)!)).toEqual({ failures: [], findings: 800 });
    const executions = new Map<string, number>();
    const catalogues = readdirSync(directory).filter(f => /^profile-.*\.json$/.test(f));
    expect(catalogues.length, "missing profiling evidence cannot pass").toBeGreaterThan(0);
    for (const file of catalogues) {
      const catalogue = JSON.parse(readFileSync(join(directory, file), "utf8"));
      const queries = new Map<number, { kind: string; sql: string }>(catalogue.queries.map((q: { id: number; kind: string; sql: string }) => [q.id, q]));
      const events = readFileSync(join(directory, file.replace(/\.json$/, ".jsonl")), "utf8").trim().split("\n").filter(Boolean).map(l => JSON.parse(l));
      expect(events.filter(e => e.kind === "sql")).toHaveLength(catalogue.executionCoverage.completed);
      for (const event of events) {
        const query = queries.get(event.query)!;
        const table = query.kind === "sql" ? /^\s*INSERT\s+INTO\s+(kind|field|vidx|vcov|file_kind_counts|kfreq)\b/i.exec(query.sql)?.[1] : undefined;
        if (table) executions.set(table, (executions.get(table) ?? 0) + 1);
      }
    }
    expect(Object.fromEntries(executions)).toEqual({ kind: 1, field: 1, kfreq: 4, file_kind_counts: 4, vidx: 4, vcov: 4 });
  } finally { rmSync(directory, { recursive: true, force: true }); }
}, 55_000);
