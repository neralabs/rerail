/**
 * phpcs-squiz-php-disallow-size-functions-in-loops (packs/php/) must not steer an agent into breaking a loop
 *. PHP_CodeSniffer's sniff reports every count(), sizeof() or strlen() in a loop condition and advises
 * hoisting it before the loop. That is only correct when the loop leaves the measured value alone. When the loop
 * grows, shrinks or replaces it (a work queue drained with array_shift, a buffer filled with .=, a fixed-point loop
 * that runs until a collection stops growing), the size has to be read on every pass, and an agent that follows the
 * steer changes what the loop does. Of the sniff's 48 findings on the pack's pinned Symfony and Laravel corpora, 21 are
 * such loops; packs/php/divergences.json records each one the pack checks can score.
 *
 * Each case below is one loop; the test asserts exactly which lines the rule reports.
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
const RULE_ID = "phpcs-squiz-php-disallow-size-functions-in-loops";
const RULE_PATH = repositoryPath("packs", "php", "rules", `${RULE_ID}.json`);

const evaluationSchema = z.object({
  violations: z.array(z.object({ path: z.string(), message: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function compile(): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = readRuleDocument(RULE_PATH);
  return runtime.compile(JSON.stringify(document.rule), "json", capabilities);
}

/** The 1-based lines the rule reports in `source`, each with its message. */
function findings(compiled: NativeCompiledCodeRuleDsl, source: string): { line: number; message: string }[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "php", path: "src/loop.php", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => ({ line: violation.span.startLine + 1, message: violation.message })).sort((a, b) => a.line - b.line);
}

const php = (...lines: string[]) => ["<?php", ...lines, ""].join("\n");

/** Loops that leave the measured value alone: hoisting the size is safe, so the rule reports it (line 2 of each source). */
const REPORTED: ReadonlyArray<[string, string]> = [
  ["a for loop over an array it only reads", php("for ($i = 0; $i < count($items); $i++) {", "    echo $items[$i];", "}")],
  ["a while loop over a string it only reads", php("while ($pos < strlen($text)) {", "    $pos++;", "}")],
  ["sizeof() as well as count()", php("while ($i < sizeof($rows)) {", "    $i++;", "}")],
  ["a loop that writes a different variable", php("while (count($a) > $n) {", "    $b[] = 1;", "    $n++;", "}")],
  ["count() with a mode argument", php("for ($i = 0; $i < count($tree, COUNT_RECURSIVE); $i++) {", "    visit($i);", "}")],
  ["a $this property the loop only reads, with no method call on $this", php("for ($i = 0; $i < count($this->items); $i++) {", "    echo format($this->items[$i]);", "}")],
  ["a loop that changes another array than the one it counts", php("for ($i = 1; $i < count($lines) - 1; ++$i) {", "    array_splice($this->lines, $i, 0, [$lines[$i]]);", "}")],
  ["a value set once in the for initializer, which runs before the loop", php("for ($items = loadItems(), $i = 0; $i < count($items); ++$i) {", "    echo $items[$i];", "}")],
  ["a loop whose closure assigns its own parameter of the same name", php("for ($i = 0; $i < count($items); ++$i) {", "    $f = function ($items) { $items = []; };", "}")],
  ["a comment before the argument", php("for ($i = 0; $i < count(/* items to visit */ $items); ++$i) {", "    echo $items[$i];", "}")],
  [
    "an inner loop that only reads, inside an outer loop that changes the value",
    php("for (; strlen($this->currentLine) > $cursor; ++$cursor) {", "    $value .= $this->currentLine[$cursor];", "}"),
  ],
];

/** The inner loop above, wrapped in an outer loop that moves to the next line: only the inner condition is judged. */
const NESTED = php(
  "do {",
  "    for (; strlen($this->currentLine) > $cursor; ++$cursor) {",
  "        $value .= $this->currentLine[$cursor];",
  "    }",
  "    $lines[] = $value;",
  "} while ($this->moveToNextLine() && count($lines) < $max);",
);

/** Loops that change the measured value: the size must be read on every pass, so the rule stays silent. */
const SILENT: ReadonlyArray<[string, string]> = [
  [
    "a fixed-point loop that runs until the collection stops growing",
    php("do {", "    $before = count($defs);", "    foreach ($defs as $d) {", "        $defs += expand($d);", "    }", "} while (count($defs) !== $before);"),
  ],
  ["a work queue drained with array_shift", php("while (count($queue) > 0) {", "    run(array_shift($queue));", "}")],
  ["a batch filled with []=", php("while (count($out) < $size && null !== $next = fetch()) {", "    $out[] = $next;", "}")],
  ["a list replaced on each pass", php("while (count($nodes) > 1) {", "    $nodes = mergePairs($nodes);", "}")],
  ["a buffer grown with .=", php("while (strlen($buf) < $size) {", "    $buf .= fread($fh, 1024);", "}")],
  ["a pool shrunk with unset", php("while (count($pool) > $max) {", "    unset($pool[array_key_first($pool)]);", "}")],
  ["a list grown with array_push", php("for ($i = 0; $i < count($list); $i++) {", "    array_push($list, ...expand($list[$i]));", "}")],
  ["a list rewritten with array_splice", php("for ($i = 0; $i < count($attributes); ++$i) {", "    array_splice($attributes, $i--, 1, $replacement);", "}")],
  ["a $this property changed through a method on $this", php("while ($more && strlen($this->currentLine) <= $indent) {", "    $more = $this->moveToNextLine();", "}")],
  ["a $this property appended through the property", php("while (count($this->stack) > 0) {", "    array_pop($this->stack);", "}")],
  ["a buffer grown with .= and measured through a named argument", php("while (strlen(string: $buf) < 10) {", "    $buf .= 'x';", "}")],
  ["a list changed through array_push with a comment before its argument", php("for ($i = 0; $i < count($list); $i++) {", "    array_push(/* grow */ $list, ...expand($list[$i]));", "}")],
];

describe(RULE_ID, () => {
  it.each(REPORTED)("reports %s", (_name, source) => {
    const reported = findings(compile(), source);
    expect(reported.map(finding => finding.line)).toEqual([2]);
  });

  it.each(SILENT)("stays silent on %s", (_name, source) => {
    expect(findings(compile(), source)).toEqual([]);
  });

  it("judges each size call by its own loop, not by the loops around it", () => {
    expect(findings(compile(), NESTED).map(finding => finding.line)).toEqual([3]);
  });

  it("reports only the sizes the loop leaves alone when one condition measures several values", () => {
    const source = php("while (count($values) && count($temp) < count($subarray)) {", "    $temp[] = array_shift($values);", "}");
    const reported = findings(compile(), source);
    expect(reported).toHaveLength(1);
    expect(reported[0]?.message).toContain("$subarray");
  });

  it("names the measured value and the safe rewrite in its message", () => {
    const [finding] = findings(compile(), REPORTED[0]![1]);
    expect(finding?.message).toBe(
      "Assign the size of $items to a variable before the loop and use that in the condition; as written it is recomputed on every pass.",
    );
  });

  it("names the value, not the label, of a named argument", () => {
    const [finding] = findings(compile(), php("while (strlen(string: $text) > $pos) {", "    $pos++;", "}"));
    expect(finding?.message).toContain("Assign the size of $text to a variable");
  });
});
