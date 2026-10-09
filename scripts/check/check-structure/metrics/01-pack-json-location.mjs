// #1 `pack.json` outside `packs/`.
//
// Predicate: every tracked file named `pack.json` is `packs/<id>/pack.json` (a pack's manifest).
// Exemption `validator-inputs`: `fixtures/packs/**/pack.json` are inputs to the pack validator's tests,
// not packs; they live with the other fixture data.

import { basename, finding, sortFindings } from "../lib.mjs";

export const id = 1;
export const title = "`pack.json` outside `packs/<id>/`";
export const target = "0";

export function check(repo) {
  const findings = [];
  for (const path of repo.files) {
    if (basename(path) !== "pack.json") continue;
    if (/^packs\/[^/]+\/pack\.json$/.test(path)) continue;
    if (path.startsWith("fixtures/packs/")) continue;
    findings.push(finding(path, "pack.json belongs only in packs/<id>/ (validator inputs: fixtures/packs/)"));
  }
  return { actual: String(findings.length), findings: sortFindings(findings) };
}
