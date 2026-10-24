import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";
const required=process.env.RERAIL_REQUIRE_PRODUCER_SHAPE==="1";
const engine=new URL("../../../../src/engine/index.ts",import.meta.url).href;
const caps=new URL("../../../../native/src/code_rule_dsl/capabilities.json",import.meta.url).href;
it.skipIf(!required)("reuses empty relational backing across clauses without indexes, ANALYZE or stale rows",()=>{
 const directory=mkdtempSync(join(tmpdir(),"rerail-relational-storage-"));
 try {
  const program=`
   import {readFileSync} from 'node:fs';
   import {loadNativeBinding} from ${JSON.stringify(engine)};
   const n=loadNativeBinding(),w=new n.CodeRuleDslWorkspace();
   const evidence={as:'e',query:{ast:{language:'python',rule:{kind:'identifier'}}},project:{path:'file.path'}};
   const relational={inputs:{events:{evidence}},stages:[{name:'empty',query:'SELECT * FROM events WHERE 0',indexes:[['__path','__start']]}],query:'SELECT __path path,__start start_byte,__end end_byte FROM events'};
   const clauses=Array.from({length:6},(_,i)=>({id:'c'+i,relational,violation:{code:'FOUND',message:'Found',anchor:'witness.span'}}));
   w.addCodeRule(n.CodeRuleDslRuntime.compile(JSON.stringify({format:'rerail.policy/v1',id:'shared-storage',revision:1,mode:'log',clauses}),'json',readFileSync(new URL(${JSON.stringify(caps)}),'utf8')));
   try {for(const [revision,source] of [[1,'target()\\n'],[2,'pass\\n'],[3,'other()\\n']])console.log(await w.applyAsync(JSON.stringify({revision,removed:[],files:[{path:'a.py',language:'python',source}]})));}finally{w.close();}
  `;
  const child=spawnSync(process.execPath,['--import','tsx','--input-type=module','-e',program],{encoding:'utf8',timeout:30_000,maxBuffer:8*1024*1024,env:{...process.env,RERAIL_SQL_PROFILE_DIR:directory}});
  expect(child.error).toBeUndefined();expect(child.status,child.stderr).toBe(0);
  const evaluations=child.stdout.trim().split('\n').map(line=>JSON.parse(line).evaluation);
  expect(evaluations.map(e=>e.failures)).toEqual([[],[],[]]);expect(evaluations.map(e=>e.violations.length)).toEqual([6,0,6]);
  let configuration=0,indexes=0,statistics=0;
  for(const file of readdirSync(directory).filter(f=>/^profile-.*\.json$/.test(f))){
   const catalogue=JSON.parse(readFileSync(join(directory,file),'utf8'));
   const queries=new Map<number,string>(catalogue.queries.map((q:{id:number;sql:string})=>[q.id,q.sql]));
   const events=readFileSync(join(directory,file.replace(/\.json$/,'.jsonl')),'utf8').split('\n').filter(Boolean).map(l=>JSON.parse(l));
   expect(events.filter(e=>e.kind==='sql')).toHaveLength(catalogue.executionCoverage.completed);
   for(const e of events.filter(e=>e.kind==='sql'&&e.origin==='engine')){const sql=queries.get(e.query)??'';configuration+=Number(sql.includes('PRAGMA analysis_limit=1000'));indexes+=Number(sql.startsWith('CREATE INDEX')&&sql.includes('__empty_'));statistics+=Number(sql.startsWith('ANALYZE temp.')&&sql.includes('empty'));}
  }
  expect(configuration,'one reusable clause connection in each evaluation').toBe(3);
  expect([indexes,statistics],'empty relations need no physical indexes or statistics').toEqual([0,0]);
 }finally{rmSync(directory,{recursive:true,force:true});}
},35_000);
