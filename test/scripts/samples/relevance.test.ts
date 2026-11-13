import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { NEAR_MISS_CAP, candidateRule, nearMissCandidates, relaxedDocument, selectRelevant } from "../../../scripts/samples/relevance.js";
import { sampleKey, sha256, type Sample } from "../../../scripts/samples/samples.js";

function sample(path: string, text: string, expected: [string, { line: number }[]][], pinned = false): Sample {
  const bytes = Buffer.from(text, "utf8");
  return {
    toolPack: { pack: "python", tool: "bandit" },
    short: "demo",
    source: { repo: "owner/demo", commit: "0".repeat(40), path, license: "MIT", sha256: sha256(bytes) },
    bytes,
    labelledBy: { tool: "bandit", version: "1.9.4", how: "port-readiness recording" },
    expected: new Map(expected),
    context: [],
    pinned,
    origins: ["golden"],
  };
}

describe("relaxed queries", () => {
  it("keep the node a clause anchors on and drop every condition on it", () => {
    const rule = { kind: "call", has: { field: "function", regex: "^eval$" }, not: { inside: { kind: "lambda" } } };
    expect(candidateRule(rule, {})).toEqual({ kind: "call" });
    expect(candidateRule({ all: [{ kind: "call" }, { pattern: "$F($$$)" }, { not: { kind: "x" } }] }, {})).toEqual({ all: [{ kind: "call" }, { pattern: "$F($$$)" }] });
  });

  it("inline utilities, and widen an any whose alternative anchors on no node", () => {
    expect(candidateRule({ matches: "callee" }, { callee: { any: [{ kind: "call" }, { kind: "attribute" }] } })).toEqual({ any: [{ kind: "call" }, { kind: "attribute" }] });
    expect(candidateRule({ kind: "call", any: [{ kind: "x" }, { has: { kind: "y" } }] }, {})).toEqual({ kind: "call" });
  });

  it("anchor a rule with no node of its own on the node its relation needs", () => {
    expect(candidateRule({ regex: "^TODO", inside: { kind: "comment" } }, {})).toEqual({ kind: "comment" });
    expect(candidateRule({ regex: "^x$" }, {})).toBeUndefined();
  });

  it("turn each AST clause into a probe, including each case of an option, and no text clause", () => {
    const document = {
      format: "rerail.policy/v1",
      id: "demo",
      scope: { include: ["**/*.py"], exclude: [] },
      clauses: [
        { id: "a", forbid: { ast: { language: "python", rule: { kind: "assert_statement", has: { kind: "x" } }, when: { x: 1 } } }, violation: { code: "A" } },
        { id: "b", forbid: { ast: { $option: "o", cases: { true: { language: "python", rule: { kind: "call" } } }, default: { language: "python", rule: { kind: "lambda" } } } }, violation: { code: "A" } },
        { id: "c", forbid: { regex: { source: "file.content", pattern: "x" } }, violation: { code: "A" } },
      ],
    };
    const relaxed = relaxedDocument(document)!;
    expect(relaxed.scope).toEqual(document.scope);
    expect((relaxed.clauses as { forbid: { ast: unknown } }[]).map((clause) => clause.forbid.ast)).toEqual([
      { language: "python", rule: { kind: "assert_statement" } },
      { language: "python", rule: { kind: "call" } },
      { language: "python", rule: { kind: "lambda" } },
    ]);
    expect(relaxedDocument({ clauses: [document.clauses[2]] })).toBeUndefined();
  });

  it("select, through the engine, the files holding a candidate node", async () => {
    const directory = mkdtempSync(join(tmpdir(), "relevance-"));
    try {
      const policy = join(directory, "rule.json");
      writeFileSync(policy, JSON.stringify({
        format: "rerail.policy/v1", id: "demo-eval", revision: 1, mode: "steer", description: "Avoid eval.",
        scope: { include: ["**/*.py"], exclude: [] },
        clauses: [{ id: "eval", forbid: { ast: { language: "python", rule: { kind: "call", has: { field: "function", regex: "^eval$" } } } }, violation: { anchor: "witness.span", code: "E", message: "no eval" } }],
      }));
      const calls = sample("a.py", "print(1)\n", [["demo-eval", []]]);
      const none = sample("b.py", "x = 1\n", [["demo-eval", []]]);
      const { selected, probe } = await nearMissCandidates(policy, [calls, none]);
      expect(probe).toBe("relaxed query");
      expect([...selected]).toEqual([sampleKey(calls)]);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});

describe("selectRelevant", () => {
  it("keeps a file with a labelled violation whole, clean labels included", () => {
    const positive = sample("p.py", "p", [["r1", [{ line: 1 }]], ["r2", []]]);
    const { samples, clean } = selectRelevant([positive], new Map());
    expect(samples).toEqual([positive]);
    expect(clean.get(sampleKey(positive))?.get("r2")).toEqual({ kept: true, reason: "on a file with a labelled violation" });
  });

  it("keeps the smallest near misses up to the cap, by size then sha256, each with only the rules that kept it", () => {
    const near = Array.from({ length: NEAR_MISS_CAP + 2 }, (_, index) => sample(`n${index}.py`, "x".repeat(10 + index), [["r1", []], ["r2", []]]));
    const far = sample("far.py", "y", [["r1", []], ["r2", []]]);
    const nearMisses = new Map([["r1", new Set(near.map(sampleKey))], ["r2", new Set<string>()]]);
    const { samples, clean } = selectRelevant([...near].reverse().concat(far), nearMisses);
    const kept = samples.map((entry) => entry.source.path);
    expect(kept).toEqual(["far.py", "n0.py", "n1.py", "n2.py", "n3.py", "n4.py"]);
    // r2 has no near miss: its one smallest clean file is kept, carrying r2 alone.
    expect(Object.fromEntries(samples.find((entry) => entry.source.path === "far.py")!.expected)).toEqual({ r2: [] });
    expect(Object.fromEntries(samples.find((entry) => entry.source.path === "n0.py")!.expected)).toEqual({ r1: [] });
    expect(clean.get(sampleKey(near[5]!))?.get("r1")).toEqual({ kept: false, reason: "over the cap" });
    expect(clean.get(sampleKey(far))?.get("r1")).toEqual({ kept: false, reason: "not relevant" });
    expect(clean.get(sampleKey(far))?.get("r2")).toEqual({ kept: true, reason: "smallest clean file (no near miss)" });
  });

  it("keeps a chosen clean sample (the ESLint test set's fixed copy) whatever its relevance", () => {
    const fixed = sample("f.ts", "f", [["r1", []]], true);
    const { samples, clean } = selectRelevant([fixed], new Map());
    expect(samples).toEqual([fixed]);
    expect(clean.get(sampleKey(fixed))?.get("r1")).toEqual({ kept: true, reason: "a chosen clean sample" });
  });
});
