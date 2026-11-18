// #1 `pack.json` outside `packs/`.
//
// Predicate: every tracked file named `pack.json` is `packs/<id>/pack.json` (a pack's manifest).
// Exemption `validator-inputs`: `fixtures/packs/**/pack.json` are inputs to the pack validator's tests,
// not packs; they live with the other fixture data.
// Exemption `pack-test-data`: under `packs/<id>/upstream/` a pack.json is pack test data (a recorded copy of a pack, or
// one in a publish request on its way to S3), never a pack.
// Exemption `workbench` (the `.rr` workbench, kept in git as it is: docs/repository-structure.md, "Kept as they are"):
// `docs/rule-syntax/packs/python-main/source/pack.json` is main's Python pack manifest as the workbench translated it,
// the source of the `.rr` rules beside it; never loaded as a pack.

import { basename, finding, sortFindings } from "../lib.mjs";

export const id = 1;
export const title = "`pack.json` outside `packs/<id>/`";
export const target = "0";

/** Exemption `workbench`. */
const WORKBENCH = "docs/rule-syntax/packs/python-main/source/pack.json";

export function check(repo) {
  const findings = [];
  for (const path of repo.files) {
    if (basename(path) !== "pack.json") continue;
    if (/^packs\/[^/]+\/pack\.json$/.test(path)) continue;
    if (path.startsWith("fixtures/packs/")) continue;
    if (/^packs\/[^/]+\/upstream\//.test(path) || path === WORKBENCH) continue;
    findings.push(finding(path, "pack.json belongs only in packs/<id>/ (validator inputs: fixtures/packs/)"));
  }
  return { actual: String(findings.length), findings: sortFindings(findings) };
}
