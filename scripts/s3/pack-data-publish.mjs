#!/usr/bin/env node
// The publish workflow's (.github/workflows/pack-data-publish.yml) finish and failed jobs, and the request workflow's
// (.github/workflows/pack-data-publish-request.yml) comment job: the lock files the objects prep named make, and the
// sticky comment's waiting, published, action-needed and failed states. The locks and the three publish states read
// only the objects prep named (its job output), each checked by publishedObjects, never the pull request; the waiting
// state reads the request workflow's counts per tool-pack (requestedToolPacks), never a file name of the pull request.
//
//   node scripts/s3/pack-data-publish.mjs locks <dir>     write each lock under <dir> at its repository path, and
//                                                         print the paths, one per line
//   node scripts/s3/pack-data-publish.mjs comment <state> print the comment for <state>: waiting, published,
//                                                         action-needed or failed
//
// Inputs come from the environment: OBJECTS (prep's objects, JSON) for `locks` and every comment state but waiting, with
// RUN_URL, plus COMMIT (published), REASON (action-needed) or FAILED (failed: upload's `failed` output, JSON
// `[{ key, reason }]`, empty when confirm wrote no report) with CREDENTIALS_OUTCOME and UPLOAD_OUTCOME (failed: those
// upload steps' outcomes, empty when the upload job never ran). For `waiting`: REPOSITORY (owner/name), PR, TOOL_PACKS
// (the request workflow's tool-packs, JSON, requestedToolPacks), ENVIRONMENT (the test-data-upload environment's id),
// FILES_CAP (how many changed files the pull request files endpoint lists), and FORK and BUMP ("true" or "false").

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

import { BUCKET, PUBLISH_DIRECTORY, publishedObjects, TOOL_ID } from "./pack-fixtures.mjs";
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

/** Why upload put nothing it could name, from its credentials and upload steps' outcomes (empty: the job never ran). */
function uploadFailure({ credentials = "", upload = "" }) {
  if (credentials === "failure" || (upload !== "" && upload !== "skipped")) return "upload step failed: see the run";
  if (credentials === "" && upload === "") return "upload did not run: approval rejected or job cancelled";
  return "upload job failed before its upload step: see the run";
}

const isCount = (value) => Number.isSafeInteger(value) && value >= 0;

/**
 * The request workflow's tool-packs with something to publish, each checked: `{ pack, tool, files, testData, samples,
 * provenance, lost, locked }`. `files` are the pull request's files in the tool-pack's publish/, of them `testData` test
 * data files and `samples` samples files (publish/samples/REUSE.toml, which no archive takes, is in neither);
 * `provenance` says whether publish/provenance.json is among them; `lost` counts the authored rows the pull request
 * removes from the tool's UPSTREAM.md; `locked` is `{ testData, samples }`, the `files` count of the tool-pack's
 * test-data.lock and samples.lock at the pull request's head, null without one. Counts and flags alone: the workflow
 * renders no file name of the pull request. Throws on anything else.
 */
export function requestedToolPacks(toolPacks) {
  if (!Array.isArray(toolPacks)) throw new Error("TOOL_PACKS is not a JSON array of the request workflow's tool-packs");
  return toolPacks.map((toolPack, index) => {
    const where = `tool-pack ${index}`;
    if (toolPack === null || typeof toolPack !== "object") throw new Error(`${where} is not an object`);
    const { pack, tool, files, testData, samples, provenance, lost, locked } = toolPack;
    if (typeof pack !== "string" || typeof tool !== "string" || !TOOL_ID.test(pack) || !TOOL_ID.test(tool)) throw new Error(`${where}: pack ${JSON.stringify(pack)} and tool ${JSON.stringify(tool)} must be ids`);
    if (![files, testData, samples, lost].every(isCount)) throw new Error(`${where} (${pack}/${tool}): files, testData, samples and lost must be counts`);
    if (typeof provenance !== "boolean") throw new Error(`${where} (${pack}/${tool}): provenance must be true or false`);
    if (locked === null || typeof locked !== "object" || ![locked.testData, locked.samples].every((count) => count === null || isCount(count))) {
      throw new Error(`${where} (${pack}/${tool}): locked must give each lock's files count or null`);
    }
    return { pack, tool, files, testData, samples, provenance, lost, locked: { testData: locked.testData, samples: locked.samples } };
  });
}

/** publish/ holding, with provenance.json, at least one locked test data file in this many is a whole set. */
const WHOLE_SET_STAGES_ONE_IN = 2;

/**
 * The choice of `replace` (CONTRIBUTING.md, "Merging"): the rule, and what the wrong value does. The waiting comment's
 * step 3 and the publish workflow's input description say the same.
 */
export const REPLACE_RULE =
  "**replace**: on when `publish/` holds the tool-pack's whole data set (a regeneration or re-derive staged with `fixtures:stage --all`, or a publish that deletes files from the archive); off when it holds only what is new or changed and the rest of the locked archive must stay. On over additions drops every locked file `publish/` lacks; off over a regeneration keeps the files it deleted. Prep refuses the wrong mode for test data, whose `provenance.json` must list the archive's files, but not for samples.";

/**
 * What each kind of data a tool-pack stages says about `replace`: `{ mode, reason }` per kind, `mode` "on" (a whole
 * set), "off" (additions, changes or removed rows) or undefined when the counts cannot tell (a samples set carries no
 * whole-set marker, and a tool-pack without a lock gets the same archive either way), and `reason` the line the comment
 * shows: the tool-pack's id, the counts and the signal that decided.
 */
function toolPackAdvice({ pack, tool, testData, samples, provenance, lost, locked }) {
  const name = `\`${pack}/${tool}\``;
  const advice = [];
  if (testData > 0) {
    if (locked.testData === null) advice.push({ mode: undefined, reason: `${name}: no \`test-data.lock\` yet, so both modes make the same test data archive.` });
    else if (!provenance) advice.push({ mode: "off", reason: `${name}: off. \`${PUBLISH_DIRECTORY}/\` holds ${testData} test data files and no \`provenance.json\`, which a whole set carries and replace needs.` });
    else if (testData * WHOLE_SET_STAGES_ONE_IN >= locked.testData) {
      advice.push({ mode: "on", reason: `${name}: on. \`${PUBLISH_DIRECTORY}/\` holds ${testData} test data files with \`provenance.json\` against ${locked.testData} in \`test-data.lock\`: a whole set.` });
    } else advice.push({ mode: "off", reason: `${name}: off. \`${PUBLISH_DIRECTORY}/\` holds ${testData} test data files against ${locked.testData} in \`test-data.lock\`: additions or changes, not a whole set.` });
  } else if (lost > 0) {
    advice.push({ mode: "off", reason: `${name}: off. ${lost} authored row(s) removed with nothing in \`${PUBLISH_DIRECTORY}/\`: an overlay drops their files, and a replace publishes nothing for it.` });
  }
  if (samples > 0) {
    if (locked.samples === null) advice.push({ mode: undefined, reason: `${name}: no \`samples.lock\` yet, so both modes make the same samples archive.` });
    else {
      advice.push({
        mode: undefined,
        reason: `${name}: ${samples} samples files beside ${locked.samples} in \`samples.lock\`. Nothing marks a whole sample set, so the counts cannot tell: off keeps the locked samples \`${PUBLISH_DIRECTORY}/\` lacks, on drops them; prep's summary counts them and warns when the mode looks wrong.`,
      });
    }
  }
  return advice;
}

/**
 * Which `replace` value the comment recommends for a pull request's tool-packs (requestedToolPacks): `{ mode, text,
 * reasons }`, `mode` "on", "off" or "split" (tool-packs that want different values; one run publishes every publish/ the
 * same way), `text` the verdict as the comment's step 3 states it, and `reasons` one line per kind of data each
 * tool-pack stages (toolPackAdvice). An upstream bump's pull request (`bump`) stages whole derived sets, so it is on by
 * construction. With no kind deciding, off: it keeps every locked file, and only a set rebuilt without some of its files
 * needs on.
 */
export function replaceAdvice(toolPacks, { bump = false } = {}) {
  if (bump) return { mode: "on", text: "**on**: an upstream bump's `publish/` holds each tool-pack's whole derived set.", reasons: [] };
  const advice = toolPacks.flatMap(toolPackAdvice);
  const modes = new Set(advice.map(({ mode }) => mode).filter((mode) => mode !== undefined));
  const reasons = advice.map(({ reason }) => reason);
  if (modes.size === 2) return { mode: "split", text: "**the tool-packs disagree**: one run publishes every `publish/` the same way, so publish them from separate pull requests.", reasons };
  if (modes.has("on")) return { mode: "on", text: "**on**.", reasons };
  if (modes.has("off")) return { mode: "off", text: "**off**.", reasons };
  return { mode: "off", text: "**off**, unless a sample set was rebuilt without some of its samples: nothing here marks a whole set.", reasons };
}

const REPOSITORY = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u;
const DIGITS = /^[0-9]+$/u;

/** The waiting comment (the request workflow's), for the pull request `pr` of `repository` with `toolPacks` (requestedToolPacks; [] when the pull request changes more than `filesCap` files, so they are unknown). */
function waitingComment({ repository, pr, toolPacks, fork, bump, environment, filesCap }) {
  if (typeof repository !== "string" || !REPOSITORY.test(repository)) throw new Error(`REPOSITORY ${JSON.stringify(repository)} is not owner/name`);
  if (typeof pr !== "string" || !DIGITS.test(pr)) throw new Error(`PR ${JSON.stringify(pr)} is not a pull request number`);
  const rows = toolPacks.length === 0
    ? [`| (this pull request changes more than ${filesCap} files, so its pack data cannot be listed) | | |`]
    : toolPacks.map(({ pack, tool, files, lost }) => `| \`${pack}/${tool}\` | ${files} | ${lost} |`);
  const table = ["| Tool-pack | Files in `publish/` | Authored rows removed from `UPSTREAM.md` |", "|---|---|---|", ...rows];
  if (fork) {
    // A fork's pull request is a proposal (CONTRIBUTING.md, "Proposing a change from your fork"): its data is published
    // from the pull request that ports it to a branch here, never from the fork. Same marker and state, so the comment
    // goes once the fork stops staging data.
    return [
      `${COMMENT_MARKER} state=waiting -->`,
      "### Pack data in a proposal from a fork",
      "",
      `This pull request is from a fork, so it is a proposal: its pack data is not published from it. If a maintainer takes the change, they port it to a branch here, and the data is published from that pull request ([how it works](https://github.com/${repository}/blob/main/CONTRIBUTING.md#proposing-a-change-from-your-fork)).`,
      "",
      ...table,
      "",
    ].join("\n");
  }
  if (typeof environment !== "string" || !DIGITS.test(environment)) throw new Error(`ENVIRONMENT ${JSON.stringify(environment)} is not the test-data-upload environment's id`);
  const advice = replaceAdvice(toolPacks, { bump });
  const flag = advice.mode === "on" ? " -f replace=true" : "";
  return [
    `${COMMENT_MARKER} state=waiting -->`,
    "### Pack data waiting to publish",
    "",
    "This pull request has pack data to publish. It can't merge until the data is in S3 and the locks name it.",
    "",
    ...table,
    "",
    "**By hand**",
    "",
    `1. Open the [publish workflow](https://github.com/${repository}/actions/workflows/pack-data-publish.yml) and click **Run workflow** (branch \`main\`).`,
    `2. Type **\`${pr}\`** as the pull request number. GitHub can't pre-fill workflow inputs from a link, so type it yourself.`,
    `3. ${REPLACE_RULE}${toolPacks.length === 0 ? "" : ` Here: ${advice.text}`}`,
    ...advice.reasons.map((reason) => `   - ${reason}`),
    "4. Wait for a code owner (@Moti-Velz or @jeffreysnell) to approve the `test-data-upload` deployment. They read the run's **prep** summary first.",
    "5. When the run is green, pull this branch: the workflow has committed the new locks and removed `publish/`. If someone pushed to the branch meanwhile, this comment shows the locks to commit instead.",
    "",
    "**With the gh CLI (for agents)**",
    "",
    "```sh",
    "# 1. Start the publish (-f replace=true for replace on, as step 3 says).",
    `gh workflow run pack-data-publish.yml -R ${repository} -f pr=${pr}${flag}`,
    "",
    `# 2. Find the run. Its title is "Publish pack data for #${pr}"; wait a few seconds after step 1.`,
    `gh run list -R ${repository} --workflow pack-data-publish.yml --event workflow_dispatch --limit 5 \\`,
    "  --json databaseId,status,url,displayTitle,createdAt",
    `RUN=<databaseId of the newest run titled "Publish pack data for #${pr}">`,
    "",
    "# 3. Read prep's summary (its warnings, archives, sha256, files, bytes, what changed) from its log.",
    `gh run view "$RUN" -R ${repository} --log --job "$(gh run view "$RUN" -R ${repository} --json jobs --jq '.jobs[] | select(.name == "prep") | .databaseId')"`,
    "",
    "# 4. Approval. A code owner approves in the run's page, or, if your gh user is a required reviewer:",
    `gh api repos/${repository}/actions/runs/"$RUN"/pending_deployments \\`,
    "  --jq '.[] | {environment: .environment.name, id: .environment.id, can_approve: .current_user_can_approve}'",
    `echo '{"environment_ids":[${environment}],"state":"approved","comment":"Reviewed the prep summary"}' |`,
    `  gh api -X POST repos/${repository}/actions/runs/"$RUN"/pending_deployments --input -`,
    "",
    "# 5. Wait for the run to finish; it must succeed.",
    `gh run watch "$RUN" -R ${repository} --exit-status`,
    "",
    "# 6. Pull the branch and confirm: publish/ is gone and the locks changed.",
    "git pull --ff-only",
    "git ls-files 'packs/*/upstream/*/publish/**'                    # prints nothing",
    "git log -1 --stat -- 'packs/*/upstream/*/test-data.lock' 'packs/*/upstream/*/samples.lock'",
    "```",
    "",
  ].join("\n");
}

/** The sticky comment for `state` ("waiting", "published", "action-needed" or "failed"). */
export function publishComment(state, { runUrl, objects, commit, reason, failed = [], outcomes = {}, ...request }) {
  if (state === "waiting") return waitingComment(request);
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
      ? [`- ${uploadFailure(outcomes)}`]
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
  throw new Error(`no comment state ${JSON.stringify(state)}: waiting, published, action-needed or failed`);
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
    } else if (command === "comment" && argument === "waiting") {
      const toolPacks = requestedToolPacks(JSON.parse(process.env.TOOL_PACKS ?? "[]"));
      process.stdout.write(
        publishComment("waiting", {
          repository: process.env.REPOSITORY,
          pr: process.env.PR,
          toolPacks,
          fork: process.env.FORK === "true",
          bump: process.env.BUMP === "true",
          environment: process.env.ENVIRONMENT,
          filesCap: process.env.FILES_CAP,
        }),
      );
    } else if (command === "comment" && argument !== undefined) {
      const failed = process.env.FAILED ? JSON.parse(process.env.FAILED) : [];
      const outcomes = { credentials: process.env.CREDENTIALS_OUTCOME, upload: process.env.UPLOAD_OUTCOME };
      process.stdout.write(publishComment(argument, { runUrl: process.env.RUN_URL, objects, commit: process.env.COMMIT, reason: process.env.REASON, failed, outcomes }));
    } else {
      throw new Error("usage: node scripts/s3/pack-data-publish.mjs locks <dir> | comment <waiting|published|action-needed|failed>");
    }
  } catch (error) {
    console.error(`pack-data-publish: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
