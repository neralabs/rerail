import {readFileSync} from "node:fs";
import {expect,it} from "vitest";
import {TenReplay,canonical,type TenRecord} from "../../../conformance/data-layer/batch-ten.js";
const cases=JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-repair-regressions.json","utf8")).cases as (TenRecord&{production:boolean})[];
it.each(cases)("preserves the original result for $test",async record=>{
 const replay=new TenReplay();try{for(const mode of["snapshot","workspace"]as const){
  const values=await replay.run({cases:{},sources:{},records:[]},record,mode,record.production) as {code?:string;rule?:string;line:number;message:string}[];
  expect(canonical(values.map(r=>({code:r.code??r.rule,line:r.line,message:r.message}))),mode).toEqual(canonical(record.expected));
 }}finally{replay.close();}
});
