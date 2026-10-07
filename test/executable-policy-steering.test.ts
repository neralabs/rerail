import { describe, expect, it } from "vitest";
import { type ExecutablePolicyFinding, renderExecutablePolicySteering, STEERING_FIX_CHARS } from "../src/executable-policy.js";

const span = (startLine: number, startColumn: number, endLine: number, endColumn: number) => ({
  startByte: 0, endByte: 0, startLine, startColumn, endLine, endColumn,
});

function finding(overrides: Partial<ExecutablePolicyFinding> = {}): ExecutablePolicyFinding {
  return {
    id: "sha256:x", ruleId: "p", clauseId: "c", code: "NO_LOG", mode: "block",
    message: "log of user", path: "src/a.ts", span: span(2, 14, 2, 18), captures: {}, variables: {},
    ...overrides,
  };
}

describe("renderExecutablePolicySteering", () => {
  it("renders a finding without a fix as one line", () => {
    expect(renderExecutablePolicySteering([finding()])).toBe("NO_LOG (src/a.ts:3:15): log of user");
  });

  it("appends a single-line suggested fix with its replaced range", () => {
    const fixed = finding({ fix: { span: span(2, 2, 2, 19), text: "logger.info(user)" } });
    expect(renderExecutablePolicySteering([fixed, finding({ code: "OTHER" })])).toBe([
      "NO_LOG (src/a.ts:3:15): log of user",
      "  Suggested fix: replace 3:3-3:20 with `logger.info(user)`",
      "OTHER (src/a.ts:3:15): log of user",
    ].join("\n"));
  });

  it("indents a multi-line fix and cuts one longer than the steering limit", () => {
    const multi = finding({ fix: { span: span(1, 2, 3, 4), text: "logger.info(a, {\n    b,\n  })" } });
    expect(renderExecutablePolicySteering([multi])).toBe([
      "NO_LOG (src/a.ts:3:15): log of user",
      "  Suggested fix: replace 2:3-4:5 with:",
      "    logger.info(a, {",
      "        b,",
      "      })",
    ].join("\n"));
    const long = finding({ fix: { span: span(0, 0, 0, 1), text: "x".repeat(STEERING_FIX_CHARS + 10) } });
    expect(renderExecutablePolicySteering([long])).toContain(`\`${"x".repeat(STEERING_FIX_CHARS)}… (truncated)\``);
  });

  it("renders an empty fix as a deletion", () => {
    const deletion = finding({ fix: { span: span(0, 0, 0, 3), text: "" } });
    expect(renderExecutablePolicySteering([deletion])).toContain("  Suggested fix: delete 1:1-1:4");
  });
});
