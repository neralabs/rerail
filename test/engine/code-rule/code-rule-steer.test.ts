import { describe, expect, it } from "vitest";
import { buildSteer, type CodeRuleViolation, renderSteer, SECRET_FILE_FINDING_MESSAGE, STEER_FIX_CHARS } from "../../../src/engine/code-rule/code-rule.js";

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

const FIX_IT = "Fix it, or tell the user in one line why you won't.";
const FIX_THESE = "Fix these, or tell the user in one line why you won't.";

describe("renderSteer", () => {
  it("renders one violation as the agent's steer: the edit's lead, the finding with its location and rule, the latitude", () => {
    expect(renderSteer([violation()])).toBe([
      "Rerail found 1 problem in your last edit:",
      "- src/a.ts:3:15 — log of user [p]",
      FIX_IT,
    ].join("\n"));
  });

  it("lists findings in first-seen order, not by rule, each fix under its finding, and names no state, mode or pack", () => {
    const fixed = violation({ fix: { span: span(2, 2, 2, 19), text: "logger.info(user)" } });
    const other = violation({ ruleId: "acme:no-print", mode: "steer", message: "print call", span: span(5, 0, 5, 5) });
    const steer = renderSteer([fixed, other, violation({ span: span(7, 1, 7, 2) })]);
    expect(steer).toBe([
      "Rerail found 3 problems in your last edit:",
      "- src/a.ts:3:15 — log of user [p]",
      "  Suggested fix: replace 3:3–3:20 with `logger.info(user)`",
      "- src/a.ts:6:1 — print call [acme:no-print]",
      "- src/a.ts:8:2 — log of user [p]",
      FIX_THESE,
    ].join("\n"));
    expect(steer).not.toMatch(/steered|Steer|Block|rule pack|Why\?|◆/u);
  });

  it("leads with the path alone for a finding about a whole file, and with the message for one with no path", () => {
    expect(renderSteer([violation({ span: undefined }), violation({ path: undefined, span: undefined, ruleId: "q" })])).toBe([
      "Rerail found 2 problems in your last edit:",
      "- src/a.ts — log of user [p]",
      "- log of user [q]",
      FIX_THESE,
    ].join("\n"));
  });

  it("keeps a multi-line message's later lines under its finding", () => {
    expect(renderSteer([violation({ message: "log of user\nUse the logger.\n\nIt redacts." })])).toBe([
      "Rerail found 1 problem in your last edit:",
      "- src/a.ts:3:15 — log of user [p]",
      "  Use the logger.",
      "",
      "  It redacts.",
      FIX_IT,
    ].join("\n"));
  });

  it("leads as its trigger says: a re-check, a late steer, a repository audit", () => {
    expect(renderSteer([violation()], { kind: "resteer" }).split("\n")[0]).toBe("Rerail re-checked; still open:");
    expect(renderSteer([violation()], { kind: "late" }).split("\n")[0]).toBe("Rerail finished checking your earlier edits after you moved on:");
    expect(renderSteer([violation()], { kind: "audit" }).split("\n")[0]).toBe("Rerail checked the repository and found 1 problem:");
    expect(renderSteer([violation(), violation()], { kind: "audit" }).split("\n")[0]).toBe("Rerail checked the repository and found 2 problems:");
  });

  it("puts a multi-line fix in a tilde fence, longer than any fence in the fix, and cuts one longer than the steer limit", () => {
    const multi = violation({ fix: { span: span(1, 2, 3, 4), text: "logger.info(a, {\n    b,\n  })" } });
    expect(renderSteer([multi])).toContain([
      "- src/a.ts:3:15 — log of user [p]",
      "  Suggested fix, replacing 2:3–4:5:",
      "  ~~~",
      "  logger.info(a, {",
      "      b,",
      "    })",
      "  ~~~",
    ].join("\n"));
    const fenced = violation({ fix: { span: span(0, 0, 1, 0), text: "~~~\ncode\n~~~~" } });
    expect(renderSteer([fenced])).toContain(["  ~~~~~", "  ~~~", "  code", "  ~~~~", "  ~~~~~"].join("\n"));
    const long = violation({ fix: { span: span(0, 0, 0, 1), text: "x".repeat(STEER_FIX_CHARS + 10) } });
    expect(renderSteer([long])).toContain(`\`${"x".repeat(STEER_FIX_CHARS)}… (truncated)\``);
  });

  it("fences a one-line fix with more backticks than it holds, padded where it starts or ends with one", () => {
    const fix = (text: string) => renderSteer([violation({ fix: { span: span(0, 0, 0, 3), text } })]).split("\n")[2];
    expect(fix("a `b` c")).toBe("  Suggested fix: replace 1:1–1:4 with ``a `b` c``");
    expect(fix("`x`")).toBe("  Suggested fix: replace 1:1–1:4 with `` `x` ``");
    expect(fix("a ``` b")).toBe("  Suggested fix: replace 1:1–1:4 with ````a ``` b````");
  });

  it("renders an empty fix as a deletion", () => {
    const deletion = violation({ fix: { span: span(0, 0, 0, 3), text: "" } });
    expect(renderSteer([deletion])).toContain("  Suggested fix: delete 1:1–1:4");
  });

  it("names a finding in a file named like one holding secrets by rule and place only, its message and fix withheld", () => {
    const secret = violation({
      path: "config/secrets.yaml", message: "`API_KEY: hunter2` is plaintext", fix: { span: span(0, 0, 0, 7), text: "API_KEY: ${SECRET}" },
    });
    expect(renderSteer([secret])).toBe([
      "Rerail found 1 problem in your last edit:",
      `- config/secrets.yaml:3:15 — ${SECRET_FILE_FINDING_MESSAGE} [p]`,
      FIX_IT,
    ].join("\n"));
    // Hidden alone is not secret-like: a rule about a workflow file still speaks.
    expect(renderSteer([violation({ path: ".github/workflows/ci.yml" })])).toContain("- .github/workflows/ci.yml:3:15 — log of user [p]");
  });
});

describe("buildSteer", () => {
  it("carries the rules behind the text, each once in first-seen order, with where each comes from", () => {
    const other = violation({ ruleId: "acme:no-print", mode: "log" });
    const sourceOf = (ruleId: string) => ruleId === "acme:no-print" ? { kind: "library_rule" as const, packId: null } : undefined;
    const steer = buildSteer([violation(), other, violation()], { kind: "edit" }, sourceOf);
    expect(steer.ruleIds).toEqual(["p", "acme:no-print"]);
    expect(steer.rules).toEqual([
      { id: "p", state: "steered" },
      { id: "acme:no-print", state: "logged", origin: "From your rule library." },
    ]);
    expect(steer.content).toBe(renderSteer([violation(), other, violation()]));
  });
});
