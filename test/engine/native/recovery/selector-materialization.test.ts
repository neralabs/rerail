import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";

const required = process.env.RERAIL_REQUIRE_PRODUCER_SHAPE === "1";
const engine = new URL("../../../../src/engine/index.ts", import.meta.url).href;
const capabilities = new URL("../../../../native/src/code_rule_dsl/capabilities.json", import.meta.url).href;

it.skipIf(!required)("pipelines each selector into its physical owner without a second ephemeral copy", () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-selector-family-"));
  try {
    const program = `
      import {readFileSync} from 'node:fs';
      import {loadNativeBinding} from ${JSON.stringify(engine)};
      const native=loadNativeBinding(),caps=readFileSync(new URL(${JSON.stringify(capabilities)}),'utf8');
      const ws=new native.CodeRuleDslWorkspace();
      for(const [id,kind] of [['names','identifier'],['calls','call']]) {
        const clauses=Array.from({length:65},(_,i)=>({id:'selected'+i,
          forbid:{ast:{language:'python',rule:{kind}}},
          violation:{code:'MATCH',message:'Match',anchor:'witness.span'}}));
        ws.addCodeRule(native.CodeRuleDslRuntime.compile(JSON.stringify({format:'rerail.policy/v1',id,revision:1,mode:'log',clauses}),'json',caps));
      }
      try {console.log(await ws.applyAsync(JSON.stringify({revision:1,removed:[],files:[{path:'a.py',language:'python',source:'target()\\n'}]})));}
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
    let creates = 0, indexes = 0, releases = 0;
    for (const file of readdirSync(directory).filter(f => /^profile-.*\.json$/.test(f))) {
      const catalogue = JSON.parse(readFileSync(join(directory, file), "utf8"));
      const queries = new Map<number, string>(catalogue.queries.map((q: { id: number; sql: string }) => [q.id, q.sql]));
      const events = readFileSync(join(directory, file.replace(/\.json$/, ".jsonl")), "utf8").split("\n").filter(Boolean).map(l => JSON.parse(l));
      expect(events.filter(e => e.kind === "sql")).toHaveLength(catalogue.executionCoverage.completed);
      for (const event of events.filter(e => e.kind === "sql")) {
        const sql = queries.get(event.query) ?? "";
        creates += Number(sql.startsWith('CREATE TEMP TABLE "'));
        indexes += Number(sql.startsWith("CREATE INDEX temp."));
        releases += Number(sql.startsWith("DROP TABLE temp."));
      }
    }
    expect([creates, indexes, releases], "distinct selectors retain their physical owners").toEqual([2, 2, 2]);
    const owners = candidates[0].stages.filter((s: {temporary: boolean; index: string}) => s.temporary && s.index === "fid,pre");
    expect(owners).toHaveLength(2);
    for (const owner of owners)
      expect(owner.sql, "the CTAS owner is the materialization boundary").toContain(`${owner.name}(fid,pre) AS NOT MATERIALIZED`);
  } finally { rmSync(directory, { recursive: true, force: true }); }
}, 45_000);
