/**
 * ruff-b030-except-with-non-exception-classes (packs/python/, candidate) must not flag every `except T as name`
 * (NER-1816). tree-sitter-python wraps `T as name` in an as_pattern node, which sits right after `except`; the port
 * treated that as_pattern as the handler type and reported it as a non-class expression, steering agents to rewrite
 * working error handling. Ruff 0.16.9's B030 reports only handler types that are not a name, attribute, subscript or
 * call (after flattening tuples, `*` unpacking and `+`), whether or not the handler binds a name.
 *
 * Each case below is one handler; the test asserts exactly which lines the rule reports.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../src/engine/native/native.js";
import { readRuleDocument } from "../../../../src/engine/catalog/bundled-rule-documents.js";
import { repositoryPath } from "../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_PATH = repositoryPath("packs", "python", "candidates", "rules", "ruff-b030-except-with-non-exception-classes.json");

const evaluationSchema = z.object({
  violations: z.array(z.object({ path: z.string(), message: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function compile(): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = readRuleDocument(RULE_PATH);
  return runtime.compile(JSON.stringify(document.rule), "json", capabilities);
}

/** The 1-based lines the rule reports in `source`. */
function reportedLines(compiled: NativeCompiledCodeRuleDsl, source: string): number[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path: "src/handlers.py", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.span.startLine + 1).sort((a, b) => a - b);
}

/** A try statement whose single handler is `handler`, so the handler is on line 3. */
const handle = (handler: string) => ["try:", "    run()", `${handler}:`, "    pass", ""].join("\n");

/** Handlers Ruff 0.16.9 leaves alone: every flattened type is a name, attribute, subscript or call. */
const SILENT: ReadonlyArray<[string, string]> = [
  ["a bare class", "except ValueError"],
  ["a class bound with as", "except Exception as e"],
  ["a tuple of classes bound with as", "except (ValueError, KeyError) as e"],
  ["an except* group bound with as", "except* OSError as eg"],
  ["an attribute bound with as", "except asyncio.TimeoutError as e"],
  ["a call bound with as", "except get_errors() as e"],
  ["a parenthesized class bound with as", "except (ValueError) as e"],
];

/** Handlers Ruff 0.16.9 reports: a flattened type is not a class expression, with or without a binding. */
const REPORTED: ReadonlyArray<[string, string]> = [
  ["a number bound with as", "except 1 as e"],
  ["a string bound with as", 'except "x" as e'],
  ["a number in a tuple bound with as", "except (ValueError, 1) as e"],
  ["a number without a binding", "except 1"],
];

describe("ruff-b030-except-with-non-exception-classes", () => {
  const compiled = compile();

  it.each(SILENT)("does not report %s", (_name, handler) => {
    expect(reportedLines(compiled, handle(handler))).toEqual([]);
  });

  it.each(REPORTED)("reports %s", (_name, handler) => {
    expect(reportedLines(compiled, handle(handler))).toEqual([3]);
  });
});
