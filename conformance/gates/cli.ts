/**
 * `npm run gates`: the rule gates over every gated tool-pack, from its recordings, and the summary CI posts.
 * Exit 1 when a shipped rule fails a blocking check (or the summary has a blocking failure), 2 on an exception.
 */
import { existsSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

import { PACKAGE_ROOT } from "../../src/engine/index.js";
import { DEFAULT_CACHE_ROOT } from "./corpus.js";
import { goldensPath } from "./goldens.js";
import { toolPackDirectory } from "./manifest.js";
import { mergeDirectory } from "./merge.js";
import { ProcessCommandRunner } from "./process.js";
import { renderMarkdown } from "./report.js";
import { blockingFailures, gatedToolPacks, runToolPack } from "./run.js";
import { parseShard, shardName } from "./shard.js";
import { writeSummary } from "./summary.js";

const USAGE = `usage:
  npm run gates -- [--pack <id>] [--tool <tool>] [--shard <i>/<n>] [--cache dir] [--no-cache] [--write] [--out dir]
  npm run gates -- merge <dir> [--write]
  npm run gates -- summary <dir> [--base <sha>]
`;

const DEFAULT_OUT = join(PACKAGE_ROOT, ".rerail", "reports", "gates");

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
  const flags = parse(args, ["--pack", "--tool", "--shard", "--cache", "--out"], ["--no-cache", "--write"]);
  const root = PACKAGE_ROOT;
  const shard = typeof flags["--shard"] === "string" ? parseShard(flags["--shard"]) : undefined;
  // Shards run side by side; each writes its promotions to its own report directory, and merge --write commits them.
  if (shard !== undefined && flags["--write"] === true) throw new Error(`--write does not take --shard: merge the shards' reports with gates merge <dir> --write\n${USAGE}`);
  const selected = gatedToolPacks(root).filter(({ pack, tool }) => (flags["--pack"] === undefined || flags["--pack"] === pack) && (flags["--tool"] === undefined || flags["--tool"] === tool));
  if (selected.length === 0) throw new Error(`no gated tool-pack matches${flags["--pack"] === undefined ? "" : ` --pack ${String(flags["--pack"])}`}${flags["--tool"] === undefined ? "" : ` --tool ${String(flags["--tool"])}`}`);
  const out = join(resolve(typeof flags["--out"] === "string" ? flags["--out"] : DEFAULT_OUT), shard === undefined ? "" : shardName(shard));
  const cacheRoot = resolve(typeof flags["--cache"] === "string" ? flags["--cache"] : DEFAULT_CACHE_ROOT);
  await mkdir(out, { recursive: true });
  const runner = new ProcessCommandRunner();
  let failing = 0;
  for (const { pack, tool } of selected) {
    const { report, goldens, goldensChanged } = await runToolPack({ root, pack, tool, cacheRoot, useCache: flags["--no-cache"] !== true, runner, ...(shard === undefined ? {} : { shard }) });
    const name = `${pack}--${tool}`;
    await writeFile(join(out, `${name}.json`), `${JSON.stringify(report, null, 2)}\n`);
    await writeFile(join(out, `${name}.md`), renderMarkdown(report));
    if (goldensChanged) {
      // The artifact always holds the updated goldens; only --write changes the committed file.
      await writeFile(join(out, `${name}.goldens.tsv`), goldens);
      const committed = goldensPath(toolPackDirectory(root, pack, tool));
      if (flags["--write"] === true && (existsSync(committed) || report.uncommittedPromotions > 0)) await writeFile(committed, goldens);
    }
    const blocking = blockingFailures(report);
    failing += blocking;
    console.log(`${report.toolPack}${shard === undefined ? "" : ` (${shardName(shard)}, ${report.rules.length} of ${report.shard!.rules} rules)`}: ${report.summary.shipped.pass}/${report.summary.shipped.rules} shipped pass, ${report.summary.candidates.pass}/${report.summary.candidates.rules} candidates pass; ${blocking} blocking failure(s); cache ${report.cache.hits}/${report.cache.rules}; tool-pack checks ${report.toolPackChecks.pass ? "pass" : "FAIL"}`);
  }
  console.log(`wrote ${selected.length} report(s) to ${out}`);
  return failing > 0 ? 1 : 0;
}

async function merge(args: string[]): Promise<number> {
  const [directory, ...rest] = args;
  if (directory === undefined || directory.startsWith("--")) throw new Error(USAGE);
  const flags = parse(rest, [], ["--write"]);
  const { merged, problems } = await mergeDirectory(PACKAGE_ROOT, resolve(directory), flags["--write"] === true);
  for (const report of merged) console.log(`${report.toolPack}: merged ${report.rules.length} rules; ${report.summary.shipped.pass}/${report.summary.shipped.rules} shipped pass, ${report.summary.candidates.pass}/${report.summary.candidates.rules} candidates pass; ${blockingFailures(report)} blocking failure(s)`);
  for (const problem of problems) process.stderr.write(`not merged: ${problem}\n`);
  console.log(`merged ${merged.length} sharded tool-pack(s) in ${resolve(directory)}`);
  // 2: a tool-pack's shards are missing or disagree, so it has no report (the summary fails it); 1: a shipped rule fails.
  return problems.length > 0 ? 2 : merged.some((report) => blockingFailures(report) > 0) ? 1 : 0;
}

async function summary(args: string[]): Promise<number> {
  const [directory, ...rest] = args;
  if (directory === undefined || directory.startsWith("--")) throw new Error(USAGE);
  const flags = parse(rest, ["--base"], []);
  const result = await writeSummary(PACKAGE_ROOT, resolve(directory), typeof flags["--base"] === "string" ? flags["--base"] : undefined, new ProcessCommandRunner());
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
  process.stderr.write(`conformance/gates/cli.ts: ${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 2;
});
