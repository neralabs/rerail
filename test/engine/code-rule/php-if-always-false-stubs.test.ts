/**
 * phpstan-if-always-false (packs/php/, candidate) must not steer an agent into deleting a declaration stub (NER-1810).
 * `if (false) { class Router {} }` is a deliberate idiom: the class never exists at runtime (a class_alias call
 * declares the name), but Composer's class map, IDEs and static analysers see a declaration for it. Symfony ships it
 * for every deprecated alias (src/Symfony/Bundle/FrameworkBundle/Routing/Router.php:20 and 23 more on the pack's
 * pinned corpus) and monolog for DateTimeImmutable; removing such a block broke Symfony before (symfony#66188).
 * PHPStan's if.alwaysFalse reports these too; the port does not, and packs/php/divergences.json records each one.
 *
 * Each case below is one file; the test asserts exactly which lines the rule reports.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../src/engine/native/native.js";
import { readRuleDocument } from "../../../src/engine/catalog/bundled-rule-documents.js";
import { repositoryPath } from "../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "phpstan-if-always-false";
const RULE_PATH = repositoryPath("packs", "php", "candidates", "rules", `${RULE_ID}.json`);

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
    files: [{ language: "php", path: "src/stub.php", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.span.startLine + 1).sort((a, b) => a - b);
}

const php = (...lines: string[]) => ["<?php", ...lines, ""].join("\n");

/** Declaration-only stubs: deleting them removes what the class map and IDEs read, so the rule stays silent. */
const SILENT: ReadonlyArray<[string, string]> = [
  [
    "Symfony's deprecation stub (FrameworkBundle/Routing/Router.php)",
    php(
      "namespace Symfony\\Bundle\\FrameworkBundle\\Routing;",
      "",
      "use Symfony\\Component\\Routing\\DependencyInjection\\Router as BaseRouter;",
      "",
      "class_alias(BaseRouter::class, 'Symfony\\Bundle\\FrameworkBundle\\Routing\\Router');",
      "",
      "if (false) {",
      "    /**",
      "     * @deprecated since Symfony 8.2, use Symfony\\Component\\Routing\\DependencyInjection\\Router instead",
      "     */",
      "    class Router",
      "    {",
      "    }",
      "}",
    ),
  ],
  ["the ticket's one-line stub", php("if (false) {", "    class Router extends BaseRouter {}", "}")],
  [
    "monolog's DateTimeImmutable stub",
    php(
      "namespace Monolog;",
      "",
      "class_alias(JsonSerializableDateTimeImmutable::class, 'Monolog\\DateTimeImmutable');",
      "",
      "if (false) {",
      "    /** @deprecated Use \\Monolog\\JsonSerializableDateTimeImmutable instead. */",
      "    class DateTimeImmutable extends JsonSerializableDateTimeImmutable",
      "    {",
      "    }",
      "}",
    ),
  ],
  ["an interface stub", php("if (false) {", "    interface Listener {}", "}")],
  ["a trait stub", php("if (false) {", "    trait Helpers {}", "}")],
  ["an enum stub", php("if (false) {", "    enum Suit: string {}", "}")],
  ["a function stub", php("if (false) {", "    function legacy_helper(): void {}", "}")],
  ["a class_alias call inside the block", php("if (false) {", "    \\class_alias(New_::class, Old::class);", "}")],
  ["several declarations together", php("if (false) {", "    // IDE helpers", "    class A {}", "    interface B {}", "    class_alias(A::class, 'C');", "}")],
  ["a stub guarded by 0", php("if (0) {", "    class Router {}", "}")],
];

/** Dead code that is not a stub: the rule still reports the condition, on the line given. */
const REPORTED: ReadonlyArray<[string, string, number]> = [
  ["a statement body", php("if (false) {", "    doSomething();", "}"), 2],
  ["a declaration next to a statement", php("if (false) {", "    class A {}", "    run();", "}"), 2],
  ["a yield in a dead branch (Symfony's services_uninitialized_ref fixture)", php("function f() {", "    if (false) {", "        yield 'foo2' => null;", "    }", "}"), 3],
  ["an empty body", php("if (false) {", "}"), 2],
  ["a body of comments only", php("if (false) {", "    // nothing here", "}"), 2],
  ["a call to another function than class_alias", php("if (false) {", "    class_exists(A::class);", "}"), 2],
  ["a constant condition followed through a variable", php("$debug = false;", "if ($debug) {", "    log_it();", "}"), 3],
];

describe(RULE_ID, () => {
  it.each(SILENT)("stays silent on %s", (_name, source) => {
    expect(reportedLines(compile(), source)).toEqual([]);
  });

  it.each(REPORTED)("reports %s", (_name, source, line) => {
    expect(reportedLines(compile(), source)).toEqual([line]);
  });

  it("judges each if by its own body, not the stub beside it", () => {
    const source = php("if (false) {", "    class A {}", "}", "if (false) {", "    run();", "}");
    expect(reportedLines(compile(), source)).toEqual([5]);
  });
});
