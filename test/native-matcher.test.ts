import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeRuleInput, type NativeSample } from "../src/native.js";

function rule(overrides: Partial<NativeRuleInput> = {}): NativeRuleInput {
  return {
    id: "checkout",
    reference: "Guest checkout remains available.",
    operator: "gte",
    threshold: 0.99,
    source: "assistant-text",
    boundary: "message",
    minimumTokens: 1,
    ...overrides,
  };
}

describe("native cosine matcher", () => {
  it("compiles references and returns matching rule facts", () => {
    const kernel = loadNativeBinding().CosineKernel.load(resolve("model"), 256);
    expect(kernel.replaceRules([rule()])).toEqual({ count: 1, dimensions: 256 });

    expect(
      kernel.matchText({
        source: "assistant-text",
        boundary: "message",
        text: "Guest checkout remains available.",
      }),
    ).toEqual([
      expect.objectContaining({
        ruleId: "checkout",
        operator: "gte",
        threshold: 0.99,
        similarity: expect.closeTo(1, 5),
      }),
    ]);
  });

  it("filters by source, boundary, and minimum token count", () => {
    const kernel = loadNativeBinding().CosineKernel.load(resolve("model"), 256);
    kernel.replaceRules([rule({ threshold: -1, minimumTokens: 4 })]);

    expect(kernel.matchText({ source: "user-input", boundary: "message", text: "Guest checkout remains available." })).toEqual([]);
    expect(kernel.matchText({ source: "assistant-text", boundary: "sentence", text: "Guest checkout remains available." })).toEqual([]);
    expect(kernel.matchText({ source: "assistant-text", boundary: "message", text: "Guest checkout" })).toEqual([]);
  });

  it("rejects unknown source and boundary domain values", () => {
    const kernel = loadNativeBinding().CosineKernel.load(resolve("model"), 256);
    kernel.replaceRules([rule()]);

    const invalidSource = {
      source: "assistant",
      boundary: "message",
      text: "Guest checkout remains available.",
    } as unknown as Parameters<typeof kernel.matchText>[0];
    const invalidBoundary = {
      source: "assistant-text",
      boundary: "paragraph",
      text: "Guest checkout remains available.",
    } as unknown as Parameters<typeof kernel.matchText>[0];
    expect(() => kernel.matchText(invalidSource)).toThrow(/source/i);
    expect(() => kernel.matchText(invalidBoundary)).toThrow(/boundary/i);
  });

  it("supports dissimilarity rules and atomic replacement", () => {
    const kernel = loadNativeBinding().CosineKernel.load(resolve("model"), 256);
    kernel.replaceRules([rule({ id: "old", operator: "lte", threshold: 1 })]);
    kernel.replaceRules([rule({ id: "new", operator: "lte", threshold: 1 })]);

    const matches = kernel.matchText({
      source: "assistant-text",
      boundary: "message",
      text: "Implement database backups and restore testing.",
    });
    expect(matches.map(match => match.ruleId)).toEqual(["new"]);
  });
  it("embeds batches without compiling trigger rules", () => {
    const kernel = loadNativeBinding().CosineKernel.load(resolve("model"), 256);
    const [first, second] = kernel.embedTexts([
      "Guest checkout remains available.",
      "Implement database backups and restore testing.",
    ]);

    expect(first).toBeInstanceOf(Float32Array);
    expect(first).toHaveLength(kernel.fingerprint().dimensions);
    expect(second).toHaveLength(kernel.fingerprint().dimensions);
    expect(first).not.toEqual(second);
    expect(kernel.embedTexts([])).toEqual([]);
    expect(() => kernel.embedTexts([""])).toThrow(/empty/i);
    const text = "Guest checkout remains available for every customer.";
    const [tokens] = kernel.tokenCounts([text]);
    expect(tokens).toBeGreaterThan(2);
    const [prefix] = kernel.fitTexts([text], tokens! - 1, "truncate-end");
    const [suffix] = kernel.fitTexts([text], tokens! - 1, "truncate-start");
    expect(text.startsWith(prefix!)).toBe(true);
    expect(text.endsWith(suffix!)).toBe(true);
    expect(kernel.tokenCounts([prefix!])[0]).toBeLessThanOrEqual(tokens! - 1);
    expect(kernel.tokenCounts([suffix!])[0]).toBeLessThanOrEqual(tokens! - 1);
    expect(() => kernel.fitTexts([text], tokens! - 1, "reject")).toThrow(/exceeds/i);
  });

});

function structural(id: string, conditions: unknown, overrides: Partial<NativeRuleInput> = {}): NativeRuleInput {
  return {
    id,
    conditions: JSON.stringify(conditions),
    source: "tool:*",
    boundary: "tool-call",
    minimumTokens: 1,
    ...overrides,
  };
}

function toolCall(toolName: string, input: Record<string, unknown>): NativeSample {
  const text = JSON.stringify(input);
  return { source: `tool:${toolName}`, boundary: "tool-call", text, input: text };
}

describe("native structural matcher", () => {
  const command = (test: Record<string, string>) => ({ field: "input.command", ...test });
  const kernel = () => loadNativeBinding().CosineKernel.load(resolve("model"), 256);
  const ids = (matches: readonly { ruleId: string }[]) => matches.map(match => match.ruleId);

  it("matches tool names and arguments by eq, contains, and regex", () => {
    const matcher = kernel();
    const [byContains, byName, byRegex] = ["contains-arg", "eq-name", "regex-arg"];
    expect(matcher.replaceRules([
      structural(byName, { field: "tool.name", eq: "Bash" }),
      structural(byContains, command({ contains: "rm -rf" })),
      structural(byRegex, command({ regex: "^rm\\s" })),
    ])).toEqual({ count: 3, dimensions: 256 });

    expect(ids(matcher.matchText(toolCall("Bash", { command: "rm -rf build" })))).toEqual([byContains, byName, byRegex]);
    expect(ids(matcher.matchText(toolCall("bash", { command: "echo rm -rf build" })))).toEqual([byContains]);
    expect(ids(matcher.matchText(toolCall("Bash", { command: "ls" })))).toEqual([byName]);
  });

  it("anchors a regex to the raw argument, not to the serialized JSON", () => {
    const matcher = kernel();
    matcher.replaceRules([structural("anchored", command({ regex: "^git push$" }))]);
    expect(ids(matcher.matchText(toolCall("Bash", { command: "git push" })))).toEqual(["anchored"]);
    expect(ids(matcher.matchText(toolCall("Bash", { command: "git push", cwd: "/repo" })))).toEqual(["anchored"]);
  });

  it("reads nested string arguments and treats missing or non-string values as no match", () => {
    const matcher = kernel();
    matcher.replaceRules([structural("nested", { field: "input.options.branch", eq: "main" })]);
    expect(ids(matcher.matchText(toolCall("git", { options: { branch: "main" } })))).toEqual(["nested"]);
    expect(ids(matcher.matchText(toolCall("git", { options: { branch: 1 } })))).toEqual([]);
    expect(ids(matcher.matchText(toolCall("git", { options: {} })))).toEqual([]);
    expect(ids(matcher.matchText(toolCall("git", { other: "main" })))).toEqual([]);
  });

  it("combines conditions with all, any, and not", () => {
    const matcher = kernel();
    matcher.replaceRules([
      structural("combined", {
        all: [
          { any: [{ field: "tool.name", eq: "Bash" }, { field: "tool.name", eq: "shell" }] },
          command({ regex: "\\brm\\s+-rf\\b" }),
          { not: command({ regex: "\\brm\\s+-rf\\s+(?:tmp|build)/" }) },
        ],
      }),
    ]);
    expect(ids(matcher.matchText(toolCall("shell", { command: "rm -rf src/" })))).toEqual(["combined"]);
    expect(ids(matcher.matchText(toolCall("shell", { command: "rm -rf tmp/cache" })))).toEqual([]);
    expect(ids(matcher.matchText(toolCall("Write", { command: "rm -rf src/" })))).toEqual([]);
  });

  it("skips tool rules for samples without a tool call, even under not", () => {
    const matcher = kernel();
    matcher.replaceRules([
      structural("negated", { not: command({ contains: "safe" }) }),
      structural("name-only", { field: "tool.name", eq: "Bash" }),
    ]);
    const toolResult: NativeSample = { source: "tool:Bash", boundary: "tool-call", text: "unrelated output" };
    expect(ids(matcher.matchText(toolResult))).toEqual([]);
    expect(ids(matcher.matchText(toolCall("Bash", { command: "rm -rf /" })))).toEqual(["name-only", "negated"]);
  });

  it("binds tool:* to every tool source and to no conversation source", () => {
    const matcher = kernel();
    matcher.replaceRules([structural("any-tool", { field: "input", contains: "secret" })]);
    expect(ids(matcher.matchText(toolCall("Write", { content: "secret" })))).toEqual(["any-tool"]);
    expect(ids(matcher.matchText({ source: "user-input", boundary: "message", text: "secret" }))).toEqual([]);
  });

  it("matches sample text for conversation sources without similarity facts", () => {
    const matcher = kernel();
    matcher.replaceRules([
      structural("text-rule", { field: "text", regex: "(?i)\\bforce[- ]push\\b" }, { source: "assistant-text", boundary: "message" }),
    ]);
    expect(matcher.matchText({ source: "assistant-text", boundary: "message", text: "I will Force-push the branch." })).toEqual([
      { ruleId: "text-rule", kind: "structural" },
    ]);
  });

  it("evaluates cosine and structural rules side by side on one sample", () => {
    const matcher = kernel();
    matcher.replaceRules([
      rule({ id: "similar", source: "tool:Bash", boundary: "tool-call", operator: "lte", threshold: 1 }),
      structural("exact", command({ eq: "git push" })),
    ]);
    const matches = matcher.matchText(toolCall("Bash", { command: "git push" }));
    expect(matches).toEqual([
      expect.objectContaining({ ruleId: "exact", kind: "structural" }),
      expect.objectContaining({ ruleId: "similar", kind: "similarity", operator: "lte", threshold: 1, similarity: expect.any(Number) }),
    ]);
  });

  it.each([
    ["an invalid regex", structural("bad", { all: [{ field: "tool.name", eq: "Bash" }, command({ regex: "(" })] }), /\/match\/all\/1\/regex/u],
    ["an unknown field", structural("bad", { field: "input..command", eq: "x" }), /\/match\/field/u],
    ["two tests in one condition", structural("bad", { field: "tool.name", eq: "a", regex: "a" }), /\/match.*exactly one/u],
    ["an unknown key", structural("bad", { field: "tool.name", equals: "a" }), /\/match.*equals/u],
    ["an empty all", structural("bad", { all: [] }), /\/match\/all/u],
    ["a tool field on a conversation source", structural("bad", command({ eq: "a" }), { source: "user-input", boundary: "message" }), /tool:/u],
    ["a tool field outside the tool-call boundary", structural("bad", { field: "tool.name", eq: "a" }, { boundary: "message" }), /tool-call/u],
    ["tool:* on a similarity rule", rule({ id: "bad", source: "tool:*", boundary: "tool-call" }), /tool:\*/u],
    ["a similarity rule without a reference", { ...rule({ id: "bad" }), reference: undefined }, /reference/u],
    ["a structural rule with a reference", structural("bad", { field: "text", eq: "a" }, { reference: "r" }), /reference/u],
  ])("rejects %s at compile time", (_label, input, message) => {
    const matcher = kernel();
    matcher.replaceRules([structural("kept", { field: "tool.name", eq: "Bash" })]);
    expect(() => matcher.replaceRules([input as NativeRuleInput])).toThrow(message);
    expect(ids(matcher.matchText(toolCall("Bash", { command: "ls" })))).toEqual(["kept"]);
  });

  it("rejects a regex over the policy DSL pattern limit", () => {
    const matcher = kernel();
    expect(() => matcher.replaceRules([structural("huge", { field: "text", regex: "a".repeat(16_385) }, { source: "assistant-text", boundary: "message" })]))
      .toThrow(/maxRegexPatternBytes/u);
  });
});
