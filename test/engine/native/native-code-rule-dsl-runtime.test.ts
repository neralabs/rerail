import { readFileSync } from "node:fs";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import {
  loadNativeBinding,
  type NativeCompiledCodeRuleDsl as NativeCompiledCodeRule,
  type NativeCodeRuleDslRuntimeConstructor,
  type NativeCodeRuleDslWorkspace,
  type CodeRuleDslEvaluationMode as EvaluationMode,
} from "../../../src/engine/native/native.js";
import { RULE_MODES } from "../../../src/engine/catalog/rule-mode.js";
import { fixturesPath } from "../../support/oracles.js";

const sourceSpanSchema = z.strictObject({
  startByte: z.number().int().nonnegative(),
  endByte: z.number().int().nonnegative(),
  startLine: z.number().int().nonnegative(),
  startColumn: z.number().int().nonnegative(),
  endLine: z.number().int().nonnegative(),
  endColumn: z.number().int().nonnegative(),
});

const violationSchema = z.strictObject({
  id: z.string().min(1),
  ruleId: z.string().min(1),
  clauseId: z.string().min(1),
  code: z.string().min(1),
  mode: z.enum(RULE_MODES),
  message: z.string(),
  path: z.string().optional(),
  span: sourceSpanSchema.optional(),
  captures: z.record(z.string(), z.array(sourceSpanSchema)),
  variables: z.record(z.string(), z.union([z.boolean(), z.number(), z.string()])),
  fix: z.strictObject({ span: sourceSpanSchema, text: z.string() }).optional(),
});

const codeRuleFailureSchema = z.strictObject({
  clauseId: z.string().min(1),
  code: z.enum(["budget-exceeded", "missing-capability", "source-incomplete", "source-partial", "adapter-error"]),
  message: z.string(),
  path: z.string().optional(),
});

const coverageSchema = z.strictObject({
  clauseId: z.string().min(1),
  ruleId: z.string().optional(),
  status: z.enum(["complete", "partial", "failed"]),
  reasons: z.array(z.strictObject({
    code: codeRuleFailureSchema.shape.code,
    count: z.number().int().positive(),
    paths: z.array(z.string()),
  })),
});

const evaluationResultSchema = z.strictObject({
  status: z.enum(["completed", "failed"]),
  violations: z.array(violationSchema),
  failures: z.array(codeRuleFailureSchema),
  coverage: z.array(coverageSchema),
  metrics: z.strictObject({
    files: z.number().int().nonnegative(),
    visitedNodes: z.number().int().nonnegative(),
    witnesses: z.number().int().nonnegative(),
  }),
});

const incrementalResultSchema = evaluationResultSchema.extend({
  revision: z.number().int().nonnegative(),
  addedViolationIds: z.array(z.string()),
  retractedViolationIds: z.array(z.string()),
});

type EvaluationResult = z.infer<typeof evaluationResultSchema>;

const defaultLimits = {
  maxAstPatternBytes: 16_384,
  maxFiles: 100,
  maxRegexPatternBytes: 16_384,
  maxSourceBytes: 1_000_000,
  maxVisitedNodes: 100_000,
  maxWitnesses: 10_000,
};


function capabilities(overrides: Record<string, unknown> = {}): string {
  return JSON.stringify({
    format: "rerail.policy-capabilities/v1",
    ast: {
      engine: "ast-grep-core",
      engineVersion: "0.45",
      languages: {
        python: "tree-sitter-python@0.23",
        typescript: "tree-sitter-typescript@0.23",
      },
    },
    limits: defaultLimits,
    ...overrides,
  });
}

const codeRuleDslRuntime = loadNativeBinding().CodeRuleDslRuntime;

function nativeRuntime(): NativeCodeRuleDslRuntimeConstructor {
  if (codeRuleDslRuntime === undefined) {
    throw new Error("Native binding must export CodeRuleDslRuntime before this contract can pass");
  }
  return codeRuleDslRuntime;
}
function codeRule(clauses: unknown[], overrides: Record<string, unknown> = {}): string {
  return JSON.stringify({
    clauses: clauses,
    format: "rerail.policy/v1",
    id: "runtime-contract",
    mode: "block",
    revision: 1,
    scope: {
      exclude: [],
      include: ["**/*"],
    },
    ...overrides,
  });
}

function violation(code: string): Record<string, unknown> {
  return {
    anchor: "witness.span",
    code,
    message: `${code} finding`,
  };
}

function snapshot(
  files: Array<{ language: "python" | "typescript" | "tsx"; path: string; source: string; module?: string | string[] }> = [],
  facts: Array<{ relation: string; rows: unknown[] }> = [],
): string {
  return JSON.stringify({
    facts,
    files,
    format: "rerail.repository-snapshot/v1",
    revision: 0,
  });
}

function evaluate(
  compiled: NativeCompiledCodeRule,
  input: string,
  mode: EvaluationMode = "report",
): EvaluationResult {
  return evaluationResultSchema.parse(JSON.parse(compiled.evaluate(input, mode)));
}

function compileJson(clauses: unknown[], capabilityJson = capabilities()): NativeCompiledCodeRule {
  return nativeRuntime().compile(codeRule(clauses), "json", capabilityJson);
}

/** Code-rule documents kept as engine-contract fixtures (formerly bundled defaults; every shipped rule is in a pack). */
function compileCodeRuleFile(name: string): NativeCompiledCodeRule {
  const source = readFileSync(fixturesPath("host", "code-rule-dsl", `${name}.json`), "utf8");
  return nativeRuntime().compile(source, "json", capabilities());
}

function astForbid(
  rule: Record<string, unknown>,
  language: "python" | "typescript" | "ruby" = "python",
): Record<string, unknown> {
  return {
    violation: violation("AST_MATCH"),
    forbid: {
      ast: {
        language,
        rule,
      },
    },
    id: "ast-match",
  };
}

describe("native code-rule DSL runtime availability", () => {
  it("exports the prospective CodeRuleDslRuntime N-API class", () => {
    expect(codeRuleDslRuntime, "native binding does not export CodeRuleDslRuntime").toBeDefined();
  });
});

describe.skipIf(codeRuleDslRuntime === undefined)("native code-rule DSL runtime contract", () => {
  describe("source validation and canonicalization", () => {
    it("canonicalizes equivalent YAML and JSON to identical bytes and digests", () => {
      const yaml = `
format: rerail.policy/v1
id: runtime-contract
revision: 1
mode: block
scope:
  include: ['**/*']
  exclude: []
clauses:
  - id: raw-execute
    forbid:
      regex:
        source: file.content
        pattern: 'execute_async\\(\\)'
    violation:
      code: RAW_EXECUTE
      anchor: witness.span
      message: Raw execute
`;
      const json = codeRule([
        {
          violation: { anchor: "witness.span", code: "RAW_EXECUTE", message: "Raw execute" },
          forbid: { regex: { pattern: "execute_async\\(\\)", source: "file.content" } },
          id: "raw-execute",
        },
      ]);
      const fromYaml = nativeRuntime().compile(yaml, "yaml", capabilities());
      const fromJson = nativeRuntime().compile(json, "json", capabilities());

      expect(fromYaml.canonicalJson()).toBe(fromJson.canonicalJson());
      expect(fromYaml.digest()).toBe(fromJson.digest());
      expect(JSON.parse(fromYaml.canonicalJson())).toEqual(JSON.parse(json));
    });

    it.each([
      ["unknown field", { command: ["python", "lint.py"] }, /unknown.*command.*clauses\/0/i],
      [
        "multiple constraint forms",
        { forbid: { regex: { pattern: "x", source: "file.content" } }, budget: {} },
        /exactly one/i,
      ],
    ])("rejects %s", (_label, invalidClause, expected) => {
      expect(() => compileJson([{ ...invalidClause, violation: violation("INVALID"), id: "invalid" }])).toThrow(expected);
    });

    const forbidX = { forbid: { regex: { pattern: "x", source: "file.content" } } };

    it.each([
      ["a missing mode", { mode: undefined }],
      ["a non-string mode", { mode: 2 }],
      ["a retired severity as the mode", { mode: "error" }],
    ])("rejects %s", (_label, overrides) => {
      const source = codeRule([{ ...forbidX, violation: violation("INVALID"), id: "invalid-mode" }], overrides);
      expect(() => nativeRuntime().compile(source, "json", capabilities())).toThrow("/mode must equal log, steer, or block");
    });

    it("rejects a violation severity as an unknown field", () => {
      expect(() => compileJson([{ ...forbidX, violation: { ...violation("INVALID"), severity: "error" }, id: "invalid-severity" }]))
        .toThrow("unknown field `severity` at /clauses/0/violation");
    });

    it("stamps every violation with the rule's mode", () => {
      for (const mode of RULE_MODES) {
        const compiled = nativeRuntime().compile(codeRule([{ ...forbidX, violation: violation("X"), id: "x" }], { mode }), "json", capabilities());
        const result = evaluate(compiled, snapshot([{ language: "python", path: "app.py", source: "x = 1\n" }]));
        expect(result.violations.map(entry => entry.mode)).toEqual([mode]);
      }
    });

    it("reports malformed ast-grep patterns at the code rule path", () => {
      expect(() => compileJson([astForbid({ pattern: "call(" })])).toThrow(
        /clauses\/0\/forbid\/ast\/rule\/pattern.*parse/i,
      );
    });

    it("rejects unsupported languages and grammar versions during compilation", () => {
      const unsupported = astForbid({ pattern: "print($A)" }, "ruby");
      expect(() => compileJson([unsupported])).toThrow(/language must equal python, typescript, javascript or php/);
    });

  });

  describe("ast-grep-core matcher semantics", () => {
    it("returns the root witness and metavariable capture spans", () => {
      const compiled = compileJson([astForbid({ pattern: "print($ARG)" })]);
      const source = "value = 1\nprint(value)\n";
      const result = evaluate(compiled, snapshot([{ language: "python", path: "a.py", source }]));
      const callStart = source.indexOf("print(value)");
      const argumentStart = source.indexOf("value", callStart);

      expect(result).toMatchObject({ status: "completed", failures: [] });
      expect(result.violations).toHaveLength(1);
      expect(result.violations[0]).toMatchObject({
        clauseId: "ast-match",
        code: "AST_MATCH",
        path: "a.py",
        span: {
          endByte: callStart + "print(value)".length,
          startByte: callStart,
          startColumn: 0,
          startLine: 1,
        },
      });
      expect(result.violations[0]?.captures.ARG).toEqual([
        expect.objectContaining({ endByte: argumentStart + "value".length, startByte: argumentStart }),
      ]);
    });

    it("enforces repeated metavariable equality", () => {
      const compiled = compileJson([astForbid({ pattern: "$A + $A" })]);
      const result = evaluate(
        compiled,
        snapshot([{ language: "python", path: "equal.py", source: "same + same\nleft + right\n" }]),
      );

      expect(result.violations).toHaveLength(1);
      expect(result.violations[0]?.span?.startLine).toBe(0);
      expect(result.violations[0]?.captures.A).toHaveLength(1);
    });

    it("returns ordered variadic metavariable captures", () => {
      const compiled = compileJson([astForbid({ pattern: "target($$$ARGS)" })]);
      const result = evaluate(
        compiled,
        snapshot([{ language: "python", path: "args.py", source: "target(first, second, third)\n" }]),
      );

      expect(result.violations[0]?.captures.ARGS).toEqual([
        expect.objectContaining({ startByte: 7 }),
        expect.objectContaining({ startByte: 14 }),
        expect.objectContaining({ startByte: 22 }),
      ]);
    });

    it("supports contextual patterns with a selected target node", () => {
      const compiled = compileJson([
        astForbid({
          pattern: { context: "class C:\n  def $METHOD(self):\n    pass", selector: "function_definition" },
        }),
      ]);
      const result = evaluate(
        compiled,
        snapshot([{ language: "python", path: "context.py", source: "class Real:\n  def run(self):\n    pass\n" }]),
      );

      expect(result.violations).toHaveLength(1);
      expect(result.violations[0]?.span?.startLine).toBe(1);
      expect(result.violations[0]?.captures.METHOD).toHaveLength(1);
    });

    it("composes atomic, relational, and boolean rules without config callbacks", () => {
      const compiled = compileJson([
        astForbid({
          all: [
            { pattern: "$OBJ.execute_async($$$ARGS)" },
            { inside: { kind: "function_definition", stopBy: "end" } },
            { not: { inside: { all: [{ kind: "function_definition" }, { regex: "^def test_" }], stopBy: "end" } } },
          ],
        }),
      ]);
      const result = evaluate(
        compiled,
        snapshot([
          {
            language: "python",
            path: "route.py",
            source:
              "def route():\n  return query.execute_async()\n\ndef test_route():\n  return query.execute_async()\n",
          },
        ]),
      );

      expect(result.violations).toHaveLength(1);
      expect(result.violations[0]?.span?.startLine).toBe(1);
    });

    it("supports required descendants through has", () => {
      const compiled = compileJson([
        astForbid({ all: [{ kind: "function_definition" }, { has: { pattern: "dangerous($A)", stopBy: "end" } }] }),
      ]);
      const result = evaluate(
        compiled,
        snapshot([
          {
            language: "python",
            path: "has.py",
            source: "def risky():\n  dangerous(value)\n\ndef safe():\n  return value\n",
          },
        ]),
      );

      expect(result.violations).toHaveLength(1);
      expect(result.violations[0]?.span?.startLine).toBe(0);
    });

    it("supports atomic kind and node-text regex matchers inside any", () => {
      const compiled = compileJson([
        astForbid({
          any: [{ all: [{ kind: "identifier" }, { regex: "^danger" }] }, { pattern: "unsafe($ARG)" }],
        }),
      ]);
      const result = evaluate(
        compiled,
        snapshot([
          {
            language: "python",
            path: "atomic.py",
            source: "danger_name = 1\nsafe_name = 2\nunsafe(value)\n",
          },
        ]),
      );

      expect(result.violations).toHaveLength(2);
      expect(result.violations.map((item) => item.span?.startLine)).toEqual([0, 2]);
    });

    it("supports preceding and following node relations between siblings", () => {
      const source = "def run():\n  prepare()\n  execute()\n";
      const statement = (call: string) => ({ kind: "expression_statement", has: { pattern: call } });
      const preceding = compileJson([
        astForbid({
          all: [statement("prepare()"), { precedes: { ...statement("execute()"), stopBy: "end" } }],
        }),
      ]);
      const following = compileJson([
        astForbid({
          all: [statement("execute()"), { follows: { ...statement("prepare()"), stopBy: "end" } }],
        }),
      ]);
      // The calls themselves have no siblings: each is the only child of its statement.
      const calls = compileJson([
        astForbid({
          all: [{ pattern: "prepare()" }, { precedes: { pattern: "execute()", stopBy: "end" } }],
        }),
      ]);
      const input = snapshot([{ language: "python", path: "relations.py", source }]);

      expect(evaluate(preceding, input).violations).toHaveLength(1);
      expect(evaluate(following, input).violations).toHaveLength(1);
      expect(evaluate(calls, input).violations).toEqual([]);
    });

    it("honors neighbor versus end traversal bounds", () => {
      const source = "def run():\n  prepare()\n  unrelated()\n  execute()\n";
      const statement = (call: string) => ({ kind: "expression_statement", has: { pattern: call } });
      const neighbor = compileJson([
        astForbid({
          all: [statement("execute()"), { follows: { ...statement("prepare()"), stopBy: "neighbor" } }],
        }),
      ]);
      const end = compileJson([
        astForbid({
          all: [statement("execute()"), { follows: { ...statement("prepare()"), stopBy: "end" } }],
        }),
      ]);
      const input = snapshot([{ language: "python", path: "bounds.py", source }]);

      expect(evaluate(neighbor, input).violations).toEqual([]);
      expect(evaluate(end, input).violations).toHaveLength(1);
    });

    it("runs the same witness contract for TypeScript", () => {
      const clause = astForbid({ pattern: "console.log($ARG)" }, "typescript");
      const compiled = compileJson([clause]);
      const result = evaluate(
        compiled,
        snapshot([{ language: "typescript", path: "a.ts", source: "const value = 1;\nconsole.log(value);\n" }]),
      );

      expect(result.violations).toHaveLength(1);
      expect(result.violations[0]).toMatchObject({ path: "a.ts", span: { startLine: 1 } });
      expect(result.violations[0]?.captures.ARG).toHaveLength(1);
    });

    it("does not classify incomplete syntax as a clean pass", () => {
      const compiled = compileJson([astForbid({ pattern: "print($ARG)" })]);
      const result = evaluate(compiled, snapshot([{ language: "python", path: "broken.py", source: "print(" }]));

      // The whole file is an error region, so no AST clause read it: no witness, and the clause did not run.
      expect(result).toMatchObject({ violations: [], status: "failed" });
      expect(result.failures).toEqual([expect.objectContaining({ clauseId: "ast-match", code: "source-incomplete", path: "broken.py" })]);
    });
  });

  describe("JSX grammar selection", () => {
    const jsxSource = [
      "export function View({ raw }: { raw: string }) {",
      "  console.log(raw);",
      "  return <div dangerouslySetInnerHTML={{ __html: raw }} />;",
      "}",
      "",
    ].join("\n");

    it("parses tsx snapshot files with the JSX grammar for typescript rules", () => {
      const compiled = compileJson([astForbid({ pattern: "console.log($ARG)" }, "typescript")]);
      const result = evaluate(
        compiled,
        snapshot([
          { language: "tsx", path: "view.tsx", source: jsxSource },
          { language: "typescript", path: "plain.ts", source: "console.log(1);\n" },
        ]),
      );

      expect(result).toMatchObject({ failures: [], status: "completed" });
      expect(result.violations.map(item => item.path)).toEqual(["plain.ts", "view.tsx"]);
      expect(result.violations.find(item => item.path === "view.tsx")).toMatchObject({ span: { startLine: 1 } });
    });

    it("matches JSX rules in tsx files and leaves plain typescript files inert", () => {
      const compiled = compileJson([astForbid({ pattern: "<div $$$ATTRS />" }, "typescript")]);
      const result = evaluate(
        compiled,
        snapshot([
          { language: "tsx", path: "view.tsx", source: jsxSource },
          { language: "typescript", path: "plain.ts", source: "const view = 1;\n" },
        ]),
      );

      expect(result).toMatchObject({ failures: [], status: "completed" });
      expect(result.violations.map(item => item.path)).toEqual(["view.tsx"]);
    });

    it("matches angle-bracket assertions in typescript files and leaves tsx files inert", () => {
      const compiled = compileJson([astForbid({ pattern: "<Shape>$VALUE" }, "typescript")]);
      const result = evaluate(
        compiled,
        snapshot([
          { language: "tsx", path: "view.tsx", source: jsxSource },
          { language: "typescript", path: "plain.ts", source: "const shape = <Shape>raw;\n" },
        ]),
      );

      expect(result).toMatchObject({ failures: [], status: "completed" });
      expect(result.violations.map(item => item.path)).toEqual(["plain.ts"]);
    });

    it("names every grammar's reason when no typescript dialect accepts a rule", () => {
      const rule = { has: { kind: "type_assertion" }, kind: "jsx_opening_element" };
      expect(() => compileJson([astForbid(rule, "typescript")])).toThrow(
        /rule\/kind: .*jsx_opening_element.* \(typescript grammar\); .*rule\/has\/kind: .*type_assertion.* \(tsx grammar\)/,
      );
    });

    it("never matches a sub-rule in a dialect that cannot express it, keeping the rest of the rule live", () => {
      const outsideJsx = compileJson([astForbid({ kind: "call_expression", not: { inside: { kind: "jsx_expression", stopBy: "end" } } }, "typescript")]);
      const eitherBranch = compileJson([astForbid({ any: [{ kind: "jsx_attribute" }, { pattern: "g($A)" }] }, "typescript")]);
      const files = snapshot([
        { language: "typescript", path: "plain.ts", source: "g(1);\n" },
        { language: "tsx", path: "view.tsx", source: "const v = <b id=\"x\">{g(2)}</b>;\n" },
      ]);

      expect(evaluate(outsideJsx, files)).toMatchObject({ failures: [], status: "completed" });
      expect(evaluate(outsideJsx, files).violations.map(item => item.path)).toEqual(["plain.ts"]);
      expect(evaluate(eitherBranch, files).violations.map(item => item.path)).toEqual(["plain.ts", "view.tsx", "view.tsx"]);
    });
  });

  describe("no silent ignores", () => {
    const regexForbid = { regex: { pattern: "x", source: "file.content" } };
    it.each([
      ["a malformed stopBy rule", astForbid({ inside: { kind: "call", stopBy: { kind: "module", field: "body" } }, kind: "identifier" }), /unknown field `field` at \/clauses\/0\/forbid\/ast\/rule\/inside\/stopBy/],
      ["numeric stopBy", astForbid({ has: { kind: "identifier", stopBy: 1 }, kind: "call" }), /rule\/has\/stopBy must be "neighbor", "end" or a rule object/],
      ["array kind alone", astForbid({ kind: ["call"] }), /rule\/kind must be a non-empty string$/],
      ["array kind beside a pattern", astForbid({ kind: ["call"], pattern: "print($A)" }), /rule\/kind must be a non-empty string$/],
      ["non-string nested regex", astForbid({ has: { regex: 1 }, kind: "call" }), /rule\/has\/regex must be a non-empty string$/],
      ["the removed ast capture field", { ...astForbid({ pattern: "print($A)" }), forbid: { ast: { capture: "A", language: "python", rule: { pattern: "print($A)" } } } }, /unknown field `capture` at \/clauses\/0\/forbid\/ast/],
      ["the removed regex captures field", { violation: violation("X"), forbid: { regex: { ...regexForbid.regex, captures: ["x"] } }, id: "x" }, /unknown field `captures` at \/clauses\/0\/forbid\/regex/],
      ["an unknown anchor", { violation: { ...violation("X"), anchor: "subject.id" }, forbid: regexForbid, id: "x" }, /violation\/anchor: unknown anchor `subject\.id`/],
      ["a capture anchor the query cannot bind", { ...astForbid({ pattern: "print($A)", not: { pattern: "print($B)" } }), violation: { ...violation("X"), anchor: "capture.B" } }, /`capture\.B` names no capture; the query can bind: A/],
      ["a message capture the query cannot bind", { ...astForbid({ pattern: "print($A)" }), violation: { ...violation("X"), message: "print of $ARG" } }, /violation\/message: `\$ARG` names no capture/],
      ["finding variables", { violation: { ...violation("X"), variables: { name: "x" } }, forbid: regexForbid, id: "x" }, /violation\/variables: violation variables are not supported yet/],
    ])("rejects %s", (_name, clause, error) => {
      expect(() => compileJson([clause])).toThrow(error);
    });

    it("accepts empty violation variables", () => {
      expect(() => compileJson([{ violation: { ...violation("X"), variables: {} }, forbid: regexForbid, id: "x" }])).not.toThrow();
    });


    it("bounds every pattern source by the capabilities' byte limits", () => {
      const tight = capabilities({ limits: { ...defaultLimits, maxAstPatternBytes: 8, maxRegexPatternBytes: 4 } });
      expect(() => compileJson([astForbid({ pattern: "f($A, 1)" })], tight)).not.toThrow();
      expect(() => compileJson([astForbid({ pattern: "f($A, 10)" })], tight)).toThrow(
        "/clauses/0/forbid/ast/rule/pattern is 9 bytes; capabilities.limits.maxAstPatternBytes allows at most 8",
      );
      expect(() => compileJson([astForbid({ pattern: { context: "class A:\n  x = 1", selector: "assignment" } })], tight)).toThrow(
        /rule\/pattern\/context is 16 bytes; capabilities.limits.maxAstPatternBytes/,
      );
      expect(() => compileJson([astForbid({ kind: "call", regex: "^abcd" })], tight)).toThrow(
        /rule\/regex is 5 bytes; capabilities.limits.maxRegexPatternBytes allows at most 4/,
      );
      expect(() => compileJson([{ violation: violation("X"), forbid: { regex: { pattern: "abcde", source: "file.path" } }, id: "x" }], tight)).toThrow(
        /forbid\/regex\/pattern is 5 bytes; capabilities.limits.maxRegexPatternBytes/,
      );
    });

    it("reports shape errors once and without a grammar suffix", () => {
      expect(() => compileJson([astForbid({ has: { kinds: "identifier" }, kind: "call_expression" }, "typescript")])).toThrow(
        /^unknown field `kinds` at \/clauses\/0\/forbid\/ast\/rule\/has$/,
      );
    });
  });

  describe("constraint semantics", () => {
    it("emits every regex witness with byte-accurate locations", () => {
      const compiled = compileJson([
        {
          violation: violation("RAW_EXECUTE"),
          forbid: { regex: { pattern: "execute_async\\(\\)", source: "file.content" } },
          id: "raw-execute",
        },
      ]);
      const source = "execute_async()\nallowed()\nexecute_async()\n";
      const result = evaluate(compiled, snapshot([{ language: "python", path: "calls.py", source }]));

      expect(result.status).toBe("completed");
      expect(result.violations.map((item) => item.span?.startByte)).toEqual([0, source.lastIndexOf("execute_async()")]);
    });
    it("matches file paths without inspecting file contents", () => {
      const compiled = compileJson([
        {
          violation: violation("REQUIRED_FILE"),
          forbid: { regex: { pattern: "required\\.json$", source: "file.path" } },
          id: "required-file",
        },
      ]);
      const result = evaluate(
        compiled,
        snapshot([
          { language: "python", path: ".required.json", source: "" },
          { language: "python", path: "other.json", source: "required.json" },
        ]),
      );

      expect(result.violations).toHaveLength(1);
      expect(result.violations[0]).toMatchObject({
        code: "REQUIRED_FILE",
        path: ".required.json",
        span: { startLine: 0 },
      });
    });
    it("selects repository witnesses only when another query has evidence", () => {
      const compiled = compileJson([
        {
          violation: violation("CROSS_FILE"),
          forbid: {
            select_when: {
              exists: { regex: { pattern: "danger\\(", source: "file.content" } },
              select: { regex: { pattern: "baseline\\.json$", source: "file.path" } },
            },
          },
          id: "cross-file",
        },
      ]);
      const baseline = { language: "python" as const, path: ".baseline.json", source: "" };

      const violating = evaluate(
        compiled,
        snapshot([baseline, { language: "python", path: "live.py", source: "danger()\n" }]),
      );
      const compliant = evaluate(
        compiled,
        snapshot([baseline, { language: "python", path: "live.py", source: "safe()\n" }]),
      );

      expect(violating.violations).toHaveLength(1);
      expect(violating.violations[0]).toMatchObject({ path: ".baseline.json", span: { startLine: 0 } });
      expect(compliant).toMatchObject({ violations: [], status: "completed" });
    });
    it("applies portable include and exclude scopes before matching", () => {
      const source = codeRule(
        [
          {
            violation: violation("RAW_EXECUTE"),
            forbid: { regex: { pattern: "execute_async\\(\\)", source: "file.content" } },
            id: "raw-execute",
          },
        ],
        { scope: { exclude: ["**/generated/**"], include: ["src/**/*.py"] } },
      );
      const compiled = nativeRuntime().compile(source, "json", capabilities());
      const result = evaluate(
        compiled,
        snapshot([
          { language: "python", path: "src/live/service.py", source: "execute_async()\n" },
          { language: "python", path: "src/generated/service.py", source: "execute_async()\n" },
          { language: "python", path: "tests/service.py", source: "execute_async()\n" },
        ]),
      );

      expect(result.violations.map((item) => item.path)).toEqual(["src/live/service.py"]);
    });

    // contract: `limits.maxSourceBytes` bounds what a code rule reads. A code rule scoped to `src/**` must not be
    // refused for the size of files it will never open.
    it("measures the input budget against the files in scope, not the whole snapshot", () => {
      const source = codeRule(
        [
          {
            violation: violation("RAW_EXECUTE"),
            forbid: { regex: { pattern: "execute_async\\(\\)", source: "file.content" } },
            id: "raw-execute",
          },
        ],
        { scope: { exclude: [], include: ["src/**/*.py"] } },
      );
      const constrained = capabilities({ limits: { ...defaultLimits, maxSourceBytes: 64 } });
      const compiled = nativeRuntime().compile(source, "json", constrained);
      expect(compiled.inScope("src/live/service.py")).toBe(true);
      expect(compiled.inScope("web/huge.py")).toBe(false);
      const result = evaluate(
        compiled,
        snapshot([
          { language: "python", path: "src/live/service.py", source: "execute_async()\n" },
          { language: "python", path: "web/huge.py", source: `# ${"x".repeat(10_000)}\n` },
        ]),
      );

      expect(result.failures).toEqual([]);
      expect(result.violations.map((item) => item.path)).toEqual(["src/live/service.py"]);

      // The budget still binds inside the scope.
      const inScope = evaluate(
        compiled,
        snapshot([{ language: "python", path: "src/live/big.py", source: `execute_async()\n# ${"x".repeat(10_000)}\n` }]),
      );
      expect(inScope).toMatchObject({ violations: [], status: "failed" });
      expect(inScope.failures).toEqual([expect.objectContaining({ clauseId: "runtime", code: "budget-exceeded" })]);
    });


    it.each([
      [
        "example-data-layer-layer-violations",
        "impl/module.py",
        "# module header\nfrom impl.internal import Service\n",
        "layer_violations",
      ],
      [
        "example-app-internal-import-in-function",
        "api/module.py",
        "# module header\ndef load():\n    from api import service\n",
        "INTERNAL_IMPORT_IN_FUNCTION",
      ],
      [
        "example-app-impl-bootstrap-import",
        "api/module.py",
        "# module header\nfrom impl.core.bootstrap import bootstrap\n",
        "IMPL_BOOTSTRAP_IMPORT",
      ],
      [
        "example-app-inmemory-index",
        "api/module.py",
        "# module header\n_records_index: dict[str, object] = {}\n",
        "INMEMORY_INDEX",
      ],
      [
        "example-app-unsafe-cache",
        "api/module.py",
        "# module header\n_records_cache: dict[str, object] = {}\n",
        "UNSAFE_CACHE",
      ],
    ])("matches %s violations after the first source line", (codeRuleName, path, source, code) => {
      const result = evaluate(compileCodeRuleFile(codeRuleName), snapshot([{ language: "python", path, source }]));

      expect(result).toMatchObject({ status: "completed", failures: [] });
      expect(result.violations).toHaveLength(1);
      expect(result.violations[0]).toMatchObject({ code, path });
    });

    it("limits eager resolve violations to explicit plan scope bodies", () => {
      const compiled = compileCodeRuleFile("example-data-layer-plan-scope-side-effects");
      const unrelated = evaluate(
        compiled,
        snapshot([
          {
            language: "python",
            path: "impl/common/paths.py",
            source: "from pathlib import Path\nROOT = Path(__file__).resolve()\n",
          },
        ]),
      );
      const inPlanScope = evaluate(
        compiled,
        snapshot([
          {
            language: "python",
            path: "impl/common/plan.py",
            source:
              "def load(conn, lazy):\n    with conn.session().open_write_plan() as scope:\n        value = lazy.resolve()\n        scope.execute()\n",
          },
        ]),
      );

      expect(unrelated).toMatchObject({ status: "completed", violations: [], failures: [] });
      expect(inPlanScope).toMatchObject({ status: "completed", failures: [] });
      expect(inPlanScope.violations).toHaveLength(1);
      expect(inPlanScope.violations[0]).toMatchObject({
        code: "plan_scope_side_effects",
        path: "impl/common/plan.py",
      });
    });

    it.each([
      [
        "example-app-conn-based-function",
        "def load(conn: SyncDatabaseConnection):\n    return conn.session()\n",
        "def load(txn: ReadTransaction):\n    return txn.query()\n",
        "CONN_BASED_FUNCTION",
      ],
      [
        "example-app-sync-conn-in-async",
        "async def load(db_conn):\n    return db_conn.read(fetch)\n",
        "async def load(db_conn):\n    return await db_conn.read_async(fetch)\n",
        "SYNC_CONN_IN_ASYNC",
      ],
      [
        "example-app-json-loads-attrs",
        "def load(raw: str):\n    return json.loads(raw)\n",
        "def load(raw: str):\n    return Record.model_validate(json.loads(raw))\n",
        "JSON_LOADS_ATTRS",
      ],
      [
        "example-app-model-dump-no-alias",
        "def export(model):\n    return model.model_dump()\n",
        "def export(model):\n    return model.model_dump(by_alias=True)\n",
        "MODEL_DUMP_NO_ALIAS",
      ],
      [
        "example-app-stored-connection",
        "class Service:\n    def __init__(self, conn: SyncDatabaseConnection):\n        self.conn = conn\n",
        "class Service:\n    def __init__(self, pool: DatabasePool):\n        self.pool = pool\n",
        "STORED_CONNECTION",
      ],
      [
        "example-app-str-artifact-node-id",
        "class Edge:\n    src_id: str\n",
        "class Edge:\n    src_id: ArtifactNodeId\n",
        "STR_ARTIFACT_NODE_ID",
      ],
      [
        "example-app-str-userid-field",
        "class Record:\n    owner_id: str\n",
        "class Record:\n    owner_id: UserId\n",
        "STR_USERID_FIELD",
      ],
    ])("executes formerly inert policy %s", (codeRuleName, violatingSource, allowedSource, code) => {
      const compiled = compileCodeRuleFile(codeRuleName);
      const result = evaluate(
        compiled,
        snapshot([{ language: "python", path: "api/module.py", source: violatingSource }]),
      );
      const allowed = evaluate(
        compiled,
        snapshot([{ language: "python", path: "api/module.py", source: allowedSource }]),
      );

      expect(result).toMatchObject({ status: "completed", failures: [] });
      expect(result.violations).toHaveLength(1);
      expect(result.violations[0]).toMatchObject({ code, path: "api/module.py" });
      expect(allowed).toMatchObject({ status: "completed", violations: [], failures: [] });
    });

    it("returns a concrete failure instead of partial violations when a hard limit is exceeded", () => {
      const constrained = capabilities({ limits: { ...defaultLimits, maxWitnesses: 1 } });
      const compiled = compileJson([astForbid({ pattern: "print($ARG)" })], constrained);
      const result = evaluate(
        compiled,
        snapshot([{ language: "python", path: "many.py", source: "print(one)\nprint(two)\n" }]),
      );

      expect(result).toMatchObject({ violations: [], status: "failed" });
      expect(result.failures).toEqual([expect.objectContaining({ clauseId: "ast-match", code: "budget-exceeded" })]);
    });


    it("keeps LLM evaluation standalone and reports unavailable models as a concrete failure", () => {
      const compiled = compileJson([
        {
          violation: violation("INTENT"),
          id: "intent",
          llm: {
            input: { source: "changed-symbol" },
            rubric: "Fail on direct persistence access",
            verdicts: ["pass", "fail"],
          },
        },
      ]);
      const result = evaluate(
        compiled,
        JSON.stringify({
          changedSymbol: { id: "symbol:one", source: "save_directly()" },
          facts: [],
          files: [],
          format: "rerail.repository-snapshot/v1",
          revision: 0,
        }),
      );

      expect(result.status).toBe("failed");
      expect(result.failures).toEqual([expect.objectContaining({ clauseId: "intent", code: "missing-capability" })]);
    });
  });

  describe("determinism and incremental behavior", () => {
    it("sorts violations independently of file input order and preserves stable IDs", () => {
      const compiled = compileJson([astForbid({ pattern: "print($ARG)" })]);
      const a = { language: "python" as const, path: "a.py", source: "print(a)\n" };
      const b = { language: "python" as const, path: "b.py", source: "print(b)\n" };
      const forward = evaluate(compiled, snapshot([a, b]));
      const reverse = evaluate(compiled, snapshot([b, a]));

      expect(reverse).toEqual(forward);
      expect(forward.violations.map((item) => item.path)).toEqual(["a.py", "b.py"]);
      expect(new Set(forward.violations.map((item) => item.id)).size).toBe(2);
    });

    it("short-circuits decision mode without changing execution status", () => {
      const compiled = compileJson([astForbid({ pattern: "print($ARG)" })]);
      const input = snapshot([{ language: "python", path: "many.py", source: "print(one)\nprint(two)\n" }]);
      const diagnostic = evaluate(compiled, input, "report");
      const decision = evaluate(compiled, input, "decision");

      expect(decision.status).toBe(diagnostic.status);
      expect(decision.metrics.witnesses).toBe(1);
      expect(diagnostic.metrics.witnesses).toBe(2);
    });

    it("retracts and adds stable violations across ordered incremental edits", () => {
      const compiled = compileJson([astForbid({ pattern: "print($ARG)" })]);
      const session = compiled.openEditor(snapshot([{ language: "python", path: "edit.py", source: "print(old)\n" }]));
      const initial = incrementalResultSchema.parse(JSON.parse(session.evaluate("report")));
      const repaired = incrementalResultSchema.parse(
        JSON.parse(
          session.applyEdit(
            JSON.stringify({
              newText: "value",
              oldEndByte: 5,
              path: "edit.py",
              revision: 1,
              startByte: 0,
            }),
            "report",
          ),
        ),
      );
      const regressed = incrementalResultSchema.parse(
        JSON.parse(
          session.applyEdit(
            JSON.stringify({
              newText: "print",
              oldEndByte: 5,
              path: "edit.py",
              revision: 2,
              startByte: 0,
            }),
            "report",
          ),
        ),
      );

      expect(initial).toMatchObject({ revision: 0, status: "completed" });
      expect(repaired).toMatchObject({ addedViolationIds: [], revision: 1, status: "completed" });
      expect(repaired.retractedViolationIds).toEqual([initial.violations[0]?.id]);
      expect(regressed).toMatchObject({ retractedViolationIds: [], revision: 2, status: "completed" });
      expect(regressed.addedViolationIds).toEqual([regressed.violations[0]?.id]);
      expect(regressed.violations[0]?.id).toBe(initial.violations[0]?.id);
    });

    it("rejects out-of-order edits without mutating the session", () => {
      const compiled = compileJson([astForbid({ pattern: "print($ARG)" })]);
      const session = compiled.openEditor(snapshot([{ language: "python", path: "edit.py", source: "print(old)\n" }]));

      expect(() =>
        session.applyEdit(
          JSON.stringify({ newText: "value", oldEndByte: 5, path: "edit.py", revision: 2, startByte: 0 }),
          "report",
        ),
      ).toThrow(/out.of.order.*expected.*1/i);
      expect(incrementalResultSchema.parse(JSON.parse(session.evaluate("report")))).toMatchObject({
        revision: 0,
        status: "completed",
      });
    });

    it("rejects invalid edit ranges without advancing the incremental revision", () => {
      const compiled = compileJson([astForbid({ pattern: "print($ARG)" })]);
      const session = compiled.openEditor(snapshot([{ language: "python", path: "edit.py", source: "print(old)\n" }]));

      expect(() =>
        session.applyEdit(
          JSON.stringify({ newText: "value", oldEndByte: 10_000, path: "edit.py", revision: 1, startByte: 0 }),
          "report",
        ),
      ).toThrow();
      expect(incrementalResultSchema.parse(JSON.parse(session.evaluate("report")))).toMatchObject({
        revision: 0,
        status: "completed",
      });
    });
  });
});

function workspace(): NativeCodeRuleDslWorkspace {
  const constructor = loadNativeBinding().CodeRuleDslWorkspace;
  if (constructor === undefined) {
    throw new Error("Native binding must export CodeRuleDslWorkspace before this contract can pass");
  }
  return new constructor();
}

const workspaceResultSchema = z.strictObject({
  revision: z.number(),
  replacedPaths: z.array(z.string()).nullable(),
  evaluation: evaluationResultSchema.extend({
    failures: z.array(codeRuleFailureSchema.extend({ ruleId: z.string() })),
  }),
});

async function applyWorkspace(nativeWorkspace: NativeCodeRuleDslWorkspace, revision: number, files: Array<{ path: string; language: string; source: string; module?: string | string[] }>, removed: string[] = []) {
  return workspaceResultSchema.parse(JSON.parse(await nativeWorkspace.applyAsync(JSON.stringify({ revision, files, removed })))).evaluation;
}

describe.skipIf(codeRuleDslRuntime === undefined)("native code-rule DSL workspace", () => {
  it("commits an unchanged inline resend without re-evaluating its resident file", async () => {
    const compiled = compileJson([{
      id: "needle",
      forbid: { regex: { pattern: "needle", source: "file.content" } },
      violation: violation("NEEDLE"),
    }]);
    const failureCodeRule = nativeRuntime().compile(codeRule([{
      id: "limited",
      forbid: { regex: { pattern: "needle", source: "file.content" } },
      violation: violation("LIMITED"),
    }], { id: "runtime-failure" }), "json", capabilities({
      limits: { ...defaultLimits, maxWitnesses: 0 },
    }));
    const inline = workspace();
    const asynchronous = workspace();
    inline.addCodeRule(compiled);
    asynchronous.addCodeRule(compiled);
    inline.addCodeRule(failureCodeRule);
    asynchronous.addCodeRule(failureCodeRule);
    const file = { language: "python", path: "a.py", source: "needle\n" };
    const request = (revision: number) => JSON.stringify({ revision, files: [file], removed: [] });
    try {
      const initial = inline.tryApply(request(1));
      expect(initial).not.toBeNull();
      const seeded = workspaceResultSchema.parse(JSON.parse(initial!));
      expect(seeded.evaluation.failures).toHaveLength(1);
      expect(seeded).toEqual(
        workspaceResultSchema.parse(JSON.parse(await asynchronous.applyAsync(request(1)))),
      );

      const resend = inline.tryApply(request(2));
      expect(resend).not.toBeNull();
      const result = workspaceResultSchema.parse(JSON.parse(resend!));
      expect(result).toEqual(workspaceResultSchema.parse(JSON.parse(await asynchronous.applyAsync(request(2)))));
      expect(result.revision).toBe(2);
      expect(result.evaluation.violations).toHaveLength(1);
      expect(result.evaluation.failures).toEqual(seeded.evaluation.failures);
      expect(result.evaluation.metrics).toMatchObject({ files: 0, witnesses: 0 });
      expect(() => inline.tryApply(request(2))).toThrow();
      await expect(asynchronous.applyAsync(request(2))).rejects.toThrow();
      const next = inline.tryApply(request(3));
      expect(next).not.toBeNull();
      expect(workspaceResultSchema.parse(JSON.parse(next!))).toEqual(
        workspaceResultSchema.parse(JSON.parse(await asynchronous.applyAsync(request(3)))),
      );
    } finally {
      inline.close();
      asynchronous.close();
    }
  });

  it("does not admit an out-of-scope inline file or track its later edits", async () => {
    const compiled = nativeRuntime().compile(codeRule([{
      id: "needle",
      forbid: { regex: { pattern: "needle", source: "file.content" } },
      violation: violation("NEEDLE"),
    }], { scope: { exclude: [], include: ["src/**"] } }), "json", capabilities());
    const inline = workspace();
    const asynchronous = workspace();
    inline.addCodeRule(compiled);
    asynchronous.addCodeRule(compiled);
    const valid = { language: "python", path: "src/a.py", source: "needle\n" };
    const rejected = { language: "python", path: "docs/a.py", source: "needle\n" };
    const request = (revision: number, file: typeof valid) => JSON.stringify({ revision, files: [file], removed: [] });
    try {
      expect(inline.acceptsPath(rejected.path)).toBe(false);
      const initial = inline.tryApply(request(1, valid));
      expect(initial).not.toBeNull();
      expect(workspaceResultSchema.parse(JSON.parse(initial!))).toEqual(
        workspaceResultSchema.parse(JSON.parse(await asynchronous.applyAsync(request(1, valid)))),
      );

      const ignored = inline.tryApply(request(2, rejected));
      expect(ignored).not.toBeNull();
      const result = workspaceResultSchema.parse(JSON.parse(ignored!));
      expect(result).toEqual(workspaceResultSchema.parse(JSON.parse(await asynchronous.applyAsync(request(2, rejected)))));
      expect(result.revision).toBe(2);
      expect(result.evaluation.violations).toEqual(
        workspaceResultSchema.parse(JSON.parse(initial!)).evaluation.violations,
      );
      expect(result.evaluation.metrics).toMatchObject({ files: 0, witnesses: 0 });

      const later = request(3, { ...rejected, source: "needle needle\n" });
      const laterInline = workspaceResultSchema.parse(JSON.parse(await inline.applyAsync(later)));
      expect(laterInline).toEqual(workspaceResultSchema.parse(JSON.parse(await asynchronous.applyAsync(later))));
      expect(laterInline.evaluation.metrics.files).toBe(0);
    } finally {
      inline.close();
      asynchronous.close();
    }
  });

  it.each(["typescript", "tsx"] as const)(
    "keeps shared AST candidates grammar-specific when %s arrives first",
    async firstLanguage => {
      const logged = { ast: {
        language: "typescript",
        rule: { all: [{ pattern: "console.log($ARG)" }, { regex: "^console\\.log\\(" }] },
      } };
      const compiled = compileJson([
        { violation: violation("LOG"), forbid: logged, id: "log" },
        {
          violation: violation("WARN_WHEN_LOG"),
          forbid: { select_when: {
            exists: logged,
            select: { ast: {
              language: "typescript",
              rule: { pattern: "console.warn($ARG)" },
            } },
          } },
          id: "warn-when-log",
        },
      ]);
      const secondLanguage = firstLanguage === "typescript" ? "tsx" : "typescript";
      const file = (path: string, language: "typescript" | "tsx", argument?: string) => ({
        path,
        language,
        source: [
          ...(argument === undefined ? [] : [`console.log(${argument});`]),
          "console.warn(warning);",
          ...(language === "tsx" ? ["const view = <div />;"] : []),
          "",
        ].join("\n"),
      });
      const revisions = [
        [file("a.ts", firstLanguage, "initialA"), file("b.tsx", secondLanguage, "initialB")],
        [file("a.ts", secondLanguage, "editedA"), file("b.tsx", firstLanguage, "editedB")],
        [file("a.ts", secondLanguage), file("b.tsx", firstLanguage)],
      ];
      const nativeWorkspace = workspace();
      try {
        nativeWorkspace.addCodeRule(compiled);
        for (const [revision, files] of revisions.entries()) {
          const full = evaluate(compiled, snapshot(files));
          const actual = await applyWorkspace(nativeWorkspace, revision + 1, files);
          expect(full).toMatchObject({ status: "completed", failures: [] });
          expect({ status: actual.status, violations: actual.violations, failures: actual.failures }).toEqual({
            status: full.status, violations: full.violations, failures: [],
          });
          for (const input of files) {
            const argument = revision === 0
              ? input.path === "a.ts" ? "initialA" : "initialB"
              : input.path === "a.ts" ? "editedA" : "editedB";
            expect(actual.violations.filter(item => item.path === input.path).map(item => ({
              clause: item.clauseId,
              text: item.span && input.source.slice(item.span.startByte, item.span.endByte),
              arguments: item.captures.ARG?.map(span => input.source.slice(span.startByte, span.endByte)),
            }))).toEqual(revision === 2 ? [] : [
              { clause: "log", text: `console.log(${argument})`, arguments: [argument] },
              { clause: "warn-when-log", text: "console.warn(warning)", arguments: ["warning"] },
            ]);
          }
        }
      } finally {
        nativeWorkspace.close();
      }
    },
  );

  it("evaluates a rule with a never-matching dialect sub-rule the same as a full evaluation", async () => {
    const compiled = compileJson([
      { ...astForbid({ kind: "call_expression", not: { inside: { kind: "jsx_expression", stopBy: "end" } } }, "typescript"), id: "outside-jsx" },
      { ...astForbid({ any: [{ kind: "jsx_attribute" }, { pattern: "g($A)" }] }, "typescript"), id: "either-branch" },
    ]);
    const files = [
      { language: "typescript", path: "plain.ts", source: "g(1);\n" },
      { language: "tsx", path: "view.tsx", source: "const v = <b id=\"x\">{g(2)}</b>;\nh(3);\n" },
    ] as const;
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const full = evaluate(compiled, snapshot([...files]));
      const actual = await applyWorkspace(nativeWorkspace, 1, [...files]);
      expect(full).toMatchObject({ failures: [], status: "completed" });
      expect(full.violations.map(item => `${item.clauseId}:${item.path}`).sort()).toEqual([
        "either-branch:plain.ts", "either-branch:view.tsx", "either-branch:view.tsx", "outside-jsx:plain.ts", "outside-jsx:view.tsx",
      ]);
      expect({ status: actual.status, violations: actual.violations, failures: actual.failures }).toEqual({
        status: full.status, violations: full.violations, failures: [],
      });
    } finally {
      nativeWorkspace.close();
    }
  });

  it("retains unaffected violations and removes deleted files", async () => {
    const nativeWorkspace = workspace();
    nativeWorkspace.addCodeRule(
      compileJson([
        {
          violation: violation("RAW_EXECUTE"),
          forbid: { regex: { pattern: "execute_async\\(\\)", source: "file.content" } },
          id: "raw-execute",
        },
      ]),
    );

    const initial = JSON.parse(
      await nativeWorkspace.applyAsync(
        JSON.stringify({
          files: [
            { language: "python", path: "a.py", source: "query.execute_async()\n" },
            { language: "python", path: "b.py", source: "query.execute_async()\n" },
          ],
          removed: [],
          revision: 1,
        }),
      ),
    );
    expect(initial.evaluation.violations).toHaveLength(2);

    const edited = JSON.parse(
      await nativeWorkspace.applyAsync(
        JSON.stringify({
          files: [{ language: "python", path: "a.py", source: "query.execute()\n" }],
          removed: ["b.py"],
          revision: 2,
        }),
      ),
    );
    expect(edited.evaluation.violations).toEqual([]);
    nativeWorkspace.close();
  });

  it("keeps scoped code rule input under its own cap", async () => {
    const nativeWorkspace = workspace();
    nativeWorkspace.addCodeRule(
      nativeRuntime().compile(
        codeRule(
          [
            {
              violation: violation("SCOPED"),
              forbid: { regex: { pattern: "forbidden", source: "file.content" } },
              id: "scoped",
            },
          ],
          { scope: { exclude: [], include: ["src/**"] } },
        ),
        "json",
        capabilities({ limits: { ...defaultLimits, maxSourceBytes: 16 } }),
      ),
    );

    const result = JSON.parse(
      await nativeWorkspace.applyAsync(
        JSON.stringify({
          files: [
            { language: "python", path: "src/a.py", source: "forbidden\n" },
            { language: "python", path: "vendor/b.py", source: "x".repeat(1_000) },
          ],
          removed: [],
          revision: 1,
        }),
      ),
    );
    expect(result.evaluation.status).toBe("completed");
    expect(result.evaluation.violations).toHaveLength(1);
    nativeWorkspace.close();
  });
  it("evaluates contextual AST and retained cross-file queries after the baseline completes", async () => {
    const nativeWorkspace = workspace();
    nativeWorkspace.addCodeRule(
      compileJson([
        astForbid({
          pattern: { context: "class C:\n  def $METHOD(self):\n    pass", selector: "function_definition" },
        }),
        {
          violation: violation("CROSS_FILE"),
          forbid: {
            select_when: {
              exists: { regex: { pattern: "danger\\(", source: "file.content" } },
              select: { regex: { pattern: "baseline\\.json$", source: "file.path" } },
            },
          },
          id: "cross-file",
        },
      ]),
    );

    const result = JSON.parse(
      await nativeWorkspace.applyAsync(
        JSON.stringify({
          files: [
            { language: "python", path: "context.py", source: "class Real:\n  def run(self):\n    pass\n" },
            { language: "python", path: "danger.py", source: "danger()\n" },
            { language: "typescript", path: "baseline.json", source: "{}\n" },
          ],
          removed: [],
          revision: 1,
        }),
      ),
    );
    expect(result.evaluation.violations.map((item: { code: string }) => item.code)).toEqual([
      "CROSS_FILE",
      "AST_MATCH",
    ]);
  });

  it("matches full evaluation after UTF-8 span shifts, contextual edits, syntax damage, and recovery", async () => {
    const nativeWorkspace = workspace();
    const compiled = compileJson([
      { ...astForbid({ pattern: "print($ARG)" }), id: "local" },
      { ...astForbid({ all: [{ pattern: "print($ARG)" }, { inside: { kind: "function_definition", stopBy: "end" } }] }), id: "context" },
    ]);
    nativeWorkspace.addCodeRule(compiled);
    const unchanged = { language: "python" as const, path: "other.py", source: "print('untouched')\n" };
    const sources = [
      "def f():\n    print('é')\nprint('outside')\n",
      "# inserted λ\n\ndef f():\n    print('é')\nprint('outside')\n",
      "# inserted λ\n\ndef f():\n    log('é')\nprint('outside')\n",
      "if True:\n    print('é')\nprint('outside')\n",
      "def broken(:\n    print('é')\n",
      "def repaired():\n    print('recovered')\n",
    ];
    try {
      for (let index = 0; index < sources.length; index += 1) {
        const edited = { language: "python" as const, path: "edit.py", source: sources[index]! };
        const actual = await applyWorkspace(nativeWorkspace, index + 1, index === 0 ? [unchanged, edited] : [edited]);
        const full = evaluate(compiled, snapshot([unchanged, edited]));
        // A damaged file is evaluated around its error regions and marked partial on both paths.
        if (full.status === "failed") {
          expect(actual.status).toBe("failed");
          expect(actual.violations).toEqual(full.violations);
          const identities = (failures: Array<{ clauseId: string; code: string; path?: string | undefined }>) =>
            failures.map(({ clauseId, code, path }) => ({ clauseId, code, path }))
              .sort((left, right) => left.clauseId.localeCompare(right.clauseId));
          expect(identities(actual.failures)).toEqual(identities(full.failures));
          expect(identities(actual.failures)).toEqual([
            { clauseId: "context", code: "source-partial", path: "edit.py" },
            { clauseId: "local", code: "source-partial", path: "edit.py" },
          ]);
          expect(actual.violations.filter(item => item.path === "edit.py").map(item => item.clauseId).sort())
            .toEqual(["context", "local"]);
        } else {
          expect(actual.failures).toEqual([]);
          expect(actual.violations).toEqual(full.violations);
          if (index === 1) {
            const shifted = actual.violations.find(item => item.clauseId === "local" && item.path === "edit.py");
            expect(shifted?.span).toEqual({
              startByte: 28, endByte: 39, startLine: 3, startColumn: 4, endLine: 3, endColumn: 15,
            });
            expect(shifted?.captures.ARG).toEqual([{
              startByte: 34, endByte: 38, startLine: 3, startColumn: 10, endLine: 3, endColumn: 14,
            }]);
          }
        }
        expect(actual.metrics.files).toBe(index === 0 ? 2 : 1);
      }
    } finally { nativeWorkspace.close(); }
  });

  it("scans a corpus larger than the input cap in bounded file units and evaluates only the edit", async () => {
    const nativeWorkspace = workspace();
    const compiled = nativeRuntime().compile(codeRule([astForbid({ pattern: "print($ARG)" })]), "json", capabilities({ limits: { ...defaultLimits, maxFiles: 1, maxSourceBytes: 40 } }));
    nativeWorkspace.addCodeRule(compiled);
    try {
      const files = Array.from({ length: 20 }, (_, index) => ({ language: "python", path: `file-${index}.py`, source: "print('violation')\n" }));
      const initial = await applyWorkspace(nativeWorkspace, 1, files);
      expect(initial.failures).toEqual([]);
      expect(initial.violations.map(item => item.path).sort()).toEqual(files.map(file => file.path).sort());
      const changed = await applyWorkspace(nativeWorkspace, 2, [{ ...files[0]!, source: "log('fixed')\n" }]);
      expect(changed.metrics.files).toBe(1);
      expect(changed.violations.map(item => item.path).sort()).toEqual(files.slice(1).map(file => file.path).sort());
    } finally { nativeWorkspace.close(); }
  });

  it("preserves scoped rule violations with an unrestricted fallback", async () => {
    const nativeWorkspace = workspace();
    const compiled = compileJson([
      { ...astForbid({ pattern: "print($ARG)" }), id: "print" },
      { ...astForbid({ kind: "identifier" }), id: "identifier" },
      { ...astForbid({ any: [{ pattern: "print($ARG)" }, { not: { pattern: "log($ARG)" } }] }), id: "unrestricted" },
    ]);
    nativeWorkspace.addCodeRule(compiled);
    try {
      const file = { language: "python" as const, path: "edit.py", source: "print(outside)\nlog(inside)\n" };
      const full = evaluate(compiled, snapshot([file]));
      const result = await applyWorkspace(nativeWorkspace, 1, [file]);
      expect(full).toMatchObject({ status: "completed", failures: [] });
      expect({ status: result.status, violations: result.violations, failures: result.failures }).toEqual({
        status: full.status, violations: full.violations, failures: [],
      });
      expect(result.violations.filter(item => item.clauseId === "print")).toEqual([
        expect.objectContaining({
          span: expect.objectContaining({ startByte: 0, endByte: 14 }),
          captures: expect.objectContaining({
            ARG: [expect.objectContaining({ startByte: 6, endByte: 13 })],
          }),
        }),
      ]);
      expect(result.violations.filter(item => item.clauseId === "identifier").map(item =>
        item.span && file.source.slice(item.span.startByte, item.span.endByte))).toEqual([
        "print", "outside", "log", "inside",
      ]);
      // The negated structural arm cannot be subtracted exactly by the index.
      // Its unrestricted domain must retain both the module and leaf witnesses.
      expect(result.violations.filter(item => item.clauseId === "unrestricted")).toEqual(expect.arrayContaining([
        expect.objectContaining({ span: expect.objectContaining({ startByte: 0, endByte: file.source.length }) }),
        expect.objectContaining({ span: expect.objectContaining({ startByte: 6, endByte: 13 }) }),
        expect.objectContaining({
          captures: expect.objectContaining({
            ARG: [expect.objectContaining({ startByte: 6, endByte: 13 })],
          }),
        }),
      ]));
    } finally { nativeWorkspace.close(); }
  });

  it.each(["neighbor", "default"] as const)(
    "preserves preceding and following sibling boundaries with %s stopBy",
    async stopBy => {
      const stop = stopBy === "neighbor" ? { stopBy } : {};
      const prepareStatement = { kind: "expression_statement", regex: "^prepare\\(\\)$" };
      const executeStatement = { kind: "expression_statement", regex: "^execute\\(\\)$" };
      const compiled = compileJson([
        { ...astForbid({ all: [prepareStatement, { precedes: { ...executeStatement, ...stop } }] }), id: "statement-precedes" },
        { ...astForbid({ all: [executeStatement, { follows: { ...prepareStatement, ...stop } }] }), id: "statement-follows" },
        { ...astForbid({ all: [{ pattern: "prepare()" }, { precedes: { pattern: "execute()", ...stop } }] }), id: "call-precedes" },
        { ...astForbid({ all: [{ pattern: "execute()" }, { follows: { pattern: "prepare()", ...stop } }] }), id: "call-follows" },
      ]);
      const file = {
        language: "python" as const,
        path: "neighbors.py",
        source: [
          "execute()",
          "prepare()",
          "execute()",
          "prepare()",
          "unrelated()",
          "execute()",
          "wrap(prepare())",
          "execute()",
          "prepare()",
          "wrap(execute())",
          "pair(prepare(), execute())",
          "prepare()",
          "",
        ].join("\n"),
      };
      const nativeWorkspace = workspace();
      try {
        nativeWorkspace.addCodeRule(compiled);
        const full = evaluate(compiled, snapshot([file]));
        const actual = await applyWorkspace(nativeWorkspace, 1, [file]);
        expect(full).toMatchObject({ status: "completed", failures: [] });
        expect({ status: actual.status, violations: actual.violations, failures: actual.failures }).toEqual({
          status: full.status, violations: full.violations, failures: [],
        });
        // Siblings only: a statement's neighbor is the adjacent statement itself, never a call
        // nested in it. A call is the only child of its statement and, inside an argument list,
        // its neighbors are the `(`, `,` and `)` tokens, so the call rules never match.
        for (const [clauseId, expected] of [
          ["statement-precedes", [{ line: 1, text: "prepare()" }]],
          ["statement-follows", [{ line: 2, text: "execute()" }]],
          ["call-precedes", []],
          ["call-follows", []],
        ] as const) {
          expect(actual.violations.filter(item => item.clauseId === clauseId).map(item => ({
            line: item.span?.startLine,
            text: item.span && file.source.slice(item.span.startByte, item.span.endByte),
          }))).toEqual(expected);
        }
      } finally {
        nativeWorkspace.close();
      }
    },
  );

  it("completes multiple AST clauses at the single-file traversal budget", async () => {
    const file = { language: "python" as const, path: "budget.py", source: "print(value)\nlog(other)\n" };
    const clauses = [
      { ...astForbid({ pattern: "print($ARG)" }), id: "print" },
      { ...astForbid({ pattern: "log($ARG)" }), id: "log" },
      { ...astForbid({ kind: "identifier" }), id: "identifier" },
    ];
    const input = snapshot([file]);
    const single = evaluate(compileJson([clauses[0]]), input);
    expect(single).toMatchObject({ status: "completed", failures: [] });
    const full = evaluate(compileJson(clauses), input);
    const compiled = compileJson(clauses, capabilities({
      limits: { ...defaultLimits, maxVisitedNodes: single.metrics.visitedNodes },
    }));
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const actual = await applyWorkspace(nativeWorkspace, 1, [file]);
      expect(full).toMatchObject({ status: "completed", failures: [] });
      expect({ status: actual.status, violations: actual.violations, failures: actual.failures }).toEqual({
        status: full.status, violations: full.violations, failures: [],
      });
      for (const [clauseId, expected] of [
        ["print", ["print(value)"]],
        ["log", ["log(other)"]],
        ["identifier", ["print", "value", "log", "other"]],
      ] as const) {
        expect(actual.violations.filter(item => item.clauseId === clauseId).map(item =>
          item.span && file.source.slice(item.span.startByte, item.span.endByte))).toEqual(expected);
      }
    } finally {
      nativeWorkspace.close();
    }
  });

  it("preserves dense pattern captures after selective same-node narrowing in either conjunction order", async () => {
    const pattern = { pattern: "print($ARG)" };
    const regex = { regex: "^print\\(value_(?:7|31)\\)$" };
    const compiled = compileJson([
      { ...astForbid({ all: [regex, pattern] }), id: "regex-first" },
      { ...astForbid({ all: [pattern, regex] }), id: "pattern-first" },
    ]);
    const file = {
      language: "python" as const,
      path: "dense-pattern.py",
      source: Array.from({ length: 40 }, (_, index) => `print(value_${index})\n`).join(""),
    };
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const full = evaluate(compiled, snapshot([file]));
      const actual = await applyWorkspace(nativeWorkspace, 1, [file]);
      expect(full).toMatchObject({ status: "completed", failures: [] });
      expect({ status: actual.status, violations: actual.violations, failures: actual.failures }).toEqual({
        status: full.status, violations: full.violations, failures: [],
      });
      for (const clauseId of ["regex-first", "pattern-first"]) {
        expect(actual.violations.filter(item => item.clauseId === clauseId).map(item => ({
          line: item.span?.startLine,
          text: item.span && file.source.slice(item.span.startByte, item.span.endByte),
          arguments: item.captures.ARG?.map(span => file.source.slice(span.startByte, span.endByte)),
        }))).toEqual([
          { line: 7, text: "print(value_7)", arguments: ["value_7"] },
          { line: 31, text: "print(value_31)", arguments: ["value_31"] },
        ]);
      }
    } finally {
      nativeWorkspace.close();
    }
  });

  it.each([
    {
      name: "an Any branch with no root-kind restriction",
      language: "python" as const,
      rule: { any: [{ pattern: "print($ARG)" }, { regex: "^outside$" }] },
      source: "outside = 1\nprint(value)\n",
    },
    {
      name: "ordered Any captures",
      language: "python" as const,
      rule: { any: [{ pattern: "print($FIRST)" }, { pattern: "print($SECOND)" }] },
      source: "print(value)\n",
    },
    {
      name: "negated enclosing relations",
      language: "python" as const,
      rule: { all: [
        { pattern: "$OBJ.execute_async($$$ARGS)" },
        { inside: { kind: "function_definition", stopBy: "end" } },
        { not: { inside: { all: [{ kind: "function_definition" }, { regex: "^def test_" }], stopBy: "end" } } },
      ] },
      source: "def route():\n  query.execute_async()\ndef test_route():\n  query.execute_async()\n",
    },
    {
      name: "descendant requirements on an enclosing witness",
      language: "python" as const,
      rule: { all: [{ kind: "function_definition" }, { has: { all: [{ pattern: "dangerous($A)" }, { regex: "^dangerous\\(" }], stopBy: "end" } }] },
      source: "def risky():\n  dangerous(value)\n\ndef safe():\n  return value\n",
    },
    {
      name: "preceding and following anchors outside the witness subtree",
      language: "python" as const,
      rule: { any: [
        { all: [{ pattern: "prepare()" }, { precedes: { all: [{ pattern: "execute()" }, { regex: "^execute\\(" }], stopBy: "end" } }] },
        { all: [{ pattern: "execute()" }, { follows: { all: [{ pattern: "prepare()" }, { regex: "^prepare\\(" }], stopBy: "end" } }] },
      ] },
      source: "def run():\n  prepare()\n  unrelated()\n  execute()\n",
    },
    {
      name: "a contextual selector without requiring the unselected class name",
      language: "python" as const,
      rule: { pattern: { context: "class C:\n  def $METHOD(self):\n    pass", selector: "function_definition" } },
      source: "class Real:\n  def run(self):\n    pass\n",
    },
    {
      name: "repeated metavariable equality",
      language: "python" as const,
      rule: { pattern: "$A + $A" },
      source: "same + same\nleft + right\n",
    },
    {
      name: "empty and trailing-comma variadic captures",
      language: "python" as const,
      rule: { pattern: "target($$$ARGS)" },
      source: "target()\ntarget(first, second,)\ntarget(third)\n",
    },
    {
      name: "named terminals after variadic captures under Smart matching",
      language: "python" as const,
      rule: { pattern: "print($$$BEFORE, marker, $$$AFTER)" },
      source: "print(a, marker, b)\nprint(marker)\nprint(a, other, b)\n",
    },
    {
      name: "Python attribute identifiers",
      language: "python" as const,
      rule: { pattern: "console.log($ARG)" },
      source: "console.log(value)\nconsole.warn(other)\n",
    },
    {
      name: "TypeScript property identifiers",
      language: "typescript" as const,
      rule: { pattern: "console.log($ARG)" },
      source: "console.log(value);\nconsole.warn(other);\n",
    },
    {
      name: "a regex-only root without a kind restriction",
      language: "python" as const,
      rule: { regex: "^needle$" },
      source: "value = needle\nother = needless\n",
    },
    {
      name: "a regex before a capturing pattern in a conjunction",
      language: "python" as const,
      rule: { all: [{ regex: "^target\\(" }, { pattern: "target($ARG)" }] },
      source: "target(first)\nother(second)\ntarget(third)\n",
    },
    {
      name: "a regex after a capturing pattern in a conjunction",
      language: "python" as const,
      rule: { all: [{ pattern: "target($ARG)" }, { regex: "^target\\(" }] },
      source: "target(first)\nother(second)\ntarget(third)\n",
    },
    {
      name: "anchored Unicode prefixes on both sides of eight characters",
      language: "python" as const,
      rule: { all: [{ kind: "identifier" }, { regex: "^(?:é短|αβγδεζηθικ)" }] },
      source: "short = é短suffix\nlong = αβγδεζηθικsuffix\nnear = αβγδεζηθother\n",
    },
    {
      name: "Unicode case-insensitive and scoped case-sensitive flags",
      language: "python" as const,
      rule: { all: [{ kind: "identifier" }, { any: [
        { regex: "(?i)^éclair$" },
        { regex: "^(?i:mixed)Tail$" },
      ] }] },
      source: "first = ÉCLAIR\nsecond = MiXeDTail\nthird = mixedtail\n",
    },
    {
      name: "multiline line starts inside a node rather than at its start",
      language: "python" as const,
      rule: { all: [{ kind: "string" }, { regex: "(?m)^needle" }] },
      source: "value = \"\"\"first line\nneedle in the middle\nlast line\"\"\"\n",
    },
    {
      name: "substrings within identifiers rather than whole words",
      language: "python" as const,
      rule: { all: [{ kind: "identifier" }, { regex: "needle" }] },
      source: "value = prefix_needlesuffix\nother = absent\n",
    },
    {
      name: "long free text within comments",
      language: "python" as const,
      rule: { all: [{ kind: "comment" }, { regex: "needle with spaces" }] },
      source: `# ${"padding ".repeat(40)}needle with spaces ${"suffix ".repeat(40)}\nvalue = 1\n`,
    },
    {
      name: "interior-node text spanning punctuation and an escaped string leaf",
      language: "python" as const,
      rule: { all: [
        { pattern: "print($ARG)" },
        { regex: "^print\\('before\\\\nafter'\\)$" },
      ] },
      source: "print('before\\nafter')\nprint('before after')\n",
    },
    {
      name: "an unindexable alternative to an absent literal",
      language: "python" as const,
      rule: { all: [{ kind: "identifier" }, { any: [{ regex: "^absent$" }, { regex: "^\\w+$" }] }] },
      source: "value = present\n",
    },
    {
      name: "an empty regex alternative to an absent literal",
      language: "python" as const,
      rule: { all: [{ kind: "identifier" }, { any: [{ regex: "^absent$" }, { regex: "^(?:missing|)" }] }] },
      source: "value = present\n",
    },
    {
      name: "negation without a positive text predicate",
      language: "python" as const,
      rule: { all: [{ kind: "identifier" }, { not: { regex: "^blocked$" } }] },
      source: "value = allowed\nother = blocked\n",
    },
    {
      name: "neighbor descendants without promoting deeper matches",
      language: "python" as const,
      rule: { all: [
        { kind: "call" },
        { has: { all: [{ kind: "identifier" }, { regex: "^target$" }], stopBy: "neighbor" } },
      ] },
      source: "target(value)\nouter(target)\n",
    },
    {
      name: "end descendants including matches below the nearest child",
      language: "python" as const,
      rule: { all: [
        { kind: "call" },
        { has: { all: [{ kind: "identifier" }, { regex: "^target$" }], stopBy: "end" } },
      ] },
      source: "target(value)\nouter(target)\n",
    },
    {
      name: "neighbor ancestors without promoting more distant matches",
      language: "python" as const,
      rule: { all: [
        { kind: "identifier" },
        { inside: { all: [{ kind: "call" }, { regex: "^target\\(" }], stopBy: "neighbor" } },
      ] },
      source: "target(value)\nother(target)\n",
    },
    {
      name: "end ancestors including matches beyond the nearest parent",
      language: "python" as const,
      rule: { all: [
        { kind: "identifier" },
        { inside: { all: [{ kind: "call" }, { regex: "^target\\(" }], stopBy: "end" } },
      ] },
      source: "target(value)\nother(target)\n",
    },
  ])("preserves exact witnesses and captures for $name", async ({ language, rule, source }) => {
    const nativeWorkspace = workspace();
    const compiled = compileJson([astForbid(rule, language)]);
    const file = { language, path: language === "python" ? "rules.py" : "rules.ts", source };
    nativeWorkspace.addCodeRule(compiled);
    try {
      const full = evaluate(compiled, snapshot([file]));
      const actual = await applyWorkspace(nativeWorkspace, 1, [file]);
      expect(full).toMatchObject({ status: "completed", failures: [] });
      expect({ status: actual.status, violations: actual.violations, failures: actual.failures }).toEqual({
        status: full.status,
        violations: full.violations,
        failures: full.failures.map(failure => ({ ruleId: "runtime-contract", ...failure })),
      });
    } finally {
      nativeWorkspace.close();
    }
  });

  it.each(["python", "typescript"] as const)(
    "preserves dense vocabulary and long-text predicate results in %s",
    async language => {
      const comment = language === "python" ? "#" : "//";
      const callKind = language === "python" ? "call" : "call_expression";
      const source = Array.from({ length: 32 }, (_, index) => [
        `value_${index} = ${index % 2 ? "prefix_needlesuffix" : "needle_repeat"}`,
        `${comment} ${index % 3 ? "αβγδεζηθικ" : "αβγδεζηθother"} ${"λ".repeat(180)} ${index}`,
        `print(value_${index})`,
      ].join("\n")).join("\n") + "\n";
      const rules = [
        { kind: "identifier", regex: "needle" },
        { kind: "identifier", regex: "^needle" },
        { kind: "identifier", not: { regex: "needle" } },
        { kind: "comment", regex: "^(?:#|//) αβ" },
        { kind: "comment", regex: "^(?:#|//) αβγδεζηθικ" },
        { kind: "comment", regex: "(?i)^(?:#|//) ΑΒΓΔΕΖΗΘΙΚ" },
        { kind: "comment", regex: "λ.* 31$" },
        { kind: callKind, has: { kind: "identifier", regex: "^print$", stopBy: "neighbor" } },
        { kind: "identifier", inside: { kind: callKind, regex: "^print\\(", stopBy: "end" } },
      ];
      const compiled = compileJson(rules.map((rule, index) => ({
        ...astForbid(rule, language), id: `predicate-${index}`,
      })));
      const nativeWorkspace = workspace();
      nativeWorkspace.addCodeRule(compiled);
      const file = { language, path: language === "python" ? "dense.py" : "dense.ts", source };
      try {
        const full = evaluate(compiled, snapshot([file]));
        const actual = await applyWorkspace(nativeWorkspace, 1, [file]);
        expect(full).toMatchObject({ status: "completed", failures: [] });
        expect({ status: actual.status, violations: actual.violations, failures: actual.failures }).toEqual({
          status: full.status, violations: full.violations, failures: [],
        });
      } finally {
        nativeWorkspace.close();
      }
    },
  );

  it("rebinds the same path and source across Python and TypeScript, deletion, and recreation", async () => {
    const compiled = compileJson([
      { ...astForbid({ pattern: "print($ARG)" }), id: "python", violation: violation("PYTHON_PRINT") },
      { ...astForbid({ pattern: "print($ARG)" }, "typescript"), id: "typescript", violation: violation("TYPESCRIPT_PRINT") },
    ]);
    const nativeWorkspace = workspace();
    const source = "print(value)\n";
    const stable = { language: "python" as const, path: "stable.py", source };
    const python = { language: "python" as const, path: "active", source };
    const typescript = { language: "typescript" as const, path: "active", source };
    const changes = [
      { files: [stable, python], removed: [], active: python, code: "PYTHON_PRINT" },
      { files: [typescript], removed: [], active: typescript, code: "TYPESCRIPT_PRINT" },
      { files: [typescript], removed: [], active: typescript, code: "TYPESCRIPT_PRINT" },
      { files: [], removed: ["active"], active: undefined, code: undefined },
      { files: [python], removed: [], active: python, code: "PYTHON_PRINT" },
      { files: [typescript], removed: [], active: typescript, code: "TYPESCRIPT_PRINT" },
    ];
    nativeWorkspace.addCodeRule(compiled);
    try {
      for (const [index, change] of changes.entries()) {
        const actual = await applyWorkspace(nativeWorkspace, index + 1, change.files, change.removed);
        const full = evaluate(compiled, snapshot([stable, ...(change.active === undefined ? [] : [change.active])]));
        expect(full).toMatchObject({ status: "completed", failures: [] });
        expect({ status: actual.status, violations: actual.violations, failures: actual.failures }).toEqual({
          status: full.status,
          violations: full.violations,
          failures: [],
        });
        expect(actual.violations.filter(item => item.path === "active").map(item => item.code)).toEqual(
          change.code === undefined ? [] : [change.code],
        );
        expect(actual.violations.filter(item => item.path === stable.path).map(item => item.code)).toEqual(["PYTHON_PRINT"]);
      }
    } finally {
      nativeWorkspace.close();
    }
  });

  it("replaces literal and predicate values, clears their last match, and rebuilds after context, deletion, and language changes", async () => {
    const nativeWorkspace = workspace();
    const compiled = compileJson([
      { ...astForbid({ pattern: "print('needle')" }), id: "python" },
      { ...astForbid({ pattern: "console.log('needle')" }, "typescript"), id: "typescript" },
      { ...astForbid({ all: [{ kind: "string" }, { regex: "^'token'$" }] }), id: "value" },
      { ...astForbid({ all: [
        { pattern: "print($ARG)" },
        { inside: { all: [{ kind: "function_definition" }, { regex: "^def active\\(" }], stopBy: "end" } },
      ] }), id: "context" },
    ]);
    const unchanged = { language: "python" as const, path: "unchanged.py", source: "print('needle')\n" };
    const states = [
      { language: "python" as const, source: "print('needle')\n" },
      { language: "python" as const, source: "print('thread')\n" },
      { language: "python" as const, source: "trace('thread')\n" },
      { language: "python" as const, source: "print('needle')\n" },
      undefined,
      { language: "python" as const, source: "print('needle')\n" },
      { language: "typescript" as const, source: "console.log('needle');\n" },
      { language: "python" as const, source: "console.log('needle');\n" },
      { language: "typescript" as const, source: "console.log('needle');\n" },
      { language: "python" as const, source: "def active():\n  print('token')\n" },
      { language: "python" as const, source: "def idle():\n  print('token')\n" },
      { language: "python" as const, source: "def idle():\n  print('other')\n" },
      { language: "python" as const, source: "def active():\n  print('other')\n" },
      undefined,
      { language: "python" as const, source: "def active():\n  print('token')\n" },
      undefined,
      { language: "python" as const, source: "def idle():\n  print('token')\n" },
    ];
    nativeWorkspace.addCodeRule(compiled);
    try {
      for (const [index, state] of states.entries()) {
        const edited = state === undefined ? [] : [{ ...state, path: "edit.py" }];
        const actual = await applyWorkspace(
          nativeWorkspace, index + 1, index === 0 ? [unchanged, ...edited] : edited,
          state === undefined ? ["edit.py"] : [],
        );
        const full = evaluate(compiled, snapshot([unchanged, ...edited]));
        expect(full).toMatchObject({ status: "completed", failures: [] });
        expect({ status: actual.status, violations: actual.violations, failures: actual.failures }).toEqual({
          status: full.status,
          violations: full.violations,
          failures: full.failures.map(failure => ({ ruleId: "runtime-contract", ...failure })),
        });
      }
    } finally {
      nativeWorkspace.close();
    }
  });

  it.each(["safe()\n", "safe()\nprint(value)\n"])(
    "does not turn exhausted traversal into an absent-pattern pass for %j",
    async source => {
      const nativeWorkspace = workspace();
      const compiled = compileJson(
        [astForbid({ pattern: "print($ARG)" })],
        capabilities({ limits: { ...defaultLimits, maxVisitedNodes: 1 } }),
      );
      nativeWorkspace.addCodeRule(compiled);
      try {
        const result = await applyWorkspace(nativeWorkspace, 1, [{ language: "python", path: "limited.py", source }]);
        expect(result).toMatchObject({ status: "failed", violations: [] });
        expect(result.failures.map(({ ruleId, clauseId, code, path }) => ({ ruleId, clauseId, code, path }))).toEqual([
          { ruleId: "runtime-contract", clauseId: "ast-match", code: "budget-exceeded", path: "limited.py" },
        ]);
      } finally {
        nativeWorkspace.close();
      }
    },
  );

  it("applies witness limits to real predicate matches and recovers when the last value disappears", async () => {
    const nativeWorkspace = workspace();
    const compiled = compileJson(
      [astForbid({ all: [{ pattern: "print($ARG)" }, { regex: "^print\\(" }] })],
      capabilities({ limits: { ...defaultLimits, maxWitnesses: 0 } }),
    );
    nativeWorkspace.addCodeRule(compiled);
    try {
      for (const [index, source] of ["safe()\n", "print(value)\n", "safe(value)\n"].entries()) {
        const file = { language: "python" as const, path: "limited.py", source };
        const actual = await applyWorkspace(nativeWorkspace, index + 1, [file]);
        const full = evaluate(compiled, snapshot([file]));
        expect({ status: actual.status, violations: actual.violations }).toEqual({
          status: full.status,
          violations: full.violations,
        });
        if (index === 1) {
          expect(actual).toMatchObject({ status: "failed", violations: [] });
          expect(actual.failures.map(({ ruleId, clauseId, code, path }) => ({ ruleId, clauseId, code, path }))).toEqual([
            { ruleId: "runtime-contract", clauseId: "ast-match", code: "budget-exceeded", path: "limited.py" },
          ]);
        } else {
          expect(full).toMatchObject({ status: "completed", violations: [], failures: [] });
          expect(actual.failures).toEqual([]);
        }
      }
    } finally {
      nativeWorkspace.close();
    }
  });

  it("retains AST selections only while complete evidence remains, including deletion and recovery", async () => {
    const nativeWorkspace = workspace();
    const compiled = compileJson([{
      id: "cross-file", violation: violation("CROSS_FILE"),
      forbid: { select_when: {
        select: { ast: { language: "python", rule: { pattern: "report($ARG)" } } },
        exists: { ast: { language: "python", rule: { pattern: "print('marker')" } } },
      } },
    }]);
    const selected = { language: "python" as const, path: "selected.py", source: "report(first)\nreport(second)\n" };
    const evidence = { language: "python" as const, path: "evidence.py", source: "print('marker')\n" };
    const backup = { ...evidence, path: "backup.py" };
    const files = new Map([selected, evidence, backup].map(file => [file.path, file]));
    const changes = [
      { files: [selected, evidence, backup], removed: [], matched: true },
      { files: [{ ...evidence, source: "print('absent')\n" }], removed: [], matched: true },
      { files: [], removed: ["backup.py"], matched: false },
      { files: [evidence], removed: [], matched: true },
      { files: [{ ...evidence, source: "def broken(:\n" }], removed: [], matched: false },
      { files: [evidence], removed: [], matched: true },
      { files: [], removed: ["evidence.py"], matched: false },
    ];
    nativeWorkspace.addCodeRule(compiled);
    try {
      for (const [index, change] of changes.entries()) {
        for (const path of change.removed) files.delete(path);
        for (const file of change.files) files.set(file.path, file);
        const actual = await applyWorkspace(nativeWorkspace, index + 1, change.files, change.removed);
        if (index === 4) {
          // Absent evidence in a damaged file is not a clean pass: the global clause reports it partial.
          const full = evaluate(compiled, snapshot([...files.values()]));
          expect(actual).toMatchObject({ status: "failed", violations: [] });
          expect(full).toMatchObject({ status: "failed", violations: [] });
          const identities = (failures: Array<{ clauseId: string; code: string; path?: string | undefined }>) =>
            failures.map(({ clauseId, code, path }) => ({ clauseId, code, path }));
          expect(identities(actual.failures)).toEqual([{ clauseId: "cross-file", code: "source-partial", path: "evidence.py" }]);
          expect(identities(full.failures)).toEqual(identities(actual.failures));
        } else {
          const full = evaluate(compiled, snapshot([...files.values()]));
          expect(full).toMatchObject({ status: "completed", failures: [] });
          expect(actual).toMatchObject({ status: "completed", failures: [] });
          expect(actual.violations).toEqual(full.violations);
          expect(actual.violations.map(item => [item.path, item.span?.startLine])).toEqual(
            change.matched ? [["selected.py", 0], ["selected.py", 1]] : [],
          );
        }
      }
    } finally {
      nativeWorkspace.close();
    }
  });

  it("evaluates cross-file source conditions from cached units beyond the repository input cap", async () => {
    const nativeWorkspace = workspace();
    const compiled = nativeRuntime().compile(codeRule([{
      id: "cross-file", violation: violation("CROSS_FILE"),
      forbid: { select_when: {
        select: { regex: { pattern: "baseline\\.json$", source: "file.path" } },
        exists: { ast: { language: "python", rule: { pattern: "print($ARG)" } } },
      } },
    }]), "json", capabilities({ limits: { ...defaultLimits, maxFiles: 1, maxSourceBytes: 40 } }));
    nativeWorkspace.addCodeRule(compiled);
    try {
      const files = Array.from({ length: 20 }, (_, index) => ({ language: "python", path: `file-${index}.py`, source: "safe()\n" }));
      files.push({ language: "python", path: "baseline.json", source: "{}" });
      const initial = await applyWorkspace(nativeWorkspace, 1, files);
      expect(initial).toMatchObject({ violations: [], failures: [], status: "completed" });
      const changed = await applyWorkspace(nativeWorkspace, 2, [{ ...files[0]!, source: "print('violation')\n" }]);
      expect(changed.failures).toEqual([]);
      expect(changed.violations).toEqual([expect.objectContaining({ code: "CROSS_FILE", path: "baseline.json" })]);
      const cleared = await applyWorkspace(nativeWorkspace, 3, [files[0]!]);
      expect(cleared).toMatchObject({ violations: [], failures: [], status: "completed" });
    } finally { nativeWorkspace.close(); }
  });
  it("checks the source budget before parsing and recovers cached AST witnesses", async () => {
    const nativeWorkspace = workspace();
    const compiled = nativeRuntime().compile(
      codeRule([astForbid({ pattern: "print($ARG)" })]),
      "json",
      capabilities({ limits: { ...defaultLimits, maxSourceBytes: 24 } }),
    );
    nativeWorkspace.addCodeRule(compiled);
    try {
      const malformed = await applyWorkspace(nativeWorkspace, 1, [{
        language: "python",
        path: "edit.py",
        source: "def broken(:\n    print('x')\n",
      }]);
      expect(malformed).toMatchObject({ violations: [], status: "failed" });
      expect(malformed.failures).toEqual([
        expect.objectContaining({ clauseId: "ast-match", code: "budget-exceeded", path: "edit.py" }),
      ]);

      const violating = await applyWorkspace(nativeWorkspace, 2, [{
        language: "python",
        path: "edit.py",
        source: "print('x')\n",
      }]);
      expect(violating.status).toBe("completed");
      expect(violating.failures).toEqual([]);
      expect(violating.violations).toEqual([expect.objectContaining({ code: "AST_MATCH", path: "edit.py" })]);

      const oversized = await applyWorkspace(nativeWorkspace, 3, [{
        language: "python",
        path: "edit.py",
        source: "safe()\n" + "x".repeat(100),
      }]);
      expect(oversized).toMatchObject({ violations: [], status: "failed" });
      expect(oversized.failures).toEqual([
        expect.objectContaining({ clauseId: "ast-match", code: "budget-exceeded", path: "edit.py" }),
      ]);

      const recovered = await applyWorkspace(nativeWorkspace, 4, [{
        language: "python",
        path: "edit.py",
        source: "safe()\n",
      }]);
      expect(recovered).toMatchObject({ violations: [], failures: [], status: "completed" });
    } finally {
      nativeWorkspace.close();
    }
  });
  describe("violation anchors, message interpolation, and suggested fixes", () => {
    const reported = (id: string, forbid: unknown, spec: Record<string, unknown>) => ({ violation: { ...violation(id.toUpperCase()), ...spec }, forbid, id });
    const ast = (rule: Record<string, unknown>) => ({ ast: { language: "typescript", rule } });
    const clauses = [
      reported("log", ast({ pattern: "console.log($$$ARGS)" }), { anchor: "capture.ARGS", message: "log of $$$ARGS", fix: "logger.info($$$ARGS)" }),
      reported("ret", ast({ kind: "return_statement", has: { pattern: "$V" } }), { anchor: "witness.end", message: "returns $V" }),
      reported("start", ast({ kind: "class_declaration", has: { kind: "type_identifier", pattern: "$NAME" } }), { anchor: "witness.start", message: "class $NAME" }),
      reported("todo", { regex: { pattern: "TODO\\((?<WHO>[a-z]+)\\)", source: "file.content" } }, { anchor: "capture.WHO", message: "todo for $WHO", fix: "TODO($WHO, #1)" }),
      reported("button", ast({ pattern: "<button $$$ATTRS>$$$BODY</button>" }), { message: "raw <button> with $$$ATTRS", fix: "<Button $$$ATTRS>$$$BODY</Button>" }),
    ];
    const sources = [
      "export class Store {}\nfunction f() {\n  console.log(a, {\n    b,\n  });\n  // TODO(ann)\n  return a;\n}\n",
      "// head\nexport class Store {}\nfunction f() {\n  x; console.log(a, {\n    b,\n  });\n  // TODO(bob)\n  return a + 1;\n}\n",
      "// head\nexport class Store {}\nfunction f() {\n      x; console.log(a, {\n    b,\n  });\n  // TODO(bob)\n  return a + 1;\n}\n",
    ];
    const tsx = "export const V = () => <button type=\"button\">Go</button>;\n";

    it("reports anchored spans, interpolated messages, and fixes on the stateless path", () => {
      const result = evaluate(compileJson(clauses), snapshot([
        { language: "typescript", path: "a.ts", source: sources[0]! },
        { language: "tsx", path: "v.tsx", source: tsx },
      ]));
      expect(result.failures).toEqual([]);
      const byClause = new Map(result.violations.map(item => [item.clauseId, item]));
      const at = (id: string) => {
        const span = byClause.get(id)!.span!;
        return [span.startLine, span.startColumn, span.endLine, span.endColumn];
      };
      expect(at("log")).toEqual([2, 14, 4, 3]);
      expect(byClause.get("log")).toMatchObject({ message: "log of a, {…", fix: { text: "logger.info(a, {\n    b,\n  })", span: { startLine: 2, startColumn: 2 } } });
      expect(at("ret")).toEqual([6, 11, 6, 11]);
      expect(byClause.get("ret")?.message).toBe("returns a");
      expect(at("start")).toEqual([0, 7, 0, 7]);
      expect(byClause.get("start")?.message).toBe("class Store");
      expect(at("todo")).toEqual([5, 10, 5, 13]);
      expect(byClause.get("todo")).toMatchObject({ message: "todo for ann", fix: { text: "TODO(ann, #1)" }, captures: { WHO: [expect.objectContaining({ startColumn: 10 })] } });
      expect(byClause.get("button")).toMatchObject({ path: "v.tsx", message: "raw <button> with type=\"button\"", fix: { text: "<Button type=\"button\">Go</Button>" } });
    });

    it("keeps workspace violations identical to a fresh evaluation across edits", async () => {
      const compiled = compileJson(clauses);
      const nativeWorkspace = workspace();
      nativeWorkspace.addCodeRule(compiled);
      try {
        for (const [index, source] of sources.entries()) {
          const incremental = await applyWorkspace(nativeWorkspace, index + 1, [{ language: "typescript", path: "a.ts", source }]);
          const fresh = evaluate(compiled, snapshot([{ language: "typescript", path: "a.ts", source }]));
          expect(incremental.violations).toEqual(fresh.violations);
          expect(incremental.violations.filter(item => item.fix !== undefined)).toHaveLength(2);
        }
      } finally {
        nativeWorkspace.close();
      }
    });
  });

  describe("error-tolerant evaluation around syntax errors", () => {
    const tsClause = (id: string, rule: Record<string, unknown>) => ({ ...astForbid(rule, "typescript"), id });
    const identities = (failures: Array<{ clauseId: string; code: string; path?: string | undefined }>) =>
      failures.map(({ clauseId, code, path }) => ({ clauseId, code, path }))
        .sort((left, right) => left.clauseId.localeCompare(right.clauseId));
    // Syntax errors TypeScript rejects too (a missing comma; a bare `>` in JSX text), so the error regions stay
    // whatever the grammars fix.
    const missingComma = 'const m = importOriginal(1 load("./api"));\ndebugger;\n';
    const jsxGreaterThan = "export const View = () => <p>Domain > capability map</p>;\ndebugger;\n";

    it.each([
      ["typescript", "api.test.ts", missingComma],
      ["tsx", "map.tsx", jsxGreaterThan],
    ] as const)("reports sound %s witnesses outside the error region and marks the file partial", async (language, path, source) => {
      const nativeWorkspace = workspace();
      const compiled = compileJson([
        tsClause("debugger", { kind: "debugger_statement" }),
        tsClause("no-class", { all: [{ kind: "program" }, { not: { has: { kind: "class_declaration", stopBy: "end" } } }] }),
        tsClause("unguarded", { all: [{ kind: "debugger_statement" }, { not: { follows: { kind: "import_statement", stopBy: "end" } } }] }),
      ]);
      nativeWorkspace.addCodeRule(compiled);
      try {
        const actual = await applyWorkspace(nativeWorkspace, 1, [{ language, path, source }]);
        const full = evaluate(compiled, snapshot([{ language, path, source }]));
        expect(actual.status).toBe("failed");
        expect(actual.violations).toEqual(full.violations);
        expect(actual.violations.map(item => [item.clauseId, item.span?.startLine])).toEqual([["debugger", 1]]);
        expect(identities(actual.failures)).toEqual([
          { clauseId: "debugger", code: "source-partial", path },
          { clauseId: "no-class", code: "source-partial", path },
          { clauseId: "unguarded", code: "source-incomplete", path },
        ]);
        expect(identities(full.failures)).toEqual(identities(actual.failures));
        expect(actual.failures.find(item => item.clauseId === "debugger")?.message).toMatch(/first at line 1, column \d+/);
      } finally {
        nativeWorkspace.close();
      }
    });

    it("keeps anchor-selected candidates outside error regions on the indexed workspace path", async () => {
      // The stateless path visits every node; the workspace selects roots through the anchor index.
      const nativeWorkspace = workspace();
      const compiled = compileJson([
        tsClause("pattern", { pattern: "console.log($A)" }),
        tsClause("kind-has", { kind: "call_expression", has: { kind: "identifier", regex: "^report$", stopBy: "end" } }),
        tsClause("inside", { kind: "debugger_statement", inside: { kind: "function_declaration", stopBy: "end" } }),
        tsClause("any", { any: [{ pattern: "report($X)" }, { kind: "debugger_statement" }] }),
        tsClause("not-pattern", { all: [{ kind: "call_expression" }, { not: { pattern: "console.log($A)" } }] }),
        tsClause("jsx", { kind: "jsx_attribute", has: { kind: "property_identifier", regex: "^title$" } }),
      ]);
      nativeWorkspace.addCodeRule(compiled);
      try {
        const files = [
          { language: "typescript" as const, path: "api.test.ts", source: `${missingComma}console.log(m);\nfunction run() {\n  report(1);\n  debugger;\n}\n` },
          { language: "tsx" as const, path: "map.tsx", source: `${jsxGreaterThan}export const Title = () => <h1 title="map">{report(2)}</h1>;\n` },
        ];
        const actual = await applyWorkspace(nativeWorkspace, 1, files);
        const full = evaluate(compiled, snapshot(files as never));
        expect(actual.violations).toEqual(full.violations);
        expect(identities(actual.failures)).toEqual(identities(full.failures));
        expect(new Set(actual.violations.map(item => item.clauseId)))
          .toEqual(new Set(["pattern", "kind-has", "inside", "any", "not-pattern", "jsx"]));
        expect(new Set(actual.failures.map(item => item.code))).toEqual(new Set(["source-partial"]));
      } finally {
        nativeWorkspace.close();
      }
    });

    it("suppresses witnesses inside the error region and keeps clean-file absence claims", async () => {
      const nativeWorkspace = workspace();
      const compiled = compileJson([
        tsClause("debugger", { kind: "debugger_statement" }),
        tsClause("no-class", { all: [{ kind: "program" }, { not: { has: { kind: "class_declaration", stopBy: "end" } } }] }),
      ]);
      nativeWorkspace.addCodeRule(compiled);
      try {
        const clean = { language: "typescript", path: "clean.ts", source: "debugger;\n" };
        const broken = { language: "typescript", path: "broken.ts", source: "debugger; // clean text keeps the error region under half the file\nfunction f( {\n  debugger;\n" };
        const actual = await applyWorkspace(nativeWorkspace, 1, [clean, broken]);
        expect(actual.violations.map(item => [item.clauseId, item.path, item.span?.startLine])).toEqual([
          ["debugger", "broken.ts", 0],
          ["debugger", "clean.ts", 0],
          ["no-class", "clean.ts", 0],
        ]);
        expect(identities(actual.failures)).toEqual([
          { clauseId: "debugger", code: "source-partial", path: "broken.ts" },
          { clauseId: "no-class", code: "source-partial", path: "broken.ts" },
        ]);
        expect(actual.failures[0]?.message).toMatch(/first at line 2, column \d+/);
        expect(actual.violations).toEqual(evaluate(compiled, snapshot([clean, broken] as never)).violations);

        const repaired = await applyWorkspace(nativeWorkspace, 2, [{ ...broken, source: "debugger;\nfunction f() {\n  debugger;\n}\n" }]);
        expect(repaired).toMatchObject({ status: "completed", failures: [] });
        expect(repaired.violations.filter(item => item.path === "broken.ts").map(item => item.clauseId).sort())
          .toEqual(["debugger", "debugger", "no-class"]);

        const damaged = await applyWorkspace(nativeWorkspace, 3, [{ ...broken, source: "debugger;\nfunction f() {\n  debugger;\n}\nconst x = (;\n" }]);
        expect(damaged.violations.filter(item => item.path === "broken.ts").map(item => [item.clauseId, item.span?.startLine]))
          .toEqual([["debugger", 0], ["debugger", 2]]);
        expect(identities(damaged.failures).map(item => item.code)).toEqual(["source-partial", "source-partial"]);
      } finally {
        nativeWorkspace.close();
      }
    });

    it("reads nothing from a file whose error regions cover half of it or more, sound clause or not", async () => {
      // The grammar recovers `debugger;`, then reads the prose as one ERROR to the end: 95% of the bytes.
      const prose = { language: "typescript", path: "notes.ts", source: `debugger;\n${"Install it: run the script, then open the page.\n".repeat(5)}` };
      const clean = { language: "typescript", path: "clean.ts", source: "debugger;\n" };
      const nativeWorkspace = workspace();
      const compiled = compileJson([
        tsClause("debugger", { kind: "debugger_statement" }),
        tsClause("unguarded", { all: [{ kind: "debugger_statement" }, { not: { follows: { kind: "import_statement", stopBy: "end" } } }] }),
        { id: "regex", forbid: { regex: { pattern: "debugger", source: "file.content" } }, violation: violation("REGEX") },
        // A cross-file query names the unread file and reports its detail too.
        {
          id: "gated",
          forbid: { select_when: {
            exists: { regex: { pattern: "debugger", source: "file.content" } },
            select: { ast: { language: "typescript", rule: { kind: "debugger_statement" } } },
          } },
          violation: violation("GATED"),
        },
      ]);
      nativeWorkspace.addCodeRule(compiled);
      try {
        const actual = await applyWorkspace(nativeWorkspace, 1, [clean, prose]);
        // The stateless path fails a whole query that cannot read some file, so compare it on that file alone.
        const full = evaluate(compiled, snapshot([prose] as never));
        // Regex clauses read the bytes and still report; no AST clause reports from the unread file.
        expect(actual.violations.map(item => [item.clauseId, item.path])).toEqual([
          ["debugger", "clean.ts"],
          ["unguarded", "clean.ts"],
          ["regex", "clean.ts"],
          ["regex", "notes.ts"],
        ]);
        expect(full.violations).toEqual(actual.violations.filter(item => item.path === "notes.ts"));
        const unread = "file did not parse: error regions cover 95% of it, first at line 2, column 1";
        const failures = (evaluation: { failures: ReadonlyArray<{ clauseId: string; code: string; path?: string | undefined; message: string }> }) => evaluation.failures
          .map(({ clauseId, code, path, message }) => ({ clauseId, code, path, message }))
          .sort((left, right) => left.clauseId.localeCompare(right.clauseId));
        expect(failures(actual)).toEqual([
          { clauseId: "debugger", code: "source-incomplete", path: "notes.ts", message: unread },
          { clauseId: "gated", code: "source-incomplete", path: "notes.ts", message: unread },
          { clauseId: "unguarded", code: "source-incomplete", path: "notes.ts", message: unread },
        ]);
        expect(failures(full)).toEqual(failures(actual));

        const repaired = await applyWorkspace(nativeWorkspace, 2, [{ ...prose, source: "debugger;\n" }]);
        expect(repaired).toMatchObject({ status: "completed", failures: [] });
      } finally {
        nativeWorkspace.close();
      }
    });

    it("continues capture enumeration after an eslint-suppressed binding", async () => {
      const nativeWorkspace = workspace();
      const compiled = compileJson([{
        violation: {
          ...violation("NO_CONSOLE"),
          anchor: "capture.X",
          each: "X",
          suppress: { eslint: { rules: ["no-console"] } },
        },
        forbid: {
          ast: {
            language: "typescript",
            rule: { kind: "program", has: { pattern: "$X" } },
          },
        },
        id: "capture-anchor",
      }]);
      nativeWorkspace.addCodeRule(compiled);
      try {
        const file = {
          language: "typescript" as const,
          path: "a.ts",
          source: "first; // eslint-disable-line no-console\nsecond;\n",
        };
        const actual = await applyWorkspace(nativeWorkspace, 1, [file]);
        expect(actual.violations.map(item => file.source.slice(item.span!.startByte, item.span!.endByte))).toEqual(["second;"]);
        expect(actual).toMatchObject({ status: "completed", failures: [] });
      } finally {
        nativeWorkspace.close();
      }
    });
    it("honours oxlint's disable directives as oxlint 1.88.0 reads them", async () => {
      const nativeWorkspace = workspace();
      const compiled = compileJson([{
        violation: {
          ...violation("RULES_OF_HOOKS"),
          suppress: { eslint: { rules: ["react-hooks/rules-of-hooks"] }, oxlint: { rules: ["rules-of-hooks"] } },
        },
        forbid: {
          ast: {
            language: "typescript",
            rule: { kind: "call_expression", has: { field: "function", kind: "identifier", regex: "^useEffect$" }, inside: { kind: "if_statement", stopBy: "end" } },
          },
        },
        id: "conditional-hook",
      }]);
      nativeWorkspace.addCodeRule(compiled);
      try {
        const source = [
          "import { useEffect } from \"react\";",
          "// oxlint-disable react-hooks/rules-of-hooks",
          "function Avatar({ x }) { if (x) useEffect(() => {}); return null; }",
          "// oxlint-enable react-hooks/rules-of-hooks",
          "function Badge({ x }) { if (x) useEffect(() => {}); return null; } // oxlint-disable-line react-hooks/rules-of-hooks",
          "// oxlint-disable-next-line react-hooks/rules-of-hooks",
          "function Chip({ x }) { if (x) useEffect(() => {}); return null; }",
          "// biome-ignore lint/correctness/useHookAtTopLevel: intentional",
          "function Dot({ x }) { if (x) useEffect(() => {}); return null; }",
          "/* eslint-disable react-hooks/rules-of-hooks */",
          "function Eslinted({ x }) { if (x) useEffect(() => {}); return null; }",
          "/* eslint-enable react-hooks/rules-of-hooks */",
          "function Plain({ x }) { if (x) useEffect(() => {}); return null; }",
          "",
        ].join("\n");
        const actual = await applyWorkspace(nativeWorkspace, 1, [{ language: "tsx" as const, path: "src/avatar.tsx", source }]);
        expect(actual).toMatchObject({ status: "completed", failures: [] });
        // 1-based lines 9 (Dot, under biome-ignore) and 13 (Plain), as `oxlint` 1.88.0 reports them.
        expect(actual.violations.map(item => item.span!.startLine + 1)).toEqual([9, 13]);
      } finally {
        nativeWorkspace.close();
      }
    });

  });

  describe("utils, matches, and constraints", () => {
    const tsClause = (id: string, ast: Record<string, unknown>) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "typescript", ...ast } },
      id,
    });
    const tailAwait = tsClause("tail-await", {
      rule: { kind: "return_statement", has: { matches: "tail" } },
      utils: {
        tail: { any: [
          { kind: "await_expression" },
          { kind: "parenthesized_expression", has: { matches: "tail" } },
          { kind: "ternary_expression", has: { matches: "tail" } },
        ] },
      },
    });
    const conditionalHook = tsClause("conditional-hook", {
      rule: { pattern: "$HOOK($$$ARGS)", inside: { stopBy: "end", matches: "conditional" } },
      utils: { conditional: { any: [{ kind: "if_statement" }, { kind: "ternary_expression" }, { kind: "for_statement" }] } },
      constraints: { HOOK: { regex: "^use[A-Z]" } },
    });
    const jsxOrCall = tsClause("jsx-or-call", {
      rule: { any: [{ matches: "jsx" }, { pattern: "g($A)" }] },
      utils: { jsx: { kind: "jsx_self_closing_element" } },
    });
    const texts = (violations: Array<{ clauseId: string; path?: string | undefined; span?: { startByte: number; endByte: number } | undefined }>, files: Array<{ path: string; source: string }>) =>
      violations.map(item => {
        const source = files.find(file => file.path === item.path)!.source;
        return `${item.clauseId}:${item.path}:${item.span === undefined ? "" : source.slice(item.span.startByte, item.span.endByte)}`;
      }).sort();

    it("matches the stateless evaluation on the indexed workspace path across edits", async () => {
      const compiled = compileJson([tailAwait, conditionalHook, jsxOrCall]);
      const revisions = [
        [
          { language: "typescript", path: "a.ts", source: "async function f() { return ((await x)); }\nfunction C() { if (x) { useEffect(f); } }\ng(1);\n" },
          { language: "tsx", path: "v.tsx", source: "const v = <br />;\nconst w = ok ? useMemo(h) : null;\n" },
        ],
        [
          { language: "typescript", path: "a.ts", source: "async function f() { return c ? (((await x))) : y; }\nfunction C() { useEffect(f); }\n" },
          { language: "tsx", path: "v.tsx", source: "const v = <br />;\nconst w = ok ? useMemo(h) : null;\n" },
        ],
      ];
      const expected = [
        [
          "conditional-hook:a.ts:useEffect(f)",
          "conditional-hook:v.tsx:useMemo(h)",
          "jsx-or-call:a.ts:g(1)",
          "jsx-or-call:v.tsx:<br />",
          "tail-await:a.ts:return ((await x));",
        ],
        [
          "conditional-hook:v.tsx:useMemo(h)",
          "jsx-or-call:v.tsx:<br />",
          "tail-await:a.ts:return c ? (((await x))) : y;",
        ],
      ];
      const nativeWorkspace = workspace();
      try {
        nativeWorkspace.addCodeRule(compiled);
        for (const [index, files] of revisions.entries()) {
          const full = evaluate(compiled, snapshot(files as never));
          const actual = await applyWorkspace(nativeWorkspace, index + 1, files);
          expect(full).toMatchObject({ status: "completed", failures: [] });
          expect({ status: actual.status, violations: actual.violations, failures: actual.failures })
            .toEqual({ status: full.status, violations: full.violations, failures: [] });
          expect(texts(actual.violations, files)).toEqual(expected[index]);
        }
      } finally {
        nativeWorkspace.close();
      }
    });

    it("fails only the clause whose recursion exceeds its depth or work limit", async () => {
      const nested = tsClause("nested", {
        rule: { kind: "expression_statement", has: { stopBy: "end", matches: "nested" } },
        utils: { nested: { kind: "array", any: [{ not: { has: { kind: "array" } } }, { has: { matches: "nested" } }] } },
      });
      const slow = tsClause("slow", {
        rule: { kind: "program", matches: "slow" },
        // Every block re-walks every nested block before its last test fails: exponential in depth. That test is
        // a pattern, which the anchor index cannot rule out, so both paths do the work.
        utils: { slow: { has: { stopBy: "end", kind: "statement_block", all: [{ matches: "slow" }, { pattern: "{ $A; $A; }" }] } } },
      });
      const plain = tsClause("plain", { rule: { kind: "expression_statement" } });
      const compiled = nativeRuntime().compile(
        codeRule([nested, slow, plain]),
        "json",
        capabilities({ limits: { ...defaultLimits, maxVisitedNodes: 50_000 } }),
      );
      const files = [{ language: "typescript", path: "deep.ts", source: `x = ${"[".repeat(2_000)}${"]".repeat(2_000)};\n{{{{{{{{{{{{{{{{{{{{{{{{{{{{{{ y; }}}}}}}}}}}}}}}}}}}}}}}}}}}}}}\n` }];
      const expectedFailures = [
        { clauseId: "nested", code: "budget-exceeded", message: expect.stringContaining("depth limit"), path: "deep.ts" },
        { clauseId: "slow", code: "budget-exceeded", message: expect.stringContaining("work budget"), path: "deep.ts" },
      ];
      const full = evaluate(compiled, snapshot(files as never));
      expect(full.failures).toEqual(expectedFailures);
      expect(full.violations.map(item => item.clauseId)).toEqual(["plain", "plain"]);
      const nativeWorkspace = workspace();
      try {
        nativeWorkspace.addCodeRule(compiled);
        const actual = await applyWorkspace(nativeWorkspace, 1, files);
        expect(actual.failures).toEqual(expectedFailures.map(item => ({ ...item, ruleId: "runtime-contract" })));
        expect(actual.violations).toEqual(full.violations);
      } finally {
        nativeWorkspace.close();
      }
    });

    it("reports sound witnesses outside syntax errors and skips rules a reference makes unsound", async () => {
      const unsound = tsClause("top-level-call", {
        rule: { pattern: "$F()", matches: "top-level" },
        utils: { "top-level": { not: { inside: { stopBy: "end", kind: "function_declaration" } } } },
      });
      const compiled = compileJson([tailAwait, conditionalHook, unsound]);
      const files = [{
        language: "typescript",
        path: "broken.ts",
        source: "async function f() { return (await x); }\nif (a) { useRef(); }\nconst m = importOriginal(1 load(\"./api\"));\n",
      }];
      const full = evaluate(compiled, snapshot(files as never));
      const nativeWorkspace = workspace();
      try {
        nativeWorkspace.addCodeRule(compiled);
        const actual = await applyWorkspace(nativeWorkspace, 1, files);
        expect(actual.violations).toEqual(full.violations);
        expect(texts(actual.violations, files)).toEqual([
          "conditional-hook:broken.ts:useRef()",
          "tail-await:broken.ts:return (await x);",
        ]);
        const codes = (failures: Array<{ clauseId: string; code: string }>) =>
          failures.map(({ clauseId, code }) => `${clauseId}:${code}`).sort();
        expect(codes(actual.failures)).toEqual([
          "conditional-hook:source-partial",
          "tail-await:source-partial",
          "top-level-call:source-incomplete",
        ]);
        expect(codes(full.failures)).toEqual(codes(actual.failures));
      } finally {
        nativeWorkspace.close();
      }
    });
  });

  describe("capture comparison", () => {
    it("filters matches by comparing two captures on the indexed workspace path across edits", async () => {
      const unsortedKey = {
        violation: { ...violation("UNSORTED"), anchor: "capture.KEY", message: "$KEY should come before $PREV" },
        forbid: {
          ast: {
            language: "typescript",
            rule: {
              kind: "pair",
              has: { kind: "property_identifier", pattern: "$KEY" },
              follows: { regex: "^,$", follows: { kind: "pair", has: { kind: "property_identifier", pattern: "$PREV" } } },
              compare: { left: "PREV", op: "gt", right: "KEY", collation: "fold-lower" },
            },
          },
        },
        id: "unsorted-key",
      };
      const compiled = compileJson([unsortedKey]);
      const sources = ["const o = { b: 1, a: 2, C: 3 };\n", "const o = { a: 1, b: 2, C: 3, B: 4 };\n"];
      const expected = [["a should come before b"], ["B should come before C"]];
      const nativeWorkspace = workspace();
      try {
        nativeWorkspace.addCodeRule(compiled);
        for (const [index, source] of sources.entries()) {
          const files = [{ language: "typescript", path: "a.ts", source }];
          const full = evaluate(compiled, snapshot(files as never));
          const actual = await applyWorkspace(nativeWorkspace, index + 1, files);
          expect(full).toMatchObject({ status: "completed", failures: [] });
          expect(actual.violations).toEqual(full.violations);
          expect(actual.violations.map(item => item.message)).toEqual(expected[index]);
        }
      } finally {
        nativeWorkspace.close();
      }
    });
  });

  describe("count within a node", () => {
    it("matches the stateless evaluation on the indexed workspace path across edits", async () => {
      const decisions = { any: [{ kind: "if_statement" }, { kind: "ternary_expression" }, { kind: "for_statement" }] };
      const functionScopes = { any: [{ kind: "function_declaration" }, { kind: "arrow_function" }, { kind: "method_definition" }] };
      const complex = {
        ...astForbid({ kind: "function_declaration", count: { has: { ...decisions, stopBy: functionScopes }, min: 2 } }, "typescript"),
        id: "complex",
      };
      const shallow = { ...astForbid({ kind: "if_statement", count: { inside: { kind: "if_statement", stopBy: "end" }, max: 0 } }, "typescript"), id: "shallow" };
      const compiled = compileJson([complex, shallow]);
      const sources = [
        "function f(a) { if (a) { if (a) {} } }\nfunction g(a) { const k = () => a ? 1 : a ? 2 : 3; }\n",
        "function f(a) { if (a) {} }\nfunction g(a) { if (a) {} for (;;) {} }\n",
      ];
      const expected = [["complex:function f", "shallow:if (a) { i"], ["complex:function g", "shallow:if (a) {}", "shallow:if (a) {}"]];
      const nativeWorkspace = workspace();
      try {
        nativeWorkspace.addCodeRule(compiled);
        for (const [index, source] of sources.entries()) {
          const files = [{ language: "typescript", path: "a.ts", source }];
          const full = evaluate(compiled, snapshot(files as never));
          const actual = await applyWorkspace(nativeWorkspace, index + 1, files);
          expect(full).toMatchObject({ status: "completed", failures: [] });
          expect(actual.violations).toEqual(full.violations);
          const texts = actual.violations.map(item => `${item.clauseId}:${source.slice(item.span!.startByte, item.span!.endByte).slice(0, 10)}`).sort();
          expect(texts).toEqual(expected[index]);
        }
      } finally {
        nativeWorkspace.close();
      }
    });
  });

  describe("stopBy rules", () => {
    const tsClause = (id: string, rule: Record<string, unknown>) => ({ ...astForbid(rule, "typescript"), id });
    const rebinding = { any: [{ kind: "function_declaration" }, { kind: "function_expression" }] };
    const invalidThis = tsClause("invalid-this", { kind: "this", not: { inside: { kind: "class_body", stopBy: rebinding } } });
    const ownReturn = tsClause("own-return", { kind: "function_declaration", has: { kind: "return_statement", stopBy: { kind: "arrow_function" } } });
    const beforeBarrier = tsClause("before-barrier", { kind: "expression_statement", precedes: { pattern: "done();", stopBy: { pattern: "barrier();" } } });
    const texts = (violations: Array<{ clauseId: string; span?: { startByte: number; endByte: number } | undefined }>, source: string) =>
      violations.map(item => `${item.clauseId}:${item.span === undefined ? "" : source.slice(item.span.startByte, item.span.endByte).split("\n")[0]}`).sort();

    it("matches the stateless evaluation on the indexed workspace path across edits", async () => {
      const compiled = compileJson([invalidThis, ownReturn, beforeBarrier]);
      const sources = [
        "class A { m() { const f = () => this.a; function g() { return this; } } }\nfunction h() { const k = () => { return 1; }; }\na();\ndone();\n",
        "class A { m() { const f = () => this.a; } }\nfunction h() { return 1; }\na();\nbarrier();\ndone();\n",
      ];
      const expected = [
        ["before-barrier:a();", "invalid-this:this", "own-return:function g() { return this; }"],
        ["before-barrier:barrier();", "own-return:function h() { return 1; }"],
      ];
      const nativeWorkspace = workspace();
      try {
        nativeWorkspace.addCodeRule(compiled);
        for (const [index, source] of sources.entries()) {
          const files = [{ language: "typescript", path: "a.ts", source }];
          const full = evaluate(compiled, snapshot(files as never));
          const actual = await applyWorkspace(nativeWorkspace, index + 1, files);
          expect(full).toMatchObject({ status: "completed", failures: [] });
          expect(actual.violations).toEqual(full.violations);
          expect(texts(actual.violations, source)).toEqual(expected[index]);
        }
      } finally {
        nativeWorkspace.close();
      }
    });

    it("skips relations whose stop rule walks outside the witness in files with syntax errors", async () => {
      const inClass = tsClause("in-class", { kind: "this", inside: { kind: "class_body", stopBy: rebinding } });
      const compiled = compileJson([inClass, ownReturn]);
      const files = [{ language: "typescript", path: "broken.ts", source: "function h() { return 1; }\nclass A { m() { this.a; } }\nconst x = (;\n" }];
      const full = evaluate(compiled, snapshot(files as never));
      const nativeWorkspace = workspace();
      try {
        nativeWorkspace.addCodeRule(compiled);
        const actual = await applyWorkspace(nativeWorkspace, 1, files);
        expect(actual.violations).toEqual(full.violations);
        expect(texts(actual.violations, files[0]!.source)).toEqual(["own-return:function h() { return 1; }"]);
        const codes = (failures: Array<{ clauseId: string; code: string }>) => failures.map(({ clauseId, code }) => `${clauseId}:${code}`).sort();
        expect(codes(actual.failures)).toEqual(["in-class:source-incomplete", "own-return:source-partial"]);
        expect(codes(full.failures)).toEqual(codes(actual.failures));
      } finally {
        nativeWorkspace.close();
      }
    });
  });

  it("keeps zero-file code rule failures while eligible AST code rules still evaluate", async () => {
    const nativeWorkspace = workspace();
    const clauses = [astForbid({ pattern: "print($ARG)" })];
    nativeWorkspace.addCodeRule(nativeRuntime().compile(
      codeRule(clauses, { id: "disabled-input" }),
      "json",
      capabilities({ limits: { ...defaultLimits, maxFiles: 0 } }),
    ));
    nativeWorkspace.addCodeRule(nativeRuntime().compile(
      codeRule(clauses, { id: "eligible-input" }), "json", capabilities(),
    ));
    try {
      const malformed = await applyWorkspace(nativeWorkspace, 1, [{
        language: "python", path: "edit.py", source: "def broken(:\n",
      }]);
      expect(new Map(malformed.failures.map(item => [item.ruleId, item.code]))).toEqual(new Map([
        ["disabled-input", "budget-exceeded"],
        ["eligible-input", "source-partial"],
      ]));

      const valid = await applyWorkspace(nativeWorkspace, 2, [{
        language: "python", path: "edit.py", source: "print('x')\n",
      }]);
      expect(valid.failures).toEqual([
        expect.objectContaining({ ruleId: "disabled-input", code: "budget-exceeded", path: "edit.py" }),
      ]);
      expect(valid.violations).toEqual([
        expect.objectContaining({ ruleId: "eligible-input", code: "AST_MATCH", path: "edit.py" }),
      ]);
    } finally {
      nativeWorkspace.close();
    }
  });
});

describe.skipIf(codeRuleDslRuntime === undefined)("trust sweep (M6)", () => {
  it("rejects a document that repeats a key instead of keeping the last one", () => {
    const rule = `{"kind": "identifier", "not": {"regex": "^a$"}, "not": {"regex": "^b$"}}`;
    const source = `{"format": "rerail.policy/v1", "id": "dup", "revision": 1, "mode": "steer", "clauses": [{"id": "c",
      "forbid": {"ast": {"language": "python", "rule": ${rule}}}, "violation": ${JSON.stringify(violation("X"))}}]}`;
    expect(() => nativeRuntime().compile(source, "json", capabilities())).toThrow(/^duplicate key `not` at \/clauses\/0\/forbid\/ast\/rule at line 2/);
    expect(() => nativeRuntime().compile("format: rerail.policy/v1\nid: a\nid: b\nrevision: 1\nclauses: []\n", "yaml", capabilities())).toThrow(
      /^invalid YAML: duplicate key `id` at \//,
    );
  });

  it("keeps kindless rules, which report anonymous tokens, on the indexed workspace path", async () => {
    const operator = { ...astForbid({ regex: "^==$", inside: { kind: "binary_expression" } }, "typescript"), id: "loose-equality" };
    const compiled = compileJson([operator]);
    const files = [{ language: "typescript", path: "a.ts", source: "if (a == b) {}\n" }];
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const actual = await applyWorkspace(nativeWorkspace, 1, files);
      expect(actual.violations).toEqual(evaluate(compiled, snapshot(files as never)).violations);
      expect(actual.violations.map(item => files[0]!.source.slice(item.span!.startByte, item.span!.endByte))).toEqual(["=="]);
    } finally {
      nativeWorkspace.close();
    }
  });

  it.each(["expression", "statement", "declaration", "primary_expression", "pattern"])("rejects the abstract kind %s, which no node carries", kind => {
    expect(() => compileJson([astForbid({ kind }, "typescript")])).toThrow(
      `/clauses/0/forbid/ast/rule/kind: \`${kind}\` is an abstract kind (a grammar supertype) that no node carries; list its concrete kinds under \`any\` (typescript, tsx, javascript grammars)`,
    );
    if (["expression", "primary_expression", "pattern"].includes(kind)) {
      expect(() => compileJson([astForbid({ kind: "call", has: { kind } })])).toThrow(/rule\/has\/kind: `\w+` is an abstract kind/);
    }
  });

  it("rejects a Python pattern whose $-name is not a metavariable, and keeps TypeScript's literal $-identifiers", async () => {
    expect(() => compileJson([astForbid({ pattern: "time.$At" })])).toThrow(
      "/clauses/0/forbid/ast/rule/pattern: `$At` is not a metavariable (names use A-Z, 0-9 and _) and Python source has no `$`, so the pattern never matches (python grammar)",
    );
    const compiled = compileJson([astForbid({ pattern: "time.$At" }, "typescript")]);
    const files = [{ language: "typescript", path: "a.ts", source: "time.At;\ntime.$At;\n" }];
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const actual = await applyWorkspace(nativeWorkspace, 1, files);
      expect(actual.violations).toEqual(evaluate(compiled, snapshot(files as never)).violations);
      expect(actual.violations.map(item => item.span?.startLine)).toEqual([1]);
    } finally {
      nativeWorkspace.close();
    }
  });

  it("lets a comparison read names its own negation or count binds, on the indexed workspace path", async () => {
    const key = (name: string) => ({ kind: "property_identifier", pattern: `$${name}` });
    const later = { stopBy: "end", kind: "pair", has: key("J"), compare: { left: "J", op: "eq", right: "K" } };
    const lastOfEach = { ...astForbid({ kind: "pair", all: [{ has: key("K") }, { not: { precedes: later } }] }, "typescript"), id: "last-of-each" };
    const repeated = { ...astForbid({ kind: "pair", has: key("K"), count: { follows: later, min: 1 } }, "typescript"), id: "repeated" };
    const compiled = compileJson([lastOfEach, repeated]);
    const source = "const o = { a: 1, b: 2, a: 3, c: 4 };\n";
    const files = [{ language: "typescript", path: "a.ts", source }];
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const actual = await applyWorkspace(nativeWorkspace, 1, files);
      expect(actual.violations).toEqual(evaluate(compiled, snapshot(files as never)).violations);
      const texts = actual.violations.map(item => `${item.clauseId}:${source.slice(item.span!.startByte, item.span!.endByte)}`).sort();
      expect(texts).toEqual(["last-of-each:a: 3", "last-of-each:b: 2", "last-of-each:c: 4", "repeated:a: 3"]);
    } finally {
      nativeWorkspace.close();
    }
    expect(() => compileJson([astForbid({ kind: "pair", has: key("K"), inside: { kind: "program", stopBy: { kind: "object", compare: { left: "K", op: "eq", right: "K" } } } }, "typescript")])).toThrow(
      /inside\/stopBy\/compare\/left: `\$K` is never bound where this comparison runs/,
    );
  });

  it("skips, in files with syntax errors, a rule that reads a capture an outward relation under any may leave unbound", async () => {
    const optionalPrevious = [{ follows: { kind: "expression_statement", pattern: "$PREV" } }, { regex: "." }];
    const differs = {
      ...astForbid({ kind: "expression_statement", pattern: "$CUR", any: optionalPrevious, not: { compare: { left: "PREV", op: "eq", right: "CUR" } } }, "typescript"),
      id: "differs",
    };
    const inside = { ...astForbid({ kind: "call_expression", has: { kind: "arguments", pattern: "$ARGS" } }, "typescript"), id: "inside" };
    const compiled = compileJson([differs, inside]);
    const files = [{ language: "typescript", path: "a.ts", source: "f();\nf(;\nf();\n" }];
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const actual = await applyWorkspace(nativeWorkspace, 1, files);
      const full = evaluate(compiled, snapshot(files as never));
      expect(actual.violations).toEqual(full.violations);
      expect(actual.violations.map(item => item.clauseId)).not.toContain("differs");
      const codes = (failures: Array<{ clauseId: string; code: string }>) => failures.map(({ clauseId, code }) => `${clauseId}:${code}`).sort();
      expect(codes(actual.failures)).toEqual(["differs:source-incomplete", "inside:source-partial"]);
      expect(codes(full.failures)).toEqual(codes(actual.failures));
    } finally {
      nativeWorkspace.close();
    }
  });

  it("renders escaped and digit-led dollar names in messages literally, on the indexed workspace path", async () => {
    const clause = { ...astForbid({ pattern: "print($ARG)" }), violation: { ...violation("PRINT"), message: "print($ARG): set \\$HOME, costs $100" } };
    const compiled = compileJson([clause]);
    const files = [{ language: "python", path: "a.py", source: "print(x)\n" }];
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const actual = await applyWorkspace(nativeWorkspace, 1, files);
      expect(actual.violations).toEqual(evaluate(compiled, snapshot(files as never)).violations);
      expect(actual.violations.map(item => item.message)).toEqual(["print(x): set $HOME, costs $100"]);
    } finally {
      nativeWorkspace.close();
    }
    expect(() => compileJson([{ ...clause, violation: { ...violation("PRINT"), message: "set $HOME" } }])).toThrow(
      "/clauses/0/violation/message: `$HOME` names no capture; the query can bind: ARG; write `\\$HOME` for literal text",
    );
  });

  it("carries a literal fix on regex witnesses without named groups, on the indexed workspace path", async () => {
    const clause = (id: string, pattern: string) => ({ id, violation: { ...violation("DEBUGGER"), fix: "" }, forbid: { regex: { pattern, source: "file.content" } } });
    const compiled = compileJson([clause("plain", "debugger;"), clause("grouped", "(?<D>debugger;)")]);
    const files = [{ language: "typescript", path: "a.ts", source: "debugger;\n" }];
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const actual = await applyWorkspace(nativeWorkspace, 1, files);
      expect(actual.violations).toEqual(evaluate(compiled, snapshot(files as never)).violations);
      expect(actual.violations.map(item => [item.clauseId, item.fix?.text, item.fix?.span.endByte])).toEqual([["grouped", "", 9], ["plain", "", 9]]);
    } finally {
      nativeWorkspace.close();
    }
  });

  it("reports source-partial for a global clause only where an AST leaf read the broken file, like the stateless path", async () => {
    const regexGate = {
      id: "cross-file",
      violation: violation("CROSS_FILE"),
      forbid: { select_when: { exists: { regex: { pattern: "danger\\(", source: "file.content" } }, select: { regex: { pattern: "print", source: "file.content" } } } },
    };
    const astGate = {
      id: "ast-gate",
      violation: violation("AST_GATE"),
      forbid: { select_when: { exists: { regex: { pattern: "danger\\(", source: "file.content" } }, select: { ast: { language: "python", rule: { pattern: "print($A)" } } } } },
    };
    const compiled = compileJson([astForbid({ pattern: "print($ARG)" }), regexGate, astGate]);
    const files = [
      { language: "python", path: "broken.py", source: "def broken(:\nprint('x')\n" },
      { language: "python", path: "danger.py", source: "danger()\n" },
    ];
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const actual = await applyWorkspace(nativeWorkspace, 1, files);
      const full = evaluate(compiled, snapshot(files as never));
      const codes = (failures: Array<{ clauseId: string; code: string; path?: string | undefined }>) =>
        failures.map(({ clauseId, code, path }) => `${clauseId}:${code}:${path ?? ""}`).sort();
      expect(codes(actual.failures)).toEqual(codes(full.failures));
      expect(codes(actual.failures)).not.toContain("cross-file:source-partial:broken.py");
      expect(actual.violations.map(item => item.clauseId).sort()).toEqual(full.violations.map(item => item.clauseId).sort());
    } finally {
      nativeWorkspace.close();
    }
  });

  it("keeps sibling relations within the time budget in a long file with one MISSING token", async () => {
    const statements = Array.from({ length: 3000 }, (_, index) => `f${index}();\n`).join("");
    const files = [{ language: "typescript", path: "long.ts", source: `${statements}import x from 'x';\nfunction g() {\n` }];
    const compiled = compileJson([{ ...astForbid({ kind: "expression_statement", follows: { kind: "import_statement", stopBy: "end" } }, "typescript"), id: "after-import" }]);
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const actual = await applyWorkspace(nativeWorkspace, 1, files);
      expect(actual.failures.map(item => item.code)).toEqual(["source-partial"]);
      expect(actual.violations).toEqual(evaluate(compiled, snapshot(files as never)).violations);
    } finally {
      nativeWorkspace.close();
    }
  });
});

describe.skipIf(codeRuleDslRuntime === undefined)("per-node values (M9a)", () => {
  async function workspaceAndFull(clauses: unknown[], files: Array<{ language: "python" | "typescript" | "tsx"; path: string; source: string }>) {
    const compiled = compileJson(clauses);
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      return { actual: await applyWorkspace(nativeWorkspace, 1, files), full: evaluate(compiled, snapshot(files)) };
    } finally {
      nativeWorkspace.close();
    }
  }
  const texts = (violations: Array<{ clauseId: string; span?: { startByte: number; endByte: number; startLine: number } | undefined }>, source: string) =>
    violations.map(item => `${item.clauseId}:${item.span!.startLine + 1}:${source.slice(item.span!.startByte, item.span!.endByte)}`).sort();

  it("reads decoded and folded values, on the indexed workspace path", async () => {
    const source = "s.bind(('0.0.0.0', 80))\ns.bind(('0.0.' '0.0', 80))\nos.chmod(p, 0o644 | 0o020)\nos.chmod(p, mode)\nx = 24 * 3600 + 1\nprint(list())\ndef f(list):\n    return list()\n";
    const clause = (id: string, rule: unknown, constraints?: unknown) => ({ violation: violation("AST_MATCH"), forbid: { ast: { language: "python", rule, constraints } }, id });
    const { actual, full } = await workspaceAndFull(
      [
        clause("b104", { pattern: "$S.bind(($H, $P))" }, { H: { value: { eq: "0.0.0.0" } } }),
        clause("b103", { pattern: "os.chmod($P, $M)" }, { M: { value: { bitsAny: 18 } } }),
        clause("long", { kind: "integer", inside: { kind: "binary_operator" }, not: { value: { lt: 100 } } }),
        clause("falsy", { kind: "call", value: { truthy: false } }),
        // `not` over an unknown value is unknown: no call here is known to be truthy.
        clause("unknown", { kind: "call", not: { value: { truthy: false } } }),
      ],
      [{ language: "python" as const, path: "a.py", source }],
    );
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual([
      "b103:3:os.chmod(p, 0o644 | 0o020)",
      "b104:1:s.bind(('0.0.0.0', 80))",
      "b104:2:s.bind(('0.0.' '0.0', 80))",
      "falsy:6:list()",
      "long:3:0o644",
      "long:5:3600",
    ]);
  });

  it("compares values, property names and tokens, on the indexed workspace path", async () => {
    const source = "const o = { a: 1, 'a': 2, 0x1: 3, [b]: 4, 1: 5 };\nf(1 + 2, 1+2);\nf(10, 9);\nf('10', '9');\nf(x, 1);\n";
    const clause = (id: string, rule: unknown) => ({ violation: violation("AST_MATCH"), forbid: { ast: { language: "typescript", rule } }, id });
    const { actual, full } = await workspaceAndFull(
      [
        clause("dupe", { kind: "pair", pattern: "$P", follows: { stopBy: "end", kind: "pair", pattern: "$Q", compare: { left: "Q", op: "eq", right: "P", text: "key" } } }),
        clause("tokens", { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "tokens" } }),
        clause("greater", { pattern: "f($A, $B)", compare: { left: "A", op: "gt", right: "B", text: "value" } }),
        clause("unknown", { pattern: "f($A, $B)", not: { compare: { left: "A", op: "gt", right: "B", text: "value" } } }),
      ],
      [{ language: "typescript" as const, path: "a.ts", source }],
    );
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    // `'a'` repeats `a`. `[b]` has no static name, so it equals no key and the search back from `1` goes on to `0x1`.
    expect(texts(actual.violations, source)).toEqual([
      "dupe:1:'a': 2",
      "dupe:1:1: 5",
      "greater:3:f(10, 9)",
      "tokens:2:f(1 + 2, 1+2)",
      "unknown:2:f(1 + 2, 1+2)",
      "unknown:4:f('10', '9')",
    ]);
  });

  it("compares Python structurally under Ruff's three equalities, on the indexed workspace path", async () => {
    const source = "f(1, 0x1)\nf(1, 1.0)\nf('a' 'b', 'ab' '')\nf(-1, -(1))\nf('\\N{NOT A NAME}', 'a')\n";
    const clause = (id: string, text: string, op = "eq") => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op, right: "B", text } } } },
      id,
    });
    const { actual, full } = await workspaceAndFull(
      [clause("plain", "structure"), clause("hash", "structure-hash"), clause("parts", "structure-parts"), clause("differ", "structure", "ne")],
      [{ language: "python" as const, path: "a.py", source }],
    );
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    // An unknown `\N{...}` name leaves the fifth call neither equal nor unequal.
    expect(texts(actual.violations, source)).toEqual([
      "differ:2:f(1, 1.0)",
      "hash:1:f(1, 0x1)",
      "hash:2:f(1, 1.0)",
      "hash:3:f('a' 'b', 'ab' '')",
      "hash:4:f(-1, -(1))",
      "parts:1:f(1, 0x1)",
      "parts:4:f(-1, -(1))",
      "plain:1:f(1, 0x1)",
      "plain:3:f('a' 'b', 'ab' '')",
      "plain:4:f(-1, -(1))",
    ]);
  });

  it("keeps a value rule sound in a file with syntax errors, on the indexed workspace path", async () => {
    const source = "undefined;\n0;\nfunction (;\n";
    const clause = { violation: violation("AST_MATCH"), forbid: { ast: { language: "typescript", rule: { kind: "expression_statement", has: { value: { truthy: false } } } } }, id: "falsy" };
    const { actual, full } = await workspaceAndFull([clause], [{ language: "typescript" as const, path: "a.ts", source }]);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures.map(item => item.code)).toEqual(["source-partial"]);
    // `undefined` needs the file's bindings, which an error region may hide; `0` does not.
    expect(texts(actual.violations, source)).toEqual(["falsy:2:0;"]);
  });
});

describe.skipIf(codeRuleDslRuntime === undefined)("binding values (M9b)", () => {
  async function workspaceAndFull(clauses: unknown[], files: Array<{ language: "python" | "typescript" | "tsx"; path: string; source: string }>) {
    const compiled = compileJson(clauses);
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      return { actual: await applyWorkspace(nativeWorkspace, 1, files), full: evaluate(compiled, snapshot(files)) };
    } finally {
      nativeWorkspace.close();
    }
  }
  const texts = (violations: Array<{ clauseId: string; span?: { startByte: number; endByte: number; startLine: number } | undefined }>, source: string) =>
    violations.map(item => `${item.clauseId}:${item.span!.startLine + 1}:${source.slice(item.span!.startByte, item.span!.endByte)}`).sort();

  it("matches a binding's value, annotation and site, and follows values, on the indexed workspace path", async () => {
    const source = "ages = {'Tom': 23}\nages.get('Cat', None)\nages = ['Tom']\nages.get('Cat', None)\ndef f(d: dict, **kw):\n    d.get(1, None)\n    kw.get(1, None)\nmode = 0o777\nos.chmod(p, mode)\n";
    const clause = (id: string, rule: unknown, constraints?: unknown) => ({ violation: violation("AST_MATCH"), forbid: { ast: { language: "python", rule, constraints } }, id });
    const dict = {
      any: [
        { resolves: { binding: { value: { kind: "dictionary" } } } },
        { resolves: { binding: { annotation: { regex: "^dict" } } } },
        { resolves: { binding: { site: { kind: "dictionary_splat_pattern" } } } },
      ],
    };
    const { actual, full } = await workspaceAndFull(
      [clause("sim910", { pattern: "$D.get($K, None)" }, { D: dict }), clause("b103", { pattern: "os.chmod($P, $M)" }, { M: { value: { bitsAny: 2, follow: true } } })],
      [{ language: "python" as const, path: "a.py", source }],
    );
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["b103:9:os.chmod(p, mode)", "sim910:2:ages.get('Cat', None)", "sim910:6:d.get(1, None)", "sim910:7:kw.get(1, None)"]);
  });

  it("follows aliases through a recursive utility, on the indexed workspace path", async () => {
    const source = "var foo = JSON; function f(foo) { foo(); }\nvar a = JSON, b = a; b();\nlet c = JSON; c = Math; c();\n";
    const clause = {
      violation: violation("AST_MATCH"),
      forbid: {
        ast: {
          language: "typescript",
          rule: { pattern: "$F()" },
          constraints: { F: { matches: "json" } },
          utils: { json: { any: [{ kind: "identifier", resolves: { global: ["JSON"] } }, { kind: "identifier", resolves: { binding: { value: { matches: "json" } } } }] } },
        },
      },
      id: "no-obj-calls",
    };
    const { actual, full } = await workspaceAndFull([clause], [{ language: "typescript" as const, path: "a.ts", source }]);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["no-obj-calls:2:b()"]);
  });

  it("binds call arguments to signature slots, on the indexed workspace path", async () => {
    const source = "requests.get(url)\nrequests.get(url, timeout=5)\nrequests.get(url, **kw)\nqs.extra(None, 'raw')\nqs.extra(where=['a'])\nqs.extra(*args)\n";
    const clause = (id: string, rule: unknown, constraints?: unknown) => ({ violation: violation("AST_MATCH"), forbid: { ast: { language: "python", rule, constraints } }, id });
    const { actual, full } = await workspaceAndFull(
      [
        clause("b113", { pattern: "requests.get($$$)", arguments: { signature: ["url", "params", "*", "timeout"], absent: ["timeout"] } }),
        clause("b610", { pattern: "$Q.extra($$$)", arguments: { signature: ["select", "where", "params", "tables", "order_by", "select_params"], bind: { W: "where" } } }, { W: { not: { kind: "list" } } }),
      ],
      [{ language: "python" as const, path: "a.py", source }],
    );
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["b113:1:requests.get(url)", "b610:4:qs.extra(None, 'raw')"]);
  });

  it("compares captures by node identity, on the indexed workspace path", async () => {
    const source = "for x in y:\n    i += 1\nfor x in y:\n    i += 1\n";
    const clause = (id: string, text: string) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", rule: { kind: "for_statement", pattern: "$A", follows: { stopBy: "end", kind: "for_statement", pattern: "$B", compare: { left: "A", op: "ne", right: "B", text } } } } },
      id,
    });
    const { actual, full } = await workspaceAndFull([clause("node", "node"), clause("source", "source")], [{ language: "python" as const, path: "a.py", source }]);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["node:3:for x in y:\n    i += 1"]);
  });

  it("finds call arguments syntactically, on the indexed workspace path", async () => {
    const source = "zip(a, b)\nzip(a, b, strict=True)\nzip(a, b, **kw)\nzip(a, b, **{'strict': True})\n";
    const clause = (id: string, mode: string) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", rule: { pattern: "zip($$$)", arguments: { signature: ["*", "strict"], absent: ["strict"], mode } } } },
      id,
    });
    const { actual, full } = await workspaceAndFull([clause("language", "language"), clause("syntactic", "syntactic")], [{ language: "python" as const, path: "a.py", source }]);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["language:1:zip(a, b)", "syntactic:1:zip(a, b)", "syntactic:3:zip(a, b, **kw)", "syntactic:4:zip(a, b, **{'strict': True})"]);
  });

  it("reads a binding through lookup_symbol and only_binding, on the indexed workspace path", async () => {
    const source = "import os\nMODE = os.P_WAIT\ndef f():\n    global MODE\n    g(MODE)\ndef h():\n    g(MODE)\nb = 1\nb = os.sep\ng(b)\n";
    const clause = (id: string, binding: unknown) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", rule: { pattern: "g($X)" }, constraints: { X: { kind: "identifier", resolves: { binding } } } } },
      id,
    });
    const value = { value: { kind: "attribute" } };
    const { actual, full } = await workspaceAndFull(
      [clause("resolve", value), clause("lookup", { ...value, stopAtGlobal: true }), clause("only", { ...value, only: true })],
      [{ language: "python" as const, path: "a.py", source }],
    );
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["lookup:10:g(b)", "lookup:7:g(MODE)", "only:5:g(MODE)", "only:7:g(MODE)", "resolve:10:g(b)", "resolve:5:g(MODE)", "resolve:7:g(MODE)"]);
  });

  it("does not evaluate binding rules in a file with syntax errors, on the indexed workspace path", async () => {
    const source = "const a = 'x';\nsetTimeout(a);\nfunction (;\n";
    const clause = { violation: violation("AST_MATCH"), forbid: { ast: { language: "typescript", rule: { pattern: "setTimeout($S)" }, constraints: { S: { value: { type: "string", follow: true } } } } }, id: "implied-eval" };
    const { actual, full } = await workspaceAndFull([clause], [{ language: "typescript" as const, path: "a.ts", source }]);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.violations).toEqual([]);
    expect(actual.failures.map(item => item.code)).toEqual(["source-incomplete"]);
  });
});

describe.skipIf(codeRuleDslRuntime === undefined)("file-local name resolution (M7)", () => {
  async function workspaceAndFull(clauses: unknown[], files: Array<{ language: "python" | "typescript" | "tsx"; path: string; source: string; module?: string | string[] }>) {
    const compiled = compileJson(clauses);
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      return { actual: await applyWorkspace(nativeWorkspace, 1, files), full: evaluate(compiled, snapshot(files)) };
    } finally {
      nativeWorkspace.close();
    }
  }
  const texts = (violations: Array<{ clauseId: string; span?: { startByte: number; endByte: number; startLine: number } | undefined }>, source: string) =>
    violations.map(item => `${item.clauseId}:${item.span!.startLine + 1}:${source.slice(item.span!.startByte, item.span!.endByte)}`).sort();

  it("reports Python builtins no binding shadows, in Ruff's order, on the indexed workspace path", async () => {
    const source = "print(len)\nlen = 1\ndef f(print):\n    return print(len)\ndef g():\n    return input()\ninput = None\n";
    const builtin = { ...astForbid({ kind: "identifier", resolves: { global: ["print", "len", "input"] } }), id: "builtin" };
    const shadowed = {
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", rule: { pattern: "$F($$$)" }, constraints: { F: { kind: "identifier", not: { resolves: { global: ["print", "input"] } } } } } },
      id: "shadowed",
    };
    const files = [{ language: "python" as const, path: "a.py", source }];
    const { actual, full } = await workspaceAndFull([builtin, shadowed], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["builtin:1:len", "builtin:1:print", "shadowed:4:print(len)", "shadowed:6:input()"]);
  });

  it("separates TypeScript values from types and honors /* global */ comments only when asked", async () => {
    const source = "/* global jQuery */\ninterface Window { x: number }\nlet w: Window = window;\njQuery(alert);\n";
    const values = { ...astForbid({ resolves: { global: ["Window", "window", "alert", "jQuery"] } }, "typescript"), id: "plain" };
    const directives = { ...astForbid({ resolves: { global: ["alert"], directives: "eslint" } }, "typescript"), id: "eslint" };
    const files = [{ language: "typescript" as const, path: "a.ts", source }];
    const { actual, full } = await workspaceAndFull([values, directives], files);
    expect(actual.violations).toEqual(full.violations);
    expect(texts(actual.violations, source)).toEqual(["eslint:4:alert", "eslint:4:jQuery", "plain:3:window", "plain:4:alert", "plain:4:jQuery"]);
  });

  it("resolves Python callees through the file's imports, on the indexed workspace path", async () => {
    const source = "import subprocess as sp\nfrom os import path\nsp.run(a)\npath.join(b)\nrun(c)\ndef f():\n    from subprocess import run\n    return run(d)\n";
    const rule = { pattern: "$F($$$)" };
    const clause = {
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", rule, constraints: { F: { resolves: { import: ["subprocess.run", "os.path.join"] } } } } },
      id: "import",
    };
    const files = [{ language: "python" as const, path: "a.py", source }];
    const { actual, full } = await workspaceAndFull([clause], files);
    expect(actual.violations).toEqual(full.violations);
    expect(texts(actual.violations, source)).toEqual(["import:3:sp.run(a)", "import:4:path.join(b)", "import:8:run(d)"]);
  });

  it("matches unresolved, star-imported, any-global and qualified names (M10a), on the indexed workspace path", async () => {
    const source = "import os\nx = 1\ndel x\nprint(x, missing, os.sep)\nfrom m import *\nstarred\nclass C:\n    __qualname__\n";
    const clauses = [
      { ...astForbid({ kind: "identifier", resolves: { unresolved: true } }), id: "undefined" },
      { ...astForbid({ kind: "identifier", resolves: { unresolved: "star" } }), id: "star" },
      { ...astForbid({ kind: "identifier", resolves: { global: true } }), id: "global" },
      { ...astForbid({ kind: "attribute", resolves: { import: true } }), id: "import" },
    ];
    const files = [{ language: "python" as const, path: "a.py", source }];
    const { actual, full } = await workspaceAndFull(clauses, files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["global:4:print", "import:4:os.sep", "star:6:starred", "undefined:4:missing", "undefined:4:x"]);
  });

  it("reports unused Python bindings through binds (M10a), on the indexed workspace path", async () => {
    const source = "import os\nimport sys\n__all__ = [\"sys\"]\ndef f():\n    a = 1\n    b = 2\n    print(b)\n    c = 3\n    c = 4\n    return None\n";
    const imports = { ...astForbid({ any: [{ kind: "dotted_name" }, { kind: "identifier" }], binds: { form: "import", latest: true, used: false } }), id: "import" };
    const locals = { ...astForbid({ kind: "identifier", binds: { form: "assignment", scope: "function", latest: true, used: false } }), id: "local" };
    const files = [{ language: "python" as const, path: "a.py", source }];
    const { actual, full } = await workspaceAndFull([imports, locals], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["import:1:os", "local:5:a", "local:9:c"]);
  });

  it("filters the binding a reference reads (M10a), on the indexed workspace path", async () => {
    const source = "import os\ncount = 0\ndef f():\n    global count\n    count += 1\n    return os, count\n";
    const imports = { ...astForbid({ kind: "identifier", resolves: { binding: { form: "import", scope: "module", nestedUse: true } } }), id: "import" };
    const outer = { ...astForbid({ kind: "identifier", resolves: { binding: { outer: true, latest: true } } }), id: "outer" };
    const files = [{ language: "python" as const, path: "a.py", source }];
    const { actual, full } = await workspaceAndFull([imports, outer], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["import:6:os", "outer:6:count"]);
  });

  it("reports Python redefinitions through binds.redefines (M10a), on the indexed workspace path", async () => {
    const source = "import os\nimport os\nif c:\n    import sys\nelse:\n    import sys\nfor os in []:\n    pass\n";
    const redefined = { ...astForbid({ kind: "dotted_name", binds: { form: "import", redefines: { form: "import", used: false, sameImport: true } } }), id: "f811" };
    const loop = { ...astForbid({ kind: "identifier", binds: { form: "for", redefines: { form: "import" } } }), id: "f402" };
    const files = [{ language: "python" as const, path: "a.py", source }];
    const { actual, full } = await workspaceAndFull([redefined, loop], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["f402:7:os", "f811:2:os"]);
  });

  it("counts a name's bindings within a captured loop through binds.bindings, on the indexed workspace path", async () => {
    const source = "def f(y):\n    i = 0\n    for x in y:\n        i += 1\n        match x:\n            case [i]:\n                pass\n    j = 0\n    for x in y:\n        j += 1\n        print([j for j in x])\n";
    const once = {
      ...astForbid({ kind: "identifier", inside: { kind: "for_statement", pattern: "$LOOP", stopBy: "end" }, binds: { form: "augmented", bindings: { within: "LOOP", max: 1 } } }),
      id: "once",
    };
    const files = [{ language: "python" as const, path: "a.py", source }];
    const { actual, full } = await workspaceAndFull([once], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["once:10:j"]);
  });

  it("reports TypeScript redeclarations and unused variables where ESLint does (M10b), on the indexed workspace path", async () => {
    const source = "var a = 1; var a = 2;\nvar Object = 0;\nlet b;\nb = 1;\n";
    const clause = (id: string, rule: unknown) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "typescript", languageOptions: { sourceType: "script" }, rule } },
      id,
    });
    const redeclare = clause("redeclare", { kind: "identifier", any: [{ binds: { redefines: {} } }, { binds: { builtin: true } }] });
    const unused = clause("unused", { kind: "identifier", resolves: { write: "last", binding: { used: false } } });
    const files = [{ language: "typescript" as const, path: "a.ts", source }];
    const { actual, full } = await workspaceAndFull([redeclare, unused], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["redeclare:1:a", "redeclare:2:Object", "unused:1:a", "unused:2:Object", "unused:4:b"]);
  });

  it("reports unused TypeScript variables through binds (M10b), on the indexed workspace path", async () => {
    const source = "import React from 'react';\nimport { a, b } from 'm';\nlet c = 0;\nc++;\nfunction f(d, e) { return e; }\nexport const g = <div>{a}</div>;\n";
    const clause = (id: string, languageOptions: unknown) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "typescript", languageOptions, rule: { kind: "identifier", binds: { used: false, exported: false } } } },
      id,
    });
    const files = [{ language: "tsx" as const, path: "a.tsx", source }];
    const { actual, full } = await workspaceAndFull([clause("pragma", {}), clause("none", { jsxPragma: null })], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["none:1:React", "none:2:b", "none:3:c", "none:5:d", "none:5:f", "pragma:2:b", "pragma:3:c", "pragma:5:d", "pragma:5:f"]);
  });

  it("parses format strings and reads a format call's arguments (M11a), on the indexed workspace path", async () => {
    const source = "'%(a)s %z' % d\n'{0}{}'.format(1, 2)\n'{a}'.format(a=1, b=2)\n'%(a)s' % {'a': 1, 'b': 2}\nf'{x}%z' % y\n";
    const clauses = [
      { ...astForbid({ kind: "string", parse: { grammar: "percentFormat", has: { kind: "error", capture: "C" } } }), id: "bad" },
      { ...astForbid({ kind: "string", parse: { grammar: "braceFormat", all: [{ has: { kind: "auto", stopBy: "end" } }, { has: { kind: "index", stopBy: "end" } }] } }), id: "mixed" },
      { ...astForbid({ pattern: "$S.format($$$A)", formatArgs: { style: "brace", string: "S", args: "A", unused: { kind: "keyword_argument" } } }), id: "keyword" },
      { ...astForbid({ pattern: "$S % $R", formatArgs: { style: "percent", string: "S", args: "R", unused: { kind: "pair" } } }), id: "pair" },
    ];
    const files = [{ language: "python" as const, path: "a.py", source }];
    const { actual, full } = await workspaceAndFull(clauses, files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["bad:1:'%(a)s %z'", "keyword:3:'{a}'.format(a=1, b=2)", "mixed:2:'{0}{}'", "pair:4:'%(a)s' % {'a': 1, 'b': 2}"]);
    expect(actual.violations.find(violation => violation.clauseId === "bad")?.variables).toBeDefined();
  });

  it("reads names in string type definitions (M11a), on the indexed workspace path", async () => {
    const source = "import os\nimport json\nfrom typing import Literal, cast\ndef f(value: \"os.PathLike\") -> \"Missing\":\n    local: int\n    return cast(\"Literal['json']\", value)\nx: \"List[\"\n";
    const strings = [{ kind: "string" }, { kind: "concatenated_string" }];
    const clauses = [
      { ...astForbid({ any: strings, parse: { grammar: "annotation", has: { kind: "identifier", resolves: { unresolved: true }, stopBy: "end", capture: "N" } } }), id: "undefined", violation: { anchor: "capture.N", code: "undefined", message: "undefined $N" } },
      { ...astForbid({ any: strings, parse: { grammar: "annotation", kind: "ERROR" } }), id: "syntax" },
      { ...astForbid({ any: [{ kind: "dotted_name" }, { kind: "identifier" }], binds: { form: "import", latest: true, used: false } }), id: "import" },
      { ...astForbid({ kind: "identifier", binds: { form: "annotation", scope: "function", used: false } }), id: "annotation" },
    ];
    const files = [{ language: "python" as const, path: "a.py", source }];
    const { actual, full } = await workspaceAndFull(clauses, files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["annotation:5:local", "import:2:json", "syntax:7:\"List[\"", "undefined:4:Missing"]);
  });

  it("refuses files with syntax errors rather than reading a hidden binding as absent", async () => {
    const files = [{ language: "python" as const, path: "a.py", source: "print(1)\ndef (:\n" }];
    const { actual, full } = await workspaceAndFull([astForbid({ kind: "identifier", resolves: { global: ["print"] } })], files);
    expect(actual.violations).toEqual([]);
    expect(actual.failures.map(item => item.code)).toEqual(["source-incomplete"]);
    expect(full.failures.map(item => item.code)).toEqual(["source-incomplete"]);
  });

  it("limits global names to a query's language options (M8), on the indexed workspace path", async () => {
    const source = "Reflect();\nnew String(1);\nthis.eval(2);\n";
    const clause = (id: string, languageOptions: unknown) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "typescript", languageOptions, rule: { any: [{ kind: "identifier" }, { kind: "this" }], resolves: { global: ["Reflect", "String"], this: "global" } } } },
      id,
    });
    const files = [{ language: "typescript" as const, path: "a.ts", source }];
    const { actual, full } = await workspaceAndFull(
      [clause("es5", { ecmaVersion: 5, sourceType: "script" }), clause("off", { globals: { String: "off" } }), clause("latest", {})],
      files,
    );
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["es5:2:String", "es5:3:this", "latest:1:Reflect", "latest:2:String", "off:1:Reflect"]);
  });

  it("reads a Python query's version for builtins and annotations (M8), on the indexed workspace path", async () => {
    const source = "def f(x: Sequence) -> None:\n    return aiter(x)\nfrom typing import Sequence\n";
    const clause = (id: string, pythonVersion: string) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", languageOptions: { pythonVersion }, rule: { kind: "identifier", resolves: { global: ["aiter"], import: ["typing.Sequence"] } } } },
      id,
    });
    const files = [{ language: "python" as const, path: "a.py", source }];
    const { actual, full } = await workspaceAndFull([clause("py39", "3.9"), clause("py313", "3.13"), clause("py314", "3.14")], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["py313:2:aiter", "py314:1:Sequence", "py314:2:aiter"]);
  });

  it("matches import sites by the names they import, with prefixes (M8), on the indexed workspace path", async () => {
    const source = "import pickle\nfrom xml.etree import ElementTree\nfrom . import pickle as p\nimport pickles\n";
    const clause = {
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", rule: { kind: "dotted_name", resolves: { imported: ["pickle", "xml.etree"], prefix: true } } } },
      id: "imported",
    };
    const { actual, full } = await workspaceAndFull([clause], [{ language: "python" as const, path: "a.py", source }]);
    expect(actual.violations).toEqual(full.violations);
    expect(texts(actual.violations, source)).toEqual(["imported:1:pickle", "imported:2:ElementTree", "imported:2:xml.etree"]);
  });

  it("asks only whether a global name is shadowed with declared: false, on the indexed workspace path", async () => {
    const source = "window.alert(1);\nalert(2);\nfunction f(alert) { alert(3); }\n";
    const clause = (id: string, declared: boolean) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "typescript", languageOptions: { ecmaVersion: 5, sourceType: "script", globals: { alert: "off" } }, rule: { kind: "identifier", resolves: { global: ["window", "alert"], declared } } } },
      id,
    });
    const files = [{ language: "typescript" as const, path: "a.js", source }];
    const { actual, full } = await workspaceAndFull([clause("declared", true), clause("shadow", false)], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["shadow:1:window", "shadow:2:alert"]);
  });

  it("counts a script's top-level declarations as the global with scriptDeclarations, on the indexed workspace path", async () => {
    const source = "var Array;\nArray.prototype.a = 1;\nfunction f(Array) { Array; }\n";
    const clause = (id: string, sourceType: string, scriptDeclarations?: true) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "typescript", languageOptions: { sourceType }, rule: { kind: "identifier", resolves: { global: ["Array"], ...(scriptDeclarations ? { scriptDeclarations } : {}) } } } },
      id,
    });
    const files = [{ language: "typescript" as const, path: "a.js", source }];
    const { actual, full } = await workspaceAndFull([clause("off", "script"), clause("script", "script", true), clause("module", "module", true)], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["script:2:Array"]);
  });

  it("reads blocks as scopes only from ES2015, one table per reading, on the indexed workspace path", async () => {
    const source = "{ function alert() {} }\nalert();\n";
    const clause = (id: string, ecmaVersion: number) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "typescript", languageOptions: { ecmaVersion, sourceType: "script" }, rule: { kind: "identifier", resolves: { global: ["alert"], declared: false } } } },
      id,
    });
    const files = [{ language: "typescript" as const, path: "a.js", source }];
    const { actual, full } = await workspaceAndFull([clause("es5", 5), clause("es2015", 2015)], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["es2015:2:alert"]);
  });

  it("matches a this in eslint-scope's global scope with this: globalScope, on the indexed workspace path", async () => {
    const source = "this.alert(1);\n{ this.alert(2); }\nfunction f() { this.alert(3); }\n";
    const clause = (id: string, ecmaVersion: number) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "typescript", languageOptions: { ecmaVersion, sourceType: "script" }, rule: { kind: "this", resolves: { this: "globalScope" } } } },
      id,
    });
    const files = [{ language: "typescript" as const, path: "a.js", source }];
    const { actual, full } = await workspaceAndFull([clause("es5", 5), clause("es2015", 2015)], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["es2015:1:this", "es5:1:this", "es5:2:this"]);
  });

  it("reads a CommonJS file's top-level this as the global object, as ESLint's no-eval does, on the indexed workspace path", async () => {
    const source = "this.eval(1);\nfunction f() { this.eval(2); }\n";
    const clause = (id: string, sourceType: string) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "typescript", languageOptions: { sourceType }, rule: { kind: "this", resolves: { this: "global" } } } },
      id,
    });
    const files = [{ language: "typescript" as const, path: "a.js", source }];
    const { actual, full } = await workspaceAndFull([clause("commonjs", "commonjs"), clause("module", "module")], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["commonjs:1:this"]);
  });

  it("binds a non-strict ordinary function's this to the global object with sloppyFunctions, on the indexed workspace path", async () => {
    const source = "function f() { this.eval(1); }\nfunction g() { 'use strict'; this.eval(2); }\n";
    const clause = (id: string, languageOptions: unknown) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "typescript", languageOptions, rule: { kind: "this", resolves: { this: "global", sloppyFunctions: true } } } },
      id,
    });
    const files = [{ language: "typescript" as const, path: "a.js", source }];
    const { actual, full } = await workspaceAndFull(
      [
        clause("es5", { ecmaVersion: 5, sourceType: "script" }),
        clause("es3", { ecmaVersion: 3, sourceType: "script" }),
        clause("implied", { ecmaVersion: 5, sourceType: "script", parserOptions: { ecmaFeatures: { impliedStrict: true } } }),
        clause("module", { sourceType: "module" }),
      ],
      files,
    );
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["es3:1:this", "es3:2:this", "es5:1:this"]);
  });

  it("names relative imports by their module text with relative: text, on the indexed workspace path", async () => {
    const source = "from .pdb import set_trace\nfrom . import pdb\nfrom pdb import foo\n";
    const clause = (id: string, relative?: "text") => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", rule: { kind: "dotted_name", resolves: { imported: ["pdb", "pdb.set_trace"], ...(relative ? { relative } : {}) } } } },
      id,
    });
    const files = [{ language: "python" as const, path: "a.py", source }];
    const { actual, full } = await workspaceAndFull([clause("never"), clause("text", "text")], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["never:3:pdb", "text:1:pdb", "text:1:set_trace", "text:2:pdb", "text:3:pdb"]);
  });

  it("keeps an imported-only query partial in a file with syntax errors, on the indexed workspace path", async () => {
    const source = "from .__future__ import nope\ndef (:\n";
    const clause = (id: string, resolves: unknown) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", rule: { kind: "dotted_name", resolves } } },
      id,
    });
    const clauses = [clause("imported", { imported: ["__future__"], relative: "text" }), clause("global", { global: ["nope"] })];
    const files = [{ language: "python" as const, path: "broken.py", source }];
    const { actual, full } = await workspaceAndFull(clauses, files);
    expect(actual.violations).toEqual(full.violations);
    expect(texts(actual.violations, source)).toEqual(["imported:1:__future__"]);
    for (const failures of [actual.failures, full.failures]) {
      expect(failures.map(item => `${item.clauseId}:${item.code}`).sort()).toEqual(["global:source-incomplete", "imported:source-partial"]);
    }
  });

  it("applies a version-gated query only to its versions, soundly, on the indexed workspace path", async () => {
    const source = "zip(a, b)\n";
    const clause = (id: string, pythonVersion: string) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", languageOptions: { pythonVersion }, when: { pythonVersion: { min: "3.10" } }, rule: { kind: "identifier", regex: "^zip$" } } },
      id,
    });
    const files = [{ language: "python" as const, path: "a.py", source }];
    const { actual, full } = await workspaceAndFull([clause("py39", "3.9"), clause("py310", "3.10")], files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(full.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["py310:1:zip"]);
  });

  it("applies a source-gated query only to its sourceType and ecmaFeatures, soundly, on the indexed workspace path", async () => {
    const source = "'use strict';\n";
    const clause = (id: string, languageOptions: unknown) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "typescript", languageOptions, when: { sourceType: ["script", "commonjs"], ecmaFeatures: { impliedStrict: false } }, rule: { kind: "expression_statement" } } },
      id,
    });
    const clauses = [
      clause("script", { sourceType: "script" }),
      clause("commonjs", { sourceType: "commonjs" }),
      clause("module", { sourceType: "module" }),
      clause("implied", { sourceType: "script", parserOptions: { ecmaFeatures: { impliedStrict: true } } }),
    ];
    const files = [{ language: "typescript" as const, path: "a.ts", source }];
    const { actual, full } = await workspaceAndFull(clauses, files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(full.failures).toEqual([]);
    expect(actual.violations.map(item => item.clauseId).sort()).toEqual(["commonjs", "script"]);
  });

  it("keeps a gated query's captures, constraints and templates whether its gate holds, on the indexed workspace path", async () => {
    const source = "function foo() {}\n";
    const clause = (id: string, languageOptions: unknown, when: unknown) => ({
      violation: { anchor: "capture.NAME", code: "AST_MATCH", fix: "$NAME", message: "fn $NAME" },
      forbid: { ast: {
        constraints: { NAME: { regex: "^f" } },
        language: "typescript",
        languageOptions,
        rule: { compare: { left: "NAME", op: "eq", right: "NAME" }, has: { kind: "identifier", pattern: "$NAME", stopBy: "neighbor" }, kind: "function_declaration" },
        when,
      } },
      id,
    });
    const clauses = [
      clause("script", { sourceType: "script" }, { sourceType: "script" }),
      clause("module", {}, { sourceType: "script" }),
      clause("es3", { ecmaVersion: 3, sourceType: "script" }, { ecmaVersion: { max: 3 } }),
      clause("latest", {}, { ecmaVersion: { max: 3 } }),
      clause("implied", { parserOptions: { ecmaFeatures: { impliedStrict: true } } }, { ecmaFeatures: { impliedStrict: true } }),
      clause("sloppy", {}, { ecmaFeatures: { impliedStrict: true } }),
    ];
    const files = [{ language: "typescript" as const, path: "a.js", source }];
    const { actual, full } = await workspaceAndFull(clauses, files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(full.failures).toEqual([]);
    expect(actual.violations.map(item => `${item.clauseId}:${item.message}`).sort()).toEqual(["es3:fn foo", "implied:fn foo", "script:fn foo"]);
  });

  it("applies a query only to files whose module path meets its file condition, on the indexed workspace path", async () => {
    const source = "x = 1\n";
    const clause = (id: string, file: unknown) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", file, rule: { kind: "identifier" } } },
      id,
    });
    const clauses = [
      clause("loose", { inPackage: false }),
      clause("sub", { module: { regex: "^pkg\\.sub\\." } }),
      clause("init", { moduleName: { regex: "^__init__$" } }),
      clause("not-sub", { inPackage: true, module: { notRegex: "^pkg\\.sub\\." }, moduleName: { regex: ".", notRegex: "^b$" } }),
    ];
    const files = [
      { language: "python" as const, path: "loose/a.py", source },
      { language: "python" as const, path: "pkg/sub/b.py", source, module: "pkg.sub.b" },
      { language: "python" as const, path: "pkg/__init__.py", source, module: "pkg.__init__" },
    ];
    const { actual, full } = await workspaceAndFull(clauses, files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(actual.violations.map(item => `${item.clauseId}:${item.path}`).sort()).toEqual(["init:pkg/__init__.py", "loose:loose/a.py", "not-sub:pkg/__init__.py", "sub:pkg/sub/b.py"]);
    // A new module path for unchanged source is re-evaluated.
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compileJson(clauses));
      await applyWorkspace(nativeWorkspace, 1, files);
      const moved = await applyWorkspace(nativeWorkspace, 2, [{ ...files[1]!, module: "pkg.other.b" }]);
      expect(moved.violations.map(item => `${item.clauseId}:${item.path}`).sort()).toEqual(["init:pkg/__init__.py", "loose:loose/a.py", "not-sub:pkg/__init__.py"]);
    } finally {
      nativeWorkspace.close();
    }
  });

  it("binds private names per class body as ESLint's no-unused-private-class-members reads them, on the indexed workspace path", async () => {
    const source = "class A {\n  #read = 1;\n  #written = 0;\n  get #pair() { return 1; }\n  set #pair(v) {}\n  m(o) { this.#read; this.#written = 1; this.#written += 1; #pair in o; }\n}\n";
    const clause = (id: string, binds: unknown) => ({ violation: violation("AST_MATCH"), forbid: { ast: { language: "typescript", rule: { kind: "private_property_identifier", binds } } }, id });
    const clauses = [clause("unused", { used: false }), clause("accessor", { form: "accessor", redefines: {} }), clause("unreferenced", { referenced: false })];
    const { actual, full } = await workspaceAndFull(clauses, [{ language: "typescript", path: "a.ts", source }]);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual(["accessor:5:#pair", "unused:3:#written"]);
  });

  it("declares typescript-eslint's lib globals in their namespaces, on the indexed workspace path", async () => {
    const source = "type NodeListOf = 1;\nlet a: NodeList = NodeList;\nNodeListOf;\nlet b: Missing;\n";
    const names = [{ kind: "identifier" }, { kind: "type_identifier" }];
    const clause = (id: string, rule: unknown, languageOptions: unknown) => ({ violation: violation("AST_MATCH"), forbid: { ast: { language: "typescript", languageOptions, rule } }, id });
    const clauses = [
      clause("undef", { any: names, resolves: { unresolved: true } }, { lib: ["dom"] }),
      clause("undef-nolib", { any: names, resolves: { unresolved: true } }, {}),
      clause("builtin", { any: names, binds: { builtin: true } }, { sourceType: "script", lib: ["dom"] }),
    ];
    const { actual, full } = await workspaceAndFull(clauses, [{ language: "typescript", path: "a.ts", source }]);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(texts(actual.violations, source)).toEqual([
      "builtin:1:NodeListOf",
      "undef-nolib:2:NodeList",
      "undef-nolib:2:NodeList",
      "undef-nolib:3:NodeListOf",
      "undef-nolib:4:Missing",
      "undef:3:NodeListOf",
      "undef:4:Missing",
    ]);
  });

  it("applies a query only to files whose path meets its file condition, in every language, on the indexed workspace path", async () => {
    const clause = (id: string, language: "typescript" | "python", file: unknown) => ({
      violation: violation("AST_MATCH"),
      forbid: { ast: { language, file, rule: { kind: language === "python" ? "module" : "program" } } },
      id,
    });
    const clauses = [
      clause("declaration", "typescript", { declaration: true }),
      clause("code", "typescript", { declaration: false, path: { regex: "^src/", notRegex: "\\.test\\.tsx?$" } }),
      clause("tests", "python", { path: { regex: "(?:^|/)tests/" } }),
    ];
    const files = [
      { language: "typescript" as const, path: "src/types.d.ts", source: "declare const a: number;\n" },
      { language: "typescript" as const, path: "src/a.ts", source: "let a = 1;\n" },
      { language: "typescript" as const, path: "src/a.test.ts", source: "let a = 1;\n" },
      { language: "tsx" as const, path: "src/View.tsx", source: "export const V = () => <div />;\n" },
      { language: "python" as const, path: "pkg/tests/test_a.py", source: "x = 1\n", module: "pkg.tests.test_a" },
      { language: "python" as const, path: "pkg/a.py", source: "x = 1\n", module: "pkg.a" },
    ];
    const { actual, full } = await workspaceAndFull(clauses, files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(actual.violations.map(item => `${item.clauseId}:${item.path}`).sort()).toEqual([
      "code:src/View.tsx",
      "code:src/a.ts",
      "declaration:src/types.d.ts",
      "tests:pkg/tests/test_a.py",
    ]);
  });

  it("reads dotted directory and file names from a module sent as a list, on the indexed workspace path", async () => {
    const source = "class C: ...\ndef f(x=C()): ...\nfrom . import x\nx.y\n";
    const clauses = [
      {
        violation: violation("AST_MATCH"),
        forbid: { ast: { language: "python", rule: { any: [{ kind: "identifier" }, { kind: "attribute" }], resolves: { import: ["json.__init__.C", "json.x.y", "good.x.y", "good.my.mod.C"] } } } },
        id: "qualified",
      },
      { violation: violation("AST_MATCH"), forbid: { ast: { language: "python", file: { package: { regex: "\\." } }, rule: { kind: "module" } } }, id: "package" },
      { violation: violation("AST_MATCH"), forbid: { ast: { language: "python", file: { moduleName: { regex: "\\." }, inPackage: true }, rule: { kind: "module" } } }, id: "name" },
    ];
    const files = [
      { language: "python" as const, path: "json.d/__init__.py", source, module: ["json.d", "__init__"] },
      { language: "python" as const, path: "good/my.mod.py", source, module: ["good", "my.mod"] },
    ];
    const { actual, full } = await workspaceAndFull(clauses, files);
    expect(actual.violations).toEqual(full.violations);
    expect(actual.failures).toEqual([]);
    expect(actual.violations.map(item => `${item.clauseId}:${item.path}:${source.slice(item.span?.startByte, item.span?.endByte).split("\n")[0]}`).sort()).toEqual([
      "name:good/my.mod.py:class C: ...",
      "package:json.d/__init__.py:class C: ...",
      "qualified:good/my.mod.py:x.y",
      "qualified:json.d/__init__.py:C",
      "qualified:json.d/__init__.py:x.y",
    ]);
  });

  it("qualifies Python definitions by the file's module path (M8), on the indexed workspace path", async () => {
    const source = "def suppress(): pass\nwith suppress(): pass\nfrom . import sibling\nsibling.run()\n";
    const clause = {
      violation: violation("AST_MATCH"),
      forbid: { ast: { language: "python", rule: { any: [{ kind: "identifier" }, { kind: "attribute" }], resolves: { import: ["contextlib.suppress", "pkg.contextlib.suppress", "pkg.sibling.run"] } } } },
      id: "module",
    };
    const stem = [{ language: "python" as const, path: "fn/contextlib.py", source }];
    const stemmed = await workspaceAndFull([clause], stem);
    expect(stemmed.actual.violations).toEqual(stemmed.full.violations);
    expect(texts(stemmed.actual.violations, source)).toEqual(["module:2:suppress"]);
    const packaged = [{ language: "python" as const, path: "src/pkg/contextlib.py", source, module: "pkg.contextlib" }];
    const moduled = await workspaceAndFull([clause], packaged);
    expect(moduled.actual.violations).toEqual(moduled.full.violations);
    expect(texts(moduled.actual.violations, source)).toEqual(["module:2:suppress", "module:4:sibling.run"]);
    // A new module path for unchanged source is re-evaluated.
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compileJson([clause]));
      await applyWorkspace(nativeWorkspace, 1, packaged);
      const moved = await applyWorkspace(nativeWorkspace, 2, [{ ...packaged[0]!, module: "other.contextlib" }]);
      expect(texts(moved.violations, source)).toEqual([]);
    } finally {
      nativeWorkspace.close();
    }
  });
});

describe.skipIf(codeRuleDslRuntime === undefined)("violation multiplicity (M12b)", () => {
  it("reports one violation per distinct binding with each, with the same ids on the indexed workspace path", async () => {
    const rule = { kind: "comparison_operator", has: { any: [{ kind: "true" }, { kind: "false" }], pattern: "$OP" } };
    const once = astForbid(rule);
    const each = { ...astForbid(rule), id: "each", violation: { ...violation("AST_MATCH"), each: "OP", anchor: "capture.OP" } };
    const compiled = compileJson([once, each]);
    const source = "if res == True != False:\n    pass\n";
    const files = [{ language: "python", path: "a.py", source }];
    const stateless = evaluate(compiled, snapshot(files as never)).violations;
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const actual = await applyWorkspace(nativeWorkspace, 1, files);
      expect(actual.violations).toEqual(stateless);
    } finally {
      nativeWorkspace.close();
    }
    const texts = stateless.map(item => `${item.clauseId}:${source.slice(item.span!.startByte, item.span!.endByte)}`).sort();
    expect(texts).toEqual(["ast-match:res == True != False", "each:False", "each:True"]);
    expect(new Set(stateless.map(item => item.id)).size).toBe(3);
    // Without each, a violation's id is the one it always had.
    const alone = evaluate(compileJson([once]), snapshot(files as never)).violations;
    expect(alone.map(item => item.id)).toEqual(stateless.filter(item => item.clauseId === "ast-match").map(item => item.id));
  });

  it("reads a transform's value and default the same on both paths", async () => {
    const rule = { any: [{ pattern: "f($A)" }, { pattern: "g()" }] };
    const base = astForbid(rule, "typescript") as { forbid: { ast: Record<string, unknown> } };
    const transform = { V: { replace: { source: "$A", value: true, default: "", replace: "^$", by: "none" } } };
    const clause = {
      ...base,
      id: "values",
      violation: { ...violation("AST_MATCH"), message: "[$V]" },
      forbid: { ast: { ...base.forbid.ast, transform } },
    };
    const compiled = compileJson([clause]);
    const source = "f(' \\x61 ');\nf(1e2);\nf(undefined);\nf(a.b);\ng();\n";
    const files = [{ language: "typescript", path: "a.ts", source }];
    const stateless = evaluate(compiled, snapshot(files as never)).violations;
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const actual = await applyWorkspace(nativeWorkspace, 1, files);
      expect(actual.violations).toEqual(stateless);
    } finally {
      nativeWorkspace.close();
    }
    expect(stateless.map(item => item.message)).toEqual(["[ a ]", "[100]", "[undefined]", "[none]", "[none]"]);
  });

  it("reports one violation per match of a same-node regex group with each, on both paths", async () => {
    const rule = { kind: "format_specifier", regex: "\\\\[nrt\\\\]|(?<BS>\\\\)(?:[^nrt\\\\]|$)" };
    const each = { ...astForbid(rule), id: "each", violation: { ...violation("AST_MATCH"), each: "BS", anchor: "capture.BS", message: "escape $BS" } };
    const compiled = compileJson([each]);
    const source = 'f"{x:\\d\\n\\q}"\n';
    const files = [{ language: "python", path: "a.py", source }];
    const stateless = evaluate(compiled, snapshot(files as never)).violations;
    const nativeWorkspace = workspace();
    try {
      nativeWorkspace.addCodeRule(compiled);
      const actual = await applyWorkspace(nativeWorkspace, 1, files);
      expect(actual.violations).toEqual(stateless);
    } finally {
      nativeWorkspace.close();
    }
    expect(stateless.map(item => item.span!.startByte).sort()).toEqual([5, 9]);
    expect(new Set(stateless.map(item => item.id)).size).toBe(2);
  });
});
