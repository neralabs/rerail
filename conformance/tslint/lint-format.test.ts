import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { FIXTURE_RULES, loadExcludedRules, loadRuleList, loadScoredRules, loadVariants } from "./fixtures.js";
import { LintSyntaxError, parseLintFile, preprocessDirectives, satisfiesRange, type ParsedLintFile } from "./lint-format.js";
import { COMMITTED_PORTS_DIRECTORY } from "./runner.js";

// Verbatim excerpts of palantir/tslint test/rules fixtures (Apache-2.0) at the pinned SHA, so these run without a fetch.
const NO_DEBUGGER = `var testVariable = "debugger";

function testFunction(): number {
    if (testVariable === "debugger") {
        debugger;
        ~~~~~~~~~  [Use of debugger statements is forbidden]
    }
}
`;

// no-var-keyword/module.d.ts.lint: shortcut message ids resolved from a trailing definition.
const SHORTCUT = `export var x: number;
       ~~~ [0]

declare global {
    var x: number;
}

[0]: Forbidden 'var' keyword, use 'let' or 'const' instead
`;

// curly/defaults/test.ts.lint (excerpt): a failure spanning two lines, opened by a bare ~~~ line.
const MULTILINE = `    if (z == 5) // failure
    ~~~~~~~~~~~~~~~~~~~~~~
        console.log("z");
~~~~~~~~~~~~~~~~~~~~~~~~~ [if statements must be braced]
`;

// trailing-comma/singleline-always/test.ts.lint (excerpt): ~nil marks zero-width failures.
const NIL = `var b = [{a: 1, b: 2, d: 34, c: (a + b)}];
                                       ~nil    [Missing trailing comma]
                                        ~nil   [Missing trailing comma]
`;

// no-shadowed-variable/temporal-dead-zone/test.ts.lint (excerpt): printf-style template substitution.
const TEMPLATE = `['a', 'b', 'c'].map((path) => path.resolve(path))
                     ~~~~ [err % ('path')]

[err]: Shadowed name: '%s'
`;

function evaluated(parsed: ParsedLintFile): Extract<ParsedLintFile, { skipped: false }> {
  if (parsed.skipped) throw new Error(`unexpectedly skipped: ${parsed.requirement}`);
  return parsed;
}

describe("TSLint .lint parser", () => {
  it("strips markup and reports a single-line failure", () => {
    const parsed = evaluated(parseLintFile(NO_DEBUGGER));
    expect(parsed.source.split("\n")).toHaveLength(8);
    expect(parsed.source).not.toContain("~");
    expect(parsed.failures).toEqual([{ startLine: 4, startCol: 8, endLine: 4, endCol: 17, message: "Use of debugger statements is forbidden" }]);
  });

  it("resolves [0] style shortcuts and drops the definition line from the source", () => {
    const parsed = evaluated(parseLintFile(SHORTCUT));
    expect(parsed.failures).toEqual([
      { startLine: 0, startCol: 7, endLine: 0, endCol: 10, message: "Forbidden 'var' keyword, use 'let' or 'const' instead" },
    ]);
    expect(parsed.source).not.toContain("[0]:");
  });

  it("joins multi-line failures and ends them at the closing marker", () => {
    const parsed = evaluated(parseLintFile(MULTILINE));
    expect(parsed.failures).toEqual([{ startLine: 0, startCol: 4, endLine: 1, endCol: 25, message: "if statements must be braced" }]);
    expect(parsed.source).toBe('    if (z == 5) // failure\n        console.log("z");\n');
  });

  it("treats ~nil as a zero-width failure", () => {
    const parsed = evaluated(parseLintFile(NIL));
    expect(parsed.failures).toEqual([
      { startLine: 0, startCol: 39, endLine: 0, endCol: 39, message: "Missing trailing comma" },
      { startLine: 0, startCol: 40, endLine: 0, endCol: 40, message: "Missing trailing comma" },
    ]);
  });

  it("formats `name % ('arg')` messages against their template", () => {
    expect(evaluated(parseLintFile(TEMPLATE)).failures[0]?.message).toBe("Shadowed name: 'path'");
  });

  it("gates on the [typescript] requirement line and #if directives", () => {
    expect(parseLintFile("[typescript]: < 2.9.0\nfoo;\n")).toEqual({ skipped: true, requirement: "< 2.9.0" });
    expect(evaluated(parseLintFile("[typescript]: >= 2.4.0\nfoo;\n")).source).toBe("foo;\n");
    expect(preprocessDirectives("#if typescript >= 3.0.0\nnew;\n#else\nold;\n#endif\n", "4.0.2")).toBe("new;\n");
    expect(satisfiesRange("4.0.2", ">= 2.4.0 < 3.5.0")).toBe(false);
    expect(satisfiesRange("4.0.2", "<=3.4.5 || >=4.0.0")).toBe(true);
  });

  it("rejects an unterminated multi-line failure", () => {
    expect(() => parseLintFile("foo\n~~~\nbar\n")).toThrow(LintSyntaxError);
  });
});

describe("excluded rules", () => {
  it("names core rules only, none of which keeps a port", () => {
    const core = new Set(loadRuleList().map(rule => rule.name));
    const excluded = [...loadExcludedRules().keys()];
    expect(excluded.filter(rule => !core.has(rule))).toEqual([]);
    expect(excluded.filter(rule => existsSync(join(COMMITTED_PORTS_DIRECTORY, `${rule}.json`)) || existsSync(join(COMMITTED_PORTS_DIRECTORY, rule)))).toEqual([]);
    expect(loadScoredRules()).toHaveLength(core.size - excluded.length);
  });
});

describe.skipIf(!existsSync(FIXTURE_RULES))("fetched TSLint fixtures", () => {
  it("parses every .lint file of every rule without markup errors", () => {
    const errors: string[] = [];
    let files = 0;
    for (const rule of loadRuleList()) {
      for (const variant of loadVariants(rule.name)) {
        for (const file of variant.files) {
          files += 1;
          if (file.parsed.skipped && "parseError" in file.parsed) errors.push(`${file.path}: ${file.parsed.parseError}`);
        }
      }
    }
    expect(errors).toEqual([]);
    expect(files).toBeGreaterThan(400);
  });

  it("matches the inline excerpts against the real fixtures", () => {
    const real = readFileSync(join(FIXTURE_RULES, "no-debugger", "test.ts.lint"), "utf8");
    expect(real).toBe(NO_DEBUGGER);
    const moduleDts = evaluated(parseLintFile(readFileSync(join(FIXTURE_RULES, "no-var-keyword", "module.d.ts.lint"), "utf8")));
    expect(moduleDts.failures).toHaveLength(3);
  });
});
