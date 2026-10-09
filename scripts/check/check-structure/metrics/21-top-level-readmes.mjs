// #21 Top-level directories have a README.
//
// Predicate: every top-level directory holding tracked files has a tracked `README.md` directly in it.
// Exemption `dot-directories`: directories starting with `.` (`.github/`, `.cargo/`, `.changeset/`, the
// host plugin manifests) are tool configuration, read by tools, not people browsing the tree.
// Exemption `reuse-licenses`: `LICENSES/` is REUSE's licence-text directory; REUSE rejects any file in it
// that is not a licence text.

import { finding, sortFindings, topLevelDirectories } from "../lib.mjs";

export const id = 21;
export const title = "Top-level dirs have a README";
export const target = "100%";

export function check(repo) {
  const dirs = topLevelDirectories(repo).filter((dir) => !dir.startsWith(".") && dir !== "LICENSES");
  const findings = dirs
    .filter((dir) => !repo.has(`${dir}/README.md`))
    .map((dir) => finding(`${dir}/README.md`, "missing: every top-level directory explains itself"));
  const passing = dirs.length - findings.length;
  const percent = dirs.length === 0 ? 100 : Math.floor((passing * 100) / dirs.length);
  return { actual: `${percent}% (${passing}/${dirs.length})`, findings: sortFindings(findings) };
}
