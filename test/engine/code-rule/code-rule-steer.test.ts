import { describe, expect, it } from "vitest";
import { type CodeRuleViolation, renderSteer, STEER_FIX_CHARS } from "../../../src/engine/code-rule/code-rule.js";

const span = (startLine: number, startColumn: number, endLine: number, endColumn: number) => ({
  startByte: 0, endByte: 0, startLine, startColumn, endLine, endColumn,
});

function violation(overrides: Partial<CodeRuleViolation> = {}): CodeRuleViolation {
  return {
    id: "sha256:x", ruleId: "p", clauseId: "c", code: "NO_LOG", mode: "block",
    message: "log of user", path: "src/a.ts", span: span(2, 14, 2, 18), captures: {}, variables: {},
    ...overrides,
  };
}

const BLOCK_SENTENCE = "Set to Block, which steers until blocking ships: fix it, or tell the person in one line why you won't.";

describe("renderSteer", () => {
  it("renders one rule's violation as a Rerail steer with its mode, location and Why link", () => {
    expect(renderSteer([violation()])).toBe([
      "◆ Rerail · p  steered",
      BLOCK_SENTENCE,
      "log of user (src/a.ts:3:15)",
      "Why? → #rule/p",
    ].join("\n"));
  });

  it("groups violations under their rule, in first-seen order, with each fix under its violation and each rule's source", () => {
    const fixed = violation({ fix: { span: span(2, 2, 2, 19), text: "logger.info(user)" } });
    const other = violation({ ruleId: "acme:no-print", mode: "steer", message: "print call", span: span(5, 0, 5, 5) });
    const sourceOf = (ruleId: string) => ruleId === "acme:no-print" ? { kind: "library_rule" as const, packId: null } : undefined;
    expect(renderSteer([fixed, other, violation({ span: span(7, 1, 7, 2) })], sourceOf)).toBe([
      "◆ Rerail · p  steered",
      BLOCK_SENTENCE,
      "log of user (src/a.ts:3:15)",
      "  Suggested fix: replace 3:3-3:20 with `logger.info(user)`",
      "log of user (src/a.ts:8:2)",
      "Why? → #rule/p",
      "",
      "◆ Rerail · acme:no-print  steered",
      "From your rule library. Set to Steer: fix it, or tell the person in one line why you won't.",
      "print call (src/a.ts:6:1)",
      "Why? → #rule/acme%3Ano-print",
    ].join("\n"));
  });

  it("indents a multi-line fix and cuts one longer than the steer limit", () => {
    const multi = violation({ fix: { span: span(1, 2, 3, 4), text: "logger.info(a, {\n    b,\n  })" } });
    expect(renderSteer([multi])).toContain([
      "log of user (src/a.ts:3:15)",
      "  Suggested fix: replace 2:3-4:5 with:",
      "    logger.info(a, {",
      "        b,",
      "      })",
    ].join("\n"));
    const long = violation({ fix: { span: span(0, 0, 0, 1), text: "x".repeat(STEER_FIX_CHARS + 10) } });
    expect(renderSteer([long])).toContain(`\`${"x".repeat(STEER_FIX_CHARS)}… (truncated)\``);
  });

  it("renders an empty fix as a deletion", () => {
    const deletion = violation({ fix: { span: span(0, 0, 0, 3), text: "" } });
    expect(renderSteer([deletion])).toContain("  Suggested fix: delete 1:1-1:4");
  });
});
