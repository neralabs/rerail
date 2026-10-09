import { existsSync, renameSync, rmSync } from "node:fs";
import { resolve } from "node:path";
import Database from "better-sqlite3";
import { parseArguments } from "./daemon.js";
import { defaultRerailDatabasePath, resetRepositoryHistory } from "./database.js";
import { defaultRulesDumpPath, exportScopedRules, importScopedRules } from "./rule-bootstrap.js";
import { openScopedRuleCatalog, resolveRepositoryContext } from "./scoped-rule-catalog.js";

/** Where `rerail update` leaves a consistent copy of the Rerail database before it switches versions. */
export function preUpdateSnapshotPath(databasePath: string): string {
  return `${databasePath}.pre-update`;
}

/**
 * Before an update swaps the active version, copy the whole Rerail database (every project's
 * rules and every repository's history) to `rerail.sqlite.pre-update`, replacing the previous
 * copy. A copy that fails is reported, never fatal; the update must not be blocked by a
 * database the new version may be the fix for.
 */
export function snapshotDatabaseBeforeUpdate(databasePath = defaultRerailDatabasePath()): void {
  if (!existsSync(databasePath)) return;
  const target = preUpdateSnapshotPath(databasePath);
  const staging = `${target}.${process.pid}.tmp`;
  try {
    rmSync(staging, { force: true });
    const source = new Database(databasePath, { readonly: true, fileMustExist: true });
    try {
      source.pragma("busy_timeout = 5000");
      source.prepare("VACUUM INTO ?").run(staging);
    } finally {
      source.close();
    }
    renameSync(staging, target);
    process.stdout.write(`Copied ${databasePath} to ${target}\n`);
  } catch (error) {
    rmSync(staging, { force: true });
    process.stderr.write(`rerail: could not copy ${databasePath} before the update: ${(error as Error).message}\n`);
  }
}

export function runRulesCommand(args: readonly string[], usage: () => string): number {
  const action = args[0];
  const database = defaultRerailDatabasePath();
  if (action === "export") {
    const { options } = parseArguments(
      args.slice(1),
      { flags: [], options: ["--output"] },
      argument => `Unknown rules export option: ${argument}\n\n${usage()}`,
    );
    const rawOutput = options.get("--output");
    const output = rawOutput === undefined ? defaultRulesDumpPath(database) : resolve(rawOutput);
    const store = openScopedRuleCatalog(process.cwd(), database);
    let count: number;
    try {
      count = exportScopedRules(store, output);
    } finally {
      store.close();
    }
    process.stdout.write(`Exported ${count} rules from ${database} to ${output}\n`);
    return 0;
  }
  if (action === "import") {
    const file = args[1];
    if (file === undefined || file.startsWith("--")) throw new Error(`rules import needs a dump file\n\n${usage()}`);
    parseArguments(args.slice(2), { flags: [], options: [] }, argument => `Unknown rules import option: ${argument}\n\n${usage()}`);
    const store = openScopedRuleCatalog(process.cwd(), database);
    try {
      const count = importScopedRules(store, resolve(file));
      process.stdout.write(`Imported ${count} rules from ${resolve(file)} into ${database}\n`);
    } finally {
      store.close();
    }
    return 0;
  }
  throw new Error(`Unknown rules action: ${action ?? "(none)"}\n\n${usage()}`);
}

function plural(count: number, noun: string): string {
  return count === 1 ? noun : `${noun}s`;
}

/**
 * Delete this repository's execution history, across all its worktrees, from the Rerail database. Rules, other
 * repositories, and settings stay; live sessions never observe a rule reset.
 */
export function runResetCommand(args: readonly string[], usage: () => string): number {
  parseArguments(args, { flags: [], options: [] }, argument => `Unknown reset option: ${argument}\n\n${usage()}`);
  const database = defaultRerailDatabasePath();
  const repository = resolveRepositoryContext(process.cwd());
  if (!existsSync(database)) {
    process.stdout.write(`No Rerail database at ${database}; nothing to reset.\n`);
    return 0;
  }
  const { sessions, openSessions } = resetRepositoryHistory(database, repository.repositoryId);
  const kept = openSessions > 0 ? `, ${openSessions} running ${plural(openSessions, "session")} kept (history dropped)` : "";
  process.stdout.write(
    `Removed the history of this repository (${repository.repositoryId}, across all its worktrees) from ${database}: `
    + `${sessions} ended ${plural(sessions, "session")} deleted`
    + kept
    + ". Rules and other repositories are unchanged.\n",
  );
  return 0;
}
