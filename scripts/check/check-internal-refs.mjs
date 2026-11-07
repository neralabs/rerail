#!/usr/bin/env node
// No internal ticket ID (`NER-` and digits, any case) in a tracked file's path or text. The repository is public and
// the tracker those IDs point to is not: name the behavior, the pull request or the commit instead.
//
// Scope: every tracked path, and the text of every tracked file Git does not take for binary (`git grep -I`).
// Commit messages and pull request titles are not checked.
//
//   node scripts/check/check-internal-refs.mjs   (npm run check:internal-refs; part of npm run check)
import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** An internal ticket ID. Its source is also the `git grep -P` pattern, so paths and text match alike. */
export const TICKET = /\bner-\d+/iu;

const git = (root, args) => execFileSync("git", args, { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });

/** Every tracked path and text line under `root` that names a ticket, as `{ path, line, text }` (`line` 0 for a path). */
export function internalRefProblems(root) {
  const problems = git(root, ["ls-files", "-z"])
    .split("\0")
    .filter((path) => TICKET.test(path))
    .map((path) => ({ path, line: 0, text: path }));
  let matches = "";
  try {
    matches = git(root, ["grep", "-I", "-n", "-i", "-z", "-P", "-e", TICKET.source]);
  } catch (error) {
    if (error.status !== 1) throw error;
  }
  for (const record of matches.split("\n")) {
    if (record === "") continue;
    const [path, line, text] = record.split("\0");
    problems.push({ path, line: Number(line), text: text.trim() });
  }
  return problems;
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const problems = internalRefProblems(root);
  for (const { path, line, text } of problems) {
    console.error(line === 0 ? `${path}: the path names an internal ticket` : `${path}:${line}: ${text.slice(0, 160)}`);
  }
  console.log(
    problems.length === 0
      ? "check-internal-refs: no internal ticket IDs in tracked files"
      : `check-internal-refs: ${problems.length} internal ticket ID(s); name the behavior, pull request or commit instead`,
  );
  process.exit(problems.length === 0 ? 0 : 1);
}
