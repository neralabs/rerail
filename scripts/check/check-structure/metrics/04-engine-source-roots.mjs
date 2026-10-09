// #4 Engine source roots.
//
// Predicate: the Rust engine compiles only from `native/`. A tracked `*.rs` file under native/ whose
// `include_str!`/`include_bytes!`/`include!` path resolves outside native/ is a second source root, and
// so is any tracked file under the retired `code-rule-dsl/` directory. Paths are resolved when the macro
// argument is a string literal (relative to the including file) or
// `concat!(env!("CARGO_MANIFEST_DIR"), "…")` (relative to the crate's Cargo.toml); other arguments, such
// as the `fixtures_dir!()` oracle macro or `OUT_DIR`, are not repository paths and are skipped.
// Exemption `test-inputs`: a test-only include (in `tests.rs`, `*_tests.rs`, `test_support.rs`, under
// native/tests/, or inside the item a `#[cfg(test)]` attribute applies to) may read `fixtures/**`,
// `packs/**`, or a `*.generated.*` file: test data and generated contracts the engine is checked against,
// never compiled into the shipped engine. A `#[cfg(test)]` item extends to its brace-matched body, or to
// its `;` when it has none, so `#[cfg(test)] mod tests;` exempts nothing that follows it. Comments,
// string and character literals are skipped when matching braces.

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

const RAW_STRING = /b?r(#*)"/y;
const CHAR = /'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^\\'\n])'/uy;

/** `text` with comments and the contents of string and character literals blanked, offsets kept. */
function code(text) {
  const out = text.split("");
  const blank = (from, to) => {
    for (let i = from; i < to && i < out.length; i += 1) if (out[i] !== "\n") out[i] = " ";
  };
  let i = 0;
  while (i < text.length) {
    RAW_STRING.lastIndex = i;
    CHAR.lastIndex = i;
    const raw = /\w/.test(text[i - 1] ?? "") ? null : RAW_STRING.exec(text);
    const char = text[i] === "'" ? CHAR.exec(text) : null;
    let end;
    if (text.startsWith("//", i)) {
      end = text.indexOf("\n", i);
      if (end < 0) end = text.length;
      blank(i, end);
    } else if (text.startsWith("/*", i)) {
      let depth = 0;
      end = i;
      do {
        if (text.startsWith("/*", end)) depth += 1;
        else if (text.startsWith("*/", end)) depth -= 1;
        end += text.startsWith("/*", end) || text.startsWith("*/", end) ? 2 : 1;
      } while (depth > 0 && end < text.length);
      blank(i, end);
    } else if (raw) {
      const close = text.indexOf(`"${raw[1]}`, i + raw[0].length);
      end = close < 0 ? text.length : close + 1 + raw[1].length;
      blank(i + raw[0].length, close < 0 ? end : close);
    } else if (text[i] === '"') {
      end = i + 1;
      while (end < text.length && text[end] !== '"') end += text[end] === "\\" ? 2 : 1;
      blank(i + 1, end);
      end += 1;
    } else if (char) {
      end = i + char[0].length;
      blank(i + 1, end - 1);
    } else {
      end = i + 1;
    }
    i = end;
  }
  return out.join("");
}

const CFG_TEST = /#\[\s*cfg\s*\(\s*test\s*\)\s*\]/g;

/** The [start, end) spans of the items a `#[cfg(test)]` attribute applies to. */
function testSpans(text) {
  const source = code(text);
  const spans = [];
  CFG_TEST.lastIndex = 0;
  for (let match = CFG_TEST.exec(source); match; match = CFG_TEST.exec(source)) {
    let depth = 0;
    let end = source.length;
    for (let i = match.index + match[0].length; i < source.length; i += 1) {
      const c = source[i];
      if (c === "(" || c === "[" || c === "{") depth += 1;
      else if (c === ")" || c === "]") depth -= 1;
      else if (c === "}") {
        depth -= 1;
        if (depth === 0) {
          end = i + 1;
          break;
        }
      } else if (c === ";" && depth === 0) {
        end = i + 1;
        break;
      }
    }
    spans.push([match.index, end]);
  }
  return spans;
}

function isTestFile(path) {
  const base = basename(path);
  return base === "tests.rs" || base.endsWith("_tests.rs") || base === "test_support.rs" || path.startsWith("native/tests/");
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
    const spans = isTestFile(path) ? null : testSpans(text);
    const isTestOnly = (index) => spans === null || spans.some(([start, end]) => start <= index && index < end);
    INCLUDE.lastIndex = 0;
    for (let match = INCLUDE.exec(text); match; match = INCLUDE.exec(text)) {
      const literal = match[2] ?? match[3];
      const base = match[2] === undefined ? crateDir(repo, path) : dirname(path);
      const target = posix.normalize(posix.join(base, literal.replace(/^\/+/, "")));
      if (target.startsWith("native/")) continue;
      if (isTestOnly(match.index) && testInput(target)) continue;
      roots.add(target.split("/")[0]);
      findings.push(finding(path, `${match[1]}! reaches outside native/: ${target}`, lineAt(text, match.index)));
    }
  }
  return { actual: String(roots.size), findings: sortFindings(findings) };
}
