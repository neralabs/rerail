import { describe, expect, it } from "vitest";
import { type CodeRuleViolation, renderSteer, STEER_FIX_CHARS } from "../src/code-rule.js";

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

describe("renderSteer", () => {
  it("renders a violation without a fix as one line", () => {
    expect(renderSteer([violation()])).toBe("NO_LOG (src/a.ts:3:15): log of user");
  });

  it("appends a single-line suggested fix with its replaced range", () => {
    const fixed = violation({ fix: { span: span(2, 2, 2, 19), text: "logger.info(user)" } });
    expect(renderSteer([fixed, violation({ code: "OTHER" })])).toBe([
      "NO_LOG (src/a.ts:3:15): log of user",
      "  Suggested fix: replace 3:3-3:20 with `logger.info(user)`",
      "OTHER (src/a.ts:3:15): log of user",
    ].join("\n"));
  });

  it("indents a multi-line fix and cuts one longer than the steer limit", () => {
    const multi = violation({ fix: { span: span(1, 2, 3, 4), text: "logger.info(a, {\n    b,\n  })" } });
    expect(renderSteer([multi])).toBe([
      "NO_LOG (src/a.ts:3:15): log of user",
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
