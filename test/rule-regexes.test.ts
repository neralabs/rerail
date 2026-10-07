import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { assertKernelRegexes, kernelRegexIssues } from "../src/rule-regexes.js";
import { parseCosineRule, type CosineRule, type MatchCondition } from "../src/rule-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
import { MOCK_COSINE_RULES } from "../src/seed.js";

const LOOK_AROUND = /^is not a valid regex: .*look-around/su;
const command = (regex: string): MatchCondition => ({ field: "input.command", regex });

function structural(match: unknown): CosineRule {
  return parseCosineRule({
    id: "structural",
    mode: "steer",
    match,
    sample: { source: "tool:Bash", boundary: "tool-call" },
    action: { content: "c" },
  });
}

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function directory(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "rerail-rule-regexes-"));
  directories.push(path);
  return path;
}

describe("kernel regex validation", () => {
  it("locates every pattern the kernel refuses, however deeply nested", () => {
    const rule = structural({
      all: [
        command("\\bgit\\b"),
        { any: [command("(?!main)"), { not: command("(a)\\1") }] },
      ],
    });
    expect(kernelRegexIssues(rule)).toEqual([
      { path: ["match", "all", 1, "any", 0, "regex"], message: expect.stringMatching(LOOK_AROUND) },
      { path: ["match", "all", 1, "any", 1, "not", "regex"], message: expect.stringMatching(/backreferences are not supported/u) },
    ]);
    expect(() => assertKernelRegexes(rule)).toThrow(/^rule\.match\.all\.1\.any\.0\.regex is not a valid regex/u);
  });

  it("enforces the policy DSL's maxRegexPatternBytes", () => {
    expect(kernelRegexIssues(structural(command("a".repeat(16_385))))).toEqual([
      { path: ["match", "regex"], message: "is 16385 bytes; maxRegexPatternBytes allows at most 16384" },
    ]);
    expect(kernelRegexIssues(structural(command("a".repeat(16_384))))).toEqual([]);
  });

  it("has nothing to check on a similarity rule", () => {
    expect(kernelRegexIssues(MOCK_COSINE_RULES[0]!)).toEqual([]);
  });

  it("stops both rule stores writing a pattern the kernel would refuse", async () => {
    const root = await directory();
    const bad = { ...structural(command("x")), match: command("(?!main)") };

    const store = new SqliteRuleStore(join(root, "rules.sqlite"));
    expect(() => store.upsertCosine(bad)).toThrow(/^rule\.match\.regex is not a valid regex/u);
    expect(store.list()).toEqual([]);
    store.close();

    const catalog = ScopedRuleCatalog.open(join(root, "catalog.sqlite"), { repositoryId: "repo", workspaceRoot: "/repo" });
    expect(() => catalog.upsertCosine(bad)).toThrow(/^rule\.match\.regex is not a valid regex/u);
    expect(() => catalog.restore([{ document: { kind: "conversation", rule: bad } }])).toThrow(/regex/u);
    expect(catalog.listOwned()).toEqual([]);
    catalog.close();
  });
});
