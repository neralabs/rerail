import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";

const required = process.env.RERAIL_REQUIRE_PRODUCER_SHAPE === "1";
const engine = new URL("../../../src/engine/index.ts", import.meta.url).href;
const capabilities = new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url).href;

it.skipIf(!required)("exports real shared structural bindings with pinned source generations", () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-shared-producers-"));
  try {
    const program = `
      import {readFileSync} from 'node:fs';
      import {loadNativeBinding} from ${JSON.stringify(engine)};
      const native=loadNativeBinding(),caps=readFileSync(new URL(${JSON.stringify(capabilities)}),'utf8');
      const ws=new native.CodeRuleDslWorkspace();
      for(const [id,kind] of [['names','identifier'],['calls','call']]) {
        ws.addCodeRule(native.CodeRuleDslRuntime.compile(JSON.stringify({format:'rerail.policy/v1',id,revision:1,mode:'log',
          clauses:[{id:'selected',forbid:{ast:{language:'python',rule:{kind,regex:'target'}}},
          violation:{code:'MATCH',message:'Match',anchor:'witness.span'}}]}),'json',caps));
      }
      try {
        const result=JSON.parse(await ws.applyAsync(JSON.stringify({revision:1,removed:[],files:[{path:'a.py',language:'python',source:'target(value)\\n'}]}))).evaluation;
        console.log(JSON.stringify(result));
      } finally {ws.close();}
    `;
    const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
      encoding: "utf8", timeout: 30_000, maxBuffer: 8 * 1024 * 1024,
      env: { ...process.env, RERAIL_SQL_PROFILE_DIR: directory },
    });
    expect(child.error).toBeUndefined(); expect(child.status, child.stderr).toBe(0);
    const result = JSON.parse(child.stdout.trim().split("\n").at(-1)!);
    expect(result.failures).toEqual([]); expect(result.violations).toHaveLength(2);
    const files = readdirSync(directory).filter(f => /^producer-plan-.*\.json$/.test(f));
    expect(files.length, "actual candidate compilation must export its producer plan").toBeGreaterThan(0);
    const manifests = files.map(f => JSON.parse(readFileSync(join(directory, f), "utf8")));
    const spans = manifests.flatMap(m => m.bindings).filter(b => b.contract === "syntax.source_spans.v1");
    expect(spans.length).toBeGreaterThan(0);
    expect(spans.some(b => b.consumers.length >= 2), "different selectors must reference one span binding").toBe(true);
    const consumers = spans.flatMap(b => b.consumers).join("\n");
    expect(consumers).toContain("rule names clause selected");
    expect(consumers).toContain("rule calls clause selected");
    expect(consumers).toContain("policy sha256:");
    for (const binding of spans) {
      expect(binding.inputs.tape.external.generation).toMatch(/[A-F0-9]{64}/);
      expect(binding.inputs.tape.external.generation).not.toMatch(/placeholder|pending|illustrative/);
    }
    const checked = spawnSync("python3", ["scripts/profiling/shared_work.py", "check", ...files.map(f => join(directory, f)), "--output", join(directory, "checked.json")], { encoding: "utf8", timeout: 10_000 });
    expect(checked.status, checked.stderr).toBe(0);
    const report = JSON.parse(readFileSync(join(directory, "checked.json"), "utf8"));
    expect(report.valid).toBe(true); expect(report.clashes).toEqual([]);
    expect(report.sharedBindings.some((b: { contract: string }) => b.contract === "syntax.source_spans.v1")).toBe(true);
  } finally { rmSync(directory, { recursive: true, force: true }); }
}, 45_000);

it.skipIf(!required)("fills one shared span owner across two rules and two result packets", () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-span-wave-"));
  try {
    const program = `
      import {readFileSync} from 'node:fs';
      import {loadNativeBinding} from ${JSON.stringify(engine)};
      const native=loadNativeBinding(),caps=readFileSync(new URL(${JSON.stringify(capabilities)}),'utf8');
      const ws=new native.CodeRuleDslWorkspace();
      for(const [id,kind] of [['names','identifier'],['calls','call']]) {
        const clauses=Array.from({length:65},(_,i)=>({id:'selected'+i,
          forbid:{ast:{language:'python',rule:{kind,regex:'target'}}},
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
    const result = JSON.parse(child.stdout.trim().split("\n").at(-1)!).evaluation;
    expect(result.failures).toEqual([]); expect(result.violations).toHaveLength(130);
    const candidates = readdirSync(directory).filter(f => /^candidate-.*\.json$/.test(f))
      .map(f => JSON.parse(readFileSync(join(directory, f), "utf8"))).filter(c => c.variant === "normal");
    expect(candidates).toHaveLength(1); expect(candidates[0].queries).toHaveLength(2);
    let fills = 0;
    for (const file of readdirSync(directory).filter(f => /^profile-.*\.json$/.test(f))) {
      const catalogue = JSON.parse(readFileSync(join(directory, file), "utf8"));
      const queries = new Map<number, string>(catalogue.queries.map((q: { id: number; sql: string }) => [q.id, q.sql]));
      const events = readFileSync(join(directory, file.replace(/\.json$/, ".jsonl")), "utf8").split("\n").filter(Boolean).map(l => JSON.parse(l));
      expect(events.filter(e => e.kind === "sql")).toHaveLength(catalogue.executionCoverage.completed);
      fills += events.filter(e => e.kind === "sql" && queries.get(e.query)?.includes("INSERT INTO decoded_spans")).length;
    }
    expect(fills, "independent ready selectors must feed one physical fill, across both result packets").toBe(1);
  } finally { rmSync(directory, { recursive: true, force: true }); }
}, 45_000);
