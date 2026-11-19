#!/usr/bin/env node
// Which Rust packages' tests a change can affect: ci.yml's `changes` job runs this on a pull request's files and the
// Rust jobs test only what it selects. A package is selected when a file it compiles changed (its targets' module
// trees, followed through `mod`, `#[path]` and `include!`/`include_str!`/`include_bytes!`, as rustc resolves them),
// or a file in its directory, and then every package that depends on a selected one, of any kind (normal, dev, build),
// transitively: their tests compile and run the changed code too.
//
// A changed data file the tests read at run time or compile in (DATA_ROOTS: the fixtures, the packs, the conformance
// suites, the language registry, the rule syntax's library and examples, the data-layer gate) selects the packages
// whose sources name it (`dataMentions`), and every package above them: a string literal that names the root and the
// file's first directory under it (`packs/python/...` for packs/python/rules/x.rerail; `fixtures_dir!().join("php/..")`
// or `oracle!("php/..")` for fixtures/php/...), or names the root alone (a loader that joins the rest at run time),
// which counts for every file under it. A literal made of `..` alone that reaches the repository root from the
// package's directory (`CARGO_MANIFEST_DIR` joined with `../../..`, a test walking the whole repository) counts for
// every file under every root. A literal with whitespace in it is prose (a message), not a path.
//
// Everything is selected (`all`) when the change reaches every package, or when this cannot tell:
//   - the build's own inputs: the lockfile, the workspace manifest, the toolchain, cargo's configuration, the patched
//     dependencies under native/vendor/, this workflow and its actions, package.json (it holds the cargo commands), the
//     LFS attributes (the engine embeds LFS objects), this script, the language registry's generated module;
//   - a file under native/ that no package's module tree claims and that is not in a package's own directory: the
//     engine's sources are mounted by many crates (tools/cratify), so who reads such a file cannot be told;
//   - a failure of any kind.
// A file outside every package that neither FORCE_ALL nor DATA_ROOTS names (TypeScript, docs) selects nothing.
//
//   node scripts/check/ci/rust-selection.mjs <changed-files.txt>              JSON to stdout
//   node scripts/check/ci/rust-selection.mjs <changed-files.txt> --github     `name=value` lines for $GITHUB_OUTPUT
//   ... --base-package-json <file>   the base's package.json: package.json then selects every package only when what
//                                    the Rust builds read of it changed (`cargoInputs`)
//
// The files list holds one repository-relative path per line (a rename lists both names).
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

/** Globs (`**` whole segments, `*` within one) whose change selects every package. */
export const FORCE_ALL = [
  "Cargo.lock",
  "Cargo.toml",
  "rust-toolchain.toml",
  ".cargo/**",
  "native/vendor/**",
  ".github/workflows/ci.yml",
  ".github/actions/**",
  "package.json",
  ".gitattributes",
  "scripts/check/ci/rust-selection.mjs",
  "src/engine/code-rule/code-rule-language-registry.generated.ts",
];

/** Data the packages' tests and build scripts read from outside their sources, selected by `dataMentions`. */
export const DATA_ROOTS = ["fixtures", "packs", "conformance", "languages", "docs/rule-syntax", "gate/data-layer"];

/** Where a file outside every package's own directory and module trees still cannot be told apart. */
const UNCLAIMED_ALL = ["native/**"];

function globRegExp(glob) {
  const body = glob
    .split("/")
    .map((part) => (part === "**" ? "(?:.*)" : part.replace(/[.+^${}()|[\]\\]/gu, "\\$&").replaceAll("*", "[^/]*")))
    .join("/")
    .replaceAll("(?:.*)/", "(?:.*/)?");
  return new RegExp(`^${body}$`, "u");
}

const matcher = (globs) => {
  const patterns = globs.map(globRegExp);
  return (path) => patterns.some((pattern) => pattern.test(path));
};

/**
 * The tokens of Rust source that module resolution needs: identifiers, punctuation and string literals (their
 * values), comments and char literals dropped.
 */
export function tokens(text) {
  const out = [];
  let i = 0;
  const n = text.length;
  while (i < n) {
    const c = text[i];
    if (c === "/" && text[i + 1] === "/") {
      while (i < n && text[i] !== "\n") i++;
    } else if (c === "/" && text[i + 1] === "*") {
      let depth = 1;
      i += 2;
      while (i < n && depth > 0) {
        if (text[i] === "/" && text[i + 1] === "*") (depth++, (i += 2));
        else if (text[i] === "*" && text[i + 1] === "/") (depth--, (i += 2));
        else i++;
      }
    } else if (c === '"' || ((c === "b" || c === "c") && text[i + 1] === '"')) {
      i += c === '"' ? 1 : 2;
      let value = "";
      while (i < n && text[i] !== '"') {
        if (text[i] === "\\") (value += text[i + 1] ?? "", (i += 2));
        else value += text[i++];
      }
      i++;
      out.push({ string: value });
    } else if ((c === "r" || ((c === "b" || c === "c") && text[i + 1] === "r")) && /^[bc]?r#*"/u.test(text.slice(i, i + 260))) {
      const head = /^[bc]?r(#*)"/u.exec(text.slice(i, i + 260));
      const close = `"${head[1]}`;
      const start = i + head[0].length;
      const end = text.indexOf(close, start);
      out.push({ string: text.slice(start, end < 0 ? n : end) });
      i = end < 0 ? n : end + close.length;
    } else if (c === "'") {
      // A char literal ('x', '\n', '\u{1F600}') or a lifetime ('a): only the literal is skipped whole.
      if (text[i + 1] === "\\") {
        const end = text.indexOf("'", i + 2);
        i = end < 0 ? n : end + 1;
      } else if (text[i + 2] === "'") i += 3;
      else i++;
    } else if (/[A-Za-z_]/u.test(c)) {
      let j = i + 1;
      while (j < n && /[A-Za-z0-9_]/u.test(text[j])) j++;
      out.push({ ident: text.slice(i, j) });
      i = j;
    } else if (/\s/u.test(c)) i++;
    else out.push({ punct: c }), i++;
  }
  return out;
}

/**
 * A file's module declarations and includes: `{ mods: [{ name, paths, inline }], includes: [{ path, inline }] }`, where
 * `paths` are the module's `#[path]` values (`undefined` for none; two when a `cfg_attr` sets one) and `inline` is the
 * chain of enclosing inline modules (`{ name, paths }`) inside the file. `mod x { ... }` blocks are themselves
 * listed in `inline` chains, not as mods.
 */
export function declarations(text) {
  const t = tokens(text);
  const mods = [];
  const includes = [];
  const stack = []; // per `{`: an inline module frame, or null for any other block
  let pendingPath;
  for (let i = 0; i < t.length; i++) {
    const tok = t[i];
    if (tok.punct === "#" && t[i + 1]?.punct === "[") {
      // An attribute: `#[path = "..."]` is kept for the item it precedes.
      let depth = 0;
      let j = i + 1;
      for (; j < t.length; j++) {
        if (t[j].punct === "[") depth++;
        else if (t[j].punct === "]" && --depth === 0) break;
      }
      if (t[i + 2]?.ident === "path" && t[i + 3]?.punct === "=" && t[i + 4]?.string !== undefined) pendingPath = [t[i + 4].string];
      else if (t[i + 2]?.ident === "cfg_attr") {
        // `#[cfg_attr(cond, path = "...")]`: the path when cond holds, the default otherwise; both are kept.
        for (let k = i + 3; k < j - 2; k++) {
          if (t[k].ident === "path" && t[k + 1].punct === "=" && t[k + 2].string !== undefined) pendingPath = [t[k + 2].string, undefined];
        }
      }
      i = j;
      continue;
    }
    if (tok.ident === "mod" && t[i + 1]?.ident !== undefined && (t[i + 2]?.punct === ";" || t[i + 2]?.punct === "{")) {
      const name = t[i + 1].ident;
      const inline = stack.filter(Boolean);
      const paths = pendingPath ?? [undefined];
      if (t[i + 2].punct === ";") mods.push({ name, paths, inline });
      else stack.push({ name, paths });
      pendingPath = undefined;
      i += 2;
      continue;
    }
    if (/^include(?:_str|_bytes)?$/u.test(tok.ident ?? "") && t[i + 1]?.punct === "!" && t[i + 2]?.punct === "(" && t[i + 3]?.string !== undefined) {
      includes.push({ path: t[i + 3].string, inline: stack.filter(Boolean) });
      continue;
    }
    if (tok.punct === "{") stack.push(null), (pendingPath = undefined);
    else if (tok.punct === "}") stack.pop(), (pendingPath = undefined);
    else if (tok.punct === ";") pendingPath = undefined;
  }
  return { mods, includes };
}

const escapeRegExp = (text) => text.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
const ROOT_PATTERNS = DATA_ROOTS.map((root) => [root, new RegExp(`(?:^|[^A-Za-z0-9_.-])${escapeRegExp(root)}(?=$|[^A-Za-z0-9_.-])(?:/([^/{}]+))?`, "gu")]);
const firstSegment = (path) => path.replace(/^\/+/u, "").split("/")[0];

/**
 * The key under which `dataMentions` lists the literals made of `..` alone (`"../../.."`, or `"/../.."` as
 * `concat!(env!("CARGO_MANIFEST_DIR"), ...)` appends it), resolved by `select` against each package's directory.
 */
const PARENTS = "..";
const PARENTS_PATTERN = /^\/?(?:\.\.\/)*\.\.\/?$/u;

/** Whether token `i` opens a call of the macro `name`: `name ! (`. */
const isMacroCall = (t, i, name) => t[i].ident === name && t[i + 1]?.punct === "!" && t[i + 2]?.punct === "(";

/** Adds what a path (a literal's value, `/`-separated) names under each data root. */
function rootMentions(path, add) {
  for (const [root, pattern] of ROOT_PATTERNS) {
    for (const match of path.matchAll(pattern)) add(root, match[1]);
  }
}

/** Whether the literal at token `i` is compared or matched (`c == ".."`, `".." != c`, `".." =>`): a value, not a path. */
function isOperand(t, i) {
  const before = `${t[i - 2]?.punct ?? ""}${t[i - 1]?.punct ?? ""}`;
  const after = `${t[i + 1]?.punct ?? ""}${t[i + 2]?.punct ?? ""}`;
  return ["==", "!="].includes(before) || ["==", "!=", "=>"].includes(after);
}

/**
 * A string literal without whitespace: the data roots it names, or itself when it is made of `..` alone and used as a
 * path, not compared with a path's components.
 */
function literalMention(t, i, add) {
  const { string } = t[i];
  if (string === undefined || /\s/u.test(string)) return false;
  const path = string.replaceAll("\\", "/");
  rootMentions(path, add);
  if (PARENTS_PATTERN.test(path) && !isOperand(t, i)) add(PARENTS, path.replace(/^\//u, ""));
  return true;
}

/** The literal that follows `fixtures_dir!()` from token `j`: `.join("<path>")` or `, "<path>"`. */
function fixturesSuffix(t, j) {
  if (t[j]?.punct === "." && t[j + 1]?.ident === "join" && t[j + 2]?.punct === "(") return t[j + 3]?.string;
  return t[j]?.punct === "," ? t[j + 1]?.string : undefined;
}

/** `fixtures_dir!()`: the first directory of the path joined to it, or the whole of fixtures/. */
function fixturesDirMention(t, i, add) {
  if (!isMacroCall(t, i, "fixtures_dir") || t[i + 3]?.punct !== ")") return false;
  let j = i + 4;
  while (t[j]?.punct === ")") j++;
  const next = fixturesSuffix(t, j);
  add("fixtures", next === undefined ? undefined : firstSegment(next));
  return true;
}

/** `oracle!("<dir>/...")`: `<dir>` under fixtures/. */
function oracleMention(t, i, add) {
  if (!isMacroCall(t, i, "oracle") || t[i + 3]?.string === undefined) return false;
  add("fixtures", firstSegment(t[i + 3].string));
  return true;
}

/** The forms that name data, tried in order on each token: the first that matches it adds what it names. */
const MENTIONS = [literalMention, fixturesDirMention, oracleMention];

/**
 * Which data under DATA_ROOTS a source file names: `{ <root>: Set of first directories, or "*" for the root alone }`,
 * and under PARENTS the literals made of `..` alone. Paths are its string literals without whitespace, and the
 * fixtures' macros: `fixtures_dir!()` followed by `.join("<dir>/...")` or `, "/<dir>/..."` names `<dir>`, followed
 * by anything else the whole of fixtures/; `oracle!("<dir>/...")` names `<dir>`.
 */
export function dataMentions(text) {
  const t = tokens(text);
  const found = {};
  const add = (root, sub) => (found[root] ??= new Set()).add(sub === undefined || sub === "" ? "*" : sub);
  for (let i = 0; i < t.length; i++) MENTIONS.some((mention) => mention(t, i, add));
  return found;
}

/**
 * Every file a target whose root is `root` (absolute) compiles: its module tree as rustc resolves it, and the files
 * it includes. Files that do not exist (a `#[cfg]`'d module of another platform still resolves to one that does;
 * generated sources live in OUT_DIR) are left out.
 */
export function moduleFiles(root, cache = new Map()) {
  // As rustc_expand's module.rs: each module has a directory, and a file module found as `x.rs` (not `x/mod.rs`, not
  // through `#[path]`) a relative offset `x`, which only its children found by name and its inline modules without a
  // `#[path]` step into. A `#[path]` on `mod x;` or on an inline module is relative to the directory alone.
  const seen = new Set();
  const visit = (file, offset) => {
    const key = `${file}\0${offset ?? ""}`;
    if (seen.has(key) || !existsSync(file)) return;
    seen.add(key);
    let decls = cache.get(file);
    if (decls === undefined) {
      decls = declarations(readFileSync(file, "utf8"));
      cache.set(file, decls);
    }
    const dir = dirname(file);
    // The (directory, offset) states an inline module chain can stand for: each frame is its path, or its name.
    const states = (inline) =>
      inline.reduce(
        (all, frame) =>
          all.flatMap((state) =>
            frame.paths.map((path) => ({ dir: path === undefined ? join(state.dir, state.offset ?? "", frame.name) : resolve(state.dir, path), offset: undefined })),
          ),
        [{ dir, offset }],
      );
    for (const { name, paths, inline } of decls.mods) {
      for (const state of states(inline)) {
        for (const path of paths) {
          if (path !== undefined) visit(resolve(state.dir, path), undefined);
          else {
            visit(join(state.dir, state.offset ?? "", `${name}.rs`), name);
            visit(join(state.dir, state.offset ?? "", name, "mod.rs"), undefined);
          }
        }
      }
    }
    for (const { path } of decls.includes) {
      // include!-family paths are relative to the including file's directory.
      const included = resolve(dir, path);
      if (existsSync(included)) seen.add(`${included}\0include`);
    }
  };
  // A crate root (lib.rs, main.rs, a test, an example, a build script) resolves its modules beside itself.
  visit(root, undefined);
  return new Set([...seen].map((key) => key.split("\0")[0]));
}

/** `cargo metadata --no-deps` of the workspace at `root`, with the toolchain cargo runs under left to the caller. */
export function workspaceMetadata(root) {
  const run = spawnSync("cargo", ["metadata", "--no-deps", "--offline", "--format-version", "1", "--manifest-path", join(root, "Cargo.toml")], {
    encoding: "utf8",
    maxBuffer: 1 << 28,
  });
  if (run.status !== 0) throw new Error(`cargo metadata failed: ${run.stderr}`);
  return JSON.parse(run.stdout);
}

/** Adds `value` to the set `map` holds at `key`, creating it. */
const addTo = (map, key, value) => (map.get(key) ?? map.set(key, new Set()).get(key)).add(value);

/** Whether `dir` (absolute) is the repository root `root` or a directory above it. */
function reachesRoot(root, dir) {
  const down = relative(dir, root);
  return down === "" || !(down.startsWith("..") || isAbsolute(down));
}

/**
 * What a package whose directory is `dir` reads of the data, from a file's `dataMentions`: `[root, subs]` pairs, and
 * every root whole when a literal made of `..` alone reaches the repository root from `dir`.
 */
function dataReads(found, root, dir) {
  const { [PARENTS]: parents = [], ...named } = found;
  const whole = [...parents].some((path) => reachesRoot(root, resolve(dir, path)));
  return [...Object.entries(named), ...(whole ? DATA_ROOTS.map((data) => [data, ["*"]]) : [])];
}

/** Records the data a package's source file reads: `graph.readers`, data root -> package -> first directories. */
function addReader(graph, pkg, file) {
  let found = graph.mentions.get(file);
  if (found === undefined) graph.mentions.set(file, (found = dataMentions(readFileSync(file, "utf8"))));
  for (const [root, subs] of dataReads(found, graph.root, dirname(pkg.manifest_path))) {
    const byPackage = (graph.readers[root] ??= new Map());
    for (const sub of subs) addTo(byPackage, pkg.name, sub);
  }
}

/** Records the files a package's target compiles (`graph.claimed`) and the data its Rust sources read. */
function addTarget(graph, pkg, target) {
  for (const file of moduleFiles(target.src_path, graph.modules)) {
    addTo(graph.claimed, graph.rel(file), pkg.name);
    if (file.endsWith(".rs")) addReader(graph, pkg, file);
  }
}

/** Records a package's path dependencies: their directories as graph nodes, and the package among their dependents. */
function addDependencies(graph, metadata, pkg) {
  for (const dep of pkg.dependencies) {
    if (dep.path === undefined) continue;
    const node = metadata.packages.find((p) => dirname(p.manifest_path) === dep.path)?.name ?? `path:${graph.rel(dep.path)}`;
    if (!graph.dirs.has(graph.rel(dep.path))) graph.dirs.set(graph.rel(dep.path), node);
    addTo(graph.dependents, node, pkg.name);
  }
}

/**
 * The workspace at `root` as `select` reads it: package directories and path dependencies outside the workspace
 * (tools/bitlex) as graph nodes (`dirs`, relative directory -> node), each node's dependents, the packages compiling
 * each file (`claimed`, relative file -> packages), and who reads which data (`readers`).
 */
function workspaceGraph(root, metadata) {
  const rel = (abs) => relative(root, abs).split(sep).join("/");
  const graph = { root, rel, dirs: new Map(), dependents: new Map(), claimed: new Map(), readers: {}, modules: new Map(), mentions: new Map() };
  for (const pkg of metadata.packages) {
    graph.dirs.set(rel(dirname(pkg.manifest_path)), pkg.name);
    addDependencies(graph, metadata, pkg);
    for (const target of pkg.targets) addTarget(graph, pkg, target);
  }
  return graph;
}

/** The packages whose sources name the data file `path` under `root`: the root alone, or its first directory. */
function dataReaders(readers, root, path) {
  const sub = path.slice(root.length + 1).split("/");
  const first = sub.length > 1 ? sub[0] : undefined;
  return [...(readers[root] ?? new Map())].filter(([, subs]) => subs.has("*") || first === undefined || subs.has(first)).map(([name]) => name);
}

/** Whether the relative directory `dir` holds the relative path `path` ("" holds everything). */
const holds = (dir, path) => dir === "" || path === dir || path.startsWith(`${dir}/`);

/** The deepest graph node whose directory holds `path`: `[dir, node]`. */
function owner(dirs, path) {
  let best;
  for (const [dir, node] of dirs) if (holds(dir, path) && (best === undefined || dir.length > best[0].length)) best = [dir, node];
  return best;
}

const forcedAll = matcher(FORCE_ALL);
const unclaimedAll = matcher(UNCLAIMED_ALL);

/** A file some module tree claims selects the packages compiling it. */
function compiledBy(graph, path) {
  const compiling = graph.claimed.get(path);
  return compiling === undefined ? undefined : { seeds: [...compiling] };
}

/** A file under a data root selects the packages reading it. */
function readBy(graph, path) {
  const root = DATA_ROOTS.find((data) => path.startsWith(`${data}/`));
  if (root === undefined) return undefined;
  const seeds = dataReaders(graph.readers, root, path);
  return { seeds, reason: seeds.length > 0 ? `${path} is read by ${seeds.join(", ")}` : undefined };
}

/** A file in a package's own directory selects that package. */
function ownedBy(graph, path) {
  const own = owner(graph.dirs, path);
  // The engine's directory holds the sources every crate mounts: a file there no tree claims could be anyone's.
  if (own === undefined || own[0] === "" || (own[1] === "rerail-engine" && unclaimedAll(path))) return undefined;
  return { seeds: [own[1]] };
}

/** A file under native/ nothing above claims selects every package. */
function unclaimed(graph, path) {
  return unclaimedAll(path) ? { all: `${path} is under native/ and no package's module tree claims it` } : undefined;
}

/**
 * What a changed file selects, by the first rule that answers: `{ seeds, reason? }`, or `{ all: reason }`. A file no
 * rule answers for (TypeScript, docs) selects nothing.
 */
const PATH_RULES = [compiledBy, readBy, ownedBy, unclaimed];

/** The verdict of the first of PATH_RULES that answers for `path`. */
function classify(graph, path) {
  for (const rule of PATH_RULES) {
    const verdict = rule(graph, path);
    if (verdict !== undefined) return verdict;
  }
  return { seeds: [] };
}

/** The nodes `seeds` and every package that depends on one of them, transitively. */
function withDependents(seeds, dependents) {
  const selected = new Set();
  const queue = [...seeds];
  while (queue.length > 0) {
    const node = queue.pop();
    if (selected.has(node)) continue;
    selected.add(node);
    queue.push(...(dependents.get(node) ?? []));
  }
  return selected;
}

/**
 * The selection for `changed` (repository-relative paths) in the workspace at `root` described by `metadata`:
 * `{ all, packages, reasons }`, `packages` sorted.
 */
export function select(root, metadata, changed) {
  const names = metadata.packages.map((p) => p.name).sort();
  const everything = (reason) => ({ all: true, packages: names, reasons: [reason] });
  const forced = changed.find((path) => forcedAll(path));
  if (forced !== undefined) return everything(`${forced} reaches every package`);
  const graph = workspaceGraph(root, metadata);
  const seeds = new Set();
  const reasons = [];
  for (const path of changed) {
    const verdict = classify(graph, path);
    if (verdict.all !== undefined) return everything(verdict.all);
    for (const name of verdict.seeds) seeds.add(name);
    if (verdict.reason !== undefined) reasons.push(verdict.reason);
  }
  const selected = withDependents(seeds, graph.dependents);
  const packages = names.filter((name) => selected.has(name));
  reasons.push(...[...seeds].sort().map((seed) => `${seed} selected`));
  return { all: packages.length === names.length, packages, reasons };
}

/** The `changes` job's outputs for a selection. */
export function outputs({ all, packages }) {
  return {
    "rust-all": String(all),
    "rust-packages": JSON.stringify(packages),
    "rust-cargo-args": all ? "--workspace" : packages.map((name) => `-p ${name}`).join(" "),
    "rust-nextest-filter": all ? "all()" : packages.length === 0 ? "none()" : packages.map((name) => `package(${name})`).join(" | "),
    "rust-engine": String(packages.includes("rerail-engine")),
  };
}

/**
 * What of package.json the Rust builds and tests read: napi's configuration and the scripts that run cargo, napi or
 * the engine's checks. A pull request that changes only the rest (dependencies, the TypeScript scripts) leaves them.
 */
export function cargoInputs(packageJson) {
  const { napi, scripts = {} } = JSON.parse(packageJson);
  return JSON.stringify({ napi, scripts: Object.entries(scripts).filter(([, command]) => /cargo|napi|native|rust/u.test(command)) });
}

/**
 * The command line: the changed-files list, the base's package.json (`--base-package-json <file>`) and `--github`.
 * The list is the argument that is no flag and not the base's file, wherever it stands.
 */
export function cliArguments(args) {
  const base = args.indexOf("--base-package-json");
  const basePackageJson = base < 0 ? undefined : args[base + 1];
  const list = args.find((arg, i) => !arg.startsWith("--") && (base < 0 || i !== base + 1));
  return { list, basePackageJson, github: args.includes("--github") };
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { list, basePackageJson, github } = cliArguments(process.argv.slice(2));
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
  let selection;
  try {
    let changed = readFileSync(list, "utf8").split("\n").map((line) => line.trim()).filter(Boolean);
    // package.json reaches every package only when what cargo reads of it changed (`cargoInputs`).
    if (basePackageJson !== undefined && changed.includes("package.json")) {
      const same = cargoInputs(readFileSync(basePackageJson, "utf8")) === cargoInputs(readFileSync(join(root, "package.json"), "utf8"));
      if (same) changed = changed.filter((path) => path !== "package.json");
    }
    selection = select(root, workspaceMetadata(root), changed);
  } catch (error) {
    // Fail open: every package's tests run.
    console.error(`rust-selection: ${error.message}; selecting every package`);
    selection = { all: true, packages: ["all"], reasons: ["the selection failed"] };
    try {
      selection.packages = workspaceMetadata(root).packages.map((p) => p.name).sort();
    } catch {
      // ["all"], as ci.yml's fallback: never `[]`, which would read as "no crate's tests to run".
    }
  }
  for (const reason of selection.reasons) console.error(`rust-selection: ${reason}`);
  if (github) {
    for (const [name, value] of Object.entries(outputs(selection))) console.log(`${name}=${value}`);
  } else {
    console.log(JSON.stringify({ ...selection, outputs: outputs(selection) }, null, 2));
  }
}
