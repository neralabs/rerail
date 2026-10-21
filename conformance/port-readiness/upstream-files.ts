/**
 * Where the generator and the gates find upstream's files and this checkout's: the repository root, the committed
 * runtimes, upstream's licence and its test files at the pinned commit. Nothing here reaches the engine, so the
 * generator (whose import graph is its key, recording/generator-key.ts) reads these without an engine edit re-keying its
 * answers.
 */
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import type { GatesManifest } from "./manifest.js";
import type { GatesRecording } from "./recording/schema.js";

/**
 * The checkout root the generator reads: the same directory as the engine's PACKAGE_ROOT, defined here so the generator
 * never imports the engine barrel and an engine edit does not re-key every tool-pack.
 */
export const REPOSITORY_ROOT = fileURLToPath(new URL("../../", import.meta.url));

/** Where the recorder's runtimes and their pins are committed, relative to the repository root. */
export const RUNTIMES_DIRECTORY = "conformance/recorders/port-readiness/runtimes";

/**
 * A licence file's name: LICENSE or LICENCE in any letter case, bare or with any extension or suffix (`licence.txt`,
 * `license.md`, `LICENSE-MIT`). Every such file at the root of upstream's repository is part of its licence, and the
 * pack keeps each under the same name in licenses/<project>/.
 */
const LICENSE_FILE = /^licen[cs]e(?:[.-].*)?$/iu;

/** The licence files' names (LICENSE_FILE) as non-cone sparse-checkout patterns, for a checkout of upstream's root. */
export const LICENSE_SPARSE_PATTERNS = ["/[Ll][Ii][Cc][Ee][Nn][CcSs][Ee]", "/[Ll][Ii][Cc][Ee][Nn][CcSs][Ee][.-]*"];

/** The licence files directly in `directory`, in name order (code units: `LICENSE-APACHE` before `LICENSE-MIT`). */
export function licenseFiles(directory: string): string[] {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && LICENSE_FILE.test(entry.name))
    .map((entry) => entry.name)
    .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

/** A licence as the gates compare it: `directory`'s licence files, their bytes one after another; null when it has none. */
export function licenseBytes(directory: string): Buffer | null {
  const names = licenseFiles(directory);
  return names.length === 0 ? null : Buffer.concat(names.map((name) => readFileSync(join(directory, name))));
}

/** Where the pack keeps upstream's licence and UPSTREAM.md, relative to the repository root. */
export function keptLicenseDirectory(manifest: Pick<GatesManifest, "rerail">): string {
  return `packs/${manifest.rerail.pack}/licenses/${manifest.rerail.project}`;
}

/** Upstream's licence text at the root of its checkout at the pinned commit (licenseBytes), as a live adapter records it. */
export function upstreamLicenseText(checkout: string): string {
  const bytes = licenseBytes(checkout);
  if (bytes === null) throw new Error(`upstream's checkout at ${checkout} has no LICENSE or LICENCE file at its root`);
  return bytes.toString("utf8");
}

/**
 * A committed case source as upstream's adapter decoded it: UTF-8, or, for bytes that are not UTF-8, the Latin-1 a PEP 263
 * coding line declares (Bandit's trojansource_latin1.py); recorded parser verdicts are keyed by the decoded text.
 */
export function decodeSource(bytes: Buffer): string {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    const head = bytes.subarray(0, 400).toString("latin1").split(/\r?\n/u).slice(0, 2).join("\n");
    const coding = /coding[:=][ \t]*([-\w.]+)/u.exec(head)?.[1]?.toLowerCase();
    if (coding !== undefined && /^(latin-?1|iso-?8859-1|l1|cp819)$/u.test(coding.replaceAll("_", "-"))) return bytes.toString("latin1");
    return bytes.toString("utf8");
  }
}

/**
 * The code cells of a notebook joined, as the words of upstream's notebook test files; a text that is not a notebook is
 * returned unchanged. The Ruff adapter's `notebook-code-cells` transform, shared so the gates rebuild its texts exactly.
 */
export function notebookCodeCells(text: string): string {
  try {
    const notebook = JSON.parse(text) as { cells?: { cell_type?: string; source?: string | string[] }[] };
    return (notebook.cells ?? []).filter((cell) => cell.cell_type === "code")
      .map((cell) => (Array.isArray(cell.source) ? cell.source.join("") : cell.source ?? "")).join("\n");
  } catch {
    return text;
  }
}

export type RecordedFixtureTexts = NonNullable<GatesRecording["fixtureTexts"]>;

/** Where the gates and the recorder keep upstream's checkout at the pinned commit: `<cache>/upstream/<repository name>/<commit>`. */
export function upstreamCheckoutDirectory(cacheRoot: string, pins: { repository: string; commit: string }): string {
  return resolve(cacheRoot, "upstream", pins.repository.slice(pins.repository.lastIndexOf("/") + 1), pins.commit);
}

/**
 * fixtureTexts rebuilt from a checkout of upstream at the pinned commit: each recorded file read (UTF-8, as the live
 * adapters read it), its transform applied and its sha256 checked, then the inline texts. A missing file or another
 * digest throws: the answers are stale, and a silent narrowing would let not-overfit pass what it should not.
 */
export function rebuildFixtureTexts(checkout: string, recorded: RecordedFixtureTexts): string[] {
  const texts = recorded.files.map(({ path, sha256: digest, transform }) => {
    const file = join(checkout, path);
    if (!existsSync(file)) throw new Error(`upstream's ${path} is missing from the checkout at ${checkout}`);
    const raw = readFileSync(file, "utf8");
    const text = transform === "notebook-code-cells" ? notebookCodeCells(raw) : raw;
    const actual = createHash("sha256").update(text).digest("hex");
    if (actual !== digest) throw new Error(`upstream's ${path}${transform === undefined ? "" : ` (${transform})`} has sha256 ${actual.slice(0, 12)}, recorded ${digest.slice(0, 12)}: the answers are stale`);
    return text;
  });
  return [...texts, ...recorded.inline.map(({ text }) => text)];
}
