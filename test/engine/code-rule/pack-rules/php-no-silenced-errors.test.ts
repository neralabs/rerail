/**
 * phpcs-generic-php-no-silenced-errors (packs/php/) must not steer an agent into removing an @ that the code needs
 *. PHP_CodeSniffer's sniff reports every @ operator and its message only says silencing is discouraged, so
 * an agent removes the @. Two idioms need it:
 *
 * - The race-safe directory or file call, `!@mkdir($d) && !is_dir($d)`: when another process creates the directory
 *   first, mkdir() warns, and the is_dir() re-check right after it is what decides whether that was a failure. Without
 *   the @, a lost race surfaces as a warning (an exception under most frameworks' error handlers).
 * - Symfony's deprecation convention, `@trigger_error('...', E_USER_DEPRECATED)`: the notice is silenced so that only
 *   error handlers that opt in log it; without the @, every deprecation reaches the default handler.
 *
 * Each case below is one source; the test asserts exactly which lines the rule reports.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../src/engine/native/native.js";
import { readRuleDocument } from "../../../../src/engine/catalog/rule-documents.js";
import { repositoryPath } from "../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "phpcs-generic-php-no-silenced-errors";
const RULE_PATH = repositoryPath("packs", "php", "rules", `${RULE_ID}.json`);

const evaluationSchema = z.object({
  violations: z
    .array(z.object({ clauseId: z.string(), path: z.string(), message: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) }))
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

function compile(): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = readRuleDocument(RULE_PATH);
  return runtime.compile(JSON.stringify(document.rule), "json", capabilities);
}

/** What the rule reports in `source`: each finding's 1-based line, the clause that reported it, and its message. */
function findings(compiled: NativeCompiledCodeRuleDsl, source: string): { line: number; clauseId: string; message: string }[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "php", path: "src/silenced.php", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations
    .map(violation => ({ line: violation.span.startLine + 1, clauseId: violation.clauseId, message: violation.message }))
    .sort((a, b) => a.line - b.line);
}

const php = (...lines: string[]) => ["<?php", ...lines, ""].join("\n");

const UPSTREAM_MESSAGE =
  "Remove the @ and handle the failure explicitly, by checking the return value or converting warnings with set_error_handler().";
const FILESYSTEM_CALL_MESSAGE =
  "Recheck the outcome right after this silenced call, as in `!@mkdir($dir) && !is_dir($dir)`, or drop the @ and check the result.";

/**
 * An @ whose failure nothing checks: the rule reports it on line 2, from the clause named. A silenced mkdir(), rmdir()
 * or unlink() with a first argument is the `filesystem-call` clause's; every other @ is the `upstream` clause's.
 */
const REPORTED: ReadonlyArray<[string, string, string]> = [
  ["a silenced file read", php("$data = @file_get_contents($path);"), "upstream"],
  ["a silenced mkdir with no re-check", php("if (!is_dir($d) && !@mkdir($d, 0o777, true)) { throw new RuntimeException(); }"), "filesystem-call"],
  ["a silenced mkdir whose result is only used as a condition", php("if (@mkdir($tmp = $dir . '.tmp')) { rename($tmp, $dir); }"), "filesystem-call"],
  ["a silenced mkdir re-checked against a different path", php("if (!@mkdir($a) && !is_dir($b)) { throw new RuntimeException(); }"), "filesystem-call"],
  ["a silenced unlink with no re-check", php("@unlink($file);"), "filesystem-call"],
  ["a silenced unlink with no argument", php("@unlink();"), "upstream"],
  ["a silenced unlink first-class callable", php("$remove = @unlink(...);"), "upstream"],
  [
    "a silenced unlink inside a closure whose call is re-checked",
    php("(function () use ($f) { @unlink($f); return true; })() || file_exists($f);"),
    "filesystem-call",
  ],
  ["a silenced unlink passed to a call that is re-checked", php("foo(@unlink($f)) || file_exists($f);"), "filesystem-call"],
  ["a silenced trigger_error that is not a deprecation", php("@trigger_error('Disk is full', E_USER_WARNING);"), "upstream"],
  ["a silenced trigger_error at the default level", php("@trigger_error('Something happened');"), "upstream"],
  ["a silenced trigger_error whose message is E_USER_DEPRECATED", php("@trigger_error(E_USER_DEPRECATED);"), "upstream"],
  ["a silenced call that merely mentions E_USER_DEPRECATED", php("@log_error('X is deprecated', E_USER_DEPRECATED);"), "upstream"],
  ["a silenced is_dir() followed by an is_dir() re-check (only mkdir/rmdir/unlink are exempt)", php("if (!@is_dir($d) && !is_dir($d)) { mkdir($d); }"), "upstream"],
];

/** An @ the code needs, followed by the check that decides the outcome: the rule stays silent. */
const SILENT: ReadonlyArray<[string, string]> = [
  [
    "the race-safe mkdir idiom from Laravel-Excel's TemporaryFileFactory",
    php("if (!is_dir($d) && !@mkdir($d) && !is_dir($d)) { throw new RuntimeException(); }"),
  ],
  ["mkdir re-checked with is_dir", php("if (!@mkdir($dir, 0o777, true) && !is_dir($dir)) {", "    throw new RuntimeException();", "}")],
  [
    "mkdir of a computed path re-checked with is_dir",
    php("if (!is_dir(\\dirname($path)) && !@mkdir(\\dirname($path), 0o777, true) && !is_dir(\\dirname($path))) {", "    return false;", "}"),
  ],
  ["mkdir of a $this property re-checked with is_dir", php("if (!is_dir($this->root) && !@mkdir($this->root, 0o777, true) && !is_dir($this->root)) {", "    throw new RuntimeException();", "}")],
  ["mkdir re-checked with ||", php("@mkdir($dir) || is_dir($dir) || fail();")],
  ["mkdir re-checked with a lower-case or", php("@mkdir($dir) or is_dir($dir) or fail();")],
  ["mkdir re-checked with a lower-case and", php("if (!@mkdir($dir) and !is_dir($dir)) { throw new RuntimeException(); }")],
  ["mkdir re-checked with an upper-case AND", php("if (!@mkdir($dir) AND !is_dir($dir)) { throw new RuntimeException(); }")],
  ["a fully qualified mkdir re-checked with a fully qualified is_dir", php("if (!@\\mkdir($dir) && !\\is_dir($dir)) { throw new RuntimeException(); }")],
  ["unlink re-checked with file_exists", php("$cleared = @unlink($this->file) || !file_exists($this->file);")],
  ["unlink re-checked with is_file", php("if (!@unlink($f) && is_file($f)) {}")],
  ["unlink re-checked inside a larger expression", php("$pruned = (@unlink($file) || !file_exists($file)) && $pruned;")],
  ["rmdir re-checked with is_dir", php("if (!@rmdir($dir) && is_dir($dir)) { throw new RuntimeException(); }")],
  ["Symfony's deprecation notice", php("@trigger_error('X is deprecated', E_USER_DEPRECATED);")],
  ["a deprecation notice built with sprintf and a fully qualified constant", php("@trigger_error(sprintf('The \"%s\" class is deprecated.', Foo::class), \\E_USER_DEPRECATED);")],
  ["a fully qualified trigger_error deprecation", php("@\\trigger_error('X is deprecated', E_USER_DEPRECATED);")],
  ["a deprecation notice with a named level", php("@trigger_error('X is deprecated', error_level: E_USER_DEPRECATED);")],
  ["a deprecation notice with both arguments named", php("@trigger_error(message: 'X is deprecated', error_level: E_USER_DEPRECATED);")],
  ["a user_error deprecation notice", php("@user_error('X is deprecated', E_USER_DEPRECATED);")],
];

describe(RULE_ID, () => {
  it.each(REPORTED)("reports %s", (_name, source, clauseId) => {
    expect(findings(compile(), source).map(finding => [finding.clauseId, finding.line])).toEqual([[clauseId, 2]]);
  });

  it.each(SILENT)("stays silent on %s", (_name, source) => {
    expect(findings(compile(), source)).toEqual([]);
  });

  it("still reports an unrelated @ inside the re-check idiom's condition", () => {
    const source = php("if (!@mkdir($d) && !is_dir($d)", "    && !@chmod($d, 0o755)) {", "    throw new RuntimeException();", "}");
    expect(findings(compile(), source).map(finding => [finding.clauseId, finding.line])).toEqual([["upstream", 3]]);
  });

  it("still reports other @ operators in a file with an unrelated syntax error", () => {
    // Only the re-check exemption reads past the @ expression, so only @mkdir, @rmdir and @unlink go unchecked there.
    const input = JSON.stringify({
      format: "rerail.repository-snapshot/v1",
      revision: 0,
      facts: [],
      files: [{ language: "php", path: "src/silenced.php", source: php("$data = @file_get_contents($path);", "@mkdir($d);", "function ( {") }],
    });
    const result = z
      .object({
        violations: z.array(z.object({ clauseId: z.string(), span: z.object({ startLine: z.number() }) })),
        failures: z.array(z.object({ clauseId: z.string(), code: z.string() })),
      })
      .parse(JSON.parse(compile().evaluate(input, "report")));
    expect(result.violations.map(violation => [violation.clauseId, violation.span.startLine + 1])).toEqual([["upstream", 2]]);
    expect(result.failures.map(failure => [failure.clauseId, failure.code]).sort()).toEqual([
      ["filesystem-call", "source-incomplete"],
      ["upstream", "source-partial"],
    ]);
  });

  it("names the alternatives to removing @ in its message, from either clause", () => {
    expect(findings(compile(), php("$data = @file_get_contents($path);"))).toEqual([{ line: 2, clauseId: "upstream", message: UPSTREAM_MESSAGE }]);
    expect(findings(compile(), php("@unlink($file);"))).toEqual([{ line: 2, clauseId: "filesystem-call", message: FILESYSTEM_CALL_MESSAGE }]);
  });
});
