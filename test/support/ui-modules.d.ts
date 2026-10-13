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
}

declare module "*/ui/poll.js" {
  export function titleFromPrompts(prompts: readonly string[]): string | null;
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
  export function changeScope(scope: { repository?: string; checkout?: string } | null): void;
  export function showMoreActivity(): Promise<void>;
  export type Suppressions = "honour" | "ignore";
  export interface FolderState { folder: string; on: boolean; mode: string; suppressions: Suppressions }
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
  export function suppressionsText(stored: object, checkoutPath?: string): string;
  export function nextSuppressions(stored: object): "honour" | "ignore";
  export function optionsText(stored: object, checkoutPath?: string): string | null;
  export function isRunning(stored: { enabled: boolean; configError?: string | null }): boolean;
  export function onText(stored: object, checkoutPath?: string): string;
  export function packSuppressions(rules: readonly object[], owner: "repository" | "checkout"): { text: string; next: "honour" | "ignore" };
}

declare module "*/ui/state.js" {
  export function sinceFor(range?: number): number;
  export const data: {
    rules: Map<string, unknown>;
    findings: Array<{ ruleId: string; mode?: string; at: number }>;
    findingsTotal: number;
    findingsSince: number;
    activity: Record<string, unknown>;
    activityAll: Record<string, unknown>;
    packs: unknown[];
    attachedPacks: Map<string, string[]>;
    repository: { setUp: boolean } | null;
    recommendations: Array<{ packId: string; folder?: string }> | null;
  };
  export function setScope(scope: { repository?: string; checkout?: string } | null): void;
  export const ui: { filter: string; query: string; range: number; actPack: string; actRule: string | null; recentShown: number; view: string; settingUp: boolean };
  export function keptPacks(): readonly string[];
  export function isAudited(id: string): boolean;
  export function summary(): { logged: number; triggers: number; on: unknown[] };
  export function runningCount(): number;
  export function matches(stored: object): boolean;
  export function activityView(): { filtered: boolean; list: unknown[]; shown: unknown[]; none: boolean; more: false | { count: number | null; of: number | null } };
  export interface RuleGroup { name: string; all: Array<{ rule: { id: string } }>; shown: Array<{ rule: { id: string } }> }
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
  /** The page's rule setting writes; `toast`, `invalidate` and `owner` are the page's. */
  export function settingWrites(page: { toast: (message: string) => void; invalidate: () => void; owner: () => "repository" | "checkout" }): {
    setEnabled(ids: readonly string[], enabled: boolean, label: string): Promise<void>;
    setSuppressions(ids: readonly string[], suppressions: "honour" | "ignore", label: string): Promise<void>;
    setPackSuppressions(packId: string, suppressions: "honour" | "ignore", label: string): Promise<void>;
  };
}
