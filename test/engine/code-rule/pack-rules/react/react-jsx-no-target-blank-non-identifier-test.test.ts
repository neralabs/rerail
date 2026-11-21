/**
 * react-jsx-no-target-blank must treat a `target` and a `rel` conditional whose tests are both not identifiers as
 * testing the same thing, as upstream does (#261).
 *
 * eslint-plugin-react 7.37.5's getStringFromValue compares `target.expression.test.name` with
 * `rel.expression.test.name`. Only an Identifier has a `name`: any other test (a call, a member access, a negation)
 * reads `undefined`, so two such tests compare equal, even when they differ. Upstream then reads the `rel` branch on
 * the side where `target` is exactly `_blank`, and only that branch must hold `noreferrer`. An identifier test beside
 * a non-identifier one never compares equal, so both `rel` branches must hold `noreferrer`.
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
const RULE_ID = "react-jsx-no-target-blank";
const document = await readFile(repositoryPath("packs", "react", "rules", `${RULE_ID}.json`), "utf8");

const evaluationSchema = z.object({
  violations: z.array(z.object({ span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const rule = runtime.compile(document, "json", capabilities);

/** The sorted 1-based lines the rule reports in `source`. */
function reportedLines(source: string, language = "tsx", path = "src/link.tsx"): number[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language, path, source }],
  });
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.span.startLine + 1).sort((a, b) => a - b);
}

/** One `<a>` per line, with an expression `href` and the given `target` and `rel` expressions. */
const links = (...pairs: Array<[target: string, rel: string]>) =>
  pairs.map(([target, rel], index) => `const a${index} = <a href={href} target={${target}} rel={${rel}}>x</a>;`).join("\n") + "\n";

describe(RULE_ID, () => {
  it("does not report target and rel that share a call or member test, with noreferrer on the _blank side (issue repro)", () => {
    const source = links(
      ['href.startsWith("http") ? "_blank" : undefined', 'href.startsWith("http") ? "noopener noreferrer" : undefined'],
      ['link.external ? "_blank" : undefined', 'link.external ? "noreferrer" : undefined'],
      ['f() ? undefined : "_blank"', 'f() ? undefined : "noreferrer"'],
      ['(f()) ? "_blank" : undefined', '!ok ? "noreferrer" : undefined'],
      ['a() ? "_blank" : undefined', 'b.c ? "noreferrer" : undefined'],
    );
    expect(reportedLines(source)).toEqual([]);
  });

  it("does not report target and rel that share an identifier test, which was already silent", () => {
    expect(reportedLines(links(['isExternal ? "_blank" : undefined', 'isExternal ? "noopener noreferrer" : undefined']))).toEqual([]);
  });

  it("reports a shared non-identifier test whose rel holds noreferrer only on the side target is not _blank", () => {
    expect(reportedLines(links(['f() ? "_blank" : undefined', 'f() ? undefined : "noreferrer"']))).toEqual([1]);
  });

  it("reports a shared non-identifier test whose target is _blank in another case, though both rel branches hold noreferrer", () => {
    expect(reportedLines(links(['f() ? "_BLANK" : undefined', 'f() ? "noreferrer" : "noreferrer"']))).toEqual([1]);
  });

  it("reports an identifier test beside a non-identifier test unless both rel branches hold noreferrer", () => {
    const source = links(
      ['isExternal ? "_blank" : undefined', 'f() ? "noreferrer" : undefined'],
      ['f() ? "_blank" : undefined', 'isExternal ? "noreferrer" : undefined'],
      ['(isExternal) ? "_blank" : undefined', 'f() ? "noreferrer" : undefined'],
      ['f() ? "_blank" : undefined', 'g() ? "noreferrer" : "noreferrer"'],
      ['undefined ? "_blank" : x', 'f() ? "noreferrer" : undefined'],
    );
    expect(reportedLines(source)).toEqual([1, 2, 3, 5]);
  });

  it("reads a JavaScript file's shared call test as one test too", () => {
    const source = links(['href.startsWith("http") ? "_blank" : undefined', 'href.startsWith("http") ? "noreferrer" : undefined'], ['f() ? "_blank" : undefined', 'f() ? undefined : "noreferrer"']);
    expect(reportedLines(source, "javascript", "src/link.jsx")).toEqual([2]);
  });
});
