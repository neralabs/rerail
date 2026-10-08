import type { Consolidated } from "./consolidated.js";
import type { CorpusTiming } from "./corpus.js";
import type { ExpectedFailure } from "./lint-format.js";
import { annotationOf, type ActualViolation, type FileResult, type FinalVerdict, type RuleResult } from "./runner.js";

export interface ConformanceReport {
  generatedAt: string;
  label: string;
  tslintSha: string;
  typescriptVersion: string;
  kernel: { path: string; version: string };
  runs: number;
  inventory: Inventory;
  rules: RuleResult[];
  corpus: CorpusTiming | undefined;
  consolidated: Consolidated;
  /** Golden rules that miss their golden.json entry (`golden.ts`); empty when the gate holds. */
  goldenFailures: string[];
}

export interface Inventory {
  /** TSLint core rules in tslint-rules.tsv. */
  rulesListed: number;
  /** Rules taken out of the equivalence set (excluded-rules.json); never evaluated. */
  rulesExcluded: Array<{ rule: string; reason: string }>;
  /** rulesListed minus rulesExcluded; every count below is over these. */
  rulesScored: number;
  rulesWithFixtures: number;
  rulesWithoutFixtures: string[];
  variants: number;
  lintFiles: number;
  versionSkipped: number;
  markupErrors: string[];
}

const VERDICT_ORDER: FinalVerdict[] = ["PASS", "PARTIAL", "BLOCKED", "INFEASIBLE", "COMPILE_ERROR", "NO_PORT"];

const percent = (value: number | null): string => (value === null ? "-" : `${(value * 100).toFixed(0)}%`);
const cell = (text: string): string => text.replace(/\|/g, "\\|").replace(/\n/g, " ");
const ms = (value: number | null): string => (value === null ? "-" : value.toFixed(2));

/** `14 excluded (formatter)`: the count, tagged by the first word of each distinct reason. */
export function excludedCount(excluded: ReadonlyArray<{ reason: string }>): string {
  const tags = [...new Set(excluded.map(({ reason }) => reason.split(/[\s;,]/)[0] ?? reason))];
  return `${excluded.length} excluded${tags.length > 0 ? ` (${tags.join(", ")})` : ""}`;
}

/** JSON form drops fixture sources; everything else in the run is kept. */
export function reportJson(report: ConformanceReport): string {
  return `${JSON.stringify(
    {
      ...report,
      rules: report.rules.map(rule => ({
        ...rule,
        results: rule.results.map(({ source: _source, ...result }) => result),
        repaired: rule.repaired === undefined ? undefined : { ...rule.repaired, results: rule.repaired.results.map(({ source: _source, ...result }) => result) },
      })),
    },
    null,
    2,
  )}\n`;
}

export function reportMarkdown(report: ConformanceReport): string {
  const lines: string[] = [];
  const { inventory } = report;
  lines.push(`# TSLint conformance: ${report.label}`, "");
  lines.push(`- generated ${report.generatedAt}`);
  lines.push(`- fixtures palantir/tslint@${report.tslintSha.slice(0, 12)}, TS directives evaluated as typescript ${report.typescriptVersion}`);
  lines.push(`- kernel ${report.kernel.path} (${report.kernel.version}); timings median of ${report.runs}`);
  lines.push(`- rules: ${inventory.rulesListed} TSLint core rules, ${excludedCount(inventory.rulesExcluded)}, ${inventory.rulesScored} scored`);
  lines.push(
    `- inventory: ${inventory.rulesWithFixtures}/${inventory.rulesScored} scored rules have fixtures, ${inventory.variants} variants, ${inventory.lintFiles} .lint files, ${inventory.versionSkipped} skipped by TypeScript version gate`,
  );
  if (inventory.rulesWithoutFixtures.length > 0) lines.push(`- rules without fixtures: ${inventory.rulesWithoutFixtures.join(", ")}`);
  if (inventory.markupErrors.length > 0) lines.push(`- markup errors: ${inventory.markupErrors.join("; ")}`);
  lines.push("");

  lines.push("## Summary", "", "| verdict | rules |", "|---|---|");
  for (const verdict of VERDICT_ORDER) {
    const count = report.rules.filter(rule => rule.verdict === verdict).length;
    if (count > 0) lines.push(`| ${verdict} | ${count} |`);
  }
  lines.push("");

  const golden = report.rules.filter(rule => rule.golden);
  lines.push("## Golden set", "");
  lines.push(
    `${golden.length} rules ship in the opt-in pack (G in the rules table), scored with their shipped docs. Gate (golden.json): ${
      report.goldenFailures.length === 0 ? "every golden rule meets its entry." : `${report.goldenFailures.length} failure(s):`
    }`,
  );
  for (const failure of report.goldenFailures) lines.push(`- ${cell(failure)}`);
  const scoped = golden.filter(rule => rule.outOfScope.length > 0);
  if (scoped.length > 0) {
    lines.push("", `Fixture files outside the shipped scope (not scored, their expected failures are not misses): ${scoped.reduce((sum, rule) => sum + rule.outOfScope.length, 0)}.`, "");
    lines.push("| rule | files |", "|---|---|");
    for (const rule of scoped) lines.push(`| ${rule.rule} | ${rule.outOfScope.map(file => `\`${file}\``).join(", ")} |`);
  }
  lines.push("");

  lines.push(...consolidatedMarkdown(report.consolidated));

  if (report.corpus !== undefined) {
    const corpus = report.corpus;
    lines.push("## Corpus timing", "");
    lines.push(
      `${corpus.rules.length} ported rules in one workspace over \`${corpus.directory}\`: ${corpus.files} files, ${(corpus.bytes / 1_048_576).toFixed(2)} MiB.`,
      "",
      "| measure | median ms | runs ms |",
      "|---|---|---|",
      `| compile all ports | ${corpus.compileMs.toFixed(1)} | |`,
      `| cold full load | ${corpus.coldMedianMs.toFixed(1)} | ${corpus.coldRunsMs.map(value => value.toFixed(1)).join(", ")} |`,
      ...corpus.edits.map(
        edit =>
          `| 1-file edit, ${edit.which} file (\`${edit.path}\`, ${(edit.bytes / 1024).toFixed(0)} KiB) | ${edit.medianMs.toFixed(2)} | ${edit.runsMs.map(value => value.toFixed(2)).join(", ")} |`,
      ),
      "",
      `Result: ${corpus.findings} findings, ${corpus.failures} failures${corpus.failedPaths.length > 0 ? ` in ${corpus.failedPaths.length} files: ${corpus.failedPaths.map(path => `\`${path}\``).join(", ")}` : ""}; ${corpus.partialPaths.length} partial files (\`source-partial\`)${corpus.partialPaths.length > 0 ? `: ${corpus.partialPaths.map(path => `\`${path}\``).join(", ")}` : ""}.`,
    );
    if (corpus.compileErrors.length > 0) lines.push(`Left out (compile error): ${corpus.compileErrors.join(", ")}.`);
    lines.push("");
  }

  lines.push("## Rules", "");
  lines.push(
    "G = golden (in the opt-in pack). Raw = the harness verdict (sidecar verdict when there is one). Effective = PASS (repaired fixture) when the port passes every fixture once invalid fixture syntax is repaired (repairs.ts) and that was its only blocker. TP/FP/FN are raw; `rep` is the score on repaired fixtures. `+Ne` = engine failures (block PASS); `Np` = files the kernel marked `source-partial` (syntax errors; AST checks ran outside them; informational); `No` = fixture files outside a golden doc's shipped scope (not scored).",
    "",
  );
  lines.push("| rule | G | T | F | raw | effective | PASS class | TP/FP/FN | rep | end | primitive | secondary | needs | reason | doc KiB | eval ms |");
  lines.push("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
  const summaries = new Map(report.consolidated.rules.map(summary => [summary.rule, summary]));
  for (const rule of report.rules) {
    const summary = summaries.get(rule.rule)!;
    const scored = rule.auto === "PASS" || rule.auto === "PARTIAL";
    const engine = `${rule.engineFailures > 0 ? ` +${rule.engineFailures}e` : ""}${rule.partialFiles > 0 ? ` ${rule.partialFiles}p` : ""}${rule.outOfScope.length > 0 ? ` ${rule.outOfScope.length}o` : ""}`;
    const repaired = summary.repaired_tp_fp_fn === undefined ? "" : summary.repaired_tp_fp_fn.join("/");
    lines.push(
      `| ${rule.rule} | ${rule.golden ? "G" : ""} | ${rule.requiresTypeInfo ? "T" : ""} | ${rule.hasFixer ? "F" : ""} | ${summary.raw} | ${summary.effective} | ${summary.passClass ?? ""} | ${
        scored ? `${rule.truePositives}/${rule.falsePositives}/${rule.falseNegatives}${engine}` : "-"
      } | ${repaired} | ${scored && rule.truePositives > 0 ? `${rule.endAgreements}/${rule.truePositives}` : "-"} | ${cell(summary.primitive ?? "")} | ${cell(summary.secondary.join(", "))} | ${cell(
        summary.needs.join(", "),
      )} | ${cell(summary.reason)} | ${summary.documentKiB} | ${ms(rule.evalMs)} |`,
    );
  }
  if (inventory.rulesExcluded.length > 0) {
    lines.push("", "## Excluded rules", "", "Not evaluated and not counted in any verdict above (`excluded-rules.json`).", "", "| rule | reason |", "|---|---|");
    for (const { rule, reason } of inventory.rulesExcluded) lines.push(`| ${rule} | ${cell(reason)} |`);
  }
  const noted = report.rules.filter(rule => rule.auto !== "NO_PORT" && rule.notes.length > 0);
  if (noted.length > 0) {
    lines.push("", "## Notes", "");
    for (const rule of noted) lines.push(`- **${rule.rule}**: ${rule.notes.join("; ")}`);
  }
  return `${lines.join("\n")}\n`;
}

function caret(sourceLine: string, startCol: number, endCol: number | undefined): string {
  const width = endCol === undefined ? Math.max(1, sourceLine.length - startCol) : Math.max(1, endCol - startCol);
  return `${" ".repeat(startCol)}^${"~".repeat(width - 1)}`;
}

function describe(kind: string, failure: ExpectedFailure | ActualViolation, source: string[], detail: string): string[] {
  const text = source[failure.startLine] ?? "";
  const gutter = String(failure.startLine + 1).padStart(5);
  const endCol = failure.endLine === failure.startLine ? failure.endCol : undefined;
  const span = `${failure.startLine + 1}:${failure.startCol + 1}-${failure.endLine + 1}:${failure.endCol + 1}`;
  return [`    ${kind} ${span}  ${detail}`, `    ${gutter} | ${text}`, `          | ${caret(text, failure.startCol, endCol)}`];
}

/** Newlines as spaces; other control and non-ASCII whitespace characters as `\u{..}` escapes so they stay visible. */
const visible = (code: string): string =>
  code.replace(/\n/g, " ").replace(/[\u0000-\u001f\u007f-\u00a0\u1680\u180e\u2000-\u200f\u2028\u2029\u202f\u205f\u3000\ufeff]/g, char => `\\u{${char.codePointAt(0)!.toString(16)}}`);

function consolidatedMarkdown(consolidated: Consolidated): string[] {
  const lines: string[] = [];
  const counts = (record: Record<string, number>): string =>
    Object.entries(record)
      .sort((a, b) => b[1] - a[1])
      .map(([key, count]) => `${key} ${count}`)
      .join(", ");
  lines.push("## Headline", "");
  lines.push(`- raw verdicts: ${counts(consolidated.rawCounts)}`);
  lines.push(`- effective verdicts: ${counts(consolidated.effectiveCounts)}`);
  lines.push(`- PASS classes (effective PASS): ${counts(consolidated.passClassCounts)}`);
  lines.push("");

  lines.push("## Blocking primitives", "");
  lines.push(
    "Primary primitive per rule: the one missing capability that accounts for the most unmatched findings on valid code (fixture-artifact only when it is the sole blocker; grammar-gap counts, since real code hits it). `blocked` = effective verdict not PASS; `PASS needs it` = the rule passes the fixtures but is fixture-fit, bounded or option-baked without it. (T) = TSLint needs type info.",
    "",
    "| primitive | blocked | PASS needs it | count |",
    "|---|---|---|---|",
  );
  for (const entry of consolidated.histogram) {
    lines.push(`| ${cell(entry.primitive)} | ${entry.blocked.join(", ")} | ${entry.passNeeds.join(", ")} | ${entry.blocked.length} + ${entry.passNeeds.length} |`);
  }
  lines.push("", "### Secondary mentions", "", "| primitive | rules | count |", "|---|---|---|");
  for (const entry of consolidated.secondary) lines.push(`| ${cell(entry.primitive)} | ${entry.rules.join(", ")} | ${entry.rules.length} |`);
  lines.push("");

  lines.push("## Type layer", "");
  lines.push("What the checker is asked by the rules that need it (sidecar `needs`), and how many rules each cumulative capability set would unblock.", "");
  lines.push("| checker question | rules | count |", "|---|---|---|");
  for (const entry of consolidated.typeNeeds) lines.push(`| ${entry.need} | ${entry.rules.join(", ")} | ${entry.rules.length} |`);
  lines.push("", "| type layer provides | rules fully covered | count |", "|---|---|---|");
  for (const entry of consolidated.typeUnblocked) lines.push(`| ${entry.provides.join(" + ")} | ${entry.rules.join(", ")} | ${entry.rules.length} |`);
  lines.push("");

  lines.push("## Large ports", "", "Ports over 50 KiB (default + overrides). Enumeration blow-up is evidence for `utils` and `metavar-compare`.", "");
  lines.push("| rule | KiB |", "|---|---|");
  for (const entry of consolidated.largePorts) lines.push(`| ${entry.rule} | ${entry.kib} |`);
  lines.push("");

  lines.push("## Grammar gaps", "");
  lines.push("Each construct checked live: tsc syntax/grammar diagnostics (is it valid TypeScript?) and the kernel (parse error, or a rule that matches only the wrong tree).", "");
  lines.push("| construct | code | kernel | tsc | verdict | seen in |", "|---|---|---|---|---|---|");
  for (const probe of consolidated.grammar) {
    lines.push(`| ${probe.id} | \`${cell(visible(probe.code.trim()))}\` | ${probe.kernel} | ${cell(probe.tscErrors.join("; ") || "valid")} | ${probe.verdict} | ${cell(probe.seenIn)} |`);
  }
  lines.push("");

  lines.push("## Engine behaviours", "");
  lines.push("Minimal repros run live against the kernel (engine-probes.ts). Expected = upstream ast-grep semantics unless noted.", "");
  lines.push("| probe | claim | source | expected | observed | deviates |", "|---|---|---|---|---|---|");
  for (const probe of consolidated.engine) {
    const observed = probe.compileError !== undefined ? `compile error: ${probe.compileError}` : probe.observed.map(text => `\`${text}\``).join(", ") || "none";
    const expected = probe.expected === null ? "compile error" : probe.expected.map(text => `\`${text}\``).join(", ") || "none";
    lines.push(
      `| ${probe.id} | ${cell(probe.claim)} | \`${cell(probe.source.trim().replace(/\n/g, " "))}\` | ${cell(expected)} (${probe.basis}) | ${cell(observed)} | ${probe.deviates ? "**yes**" : "no"} |`,
    );
  }
  lines.push("");
  return lines;
}

/** Tight per-rule diff for the porting inner loop. Lines and columns shown 1-based. */
export function ruleDetail(rule: RuleResult, showEnds = false): string {
  const out: string[] = [];
  const scored = rule.auto === "PASS" || rule.auto === "PARTIAL";
  out.push(
    `${rule.rule}  ${rule.verdict}${rule.verdict !== rule.auto ? ` (auto ${rule.auto})` : ""}${rule.golden ? "  [golden]" : ""}${rule.requiresTypeInfo ? "  [needs type info]" : ""}${rule.hasFixer ? "  [has fixer]" : ""}`,
  );
  if (scored) {
    out.push(
      `  TP ${rule.truePositives}  FP ${rule.falsePositives}  FN ${rule.falseNegatives}  P ${percent(rule.precision)}  R ${percent(rule.recall)}  end ${rule.endAgreements}/${rule.truePositives}  engine failures ${rule.engineFailures}  partial files ${rule.partialFiles}  eval ${ms(rule.evalMs)} ms`,
    );
  }
  if (rule.compileError !== undefined) out.push(`  compile error: ${rule.compileError}`);
  if (rule.sidecar !== undefined) out.push(`  sidecar: ${rule.sidecar.verdict}${rule.sidecar.primitive === undefined ? "" : ` [${rule.sidecar.primitive}]`} ${rule.sidecar.reason}`);
  const { goldenEntry } = rule;
  if (goldenEntry !== undefined) {
    out.push(`  golden: expects ${goldenEntry.verdict}, TP ${goldenEntry.truePositives}${goldenEntry.primitive === undefined ? "" : ` [${goldenEntry.primitive}]`}${goldenEntry.reason === undefined ? "" : ` ${goldenEntry.reason}`}`);
  } else if (rule.golden) out.push("  golden: no golden.json entry");
  for (const item of annotationOf(rule)?.evidence ?? []) out.push(`  evidence ${item.variant}${item.file === undefined ? "" : `/${item.file}`}:${item.line}  ${item.why}`);
  for (const note of rule.notes) out.push(`  note: ${note}`);
  if (rule.fixes !== undefined) out.push(`  fixes: ${rule.fixes.matched}/${rule.fixes.files} TSLint .fix files reproduced`);
  const { repaired } = rule;
  if (repaired !== undefined) {
    out.push(
      `  repaired fixtures (${repaired.kinds.join(", ")}): ${repaired.auto}  TP ${repaired.truePositives}  FP ${repaired.falsePositives}  FN ${repaired.falseNegatives}  engine failures ${repaired.engineFailures}  partial files ${repaired.partialFiles}${repaired.droppedExpected > 0 ? `  dropped expected ${repaired.droppedExpected}` : ""}`,
    );
    for (const result of repaired.results) {
      const source = result.source.split("\n");
      out.push(`  repaired ${result.variant}/${result.lintPath}  expected ${result.expected.length}  TP ${result.truePositives}`);
      for (const failure of result.engineFailures) out.push(`    ENGINE ${failure.code} [${failure.clauseId}] ${failure.message}`);
      if (result.partial !== undefined) out.push(`    PARTIAL ${result.partial}`);
      for (const failure of result.missed) out.push(...describe("FN", failure, source, `expected [${failure.message}]`));
      for (const violation of result.extra) out.push(...describe("FP", violation, source, `clause ${violation.clauseId}`));
    }
  }

  const byVariant = new Map<string, FileResult[]>();
  for (const result of rule.results) byVariant.set(result.variant, [...(byVariant.get(result.variant) ?? []), result]);
  for (const info of rule.variantInfo) {
    const results = byVariant.get(info.id) ?? [];
    const clean = results.every(result => result.skipped === undefined && result.outOfScope === undefined && result.missed.length === 0 && result.extra.length === 0 && result.engineFailures.length === 0);
    const options = info.config.options.length === 0 ? "no options" : `options ${JSON.stringify(info.config.options)}`;
    const document = results[0]?.document ?? "(no document)";
    out.push("", `variant ${info.id}  ${options}${info.typeChecked ? "  typeChecked" : ""}  doc ${document}  ${clean ? "ok" : ""}`.trimEnd());
    for (const result of results) {
      if (result.skipped !== undefined) {
        out.push(`  ${result.lintPath}  skipped: ${result.skipped}`);
        continue;
      }
      if (result.outOfScope !== undefined) {
        out.push(`  ${result.lintPath}  out of scope: outside the shipped doc's scope, expected ${result.expected.length} not scored`);
        continue;
      }
      const ok = result.missed.length === 0 && result.extra.length === 0 && result.engineFailures.length === 0;
      const ends = result.endMismatches.length > 0 ? `  end mismatch ${result.endMismatches.length}` : "";
      out.push(`  ${result.lintPath}${result.javascript ? " (js)" : ""}  expected ${result.expected.length}  TP ${result.truePositives}${ok ? "  ok" : ""}${ends}`);
      for (const failure of result.engineFailures) out.push(`    ENGINE ${failure.code} [${failure.clauseId}] ${failure.message}`);
      if (result.partial !== undefined) out.push(`    PARTIAL ${result.partial}`);
      if (result.fix !== undefined) {
        const { fix } = result;
        out.push(`    FIX ${fix.matched ? "matches" : "differs from"} ${result.lintPath.replace(/\.lint$/, ".fix")} (${fix.emitted} emitted)`);
        if (fix.firstDiff !== undefined) {
          out.push(`      first difference at line ${fix.firstDiff.line + 1}`, `      expected | ${fix.firstDiff.expected}`, `        actual | ${fix.firstDiff.actual}`);
        }
      }
      const source = result.source.split("\n");
      for (const failure of result.missed) out.push(...describe("FN", failure, source, `expected [${failure.message}]`));
      for (const violation of result.extra) out.push(...describe("FP", violation, source, `clause ${violation.clauseId}`));
      for (const { expected, actual } of showEnds ? result.endMismatches : []) {
        out.push(`    END ${expected.startLine + 1}:${expected.startCol + 1} expected end ${expected.endLine + 1}:${expected.endCol + 1}, got ${actual.endLine + 1}:${actual.endCol + 1}`);
      }
    }
  }
  return `${out.join("\n")}\n`;
}
