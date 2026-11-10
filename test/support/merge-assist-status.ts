/**
 * A Merge Assist status as the service reports it, and a recording service that answers with it, for the layers that
 * only call the service (API, CLI).
 */
import type { CandidateState, Conflict, ConflictAction, Finding, FindingAction, MergeAssistStatus } from "../../src/engine/index.js";
import type { MergeAssistActor, MergeAssistError, MergeAssistService } from "../../src/runtime/index.js";

export function sampleFinding(overrides: Partial<Finding> & Pick<Finding, "id" | "tier" | "status">): Finding {
  return {
    candidate: {
      id: "candidate:retries-in-transport-3f9a1c",
      decision: "Retries live in the transport layer; callers don't wrap requests in retry loops.",
      scope: ["src/api/**"],
      level: "code",
      author: "jbarnes",
      createdAt: "2026-11-03T14:02:11Z",
    },
    peer: {
      kind: "pull-request",
      ref: "jbarnes/session-retry",
      commit: "0123456789abcdef0123456789abcdef01234567",
      number: 12,
      title: "Session retry",
      url: "https://github.com/acme/app/pull/12",
      author: "jbarnes",
      trusted: true,
    },
    reason: "Jeremy has write access and your branch's changes pass its rule.",
    evidence: { violations: [], conflicts: [], mine: null, judgment: null },
    message: null,
    held: null,
    updatedAt: "2026-11-03T14:05:00Z",
    ...overrides,
  };
}

export function sampleConflict(overrides: Partial<Conflict> = {}): Conflict {
  return {
    id: "conflict:overlap:0123456789abcdef01234567:89abcdef",
    kind: "overlap",
    status: "open",
    level: "ambient",
    shortId: "a1b2c3",
    signals: null,
    score: 120,
    reasons: ["local code: ×1", "+40 lines on theirs in 2 h", "3 callers", "pull request open: about 24 h to merge", "no verdict yet: even odds either side loses"],
    queued: false,
    queuedFor: null,
    snooze: null,
    question: "overlap:0123456789abcdef01234567",
    title: "model.rs · fn compile",
    mine: {
      label: "This branch", ref: "tbelanger/export-api", sha: "1".repeat(40), author: "Tim Belanger", tip: null, url: null,
      summary: "edits native/src/model.rs", location: "native/src/model.rs:469", hunk: "@@ -469 +469 @@\n-matchers: Vec<GrammarMatcher>,\n+matchers: Vec<GrammarMatcher>, // cloned", decisionId: null,
    },
    theirs: {
      label: "PR #398", ref: "refs/pull/398/head", sha: "2".repeat(40), author: "jbarnes", tip: { at: "2026-11-03T14:00:00Z", pushed: true }, url: "https://github.com/acme/app/pull/398",
      summary: "Share compiled matchers", location: "native/src/model.rs:470", hunk: "@@ -470 +470 @@\n+matchers: Arc<[GrammarMatcher]>,", decisionId: null,
    },
    evidence: ["Both sides change native/src/model.rs within 3 lines of each other."],
    detectedAt: "2026-11-03T14:05:00Z",
    resolution: null,
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
    experimental: { decisionLedger: false },
    readiness: {
      judge: { available: true, runner: "claude", problem: null },
      github: { state: "signed-in", login: "tbelanger", repository: "acme/app" },
      candidatesIgnored: false,
      rulesActive: true,
    },
    budget: { used: 3, limit: 40, windowMinutes: 60 },
    scanning: false,
    lastScan: { at: "2026-11-03T14:05:00Z", peers: { branches: 2, pullRequests: 1 }, problems: [] },
    own: [],
    findings: [],
    conflicts: [],
    decisionLog: { questions: [], problems: [] },
    ...overrides,
  };
}

export type ServiceCall =
  | { readonly method: "status" | "scan"; readonly checkout: string }
  | { readonly method: "setEnabled"; readonly checkout: string; readonly enabled: boolean }
  | { readonly method: "setCandidateState"; readonly checkout: string; readonly candidateId: string; readonly state: CandidateState }
  | { readonly method: "resolveFinding"; readonly checkout: string; readonly findingId: string; readonly action: FindingAction; readonly actor: MergeAssistActor }
  | {
    readonly method: "resolveConflict";
    readonly checkout: string;
    readonly conflictId: string;
    readonly action: ConflictAction;
    readonly text: string | undefined;
    readonly actor: MergeAssistActor;
  };

/** Records every call and answers with the status it was given, or throws `failure` when one is set. */
export class RecordingMergeAssistService implements MergeAssistService {
  readonly calls: ServiceCall[] = [];
  failure: MergeAssistError | undefined;
  readonly #status: (checkout: string) => MergeAssistStatus;

  constructor(status: (checkout: string) => MergeAssistStatus = checkout => sampleStatus(checkout)) {
    this.#status = status;
  }

  #answer(call: ServiceCall): Promise<MergeAssistStatus> {
    this.calls.push(call);
    return this.failure === undefined ? Promise.resolve(this.#status(call.checkout)) : Promise.reject(this.failure);
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

  setCandidateState(checkout: string, candidateId: string, state: CandidateState): Promise<MergeAssistStatus> {
    return this.#answer({ method: "setCandidateState", checkout, candidateId, state });
  }

  resolveFinding(checkout: string, findingId: string, action: FindingAction, actor: MergeAssistActor = "person"): Promise<MergeAssistStatus> {
    return this.#answer({ method: "resolveFinding", checkout, findingId, action, actor });
  }

  resolveConflict(checkout: string, conflictId: string, action: ConflictAction, text?: string, actor: MergeAssistActor = "person"): Promise<MergeAssistStatus> {
    return this.#answer({ method: "resolveConflict", checkout, conflictId, action, text, actor });
  }
}
