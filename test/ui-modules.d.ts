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
}

declare module "*/ui/state.js" {
  export function sinceFor(range?: number): number;
}
