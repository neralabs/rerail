/**
 * bandit-b603-subprocess-without-shell-equals-true and bandit-b607-start-process-with-partial-path read Bandit's
 * `subprocess` function set: `subprocess.Popen`, `call`, `check_call`, `check_output` and `run`, resolved through
 * imports. B607 also reads `subprocess.getoutput` and `subprocess.getstatusoutput` from the `shell` set. Bandit reads
 * only a `shell=` keyword: a `**kw` argument, even a dict literal, or a positional ninth argument does not count as
 * passing it, and a `shell` argument Bandit cannot decide (a variable) counts as true. B607 reads the first positional
 * argument, and the first element when it is a list (not a tuple); a comment before that element is not one.
 *
 * Each case is one line of SNIPPET, except the last three, which open a list across two lines; the expected positions (1-based line, 0-based column) are what Bandit 1.9.4
 * reports on it (`bandit -t B603,B607`, its line number and col_offset).
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(
  new URL("../../../../../native/src/code_rule_dsl/", import.meta.url),
);
const capabilities = await readFile(
  join(CODE_RULE_DIRECTORY, "capabilities.json"),
  "utf8",
);
const runtime = loadNativeBinding().CodeRuleDslRuntime;
if (runtime === undefined)
  throw new Error("Native binding must export CodeRuleDslRuntime");

const evaluationSchema = z.object({
  violations: z
    .array(
      z.object({
        span: z.object({
          startLine: z.number().int().nonnegative(),
          startColumn: z.number().int().nonnegative(),
        }),
      }),
    )
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

const SNIPPET = [
  "import subprocess",
  "from subprocess import run as start, check_output",
  "import os",
  "",
  "subprocess.run(cmd)",
  "subprocess.check_output(cmd)",
  "subprocess.check_call(cmd)",
  "subprocess.call(cmd, **kw)",
  'subprocess.run(["git", "status"])',
  "subprocess.Popen(cmd)",
  "start(cmd, shell=False)",
  "check_output(cmd, shell=None)",
  "subprocess.run(cmd, shell=True)",
  "subprocess.run(cmd, shell=flag)",
  "subprocess.run(*args)",
  "subprocess.Popen(cmd, 0, None, None, None, None, None, True, True)",
  'subprocess.check_call("ls -l")',
  'subprocess.check_output(["/bin/ls", "-l"])',
  'subprocess.run(("git", "status"))',
  'subprocess.getoutput("ls")',
  'subprocess.run(args=["git"])',
  'os.system("ls")',
  "subprocess.run(cmd, shell=0)",
  "subprocess.Popen(cmd, **{'stdout': None})",
  'subprocess.run(cmd, shell="")',
  "subprocess.run(cmd, shell=[])",
  "flag = False",
  "subprocess.run(cmd, shell=flag)",
  'runner.run(["git", "status"])',
  "def check_call(command):",
  "    return command",
  'check_call("ls")',
  'subprocess.Popen(["ls"])',
  "subprocess.Popen([])",
  "subprocess.Popen(cmd, **{'shell': True})",
  "subprocess.Popen([  # comment",
  '    "ls", "-l"], 1)',
  "subprocess.run([  # comment",
  '    "ls"])',
  'subprocess.run(["ls",  # comment',
  '    "-l"])',
  "",
].join("\n");

/** The positions, as `line:column`, the candidate rule `ruleId` reports in SNIPPET, in source order. */
async function reportedPositions(ruleId: string): Promise<string[]> {
  const document = await readFile(
    repositoryPath("packs", "python", "candidates", "rules", `${ruleId}.json`),
    "utf8",
  );
  const rule = runtime!.compile(document, "json", capabilities);
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path: "app/run.py", source: SNIPPET }],
  });
  const result = evaluationSchema.parse(
    JSON.parse(rule.evaluate(input, "report")),
  );
  expect(result.failures).toEqual([]);
  return result.violations
    .map(({ span }) => span)
    .sort((a, b) => a.startLine - b.startLine || a.startColumn - b.startColumn)
    .map((span) => `${span.startLine + 1}:${span.startColumn}`);
}

describe("Bandit's subprocess function set", () => {
  it("bandit-b603-subprocess-without-shell-equals-true reports every subprocess call without a true shell keyword", async () => {
    const lines = [
      5, 6, 7, 8, 9, 10, 11, 12, 15, 16, 17, 18, 19, 21, 23, 24, 25, 26, 33, 34,
      35, 36, 38, 40,
    ];
    expect(
      await reportedPositions(
        "bandit-b603-subprocess-without-shell-equals-true",
      ),
    ).toEqual(lines.map((line) => `${line}:0`));
  });

  it("bandit-b607-start-process-with-partial-path reports a bare program name, alone or first in a list", async () => {
    expect(
      await reportedPositions("bandit-b607-start-process-with-partial-path"),
    ).toEqual(["9:0", "17:0", "20:0", "22:0", "33:0", "36:0", "38:0", "40:0"]);
  });
});
