/**
 * bandit-b105-hardcoded-password-string and bandit-b107-hardcoded-password-default (packs/python/candidates/) must
 * report what Bandit 1.9.4's general_hardcoded_password plugin reports.
 *
 * B107 checks only `FunctionDef`: a lambda's or an `async def`'s parameter defaults are not reported (#267).
 *
 * B105 checks each `str` constant by its parent (#268, #269):
 * - Assign: a target is a candidate name or attribute; an annotated assignment is an AnnAssign and is not read.
 * - Compare: the left operand is a candidate name or attribute and the first comparator a `str` constant; every `str`
 *   comparator is then reported, at its own position. A string on the left, bytes, or a string after another
 *   comparator is not.
 * - Dict: the key is a candidate string and its value any constant (bytes and None included); the key is reported.
 * - Subscript: `d["password"] = "x"` reports the key when the assigned value is a `str` constant.
 *
 * Each case is one line, checked against `uvx bandit==1.9.4 -t B105,B107 -f json`. A B105 case lists Bandit's 0-based
 * `col_offset`s for the line; a B107 case says whether Bandit reports it.
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

const evaluationSchema = z.object({
  violations: z
    .array(z.object({ span: z.object({ startLine: z.number().int().nonnegative(), startColumn: z.number().int().nonnegative() }) }))
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

async function compile(ruleId: string): Promise<NativeCompiledCodeRuleDsl> {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = await readFile(repositoryPath("packs", "python", "candidates", "rules", `${ruleId}.json`), "utf8");
  return runtime.compile(document, "json", capabilities);
}

/** The sorted 0-based columns the rule reports on the first line of `source`. */
function reportedColumns(rule: NativeCompiledCodeRuleDsl, source: string): number[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path: "src/app.py", source: `${source}\n` }],
  });
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  expect(result.violations.every(violation => violation.span.startLine === 0)).toBe(true);
  return result.violations.map(violation => violation.span.startColumn).sort((a, b) => a - b);
}

/** Whether Bandit reports the line. Bandit anchors B107 at the `def`, the port at the parameter. */
const B107: readonly (readonly [string, boolean])[] = [
  // #267
  ['h = lambda url, secret="abc": url', false],
  ['async def g(token="abc"): ...', false],
  ['def f(password="hunter2"): ...', true],
  ['f = lambda password="x": None', false],
  ['def f(user, token="abc", *, secret="x"): ...', true],
];

const B105: readonly (readonly [string, number[]])[] = [
  // #268
  ['while b"\\n" not in token_bytes: ...', []],
  ['if "abc" == password: ...', []],
  ['if password == b"bytes": ...', []],
  ['if x == password == "zzz": ...', []],
  ['if password == "hunter2": ...', [15]],
  ['if password == "a" == "b": ...', [15, 22]],
  ['if password == x == "b": ...', []],
  ['if password == f"x": ...', []],
  ['if password == "a" "b": ...', [15]],
  ['if obj.password != "c": ...', [19]],
  ['if password in ("a",): ...', []],
  // #269
  ['cfg = {"password": "hunter2"}', [7]],
  ['cfg = {"password": b"bytes"}', [7]],
  ['cfg = {"password": None}', [7]],
  ['cfg = {"password": f"x{y}"}', []],
  ['cfg = {"my_token": "v", "other": "w"}', [7]],
  ['cfg = {"nope": "v", b"password": "v"}', []],
  ['cfg = {k: "v" for k in ["password"]}', []],
  ['d["password"] = "hunter2"', [2]],
  ['d["password"] = b"bytes"', []],
  ['d["password"] = 1', []],
  ['d.x["secret"] = "x"', [4]],
  ['a = d["password"] = "x"', [6]],
  // Assignments
  ['password = "hunter2"', [11]],
  ['self.password = "x"', [16]],
  ['password = b"bytes"', []],
  ['password = f"x"', []],
  ['password: str = "annotated"', []],
  ['password = "a" "b"', [11]],
  ['x = password = "chain"', [15]],
  ['password = x = "chain"', [15]],
];

describe("bandit-b107-hardcoded-password-default", async () => {
  const rule = await compile("bandit-b107-hardcoded-password-default");
  it.each(B107)("%s is reported: %s", (source, reported) => {
    expect(reportedColumns(rule, source).length > 0).toBe(reported);
  });
});

describe("bandit-b105-hardcoded-password-string", async () => {
  const rule = await compile("bandit-b105-hardcoded-password-string");
  it.each(B105)("%s reports columns %j", (source, columns) => {
    expect(reportedColumns(rule, source)).toEqual(columns);
  });
});
