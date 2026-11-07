import {readFileSync} from "node:fs";
import {expect,it} from "vitest";
import {TenReplay,canonical,type TenRecord} from "../../../conformance/data-layer/batch-ten.js";
const cases=JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-file-contract-regressions.json","utf8")).cases as (TenRecord&{production:boolean})[];
for(const record of cases)it(`preserves raw files and complete original results: ${record.test}`,async()=>{
 const replay=new TenReplay();try{for(const mode of["snapshot","workspace"]as const){
  const expected=record.production||record.check==="graph_lazy_sql_plan"?canonical(record.expected):record.expected;
  const actual=await replay.run({cases:{},sources:{},records:[]},record,mode,record.production);
  expect(actual,`${mode}: ${record.test}\nexpected ${JSON.stringify(expected)}\nreceived ${JSON.stringify(actual)}`).toEqual(expected);
 }}finally{replay.close();}
});
it("retains canonical source proofs when SQL reuses prepared rows",async()=>{
 const record=cases.find(c=>c.test==="matching-explicit-auxiliary-False")!;
 const replay=new TenReplay();
 try{for(const mode of ["snapshot","snapshot","workspace"]as const)expect(await replay.run({cases:{},sources:{},records:[]},record,mode)).toEqual(record.expected);}finally{replay.close();}
});
