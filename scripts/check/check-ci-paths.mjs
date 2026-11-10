#!/usr/bin/env node
// The path filters that let a pull request skip a CI job still cover everything the job imports. ci.yml's `changes`
// job runs dorny/paths-filter, and each filter below gates the jobs it names: a pull request whose files match no glob
// of a filter skips them. A job may be skipped only when nothing it reads changed, so this recomputes each gated job's
// TypeScript/JavaScript import closure from its entry points (esbuild's metafile, node_modules left out: the lockfile
// is in every filter) and fails, naming the file, the filter and a file that imports it, when a file of the closure
// matches no glob of the filter. It also checks, per filter:
//   - the `changes` output fails open: `'true'` off pull requests, past the API's 3000-file listing cap, or when the
//     filter step left the output empty (EXPECTED_OUTPUT);
//   - the filter matches every file all jobs share (COMMON);
//   - each job it names needs `changes` and is skipped by its output, and nothing in ci.yml reads an output this check
//     does not know (UNGUARDED lists the filters it leaves alone, with why);
//   - every file `reads` lists exists and matches it;
// and that ci-result needs every other job, since a skipped job passes only because it does.
//
// What the check cannot see: files a job reads at runtime rather than imports (a path built with `join`, a spawned
// script, a fixture directory, the dashboard's `ui/`). `reads` holds the ones found by reading the jobs' suites and
// scripts, so a filter narrowed below them fails; a new runtime read is the change author's to add, to the filter and
// to `reads`.
//
// Globs are read as paths-filter reads them (picomatch, `dot: true`) for the subset ci.yml uses: `*`, `?` and `**` as a
// whole segment. A filter that uses anything else (braces, classes, negation, change types) fails here rather than be
// read differently.
//
//   node scripts/check/check-ci-paths.mjs   (npm run check:ci-paths; part of npm run check)
import { existsSync, globSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { parse } from "yaml";
import { globToRegExp } from "./check-structure/lib.mjs";

export const WORKFLOW = ".github/workflows/ci.yml";

/** One file each glob every gated filter must hold has to match: what every job installs, builds or runs with. */
export const COMMON = [
  ".github/workflows/ci.yml",
  ".github/actions/node-deps/action.yml",
  "package.json",
  "package-lock.json",
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  ".cargo/config.toml",
  ".gitattributes",
];

/** What the package build compiles (tsc over tsconfig.build.json's `include`): the jobs that run the built package. */
const PACKAGE_BUILD = ["src/**/*.ts"];

/** The files the dashboard and set-up read from the package at runtime, besides the engine: hand-checked. */
const PACKAGE_READS = ["ui/index.html", "model/manifest.json", "native/Cargo.toml", "languages/python.json", "tsconfig.json"];

/**
 * The packs the dashboard browser tests install from this checkout (`INSTALLED_PACKS` in e2e/support/scenarios.ts),
 * as one file of each, so a pack added there and not to the `e2e` filter fails here.
 */
export function e2eInstalledPackReads(root) {
  const source = readFileSync(resolve(root, "e2e/support/scenarios.ts"), "utf8");
  const list = /const INSTALLED_PACKS = \[([^\]]*)\]/u.exec(source);
  if (list === null) throw new Error("e2e/support/scenarios.ts: INSTALLED_PACKS not found; update e2eInstalledPackReads");
  return [...list[1].matchAll(/"([^"]+)"/gu)].map(([, id]) => `packs/${id}/package.json`);
}

/**
 * Each gated filter of ci.yml's `changes` job: the jobs it skips, the entry points of what they run (globs, every one
 * of which must match a file), and files they read at runtime that no import reaches (a list, or a function of the
 * repository root). The tools job's conformance steps are gated by `conformance` too; its script tests and reference
 * examples run on every pull request.
 */
export const GROUPS = {
  conformance: {
    jobs: ["conformance", "conformance-report"],
    // cli.ts scores and judges; shards.ts forks worker.ts.
    entries: ["conformance/languages/cli.ts", "conformance/languages/worker.ts"],
    // The packs and their locks (the test data and samples they name), the lists `--base` compares, the engine.
    reads: [
      "packs/python/package.json",
      "packs/python/upstream/ruff/test-data.lock",
      "conformance/languages/ungated-rules.json",
      "conformance/languages/pending.json",
      "native/Cargo.toml",
      "languages/python.json",
      "tsconfig.json",
    ],
  },
  e2e: {
    jobs: ["ui-e2e"],
    // Playwright loads the config and the specs; each worker runs seed-scenario.ts and the daemon from source.
    entries: ["playwright.config.ts", "e2e/**/*.ts", "e2e/support/register-tsx.mjs", "src/cli/daemon-entry.ts", ...PACKAGE_BUILD],
    reads: (root) => [...PACKAGE_READS, ...e2eInstalledPackReads(root)],
  },
  integration: {
    jobs: ["integration", "installed-packs"],
    // vitest over integration/ (installed-packs runs one of its files), the OMP host extension the build:test-host
    // step bundles, and the built package.
    entries: ["vitest.config.ts", "integration/**/*.ts", ...PACKAGE_BUILD],
    // The dashboard page daemon-cli.test.ts compares, the model the OMP host loads, Codex's hooks file, the OMP host
    // the job provisions, the pack claude-hook-rules.test.ts adds from this checkout.
    reads: [...PACKAGE_READS, "hooks/codex.json", "scripts/host/install-omp-host.mjs", "packs/complexity/package.json"],
  },
  sessions: {
    jobs: ["real-sessions", "provider-smoke"],
    // The real-session runs, the host-auth child host-harness.ts starts, the OMP host install, the built package.
    entries: ["scripts/host/host-lifecycle.ts", "scripts/host/host-smoke.ts", "scripts/host/host-auth.ts", "scripts/host/install-omp-host.mjs", ...PACKAGE_BUILD],
    // Claude Code and Cursor load this checkout as a plugin directory, Codex its hooks file.
    reads: [...PACKAGE_READS, "hooks/codex.json", ".claude-plugin/plugin.json", ".cursor-plugin/plugin.json", ".codex-plugin/plugin.json", "skills/rerail-create/SKILL.md"],
  },
};

/** Filters of the `changes` job this check leaves alone, and why. */
export const UNGUARDED = {
  rust: "the Rust jobs build and test crates; cargo's inputs are no TypeScript import closure",
};

/** The `changes` output of filter `name`: the filter's answer on a pull request it could list whole, else `'true'`. */
export const expectedOutput = (name) =>
  `\${{ github.event_name == 'pull_request' && github.event.pull_request.changed_files < 3000 && steps.filter.outputs.${name} || 'true' }}`;

/** Why `glob` would be read differently here than by paths-filter, or undefined. */
function unsupportedGlob(glob) {
  if (typeof glob !== "string") return "is not a plain glob (change-type filters are not supported)";
  if (/[{}[\]()!]/u.test(glob)) return "uses glob syntax this check does not read as picomatch does";
  if (glob.split("/").some((segment) => segment.includes("**") && segment !== "**")) return "uses ** inside a segment";
  return undefined;
}

/** Every file `entries` (repository-relative globs) import, as `{ path: importer }`, the entries mapped to null. */
async function importClosure(root, entries) {
  const entryPoints = [];
  const empty = [];
  for (const entry of entries) {
    const matched = globSync(entry, { cwd: root }).filter((path) => !path.endsWith(".d.ts"));
    if (matched.length === 0) empty.push(entry);
    entryPoints.push(...matched);
  }
  const { metafile } = await build({
    absWorkingDir: root,
    entryPoints: [...new Set(entryPoints)].sort(),
    bundle: true,
    packages: "external",
    platform: "node",
    format: "esm",
    metafile: true,
    write: false,
    outdir: "out",
    logLevel: "silent",
  });
  const importer = new Map();
  for (const [path, input] of Object.entries(metafile.inputs)) {
    if (!importer.has(path)) importer.set(path, null);
    for (const imported of input.imports) {
      if (imported.external || imported.path.startsWith("<")) continue;
      if (!importer.get(imported.path)) importer.set(imported.path, path);
    }
  }
  return { importer, empty };
}

function ciResultProblems(workflow) {
  const required = [workflow.jobs["ci-result"]?.needs ?? []].flat();
  return Object.keys(workflow.jobs)
    .filter((name) => name !== "ci-result" && !required.includes(name))
    .map((job) => ({ group: "ci-result", message: `ci-result does not need job ${job}, so its failure would not fail CI result` }));
}

function unknownOutputProblems(workflowText, groups, unguarded) {
  return [...new Set([...workflowText.matchAll(/needs\.changes\.outputs\.([\w-]+)/gu)].map(([, name]) => name))]
    .filter((reference) => !(reference in groups) && !(reference in unguarded))
    .map((reference) => ({ group: reference, message: `${WORKFLOW} gates on changes output ${reference}, which this check does not know: add it to GROUPS` }));
}

/** What a filter's globs match, or undefined after reporting a glob this check cannot read. */
function filterMatcher(items, report) {
  const globs = items.flat(Infinity);
  const bad = globs.map((glob) => [glob, unsupportedGlob(glob)]).filter(([, reason]) => reason !== undefined);
  for (const [glob, reason] of bad) report(`filter glob ${JSON.stringify(glob)} ${reason}`);
  if (bad.length > 0) return undefined;
  const patterns = globs.map((glob) => globToRegExp(glob));
  return (path) => patterns.some((pattern) => pattern.test(path));
}

function reportFilterBasics(changes, name, matches, report) {
  if (changes.outputs?.[name] !== expectedOutput(name)) report(`changes output ${name} must be ${expectedOutput(name)}, so it fails open`);
  for (const path of COMMON) if (!matches(path)) report(`the ${name} filter misses a file every job reads`, path);
}

function reportJobGate(workflow, job, name, report) {
  const definition = workflow.jobs?.[job];
  const needs = [definition?.needs ?? []].flat();
  if (definition === undefined) report(`job ${job} is not in ${WORKFLOW}`);
  else if (!needs.includes("changes") || !String(definition.if ?? "").includes(`needs.changes.outputs.${name} != 'false'`)) {
    report(`job ${job} must need changes and be skipped when needs.changes.outputs.${name} is 'false'`);
  }
}

function reportRuntimeReads(root, group, name, matches, report) {
  const reads = typeof group.reads === "function" ? group.reads(root) : group.reads;
  for (const path of reads) {
    if (!existsSync(resolve(root, path))) report("a runtime read this check lists no longer exists: update reads", path);
    else if (!matches(path)) report(`read at runtime by ${group.jobs.join(", ")}, matches no glob of the ${name} filter`, path);
  }
}

async function reportImportClosure(root, group, name, matches, report) {
  const { importer, empty } = await importClosure(root, group.entries);
  for (const entry of empty) report(`entry point glob matches no file: update entries`, entry);
  for (const [path, from] of [...importer].sort(([a], [b]) => a.localeCompare(b))) {
    if (path.split("/").includes("node_modules") || matches(path)) continue;
    report(`${from === null ? "an entry point" : `imported by ${from}`}, matches no glob of the ${name} filter`, path);
  }
}

/** Every way one filter group falls short of what its jobs read. */
async function groupProblems(root, { workflow, changes, filters }, name, group) {
  const problems = [];
  const report = (message, path) => problems.push({ group: name, ...(path === undefined ? {} : { path }), message });
  const items = filters?.[name];
  if (!Array.isArray(items)) {
    report(`${WORKFLOW}: the changes job has no ${name} filter`);
    return problems;
  }
  const matches = filterMatcher(items, report);
  if (matches === undefined) return problems;
  reportFilterBasics(changes, name, matches, report);
  for (const job of group.jobs) reportJobGate(workflow, job, name, report);
  reportRuntimeReads(root, group, name, matches, report);
  await reportImportClosure(root, group, name, matches, report);
  return problems;
}

/**
 * Every way ci.yml's path filters under `root` fall short of what the jobs they gate read, as `{ group, path?,
 * message }`.
 */
export async function ciPathProblems(root, groups = GROUPS, unguarded = UNGUARDED) {
  const workflowText = readFileSync(resolve(root, WORKFLOW), "utf8");
  const workflow = parse(workflowText);
  const changes = workflow?.jobs?.changes;
  const step = changes?.steps?.find((candidate) => candidate.id === "filter");
  if (typeof step?.with?.filters !== "string") {
    return [{ group: "changes", message: `${WORKFLOW}: the changes job has no paths-filter step with id filter` }];
  }
  const filters = parse(step.with.filters);
  const problems = [...ciResultProblems(workflow), ...unknownOutputProblems(workflowText, groups, unguarded)];
  for (const [name, group] of Object.entries(groups)) problems.push(...await groupProblems(root, { workflow, changes, filters }, name, group));
  return problems;
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const problems = await ciPathProblems(root);
  for (const { group, path, message } of problems) console.error(`${path === undefined ? "" : `${path}: `}[${group}] ${message}`);
  console.log(
    problems.length === 0
      ? `check-ci-paths: the ${Object.keys(GROUPS).join(", ")} filters in ${WORKFLOW} cover what their jobs import and read`
      : `check-ci-paths: ${problems.length} problem(s); widen the filter in ${WORKFLOW} or fix the entry points here`,
  );
  process.exit(problems.length === 0 ? 0 : 1);
}
