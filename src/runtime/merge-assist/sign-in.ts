/**
 * `gh`'s sign-in for each repository as `gh` last said it, kept with when in `merge_assist_state`
 * (`github:<repository>`). Every listing of open pull requests reads it afresh and keeps what `gh` said; a status
 * shows the kept reading and runs `gh` only while none is kept (just turned on, or the repository took a new key).
 * Nothing is kept while Merge Assist is off for the repository.
 */
import { z } from "zod";
import { mergeAssistEnabled } from "../../engine/catalog/repository-registry.js";
import { githubRepositoryOf } from "../../engine/merge-assist/branch-git.js";
import { githubReadinessSchema, type GithubReadiness, type ShownReadiness } from "../../engine/merge-assist/model.js";
import type { RerailDatabase } from "../../engine/storage/database.js";
import type { GithubClient } from "./github.js";
import { STATE_KEYS, isoSeconds, readState, writeState } from "./state.js";

/** A reading as kept: what `gh` said, and when (epoch ms). */
const keptSchema = z.object({ at: z.number(), readiness: githubReadinessSchema });

export interface SignInReadsOptions {
  /** The Rerail database, opened on first use. */
  readonly database: () => RerailDatabase;
  /** The person's `gh`, made on first use. */
  readonly github: () => GithubClient;
  readonly now: () => number;
}

/** `gh`'s sign-in per repository: read when a listing needs it, shown as last read. */
export class SignInReads {
  readonly #options: SignInReadsOptions;
  /** Reads running in this process, by repository: every caller meanwhile shares one. */
  readonly #reads = new Map<string, Promise<ShownReadiness>>();

  constructor(options: SignInReadsOptions) {
    this.#options = options;
  }

  /**
   * What a status shows for the repository: `unchecked` while Merge Assist is off for it (no `gh` runs), else the
   * kept reading, read now only while none is kept.
   */
  async shown(repositoryKey: string, enabled: boolean): Promise<ShownReadiness> {
    if (!enabled) {
      const repository = githubRepositoryOf(repositoryKey);
      return { state: "unchecked", login: null, repository: repository === null ? null : `${repository.owner}/${repository.name}`, readAt: null };
    }
    const kept = readState(this.#options.database(), STATE_KEYS.github(repositoryKey), keptSchema);
    if (kept === undefined) return await this.read(repositoryKey);
    return { ...kept.readiness, readAt: isoSeconds(kept.at) };
  }

  /** `gh`'s sign-in read now, or the read already running in this process; kept while Merge Assist is on. */
  read(repositoryKey: string): Promise<ShownReadiness> {
    const repository = githubRepositoryOf(repositoryKey);
    if (repository === null) return Promise.resolve({ state: "not-github", login: null, repository: null, readAt: null });
    let reading = this.#reads.get(repositoryKey);
    if (reading === undefined) {
      reading = this.#options.github().readiness(repository)
        .then(readiness => this.#keep(repositoryKey, readiness))
        .finally(() => this.#reads.delete(repositoryKey));
      this.#reads.set(repositoryKey, reading);
    }
    return reading;
  }

  /** Keep what `gh` said, while Merge Assist is on for the repository: turned off meanwhile, it keeps none. */
  #keep(repositoryKey: string, readiness: GithubReadiness): ShownReadiness {
    const at = this.#options.now();
    this.#options.database().transaction(tx => {
      if (mergeAssistEnabled(tx, repositoryKey)) writeState(tx, STATE_KEYS.github(repositoryKey), { at, readiness }, at);
    }, { behavior: "immediate" });
    return { ...readiness, readAt: isoSeconds(at) };
  }
}
