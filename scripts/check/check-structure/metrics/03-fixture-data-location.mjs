// #3 Fixture data outside `fixtures/` and `packs/<id>/upstream/`, or pack test data in git.
//
// Predicate (owned by ../fixtures.mjs, `fixtureLayout(repo).outside`): a tracked file is fixture data when
// a directory in its path is named `fixtures`/`__fixtures__`, its basename matches
// /(^|[-_.])fixtures?([-_.]|$)/i, it is a `*.expected.json`/`*.cases.json` recording, or it is a code-rule
// JSON document; it may live only under `fixtures/`, `packs/<id>/upstream/`, or (rule documents only)
// `packs/<id>/rules/`.
// Exemption `pack-candidates` (in fixtures.mjs): `packs/<id>/candidates/**` holds candidate ports and their
// hand-labelled probe packs, pack data scored but never shipped.
// Pack test data, a path PACK_FIXTURE_PATTERNS (scripts/s3/pack-fixtures.mjs) matches, lives in S3, not git: no tracked
// file may be pack test data. A pull request that adds or changes it tracks it with `git add -f` for review; before
// merge a maintainer runs `npm run fixtures:push`, which uploads it and untracks it (CONTRIBUTING.md, "Pack test data").

import { isPackFixture } from "../../../s3/pack-fixtures.mjs";
import { fixtureLayout } from "../fixtures.mjs";
import { finding, sortFindings } from "../lib.mjs";

export const id = 3;
export const title = "Fixture data outside `fixtures/` and `packs/*/upstream`, or pack test data in git";
export const target = "0";

export function check(repo) {
  const { outside } = fixtureLayout(repo);
  const findings = [
    ...outside.map((path) => finding(path, "fixture data belongs in fixtures/<family>/ or packs/<id>/upstream/<tool>/")),
    ...repo.files
      .filter((path) => isPackFixture(path))
      .map((path) => finding(path, "pack test data is in S3; a maintainer runs `npm run fixtures:push` before merge")),
  ];
  return { actual: String(findings.length), findings: sortFindings(findings) };
}
