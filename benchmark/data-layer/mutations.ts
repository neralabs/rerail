// The mutation stage of `just rerail-sql-loop` (and `loop.ts mutations`): native and SQL over each ported rule's base
// (the whole tree, or the loop's subset), then over each recorded mutation (`<policies>/<linter>/mutations/`) overlaid
// on it, compared with each other. Native is the oracle here, not the linter. A mutation is an incremental apply of
// its changed files on the two workspaces that already hold the base, after which the base is restored. The subset
// has findings only for the clauses that fire on the clean tree; the mutations make others fire, so this is where
// those clauses' true positives are compared.
//
//   mutations.ts child --addon A --corpus C --policies P --linter L [--subset S] [--only m1,m2]: one rule, one JSON line per case
import { type ChildProcess, spawn } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { collectRepository, modulePath } from "./corpus.js";
import { compareStrings, loadEngine, loadRules, RuleWorkspace, type SourceFile } from "./engine.js";
import { loadLinterOracle } from "./oracle.js";
import { compareLines, linesOf, parseLine, type ClauseParity } from "./parity.js";

const script = fileURLToPath(import.meta.url);

/** The base's own case: the rule over the clean base, before any mutation. */
export const BASE = "(base)";

/**
 * Which mutations run: every recorded one, or a selection (`linter → mutation names`; a linter it leaves out runs its
 * base only) with the clauses each selected mutation is there for (`linter/mutation → linter/clause`s).
 */
export type MutationSet = { kind: "all" } | { kind: "selection"; path: string; mutations: Record<string, string[]>; covers: Record<string, string[]> };

const selectionSchema = z.object({
  format: z.literal("rerail.sql-loop-mutations/v1"),
  /** How the selection was made, and from which run. */
  method: z.string(),
  mutations: z.record(z.string(), z.array(z.string())),
  covers: z.record(z.string(), z.array(z.string())),
});
type Selection = z.infer<typeof selectionSchema>;

export function readMutationSelection(path: string): MutationSet {
  const selection = selectionSchema.parse(JSON.parse(readFileSync(path, "utf8")));
  return { kind: "selection", path, mutations: selection.mutations, covers: selection.covers };
}

const caseEventSchema = z.object({
  kind: z.literal("case"),
  linter: z.string(),
  name: z.string(),
  /** Mutated Python files the rule accepts (the base: every accepted file). */
  files: z.number(),
  nativeMs: z.number(),
  sqlMs: z.number(),
  /** Native's violations by clause (`linter/clause`) over the base under this case. */
  findings: z.record(z.string(), z.number()),
  /** Native's violations by clause that the clean base does not have: the findings the mutation makes. */
  added: z.record(z.string(), z.number()),
  /** Native lines compared (budget failures' files apart). */
  compared: z.number(),
  differing: z.array(z.object({ path: z.string(), onlyNative: z.array(z.string()), onlySql: z.array(z.string()) })),
});
const childEventSchema = z.discriminatedUnion("kind", [
  caseEventSchema,
  z.object({ kind: z.literal("done"), linter: z.string(), setupMs: z.number(), wallMs: z.number() }),
]);
export type MutationCase = z.infer<typeof caseEventSchema>;

function byClause(linter: string, lines: readonly string[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const line of lines.map(parseLine)) if (line.kind === "violation") counts[`${linter}/${line.clause}`] = (counts[`${linter}/${line.clause}`] ?? 0) + 1;
  return counts;
}

/** Both engines' lines after one apply, and each apply's wall time. */
interface Applied {
  nativeMs: number;
  sqlMs: number;
  native: string[];
  sql: string[];
}

/** One rule: both engines over the base (the tree, or the subset), then each selected mutation overlaid and restored; a case line after each. */
async function child(argv: readonly string[]): Promise<void> {
  const value = (name: string) => {
    const index = argv.indexOf(name);
    return index < 0 ? undefined : argv[index + 1];
  };
  const linter = value("--linter")!;
  const policies = value("--policies")!;
  const only = value("--only")?.split(",").filter(name => name.length > 0);
  const started = performance.now();
  const engine = loadEngine(value("--addon"));
  const [rule] = loadRules(engine, policies, [linter]);
  const repository = collectRepository(engine, value("--corpus")!);
  const oracle = loadLinterOracle(policies, linter);
  if (oracle === undefined) throw new Error(`${linter}: no recorded mutations under ${policies}`);
  if (repository.git.commit !== oracle.dataLayerCommit) throw new Error(`${repository.root} is at ${repository.git.commit}, but ${linter}'s mutations were recorded at ${oracle.dataLayerCommit}`);
  const mutations = only === undefined ? oracle.mutations : oracle.mutations.filter(mutation => only.includes(mutation.name));
  const unknown = only?.filter(name => !oracle.mutations.some(mutation => mutation.name === name)) ?? [];
  if (unknown.length > 0) throw new Error(`${linter}: no recorded mutation ${unknown.join(", ")}`);
  // The subset manifest's `files[].path`, or every tracked Python file.
  const subset = value("--subset");
  const basePaths = subset === undefined ? undefined : new Set(z.object({ files: z.array(z.object({ path: z.string() })) }).parse(JSON.parse(readFileSync(subset, "utf8"))).files.map(file => file.path));
  const base = basePaths === undefined ? repository.files : repository.files.filter(file => basePaths.has(file.path));
  const byPath = new Map(base.map(file => [file.path, file]));
  const native = new RuleWorkspace(engine, [rule!]);
  const sql = new RuleWorkspace(engine, [rule!], { engine: "sql" });
  // Compiles the SQL program outside the tree's apply.
  sql.engineReport();
  const setupMs = performance.now() - started;
  const emit = (event: z.infer<typeof childEventSchema>) => new Promise<void>(done => process.stdout.write(`${JSON.stringify(event)}\n`, () => done()));
  // One apply at a time per process: the addon answers a second concurrent apply WORKSPACE_BUSY.
  const both = async (files: readonly SourceFile[], removed: readonly string[] = []): Promise<Applied> => {
    const left = await native.apply(files, removed);
    const right = await sql.apply(files, removed);
    return { nativeMs: left.wallMs, sqlMs: right.wallMs, native: linesOf(left.result), sql: linesOf(right.result) };
  };
  const accepted = base.filter(file => native.accepts(file.path));
  const clean = await both(accepted);
  const baseLines = new Set(clean.native);
  const report = async (name: string, files: number, applied: Applied) => {
    const parity: ClauseParity = compareLines(linter, applied.native, applied.sql);
    await emit({
      kind: "case", linter, name, files, nativeMs: applied.nativeMs, sqlMs: applied.sqlMs,
      findings: byClause(linter, applied.native),
      added: byClause(linter, applied.native.filter(line => !baseLines.has(line))),
      compared: parity.compared,
      differing: parity.files,
    });
  };
  await report(BASE, accepted.length, clean);
  for (const mutation of mutations) {
    const overlay = [...mutation.files]
      .filter(([path]) => path.endsWith(".py") && native.accepts(path))
      .map(([path, source]): SourceFile => ({ path, language: "python", source, ...modulePath(engine, repository.root, path) }));
    await report(mutation.name, overlay.length, await both(overlay));
    await both(overlay.flatMap(file => byPath.get(file.path) ?? []), overlay.filter(file => !byPath.has(file.path)).map(file => file.path));
  }
  await emit({ kind: "done", linter, setupMs, wallMs: performance.now() - started });
}

export interface MutationOptions {
  addon: string;
  corpus: string;
  policies: string;
  set: MutationSet;
  /** The base: the loop's subset manifest, or (undefined) the whole tree. */
  subset: string | undefined;
  /** Rules evaluated at once, one child process each. */
  jobs: number;
  /** A rule's child is killed past this many wall seconds. */
  capSeconds: number;
}

export interface MutationRun {
  wallMs: number;
  cases: MutationCase[];
  /** Rules whose child failed or passed its cap, with why. */
  errors: { linter: string; detail: string }[];
  /** Each rule's child: its setup (addon, rule, corpus, SQL program) and its whole wall time. */
  rules: { linter: string; setupMs: number; wallMs: number }[];
}

/** Every rule with recorded mutations, each in its own child, `jobs` at a time, the most mutations first. */
export async function runMutations(options: MutationOptions): Promise<MutationRun> {
  const started = performance.now();
  const manifestSchema = z.object({ mutations: z.record(z.string(), z.unknown()) });
  // Most recorded mutations first.
  const linters = readdirSync(options.policies, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && existsSync(resolve(options.policies, entry.name, "manifest.json")))
    .map(entry => ({ linter: entry.name, mutations: Object.keys(manifestSchema.parse(JSON.parse(readFileSync(resolve(options.policies, entry.name, "manifest.json"), "utf8"))).mutations).length }))
    .sort((left, right) => right.mutations - left.mutations || compareStrings(left.linter, right.linter))
    .map(entry => entry.linter);
  const run: MutationRun = { wallMs: 0, cases: [], errors: [], rules: [] };
  const children = new Set<ChildProcess>();
  const kill = () => {
    for (const process of children) process.kill("SIGKILL");
  };
  process.once("exit", kill);
  let next = 0;
  const slot = async () => {
    for (let index = next++; index < linters.length; index = next++) {
      const linter = linters[index]!;
      const only = options.set.kind === "all" ? [] : ["--only", (options.set.mutations[linter] ?? []).join(",")];
      const subset = options.subset === undefined ? [] : ["--subset", options.subset];
      const spawned = spawn(process.execPath, [
        ...process.execArgv, script, "child", "--addon", options.addon, "--corpus", options.corpus, "--policies", options.policies, "--linter", linter, ...subset, ...only,
      ], { stdio: ["pipe", "pipe", "pipe"] });
      children.add(spawned);
      let stderr = "";
      spawned.stderr!.on("data", chunk => {
        stderr = (stderr + String(chunk)).slice(-4000);
      });
      let done = false;
      const timer = setTimeout(() => {
        run.errors.push({ linter, detail: `killed past ${options.capSeconds} s wall` });
        spawned.kill("SIGKILL");
      }, options.capSeconds * 1000);
      createInterface({ input: spawned.stdout! }).on("line", line => {
        const event = childEventSchema.parse(JSON.parse(line));
        if (event.kind === "case") run.cases.push(event);
        else {
          done = true;
          run.rules.push({ linter, setupMs: event.setupMs, wallMs: event.wallMs });
        }
      });
      const code = await new Promise<number | null>(resolveExit => spawned.on("close", resolveExit));
      clearTimeout(timer);
      children.delete(spawned);
      if (!done && !run.errors.some(error => error.linter === linter)) run.errors.push({ linter, detail: `exited ${code}: ${stderr.trim().split("\n").slice(-3).join(" | ")}` });
    }
  };
  try {
    await Promise.all(Array.from({ length: Math.min(options.jobs, linters.length) }, slot));
  } finally {
    kill();
    process.off("exit", kill);
  }
  run.wallMs = performance.now() - started;
  run.cases.sort((left, right) => compareStrings(left.linter, right.linter) || (left.name === BASE ? -1 : right.name === BASE ? 1 : compareStrings(left.name, right.name)));
  return run;
}

const coverSchema = z.looseObject({ clauses: z.array(z.looseObject({ clause: z.string(), hit: z.string().nullable() })) });

/** The clauses (`linter/clause`) the subset has a finding file for: `hit` in a distillation's `cover.json` (`--cover`). */
export function readSubsetHits(path: string): Set<string> {
  return new Set(coverSchema.parse(JSON.parse(readFileSync(path, "utf8"))).clauses.flatMap(entry => (entry.hit === null ? [] : [entry.clause])));
}

export interface MutationSummary {
  mutations: number;
  /** Mutations that make native find something the clean base does not have. */
  withFindings: number;
  /** Clauses a mutation makes native find something for (`linter/clause`). */
  truePositiveClauses: string[];
  /** Of those, the clauses the subset has no finding file for: this is their first true positive. */
  newlyCovered: string[];
  /** Cases (bases included) where SQL's lines differ from native's. */
  differing: MutationCase[];
  /** A selection's clauses that none of its mutations gives a true positive any more: the selection is stale. */
  lost: string[];
}

export function summarizeMutations(run: MutationRun, set: MutationSet, subsetHits: ReadonlySet<string>): MutationSummary {
  const mutations = run.cases.filter(entry => entry.name !== BASE);
  const truePositiveClauses = [...new Set(mutations.flatMap(entry => Object.keys(entry.added)))].sort(compareStrings);
  const found = new Set(truePositiveClauses);
  return {
    mutations: mutations.length,
    withFindings: mutations.filter(entry => Object.keys(entry.added).length > 0).length,
    truePositiveClauses,
    newlyCovered: truePositiveClauses.filter(clause => !subsetHits.has(clause)),
    differing: run.cases.filter(entry => entry.differing.length > 0),
    lost: set.kind === "all" ? [] : [...new Set(Object.values(set.covers).flat())].filter(clause => !found.has(clause)).sort(compareStrings),
  };
}

/**
 * The fewest mutations (greedy set cover) that give every clause the subset has no finding file for its first true
 * positive, as far as `run` found one: each step takes the mutation adding the most such clauses, the cheaper (native
 * plus SQL apply) on a tie.
 */
export function selectMutations(run: MutationRun, subsetHits: ReadonlySet<string>, method: string): Selection {
  const candidates = run.cases.filter(entry => entry.name !== BASE).map(entry => ({ entry, clauses: Object.keys(entry.added).filter(clause => !subsetHits.has(clause)), cost: entry.nativeMs + entry.sqlMs }));
  const needed = new Set(candidates.flatMap(candidate => candidate.clauses));
  const selection: Selection = { format: "rerail.sql-loop-mutations/v1", method, mutations: {}, covers: {} };
  while (needed.size > 0) {
    let best: { entry: MutationCase; gained: string[]; cost: number } | undefined;
    for (const candidate of candidates) {
      const gained = candidate.clauses.filter(clause => needed.has(clause));
      if (best === undefined || gained.length > best.gained.length || (gained.length === best.gained.length && candidate.cost < best.cost)) best = { entry: candidate.entry, gained, cost: candidate.cost };
    }
    for (const clause of best!.gained) needed.delete(clause);
    (selection.mutations[best!.entry.linter] ??= []).push(best!.entry.name);
    selection.covers[`${best!.entry.linter}/${best!.entry.name}`] = best!.gained.sort(compareStrings);
  }
  for (const names of Object.values(selection.mutations)) names.sort(compareStrings);
  selection.mutations = Object.fromEntries(Object.entries(selection.mutations).sort(([left], [right]) => compareStrings(left, right)));
  selection.covers = Object.fromEntries(Object.entries(selection.covers).sort(([left], [right]) => compareStrings(left, right)));
  return selection;
}

/** The stage's report: its counts, then each differing case's lines (up to `show` cases). */
export function printMutations(run: MutationRun, summary: MutationSummary, label: string, show: number): void {
  const slowest = [...run.rules].sort((left, right) => right.wallMs - left.wallMs)[0];
  console.log(`MUTATIONS ${summary.differing.length === 0 && run.errors.length === 0 ? "OK" : summary.differing.length > 0 ? "DIFFER" : "ERROR"}: ${label}; ${summary.mutations} mutations and ${run.rules.length} bases compared, ${summary.withFindings} mutations with a finding the base lacks; ${summary.truePositiveClauses.length} clauses get a true positive from a mutation, ${summary.newlyCovered.length} of them none from the subset; ${summary.differing.length} cases differ; ${(run.wallMs / 1000).toFixed(1)} s wall (slowest rule ${slowest === undefined ? "—" : `${slowest.linter} ${(slowest.wallMs / 1000).toFixed(1)} s`})`);
  for (const error of run.errors) console.log(`  ERROR ${error.linter}: ${error.detail}`);
  if (summary.lost.length > 0) console.log(`  the selection is stale: ${summary.lost.length} of its clauses get no true positive (${summary.lost.slice(0, show).join(", ")}); re-select with \`just rerail-sql-loop-tool mutations --select\``);
  for (const entry of summary.differing.slice(0, show)) {
    console.log(`  ${entry.linter} ${entry.name}: ${entry.differing.length} files differ`);
    for (const file of entry.differing.slice(0, 3)) {
      console.log(`    @ ${file.path || "(no file)"}: ${file.onlyNative.length} only native, ${file.onlySql.length} only SQL`);
      for (const line of file.onlyNative.slice(0, 3)) console.log(`      - native: ${line.slice(0, 240)}`);
      for (const line of file.onlySql.slice(0, 3)) console.log(`      + sql:    ${line.slice(0, 240)}`);
    }
  }
}

if (resolve(process.argv[1] ?? "") === script && process.argv[2] === "child") {
  await child(process.argv.slice(3));
  // The addon's worker threads keep the process alive; every line is flushed by now.
  process.exit(0);
}
