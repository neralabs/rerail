/**
 * What a daemon's walks read whole, kept for the next daemon. A walk reads a file it holds no observation of to learn its
 * digest, and a daemon that starts holds none: it read every file of every worktree again, minutes of work in a
 * repository with many worktrees, after each restart, update or crash. For each file a walk read whole, the root the walk
 * started at keeps the file's stamp (`fileStamp`: device, file id, size, modification and change times) and the SHA-256
 * of its bytes, or that an unlisted file was binary. The next daemon's first walk of that root takes a kept read in place
 * of reading a file whose stamp is the same, and reads every other file (`CodeRuleBridge`).
 *
 * A kept read never stands in for a file whose bytes may have changed: it is kept only when the stamp moves with every
 * write ({@link stampTellsChanges}) and the read came after the stamp's clock tick, as Git's racy-index rule asks; and
 * the next daemon takes none of a root's reads when the clock is behind the last walk that kept them. Files named like
 * ones holding secrets are never kept. The reads of a root live in one file of `folder`, written off the event path; a
 * root that is gone, or that no walk used for 30 days, is dropped, and past `maxBytes` the least recently used go first.
 * A read or write that fails costs the next walks reads, never a change they miss, and the daemon's log says so once.
 * The reads of a root are judged, written and read `SLICE` at a time, so a root of many files holds the daemon's thread,
 * which answers every hook, a few milliseconds at a time.
 */
import { createHash, randomUUID } from "node:crypto";
import { lstat, mkdir, open, readdir, readFile, rename, rm, stat, utimes, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { setImmediate as nextTurn } from "node:timers/promises";
import { promisify } from "node:util";
import { gunzip, gzip } from "node:zlib";
import { z } from "zod/v4";
import { sensitiveByName } from "../path-boundary.js";
import type { ObservedSource } from "./code-rule-bridge.js";

/** A file as a walk read it whole: its stamp then, and the SHA-256 of its bytes, or that it was an unlisted binary file. */
export type KeptRead =
  | { readonly kind: "text"; readonly stamp: string; readonly digest: string }
  | { readonly kind: "binary"; readonly stamp: string };
/** A root's kept reads, by path relative to it (`/`-separated). */
export type KeptReads = ReadonlyMap<string, KeptRead>;

/** What a walk of a root found, and what it compared that with: what `WalkDigests.keep` writes when something is new. */
export interface WalkReads {
  readonly sources: ReadonlyMap<string, ObservedSource>;
  /** The observations the walk started from: one it found the same is the same object. */
  readonly previous: ReadonlyMap<string, ObservedSource>;
  /** The reads it took from the last daemon, if any. */
  readonly kept: KeptReads | undefined;
  /** Filesystem-clock reading taken as the walk began. */
  readonly observedAt: number;
}

/** The folder beside the content store that holds the kept reads. */
export const WALK_DIGESTS_FOLDER = "walk-digests";
/** The most the kept reads of every root take on disk; past it, the roots used longest ago are dropped first. */
export const MAX_WALK_DIGEST_BYTES = 128 * 1024 * 1024;
/** A root no walk used for this long is dropped: its worktree is likely gone from every repository Rerail watches. */
const MAX_IDLE_MS = 30 * 24 * 60 * 60 * 1000;
/** How long a kept root waits to be written, so a run of walks writes it once; `close` writes what still waits. */
const WRITE_DELAY_MS = 5_000;
/** The least time between two writes of one root, which a session writing on every turn would otherwise have rewritten after every walk. */
const REWRITE_GAP_MS = 60_000;
/** A write cut short leaves its temporary file; one this old belongs to no write still running. */
const LEFTOVER_MS = 60 * 60 * 1000;
const FORMAT = 1;
const SUFFIX = ".reads";
/** The most a root's first line, which names it, may take. */
const MAX_HEADER_BYTES = 8 * 1024;
/**
 * Reads handled between two turns of the event loop. Each read takes a microsecond or two to judge, write or read
 * back: a root of 100,000 files done in one piece would hold the daemon's thread, and each hook waiting on it, for a
 * tenth of a second; 2,000 at a time hold it for a few milliseconds.
 */
const SLICE = 2_000;

const gzipped = promisify(gzip);
const gunzipped = promisify(gunzip);

/**
 * Whether a stamp moves with every write to the file, whatever its modification time says: a program may set that time
 * back (`touch -d`, a copy or archive tool that keeps times), but a write moves the change time, and setting the
 * modification time afterwards does not put it back. Only a change of the system clock, or a program that sets the
 * change time itself (Windows allows that through `SetFileInformationByHandle`), can; a running daemon's walks miss
 * such a write too, as they take a file whose stamp held as unchanged (`staleStamp`). A stamp with no file id or
 * change time cannot tell, nor can one whose change time sits on a 10 ms grid: FAT and exFAT keep no change time and
 * report the modification time in its place, and a file system with coarser times leaves a write within the same tick
 * unseen.
 */
export function stampTellsChanges(stamp: string): boolean {
  const fields = stamp.split(":").map(Number);
  if (fields.length !== 5 || fields.some(field => !Number.isFinite(field))) return false;
  const [, ino, , , ctimeMs] = fields as [number, number, number, number, number];
  return ino !== 0 && ctimeMs > 0 && ctimeMs % 10 !== 0;
}

/** The latest of a stamp's modification and change times: what a walk records as the file's `modifiedAt`. */
function stampModifiedAt(stamp: string): number {
  const fields = stamp.split(":");
  return Math.max(Number(fields[3]), Number(fields[4]));
}

/**
 * What of `source`, at `path` in its walk's root, the next daemon may take in place of a read; undefined for a file it
 * must read again. Only a file read whole (a text's digest) or found binary is kept, and only when its stamp tells a
 * change ({@link stampTellsChanges}), the walk read it after the stamp's clock tick (Git's racy-index rule: a write in the
 * same tick leaves the same stamp), and no write moved it while the walk read. A file named like one holding secrets is
 * left out, as the content store leaves it out.
 */
export function keptRead(path: string, source: ObservedSource): KeptRead | undefined {
  const { stamp, modifiedAt, observedAt } = source;
  if (stamp === undefined || modifiedAt === undefined || observedAt === undefined || source.readError !== undefined) return undefined;
  if (!(modifiedAt < observedAt) || modifiedAt !== stampModifiedAt(stamp) || !stampTellsChanges(stamp)) return undefined;
  if (path === "" || path.startsWith("../") || sensitiveByName(path)) return undefined;
  return source.digest === undefined ? { kind: "binary", stamp } : { kind: "text", stamp, digest: source.digest };
}

/** Whether `source` is what `kept` stands for, as a walk observes a file it takes from a kept read. */
function keptAsItIs(kept: KeptRead, source: ObservedSource): boolean {
  return kept.stamp === source.stamp && (kept.kind === "binary" ? source.digest === undefined : source.digest === kept.digest);
}

/**
 * Whether a walk found nothing the kept reads of its root lack: each source is the one it started from, the one the
 * last daemon kept, or one the next daemon may not take. Only a source the walk read is judged by `keptRead`, so the
 * first walk after a start, which took its files from the kept reads, costs two comparisons a file, a slice at a time.
 */
async function holdsNothingNew(walk: WalkReads): Promise<boolean> {
  let count = 0;
  for (const [path, source] of walk.sources) {
    count += 1;
    if (count % SLICE === 0) await nextTurn();
    if (walk.previous.get(path) === source) continue;
    const kept = walk.kept?.get(path);
    if (kept !== undefined && keptAsItIs(kept, source)) continue;
    if (keptRead(path, source) !== undefined) return false;
  }
  return true;
}

/** Calls `take` with each read of `sources` the next daemon may take ({@link keptRead}), letting the event loop run between slices. */
async function forEachKeptRead(sources: ReadonlyMap<string, ObservedSource>, take: (path: string, read: KeptRead) => void): Promise<void> {
  let count = 0;
  for (const [path, source] of sources) {
    const read = keptRead(path, source);
    if (read !== undefined) take(path, read);
    count += 1;
    if (count % SLICE === 0) await nextTurn();
  }
}

/** A root's file name: its path's digest, which any length or letter of path fits. */
function fileName(root: string): string {
  return `${createHash("sha256").update(root).digest("hex").slice(0, 32)}${SUFFIX}`;
}

type StoredRead = [path: string, stamp: string, digest: string | null];

function isStoredRead(value: unknown): value is StoredRead {
  return Array.isArray(value) && value.length === 3 && typeof value[0] === "string" && typeof value[1] === "string"
    && (value[2] === null || typeof value[2] === "string");
}

/** A root file's first line: the root, and when the walk that kept its reads began on the filesystem's clock. */
const HeaderSchema = z.object({ format: z.literal(FORMAT), root: z.string(), observedAt: z.number() });
type Header = z.infer<typeof HeaderSchema>;

function parseHeader(line: string): Header | undefined {
  try {
    const parsed = HeaderSchema.safeParse(JSON.parse(line));
    return parsed.success ? parsed.data : undefined;
  } catch {
    return undefined;
  }
}

/** The reads of `sources` the next daemon may take, one JSON line each (`StoredRead`), so they can be read back a slice at a time. */
async function storedReads(sources: ReadonlyMap<string, ObservedSource>): Promise<Buffer> {
  const chunks: Buffer[] = [];
  let lines: string[] = [];
  await forEachKeptRead(sources, (path, read) => {
    lines.push(`${JSON.stringify([path, read.stamp, read.kind === "text" ? read.digest : null] satisfies StoredRead)}\n`);
    if (lines.length < SLICE) return;
    chunks.push(Buffer.from(lines.join("")));
    lines = [];
  });
  chunks.push(Buffer.from(lines.join("")));
  return Buffer.concat(chunks);
}

/** A root's file: its first line names the root and when its walk began, the rest is its reads, a line each, compressed. */
async function encode(root: string, walk: Pick<WalkReads, "sources" | "observedAt">): Promise<Buffer> {
  const header = Buffer.from(`${JSON.stringify({ format: FORMAT, root, observedAt: walk.observedAt } satisfies Header)}\n`);
  return Buffer.concat([header, await gzipped(await storedReads(walk.sources))]);
}

/** Each line of `body`, without its newline. */
function* linesOf(body: Buffer): Generator<string> {
  for (let start = 0; start < body.length;) {
    const newline = body.indexOf(0x0a, start);
    const stop = newline < 0 ? body.length : newline;
    yield body.toString("utf8", start, stop);
    start = stop + 1;
  }
}

/** The reads in a root's file, or undefined for a file of another root or format, or one that does not decode. */
async function decode(root: string, data: Buffer): Promise<{ header: Header; reads: Map<string, KeptRead> } | undefined> {
  const end = data.indexOf(0x0a);
  const header = end < 0 ? undefined : parseHeader(data.subarray(0, end).toString("utf8"));
  if (header?.root !== root) return undefined;
  const reads = new Map<string, KeptRead>();
  let count = 0;
  for (const line of linesOf(await gunzipped(data.subarray(end + 1)))) {
    const item: unknown = JSON.parse(line);
    if (!isStoredRead(item)) return undefined;
    const [path, stamp, digest] = item;
    reads.set(path, digest === null ? { kind: "binary", stamp } : { kind: "text", stamp, digest });
    count += 1;
    if (count % SLICE === 0) await nextTurn();
  }
  return { header, reads };
}

/** The root a file names on its first line, read without the rest; undefined when it names none. */
async function headerOf(path: string): Promise<Header | undefined> {
  const handle = await open(path, "r");
  try {
    const buffer = Buffer.alloc(MAX_HEADER_BYTES);
    const { bytesRead } = await handle.read(buffer, 0, MAX_HEADER_BYTES, 0);
    const end = buffer.subarray(0, bytesRead).indexOf(0x0a);
    return end < 0 ? undefined : parseHeader(buffer.subarray(0, end).toString("utf8"));
  } finally {
    await handle.close();
  }
}

async function gone(path: string): Promise<boolean> {
  try {
    await lstat(path);
    return false;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "ENOENT";
  }
}

interface RootFile {
  readonly root: string;
  readonly bytes: number;
  usedAt: number;
}

/**
 * The kept reads of every root a daemon's walks start at, one file per root in `folder` (`WALK_DIGESTS_FOLDER` beside
 * the content store, which holds what the walks read). Shared by the runtimes of an engine service. A failed read or
 * write costs only a read of each file it held: the walk reads what it finds no kept read for, as before.
 */
export class WalkDigests {
  readonly #folder: string;
  readonly #maxBytes: number;
  readonly #log: (line: string) => void;
  /** The failures the log has told of: one line each per daemon, as a full disk would fail every write. */
  readonly #reported = new Set<"read" | "write">();
  /** What the folder holds, by file name: read once, before the first write, removal or use is noted, dropping what is stale. */
  #files: Map<string, RootFile> | undefined;
  readonly #pending = new Map<string, Pick<WalkReads, "sources" | "observedAt">>();
  readonly #writtenAt = new Map<string, number>();
  #timer: NodeJS.Timeout | undefined;
  /**
   * Each walk's check for anything new, and each removal of a root, one after another, so a later walk's reads are
   * never replaced by an earlier one's; never rejects.
   */
  #checks: Promise<void> = Promise.resolve();
  /** Every write and removal, one after another; never rejects. */
  #disk: Promise<void> = Promise.resolve();
  #closed = false;

  /** `log` takes each line that says what failed, the daemon's log (stderr) by default. */
  constructor(folder: string, options: { maxBytes?: number; log?: (line: string) => void } = {}) {
    this.#folder = folder;
    this.#maxBytes = options.maxBytes ?? MAX_WALK_DIGEST_BYTES;
    this.#log = options.log ?? (line => process.stderr.write(line));
  }

  /**
   * The reads kept for `root` (physical), for a walk that began at `observedAt` on the filesystem's clock. None when the
   * clock is behind the walk that kept them: a file written after the clock went back may carry a stamp it had before.
   */
  async load(root: string, observedAt: number): Promise<KeptReads> {
    // What a walk that finished already found is handed on.
    await this.#checks;
    const pending = this.#pending.get(root);
    if (pending !== undefined) return await this.#pendingReads(pending, observedAt);
    const name = fileName(root);
    try {
      const decoded = await decode(root, await readFile(join(this.#folder, name)));
      if (decoded === undefined || observedAt < decoded.header.observedAt) return new Map();
      this.#used(name);
      return decoded.reads;
    } catch (error) {
      // Missing, cut short or not ours: the walk reads every file, as with no kept reads.
      this.#failed("read", error);
      return new Map();
    }
  }

  /**
   * Keeps what a walk of `root` read, written within `WRITE_DELAY_MS`, and no sooner than `REWRITE_GAP_MS` after the
   * root's last write; `close` writes what still waits. A walk that found nothing new writes nothing.
   */
  keep(root: string, walk: WalkReads): void {
    if (this.#closed) return;
    this.#check(async () => {
      if (await holdsNothingNew(walk)) return;
      this.#pending.set(root, { sources: walk.sources, observedAt: walk.observedAt });
      this.#schedule();
    });
  }

  /** Drops the reads of `root`, a worktree that is gone, after what the walks before kept of it. */
  forget(root: string): void {
    this.#check(async () => {
      this.#pending.delete(root);
      this.#writtenAt.delete(root);
      this.#queue(async () => {
        const files = await this.#opened();
        await this.#remove(files, fileName(root));
        await this.#trim(files);
      });
    });
  }

  /** Writes every root still waiting, the walks that finished before it included, then keeps nothing more. */
  async close(): Promise<void> {
    this.#closed = true;
    clearTimeout(this.#timer);
    this.#timer = undefined;
    await this.#checks;
    this.#queue(async () => await this.#writeDue(true));
    await this.#disk;
  }

  #check(operation: () => Promise<void>): void {
    this.#checks = this.#checks.then(operation).catch((error: unknown) => this.#failed("write", error));
  }

  async #pendingReads(pending: Pick<WalkReads, "sources" | "observedAt">, observedAt: number): Promise<KeptReads> {
    const reads = new Map<string, KeptRead>();
    if (observedAt < pending.observedAt) return reads;
    await forEachKeptRead(pending.sources, (path, read) => reads.set(path, read));
    return reads;
  }

  #schedule(): void {
    if (this.#timer !== undefined || this.#closed) return;
    this.#timer = setTimeout(() => {
      this.#timer = undefined;
      this.#queue(async () => await this.#writeDue(false));
    }, WRITE_DELAY_MS);
    this.#timer.unref?.();
  }

  #queue(operation: () => Promise<void>): void {
    this.#disk = this.#disk.then(operation).catch((error: unknown) => this.#failed("write", error));
  }

  /**
   * Tells the log, once per daemon, that a read or write of the kept reads failed: the walks then read those files again,
   * which is slower, not wrong. A file that is not there (no daemon kept that root, or another removed it) is no failure.
   */
  #failed(what: "read" | "write", error: unknown): void {
    if ((error as NodeJS.ErrnoException | undefined)?.code === "ENOENT" || this.#reported.has(what)) return;
    this.#reported.add(what);
    const reason = error instanceof Error ? error.message : String(error);
    const line = what === "write"
      ? `rerail: cannot keep what the source walks read in ${this.#folder}: ${reason}. After a restart the daemon reads those files again.\n`
      : `rerail: cannot read what the last daemon's source walks read in ${this.#folder}: ${reason}. The walks read those files again.\n`;
    try {
      this.#log(line);
    } catch {
      // A log that cannot take the line leaves nothing else to tell, and the queue of writes must go on.
    }
  }

  async #writeDue(all: boolean): Promise<void> {
    const files = await this.#opened();
    for (const [root, walk] of [...this.#pending]) {
      const writtenAt = this.#writtenAt.get(root);
      if (!all && writtenAt !== undefined && Date.now() - writtenAt < REWRITE_GAP_MS) continue;
      this.#pending.delete(root);
      this.#writtenAt.set(root, Date.now());
      await this.#write(files, root, walk);
    }
    await this.#trim(files);
    if (this.#pending.size > 0) this.#schedule();
  }

  async #write(files: Map<string, RootFile>, root: string, walk: Pick<WalkReads, "sources" | "observedAt">): Promise<void> {
    const name = fileName(root);
    const data = await encode(root, walk);
    // One root past a quarter of the bound would leave little room for the rest: it is read whole after a restart.
    if (data.length > this.#maxBytes / 4) {
      await this.#remove(files, name);
      return;
    }
    const temporary = join(this.#folder, `${name}.${process.pid}.${randomUUID()}.tmp`);
    try {
      await mkdir(this.#folder, { recursive: true, mode: 0o700 });
      await writeFile(temporary, data, { mode: 0o600 });
      await rename(temporary, join(this.#folder, name));
      files.set(name, { root, bytes: data.length, usedAt: Date.now() });
    } catch (error) {
      this.#failed("write", error);
      // A folder that could not be made holds no temporary file either; the other roots are still written.
      await rm(temporary, { force: true }).catch(() => undefined);
    }
  }

  /** Marks a root's file used, so the bound drops it after roots used longer ago. */
  #used(name: string): void {
    this.#queue(async () => {
      const file = (await this.#opened()).get(name);
      if (file === undefined) return;
      file.usedAt = Date.now();
      // The file's time is what the next daemon's bound goes by.
      const now = new Date(file.usedAt);
      await utimes(join(this.#folder, name), now, now);
    });
  }

  async #remove(files: Map<string, RootFile>, name: string): Promise<void> {
    files.delete(name);
    await rm(join(this.#folder, name), { force: true });
  }

  /** The folder's files, read once: each names its root on its first line, and one whose root is gone, or idle too long, is dropped. */
  async #opened(): Promise<Map<string, RootFile>> {
    if (this.#files !== undefined) return this.#files;
    const files = new Map<string, RootFile>();
    let names: string[];
    try {
      names = await readdir(this.#folder);
    } catch {
      names = [];
    }
    await Promise.all(names.map(async name => {
      const path = join(this.#folder, name);
      try {
        const info = await stat(path);
        if (!name.endsWith(SUFFIX)) {
          if (name.endsWith(".tmp") && Date.now() - info.mtimeMs > LEFTOVER_MS) await rm(path, { force: true });
          return;
        }
        const header = await headerOf(path);
        if (header === undefined || Date.now() - info.mtimeMs > MAX_IDLE_MS || await gone(header.root)) await rm(path, { force: true });
        else files.set(name, { root: header.root, bytes: info.size, usedAt: info.mtimeMs });
      } catch {
        // Unreadable: left as it is, and out of the bound's count.
      }
    }));
    this.#files = files;
    return files;
  }

  /** Drops the roots used longest ago until the folder fits the bound: after each write and removal, not on a read. */
  async #trim(files: Map<string, RootFile>): Promise<void> {
    let bytes = 0;
    for (const file of files.values()) bytes += file.bytes;
    const oldestFirst = [...files].sort(([, left], [, right]) => left.usedAt - right.usedAt);
    for (const [name, file] of oldestFirst) {
      if (bytes <= this.#maxBytes) return;
      bytes -= file.bytes;
      await this.#remove(files, name);
    }
  }
}
