import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { expect,it } from "vitest";
import { readBundledCatalog } from "../../../src/engine/index.js";
import { TenReplay,canonical,readTenOracle } from "../../../conformance/data-layer/batch-ten.js";
const oracle=readTenOracle("fixtures/host/data-layer-port/batch-10-complete-api.json.gz");
const checks=["graph_lazy_sql_plan","bounded_composed_graph_paths","graphql_plan_ownership","plan_optimizer_v2","conditional_architecture","schema_introspection_data_paths","matching_architecture","hnsw_architecture","embedding_vector_orchestration","generated_index_sources"];
// The full 2,871-file assertion runs separately in each API, with its receipt below.
const unique=[...new Map(oracle.records.filter(r=>!r.test.includes("test_current_matching_architecture_is_clean_tdd_red")).map(r=>[JSON.stringify({...r,test:undefined}),r])).values()];
it("retains unchanged upstream assertions, complete observations and source digests",()=>{
 expect(Object.keys(oracle.cases)).toHaveLength(1098);
 expect(oracle.records).toHaveLength(975);
 for(const [digest,source]of Object.entries(oracle.sources))expect(createHash("sha256").update(source).digest("hex")).toBe(digest);
 for(const check of checks){const records=oracle.records.filter(r=>r.check===check);expect(records.some(r=>Array.isArray(r.expected)&&r.expected.length>0)).toBe(true);expect(records.some(r=>Array.isArray(r.expected)&&r.expected.length===0)).toBe(true);}
 expect(oracle.records.some(r=>r.test.includes("current_matching_architecture_is_clean")&&r.files.length===2871)).toBe(true);
 expect(oracle.records.filter(r=>r.entry==="conditional_architecture_findings")).toHaveLength(47);
});
it.each(checks)("matches complete %s source-adapter observations through both native APIs",async check=>{
 const replay=new TenReplay();try{for(const record of unique.filter(r=>r.check===check))for(const mode of["snapshot","workspace"]as const){const expected=check==="graph_lazy_sql_plan"?canonical(record.expected):record.expected;expect(await replay.run(oracle,record,mode),`${mode}: ${record.test}`).toEqual(expected);}}finally{replay.close();}
},90_000);
it.each(checks)("rejects a disabled %s detector on a recorded positive",async check=>{
 const record=oracle.records.find(r=>r.check===check&&Array.isArray(r.expected)&&r.expected.length>0)!;const replay=new TenReplay("packs/data-layer/rules",true);
 try{expect(await replay.run(oracle,record,"snapshot")).not.toEqual(record.expected);}finally{replay.close();}
});
it("ships the ten documents once in the private pack",()=>{
 const pack=readBundledCatalog(["packs/data-layer"]);expect(pack.documents.size).toBe(30);for(const check of checks)expect(pack.documents.has(`data-layer-${check.replaceAll("_","-")}`)).toBe(true);
});

it("binds complete runs and positive controls to the batch implementation",()=>{
 const receipt=JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-native-assertions.json","utf8"));
 expect(receipt.format).toBe("rerail.data-layer-batch-ten-assertions/v3");
 for(const mode of["snapshot","workspace"]){expect(receipt.executionPaths[mode]).toMatchObject({cases:1098,completeResults:975,positiveResults:870,distinctObservations:778,distinctPositiveObservations:680,detectorCases:974,retainedPythonCases:124,semanticFallbacks:[]});expect(receipt.executionPaths[mode].retainedPythonCaseIds).toHaveLength(124);expect(receipt.executionPaths[mode].engineSha256).toBe(receipt.engineSha256);expect(receipt.executionPaths[mode].originalCalls["matching_architecture:_source_requires_matching_scan"]).toBeUndefined();}
 expect(receipt.configured).toMatchObject({observations:1239,apiComparisons:2478,selectionComparisons:716,assertions:3194});
 expect(receipt.positiveControls.rejected).toEqual([...checks].sort());
 for(const[path,digest]of Object.entries(receipt.fileSha256))expect(createHash("sha256").update(readFileSync(path)).digest("hex"),path).toBe(digest);
});
