import { readFileSync } from "node:fs";
import { expect,it } from "vitest";
import { TenReplay,canonical,type TenOracle } from "../../../conformance/data-layer/batch-ten.js";
import type { TenRecord } from "../../../conformance/data-layer/batch-ten.js";
const oracle=JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-configured-defaults.json","utf8")) as TenOracle;
const checks=[...new Set(oracle.records.map(r=>r.check))];
it.each(checks)("preserves configured %s defaults, discovery and public diagnostics",async check=>{
 const replay=new TenReplay();try{for(const record of oracle.records.filter(r=>r.check===check))for(const mode of["snapshot","workspace"]as const)expect(await replay.run(oracle,record,mode,true),`${mode}: ${record.test}`).toEqual(canonical(record.expected));}finally{replay.close();}
},90_000);
it("selects matching files natively with the original raw-source predicate",async()=>{
 const replay=new TenReplay();try{for(const record of oracle.records.filter(r=>r.check==="matching_architecture") as (TenRecord&{selected:string[]})[])for(const mode of["snapshot","workspace"]as const)expect(await replay.run(oracle,record,mode,true,true),`${mode}: ${record.test}`).toEqual(record.selected);}finally{replay.close();}
});
