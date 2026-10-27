#!/usr/bin/env node
// The publish workflow's (.github/workflows/pack-data-publish.yml) finish and failed jobs: the lock files the objects
// prep named make, and the sticky comment's published, action-needed and failed states (its waiting state is
// .github/workflows/pack-data-publish-request.yml's). Both read only the objects prep named (its job output), each
// checked by publishedObjects, never the pull request.
//
//   node scripts/s3/pack-data-publish.mjs locks <dir>     write each lock under <dir> at its repository path, and
//                                                         print the paths, one per line
//   node scripts/s3/pack-data-publish.mjs comment <state> print the comment for <state>: published, action-needed or
//                                                         failed
//
// Inputs come from the environment: OBJECTS (prep's objects, JSON), and for `comment` RUN_URL, plus COMMIT (published),
// REASON (action-needed) or FAILED (failed: upload's `failed` output, JSON `[{ key, reason }]`, empty when upload did
// not run).

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

import { BUCKET, PUBLISH_DIRECTORY, publishedObjects } from "./pack-fixtures.mjs";
import { lockText } from "./test-data-archive.mjs";

/** The sticky comment's marker, shared with pack-data-publish-request.yml, which finds the comment by it. */
export const COMMENT_MARKER = "<!-- rerail-pack-data-publish";

/** Each object's lock: its repository path and text, sorted by path. */
export function publishedLocks(objects) {
  return publishedObjects(objects)
    .map(({ lock }) => ({ path: lock.label, text: lockText(lock) }))
    .sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
}

const objectTable = (objects) => [
  "| Object | sha256 | Files | Bytes |",
  "|---|---|---|---|",
  ...publishedObjects(objects).map(({ key, lock }) => `| \`s3://${BUCKET}/${key}\` | \`${lock.sha256}\` | ${lock.files} | ${lock.bytes} |`),
];

/** The publish/ directories the objects come from, as repository paths. */
const publishDirectories = (objects) => [...new Set(publishedObjects(objects).map(({ toolPack }) => `${toolPack.directory}/${PUBLISH_DIRECTORY}/`))].sort();

/** The sticky comment for `state` ("published", "action-needed" or "failed"). */
export function publishComment(state, { runUrl, objects, commit, reason, failed = [] }) {
  const run = `[publish run](${runUrl})`;
  if (state === "published") {
    return [
      `${COMMENT_MARKER} state=published -->`,
      "### Pack data published",
      "",
      `The ${run} uploaded and confirmed these objects in S3, then committed the new locks and removed ${publishDirectories(objects).map((path) => `\`${path}\``).join(", ")} in ${commit}.`,
      "",
      ...objectTable(objects),
      "",
    ].join("\n");
  }
  if (state === "action-needed") {
    const directories = publishDirectories(objects).map((path) => `\`${path}\``).join(", ");
    return [
      `${COMMENT_MARKER} state=action-needed -->`,
      "### Pack data published: action needed",
      "",
      `The ${run} uploaded and confirmed these objects in S3, but did not commit the locks: ${reason}.`,
      "",
      ...objectTable(objects),
      "",
      `Commit these locks, delete ${directories}, and push; or pull the branch and run the publish again.`,
      "",
      ...publishedLocks(objects).flatMap(({ path, text }) => [`\`${path}\`:`, "", "```json", text.trimEnd(), "```", ""]),
    ].join("\n");
  }
  if (state === "failed") {
    const lines = failed.length === 0
      ? ["- upload did not run: approval rejected or job cancelled"]
      : failed.map(({ key, reason: why }) => `- \`s3://${BUCKET}/${key}\`: ${String(why).replace(/\s+/gu, " ")}`);
    return [
      `${COMMENT_MARKER} state=failed -->`,
      "### Pack data publish failed",
      "",
      `The ${run} did not put and confirm every object:`,
      "",
      ...lines,
      "",
      "Nothing was committed and no lock changed. Re-running is safe.",
      "",
    ].join("\n");
  }
  throw new Error(`no comment state ${JSON.stringify(state)}: published, action-needed or failed`);
}

const [command, argument] = process.argv.slice(2);
if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const objects = JSON.parse(process.env.OBJECTS ?? "null");
    if (command === "locks" && argument !== undefined) {
      for (const { path, text } of publishedLocks(objects)) {
        mkdirSync(dirname(join(argument, path)), { recursive: true });
        writeFileSync(join(argument, path), text);
        console.log(path);
      }
    } else if (command === "comment" && argument !== undefined) {
      const failed = process.env.FAILED ? JSON.parse(process.env.FAILED) : [];
      process.stdout.write(publishComment(argument, { runUrl: process.env.RUN_URL, objects, commit: process.env.COMMIT, reason: process.env.REASON, failed }));
    } else {
      throw new Error("usage: node scripts/s3/pack-data-publish.mjs locks <dir> | comment <published|action-needed|failed>");
    }
  } catch (error) {
    console.error(`pack-data-publish: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
