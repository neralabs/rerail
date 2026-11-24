import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { IGNORED_DIRECTORIES } from "../../../src/engine/index.js";

/**
 * The engine's copy of the host's `IGNORED_DIRECTORIES` (src/engine/code-rule/ignored-directories.ts): the folders no
 * repository walk enters, as JSON data. The engine reads it (`git_workspace::ignored_directories`) for its corpus
 * listing, its artifact inputs, its `file` facts and its server graphs, so a `.rerail` run reads the files a host's
 * walk reads.
 */

const ROOT = resolve(fileURLToPath(import.meta.url), "..", "..", "..", "..");
export const IGNORED_DIRECTORIES_DATA_PATH = resolve(ROOT, "native", "crates", "observe", "src", "ignored_directories.generated.json");

export function renderIgnoredDirectoriesData(names: Iterable<string> = IGNORED_DIRECTORIES): string {
  return `${JSON.stringify([...names], null, 2)}\n`;
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  writeFileSync(IGNORED_DIRECTORIES_DATA_PATH, renderIgnoredDirectoriesData());
}
