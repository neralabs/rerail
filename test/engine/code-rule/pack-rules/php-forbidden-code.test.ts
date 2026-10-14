/**
 * psalm-forbidden-code (packs/php/) must not steer an agent into removing a var_dump() whose output is the point
 * (NER-1809). Psalm's ForbiddenCode reports every var_dump() call, and the steer reads as "remove this debugging
 * code". Two idioms use var_dump() on purpose:
 * - output buffering captures it: `ob_start(); var_dump($x); $s = ob_get_clean();` builds a string from the dump
 *   (guzzle/guzzle src/functions.php describe_type(), and Symfony tests that assert on what var_dump() prints);
 * - a dump() or dd() helper falls back to it when no richer dumper is installed (pestphp/pest src/Expectation.php).
 * Removing the call in either case changes behaviour, so the rule stays silent there. packs/php/divergences.json
 * records each upstream finding on the pinned corpora that this drops.
 *
 * Each case asserts exactly which lines the shipped rule reports.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../src/engine/native/native.js";
import { loadBundledRules } from "../../../../src/engine/catalog/bundled-rule-documents.js";
import { repositoryPath } from "../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "psalm-forbidden-code";

const evaluationSchema = z.object({
  violations: z.array(z.object({ path: z.string(), message: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function compile(): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = loadBundledRules([repositoryPath("packs", "php")]).find(candidate => candidate.rule.id === RULE_ID);
  if (document === undefined) throw new Error(`${RULE_ID} is not in packs/php`);
  return runtime.compile(JSON.stringify(document.rule), "json", capabilities);
}

/** The 1-based lines the rule reports in `source`. */
function reportedLines(source: string): number[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "php", path: "src/debug.php", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compile().evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.span.startLine + 1).sort((a, b) => a - b);
}

const php = (...lines: string[]) => ["<?php", ...lines, ""].join("\n");

/** guzzle/guzzle src/functions.php describe_type(): the dump, captured, is the function's result (NER-1809). */
const GUZZLE = php(
  "function describe_type($input): string",
  "{",
  "    switch (\\gettype($input)) {",
  "        case 'object':",
  "            return 'object('.\\get_class($input).')';",
  "        default:",
  "            \\ob_start();",
  "            \\var_dump($input);",
  "            $varDumpContent = \\ob_get_clean();",
  "",
  "            return \\str_replace('double(', 'float(', \\rtrim($varDumpContent));",
  "    }",
  "}",
);

/** The shape of pestphp/pest src/Expectation.php dump() and dd(): var_dump() is the fallback dumper (NER-1809). */
const PEST = php(
  "final class Expectation",
  "{",
  "    public function dump(mixed ...$arguments): self",
  "    {",
  "        if (function_exists('dump')) {",
  "            dump($this->value, ...$arguments);",
  "        } else {",
  "            var_dump($this->value);",
  "        }",
  "",
  "        return $this;",
  "    }",
  "",
  "    public function dd(mixed ...$arguments): void",
  "    {",
  "        if (function_exists('dd')) {",
  "            dd($this->value, ...$arguments);",
  "        }",
  "",
  "        var_dump($this->value);",
  "",
  "        exit(1);",
  "    }",
  "}",
);

/** Captured or helper var_dump() calls: removing them changes behaviour, so the rule stays silent. */
const SILENT: ReadonlyArray<[string, string]> = [
  ["guzzle's describe_type(), which returns the captured dump", GUZZLE],
  ["pest's dump() and dd() fallbacks", PEST],
  [
    "a test that asserts on what var_dump() prints, read with ob_get_clean() inside the assertion",
    php("function testDump($clone) {", "    ob_start();", "    var_dump($clone);", "    assertSame('x', ob_get_clean());", "}"),
  ],
  [
    "a dump read with ob_get_contents()",
    php("ob_start();", "var_dump($a, $b);", "$out = ob_get_contents();", "ob_end_clean();"),
  ],
  ["a free function named dd", php("function dd(...$vars): never {", "    var_dump(...$vars);", "    exit(1);", "}")],
  ["a method named DUMP, as PHP names are case-insensitive", php("class A {", "    function DUMP($v) { var_dump($v); }", "}")],
];

/** Calls the rule must still report. */
const REPORTED: ReadonlyArray<[string, string, number[]]> = [
  ["a bare var_dump()", php("var_dump($user);"), [2]],
  ["a var_dump() before output buffering starts", php("var_dump($a);", "ob_start();", "var_dump($b);", "$s = ob_get_clean();"), [2]],
  ["a var_dump() buffered but flushed to output, never read", php("ob_start();", "var_dump($a);", "ob_end_flush();"), [3]],
  ["a var_dump() after the buffer was read", php("ob_start();", "echo 'x';", "$s = ob_get_clean();", "var_dump($s);"), [5]],
  ["a var_dump() in a method whose name only starts with dump", php("class A {", "    function dumpAll($v) {", "        var_dump($v);", "    }", "}"), [4]],
  ["shell_exec() inside a dd() helper", php("function dd($cmd) {", "    return shell_exec($cmd);", "}"), [3]],
  ["shell_exec() between ob_start() and ob_get_clean()", php("ob_start();", "shell_exec('ls');", "$s = ob_get_clean();"), [3]],
  ["a backtick command", php("$out = `ls`;"), [2]],
];

describe(RULE_ID, () => {
  it.each(SILENT)("stays silent on %s", (_name, source) => {
    expect(reportedLines(source)).toEqual([]);
  });

  it.each(REPORTED)("reports %s", (_name, source, lines) => {
    expect(reportedLines(source)).toEqual(lines);
  });
});
