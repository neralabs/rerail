import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

/**
 * One golden test of `matches-upstream-tests`: a real-code file whose source is the pinned corpus file at `task`/`path`
 * (a catalog task, or a rerail corpus as `rerail:<name>`), checked against `sha256`. Its expected answer is not stored
 * here: it is the pinned upstream's findings on that file, in the tool-pack's corpus answers archive.
 */
export interface Golden {
  task: string;
  path: string;
  sha256: string;
}

/**
 * A tool-pack's goldens, by document id: a reviewed list the gates read and never write. A rule's goldens are the files
 * a real-code run found it differing on, and, when no upstream test file proves a shipped rule stays silent, a file
 * where upstream reports nothing. Which run found them is the file's `# Source:` line: NRV's real-code gate (its
 * validation/goldens/<pack>.tsv, under its id there) for most tool-packs, rerail's own for the ones NRV pins no tool
 * of. A candidate keeps its goldens until it agrees there, unless it differs on so many files that they only restate
 * that it is wrong throughout: then gates.yaml lists it under real_code_gaps instead (SYSTEMATIC_FILES).
 */
export type Goldens = Map<string, Golden[]>;

/**
 * A candidate whose goldens it differs on number more than this differs from upstream throughout, not on cases: one
 * real_code_gaps entry replaces its goldens. The candidates over it differ on 734 to 6,898 files, the rest on 335 at
 * most.
 */
export const SYSTEMATIC_FILES = 500;

/** A tool-pack's goldens file: `goldens.tsv` in its directory, `packs/<pack>/upstream/<tool>/`. */
export const GOLDENS_FILE = "goldens.tsv";

export function goldensPath(toolPackDirectory: string): string {
  return join(toolPackDirectory, GOLDENS_FILE);
}

const HEADER = "# matches-upstream-tests goldens, reviewed, never written by the gates: a rule id, then each task or rerail corpus indented once, then each file indented twice with its sha256, sorted.";
/** The second line: where the goldens come from, in words a reviewer can check. */
const SOURCE = "# Source: ";
const SHA256 = /^[0-9a-f]{64}$/u;
const FIELD = /^[^\t\r\n]+$/u;

/** Code-unit order, the same on every machine (localeCompare depends on the ICU and locale). */
export const byText = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);
const byPlace = (a: Golden, b: Golden) => byText(a.task, b.task) || byText(a.path, b.path) || byText(a.sha256, b.sha256);

/**
 * The goldens file's text: its header, `# Source: <source>`, then one golden per line, grouped by rule and then by task
 * or corpus, every level sorted, so the file has one form and an imported golden is one added line.
 */
export function serializeGoldens(goldens: Goldens, source: string): string {
  if (!FIELD.test(source)) throw new Error(`a goldens file's source cannot be written: ${JSON.stringify(source)}`);
  const lines = [HEADER, `${SOURCE}${source}`];
  for (const id of [...goldens.keys()].sort(byText)) {
    const entries = [...goldens.get(id)!].sort(byPlace);
    if (entries.length === 0) continue;
    if (!FIELD.test(id)) throw new Error(`a golden's rule id cannot be written: ${JSON.stringify(id)}`);
    lines.push(id);
    let task: string | undefined;
    for (const golden of entries) {
      if (![golden.task, golden.path].every((field) => FIELD.test(field)) || !SHA256.test(golden.sha256)) throw new Error(`a golden of ${id} cannot be written: ${JSON.stringify(golden)}`);
      if (golden.task !== task) lines.push(`\t${golden.task}`);
      task = golden.task;
      lines.push(`\t\t${golden.path}\t${golden.sha256}`);
    }
  }
  return `${lines.join("\n")}\n`;
}

/** The goldens a file's text holds; it must be exactly what serializeGoldens writes, its source line included. */
export function parseGoldens(text: string, label: string): Goldens {
  const goldens: Goldens = new Map();
  let id: string | undefined;
  let task: string | undefined;
  const lines = text.split("\n");
  const source = lines[1]?.startsWith(SOURCE) === true ? lines[1].slice(SOURCE.length) : "";
  if (source === "") throw new Error(`${label}:2: no "${SOURCE}<where the goldens come from>" line`);
  lines.forEach((line, index) => {
    const at = `${label}:${index + 1}`;
    if (line === "" || index < 2) return;
    if (!line.startsWith("\t")) {
      if (!FIELD.test(line)) throw new Error(`${at}: not a rule id`);
      id = line;
      task = undefined;
      goldens.set(id, goldens.get(id) ?? []);
    } else if (!line.startsWith("\t\t")) {
      if (id === undefined || !FIELD.test(line.slice(1))) throw new Error(`${at}: a task or corpus outside a rule`);
      task = line.slice(1);
    } else {
      const fields = line.slice(2).split("\t");
      if (id === undefined || task === undefined || fields.length !== 2 || !FIELD.test(fields[0]!) || !SHA256.test(fields[1]!)) throw new Error(`${at}: not a golden (path, then sha256)`);
      goldens.get(id)!.push({ task, path: fields[0]!, sha256: fields[1]! });
    }
  });
  if (serializeGoldens(goldens, source) !== text) throw new Error(`${label} is not in its one form (sorted, grouped, with its header and source line)`);
  return goldens;
}

/** The tool-pack's goldens, or none when it has no goldens file yet; `label` names the file in errors. */
export async function readGoldens(toolPackDirectory: string, label: string = GOLDENS_FILE): Promise<Goldens> {
  const path = goldensPath(toolPackDirectory);
  return existsSync(path) ? parseGoldens(await readFile(path, "utf8"), label) : new Map();
}
