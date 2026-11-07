import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { expect,it } from "vitest";
import { readBundledCatalog } from "../../../src/engine/index.js";
import { TenReplay,canonical,readTenOracle } from "../../../conformance/data-layer/batch-ten.js";
const oracle=readTenOracle("fixtures/host/data-layer-port/batch-10-complete-api.json.gz");
const checks=["graph_lazy_sql_plan","bounded_composed_graph_paths","graphql_plan_ownership","plan_optimizer_v2","conditional_architecture","schema_introspection_data_paths","matching_architecture","hnsw_architecture","embedding_vector_orchestration","generated_index_sources"];
it("requires native helper execution for every retained rule-relevant assertion",()=>{
 const receipt=JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-native-assertions.json","utf8"));
 const ledger=JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-assertion-ledger.json","utf8")).cases as Record<string,{kind:string}>;
 const wanted=Object.keys(ledger).filter(id=>ledger[id]!.kind==="native-helper").sort();
 expect(wanted).toHaveLength(67);
 for(const mode of ["snapshot","workspace"])expect(receipt.helperAssertions[mode].nativeCaseIds).toEqual(wanted);
});
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
it.each(checks.flatMap(check=>(["snapshot","workspace"]as const).flatMap(mode=>[0,1].map(part=>[check,mode,part]as const))))("matches complete %s source-adapter observations through %s API part %i",async(check,mode,part)=>{
 const replay=new TenReplay();try{for(const record of unique.filter(r=>r.check===check).filter((_,index)=>index%2===part)){const expected=check==="graph_lazy_sql_plan"?canonical(record.expected):record.expected;expect(await replay.run(oracle,record,mode),`${mode}: ${record.test}`).toEqual(expected);}}finally{replay.close();}
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
 expect(receipt.format).toBe("rerail.data-layer-batch-ten-assertions/v8");
 expect(receipt.originalSuite).toMatchObject({scannedTestFiles:4466,originalFiles:11,adjacentWorkflowCases:7,suiteIntegrationCases:175,missingDedicatedSuites:["plan_optimizer_v2"]});
 expect(receipt.originalSuite.integration).toMatchObject({originalRegistryCases:175,originalGateCases:7,nativeRegistryCounterparts:3,positiveAndCleanCommandComparisons:6,missing:[],skipped:[],failed:[]});
 for(const mode of ["snapshot","workspace"])expect(receipt.originalSuite.executionPaths[mode]).toMatchObject({cases:279,detectorCases:155,detectorObservations:156,positiveDetectorObservations:135,helperCases:67,helperObservations:104,hostCases:57,missing:[],skipped:[],failed:[]});
 for(const mode of["snapshot","workspace"]){expect(receipt.executionPaths[mode]).toMatchObject({cases:1098,completeResults:975,positiveResults:870,distinctObservations:781,distinctPositiveObservations:683,detectorCases:974,nativeHelperCases:67,retainedPythonCases:57,semanticFallbacks:[]});expect(receipt.executionPaths[mode].retainedPythonCaseIds).toHaveLength(57);expect(receipt.executionPaths[mode].engineSha256).toBe(receipt.engineSha256);expect(receipt.executionPaths[mode].originalCalls["matching_architecture:_source_requires_matching_scan"]).toBeUndefined();}
 expect(Object.keys(receipt.hostAssertionLedger)).toHaveLength(57);
 expect(receipt.dodCompletion).toMatchObject({assertions:57,lifecycleRules:10,lifecycleRevisions:57,approvalApiComparisons:28,approvalEncodingComparisons:16,matchingRootSourceComparisons:6});
 expect(receipt.cumulative).toMatchObject({status:"historical-joint-run",checks:100,codeFiles:9021,expected:25262,actual:25262,falsePositives:0,falseNegatives:0,engineFailures:0,coldOnly:true,measuredPhases:["host-prepare","compile","cold"]});
 expect(receipt.reaudit).toMatchObject({inputs:324,positiveInputs:251,apiComparisons:648,disabledDetectors:10,disabledApiComparisons:20,persistentRevisions:4,assertions:660});
 expect(receipt.repositoryRevalidation).toMatchObject({historicalUnchangedRules:99,revalidatedRules:["data-layer-hnsw-architecture"],files:9028,expected:0,actual:0,falsePositives:0,falseNegatives:0,engineFailures:0});
 expect(receipt.repositoryRevalidation.engineSha256).toBe(receipt.engineSha256);
 for(const[id,digest]of Object.entries(receipt.cumulative.policyFileSha256)){
  const path=Object.keys(receipt.fileSha256).find(path=>path.endsWith(`/rules/${id}.json`));
  expect(path,id).toBeDefined();
  expect(receipt.repositoryRevalidation.policyFileSha256[id]??digest,id).toBe(receipt.fileSha256[path!]);
 }
 expect(receipt.helperReplay).toEqual({observations:104,supplemental:15,apiComparisons:238,disabledHelpers:12,assertions:24});
 expect(receipt.configured).toMatchObject({observations:1392,positiveObservations:978,apiComparisons:2784,selectionComparisons:878,assertions:3662});
 expect(receipt.focusedContracts).toEqual({inputs:276,positiveInputs:194,apiComparisons:552,assertions:277});
 expect(receipt.positiveControls.rejected).toEqual([...checks].sort());
 for(const[path,digest]of Object.entries(receipt.fileSha256))expect(createHash("sha256").update(readFileSync(path)).digest("hex"),path).toBe(digest);
});
