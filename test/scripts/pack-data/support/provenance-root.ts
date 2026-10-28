/**
 * A scratch checkout for scripts/pack-data/provenance.mjs: pack `demo`, tool `tool`, one upstream project `proj` with
 * its tracked record (packs/demo/licenses/proj/UPSTREAM.md and LICENSE) and the tool's pack test data on disk.
 */
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

export const PACK = "demo";
export const TOOL = "tool";
export const PROJECT = "proj";
export const COMMIT = "0123456789abcdef0123456789abcdef01234567";
export const TOOL_DIRECTORY = `packs/${PACK}/upstream/${TOOL}`;
export const LICENCE_TEXT = "MIT License\n\nCopyright (c) the proj authors\n";

export interface ScratchCheckout {
  /** The Source row's tag cell (`—` for none). */
  tag?: string;
  /** The "Taken from it" pack-path globs. */
  taken?: string[];
  /** The "Not copied" upstream prefixes. */
  notCopied?: string[];
  /** Tool-relative paths of the pack test data on disk. */
  files?: string[];
  /** Tool-relative paths the tool's own UPSTREAM.md lists as Rerail's. */
  authored?: string[];
}

const DEFAULT_FILES = ["fixtures/a.js", "fixtures/B.js", "fixtures/renamed/a.js", "fixtures.expected.json", "authored/own.js"];

function write(root: string, path: string, text: string): void {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), text);
}

function projectRecord(tag: string, taken: string[], notCopied: string[]): string {
  return [
    "# proj 1.0.0",
    "",
    "License: MIT (Copyright (c) the proj authors); [`LICENSE`](LICENSE) is the upstream license file.",
    "",
    "## Source",
    "",
    "| Repository | Tag | Commit | Licence |",
    "|---|---|---|---|",
    `| https://github.com/example/proj | ${tag === "—" ? "—" : `\`${tag}\``} | \`${COMMIT}\` | MIT |`,
    "",
    "## Taken from it",
    "",
    "| Pack paths | Upstream paths | How |",
    "|---|---|---|",
    ...taken.map((glob) => `| \`${glob}\` | \`tests/**\` | verbatim |`),
    "",
    ...notCopied.map((prefix) => `- Not copied: \`${prefix}\` — a copyleft plugin`),
    "",
  ].join("\n");
}

/** A fresh scratch checkout's root. */
export function scratchCheckout({ tag = "v1.0.0", taken = [`upstream/${TOOL}/fixtures/**`], notCopied = [], files = DEFAULT_FILES, authored = [] }: ScratchCheckout = {}): string {
  const root = mkdtempSync(join(tmpdir(), "pack-provenance-"));
  write(root, `packs/${PACK}/licenses/${PROJECT}/UPSTREAM.md`, projectRecord(tag, taken, notCopied));
  write(root, `packs/${PACK}/licenses/${PROJECT}/LICENSE`, LICENCE_TEXT);
  for (const file of files) write(root, `${TOOL_DIRECTORY}/${file}`, `// ${file}\n`);
  if (authored.length > 0) {
    write(root, `${TOOL_DIRECTORY}/UPSTREAM.md`, ["# tool", "", "| File | Why |", "|---|---|", ...authored.map((file) => `| \`upstream/${TOOL}/${file}\` | Rerail's |`), ""].join("\n"));
  }
  return root;
}
