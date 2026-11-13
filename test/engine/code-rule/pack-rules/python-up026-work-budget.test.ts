/**
 * ruff-up026-deprecated-mock-import (packs/python/) must check a long file within its work budget. Its string branch
 * reports `mock.mock` inside a string annotation only where a `mock` import is in the module, and the engine runs a
 * rule's `all` before its `matches`: written as siblings, every node in the file searched the module for that import
 * before the string check turned it away, so a file of a few hundred lines ran out of budget and lost its findings.
 * The string check now runs first, inside `all`.
 *
 * Diagnostic locations and alias behavior match Ruff 0.16.9 (UP026).
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../../src/engine/native/native.js";

const capabilities = readFileSync(new URL("../../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const rule = readFileSync(new URL("../../../../packs/python/rules/ruff-up026-deprecated-mock-import.json", import.meta.url), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
if (runtime === undefined) throw new Error("CodeRuleDslRuntime unavailable");
const compiled = runtime.compile(rule, "json", capabilities);

function check(source: string) {
  const result = JSON.parse(compiled.evaluate(JSON.stringify({
    format: "rerail.repository-snapshot/v1", revision: 0, facts: [],
    files: [{ language: "python", path: "example.py", source }],
  }), "report")) as {
    status: string;
    failures: unknown[];
    violations: { code: string; span: { startLine: number; startColumn: number } }[];
  };
  expect(result.status).toBe("completed");
  expect(result.failures).toEqual([]);
  return result.violations.map(v => [v.code, v.span.startLine + 1, v.span.startColumn + 1]).sort();
}

describe("UP026 annotation matching stays within its work budget", () => {
  it.each([500, 5_000])("finds both violations after %i ordinary calls", count => {
    expect(check("row.get('title')\n".repeat(count) + "import mock\nmock.mock.Mock()\n"))
      .toEqual([["UP026", count + 1, 8], ["UP026", count + 2, 1]]);
  });

  it.each(["", "from unittest import mock\nmock.mock.Mock()\n"])("completes a long clean file with suffix %j", suffix => {
    expect(check("row.get('title')\n".repeat(5_000) + suffix)).toEqual([]);
  });

  it.each([
    ["import mock\nx: \"mock.mock.Mock\"\n", [["UP026", 501, 8], ["UP026", 502, 5]]],
    ["import mock as m\nx: \"m.mock.Mock\"\n", [["UP026", 501, 8]]],
    ["import mock\nx = \"mock.mock.Mock\"\n", [["UP026", 501, 8]]],
    ["import mock  # noqa: UP026\nmock.mock.Mock()  # noqa: UP026\n", []],
  ])("preserves annotation and suppression behavior for %s", (suffix, expected) => {
    expect(check("row.get('title')\n".repeat(500) + suffix)).toEqual(expected);
  });
});
