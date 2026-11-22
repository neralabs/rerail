/**
 * bandit-b108-hardcoded-tmp-directory must report a temp directory in the literal parts of an f-string (#252).
 *
 * Bandit 1.9.4's B108 (bandit/plugins/general_hardcoded_tmp.py) checks every str constant. CPython 3.12 and later parse
 * an f-string into constants for its literal runs, each placed where its text starts, so Bandit reports a run that
 * starts with /tmp, /var/tmp or /dev/shm: at the start of an f-string or after a replacement field. In an implicit
 * concatenation with a replacement field, adjacent literal text merges into one run: a plain piece that starts a run
 * is reported at its quote. A bare f-string statement is not a docstring to Bandit. The port read only whole constant
 * strings, so `f"/tmp/{name}.log"` was missed. Each case's expected positions are what Bandit 1.9.4 reports on it under
 * CPython 3.13 (line, then its column plus one).
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "bandit-b108-hardcoded-tmp-directory";
const document = await readFile(repositoryPath("packs", "python", "candidates", "rules", `${RULE_ID}.json`), "utf8");

const evaluationSchema = z.object({
  violations: z
    .array(z.object({ span: z.object({ startLine: z.number().int().nonnegative(), startColumn: z.number().int().nonnegative() }) }))
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const rule = runtime.compile(document, "json", capabilities);

/** The `line:column` (both 1-based) of each violation the rule reports in `source`, in source order. */
function reported(source: string): string[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path: "app/paths.py", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations
    .map(violation => [violation.span.startLine + 1, violation.span.startColumn + 1] as const)
    .sort(([lineA, columnA], [lineB, columnB]) => lineA - lineB || columnA - columnB)
    .map(([line, column]) => `${line}:${column}`);
}

const cases: readonly { name: string; source: string; expected: readonly string[] }[] = [
  { name: "an f-string that starts with /tmp", source: 'p = f"/tmp/{name}.log"\n', expected: ["1:7"] },
  {
    name: "every prefix spelling and temp directory",
    source: 'a = F"/var/tmp/{n}"\nb = rf"/dev/shm/{n}"\nc = f"""/tmp/{n}"""\nd = f"/tmpfoo"\n',
    expected: ["1:7", "2:8", "3:9", "4:7"],
  },
  {
    name: "a literal part after a replacement field",
    source: 'q = f"{name}/tmp/x"\nu = f"/opt/{name}/tmp"\ni = f"x{n:>{w}}/tmp"\n',
    expected: ["1:13", "2:18", "3:16"],
  },
  { name: "a bare f-string statement", source: 'f"/tmp/{name}"\n', expected: ["1:3"] },
  {
    name: "a concatenation with a replacement field",
    source:
      'a = "/tmp/" f"{n}"\nb = f"{n}" "/tmp/x"\nd = f"{a}/tmp" "x"\ne = "a" f"{b}/tmp{c}"\n' +
      'f1 = f"{a}" f"/tmp/{b}"\ng = f"/tmp/{a}" "x"\n',
    expected: ["1:5", "2:12", "3:10", "4:14", "5:15", "6:7"],
  },
  {
    name: "plain strings and docstrings, as before",
    source: 'x = "/tmp/abc"\ny = "/var/" "tmp"\n"""/tmp docstring"""\nz = "/abc/tmp"\n',
    expected: ["1:5", "2:5"],
  },
  {
    name: "f-string text that does not start a run with a temp directory",
    source:
      'z = f"/home/{name}"\nc = "abc" f"/tmp/{n}"\nj = f"{{/tmp"\nk = f"a\\n/tmp{x}"\nm = f"/opt/tmp{x}"\n',
    expected: [],
  },
];

describe(RULE_ID, () => {
  it.each(cases)("$name", ({ source, expected }) => {
    expect(reported(source)).toEqual(expected);
  });
});
