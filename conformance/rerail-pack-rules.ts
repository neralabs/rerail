/**
 * The validation contract's checks on a pack's rules written in `.rerail` (docs/pack-format.md, "Rules written in
 * .rerail"): the engine's reading of each rule file (check 1), its proof (check 2), and its examples and `.test` blocks
 * run (check 3). pack-validation.ts calls these beside its checks on JSON rules.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, matchesGlob, relative, resolve, sep } from "node:path";
import {
  describeRerailRules,
  loadNativeBinding,
  runRerailPolicies,
  unsupportedRerailRule,
  type RerailDescription,
  type RrDescription,
  type RuleMode,
} from "../src/engine/index.js";

/** One example of a rule written in `.rerail`: its own file, and the other files of its repository. */
export interface RerailExample {
  readonly path?: string | undefined;
  readonly text: string;
  readonly with?: readonly { readonly path: string; readonly text: string }[] | undefined;
}

/** What check 1 makes of a listed `.rerail` document: the engine's reading of it, or why the pack cannot ship it. */
export type RerailListed = { readonly kind: "rerail"; readonly described: RerailDescription } | { readonly kind: "failed"; readonly message: string };

/** The engine's reading of each listed `.rerail` document, by its path (absolute), in one engine call for the pack. */
export function describeListed(paths: readonly string[]): ReadonlyMap<string, RrDescription> {
  const rows = describeRerailRules(paths);
  return new Map(paths.map((path, at) => [path, rows[at]!]));
}

/**
 * Check 1 on a listed `.rerail` document, from the engine's reading `row`: the engine reads and checks it, its
 * `.policy` id is the listed `id`, and it does nothing a pack's `.rerail` rule cannot do yet. `entryMode` is the mode
 * pack.json starts it in, where it names one.
 */
export function rerailListed(id: string, row: RrDescription | undefined, entryMode: RuleMode | undefined): RerailListed {
  if (row === undefined || !row.ok) return { kind: "failed", message: `is refused by the engine: ${row?.error ?? "it was not read"}` };
  if (row.id !== id) return { kind: "failed", message: `has id ${row.id}` };
  const unsupported = unsupportedRerailRule(row, entryMode);
  return unsupported === undefined ? { kind: "rerail", described: row } : { kind: "failed", message: `cannot ship in a pack yet: ${unsupported}` };
}

/**
 * The files under the pack's `directory` that a listed `.rerail` rule includes (the engine's reading of each, `rows`),
 * as pack-relative POSIX paths: a pack's own library file is no rule, and is listed by no rule entry.
 */
export function includedPackFiles(directory: string, rows: Iterable<RrDescription>): ReadonlySet<string> {
  const root = realpathSync(directory);
  const included = new Set<string>();
  for (const row of rows) {
    if (!row.ok) continue;
    for (const file of row.files.slice(1)) {
      const inPack = relative(root, file.path);
      if (!inPack.startsWith("..")) included.add(inPack.split(sep).join("/"));
    }
  }
  return included;
}

/**
 * Check 2 on a listed `.rerail` rule: a pack's own rule has no upstream tool whose comments it honours, so it sets no
 * `suppress`, and it carries at least one `.test` block. Its examples are asked for with every rule's.
 */
export function rerailProofFailures(id: string, described: RerailDescription): string[] {
  return [
    ...(described.suppresses ? [`${id}: a violation sets suppress; a pack's own rule has no upstream tool's comments to honour`] : []),
    ...(described.tests === 0 ? [`${id}: has no .test block; a rule written in .rerail proves itself with at least one`] : []),
  ];
}

/** Whether `path` is in the scope the engine read from the rule: some include glob (any path when none) and no exclude glob. */
function inScope(described: RerailDescription, path: string): boolean {
  const { include, exclude } = described.scope;
  return (include.length === 0 || include.some(glob => matchesGlob(path, glob))) && !exclude.some(glob => matchesGlob(path, glob));
}

/**
 * The example's repository written to a new folder: its own file and the files `with` names. pack.json's schema
 * already refuses a path that leaves it; one that would is refused here too, and nothing is written outside it.
 *
 * The folder is a Git repository with one empty commit, and the example's files stay untracked, as a session's new
 * files are. A rule that reads `git_rev` (a migration rule asks which migrations have not run yet) is withheld where no
 * repository is readable, so without it such a rule's caught example could never fire.
 */
function writeCorpus(path: string, example: RerailExample): string {
  const root = mkdtempSync(join(tmpdir(), "rerail-pack-example-"));
  try {
    initEmptyRepository(root);
    for (const file of [{ path, text: example.text }, ...(example.with ?? [])]) {
      const target = resolve(root, file.path);
      const inside = relative(root, target);
      if (inside === "" || inside.startsWith("..") || isAbsolute(inside)) throw new Error(`${file.path} is not a file inside the example's repository`);
      mkdirSync(dirname(target), { recursive: true });
      writeFileSync(target, file.text);
    }
    return root;
  } catch (error) {
    rmSync(root, { recursive: true, force: true });
    throw error;
  }
}

/** `git init` and one empty commit in `root`, the same every time: no user or system config, no hooks, a fixed author and date. */
function initEmptyRepository(root: string): void {
  const date = "2000-01-01T00:00:00Z";
  const env = {
    ...process.env,
    GIT_CONFIG_GLOBAL: "/dev/null",
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_AUTHOR_NAME: "rerail",
    GIT_AUTHOR_EMAIL: "rerail@example.invalid",
    GIT_AUTHOR_DATE: date,
    GIT_COMMITTER_NAME: "rerail",
    GIT_COMMITTER_EMAIL: "rerail@example.invalid",
    GIT_COMMITTER_DATE: date,
  };
  const git = (...args: string[]): void => {
    execFileSync("git", ["-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", ...args], { cwd: root, env, stdio: "ignore" });
  };
  git("init", "-q", "-b", "main");
  git("commit", "-q", "--allow-empty", "-m", "example base");
}

/** The codes of the rule's findings on `path` in the example's repository, or why the run gave none to judge. */
async function findingsOn(described: RerailDescription, path: string, example: RerailExample): Promise<{ codes: string[] } | { failure: string }> {
  const root = writeCorpus(path, example);
  try {
    const outcome = await runRerailPolicies({ root, policies: [described.path] });
    if (outcome.status === "failed") return { failure: `the run failed: ${outcome.message}` };
    if (outcome.status === "killed") return { failure: `the run was stopped (${outcome.reason}) after ${outcome.wallMs} ms` };
    if (outcome.failures.length > 0) return { failure: `the run failed: ${outcome.failures.map(failure => failure.message).join("; ")}` };
    return { codes: outcome.findings.filter(finding => finding.policy === described.id && finding.path === path).map(finding => finding.code) };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

/** Check 3's judgement of one example from the codes of the rule's findings on its file: caught has one of the rule's codes, passes none. */
function judged(id: string, described: RerailDescription, side: "caught" | "passes", path: string, codes: readonly string[]): string[] {
  if (side === "passes") return codes.length > 0 ? [`${id}: the passes example (${path}) produces ${codes.join(", ")}`] : [];
  return codes.some(code => described.codes.includes(code)) ? [] : [`${id}: the caught example (${path}) produces no ${described.codes.join("/")} finding`];
}

/** Check 3 on one example of a `.rerail` rule: in scope, run, and judged on its own file only. */
async function rerailExampleFailures(id: string, described: RerailDescription, side: "caught" | "passes", example: RerailExample): Promise<string[]> {
  const path = example.path;
  if (path === undefined) return [`${id}: the ${side} example has no path`];
  if (!inScope(described, path)) return [`${id}: the ${side} example's path ${path} is outside the rule's scope`];
  const found = await findingsOn(described, path, example);
  return "failure" in found ? [`${id}: the ${side} example (${path}): ${found.failure}`] : judged(id, described, side, path, found.codes);
}

/**
 * Check 3 on a `.rerail` rule's example pair, each run through the `.rerail` runner on a repository of its own file
 * (and the files `with` names): the caught example gives a finding of this rule on its file, with one of its codes,
 * and the passes example none.
 */
export async function rerailExamplePairFailures(id: string, described: RerailDescription, examples: Record<"caught" | "passes", RerailExample>): Promise<string[]> {
  const caught = await rerailExampleFailures(id, described, "caught", examples.caught);
  return [...caught, ...await rerailExampleFailures(id, described, "passes", examples.passes)];
}

/** Check 3 on a `.rerail` rule file's own `.test` blocks, each run by the engine (`rerail rules test <file>`): every one passes. */
export function rerailTestFailures(id: string, path: string): string[] {
  const results = JSON.parse(loadNativeBinding().CodeRuleDslRuntime.testRr(JSON.stringify([path]), null)) as { test: string; line: number; status: string; failures: string[] }[];
  return results
    .filter(result => result.status !== "pass")
    .map(result => `${id}: its .test ${result.test} (line ${result.line}) fails: ${result.failures.join("; ")}`);
}
