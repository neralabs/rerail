/**
 * bandit-b101-assert-used (on by default) must not report test code (NER-1944). Bandit's default configuration skips
 * no file, so its B101 reports every pytest `assert`, where `assert` is the idiom; Bandit users skip B101 for tests.
 * The port excludes `tests/` directories and `test_*.py`, `*_test.py` and `conftest.py` files at any depth, and still
 * reports every other file, including a library's own `test/` package (django/test/), a Django app's `tests.py` and
 * names that only resemble a test file.
 *
 * Each case is one path holding the same assert on line 2; the test asserts whether the shipped rule reports it.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../../src/engine/native/native.js";
import { loadBundledRules } from "../../../../src/engine/catalog/bundled-rule-documents.js";
import { repositoryPath } from "../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "bandit-b101-assert-used";

const evaluationSchema = z.object({
  violations: z.array(z.object({ path: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const document = loadBundledRules([repositoryPath("packs", "python")]).find(candidate => candidate.rule.id === RULE_ID);
if (document === undefined) throw new Error(`${RULE_ID} is not in packs/python`);
const rule = runtime.compile(JSON.stringify(document.rule), "json", capabilities);

/** The sorted 1-based lines the rule reports in a file at `path` holding one assert on line 2. */
function reportedLines(path: string): number[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path, source: "def check(user):\n    assert user.is_admin\n" }],
  });
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.span.startLine + 1).sort((a, b) => a - b);
}

/** Test code, where `assert` is the idiom: not reported. */
const SKIPPED: readonly string[] = [
  "tests/test_users.py",
  "tests/helpers.py",
  "tests/unit/models/factories.py",
  "pkg/tests/test_users.py",
  "src/pkg/tests/fixtures/data.py",
  "test_users.py",
  "src/pkg/test_users.py",
  "users_test.py",
  "src/pkg/users_test.py",
  "conftest.py",
  "tests/conftest.py",
  "src/pkg/conftest.py",
];

/** Source code, including names that only resemble test code: reported. */
const REPORTED: readonly string[] = [
  "app/checks.py",
  "src/pkg/users.py",
  "app/tests.py",
  "django/test/client.py",
  "src/testing_utils.py",
  "src/pkg/testing/helpers.py",
  "src/tests_helpers.py",
  "src/pkg/contest.py",
  "src/pkg/latest.py",
  "src/pkg/attest_users.py",
  "src/protests/views.py",
];

describe(RULE_ID, () => {
  it.each(SKIPPED)("skips %s", path => {
    expect(reportedLines(path)).toEqual([]);
  });
  it.each(REPORTED)("reports %s", path => {
    expect(reportedLines(path)).toEqual([2]);
  });
});
