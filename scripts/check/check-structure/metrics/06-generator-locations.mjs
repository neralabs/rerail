// #6 Generators follow one known pattern.
//
// Predicate: every tracked generator script (basename `generate`, `generate_*` or `generate-*`, extension
// .mjs .cjs .js .ts .mts .py .php; not `generated*`) lives under `scripts/generate/` (generated engine,
// host and pack data, listed in scripts/generate/README.md) or beside the recordings it writes, under
// `fixtures/` or
// `packs/<id>/upstream/` (the fixtures convention, fixtures/README.md). In particular none sits in the
// engine's source (`native/src/`, `src/`) next to its output.

import { basename, finding, sortFindings } from "../lib.mjs";

export const id = 6;
export const title = "Generators follow one known pattern";
export const target = "0 misplaced";

const GENERATOR = /^generate(?:[-_][^/]*)?\.(mjs|cjs|js|ts|mts|py|php)$/;

export function check(repo) {
  const findings = [];
  for (const path of repo.files) {
    if (!GENERATOR.test(basename(path))) continue;
    if (path.startsWith("scripts/generate/") || path.startsWith("fixtures/")) continue;
    if (/^packs\/[^/]+\/upstream\//.test(path)) continue;
    findings.push(
      finding(path, "generator outside scripts/generate/ (engine/host/pack data) or fixtures/, packs/<id>/upstream/ (recordings)"),
    );
  }
  return { actual: String(findings.length), findings: sortFindings(findings) };
}
