import { randomUUID } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { and, asc, eq, inArray, sql } from "drizzle-orm";
import type { RerailHost } from "../engine/adapter-contract.js";
import { listRepositories, lookupCheckout, repositoryHistoryIds } from "../engine/catalog/repository-registry.js";
import { gitExecutable } from "../engine/git.js";
import { activeCoreVersion, installedPackDirectories, readRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { events, libraryRules, rules, sessions, steerFeedback } from "../engine/storage/schema.js";

/**
 * `rerail feedback`: a coding agent's one-line reason for not following a steer, kept in the Rerail database on this
 * computer for tuning the rule. Rerail sends it nowhere; `rerail feedback list` prints the notes for the person to
 * attach to a rule misfire issue.
 */

/** Where the person shares notes with Nera: the rule misfire issue form, which asks for `rerail feedback list`. */
export const RULE_MISFIRE_ISSUE_URL = "https://github.com/neralabs/rerail/issues/new?template=2-rule-misfire.yml";

const RULE_ID_MAX = 200;
const REASON_MAX = 1000;

/**
 * Control and format characters: a terminal escape would drive the terminal that prints the note, and a bidi override
 * or a zero-width character would make the line read differently from what it holds once pasted into an issue.
 */
const CONTROL = /[\p{Cc}\p{Cf}]/u;

export interface FeedbackContext {
  packageVersion: string;
  databasePath?: string;
  cwd?: string;
  env?: NodeJS.ProcessEnv;
  stdout?: { write(text: string): unknown };
  /** The whole of standard input, for a reason given as `-`. */
  stdin?: () => string;
}

/** `rerail feedback <rule-id> <reason>` (or `-`, the reason on stdin), and `list [--rule <rule-id>]`. */
export async function runFeedbackCommand(args: readonly string[], usage: () => string, context: FeedbackContext): Promise<number> {
  const [first, ...rest] = args;
  if (first === undefined) throw new Error(`rerail feedback needs a rule id and your reason\n\n${usage()}`);
  const databasePath = context.databasePath ?? defaultRerailDatabasePath(context.env);
  if (first === "list") return list(databasePath, rest, usage, context);
  // `-` takes the line on stdin, so a `'` in it needs no shell quoting (a heredoc with a quoted delimiter).
  const words = rest.length === 1 && rest[0] === "-" ? [context.stdin?.() ?? readFileSync(0, "utf8")] : rest;
  const database = openRerailDatabase(databasePath);
  try {
    return record(database, first, words, context);
  } finally {
    database.$client.close();
  }
}

const write = (context: FeedbackContext, text: string): void => {
  (context.stdout ?? process.stdout).write(`${text}\n`);
};

/** The one line the agent gave the person, as one line, with nothing a terminal would act on. */
function oneLine(words: readonly string[]): string {
  const reason = words.join(" ").replace(/\s+/gu, " ").trim();
  if (reason === "") throw new Error("rerail feedback needs your reason: the one line you gave the user");
  if (CONTROL.test(reason)) throw new Error("The reason holds control or invisible characters; give the plain line you gave the user");
  if (reason.length > REASON_MAX) throw new Error(`Keep the reason to one line of at most ${REASON_MAX} characters (this one has ${reason.length})`);
  return reason;
}

/**
 * The checkout's repository, and the session in it the rule last steered with its coding agent, as far as Rerail knows.
 * A note needs a repository: `rerail reset` deletes notes by repository, and a steer only ever comes in one.
 */
function steeredHere(database: RerailDatabase, ruleId: string, cwd: string): {
  repositoryId: string;
  session: { sessionId: string; host: RerailHost } | undefined;
} {
  // Without Git no checkout is found at all: say that, not that this is no repository.
  gitExecutable();
  const checkout = lookupCheckout(database, cwd);
  if (checkout === undefined) {
    throw new Error(`Rerail knows no repository at ${cwd}; run rerail feedback in the repository the steer came from`);
  }
  const repository = listRepositories(database).find(candidate => candidate.repositoryKey === checkout.repositoryKey);
  const repositoryIds = repository === undefined ? [checkout.repositoryKey] : repositoryHistoryIds(repository);
  const session = database.select({ sessionId: sessions.sessionId, host: sessions.host })
    .from(events).innerJoin(sessions, eq(sessions.sessionId, events.sessionId))
    .where(and(
      eq(events.kind, "steering-delivered"),
      inArray(sessions.repositoryId, repositoryIds),
      sql`EXISTS (SELECT 1 FROM json_each(${events.payloadJson}, '$.ruleIds') WHERE value = ${ruleId})`,
    ))
    .orderBy(sql`${events.sequence} DESC`).limit(1).get();
  return { repositoryId: checkout.repositoryKey, session };
}

function record(database: RerailDatabase, ruleId: string, words: readonly string[], context: FeedbackContext): number {
  const reason = oneLine(words);
  if (ruleId.length > RULE_ID_MAX) throw new Error(`A rule id is at most ${RULE_ID_MAX} characters`);
  if (CONTROL.test(ruleId)) throw new Error("A rule id holds no control or invisible characters; use the id in brackets at the end of the steer's line");
  const { repositoryId, session } = steeredHere(database, ruleId, context.cwd ?? process.cwd());
  // Which installed pack lists the rule, from the packs' manifests: a note needs no compiled rule, and compiling every
  // installed pack's rules (`loadRulePacks`) would cost each `rerail feedback` seconds.
  const home = defaultRerailHome(context.env);
  const catalog = readRuleCatalog(installedPackDirectories(home), { coreVersion: activeCoreVersion(home) });
  const packId = catalog.packOf.get(ruleId) ?? null;
  const known = session !== undefined || packId !== null
    || database.select({ id: rules.id }).from(rules).where(eq(rules.id, ruleId)).get() !== undefined
    || database.select({ id: libraryRules.ruleId }).from(libraryRules).where(eq(libraryRules.ruleId, ruleId)).get() !== undefined;
  if (!known) throw new Error(`Rerail has no rule ${ruleId}; use the id in brackets at the end of the steer's line`);
  database.insert(steerFeedback).values({
    id: randomUUID(),
    ruleId,
    packId,
    packVersion: packId === null ? null : catalog.packs.get(packId)?.version ?? null,
    host: session?.host ?? null,
    sessionId: session?.sessionId ?? null,
    repositoryId,
    reason,
    rerailVersion: context.packageVersion,
    recordedAt: Date.now(),
  }).run();
  write(context, `Rerail kept your note on ${ruleId} on this computer and sends it nowhere. Nothing more to do. `
    + `If the user wants Nera to see it, \`rerail feedback list --rule ${ruleId}\` prints it for a rule misfire issue (${RULE_MISFIRE_ISSUE_URL}).`);
  return 0;
}

/**
 * The notes, oldest first, as the person pastes them into an issue: when, the rule, its pack as `<pack>@<version>`
 * (the issue form's own wording), the coding agent and the Rerail version, then the reason. Never the session or the
 * repository, which an issue has no use for.
 */
function list(databasePath: string, args: readonly string[], usage: () => string, context: FeedbackContext): number {
  const [flag, ruleId, ...extra] = args;
  if ((flag !== undefined && (flag !== "--rule" || ruleId === undefined)) || extra.length > 0) {
    throw new Error(`Usage: rerail feedback list [--rule <rule-id>]\n\n${usage()}`);
  }
  const notes = existsSync(databasePath) ? readNotes(databasePath, ruleId) : [];
  if (notes.length === 0) {
    write(context, ruleId === undefined ? "Rerail has no feedback notes." : `Rerail has no feedback notes on ${ruleId}.`);
    return 0;
  }
  write(context, notes.map(note => {
    const pack = note.packId === null ? "own rule" : `${note.packId}@${note.packVersion ?? "unversioned"}`;
    return `${new Date(note.recordedAt).toISOString()}  ${note.ruleId}  (${pack}, ${note.host ?? "unknown agent"}, Rerail ${note.rerailVersion})\n  ${note.reason}`;
  }).join("\n"));
  return 0;
}

function readNotes(databasePath: string, ruleId: string | undefined) {
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    return database.select().from(steerFeedback)
      .where(ruleId === undefined ? undefined : eq(steerFeedback.ruleId, ruleId))
      .orderBy(asc(steerFeedback.recordedAt), sql`rowid`).all();
  } finally {
    database.$client.close();
  }
}
