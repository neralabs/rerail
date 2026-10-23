// Types of pack-fixtures.mjs, for its TypeScript consumers and tests.
export interface AwsRun {
  status: number | null;
  stdout?: string;
  stderr?: string;
  error?: NodeJS.ErrnoException;
}
export type Aws = (args: string[], options?: object) => AwsRun;
export interface ToolPack {
  pack: string;
  tool: string;
  /** packs/<pack>/upstream/<tool> */
  directory: string;
}
export interface Lock {
  /** The lock's repository path. */
  label: string;
  sha256: string;
  files: number;
  bytes: number;
}
export interface LockedRecording {
  /** The recording's file name, its key in the lock's `files` (RECORDING_FILE). */
  name: string;
  sha256: string;
  size: number;
}
/** The corpus answers archive a recording.lock names, as a Lock for verifiedCorpus. */
export interface LockedCorpus extends Lock {
  /** Its key in the lock's `files` (CORPUS_ARCHIVE). */
  name: string;
}
export interface LockedRecordings {
  recording: LockedRecording;
  corpus: LockedCorpus;
}
export interface CommandOptions {
  root?: string;
  pack?: string;
  tool?: string;
  dryRun?: boolean;
  aws?: Aws;
  log?: (line: string) => void;
}
export interface PushOptions extends CommandOptions {
  /** A git revision naming a pull request's commit: its tracked pack test data is archived from its blobs. */
  from?: string;
  /** With `from`: a checkout of that commit, where the new locks are written. */
  into?: string;
  /** With `from`: the archive is the commit's tracked pack test data alone, with no base archive under it. */
  replace?: boolean;
}

export const ROOT: string;
export const BUCKET: string;
export const REGION: string;
export const ARCHIVE_PREFIX: string;
export const ARCHIVE_CACHE: string;
export const RECORDING_PREFIX: string;
export const RECORDING_CACHE: string;
export const RECORDING_LOCK: string;
export const RECORDING_LOCK_FORMAT: string;
export const RECORDING_FILE: string;
export const CORPUS_ARCHIVE: string;
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
export function requireAws(aws?: Aws): void;
export function trackedPackFixtures(root?: string, paths?: readonly string[]): string[];
export function trackedSnapshot(root?: string, paths?: readonly string[]): {
  tracked: string[];
  edited: Map<string, Buffer | null>;
  restore(): { restored: number; kept: number };
};
export function toolPacks(root?: string): ToolPack[];
export function selectToolPacks(root?: string, pack?: string, tool?: string): ToolPack[];
export function toolPackFiles(root: string, directory: string): string[];
export function archiveKey(toolPack: Pick<ToolPack, "pack" | "tool">, digest: string): string;
export function archiveCachePath(root: string, toolPack: Pick<ToolPack, "pack" | "tool">, digest: string): string;
export function readLock(root: string, toolPack: ToolPack): Lock;
export function recordingKey(toolPack: Pick<ToolPack, "pack" | "tool">, digest: string): string;
export function recordingCacheFile(root: string, digest: string): string;
export function corpusKey(toolPack: Pick<ToolPack, "pack" | "tool">, digest: string): string;
export function corpusCacheFile(root: string, digest: string): string;
export function corpusTreeDirectory(root: string, digest: string): string;
export function lockedRecordings(text: string, label: string): LockedRecordings;
export function readRecordingLock(root: string, toolPack: ToolPack): ({ label: string } & LockedRecordings) | undefined;
export function verifiedEntries(compressed: Buffer, lock: Lock, key: string, directory: string): { path: string; bytes: Buffer }[];
export function verifiedCorpus(compressed: Buffer, lock: Lock, key: string): { path: string; bytes: Buffer }[];
export const CORPUS_TREE_MARKER: string;
export function corpusTreeReady(directory: string, lock: Pick<Lock, "sha256" | "files" | "bytes">): boolean;
export function extractCorpusTree(directory: string, entries: readonly { path: string; bytes: Buffer }[], lock: Pick<Lock, "sha256" | "files" | "bytes">): void;
export function pull(options?: CommandOptions): { toolPacks: (ToolPack & { source: "cache" | "S3" })[]; restored: number; kept: number };
/** `cache`: the recording cache directory to fill (RECORDING_CACHE under `root` by default; the recorder's `<--cache>/recordings`). */
export function recordings(options?: CommandOptions & { cache?: string }): { kept: string[]; downloaded: string[]; missing: string[]; extracted: string[] };
export function recordingsPush(options?: Omit<CommandOptions, "dryRun">): { uploaded: string[]; locks: string[] };
export function pack(options?: Omit<CommandOptions, "aws">): { changedLocks: string[] };
export function upload(options?: Omit<CommandOptions, "dryRun">): { uploaded: string[] };
export function push(options?: PushOptions): {
  changedLocks: string[];
  uploaded: string[];
  tracked: string[];
  /** With `from`: the repository paths of the files the new archives drop from the ones the commit's locks named, sorted. */
  dropped?: string[];
  script?: string;
};
/** Throws, listing each missing key with its lock and remedy, unless S3 holds every archive and recording the locks name. */
export function check(options?: Omit<CommandOptions, "dryRun">): { checked: string[] };
export function archiveCache(options?: Omit<CommandOptions, "aws" | "dryRun" | "log">): { key: string; paths: string[] };
