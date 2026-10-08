import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { parseArgs } from "node:util";
import { loadNativeBinding, nativePlatformSuffix } from "../../src/native.js";
import { timeCorpus, type CorpusTiming } from "./corpus.js";
import { ensureFixtures, fetchFixtures, loadExcludedRules, loadRuleList, loadScoredRules, loadVariants, REPOSITORY_ROOT, repositoryPath, RERAIL_ROOT, TSLINT_SHA } from "./fixtures.js";
import { FIXTURE_TYPESCRIPT_VERSION } from "./lint-format.js";
import { reportJson, reportMarkdown, ruleDetail, type ConformanceReport, type Inventory } from "./report.js";
import { summarize } from "./consolidated.js";
import { buildSummary, deltaMarkdown, parseSummary, summaryJson } from "./summary.js";
import { runEngineProbes } from "./engine-probes.js";
import { runGrammarProbes } from "./grammar-gaps.js";
import { evaluateRule, type RuleResult } from "./runner.js";
import { literalScanReport } from "./literal-scan.js";
import { renameCheck, renameMarkdown, type RenameComparison } from "./poison.js";
import { goldenRules } from "./golden.js";
import { goldenFailures } from "./golden-gate.js";

const USAGE = `TSLint conformance harness for Rerail policy ports.

  just rerail-conformance fetch                       fetch TSLint fixtures at the pinned SHA (idempotent)
  just rerail-conformance --rule <name> [--ends]      detailed diff for one rule (porting inner loop)
  just rerail-conformance golden                      golden gate: every rule in the opt-in pack meets its golden.json entry;
                                                     exit 1 with a line per regression
  just rerail-conformance [--label <l>] [--no-corpus] full run; writes report-<date>-<label>.md and summary-<date>-<label>.json
                                                     to docs/design/data_history/rerail-tslint-conformance/, full detail JSON to tmp/rerail-conformance/;
                                                     exit 1 if the golden gate fails
  just rerail-conformance --label <l> --compare <summary.json> [--delta-only]
                                                     also writes delta-<date>-<label>.md against an earlier summary;
                                                     --delta-only sends the full report .md to tmp/ (commit summary + delta)
  just rerail-conformance --corpus <dir>              all ports in one workspace over <dir> (e.g. web/src): cold + 1-file edit
  just rerail-conformance --rename-identifiers [--rule <name>]
                                                     identifier-poisoning gate: score on fixtures with user identifiers renamed
                                                     to opaque same-length names; lists rules whose score changes, exit 1 if a
                                                     golden rule changes. With --rule: that rule's renamed detail
  just rerail-conformance literals [--rule <name>] [--all]
                                                     static scan: identifier literals in ports that only fixtures use
  options: --runs <n> (default 5)  --corpus-dir <dir> (full-run corpus, default web/src)  --no-write`;

/** Committed output: `data_history/` is ignored everywhere except under docs/design. */
const REPORT_DIRECTORY = join(REPOSITORY_ROOT, "docs", "design", "data_history", "rerail-tslint-conformance");
/** Per-fixture detail: regenerable and megabytes per run, so it stays in the gitignored tmp/. */
const DETAIL_DIRECTORY = join(REPOSITORY_ROOT, "tmp", "rerail-conformance");

/** `just` runs from rerail/, so a relative path that does not exist there is taken from the repository root. */
function resolveCompare(path: string): string {
  const fromCwd = resolve(path);
  return repositoryPath(existsSync(fromCwd) ? fromCwd : resolve(REPOSITORY_ROOT, path));
}

function engineInfo(): { path: string; version: string } {
  const configured = process.env.RERAIL_NATIVE_KERNEL_PATH;
  const path = configured ?? join(RERAIL_ROOT, "native", `omp_cosine_rules.${nativePlatformSuffix()}.node`);
  let built = "";
  try {
    built = `, built ${statSync(path).mtime.toISOString()}`;
  } catch {
    built = "";
  }
  return { path: relative(REPOSITORY_ROOT, path), version: `${loadNativeBinding().health().kernelVersion}${built}` };
}

function inventory(): Inventory {
  const rules = loadScoredRules();
  const excluded = [...loadExcludedRules()].map(([rule, reason]) => ({ rule, reason }));
  let variants = 0;
  let lintFiles = 0;
  let versionSkipped = 0;
  const rulesWithoutFixtures: string[] = [];
  const markupErrors: string[] = [];
  for (const rule of rules) {
    const found = loadVariants(rule.name);
    if (found.length === 0) rulesWithoutFixtures.push(rule.name);
    variants += found.length;
    for (const variant of found) {
      for (const file of variant.files) {
        lintFiles += 1;
        if (!file.parsed.skipped) continue;
        if ("parseError" in file.parsed) markupErrors.push(`${rule.name}/${variant.id}/${file.lintPath}: ${file.parsed.parseError}`);
        else versionSkipped += 1;
      }
    }
  }
  return {
    rulesListed: loadRuleList().length,
    rulesExcluded: excluded,
    rulesScored: rules.length,
    rulesWithFixtures: rules.length - rulesWithoutFixtures.length,
    rulesWithoutFixtures,
    variants,
    lintFiles,
    versionSkipped,
    markupErrors,
  };
}

function printCorpus(corpus: CorpusTiming): void {
  console.log(`corpus       ${corpus.directory}: ${corpus.files} files, ${(corpus.bytes / 1_048_576).toFixed(2)} MiB`);
  console.log(`rules        ${corpus.rules.length} (${corpus.rules.join(", ")})${corpus.compileErrors.length > 0 ? `; compile errors: ${corpus.compileErrors.join(", ")}` : ""}`);
  console.log(`compile      ${corpus.compileMs.toFixed(1)} ms`);
  console.log(`cold load    median ${corpus.coldMedianMs.toFixed(1)} ms  [${corpus.coldRunsMs.map(value => value.toFixed(1)).join(", ")}]`);
  for (const edit of corpus.edits) {
    console.log(`1-file edit  median ${edit.medianMs.toFixed(2)} ms  [${edit.runsMs.map(value => value.toFixed(2)).join(", ")}]  ${edit.which} ${edit.path} (${(edit.bytes / 1024).toFixed(0)} KiB)`);
  }
  console.log(
    `result       ${corpus.findings} findings, ${corpus.failures} failures${corpus.failedPaths.length > 0 ? ` in ${corpus.failedPaths.length} files` : ""}, ${corpus.partialPaths.length} partial files`,
  );
  for (const path of corpus.failedPaths) console.log(`  failed     ${path}`);
  for (const path of corpus.partialPaths) console.log(`  partial    ${path}`);
}

async function main(): Promise<void> {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: {
      rule: { type: "string" },
      corpus: { type: "string" },
      "corpus-dir": { type: "string", default: "web/src" },
      "no-corpus": { type: "boolean", default: false },
      label: { type: "string", default: "run" },
      runs: { type: "string", default: "5" },
      ends: { type: "boolean", default: false },
      compare: { type: "string" },
      "delta-only": { type: "boolean", default: false },
      "no-write": { type: "boolean", default: false },
      "rename-identifiers": { type: "boolean", default: false },
      all: { type: "boolean", default: false },
      help: { type: "boolean", short: "h", default: false },
    },
  });
  if (values.help) {
    console.log(USAGE);
    return;
  }
  if (positionals[0] === "fetch") {
    fetchFixtures();
    return;
  }
  if (positionals[0] === "literals") {
    ensureFixtures();
    process.stdout.write(literalScanReport({ all: values.all, rule: values.rule }));
    return;
  }
  if (positionals[0] === "golden") {
    ensureFixtures();
    const scored = loadScoredRules();
    const golden = new Set(goldenRules());
    const results: RuleResult[] = [];
    for (const info of scored) if (golden.has(info.name)) results.push(await evaluateRule(info, 1));
    reportGolden(goldenFailures(results), golden.size);
    return;
  }
  if (positionals.length > 0) throw new Error(`unknown command ${positionals.join(" ")}\n\n${USAGE}`);
  if (values["delta-only"] && values.compare === undefined) throw new Error("--delta-only needs --compare <summary.json>");
  const runs = Number.parseInt(values.runs, 10);
  if (!Number.isInteger(runs) || runs < 1) throw new Error("--runs must be a positive integer");

  if (values.corpus !== undefined) {
    printCorpus(await timeCorpus(values.corpus, runs));
    return;
  }

  ensureFixtures();
  const rules = loadScoredRules();
  if (values.rule !== undefined) {
    const excludedReason = loadExcludedRules().get(values.rule);
    if (excludedReason !== undefined) throw new Error(`${values.rule} is excluded from the equivalence set: ${excludedReason} (see conformance/tslint/excluded-rules.json)`);
    const info = rules.find(rule => rule.name === values.rule);
    if (info === undefined) throw new Error(`${values.rule} is not a TSLint core rule (see conformance/tslint/tslint-rules.tsv)`);
    process.stdout.write(ruleDetail(await evaluateRule(info, runs, { renameIdentifiers: values["rename-identifiers"] }), values.ends));
    return;
  }

  if (values["rename-identifiers"]) {
    const comparisons: RenameComparison[] = [];
    for (const info of rules) comparisons.push(await renameCheck(info));
    const markdown = renameMarkdown(comparisons);
    process.stdout.write(markdown);
    if (!values["no-write"]) {
      mkdirSync(DETAIL_DIRECTORY, { recursive: true });
      const path = join(DETAIL_DIRECTORY, "rename-identifiers.md");
      writeFileSync(path, markdown);
      console.log(`wrote ${relative(REPOSITORY_ROOT, path)}`);
    }
    const golden = new Set(goldenRules());
    const failing = comparisons.filter(comparison => golden.has(comparison.rule) && (comparison.changed || comparison.renamerIssues.length > 0));
    if (failing.length > 0) {
      console.error(`golden rules that change on renamed fixtures: ${failing.map(comparison => comparison.rule).join(", ")}`);
      process.exitCode = 1;
    }
    return;
  }

  const results: RuleResult[] = [];
  for (const info of rules) results.push(await evaluateRule(info, runs));
  const corpus = values["no-corpus"] ? undefined : await timeCorpus(values["corpus-dir"], runs);
  const report: ConformanceReport = {
    generatedAt: new Date().toISOString(),
    label: values.label,
    tslintSha: TSLINT_SHA,
    typescriptVersion: FIXTURE_TYPESCRIPT_VERSION,
    kernel: engineInfo(),
    runs,
    inventory: inventory(),
    rules: results,
    corpus,
    consolidated: summarize(results, await runGrammarProbes(), await runEngineProbes()),
    goldenFailures: goldenFailures(results),
  };
  const markdown = reportMarkdown(report);
  const summary = buildSummary(report);
  const comparePath = values.compare === undefined ? undefined : resolveCompare(values.compare);
  const delta = comparePath === undefined ? undefined : deltaMarkdown(parseSummary(readFileSync(comparePath, "utf8")), summary);
  if (values["no-write"]) {
    process.stdout.write(delta ?? markdown);
    reportGolden(report.goldenFailures, goldenRules().length);
    return;
  }
  mkdirSync(REPORT_DIRECTORY, { recursive: true });
  mkdirSync(DETAIL_DIRECTORY, { recursive: true });
  const now = new Date();
  const date = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
  const name = `${date}-${values.label.replace(/[^\w.-]+/g, "-")}`;
  const written: string[] = [];
  const write = (path: string, text: string): void => {
    writeFileSync(path, text);
    written.push(relative(REPOSITORY_ROOT, path));
  };
  write(join(values["delta-only"] ? DETAIL_DIRECTORY : REPORT_DIRECTORY, `report-${name}.md`), markdown);
  write(join(REPORT_DIRECTORY, `summary-${name}.json`), summaryJson(summary));
  if (delta !== undefined) write(join(REPORT_DIRECTORY, `delta-${name}.md`), delta);
  write(join(DETAIL_DIRECTORY, `report-${name}.json`), reportJson(report));
  const counts = new Map<string, number>();
  for (const result of results) counts.set(result.verdict, (counts.get(result.verdict) ?? 0) + 1);
  console.log([...counts].map(([verdict, count]) => `${verdict} ${count}`).join("  "));
  if (corpus !== undefined) printCorpus(corpus);
  for (const path of written) console.log(`wrote ${path}`);
  reportGolden(report.goldenFailures, goldenRules().length);
}

function reportGolden(failures: readonly string[], rules: number): void {
  if (failures.length === 0) {
    console.log(`golden gate: all ${rules} golden rules meet golden.json`);
    return;
  }
  console.error(`golden gate: ${failures.length} failure(s)`);
  for (const failure of failures) console.error(`  ${failure}`);
  process.exitCode = 1;
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
