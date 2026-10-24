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
