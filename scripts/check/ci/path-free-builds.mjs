#!/usr/bin/env node
// No crate of this repository writes its checkout's path into what it compiles (decision 0224), so a compiler cache
// shared by the checkouts (scripts/build/farm) reuses each crate in every other checkout: libraries, build scripts and
// test binaries alike. A crate that read `env!("CARGO_MANIFEST_DIR")` was a different crate in every checkout, and so
// was every crate above it.
//
//   node scripts/check/ci/path-free-builds.mjs [target-dir]
//
// After a build, reads the dep-info rustc wrote for each of the repository's crates (`<target>/**/deps/*.d`, and the
// build scripts' under `build/`): it fails when one records reading CARGO_MANIFEST_DIR, CARGO_MANIFEST_PATH or
// RERAIL_ORACLES_DIR as it compiled (`# env-dep:`). Read them as the program runs instead
// (`std::env::var_os("CARGO_MANIFEST_DIR")`: cargo gives a test, a build script and `cargo run` the same value), or
// find the checkout (`rerail_store::source_tree`). Registry crates are left out: their paths are the same everywhere.
//
// RERAIL_FIXTURES_DIR is read as a test compiles only by `oracle!` (native/src/code_rule_dsl/fixtures.rs), which
// embeds a fixture's text, never its path: the sources are checked to hold no other `env!("RERAIL_FIXTURES_DIR")`.
// The target directory defaults to $CARGO_TARGET_DIR, else target/.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const NAMED = ["CARGO_MANIFEST_DIR", "CARGO_MANIFEST_PATH", "RERAIL_ORACLES_DIR"];
const FIXTURES_MACROS = "native/src/code_rule_dsl/fixtures.rs";

/** The `.d` files under `dir`: every `deps/` and `build/<package>/` directory of every profile and target. */
function depInfoFiles(dir, out = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      // A target directory's own trees; never a crate's OUT_DIR contents or incremental state.
      if (!["incremental", ".fingerprint", "out", "doc", "native-tests"].includes(entry.name)) depInfoFiles(path, out);
    } else if (entry.name.endsWith(".d") && /\/(deps|build\/[^/]+)$/u.test(dir)) {
      out.push(path);
    }
  }
  return out;
}

/** The sources a dep-info's first rule names: what rustc compiled, relative to the working directory or absolute. */
export function sources(text) {
  const first = text.split("\n").find((line) => line.includes(": "));
  if (first === undefined) return [];
  return first
    .slice(first.indexOf(": ") + 2)
    .split(/(?<!\\) /u)
    .filter(Boolean)
    .map((s) => s.replaceAll("\\ ", " "));
}

/** Whether a dep-info is one of this repository's crates: a source in the checkout, not cargo's registry. */
export function isOwn(srcs, root) {
  return srcs.some((s) => !isAbsolute(s) || (s.startsWith(`${root}/`) && !s.includes("/.cargo/")));
}

/** The variables of NAMED a dep-info records the compile read. */
export function readsNamed(text) {
  return NAMED.filter((name) => text.includes(`\n# env-dep:${name}=`) || text.includes(`\n# env-dep:${name}\n`));
}

/** The repository's Rust sources: git's list, or a walk of a copy without git (a build box's synced checkout). */
function rustSources() {
  try {
    return execFileSync("git", ["ls-files", "-z", "--", "*.rs"], { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).split("\0");
  } catch {
    const out = [];
    const walk = (dir) => {
      for (const entry of readdirSync(join(ROOT, dir), { withFileTypes: true })) {
        const path = dir === "" ? entry.name : `${dir}/${entry.name}`;
        if (entry.isDirectory() && !["target", "node_modules", ".git", ".worktrees"].includes(entry.name)) walk(path);
        else if (entry.isFile() && entry.name.endsWith(".rs")) out.push(path);
      }
    };
    walk("");
    return out;
  }
}

/** Sources other than the fixtures' macros that read RERAIL_FIXTURES_DIR as they compile. */
function fixturesReaders() {
  const files = rustSources();
  return files.filter(
    (file) =>
      file !== "" &&
      file !== FIXTURES_MACROS &&
      existsSync(join(ROOT, file)) &&
      /\benv!\(\s*"RERAIL_FIXTURES_DIR"/u.test(readFileSync(join(ROOT, file), "utf8")),
  );
}

function main() {
  const target = resolve(process.argv[2] ?? process.env.CARGO_TARGET_DIR ?? join(ROOT, "target"));
  const problems = [];
  for (const file of fixturesReaders()) {
    problems.push(`${file} reads RERAIL_FIXTURES_DIR as it compiles: use \`fixtures_dir!()\` (read as the test runs)`);
  }
  if (!existsSync(target) || !statSync(target).isDirectory()) {
    console.error(`path-free-builds: no target directory at ${target}: build first`);
    process.exit(2);
  }
  const files = depInfoFiles(target);
  let own = 0;
  for (const file of files) {
    const text = readFileSync(file, "utf8");
    if (!isOwn(sources(text), ROOT)) continue;
    own += 1;
    const named = readsNamed(text);
    if (named.length > 0) {
      const read = sources(text).filter((s) => !isAbsolute(s) || s.startsWith(`${ROOT}/`));
      problems.push(`${relative(target, file)} (${read[0] ?? "?"}) reads ${named.join(", ")} as it compiles`);
    }
  }
  if (problems.length > 0) {
    console.error(`path-free-builds: ${problems.length} crate build(s) write their checkout's path in (decision 0224):`);
    for (const problem of problems) console.error(`  ${problem}`);
    console.error(
      "Read the variable as the program runs (`std::env::var_os(\"CARGO_MANIFEST_DIR\")`), or find the checkout with " +
        "`rerail_store::source_tree` (scripts/check/ci/path-free-builds.mjs).",
    );
    process.exit(1);
  }
  console.log(`path-free-builds: ${own} of the repository's crate builds read no checkout path as they compile`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
