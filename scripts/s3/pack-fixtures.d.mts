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
/** The recording archive a recording.lock names, as a Lock for verifiedRecording. */
export interface LockedRecording extends Lock {
  /** Its key in the lock's `files` (RECORDING_ARCHIVE). */
  name: string;
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
export const RECORDING_LOCK: "recording.lock";
export const RECORDING_LOCK_FORMAT: "rerail.port-readiness-recording-lock/v4";
export const RECORDING_ARCHIVE: "recording.tar.zst";
export const RECORDING_ARCHIVE_FORMAT: "rerail.port-readiness-recording-archive/v1";
export const RECORDING_TREE_ROOTS: readonly string[];
export const AUTHORED_CORPUS_PATTERN: string;
export const PACK_FIXTURE_PATTERNS: readonly string[];
export const PULL_INSTRUCTION: string;
/** A pinned ESLint test set: the zip's sha256, and the packs it scores. */
export interface EslintTestSetPin {
  sha256: string;
  packs: readonly string[];
}
export const ESLINT_TEST_SET: EslintTestSetPin;
export const ESLINT_TEST_SET_TREES: string;
export function eslintTestSetKey(digest: string): string;
export function eslintTestSetCachePath(root: string, digest: string): string;
export function eslintTestSetZip(options?: { root?: string; set?: EslintTestSetPin; dryRun?: boolean; aws?: Aws; log?: (line: string) => void }): { path: string | undefined; source: "cache" | "S3" };
/** The extracted tree of the pinned set (its manifest.json's directory). */
export function eslintTestSet(options?: { root?: string; set?: EslintTestSetPin; aws?: Aws; log?: (line: string) => void }): string;
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
export function recordingArchiveCacheFile(root: string, digest: string): string;
export function recordingTreeDirectory(root: string, digest: string): string;
export function lockedRecording(text: string, label: string): LockedRecording;
export function readRecordingLock(root: string, toolPack: ToolPack): { label: string; archive: LockedRecording } | undefined;
export function verifiedEntries(compressed: Buffer, lock: Lock, key: string, directory: string): { path: string; bytes: Buffer }[];
export function verifiedRecording(compressed: Buffer, lock: Lock, key: string): { path: string; bytes: Buffer }[];
export const RECORDING_TREE_MARKER: string;
export function recordingTreeReady(directory: string, lock: Pick<Lock, "sha256" | "files" | "bytes">): boolean;
export function extractRecordingTree(directory: string, entries: readonly { path: string; bytes: Buffer }[], lock: Pick<Lock, "sha256" | "files" | "bytes">): void;
export function pull(options?: CommandOptions & { testSet?: EslintTestSetPin }): {
  toolPacks: (ToolPack & { source: "cache" | "S3" })[];
  restored: number;
  kept: number;
  /** The selected tool-packs with a samples.lock, and where each one's samples archive came from. */
  samples: (ToolPack & { source: "cache" | "S3" })[];
  /** Where the ESLint test set's zip came from, when a selected tool-pack's pack is one it scores. */
  eslintTestSet?: "cache" | "S3";
};
/** `cache`: the recording cache directory to fill (RECORDING_CACHE under `root` by default; the recorder's `<--cache>/recordings`). */
export function recordings(options?: CommandOptions & { cache?: string }): { kept: string[]; downloaded: string[]; missing: string[]; extracted: string[] };
export function recordingsPush(options?: Omit<CommandOptions, "dryRun">): { uploaded: string[]; locks: string[] };
/** A tool-pack's real-code samples manifest, beside test-data.lock and in its format. */
export const SAMPLES_LOCK: "samples.lock";
export const SAMPLES_PREFIX: string;
export function samplesKey(toolPack: Pick<ToolPack, "pack" | "tool">, digest: string): string;
export function samplesArchiveCachePath(root: string, toolPack: Pick<ToolPack, "pack" | "tool">, digest: string): string;
export function samplesTreeDirectory(root: string, toolPack: Pick<ToolPack, "pack" | "tool">, digest: string): string;
/** The tool-pack's samples.lock, or undefined when it has none; throws on a malformed one. */
export function readSamplesLock(root: string, toolPack: Pick<ToolPack, "pack" | "tool">): Lock | undefined;
export function verifiedSamples(compressed: Buffer, lock: Lock, key: string): { path: string; bytes: Buffer }[];
/** The tool-pack's extracted samples tree, verified and extracted on demand; undefined when it has no samples.lock. */
export function samplesTree(options: { root?: string; pack: string; tool: string; aws?: Aws }): string | undefined;
export function samples(options?: CommandOptions): { kept: string[]; downloaded: string[]; extracted: string[] };
export function samplesPush(options?: Omit<CommandOptions, "dryRun">): { uploaded: string[]; locks: string[] };
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
/** Throws, listing each missing key with its lock and remedy, unless S3 holds every archive the locks name. */
export function check(options?: Omit<CommandOptions, "dryRun"> & { testSet?: EslintTestSetPin }): { checked: string[] };
export function archiveCache(options?: Omit<CommandOptions, "aws" | "dryRun" | "log"> & { testSet?: EslintTestSetPin }): { key: string; paths: string[] };
