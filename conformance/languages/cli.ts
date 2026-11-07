import { mkdirSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { parseArgs } from "node:util";
import { REPORTS_DIRECTORY, listPortedPacks, loadPack, PACKS_ROOT, RERAIL_ROOT } from "./pack.js";
import { reportJson, type T4Report } from "./report.js";
import { reportFailures, runPack } from "./runner.js";

const USAGE = `Language-pack rule conformance (T4).

  node --import tsx conformance/languages/cli.ts --pack <name> [options]
  node --import tsx conformance/languages/cli.ts --check [--pack <name>]

  --pack <name>         pack under packs/<name>/ (or --pack-dir <dir>)
  --fixtures            score the upstream rule fixtures only
  --corpus <name>       score one pinned corpus (repeatable); with neither, fixtures and every corpus
  --corpus-root <dir>   corpora checkouts, <dir>/<corpus name>; default the nearest .rerail/corpora/<pack>
  --rule <id>           score one rule (repeatable); prints every FP and FN
  --write               write .rerail/reports/<pack>-t4-rules-<date>.json (gitignored)
  --date <YYYY-MM-DD>   report date (default today, UTC)
  --check               CI gate: fixtures of every pack (or --pack); exit 1 on any unexplained FP/FN, engine
                        failure, stale fixture divergence, recording error, or shipped port with no fixture TP
                        and no pack.json examples`;

function formatRatio(value: number | null): string {
  return (value === null ? "-" : value.toFixed(3)).padStart(5);
}

function print(report: T4Report, detail: boolean): void {
  console.log(`${report.pack} ${report.tier} ${report.date}  kernel ${report.kernel.version}`);
  for (const source of report.sources) {
    console.log(
      `  ${source.source}: ${source.units} unit(s), ${source.files} file(s), ${source.bytes} bytes, ${source.skipped} skipped, ${source.outsideScope} upstream finding(s) outside scope, ${source.syntaxErrorFiles.count} syntax-error file(s) (T1), ${source.evaluationMs} ms`,
    );
    if (detail) for (const file of source.syntaxErrorFiles.files) console.log(`      SYNTAX ${file.path}:${file.line}:${file.column}`);
  }
  const width = Math.max(4, ...report.rules.map(row => row.rule.length));
  if (report.rules.length > 0) console.log(`  ${"rule".padEnd(width)}  source            tp     fp     fn   prec  recall  div  fail  syn  unanalysed`);
  for (const row of report.rules) {
    console.log(
      `  ${row.rule.padEnd(width)}  ${row.source.padEnd(16)} ${String(row.tp).padStart(4)} ${String(row.fp).padStart(6)} ${String(row.fn).padStart(6)}  ${formatRatio(row.precision)}  ${formatRatio(row.recall)}  ${String(row.divergences.length).padStart(3)}  ${String(row.engineFailures).padStart(4)}  ${String(row.syntaxErrorFiles.count).padStart(3)}  ${String(row.unanalysed).padStart(10)}`,
    );
    if (!detail) continue;
    for (const point of row.falsePositives) console.log(`      FP ${point.path}:${point.line}${point.column === undefined ? "" : `:${point.column}`}`);
    for (const point of row.falseNegatives) console.log(`      FN ${point.path}:${point.line}${point.column === undefined ? "" : `:${point.column}`}`);
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

async function main(): Promise<void> {
  const { values } = parseArgs({
    options: {
      pack: { type: "string" },
      "pack-dir": { type: "string" },
      fixtures: { type: "boolean", default: false },
      corpus: { type: "string", multiple: true, default: [] },
      "corpus-root": { type: "string" },
      rule: { type: "string", multiple: true },
      write: { type: "boolean", default: false },
      date: { type: "string", default: new Date().toISOString().slice(0, 10) },
      check: { type: "boolean", default: false },
      help: { type: "boolean", default: false },
    },
  });
  if (values.help) {
    console.log(USAGE);
    return;
  }
  const directories =
    values["pack-dir"] !== undefined
      ? [resolve(values["pack-dir"])]
      : values.pack !== undefined
        ? [join(PACKS_ROOT, values.pack)]
        : values.check
          ? listPortedPacks().map(name => join(PACKS_ROOT, name))
          : [];
  if (directories.length === 0) throw new Error(`--pack, --pack-dir or --check is required\n\n${USAGE}`);
  if (values.check && (values.corpus.length > 0 || values.write)) throw new Error("--check runs the fixture tier only; it takes neither --corpus nor --write");

  let failed = false;
  for (const directory of directories) {
    const loaded = loadPack(directory);
    const fixturesOnly = values.check || (values.fixtures && values.corpus.length === 0);
    const report = await runPack(loaded, {
      ...(values.rule === undefined ? {} : { rules: values.rule }),
      fixtures: values.check || values.fixtures || values.corpus.length === 0,
      corpora: fixturesOnly ? [] : values.corpus.length > 0 ? values.corpus : loaded.pack.corpora.map(corpus => corpus.name),
      ...(values["corpus-root"] === undefined ? {} : { corpusRoot: resolve(values["corpus-root"]) }),
      date: values.date,
    });
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

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 2;
});
