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
// The changes it cannot see as files are mapped to what the tests do see: a change to the engine's inputs (native/,
// accel/, forest/, languages/, cargo's files) is a change to native/rerail_engine.node, which the engine's loader names
// ("native"); a change under src/ is also one under dist/, which the tests of the built package name.
//
// Every test file is selected (`all`) when the change reaches every test or this cannot tell: the dependencies and
// their lockfile, the TypeScript and vitest configuration, this workflow and its actions, this script; a failure of
// any kind.
//
//   node scripts/check/ci/test-selection.mjs <changed-files.txt>             JSON to stdout
//   node scripts/check/ci/test-selection.mjs <changed-files.txt> --github    `name=value` lines for $GITHUB_OUTPUT
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, posix, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

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
];

/** vitest's setup files (vitest.config.ts): every test file runs them. */
export const SETUP = ["test/support/setup-rerail-home.ts", "test/support/temporary/run-folder.ts"];

/** Files outside the unit suite that a unit job also runs on its first shard: Windows' hook integration test. */
export const WINDOWS_EXTRA = ["integration/claude-hook-rules.test.ts"];

/** What the engine's addon is built from: a change to any of them is a change to the addon the tests load. */
const ENGINE_INPUTS = ["native/", "accel/", "forest/", "languages/", ".cargo/", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"];

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

/** Calls whose first argument names a module, which the import closure already follows. */
const MODULE_CALLS = new Set(["require", "mock", "doMock", "unmock", "importActual", "importMock", "resolve"]);

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
    if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier !== undefined) specifiers.add(node.moduleSpecifier);
    else if (ts.isExternalModuleReference(node)) specifiers.add(node.expression);
    else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument)) specifiers.add(node.argument.literal);
    else if (ts.isCallExpression(node) && node.arguments.length > 0) {
      const callee = node.expression;
      const name = callee.kind === ts.SyntaxKind.ImportKeyword ? "import" : ts.isIdentifier(callee) ? callee.text : ts.isPropertyAccessExpression(callee) ? callee.name.text : undefined;
      if (name === "import" || MODULE_CALLS.has(name)) specifiers.add(node.arguments[0]);
    }
    if (
      !specifiers.has(node) &&
      (ts.isStringLiteral(node) ||
        ts.isNoSubstitutionTemplateLiteral(node) ||
        node.kind === ts.SyntaxKind.TemplateHead ||
        node.kind === ts.SyntaxKind.TemplateMiddle ||
        node.kind === ts.SyntaxKind.TemplateTail)
    ) {
      out.push(node.text);
    }
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
    if (literal === "" || /\s/u.test(literal) || literal.includes("://")) continue;
    let text = literal.replaceAll("\\", "/");
    const glob = text.search(/[*?[{]/u);
    if (glob >= 0) text = text.slice(0, glob).replace(/[^/]*$/u, "");
    text = text.replace(/\/+$/u, "");
    if (text === "" || /^\.+$/u.test(text)) continue;
    const fromRoot = text.replace(/^(?:\.{1,2}\/)+/u, "").replace(/^\/+/u, "");
    const oneWord = !fromRoot.includes("/");
    if (fromRoot !== "" && (!oneWord || (topLevel.has(fromRoot) && !COMMON_WORDS.has(fromRoot)))) paths.add(fromRoot);
    if (text.startsWith("./") || text.startsWith("../")) {
      const fromFile = posix.normalize(posix.join(posix.dirname(file), text));
      if (!fromFile.startsWith("..") && fromFile !== ".") paths.add(fromFile);
    }
    const last = text.split("/").at(-1);
    if (glob < 0 && last.includes(".") && last.length > 3 && !last.startsWith(".") && !COMMON_NAMES.has(last)) names.add(last);
  }
  return { paths, names };
}

/** Whether `named` (from `namedPaths`) holds `path`, a directory above it, or its file name. */
export function names(named, path) {
  if (named.names.has(posix.basename(path))) return true;
  for (let at = path; at !== "."; at = posix.dirname(at)) if (named.paths.has(at)) return true;
  return false;
}

/** The changed files with what the tests see of them: the addon for the engine's inputs, dist/ for src/. */
export function seen(changed) {
  const out = new Set(changed);
  for (const path of changed) {
    if (startsWithAny(path, ENGINE_INPUTS)) out.add("native/rerail_engine.node");
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

/**
 * Each entry's closure (`Map` entry -> Set of repository-relative files): its imports, the setup files' and those of
 * every code file a file of it names by path, and what each of those files names (`named`: file -> namedPaths).
 */
export async function closures(root, entries) {
  const { build } = await import("esbuild");
  const ts = (await import("typescript")).default;
  const imports = new Map(); // code file -> its import closure
  const named = new Map();
  const topLevel = new Set(readdirSync(root));
  const nameOf = (file) => {
    let found = named.get(file);
    if (found === undefined) {
      const text = CODE.test(file) && existsSync(resolve(root, file)) ? readFileSync(resolve(root, file), "utf8") : "";
      named.set(file, (found = namedPaths(file, text === "" ? [] : stringLiterals(ts, file, text), topLevel)));
    }
    return found;
  };
  // Bundle the entries, then every code file they name by path that is not bundled yet, until none is left.
  let pending = [...new Set([...entries, ...SETUP])];
  while (pending.length > 0) {
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
    const next = new Set();
    for (const closure of imports.values()) {
      for (const file of closure) {
        for (const path of nameOf(file).paths) {
          if (CODE.test(path) && !path.endsWith(".d.ts") && !imports.has(path) && existsSync(resolve(root, path))) next.add(path);
        }
      }
    }
    pending = [...next];
  }
  // An entry's closure: its imports, the setup files', and transitively those of the code files they name.
  const full = new Map();
  const expand = (entry) => {
    const out = new Set();
    const queue = [entry, ...SETUP];
    while (queue.length > 0) {
      const file = queue.pop();
      for (const imported of imports.get(file) ?? [file]) {
        if (out.has(imported)) continue;
        out.add(imported);
        for (const path of nameOf(imported).paths) if (imports.has(path) && !out.has(path)) queue.push(path);
      }
    }
    return out;
  };
  for (const entry of entries) full.set(entry, expand(entry));
  return { closure: full, named: nameOf };
}

/** The test files of `tests` (with `closure` and `named` from `closures`) that `changed` can affect. */
export function affected(tests, { closure, named }, changed) {
  const paths = seen(changed);
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
  const selected = affected(tests, graph, changed);
  const windowsExtra = affected(WINDOWS_EXTRA, graph, changed).length > 0;
  const long = selected.join(" ").length > MOST_CHARACTERS;
  const unit = selected.length === 0 ? "none" : long || selected.length === tests.length ? "all" : "some";
  return { unit, tests: selected, all: tests.length, windowsExtra, reasons: [`${selected.length} of ${tests.length} test files`] };
}

/** The `select-tests` job's outputs. */
export function outputs({ unit, tests, all, windowsExtra }) {
  const count = unit === "all" ? all : tests.length;
  const windows = unit !== "none" || windowsExtra;
  return {
    unit,
    "unit-tests": unit === "some" ? tests.join(" ") : "",
    "unit-shards": JSON.stringify(unit === "none" ? [] : shards(count, all, SHARDS.linux)),
    "windows-shards": JSON.stringify(windows ? shards(unit === "none" ? 0 : count, all, SHARDS.windows) : []),
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
