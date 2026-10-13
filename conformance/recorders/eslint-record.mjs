// Shared by the ESLint-based packs' recorders (packs/<id>/upstream/<tool>/generate.mjs): pins, the corpus file set the
// conformance runner scores, the guard that keeps the recording machine out of fixtures, and
// ESLint's findings in the recording format `rerail/conformance/languages/pack.ts` reads (`recordingSchema`).
// Run offline by a maintainer, never in CI.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { homedir, tmpdir } from "node:os";
import { dirname, join, matchesGlob, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

export const REPOSITORY = dirname(dirname(dirname(fileURLToPath(import.meta.url))));

/** The host's ignored directories (`IGNORED_DIRECTORIES`, rerail/src/engine/code-rule/code-rule-enforcer.ts). */
const IGNORED = new Set([".git", ".hg", ".svn", ".rerail", ".venv", "__pycache__", "node_modules", "dist", "build", ".next", ".turbo", "coverage", "target", "vendor"]);
/** The ecma family's extensions (`CODE_RULE_DIALECTS`, rerail/src/engine/code-rule/code-rule-languages.ts). */
export const ECMA_EXTENSIONS = [".ts", ".mts", ".cts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];

/** The conformance pack manifest, `packs/<name>/conformance.json`. */
export function pack(name) {
  return JSON.parse(readFileSync(join(REPOSITORY, "packs", name, "conformance.json"), "utf8"));
}

/** The working directory upstream tests see while they declare their cases (`withRecordingCwd`): the same everywhere. */
export const RECORDING_CWD = "/work/project";

/**
 * Runs `declare` (importing an upstream test file) with process.cwd() answering RECORDING_CWD, so a test that builds a
 * case's `filename` from the working directory (unicorn's filename-case: `path.join(cwd, …)`) names the same file on
 * every machine.
 */
export async function withRecordingCwd(declare) {
  const realCwd = process.cwd;
  process.cwd = () => RECORDING_CWD;
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
 * upstream test that named its file from the recorder's working directory, home or temp folder (#31's filename-case
 * fixtures held `/private/tmp/claude-501/…`), which breaks Windows checkouts and leaks the maintainer's machine.
 * Pin the working directory with `withRecordingCwd`, or keep only the name below the test's own fixture directory.
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

/** `require` resolving from a tools directory (`.rerail/tools/<pack>`), after checking every pinned package's version. */
export function pinnedRequire(tools, pins) {
  const require = createRequire(join(tools, "package.json"));
  for (const [name, version] of Object.entries(pins)) {
    const found = JSON.parse(readFileSync(join(tools, "node_modules", name, "package.json"), "utf8")).version;
    if (found !== version) throw new Error(`${name} ${found} under ${tools}; this recorder pins ${version}`);
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

function walk(root, path, files) {
  if (!existsSync(path)) throw new Error(`${path} does not exist`);
  if (!statSync(path).isDirectory()) {
    files.push(relative(root, path).split(sep).join("/"));
    return;
  }
  for (const entry of readdirSync(path, { withFileTypes: true })) {
    if (entry.isSymbolicLink()) continue;
    const child = join(path, entry.name);
    if (entry.isDirectory()) {
      if (!IGNORED.has(entry.name)) walk(root, child, files);
    } else if (entry.isFile()) files.push(relative(root, child).split(sep).join("/"));
  }
}

/** The corpus-relative files the conformance runner scores: the pack's paths, ecma extensions, minus `exclude`. */
export function corpusFiles(root, corpus) {
  assertCommit(root, corpus.commit);
  const files = [];
  for (const path of corpus.paths) walk(root, join(root, path), files);
  return [...new Set(files)]
    .filter(file => ECMA_EXTENSIONS.some(extension => file.endsWith(extension)))
    .filter(file => !corpus.exclude.some(pattern => matchesGlob(file, pattern)))
    .sort();
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

/** Writes a recording as packs/php/upstream/common.py does: findings sorted, one-space indent. */
export function writeRecording(path, tool, version, source, findings) {
  const sorted = [...findings].sort(
    (a, b) => (a.rule < b.rule ? -1 : a.rule > b.rule ? 1 : 0) || (a.path < b.path ? -1 : a.path > b.path ? 1 : 0) || a.line - b.line || (a.column ?? 0) - (b.column ?? 0),
  );
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify({ tool, version, source, findings: sorted }, null, 1)}\n`);
}

/**
 * Lints `files` (relative to `cwd`) with exactly `config` (a flat config array; no config file is read) and returns
 * the recording's findings for `rules` plus the files ESLint could not parse.
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
