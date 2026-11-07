import {expect,it} from "vitest";
import {TenReplay,canonical,observedCode,positiveFamilies,readTenOracle} from "../../../conformance/data-layer/batch-ten.js";
const oracle=readTenOracle("fixtures/host/data-layer-port/batch-10-complete-api.json.gz");
it.each(positiveFamilies(oracle))("detects removal of $check diagnostic $code",async({code,record})=>{
 const replay=new TenReplay("packs/data-layer/rules",code);
 const expected=(record.expected as unknown[]).filter(finding=>observedCode(record,finding)!==code);
 try{for(const mode of["snapshot","workspace"]as const)expect(await replay.run(oracle,record,mode),mode).toEqual(record.check==="graph_lazy_sql_plan"?canonical(expected):expected);}finally{replay.close();}
});
