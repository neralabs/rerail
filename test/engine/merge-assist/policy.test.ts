import { describe, expect, it } from "vitest";
import {
  type CheckResult,
  type ContractBreak,
  decideCriteria,
  decideHardChecks,
  decidePanel,
  defaultPolicy,
  parsePolicy,
  reservedHits,
  type SideMeasures,
} from "../../../src/engine/merge-assist/policy.js";
import { questionOf, sanitizeIntent, stripCrossReferences } from "../../../src/engine/merge-assist/conflict-record.js";

function check(name: string, status: CheckResult["status"], kind: CheckResult["kind"] = "tests"): CheckResult {
  return { name, kind, status, detail: "", log: "" };
}

function contractBreak(overrides: Partial<ContractBreak> = {}): ContractBreak {
  return { path: "src/api.ts", name: "fetchUser", change: "changed", before: "export function fetchUser(id: string)", after: "export function fetchUser(id: number)", public: true, dependents: ["src/app.ts"], declared: false, ...overrides };
}

function measures(overrides: Partial<SideMeasures> = {}): SideMeasures {
  return { callersAffected: 0, sourceLines: 40, sourceFiles: 2, benchmarks: {}, ...overrides };
}

describe("parsePolicy", () => {
  it("fills every default from a bare version", () => {
    expect(defaultPolicy()).toEqual({
      version: 1,
      steps: ["precedent", "hard-checks", "criteria", "panel"],
      checks: { setup: null, run: [], timeoutMinutes: 30 },
      contracts: { enabled: true, declared: [] },
      criteria: { compatibility: { enabled: true, minDifference: 1 }, blastRadius: { enabled: true, ratio: 3, minLines: 100 }, benchmarks: [] },
      panel: { judges: 3, agreement: 3 },
      reserve: ["public-api", "security", "data-model"],
    });
  });

  it("reads a full policy and defaults agreement to every judge", () => {
    const parsed = parsePolicy([
      "version: 1",
      "steps: [hard-checks, panel]",
      "checks:",
      "  setup: npm ci",
      "  run:",
      "    - { name: types, kind: typecheck, run: npx tsc --noEmit }",
      "    - { name: unit, kind: tests, run: npm test }",
      "criteria:",
      "  benchmarks:",
      "    - { name: parse, run: node bench.js, better: lower }",
      "panel: { judges: 5 }",
      "reserve: [security]",
    ].join("\n"));
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.policy.steps).toEqual(["hard-checks", "panel"]);
    expect(parsed.policy.checks.run.map(entry => entry.name)).toEqual(["types", "unit"]);
    expect(parsed.policy.criteria.benchmarks[0]).toEqual({ name: "parse", run: "node bench.js", better: "lower", minChange: 0.05 });
    expect(parsed.policy.panel).toEqual({ judges: 5, agreement: 5 });
    expect(parsed.policy.reserve).toEqual(["security"]);
  });

  it("refuses steps out of order, unknown keys, duplicate keys and an agreement above the panel", () => {
    const problems = (text: string) => {
      const parsed = parsePolicy(text);
      return parsed.ok ? [] : parsed.problems;
    };
    expect(problems("version: 1\nsteps: [panel, precedent]")).toEqual([expect.stringContaining("steps:")]);
    expect(problems("version: 1\nreview: true")).toEqual([expect.stringContaining("review")]);
    expect(problems("version: 1\nversion: 1")[0]).toMatch(/^not YAML/u);
    expect(problems("version: 1\npanel: { judges: 3, agreement: 4 }")).toEqual([expect.stringContaining("panel")]);
    expect(problems("version: 2")).toEqual([expect.stringContaining("version")]);
  });
});

describe("sanitizeIntent", () => {
  it("drops cross-references, supersede and outcome sentences, and template comments", () => {
    const text = [
      "Move retries into the transport. <!-- Describe your change -->",
      "This supersedes #120. It keeps callers simple.",
      "Closes #99",
      "See https://github.com/acme/app/pull/120 for the earlier attempt.",
      "Fixes acme/app#7 and GH-8.",
    ].join("\n");
    expect(sanitizeIntent(text)).toBe("Move retries into the transport.\nIt keeps callers simple.\nSee for the earlier attempt.\nFixes and .");
  });

  it("keeps code-like hashes out of reach of the reference filter only in diffs it is asked to clean", () => {
    expect(stripCrossReferences("+ changelog: fix (#4521)\n+ const merged = true;")).toBe("+ changelog: fix \n+ const merged = true;");
    expect(sanitizeIntent("const y = computed(() => x.value.y) // runs: [] (#12, #14)")).toBe("const y = computed(() => x.value.y) // runs: []");
  });
});

describe("questionOf", () => {
  it("keys the overlap the same whichever side is a", () => {
    const left = { files: ["src/a.ts", "src/b.ts"], symbols: ["fetchUser"] };
    const right = { files: ["src/b.ts", "src/c.ts"], symbols: ["fetchUser", "other"] };
    expect(questionOf(left, right)).toBe(questionOf(right, left));
    expect(questionOf(left, right)).not.toBe(questionOf(left, { files: ["src/a.ts"], symbols: [] }));
  });
});

describe("decideHardChecks", () => {
  it("lets the side that passes win over the side that fails the same check", () => {
    const result = decideHardChecks({ a: [check("unit", "fail")], b: [check("unit", "pass")] }, null);
    expect(result.winner).toBe("b");
    expect(result.reason).toContain("Side a fails unit");
  });

  it("decides nothing when both fail, or one is unknown", () => {
    expect(decideHardChecks({ a: [check("unit", "fail")], b: [check("unit", "fail")] }, null).winner).toBeNull();
    expect(decideHardChecks({ a: [check("unit", "fail")], b: [check("unit", "unknown")] }, null).winner).toBeNull();
    expect(decideHardChecks({ a: [check("unit", "fail"), check("types", "pass", "typecheck")], b: [check("unit", "pass"), check("types", "fail", "typecheck")] }, null).winner).toBeNull();
  });

  it("holds an undeclared public break with dependents against a side, and nothing else", () => {
    expect(decideHardChecks(null, { a: [contractBreak()], b: [] }).winner).toBe("b");
    expect(decideHardChecks(null, { a: [contractBreak({ declared: true })], b: [] }).winner).toBeNull();
    expect(decideHardChecks(null, { a: [contractBreak({ public: false })], b: [] }).winner).toBeNull();
    expect(decideHardChecks(null, { a: [contractBreak({ dependents: [] })], b: [] }).winner).toBeNull();
    expect(decideHardChecks(null, { a: [contractBreak()], b: [contractBreak()] }).winner).toBeNull();
  });
});

describe("decideCriteria", () => {
  const criteria = defaultPolicy().criteria;

  it("decides by compatibility first, then blast radius, then benchmarks", () => {
    expect(decideCriteria(criteria, { a: measures({ callersAffected: 3 }), b: measures({ sourceLines: 900 }) }).reason).toMatch(/^Compatibility: side b/u);
    expect(decideCriteria(criteria, { a: measures({ sourceLines: 400 }), b: measures({ sourceLines: 100 }) }).winner).toBe("b");
    expect(decideCriteria(criteria, { a: measures({ sourceLines: 90 }), b: measures({ sourceLines: 10 }) }).winner).toBeNull();
    const benchmarked = { ...criteria, benchmarks: [{ name: "parse", run: "x", better: "lower" as const, minChange: 0.05 }] };
    expect(decideCriteria(benchmarked, { a: measures({ benchmarks: { parse: 100 } }), b: measures({ benchmarks: { parse: 80 } }) }).winner).toBe("b");
    expect(decideCriteria(benchmarked, { a: measures({ benchmarks: { parse: 100 } }), b: measures({ benchmarks: { parse: 98 } }) }).winner).toBeNull();
    expect(decideCriteria(benchmarked, { a: measures({ benchmarks: { parse: null } }), b: measures({ benchmarks: { parse: 80 } }) }).winner).toBeNull();
  });
});

describe("decidePanel and reservedHits", () => {
  it("needs the agreement, and counts abstentions for neither side", () => {
    expect(decidePanel(["a", "a", "a"], 3).winner).toBe("a");
    expect(decidePanel(["a", "a", null], 3).winner).toBeNull();
    expect(decidePanel(["b", "b", "a"], 2).winner).toBe("b");
  });

  it("reports the classes a side falls in that the policy reserves", () => {
    expect(reservedHits(["public-api", "security"], { a: ["security", "data-model"], b: [] })).toEqual(["security"]);
    expect(reservedHits([], { a: ["security"], b: ["public-api"] })).toEqual([]);
  });
});
