import {expect,it} from "vitest";
import {TenReplay,canonical,readTenOracle} from "../../../conformance/data-layer/batch-ten.js";
const oracle=readTenOracle("fixtures/host/data-layer-port/batch-10-configured-defaults.json.gz");
it.each(["snapshot","workspace"] as const)("configured repository %s uses the shipped matching rule",async mode=>{
 const records=oracle.records.filter(record=>record.files.length>200);expect(records).toHaveLength(1);
 const replay=new TenReplay();try{for(const record of records)expect(await replay.run(oracle,record,mode,true)).toEqual(canonical(record.expected));}finally{replay.close();}
},110_000);
