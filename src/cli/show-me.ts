/**
 * `rerail show-me`: the launcher half of `/rerail-show-me`. It adds a demo rule to this checkout's repository, scoped
 * to one demo file, so the coding agent can break it on purpose and be steered; `--cleanup` removes the demo file and
 * the rule again. The coding agent writes the file itself, through its own edit tool, so the steer comes the way a
 * real one does.
 */
import { existsSync, lstatSync, readFileSync, unlinkSync } from "node:fs";
import { join, resolve } from "node:path";
import { and, eq } from "drizzle-orm";
import { parseArguments } from "../daemon/daemon.js";
import { loadBundledCatalog } from "../engine/catalog/bundled-rules.js";
import { repositoryIdentityLocation, repositoryName } from "../engine/catalog/repository-identity.js";
import type { RepositoryKey } from "../engine/catalog/repository-model.js";
import { lookupCheckout, recordCheckout } from "../engine/catalog/repository-registry.js";
import { addAttachment, removeAttachment } from "../engine/catalog/rule-configuration.js";
import { removeLibraryRule, upsertLibraryRule } from "../engine/catalog/rule-library.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { libraryRules, ruleAttachments } from "../engine/storage/schema.js";
import { workspaceStartState } from "../runtime/repository-start.js";

export const SHOW_ME_RULE_ID = "local:rerail-show-me";
/** The one file the demo rule checks, at the top of the checkout. */
export const SHOW_ME_FILE = "rerail-show-me.js";
/** Marks the demo rule as the demo's own, so cleanup never removes a rule someone else gave this id. */
const SHOW_ME_DESCRIPTION = "The /rerail-show-me demo rule. The demo removes it when it ends.";
/** The demo file's first line; cleanup deletes only a file that starts with it. */
const SHOW_ME_MARKER = "// rerail-show-me: a demo file. /rerail-show-me deletes it when the demo ends.";
export const SHOW_ME_MESSAGE = "Nera's demo rule: greet with \"Hello from Nera\", not \"Hello, world\".";
export const SHOW_ME_VIOLATING = `${SHOW_ME_MARKER}\nexport const greeting = () => "Hello, world";\n`;
export const SHOW_ME_FIXED = `${SHOW_ME_MARKER}\nexport const greeting = () => "Hello from Nera";\n`;

export const SHOW_ME_RULE = {
  format: "rerail.policy/v1",
  id: SHOW_ME_RULE_ID,
  revision: 1,
  mode: "steer",
  description: SHOW_ME_DESCRIPTION,
  scope: { include: [SHOW_ME_FILE], exclude: [] },
  clauses: [{
    id: "hello-from-nera",
    forbid: { regex: { pattern: String.raw`Hello,? world`, source: "file.content" } },
    violation: { anchor: "witness.span", code: "SHOW_ME", message: SHOW_ME_MESSAGE },
  }],
} as const;

/** Why the demo did not start or clean up; the launcher reports it and exits 1. */
export class ShowMeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ShowMeError";
  }
}

export interface ShowMeStart {
  ruleId: string;
  /** The demo file's absolute path; the coding agent writes it. */
  file: string;
  /** The demo file's path in the checkout, as steers name it. */
  path: string;
  /** What the coding agent writes first: it breaks the rule. */
  violating: string;
  /** What the steer asks for. */
  fixed: string;
  message: string;
  repositoryName: string;
}

export interface ShowMeCleanup {
  fileRemoved: boolean;
  ruleRemoved: boolean;
}

/** Whether `file` is absent (`"absent"`), the demo's own (`"demo"`), or something else that must be left alone. */
function fileOwner(file: string): "absent" | "demo" | "foreign" {
  // Not followed: a symlink, even a dangling one, is someone else's, and writing through it would land outside the repo.
  const stats = lstatSync(file, { throwIfNoEntry: false });
  if (stats === undefined) return "absent";
  if (!stats.isFile()) return "foreign";
  return readFileSync(file, "utf8").startsWith(SHOW_ME_MARKER) ? "demo" : "foreign";
}

/**
 * Whether the library holds no demo rule, the demo's own, or a rule someone else gave the demo's id. Judged on the
 * stored row, so a row this build cannot read back (quarantined) is still someone else's unless it is the demo's.
 */
function ruleOwner(database: RerailDatabase): "absent" | "demo" | "foreign" {
  const row = database.select().from(libraryRules).where(eq(libraryRules.ruleId, SHOW_ME_RULE_ID)).get();
  if (row === undefined) return "absent";
  const stored = row.document as { rule?: { description?: unknown } } | null;
  return row.kind === "code" && stored?.rule?.description === SHOW_ME_DESCRIPTION ? "demo" : "foreign";
}

/** Delete the demo file when it is the demo's own; anyone else's file is left alone. */
function removeDemoFile(file: string): boolean {
  const removed = fileOwner(file) === "demo";
  if (removed) unlinkSync(file);
  return removed;
}

/**
 * Take the demo rule out of `repositoryKey`'s repository, and out of the library once no repository has it: the rule
 * is one per machine, so a demo running in another repository keeps its rule.
 */
function removeDemoRule(database: RerailDatabase, repositoryKey: RepositoryKey | undefined): boolean {
  if (ruleOwner(database) !== "demo") return false;
  const detached = repositoryKey !== undefined
    && removeAttachment(database, { repositoryKey, sourceKind: "library_rule", sourceId: SHOW_ME_RULE_ID, folder: "" });
  const elsewhere = database.select({ key: ruleAttachments.repositoryKey }).from(ruleAttachments)
    .where(and(eq(ruleAttachments.sourceKind, "library_rule"), eq(ruleAttachments.sourceId, SHOW_ME_RULE_ID))).limit(1).get();
  if (elsewhere !== undefined) return detached;
  return removeLibraryRule(database, SHOW_ME_RULE_ID) || detached;
}

/**
 * Add the demo rule to `repo`'s repository, after deleting a demo file an interrupted demo left behind. Rerail must be
 * on and the repository set up there: the demo never starts Rerail or sets a repository up, so cleanup never has to
 * undo that. A demo rule already there stays: adding it again changes nothing.
 */
export function startShowMe(repo: string, databasePath = defaultRerailDatabasePath()): ShowMeStart {
  const workspace = resolve(repo);
  const name = repositoryName(workspace);
  const state = workspaceStartState(workspace, databasePath);
  if (state === "stopped") throw new ShowMeError(`Rerail is not on in ${name}. Start it first (rerail start), then try the demo again.`);
  if (state === "not-set-up") throw new ShowMeError(`Rerail is on in ${name}, which is not set up yet. Set it up first (rerail start), then try the demo again.`);
  const file = join(repositoryIdentityLocation(workspace).workspaceRoot, SHOW_ME_FILE);
  const database = openRerailDatabase(databasePath);
  try {
    if (fileOwner(file) === "foreign") throw new ShowMeError(`${file} was not written by the demo, so the demo left it alone. Move it, then try again.`);
    if (ruleOwner(database) === "foreign") throw new ShowMeError(`Your library has a rule ${SHOW_ME_RULE_ID} the demo did not add, so the demo left it alone.`);
    removeDemoFile(file);
    const { repositoryKey } = recordCheckout(database, workspace);
    upsertLibraryRule(database, loadBundledCatalog(), SHOW_ME_RULE, "code");
    addAttachment(database, { repositoryKey, sourceKind: "library_rule", sourceId: SHOW_ME_RULE_ID, folder: "" });
  } finally {
    database.$client.close();
  }
  return {
    ruleId: SHOW_ME_RULE_ID,
    file,
    path: SHOW_ME_FILE,
    violating: SHOW_ME_VIOLATING,
    fixed: SHOW_ME_FIXED,
    message: SHOW_ME_MESSAGE,
    repositoryName: name,
  };
}

/** Remove the demo file, and the demo rule from this repository; with neither left, it changes nothing. */
export function cleanUpShowMe(repo: string, databasePath = defaultRerailDatabasePath()): ShowMeCleanup {
  const workspace = resolve(repo);
  const file = join(repositoryIdentityLocation(workspace).workspaceRoot, SHOW_ME_FILE);
  const fileRemoved = removeDemoFile(file);
  if (!existsSync(databasePath)) return { fileRemoved, ruleRemoved: false };
  const database = openRerailDatabase(databasePath);
  try {
    return { fileRemoved, ruleRemoved: removeDemoRule(database, lookupCheckout(database, workspace)?.repositoryKey) };
  } finally {
    database.$client.close();
  }
}

export interface ShowMeCommandIo {
  stdout?: { write(text: string): unknown };
  stderr?: { write(text: string): unknown };
}

function cleanupText({ fileRemoved, ruleRemoved }: ShowMeCleanup): string {
  const removed = [fileRemoved ? SHOW_ME_FILE : undefined, ruleRemoved ? "the demo rule" : undefined].filter(item => item !== undefined);
  return removed.length === 0 ? "● Nothing to clean up: the demo left nothing behind." : `● The demo is cleaned up: removed ${removed.join(" and ")}.`;
}

/** `rerail show-me [--repo <dir>] [--json]` and `rerail show-me --cleanup [--repo <dir>] [--json]`. */
export function runShowMeCommand(args: readonly string[], usage: () => string, io: ShowMeCommandIo = {}): number {
  const stdout = io.stdout ?? process.stdout;
  const { flags, options } = parseArguments(
    args,
    { flags: ["--cleanup", "--json"], options: ["--repo"] },
    argument => `Unknown show-me option: ${argument}\n\n${usage()}`,
  );
  const repo = resolve(options.get("--repo") ?? process.cwd());
  const json = flags.has("--json");
  try {
    if (flags.has("--cleanup")) {
      const cleanup = cleanUpShowMe(repo);
      stdout.write(`${json ? JSON.stringify(cleanup) : cleanupText(cleanup)}\n`);
      return 0;
    }
    const start = startShowMe(repo);
    stdout.write(json ? `${JSON.stringify(start)}\n` : [
      `● Added the demo rule ${start.ruleId} to ${start.repositoryName}. It checks ${start.path} only.`,
      `  Write "Hello, world" in ${start.file} and Rerail steers the edit.`,
      "  Clean up with rerail show-me --cleanup.",
      "",
    ].join("\n"));
    return 0;
  } catch (error) {
    if (!(error instanceof ShowMeError)) throw error;
    (io.stderr ?? process.stderr).write(`● ${error.message}\n`);
    return 1;
  }
}
