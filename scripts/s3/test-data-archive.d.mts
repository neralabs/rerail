// Types of test-data-archive.mjs, for its TypeScript consumers (conformance/port-readiness/recording/corpus-answers.ts).
export interface ArchiveEntry {
  path: string;
  bytes: Buffer;
}
export interface Archive {
  tar: Buffer;
  sha256: string;
  files: number;
  bytes: number;
}

export const TEST_DATA_LOCK: string;
export const LOCK_FORMAT: number;
export function checkEntryPath(path: string): void;
export function buildArchive(entries: readonly ArchiveEntry[]): Archive;
export function sha256(buffer: Buffer | string): string;
export function compressArchive(tar: Buffer): Buffer;
export function archiveSizeBound(size: { files: number; bytes: number }): number;
export function decompressArchive(compressed: Buffer, maxLength?: number): Buffer;
export function readArchive(tar: Buffer): ArchiveEntry[];
export function lockText(lock: { sha256: string; files: number; bytes: number }): string;
export function parseLock(text: string, label: string): { sha256: string; files: number; bytes: number };
