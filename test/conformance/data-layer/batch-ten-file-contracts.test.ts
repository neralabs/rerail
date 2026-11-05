import {readFileSync} from "node:fs";
import {expect,it} from "vitest";
import {TenReplay,canonical,type TenRecord} from "../../../conformance/data-layer/batch-ten.js";
const cases=JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-file-contract-regressions.json","utf8")).cases as (TenRecord&{production:boolean})[];
it.each(cases)("preserves raw files and complete original results: $test",async record=>{
 const replay=new TenReplay();try{for(const mode of["snapshot","workspace"]as const){
  const expected=record.production||record.check==="graph_lazy_sql_plan"?canonical(record.expected):record.expected;
  expect(await replay.run({cases:{},sources:{},records:[]},record,mode,record.production),mode).toEqual(expected);
 }}finally{replay.close();}
});
