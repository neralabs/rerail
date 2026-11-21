// #12 Max files in one flat directory.
//
// Predicate: every code or test directory (a directory with at least one tracked .ts .tsx .mts .cts .js
// .mjs .cjs .rs .py .php .sh file directly in it) holds at most 25 tracked files directly.
// Exemption `pack-data`: `packs/<id>/{rules,candidates,upstream,licenses}/**` are pack data, one file per
// rule or expected findings; their size follows the upstream rule set.
// Exemption `fixture-data`: `fixtures/**` is recorded data laid out per engine feature (#10).
// Exemption `vendored`: `native/grammars/**` is third-party code kept byte-identical.
// Exemption `kept-records` (the `.rerail` workbench and the design notes, kept in git as they are:
// docs/repository-structure.md, "Kept as they are"): each directory KEPT_FLAT lists, exactly (never its
// subdirectories), with its reason.
// Exemption `engine-layout`: each directory of the engine line ENGINE_LAYOUT lists, exactly, until the line's layout
// is split with its owner in a follow-up.

import { compare, dirname, finding, isCode, isDataPath, isVendored, sortFindings } from "../lib.mjs";

export const id = 12;
export const title = "Max files in one flat code/test dir";
export const target = "≤25";

export const LIMIT = 25;

/** Exemption `kept-records`: a directory, and why it stays flat. */
const KEPT_FLAT = new Map([
  ["docs/final-shape/notes/", "the design notes, one file per numbered note, with the scripts a note ran"],
  ["docs/rule-syntax/data-layer/", "the workbench's data-layer rules, one `.rerail` per rule with the script that generates or checks it"],
  ["docs/rule-syntax/packs/javascript/", "the workbench's JavaScript rules, one `.rerail` per pack rule with its generator and check scripts"],
  ["docs/rule-syntax/packs/next/", "the workbench's Next.js rules, one `.rerail` per pack rule with its generator and check scripts"],
  ["docs/rule-syntax/packs/node/", "the workbench's Node rules, one `.rerail` per pack rule with its generator and check scripts"],
  ["docs/rule-syntax/packs/python/", "the workbench's Python rules, one `.rerail` per pack rule with its generator and check scripts"],
  ["docs/rule-syntax/packs/react/", "the workbench's React rules, one `.rerail` per pack rule with its generator and check scripts"],
  ["docs/rule-syntax/packs/sql/", "the workbench's SQL rules, one `.rerail` per pack rule with its generator and check scripts"],
  ["docs/rule-syntax/packs/vue/", "the workbench's Vue rules, one `.rerail` per pack rule with its generator and check scripts"],
]);

/** Exemption `engine-layout`: the engine line's directories, each exactly ("" is the repository root), and why. */
const LAYOUT = "the engine line's layout; split in a follow-up with its owner";
const ENGINE_LAYOUT = new Map([
  ["", "AGENTS.md and CLAUDE.md, the engine line's agent instructions, which agents read only at the repository root"],
  ["native/crates/compiler/src/", LAYOUT],
  ["native/lab/tape16/src/", LAYOUT],
  ["native/src/code_rule_dsl/", LAYOUT],
  ["native/src/code_rule_dsl/compile/", LAYOUT],
  ["native/src/code_rule_dsl/logic/", LAYOUT],
  ["native/src/code_rule_dsl/logic/plan/", LAYOUT],
  ["native/src/code_rule_dsl/logic/plan/corpus/", LAYOUT],
  ["native/src/code_rule_dsl/matcher/", LAYOUT],
  ["native/src/code_rule_dsl/matcher/p_anchors/sql/", LAYOUT],
  ["native/src/code_rule_dsl/sql_executor/", LAYOUT],
  ["native/src/code_rule_dsl/tape/", LAYOUT],
  ["native/src/code_rule_dsl/transducer/packed/", LAYOUT],
  ["native/src/content_store/", LAYOUT],
  ["scripts/profiling/", LAYOUT],
  ["test/engine/native/", LAYOUT],
  ["test/scripts/", LAYOUT],
  ["tools/cratify/", LAYOUT],
]);

function exempt(dir) {
  if (KEPT_FLAT.has(`${dir}/`) || ENGINE_LAYOUT.has(dir === "" ? "" : `${dir}/`)) return true;
  return /^packs\/[^/]+\/(rules|candidates|upstream|licenses)(\/|$)/.test(dir) || isDataPath(`${dir}/`) || isVendored(`${dir}/`);
}

export function check(repo) {
  const counts = new Map();
  const code = new Set();
  for (const path of repo.files) {
    const dir = dirname(path);
    counts.set(dir, (counts.get(dir) ?? 0) + 1);
    if (isCode(path)) code.add(dir);
  }
  const findings = [];
  let max = 0;
  let maxDir = "";
  for (const dir of [...code].sort(compare)) {
    if (exempt(dir)) continue;
    const count = counts.get(dir);
    if (count > max) [max, maxDir] = [count, dir];
    if (count > LIMIT) findings.push(finding(dir === "" ? "." : `${dir}/`, `${count} files directly in this directory (limit ${LIMIT})`));
  }
  return { actual: `${max} (${maxDir === "" ? "." : `${maxDir}/`})`, findings: sortFindings(findings) };
}
