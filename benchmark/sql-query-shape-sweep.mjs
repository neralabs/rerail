// Fresh-process, alternating whole-workspace prototypes. All rewrite code is cfg(test).
// Usage: node benchmark/sql-query-shape-sweep.mjs /absolute/sweep.json
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { cpus, loadavg } from "node:os";
import { join, resolve } from "node:path";

const definition = JSON.parse(readFileSync(resolve(process.argv[2]),"utf8"));
const base = JSON.parse(readFileSync(definition.baseConfig,"utf8"));
const output = resolve(definition.outputDirectory);
if (existsSync(output)) throw new Error(`Output directory already exists: ${output}`);
mkdirSync(output,{recursive:true});
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
const median = values => [...values].sort((a,b)=>a-b)[Math.floor(values.length/2)];
const metadata = { definition,base,testBinarySha256:hash(readFileSync(definition.testBinary)),
  sourceManifest:JSON.parse(readFileSync(definition.sourceManifest,"utf8")),
  policies:base.policies.map(path=>({path,sha256:hash(readFileSync(path))})),
  files:base.files.map(path=>({path,sha256:hash(readFileSync(join(base.corpus,path)))})),
  machine:{cpu:cpus()[0]?.model,logicalCpus:cpus().length,node:process.version},
  interpretation:"Fresh processes, alternating variant order, identical native output oracle and file order. One test-only rewrite shape per selected family. CPU includes rewrite dispatch and normal test-build terminal counters. No query replay, tracing, or EXPLAIN during timed apply. Diagnostic system allocator. These prototype timings do not establish shipped-addon or production-allocator performance." };
const records = [];
for (let round=0;round<(definition.rounds??3);round++) {
  for (const variant of round%2?[...definition.variants].reverse():definition.variants) {
    const name = `${round}-${variant.name}`;
    const configPath = join(output,`${name}.config.json`),directory=join(output,name);
    writeFileSync(configPath,JSON.stringify({...base,shapePhase:"rewrite",shapeRewrite:variant.rewrite??{}},null,2)+"\n");
    const environment = {...process.env};
    for (const key of Object.keys(environment)) if (key.startsWith("RERAIL_SQL_") || key==="RERAIL_BENCH_WORKERS") delete environment[key];
    Object.assign(environment,{RERAIL_SQL_SHAPE_CONFIG:configPath,RERAIL_SQL_SHAPE_OUTPUT:directory,RERAIL_BENCH_WORKERS:String(base.threads??4)});
    const load = loadavg();
    const child=spawnSync(definition.testBinary,["--exact","code_rule_dsl::workspace::query_shape_workload::replay_sql_shapes_on_workload","--ignored","--nocapture","--test-threads=1"],
      {env:environment,encoding:"utf8",timeout:definition.timeoutMs??240000,maxBuffer:8*1024*1024});
    writeFileSync(join(output,`${name}.log`),(child.stdout??"")+(child.stderr??""));
    let result={};
    if (child.status!==0) result={error:child.error?.message??`Test process exited ${child.status}; see ${name}.log`};
    else {
      const run=JSON.parse(readFileSync(join(directory,"workload.json"),"utf8"));
      const sql=run.timings.find(t=>t.engine==="sql"),native=run.timings.find(t=>t.engine==="native");
      result={cpuMs:sql.applyCpuNs/1e6,wallMs:Number(sql.applyWallNs)/1e6,nativeCpuMs:native.applyCpuNs/1e6,
        digest:run.digest,findings:run.findings,parity:run.completeWorkspaceParity,
        rewrites:JSON.parse(readFileSync(join(directory,"replay.json"),"utf8"))};
    }
    records.push({round,variant:variant.name,rewrite:variant.rewrite??{},load,...result});
    writeFileSync(join(output,"comparison.json"),JSON.stringify({...metadata,records},null,2)+"\n");
    console.error(JSON.stringify(records.at(-1)));
  }
}
const summary=definition.variants.map(variant=>{
  const runs=records.filter(r=>r.variant===variant.name),good=runs.filter(r=>!r.error&&r.parity);
  return {name:variant.name,runs:good.length,parity:good.length===runs.length,
    ...(good.length?{cpuMs:median(good.map(r=>r.cpuMs)),wallMs:median(good.map(r=>r.wallMs)),nativeCpuMs:median(good.map(r=>r.nativeCpuMs))}:{})};
});
writeFileSync(join(output,"comparison.json"),JSON.stringify({...metadata,records,summary},null,2)+"\n");
console.log(JSON.stringify(summary,null,2));
if (summary.some(s=>!s.parity)) process.exitCode=1;
