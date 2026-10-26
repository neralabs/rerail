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
  expect(configuration,'one SQL owner across all workspace evaluations').toBe(1);
  expect([indexes,statistics],'empty relations need no physical indexes or statistics').toEqual([0,0]);
 }finally{rmSync(directory,{recursive:true,force:true});}
},35_000);

it.skipIf(!required)("preserves deferred CAST affinity through a later stored stage",()=>{
 const program=`
  import {readFileSync} from 'node:fs';import {loadNativeBinding} from ${JSON.stringify(engine)};
  const n=loadNativeBinding();const policy={format:'rerail.policy/v1',id:'affinity',revision:1,mode:'log',clauses:[{id:'typed',relational:{inputs:{events:{evidence:{as:'e',query:{ast:{language:'python',rule:{kind:'identifier'}}},project:{path:'file.path'}}}},stages:[{name:'a',query:"SELECT __path,__start,__end,'01' AS value FROM events",deferred:{value:"CAST('01' AS INTEGER)"}},{name:'b',query:'SELECT * FROM a'}],query:"SELECT __path path,__start start_byte,__end end_byte FROM b WHERE value='01'"},violation:{code:'MATCH',message:'Match',anchor:'witness.span'}}]};
  const results=[];
  for(let i=0;i<3;i++){
   const r=n.CodeRuleDslRuntime.compile(JSON.stringify(policy),'json',readFileSync(new URL(${JSON.stringify(caps)}),'utf8'));
   results.push(JSON.parse(r.evaluate(JSON.stringify({format:'rerail.repository-snapshot/v1',revision:1,files:[{path:'a.py',language:'python',source:'target()'}]}),'report')));
  }
  console.log(JSON.stringify(results));
 `;
 const child=spawnSync(process.execPath,['--import','tsx','--input-type=module','-e',program],{encoding:'utf8',timeout:30_000});
 expect(child.error).toBeUndefined();expect(child.status,child.stderr).toBe(0);
 const results=JSON.parse(child.stdout);expect(results).toHaveLength(3);
 for(const result of results){expect(result.failures).toEqual([]);expect(result.violations).toHaveLength(1);}
});

it("preserves stored-stage affinity and collation at SQL pipeline boundaries", () => {
 const policies = [
  "SELECT __path,__start,__end,+1 value FROM events UNION ALL SELECT __path,__start,__end,CAST('01' AS TEXT) value FROM events",
  "SELECT __path,__start,__end,CAST('a' AS TEXT) COLLATE NOCASE value FROM events",
 ];
 const predicates = ["value='1'", "value='A'"];
 const program = `
  import {readFileSync} from 'node:fs';import {loadNativeBinding} from ${JSON.stringify(engine)};
  const n=loadNativeBinding(),caps=readFileSync(new URL(${JSON.stringify(caps)}),'utf8');
  const queries=${JSON.stringify(policies)}, predicates=${JSON.stringify(predicates)};
  const results=queries.map((query,i)=>{
    const policy={format:'rerail.policy/v1',id:'boundary'+i,revision:1,mode:'log',clauses:[{id:'test',relational:{inputs:{events:{evidence:{as:'e',query:{ast:{language:'python',rule:{kind:'identifier'}}},project:{path:'file.path'}}}},stages:[{name:'stored',query}],query:'SELECT __path path,__start start_byte,__end end_byte,value FROM stored WHERE '+predicates[i],key:['value']},violation:{code:'FOUND',message:'Found',anchor:'witness.span'}}]};
    const rule=n.CodeRuleDslRuntime.compile(JSON.stringify(policy),'json',caps);
    return JSON.parse(rule.evaluate(JSON.stringify({format:'rerail.repository-snapshot/v1',revision:1,files:[{path:'a.py',language:'python',source:'target()'}]}),'report'));
  });console.log(JSON.stringify(results));
 `;
 const child=spawnSync(process.execPath,['--import','tsx','--input-type=module','-e',program],{encoding:'utf8',timeout:30_000});
 expect(child.error).toBeUndefined();expect(child.status,child.stderr).toBe(0);
 for(const [i,result] of JSON.parse(child.stdout).entries()){expect(result.failures).toEqual([]);expect(result.violations,policies[i]).toHaveLength(0);}
});

it.skipIf(!required)("keeps one indexed owner for a stage read repeatedly inside a fused family", () => {
 const directory=mkdtempSync(join(tmpdir(),"rerail-indexed-family-"));
 try {
  const program=`
   import {readFileSync} from 'node:fs';import {loadNativeBinding} from ${JSON.stringify(engine)};
   const n=loadNativeBinding(),caps=readFileSync(new URL(${JSON.stringify(caps)}),'utf8');
   const evidence={as:'e',query:{ast:{language:'python',rule:{kind:'identifier',pattern:'$NAME'}}},project:{name:'capture.NAME'}};
   const stages=[{name:'facts',query:'SELECT __path,__start,__end,name FROM events',indexes:[['name']]},
    {name:'left_rows',query:'SELECT f.*,(SELECT count(*) FROM facts k WHERE k.name=f.name) AS n FROM facts f'},
    {name:'right_rows',query:'SELECT f.*,(SELECT count(*) FROM facts k WHERE k.name=f.name) AS n FROM facts f'}];
   const policy={format:'rerail.policy/v1',id:'indexed-family',revision:1,mode:'log',clauses:[{id:'all',relational:{inputs:{events:{evidence}},stages,query:'SELECT l.__path path,l.__start start_byte,l.__end end_byte FROM left_rows l JOIN right_rows r ON r.name=l.name WHERE l.n=r.n'},violation:{code:'ROW',message:'Row',anchor:'witness.span'}}]};
   const rule=n.CodeRuleDslRuntime.compile(JSON.stringify(policy),'json',caps),ws=new n.CodeRuleDslWorkspace();ws.addCodeRule(rule);
   const source=Array.from({length:2000},(_,i)=>'v'+i).join(String.fromCharCode(10));
   try {console.log(await ws.applyAsync(JSON.stringify({revision:1,files:[{path:'a.py',language:'python',source}],removed:[]})));}finally{ws.close();}
  `;
  const child=spawnSync(process.execPath,['--import','tsx','--input-type=module','-e',program],{encoding:'utf8',timeout:30_000,maxBuffer:8*1024*1024,env:{...process.env,RERAIL_SQL_PROFILE_DIR:directory,RERAIL_SQL_PROFILE_SCOPE:'relational'}});
  expect(child.error).toBeUndefined();expect(child.status,child.stderr).toBe(0);
  const result=JSON.parse(child.stdout.trim()).evaluation;
  expect(result.failures).toEqual([]);expect(result.violations).toHaveLength(2000);
  let indexes=0;
  for(const file of readdirSync(directory).filter(f=>/^profile-.*\.json$/.test(f))){
   const catalogue=JSON.parse(readFileSync(join(directory,file),'utf8'));
   const queries=new Map<number,string>(catalogue.queries.map((q:{id:number;sql:string})=>[q.id,q.sql]));
   const events=readFileSync(join(directory,file.replace(/\.json$/,'.jsonl')),'utf8').split('\n').filter(Boolean).map(l=>JSON.parse(l));
   indexes+=events.filter(e=>e.kind==='sql' && /^CREATE INDEX/.test(queries.get(e.query)??'') && /\("name"\)/.test(queries.get(e.query)??'')).length;
  }
  expect(indexes,'one authored lookup index must serve every reader of this producer').toBe(1);
 }finally{rmSync(directory,{recursive:true,force:true});}
},35_000);

it.skipIf(!required)("does not hydrate repository source for a complete empty admission gate", () => {
 const program=`
  import {readFileSync} from 'node:fs';import {loadNativeBinding} from ${JSON.stringify(engine)};
  const n=loadNativeBinding(),cap=JSON.parse(readFileSync(new URL(${JSON.stringify(caps)}),'utf8'));
  cap.limits.maxSourceBytes=32;
  const policy={format:'rerail.policy/v1',id:'empty-source-gate',revision:1,mode:'log',clauses:[{id:'gate',relational:{requireNonemptyInput:'gate',inputs:{gate:{evidence:{as:'e',query:{ast:{language:'python',rule:{all:[{kind:'module'},{regex:'\\\\b\\\\B'}]}}},project:{path:'file.path'}}}},query:'SELECT __path path,__start start_byte,__end end_byte FROM gate WHERE source_text(__path,__start,__end) IS NOT NULL'},violation:{code:'ROW',message:'Row',anchor:'witness.span'}}]};
  for(const admitted of [false,true]) {
   if(admitted) policy.clauses[0].relational.inputs.gate.evidence.query.ast.rule={kind:'module'};
   const w=new n.CodeRuleDslWorkspace();w.addCodeRule(n.CodeRuleDslRuntime.compile(JSON.stringify(policy),'json',JSON.stringify(cap)));
   try {console.log(await w.applyAsync(JSON.stringify({revision:1,files:Array.from({length:3},(_,i)=>({path:i+'.py',language:'python',source:'name = 1\\n'.repeat(3)})),removed:[]})));}finally{w.close();}
  }
 `;
 const child=spawnSync(process.execPath,['--import','tsx','--input-type=module','-e',program],{encoding:'utf8',timeout:30_000});
 expect(child.error).toBeUndefined();expect(child.status,child.stderr).toBe(0);
 const [empty,admitted]=child.stdout.trim().split('\n').map(line=>JSON.parse(line).evaluation);
 expect(empty.failures).toEqual([]);expect(empty.violations).toEqual([]);
 expect(admitted.failures.some((f:{message:string})=>f.message.includes('maxSourceBytes'))).toBe(true);
 expect(admitted.violations).toEqual([]);
});

it.skipIf(!required)("streams pure wide projections into narrow window consumers without copying helper rows", () => {
 const directory=mkdtempSync(join(tmpdir(),"rerail-projection-stream-"));
 try {
  const program=`
   import {readFileSync} from 'node:fs';import {loadNativeBinding} from ${JSON.stringify(engine)};
   const n=loadNativeBinding(),caps=readFileSync(new URL(${JSON.stringify(caps)}),'utf8');
   const padding=Array.from({length:48},(_,i)=>'NULL AS unused_'+i).join(',');
   const relational={inputs:{events:{evidence:{as:'e',query:{ast:{language:'python',rule:{kind:'identifier',pattern:'$NAME'}}},project:{name:'capture.NAME'}}}},stages:[
    {name:'helper',query:'SELECT __path,__start,__end,name,'+padding+' FROM events'},
    {name:'keys',query:'SELECT __path,__start,row_number() OVER(PARTITION BY __path ORDER BY __start)-1 AS ordinal FROM helper'}],
    query:'SELECT h.__path path,h.__start start_byte,h.__end end_byte,h.name,k.ordinal FROM helper h CROSS JOIN keys k ON k.__path=h.__path AND k.__start=h.__start'};
   const policy={format:'rerail.policy/v1',id:'pure-projection',revision:1,mode:'log',clauses:[{id:'rows',relational,violation:{code:'ROW',message:'Row',anchor:'witness.span'}}]};
   const w=new n.CodeRuleDslWorkspace();w.addCodeRule(n.CodeRuleDslRuntime.compile(JSON.stringify(policy),'json',caps));
   try {console.log(await w.applyAsync(JSON.stringify({revision:1,files:[{path:'a.py',language:'python',source:Array.from({length:2000},(_,i)=>'v'+i).join(String.fromCharCode(10))}],removed:[]})));}finally{w.close();}
  `;
  const child=spawnSync(process.execPath,['--import','tsx','--input-type=module','-e',program],{encoding:'utf8',timeout:30_000,maxBuffer:8*1024*1024,env:{...process.env,RERAIL_SQL_PROFILE_DIR:directory,RERAIL_SQL_PROFILE_SCOPE:'relational'}});
  expect(child.error).toBeUndefined();expect(child.status,child.stderr).toBe(0);
  const result=JSON.parse(child.stdout.trim()).evaluation;
  expect(result.failures).toEqual([]);expect(result.violations).toHaveLength(2000);
  let found=false;
  for(const file of readdirSync(directory).filter(f=>/^profile-.*\.json$/.test(f))){
   const catalogue=JSON.parse(readFileSync(join(directory,file),'utf8'));
   for(const query of catalogue.queries.filter((q:{sql:string})=>q.sql.includes('unused_47')&&q.sql.includes('SELECT h.__path'))){
    found=true;
    expect(query.plan.filter((row:unknown[])=>String(row[3]).includes('MATERIALIZE helper'))).toEqual([]);
   }
  }
  expect(found).toBe(true);
 }finally{rmSync(directory,{recursive:true,force:true});}
},35_000);

it.skipIf(!required)("preserves failed SQL inputs before workspace cleanup without enabling statement tracing", () => {
 const directory=mkdtempSync(join(tmpdir(),"rerail-failed-relations-"));
 try {
  const program=`
   import {readFileSync} from 'node:fs';import {loadNativeBinding} from ${JSON.stringify(engine)};
   const n=loadNativeBinding(),caps=readFileSync(new URL(${JSON.stringify(caps)}),'utf8');
   const relational={inputs:{events:{evidence:{as:'e',query:{ast:{language:'python',rule:{kind:'identifier',pattern:'$NAME'}}},project:{name:'capture.NAME'}}}},
    stages:[{name:'bad',query:"SELECT __path,__start,__end,rx_extract(name,'(x)',9) AS bad FROM events",indexes:[['bad']]}],
    query:'SELECT a.__path path,a.__start start_byte,a.__end end_byte FROM bad a JOIN bad b ON a.bad=b.bad'};
   const w=new n.CodeRuleDslWorkspace();w.addCodeRule(n.CodeRuleDslRuntime.compile(JSON.stringify({format:'rerail.policy/v1',id:'failed-sql',revision:1,mode:'log',clauses:[{id:'rows',relational,violation:{code:'ROW',message:'Row',anchor:'witness.span'}}]}),'json',caps));
   try {console.log(await w.applyAsync(JSON.stringify({revision:1,files:[{path:'a.py',language:'python',source:'éxx'}],removed:[]})));}finally{w.close();}
  `;
  const child=spawnSync(process.execPath,['--import','tsx','--input-type=module','-e',program],{encoding:'utf8',timeout:30_000,env:{...process.env,RERAIL_SQL_PROFILE_DIR:undefined,RERAIL_SQL_FAILURE_DIR:directory}});
  expect(child.error).toBeUndefined();expect(child.status,child.stderr).toBe(0);
  const result=JSON.parse(child.stdout.trim()).evaluation;
  expect(result.failures.some((f:{message:string})=>f.message.includes('regex group'))).toBe(true);
  const files=readdirSync(directory),manifests=files.filter(f=>/^failure-.*\.json$/.test(f));
  expect(manifests).toHaveLength(1);expect(files.filter(f=>/^profile-/.test(f))).toEqual([]);
  const snapshot=JSON.parse(readFileSync(join(directory,manifests[0]!), 'utf8'));
  expect(snapshot.context).toMatchObject({rule:'failed-sql',clause:'rows',stage:'bad'});
  expect(snapshot.context.budget.sqlRemaining).toBeTypeOf('number');
  expect(snapshot.context.budget.clause).toMatchObject({kind:'work',limit:50_000,observed:false});
  expect(snapshot.context.budget.clause.spent).toBeTypeOf('number');
  expect(snapshot.tempError).toBeNull();expect(snapshot.temp).toBeTypeOf('string');
  const read=spawnSync('python3',['-c',`import sqlite3,sys,json
c=sqlite3.connect(sys.argv[1]);tables=c.execute("SELECT name FROM sqlite_schema WHERE type='table' AND name LIKE '%events'").fetchall();assert len(tables)==1,tables
print(json.dumps(c.execute('SELECT name FROM "'+tables[0][0].replace('"','""')+'"').fetchall()))`,join(directory,snapshot.main)],{encoding:'utf8',timeout:5_000});
  expect(read.status,read.stderr).toBe(0);expect(JSON.parse(read.stdout)).toEqual([['éxx']]);
 }finally{rmSync(directory,{recursive:true,force:true});}
},35_000);
