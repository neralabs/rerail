/**
 * `gh`'s sign-in for each repository as `gh` last said it, kept in `merge_assist_state` (`github:<repository>`) with
 * when, one reading per `gh` environment (gh-config.ts: a CLI and the daemon may run with different token variables or
 * configuration folders, and so hold different sign-ins), each with the state of `gh`'s hosts file at the time. Every
 * listing of open pull requests reads it afresh. A status shows the kept reading of its own environment and reads `gh`
 * in its answer only when none is kept or `gh`'s hosts file changed since (a login, logout, switch or refresh), so a
 * person sees that change at the next status read. A reading as old as `gh`'s reuse below is read again when a status
 * asks for it, off the answer's wait: that status shows the kept one and the new one serves the next, or, in a process
 * that answers once (the CLI), that status waits for it, as every status did before. No timer runs: the age is read
 * only when a status asks. Nothing is kept while Merge Assist is off.
 */
import { z } from "zod";
import { mergeAssistEnabled } from "../../engine/catalog/repository-registry.js";
import { githubRepositoryOf } from "../../engine/merge-assist/branch-git.js";
import { githubReadinessSchema, type ShownReadiness } from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import type { GithubClient, GithubRepository } from "./github.js";
import { STATE_KEYS, isoSeconds, readState, writeState } from "./state.js";

/**
 * How long a reading stands before the next status asks `gh` again: five minutes since Merge Assist began, so a token
 * GitHub refused, or a sign-in no file of `gh`'s told, shows within it.
 */
const GITHUB_READINESS_MS = 5 * 60_000;
/** The same for a `gh` that could not tell: asked again soon, but not at every status read (a page reads every 5 s). */
const GITHUB_UNAVAILABLE_MS = 60_000;
/** How many `gh` environments a repository keeps a reading for; the one written longest ago goes first. */
const ENVIRONMENTS_KEPT = 4;
/**
 * The hosts file's part of a stamp whose stat had not answered when `gh` did, until it answers: a status whose stat
 * answers first reads `gh` again.
 */
const UNANSWERED = "unanswered";

/** A reading as kept: what `gh` said, when (epoch ms), and the stamp of its sign-in as `gh` started. */
const readingSchema = z.object({ at: z.number(), readiness: githubReadinessSchema, environment: z.string(), file: z.string() });
type Reading = z.infer<typeof readingSchema>;
/** The row: the newest reading at its top, as every build before kept it, and one reading per environment. */
const keptSchema = z.object({ at: z.number(), readiness: githubReadinessSchema, readings: z.array(readingSchema).optional() });
type Kept = z.infer<typeof keptSchema>;

export interface SignInReadsOptions {
  /** The Rerail database, opened on first use. */
  readonly database: () => RerailDatabase;
  /** The person's `gh`, made on first use. */
  readonly github: () => GithubClient;
  readonly now: () => number;
  /** The process answers once and ends (the CLI): a status read shows a re-read it starts, since no later read would. */
  readonly answersOnce?: boolean;
  /** Whether the service stopped: a read that answers after keeps nothing, as the database may be closing. */
  readonly stopped: () => boolean;
}

function shownOf(kept: Reading): ShownReadiness {
  return { ...kept.readiness, readAt: isoSeconds(kept.at) };
}

/** What `file` answers if it answers before `by` settles, else undefined: nothing waits on a stat past `by`. */
async function answered(file: Promise<string>, by: Promise<unknown>): Promise<string | undefined> {
  return await Promise.race([file, by.then(() => undefined, () => undefined)]);
}

/** The readings with this one in place of its environment's, the one written longest ago dropped past the bound. */
function withReading(readings: readonly Reading[], reading: Reading): Reading[] {
  const others = readings.filter(kept => kept.environment !== reading.environment);
  return [...others, reading].slice(-ENVIRONMENTS_KEPT);
}

/** The row with the late stat's answer in place of `unanswered`, while that reading is still the one kept; else none. */
function answeredLate(row: Kept | undefined, readings: readonly Reading[], reading: Reading, file: string): Kept | undefined {
  const kept = readings.find(candidate => candidate.environment === reading.environment);
  if (row === undefined || kept?.at !== reading.at || kept.file !== UNANSWERED) return undefined;
  return { ...row, readings: withReading(readings, { ...kept, file }) };
}

/** `gh`'s sign-in per repository: read when a listing needs it or it changed, shown as last read. */
export class SignInReads {
  readonly #options: SignInReadsOptions;
  /** Reads running in this process, by repository: every caller meanwhile shares one, so at most one `gh` runs. */
  readonly #reads = new Map<string, Promise<ShownReadiness>>();

  constructor(options: SignInReadsOptions) {
    this.#options = options;
  }

  /**
   * What a status shows for the repository: `unchecked` while Merge Assist is off for it (no `gh` runs), else this
   * environment's kept reading, read now when none is kept or `gh`'s hosts file changed since. The file's stat is
   * waited for no longer than `answerBy`, the status's own work: one that has not answered by then leaves the kept
   * reading shown with its time.
   */
  async shown(repositoryKey: string, enabled: boolean, answerBy: Promise<unknown>): Promise<ShownReadiness> {
    if (!enabled) {
      const repository = githubRepositoryOf(repositoryKey);
      return { state: "unchecked", login: null, repository: repository === null ? null : `${repository.owner}/${repository.name}`, readAt: null };
    }
    const stamp = this.#options.github().signInStamp();
    const kept = this.#kept(repositoryKey, stamp.environment);
    if (kept === undefined) return await this.read(repositoryKey);
    const file = await answered(stamp.file, answerBy);
    if (file !== undefined && file !== kept.file) return await this.read(repositoryKey);
    return await this.#rereadIfDue(repositoryKey, kept);
  }

  /** `gh`'s sign-in read now, or the read already running in this process; kept while Merge Assist is on. */
  read(repositoryKey: string): Promise<ShownReadiness> {
    const repository = githubRepositoryOf(repositoryKey);
    if (repository === null) return Promise.resolve({ state: "not-github", login: null, repository: null, readAt: null });
    let reading = this.#reads.get(repositoryKey);
    if (reading === undefined) {
      reading = this.#readNow(repositoryKey, repository).finally(() => this.#reads.delete(repositoryKey));
      this.#reads.set(repositoryKey, reading);
    }
    return reading;
  }

  /** Resolves once the reads of `gh` running now have answered and kept what they said. */
  async settled(): Promise<void> {
    await Promise.allSettled([...this.#reads.values()]);
  }

  /** This environment's kept reading for the repository; none from a build that kept one reading for all. */
  #kept(repositoryKey: string, environment: string): Reading | undefined {
    const row = readState(this.#options.database(), STATE_KEYS.github(repositoryKey), keptSchema);
    return row?.readings?.find(reading => reading.environment === environment);
  }

  /**
   * The kept reading, and when it is as old as `gh`'s reuse, a read of `gh` started for the next status read; one that
   * answers once waits for it.
   */
  async #rereadIfDue(repositoryKey: string, kept: Reading): Promise<ShownReadiness> {
    const reuse = kept.readiness.state === "unavailable" ? GITHUB_UNAVAILABLE_MS : GITHUB_READINESS_MS;
    if (this.#options.now() - kept.at < reuse) return shownOf(kept);
    const reading = this.read(repositoryKey);
    if (this.#options.answersOnce === true) return await reading;
    void reading.catch((error: unknown) => {
      process.stderr.write(`rerail: Merge Assist could not keep gh's sign-in: ${error instanceof Error ? error.message : String(error)}\n`);
    });
    return shownOf(kept);
  }

  /**
   * Run `gh` and keep what it said with the stamp taken as it started. A stat that has not answered when `gh` did (a
   * `gh` that answers with no process, a drive that stalls) is kept `unanswered`, and its answer replaces that when it
   * comes, so a slow stat costs no read of `gh` at the next status.
   */
  async #readNow(repositoryKey: string, repository: GithubRepository): Promise<ShownReadiness> {
    const github = this.#options.github();
    // Stamped as gh starts: a change while it runs moves the stamp past the kept one, so the next status reads again.
    const stamp = github.signInStamp();
    const said = github.readiness(repository);
    const [readiness, file] = await Promise.all([said, answered(stamp.file, said)]);
    const reading: Reading = { at: this.#options.now(), readiness, environment: stamp.environment, file: file ?? UNANSWERED };
    this.#write(repositoryKey, readings => ({ at: reading.at, readiness, readings: withReading(readings, reading) }));
    if (file === undefined) this.#keepLate(repositoryKey, reading, stamp.file);
    return shownOf(reading);
  }

  /** The late stat's answer in place of `unanswered`; one that cannot be kept leaves the next status to read `gh` again. */
  #keepLate(repositoryKey: string, reading: Reading, file: Promise<string>): void {
    file.then(late => this.#write(repositoryKey, (readings, row) => answeredLate(row, readings, reading, late))).catch((error: unknown) => {
      process.stderr.write(`rerail: Merge Assist could not keep gh's sign-in: ${error instanceof Error ? error.message : String(error)}\n`);
    });
  }

  /**
   * One change of the repository's row, while Merge Assist is on for it (turned off meanwhile, it keeps none) and the
   * service runs (a stopped one's database may be closing); a change that returns nothing writes nothing.
   */
  #write(repositoryKey: string, change: (readings: Reading[], row: Kept | undefined) => Kept | undefined): void {
    if (this.#options.stopped()) return;
    const key = STATE_KEYS.github(repositoryKey);
    this.#options.database().transaction(tx => {
      if (!mergeAssistEnabled(tx, repositoryKey)) return;
      const row = readState(tx, key, keptSchema);
      const next = change(row?.readings ?? [], row);
      if (next !== undefined) writeState(tx, key, next, this.#options.now());
    }, { behavior: "immediate" });
  }
}
