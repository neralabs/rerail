import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { conversationRuleSchema, parseConversationRule, parseConversationRuleDocument } from "../src/rule-schema.js";

describe("parseConversationRuleDocument", () => {
  it("accepts a versioned dissimilarity rule", () => {
    const rules = parseConversationRuleDocument({
      version: 1,
      rules: [
        {
          id: "checkout-drift",
          mode: "steer",
          reference: "Guest checkout remains available.",
          match: { operator: "lte", threshold: 0.62 },
          sample: {
            source: "assistant-text",
            boundary: "message",
            minimumTokens: 30,
          },
          action: {
            content: "Check the checkout requirement.",
          },
          repeat: { mode: "after-gap", gapTurns: 5 },
        },
      ],
    });

    expect(rules).toEqual([
      expect.objectContaining({
        id: "checkout-drift",
        match: { operator: "lte", threshold: 0.62 },
        repeat: { mode: "after-gap", gapTurns: 5 },
      }),
    ]);
  });

  it("rejects a threshold outside cosine range", () => {
    expect(() =>
      parseConversationRuleDocument({
        version: 1,
        rules: [
          {
            id: "invalid",
            mode: "log",
            reference: "reference",
            match: { operator: "gte", threshold: 1.1 },
            sample: { source: "assistant-text", boundary: "message" },
            action: { content: "record" },
          },
        ],
      }),
    ).toThrow(/threshold/i);
  });

  it("reports the failing field path and accepts tool sources through the shared zod schema", () => {
    expect(() => parseConversationRule({ id: "x", mode: "steer", reference: "r", match: { operator: "gte", threshold: 0.5 }, sample: { source: "tool:", boundary: "message" }, action: { content: "c" } }, "rules[0]"))
      .toThrow("rules[0].sample.source must be user-input, assistant-text, or tool:<name>");
    expect(() => parseConversationRule({ id: "x", mode: "steer", reference: "r", match: { operator: "gte", threshold: 0.5 }, sample: { source: "user-input", boundary: "message" }, action: { content: "c" }, extra: 1 }))
      .toThrow(/^rule Unrecognized key/u);
    expect(conversationRuleSchema.parse({
      id: "x",
      mode: "log",
      reference: "r",
      match: { operator: "lte", threshold: -1 },
      sample: { source: "tool:write", boundary: "tool-call", minimumTokens: 8_192 },
      action: { content: "c" },
      repeat: { mode: "after-gap", gapTurns: 2 },
    })).toMatchObject({ sample: { source: "tool:write" }, repeat: { gapTurns: 2 } });
  });

  it("requires a log, steer, or block mode and accepts no delivery on the action", () => {
    const rule = {
      id: "x", mode: "block", reference: "r",
      match: { operator: "gte", threshold: 0.5 },
      sample: { source: "user-input", boundary: "message" },
      action: { content: "c" },
    };
    expect(parseConversationRule(rule).mode).toBe("block");
    const { mode: _mode, ...modeless } = rule;
    expect(() => parseConversationRule(modeless)).toThrow(/^rule\.mode /u);
    expect(() => parseConversationRule({ ...rule, mode: "observe" })).toThrow(/^rule\.mode /u);
    expect(() => parseConversationRule({ ...rule, action: { delivery: "steer", content: "c" } })).toThrow(/^rule\.action Unrecognized key/u);
  });
});

describe("structural match conditions", () => {
  const BASH_CALL = { source: "tool:Bash", boundary: "tool-call" } as const;
  const ANY_TOOL_CALL = { source: "tool:*", boundary: "tool-call" } as const;
  const toolRule = (overrides: Record<string, unknown> = {}) => ({
    id: "no-force-push",
    mode: "steer",
    match: { field: "input.command", regex: "\\bgit\\s+push\\b[^\\n]*--force\\b" },
    sample: BASH_CALL,
    action: { content: "Do not force-push." },
    ...overrides,
  });

  it("accepts leaf conditions and all/any/not over them without a reference", () => {
    expect(parseConversationRule(toolRule())).toEqual(toolRule());
    const nested = {
      all: [
        { any: [{ field: "tool.name", eq: "Bash" }, { field: "tool.name", regex: "^shell$" }] },
        { not: { field: "input.command", contains: "--dry-run" } },
        { field: "input", contains: "push" },
      ],
    };
    expect(parseConversationRule(toolRule({ match: nested, sample: ANY_TOOL_CALL })).match).toEqual(nested);
    expect(parseConversationRule(toolRule({
      match: { field: "text", regex: "(?i)force push" },
      sample: { source: "assistant-text", boundary: "message" },
    })).match).toEqual({ field: "text", regex: "(?i)force push" });
  });

  it.each([
    ["a tool field on a conversation source", { sample: { source: "user-input", boundary: "message" } }, /rule\.sample\.source .*tool:/u],
    ["a tool field outside the tool-call boundary", { sample: { ...BASH_CALL, boundary: "message" } }, /rule\.sample\.boundary .*tool-call/u],
    ["a reference", { reference: "force push" }, /rule\.reference/u],
    ["minimumTokens", { sample: { ...BASH_CALL, minimumTokens: 4 } }, /rule\.sample\.minimumTokens/u],
    ["two tests in one condition", { match: { field: "tool.name", eq: "Bash", regex: "Bash" } }, /rule\.match/u],
    ["an unknown field", { match: { field: "input.", eq: "x" } }, /rule\.match\.field/u],
    ["an empty any", { match: { any: [] } }, /rule\.match\.any/u],
    ["an empty condition", { match: {} }, /^rule\.match must contain field, all, any, or not$/u],
    ["an empty condition inside all", { match: { all: [{}] } }, /^rule\.match\.all\.0 must contain field, all, any, or not$/u],
    ["an empty condition under not", { match: { not: {} } }, /^rule\.match\.not must contain field, all, any, or not$/u],
  ])("rejects %s on a structural rule", (_label, overrides, message) => {
    expect(() => parseConversationRule(toolRule(overrides))).toThrow(message);
  });

  it("keeps similarity rules requiring a reference and a concrete source", () => {
    const similarity = { match: { operator: "gte", threshold: 0.8 } };
    expect(() => parseConversationRule(toolRule(similarity))).toThrow(/rule\.reference/u);
    expect(() => parseConversationRule(toolRule({ ...similarity, reference: "force push", sample: ANY_TOOL_CALL })))
      .toThrow(/rule\.sample\.source .*tool:\*/u);
  });
});

describe("rule-schema module", () => {
  it("imports nothing but zod and the import-free rule modes, because browser bundles share it", () => {
    const imports = (path: string) => [...readFileSync(resolve(path), "utf8").matchAll(/^import\b.*?from\s+"([^"]+)"/gmu)].map(match => match[1]);
    expect(imports("src/rule-schema.ts")).toEqual(["zod", "./rule-mode.js"]);
    expect(imports("src/rule-mode.ts")).toEqual([]);
  });
});
