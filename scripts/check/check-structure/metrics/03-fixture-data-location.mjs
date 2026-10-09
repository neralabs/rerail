// #3 Fixture data outside `fixtures/` and `packs/<id>/upstream/`.
//
// Predicate (owned by ../fixtures.mjs, `fixtureLayout(repo).outside`): a tracked file is fixture data when
// a directory in its path is named `fixtures`/`__fixtures__`, its basename matches
// /(^|[-_.])fixtures?([-_.]|$)/i, it is a `*.expected.json`/`*.cases.json` recording, or it is a code-rule
// JSON document; it may live only under `fixtures/`, `packs/<id>/upstream/`, or (rule documents only)
// `packs/<id>/rules/`.
// Exemption `pack-candidates` (in fixtures.mjs): `packs/<id>/candidates/**` holds candidate ports and their
// hand-labelled probe packs, pack data scored but never shipped.

import { fixtureLayout } from "../fixtures.mjs";
import { finding, sortFindings } from "../lib.mjs";

export const id = 3;
export const title = "Fixture data outside `fixtures/` and `packs/*/upstream`";
export const target = "0";

export function check(repo) {
  const { outside } = fixtureLayout(repo);
  const findings = outside.map((path) =>
    finding(path, "fixture data belongs in fixtures/<family>/ or packs/<id>/upstream/<tool>/"),
  );
  return { actual: String(findings.length), findings: sortFindings(findings) };
}
