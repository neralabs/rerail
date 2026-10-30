// Shared by the ESLint-based packs' generators (packs/<id>/upstream/<tool>/generate.mjs): pins, the guard that keeps
// the generating machine out of fixtures, each unit's upstream origin and the ECMA rename variants, and
// ESLint's findings in the expected-findings format `rerail/conformance/languages/pack.ts` reads (`expectedFindingsSchema`).
// Run, through them, by `npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release; never
// pull request CI).
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { homedir, tmpdir } from "node:os";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { expectedFindingsWritten } from "../s3/pack-fixtures.mjs";
import { renameVariant } from "./rename/ecmascript.ts";

export const REPOSITORY = dirname(dirname(dirname(fileURLToPath(import.meta.url))));

/** The working directory upstream tests see while they declare their cases (`withUpstreamTestCwd`): the same everywhere. */
export const UPSTREAM_TEST_CWD = "/work/project";

/**
 * Runs `declare` (importing an upstream test file) with process.cwd() answering UPSTREAM_TEST_CWD, so a test that builds a
 * case's `filename` from the working directory (unicorn's filename-case: `path.join(cwd, …)`) names the same file on
 * every machine.
 */
export async function withUpstreamTestCwd(declare) {
  const realCwd = process.cwd;
  process.cwd = () => UPSTREAM_TEST_CWD;
  try {
    return await declare();
  } finally {
    process.cwd = realCwd;
  }
}

/** `directory`'s segments below its root: `/` separators, no drive letter, no leading or trailing `/`. */
function segmentsOf(directory) {
  return directory.replaceAll("\\", "/").replace(/^[A-Za-z]:/u, "").replace(/^\/+/u, "").replace(/\/+$/u, "");
}

/**
 * This machine's own directories, which no fixture may name: the working directory, home and temp, and their real
 * paths. A directory one segment deep (`/tmp`, `/root`) is too common in upstream test code to tell apart, and is left out.
 */
export function machineDirectories() {
  const directories = new Set();
  for (const directory of [process.cwd(), homedir(), tmpdir()]) {
    directories.add(directory);
    try {
      directories.add(realpathSync(directory));
    } catch {
      // A directory that cannot be resolved still counts as given.
    }
  }
  return [...new Set([...directories].map(segmentsOf))].filter(directory => directory.split("/").length >= 2);
}

const escape = text => text.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");

/**
 * Throws when a case about to be written would carry this machine's directories, in its fixture path or its code: an
 * upstream test that named its file from the generator's working directory, home or temp folder (#31's filename-case
 * fixtures held `/private/tmp/claude-501/…`), which breaks Windows checkouts and leaks the maintainer's machine.
 * Pin the working directory with `withUpstreamTestCwd`, or keep only the name below the test's own fixture directory.
 * `directories` are `machineDirectories()`' form; a match is whole segments, so `/Users/jeff` never matches `/Users/jeffrey`.
 */
export function assertPortableCase(path, code = "", directories = machineDirectories()) {
  const fixturePath = `/${path.replaceAll("\\", "/")}/`;
  const text = code.replaceAll("\\", "/");
  for (const directory of directories) {
    if (fixturePath.includes(`/${directory}/`)) throw new Error(`fixture path ${path} carries this machine's directory /${directory}`);
    if (new RegExp(`/${escape(directory)}(?![^/\\s"'\`)\\]}*,;:])`, "u").test(text)) {
      throw new Error(`fixture ${path}'s code carries this machine's directory /${directory}`);
    }
  }
}

/**
 * Throws when an authored case's expected findings (`reports`, rule name to count) name a rule the expected findings do not
 * run: a misspelled rule would read as no finding and pass whatever the tool reports on the case.
 */
export function assertKnownReportRules(name, reports, rules) {
  for (const rule of Object.keys(reports)) {
    if (!rules.includes(rule)) throw new Error(`AUTHORED ${name}: reports names ${rule}, which is not a pack rule`);
  }
}

/** `require` resolving from a tools directory (`.rerail/tools/<pack>`), after checking every pinned package's version. */
export function pinnedRequire(tools, pins) {
  const require = createRequire(join(tools, "package.json"));
  for (const [name, version] of Object.entries(pins)) {
    const found = JSON.parse(readFileSync(join(tools, "node_modules", name, "package.json"), "utf8")).version;
    if (found !== version) throw new Error(`${name} ${found} under ${tools}; this generator pins ${version}`);
  }
  return require;
}

export function gitHead(directory) {
  return execFileSync("git", ["-C", directory, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
}

export function assertCommit(directory, commit) {
  const head = gitHead(directory);
  if (head !== commit) throw new Error(`${directory} is at ${head}; pinned ${commit}`);
}

/** ESLint's line terminators; the ecma family's lines are ESLint's. */
const LINE_BREAK = /\r\n|[\r\n\u2028\u2029]/u;

/** ESLint's 1-based UTF-16 column → 1-based Unicode code points from the start of the line. */
export function codePointColumn(text, line, column) {
  const lineText = text.split(LINE_BREAK)[line - 1] ?? "";
  return [...lineText.slice(0, column - 1)].length + 1;
}

/**
 * One file's ESLint messages → findings of the listed rules, and whether ESLint could not parse it (a fatal
 * message: such a file is skipped, never scored).
 */
export function findingsOf(result, text, path, rules) {
  const fatal = result.messages.find(message => message.fatal);
  if (fatal !== undefined) return { fatal: `${fatal.line}:${fatal.column} ${fatal.message}`, findings: [] };
  const findings = result.messages
    .filter(message => rules.includes(message.ruleId))
    .map(message => ({ rule: message.ruleId, path, line: message.line, column: codePointColumn(text, message.line, message.column) }));
  return { fatal: undefined, findings };
}

/** Writes expected findings as packs/php/upstream/common.py does: findings sorted, one-space indent. */
export function writeExpectedFindings(path, tool, version, source, findings) {
  const sorted = [...findings].sort(
    (a, b) => (a.rule < b.rule ? -1 : a.rule > b.rule ? 1 : 0) || (a.path < b.path ? -1 : a.path > b.path ? 1 : 0) || a.line - b.line || (a.column ?? 0) - (b.column ?? 0),
  );
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify({ tool, version, source, findings: sorted }, null, 1)}\n`);
  console.log(expectedFindingsWritten(path, source.kind));
}

/**
 * Lints `files` (relative to `cwd`) with exactly `config` (a flat config array; no config file is read) and returns
 * the expected findings for `rules` plus the files ESLint could not parse.
 */
export async function lint(ESLint, cwd, config, files, rules) {
  const eslint = new ESLint({ cwd, overrideConfigFile: true, overrideConfig: config, ignore: false, warnIgnored: false, errorOnUnmatchedPattern: false });
  const findings = [];
  const skipped = [];
  const batch = 200;
  for (let index = 0; index < files.length; index += batch) {
    const results = await eslint.lintFiles(files.slice(index, index + batch).map(file => join(cwd, file)));
    for (const result of results) {
      const path = relative(cwd, result.filePath).split(sep).join("/");
      const text = readFileSync(result.filePath, "utf8");
      const outcome = findingsOf(result, text, path, rules);
      if (outcome.fatal !== undefined) skipped.push({ path, reason: outcome.fatal });
      findings.push(...outcome.findings);
    }
  }
  return { findings, skipped };
}

/** Where a tool's rename variants sit under its fixtures directory: `renamed/<the original's path>`. */
export const RENAMED_DIRECTORY = "renamed";

/** The files the rename rewrites: the ECMA sources. A unit's other files (`package.json`) are copied as they are. */
const ECMA_SOURCE = /\.[cm]?[jt]sx?$/u;

const unitPaths = unit => (typeof unit === "string" ? [unit] : unit);

/** A unit's key in `source.units` (pack.ts `unitKey`): its path, or the first path of a multi-file unit. */
export const unitKey = unit => unitPaths(unit)[0];

/** Whether `path` (relative to the fixtures directory) is a rename variant's file. */
export const isVariantPath = path => path.startsWith(`${RENAMED_DIRECTORY}/`);

/**
 * Writes, under `fixtures/renamed/`, a renamed copy of every unit `origins` names: its ECMA sources through
 * scripts/pack-data/rename/ecmascript.ts (its identifiers, order-preserving; literals kept), its other files
 * copied. Upstream then lints the copy like any unit, and its findings label it. A unit whose rename would change no
 * source, or change one's length or parse, gets no variant. Returns each variant (`of`, the original's key; `unit`,
 * shaped like the original; `paths`, original path to variant path) and the units skipped, with why.
 */
export function writeRenameVariants(fixtures, units, origins) {
  const variants = [];
  const skipped = [];
  for (const unit of units) {
    const of = unitKey(unit);
    if (!origins.has(of)) continue;
    const outputs = [];
    let problem;
    for (const path of unitPaths(unit)) {
      const text = readFileSync(join(fixtures, path), "utf8");
      const renamed = ECMA_SOURCE.test(path) ? renameVariant(text, path) : { source: undefined };
      if (renamed.problem !== undefined && renamed.problem !== "nothing to rename") problem = `${path}: ${renamed.problem}`;
      outputs.push({ path, variant: `${RENAMED_DIRECTORY}/${path}`, source: renamed.source });
    }
    if (problem === undefined && outputs.every(output => output.source === undefined)) problem = "nothing to rename";
    if (problem !== undefined) {
      skipped.push({ of, problem });
      continue;
    }
    for (const { path, variant, source } of outputs) {
      mkdirSync(dirname(join(fixtures, variant)), { recursive: true });
      if (source === undefined) copyFileSync(join(fixtures, path), join(fixtures, variant));
      else writeFileSync(join(fixtures, variant), source);
    }
    const paths = new Map(outputs.map(({ path, variant }) => [path, variant]));
    variants.push({ of, unit: typeof unit === "string" ? paths.get(unit) : unit.map(path => paths.get(path)), paths });
  }
  return { variants, skipped };
}

const isPlainObject = value => typeof value === "object" && value !== null && !Array.isArray(value);

/** `value` without its empty objects (recursively); undefined when nothing is left. */
function withoutEmpty(value) {
  if (!isPlainObject(value)) return value;
  const out = {};
  for (const [key, inner] of Object.entries(value)) {
    const kept = withoutEmpty(inner);
    if (kept !== undefined) out[key] = kept;
  }
  return Object.keys(out).length === 0 ? undefined : out;
}

/**
 * What a unit's `source.units[key].languageOptions` records of the language ESLint linted `path` under, beyond ESLint's
 * flat-config defaults for its extension: its `ecmaVersion`, `sourceType` and `parserOptions` (never `globals`, which
 * no generator applies, nor the parser, which the extension gives). Left out as defaults: `ecmaVersion: "latest"`, the
 * extension's own `sourceType` (`commonjs` for `.cjs` and `.cts`, `module` otherwise) and `ecmaFeatures.jsx` (the
 * extension's: `.jsx` and `.tsx` are JSX). Undefined when nothing is left, so most units carry nothing.
 */
export function unitLanguage(path, languageOptions) {
  if (!isPlainObject(languageOptions)) return undefined;
  const own = {};
  if (languageOptions.ecmaVersion !== undefined && languageOptions.ecmaVersion !== "latest") own.ecmaVersion = languageOptions.ecmaVersion;
  const defaultSourceType = /\.c[jt]s$/u.test(path) ? "commonjs" : "module";
  if (languageOptions.sourceType !== undefined && languageOptions.sourceType !== defaultSourceType) own.sourceType = languageOptions.sourceType;
  if (isPlainObject(languageOptions.parserOptions)) {
    const { ecmaFeatures, ...parserOptions } = JSON.parse(JSON.stringify(languageOptions.parserOptions));
    if (isPlainObject(ecmaFeatures)) {
      const { jsx: _jsx, ...features } = ecmaFeatures;
      parserOptions.ecmaFeatures = features;
    }
    const kept = withoutEmpty(parserOptions);
    if (kept !== undefined) own.parserOptions = kept;
  }
  return Object.keys(own).length === 0 ? undefined : own;
}

/**
 * The expected findings' `files` and `units` once ESLint has run, and its findings. `kept` are the original units that
 * parsed; a variant is kept when its original is and none of its files is in `unparsed`. A dropped variant's files
 * are deleted and its findings dropped. `languages` maps an original unit's key to the language options ESLint linted
 * it under (`unitLanguage`'s form; a unit with none has no entry), recorded as its `languageOptions` and its rename
 * variant's: a generator that lints a unit under its upstream case's own language passes them, so the conformance
 * runner evaluates the unit under the same language.
 */
export function withOrigins(fixtures, kept, origins, variants, unparsed, findings, languages = new Map()) {
  const keptKeys = new Set(kept.map(unitKey));
  const keptVariants = [];
  const dropped = new Set();
  for (const variant of variants) {
    const paths = unitPaths(variant.unit);
    if (keptKeys.has(variant.of) && !paths.some(path => unparsed.has(path))) keptVariants.push(variant);
    else for (const path of paths) dropped.add(path);
  }
  for (const path of dropped) rmSync(join(fixtures, path), { force: true });
  const entries = [];
  const language = key => (languages.get(key) === undefined ? {} : { languageOptions: languages.get(key) });
  for (const key of keptKeys) if (origins.has(key)) entries.push([key, { rules: [...new Set(origins.get(key))].sort(), ...language(key) }]);
  for (const variant of keptVariants) {
    entries.push([unitKey(variant.unit), { rules: [...new Set(origins.get(variant.of))].sort(), renamedFrom: variant.of, ...language(variant.of) }]);
  }
  entries.sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0));
  return {
    files: [...kept, ...keptVariants.map(variant => variant.unit)],
    units: Object.fromEntries(entries),
    findings: findings.filter(finding => !dropped.has(finding.path)),
  };
}
