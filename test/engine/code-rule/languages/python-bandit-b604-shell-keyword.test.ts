/**
 * bandit-b604-any-other-function-with-shell-equals-true (packs/python/candidates/) must read only a `shell=` keyword,
 * as Bandit 1.9.4's has_shell does. The port bound a call's second positional argument to `shell`, so
 * pytest.mark.parametrize, getattr, re.sub and any helper with a truthy second argument were flagged; and it needed a
 * known value, so `shell=<expression>` was missed. has_shell treats a `shell=` keyword as truthy unless it is a falsy
 * constant (False, None, a zero number, an empty string or bytes), an empty list or an empty dict.
 *
 * Each case below is one call on line 1; the test asserts whether the candidate rule reports it.
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
const RULE_ID = "bandit-b604-any-other-function-with-shell-equals-true";
const rule = await readFile(repositoryPath("packs", "python", "candidates", "rules", `${RULE_ID}.json`), "utf8");

const evaluationSchema = z.object({
  violations: z.array(z.object({ span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function compile(): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  return runtime.compile(rule, "json", capabilities);
}

/** The 1-based lines the rule reports in `source`. */
function reportedLines(source: string): number[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path: "src/app.py", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compile().evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.span.startLine + 1).sort((a, b) => a - b);
}

/** A `shell=` keyword has_shell reads as truthy: Bandit reports it. */
const REPORTED: readonly string[] = [
  "helper(cmd, shell=True)",
  "helper(cmd, shell=1)",
  'helper(cmd, shell="yes")',
  "helper(cmd, shell=use_shell)",
  "helper(cmd, shell=get())",
  'helper(cmd, shell=f"{x}")',
  "helper(cmd, shell=[1])",
  "helper(cmd, shell={'a': 1})",
  "helper(cmd, shell=(1, 2))",
  "os.popen2(cmd, shell=True)",
];

/** No `shell=` keyword, or one has_shell reads as falsy: Bandit stays silent. */
const SILENT: readonly string[] = [
  '@pytest.mark.parametrize("a", [1, 2])\ndef test_a(a): pass',
  'getattr(a, "b", True)',
  're.sub("a", "b", "c")',
  "helper(cmd, True)",
  "helper(cmd, shell=False)",
  "helper(cmd, shell=None)",
  "helper(cmd, shell=0)",
  "helper(cmd, shell=0.0)",
  'helper(cmd, shell="")',
  'helper(cmd, shell=b"")',
  "helper(cmd, shell=[])",
  "helper(cmd, shell={})",
  'helper(cmd, **{"shell": True})',
  "import subprocess\nsubprocess.Popen(cmd, shell=True)",
];

describe(RULE_ID, () => {
  it.each(REPORTED)("reports %s", source => {
    expect(reportedLines(source)).toEqual([1]);
  });

  it.each(SILENT)("stays silent on %s", source => {
    expect(reportedLines(source)).toEqual([]);
  });
});
