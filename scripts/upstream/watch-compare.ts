/**
 * What changed upstream between a tool-pack's pinned commit and a release's, in the files its `watch:` block names
 * (conformance/languages/watch.ts): the tests the generator reads (an ESLint-family file with its RuleTester case diff),
 * the ported rules' code, the code they share, and tests upstream added for a ported rule that the generator does not
 * read. Ruff's and Clippy's rule code is found through their indexes (`codes.rs`, `declared_lints.rs`), and the tests
 * they add through their test lists (`#[test_case]` in a plugin's `mod.rs`, `//~` annotations). Static throughout:
 * upstream's files are read through the GitHubClient and parsed, never run.
 */
import { pathGlob } from "../s3/pack-fixtures.mjs";
import { watchMatcher, type ExpandedWatch, type WatchPatternSet } from "../../conformance/languages/watch.js";
import { blobsMatching, changedFiles, type FileChange, type GitHubClient } from "./github.js";
import { compareCases, extractCases, type CaseCounts } from "./rule-tester.js";
import {
  CLIPPY_DECLARED_LINTS,
  CLIPPY_LINTS,
  RUFF_CODES,
  RUFF_FIXTURES,
  RUFF_REGISTRY,
  RUFF_RULES,
  clippyAnnotatedLints,
  clippyDeclaredLints,
  phpNamedFiles,
  ruffCodes,
  ruffTestCases,
  type RuffRule,
} from "./watch-indexes.js";

/** A changed test file the generator reads: the rule it is watched for, and its case diff (ESLint family only). */
export interface TestFileChange extends FileChange {
  rule: string | null;
  cases: CaseCounts | null;
}

/** A changed file of the ported rules' code, and which ported rules it is the code of. */
export interface CodeFileChange extends FileChange {
  rules: string[];
}

/** A test upstream added for a ported rule (null when the watch cannot say which) that the generator does not read. */
export interface NewTest {
  rule: string | null;
  path: string;
}

/** What changed upstream in a tool-pack's watched files between two commits. */
export interface WatchChanges {
  tests: { files: TestFileChange[]; cases: CaseCounts | null };
  code: { rules: string[]; files: CodeFileChange[] };
  shared: { files: FileChange[] };
  newTests: NewTest[];
}

/** What compareWatch compares: an upstream repository's watched files at two commits. */
export interface WatchComparison {
  repository: string;
  /** Whether the tests are ESLint RuleTester files, whose cases are diffed. */
  ruleTester: boolean;
  watch: ExpandedWatch;
  from: string;
  to: string;
}

function byBytes(a: string, b: string): number {
  return Buffer.compare(Buffer.from(a), Buffer.from(b));
}

function sortedUnique(items: Iterable<string>): string[] {
  return [...new Set(items)].sort(byBytes);
}

/** The rules of `set` whose own patterns match `path`, byte-sorted. */
function rulesMatching(set: WatchPatternSet, path: string): string[] {
  return Object.entries(set.byRule).filter(([, patterns]) => patterns.some((pattern) => (pattern.includes("*") ? pathGlob(pattern).test(path) : pattern === path))).map(([rule]) => rule);
}

/** A file's text at both commits: null where it does not exist. */
async function bothTexts(github: GitHubClient, comparison: WatchComparison, file: FileChange): Promise<[string | null, string | null]> {
  return await Promise.all([
    file.change === "added" ? null : github.file(comparison.repository, comparison.from, file.path),
    file.change === "deleted" ? null : github.file(comparison.repository, comparison.to, file.path),
  ]);
}

/** One index's view of the ported rules' code: the extra paths to read, and which rules a changed file is the code of. */
interface CodeIndex {
  patterns: string[];
  /** The ported rules whose index entry differs between the commits, with the index file. */
  moved: { path: string; rules: string[] } | null;
  rulesOf(github: GitHubClient, file: FileChange): Promise<string[]>;
  /** Tests added for ported rules that the index's own test lists show. */
  newTests(github: GitHubClient, changes: FileChange[], isTest: (path: string) => boolean): Promise<NewTest[]>;
}

/** An index file at both commits; throws when it is missing at either. */
async function indexTexts(github: GitHubClient, comparison: WatchComparison, path: string): Promise<[string, string]> {
  const [before, after] = await Promise.all([github.file(comparison.repository, comparison.from, path), github.file(comparison.repository, comparison.to, path)]);
  if (before === null || after === null) throw new Error(`${comparison.repository} has no ${path} at ${before === null ? comparison.from : comparison.to}`);
  return [before, after];
}

/** Ruff: each ported code's plugin and struct (codes.rs), the struct's file found by its name in the plugin's rules/. */
async function ruffIndex(github: GitHubClient, comparison: WatchComparison): Promise<CodeIndex> {
  const [[codesBefore, codesAfter], [registryBefore, registryAfter]] = await Promise.all([indexTexts(github, comparison, RUFF_CODES), indexTexts(github, comparison, RUFF_REGISTRY)]);
  const [before, after] = [ruffCodes(codesBefore, registryBefore), ruffCodes(codesAfter, registryAfter)];
  const ported = [...new Map(comparison.watch.rules.flatMap((code) => [before.get(code), after.get(code)].filter((rule): rule is RuffRule => rule !== undefined).map((rule) => [`${code} ${rule.plugin} ${rule.struct}`, { code, ...rule }] as const))).values()];
  const missing = comparison.watch.rules.filter((code) => !before.has(code));
  if (missing.length > 0) throw new Error(`${RUFF_CODES} at the pin has no ${missing.join(", ")}`);
  const moved = comparison.watch.rules.filter((code) => JSON.stringify(before.get(code)) !== JSON.stringify(after.get(code)));
  const plugins = sortedUnique(ported.map((rule) => rule.plugin));
  return {
    patterns: plugins.map((plugin) => `${RUFF_RULES}/${plugin}/**`),
    moved: moved.length === 0 ? null : { path: RUFF_CODES, rules: moved },
    async rulesOf(client, file) {
      const candidates = ported.filter((rule) => file.path.startsWith(`${RUFF_RULES}/${rule.plugin}/rules/`));
      if (candidates.length === 0) return [];
      const texts = await bothTexts(client, comparison, file);
      return sortedUnique(candidates.filter((rule) => texts.some((text) => text !== null && new RegExp(`\\b${rule.struct}\\b`, "u").test(text))).map((rule) => rule.code));
    },
    async newTests(client, changes, isTest) {
      const found: NewTest[] = [];
      for (const plugin of plugins) {
        const file = changes.find((change) => change.path === `${RUFF_RULES}/${plugin}/mod.rs`);
        if (file === undefined) continue;
        const [old, fresh] = await bothTexts(client, comparison, file);
        const known = new Set(ruffTestCases(old ?? "").map((entry) => `${entry.struct}\u0000${entry.file}`));
        for (const entry of ruffTestCases(fresh ?? "")) {
          const path = `${RUFF_FIXTURES}/${plugin}/${entry.file}`;
          if (known.has(`${entry.struct}\u0000${entry.file}`) || isTest(path)) continue;
          for (const rule of ported.filter((candidate) => candidate.plugin === plugin && candidate.struct === entry.struct)) found.push({ rule: rule.code, path });
        }
      }
      return found;
    },
  };
}

/** Clippy: each ported lint's module (declared_lints.rs), its code that module's file or directory. */
async function clippyIndex(github: GitHubClient, comparison: WatchComparison): Promise<CodeIndex> {
  const [before, after] = (await indexTexts(github, comparison, CLIPPY_DECLARED_LINTS)).map(clippyDeclaredLints) as [Map<string, string>, Map<string, string>];
  const missing = comparison.watch.rules.filter((lint) => !before.has(lint));
  if (missing.length > 0) throw new Error(`${CLIPPY_DECLARED_LINTS} at the pin declares no ${missing.join(", ")}`);
  const modules = new Map<string, string[]>();
  for (const lints of [before, after]) for (const [lint, module] of lints) modules.set(module, sortedUnique([...(modules.get(module) ?? []), lint]));
  const ported = comparison.watch.rules.flatMap((lint) => sortedUnique([before.get(lint), after.get(lint)].filter((module): module is string => module !== undefined)).map((module) => ({ lint, module })));
  const moved = comparison.watch.rules.filter((lint) => before.get(lint) !== after.get(lint));
  return {
    patterns: sortedUnique(ported.flatMap(({ module }) => [`${CLIPPY_LINTS}/${module}.rs`, `${CLIPPY_LINTS}/${module}/**`])),
    moved: moved.length === 0 ? null : { path: CLIPPY_DECLARED_LINTS, rules: moved },
    rulesOf(_client, file) {
      const name = file.path.split("/").pop()!.replace(/\.rs$/u, "");
      return Promise.resolve(sortedUnique(ported.filter(({ lint, module }) => {
        if (file.path === `${CLIPPY_LINTS}/${module}.rs`) return true;
        if (!file.path.startsWith(`${CLIPPY_LINTS}/${module}/`)) return false;
        // A file named after another lint of the module is that lint's code; any other file (mod.rs, utils.rs) is every lint's.
        return name === lint || !(modules.get(module) ?? []).includes(name);
      }).map(({ lint }) => lint)));
    },
    async newTests(client, changes, isTest) {
      const isNew = watchMatcher(comparison.watch.newTests);
      const lints = new Set(comparison.watch.rules);
      const found: NewTest[] = [];
      for (const file of changes.filter((change) => change.change === "added" && change.path.endsWith(".rs") && isNew(change.path) && !isTest(change.path))) {
        const text = await client.file(comparison.repository, comparison.to, file.path);
        for (const lint of [...clippyAnnotatedLints(text ?? "")].filter((name) => lints.has(name)).sort(byBytes)) found.push({ rule: lint, path: file.path });
      }
      return found;
    },
  };
}

/** The tests' changes, with an ESLint-family file's case diff. */
async function testChanges(github: GitHubClient, comparison: WatchComparison, changes: FileChange[]): Promise<WatchChanges["tests"]> {
  const isTest = watchMatcher(comparison.watch.tests);
  const files = await Promise.all(changes.filter((change) => isTest(change.path)).map(async (change): Promise<TestFileChange> => {
    const rule = rulesMatching(comparison.watch.tests, change.path)[0] ?? null;
    if (!comparison.ruleTester || rule === null) return { ...change, rule, cases: null };
    const [old, fresh] = await bothTexts(github, comparison, change);
    const cases = compareCases(old === null ? [] : extractCases(old, change.path, rule), fresh === null ? [] : extractCases(fresh, change.path, rule));
    return { ...change, rule, cases };
  }));
  const cases = comparison.ruleTester ? files.reduce<CaseCounts>((sum, file) => ({
    added: sum.added + (file.cases?.added ?? 0),
    removed: sum.removed + (file.cases?.removed ?? 0),
    changed: sum.changed + (file.cases?.changed ?? 0),
    unresolved: sum.unresolved + (file.cases?.unresolved ?? 0),
  }), { added: 0, removed: 0, changed: 0, unresolved: 0 }) : null;
  return { files, cases };
}

/**
 * `comparison.watch` with the files each named test (`namedTests`) names at either commit added to its tests, each
 * for that test's rules: a file the test stops or starts naming is still compared.
 */
async function withNamedTests(github: GitHubClient, comparison: WatchComparison): Promise<ExpandedWatch> {
  const { watch } = comparison;
  const named = await Promise.all(Object.entries(watch.namedTests).map(async ([test, rules]) => {
    const texts = await Promise.all([comparison.from, comparison.to].map((commit) => github.file(comparison.repository, commit, test)));
    return { rules, files: texts.flatMap((text) => (text === null ? [] : phpNamedFiles(test, text))) };
  }));
  if (named.length === 0) return watch;
  const byRule = new Map(Object.entries(watch.tests.byRule).map(([rule, patterns]) => [rule, [...patterns]]));
  for (const { rules, files } of named) for (const rule of rules) byRule.set(rule, [...(byRule.get(rule) ?? []), ...files]);
  const tests: WatchPatternSet = {
    paths: sortedUnique([...watch.tests.paths, ...named.flatMap(({ files }) => files)]),
    globs: watch.tests.globs,
    byRule: Object.fromEntries([...byRule.keys()].sort(byBytes).map((rule) => [rule, sortedUnique(byRule.get(rule)!)])),
  };
  return { ...watch, tests };
}

/** The watched files' changes between `request.from` and `request.to`. */
export async function compareWatch(github: GitHubClient, request: WatchComparison): Promise<WatchChanges> {
  if (request.from === request.to) return { tests: { files: [], cases: request.ruleTester ? { added: 0, removed: 0, changed: 0, unresolved: 0 } : null }, code: { rules: [], files: [] }, shared: { files: [] }, newTests: [] };
  const comparison: WatchComparison = { ...request, watch: await withNamedTests(github, request) };
  const { watch } = comparison;
  const indexes = await Promise.all(watch.code.indexes.map((index) => (index === "ruff-codes" ? ruffIndex(github, comparison) : clippyIndex(github, comparison))));
  const patterns = sortedUnique([watch.tests, watch.code, watch.shared, watch.newTests].flatMap((set) => [...set.paths, ...set.globs]).concat(indexes.flatMap((index) => index.patterns)));
  const [before, after] = await Promise.all([blobsMatching(github, comparison.repository, comparison.from, patterns), blobsMatching(github, comparison.repository, comparison.to, patterns)]);
  const changes = changedFiles(before, after);
  const [isTest, isCode, isShared, isNew] = [watch.tests, watch.code, watch.shared, watch.newTests].map((set) => watchMatcher(set)) as [(path: string) => boolean, (path: string) => boolean, (path: string) => boolean, (path: string) => boolean];

  const tests = await testChanges(github, comparison, changes);

  const codeFiles: CodeFileChange[] = [];
  for (const change of changes) {
    if (isTest(change.path)) continue;
    const own = isCode(change.path) ? rulesMatching(watch.code, change.path) : [];
    const indexed = (await Promise.all(indexes.map((index) => index.rulesOf(github, change)))).flat();
    const rules = sortedUnique([...own, ...indexed]);
    if (isCode(change.path) || rules.length > 0) codeFiles.push({ ...change, rules });
  }
  for (const { moved } of indexes) if (moved !== null) codeFiles.push({ path: moved.path, change: "modified", rules: moved.rules });
  codeFiles.sort((a, b) => byBytes(a.path, b.path));
  const counted = new Set([...tests.files, ...codeFiles].map((file) => file.path));
  const shared = changes.filter((change) => isShared(change.path) && !counted.has(change.path));

  const newTests: NewTest[] = [];
  if (indexes.length === 0) {
    for (const change of changes.filter((entry) => entry.change === "added" && isNew(entry.path) && !isTest(entry.path))) {
      const rules = rulesMatching(watch.newTests, change.path);
      newTests.push(...(rules.length === 0 ? [{ rule: null, path: change.path }] : rules.map((rule) => ({ rule, path: change.path }))));
    }
  }
  for (const index of indexes) newTests.push(...await index.newTests(github, changes, isTest));
  newTests.sort((a, b) => byBytes(a.path, b.path) || byBytes(a.rule ?? "", b.rule ?? ""));

  return { tests, code: { rules: sortedUnique(codeFiles.flatMap((file) => file.rules)), files: codeFiles }, shared: { files: shared }, newTests };
}
