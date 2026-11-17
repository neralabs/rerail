/**
 * Ruff's rule selection, read as data from the config Ruff uses at the root (`.ruff.toml`, `ruff.toml` or
 * `pyproject.toml` `[tool.ruff]`) and the configs its `extend` names in the repository (`ruff-config-file.ts`,
 * `ruff-selection.ts`), and the verdict it gives on each port's noqa codes (`ruff-rule-decision.ts`), with the per-file
 * ignores that turn a code off in whole folders. What an extended config Rerail does not read may decide is unknown, as
 * is every rule in a folder below the root holding a Ruff config of its own, which Ruff uses there instead.
 */
import { combineDecisions } from "../linter-decisions.js";
import {
  type FolderDecision,
  inFolder,
  type LinterConfig,
  type LinterNote,
  type LinterReader,
  type Port,
  type PortDecision,
  type PortVerdict,
} from "../linter-config.js";
import { RUFF_RULES, type RuffRuleInfo } from "../generated/ruff-rules.generated.js";
import { repositoryFiles } from "../repository-files.js";
import { extendChain, folderOf, readRuffFile, RUFF_FILES, type RuffPerFileIgnore } from "./ruff-config-file.js";
import { codeDecision, RUFF_VERSION } from "./ruff-rule-decision.js";
import { selectionOf, type RuffSelection } from "./ruff-selection.js";
import { ruffSelects, ruffText } from "./ruff-selectors.js";

/** Ruff's rule selection at `root`; undefined when there is no Ruff config, or (with a note) when it cannot be read or Ruff refuses it. */
export function readRuffSelection(root: string, notes: LinterNote[]): RuffSelection | undefined {
  for (const name of RUFF_FILES) {
    const first = readRuffFile(root, name, notes, false);
    if (first === "absent" || first === "not-ruff") continue;
    if (!("name" in first)) {
      notes.push({ linter: "ruff", text: `${name} is not read: ${first.why}.` });
      return undefined;
    }
    const { chain, end } = extendChain(root, first, notes);
    return selectionOf(chain, end, notes);
  }
  return undefined;
}

/** A note where a config selects preview rule `code` by its code and Ruff does not run it, preview being off. */
function previewNotes(ruleId: string, selection: RuffSelection, code: string, info: RuffRuleInfo): string[] {
  if (info.status !== "preview" || selection.preview !== false) return [];
  const named = selection.layers.flatMap(layer => [...layer.select ?? [], ...layer.extendSelect]).find(by => by.selector === code);
  return named === undefined ? [] : [`${ruleId}: ${ruffText(named)} selects a preview rule, which Ruff runs only with preview = true; it is not on.`];
}

/** A note where an ignore is overridden by a more specific select. */
function overriddenNotes(ruleId: string, selection: RuffSelection, code: string, decision: PortDecision): string[] {
  if (decision.on !== true || !decision.explicit) return [];
  const overridden = selection.layers.flatMap(layer => layer.ignore).find(ignored => ruffSelects(ignored.selector, code));
  return overridden === undefined ? [] : [`${ruleId}: ${ruffText(overridden)} is overridden by ${decision.because}, which Ruff applies after it; it stays on.`];
}

/** What a code's per-file ignores decide: for every file where one covers every file, per folder, and notes. */
interface PerFileVerdict {
  readonly whole?: PortDecision;
  readonly folders: FolderDecision[];
  readonly notes: string[];
}

/** Whether folder `folder` is `ancestor` or below it. */
const within = (folder: string, ancestor: string): boolean => ancestor === "" || folder === ancestor || folder.startsWith(`${ancestor}/`);

/**
 * What `!pattern` per-file ignores of `code` decide: off in every file but those the patterns match, so off for every
 * file, back to `decision` in the one folder all of them leave out (unless a plain per-file ignore covers it); unknown
 * where a pattern leaves out narrower files than whole folders, as the code is off in most files and on in some.
 */
function negatedVerdict(negated: readonly RuffPerFileIgnore[], ignored: readonly string[], decision: PortDecision): Pick<PerFileVerdict, "whole" | "folders"> {
  const kept = negated.filter(entry => entry.folder !== "");
  if (kept.length === 0) return { folders: [] };
  const named = kept.map(entry => ruffText(entry.ignore)).join("; ");
  if (kept.some(entry => entry.folder === undefined)) return { whole: { on: "unknown", because: `${named} ignores it in every file but some, which Rerail does not place in whole folders` }, folders: [] };
  const deepest = kept.map(entry => entry.folder!).sort((left, right) => right.length - left.length)[0]!;
  const spared = kept.every(entry => within(deepest, entry.folder!)) && !ignored.some(folder => within(deepest, folder));
  const back = decision.on === false || !spared ? [] : [inFolder(deepest, { ...decision, because: `${named} leaves it to ${decision.because}` })];
  return { whole: { on: false, explicit: true, because: `${named} ignores it in every file outside ${kept.map(entry => `${entry.folder!}/`).join(", ")}` }, folders: back };
}

/** What the per-file ignores of `code` decide: off in the folders a pattern covers whole, notes for narrower patterns. */
function perFileVerdict(ruleId: string, selection: RuffSelection, code: string, decision: PortDecision): PerFileVerdict {
  const naming = selection.perFileIgnores.filter(entry => ruffSelects(entry.ignore.selector, code));
  const plain = naming.filter(entry => !entry.pattern.startsWith("!"));
  const everyFile = plain.find(entry => entry.folder === "");
  const folders = plain.flatMap(entry => (entry.folder === undefined || entry.folder === "" ? [] : [{ folder: entry.folder, on: false, explicit: true, because: ruffText(entry.ignore) }]));
  const notes = plain.filter(entry => entry.folder === undefined).map(entry => `${ruleId}: ${ruffText(entry.ignore)} ignores it only in the files ${JSON.stringify(entry.pattern)} matches; it stays on.`);
  if (everyFile !== undefined) return { whole: { on: false, explicit: true, because: ruffText(everyFile.ignore) }, folders, notes };
  const negated = negatedVerdict(naming.filter(entry => entry.pattern.startsWith("!")), folders.map(folder => folder.folder), decision);
  return { ...negated, folders: [...folders, ...negated.folders], notes };
}

/** What the selection says about one of a port's codes. */
function codeVerdict(ruleId: string, selection: RuffSelection, code: string): PortVerdict & { readonly folders: readonly FolderDecision[]; readonly notes: readonly string[] } {
  const info = RUFF_RULES.get(code);
  if (info === undefined) return { folders: [], notes: [`${ruleId}: ${JSON.stringify(code)} is not a rule of Ruff ${RUFF_VERSION}; Ruff's config is not used for it.`] };
  const decision = codeDecision(selection, code, info);
  const perFile = perFileVerdict(ruleId, selection, code, decision);
  return {
    decision: perFile.whole ?? decision,
    folders: perFile.folders,
    notes: [...previewNotes(ruleId, selection, code, info), ...overriddenNotes(ruleId, selection, code, decision), ...perFile.notes],
  };
}

/** A folder below the root holding a config Ruff uses there instead of the root's, with that config. */
interface NestedConfig {
  readonly folder: string;
  readonly file: string;
}

/** The folders below the root holding a Ruff config (the first of `RUFF_FILES` there that is one), each with it. */
function nestedConfigs(root: string): NestedConfig[] {
  const byFolder = new Map<string, string[]>();
  for (const path of repositoryFiles(root)) {
    const name = path.split("/").at(-1)!;
    if (path.includes("/") && RUFF_FILES.includes(name)) byFolder.set(folderOf(path), [...byFolder.get(folderOf(path)) ?? [], path]);
  }
  return [...byFolder].flatMap(([folder, paths]) => {
    const file = RUFF_FILES.map(name => `${folder}/${name}`).find(path => paths.includes(path) && readRuffFile(root, path, [], false) !== "not-ruff");
    return file === undefined ? [] : [{ folder, file }];
  });
}

/** The verdict of Ruff's selection on `port`, over its noqa codes, every rule unknown in a folder with a Ruff config of its own. */
function ruffVerdict(selection: RuffSelection, nested: readonly NestedConfig[], port: Port): PortVerdict | undefined {
  if (port.suppress.noqa.length === 0) return undefined;
  const verdicts = port.suppress.noqa.map(code => codeVerdict(port.ruleId, selection, code));
  const decision = combineDecisions(verdicts.flatMap(verdict => (verdict.decision === undefined ? [] : [verdict.decision])));
  // Ruff uses a folder's own config for its files, so nothing the root's decides applies in it.
  const unknown = decision === undefined ? [] : nested.map(({ folder, file }) => ({ folder, on: "unknown" as const, because: `${file}, which Rerail does not read, is Ruff's config for ${folder}/` }));
  return {
    ...(decision === undefined ? {} : { decision }),
    folders: [...verdicts.flatMap(verdict => verdict.folders).filter(({ folder }) => !nested.some(config => within(folder, config.folder))), ...unknown],
    notes: verdicts.flatMap(verdict => verdict.notes),
  };
}

export const ruffReader: LinterReader = {
  linter: "ruff",
  kinds: ["noqa"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const selection = readRuffSelection(root, notes);
    if (selection === undefined) return undefined;
    const nested = nestedConfigs(root);
    for (const { folder, file } of nested) {
      notes.push({ linter: "ruff", text: `${file} is not read: Ruff uses it, not ${selection.file}, for the files in ${folder}/, so the rules Rerail ports from Ruff keep the pack's values there.` });
    }
    return { verdict: port => ruffVerdict(selection, nested, port) };
  },
};
