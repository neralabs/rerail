import { describe, expect, it } from "vitest";
import { compareSides, coverage } from "../benchmark/query-parity.js";

describe("query parity comparison", () => {
  it("reports different incremental edit applies", () => {
    const differences = compareSides([
      {
        side: { name: "reference", addon: "/reference.node", environment: {} },
        workloads: { edits: { applies: [{ findings: { value: [{ id: "before" }] } }] } },
      },
      {
        side: { name: "candidate", addon: "/candidate.node", environment: {} },
        workloads: { edits: { applies: [{ findings: { value: [{ id: "after" }] } }] } },
      },
    ]);

    expect(differences).toEqual([expect.objectContaining({ side: "candidate", workload: "edits", section: "applies" })]);
  });

  it("ignores raw edit hashes and finding order when applies are semantically equal", () => {
    const differences = compareSides([
      {
        side: { name: "reference", addon: "/reference.node", environment: {} },
        workloads: {
          edits: {
            applies: [{
              status: "completed",
              findings: { value: [{ id: "a", message: "first" }, { id: "b", message: "second" }] },
              failures: { value: [] },
              coverage: { value: [] },
              phaseAFindingSha256: "sha256:reference-order",
            }],
          },
        },
      },
      {
        side: { name: "candidate", addon: "/candidate.node", environment: {} },
        workloads: {
          edits: {
            applies: [{
              status: "completed",
              findings: { value: [{ message: "second", id: "b" }, { message: "first", id: "a" }] },
              failures: { value: [] },
              coverage: { value: [] },
              phaseAFindingSha256: "sha256:candidate-order",
            }],
          },
        },
      },
    ]);

    expect(differences).toEqual([]);
  });
});

describe("query parity coverage", () => {
  it("counts failures for their policy and check", () => {
    const records = coverage([
      { path: "policy.json", source: JSON.stringify({ id: "policy-a" }), clauses: [{ id: "check-a" }, { id: "check-b" }] },
    ], [
      { ruleId: "policy-a", clauseId: "check-a", code: "source-partial", message: "incomplete source" },
      { ruleId: "policy-a", clauseId: "check-a", code: "budget-exceeded", message: "budget exhausted" },
    ]);

    expect(records).toEqual([
      { policyId: "policy-a", checkId: "check-a", status: "partial", failureCount: 2 },
      { policyId: "policy-a", checkId: "check-b", status: "complete", failureCount: 0 },
    ]);
  });
});
