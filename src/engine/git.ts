// Git access lives in src/engine/git-access/. This file only hands its names on, for src/setup/preflight.ts and
// src/setup/harness-registration.ts, which import them from here and change in a pull request of their own, and for
// the tests written against this file.
export {
  findGit, gitBash, gitBehindLauncher, gitCandidates, gitExecutable, gitInstalled, GitUnavailableError, searchGitFrom,
  type FoundGit, type GitPathProbe, type GitProbe, type GitSearch,
} from "./git-access/find-git.js";
export { gitHome, runGit, type BlobLimits, type CrlfRule, type GitResult, type RunGitOptions } from "./git-access/git-process.js";
export { GitReads } from "./git-access/checkout-git.js";
