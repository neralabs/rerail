// `just rerail-data-layer <command>`: the nera-data-layer benchmark. The ported linter rules and their recorded oracles
// live outside the repository (`--policies`, one `<linter>/` directory each); so does the corpus (`--corpus`, a clean
// nera-data-layer checkout). See benchmark/README.md.
import { realpathSync } from "node:fs";
import { resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";
import { collectRepository } from "./corpus.js";
import { runConformance } from "./conformance.js";
import { loadEngine, loadRules, type Engine, type Matcher } from "./engine.js";
import { passIds, runPass, runPasses, writePassResult, writeProfile } from "./profile.js";
import { reportWholeTree, runWholeTree, runWholeTreeChildren } from "./whole-tree.js";

const processStarted = performance.now();
const processCpu = process.cpuUsage();
const script = fileURLToPath(import.meta.url);
const commands = ["whole-tree", "profile", "conformance", "profile-pass"] as const;
type Command = typeof commands[number];

class UsageError extends Error {}

interface Options {
  command: Command;
  addon: string | undefined;
  capabilities: string | undefined;
  corpus: string;
  policies: string;
  oracles: string;
  output: string | undefined;
  rules: string[];
  passes: string[];
  runs: number;
  repeat: number;
  jobs: number;
  top: number;
  show: number;
  allowLimits: boolean;
  matcher: Matcher;
  childRun: number | undefined;
  passResult: string | undefined;
}

const valued = ["--addon", "--capabilities", "--corpus", "--policies", "--oracles", "--output", "--rule", "--pass", "--runs", "--repeat", "--jobs", "--top", "--show", "--child-run", "--pass-result", "--engine", "--sql-batch"];

function count(name: string, value: string, minimum: number): number {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < minimum) throw new UsageError(`${name} takes an integer ≥ ${minimum}, not ${value}`);
  return parsed;
}

export function parseOptions(argv: readonly string[], env: NodeJS.ProcessEnv = process.env): Options {
  const [command, ...rest] = argv;
  if (!commands.includes(command as Command)) throw new UsageError(`unknown command ${command ?? "(none)"}`);
  const values: Record<string, string[]> = {};
  let allowLimits = false;
  for (let index = 0; index < rest.length; index += 1) {
    const argument = rest[index]!;
    if (argument === "--allow-limits") {
      allowLimits = true;
      continue;
    }
    const [key, inline] = argument.includes("=") ? [argument.slice(0, argument.indexOf("=")), argument.slice(argument.indexOf("=") + 1)] : [argument, undefined];
    if (!valued.includes(key)) throw new UsageError(`unknown option ${argument}`);
    const value = inline ?? rest[++index];
    if (value === undefined) throw new UsageError(`${key} needs a value`);
    (values[key] ??= []).push(value);
  }
  const last = (key: string) => values[key]?.at(-1);
  const corpus = last("--corpus") ?? env.RERAIL_DATA_LAYER_CORPUS;
  const policies = last("--policies") ?? env.RERAIL_DATA_LAYER_POLICIES;
  if (corpus === undefined) throw new UsageError("--corpus (or RERAIL_DATA_LAYER_CORPUS) is required");
  if (policies === undefined) throw new UsageError("--policies (or RERAIL_DATA_LAYER_POLICIES) is required");
  const output = last("--output");
  if ((command === "whole-tree" || command === "profile") && output === undefined) throw new UsageError(`${command} needs --output <dir>`);
  const engine = last("--engine") ?? "native";
  if (engine !== "native" && engine !== "sql") throw new UsageError(`--engine takes native or sql, not ${engine}`);
  if (engine === "sql" && command !== "whole-tree") throw new UsageError("--engine sql runs whole-tree only");
  const sqlBatch = last("--sql-batch");
  if (sqlBatch !== undefined && engine !== "sql") throw new UsageError("--sql-batch needs --engine sql");
  return {
    command: command as Command,
    addon: last("--addon") ?? env.RERAIL_ENGINE_PATH,
    capabilities: last("--capabilities") === undefined ? undefined : resolve(last("--capabilities")!),
    corpus: resolve(corpus),
    policies: resolve(policies),
    oracles: resolve(last("--oracles") ?? policies),
    output: output === undefined ? undefined : resolve(output),
    rules: values["--rule"] ?? [],
    passes: values["--pass"] ?? [],
    runs: count("--runs", last("--runs") ?? "1", 1),
    repeat: count("--repeat", last("--repeat") ?? "1", 1),
    jobs: count("--jobs", last("--jobs") ?? "1", 1),
    top: count("--top", last("--top") ?? "40", 1),
    show: count("--show", last("--show") ?? "10", 0),
    allowLimits,
    matcher: { engine, ...(sqlBatch === undefined ? {} : { sqlBatchSize: count("--sql-batch", sqlBatch, 1) }) },
    childRun: last("--child-run") === undefined ? undefined : count("--child-run", last("--child-run")!, 1),
    passResult: last("--pass-result"),
  };
}

/** What a child process needs to load the same addon, corpus and rules. */
function childArguments(options: Options, engine: Engine): string[] {
  return [
    "--addon", engine.addon.path, "--capabilities", engine.capabilities.path, "--corpus", options.corpus, "--policies", options.policies,
    ...(options.output === undefined ? [] : ["--output", options.output]),
    ...options.rules.flatMap(rule => ["--rule", rule]),
    ...(options.allowLimits ? ["--allow-limits"] : []),
    "--engine", options.matcher.engine,
    ...(options.matcher.sqlBatchSize === undefined ? [] : ["--sql-batch", String(options.matcher.sqlBatchSize)]),
  ];
}

function usage(): string {
  return [
    "usage: node --import tsx benchmark/data-layer/data-layer.ts <command> --corpus <nera-data-layer> --policies <dir> [options]",
    "",
    "commands:",
    "  whole-tree   one apply of every rule over the whole corpus, in a fresh process per run",
    "               [--runs N] --output <dir>: wall, CPU, counts, sha256 of the sorted finding+failure lines",
    "  profile      single-file applies per pass (baselines, all rules, each rule alone)",
    "               --output <dir> [--repeat N] [--jobs N] [--top N] [--pass <id>]...",
    "  conformance  each rule against the recorded oracles (tree and mutations) [--oracles <dir>] [--show N]",
    "",
    "options:",
    "  --addon <path>     the native addon (else RERAIL_ENGINE_PATH, else native/rerail_engine.<platform>.node)",
    "  --capabilities <path>  the capabilities rules compile against (default: the bundled ones, production's limits)",
    "  --rule <linter>    only these rules (repeatable)",
    "  --allow-limits     accept an addon whose work allowances are bounded (not an `unbounded-work` build)",
    "  --engine <name>    whole-tree: which engine matches AST clauses, native (default) or sql; sql reports the",
    "                     clauses it refuses and digests the lines of the clauses it ran apart",
    "  --sql-batch <n>    files per SQL batch (default 64)",
  ].join("\n");
}

async function main(): Promise<number> {
  const argv = process.argv.slice(2);
  if (argv.length === 0 || argv.includes("--help") || argv.includes("-h")) {
    process.stdout.write(`${usage()}\n`);
    return 0;
  }
  let options: Options;
  try {
    options = parseOptions(argv);
  } catch (error) {
    if (!(error instanceof UsageError)) throw error;
    process.stderr.write(`rerail-data-layer: ${error.message}\n\n${usage()}\n`);
    return 2;
  }
  const engine = loadEngine(options.addon, options.capabilities);
  if (engine.addon.work === "bounded" && !options.allowLimits) {
    process.stderr.write(`rerail-data-layer: ${engine.addon.path} enforces per-file and per-clause work allowances (kernel ${engine.addon.kernelVersion}); build it with \`npm run build:native:unbounded-work\` or pass --allow-limits\n`);
    return 2;
  }
  const rules = loadRules(engine, options.policies, options.rules);
  const repository = collectRepository(engine, realpathSync(options.corpus));
  switch (options.command) {
    case "whole-tree": {
      if (options.childRun !== undefined) {
        const run = await runWholeTree({ engine, matcher: options.matcher, repository, rules, output: options.output!, run: options.childRun, processStarted, processCpu });
        process.stdout.write(`${JSON.stringify(run)}\n`);
        return 0;
      }
      const runs = await runWholeTreeChildren({ script, childArguments: childArguments(options, engine), runs: options.runs });
      return reportWholeTree({ engine, matcher: options.matcher, repository, rules, output: options.output!, runs });
    }
    case "profile-pass": {
      const [id] = options.passes;
      if (id === undefined || options.passResult === undefined) throw new Error("profile-pass needs --pass and --pass-result");
      writePassResult(options.passResult, await runPass(engine, id, rules, repository.files, options.repeat));
      return 0;
    }
    case "profile": {
      const ids = options.passes.length > 0 ? options.passes : passIds(rules);
      if (!ids.includes("all")) ids.push("all");
      const results = await runPasses({ engine, rules, files: repository.files, ids, repeat: options.repeat, jobs: options.jobs, script, childArguments: childArguments(options, engine) });
      writeProfile({ engine, repository, rules, results, output: options.output!, repeat: options.repeat, jobs: options.jobs, top: options.top });
      console.log(`profile: ${resolve(options.output!, "profile.md")}`);
      return 0;
    }
    case "conformance":
      return runConformance({ engine, repository, rules, oracles: options.oracles, show: options.show });
  }
}

if (resolve(process.argv[1] ?? "") === script) process.exitCode = await main();
