import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { compareDeterministicStrings } from "../../src/engine/index.js";
import { ProcessCommandRunner } from "../process.js";
import { ungatedChecks } from "./checks/ungated.js";
import type { ToolPackVerdict } from "./gates.js";
import { REPORTS_DIRECTORY, listPortedPacks, loadPack, PACKS_ROOT, RERAIL_ROOT } from "./pack.js";
import { reportJson, type T4Report } from "./report.js";
import { reportFailures, runPack } from "./runner.js";
import { evaluateShard, parseShard, reportPacks, WorkerPool, type PlanOptions, type Shard } from "./shards.js";
import { summarize, writeSummary } from "./summary.js";

const USAGE = `Language-pack rule conformance (T4).

  node --import tsx conformance/languages/cli.ts --pack <name> [options]
  node --import tsx conformance/languages/cli.ts --check [--pack <name>] [--base <sha>] [--out <dir>]
  node --import tsx conformance/languages/cli.ts --check [--pack <name>] --shard <k>/<n> --results-out <dir>
  node --import tsx conformance/languages/cli.ts --check [--pack <name>] --results <dir> [--base <sha>] [--out <dir>]

  --pack <name>         pack under packs/<name>/ (or --pack-dir <dir>)
  --fixtures            score the upstream rule fixtures
  --samples             score the real-code samples of every tool-pack with a samples.lock (fetched from S3 once,
                        verified), with any samples staged under its publish/samples/ laid over them; with no source
                        chosen, both
  --rule <id>           score one rule (repeatable); prints every FP, FN and reviewed finding
  --write               write .rerail/reports/<pack>-t4-rules-<date>.json (gitignored)
  --date <YYYY-MM-DD>   report date (default today, UTC)
  --check               CI gate: the fixtures and samples of every pack (or --pack), then every port of each gated
                        tool-pack (gates.yaml) through the conformance checks (stays-silent and holds-under-rename
                        among them), and the not-gated list; exit 1 on any unexplained FP/FN, missed label or finding
                        on a clean sample, engine failure, stale divergence, test data or samples error, shipped port
                        with no fixture TP and no pack.json examples, or shipped port of a gated tool-pack that fails
                        a check. Whether a document loads and is compact is pack validation's check 1 (npm test), not
                        this check's. Spread over worker processes (shards.ts)
  --base <sha>          with --check: the commit the not-gated and pending lists may only shrink against
  --out <dir>           with --check: write summary.json, summary.md, gates.json and the pull request comment
                        (comment.md) there
  --shard <k>/<n>       with --check and --results-out: evaluate shard k of n of every pack's scoring, write the
                        results to <dir>, and judge nothing
  --results <dir>       with --check: report and judge from the results every shard wrote to <dir>, evaluating
                        nothing`;

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

/** Print every gated tool-pack's verdict and the not-gated list, and write the summary. */
async function gate(scored: ReadonlyMap<string, { report: T4Report; verdicts: ToolPackVerdict[] }>, base: string | undefined, out: string | undefined): Promise<boolean> {
  // The verdicts in path order: gatedToolPacks lists each pack's tool-packs together, in path order too.
  const verdicts = [...scored].sort(([a], [b]) => compareDeterministicStrings(a, b)).flatMap(([, { verdicts: packVerdicts }]) => packVerdicts);
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
      shard: { type: "string" },
      "results-out": { type: "string" },
      results: { type: "string" },
      help: { type: "boolean", default: false },
    },
  });
  if (values.help) return { values, directories: [], shard: undefined };
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
  if (!values.check && (values.base !== undefined || values.out !== undefined || values.shard !== undefined || values.results !== undefined))
    throw new Error("--base, --out, --shard and --results go with --check");
  if ((values.shard === undefined) !== (values["results-out"] === undefined)) throw new Error("--shard and --results-out go together");
  if (values.shard !== undefined && (values.results !== undefined || values.base !== undefined || values.out !== undefined))
    throw new Error("--shard only evaluates; --results, --base and --out go with the run that reports");
  return { values, directories, shard: values.shard === undefined ? undefined : parseShard(values.shard) };
}

/** Where `--check` reads and writes, beyond the packs and the options. */
interface CheckPaths {
  /** `--shard` and `--results-out`: evaluate this shard only, into this directory. */
  shard?: { shard: Shard; out: string };
  /** `--results`: the shards' results to report from. */
  results?: string;
  base?: string;
  out?: string;
}

/**
 * `--check`: every pack's report from the shards' results (evaluating them here first, as shard 1 of 1, unless
 * `--results` names them), printed in order, then the gate; or, with `--shard`, only that shard's results.
 */
async function check(directories: readonly string[], options: PlanOptions, paths: CheckPaths): Promise<boolean> {
  const pool = new WorkerPool();
  const temporary = paths.results === undefined && paths.shard === undefined ? mkdtempSync(join(tmpdir(), "rerail-conformance-")) : undefined;
  try {
    if (paths.shard !== undefined) {
      const { shard, out } = paths.shard;
      const started = performance.now();
      const jobs = await evaluateShard(pool, directories, options, shard, out);
      console.log(`shard ${shard.index}/${shard.count}: ${jobs} evaluation(s) of ${directories.length} pack(s) on ${pool.size} worker(s) in ${Math.round((performance.now() - started) / 1000)} s, results in ${out}`);
      return true;
    }
    if (temporary !== undefined) await evaluateShard(pool, directories, options, { index: 1, count: 1 }, temporary);
    let failed = false;
    const scored = new Map<string, { report: T4Report; verdicts: ToolPackVerdict[] }>();
    for (const { report: reported, verdicts } of reportPacks(pool, directories, options, paths.results ?? temporary!)) {
      const report = await reported;
      print(report, false);
      scored.set(report.pack, { report, verdicts: await verdicts });
      failed ||= !report.pass;
    }
    return (await gate(scored, paths.base, paths.out)) && !failed;
  } finally {
    pool.close();
    if (temporary !== undefined) rmSync(temporary, { recursive: true, force: true });
  }
}

async function main(): Promise<void> {
  const { values, directories, shard } = parseCli(process.argv.slice(2));
  if (values.help) {
    console.log(USAGE);
    return;
  }
  // With no source chosen, every source; --check chooses every source too.
  const everySource = values.check || (!values.fixtures && !values.samples);
  const options: PlanOptions = {
    ...(values.rule === undefined ? {} : { rules: values.rule }),
    fixtures: everySource || values.fixtures,
    samples: everySource || values.samples,
    date: values.date,
  };
  if (values.check) {
    const paths: CheckPaths = {
      ...(shard === undefined ? {} : { shard: { shard, out: values["results-out"]! } }),
      ...(values.results === undefined ? {} : { results: values.results }),
      ...(values.base === undefined ? {} : { base: values.base }),
      ...(values.out === undefined ? {} : { out: values.out }),
    };
    if (!(await check(directories, options, paths))) process.exitCode = 1;
    return;
  }
  let failed = false;
  for (const directory of directories) {
    const report = await runPack(loadPack(directory), options);
    print(report, values.rule !== undefined);
    failed ||= !report.pass;
    if (values.write) {
      mkdirSync(REPORTS_DIRECTORY, { recursive: true });
      const path = join(REPORTS_DIRECTORY, `${report.pack}-t4-rules-${report.date}.json`);
      writeFileSync(path, reportJson(report));
      console.log(`  wrote ${relative(RERAIL_ROOT, path)}`);
    }
  }
  if (failed) process.exitCode = 1;
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error: unknown) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 2;
  });
}
