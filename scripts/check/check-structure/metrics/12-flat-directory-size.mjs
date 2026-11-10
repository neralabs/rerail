// #12 Max files in one flat directory.
//
// Predicate: every code or test directory (a directory with at least one tracked .ts .tsx .mts .cts .js
// .mjs .cjs .rs .py .php .sh file directly in it) holds at most 25 tracked files directly.
// Exemption `pack-data`: `packs/<id>/{rules,candidates,upstream,licenses}/**` are pack data, one file per
// rule or expected findings; their size follows the upstream rule set.
// Exemption `fixture-data`: `fixtures/**` is recorded data laid out per engine feature (#10).
// Exemption `vendored`: `native/grammars/**` is third-party code kept byte-identical.

import { compare, dirname, finding, isCode, isDataPath, isVendored, sortFindings } from "../lib.mjs";

export const id = 12;
export const title = "Max files in one flat code/test dir";
export const target = "≤25";

export const LIMIT = 25;

function exempt(dir) {
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
