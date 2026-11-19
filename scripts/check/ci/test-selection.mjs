#!/usr/bin/env node
// Which unit test files a pull request's change can affect: ci.yml's `select-tests` job runs this on the pull request's
// files, and the unit jobs (Linux and Windows) run only those files, on as many shards as they need, or none.
//
// A test file is selected when a changed file is in what it runs or reads:
//   - its import closure (esbuild's metafile from the file, with vitest's setup files, node_modules left out), and the
//     closures of the scripts and modules it names by path (a test that spawns `scripts/x.mjs` runs x's imports too);
//   - a path a file of that closure names in a string literal: the changed file itself, a directory above it (a
//     loader that reads `packs/` names "packs"), or its file name; read from the repository's root and from the
//     naming file's directory. A literal with whitespace in it is prose, not a path.
// The changes it cannot see as files are mapped to what the tests do see: a change to the engine's inputs (its crates,
// cargo's files and the files it embeds, as engine-inputs.mjs finds them) is a change to native/rerail_engine.node,
// which the engine's loader names ("native"); a change under src/ is also one under dist/, which the tests of the
// built package name.
//
// Every test file is selected (`all`) when the change reaches every test or this cannot tell: the dependencies and
// their lockfile, the TypeScript and vitest configuration, this workflow and its actions, this script and the
// engine-inputs.mjs it reads; a failure of any kind.
//
//   node scripts/check/ci/test-selection.mjs <changed-files.txt>             JSON to stdout
//   node scripts/check/ci/test-selection.mjs <changed-files.txt> --github    `name=value` lines for $GITHUB_OUTPUT
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, posix, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { engineSources, isEngineInput } from "./engine-inputs.mjs";

/** Files whose change selects every test file (exact paths, or a directory's prefix ending in `/`). */
export const FORCE_ALL = [
  "package.json",
  "package-lock.json",
  ".npmrc",
  "vitest.config.ts",
  "tsconfig.json",
  "tsconfig.build.json",
  ".github/workflows/ci.yml",
  ".github/actions/",
  "scripts/check/ci/test-selection.mjs",
  "scripts/check/ci/engine-inputs.mjs",
];

/** vitest's setup files (vitest.config.ts): every test file runs them. */
export const SETUP = ["test/support/setup-rerail-home.ts", "test/support/temporary/run-folder.ts"];

/** Files outside the unit suite that a unit job also runs on its first shard: Windows' hook integration test. */
export const WINDOWS_EXTRA = ["integration/claude-hook-rules.test.ts"];

/** The shards each unit job runs when it runs every test file (ci.yml's matrices before selection). */
export const SHARDS = { linux: 3, windows: 5 };

/**
 * The longest list of selected paths a job takes on its command line, which node gets whole from bash (Windows'
 * CreateProcess takes 32,767 characters; every test file's path together is about 20,000). Past it, everything.
 */
const MOST_CHARACTERS = 30_000;

const CODE = /\.(?:[cm]?[jt]sx?)$/u;
const toPosix = (path) => path.split(sep).join("/");
const startsWithAny = (path, prefixes) => prefixes.some((prefix) => (prefix.endsWith("/") ? path.startsWith(prefix) : path === prefix));

/**
 * Calls whose first argument names a module, which the import closure already follows: by the callee's name or, for a
 * method (`vi.mock`), the method's. `resolve` is one only as `require.resolve` or `import.meta.resolve`: alone, or on
 * another object, it is node:path's, whose argument is a path.
 */
const MODULE_CALLS = new Set(["require", "mock", "doMock", "unmock", "importActual", "importMock"]);

/** Whether `node` is `require` or `import.meta`, whose `resolve` takes a module's specifier. */
function isModuleResolver(ts, node) {
  if (ts.isIdentifier(node)) return node.text === "require";
  return ts.isMetaProperty(node) && node.keywordToken === ts.SyntaxKind.ImportKeyword && node.name.text === "meta";
}

/** Whether a call to `callee` names a module by its first argument (`import()`, `require()`, `vi.mock()` and the like). */
function isModuleCall(ts, callee) {
  if (callee.kind === ts.SyntaxKind.ImportKeyword) return true;
  if (ts.isIdentifier(callee)) return MODULE_CALLS.has(callee.text);
  if (!ts.isPropertyAccessExpression(callee)) return false;
  if (callee.name.text === "resolve") return isModuleResolver(ts, callee.expression);
  return MODULE_CALLS.has(callee.name.text);
}

/** The module a declaration names (`import`, `export ... from`, `import x = require()`, an `import()` type), if any. */
function declaredSpecifier(ts, node) {
  if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) return node.moduleSpecifier;
  if (ts.isExternalModuleReference(node)) return node.expression;
  if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument)) return node.argument.literal;
  return undefined;
}

/** The module a call names by its first argument, if it is a module call. */
function calledSpecifier(ts, node) {
  return ts.isCallExpression(node) && node.arguments.length > 0 && isModuleCall(ts, node.expression) ? node.arguments[0] : undefined;
}

/** Whether `node` is a string literal or a template's literal part. */
function isTextLiteral(ts, node) {
  const { TemplateHead, TemplateMiddle, TemplateTail } = ts.SyntaxKind;
  return ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node) || [TemplateHead, TemplateMiddle, TemplateTail].includes(node.kind);
}

/**
 * Every string literal's text in a TypeScript or JavaScript source (templates' literal parts too), less the modules
 * it imports (`import`, `export ... from`, `import()`, `require()`, `vi.mock()` and the like): the import closure has
 * those.
 */
export function stringLiterals(ts, file, text) {
  const out = [];
  const source = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, false, file.endsWith("x") ? ts.ScriptKind.TSX : undefined);
  const specifiers = new Set();
  const visit = (node) => {
    const specifier = declaredSpecifier(ts, node) ?? calledSpecifier(ts, node);
    if (specifier !== undefined) specifiers.add(specifier);
    if (!specifiers.has(node) && isTextLiteral(ts, node)) out.push(node.text);
    ts.forEachChild(node, visit);
  };
  visit(source);
  return out;
}

/**
 * Top-level directories a one-word literal is too common a word to read as: the product's own code (src/, dist/),
 * the tests, the scripts and the docs, and the product's own folder in every checkout (`.rerail`). A literal naming one of them with a path below it (`src/cli`, `test/**`) is read.
 */
const COMMON_WORDS = new Set(["src", "test", "dist", "scripts", "docs", "e2e", "integration", ".rerail"]);

/** File names too common to say which file a literal means: only a path to them names one. */
const COMMON_NAMES = new Set(["README.md", "package.json", "index.ts", "index.js", "mod.rs", "lib.rs", "Cargo.toml"]);

/** Whether a literal can name a path: not empty, not prose (whitespace in it), not a URL. */
function readsAsPath(literal) {
  return literal !== "" && !/\s/u.test(literal) && !literal.includes("://");
}

/**
 * A literal as a path (`/` for `\`, a glob up to the directory of its first wildcard, no trailing `/`) and whether it
 * was a glob; undefined when it names no path.
 */
function literalPath(literal) {
  if (!readsAsPath(literal)) return undefined;
  let text = literal.replaceAll("\\", "/");
  const wildcard = text.search(/[*?[{]/u);
  if (wildcard >= 0) text = text.slice(0, wildcard).replace(/[^/]*$/u, "");
  text = text.replace(/\/+$/u, "");
  return /^\.*$/u.test(text) ? undefined : { text, glob: wildcard >= 0 };
}

/**
 * Whether `fromRoot`, a literal's path from the repository's root, names one: a path with a `/` in it, or a top-level
 * entry that is not one of COMMON_WORDS.
 */
function namesFromRoot(fromRoot, topLevel) {
  return fromRoot !== "" && (fromRoot.includes("/") || (topLevel.has(fromRoot) && !COMMON_WORDS.has(fromRoot)));
}

/** The path `text` names from `file`'s directory when it starts with `./` or `../` and stays in the repository. */
function pathFromFile(file, text) {
  if (!text.startsWith("./") && !text.startsWith("../")) return undefined;
  const fromFile = posix.normalize(posix.join(posix.dirname(file), text));
  return fromFile.startsWith("..") || fromFile === "." ? undefined : fromFile;
}

/** The repository paths `text`, a literal's path in `file`, names: from the root, and from `file`'s directory. */
function pathsFrom(file, text, topLevel) {
  const fromRoot = text.replace(/^(?:\.{1,2}\/)+/u, "").replace(/^\/+/u, "");
  return [namesFromRoot(fromRoot, topLevel) ? fromRoot : undefined, pathFromFile(file, text)].filter((path) => path !== undefined);
}

/** The file name a literal's path (from `literalPath`) names: its last segment, unless a glob, a dotfile or too common. */
function fileName({ text, glob }) {
  const last = text.split("/").at(-1);
  return !glob && last.includes(".") && last.length > 3 && !last.startsWith(".") && !COMMON_NAMES.has(last) ? last : undefined;
}

/**
 * The repository paths a file names: each literal without whitespace, as a path from the repository's root (leading
 * `./` and `../` dropped; a glob up to its first wildcard), and, when it starts with `./` or `../`, from the file's own
 * directory; and its last segment as a file name (not one of COMMON_NAMES). A literal of one word names a path only when it is a top-level
 * entry of the repository (`topLevel`) and not one of COMMON_WORDS.
 */
export function namedPaths(file, literals, topLevel = new Set()) {
  const paths = new Set();
  const names = new Set();
  for (const literal of literals) {
    const path = literalPath(literal);
    if (path === undefined) continue;
    for (const named of pathsFrom(file, path.text, topLevel)) paths.add(named);
    const name = fileName(path);
    if (name !== undefined) names.add(name);
  }
  return { paths, names };
}

/** Whether `named` (from `namedPaths`) holds `path`, a directory above it, or its file name. */
export function names(named, path) {
  if (named.names.has(posix.basename(path))) return true;
  for (let at = path; at !== "."; at = posix.dirname(at)) if (named.paths.has(at)) return true;
  return false;
}

/**
 * The changed files with what the tests see of them: the addon for the engine's inputs (`sources`, from
 * engine-inputs.mjs's `engineSources`), dist/ for src/.
 */
export function seen(changed, sources) {
  const out = new Set(changed);
  for (const path of changed) {
    if (isEngineInput(path, sources)) out.add("native/rerail_engine.node");
    if (path.startsWith("src/")) out.add(`dist/${path.slice(4).replace(/\.tsx?$/u, ".js")}`);
  }
  return [...out];
}

/** vitest's test files (`vitest list --filesOnly`, as `npm test` runs them), repository-relative. */
export function testFiles(root) {
  const run = spawnSync(process.execPath, [resolve(root, "node_modules/vitest/vitest.mjs"), "list", "--dir", "test", "--filesOnly", "--json"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 1 << 26,
    env: { ...process.env, CI: "1" },
  });
  if (run.status !== 0) throw new Error(`vitest list failed: ${run.stderr}`);
  return JSON.parse(run.stdout.slice(run.stdout.indexOf("["))).map(({ file }) => toPosix(relative(root, file))).sort();
}

/** The string literals of `file` under `root`: none when it is not a code file or is not there. */
function fileLiterals(ts, root, file) {
  const path = resolve(root, file);
  return CODE.test(file) && existsSync(path) ? stringLiterals(ts, file, readFileSync(path, "utf8")) : [];
}

/** A function from a repository file to the paths it names (`namedPaths`), each file read once. */
function pathNamer(ts, root) {
  const named = new Map();
  const topLevel = new Set(readdirSync(root));
  return (file) => {
    let found = named.get(file);
    if (found === undefined) named.set(file, (found = namedPaths(file, fileLiterals(ts, root, file), topLevel)));
    return found;
  };
}

/** Bundles `pending` and records each one's import closure in `imports`: esbuild's inputs for it, or itself alone. */
async function bundle(build, root, pending, imports) {
  const { metafile } = await build({
    absWorkingDir: root,
    entryPoints: pending,
    bundle: true,
    packages: "external",
    platform: "node",
    format: "esm",
    metafile: true,
    write: false,
    outdir: "out",
    logLevel: "silent",
  });
  for (const output of Object.values(metafile.outputs)) {
    if (output.entryPoint !== undefined) imports.set(output.entryPoint, new Set(Object.keys(output.inputs)));
  }
  for (const file of pending) if (!imports.has(file)) imports.set(file, new Set([file]));
}

/** Whether `path` is a code file under `root` that is not bundled yet (not in `imports`). */
function isUnbundledCode(root, imports, path) {
  return CODE.test(path) && !path.endsWith(".d.ts") && !imports.has(path) && existsSync(resolve(root, path));
}

/** The code files not bundled yet that a file of a closure in `imports` names by path. */
function unbundled(root, imports, nameOf) {
  const next = new Set();
  for (const closure of imports.values()) {
    for (const file of closure) {
      for (const path of nameOf(file).paths) if (isUnbundledCode(root, imports, path)) next.add(path);
    }
  }
  return [...next];
}

/**
 * Each code file's import closure (file -> Set): bundles `entries`, then every code file they name by path that is
 * not bundled yet, until none is left.
 */
async function importClosures(root, entries, nameOf) {
  const { build } = await import("esbuild");
  const imports = new Map();
  for (let pending = entries; pending.length > 0; pending = unbundled(root, imports, nameOf)) await bundle(build, root, pending, imports);
  return imports;
}

/** The bundled files `file` names that `out` does not hold yet. */
function namedBundled(file, out, { imports, nameOf }) {
  return [...nameOf(file).paths].filter((path) => imports.has(path) && !out.has(path));
}

/** Adds `file`'s imports to `out`, queueing the bundled files each newly added one names. */
function take(file, out, queue, graph) {
  for (const imported of graph.imports.get(file) ?? [file]) {
    if (out.has(imported)) continue;
    out.add(imported);
    queue.push(...namedBundled(imported, out, graph));
  }
}

/** An entry's closure: its imports, the setup files', and transitively those of the code files they name. */
function expand(entry, graph) {
  const out = new Set();
  const queue = [entry, ...SETUP];
  while (queue.length > 0) take(queue.pop(), out, queue, graph);
  return out;
}

/**
 * Each entry's closure (`Map` entry -> Set of repository-relative files): its imports, the setup files' and those of
 * every code file a file of it names by path, and what each of those files names (`named`: file -> namedPaths).
 */
export async function closures(root, entries) {
  const ts = (await import("typescript")).default;
  const nameOf = pathNamer(ts, root);
  const imports = await importClosures(root, [...new Set([...entries, ...SETUP])], nameOf);
  const closure = new Map(entries.map((entry) => [entry, expand(entry, { imports, nameOf })]));
  return { closure, named: nameOf };
}

/**
 * The test files of `tests` (with `closure` and `named` from `closures`) that `changed` can affect, with `sources`
 * the engine's inputs (engine-inputs.mjs's `engineSources`).
 */
export function affected(tests, { closure, named }, changed, sources) {
  const paths = seen(changed, sources);
  return tests.filter((test) => {
    for (const file of closure.get(test)) {
      if (paths.includes(file)) return true;
      const fileNames = named(file);
      if (paths.some((path) => names(fileNames, path))) return true;
    }
    return false;
  });
}

/** The shards of `total` a job runs for `count` of `all` test files: as many as their share needs, at least one. */
export function shards(count, all, total) {
  const n = Math.min(total, Math.max(1, Math.ceil((total * count) / all)));
  return Array.from({ length: n }, (_, i) => i + 1);
}

/** The selection for `changed` under `root`: `{ unit, tests, windowsExtra, reasons }`. */
export async function select(root, changed) {
  const forced = changed.find((path) => startsWithAny(path, FORCE_ALL));
  const tests = testFiles(root);
  if (forced !== undefined) return { unit: "all", tests, all: tests.length, windowsExtra: true, reasons: [`${forced} reaches every test`] };
  const graph = await closures(root, [...tests, ...WINDOWS_EXTRA]);
  const sources = engineSources(root);
  const selected = affected(tests, graph, changed, sources);
  const windowsExtra = affected(WINDOWS_EXTRA, graph, changed, sources).length > 0;
  const long = selected.join(" ").length > MOST_CHARACTERS;
  const unit = selected.length === 0 ? "none" : long || selected.length === tests.length ? "all" : "some";
  return { unit, tests: selected, all: tests.length, windowsExtra, reasons: [`${selected.length} of ${tests.length} test files`] };
}

/** Windows' shards: the selection's, its first alone when only its hook test is selected, or none. */
function windowsShards(unit, count, all, windowsExtra) {
  if (unit !== "none") return shards(count, all, SHARDS.windows);
  return windowsExtra ? shards(0, all, SHARDS.windows) : [];
}

/** The `select-tests` job's outputs. */
export function outputs({ unit, tests, all, windowsExtra }) {
  const count = unit === "all" ? all : tests.length;
  return {
    unit,
    "unit-tests": unit === "some" ? tests.join(" ") : "",
    "unit-shards": JSON.stringify(unit === "none" ? [] : shards(count, all, SHARDS.linux)),
    "windows-shards": JSON.stringify(windowsShards(unit, count, all, windowsExtra)),
  };
}

const EVERYTHING = { unit: "all", tests: [], all: 1, windowsExtra: true, reasons: ["the selection failed"] };

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [list, flag] = process.argv.slice(2);
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
  let selection;
  try {
    const changed = readFileSync(list, "utf8").split("\n").map((line) => line.trim()).filter(Boolean);
    selection = await select(root, changed);
  } catch (error) {
    // Fail open: every test file, on every shard.
    console.error(`test-selection: ${error.message}; selecting every test`);
    selection = EVERYTHING;
  }
  for (const reason of selection.reasons) console.error(`test-selection: ${reason}`);
  if (flag === "--github") {
    for (const [name, value] of Object.entries(outputs(selection))) console.log(`${name}=${value}`);
  } else {
    console.log(JSON.stringify({ ...selection, outputs: outputs(selection) }, null, 2));
  }
}
