import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/index.js";
const native=loadNativeBinding();
const profiled="NativeSqlSpike" in native;
const caps=readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json",import.meta.url),"utf8");
const source="class A:\n def f(self):\n  def inner(): return 1\n  return inner()\n";
function compare(predicate:string, empty=false) {
  const policy={format:"rerail.policy/v1",id:"completed",revision:1,mode:"log",clauses:[{id:"rows",relational:{
    inputs:{gate:{evidence:{as:"f",query:{ast:{language:"python",syntax:"valid",rule:{kind:"module"}}},project:{path:"file.path"}}}},
    syntax:{name:"nodes",gate:"gate",fields:["name"]},
    stages:[{name:"descendants",reachability:{syntax:"nodes",roots:empty?"SELECT __path,id FROM nodes WHERE 0":"SELECT __path,id FROM nodes WHERE kind='function_definition'"}}],
    query:`SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,d.root,d.node FROM descendants d JOIN nodes n ON n.__path=d.__path AND n.id=d.node WHERE ${predicate}`,
    key:["root","node"]},violation:{code:"ROW",message:"Row",anchor:"witness.span"}}]};
  const rule=native.CodeRuleDslRuntime.compile(JSON.stringify(policy),"json",caps);
  const input=JSON.stringify({format:"rerail.repository-snapshot/v1",revision:0,files:["a.py","b.py"].map(path=>({path,source,language:"python"}))});
  const old=process.env.RERAIL_SQL_COMPLETED_OUTPUT;
  const run=(mode:string)=>{process.env.RERAIL_SQL_COMPLETED_OUTPUT=mode;const r=JSON.parse(rule.evaluate(input,"report"));expect(r.failures).toEqual([]);return r.violations.map((v:unknown)=>JSON.stringify(v)).sort();};
  try {const expected=run("table");expect(run("virtual")).toEqual(expected);return expected.length;}
  finally {if(old===undefined)delete process.env.RERAIL_SQL_COMPLETED_OUTPUT;else process.env.RERAIL_SQL_COMPLETED_OUTPUT=old;}
}
it.skipIf(!profiled)("publishes complete forward and reverse rows with file-qualified identities",()=>{
  expect(compare("1")).toBeGreaterThan(10);
  expect(compare("d.node IN (SELECT id FROM nodes WHERE kind='identifier')")).toBeGreaterThan(0);
});
it.skipIf(!profiled)("retains SQLite real, text affinity, null and collation decisions",()=>{
  expect(compare("d.root IN (SELECT CAST(id AS REAL) FROM nodes WHERE kind='function_definition')")).toBeGreaterThan(0);
  expect(compare("EXISTS(SELECT 1 FROM (SELECT CAST(id AS TEXT) AS id,__path FROM nodes) t WHERE t.__path=d.__path AND t.id=d.root)")).toBe(0);
  expect(compare("d.root=NULL")).toBe(0);
  expect(compare("d.__path COLLATE NOCASE='A.PY'")).toBeGreaterThan(0);
});
it.skipIf(!profiled)("completes empty output and deduplicates indexed IN requests",()=>{
  expect(compare("1",true)).toBe(0);
  expect(compare("d.root IN (SELECT id FROM nodes UNION ALL SELECT id FROM nodes)")).toBeGreaterThan(0);
});
