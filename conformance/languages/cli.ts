import { mkdirSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { ProcessCommandRunner } from "../process.js";
import { loadEngine } from "./checks/engine.js";
import { ungatedChecks } from "./checks/ungated.js";
import { judgeToolPack, type ToolPackVerdict } from "./gates.js";
import { gatedToolPacks } from "./manifest.js";
import { REPORTS_DIRECTORY, listPortedPacks, loadPack, PACKS_ROOT, RERAIL_ROOT, type LoadedPack } from "./pack.js";
import { reportJson, type T4Report } from "./report.js";
import { reportFailures, runPack } from "./runner.js";
import { summarize, writeSummary } from "./summary.js";

const USAGE = `Language-pack rule conformance (T4).

  node --import tsx conformance/languages/cli.ts --pack <name> [options]
  node --import tsx conformance/languages/cli.ts --check [--pack <name>] [--base <sha>] [--out <dir>]

  --pack <name>         pack under packs/<name>/ (or --pack-dir <dir>)
  --fixtures            score the upstream rule fixtures
  --samples             score the real-code samples of every tool-pack with a samples.lock (fetched from S3 once,
                        verified), with any samples staged under its publish/samples/ laid over them; with no source
                        chosen, both
  --rule <id>           score one rule (repeatable); prints every FP, FN and reviewed finding
  --write               write .rerail/reports/<pack>-t4-rules-<date>.json (gitignored)
  --date <YYYY-MM-DD>   report date (default today, UTC)
  --check               CI gate: the fixtures and samples of every pack (or --pack), then every rule of each gated
                        tool-pack (gates.yaml) through loads, compact and the conformance checks (stays-silent and
                        holds-under-rename among them), and the not-gated list; exit 1 on any unexplained FP/FN,
                        missed label or finding on a clean sample, engine failure, stale divergence, test data or
                        samples error, shipped port with no fixture TP and no pack.json examples, or shipped rule of a
                        gated tool-pack that fails a check
  --base <sha>          with --check: the commit the not-gated and pending lists may only shrink against
  --out <dir>           with --check: write summary.json, summary.md, gates.json and the pull request comment
                        (comment.md) there`;

function formatRatio(value: number | null): string {
  return (value === null ? "-" : value.toFixed(3)).padStart(5);
}

function print(report: T4Report, detail: boolean): void {
  console.log(`${report.pack} ${report.tier} ${report.date}  kernel ${report.kernel.version}`);
  for (const source of report.sources) {
    console.log(
      `  ${source.source}: ${source.units} unit(s), ${source.files} file(s), ${source.bytes} bytes, ${source.syntaxErrorFiles.count} syntax-error file(s) (T1), ${source.evaluationMs} ms`,
    );
    if (detail) for (const file of source.syntaxErrorFiles.files) console.log(`      SYNTAX ${file.path}:${file.line}:${file.column}`);
  }
  const width = Math.max(4, ...report.rules.map(row => row.rule.length));
  // `ctx`: engine failures on samples' context files, which no label judges; shown, never failing. `silent/own`: the
  // rule's own upstream units it and upstream are silent on; `ren fp/fn`: unexplained FP and FN on rename variants.
  if (report.rules.length > 0) console.log(`  ${"rule".padEnd(width)}  source            tp     fp     fn   prec  recall  div  fail  ctx  syn  reviewed  silent/clean  silent/own  ren fp/fn`);
  for (const row of report.rules) {
    console.log(
      `  ${row.rule.padEnd(width)}  ${row.source.padEnd(16)} ${String(row.tp).padStart(4)} ${String(row.fp).padStart(6)} ${String(row.fn).padStart(6)}  ${formatRatio(row.precision)}  ${formatRatio(row.recall)}  ${String(row.divergences.length).padStart(3)}  ${String(row.engineFailures).padStart(4)}  ${String(row.contextFailures).padStart(3)}  ${String(row.syntaxErrorFiles.count).padStart(3)}  ${String(row.reviewed).padStart(8)}  ${`${row.silentCleanSamples}/${row.cleanSamples}`.padStart(12)}  ${`${row.silentOwnUnits}/${row.ownUnits}`.padStart(10)}  ${(row.renamed === null ? "-" : `${row.renamed.fp}/${row.renamed.fn}`).padStart(9)}`,
    );
    if (!detail) continue;
    for (const point of row.falsePositives) console.log(`      FP ${point.path}:${point.line}${point.column === undefined ? "" : `:${point.column}`}`);
    for (const point of row.falseNegatives) console.log(`      FN ${point.path}:${point.line}${point.column === undefined ? "" : `:${point.column}`}`);
    for (const point of row.reviewedFindings) console.log(`      REVIEW ${point.path}:${point.line} (a labelled sample's unlabelled line; not a failure)`);
    for (const divergence of row.divergences) console.log(`      ${divergence.side.toUpperCase()} ${divergence.path}:${divergence.line} explained (${divergence.kind}): ${divergence.evidence}`);
    for (const file of row.syntaxErrorFiles.files) console.log(`      SYNTAX ${file.path}:${file.line}:${file.column} skipped (syntax error, T1)`);
  }
  const { coverage } = report;
  console.log(
    `  coverage: ${coverage.rules}/${coverage.minimumRules} rules; uncovered capabilities: ${coverage.uncovered.length === 0 ? "none" : coverage.uncovered.join(", ")}`,
  );
  for (const exemption of coverage.exempt) console.log(`  exempt: ${exemption.capability} (${exemption.reason}); covered by ${exemption.coveredBy.join("; ")}`);
  for (const problem of reportFailures(report)) console.log(`  FAIL ${problem}`);
  console.log(`  ${report.pass ? "PASS" : "FAIL"}`);
}

/** Judge every gated tool-pack of the scored packs, check the not-gated list, and print and write the summary. */
async function gate(scored: ReadonlyMap<string, { loaded: LoadedPack; report: T4Report }>, base: string | undefined, out: string | undefined): Promise<boolean> {
  const engine = loadEngine();
  const verdicts: ToolPackVerdict[] = [];
  for (const { pack, tool } of gatedToolPacks(RERAIL_ROOT)) {
    const entry = scored.get(pack);
    if (entry !== undefined) verdicts.push(judgeToolPack(RERAIL_ROOT, engine, entry.loaded, entry.report, tool));
  }
  const ungated = await ungatedChecks(RERAIL_ROOT, { ...(base === undefined ? {} : { base }), runner: new ProcessCommandRunner() });
  const summary = summarize([...scored.values()].map(({ report }) => ({ pack: report.pack, failures: reportFailures(report) })), verdicts, ungated);
  for (const verdict of verdicts) {
    const shipped = verdict.rules.filter(rule => rule.location === "shipped");
    const candidates = verdict.rules.filter(rule => rule.location === "candidate");
    console.log(`${verdict.toolPack} gates: shipped ${shipped.filter(rule => rule.pass).length}/${shipped.length} pass; candidates ${candidates.filter(rule => rule.pass).length}/${candidates.length} ready to promote`);
    for (const check of verdict.checks.filter(item => !item.ok)) console.log(`  FAIL tool-pack ${check.id}: ${check.detail}`);
    for (const rule of shipped.filter(item => !item.pass)) console.log(`  FAIL ${rule.id} (${rule.stoppedAt}): ${rule.failure!.check}: ${rule.failure!.why}`);
  }
  for (const check of ungated.checks.filter(item => !item.ok)) console.log(`FAIL ${check.id}: ${check.detail}`);
  console.log(`gates: ${summary.blocking.length} blocking failure(s); ${summary.promotable.length} candidate(s) ready to promote`);
  if (out !== undefined) {
    await writeSummary(out, summary);
    writeFileSync(join(out, "gates.json"), `${JSON.stringify(verdicts, null, 2)}\n`);
    console.log(`wrote ${relative(RERAIL_ROOT, resolve(out))}/{summary.json,summary.md,comment.md,gates.json}`);
  }
  return summary.pass;
}

/**
 * The command line, checked: the pack directories to score and the options. `--check` judges the packs under the
 * repository's packs/ (`gate` walks RERAIL_ROOT), so it takes no `--pack-dir` outside them.
 */
export function parseCli(args: readonly string[]) {
  const { values } = parseArgs({
    args: [...args],
    options: {
      pack: { type: "string" },
      "pack-dir": { type: "string" },
      fixtures: { type: "boolean", default: false },
      samples: { type: "boolean", default: false },
      rule: { type: "string", multiple: true },
      write: { type: "boolean", default: false },
      date: { type: "string", default: new Date().toISOString().slice(0, 10) },
      check: { type: "boolean", default: false },
      base: { type: "string" },
      out: { type: "string" },
      help: { type: "boolean", default: false },
    },
  });
  if (values.help) return { values, directories: [] };
  if (values.check && values["pack-dir"] !== undefined) throw new Error("--check judges the repository's packs/; it does not take --pack-dir (use --pack <name>)");
  const directories =
    values["pack-dir"] !== undefined
      ? [resolve(values["pack-dir"])]
      : values.pack !== undefined
        ? [join(PACKS_ROOT, values.pack)]
        : values.check
          ? listPortedPacks().map(name => join(PACKS_ROOT, name))
          : [];
  if (directories.length === 0) throw new Error(`--pack, --pack-dir or --check is required\n\n${USAGE}`);
  if (values.check && values.write) throw new Error("--check runs the fixtures and the samples; it does not take --write");
  if (values.check && values.rule !== undefined) throw new Error("--check judges every rule; it does not take --rule");
  if (!values.check && (values.base !== undefined || values.out !== undefined)) throw new Error("--base and --out go with --check");
  return { values, directories };
}

async function main(): Promise<void> {
  const { values, directories } = parseCli(process.argv.slice(2));
  if (values.help) {
    console.log(USAGE);
    return;
  }
  // With no source chosen, every source; --check chooses every source too.
  const everySource = values.check || (!values.fixtures && !values.samples);
  let failed = false;
  const scored = new Map<string, { loaded: LoadedPack; report: T4Report }>();
  for (const directory of directories) {
    const loaded = loadPack(directory);
    const report = await runPack(loaded, {
      ...(values.rule === undefined ? {} : { rules: values.rule }),
      fixtures: everySource || values.fixtures,
      samples: everySource || values.samples,
      date: values.date,
    });
    print(report, values.rule !== undefined);
    scored.set(loaded.pack.pack, { loaded, report });
    failed ||= !report.pass;
    if (values.write) {
      mkdirSync(REPORTS_DIRECTORY, { recursive: true });
      const path = join(REPORTS_DIRECTORY, `${report.pack}-t4-rules-${report.date}.json`);
      writeFileSync(path, reportJson(report));
      console.log(`  wrote ${relative(RERAIL_ROOT, path)}`);
    }
  }
  if (values.check) failed = !(await gate(scored, values.base, values.out)) || failed;
  if (failed) process.exitCode = 1;
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error: unknown) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 2;
  });
}
