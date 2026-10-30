/**
 * Typings for the plain-ES-module UI files the tests import. `tsconfig` has no `allowJs`, so a
 * relative `.js` import is an implicit `any` under `strict`; wildcard ambient modules are the
 * one form TypeScript matches against a relative specifier. Only the surface the tests use.
 */
declare module "*/ui/api.js" {
  export function send(method: string, path: string, body: unknown, options?: { signal?: AbortSignal }): Promise<{ status: number; etag?: string; body: unknown }>;
}

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
    /** Whose content runs: Nera's, approved, or approved while an edit waits for approval. */
    approval: { state: "shipped" | "approved" | "pending"; approvedAt: number | null };
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
    /** Every rule waiting for approval (`GET /api/rule-approvals`); `null` until read. */
    approvals: unknown[] | null;
    /** The repository's rules that run nowhere until approved. */
    awaiting: Array<{ sourceKind: string; id: string; folders: string[] }>;
    awaitingApproval: number;
    repository: { setUp: boolean; repositoryKey?: string } | null;
    checkout: { checkoutPath: string; folder: string; state: string } | null;
    recommendations: Array<{ packId: string; folder?: string }> | null;
    followGitignore: boolean | null;
    sessionDetail: Map<string, SessionDetail>;
    missing: boolean;
    disconnected: boolean;
  };
  export function setScope(scope: { repository?: string; checkout?: string } | null): void;
  export const ui: { filter: string; query: string; range: number; actRule: string | null; histOutcome: "all" | "steered" | "logged"; view: string; settingUp: boolean; selectedSession: string | null; panel: object | null };
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
    keepPack(id: string, keep: boolean, message: string): Promise<void> | undefined;
  };
}

declare module "*/ui/settings-writes.js" {
  /** One settings write: the folder, the targets and the values it sets (`on: null` clears). */
  export interface SettingPut { folder: string; targets: Array<{ kind: "rule" | "pack"; id: string }>; setting: { on?: boolean | null; suppressions?: "honour" | "ignore" } }
  /** The writes the whole-pack switch sends for pack `packId`: per folder, the pack's on, then its rules' own on cleared. */
  export function packSwitchWrites(packId: string, enabled: boolean): SettingPut[];
  /** The page's rule setting writes; `toast`, `invalidate` and `owner` are the page's. */
  export function settingWrites(page: { toast: (message: string) => void; invalidate: () => void; owner: () => "repository" | "checkout" }): {
    setEnabled(ids: readonly string[], enabled: boolean, label: string): Promise<void>;
    setPackEnabled(packId: string, enabled: boolean, label: string): Promise<void>;
    setPackSuppressions(packId: string, suppressions: "honour" | "ignore", label: string): Promise<void>;
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
  export type ViewName = "overview" | "pack" | "created" | "packs" | "review" | "create" | "history" | "sessions" | "settings" | "approvals";
  export interface Route {
    view: ViewName | null; pack?: string; filterRule?: string; outcome?: "all" | "steered" | "logged";
    sessionFilter?: "all" | "corr" | "open"; rule?: string; session?: string; jump?: boolean;
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
  /** What `#content` shows: the view's name, or the page shown in its place. */
  export function pageKey(): string;
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
  export interface RuleCounts { total: number; on: number; awaiting: number }
  export function ruleCountsOf(entries: ReadonlyArray<object>, awaitingApproval?: number): RuleCounts;
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

declare module "*/ui/rule-diff.js" {
  export function escapeInvisible(text: string): string;
  export function visibleJson(value: unknown): string;
  export interface DiffLine { op: "same" | "added" | "removed"; text: string }
  export function lineDiff(before: string, after: string): DiffLine[];
}

declare module "*/ui/approval-model.js" {
  export interface DiffLine { op: "same" | "added" | "removed"; text: string }
  export const APPROVALS_HASH: string;
  export interface PendingRule {
    kind: "code" | "conversation"; owner: "library" | "pack"; id: string; packId: string | null; rule: { id: string };
    revision: number; contentHash: string; copiedFromRuleId: string | null; approved: { rule: object; approvedAt: number } | null;
  }
  export interface ApprovalView {
    key: string; id: string; kind: string; title: string; owner: string; runs: "approved" | "nowhere"; approvedAt: number | null;
    copiedFrom: string | null; lines: DiffLine[]; changed: number; body: { owner: string; id: string; contentHash: string };
  }
  export interface AwaitingReference { sourceKind: string; id: string; folders: string[] }
  export function awaitingCount(approvals: readonly PendingRule[] | null): number;
  export function awaitingText(n: number): string;
  export function ownerText(pending: PendingRule, packName?: (id: string) => string): string;
  export function pendingTitle(pending: PendingRule): string;
  export function waitingTitle(reference: AwaitingReference, approvals: readonly PendingRule[] | null, fallback?: (id: string) => string | undefined): string;
  export function approvalView(pending: PendingRule, packName?: (id: string) => string): ApprovalView;
  export function pendingNote(approval: { state: string; approvedAt: number | null } | null | undefined): { runs: "approved" | "nowhere"; label: string; tip: string } | null;
  export function awaitingReferences(unavailable: ReadonlyArray<{ sourceKind: string; sourceId: string; folder: string; reason: string }> | undefined): AwaitingReference[];
  export function awaitingOnPage(references: readonly AwaitingReference[], key: string, approvals: readonly PendingRule[] | null, packIdOf?: (id: string) => string | undefined): AwaitingReference[];
  export function isRuleChanged(error: unknown): boolean;
}
