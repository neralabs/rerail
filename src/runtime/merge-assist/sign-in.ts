/**
 * `gh`'s sign-in for each repository as `gh` last said it, kept in `merge_assist_state` (`github:<repository>`) with
 * when, one reading per `gh` environment (gh-config.ts: a CLI and the daemon may run with different token variables or
 * configuration folders, and so hold different sign-ins), each with the state of `gh`'s hosts file at the time. A
 * status shows the kept reading of its own environment and reads `gh` in its answer only when none is kept or `gh`'s
 * hosts file changed since (a login, logout, switch or refresh), so a person sees that change at the next status read.
 * A reading as old as `gh`'s reuse below is read again when a status asks for it, off the answer's wait: that status
 * shows the kept one and the new one serves the next, or, in a process that answers once (the CLI), that status waits
 * for it, as every status did before. A listing of open pull requests starts from the kept reading under that age
 * while the hosts file has not moved, else reads `gh` first; a scan always reads it afresh. No timer runs: the age is
 * read only when a status or a listing asks. Each reading a status shows says how long until a status reads `gh` again
 * (`readAgainInMs`), counted on this process's clock, so a page can ask then whatever its own clock says. Nothing is kept
 * while Merge Assist is off.
 */
import { z } from "zod";
import { mergeAssistEnabled } from "../../engine/catalog/repository-registry.js";
import { githubRepositoryOf } from "../../engine/merge-assist/branch-git.js";
import { githubReadinessSchema, type GithubReadiness, type ShownReadiness } from "../../engine/merge-assist/model.js";
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
 * The hosts file's part of a stamp whose stat had not answered when `gh` started (`STAT_WAIT_MS`): a status whose stat
 * answers reads `gh` again.
 */
const UNANSWERED = "unanswered";
/**
 * How long a read of `gh` waits for the stat of its hosts file before `gh` starts. A stat answers in far less; one that
 * does not (a drive that stalls) holds `gh` no longer, and the reading is kept `unanswered`.
 */
const STAT_WAIT_MS = 1_000;

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

/** How long a reading stands before a status reads `gh` again: five minutes, or one while `gh` could not tell. */
function reuseOf(kept: Reading): number {
  return kept.readiness.state === "unavailable" ? GITHUB_UNAVAILABLE_MS : GITHUB_READINESS_MS;
}

/** The reading as a status shows it at `now`: what `gh` said, when, and how long until it is read again (0 once due). */
function shownOf(kept: Reading, now: number): ShownReadiness {
  return { ...kept.readiness, readAt: isoSeconds(kept.at), readAgainInMs: Math.max(0, kept.at + reuseOf(kept) - now) };
}

/** What `file` answers if it answers before `by` settles, else undefined: nothing waits on a stat past `by`. */
async function answered(file: Promise<string>, by: Promise<unknown>): Promise<string | undefined> {
  return await Promise.race([file, by.then(() => undefined, () => undefined)]);
}

/** What `file` answers within `ms`, else undefined; the timer ends with the wait. */
async function answeredWithin(file: Promise<string>, ms: number): Promise<string | undefined> {
  let timer: NodeJS.Timeout | undefined;
  try {
    return await answered(file, new Promise(resolve => { timer = setTimeout(resolve, ms); }));
  } finally {
    clearTimeout(timer);
  }
}

/** The readings with this one in place of its environment's, the one written longest ago dropped past the bound. */
function withReading(readings: readonly Reading[], reading: Reading): Reading[] {
  const others = readings.filter(kept => kept.environment !== reading.environment);
  return [...others, reading].slice(-ENVIRONMENTS_KEPT);
}

/** A read of `gh` running in this process, with the stamp of its hosts file it keeps, taken before `gh` started. */
interface RunningRead {
  readonly reading: Promise<ShownReadiness>;
  /** The stamp; undefined until it is taken: `gh` has not started, and reads the file as it is then or later. */
  readonly file: () => string | undefined;
}

/**
 * Whether a caller that found `gh`'s hosts file at `file` (undefined: not compared) takes `running`'s answer: `gh` read
 * the file as that caller found it or later. Not when `running` kept another stamp: the file moved since it started.
 */
function shares(running: RunningRead, file: string | undefined): boolean {
  const stamped = running.file();
  return file === undefined || stamped === undefined || stamped === file;
}

/** `gh`'s sign-in per repository: read when none is kept, it changed or it is due, shown as last read. */
export class SignInReads {
  readonly #options: SignInReadsOptions;
  /**
   * The read running in this process, by repository: callers meanwhile share it unless the hosts file moved past its
   * stamp, and a read for those starts once it ended, so at most one `gh` runs.
   */
  readonly #reads = new Map<string, RunningRead>();

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
    if (file !== undefined && file !== kept.file) return await this.read(repositoryKey, file);
    return await this.#rereadIfDue(repositoryKey, kept);
  }

  /**
   * A listing of open pull requests with the sign-in it starts from. Unless `fresh` (a scan), that is this
   * environment's kept reading while it is younger than its reuse age and `gh`'s hosts file has not moved since, as a
   * listing reused one before; else `gh`'s sign-in read now. The listing with the kept reading starts at once and the
   * stat is waited for no longer than it: a stat that has not answered by then, or a hosts file that moved, reads `gh`
   * and lists again. A reading that lists nothing (not signed in) is therefore read again at each listing.
   */
  async listed<T>(repositoryKey: string, fresh: boolean, list: (readiness: GithubReadiness) => Promise<T>): Promise<T> {
    const stamp = this.#options.github().signInStamp();
    const kept = this.#kept(repositoryKey, stamp.environment);
    if (fresh || kept === undefined || this.#due(kept)) return await list(await this.read(repositoryKey));
    const listing = list(kept.readiness);
    const file = await answered(stamp.file, listing);
    if (file === kept.file) return await listing;
    return await list(await this.read(repositoryKey, file));
  }

  /**
   * `gh`'s sign-in read now, or the read already running in this process unless the stamp it keeps is not `file`, the
   * hosts file as the caller found it; kept while Merge Assist is on.
   */
  read(repositoryKey: string, file?: string): Promise<ShownReadiness> {
    const repository = githubRepositoryOf(repositoryKey);
    if (repository === null) return Promise.resolve({ state: "not-github", login: null, repository: null, readAt: null });
    const running = this.#reads.get(repositoryKey);
    if (running !== undefined && shares(running, file)) return running.reading;
    let stamped: string | undefined;
    const before = running === undefined ? Promise.resolve() : running.reading.then(() => undefined, () => undefined);
    const reading = before
      .then(async () => await this.#readNow(repositoryKey, repository, taken => { stamped = taken; }))
      .finally(() => { if (this.#reads.get(repositoryKey)?.reading === reading) this.#reads.delete(repositoryKey); });
    this.#reads.set(repositoryKey, { reading, file: () => stamped });
    return reading;
  }

  /**
   * What `shown` compares to tell that `gh`'s sign-in changed, as one string: this environment and its hosts file's
   * stamp, read with one stat and no process, so a login, logout, switch or refresh made with `gh` moves it; empty for
   * a repository not on GitHub, for which no status runs `gh`.
   */
  async stamp(repositoryKey: string): Promise<string> {
    if (githubRepositoryOf(repositoryKey) === null) return "";
    const stamp = this.#options.github().signInStamp();
    return `${stamp.environment}\u0000${await stamp.file}`;
  }

  /** Resolves once the reads of `gh` running now have answered and kept what they said. */
  async settled(): Promise<void> {
    await Promise.allSettled([...this.#reads.values()].map(running => running.reading));
  }

  /** This environment's kept reading for the repository; none from a build that kept one reading for all. */
  #kept(repositoryKey: string, environment: string): Reading | undefined {
    const row = readState(this.#options.database(), STATE_KEYS.github(repositoryKey), keptSchema);
    return row?.readings?.find(reading => reading.environment === environment);
  }

  /** Whether the kept reading is as old as `gh`'s reuse: five minutes, or one while `gh` could not tell. */
  #due(kept: Reading): boolean {
    return this.#options.now() - kept.at >= reuseOf(kept);
  }

  /**
   * The kept reading, and when it is as old as `gh`'s reuse, a read of `gh` started for the next status read; one that
   * answers once waits for it.
   */
  async #rereadIfDue(repositoryKey: string, kept: Reading): Promise<ShownReadiness> {
    if (!this.#due(kept)) return shownOf(kept, this.#options.now());
    const reading = this.read(repositoryKey);
    if (this.#options.answersOnce === true) return await reading;
    void reading.catch((error: unknown) => {
      process.stderr.write(`rerail: Merge Assist could not keep gh's sign-in: ${error instanceof Error ? error.message : String(error)}\n`);
    });
    return shownOf(kept, this.#options.now());
  }

  /**
   * Stamp `gh`'s hosts file (`stamped` hears it), then run `gh` and keep what it said with that stamp: `gh` reads the
   * file as the stamp found it or later, so a change made after the stamp moves it past the kept one and the next status
   * reads again. A stat that has not answered within `STAT_WAIT_MS` (a drive that stalls) is kept `unanswered`, never
   * its later answer, which may come after `gh` read the file.
   */
  async #readNow(repositoryKey: string, repository: GithubRepository, stamped: (file: string) => void): Promise<ShownReadiness> {
    const github = this.#options.github();
    const stamp = github.signInStamp();
    const file = (await answeredWithin(stamp.file, STAT_WAIT_MS)) ?? UNANSWERED;
    stamped(file);
    const readiness = await github.readiness(repository);
    const reading: Reading = { at: this.#options.now(), readiness, environment: stamp.environment, file };
    this.#write(repositoryKey, readings => ({ at: reading.at, readiness, readings: withReading(readings, reading) }));
    return shownOf(reading, reading.at);
  }

  /**
   * One change of the repository's row, while Merge Assist is on for it (turned off meanwhile, it keeps none) and the
   * service runs (a stopped one's database may be closing).
   */
  #write(repositoryKey: string, change: (readings: Reading[]) => Kept): void {
    if (this.#options.stopped()) return;
    const key = STATE_KEYS.github(repositoryKey);
    this.#options.database().transaction(tx => {
      if (!mergeAssistEnabled(tx, repositoryKey)) return;
      writeState(tx, key, change(readState(tx, key, keptSchema)?.readings ?? []), this.#options.now());
    }, { behavior: "immediate" });
  }
}
