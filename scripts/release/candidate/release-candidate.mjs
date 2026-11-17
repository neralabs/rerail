#!/usr/bin/env node
// One action of release-pr.yml on the release (candidate.mjs; docs/release.md, "Freezing a release"):
//   update        a push to main: rebuilds the Release pull request on main's tip, or, frozen, leaves it alone
//   freeze        holds the release at $TARGET (a commit on main, or the latest release's tag; main's tip when empty),
//                 with $FIXES cherry-picked onto it (commits on main or merged pull request numbers)
//   cherry-pick   adds $TARGET, a commit on main or a merged pull request's number, to the frozen release
//   unfreeze      follows main again
// It builds in the checkout, which must be clean and hold the full history, and pushes to $RELEASE_REMOTE (origin when
// unset) in one push or not at all. The Release commit is written by the candidate's own release-version.mjs, with the
// candidate's own dependencies when its package-lock.json is not the checkout's. It writes the pull request's
// description to <body.md>, a comment for it to <comment.md> when the action has one, and `releasable`, `changed`,
// `frozen`, `candidate` and `comment` to $GITHUB_OUTPUT when it is set.
//
//   [TARGET=<commit|tag|number>] [FIXES="<commit|number> ..."] node scripts/release/candidate/release-candidate.mjs <action> <body.md> <comment.md>
import { execFileSync } from "node:child_process";
import { appendFileSync, writeFileSync } from "node:fs";
import { actionComment, gitIn, runReleaseAction } from "./candidate.mjs";
import { ROOT } from "../packs.mjs";

const [action, bodyPath, commentPath] = process.argv.slice(2);
if (action === undefined || bodyPath === undefined || commentPath === undefined) throw new Error("usage: release-candidate.mjs <update|freeze|cherry-pick|unfreeze> <body.md> <comment.md>");
const { TARGET: target = "", FIXES: fixes = "", RELEASE_REMOTE: remote = "origin", GITHUB_ACTOR: actor = "someone", GITHUB_REPOSITORY: repository = "" } = process.env;

const git = gitIn(ROOT);
/** The commit whose dependencies are installed: the checkout's when the action starts. */
const installed = git(["rev-parse", "HEAD"]);

/** Writes the Release commit's changes into the checkout, which is at the candidate, with the candidate's own script. */
function version(frozen) {
  // release-version.mjs runs `changeset version` and reads semver: the candidate's lockfile decides which.
  const sameDependencies = git(["diff", "--name-only", installed, "HEAD", "--", "package-lock.json"]) === "";
  if (!sameDependencies) execFileSync("npm", ["ci"], { cwd: ROOT, stdio: "inherit" });
  const frozenAt = frozen === null ? null : { ...frozen, baseSubject: git(["log", "-1", "--format=%s", frozen.base]) };
  const env = { ...process.env, RELEASE_FROZEN: frozenAt === null ? "" : JSON.stringify(frozenAt) };
  // Its own `releasable` would be read as this action's.
  delete env.GITHUB_OUTPUT;
  execFileSync(process.execPath, ["scripts/release/release-version.mjs", bodyPath], { cwd: ROOT, stdio: "inherit", env });
}

/** The commit on main that merged pull request `number`. */
function resolvePull(number) {
  const pull = JSON.parse(execFileSync("gh", ["pr", "view", String(number), "--json", "state,baseRefName,mergeCommit"], { cwd: ROOT, encoding: "utf8" }));
  if (pull.state !== "MERGED" || pull.baseRefName !== "main") throw new Error(`pull request ${number} is not merged to main yet. Merge it first, so main never lacks a fix the release has.`);
  return pull.mergeCommit.oid;
}

let result;
try {
  result = runReleaseAction({ action, target, fixes, git, remote, version, resolvePull });
} catch (error) {
  // The reason alone, as an annotation on the run: every refusal says what to run next.
  console.error(`::error title=Release ${action} refused::${String(error.message).replaceAll("\n", " ")}`);
  process.exit(1);
}

const comment = actionComment({ action, actor, repository, result });
if (comment !== null) writeFileSync(commentPath, `${comment}\n`);
if (result.notice !== undefined) console.log(`::notice::${result.notice}`);
if (result.conflicts !== undefined) console.log(`::warning title=The frozen release can't be merged::main changed ${result.conflicts.join(", ")}, which the frozen release rewrites: see the comment on the Release pull request.`);
console.log(JSON.stringify(result));
if (process.env.GITHUB_OUTPUT !== undefined) {
  appendFileSync(process.env.GITHUB_OUTPUT, `releasable=${result.releasable}\nchanged=${result.changed}\nfrozen=${result.frozen}\ncandidate=${result.candidate ?? ""}\ncomment=${comment !== null}\n`);
}
