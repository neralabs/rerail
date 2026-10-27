// #3 Fixture data outside `fixtures/` and `packs/<id>/upstream/`, pack test data in git or unpinned, or pack data
// waiting to publish.
//
// Predicate (owned by ../fixtures.mjs, `fixtureLayout(repo).outside`): a tracked file is fixture data when
// a directory in its path is named `fixtures`/`__fixtures__`, its basename matches
// /(^|[-_.])fixtures?([-_.]|$)/i, it is a `*.expected.json`/`*.cases.json` recording, or it is a code-rule
// JSON document; it may live only under `fixtures/`, `packs/<id>/upstream/`, or (rule documents only)
// `packs/<id>/rules/`.
// Exemption `pack-candidates` (in fixtures.mjs): `packs/<id>/candidates/**` holds candidate ports and their
// hand-labelled probe packs, pack data scored but never shipped.
// Pack test data, a path PACK_FIXTURE_PATTERNS (scripts/s3/pack-fixtures.mjs) matches, derived or authored, lives in
// one archive per tool-pack in S3, pinned by sha256 in the tool directory's tracked `test-data.lock`, so no tracked file
// may be pack test data in that pulled layout. A pull request publishes new or changed data through the tool-pack's
// tracked `publish/` (`npm run fixtures:stage` puts it there): the publish workflow
// (.github/workflows/pack-data-publish.yml) uploads it after a code owner approves, then commits the new locks and
// deletes `publish/`. While any `publish/` is tracked, this metric fails once per tool-pack, so the pull request cannot
// merge before its data is in S3 and its locks name it (CONTRIBUTING.md, "Pack test data"). Every tool directory
// (`packs/<id>/upstream/<tool>/` with a tracked file outside `publish/`) has a tracked `test-data.lock` that
// `parseLock` (scripts/s3/test-data-archive.mjs) accepts, and its `samples.lock`, which pins its real-code samples
// archive the same way and is optional, must be one parseLock accepts too. No exemptions.

import { isPackFixture, PUBLISH_DIRECTORY, PUBLISH_WORKFLOW, publishPath, SAMPLES_LOCK } from "../../../s3/pack-fixtures.mjs";
import { parseLock, TEST_DATA_LOCK } from "../../../s3/test-data-archive.mjs";
import { fixtureLayout } from "../fixtures.mjs";
import { finding, sortFindings } from "../lib.mjs";

export const id = 3;
export const title = "Fixture data outside `fixtures/` and `packs/*/upstream`, pack test data in git or unpinned, or pack data waiting to publish";
export const target = "0";

/** A file in a tool directory, `packs/<pack>/upstream/<tool>/`; every pack test data path is one. */
const TOOL_FILE = /^packs\/([^/]+)\/upstream\/([^/]+)\/./u;
/** How many of a tool-pack's publish/ paths its finding names. */
const SHOWN_PUBLISH_PATHS = 5;

/** The tool directories (`packs/<pack>/upstream/<tool>/` holding a tracked file outside publish/) by path, with their pack and tool. */
function toolDirectories(repo) {
  const directories = new Map();
  for (const path of repo.files) {
    const match = TOOL_FILE.exec(path);
    if (match && publishPath(path) === undefined) directories.set(`packs/${match[1]}/upstream/${match[2]}/`, { pack: match[1], tool: match[2] });
  }
  return directories;
}

/** A lock's finding when parseLock rejects it. */
function malformedLock(repo, path) {
  try {
    parseLock(repo.text(path), path);
    return [];
  } catch (error) {
    return [finding(path, error instanceof Error ? error.message : String(error))];
  }
}

/**
 * A tool directory's `test-data.lock` finding when it is malformed, or untracked while the tool-pack publishes nothing
 * (a publish writes it), and its `samples.lock`'s when malformed.
 */
function lockFindings(repo, directory, { pack, tool }) {
  const path = `${directory}${TEST_DATA_LOCK}`;
  const samples = `${directory}${SAMPLES_LOCK}`;
  const publishing = repo.files.some((file) => file.startsWith(`${directory}${PUBLISH_DIRECTORY}/`));
  const unpinned = !repo.has(path) && !publishing;
  return [
    ...(unpinned ? [finding(path, `the tool-pack's test data archive is unpinned: stage its data into packs/${pack}/upstream/${tool}/${PUBLISH_DIRECTORY}/ (\`npm run fixtures:stage -- --pack ${pack} --tool ${tool}\`) and run the publish workflow (${PUBLISH_WORKFLOW}), which writes the lock`)] : []),
    ...(repo.has(path) ? malformedLock(repo, path) : []),
    ...(repo.has(samples) ? malformedLock(repo, samples) : []),
  ];
}

function packFixtureFinding(path) {
  const [, pack, tool] = TOOL_FILE.exec(path);
  return finding(
    path,
    `pack test data is never tracked in the pulled layout: move it under packs/${pack}/upstream/${tool}/${PUBLISH_DIRECTORY}/ (\`npm run fixtures:stage -- --pack ${pack} --tool ${tool}\`) to publish it`,
  );
}

/** One finding per tool-pack with a tracked publish/: the pull request's data is not in S3 yet. */
function publishFindings(repo) {
  const waiting = new Map();
  for (const path of repo.files) {
    const found = publishPath(path);
    if (found === undefined) continue;
    const directory = `${found.directory}/${PUBLISH_DIRECTORY}/`;
    waiting.set(directory, [...(waiting.get(directory) ?? []), path]);
  }
  return [...waiting].map(([directory, paths]) => {
    const shown = [...paths].sort().slice(0, SHOWN_PUBLISH_PATHS).map((path) => `\`${path}\``).join(", ");
    return finding(
      directory,
      `Pack data waiting to publish: run the publish workflow (${PUBLISH_WORKFLOW}) for this pull request: ${paths.length} file(s), e.g. ${shown}`,
    );
  });
}

export function check(repo) {
  const { outside } = fixtureLayout(repo);
  const findings = [
    ...outside.map((path) => finding(path, "fixture data belongs in fixtures/<family>/ or packs/<id>/upstream/<tool>/")),
    ...repo.files.filter((path) => isPackFixture(path)).map(packFixtureFinding),
    ...publishFindings(repo),
    ...[...toolDirectories(repo)].flatMap(([directory, names]) => lockFindings(repo, directory, names)),
  ];
  return { actual: String(findings.length), findings: sortFindings(findings) };
}
