import {readFileSync} from "node:fs";
import {expect,it} from "vitest";
import {HelperReplay,type HelperRecord} from "../../../conformance/data-layer/helper-assertions.js";
const oracle=JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-helper-assertions.json","utf8")) as {cases:Record<string,unknown>;records:HelperRecord[];supplemental:HelperRecord[]};
oracle.records.push(...oracle.supplemental);
for(const entry of [...new Set(oracle.records.map(record=>record.entry))]){
 it(`matches original helper ${entry} through both native APIs`,async()=>{
  const replay=new HelperReplay();try{for(const record of oracle.records.filter(record=>record.entry===entry))for(const mode of ["snapshot","workspace"]as const)expect(await replay.run(record,mode),`${mode}: ${record.test}`).toEqual(record.expected);}finally{replay.close();}
 },90_000);
 it(`rejects disabled native helper ${entry}`,async()=>{
  const record=oracle.records.find(record=>record.entry===entry&&Boolean(record.expected)&&(!Array.isArray(record.expected)||record.expected.length>0))??oracle.records.find(record=>record.entry===entry)!;
  const replay=new HelperReplay();try{
   let rejected=false;try{rejected=JSON.stringify(await replay.run(record,"snapshot",true))!==JSON.stringify(record.expected);}catch(error){if(!(error instanceof Error)||!/^Native (syntax|Boolean) contract did not publish its result$/.test(error.message))throw error;rejected=true;}
   expect(rejected,record.test).toBe(true);
  }finally{replay.close();}
 });
}
