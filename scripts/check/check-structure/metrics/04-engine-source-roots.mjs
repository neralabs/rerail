// #4 Engine source roots.
//
// Predicate: the Rust engine compiles only from `native/`. A tracked `*.rs` file under native/ whose
// `include_str!`/`include_bytes!`/`include!` path resolves outside native/ is a second source root, and
// so is any tracked file under the retired `code-rule-dsl/` directory. Paths are resolved when the macro
// argument is a string literal (relative to the including file) or
// `concat!(env!("CARGO_MANIFEST_DIR"), "…")` (relative to the crate's Cargo.toml); other arguments, such
// as the `fixtures_dir!()` oracle macro or `OUT_DIR`, are not repository paths and are skipped.
// Exemption `test-inputs`: a test-only include (in `tests.rs`, `*_tests.rs`, `test_support.rs`, under
// native/tests/, or after the file's first `#[cfg(test)]`) may read `fixtures/**`, `packs/**`, or a
// `*.generated.*` file: test data and generated contracts the engine is checked against, never compiled
// into the shipped engine.

import { posix } from "node:path";
import { basename, dirname, finding, lineAt, sortFindings } from "../lib.mjs";

export const id = 4;
export const title = "Engine source roots";
export const target = "1 (native/)";

const INCLUDE = /\b(include_str|include_bytes|include)!\s*\(\s*(?:"((?:[^"\\]|\\.)*)"|concat!\s*\(\s*env!\s*\(\s*"CARGO_MANIFEST_DIR"\s*\)\s*,\s*"((?:[^"\\]|\\.)*)"\s*\))/g;

function crateDir(repo, path) {
  for (let dir = dirname(path); dir !== ""; dir = dirname(dir)) {
    if (repo.has(`${dir}/Cargo.toml`)) return dir;
  }
  return "";
}

function isTestOnly(path, text, index) {
  const base = basename(path);
  if (base === "tests.rs" || base.endsWith("_tests.rs") || base === "test_support.rs") return true;
  if (path.startsWith("native/tests/")) return true;
  const cfgTest = text.indexOf("#[cfg(test)]");
  return cfgTest >= 0 && cfgTest < index;
}

function testInput(target) {
  return target.startsWith("fixtures/") || target.startsWith("packs/") || basename(target).includes(".generated.");
}

export function check(repo) {
  const findings = [];
  const roots = new Set(["native"]);
  for (const path of repo.files) {
    if (path.startsWith("code-rule-dsl/")) {
      findings.push(finding(path, "code-rule-dsl/ is retired: engine data lives under native/"));
      roots.add("code-rule-dsl");
    }
    if (!path.startsWith("native/") || !path.endsWith(".rs")) continue;
    const text = repo.text(path);
    INCLUDE.lastIndex = 0;
    for (let match = INCLUDE.exec(text); match; match = INCLUDE.exec(text)) {
      const literal = match[2] ?? match[3];
      const base = match[2] === undefined ? crateDir(repo, path) : dirname(path);
      const target = posix.normalize(posix.join(base, literal.replace(/^\/+/, "")));
      if (target.startsWith("native/")) continue;
      if (isTestOnly(path, text, match.index) && testInput(target)) continue;
      roots.add(target.split("/")[0]);
      findings.push(finding(path, `${match[1]}! reaches outside native/: ${target}`, lineAt(text, match.index)));
    }
  }
  return { actual: String(roots.size), findings: sortFindings(findings) };
}
