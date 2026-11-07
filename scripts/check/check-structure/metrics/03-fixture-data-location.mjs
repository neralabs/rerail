// #3 Fixture data outside `fixtures/` and `packs/<id>/upstream/`, or pack test data in git or unpinned.
//
// Predicate (owned by ../fixtures.mjs, `fixtureLayout(repo).outside`): a tracked file is fixture data when
// a directory in its path is named `fixtures`/`__fixtures__`, its basename matches
// /(^|[-_.])fixtures?([-_.]|$)/i, it is a `*.expected.json`/`*.cases.json` recording, or it is a code-rule
// JSON document; it may live only under `fixtures/`, `packs/<id>/upstream/`, or (rule documents only)
// `packs/<id>/rules/`.
// Exemption `pack-candidates` (in fixtures.mjs): `packs/<id>/candidates/**` holds candidate ports and their
// hand-labelled probe packs, pack data scored but never shipped.
// Pack test data, a path PACK_FIXTURE_PATTERNS (scripts/s3/pack-fixtures.mjs) matches, derived or authored, lives in
// one archive per tool-pack in S3, pinned by sha256 in the tool directory's tracked `test-data.lock`. No tracked file
// may be pack test data: a pull request that adds or changes it tracks it with `git add -f` for review, and before
// merge a maintainer runs `npm run fixtures:push -- --pack P --tool T`, which archives and uploads it and rewrites the
// lock, then untracks it (CONTRIBUTING.md, "Pack test data"). Every tool directory (`packs/<id>/upstream/<tool>/`
// with a tracked file) has a tracked `test-data.lock` that `parseLock` (scripts/s3/test-data-archive.mjs) accepts.
// No exemptions.

import { isPackFixture } from "../../../s3/pack-fixtures.mjs";
import { parseLock, TEST_DATA_LOCK } from "../../../s3/test-data-archive.mjs";
import { fixtureLayout } from "../fixtures.mjs";
import { finding, sortFindings } from "../lib.mjs";

export const id = 3;
export const title = "Fixture data outside `fixtures/` and `packs/*/upstream`, or pack test data in git or unpinned";
export const target = "0";

/** A file in a tool directory, `packs/<pack>/upstream/<tool>/`; every pack test data path is one. */
const TOOL_FILE = /^packs\/([^/]+)\/upstream\/([^/]+)\/./u;

const push = (pack, tool) => `npm run fixtures:push -- --pack ${pack} --tool ${tool}`;

/** The tool directories (`packs/<pack>/upstream/<tool>/` holding a tracked file) by path, with their pack and tool. */
function toolDirectories(repo) {
  const directories = new Map();
  for (const path of repo.files) {
    const match = TOOL_FILE.exec(path);
    if (match) directories.set(`packs/${match[1]}/upstream/${match[2]}/`, { pack: match[1], tool: match[2] });
  }
  return directories;
}

/** A tool directory's `test-data.lock` finding when it is untracked or parseLock rejects it. */
function lockFindings(repo, directory, { pack, tool }) {
  const path = `${directory}${TEST_DATA_LOCK}`;
  if (!repo.has(path)) return [finding(path, `the tool-pack's test data archive is unpinned: \`${push(pack, tool)}\` writes it`)];
  try {
    parseLock(repo.text(path), path);
    return [];
  } catch (error) {
    return [finding(path, error instanceof Error ? error.message : String(error))];
  }
}

function packFixtureFinding(path) {
  const [, pack, tool] = TOOL_FILE.exec(path);
  return finding(
    path,
    `the tool-pack's archive holds pack test data: before merge a maintainer runs \`${push(pack, tool)}\` and \`git rm --cached\` it`,
  );
}

export function check(repo) {
  const { outside } = fixtureLayout(repo);
  const findings = [
    ...outside.map((path) => finding(path, "fixture data belongs in fixtures/<family>/ or packs/<id>/upstream/<tool>/")),
    ...repo.files.filter((path) => isPackFixture(path)).map(packFixtureFinding),
    ...[...toolDirectories(repo)].flatMap(([directory, names]) => lockFindings(repo, directory, names)),
  ];
  return { actual: String(findings.length), findings: sortFindings(findings) };
}
