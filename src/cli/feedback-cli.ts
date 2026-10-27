import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { join } from "node:path";
import { and, asc, eq, sql } from "drizzle-orm";
import type { RerailHost } from "../engine/adapter-contract.js";
import { listRepositories, lookupCheckout, repositoryHistoryIds } from "../engine/catalog/repository-registry.js";
import { loadRulePacks, rulePackIdOf } from "../engine/catalog/rule-packs.js";
import { ENTRY_POINTS, PACKAGE_ROOT } from "../engine/package-root.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { events, libraryRules, rules, sessions, steerFeedback, type SteerFeedbackSharing } from "../engine/storage/schema.js";
import { nodeExecutable } from "../runtime/daemon-control.js";
import { readConsent, updateConsent, type FeedbackSharingAnswer } from "./consent.js";

/**
 * `rerail feedback`: a coding agent's one-line reason for not following a steer, kept in the Rerail database for
 * tuning the rule. Nothing leaves the computer until the person says yes: once (`answer <id> yes`), or for every note
 * (`always`); `no` keeps every note local from then on. An approved note goes to Nera's feedback API from a detached
 * `rerail feedback flush`, so the agent's command returns at once; a note the API did not take stays queued for the
 * next flush.
 */

/** Nera's feedback API (the `nera-rerail-feedback` Azure Function); `RERAIL_FEEDBACK_URL` points a test elsewhere. */
export const FEEDBACK_API_URL = "https://nera-rerail-feedback.azurewebsites.net/api/feedback";

/** The API's limits, checked here so a note it would refuse is refused before it is kept. */
const RULE_ID_MAX = 200;
const REASON_MAX = 1000;
const SEND_TIMEOUT_MS = 10_000;

const ANSWERS = ["yes", "always", "no"] as const;
type Answer = (typeof ANSWERS)[number];

/** `ask`: no standing answer, so the person is asked about each note. */
type Sharing = FeedbackSharingAnswer | "ask";
const SHARING_SETTINGS: readonly Sharing[] = ["ask", "always", "never"];

export interface FeedbackContext {
  home: string;
  packageVersion: string;
  databasePath?: string;
  cwd?: string;
  env?: NodeJS.ProcessEnv;
  stdout?: { write(text: string): unknown };
  fetch?: typeof fetch;
  /** Send the queued notes without holding the caller: a detached `rerail feedback flush`. */
  startFlush?: () => void;
}

/** `rerail feedback <rule-id> <reason>`, `answer <id> yes|always|no`, `sharing [ask|always|never]`, and the internal `flush`. */
export async function runFeedbackCommand(args: readonly string[], usage: () => string, context: FeedbackContext): Promise<number> {
  const [first, ...rest] = args;
  if (first === undefined) throw new Error(`rerail feedback needs a rule id and your reason\n\n${usage()}`);
  if (first === "answer") return await withDatabase(context, database => answer(database, rest, usage, context));
  if (first === "sharing") return sharing(rest, usage, context);
  if (first === "flush") {
    if (rest.length > 0) throw new Error("rerail feedback flush takes no arguments");
    return await withDatabase(context, database => flush(database, context));
  }
  return await withDatabase(context, database => record(database, first, rest, context));
}

async function withDatabase(context: FeedbackContext, run: (database: RerailDatabase) => number | Promise<number>): Promise<number> {
  const database = openRerailDatabase(context.databasePath ?? defaultRerailDatabasePath(context.env));
  try {
    return await run(database);
  } finally {
    database.$client.close();
  }
}

const write = (context: FeedbackContext, text: string): void => {
  (context.stdout ?? process.stdout).write(`${text}\n`);
};

/** The one line the agent gave the person, as one line. */
function oneLine(words: readonly string[]): string {
  const reason = words.join(" ").replace(/\s+/gu, " ").trim();
  if (reason === "") throw new Error("rerail feedback needs your reason: the one line you gave the user");
  if (reason.length > REASON_MAX) throw new Error(`Keep the reason to one line of at most ${REASON_MAX} characters (this one has ${reason.length})`);
  return reason;
}

/** The session in this repository the rule last steered, with its coding agent; undefined when it steered none here. */
function steeredSession(database: RerailDatabase, ruleId: string, cwd: string): { sessionId: string; host: RerailHost } | undefined {
  const checkout = lookupCheckout(database, cwd);
  if (checkout === undefined) return undefined;
  const repository = listRepositories(database).find(candidate => candidate.repositoryKey === checkout.repositoryKey);
  const repositoryIds = repository === undefined ? [checkout.repositoryKey] : repositoryHistoryIds(repository);
  return database.select({ sessionId: sessions.sessionId, host: sessions.host })
    .from(events).innerJoin(sessions, eq(sessions.sessionId, events.sessionId))
    .where(and(
      eq(events.kind, "steering-delivered"),
      sql`${sessions.repositoryId} IN (${sql.join(repositoryIds.map(id => sql`${id}`), sql`, `)})`,
      sql`EXISTS (SELECT 1 FROM json_each(${events.payloadJson}, '$.ruleIds') WHERE value = ${ruleId})`,
    ))
    .orderBy(sql`${events.sequence} DESC`).limit(1).get();
}

function record(database: RerailDatabase, ruleId: string, words: readonly string[], context: FeedbackContext): number {
  const reason = oneLine(words);
  if (ruleId.length > RULE_ID_MAX) throw new Error(`A rule id is at most ${RULE_ID_MAX} characters`);
  const steered = steeredSession(database, ruleId, context.cwd ?? process.cwd());
  const packs = loadRulePacks();
  const packId = rulePackIdOf(ruleId, packs);
  const known = steered !== undefined || packId !== null
    || database.select({ id: rules.id }).from(rules).where(eq(rules.id, ruleId)).get() !== undefined
    || database.select({ id: libraryRules.ruleId }).from(libraryRules).where(eq(libraryRules.ruleId, ruleId)).get() !== undefined;
  if (!known) throw new Error(`Rerail has no rule ${ruleId}; use the id in brackets at the end of the steer's line`);
  const standing = readConsent(context.home).feedback?.sharing;
  const shared: SteerFeedbackSharing = standing === "always" ? "queued" : standing === "never" ? "local" : "ask";
  const id = randomUUID();
  database.insert(steerFeedback).values({
    id,
    ruleId,
    packId,
    packVersion: packId === null ? null : packs.find(pack => pack.id === packId)?.version ?? null,
    host: steered?.host ?? null,
    sessionId: steered?.sessionId ?? null,
    reason,
    rerailVersion: context.packageVersion,
    recordedAt: Date.now(),
    sharing: shared,
  }).run();
  if (shared === "queued") {
    write(context, `Rerail kept your note on ${ruleId} and is sending it to Nera: the user chose to share every note. Nothing more to do.`);
    startFlush(context);
  } else if (shared === "local") {
    write(context, `Rerail kept your note on ${ruleId} on this computer only. Nothing more to do.`);
  } else {
    write(context, [
      `Rerail kept your note on ${ruleId} on this computer (note ${id}).`,
      "Ask the user, in one line, whether Rerail may send it to Nera to improve the rule. It would send the rule id, "
        + "its rule pack and version, your coding agent's name and your reason, nothing else. Do not answer for them. "
        + "Then run the command for their answer:",
      `  yes, this time:  rerail feedback answer ${id} yes`,
      `  yes, always:     rerail feedback answer ${id} always`,
      `  no:              rerail feedback answer ${id} no`,
    ].join("\n"));
  }
  return 0;
}

function answer(database: RerailDatabase, args: readonly string[], usage: () => string, context: FeedbackContext): number {
  const [id, choice, ...extra] = args;
  if (id === undefined || !ANSWERS.includes(choice as Answer) || extra.length > 0) {
    throw new Error(`Usage: rerail feedback answer <note-id> yes|always|no\n\n${usage()}`);
  }
  const note = database.select({ ruleId: steerFeedback.ruleId, sharing: steerFeedback.sharing })
    .from(steerFeedback).where(eq(steerFeedback.id, id)).get();
  if (note === undefined) throw new Error(`Rerail has no feedback note ${id}`);
  const answeredAt = new Date().toISOString();
  if (choice === "no") {
    if (note.sharing === "ask" || note.sharing === "queued") {
      database.update(steerFeedback).set({ sharing: "local" }).where(eq(steerFeedback.id, id)).run();
    }
    updateConsent(context.home, consent => ({ ...consent, feedback: { sharing: "never", answeredAt } }));
    const kept = note.sharing === "sent" ? "Note was already sent, but no other note will be" : "Note stays on this computer";
    write(context, `${kept}, and Rerail will not ask again. \`rerail feedback sharing ask\` turns asking back on.`);
    return 0;
  }
  if (note.sharing === "ask" || note.sharing === "local") {
    database.update(steerFeedback).set({ sharing: "queued" }).where(eq(steerFeedback.id, id)).run();
  }
  if (choice === "always") updateConsent(context.home, consent => ({ ...consent, feedback: { sharing: "always", answeredAt } }));
  if (note.sharing !== "sent" && note.sharing !== "rejected") startFlush(context);
  write(context, choice === "always"
    ? `Sending the note on ${note.ruleId} to Nera. From now on Rerail sends every note without asking; \`rerail feedback sharing ask\` turns that off.`
    : `Sending the note on ${note.ruleId} to Nera. Rerail will ask again about the next one.`);
  return 0;
}

const SHARING_TEXT: Readonly<Record<Sharing, string>> = {
  ask: "Rerail asks before sending each feedback note to Nera.",
  always: "Rerail sends every feedback note to Nera without asking.",
  never: "Rerail keeps every feedback note on this computer and never asks.",
};

function sharing(args: readonly string[], usage: () => string, context: FeedbackContext): number {
  const [setting, ...extra] = args;
  if (extra.length > 0 || (setting !== undefined && !SHARING_SETTINGS.includes(setting as Sharing))) {
    throw new Error(`Usage: rerail feedback sharing [ask|always|never]\n\n${usage()}`);
  }
  if (setting !== undefined) {
    const answeredAt = new Date().toISOString();
    updateConsent(context.home, ({ feedback: _feedback, ...others }) =>
      setting === "ask" ? others : { ...others, feedback: { sharing: setting as FeedbackSharingAnswer, answeredAt } });
  }
  write(context, SHARING_TEXT[readConsent(context.home).feedback?.sharing ?? "ask"]);
  return 0;
}

function startFlush(context: FeedbackContext): void {
  if (context.startFlush !== undefined) {
    context.startFlush();
    return;
  }
  const child = spawn(nodeExecutable(), [join(PACKAGE_ROOT, ENTRY_POINTS.cli), "feedback", "flush"], {
    detached: true,
    stdio: "ignore",
    env: context.env ?? process.env,
  });
  child.on("error", () => undefined);
  child.unref();
}

/**
 * Send every queued note, oldest first. Accepted notes are `sent`; a note the API refuses as malformed is `rejected`
 * and never retried; anything else (offline, a timeout, a server error) stops the run and leaves the rest queued.
 */
async function flush(database: RerailDatabase, context: FeedbackContext): Promise<number> {
  const url = (context.env ?? process.env).RERAIL_FEEDBACK_URL ?? FEEDBACK_API_URL;
  const send = context.fetch ?? fetch;
  const queued = database.select().from(steerFeedback).where(eq(steerFeedback.sharing, "queued"))
    .orderBy(asc(steerFeedback.recordedAt), sql`rowid`).all();
  let sent = 0;
  let rejected = 0;
  for (const note of queued) {
    // Never the session, the repository or anything else Rerail records about the note.
    const body = {
      id: note.id,
      ruleId: note.ruleId,
      packId: note.packId,
      packVersion: note.packVersion,
      agent: note.host,
      reason: note.reason,
      rerailVersion: note.rerailVersion,
      recordedAt: new Date(note.recordedAt).toISOString(),
    };
    let status: number;
    try {
      const response = await send(url, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
        signal: AbortSignal.timeout(SEND_TIMEOUT_MS),
      });
      status = response.status;
    } catch {
      break;
    }
    if (status >= 200 && status < 300) {
      database.update(steerFeedback).set({ sharing: "sent", sentAt: Date.now() }).where(eq(steerFeedback.id, note.id)).run();
      sent += 1;
    } else if (status >= 400 && status < 500 && status !== 408 && status !== 429) {
      database.update(steerFeedback).set({ sharing: "rejected" }).where(eq(steerFeedback.id, note.id)).run();
      rejected += 1;
    } else {
      break;
    }
  }
  const left = queued.length - sent - rejected;
  write(context, `Sent ${sent} feedback ${sent === 1 ? "note" : "notes"}${rejected > 0 ? `, ${rejected} refused by the API` : ""}${left > 0 ? `, ${left} still queued` : ""}.`);
  return 0;
}
