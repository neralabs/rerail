/**
 * A sample set as its canonical archive (test-data-archive.mjs: the same files always make the same tar) and the
 * files committed for it: `samples.lock` naming the tar by sha256, file count and bytes, and `SAMPLES.md` beside it.
 * The compressed archive goes to the cache path pack-fixtures.mjs fetches samples into and pushes them from.
 */
import { mkdirSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

import { buildArchive, compressArchive, lockText } from "../s3/test-data-archive.mjs";
import type { ToolPack } from "./samples.js";

export const SAMPLES_LOCK = "samples.lock";
export const SAMPLES_NOTE = "SAMPLES.md";

export interface PackedSamples {
  sha256: string;
  files: number;
  bytes: number;
  compressed: Buffer;
}

export function packSamples(entries: readonly { path: string; bytes: Buffer }[]): PackedSamples {
  const archive = buildArchive(entries);
  return { sha256: archive.sha256, files: archive.files, bytes: archive.bytes, compressed: compressArchive(archive.tar) };
}

/** `.rerail/pack-data/archives/samples/<pack>/<tool>/<sha256>.tar.zst`: where pack-fixtures.mjs caches a samples archive. */
export function samplesArchivePath(root: string, { pack, tool }: ToolPack, digest: string): string {
  return join(root, ".rerail", "pack-data", "archives", "samples", pack, tool, `${digest}.tar.zst`);
}

const writeAtomically = (path: string, bytes: Buffer | string): void => {
  mkdirSync(dirname(path), { recursive: true });
  const temporary = `${path}.${process.pid}.tmp`;
  writeFileSync(temporary, bytes);
  renameSync(temporary, path);
};

/** Writes the cached archive, then the tool-pack's samples.lock and SAMPLES.md. */
export function writeSampleSet(root: string, toolPack: ToolPack, packed: PackedSamples, note: string): void {
  writeAtomically(samplesArchivePath(root, toolPack, packed.sha256), packed.compressed);
  const directory = join(root, "packs", toolPack.pack, "upstream", toolPack.tool);
  writeAtomically(join(directory, SAMPLES_LOCK), lockText({ sha256: packed.sha256, files: packed.files, bytes: packed.bytes }));
  writeAtomically(join(directory, SAMPLES_NOTE), note);
}
