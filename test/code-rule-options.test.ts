import { describe, expect, it } from "vitest";
import { CodeRuleOptionError, codeRuleOptionValues, resolveCodeRuleOptions } from "../src/code-rule-options.js";

function withNumber(config: unknown, declaration: Record<string, unknown> = { type: "number", default: 0 }) {
  return {
    format: "rerail.policy/v1",
    id: "test:numbers",
    revision: 1,
    mode: "steer",
    clauses: [
      { id: "options", define: { options: { n: declaration }, config: { n: config } } },
      { id: "use", message: { $option: "n", cases: { "1": "one" }, default: "other" } },
    ],
  };
}

describe("resolveCodeRuleOptions", () => {
  it("inserts replacement tokens in map and template values literally", () => {
    const resolved = resolveCodeRuleOptions({
      clauses: [{
        id: "options",
        define: {
          options: {
            path: { type: "string", default: "src/$$" },
            paths: { type: "array", items: "string", default: ["src/$$"] },
          },
        },
      }],
      mapped: { $option: "paths", map: "{}/**/*.ts" },
      templated: { $option: "path", template: "{}/**/*.ts" },
    }) as { mapped: string[]; templated: string };

    expect(resolved.mapped).toEqual(["src/$$/**/*.ts"]);
    expect(resolved.templated).toBe("src/$$/**/*.ts");
  });

  it("rejects an array template value even when its template has no placeholder", () => {
    expect(() => resolveCodeRuleOptions({
      clauses: [{
        id: "options",
        define: {
          options: {
            paths: { type: "array", items: "string", default: ["src"] },
          },
        },
      }],
      templated: { $option: "paths", template: "identifier" },
    })).toThrow("the option's value is not text");
  });
});

describe("code rule option numbers", () => {
  it("refuses integers beyond the safe range, which JSON parsing may already have rounded", () => {
    // Raw native JSON `9007199254740993` arrives here as 2^53; both must be refused, not select different cases.
    for (const value of [2 ** 53, -(2 ** 53), 2 ** 60, 1e21]) {
      expect(() => codeRuleOptionValues(withNumber(value))).toThrow(CodeRuleOptionError);
    }
    expect(() => codeRuleOptionValues(withNumber(2 ** 53))).toThrow("9007199254740992 is not exactly representable as a JavaScript number");
    expect(() => codeRuleOptionValues(withNumber(1e21))).toThrow("1e+21 is not exactly representable as a JavaScript number");
    expect(() => codeRuleOptionValues(withNumber(Number.NaN))).toThrow(/is not exactly representable as a JavaScript number/);
    expect(() => codeRuleOptionValues(withNumber(Number.POSITIVE_INFINITY))).toThrow(/is not exactly representable/);
    expect(() => codeRuleOptionValues(withNumber(1, { type: "number", default: 2 ** 53 }))).toThrow(CodeRuleOptionError);
    expect(() => codeRuleOptionValues(withNumber(1, { type: "number", enum: [1, 2 ** 53], default: 1 })))
      .toThrow(expect.objectContaining({ path: ["clauses", 0, "define", "options", "n", "enum", 1] }));
    expect(() => codeRuleOptionValues(withNumber([1, 2 ** 53], { type: "array", items: "number", default: [] })))
      .toThrow(expect.objectContaining({ path: ["clauses", 0, "define", "config", "n", 1] }));
    expect(codeRuleOptionValues(withNumber(2 ** 53 - 1)).get("n")).toBe(2 ** 53 - 1);
    expect(resolveCodeRuleOptions(withNumber(1.0))).toMatchObject({ clauses: [{}, { message: "one" }] });
  });
});
