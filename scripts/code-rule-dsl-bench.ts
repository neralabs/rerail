import { readFile, readdir, stat } from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { IGNORED_DIRECTORIES, languageFor } from "../src/code-rule-enforcer.js";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../src/native.js";
import { bundledCodeRuleDslDirectory } from "../src/code-rule-dsl-catalog.js";

/**
 * Throughput check for TypeScript AST rules over a real source tree.
 *
 *   node --import tsx scripts/code-rule-dsl-bench.ts [directory] [--pattern <ast-grep pattern>] [--runs <n>]
 *
 * The directory must lie inside the repository checkout. Defaults to the nera-landing `web/src` tree next to this package and a `console.log($$$ARGS)` rule, which
 * exercises both TypeScript grammars: `.ts` files parse with the TypeScript grammar and `.tsx` with TSX.
 * Evaluation goes through the incremental workspace the enforcer uses, so a file the grammar cannot parse is
 * reported as a per-file failure while every other file still yields violations.
 */

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const REPOSITORY = resolve(ROOT, "..");
type SnapshotFile = { language: "typescript" | "tsx"; path: string; source: string };

function option(name: string, fallback: string): string {
  const index = process.argv.indexOf(name);
  if (index === -1) return fallback;
  const value = process.argv[index + 1];
  if (value === undefined || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

async function collect(directory: string, root: string, files: SnapshotFile[]): Promise<void> {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (!IGNORED_DIRECTORIES.has(entry.name)) await collect(path, root, files);
      continue;
    }
    const language = languageFor(path);
    if (language !== "typescript" && language !== "tsx") continue;
    files.push({ language, path: relative(root, path), source: await readFile(path, "utf8") });
  }
}

function compile(pattern: string): NativeCompiledCodeRuleDsl {
  const runtime = loadNativeBinding().CodeRuleDslRuntime;
  if (runtime === undefined) throw new Error("Native binding does not export CodeRuleDslRuntime; run npm run build:native");
  const codeRule = JSON.stringify({
    clauses: [{
      forbid: { ast: { language: "typescript", rule: { pattern } } },
      id: "bench",
      violation: { anchor: "witness.span", code: "BENCH_MATCH", message: `Matched ${pattern}` },
    }],
    format: "rerail.policy/v1",
    id: "bench:typescript-ast",
    mode: "log",
    revision: 1,
    scope: { exclude: [], include: ["**/*"] },
  });
  return runtime.compile(codeRule, "json", capabilitiesJson);
}

const capabilitiesJson = await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");
const directory = resolve(REPOSITORY, process.argv[2]?.startsWith("--") === false ? process.argv[2] : join("web", "src"));
if (!directory.startsWith(REPOSITORY + sep)) throw new Error(`${directory} is outside the repository checkout`);
const pattern = option("--pattern", "console.log($$$ARGS)");
const runs = Number.parseInt(option("--runs", "5"), 10);
if (!(await stat(directory)).isDirectory()) throw new Error(`${directory} is not a directory`);

const files: SnapshotFile[] = [];
await collect(directory, directory, files);
files.sort((a, b) => a.path.localeCompare(b.path));
const bytes = files.reduce((total, file) => total + Buffer.byteLength(file.source), 0);
const counts = { tsx: files.filter(file => file.language === "tsx").length, typescript: files.filter(file => file.language === "typescript").length };

const compileStarted = performance.now();
const compiled = compile(pattern);
const compileMs = performance.now() - compileStarted;
const request = JSON.stringify({ files, removed: [], revision: 1 });

const timings: number[] = [];
let last: { status: string; violations: unknown[]; failures: Array<{ code: string; path?: string }>; metrics: { visitedNodes: number } } | undefined;
for (let run = 0; run < runs; run += 1) {
  const workspace = new (loadNativeBinding().CodeRuleDslWorkspace)();
  workspace.addCodeRule(compiled);
  const started = performance.now();
  ({ evaluation: last } = JSON.parse(await workspace.applyAsync(request)));
  timings.push(performance.now() - started);
  workspace.close();
}
if (last === undefined) throw new Error("--runs must be at least 1");
timings.sort((a, b) => a - b);
const best = timings[0]!;
const median = timings[Math.floor(timings.length / 2)]!;

console.log(`directory      ${directory}`);
console.log(`pattern        ${pattern}`);
console.log(`files          ${files.length} (${counts.typescript} .ts, ${counts.tsx} .tsx), ${(bytes / 1_048_576).toFixed(2)} MiB`);
console.log(`compile        ${compileMs.toFixed(1)} ms`);
console.log(`evaluate       best ${best.toFixed(1)} ms, median ${median.toFixed(1)} ms over ${runs} runs`);
console.log(`throughput     ${(files.length / (best / 1000)).toFixed(0)} files/s, ${(bytes / 1_048_576 / (best / 1000)).toFixed(1)} MiB/s`);
console.log(`result         ${last.status}, ${last.violations.length} violations, ${last.failures.length} failures, ${last.metrics.visitedNodes} nodes visited`);
for (const failure of last.failures.slice(0, 20)) console.log(`  failure      ${failure.code} ${failure.path ?? ""}`);
