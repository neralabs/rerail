#!/usr/bin/env node
// Nodes live in tapes (decision 0026, after the SQL index prototype: "nothing may store one row per node",
// t4 §1.1, Representation rules 1-4; dead end D14, persisted sidecars). The rule is about what is
// stored: the store tests `no_relation_stores_a_row_per_node` and S7 hold every table to at most
// 0.25 rows per node. In-memory state a reader builds from a tape is not covered.
//
// This check keeps the design's vocabulary to that rule, over the engine's non-test Rust
// (`native/src` and each crate's `src/`, `#[cfg(test)]` modules resolved from each `lib.rs`) and the design documents
// (`docs/final-shape/*.md`):
//   no-sidecar          the word "sidecar": a structure kept beside the tape under its digest is
//                       either stored (ruled out) or a reader's working state (not a structure of
//                       the design).
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const SRC = join(ROOT, "native/src");

/** A cfg that only test builds enable. */
const TEST_CFG = /#\[cfg\((?:all\()?\s*(?:test|feature\s*=\s*"metal-tests")/u;

/** The module trees from the engine's `lib.rs` and each crate's: each non-test file. A module's file is found as
 * rustc finds it: `mod m;` in a `mod.rs`, `lib.rs` or `#[path]` file is beside it, elsewhere under the file's stem;
 * an inline `mod m { ... }` adds `m/` (or its `#[path]`); a `#[path]` is relative to the file's directory at its top
 * level and to the inline module's directory inside one. Each crate's sources are in its own `src/`. */
function productionFiles() {
  const files = new Set();
  const visit = (file, modRs) => {
    if (files.has(file) || !existsSync(file)) return;
    files.add(file);
    const own = modRs ? dirname(file) : join(dirname(file), file.slice(dirname(file).length + 1).replace(/\.rs$/u, ""));
    // Each open inline module: its directory and the brace depth it closes at.
    const inline = [];
    let depth = 0;
    let skipUntil = -1;
    const lines = readFileSync(file, "utf8").split("\n");
    for (let i = 0; i < lines.length; i++) {
      const code = lines[i].replace(/\/\/.*$/u, "");
      const declaration = /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z0-9_]+)\s*([;{])/u.exec(code);
      if (declaration && skipUntil < 0) {
        let attributes = "";
        for (let j = i - 1; j >= 0 && /^\s*(#\[|\/\/)/u.test(lines[j]); j--) attributes = lines[j] + attributes;
        const path = /#\[path\s*=\s*"([^"]+)"\]/u.exec(attributes);
        const name = declaration[1];
        const here = inline.length ? inline[inline.length - 1].dir : own;
        const pathBase = inline.length ? inline[inline.length - 1].dir : dirname(file);
        if (TEST_CFG.test(attributes)) {
          if (declaration[2] === "{") skipUntil = depth;
        } else if (declaration[2] === "{") {
          inline.push({ dir: path ? join(pathBase, path[1]) : join(here, name), closes: depth });
        } else if (path) {
          visit(join(pathBase, path[1]), true);
        } else {
          const flat = join(here, `${name}.rs`);
          if (existsSync(flat)) visit(flat, false);
          else visit(join(here, name, "mod.rs"), true);
        }
      }
      for (const ch of code) {
        if (ch === "{") depth++;
        else if (ch === "}") {
          depth--;
          if (skipUntil >= 0 && depth === skipUntil) skipUntil = -1;
          while (inline.length && depth === inline[inline.length - 1].closes) inline.pop();
        }
      }
    }
  };
  visit(join(SRC, "lib.rs"), true);
  const crates = join(ROOT, "native/crates");
  for (const crate of readdirSync(crates).sort()) visit(join(crates, crate, "src", "lib.rs"), true);
  return [...files];
}

/** A file's lines with each test-only item (a `#[cfg(test)]` module, function, impl, ...) blanked:
 * from its attributes to the brace that closes it, or to its `;`. Braces in strings and comments
 * are counted too, which is close enough for Rust as rustfmt lays it out. */
function productionText(text) {
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    if (!TEST_CFG.test(lines[i])) continue;
    let j = i + 1;
    while (j < lines.length && /^\s*(#\[|\/\/)/u.test(lines[j])) j++;
    let depth = 0;
    let opened = false;
    let end = j;
    for (; end < lines.length; end++) {
      const code = lines[end].replace(/\/\/.*$/u, "");
      for (const ch of code) {
        if (ch === "{") (depth++, (opened = true));
        else if (ch === "}") depth--;
      }
      if ((opened && depth <= 0) || (!opened && /;\s*$/u.test(code))) break;
    }
    for (let k = i; k <= Math.min(end, lines.length - 1); k++) lines[k] = "";
    i = end;
  }
  return lines;
}

function violations() {
  const out = [];
  for (const file of productionFiles()) {
    const path = relative(ROOT, file);
    const lines = productionText(readFileSync(file, "utf8"));
    lines.forEach((line, i) => {
      if (/sidecar/iu.test(line)) out.push({ rule: "no-sidecar", path, key: line.trim(), line: i + 1 });
    });
  }
  const docs = join(ROOT, "docs/final-shape");
  for (const name of readdirSync(docs).filter((name) => name.endsWith(".md")).sort()) {
    const path = relative(ROOT, join(docs, name));
    readFileSync(join(docs, name), "utf8").split("\n").forEach((line, i) => {
      if (/sidecar/iu.test(line)) out.push({ rule: "no-sidecar", path, key: line.trim().slice(0, 120), line: i + 1 });
    });
  }
  return out;
}

const found = violations();
for (const entry of found) console.error(`${entry.path}:${entry.line}: ${entry.rule}: ${entry.key}`);
if (found.length) {
  console.error(`\n${found.length} use(s) of "sidecar". Nodes live in tapes; nothing is stored beside them (decision 0026).`);
  process.exit(1);
}
console.log("check-node-structures: no sidecars");
