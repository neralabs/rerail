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
  /** How many files the locked archive holds, or null when none was read (no lock, or S3 lacks the archive). */
  locked: number | null;
  /** How many of the locked archive's files publish/ stages again: with `locked`, the signal modeWarning reads. */
  restaged: number;
  /** SPDX expression → how many of the archive's files are under it. */
  licences: Record<string, number>;
  /** A samples archive's: each repository its labels name as `source.repo`, with the `source.license` they record. */
  sources?: Record<string, string>;
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
/** A pack or tool directory name. */
export const TOOL_ID: RegExp;
export const BUCKET: string;
export const REGION: string;
export const ARCHIVE_PREFIX: string;
export const ARCHIVE_CACHE: string;
export const AUTHORED_CORPUS_PATTERN: string;
export const PACK_FIXTURE_PATTERNS: readonly string[];
/** The `format` of every test-data archive's `upstream/<tool>/provenance.json`. */
export const PROVENANCE_FORMAT: "rerail.pack-provenance/v1";
export const PULL_INSTRUCTION: string;
export const PUBLISH_DIRECTORY: "publish";
export const PUBLISH_WORKFLOW: string;
export const PUBLISH_MANIFEST_FORMAT: "rerail.pack-data-publish/v1";
export function publishPath(path: string): (ToolPack & { path: string }) | undefined;
export function isPackFixture(path: string): boolean;
export function listedAuthored(record: string, text: string): string[];
export interface UpstreamSources {
  licence: string | undefined;
  source: { repository: string; tag: string | undefined; commit: string | undefined; licence: string | undefined } | undefined;
  taken: { glob: string; how: string }[];
  notCopied: { prefix: string; glob: string | undefined }[];
  malformed: string[];
}
export function readUpstreamSources(text: string): UpstreamSources;
export function copyKind(how: string): string;
export function decidingPattern<P extends { glob: string; licence: string | undefined; how: string }>(covering: readonly P[]): { pattern?: P; tie?: P[] };
export function pathGlob(glob: string): RegExp;
export function toolRecords(root?: string): string[];
export function authoredListing(root?: string): Set<string>;
export function packFixtureKind(path: string, authored?: ReadonlySet<string>): "authored" | "derived" | undefined;
export function missingPackFixture(path: string): string;
export function packFixtureAbsence(path: string): string | undefined;
export function requirePackFixture(path: string): string;
export function packFixturesWritten(paths: readonly string[], root?: string): string;
export function expectedFindingsWritten(path: string, kind: string): string;
export function requireAws(aws?: Aws): void;
export function toolPacks(root?: string): ToolPack[];
export function selectToolPacks(root?: string, pack?: string, tool?: string): ToolPack[];
export function toolPackFiles(root: string, directory: string): string[];
export function publishFiles(root: string, directory: string): string[];
export function objectTagging(tags?: Partial<UploadTags>): string;
export function licenceAlternatives(expression: string): string[][] | undefined;
export function licenceAllowed(expression: string, allowed: ReadonlySet<string>): boolean;
export function inToolSubdirectory(packPath: string): boolean;
export function archiveKey(toolPack: Pick<ToolPack, "pack" | "tool">, digest: string): string;
export function archiveCachePath(root: string, toolPack: Pick<ToolPack, "pack" | "tool">, digest: string): string;
export function readLock(root: string, toolPack: ToolPack): Lock;
export function verifiedEntries(compressed: Buffer, lock: Lock, key: string, directory: string): { path: string; bytes: Buffer }[];
export const TREE_MARKER: string;
export function treeReady(directory: string, lock: Pick<Lock, "sha256" | "files" | "bytes">): boolean;
export function extractTree(directory: string, entries: readonly { path: string; bytes: Buffer }[], lock: Pick<Lock, "sha256" | "files" | "bytes">): void;
/** The GitHub Actions files reportArchives appends to, when set. */
export interface ActionsEnvironment {
  GITHUB_OUTPUT?: string;
  GITHUB_STEP_SUMMARY?: string;
}
/** `<pack> <tool> <kind> <sha256>` per archive, sorted. */
export function archiveLines(archives: readonly { toolPack: Pick<ToolPack, "pack" | "tool">; kind: "test-data" | "samples"; sha256: string }[]): string[];
export function reportArchives(lines: readonly string[], options?: { env?: ActionsEnvironment; summary?: boolean }): void;
export function pull(options?: CommandOptions & { env?: ActionsEnvironment }): {
  /** Each selected tool-pack, where its archive came from, and how many publish/ files were laid over it. */
  toolPacks: (ToolPack & { source: "cache" | "S3" | "publish/"; published: number })[];
  /** The selected tool-packs with a samples.lock, and where each one's samples archive came from. */
  samples: (ToolPack & { source: "cache" | "S3" })[];
};
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
export function modeWarning(object: Pick<PublishedObject, "kind" | "pack" | "tool" | "locked" | "restaged">, replace: boolean): string | undefined;
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
/** `archives`: archiveLines of the locks the key names. */
export function archiveCache(options?: Omit<CommandOptions, "aws" | "dryRun" | "log">): { key: string; paths: string[]; archives: string[] };
