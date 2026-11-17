// The Release pull requests that merged, read from GitHub with gh: release-tag.mjs tags what the last one released,
// and release-train-check.mjs holds a release tag to a commit one of them released. Like candidate.mjs, it imports
// nothing that needs `npm ci`.
import { execFileSync } from "node:child_process";
import { ROOT } from "../packs.mjs";
import { RELEASE_BRANCH, releasedCommit } from "./candidate.mjs";

const gh = (args) => execFileSync("gh", args, { cwd: ROOT, encoding: "utf8" }).trim();

/** The merged Release pull requests, last merged first, as `{ number, head }`: each one's number and head commit. */
export function mergedReleaseHeads(limit = 20) {
  const merged = JSON.parse(gh(["pr", "list", "--state", "merged", "--head", RELEASE_BRANCH, "--base", "main", "--limit", String(limit), "--json", "number,headRefOid,mergedAt"]));
  return merged.sort((a, b) => b.mergedAt.localeCompare(a.mergedAt)).map(({ number, headRefOid }) => ({ number, head: headRefOid }));
}

/** The commit that merging the Release pull request with head `head` released (releasedCommit), or null. */
export const releasedBy = (head) => releasedCommit(head, gh(["api", `repos/{owner}/{repo}/commits/${head}`, "--jq", ".commit.message"]));
