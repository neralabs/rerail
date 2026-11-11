/**
 * A GitHubClient over in-memory commits (commit to path to text), for the nightly report's tests: trees list blob shas
 * as git computes them, files answer their text, and every read is recorded. Nothing reaches the network.
 */
import { createHash } from "node:crypto";

import type { GitHubClient } from "../../../../scripts/upstream/github.js";

/** Git's blob sha of `text`. */
function blobSha(text: string): string {
  const bytes = Buffer.from(text);
  return createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
}

/** A GitHubClient answering from `commits`, and the reads it was asked for (`tree <repository>@<commit>:<dir>` or `file <repository>@<commit>:<path>`). */
export function fakeGitHub(commits: Record<string, Record<string, string>>): { github: GitHubClient; reads: string[] } {
  const reads: string[] = [];
  const files = (commit: string): Record<string, string> => {
    const tree = commits[commit];
    if (tree === undefined) throw new Error(`no commit ${commit}`);
    return tree;
  };
  const github: GitHubClient = {
    tree(repository, commit, directory, recursive) {
      reads.push(`tree ${repository}@${commit}:${directory}${recursive ? " recursive" : ""}`);
      const prefix = directory === "" ? "" : `${directory}/`;
      const blobs = new Map<string, string>();
      for (const [path, text] of Object.entries(files(commit))) {
        if (!path.startsWith(prefix)) continue;
        if (recursive || !path.slice(prefix.length).includes("/")) blobs.set(path, blobSha(text));
      }
      return Promise.resolve(blobs);
    },
    file(repository, commit, path) {
      reads.push(`file ${repository}@${commit}:${path}`);
      return Promise.resolve(files(commit)[path] ?? null);
    },
  };
  return { github, reads };
}
