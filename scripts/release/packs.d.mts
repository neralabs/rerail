// Types of packs.mjs, for packs-pack.ts.
export interface ReleasePack {
  id: string;
  name: string;
  version: string;
}

export interface GateReport {
  testResults: { assertionResults: { ancestorTitles: string[]; title: string; status: string }[] }[];
}

export const ROOT: string;
export const PACKS: string;
export const REGISTRY: string;
export const PUBLISH_TAG: string;
export function releaseTag(name: string, version: string): string;
export function releaseTags(): string[];
export function createReleaseTag(tag: string, sha: string): string;
export function listPacks(packsDirectory?: string): ReleasePack[];
export function assertCommitOnGitHub(sha: string): void;
export function alreadyOnRegistry(output: string): boolean;
export function releasePacks(
  manifest: readonly { name: string; version: string; file: string }[],
  tags: readonly string[],
  steps: {
    publish(file: string): { ok: boolean; output: string };
    assertRestricted(name: string): Promise<void>;
    tag(tag: string): void;
    log(line: string): void;
  },
): Promise<void>;
export function planPacks(packs: readonly ReleasePack[], tags: readonly string[]): { publish: string[]; first: string[] };
export function planPromotions(
  packs: readonly ReleasePack[],
  tags: readonly string[],
  latestOf: (name: string) => string | null,
): { promote: (ReleasePack & { latest: string | null })[]; unreleased: ReleasePack[] };
export function promotePacks(
  promotions: readonly { name: string; version: string }[],
  tags: readonly string[],
  steps: { promote(name: string, version: string): void; log(line: string): void },
): string[];
export function syncPackLockfile<Lock extends { packages: Record<string, { version?: string }> }>(lock: Lock, packs: readonly ReleasePack[]): Lock;
export function gateFailures(report: GateReport, ids: readonly string[], checkCount: number): string[];
export function packIds(args: readonly string[], packs?: readonly ReleasePack[]): string[];
