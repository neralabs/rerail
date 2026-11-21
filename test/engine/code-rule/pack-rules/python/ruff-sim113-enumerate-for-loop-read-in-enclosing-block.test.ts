/**
 * ruff-sim113-enumerate-for-loop must not report a counter that is read after its loop in a block enclosing the one
 * that holds the counter and the loop. Ruff counts every load of the counter in its scope that starts at or after the
 * initial assignment and lies outside the loop, wherever it sits; the port found such a load only among the statements
 * that follow the initial assignment in its own block, so a read after an enclosing `if`, `for` or `with` was missed
 * and the increment was reported.
 *
 * Each case is one function; the expected lines are what Ruff 0.16.9 reports on it (`ruff check --isolated
 * --select SIM113`).
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../../../src/engine/native/native.js";

const capabilities = readFileSync(new URL("../../../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const rule = readFileSync(
  new URL("../../../../../packs/python/rules/ruff-sim113-enumerate-for-loop.json", import.meta.url),
  "utf8",
);
const runtime = loadNativeBinding().CodeRuleDslRuntime;
if (runtime === undefined) throw new Error("CodeRuleDslRuntime unavailable");
const compiled = runtime.compile(rule, "json", capabilities);

/** The sorted 1-based lines SIM113 reports in `source`. */
function reportedLines(source: string): number[] {
  const result = JSON.parse(compiled.evaluate(JSON.stringify({
    format: "rerail.repository-snapshot/v1", revision: 0, facts: [],
    files: [{ language: "python", path: "example.py", source }],
  }), "report")) as {
    status: string;
    failures: unknown[];
    violations: { code: string; span: { startLine: number } }[];
  };
  expect(result.status).toBe("completed");
  expect(result.failures).toEqual([]);
  return result.violations.map(v => v.span.startLine + 1).sort((a, b) => a - b);
}

/** The counter is read after the loop, outside the block that holds it: Ruff reports nothing. */
const SILENT: readonly (readonly [string, string])[] = [
  ["read after an enclosing if", `def run(rows, ready):
    if ready:
        seen = 0
        for row in rows:
            print(row)
            seen += 1
    if seen > 0:
        print("some")
`],
  ["returned after an enclosing for", `def run(groups):
    for group in groups:
        seen = 0
        for row in group:
            print(row)
            seen += 1
    return seen
`],
  ["read after two enclosing blocks", `def run(rows, ready):
    with open("log") as handle:
        if ready:
            seen = 0
            for row in rows:
                print(row)
                seen += 1
    print(seen)
`],
  ["read in the else branch", `def run(rows, ready):
    if ready:
        seen = 0
        for row in rows:
            print(row)
            seen += 1
    else:
        print(seen)
`],
  ["read in an elif condition", `def run(rows, ready):
    if ready:
        seen = 0
        for row in rows:
            print(row)
            seen += 1
    elif seen:
        pass
`],
  ["read in a nested function defined after", `def run(rows, ready):
    if ready:
        seen = 0
        for row in rows:
            print(row)
            seen += 1
    def show():
        return seen
    return show
`],
  ["read after an enclosing if at module level", `if ready:
    seen = 0
    for row in rows:
        print(row)
        seen += 1
print(seen)
`],
  ["read after the loop in the same block", `def run(rows):
    seen = 0
    for row in rows:
        print(row)
        seen += 1
    print(seen)
`],
];

/** No load of the counter's initial or loop binding follows the loop: Ruff reports the increment. */
const REPORTED: readonly (readonly [string, string, number])[] = [
  ["no read at all", `def run(rows, ready):
    if ready:
        seen = 0
        for row in rows:
            print(row)
            seen += 1
    print("done")
`, 6],
  ["read only before the initial assignment", `def run(rows, ready):
    print(seen)
    if ready:
        seen = 0
        for row in rows:
            print(row)
            seen += 1
`, 7],
  ["rebound before the read in the enclosing block", `def run(rows, ready):
    if ready:
        seen = 0
        for row in rows:
            print(row)
            seen += 1
    seen = 5
    print(seen)
`, 6],
  ["another name read in the enclosing block", `def run(rows, ready):
    if ready:
        seen = 0
        for row in rows:
            print(row)
            seen += 1
    print(other)
`, 6],
];

describe("ruff-sim113-enumerate-for-loop: a read in an enclosing block", () => {
  it.each(SILENT)("is silent when the counter is %s", (_name, source) => {
    expect(reportedLines(source)).toEqual([]);
  });

  it.each(REPORTED)("reports the increment with %s", (_name, source, line) => {
    expect(reportedLines(source)).toEqual([line]);
  });
});

/**
 * The search for a later read walks the read's ancestors and, before each, the earlier statements and the blocks and
 * clauses inside them, never into expressions; long files stay in budget.
 */
describe("ruff-sim113-enumerate-for-loop: a read in an enclosing block, in a long file", () => {
  const reported = REPORTED[0]![1];
  const silent = SILENT[0]![1];

  it("keeps its answers after 5,000 statements", () => {
    const prefix = "row.get('title')\n".repeat(5_000);
    expect(reportedLines(prefix + reported + silent)).toEqual([5_006]);
  });

  it("keeps its answers after 500 earlier uses of the counter's name in the function", () => {
    const uses = "    print(seen)\n    seen = None\n".repeat(250);
    const withUses = (source: string) => source.replace("\n    if ready:", `\n${uses}    if ready:`);
    expect(reportedLines(withUses(reported) + withUses(silent))).toEqual([506]);
  });

  it("checks 300 loops, each with its own counter", () => {
    const loop = (index: number) => `    seen${index} = 0\n    for row in rows:\n        seen${index} += 1\n`;
    const body = Array.from({ length: 300 }, (_, index) => loop(index)).join("");
    expect(reportedLines(`def run(rows):\n${body}`)).toEqual(Array.from({ length: 300 }, (_, index) => 4 + 3 * index));
  });

  it("checks 100 loops that reuse one counter name, each read after an enclosing if", () => {
    const loop = "if ready:\n    seen = 0\n    for row in rows:\n        print(row)\n        seen += 1\nprint(seen)\n";
    expect(reportedLines(loop.repeat(100))).toEqual([]);
  });
});
