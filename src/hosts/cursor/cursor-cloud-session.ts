import { dirname } from "node:path";
import { z } from "zod/v4";
import { answeredFacts, type AdapterEvent, type AdapterSession, type EventAnswerFacts, type RerailEventLoop } from "../../engine/adapter-contract.js";
import { steers } from "../../engine/catalog/rule-mode.js";
import { readLedgerFile, writeLedgerFile } from "../../engine/storage/ledger-paths.js";
import { persistPending } from "../../harness/hook-adapter.js";
import { linkedByAnswer, presentedSessionStartNote } from "../../presentation/session-presentation.js";

/**
 * What a Cursor cloud session's hooks record of its start, beside its held steers' file (`<pending>.cloud-start`, read
 * and written under the session's lock): that the file is there says its start was sent, and `owedNote` is the
 * session-start note the model has not been given yet. That is the note's text as the daemon's answer to the start
 * gave it, or null where the start was left for a daemon that was not reading and so had no answer: the hook that
 * carries the note then builds it from its own answer. Once given, the file holds no note.
 */
const cloudStartSchema = z.object({ owedNote: z.string().nullable().optional() }).strict();
type CloudStart = z.infer<typeof cloudStartSchema>;

const CLOUD_START_SUFFIX = ".cloud-start";

/**
 * The session's recorded start, or undefined when none was sent. A file that does not read as one says the start was
 * sent and owes nothing: sending the start again would do no harm, but the note would reach the model twice.
 */
async function readCloudStart(pending: string): Promise<CloudStart | undefined> {
  const text = await readLedgerFile(`${pending}${CLOUD_START_SUFFIX}`, dirname(pending));
  if (text === undefined) return undefined;
  try {
    return cloudStartSchema.parse(JSON.parse(text));
  } catch {
    return {};
  }
}

async function writeCloudStart(pending: string, start: CloudStart): Promise<void> {
  await writeLedgerFile(`${pending}${CLOUD_START_SUFFIX}`, dirname(pending), JSON.stringify(start));
}

/**
 * On a Cursor cloud agent's VM (`isCursorCloud`), where Cursor fires no `sessionStart`, records the start of the
 * agent's own session at its first hook. Rerail does not start or open a Cursor session: Cursor's agent started it,
 * and this sends the daemon the same start event Cursor's `sessionStart` hook sends locally (`cursorHookEvent`), once,
 * before that first hook's own event. The note the start's answer carries is kept in the session's sidecar until the
 * first `postToolUse` the daemon answers, the first hook whose answer Cursor lets carry it to the model
 * (`owedCloudNote`). Steers the start's answer brings are held for the next hook that can carry them.
 *
 * A later hook finds the sidecar and sends no start, so the daemon is never sent one for a session it already holds
 * open from an earlier hook; a session the idle sweep closed is opened again by its next event, as everywhere. The
 * caller keeps the session's lock through this (`SessionLock.kept`), so a parallel first hook waits for the record
 * rather than sending a second start. While the daemon is not reading (`RerailEventLoop.untaken`) the start is left
 * for it as the hook's own event is, the hook holds no lock, and the start is recorded once that event is left
 * (`recordLeftCloudStart`): returns true then.
 */
export async function cloudSessionStart(descriptor: AdapterSession, eventLoop: RerailEventLoop, pending: string): Promise<boolean> {
  if (await readCloudStart(pending) !== undefined) return false;
  // One id for every hook's start of the session, so a start two parallel hooks both left is recorded once.
  const start: AdapterEvent = { kind: "session-start", eventId: `${descriptor.sessionId}:session-start`, session: descriptor };
  const deliveries = await eventLoop.ingest(start);
  if (eventLoop.untaken === true) return true;
  const answered = answeredFacts(eventLoop);
  // A steer held for a later event keeps this answer's links, so that event's answer need carry none.
  await persistPending(pending, linkedByAnswer(deliveries.filter(delivery => steers(delivery.requested)), answered));
  await writeCloudStart(pending, { owedNote: await presentedSessionStartNote(descriptor, answered) });
  return false;
}

/**
 * Records a start `cloudSessionStart` left for a daemon that was not reading, owing a note its answer never gave, unless
 * another hook of the session recorded one meanwhile. The caller holds the session's lock.
 */
export async function recordLeftCloudStart(pending: string): Promise<void> {
  if (await readCloudStart(pending) === undefined) await writeCloudStart(pending, { owedNote: null });
}

/**
 * The session-start note the Cursor cloud session still owes the model, taken for this `postToolUse`'s answer and
 * marked given, so no later hook repeats it; undefined once given, or where no start was recorded. A start left
 * untaken has no text: the note is built from this hook's answer (`answered`), which reads what the daemon did not say.
 */
export async function owedCloudNote(pending: string, descriptor: AdapterSession, answered: EventAnswerFacts): Promise<string | undefined> {
  const start = await readCloudStart(pending);
  if (start?.owedNote === undefined) return undefined;
  const note = start.owedNote ?? await presentedSessionStartNote(descriptor, answered);
  await writeCloudStart(pending, {});
  return note;
}

/**
 * Records that a Cursor cloud session's own `sessionStart`, which the daemon answered, gave the model its note, so its
 * later hooks send no start and owe none: Cursor fires none in its cloud agents, but a VM that does fire one is served
 * as a local session is.
 */
export async function recordCloudNoteGiven(pending: string): Promise<void> {
  await writeCloudStart(pending, {});
}
