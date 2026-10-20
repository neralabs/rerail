import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

import { shardName, type Shard } from "./shard.js";

/**
 * The gate layer's own code, hashed from the working tree: part of every rule's key, except the files below, which
 * decide no cached verdict. The key covers what a cached result can depend on: run.ts's import graph, the files that
 * graph reads by path (corpus-tasks.json), and cli.ts, which chooses the run's options. Left out is what no cached
 * verdict depends on: what only the report job runs or reads (the summary, the pull request comment, the shard merge,
 * the not-gated and pending lists), report rendering (a leg renders report.md from a finished report), what only the
 * recorder runs (recording/diff.ts), the leg plan (plan.mjs, which picks the legs, not what they find), the record plan
 * (recording/record-plan.mjs, which picks the tool-packs a label records) and the README, so changing them re-judges
 * nothing;
 * cache.test.ts checks that run.ts's import graph reaches none of them. An excluded module must stay free of top-level
 * side effects, because cli.ts imports it in every leg.
 */
export const GATE_CODE = [
  "conformance/port-readiness",
  ":(exclude)conformance/port-readiness/summary.ts",
  ":(exclude)conformance/port-readiness/comment.ts",
  ":(exclude)conformance/port-readiness/README.md",
  ":(exclude)conformance/port-readiness/ungated.ts",
  ":(exclude)conformance/port-readiness/ungated-rules.json",
  ":(exclude)conformance/port-readiness/pending.json",
  ":(exclude)conformance/port-readiness/report.ts",
  ":(exclude)conformance/port-readiness/merge.ts",
  ":(exclude)conformance/port-readiness/recording/diff.ts",
  ":(exclude)conformance/port-readiness/plan.mjs",
  ":(exclude)conformance/port-readiness/plan.d.mts",
  ":(exclude)conformance/port-readiness/recording/record-plan.mjs",
  ":(exclude)conformance/port-readiness/recording/record-plan.d.mts",
] as const;

/**
 * The npm packages the gate process loads (through run.ts and the src/engine barrel), plus tsx, which compiles it:
 * a bump of one, or of anything it depends on, can change what a verdict reads (the schema validator, the YAML reader,
 * the SQLite binding), so their locked closure is part of every rule's key (`dependencyDigest`). A package missing
 * here leaves a stale verdict, the opposite of FAMILY_SOURCES (engine.ts), where a missing entry only costs a
 * re-judge; cache.test.ts therefore checks the list against a bundle of run.ts.
 */
export const GATE_PACKAGES = ["apache-arrow", "better-sqlite3", "diff", "drizzle-orm", "semver", "tsx", "yaml", "zod"] as const;

/** Tags every cached entry, so an entry written by another layout of the cache is a miss. */
export const CACHE_FORMAT = "rerail.port-readiness-cache/v1";

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

/** sha256 of every file `git ls-files` lists under `pathspec` at `root`, by path, hashed from the working tree. */
export function trackedDigest(root: string, pathspec: string | readonly string[]): string {
  const listed = execFileSync("git", ["ls-files", "-z", "--", ...(typeof pathspec === "string" ? [pathspec] : pathspec)], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 })
    .split("\0")
    .filter((path) => path !== "")
    .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  return sha256([...new Set(listed)].map((path) => {
    const absolute = join(root, path);
    return `${path}\0${existsSync(absolute) ? sha256(readFileSync(absolute)) : "deleted"}`;
  }).join("\n"));
}

/** One `packages` entry of package-lock.json, as far as the closure reads it. */
interface LockedPackage {
  version?: string;
  integrity?: string;
  resolved?: string;
  link?: boolean;
  dependencies?: Record<string, string>;
  optionalDependencies?: Record<string, string>;
  peerDependencies?: Record<string, string>;
  peerDependenciesMeta?: Record<string, { optional?: boolean }>;
}

/**
 * Every package location `roots` reach in a package-lock.json (lockfileVersion 2 or later), through their
 * dependencies, optional dependencies and peers, each found as npm finds it: in the dependent's own `node_modules`,
 * then in each enclosing one up to the top. The value is the location's version and integrity (or resolved URL).
 *
 * Type packages are skipped: nothing loads them. An optional dependency or optional peer the lockfile does not install
 * is skipped; any other dependency it does not install throws, as does a root it does not install, and so does a
 * workspace link, whose sources the key would not cover.
 */
export function lockedClosure(lock: unknown, roots: readonly string[]): Map<string, string> {
  const { lockfileVersion, packages } = (lock ?? {}) as { lockfileVersion?: unknown; packages?: unknown };
  if (typeof lockfileVersion !== "number" || lockfileVersion < 2 || packages === null || typeof packages !== "object") {
    throw new Error("package-lock.json: expected lockfileVersion 2 or later, with a packages map");
  }
  const installed = packages as Record<string, LockedPackage>;
  const find = (from: string, name: string): string | undefined => {
    for (let at = from; ; at = at.slice(0, Math.max(at.lastIndexOf("/node_modules/"), 0))) {
      const location = at === "" ? `node_modules/${name}` : `${at}/node_modules/${name}`;
      if (Object.hasOwn(installed, location)) return location;
      if (at === "") return undefined;
    }
  };
  const queue = roots.map((root) => {
    const location = `node_modules/${root}`;
    if (!Object.hasOwn(installed, location)) throw new Error(`package-lock.json installs no ${root}`);
    return location;
  });
  const closure = new Map<string, string>();
  while (queue.length > 0) {
    const location = queue.pop()!;
    if (closure.has(location)) continue;
    const entry = installed[location]!;
    if (entry.link === true) throw new Error(`package-lock.json: ${location} links to the workspace ${entry.resolved ?? ""}, whose sources the key does not cover`);
    closure.set(location, `${entry.version ?? ""}\0${entry.integrity ?? entry.resolved ?? ""}`);
    const wanted = [
      ...Object.keys(entry.dependencies ?? {}).map((name) => ({ name, optional: false })),
      ...Object.keys(entry.optionalDependencies ?? {}).map((name) => ({ name, optional: true })),
      ...Object.keys(entry.peerDependencies ?? {}).map((name) => ({ name, optional: entry.peerDependenciesMeta?.[name]?.optional === true })),
    ];
    for (const { name, optional } of wanted) {
      if (name.startsWith("@types/")) continue;
      const found = find(location, name);
      if (found !== undefined) queue.push(found);
      else if (!optional) throw new Error(`package-lock.json: ${location} needs ${name}, which it does not install`);
    }
  }
  return closure;
}

/**
 * The npm side of every rule's key, in the checkout at `root`: the locked closure of GATE_PACKAGES, and the
 * package.json fields on the verdict path. Those are the command CI runs, `npm run port-readiness`, which is
 * `scripts["port-readiness"]` with its `preport-readiness` and `postport-readiness` scripts when there are any;
 * `scripts["build:native"]`, the command that builds the engine binary every verdict is judged on; and `type`, which
 * says how tsx loads every .ts file. Every other package.json field, every other script and every other locked package
 * stays out, so a bump the gates never load re-judges nothing.
 *
 * Some keyed entries are never loaded (json-with-bigint, @opentelemetry/api, the esbuild binaries of other
 * platforms): a bump of one costs a re-judge, never a verdict, which is the safe direction. Left out on purpose: the
 * Node.js version (CI pins only the major, in .github/actions/node-deps/action.yml, which no key covers), @napi-rs/cli
 * (the native binary is no part of the key, engine.ts) and tsconfig.json; the uncached nightly judges every rule
 * again, which catches a change to any of them.
 */
export function dependencyDigest(root: string): string {
  const lock: unknown = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
  const { type, scripts } = JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as { type?: unknown; scripts?: Record<string, unknown> };
  const fields = { type, script: scripts?.["port-readiness"], pre: scripts?.["preport-readiness"], post: scripts?.["postport-readiness"], native: scripts?.["build:native"] };
  const locked = [...lockedClosure(lock, GATE_PACKAGES)].map(([location, value]) => `${location}\0${value}`).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  return sha256([JSON.stringify(fields), ...locked].join("\n"));
}

/** The inputs every rule of a tool-pack shares; each is a digest or the text itself. */
export interface SharedKeyInputs {
  gateCode: string;
  dependencies: string;
  gatesYaml: string;
  recordingJson: string;
  corpusJson: string;
  corpusTasks: string;
  corpora: string;
}

/** The inputs one rule adds. */
export interface RuleKeyInputs {
  /** The engine the rule can be judged by: the shared engine and its own language family's (engine.ts's `engineKey`). */
  engine: string;
  document: string;
  conformanceEntry: string;
  goldens: string;
  caseSources: string[];
}

/**
 * A rule's result key: sha256 over the gate code and its dependencies, the tool-pack's manifest and recordings, the
 * corpus pins, and the rule's own engine key, document, conformance entry, goldens and case sources, each digested in
 * that order. The engine is the rule's own input rather than the tool-pack's because it is only the part of the
 * engine the rule's language family reads.
 */
export function ruleKey(shared: SharedKeyInputs, rule: RuleKeyInputs): string {
  const parts = [
    shared.gateCode, shared.dependencies, shared.gatesYaml, shared.recordingJson, shared.corpusJson, shared.corpusTasks, shared.corpora,
    rule.engine, rule.document, rule.conformanceEntry, rule.goldens, ...rule.caseSources,
  ];
  return sha256(parts.map((part) => sha256(part)).join("\n"));
}

/** What a tool-pack's own checks read of every rule of it, whichever shard runs them. */
export interface ToolPackKeyInputs {
  /** The engine every rule can be judged by: engine.ts's `engineKey` over all their families. */
  engine: string;
  /** Every rule's upstream name: the known gaps are proven on their cases. */
  names: string[];
  /** Every rule's goldens, as `<id>\0<goldens>`: the corpus and corpus-complete cover them all. */
  goldens: string[];
  /** Every committed case source, as `<file>\0<sha256>`: the gap proofs read them. */
  caseSources: string[];
}

/**
 * The key of a run's tool-pack checks (the corpus, corpus-complete and the known gaps): the shared inputs, what the
 * checks read of every rule, and the judged rules' keys, whose results the gap counts come from. A shard judges only
 * its own rules, but these checks read all of them, so a shard's cached checks miss when another shard's rule changes
 * what they read; otherwise one shard replays the old checks, another computes new ones, and the merge refuses them.
 */
export function toolPackKey(shared: SharedKeyInputs, pack: ToolPackKeyInputs, judgedKeys: readonly string[]): string {
  const sorted = (values: readonly string[]) => [...values].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0)).join("\n");
  const parts = [
    shared.gateCode, shared.dependencies, shared.gatesYaml, shared.recordingJson, shared.corpusJson, shared.corpusTasks, shared.corpora,
    pack.engine, sorted(pack.names), sorted(pack.goldens), sorted(pack.caseSources), sorted(judgedKeys),
  ];
  return sha256(parts.map((part) => sha256(part)).join("\n"));
}

/**
 * Where one rule's cached result lives: `<cache>/results/<pack>/<tool>/<key>.json`, or under the shard's directory
 * (`<tool>/shard-<index>of<count>/`) when the tool-pack is sharded, so each CI leg caches its own shard.
 */
export function resultPath(cacheRoot: string, pack: string, tool: string, key: string, shard?: Shard): string {
  return join(cacheRoot, "results", pack, tool, ...(shard === undefined ? [] : [shardName(shard)]), `${key}.json`);
}

/**
 * A cached entry, or undefined when there is none or it is not one `valid` accepts: an entry that is not JSON, was
 * written by another cache layout, or does not have the shape the run reads is a miss, never a verdict.
 *
 * The shape check guards against corruption, not forgery. A pull request run saves its results (port-readiness.yml), and
 * GitHub scopes a pull request's cache to that pull request: main never reads it, and a push to main reads only what
 * main's own runs saved. A pull request could therefore write verdicts its own head then reads back as passes, but it
 * can edit the gate code that decides them just as easily, so reading its own cache opens no route it did not have.
 */
export async function readCached<T>(cacheRoot: string, pack: string, tool: string, key: string, valid: (value: unknown) => value is T, shard?: Shard): Promise<T | undefined> {
  const path = resultPath(cacheRoot, pack, tool, key, shard);
  if (!existsSync(path)) return undefined;
  try {
    const entry: unknown = JSON.parse(await readFile(path, "utf8"));
    if (entry === null || typeof entry !== "object" || (entry as { format?: unknown }).format !== CACHE_FORMAT) return undefined;
    const value = (entry as { value?: unknown }).value;
    return valid(value) ? value : undefined;
  } catch {
    return undefined;
  }
}

export async function writeCached(cacheRoot: string, pack: string, tool: string, key: string, value: unknown, shard?: Shard): Promise<void> {
  const path = resultPath(cacheRoot, pack, tool, key, shard);
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify({ format: CACHE_FORMAT, value })}\n`);
  await rename(temporary, path);
}
