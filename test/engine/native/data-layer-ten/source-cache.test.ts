import {expect,it} from "vitest";
import {readFileSync} from "node:fs";
import {TenReplay,type TenRecord} from "../../../../conformance/data-layer/batch-ten.js";
it("retains canonical source proofs when SQL reuses prepared rows",async()=>{
 const cases=JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-file-contract-regressions.json","utf8")).cases as TenRecord[];
 const record=cases.find(c=>c.test==="matching-explicit-auxiliary-False")!;
 const replay=new TenReplay();
 try{for(const mode of ["snapshot","snapshot","workspace"]as const)expect(await replay.run({cases:{},sources:{},records:[]},record,mode)).toEqual(record.expected);}finally{replay.close();}
});
