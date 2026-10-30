import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { expectedFindingsSchema } from "../../conformance/languages/pack.js";
// @ts-expect-error: a JavaScript module without declarations, run by the generators.
import { unitLanguage, withOrigins, writeRenameVariants } from "../../scripts/pack-data/eslint-expected.mjs";
import { inUnitLanguage, queryLanguageOptions } from "../../conformance/languages/runner.js";
import { loadNativeBinding } from "../../src/engine/index.js";
import { capabilitiesJson } from "../../conformance/engine.js";

const directories: string[] = [];
afterEach(() => {
  for (const directory of directories.splice(0)) rmSync(directory, { recursive: true, force: true });
});

function fixtures(files: Record<string, string>): string {
  const directory = mkdtempSync(join(tmpdir(), "rerail-unit-origins-"));
  directories.push(directory);
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(directory, path)), { recursive: true });
    writeFileSync(join(directory, path), text);
  }
  return directory;
}

const expectedFindings = (files: unknown[], units: unknown) => ({
  tool: "t",
  version: "1",
  source: { kind: "fixtures", origin: "https://example.test@0", rules: ["r1", "r2"], files, units },
  findings: [],
});

describe("rename variants and unit origins", () => {
  it("labels each upstream unit with its rules and each kept rename variant with its original", () => {
    const directory = fixtures({
      "r1/001.js": "const foo = 1;\n",
      "r1/002.js": "0;\n",
      "r2/001.js": "let bar = 'baz';\n",
      "r2/package.json": "{}\n",
      "rerail-probe.js": "const mine = 1;\n",
    });
    const units = ["r1/001.js", "r1/002.js", ["r2/001.js", "r2/package.json"], "rerail-probe.js"];
    const origins = new Map([
      ["r1/001.js", ["r1"]],
      ["r1/002.js", ["r1"]],
      ["r2/001.js", ["r2"]],
    ]);
    const { variants, skipped } = writeRenameVariants(directory, units, origins);
    expect(skipped).toEqual([{ of: "r1/002.js", problem: "nothing to rename" }]);
    expect(variants.map((variant: { unit: unknown }) => variant.unit)).toEqual(["renamed/r1/001.js", ["renamed/r2/001.js", "renamed/r2/package.json"]]);
    expect(readFileSync(join(directory, "renamed/r2/package.json"), "utf8")).toBe("{}\n");
    expect(readFileSync(join(directory, "renamed/r1/001.js"), "utf8")).not.toContain("foo");

    const findings = [{ rule: "r1", path: "renamed/r1/001.js", line: 1, column: 7 }];
    const unparsed = new Set(["renamed/r1/001.js"]);
    const written = withOrigins(directory, units, origins, variants, unparsed, findings);
    expect(written.files).toEqual([...units, ["renamed/r2/001.js", "renamed/r2/package.json"]]);
    expect(written.units).toEqual({
      "r1/001.js": { rules: ["r1"] },
      "r1/002.js": { rules: ["r1"] },
      "r2/001.js": { rules: ["r2"] },
      "renamed/r2/001.js": { rules: ["r2"], renamedFrom: "r2/001.js" },
    });
    expect(written.findings).toEqual([]);
    expect(existsSync(join(directory, "renamed/r1/001.js"))).toBe(false);
    expect(expectedFindingsSchema.safeParse(expectedFindings(written.files, written.units)).success).toBe(true);
  });

  it("refuses an origin for a unit the expected findings do not list, and a variant of no original", () => {
    expect(expectedFindingsSchema.safeParse(expectedFindings(["a.js"], { "b.js": { rules: ["r1"] } })).success).toBe(false);
    expect(expectedFindingsSchema.safeParse(expectedFindings(["a.js", "renamed/a.js"], { "renamed/a.js": { rules: ["r1"], renamedFrom: "a.js" } })).success).toBe(false);
  });

  it("records a unit's own language, beyond its extension's defaults, on it and on its rename variant", () => {
    expect(unitLanguage("a.js", { ecmaVersion: 5, sourceType: "script", parserOptions: { ecmaFeatures: { jsx: true, globalReturn: true } } })).toEqual({
      ecmaVersion: 5,
      sourceType: "script",
      parserOptions: { ecmaFeatures: { globalReturn: true } },
    });
    expect(unitLanguage("a.js", { ecmaVersion: "latest", sourceType: "module", parserOptions: { ecmaFeatures: { jsx: true } } })).toBeUndefined();
    expect(unitLanguage("a.cjs", { sourceType: "commonjs" })).toBeUndefined();
    expect(unitLanguage("a.cjs", { sourceType: "module" })).toEqual({ sourceType: "module" });

    const directory = fixtures({ "r1/001.js": "var foo = 1;\n" });
    const origins = new Map([["r1/001.js", ["r1"]]]);
    const { variants } = writeRenameVariants(directory, ["r1/001.js"], origins);
    const language = { ecmaVersion: 5, sourceType: "script" };
    const written = withOrigins(directory, ["r1/001.js"], origins, variants, new Set(), [], new Map([["r1/001.js", language]]));
    expect(written.units).toEqual({
      "r1/001.js": { rules: ["r1"], languageOptions: language },
      "renamed/r1/001.js": { rules: ["r1"], renamedFrom: "r1/001.js", languageOptions: language },
    });
    expect(expectedFindingsSchema.safeParse(expectedFindings(written.files, written.units)).success).toBe(true);
    const mismatched = { ...written.units, "renamed/r1/001.js": { rules: ["r1"], renamedFrom: "r1/001.js" } };
    expect(expectedFindingsSchema.safeParse(expectedFindings(written.files, mismatched)).success).toBe(false);
    expect(expectedFindingsSchema.safeParse(expectedFindings(["a.js"], { "a.js": { rules: ["r1"], languageOptions: { globals: {} } } })).success).toBe(false);
  });

  it("evaluates a unit in its language: merged into each ECMA AST clause's languageOptions, option cases included", () => {
    const language = { ecmaVersion: 5, sourceType: "script" as const, parserOptions: { ecmaFeatures: { jsx: true, impliedStrict: true }, jsxPragma: "h", project: "x" } };
    expect(queryLanguageOptions(language)).toEqual({ ecmaVersion: 5, sourceType: "script", parserOptions: { ecmaFeatures: { impliedStrict: true } }, jsxPragma: "h" });
    const violation = { anchor: "witness.span", code: "c", message: "m" };
    const document = {
      format: "rerail.policy/v1",
      id: "language-probe",
      revision: 1,
      mode: "log",
      description: "d",
      scope: { include: ["**/*"], exclude: [] },
      clauses: [
        { id: "one", forbid: { ast: { language: "javascript", rule: { kind: "debugger_statement" }, languageOptions: { globals: { g: "readonly" }, parserOptions: { ecmaFeatures: { globalReturn: true } } } } }, violation },
        { id: "two", forbid: { ast: { $option: "style", cases: { a: { language: "typescript", rule: { kind: "debugger_statement" } } }, default: { language: "javascript", rule: { kind: "debugger_statement" } } } }, violation },
      ],
    };
    const merged = JSON.parse(inUnitLanguage(JSON.stringify(document), language));
    expect(merged.clauses[0].forbid.ast.languageOptions).toEqual({
      globals: { g: "readonly" },
      ecmaVersion: 5,
      sourceType: "script",
      parserOptions: { ecmaFeatures: { globalReturn: true, impliedStrict: true } },
      jsxPragma: "h",
    });
    expect(merged.clauses[1].forbid.ast.cases.a.languageOptions).toEqual(queryLanguageOptions(language));
    expect(merged.clauses[1].forbid.ast.default.languageOptions).toEqual(queryLanguageOptions(language));
    // The engine takes what the merge writes (the option clause aside: this document declares no option).
    const compilable = { ...merged, clauses: [merged.clauses[0]] };
    expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(compilable), "json", capabilitiesJson())).not.toThrow();
  });
});
