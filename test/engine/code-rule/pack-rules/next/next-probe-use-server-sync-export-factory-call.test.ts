/**
 * next-probe-use-server-sync-export must not report an export of a "use server" file whose value is a call result.
 *
 * Next.js 16's server actions transform rejects, at compile time, an exported literal and an exported function or
 * arrow that is not async. Whether a value computed by a call, such as next-safe-action's
 * `action.metadata(…).action(async …)`, is a function is checked only at runtime, where any function passes, so the
 * export cannot be judged statically.
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
const RULE_ID = "next-probe-use-server-sync-export";
const document = await readFile(repositoryPath("packs", "next", "candidates", "probes", "rules", `${RULE_ID}.json`), "utf8");

const evaluationSchema = z.object({
  violations: z
    .array(z.object({ span: z.object({ startLine: z.number().int().nonnegative(), startColumn: z.number().int().nonnegative() }) }))
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const rule = runtime.compile(document, "json", capabilities);

/** The sorted 1-based line:column positions the rule reports in `source`. */
function reported(source: string): string[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "typescript", path: "app/actions.ts", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations
    .map(violation => [violation.span.startLine + 1, violation.span.startColumn + 1] as const)
    .sort((a, b) => a[0] - b[0] || a[1] - b[1])
    .map(([line, column]) => `${line}:${column}`);
}

const lines = (...source: string[]) => [...source, ""].join("\n");

describe(RULE_ID, () => {
  it("does not report an action built by a factory call, and still reports a literal and a sync function (issue repro)", () => {
    const source = lines(
      '"use server";',
      'export const doIt = action.metadata({ name: "x" }).action(async () => {});',
      'export const VERSION = "1";',
      "export function reset() {}",
    );
    expect(reported(source)).toEqual(["3:14", "4:17"]);
  });

  it("does not report other call results, whatever the callee is named", () => {
    const source = lines(
      '"use server";',
      "export const save = createSafeAction(schema, async (input: Input) => input);",
      "export const remove = wrap(handler);",
      "export const load = makeLoader()();",
    );
    expect(reported(source)).toEqual([]);
  });

  it("still does not report an async function, arrow or function expression", () => {
    const source = lines(
      '"use server";',
      "export async function save() {}",
      "export const archive = async (id: string) => id;",
      "export const restore = async function () {};",
    );
    expect(reported(source)).toEqual([]);
  });

  it("still reports a sync arrow, a sync function expression and a number", () => {
    const source = lines(
      '"use server";',
      "export const archive = (id: string) => id;",
      "export const restore = function () {};",
      "export const LIMIT = 1;",
    );
    expect(reported(source)).toEqual(["2:14", "3:14", "4:14"]);
  });

  it("does not report a call result outside a \"use server\" file", () => {
    expect(reported(lines("export const doIt = action(async () => {});", "export const VERSION = 1;"))).toEqual([]);
  });
});
