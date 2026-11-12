/**
 * A Merge Assist status as the service reports it, and a recording service that answers with it, for the layers that
 * only call the service (API, CLI).
 */
import { DEFAULT_MERGE_ASSIST_SETTINGS } from "../../../src/engine/index.js";
import type { Conflict, MergeAssistSettings, MergeAssistStatus } from "../../../src/engine/index.js";
import type { MergeAssistActor, MergeAssistError, MergeAssistService, MergeAssistSettingsChange } from "../../../src/runtime/index.js";

export function sampleConflict(overrides: Partial<Conflict> = {}): Conflict {
  return {
    id: "conflict:overlap:0123456789abcdef01234567:89abcdef",
    kind: "overlap",
    status: "open",
    level: "ambient",
    shortId: "a1b2c3",
    signals: null,
    score: 120,
    reasons: ["local code: ×1", "+40 lines on theirs in 2 h", "3 callers", "pull request open: about 24 h to merge", "even odds either side loses"],
    queued: false,
    queuedFor: null,
    question: "overlap:0123456789abcdef01234567",
    title: "model.rs · fn compile",
    mine: {
      label: "This branch", ref: "tbelanger/export-api", sha: "1".repeat(40), author: "Tim Belanger", tip: null, url: null,
      summary: "edits native/src/model.rs", location: "native/src/model.rs:469", hunk: "@@ -469 +469 @@\n-matchers: Vec<GrammarMatcher>,\n+matchers: Vec<GrammarMatcher>, // cloned",
    },
    theirs: {
      label: "PR #398", ref: "refs/pull/398/head", sha: "2".repeat(40), author: "jbarnes", tip: { at: "2026-11-03T14:00:00Z", pushed: true }, url: "https://github.com/acme/app/pull/398",
      summary: "Share compiled matchers", location: "native/src/model.rs:470", hunk: "@@ -470 +470 @@\n+matchers: Arc<[GrammarMatcher]>,",
    },
    evidence: ["Both sides change native/src/model.rs within 3 lines of each other."],
    detectedAt: "2026-11-03T14:05:00Z",
    dismissal: null,
    summary: null,
    hook: null,
    ...overrides,
  };
}

export function sampleStatus(checkout: string, overrides: Partial<MergeAssistStatus> = {}): MergeAssistStatus {
  return {
    repositoryKey: "remote:github.com/acme/app",
    checkout,
    branch: "tbelanger/export-api",
    baseBranch: "main",
    enabled: true,
    settings: DEFAULT_MERGE_ASSIST_SETTINGS,
    readiness: {
      judge: { available: true, runner: "claude", problem: null },
      github: { state: "signed-in", login: "tbelanger", repository: "acme/app" },
    },
    budget: { used: 3, limit: 40, windowMinutes: 60 },
    scanning: false,
    lastScan: { at: "2026-11-03T14:05:00Z", peers: { branches: 2, pullRequests: 1 }, problems: [] },
    conflicts: [],
    ...overrides,
  };
}

export type ServiceCall =
  | { readonly method: "status" | "scan" | "settings"; readonly checkout: string }
  | { readonly method: "setEnabled"; readonly checkout: string; readonly enabled: boolean }
  | { readonly method: "updateSettings"; readonly checkout: string; readonly change: MergeAssistSettingsChange }
  | { readonly method: "dismissConflict"; readonly checkout: string; readonly conflictId: string; readonly actor: MergeAssistActor };

/**
 * Records every call and answers with the status it was given, or throws `failure` when one is set. Its settings are
 * the status's, changed by each {@link updateSettings} for the statuses that follow.
 */
export class RecordingMergeAssistService implements MergeAssistService {
  readonly calls: ServiceCall[] = [];
  failure: MergeAssistError | undefined;
  readonly #status: (checkout: string) => MergeAssistStatus;
  #settings: MergeAssistSettings | undefined;

  constructor(status: (checkout: string) => MergeAssistStatus = checkout => sampleStatus(checkout)) {
    this.#status = status;
  }

  #current(checkout: string): MergeAssistStatus {
    const status = this.#status(checkout);
    return this.#settings === undefined ? status : { ...status, settings: this.#settings };
  }

  #answer(call: ServiceCall): Promise<MergeAssistStatus> {
    this.calls.push(call);
    return this.failure === undefined ? Promise.resolve(this.#current(call.checkout)) : Promise.reject(this.failure);
  }

  settings(checkout: string): Promise<MergeAssistSettings> {
    return this.#answer({ method: "settings", checkout }).then(status => status.settings);
  }

  updateSettings(checkout: string, change: MergeAssistSettingsChange): Promise<MergeAssistStatus> {
    if (this.failure === undefined) this.#settings = { ...this.#current(checkout).settings, ...change };
    return this.#answer({ method: "updateSettings", checkout, change });
  }

  status(checkout: string): Promise<MergeAssistStatus> {
    return this.#answer({ method: "status", checkout });
  }

  scan(checkout: string): Promise<MergeAssistStatus> {
    return this.#answer({ method: "scan", checkout });
  }

  setEnabled(checkout: string, enabled: boolean): Promise<MergeAssistStatus> {
    return this.#answer({ method: "setEnabled", checkout, enabled });
  }

  dismissConflict(checkout: string, conflictId: string, actor: MergeAssistActor = "person"): Promise<MergeAssistStatus> {
    return this.#answer({ method: "dismissConflict", checkout, conflictId, actor });
  }
}
