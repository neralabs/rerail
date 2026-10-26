/**
 * `npm run port-readiness`: the port readiness gates over every gated tool-pack, from its recordings, and the summary CI posts.
 * Exit 1 when a shipped rule fails a blocking check, a fresh verdict differs from the cached one under the same key
 * (--refresh-cache), or the summary has a blocking failure; 2 on an exception.
 */
import { mkdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

import { PACKAGE_ROOT } from "../../src/engine/index.js";
import { DEFAULT_CACHE_ROOT } from "./corpus.js";
import { staleNativeBinary } from "./engine.js";
import { checkoutBinary, mergeDirectory } from "./merge.js";
import { ProcessCommandRunner } from "./process.js";
import { driftLine, renderMarkdown } from "./report.js";
import { blockingFailures, gatedToolPacks, runToolPack } from "./run.js";
import { parseShard, shardName } from "./shard.js";
import { parsePlanned, writeSummary } from "./summary.js";

const USAGE = `usage:
  npm run port-readiness -- [--pack <id>] [--tool <tool>] [--shard <i>/<n>] [--cache dir] [--no-cache | --refresh-cache] [--out dir]
  npm run port-readiness -- merge <dir>
  npm run port-readiness -- summary <dir> [--base <sha>] [--planned <JSON array of "<pack>/<tool>">]
`;

const DEFAULT_OUT = join(PACKAGE_ROOT, ".rerail", "reports", "port-readiness");

function parse(args: string[], valued: string[], flags: string[]): Record<string, string | true> {
  const parsed: Record<string, string | true> = {};
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]!;
    if (arg in parsed) throw new Error(`${arg} given twice\n${USAGE}`);
    if (valued.includes(arg)) {
      const value = args[index + 1];
      if (value === undefined || value.startsWith("--")) throw new Error(`${arg} needs a value\n${USAGE}`);
      parsed[arg] = value;
      index += 1;
    } else if (flags.includes(arg)) {
      parsed[arg] = true;
    } else {
      throw new Error(`unknown argument ${arg}\n${USAGE}`);
    }
  }
  return parsed;
}

async function gates(args: string[]): Promise<number> {
  const flags = parse(args, ["--pack", "--tool", "--shard", "--cache", "--out"], ["--no-cache", "--refresh-cache"]);
  const root = PACKAGE_ROOT;
  const shard = typeof flags["--shard"] === "string" ? parseShard(flags["--shard"]) : undefined;
  const selected = gatedToolPacks(root).filter(({ pack, tool }) => (flags["--pack"] === undefined || flags["--pack"] === pack) && (flags["--tool"] === undefined || flags["--tool"] === tool));
  if (selected.length === 0) throw new Error(`no gated tool-pack matches${flags["--pack"] === undefined ? "" : ` --pack ${String(flags["--pack"])}`}${flags["--tool"] === undefined ? "" : ` --tool ${String(flags["--tool"])}`}`);
  const out = join(resolve(typeof flags["--out"] === "string" ? flags["--out"] : DEFAULT_OUT), shard === undefined ? "" : shardName(shard));
  if (flags["--no-cache"] === true && flags["--refresh-cache"] === true) throw new Error(`--no-cache and --refresh-cache exclude each other\n${USAGE}`);
  // A native binary older than the engine sources may be another engine's build, judging under these sources' keys
  // (engine.ts): its verdicts are reported, never saved for later runs.
  const stale = staleNativeBinary(root);
  if (stale !== undefined) console.warn(`the native engine is older than ${stale}: rebuild it with npm run build:native. This run's results are not cached.`);
  // --no-cache neither reads, saves nor compares results; --refresh-cache judges every rule again, compares each fresh
  // result with the cached one under its key (a verdict that differs fails the run) and saves the fresh one. A stale
  // binary's verdicts are compared with nothing, being another build's.
  const readCache = flags["--no-cache"] !== true && flags["--refresh-cache"] !== true;
  const writeCache = flags["--no-cache"] !== true && stale === undefined;
  const compareCache = flags["--refresh-cache"] === true && stale === undefined;
  const cacheRoot = resolve(typeof flags["--cache"] === "string" ? flags["--cache"] : DEFAULT_CACHE_ROOT);
  await mkdir(out, { recursive: true });
  const runner = new ProcessCommandRunner();
  let failing = 0;
  for (const { pack, tool } of selected) {
    const report = await runToolPack({ root, pack, tool, cacheRoot, readCache, writeCache, compareCache, runner, ...(shard === undefined ? {} : { shard }) });
    const name = `${pack}--${tool}`;
    await writeFile(join(out, `${name}.json`), `${JSON.stringify(report, null, 2)}\n`);
    await writeFile(join(out, `${name}.md`), renderMarkdown(report));
    const blocking = blockingFailures(report);
    failing += blocking;
    // Each difference is one of the blocking failures; an annotation names its rule and gate in the run's page.
    for (const drift of report.cache.differing ?? []) console.log(`::error::${report.toolPack}: ${driftLine(drift)}`);
    console.log(`${report.toolPack}${shard === undefined ? "" : ` (${shardName(shard)}, ${report.rules.length} of ${report.shard!.rules} rules)`}: ${report.summary.shipped.pass}/${report.summary.shipped.rules} shipped pass, ${report.summary.candidates.pass}/${report.summary.candidates.rules} candidates pass; ${blocking} blocking failure(s); cache ${report.cache.hits}/${report.cache.rules}${compareCache ? `, compared ${report.cache.compared ?? 0}, ${report.cache.differing?.length ?? 0} differ` : ""}; tool-pack checks ${report.toolPackChecks.pass ? "pass" : "FAIL"}`);
  }
  console.log(`wrote ${selected.length} report(s) to ${out}`);
  return failing > 0 ? 1 : 0;
}

async function merge(args: string[]): Promise<number> {
  const [directory, ...rest] = args;
  if (directory === undefined || directory.startsWith("--")) throw new Error(USAGE);
  parse(rest, [], []);
  // Every shard must be one native binary's run, and the checkout's when it has one (CI's report job has none).
  const { merged, problems } = await mergeDirectory(resolve(directory), checkoutBinary());
  for (const report of merged) console.log(`${report.toolPack}: merged ${report.rules.length} rules; ${report.summary.shipped.pass}/${report.summary.shipped.rules} shipped pass, ${report.summary.candidates.pass}/${report.summary.candidates.rules} candidates pass; ${blockingFailures(report)} blocking failure(s)`);
  for (const problem of problems) process.stderr.write(`not merged: ${problem}\n`);
  console.log(`merged ${merged.length} sharded tool-pack(s) in ${resolve(directory)}`);
  // 2: a tool-pack's shards are missing or disagree, so it has no report (the summary fails it); 1: a shipped rule
  // fails, or a refresh found a fresh verdict that differs from the cached one under the same key.
  return problems.length > 0 ? 2 : merged.some((report) => blockingFailures(report) > 0) ? 1 : 0;
}

async function summary(args: string[]): Promise<number> {
  const [directory, ...rest] = args;
  if (directory === undefined || directory.startsWith("--")) throw new Error(USAGE);
  const flags = parse(rest, ["--base", "--planned"], []);
  // --planned: the tool-packs CI's plan judged; without it every gated tool-pack needs a report. Bad JSON exits 2.
  const planned = typeof flags["--planned"] === "string" ? parsePlanned(flags["--planned"]) : undefined;
  const result = await writeSummary(PACKAGE_ROOT, resolve(directory), typeof flags["--base"] === "string" ? flags["--base"] : undefined, new ProcessCommandRunner(), planned);
  console.log(`${result.shipped.pass}/${result.shipped.rules} shipped rules pass; ${result.blocking.length} blocking failure(s); wrote summary.md and summary.json to ${resolve(directory)}`);
  return result.pass ? 0 : 1;
}

async function main(): Promise<number> {
  const args = process.argv.slice(2);
  if (args[0] === "summary") return await summary(args.slice(1));
  if (args[0] === "merge") return await merge(args.slice(1));
  if (args[0] === "--help" || args[0] === "-h") {
    process.stdout.write(USAGE);
    return 0;
  }
  return await gates(args);
}

main().then((code) => {
  process.exitCode = code;
}, (error: unknown) => {
  process.stderr.write(`conformance/port-readiness/cli.ts: ${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 2;
});
