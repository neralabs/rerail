#!/usr/bin/env node
// Nodes live in tapes (decision 0026, after ast-index-spike: "nothing may store one row per node",
// t4 §1.1, Representation rules 1-4, dead end D14). Nothing else may hold a value per node: a
// node's kind, field, flags, span, parent, children and position are read from the tape's columns
// or computed in the walk that visits it. The only structure over nodes is a bare index: a key
// (a kind, a term, a predicate) to the set of nodes that have it (postings, node sets).
//
// Rules, over the engine's non-test Rust (`native/src`, with `#[cfg(test)]` modules resolved from
// `lib.rs`) and the design documents (`docs/final-shape/*.md`):
//   no-sidecar          the word "sidecar": a structure kept beside the tape is a second
//                       architecture for the same nodes.
//   node-columns        a struct with two or more collection fields named as node attributes
//                       (`parent`, `kind`, `start`, ...): a struct of arrays indexed by node.
//   node-records        a struct with four or more scalar node-attribute fields: a record per node
//                       (TapeDoc's `Rec`, Metal's `MetalNode`).
//   node-allocation     `vec![_; n]` or `Vec::with_capacity(n)` with `n` a tape's node count: an
//                       array indexed by node. A node set (`div_ceil(64)` words of bits) is an
//                       index result and is allowed.
// Stored relations are checked by the store test `no_relation_stores_a_row_per_node` (at most 0.25
// rows per node per table, as the spike's enforcement test).
//
// Structures that exist today are listed in node-structures-baseline.json, each with the list step
// that removes it. The check fails on any structure not listed, and on a listed one that is gone
// (remove its entry: the list only shrinks).
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const SRC = join(ROOT, "native/src");
const BASELINE = join(dirname(fileURLToPath(import.meta.url)), "node-structures-baseline.json");

/** Names a field takes when it holds one value per node. */
const NODE_FIELDS = new Set([
  "par", "parent", "parents", "kind", "kinds", "field", "fields", "flags", "start", "end", "starts", "ends",
  "size", "sizes", "subtree_end", "descendant_end", "start_line", "end_line", "cidx", "cidx_named", "depth",
  "first_child", "next_sibling", "prev_sibling", "previous_sibling", "child_start", "child_count",
  "next_any", "next_named", "source_start", "source_end", "tid", "text_hash", "local", "first", "next", "prev",
]);
const NODE_COUNT = /transducer_len\(\)|node_count\b|\bnodes\.len\(\)|\bn_nodes\b|\bnode_len\b/u;
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

function structs(lines) {
  const found = [];
  for (let i = 0; i < lines.length; i++) {
    const head = /^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+(\w+)[^;{(]*\{\s*$/u.exec(lines[i]);
    if (!head) continue;
    let attributes = "";
    for (let j = i - 1; j >= 0 && /^\s*(#\[|\/\/)/u.test(lines[j]); j--) attributes = lines[j] + attributes;
    if (TEST_CFG.test(attributes)) continue;
    const fields = [];
    for (let j = i + 1; j < lines.length && !/^\s*\}/u.test(lines[j]); j++) {
      const field = /^\s*(?:pub(?:\([^)]*\))?\s+)?(\w+)\s*:\s*(.+?),?\s*$/u.exec(lines[j]);
      if (field) fields.push({ name: field[1], type: field[2] });
    }
    found.push({ name: head[1], line: i + 1, repr: /#\[repr\(C/u.test(attributes), fields });
  }
  return found;
}

const COLLECTION = /^(?:Vec<|Box<\[|Arc<\[|Rc<\[|Arc<Vec<|&\[|&'\w+ \[)/u;

function violations() {
  const out = [];
  for (const file of productionFiles()) {
    const path = relative(ROOT, file);
    const lines = productionText(readFileSync(file, "utf8"));
    lines.forEach((line, i) => {
      if (/sidecar/iu.test(line)) out.push({ rule: "no-sidecar", path, key: line.trim(), line: i + 1 });
      const allocation = /vec!\[[^\]]*;\s*([^\]]+)\]|Vec::with_capacity\(([^)]*\)?)\)/u.exec(line);
      const size = allocation && (allocation[1] ?? allocation[2]);
      if (size && NODE_COUNT.test(size) && !/div_ceil\(64\)/u.test(size)) {
        out.push({ rule: "node-allocation", path, key: line.trim(), line: i + 1 });
      }
    });
    for (const struct of structs(lines)) {
      const columns = struct.fields.filter((field) => NODE_FIELDS.has(field.name) && COLLECTION.test(field.type));
      if (columns.length >= 2) {
        out.push({ rule: "node-columns", path, key: struct.name, line: struct.line, detail: columns.map((f) => f.name).join(", ") });
      }
      const scalars = struct.fields.filter((field) => NODE_FIELDS.has(field.name) && !COLLECTION.test(field.type));
      if (scalars.length >= 4) {
        out.push({ rule: "node-records", path, key: struct.name, line: struct.line, detail: scalars.map((f) => f.name).join(", ") });
      }
    }
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
const id = (entry) => `${entry.rule} ${entry.path} ${entry.key}`;
if (process.argv.includes("--list")) {
  for (const entry of found) console.log(JSON.stringify({ rule: entry.rule, path: entry.path, key: entry.key, detail: entry.detail }));
  process.exit(0);
}
const listing = existsSync(BASELINE) ? JSON.parse(readFileSync(BASELINE, "utf8")) : { tape: [], structures: [] };
// The tape's own columns, and structures over something other than nodes: kept, each with its reason.
const exempt = new Set(listing.tape.map(id));
const baseline = listing.structures;
const listed = new Map(baseline.map((entry) => [id(entry), entry]));
const seen = new Set(found.map(id));
const fresh = found.filter((entry) => !listed.has(id(entry)) && !exempt.has(id(entry)));
const gone = [...baseline, ...listing.tape].filter((entry) => !seen.has(id(entry)));
for (const entry of fresh) {
  console.error(`${entry.path}:${entry.line}: ${entry.rule}: ${entry.key}${entry.detail ? ` (${entry.detail})` : ""}`);
}
for (const entry of gone) console.error(`node-structures-baseline.json: ${entry.rule} ${entry.path} ${entry.key} is gone; remove its entry`);
if (fresh.length || gone.length) {
  console.error(`\n${fresh.length} per-node structure(s) not in the baseline, ${gone.length} stale baseline entr(y/ies). Nodes live in tapes (decision 0026).`);
  process.exit(1);
}
console.log(`check-node-structures: no new per-node structures (${baseline.length} listed for removal)`);
