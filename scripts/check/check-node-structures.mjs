#!/usr/bin/env node
// Nodes live in tapes (decision 0026, after ast-index-spike: "nothing may store one row per node",
// t4 §1.1, Representation rules 1-4; dead end D14, persisted sidecars). The rule is about what is
// stored: the store tests `no_relation_stores_a_row_per_node` and S7 hold every table to at most
// 0.25 rows per node. In-memory state a reader builds from a tape is not covered.
//
// This check keeps the design's vocabulary to that rule, over the engine's non-test Rust
// (`native/src`, `#[cfg(test)]` modules resolved from `lib.rs`) and the design documents
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

/** The module tree from lib.rs: each non-test file. */
function productionFiles() {
  const files = [];
  const visit = (file, dir) => {
    if (!existsSync(file)) return;
    const text = readFileSync(file, "utf8");
    files.push(file);
    const lines = text.split("\n");
    for (let i = 0; i < lines.length; i++) {
      const declaration = /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z0-9_]+)\s*;/u.exec(lines[i]);
      if (!declaration) continue;
      let attributes = "";
      for (let j = i - 1; j >= 0 && /^\s*(#\[|\/\/)/u.test(lines[j]); j--) attributes = lines[j] + attributes;
      if (TEST_CFG.test(attributes)) continue;
      const path = /#\[path\s*=\s*"([^"]+)"\]/u.exec(attributes);
      const name = declaration[1];
      const candidates = path ? [join(dir, path[1])] : [join(dir, `${name}.rs`), join(dir, name, "mod.rs")];
      const child = candidates.find((candidate) => existsSync(candidate));
      if (!child) continue;
      const childDir = child.endsWith("mod.rs") ? dirname(child) : join(dirname(child), name);
      visit(child, path ? dirname(child) : childDir);
    }
  };
  visit(join(SRC, "lib.rs"), SRC);
  return files;
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
