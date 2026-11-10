/**
 * Typings for the plain-ES-module UI files the tests import. `tsconfig` has no `allowJs`, so a
 * relative `.js` import is an implicit `any` under `strict`; wildcard ambient modules are the
 * one form TypeScript matches against a relative specifier. Only the surface the tests use.
 */
declare module "*/ui/sessions-model.js" {
  export interface Correction {
    key: string;
    finding: { path?: string | null; [k: string]: unknown };
    rule: unknown;
  }
  export interface CallView {
    key: string;
    verb: string;
    label: string;
    level: string;
    toolName: string;
    input: unknown;
    target: string;
    title: string;
    kind: string | null;
    callSeq: number;
    callTime: number;
    resSeq: number | null;
    resTime: number | null;
    output: unknown;
    failed: boolean;
    dur: number | null;
    lines?: number;
    correction: Correction | null;
    raw: unknown[];
  }
  export interface TurnView {
    key: string;
    ti: number;
    u: string | null;
    a: string | null;
    uSeq: number | null;
    aSeq: number | null;
    uTime: number | null;
    aTime: number | null;
    tools: CallView[];
    orphans: Correction[];
    ended: boolean;
  }
  export function sessionTurns(events: readonly unknown[]): TurnView[];
  export function placeCorrections(turns: TurnView[], corrections: readonly Correction[]): TurnView[];
  export function resumeCommand(s: { host: string; cwd: string; sessionId: string }): { name: string; cmd: string | null; how?: string };
  export function droppedEvents(s: { sessionId: string }): number;
  export function uncheckedBoundaries(s: { sessionId: string }): number;
  export function codeRulesPendingBoundaries(s: { sessionId: string }): number;
  export function sessionCorrections(sessionId: string): Correction[];
  export function sessionViolationTotal(sessionId: string): number | null;
  export function loadedFrom(sessionId: string): number;
  export const STALE_MIN: number;
  export interface SessionRow { sessionId: string; startedAt: number; endedAt: number | null; lastEventAt?: number | null; violationCount?: number; steeredViolations?: number }
  export function lastEventAt(s: SessionRow): number;
  export function openState(s: SessionRow, now?: number): "live" | "stale" | null;
  export function violationCounts(s: SessionRow): { logged: number; steered: number };
}

declare module "*/ui/poll.js" {
  export function titleFromPrompts(prompts: readonly string[]): string | null;
  /** What each poll loads for a repository in scope, by need name (`NEEDS` in poll.js). */
  export function setNeeds(needs: () => Iterable<string>): void;
  export function attachedPacksOf(
    entries: ReadonlyArray<{ folder: string; source: { kind: string; id: string } }>,
    unavailable: ReadonlyArray<{ sourceKind: string; sourceId: string; folder: string; reason?: string }>,
  ): Map<string, string[]>;
  export interface Scope { target: { repository?: string; checkout?: string }; token: number }
  export function captureScope(): Scope;
  export function inScope(scope: Scope): boolean;
  export function sendInScope(scope: Scope, method: string, path: string, body: object): Promise<boolean>;
  /** `body` may be a promise of the body; `null` sends nothing. */
  export function queueInScope(scope: Scope, method: string, path: string, body: object | Promise<object | null> | null): Promise<boolean>;
  export function recommendationsFor(scope: Scope): Promise<Array<{ packId: string; folder: string }>>;
  export function saveFollowGitignore(follow: boolean, page: { toast(message: string): void; invalidate(): void }): Promise<void>;
  export function writeMergeAssist(scope: Scope, method: string, path: string, body: object): Promise<import("../../src/engine/index.js").MergeAssistStatus | null>;
  export function changeScope(scope: { repository?: string; checkout?: string } | null): void;
  export function refresh(): Promise<void>;
  export function refreshNow(): Promise<void>;
  export function loadMoreFindings(): Promise<boolean>;
  export function loadMoreSessions(): Promise<boolean>;
  export function loadSessionFindings(id: string): Promise<boolean>;
  export function loadSession(id: string): Promise<boolean>;
  export function loadEarlier(id: string): Promise<boolean>;
  export type Suppressions = "honour" | "ignore";
  /** One folder's own on, mode and suppressions, what decided its suppressions, and what added the rule there. */
  export interface FolderState { folder: string; on: boolean; mode: string; suppressions: Suppressions; suppressionsSource: object; source: { kind: string; id: string } }
  /** A rule as the page stores it from `GET /api/repository/rules`; only the fields the tests read. */
  export interface StoredRule {
    rule: { id: string; mode: string };
    source: { kind: string; id: string };
    enabled: boolean;
    folders: string[];
    folderStates: FolderState[];
    mixed: boolean;
    suppressions: Suppressions;
    suppressionsSource: object;
    config: Record<string, unknown> | null;
    configSource: Record<string, object>;
    /** Why the configured options don't apply; the rule then runs nowhere. */
    configError: string | null;
  }
  export function toStoredRules(entries: readonly object[]): Map<string, StoredRule>;
}

declare module "*/ui/derive.js" {
  export function violationOutcome(finding: { mode?: string }): "logged" | "steered";
  export function isRunning(stored: { enabled: boolean; configError?: string | null }): boolean;
  export function packSuppressions(rules: readonly object[], packId: string, owner: "repository" | "checkout", checkoutPath?: string): { text: string; next: "honour" | "ignore" };
  export function inPack(rule: object, packId: string): boolean;
}

declare module "*/ui/state.js" {
  export function sinceFor(range?: number): number;
  /** A cursor-paged list as the page holds it. */
  export interface PagedList<Row> { key: string | null; owner: string | null; rows: Row[]; total: number; next: string | null; pages: number }
  export function pagedList<Row = never>(key?: string | null, owner?: string | null): PagedList<Row>;
  export interface SessionDetail { head: object | null; gaps: object | null; events: Array<{ sequence: number; observedAt: number; [k: string]: unknown }>; nextAfter: number; loaded: boolean; earlier: boolean; loading: Promise<unknown> | null }
  export type Violation = { id: string; auditId: string; ruleId: string; sessionId: string; mode?: string; at: number; host?: string | null };
  export const data: {
    rules: Map<string, unknown>;
    findings: PagedList<Violation>;
    ruleFindings: PagedList<Violation>;
    sessions: PagedList<{ sessionId: string; startedAt: number; violationCount: number; eventCount?: number; [k: string]: unknown }>;
    sessionFindings: PagedList<Violation>;
    activity: Record<string, unknown>;
    activityAll: Record<string, unknown>;
    activitySessions: { started: number; withViolations: number } | null;
    activityAllSessions: { started: number; withViolations: number } | null;
    packs: unknown[];
    attachedPacks: Map<string, string[]>;
    repository: { setUp: boolean; repositoryKey?: string; name?: string } | null;
    checkout: { checkoutPath: string; folder: string; state: string } | null;
    recommendations: Array<{ packId: string; folder?: string }> | null;
    followGitignore: boolean | null;
    sessionDetail: Map<string, SessionDetail>;
    missing: boolean;
    disconnected: boolean;
    /** `GET /api/merge-assist` for the scoped checkout (`MergeAssistStatus`). */
    mergeAssist: import("../../src/engine/index.js").MergeAssistStatus | null;
    mergeAssistError: string | null;
  };
  export function setScope(scope: { repository?: string; checkout?: string } | null): void;
  export const ui: {
    filter: string; query: string; range: number; actRule: string | null; histOutcome: "all" | "steered" | "logged"; view: string; settingUp: boolean;
    selectedSession: string | null; panel: object | null; mergePending: Map<string, string>; mergeOpen: Set<string>; mergeSwitching: boolean; mergeScanning: boolean;
    mergeTab: "conflicts" | "decisions" | "log" | null; mergeLogStatus: import("../../src/engine/index.js").LedgerStatus | null; mergeDraft: { id: string; action: "mine" | "theirs" | "new"; text: string } | null;
  };
  export function keptPacks(): readonly string[];
  export function isAudited(id: string): boolean;
  export function runningCount(): number;
  export function matches(stored: object): boolean;
  export interface RuleGroup { name: string; all: Array<{ rule: { id: string } }> }
  export function ruleSections(): Array<{ key: string; kind: "mine" | "pack" | "legacy"; groups: RuleGroup[] }>;
}

declare module "*/ui/attachments.js" {
  /** The page's rule pack writes; `toast` and `invalidate` are the page's. */
  export function attachmentWrites(page: { toast: (message: string) => void; invalidate: () => void }): {
    setUpRecommended(): Promise<void>;
    keepPack(id: string, keep: boolean, message: string, failed?: string): Promise<void> | undefined;
  };
}

declare module "*/ui/format.js" {
  /** A failure toast: what did not happen, then the error as a sentence. */
  export function failed(what: string, error: unknown): string;
}

declare module "*/ui/settings-writes.js" {
  /** One settings write: the folder, the targets and the values it sets (`on: null` clears). */
  export interface SettingPut { folder: string; targets: Array<{ kind: "rule" | "pack"; id: string }>; setting: { on?: boolean | null; suppressions?: "honour" | "ignore" } }
  /** The writes the whole-pack switch sends for pack `packId`: per folder, the pack's on, then its rules' own on cleared. */
  export function packSwitchWrites(packId: string, enabled: boolean): SettingPut[];
  /** The page's rule setting writes; `toast`, `invalidate` and `owner` are the page's. */
  export function settingWrites(page: { toast: (message: string) => void; invalidate: () => void; owner: () => "repository" | "checkout" }): {
    setEnabled(ids: readonly string[], enabled: boolean, label: string, failed?: string): Promise<void>;
    setPackEnabled(packId: string, enabled: boolean, label: string, failed?: string): Promise<void>;
    setPackSuppressions(packId: string, suppressions: "honour" | "ignore", label: string, failed?: string): Promise<void>;
  };
}

declare module "*/ui/pack-model.js" {
  type Stored = { rule: { id: string }; enabled: boolean; addedAt?: number | null };
  type Pack = { id: string; name: string; category: string; covers?: string; rules: Array<{ id: string; title: string; group?: string; on?: boolean }> };
  export const PAGE_FILTERS: ReadonlyArray<readonly [string, string]>;
  export const PAGE_SORTS: ReadonlyArray<readonly [string, string]>;
  export function pageKey(view: string, packId: string | null): string;
  export function pageRules(key: string): Stored[] | null;
  export function sortPageRules<R extends Stored>(rules: readonly R[], sort: string): R[];
  export function shownRules<R extends Stored>(rules: readonly R[]): R[];
  export function runningOf(rules: readonly Stored[]): number;
  export function packSwitch(rules: readonly Stored[]): { on: number; total: number; state: "true" | "false" };
  export function switchTurnsOn(state: "true" | "false"): boolean;
  export function catalogKinds(packs: readonly Pack[]): Array<[string, string]>;
  export function catalogPacks<P extends Pack>(packs: readonly P[], kind: string): P[];
  export const SHEET_FILTER_AT: number;
  export function sheetRules(pack: Pack, query: string): Pack["rules"];
}

declare module "*/ui/routes.js" {
  export type ViewName = "overview" | "pack" | "created" | "packs" | "review" | "create" | "history" | "sessions" | "settings" | "merge";
  export interface Route {
    view: ViewName | null; pack?: string; filterRule?: string; outcome?: "all" | "steered" | "logged";
    sessionFilter?: "all" | "corr" | "open"; rule?: string; session?: string; jump?: boolean; conflict?: string;
  }
  export function parseRoute(hash: string): Route;
  export function hashOf(route: Partial<Route>): string;
  export function bootRoute(search: string, hash: string): Route & { view: ViewName };
  export function urlFor(href: string, route: Partial<Route>, session?: string | null): string;
  export function repositoryUrl(href: string, key: string | null): string;
}

declare module "*/ui/views/index.js" {
  /** The need names a poll loads for the page now: the sidebar's, the open view's and an open sheet's. */
  export function currentNeeds(): string[];
}

declare module "*/ui/history-model.js" {
  export interface RankedRule { id: string; title: string; origin: string; count: number }
  export function filterRanked(ranked: RankedRule[], query: string): RankedRule[];
  export function historyOrigin(id: string): string;
  export function rankedRules(outcome?: "all" | "steered" | "logged"): RankedRule[];
  export function historyRuleId(ranked?: RankedRule[]): string | null;
  export function historyTitle(id: string): string;
  export type HistoryEmpty = { kind: "noRules" } | { kind: "noSessions" } | { kind: "noViolations"; sessions: number } | { kind: "nothingInRange" };
  export function historyEmpty(ranked?: RankedRule[]): HistoryEmpty | null;
}

declare module "*/ui/repo-rules.js" {
  export interface RuleCounts { total: number; on: number }
  export function ruleCountsOf(entries: ReadonlyArray<object>): RuleCounts;
  export function repoTag(counts: RuleCounts | null): string | null;
  export function repoLine(repo: { setUp: boolean; started: boolean; checkoutCount: number }, counts: RuleCounts | null): string;
  export function repoRuleCounts(key: string): RuleCounts | null;
  export function loadRepoRuleCounts(keys: readonly string[]): Promise<boolean>;
}

declare module "*/ui/settings-model.js" {
  export interface AgentStatus {
    state: string;
    tone: "on" | "warn" | "off" | "";
    command: { text: string; label: string } | null;
    problem?: string;
  }
  export function agentStatus(agent: { id: string; name: string; found: boolean; connected: boolean; problem: string | null }): AgentStatus;
  export function installedLabel(ms: number | null | undefined): string | null;
}

declare module "*/ui/render/rule-sheet-model.js" {
  export type AskIntent = "ask" | "check" | "scope" | "delivery";
  export const ASK: ReadonlyArray<[AskIntent, string]>;
  export const ASK_OPEN: Partial<Record<AskIntent, boolean>>;
  export interface AskRule { id: string; title: string; scope: string; mode: string }
  export function askPrompt(rule: AskRule, intent: AskIntent): string;
  export function agentSeesPrompt(id: string, text: string): string;
  export const LOG_NOTE: string;
  export function agentSeesHint(rule: { byRerail: boolean; log: boolean }): string;
  export function fmtDay(t: number | null | undefined): string;
  export interface LibraryAttachment { sourceKind: "library_rule"; sourceId: string; folder: string }
  export function libraryAttachments(stored: object): LibraryAttachment[];
  export function canDelete(stored: object): boolean;
  export function deleteConsequences(input: { repo: string; pastViolations: number; on: boolean }): string[];
  export function splitPath(path: string | null | undefined): { file: string; dir: string };
}

declare module "*/ui/views/create.js" {
  export interface CreateSuggestion { packId: string; name: string; reason: string }
  export function createSuggestions(
    recommendations: ReadonlyArray<{ packId: string; folder: string; reason: string }> | null,
    attachedPacks: ReadonlyMap<string, unknown>,
    packs: ReadonlyArray<{ id: string; name: string }>,
  ): CreateSuggestion[];
}

declare module "*/ui/first-open.js" {
  export type FirstOpenState = "no-repo" | "missing" | "not-set-up" | "stopped" | "no-packs" | "packs-no-session" | "mature";
  export interface RepositoryFacts { scoped: boolean; missing: boolean; setUp: boolean; stopped: boolean; ruleCount: number; sessionsEver: number }
  export function firstOpenState(facts: RepositoryFacts): FirstOpenState;
  export function bannerFor(state: FirstOpenState, welcomeDismissed: boolean): "stopped" | "welcome" | null;
  export function overviewBody(facts: { ruleCount: number; sessionsEver: number; topCount: number }): "no-rules" | "no-sessions" | "no-violations" | "lists";
  export function steeredTally(activity: Record<string, { measured?: boolean; distinctViolations: number; loggedViolations?: number; rerails?: number }>): { steered: number; total: number };
  export const TOP_RULES: number;
  export function topRules<R extends { id: string; title: string; fires: number }>(rules: readonly R[], limit?: number): R[];
  export function rulesOnSource(packCount: number, mineCount: number): string;
  export const LIVE_MINUTES: number;
  export function liveState(session: { endedAt: number | null; startedAt: number; lastEventAt: number | null }, now: number): "live" | "stale" | null;
}

declare module "*/ui/merge-model.js" {
  type Status = import("../../src/engine/index.js").MergeAssistStatus;
  type Finding = import("../../src/engine/index.js").Finding;
  type OwnDecision = import("../../src/engine/index.js").OwnDecision;
  type Segment = string | { code: string };
  export const RUNNER_NAMES: Record<string, string>;
  export function runnerName(id: string): string;
  export function authorOf(finding: Finding): { name: string | null; declared: boolean };
  export function safePeerUrl(url: unknown, context?: { repositoryKey?: string; repository?: string | null }): string | null;
  export function peerLabel(peer: Finding["peer"]): string;
  export function levelText(level: "code" | "note"): string;
  export interface Sections {
    talk: Array<Finding & { busy: boolean }>;
    review: Array<Finding & { busy: boolean }>;
    added: Array<Finding & { busy: boolean }>;
    own: Array<OwnDecision & { busy: boolean }>;
    notRules: Array<OwnDecision & { busy: boolean }>;
  }
  export function sectionsOf(status: Status, pending?: Map<string, string>): Sections;
  export function waitingCount(status: Status | null | undefined, pending?: Map<string, string>): number;
  export function branchText(status: Status): { branch: string; base: string };
  export function judgeText(judge: Status["readiness"]["judge"]): { ok: boolean; text: string; detail?: string | null };
  export function githubText(github: Status["readiness"]["github"]): { ok: boolean; text: string };
  export function scannedWhat(lastScan: NonNullable<Status["lastScan"]>): string;
  export function budgetText(budget: Status["budget"]): { text: string; spent: boolean };
  export function scanSummary(before: Status | null, after: Status): string;
  export function readinessChecks(readiness: Status["readiness"]): Array<{ key: string; ok: boolean; text: Segment[]; overview?: boolean }>;
  export function writeError(error: unknown): string;
  export function quoted(text: string): string;
  type Conflict = import("../../src/engine/index.js").Conflict;
  type ConflictAction = import("../../src/engine/index.js").ConflictAction;
  export const DECISION_MAX: number;
  export function conflictSections(status: Status, pending?: Map<string, string>): { open: Array<Conflict & { busy: boolean }>; closed: Array<Conflict & { busy: boolean }> };
  export function tabOf(status: Status, chosen: string | null, pending?: Map<string, string>): "conflicts" | "decisions" | "log";
  export function tabsOf(status: Status): Array<"conflicts" | "decisions" | "log">;
  export function hunkLines(hunk: string): Array<{ kind: "add" | "del" | "meta" | "ctx"; text: string }>;
  export function resolutionText(conflict: Conflict): {
    badge: string; precedent: boolean; how: string | null; who: string | null; text: string | null; rule: string | null; at: string | null;
  };
  export function draftText(conflict: Conflict, action: "mine" | "theirs" | "new"): string;
  export function draftProblem(text: string): string | null;
  export function resolveBody(conflictId: string, action: ConflictAction, text?: string): { conflictId: string; action: ConflictAction; text?: string };
  export function conflictToast(conflict: Conflict, action: ConflictAction, text: string, ignored: boolean): { done: string; fail: string };
  type LedgerStatus = import("../../src/engine/index.js").LedgerStatus;
  type DecisionLogQuestion = import("../../src/engine/index.js").DecisionLogQuestion;
  export const LEDGER_STATUS_TEXT: Record<LedgerStatus, string>;
  export const LEDGER_STATUS_TITLE: Record<LedgerStatus, string>;
  export function logStatusCounts(status: Status): Record<LedgerStatus, number>;
  export function logQuestions(status: Status, filter?: LedgerStatus | null): DecisionLogQuestion[];
  export function logWhen(iso: string): string;
}

declare module "*/ui/views/merge.js" {
  export const needs: string[];
  export const actions: Record<string, (id?: string, element?: unknown) => Promise<void> | void>;
  export function render(root: unknown): void;
  export function onInput(element: unknown): void;
  export function enter(route: { view: string | null; conflict?: string }): void;
}
