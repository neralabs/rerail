#!/usr/bin/env node
// Which Rust packages' tests a change can affect: ci.yml's `changes` job runs this on a pull request's files and the
// Rust jobs test only what it selects. A package is selected when a file it compiles changed (its targets' module
// trees, followed through `mod`, `#[path]` and `include!`/`include_str!`/`include_bytes!`, as rustc resolves them),
// or a file in its directory, and then every package that depends on a selected one, of any kind (normal, dev, build),
// transitively: their tests compile and run the changed code too.
//
// Everything is selected (`all`) when the change reaches every package, or when this cannot tell:
//   - the build's own inputs: the lockfile, the workspace manifest, the toolchain, cargo's configuration, the patched
//     dependencies under native/vendor/, this workflow and its actions, package.json (it holds the cargo commands), the
//     LFS attributes (the engine embeds LFS objects), this script;
//   - what the engine's build script and tests read at run time from outside the crates: the language registry
//     (languages/), the fixtures, the conformance suites, the packs and their rules (FORCE_ALL);
//   - a file under native/ that no package's module tree claims and that is not in a package's own directory: the
//     engine's sources are mounted by many crates (tools/cratify), so who reads such a file cannot be told;
//   - a failure of any kind.
// A file outside every package that FORCE_ALL does not name (TypeScript, docs) selects nothing.
//
//   node scripts/check/rust-selection.mjs <changed-files.txt>              JSON to stdout
//   node scripts/check/rust-selection.mjs <changed-files.txt> --github     `name=value` lines for $GITHUB_OUTPUT
//
// The files list holds one repository-relative path per line (a rename lists both names).
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { basename, dirname, join, relative, resolve, sep } from "node:path";
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
  "scripts/check/rust-selection.mjs",
  "languages/**",
  "fixtures/**",
  "conformance/**",
  "packs/**",
  "docs/rule-syntax/**",
  "gate/data-layer/**",
  "src/engine/code-rule/code-rule-language-registry.generated.ts",
];

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

/**
 * The selection for `changed` (repository-relative paths) in the workspace at `root` described by `metadata`:
 * `{ all, packages, reasons }`, `packages` sorted.
 */
export function select(root, metadata, changed) {
  const names = metadata.packages.map((p) => p.name).sort();
  const everything = (reason) => ({ all: true, packages: names, reasons: [reason] });
  const forced = matcher(FORCE_ALL);
  for (const path of changed) if (forced(path)) return everything(`${path} reaches every package`);

  const rel = (abs) => relative(root, abs).split(sep).join("/");
  // Package directories, and path dependencies outside the workspace (tools/bitlex), as graph nodes.
  const dirs = new Map(); // directory (relative) -> node name
  const dependents = new Map(); // node -> packages that depend on it
  const cache = new Map();
  const claimed = new Map(); // file (relative) -> packages compiling it
  for (const pkg of metadata.packages) {
    dirs.set(rel(dirname(pkg.manifest_path)), pkg.name);
    for (const dep of pkg.dependencies) {
      if (dep.path === undefined) continue;
      const node = metadata.packages.find((p) => dirname(p.manifest_path) === dep.path)?.name ?? `path:${rel(dep.path)}`;
      if (!dirs.has(rel(dep.path))) dirs.set(rel(dep.path), node);
      if (!dependents.has(node)) dependents.set(node, new Set());
      dependents.get(node).add(pkg.name);
    }
    for (const target of pkg.targets) {
      for (const file of moduleFiles(target.src_path, cache)) {
        const key = rel(file);
        if (!claimed.has(key)) claimed.set(key, new Set());
        claimed.get(key).add(pkg.name);
      }
    }
  }
  const owner = (path) => {
    let best;
    for (const [dir, node] of dirs) {
      if ((dir === "" || path === dir || path.startsWith(`${dir}/`)) && (best === undefined || dir.length > best[0].length)) best = [dir, node];
    }
    return best;
  };
  const unclaimedAll = matcher(UNCLAIMED_ALL);
  const seeds = new Set();
  const reasons = [];
  for (const path of changed) {
    const compiling = claimed.get(path);
    if (compiling !== undefined) {
      for (const name of compiling) seeds.add(name);
      continue;
    }
    const own = owner(path);
    // The engine's directory holds the sources every crate mounts: a file there no tree claims could be anyone's.
    if (own !== undefined && own[0] !== "" && !(own[1] === "rerail-engine" && unclaimedAll(path))) {
      seeds.add(own[1]);
      continue;
    }
    if (unclaimedAll(path)) return everything(`${path} is under native/ and no package's module tree claims it`);
  }
  const selected = new Set();
  const queue = [...seeds];
  while (queue.length > 0) {
    const node = queue.pop();
    if (selected.has(node)) continue;
    selected.add(node);
    for (const dependent of dependents.get(node) ?? []) queue.push(dependent);
  }
  const packages = names.filter((name) => selected.has(name));
  for (const seed of [...seeds].sort()) reasons.push(`${seed} changed`);
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

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [list, flag] = process.argv.slice(2);
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  let selection;
  try {
    const changed = readFileSync(list, "utf8").split("\n").map((line) => line.trim()).filter(Boolean);
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
  if (flag === "--github") {
    for (const [name, value] of Object.entries(outputs(selection))) console.log(`${name}=${value}`);
  } else {
    console.log(JSON.stringify({ ...selection, outputs: outputs(selection) }, null, 2));
  }
}
