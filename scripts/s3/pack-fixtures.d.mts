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
/** Every upload's S3 tags (objectTagging): `rerail:commit`, `rerail:pr`, `rerail:run-id` and `rerail:uploader`. */
export interface UploadTags {
  commit: string;
  pr: string;
  runId: string;
  uploader: string;
}
export interface UploadOptions extends Omit<CommandOptions, "dryRun"> {
  tags: UploadTags;
}
/** An archive `pack --from` wrote, as its manifest names it. */
export interface PublishedObject {
  kind: "test-data" | "samples";
  pack: string;
  tool: string;
  /** The lock's repository path. */
  lock: string;
  key: string;
  sha256: string;
  files: number;
  bytes: number;
  compressedBytes: number;
  lockText: string;
  /** The sha256 of the archive the commit's lock named, or null when it had none. */
  base: string | null;
  /** SPDX expression → how many of the archive's files are under it. */
  licences: Record<string, number>;
  added: string[];
  changed: string[];
  dropped: string[];
}
export interface PublishManifest {
  format: "rerail.pack-data-publish/v1";
  commit: string;
  replace: boolean;
  objects: PublishedObject[];
  /** Every publish/ path of the commit, which the commit writing the locks deletes. */
  remove: string[];
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
export const PUBLISH_DIRECTORY: "publish";
export const PUBLISH_WORKFLOW: string;
export const PUBLISH_MANIFEST_FORMAT: "rerail.pack-data-publish/v1";
export function publishPath(path: string): (ToolPack & { path: string }) | undefined;
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
export function toolPacks(root?: string): ToolPack[];
export function selectToolPacks(root?: string, pack?: string, tool?: string): ToolPack[];
export function toolPackFiles(root: string, directory: string): string[];
export function publishFiles(root: string, directory: string): string[];
export function objectTagging(tags?: Partial<UploadTags>): string;
export function licenceAllowed(expression: string, allowed: ReadonlySet<string>): boolean;
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
export function pull(options?: CommandOptions): {
  /** Each selected tool-pack, where its archive came from, and how many publish/ files were laid over it. */
  toolPacks: (ToolPack & { source: "cache" | "S3" | "publish/"; published: number })[];
  /** The selected tool-packs with a samples.lock, and where each one's samples archive came from. */
  samples: (ToolPack & { source: "cache" | "S3" })[];
};
/** `cache`: the recording cache directory to fill (RECORDING_CACHE under `root` by default; the recorder's `<--cache>/recordings`). */
export function recordings(options?: CommandOptions & { cache?: string }): { kept: string[]; downloaded: string[]; missing: string[]; extracted: string[] };
export function recordingsPush(options: UploadOptions): { uploaded: string[]; locks: string[] };
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
export function samplesPush(options: UploadOptions): { uploaded: string[]; locks: string[] };
export function pack(options?: Omit<CommandOptions, "aws">): { changedLocks: string[] };
export function upload(options: UploadOptions): { uploaded: string[] };
export function push(options: CommandOptions & { tags?: UploadTags }): { changedLocks: string[]; uploaded: string[] };
export function packFrom(options: {
  root?: string;
  /** A git revision naming the pull request's commit, whose publish/ is archived from its blobs. */
  from: string;
  replace?: boolean;
  /** Where the compressed archives are written, under their key less ARCHIVE_PREFIX. */
  out: string;
  manifest?: string;
  summary?: string;
  aws?: Aws;
  log?: (line: string) => void;
}): Promise<PublishManifest>;
export function publishSummary(manifest: PublishManifest): string;
export function stage(options?: CommandOptions & { all?: boolean }): {
  toolPacks: (ToolPack & { staged: string[]; removed: string[]; missing: string[] })[];
};
export function publishedObjects(objects: unknown): { kind: "test-data" | "samples"; toolPack: ToolPack; key: string; lock: Lock; compressedBytes: number; check(compressed: Buffer): unknown }[];
export function uploadPublished(options: { objects: unknown; archives: string; tags: UploadTags; aws?: Aws; log?: (line: string) => void }): { uploaded: string[] };
export function confirm(options: { objects: unknown; uploaded?: readonly string[]; aws?: Aws; log?: (line: string) => void }): {
  confirmed: string[];
  failed: { key: string; reason: string }[];
};
/** Throws, listing each missing key with its lock and remedy, unless S3 holds every archive the locks name. */
export function check(options?: Omit<CommandOptions, "dryRun">): { checked: string[] };
export function archiveCache(options?: Omit<CommandOptions, "aws" | "dryRun" | "log">): { key: string; paths: string[] };
