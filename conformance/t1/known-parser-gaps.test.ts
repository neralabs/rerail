import { beforeAll, describe, expect, it } from "vitest";
import { languageFor } from "../../src/code-rule-enforcer.js";
import { encodeCodeRuleWorkspaceRequest, loadNativeBinding } from "../../src/native.js";
import { capabilitiesJson } from "../engine.js";
import { dialectsOf, loadKnownParserGaps, pinnedGrammar, type KnownParserGap } from "./known-parser-gaps.js";

type Failure = { ruleId?: string; code: string; message: string; path?: string };

const ledger = loadKnownParserGaps();
const cases = ledger.gaps.map(gap => [gap.id, gap] as const);

/** One probe file per dialect of the gap's language, under a directory named for the gap. */
const probeFiles = (gap: KnownParserGap) =>
  dialectsOf(gap.language).map(dialect => {
    const path = `${gap.id}/probe${dialect.suffix}`;
    return { path, language: languageFor(path)!, source: gap.probe };
  });

// The T1 census's syntax probe (typescript-syntax.ts), one per clause language: ERROR nodes are never witnesses, so
// the clause reports nothing, and its `source-partial` failure is the engine saying the file has syntax errors.
const syntaxProbe = (language: string): string =>
  JSON.stringify({
    format: "rerail.policy/v1",
    id: `syntax-probe-${language}`,
    revision: 1,
    mode: "log",
    description: "Reports a file the grammar cannot parse.",
    scope: { include: ["**/*"], exclude: [] },
    clauses: [{ id: "check", forbid: { ast: { language, rule: { kind: "ERROR" } } }, violation: { anchor: "witness.span", code: "syntax", message: "syntax" } }],
  });

async function parseFailures(): Promise<{ failures: Failure[]; violations: number }> {
  const binding = loadNativeBinding();
  const workspace = new binding.CodeRuleDslWorkspace();
  for (const language of new Set(ledger.gaps.map(gap => gap.language))) {
    workspace.addCodeRule(binding.CodeRuleDslRuntime.compile(syntaxProbe(language), "json", capabilitiesJson()));
  }
  await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: 1, files: ledger.gaps.flatMap(probeFiles), removed: [], quiet: true }));
  const failures: Failure[] = [];
  let violations = 0;
  for (let cursor = 0; ; ) {
    const page = JSON.parse(await workspace.resultPageAsync(cursor, 8 << 20));
    failures.push(...page.failures);
    violations += page.violations.length;
    if (page.done) break;
    cursor = page.cursor;
  }
  workspace.close();
  return { failures, violations };
}

describe("known parser gaps (conformance/t1/known-parser-gaps.json)", () => {
  let result: { failures: Failure[]; violations: number };
  beforeAll(async () => {
    result = await parseFailures();
  });

  it.each(cases)("%s names the grammar Rerail pins", (_id, gap) => {
    expect({ grammar: gap.grammar, version: gap.version }).toEqual(pinnedGrammar(gap.language));
  });

  it.each(cases)("%s still fails to parse in every dialect of its language", (id, gap) => {
    for (const file of probeFiles(gap)) {
      // A typescript clause applies to JavaScript files too; only the gap's own language's clause is read.
      const own = result.failures.filter(failure => failure.path === file.path && failure.ruleId === `syntax-probe-${gap.language}`);
      if (own.length === 0) expect.fail(`${id} parses as ${file.language} now: the gap is stale, remove it from known-parser-gaps.json`);
      expect(own.map(failure => failure.code), file.path).toEqual(["source-partial"]);
    }
  });

  it("reports nothing but the probes' syntax errors", () => {
    expect(result.violations).toBe(0);
    expect(result.failures.filter(failure => failure.code !== "source-partial")).toEqual([]);
  });
});
