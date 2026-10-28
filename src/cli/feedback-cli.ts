import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { join } from "node:path";
import { and, asc, eq, inArray, sql } from "drizzle-orm";
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
 * (`always`); `no` keeps every note local from then on, the ones already waiting to go included. An approved note goes
 * to Nera's feedback API from a detached `rerail feedback flush`, so the agent's command returns at once; a note the
 * API did not take stays queued for the next flush, and no flush sends anything once the answer is `never`.
 */

/** Nera's feedback API (the `nera-rerail-feedback` Azure Function); `RERAIL_FEEDBACK_URL` points a test elsewhere. */
export const FEEDBACK_API_URL = "https://nera-rerail-feedback.azurewebsites.net/api/feedback";

/** The API's limits, checked here so a note it would refuse is refused before it is kept. */
const RULE_ID_MAX = 200;
const REASON_MAX = 1000;
const SEND_TIMEOUT_MS = 10_000;

/** Control characters, which have no place in a rule id or a one-line reason and would drive a terminal that prints them. */
const CONTROL = /[\p{Cc}]/u;

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
  if (first === "sharing") return await withDatabase(context, database => sharing(database, rest, usage, context));
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

/**
 * Where approved notes go: Nera's API, or `RERAIL_FEEDBACK_URL` from the environment. An override must be `https:`,
 * or `http:` on this computer, so a variable a repository's settings slipped into the agent's shell cannot carry a note
 * off in the clear; the override is named in what the agent is told, so the person sees where a yes sends the note.
 */
function feedbackUrl(context: FeedbackContext): URL {
  const override = (context.env ?? process.env).RERAIL_FEEDBACK_URL;
  if (override === undefined || override === "") return new URL(FEEDBACK_API_URL);
  let url: URL;
  try {
    url = new URL(override);
  } catch {
    throw new Error(`RERAIL_FEEDBACK_URL is not a URL: ${override}`);
  }
  const local = url.hostname === "127.0.0.1" || url.hostname === "localhost" || url.hostname === "[::1]";
  if (url.protocol !== "https:" && !(url.protocol === "http:" && local)) {
    throw new Error(`RERAIL_FEEDBACK_URL must be https, or http on this computer: ${override}`);
  }
  return url;
}

/** `Nera`, or the host an override names, as the sentences about sending read. */
function destination(context: FeedbackContext): string {
  const url = feedbackUrl(context);
  return url.href === FEEDBACK_API_URL ? "Nera" : `${url.host} (RERAIL_FEEDBACK_URL, not Nera)`;
}

/** The one line the agent gave the person, as one line, with nothing a terminal would act on. */
function oneLine(words: readonly string[]): string {
  const reason = words.join(" ").replace(/\s+/gu, " ").trim();
  if (reason === "") throw new Error("rerail feedback needs your reason: the one line you gave the user");
  if (CONTROL.test(reason)) throw new Error("The reason holds control characters; give the plain line you gave the user");
  if (reason.length > REASON_MAX) throw new Error(`Keep the reason to one line of at most ${REASON_MAX} characters (this one has ${reason.length})`);
  return reason;
}

/** The checkout's repository, and the session in it the rule last steered with its coding agent, as far as Rerail knows. */
function steeredHere(database: RerailDatabase, ruleId: string, cwd: string): {
  repositoryId: string | null;
  session: { sessionId: string; host: RerailHost } | undefined;
} {
  const checkout = lookupCheckout(database, cwd);
  if (checkout === undefined) return { repositoryId: null, session: undefined };
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

/** What a yes sends, as the agent relays it to the person: every field of the request, and nothing it does not carry. */
function whatIsSent(ruleId: string, packId: string | null, packVersion: string | null, host: RerailHost | null, reason: string, version: string): string {
  const pack = packId === null ? "no rule pack (a rule created here)" : `rule pack ${packId}${packVersion === null ? "" : ` ${packVersion}`}`;
  return `the rule id (${ruleId}), its ${pack}, your coding agent's name (${host ?? "unknown"}), the reason below word for word, `
    + `the Rerail version (${version}), the time, and a random note id; nothing else, and never the session, the repository, paths or code`;
}

function record(database: RerailDatabase, ruleId: string, words: readonly string[], context: FeedbackContext): number {
  // Where a yes would send it is checked before anything is kept, so a bad override keeps no note.
  const to = destination(context);
  const reason = oneLine(words);
  if (ruleId.length > RULE_ID_MAX) throw new Error(`A rule id is at most ${RULE_ID_MAX} characters`);
  if (CONTROL.test(ruleId)) throw new Error("A rule id holds no control characters; use the id in brackets at the end of the steer's line");
  const { repositoryId, session } = steeredHere(database, ruleId, context.cwd ?? process.cwd());
  const packs = loadRulePacks();
  const packId = rulePackIdOf(ruleId, packs);
  const known = session !== undefined || packId !== null
    || database.select({ id: rules.id }).from(rules).where(eq(rules.id, ruleId)).get() !== undefined
    || database.select({ id: libraryRules.ruleId }).from(libraryRules).where(eq(libraryRules.ruleId, ruleId)).get() !== undefined;
  if (!known) throw new Error(`Rerail has no rule ${ruleId}; use the id in brackets at the end of the steer's line`);
  const standing = readConsent(context.home).feedback?.sharing;
  const shared: SteerFeedbackSharing = standing === "always" ? "queued" : standing === "never" ? "local" : "ask";
  const packVersion = packId === null ? null : packs.find(pack => pack.id === packId)?.version ?? null;
  const host = session?.host ?? null;
  const id = randomUUID();
  database.insert(steerFeedback).values({
    id,
    ruleId,
    packId,
    packVersion,
    host,
    sessionId: session?.sessionId ?? null,
    repositoryId,
    reason,
    rerailVersion: context.packageVersion,
    recordedAt: Date.now(),
    sharing: shared,
  }).run();
  if (shared === "queued") {
    write(context, `Rerail kept your note on ${ruleId} and is sending it to ${to}: the user chose to share every note. Nothing more to do.`);
    startFlush(context);
  } else if (shared === "local") {
    write(context, `Rerail kept your note on ${ruleId} on this computer only. Nothing more to do.`);
  } else {
    write(context, [
      `Rerail kept your note on ${ruleId} on this computer (note ${id}):`,
      `  ${reason}`,
      `Ask the user whether Rerail may send it to ${to} to improve the rule, showing them that line as it is. `
        + `It would send ${whatIsSent(ruleId, packId, packVersion, host, reason, context.packageVersion)}. `
        + "Do not answer for them. Then run the command for their answer:",
      `  yes, this time:  rerail feedback answer ${id} yes`,
      `  yes, always:     rerail feedback answer ${id} always`,
      `  no:              rerail feedback answer ${id} no`,
    ].join("\n"));
  }
  return 0;
}

/** The answer is never: keep every note that was waiting to go, and ask no more. */
function keepEverythingLocal(database: RerailDatabase, home: string, answeredAt: string): number {
  const kept = database.update(steerFeedback).set({ sharing: "local" }).where(inArray(steerFeedback.sharing, ["ask", "queued"])).run().changes;
  updateConsent(home, consent => ({ ...consent, feedback: { sharing: "never", answeredAt } }));
  return kept;
}

function answer(database: RerailDatabase, args: readonly string[], usage: () => string, context: FeedbackContext): number {
  const [id, choice, ...extra] = args;
  const to = destination(context);
  if (id === undefined || !ANSWERS.includes(choice as Answer) || extra.length > 0) {
    throw new Error(`Usage: rerail feedback answer <note-id> yes|always|no\n\n${usage()}`);
  }
  const note = database.select({ ruleId: steerFeedback.ruleId, sharing: steerFeedback.sharing })
    .from(steerFeedback).where(eq(steerFeedback.id, id)).get();
  if (note === undefined) throw new Error(`Rerail has no feedback note ${id}`);
  const answeredAt = new Date().toISOString();
  if (choice === "no") {
    const kept = keepEverythingLocal(database, context.home, answeredAt);
    const others = kept > 1 ? `, as ${kept - 1} other ${kept === 2 ? "note" : "notes"} that had not gone yet` : "";
    const this_ = note.sharing === "sent" ? "This note was already sent, but no other will be" : `This note stays on this computer${others}`;
    write(context, `${this_}, and Rerail will not ask again. \`rerail feedback sharing ask\` turns asking back on.`);
    return 0;
  }
  if (note.sharing === "ask" || note.sharing === "local") {
    database.update(steerFeedback).set({ sharing: "queued" }).where(eq(steerFeedback.id, id)).run();
  }
  if (choice === "always") updateConsent(context.home, consent => ({ ...consent, feedback: { sharing: "always", answeredAt } }));
  if (note.sharing !== "sent" && note.sharing !== "rejected") startFlush(context);
  write(context, choice === "always"
    ? `Sending the note on ${note.ruleId} to ${to}. From now on Rerail sends every note without asking; \`rerail feedback sharing ask\` turns that off.`
    : `Sending the note on ${note.ruleId} to ${to}. Rerail will ask again about the next one.`);
  return 0;
}

const SHARING_TEXT: Readonly<Record<Sharing, string>> = {
  ask: "Rerail asks before sending each feedback note to Nera.",
  always: "Rerail sends every feedback note to Nera without asking.",
  never: "Rerail keeps every feedback note on this computer and never asks.",
};

function sharing(database: RerailDatabase, args: readonly string[], usage: () => string, context: FeedbackContext): number {
  const [setting, ...extra] = args;
  if (extra.length > 0 || (setting !== undefined && !SHARING_SETTINGS.includes(setting as Sharing))) {
    throw new Error(`Usage: rerail feedback sharing [ask|always|never]\n\n${usage()}`);
  }
  if (setting !== undefined) {
    const answeredAt = new Date().toISOString();
    if (setting === "never") keepEverythingLocal(database, context.home, answeredAt);
    else {
      updateConsent(context.home, ({ feedback: _feedback, ...others }) =>
        setting === "ask" ? others : { ...others, feedback: { sharing: "always", answeredAt } });
    }
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
 * Send every queued note, oldest first, unless the answer is now never. Accepted notes are `sent`; a note the API
 * refuses as malformed is `rejected` and never retried; anything else (offline, a timeout, a server error) stops the
 * run and leaves the rest queued. Each note's state is read again just before it goes, so a `no` given while a flush
 * runs still holds it back.
 */
async function flush(database: RerailDatabase, context: FeedbackContext): Promise<number> {
  if (readConsent(context.home).feedback?.sharing === "never") {
    write(context, "Sent no feedback notes: the user keeps them on this computer.");
    return 0;
  }
  const url = feedbackUrl(context);
  const send = context.fetch ?? fetch;
  const queued = database.select().from(steerFeedback).where(eq(steerFeedback.sharing, "queued"))
    .orderBy(asc(steerFeedback.recordedAt), sql`rowid`).all();
  let sent = 0;
  let rejected = 0;
  let withheld = 0;
  for (const note of queued) {
    const current = database.select({ sharing: steerFeedback.sharing }).from(steerFeedback).where(eq(steerFeedback.id, note.id)).get();
    if (current?.sharing !== "queued") {
      withheld += 1;
      continue;
    }
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
        // A redirect would re-post the note wherever the answer pointed.
        redirect: "error",
        signal: AbortSignal.timeout(SEND_TIMEOUT_MS),
      });
      status = response.status;
      await response.body?.cancel();
    } catch {
      break;
    }
    if (status >= 200 && status < 300) {
      // Still queued only if no `no` arrived while the request was out.
      database.update(steerFeedback).set({ sharing: "sent", sentAt: Date.now() })
        .where(and(eq(steerFeedback.id, note.id), eq(steerFeedback.sharing, "queued"))).run();
      sent += 1;
    } else if (status >= 400 && status < 500 && status !== 408 && status !== 429) {
      database.update(steerFeedback).set({ sharing: "rejected" })
        .where(and(eq(steerFeedback.id, note.id), eq(steerFeedback.sharing, "queued"))).run();
      rejected += 1;
    } else {
      break;
    }
  }
  const left = queued.length - sent - rejected - withheld;
  write(context, `Sent ${sent} feedback ${sent === 1 ? "note" : "notes"}${rejected > 0 ? `, ${rejected} refused by the API` : ""}${left > 0 ? `, ${left} still queued` : ""}.`);
  return 0;
}
