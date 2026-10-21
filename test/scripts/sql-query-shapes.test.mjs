import assert from "node:assert/strict";
import { test } from "node:test";
import { summarizeCases } from "../../benchmark/sql-query-shapes.mjs";

test("shape comparisons pair snapshots, exclude warmups, and retain failed and unbatched shapes", () => {
  const sample = (variant,cpuNs,round=0,extras={}) => ({ variant,round,metrics:{cpuNs,parity:true,patternFilters:1,orderedParity:true,...extras} });
  const fixture = (id,cost,extra) => ({case:id,family:"pattern-terminal",report:{stat1Restored:true,setup:[],analyze:[],samples:[
    sample("baseline",cost),sample("baseline",cost*100,null),sample("inline",cost/2,0,{patternFilters:20,orderedParity:false}),...extra,
  ]}});
  const summary = summarizeCases([fixture(1,100,[sample("fence",80)]),fixture(2,1000,[sample("fence",0,0,{parity:false,error:"interrupted"})])]);
  const fence = summary.results.find(r=>r.variant==="fence");
  assert.equal(fence.samples,1);
  assert.equal(fence.cpuRatio,0.8);
  assert.equal(fence.failures.length,1);
  const inline = summary.results.find(r=>r.variant==="inline");
  assert.equal(inline.cpuRatio,0.5);
  assert.equal(inline.unbatchedCases,2);
  assert.equal(inline.reorderedCases,2);
  assert.equal(summary.results.find(r=>r.variant==="baseline").cpuNs,1100);
  assert.equal(summary.issues.length,1);
});

test("ANALYZE setup errors remain distinct from query parity and preparation costs", () => {
  const metric = {cpuNs:100,prepareCpuNs:20,parity:true,orderedParity:true};
  const summary = summarizeCases([{case:0,family:"sibling-walk",report:{stat1Restored:true,setup:[],
    samples:[{variant:"baseline",round:0,metrics:metric}],analyze:[{variant:"analyze-node",setup:{cpuNs:30,error:"no such table: node"},samples:[{variant:"analyze-node",round:0,metrics:metric}]}]}}]);
  const probe = summary.results.find(r=>r.variant==="analyze-node");
  assert.equal(probe.setupErrors,1);
  assert.equal(probe.prepareCpuNs,20);
  assert.equal(probe.cpuRatio,1);
  assert.equal(summary.issues.length,0);
});

test("budget rejections are distinct from mismatches and still flag per-row terminals", () => {
  const baseline={variant:"baseline",round:0,metrics:{cpuNs:100,parity:true,factFilters:1}};
  const rejected=round=>({variant:"inline",round,metrics:{cpuNs:1e9,parity:null,error:"interrupted",factFilters:1000}});
  const summary=summarizeCases([{case:0,family:"fact-terminal",report:{stat1Restored:true,setup:[],analyze:[],samples:[baseline,rejected(null),rejected(0)]}}]);
  assert.deepEqual(summary.quality,{budgetLimitedSnapshots:1,mismatchedSnapshots:0,unexpectedFailures:0});
  const inline=summary.results.find(r=>r.variant==="inline");
  assert.equal(inline.samples,0);
  assert.equal(inline.cpuRatio,null);
  assert.equal(inline.unbatchedCases,1);
});
