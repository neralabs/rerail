import { spawnSync } from "node:child_process";
import { expect, it } from "vitest";

const required = process.env.RERAIL_REQUIRE_PRODUCER_SHAPE === "1";
const engine = new URL("../../../src/engine/index.ts", import.meta.url).href;
const capabilities = new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url).href;

function run(exclusion: string, files = 1) {
  const program = `
    import { readFileSync } from 'node:fs';
    import { loadNativeBinding } from ${JSON.stringify(engine)};
    const native=loadNativeBinding(), caps=readFileSync(new URL(${JSON.stringify(capabilities)}),'utf8');
    const rule=(id,ast,scope={include:['**/*'],exclude:[]})=>native.CodeRuleDslRuntime.compile(JSON.stringify({
      format:'rerail.policy/v1',id,revision:1,mode:'log',scope,clauses:[{id:'match',
      forbid:{ast},violation:{code:'MATCH',message:'Match',anchor:'witness.span'}}]}),'json',caps);
    const ast={language:'python',rule:{pattern:'target($X)'}};
    const ws=new native.CodeRuleDslWorkspace();
    const exclusion=${JSON.stringify(exclusion)};
    ws.addCodeRule(rule('active',ast,exclusion==='domain'?{include:['a/**'],exclude:[]}:undefined));
    if(exclusion==='domain')ws.addCodeRule(rule('other',ast,{include:['b/**'],exclude:[]}));
    if(exclusion==='path')ws.addCodeRule(rule('excluded',ast,{include:['excluded/**'],exclude:[]}));
    if(exclusion==='text')ws.addCodeRule(rule('excluded',{...ast,rule:{pattern:'absent($X)'}}));
    if(exclusion==='syntax')ws.addCodeRule(rule('excluded',{...ast,syntax:'invalid'}));
    if(exclusion==='language')ws.addCodeRule(rule('excluded',{language:'typescript',rule:{pattern:'target($X)'}}));
    const files=Array.from({length:${files}},(_,i)=>({path:(exclusion==='domain'?(i%2?'a/':'b/'):'')+'f'+i+'.py',language:'python',source:'target(1)\\n'}));
    try {
      const result=JSON.parse(await ws.applyAsync(JSON.stringify({revision:1,files,removed:[]}))).evaluation;
      console.log(JSON.stringify(result));
    } finally {ws.close();}
  `;
  const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
    encoding: "utf8", timeout: 30_000, maxBuffer: 8 * 1024 * 1024,
    env: { ...process.env, RERAIL_PROFILE: "1" },
  });
  expect(child.error).toBeUndefined(); expect(child.status, child.stderr).toBe(0);
  const result = JSON.parse(child.stdout.trim().split("\n").at(-1)!);
  expect(result.failures).toEqual([]); expect(result.violations).toHaveLength(files);
  const trace = child.stderr.split("\n").filter(l => l.startsWith("RERAIL_TRACE ")).map(l => JSON.parse(l.slice(13)));
  expect(trace.length, "requires the profiling addon; missing counters cannot pass").toBeGreaterThan(0);
  return trace;
}

it.skipIf(!required)("excluded path, text, syntax and language plans do not enter candidate preparation", () => {
  for (const exclusion of ["none", "path", "text", "syntax", "language"]) {
    const trace = run(exclusion);
    expect(trace.filter(t => t.phase === "anchor-lower" && t.name === "template")
      .map(t => t.detail.plans), exclusion).toEqual([1]);
    expect(trace.filter(t => t.phase === "anchor-batch"), exclusion).toHaveLength(1);
  }
}, 60_000);

it.skipIf(!required)("files with the same admitted frontier share one broad candidate batch", () => {
  const trace = run("path", 20);
  const batches = trace.filter(t => t.phase === "anchor-batch");
  expect(batches).toHaveLength(1);
  expect(batches[0].detail.files).toHaveLength(20);
  expect(trace.filter(t => t.phase === "anchor-lower" && t.name === "template").map(t => t.detail.plans)).toEqual([1]);
}, 30_000);


it.skipIf(!required)("preparation keeps disjoint rule file domains separate", () => {
  const trace = run("domain", 20);
  expect(trace.filter(t => t.phase === "anchor-batch").map(t => t.detail.files.length)).toEqual([10, 10]);
  expect(trace.filter(t => t.phase === "anchor-lower" && t.name === "template").map(t => t.detail.plans)).toEqual([1, 1]);
}, 30_000);
