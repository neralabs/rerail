// Types of pack-fixtures.mjs, for its TypeScript consumers and tests.
export interface AwsRun {
  status: number | null;
  stdout?: string;
  stderr?: string;
  error?: NodeJS.ErrnoException;
}
export type Aws = (args: string[], options?: object) => AwsRun;
export interface CommandOptions {
  root?: string;
  dryRun?: boolean;
  aws?: Aws;
  log?: (line: string) => void;
}

export const ROOT: string;
export const BUCKET: string;
export const REGION: string;
export const KEY_PREFIX: string;
export const AUTHORED_CORPUS_PATTERN: string;
export const PACK_FIXTURE_PATTERNS: readonly string[];
export const PULL_INSTRUCTION: string;
export function isPackFixture(path: string): boolean;
export function listedAuthored(record: string, text: string): string[];
export function toolRecords(root?: string): string[];
export function authoredListing(root?: string): Set<string>;
export function packFixtureKind(path: string, authored?: ReadonlySet<string>): "authored" | "derived" | undefined;
export function missingPackFixture(path: string): string;
export function packFixtureAbsence(path: string): string | undefined;
export function requirePackFixture(path: string): string;
export function packFixturesWritten(paths: readonly string[], root?: string): string;
export function recordingWritten(path: string, kind: string): string;
export function authoredSyncs(root?: string): { prefix: string; filters: string[] }[];
export function literalFilter(path: string): string;
export function requireAws(aws?: Aws): void;
export function trackedPackFixtures(root?: string): string[];
export function trackedDerivedPackFixtures(root?: string): string[];
export function trackedSnapshot(root?: string): {
  tracked: string[];
  edited: Map<string, Buffer | null>;
  restore(): { restored: number; kept: number };
};
export function pull(options?: CommandOptions): { restored: number; kept: number };
export function push(options?: CommandOptions): { pushed: string[] };
