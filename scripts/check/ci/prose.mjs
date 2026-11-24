#!/usr/bin/env node
// Whether a pull request changes prose alone: pending changesets (.changeset/*.md) and changelog entries
// (.changelog/*.md), the Markdown at the repository's root, and docs/'s Markdown less the rule syntax's (docs/rule-syntax/,
// an input of the engine and the translation suite: ci.yml's rust and translations filters). Prose changes no code: it
// is read as text, checks out as the same LF bytes on every platform (.gitattributes), and check-windows-paths.mjs (the
// lint job) keeps its file names valid on Windows. So it changes nothing a Windows run tests that a Linux run does not,
// and nothing a pack is validated or packed from. Two jobs read it:
//   - ci.yml's select-tests (test-selection.mjs) gives a prose-only change no Windows shard; the Linux unit job still
//     runs every test the selection finds reading it;
//   - release-packs.yml runs no pack dry run on a prose-only pull request; its Changesets job still checks the
//     changesets, and the dry run reads none.
// Markdown elsewhere is not prose here: the packs', skills', hooks' and plugins' ship in packages, and the fixtures'
// and conformance's are test inputs, some pinned byte for byte (.gitattributes). An empty list is not prose-only: a
// listing that came back empty runs everything.
//
//   node scripts/check/ci/prose.mjs <changed-files.txt>   `prose=true|false` for $GITHUB_OUTPUT
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** Whether `path` (repository-relative, `/`-separated) is prose. */
export function isProse(path) {
  if (!path.endsWith(".md")) return false;
  const [top, ...rest] = path.split("/");
  if (rest.length === 0) return true;
  if (top === ".changeset" || top === ".changelog") return rest.length === 1;
  return top === "docs" && rest[0] !== "rule-syntax";
}

/** Whether every one of `changed`, a non-empty list, is prose. */
export function proseOnly(changed) {
  return changed.length > 0 && changed.every(isProse);
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const changed = readFileSync(process.argv[2], "utf8").split("\n").map((line) => line.trim()).filter(Boolean);
  console.log(`prose=${proseOnly(changed)}`);
}
