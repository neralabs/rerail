/**
 * Commit staging: a verdict writes its decision (and, with the experimental decision ledger, its ledger entry) into the
 * working tree, and they ship only when the person's branch commits them. At the agent's `git commit` tool call, Rerail names the decision files that commit
 * would leave out, as context, once per commit attempt; it never stages anything and never blocks the commit.
 */
import { isCommandTool } from "../code-rule/code-rule-bridge.js";
import { runGit } from "../git.js";
import { CANDIDATE_DIRECTORY } from "./candidate.js";
import { DECISIONS_DIRECTORY } from "./decision-entry.js";

/** The command a shell tool call runs, or null for any other tool. */
export function shellCommandOf(toolName: string, input: unknown): string | null {
  if (!isCommandTool(toolName) || typeof input !== "object" || input === null) return null;
  const fields = input as Record<string, unknown>;
  const command = fields.command ?? fields.cmd;
  if (typeof command === "string") return command;
  return Array.isArray(command) && command.every(part => typeof part === "string") ? command.join(" ") : null;
}

/** Git's options before its subcommand that take a value as the next word. */
const GIT_VALUE_OPTIONS: Readonly<Record<string, true>> = { "-C": true, "-c": true, "--git-dir": true, "--work-tree": true, "--namespace": true, "--config-env": true };

/**
 * Whether `command` runs `git commit` in any of its simple commands, and whether with `-a`/`--all` (which commits
 * tracked changes, so only untracked files are left out). Null when it does not commit.
 */
export function gitCommitIn(command: string): { readonly all: boolean } | null {
  const unquoted = command.replace(/'[^']*'|"(?:[^"\\]|\\.)*"/gu, "''");
  for (const part of unquoted.split(/[\n;&|()]+/u)) {
    const words = part.trim().split(/\s+/u).filter(word => word !== "");
    let index = 0;
    while (index < words.length && /^[A-Za-z_][A-Za-z0-9_]*=/u.test(words[index]!)) index += 1;
    if (words[index] !== "git") continue;
    index += 1;
    while (index < words.length && words[index]!.startsWith("-")) index += GIT_VALUE_OPTIONS[words[index]!] === true ? 2 : 1;
    if (words[index] !== "commit") continue;
    const flags = words.slice(index + 1);
    return { all: flags.some(flag => flag === "--all" || /^-[A-Za-z]*a[A-Za-z]*$/u.test(flag)) };
  }
  return null;
}

/**
 * The decision files a commit would leave out: under `.rerail/candidates/`, and `.rerail/decisions/` with the decision
 * ledger (`ledger`); untracked, or (unless the commit takes every tracked change, `all`) modified or deleted and not
 * staged. Empty when Git cannot say.
 */
export function unstagedDecisionFiles(root: string, all: boolean, ledger: boolean): string[] {
  const folders = ledger ? [`${DECISIONS_DIRECTORY}/`, `${CANDIDATE_DIRECTORY}/`] : [`${CANDIDATE_DIRECTORY}/`];
  const result = runGit(root, ["status", "--porcelain=v1", "-z", "--untracked-files=all", "--", ...folders]);
  if (result.status !== 0) return [];
  const records = result.stdout.split("\0");
  const files: string[] = [];
  for (let index = 0; index < records.length; index += 1) {
    const record = records[index]!;
    if (record.length < 4) continue;
    const [staged, working] = [record[0], record[1]];
    // A rename or copy is followed by its source path.
    if (staged === "R" || staged === "C") index += 1;
    if ((staged === "?" && working === "?") || (!all && working !== " ")) files.push(record.slice(3));
  }
  return files.sort();
}

/** The context the agent gets at its commit. */
export function commitStagingNote(files: readonly string[]): string {
  return `Rerail: include these decision files in this commit: ${files.join(", ")}`;
}
