import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

/**
 * One golden test of `matches-upstream-tests`: a real-code file where `matches-real-code` found the rule and upstream
 * differ. Its source is the pinned corpus file at `task`/`path` (a catalog task, or a rerail corpus as
 * `rerail:<name>`), checked against `sha256`. Its expected answer is not stored here: it is the pinned upstream's
 * findings on that file.
 */
export interface Golden {
  task: string;
  path: string;
  sha256: string;
}

/** A pack's goldens, by document id. Goldens are never removed: once promoted, a case stays a regression test. */
export type Goldens = Map<string, Golden[]>;

/** A tool-pack's goldens file: `goldens.tsv` in its directory, `packs/<pack>/upstream/<tool>/`. */
export const GOLDENS_FILE = "goldens.tsv";

export function goldensPath(toolPackDirectory: string): string {
  return join(toolPackDirectory, GOLDENS_FILE);
}

const HEADER = "# matches-upstream-tests goldens: a rule id, then each task or rerail corpus indented once, then each file indented twice with its sha256. Written by npm run port-readiness -- --write, sorted; a golden is never removed.";
const SHA256 = /^[0-9a-f]{64}$/u;
const FIELD = /^[^\t\r\n]+$/u;

/** Code-unit order, the same on every machine (localeCompare depends on the ICU and locale). */
export const byText = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);
const byPlace = (a: Golden, b: Golden) => byText(a.task, b.task) || byText(a.path, b.path) || byText(a.sha256, b.sha256);

/**
 * The goldens file's text: one golden per line, grouped by rule and then by task or corpus, every level sorted, so a
 * run writes it the same way every time and a new golden is one added line.
 */
export function serializeGoldens(goldens: Goldens): string {
  const lines = [HEADER];
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

/** The goldens a file's text holds; it must be exactly what serializeGoldens writes. */
export function parseGoldens(text: string, label: string): Goldens {
  const goldens: Goldens = new Map();
  let id: string | undefined;
  let task: string | undefined;
  text.split("\n").forEach((line, index) => {
    const at = `${label}:${index + 1}`;
    if (line === "" || (index === 0 && line === HEADER)) return;
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
  if (serializeGoldens(goldens) !== text) throw new Error(`${label} is not in the form npm run port-readiness writes (sorted, grouped, with its header)`);
  return goldens;
}

/** The tool-pack's goldens, or none when it has no goldens file yet; `label` names the file in errors. */
export async function readGoldens(toolPackDirectory: string, label: string = GOLDENS_FILE): Promise<Goldens> {
  const path = goldensPath(toolPackDirectory);
  return existsSync(path) ? parseGoldens(await readFile(path, "utf8"), label) : new Map();
}

/** Add each differing file the rule has no golden for yet, a file being its task, path and content (sha256). Returns the goldens added. */
export function promote(goldens: Goldens, ruleId: string, differing: Golden[]): Golden[] {
  const entries = goldens.get(ruleId) ?? [];
  const added = differing.filter((golden) => !entries.some((entry) => entry.task === golden.task && entry.path === golden.path && entry.sha256 === golden.sha256));
  if (added.length > 0) goldens.set(ruleId, [...entries, ...added].sort(byPlace));
  return added;
}
