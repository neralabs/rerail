import { appendFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/index.js";

const native = loadNativeBinding();
const profiled = "NativeSqlSpike" in native;
if (process.env.RERAIL_REQUIRE_PRODUCER_SHAPE === "1" && !profiled) throw new Error("The producer-shape gate requires a code-rule-profile native build");
const engine = new URL("../../../src/engine/index.ts", import.meta.url).href;
const capabilities = new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url).href;
function run(files: number, roots: number, workspace: boolean, insertBatch = 32, policies = workspace ? 2 : 1, dependent = false) {
  const program = `
    import { readFileSync } from 'node:fs';
    import { loadNativeBinding } from ${JSON.stringify(engine)};
    const native=loadNativeBinding();
    const clause=id=>({id,relational:{partitionBy:'file',
      inputs:{gate:{evidence:{as:'n',query:{ast:{language:'python',syntax:'valid',rule:{kind:'module'}}},project:{path:'file.path'}}}},
      syntax:{name:'nodes',gate:'gate',fields:['name'],ownership:'ownership',owners:{fn:{kind:'function_definition'}}},
      stages:[{name:'descendants',reachability:{syntax:'nodes',roots:"SELECT __path,id FROM nodes WHERE kind='function_definition'"}},...(${dependent}?[{name:'again',reachability:{syntax:'nodes',roots:'SELECT __path,root AS id FROM descendants WHERE root=node'}}]:[])],
      query:"SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,n.id FROM nodes n JOIN descendants d ON d.__path=n.__path AND d.root=n.id AND d.node=n.id WHERE n.kind='function_definition'",key:['id']},
      violation:{code:'BATCH',message:'Batch',anchor:'witness.span'}});
    const compile=id=>native.CodeRuleDslRuntime.compile(JSON.stringify({format:'rerail.policy/v1',id,revision:1,mode:'log',clauses:[clause('first'),clause('second')]}),'json',readFileSync(new URL(${JSON.stringify(capabilities)}),'utf8'));
    const rule=compile('batch');
    const files=Array.from({length:${files}},(_,i)=>({path:'f'+i+'.py',language:'python',source:Array.from({length:${roots}},(_,j)=>'def f'+j+'(): return '+j+'\\n').join('')}));
    let result;
    if(${workspace}) {const ws=new native.CodeRuleDslWorkspace();for(let i=0;i<${policies};i++)ws.addCodeRule(compile('batch'+i));try {result=JSON.parse(await ws.applyAsync(JSON.stringify({revision:1,files,removed:[]}))).evaluation;}finally{ws.close();}}
    else result=JSON.parse(rule.evaluate(JSON.stringify({format:'rerail.repository-snapshot/v1',revision:1,files}),'report'));
    console.log(JSON.stringify({failures:result.failures,count:result.violations.length}));
  `;
  const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
    encoding: "utf8", timeout: 110_000, maxBuffer: 16 * 1024 * 1024,
    env: { ...process.env, RERAIL_PROFILE: "1", RERAIL_SQL_INSERT_BATCH: String(insertBatch) },
  });
  expect(child.error).toBeUndefined(); expect(child.status, child.stderr).toBe(0);
  const result = JSON.parse(child.stdout.trim().split("\n").at(-1)!);
  expect(result.failures).toEqual([]); expect(result.count).toBe(files * roots * 2 * policies);
  const trace = child.stderr.split("\n").filter(l => l.startsWith("RERAIL_TRACE ")).map(l => JSON.parse(l.slice(13)));
  if (process.env.RERAIL_BATCH_RECEIPTS) appendFileSync(process.env.RERAIL_BATCH_RECEIPTS, JSON.stringify({ files, roots, policies, workspace, insertBatch, dependent, findings: result.count, stages: trace.filter(t => t.phase === "producer-stage").map(t => t.name), waves: trace.filter(t => t.phase === "producer-wave").map(t => t.detail) }) + "\n");
  return trace;
}
it.skipIf(!profiled)("keeps broad producer passes constant as independent roots and files grow", () => {
  for (const workspace of [false, true]) {
    for (const [files, roots, insertion] of [[1, 1, 32], [1, 100, 1], [100, 1, 256]]) {
      const trace = run(files!, roots!, workspace, insertion);
      expect(trace.filter(t => t.phase === "producer-stage").map(t => t.name)).toEqual(["count", "scan", "scatter"]);
      expect(trace.filter(t => t.phase === "producer-wave")).toHaveLength(1);
    }
  }
}, 110_000);

it.skipIf(!profiled)("batches policies together and advances genuine SQL dependencies in separate waves", () => {
  const together = run(1, 1, true, 32, 100);
  expect(together.filter(t => t.phase === "producer-stage").map(t => t.name)).toEqual(["count", "scan", "scatter"]);
  const dependent = run(2, 2, false, 32, 1, true);
  expect(dependent.filter(t => t.phase === "producer-wave")[1].detail.inputDocumentsAdded).toBe(0);
  expect(dependent.filter(t => t.phase === "producer-stage").map(t => t.name)).toEqual(["count", "scan", "scatter", "count", "scan", "scatter"]);
});
