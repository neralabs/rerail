#!/usr/bin/env node
// No internal reference in a tracked file's path or text. The repository is public and what these point to is not:
// - an internal ticket ID (`NER-` and digits, any case): name the behavior, the pull request or the commit instead;
// - a `nera-` identifier (any case) that is not on PUBLIC_NAMES: Nera's private repositories and projects are named
//   that way. Describe the project neutrally instead ("Nera's design-system source", "the internal validation
//   gates"), or, for a name that is public, add it to PUBLIC_NAMES. The list holds only public identifiers, so the
//   gate itself never names a private one.
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

/** A `nera-` identifier. Its source is also the `git grep -P` pattern; a match is a problem unless it is public. */
export const NERA_NAME = /\bnera-[a-z0-9][a-z0-9-]*/iu;

/**
 * The public `nera-` identifiers, lowercase and exact: the pack test data bucket, a Nera design token and the two
 * rules about Nera tokens, and test ids.
 */
export const PUBLIC_NAMES = Object.freeze([
  "nera-test-fixtures", "nera-card-shadow", "nera-token-set", "nera-token-redefined", "nera-only", "nera-held",
]);

/** Whether `name`, a NERA_NAME match, is public. */
export const isPublicName = (name) => PUBLIC_NAMES.includes(name.toLowerCase());

const git = (root, args) => execFileSync("git", args, { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });

/** Every tracked path under `root`. */
const trackedPaths = (root) => git(root, ["ls-files", "-z"]).split("\0").filter((path) => path !== "");

/** Every tracked text line under `root` that `pattern` (a `git grep -P` source) matches, as `{ path, line, text }`. */
function grepLines(root, pattern) {
  let matches = "";
  try {
    matches = git(root, ["grep", "-I", "-n", "-i", "-z", "-P", "-e", pattern]);
  } catch (error) {
    if (error.status !== 1) throw error;
  }
  const lines = [];
  for (const record of matches.split("\n")) {
    if (record === "") continue;
    const [path, line, text] = record.split("\0");
    lines.push({ path, line: Number(line), text: text.trim() });
  }
  return lines;
}

/** The `nera-` identifiers in `text` that are not public, in order, each once. */
export function privateNames(text) {
  const names = text.matchAll(new RegExp(NERA_NAME.source, `${NERA_NAME.flags}g`));
  return [...new Set([...names].map(([name]) => name).filter((name) => !isPublicName(name)))];
}

/** Every tracked path and text line under `root` that names a ticket, as `{ path, line, text }` (`line` 0 for a path). */
export function internalRefProblems(root) {
  return [
    ...trackedPaths(root).filter((path) => TICKET.test(path)).map((path) => ({ path, line: 0, text: path })),
    ...grepLines(root, TICKET.source),
  ];
}

/**
 * Every tracked path and text line under `root` that holds a `nera-` identifier not on PUBLIC_NAMES, as
 * `{ path, line, text, names }` (`line` 0 for a path; `names` the identifiers it flags).
 */
export function privateNameProblems(root) {
  const flagged = ({ path, line, text }) => {
    const names = privateNames(text);
    return names.length === 0 ? [] : [{ path, line, text, names }];
  };
  return [
    ...trackedPaths(root).flatMap((path) => flagged({ path, line: 0, text: path })),
    ...grepLines(root, NERA_NAME.source).flatMap(flagged),
  ];
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const tickets = internalRefProblems(root);
  for (const { path, line, text } of tickets) {
    console.error(line === 0 ? `${path}: the path names an internal ticket` : `${path}:${line}: ${text.slice(0, 160)}`);
  }
  const names = privateNameProblems(root);
  for (const { path, line, text, names: flagged } of names) {
    console.error(line === 0 ? `${path}: the path names ${flagged.join(", ")}` : `${path}:${line}: ${flagged.join(", ")}: ${text.slice(0, 160)}`);
  }
  console.log(
    tickets.length === 0
      ? "check-internal-refs: no internal ticket IDs in tracked files"
      : `check-internal-refs: ${tickets.length} internal ticket ID(s); name the behavior, pull request or commit instead`,
  );
  console.log(
    names.length === 0
      ? "check-internal-refs: no nera- names outside the public list in tracked files"
      : `check-internal-refs: ${names.length} nera- name(s) outside the public list; describe the project neutrally, or add a public name to PUBLIC_NAMES`,
  );
  process.exit(tickets.length === 0 && names.length === 0 ? 0 : 1);
}
