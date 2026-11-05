import {afterAll,expect,it} from "vitest";
import {TenReplay,canonical,readTenOracle,type TenRecord} from "../../../conformance/data-layer/batch-ten.js";
const oracle=readTenOracle("fixtures/host/data-layer-port/batch-10-configured-defaults.json.gz");
const replays=new Map<string,TenReplay>();
const replay=(check:string)=>{let value=replays.get(check);if(!value){value=new TenReplay();replays.set(check,value);}return value;};
afterAll(()=>{for(const value of replays.values())value.close();});
const observations=oracle.records.filter(record=>record.files.length<=200).flatMap(record=>["snapshot","workspace"].map(mode=>({record,check:record.check,test:record.test,mode:mode as "snapshot"|"workspace"})));
it.each(observations)("configured $check $mode: $test",async({record,mode,check})=>{
 expect(await replay(check).run(oracle,record,mode,true)).toEqual(canonical(record.expected));
},90_000);
const selections=oracle.records.filter(record=>record.check==="matching_architecture") as (TenRecord&{selected:string[]})[];
it.each(selections.flatMap(record=>["snapshot","workspace"].map(mode=>({record,test:record.test,mode:mode as "snapshot"|"workspace"}))))("selection matching_architecture $mode: $test",async({record,mode})=>{
 expect(await replay("matching_architecture").run(oracle,record,mode,true,true)).toEqual(record.selected);
},90_000);
