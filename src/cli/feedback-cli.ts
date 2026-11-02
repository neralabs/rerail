import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { appendFileSync, existsSync, readFileSync } from "node:fs";
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
 * API did not take stays queued for the next flush (the daemon's start, the next note or the next answer), and no
 * flush sends anything once the answer is `never`.
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
  /** The whole of standard input, for a reason given as `-`. */
  stdin?: () => string;
  fetch?: typeof fetch;
  /** Send the queued notes without holding the caller: a detached `rerail feedback flush`. */
  startFlush?: () => void;
}

/**
 * `rerail feedback <rule-id> <reason>` (or `-`, the reason on stdin), `answer <id> yes|always|no`,
 * `sharing [ask|always|never]`, and the internal `flush`.
 */
export async function runFeedbackCommand(args: readonly string[], usage: () => string, context: FeedbackContext): Promise<number> {
  const [first, ...rest] = args;
  if (first === undefined) throw new Error(`rerail feedback needs a rule id and your reason\n\n${usage()}`);
  if (first === "answer") return await withDatabase(context, database => answer(database, rest, usage, context));
  if (first === "sharing") return await withDatabase(context, database => sharing(database, rest, usage, context));
  if (first === "flush") {
    if (rest.length > 0) throw new Error("rerail feedback flush takes no arguments");
    // Before the database opens: under never a flush has nothing to do, so it touches nothing.
    if (readConsent(context.home).feedback?.sharing === "never") {
      write(context, "Sent no feedback notes: the user keeps them on this computer.");
      return 0;
    }
    try {
      return await withDatabase(context, database => flush(database, context));
    } catch (error) {
      // A detached flush's stderr goes nowhere; the log is where the person is told to look.
      logFlush(context.home, `failed: ${error instanceof Error ? error.message : String(error)}`);
      throw error;
    }
  }
  // `-` takes the line on stdin, so a `'` in it needs no shell quoting (a heredoc with a quoted delimiter).
  const words = rest.length === 1 && rest[0] === "-" ? [context.stdin?.() ?? readFileSync(0, "utf8")] : rest;
  return await withDatabase(context, database => record(database, first, words, context));
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
 * Where a yes sends a note: Nera's API, or `RERAIL_FEEDBACK_URL` from the environment. An override must be `https:`,
 * or `http:` on this computer, so a variable a repository's settings slipped into the agent's shell cannot carry a note
 * off in the clear; the override is named in what the agent is told, so the person sees where a yes sends the note,
 * and the note keeps that destination once approved (`steer_feedback.destination`), so a flush sends it nowhere else.
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
function destinationLabel(href: string): string {
  return href === FEEDBACK_API_URL ? "Nera" : `${new URL(href).host} (RERAIL_FEEDBACK_URL, not Nera)`;
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
  const url = feedbackUrl(context).href;
  const to = destinationLabel(url);
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
    destination: shared === "queued" ? url : null,
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

/** The answer is never: keep every note that was waiting to go (claimed ones included), and ask no more. */
function keepEverythingLocal(database: RerailDatabase, home: string, answeredAt: string): number {
  const kept = database.update(steerFeedback).set({ sharing: "local", claimedAt: null, destination: null })
    .where(inArray(steerFeedback.sharing, ["ask", "queued", "sending"])).run().changes;
  updateConsent(home, consent => ({ ...consent, feedback: { sharing: "never", answeredAt } }));
  return kept;
}

/** What a yes does to a note in each state, as the agent is told; `queued` and `sending` are already on their way. */
function yesOutcome(sharing: SteerFeedbackSharing, ruleId: string, to: string): string {
  switch (sharing) {
    case "ask":
    case "local":
    case "queued":
    case "sending":
      return `Sending the note on ${ruleId} to ${to}.`;
    case "sent":
      return `The note on ${ruleId} was already sent to ${to}.`;
    case "rejected":
      return `The note on ${ruleId} was refused by ${to} as a bad note and is not sent again.`;
  }
}

function answer(database: RerailDatabase, args: readonly string[], usage: () => string, context: FeedbackContext): number {
  const [id, choice, ...extra] = args;
  if (id === undefined || !ANSWERS.includes(choice as Answer) || extra.length > 0) {
    throw new Error(`Usage: rerail feedback answer <note-id> yes|always|no\n\n${usage()}`);
  }
  const note = database.select({ ruleId: steerFeedback.ruleId, sharing: steerFeedback.sharing, destination: steerFeedback.destination })
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
  // Under never no flush sends, so a yes would leave the note queued until the answer changed: refuse it instead.
  if (choice === "yes" && readConsent(context.home).feedback?.sharing === "never") {
    throw new Error("The user keeps feedback notes on this computer (sharing is never); the note stays here. `rerail feedback sharing ask` turns asking back on first.");
  }
  // A note approved now goes where this answer says; one approved before keeps the destination it was approved for
  // (the `steer_feedback_destination` check: every note past `ask` and `local` has one).
  const approving = note.sharing === "ask" || note.sharing === "local";
  const destination = approving ? feedbackUrl(context).href : note.destination!;
  if (approving) {
    database.update(steerFeedback).set({ sharing: "queued", destination }).where(eq(steerFeedback.id, id)).run();
  }
  if (choice === "always") updateConsent(context.home, consent => ({ ...consent, feedback: { sharing: "always", answeredAt } }));
  if (note.sharing !== "sent" && note.sharing !== "rejected") startFlush(context);
  write(context, `${yesOutcome(note.sharing, note.ruleId, destinationLabel(destination))} ${choice === "always"
    ? "From now on Rerail sends every note without asking; `rerail feedback sharing ask` turns that off."
    : "Rerail will ask again about the next one."}`);
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
  const standing = readConsent(context.home).feedback?.sharing ?? "ask";
  const waiting = waitingCount(database);
  const queue = standing === "never" || waiting === 0 ? "" : ` ${waiting} approved ${waiting === 1 ? "note is" : "notes are"} waiting to be sent (see ${join(context.home, FEEDBACK_FLUSH_LOG)} if they stay).`;
  write(context, `${SHARING_TEXT[standing]}${queue}`);
  if (waiting > 0 && standing !== "never") startFlush(context);
  return 0;
}

function startFlush(context: FeedbackContext): void {
  if (context.startFlush !== undefined) {
    context.startFlush();
    return;
  }
  spawnFlush(context.home, context.env ?? process.env);
}

/** A detached `rerail feedback flush`; it has no one to tell, so a failure to start it goes to the flush log. */
function spawnFlush(home: string, env: NodeJS.ProcessEnv): void {
  const child = spawn(nodeExecutable(), [join(PACKAGE_ROOT, ENTRY_POINTS.cli), "feedback", "flush"], {
    detached: true,
    stdio: "ignore",
    env,
  });
  child.on("error", error => logFlush(home, `could not start a flush: ${error.message}`));
  child.unref();
}

/**
 * At the daemon's start, for approved notes an earlier flush could not deliver (offline, say): a flush starts only
 * when such notes wait and the answer is not never, so a daemon with nothing to send starts nothing. It never throws
 * into the daemon; what went wrong goes to the flush log.
 */
export function flushWaitingFeedback(home: string, databasePath: string, env: NodeJS.ProcessEnv = process.env): void {
  try {
    if (readConsent(home).feedback?.sharing === "never" || !existsSync(databasePath)) return;
    const database = openRerailDatabase(databasePath, { readonly: true });
    let waiting: number;
    try {
      waiting = waitingCount(database);
    } finally {
      database.$client.close();
    }
    if (waiting > 0) spawnFlush(home, env);
  } catch (error) {
    logFlush(home, `could not check for waiting notes: ${error instanceof Error ? error.message : String(error)}`);
  }
}

/** A claim older than this belongs to a flush that died mid-send; its note is queued again. */
const STALE_CLAIM_MS = 3 * SEND_TIMEOUT_MS;

/** Statuses that mean this note is bad and would be refused again; any other refusal is the endpoint's, so the note waits. */
const BAD_NOTE_STATUSES = new Set([400, 413, 422]);

/** In the Rerail home: what each flush did, since a detached flush has no one to tell. Owner-readable only. */
export const FEEDBACK_FLUSH_LOG = "feedback-flush.log";

/** Appends to the log in an existing Rerail home only, so a flush outliving its home does not bring it back. */
function logFlush(home: string, line: string): void {
  try {
    appendFileSync(join(home, FEEDBACK_FLUSH_LOG), `${new Date().toISOString()} ${line}\n`, { mode: 0o600 });
  } catch {
    // A log that cannot be written must not stop a send.
  }
}

/** `queued` plus `sending`: the notes approved and not yet accepted. */
function waitingCount(database: RerailDatabase): number {
  return database.select({ count: sql<number>`count(*)` }).from(steerFeedback)
    .where(inArray(steerFeedback.sharing, ["queued", "sending"])).get()?.count ?? 0;
}

/**
 * Send every queued note, oldest first, each to the destination it was approved for. A flush claims each note
 * (`sending`) before it posts, so two flushes started close together never send one note twice; a claim a dead flush
 * left behind is queued again. Accepted notes are `sent`; a note the API refuses as a bad note (400, 413, 422) is
 * `rejected` and never retried; any other answer (offline, a timeout, a server error, an endpoint fault such as 404)
 * stops the run and leaves the rest queued for the next flush, which `rerail daemon start`, the next note or the next
 * answer begins. A `no` while a note is out demotes it to `local`, so the claim's outcome is not written; the request
 * that was already out still lands. The caller has checked the answer is not never.
 */
async function flush(database: RerailDatabase, context: FeedbackContext): Promise<number> {
  const send = context.fetch ?? fetch;
  database.update(steerFeedback).set({ sharing: "queued", claimedAt: null })
    .where(and(eq(steerFeedback.sharing, "sending"), sql`${steerFeedback.claimedAt} < ${Date.now() - STALE_CLAIM_MS}`)).run();
  const queued = database.select().from(steerFeedback).where(eq(steerFeedback.sharing, "queued"))
    .orderBy(asc(steerFeedback.recordedAt), sql`rowid`).all();
  let sent = 0;
  let rejected = 0;
  let stopped: string | undefined;
  for (const note of queued) {
    // Where the person was told a yes sends it, whatever this flush's environment says (`steer_feedback_destination`).
    const url = new URL(note.destination!);
    const claimed = database.update(steerFeedback).set({ sharing: "sending", claimedAt: Date.now() })
      .where(and(eq(steerFeedback.id, note.id), eq(steerFeedback.sharing, "queued"))).run().changes === 1;
    if (!claimed) continue;
    const settle = (sharing: SteerFeedbackSharing, sentAt: number | null = null): void => {
      database.update(steerFeedback).set({ sharing, sentAt, claimedAt: null })
        .where(and(eq(steerFeedback.id, note.id), eq(steerFeedback.sharing, "sending"))).run();
    };
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
    } catch (error) {
      settle("queued");
      stopped = `${url.host} unreachable: ${error instanceof Error ? error.message : String(error)}`;
      break;
    }
    if (status >= 200 && status < 300) {
      settle("sent", Date.now());
      sent += 1;
    } else if (BAD_NOTE_STATUSES.has(status)) {
      settle("rejected");
      rejected += 1;
      logFlush(context.home, `note ${note.id} on ${note.ruleId} refused by ${url.host} as a bad note (${status}); not retried`);
    } else {
      settle("queued");
      stopped = `${url.host} answered ${status}`;
      break;
    }
  }
  const left = waitingCount(database);
  const summary = `Sent ${sent} feedback ${sent === 1 ? "note" : "notes"}${rejected > 0 ? `, ${rejected} refused by the API` : ""}${left > 0 ? `, ${left} still queued` : ""}.`;
  if (stopped !== undefined) logFlush(context.home, `stopped: ${stopped}; ${left} still queued`);
  write(context, stopped === undefined ? summary : `${summary} Stopped: ${stopped}.`);
  return 0;
}
