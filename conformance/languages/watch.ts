/**
 * A gated tool-pack's `watch:` block (`gates.yaml`, docs/pack-format.md "watch") expanded over its ported rules: the
 * upstream paths and globs the nightly upstream report compares between the pinned commit and upstream's latest release.
 * It reads only the pack's conformance.json and the data files the watch names, so the report can load it cheaply.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathGlob } from "../../scripts/s3/pack-fixtures.mjs";
import type { GatesManifest, GatesWatch, WatchIndex } from "./manifest.js";

/** What `{rule}` stands for in a watch pattern. */
const RULE = "{rule}";

/** Watched upstream paths: literal `paths`, `globs` (`*` within a segment, `**` across), and each rule's own patterns. */
export interface WatchPatternSet {
  paths: string[];
  globs: string[];
  /** Each ported rule's patterns: its `{rule}` patterns expanded, its `rules:` override, or its data file's paths. */
  byRule: Record<string, string[]>;
}

/** A tool-pack's watch, every `{rule}` and data file resolved. */
export interface ExpandedWatch {
  /** The rules `{rule}` expanded over, byte-sorted. */
  rules: string[];
  /** The tests the generator reads. */
  tests: WatchPatternSet;
  /**
   * Tests (`{ named-by }`) whose named files (`__DIR__ . '<relative path>'`) the generator reads, by test path, each
   * with the rules it is read for. The report reads them at both commits and adds the files they name to `tests`.
   */
  namedTests: Record<string, string[]>;
  /** The ported rules' code, and the upstream index files that locate it per rule. */
  code: WatchPatternSet & { indexes: WatchIndex[] };
  /** Code every ported rule depends on. */
  shared: { paths: string[]; globs: string[] };
  /** Where upstream adds tests for a ported rule that the generator does not read. */
  newTests: WatchPatternSet;
}

const byBytes = (left: string, right: string) => Buffer.compare(Buffer.from(left), Buffer.from(right));
const sortedUnique = (items: Iterable<string>) => [...new Set(items)].sort(byBytes);
const isGlob = (pattern: string) => pattern.includes("*");

/** Whether an upstream path is one of `set`'s paths or matches one of its globs (`pathGlob`'s semantics). */
export function watchMatcher(set: { paths: readonly string[]; globs: readonly string[] }): (path: string) => boolean {
  const paths = new Set(set.paths);
  const globs = set.globs.map(pathGlob);
  return (path) => paths.has(path) || globs.some((glob) => glob.test(path));
}

/** The rules a pack's conformance.json maps to upstream `tool`, by their upstream names. */
function portedRules(root: string, pack: string, tool: string): string[] {
  const path = join(root, "packs", pack, "conformance.json");
  const conformance = JSON.parse(readFileSync(path, "utf8")) as { rules?: { upstream?: { tool?: string; rule?: string } }[] };
  return (conformance.rules ?? []).flatMap((rule) => (rule.upstream?.tool === tool && typeof rule.upstream.rule === "string" ? [rule.upstream.rule] : []));
}

/** A data file beside the generator: `{ "rules": { "<rule>": ["<upstream path>", …] } }`. */
function readRuleFile(directory: string, file: string, label: string): Record<string, string[]> {
  const parsed: unknown = JSON.parse(readFileSync(join(directory, file), "utf8"));
  const rules = (parsed as { rules?: unknown } | null)?.rules;
  const valid = rules !== null && typeof rules === "object" && !Array.isArray(rules)
    && Object.values(rules).every((paths) => Array.isArray(paths) && paths.every((path) => typeof path === "string" && path.length > 0));
  if (!valid) throw new Error(`${label}: ${file} must be { "rules": { "<rule>": ["<upstream path>", …] } }`);
  return rules as Record<string, string[]>;
}

/** The rules `{rule}` expands over: the ports without `rule-prefix`, plus `extra-rules`. */
function watchedRules(root: string, pack: string, tool: string, watch: GatesWatch, label: string): string[] {
  const prefix = watch["rule-prefix"] ?? "";
  const ported = portedRules(root, pack, tool).map((rule) => {
    if (!rule.startsWith(prefix)) throw new Error(`${label}: watch.rule-prefix is ${prefix}, but conformance.json ports ${rule}`);
    return rule.slice(prefix.length);
  });
  const extra = watch["extra-rules"] ?? [];
  for (const rule of extra) {
    if (ported.includes(rule)) throw new Error(`${label}: watch.extra-rules lists ${rule}, which conformance.json already ports`);
  }
  return sortedUnique([...ported, ...extra]);
}

/** Collects patterns into a WatchPatternSet. */
class PatternSet {
  private readonly all = new Set<string>();
  private readonly perRule = new Map<string, Set<string>>();

  add(pattern: string, rule?: string): void {
    this.all.add(pattern);
    if (rule === undefined) return;
    const own = this.perRule.get(rule) ?? new Set<string>();
    own.add(pattern);
    this.perRule.set(rule, own);
  }

  result(): WatchPatternSet {
    const patterns = sortedUnique(this.all);
    const rules = [...this.perRule.keys()].sort(byBytes);
    return {
      paths: patterns.filter((pattern) => !isGlob(pattern)),
      globs: patterns.filter(isGlob),
      byRule: Object.fromEntries(rules.map((rule) => [rule, sortedUnique(this.perRule.get(rule)!)])),
    };
  }
}

/**
 * Adds `patterns` to `set`: a `{rule}` pattern once per rule without its own `kind` override, the rest as they are;
 * then each override.
 */
function expandPatterns(set: PatternSet, patterns: readonly string[], rules: readonly string[], overrides: Map<string, readonly string[] | undefined>): void {
  for (const pattern of patterns) {
    if (!pattern.includes(RULE)) {
      set.add(pattern);
      continue;
    }
    for (const rule of rules.filter((name) => overrides.get(name) === undefined)) set.add(pattern.replaceAll(RULE, rule), rule);
  }
  for (const [rule, own] of overrides) for (const pattern of own ?? []) set.add(pattern, rule);
}

/**
 * The `watch:` block of `packs/<pack>/upstream/<tool>/gates.yaml` (`manifest`) expanded under `root`, or undefined when
 * it has none. `{rule}` stands for each ported rule (conformance.json's `upstream.rule` for `tool`, without
 * `rule-prefix`, plus `extra-rules`); a rule under `rules:` takes its own `tests` or `code` in place of the `{rule}`
 * patterns; a `{ file }` entry adds each rule's paths from that data file beside the generator, and with `with` each
 * path with its extension replaced by every listed one; a `{ named-by }` entry records that data file's paths as
 * `namedTests`. Throws on a rule the watch names that is not watched.
 */
export function expandWatch(root: string, pack: string, tool: string, manifest: GatesManifest): ExpandedWatch | undefined {
  const watch = manifest.watch;
  if (watch === undefined) return undefined;
  const label = `packs/${pack}/upstream/${tool}/gates.yaml`;
  const rules = watchedRules(root, pack, tool, watch, label);
  const known = new Set(rules);
  const overrides = Object.entries(watch.rules ?? {});
  for (const [rule] of overrides) {
    if (!known.has(rule)) throw new Error(`${label}: watch.rules names ${rule}, which is no watched rule`);
  }

  const tests = new PatternSet();
  const testPatterns = watch.tests.filter((entry): entry is string => typeof entry === "string");
  expandPatterns(tests, testPatterns, rules, new Map(overrides.map(([rule, own]) => [rule, own.tests])));
  const namedTests = new Map<string, Set<string>>();
  for (const entry of watch.tests) {
    if (typeof entry === "string") continue;
    const file = "file" in entry ? entry.file : entry["named-by"];
    for (const [rule, paths] of Object.entries(readRuleFile(join(root, "packs", pack, "upstream", tool), file, label))) {
      if (!known.has(rule)) throw new Error(`${label}: ${file} lists ${rule}, which is no watched rule`);
      for (const path of paths) {
        if ("file" in entry) {
          // The file name's extension (`[^./]` keeps it within the last segment) replaced by each of `with`.
          for (const watched of [path, ...(entry.with ?? []).map((extension) => path.replace(/\.[^./]*$/u, "") + extension)]) tests.add(watched, rule);
        } else {
          namedTests.set(path, new Set([...(namedTests.get(path) ?? []), rule]));
        }
      }
    }
  }

  const code = new PatternSet();
  const codePatterns = watch.code.filter((entry): entry is string => typeof entry === "string");
  expandPatterns(code, codePatterns, rules, new Map(overrides.map(([rule, own]) => [rule, own.code])));
  const indexes = sortedUnique(watch.code.flatMap((entry) => (typeof entry === "string" ? [] : [entry.index]))) as WatchIndex[];

  const newTests = new PatternSet();
  expandPatterns(newTests, watch["new-tests"] ?? [], rules, new Map());

  const shared = sortedUnique(watch.shared);
  return {
    rules,
    tests: tests.result(),
    namedTests: Object.fromEntries([...namedTests.keys()].sort(byBytes).map((path) => [path, sortedUnique(namedTests.get(path)!)])),
    code: { ...code.result(), indexes },
    shared: { paths: shared.filter((pattern) => !isGlob(pattern)), globs: shared.filter(isGlob) },
    newTests: newTests.result(),
  };
}
