// Types of packs.mjs, for packs-pack.ts.
export interface ReleasePack {
  id: string;
  name: string;
  version: string;
  private: boolean;
}

export interface GateReport {
  testResults: { assertionResults: { ancestorTitles: string[]; title: string; status: string }[] }[];
}

export const ROOT: string;
export const PACKS: string;
export const REGISTRY: string;
export const PUBLISH_TAG: string;
export const HELD_PUBLISH_TAG: string;
export function releaseTag(name: string, version: string): string;
export function releaseTags(): string[];
export function createReleaseTag(tag: string, sha: string): string;
export function listPacks(packsDirectory?: string): ReleasePack[];
export function publishedPacks(packsDirectory?: string): ReleasePack[];
export function assertCommitOnGitHub(sha: string): void;
export function alreadyOnRegistry(output: string): boolean;
export function releasePacks(
  manifest: readonly { name: string; version: string; file: string }[],
  tags: readonly string[],
  steps: {
    publish(file: string, distTag: string): { ok: boolean; output: string };
    assertPublic(name: string, version: string): Promise<void>;
    readNext(name: string): Promise<string | null>;
    distTag(name: string, version: string, tag: string): void;
    tag(tag: string): void;
    log(line: string): void;
  },
  held?: readonly string[],
): Promise<{ name: string; version: string; distTag: string }[]>;
export function planPacks(packs: readonly ReleasePack[], tags: readonly string[]): { publish: string[]; first: string[] };
export interface PromotionCandidate extends ReleasePack {
  hold?: boolean;
}
export interface Promotion extends ReleasePack {
  latest: string | null;
  hold: boolean;
  release: boolean;
}
export const HOLD_TAG: string;
export function heldPacks(names: readonly string[], distTagsOf: (name: string) => Record<string, string> | null): { held: string[]; unreadable: string[] };
export function promotionRequest(packs: readonly ReleasePack[], id: string | undefined, version: string | undefined): { packs: PromotionCandidate[]; named: boolean };
export function planPromotions(
  packs: readonly PromotionCandidate[],
  tags: readonly string[],
  distTagsOf: (name: string) => Record<string, string> | null,
  options?: { named?: boolean },
): { promote: Promotion[]; unreleased: PromotionCandidate[]; held: (PromotionCandidate & { latest: string | null })[]; unreadable: PromotionCandidate[] };
export function promotePacks(
  promotions: readonly { name: string; version: string; hold?: boolean; release?: boolean }[],
  tags: readonly string[],
  steps: {
    promote(name: string, version: string): void;
    hold(name: string, version: string): void;
    release(name: string): void;
    readNext(name: string): Promise<string | null>;
    distTag(name: string, version: string, tag: string): void;
    log(line: string): void;
  },
): Promise<{ promoted: { name: string; version: string }[]; failures: string[] }>;
export function syncPackLockfile<Lock extends { packages: Record<string, { version?: string }> }>(lock: Lock, packs: readonly ReleasePack[]): Lock;
export function gateFailures(report: GateReport, ids: readonly string[], checkCount: number): string[];
export function packIds(args: readonly string[], packs?: readonly ReleasePack[]): string[];
