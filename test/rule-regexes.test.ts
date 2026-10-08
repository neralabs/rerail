import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { assertEngineRegexes, engineRegexIssues } from "../src/rule-regexes.js";
import { parseConversationRule, type ConversationRule, type MatchCondition } from "../src/rule-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
import { MOCK_CONVERSATION_RULES } from "../src/seed.js";

const LOOK_AROUND = /^is not a valid regex: .*look-around/su;
const command = (regex: string): MatchCondition => ({ field: "input.command", regex });

function structural(match: unknown): ConversationRule {
  return parseConversationRule({
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

describe("engine regex validation", () => {
  it("locates every pattern the engine refuses, however deeply nested", () => {
    const rule = structural({
      all: [
        command("\\bgit\\b"),
        { any: [command("(?!main)"), { not: command("(a)\\1") }] },
      ],
    });
    expect(engineRegexIssues(rule)).toEqual([
      { path: ["match", "all", 1, "any", 0, "regex"], message: expect.stringMatching(LOOK_AROUND) },
      { path: ["match", "all", 1, "any", 1, "not", "regex"], message: expect.stringMatching(/backreferences are not supported/u) },
    ]);
    expect(() => assertEngineRegexes(rule)).toThrow(/^rule\.match\.all\.1\.any\.0\.regex is not a valid regex/u);
  });

  it("enforces the code-rule DSL's maxRegexPatternBytes", () => {
    expect(engineRegexIssues(structural(command("a".repeat(16_385))))).toEqual([
      { path: ["match", "regex"], message: "is 16385 bytes; maxRegexPatternBytes allows at most 16384" },
    ]);
    expect(engineRegexIssues(structural(command("a".repeat(16_384))))).toEqual([]);
  });

  it("has nothing to check on a similarity rule", () => {
    expect(engineRegexIssues(MOCK_CONVERSATION_RULES[0]!)).toEqual([]);
  });

  it("stops both rule stores writing a pattern the engine would refuse", async () => {
    const root = await directory();
    const bad = { ...structural(command("x")), match: command("(?!main)") };

    const store = new SqliteRuleStore(join(root, "rules.sqlite"));
    expect(() => store.upsertConversationRule(bad)).toThrow(/^rule\.match\.regex is not a valid regex/u);
    expect(store.list()).toEqual([]);
    store.close();

    const catalog = ScopedRuleCatalog.open(join(root, "catalog.sqlite"), { repositoryId: "repo", workspaceRoot: "/repo" });
    expect(() => catalog.upsertConversationRule(bad)).toThrow(/^rule\.match\.regex is not a valid regex/u);
    expect(() => catalog.restore([{ document: { kind: "conversation", rule: bad } }])).toThrow(/regex/u);
    expect(catalog.listOwned()).toEqual([]);
    catalog.close();
  });
});
