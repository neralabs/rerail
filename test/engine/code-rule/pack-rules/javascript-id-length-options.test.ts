/**
 * eslint-id-length (packs/javascript/rules/) under options other than its defaults, which the conformance fixtures
 * never set (ESLint's expected findings are recorded at the defaults). `min` and `max` fill the length regexes and
 * `exceptions` and `exceptionPatterns` build the exemptions, for any value, so each case uses values none of ESLint's
 * own tests use. A private name is measured and named without its `#`, as `PrivateIdentifier.name` is (ESLint's
 * `tooLongPrivate` message puts the `#` outside the quotes).
 *
 * Each case asserts exactly which names the rule reports, as ESLint 10.11.0 does with those options, in a JavaScript
 * file and a TypeScript one (each language has its own clauses).
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_PATH = repositoryPath("packs", "javascript", "rules", "eslint-id-length.json");

const documentSchema = z.object({ clauses: z.array(z.object({ id: z.string(), define: z.record(z.string(), z.unknown()).optional() }).passthrough()) }).passthrough();
const evaluationSchema = z.object({
  violations: z.array(z.object({ message: z.string() })).default([]),
  failures: z.array(z.unknown()).default([]),
});

/** The rule compiled with `config` as its options' values. */
async function compile(config: Record<string, unknown>): Promise<NativeCompiledCodeRuleDsl> {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = documentSchema.parse(JSON.parse(await readFile(RULE_PATH, "utf8")));
  const options = document.clauses.find(clause => clause.id === "options");
  if (options?.define === undefined) throw new Error("eslint-id-length has no options define");
  options.define.config = config;
  return runtime.compile(JSON.stringify(document), "json", capabilities);
}

/** The messages the rule reports in `source` as `path`, sorted. */
function reported(compiled: NativeCompiledCodeRuleDsl, path: string, source: string): string[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: path.endsWith(".ts") ? "typescript" : "javascript", path, source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.message).sort();
}

const CASES: ReadonlyArray<[string, Record<string, unknown>, string, string[]]> = [
  [
    "a private field longer than max is reported, measured without its #",
    { max: 3 },
    "class Abc { #abcdef = 1; #abc = 2; }\n",
    ["Identifier name #'abcdef' is too long (> 3)."],
  ],
  [
    "a max no ESLint test uses",
    { max: 7 },
    "const abcdefgh = 1, abcdefg = 2;\n",
    ["Identifier name 'abcdefgh' is too long (> 7)."],
  ],
  [
    "a min no ESLint test uses",
    { min: 5 },
    "const abcd = 1, abcde = 2;\n",
    ["Identifier name 'abcd' is too short (< 5)."],
  ],
  [
    "exceptions no ESLint test uses, both ways",
    { min: 3, max: 5, exceptions: ["ab", "abcdefg"] },
    "const ab = 1, ac = 2, abcdefg = 3, abcdefh = 4;\n",
    ["Identifier name 'abcdefh' is too long (> 5).", "Identifier name 'ac' is too short (< 3)."],
  ],
  [
    "exceptionPatterns no ESLint test uses, both ways",
    { min: 3, max: 5, exceptionPatterns: ["^q", "_id$"] },
    "const qa = 1, ra = 2, user_id = 3, user_ix = 4;\n",
    ["Identifier name 'ra' is too short (< 3).", "Identifier name 'user_ix' is too long (> 5)."],
  ],
];

describe("eslint-id-length options", () => {
  it.each(CASES)("%s", async (_name, config, source, expected) => {
    const compiled = await compile(config);
    expect(reported(compiled, "src/names.js", source)).toEqual(expected);
    expect(reported(compiled, "src/names.ts", source)).toEqual(expected);
  });
});
