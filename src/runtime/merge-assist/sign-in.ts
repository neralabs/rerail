/**
 * `gh`'s sign-in for each repository as `gh` last said it, kept in `merge_assist_state` (`github:<repository>`) with
 * when, and with the stamp of `gh`'s sign-in file at the time (gh-config.ts). Every listing of open pull requests reads
 * it afresh. A status shows the kept reading; it reads `gh` in its answer only when none is kept or `gh`'s sign-in
 * changed since (a login, logout, switch or refresh, a token variable set or cleared), so a person sees that change at
 * the next status read. A reading that leaves pull requests unread is read again by the first status read past its
 * pace, off the answer's wait: that read shows the kept one and the new one serves the next read, or, in a process that
 * answers once (the CLI), that read waits for it as before. Nothing is kept while Merge Assist is off.
 */
import { z } from "zod";
import { mergeAssistEnabled } from "../../engine/catalog/repository-registry.js";
import { githubRepositoryOf } from "../../engine/merge-assist/branch-git.js";
import { githubReadinessSchema, type GithubReadiness, type GithubState, type ShownReadiness } from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import type { GithubClient } from "./github.js";
import { STATE_KEYS, isoSeconds, readState, writeState } from "./state.js";

/**
 * How old a reading that leaves pull requests unread is before a status read starts reading it again. No file tells
 * that `gh` was installed, that the network came back or that `gh` answers again. Five minutes as since Merge Assist
 * began; a minute while `gh` could not tell, so it is asked again soon but not at every status read: a page reads the
 * status every 5 s, and each `gh auth status` with a token GitHub refuses spends a request.
 */
const REREAD_AFTER_MS: Partial<Record<GithubState, number>> = { "signed-out": 5 * 60_000, missing: 5 * 60_000, unavailable: 60_000 };

/** A reading as kept: what `gh` said, when (epoch ms), and the stamp of its sign-in file just before (absent from an older build). */
const keptSchema = z.object({ at: z.number(), readiness: githubReadinessSchema, stamp: z.string().optional() });
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

function shownOf(kept: Kept): ShownReadiness {
  return { ...kept.readiness, readAt: isoSeconds(kept.at) };
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
   * What a status shows for the repository: `unchecked` while Merge Assist is off for it (no `gh` runs), else the kept
   * reading, read now when none is kept or `gh`'s sign-in changed since.
   */
  async shown(repositoryKey: string, enabled: boolean): Promise<ShownReadiness> {
    if (!enabled) {
      const repository = githubRepositoryOf(repositoryKey);
      return { state: "unchecked", login: null, repository: repository === null ? null : `${repository.owner}/${repository.name}`, readAt: null };
    }
    const kept = readState(this.#options.database(), STATE_KEYS.github(repositoryKey), keptSchema);
    if (kept === undefined || kept.stamp !== this.#options.github().signInStamp()) return await this.read(repositoryKey);
    return await this.#rereadIfDue(repositoryKey, kept);
  }

  /** `gh`'s sign-in read now, or the read already running in this process; kept while Merge Assist is on. */
  read(repositoryKey: string): Promise<ShownReadiness> {
    const repository = githubRepositoryOf(repositoryKey);
    if (repository === null) return Promise.resolve({ state: "not-github", login: null, repository: null, readAt: null });
    let reading = this.#reads.get(repositoryKey);
    if (reading === undefined) {
      const github = this.#options.github();
      // Stamped before gh runs: a change while it runs moves the stamp past the kept one, so the next status reads again.
      const stamp = github.signInStamp();
      reading = github.readiness(repository)
        .then(readiness => this.#keep(repositoryKey, readiness, stamp))
        .finally(() => this.#reads.delete(repositoryKey));
      this.#reads.set(repositoryKey, reading);
    }
    return reading;
  }

  /** Resolves once the reads of `gh` running now have answered and kept what they said. */
  async settled(): Promise<void> {
    await Promise.allSettled([...this.#reads.values()]);
  }

  /**
   * The kept reading, and when it leaves pull requests unread and is past its pace, a read of `gh` started for the next
   * status read; one that answers once waits for it.
   */
  async #rereadIfDue(repositoryKey: string, kept: Kept): Promise<ShownReadiness> {
    const pace = REREAD_AFTER_MS[kept.readiness.state];
    if (pace === undefined || this.#options.now() - kept.at < pace) return shownOf(kept);
    const reading = this.read(repositoryKey);
    if (this.#options.answersOnce === true) return await reading;
    void reading.catch((error: unknown) => {
      process.stderr.write(`rerail: Merge Assist could not keep gh's sign-in: ${error instanceof Error ? error.message : String(error)}\n`);
    });
    return shownOf(kept);
  }

  /**
   * Keep what `gh` said, while Merge Assist is on for the repository: turned off meanwhile, it keeps none, and neither
   * does a service that stopped.
   */
  #keep(repositoryKey: string, readiness: GithubReadiness, stamp: string): ShownReadiness {
    const at = this.#options.now();
    if (this.#options.stopped()) return { ...readiness, readAt: isoSeconds(at) };
    this.#options.database().transaction(tx => {
      if (mergeAssistEnabled(tx, repositoryKey)) writeState(tx, STATE_KEYS.github(repositoryKey), { at, readiness, stamp }, at);
    }, { behavior: "immediate" });
    return { ...readiness, readAt: isoSeconds(at) };
  }
}
