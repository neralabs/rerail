/**
 * Ruff's rule selection, read as data from the config Ruff uses at the root (`.ruff.toml`, `ruff.toml` or
 * `pyproject.toml` `[tool.ruff]`) and the configs its `extend` names in the repository: `select`, `extend-select`,
 * `ignore`, `extend-ignore`, `per-file-ignores`, `extend-per-file-ignores`, `preview` and `explicit-preview-rules`, each
 * from the `lint` section where it is set there, else from the top level, as Ruff reads them.
 *
 * A port's noqa codes are decided as Ruff 0.16.9 decides them (`as_rule_table` in ruff_workspace's
 * `configuration.rs`): Ruff's default selection (`DEFAULT_SELECTORS` in ruff_linter's `settings/mod.rs`, or the
 * default categories with preview), then each config's selectors from the extended one to the root's. A code a selector
 * decides is explicit; one none decides follows the default, an implicit on or off. A preview rule runs only with
 * preview (and, with `explicit-preview-rules`, only when selected by its code). What an extended config Rerail does not
 * read may decide is unknown.
 */
import { folderOfGlob } from "./eslint-flat-config.js";
import { readFormat, readTomlStringLists, type TomlStringLists } from "./formats/lint-config-formats.js";
import { combineDecisions } from "./linter-decisions.js";
import {
  NOT_A_FILE,
  readRootFile,
  type FolderDecision,
  type LinterConfig,
  type LinterNote,
  type LinterReader,
  type Port,
  type PortDecision,
  type PortVerdict,
} from "./linter-config.js";
import { RUFF_RULES, type RuffRuleInfo } from "./generated/ruff-rules.generated.js";

/** The Ruff release whose rules and defaults `RUFF_RULES` holds (packs/python/upstream/ruff/gates.yaml). */
const RUFF_VERSION = "0.16.9";
/** Ruff's config files in the order of precedence when one folder has several. */
const RUFF_FILES = [".ruff.toml", "ruff.toml", "pyproject.toml"];
/** Ruff's rule selection lists; each is read from the `lint` section, else from the top level as Ruff does. */
const RUFF_LISTS = ["select", "extend-select", "ignore", "extend-ignore"] as const;
const RUFF_TABLES = ["per-file-ignores", "extend-per-file-ignores"] as const;
const RUFF_SCALARS = ["preview", "explicit-preview-rules"] as const;
/** Ruff's rule categories (`Category` in ruff_linter's `codes.rs`), selectors only with preview. */
const RUFF_CATEGORIES = new Set([...RUFF_RULES.values()].map(rule => rule.category));
/** A Ruff selector: `ALL`, a rule code or prefix of one (`E`, `E7`, `E711`, `PLR09`), or a category. */
const RUFF_CODE_SELECTOR = /^(?:ALL|[A-Z]+[0-9]*)$/u;

/** A Ruff selector in one of the rule selection options, with the file and key path that hold it. */
export interface RuffSelector {
  readonly file: string;
  readonly key: string;
  readonly selector: string;
}

/** One config file's rule selection, as Ruff reads it into one rule selection. */
export interface RuffLayer {
  readonly file: string;
  /** `select`, which replaces every earlier selection; undefined where the file does not set it. */
  readonly select: readonly RuffSelector[] | undefined;
  /** The key path of `select`, where the file sets it. */
  readonly selectKey?: string;
  readonly extendSelect: readonly RuffSelector[];
  /** `ignore` and `extend-ignore`. */
  readonly ignore: readonly RuffSelector[];
}

type RuffPerFileIgnore = { readonly pattern: string; readonly folder: string | undefined; readonly ignore: RuffSelector };

/** Ruff's rule selection in the config Ruff uses at the root. */
export interface RuffSelection {
  /** The config Ruff uses at the root. */
  readonly file: string;
  /** Each config's selection in the order Ruff applies them: the configs `extend` names first, the root's last. */
  readonly layers: readonly RuffLayer[];
  /** `per-file-ignores` and `extend-per-file-ignores`: a selector ignored in the files a pattern matches. */
  readonly perFileIgnores: readonly RuffPerFileIgnore[];
  /** Whether preview is on for the linter; undefined when no config read sets it and one not read may. */
  readonly preview: boolean | undefined;
  /** `explicit-preview-rules`; undefined as `preview`. */
  readonly explicitPreviewRules: boolean | undefined;
  /** The extended config Rerail does not read, which may select or ignore any rule: how the person knows it. */
  readonly unread?: string;
}

/** A Ruff setting a config sets: `true`, `false`, or undefined where it does not set it. */
type Flag = boolean | undefined;

/** One Ruff config file as read: its rule selection, its per-file ignores, its flags, and the config it extends. */
interface RuffFile {
  readonly name: string;
  readonly layer: RuffLayer;
  /** `per-file-ignores`, which replaces the extended config's; undefined where the file does not set it. */
  readonly perFileIgnores: readonly RuffPerFileIgnore[] | undefined;
  readonly extendPerFileIgnores: readonly RuffPerFileIgnore[];
  /** `lint.preview`, the top-level `preview`, and `explicit-preview-rules`, each as the file sets it. */
  readonly lintPreview: Flag;
  readonly preview: Flag;
  readonly explicitPreviewRules: Flag;
  readonly extend: string | undefined;
}

/** How many configs deep `extend` is followed; Ruff has no limit, but a chain this long is not one a person wrote. */
const MAX_RUFF_EXTENDS = 8;

/** What one file holds, as `readTomlStringLists` found it, with the prefix of its Ruff section. */
interface RuffToml {
  readonly name: string;
  readonly prefix: string;
  readonly found: TomlStringLists;
  readonly notes: LinterNote[];
}

const sectionsOf = (prefix: string): string[] => [`${prefix}lint.`, prefix];

/** Whether `selector` is one Ruff takes: `ALL`, a code or prefix, or a category. */
const isRuffSelector = (selector: string): boolean => RUFF_CODE_SELECTOR.test(selector) || RUFF_CATEGORIES.has(selector);

/** The selectors of `option`, from the lint section where it is set there, else the top level; undefined when unset. */
function selectorsOf(toml: RuffToml, option: string): { key: string; selectors: RuffSelector[] } | undefined {
  const key = sectionsOf(toml.prefix).map(section => `${section}${option}`).find(path => toml.found.lists.has(path));
  if (key === undefined) return undefined;
  const selectors = toml.found.lists.get(key)!.flatMap(selector => {
    if (isRuffSelector(selector)) return [{ file: toml.name, key, selector }];
    toml.notes.push({ linter: "ruff", text: `${toml.name}: ${JSON.stringify(selector)} in ${key} is not a Ruff rule code or prefix.` });
    return [];
  });
  return { key, selectors };
}

/** `per-file-ignores` or `extend-per-file-ignores` as read; undefined when unset. */
function perFileIgnoresOf(toml: RuffToml, option: string): RuffPerFileIgnore[] | undefined {
  const key = sectionsOf(toml.prefix).map(section => `${section}${option}`).find(path => toml.found.tables.has(path));
  if (key === undefined) return undefined;
  return [...toml.found.tables.get(key)!].flatMap(([pattern, ignored]) => ignored.flatMap(selector => {
    if (!isRuffSelector(selector)) {
      toml.notes.push({ linter: "ruff", text: `${toml.name}: ${JSON.stringify(selector)} in ${key} is not a Ruff rule code or prefix.` });
      return [];
    }
    return [{ pattern, folder: pattern.startsWith("!") ? undefined : folderOfGlob(pattern), ignore: { file: toml.name, key: `${key}.${JSON.stringify(pattern)}`, selector } }];
  }));
}

/** A boolean setting at `path`; undefined when unset, or (with a note) when it is not `true` or `false`. */
function flagOf(toml: RuffToml, path: string): Flag {
  const value = toml.found.scalars.get(path);
  if (value === undefined) return undefined;
  if (value === "true" || value === "false") return value === "true";
  toml.notes.push({ linter: "ruff", text: `${toml.name}: ${path} = ${value} is not true or false; not read.` });
  return undefined;
}

/** The file's own flag `option`: from the lint section where it is set there, else the top level. */
const lintFlagOf = (toml: RuffToml, option: string): Flag => flagOf(toml, `${toml.prefix}lint.${option}`) ?? flagOf(toml, `${toml.prefix}${option}`);

/** What `readTomlStringLists` makes of a Ruff file whose Ruff section starts at `prefix`, or why it cannot be read. */
function parseRuffToml(text: string, prefix: string) {
  const sections = sectionsOf(prefix);
  const paths = (options: readonly string[]): string[] => options.flatMap(option => sections.map(section => `${section}${option}`));
  return readFormat(() => readTomlStringLists(text, paths(RUFF_LISTS), paths(RUFF_TABLES), [`${prefix}extend`], paths(RUFF_SCALARS)));
}

/** The Ruff file `toml` holds, as Ruff reads it. */
function ruffFileOf(toml: RuffToml): RuffFile {
  const select = selectorsOf(toml, "select");
  return {
    name: toml.name,
    layer: {
      file: toml.name,
      select: select?.selectors,
      ...(select === undefined ? {} : { selectKey: select.key }),
      extendSelect: selectorsOf(toml, "extend-select")?.selectors ?? [],
      ignore: [...selectorsOf(toml, "ignore")?.selectors ?? [], ...selectorsOf(toml, "extend-ignore")?.selectors ?? []],
    },
    perFileIgnores: perFileIgnoresOf(toml, "per-file-ignores"),
    extendPerFileIgnores: perFileIgnoresOf(toml, "extend-per-file-ignores") ?? [],
    lintPreview: flagOf(toml, `${toml.prefix}lint.preview`),
    preview: flagOf(toml, `${toml.prefix}preview`),
    explicitPreviewRules: lintFlagOf(toml, "explicit-preview-rules"),
    extend: toml.found.strings.get(`${toml.prefix}extend`),
  };
}

/**
 * Ruff config file `name` (a path relative to the root), as Ruff reads it: from `[tool.ruff]` in a `pyproject.toml`,
 * from the top in any other file. `absent` when there is no such file, `not-ruff` for a `pyproject.toml` without
 * `[tool.ruff]`, undefined (with a note) when it cannot be read.
 */
function readRuffFile(root: string, name: string, notes: LinterNote[]): RuffFile | "absent" | "not-ruff" | undefined {
  const read = readRootFile(root, name);
  if (!read.present) return "absent";
  if (read.text === undefined) {
    notes.push({ linter: "ruff", text: `${name} is not read: ${NOT_A_FILE}` });
    return undefined;
  }
  const prefix = name.split("/").at(-1) === "pyproject.toml" ? "tool.ruff." : "";
  const found = parseRuffToml(read.text, prefix);
  if (!found.ok) return unparsedRuffFile(name, read.text, prefix, found.why, notes);
  return hasRuffSection(prefix, found.value) ? ruffFileOf({ name, prefix, found: found.value, notes }) : "not-ruff";
}

/** Whether the file holds a Ruff config: any file but a pyproject.toml, or one with a [tool.ruff] section, as Ruff skips one without. */
function hasRuffSection(prefix: string, found: TomlStringLists): boolean {
  return prefix === "" || found.paths.some(path => path === "tool.ruff" || path.startsWith("tool.ruff."));
}

/** A Ruff file that does not parse: not read, with a note; a pyproject.toml that may hold no Ruff config is skipped. */
function unparsedRuffFile(name: string, text: string, prefix: string, why: string, notes: LinterNote[]): "not-ruff" | undefined {
  if (prefix === "" || text.includes("ruff")) notes.push({ linter: "ruff", text: `${name} is not read: ${why}.` });
  return prefix === "" ? undefined : "not-ruff";
}

/** Whether `extend` names a path outside any repository: absolute, from home, or with a drive or backslash. */
const isOutsidePath = (extend: string): boolean => /^(?:[/~]|[A-Za-z]:)/u.test(extend) || extend.includes("\\");

/**
 * The config `extend` names from config `from`, as a path relative to the root: `""` for the root folder itself,
 * undefined outside the repository.
 */
function extendedPath(from: string, extend: string): string | undefined {
  if (isOutsidePath(extend)) return undefined;
  const parts: string[] = from.split("/").slice(0, -1);
  for (const part of extend.split("/").filter(step => step !== "" && step !== ".")) {
    if (part !== "..") parts.push(part);
    else if (parts.pop() === undefined) return undefined;
  }
  return parts.join("/");
}

/** Why the extended config of `current` is not read, if it is not: `target` is its path, undefined outside the repository. */
function unreadExtension(chain: readonly RuffFile[], target: string | undefined): string | undefined {
  if (target === undefined) return "it is outside the repository";
  if (target === "") return "it names the repository's root folder, not a config file";
  return chain.some(file => file.name === target) || chain.length > MAX_RUFF_EXTENDS ? "its extend chain loops or runs too deep" : undefined;
}

/** The config the last of `chain` extends, as read: a Ruff file, none, one that cannot be read (noted), or why it is not read. */
function extendedFile(root: string, chain: readonly RuffFile[], notes: LinterNote[]): RuffFile | "not-ruff" | "unreadable" | { readonly why: string } {
  const current = chain.at(-1)!;
  const target = extendedPath(current.name, current.extend!);
  const skipped = unreadExtension(chain, target);
  if (skipped !== undefined) return { why: skipped };
  const base = readRuffFile(root, target!, notes);
  if (base === "absent") return { why: "there is no such file" };
  return base ?? "unreadable";
}

/** Why the config `current` extends ends the chain Rerail reads, or the next config in it. */
function nextInChain(root: string, chain: readonly RuffFile[], notes: LinterNote[]): RuffFile | { unread: string | undefined } {
  const current = chain.at(-1)!;
  const named = `${current.name}: the config it extends, ${JSON.stringify(current.extend)},`;
  const base = extendedFile(root, chain, notes);
  if (base === "not-ruff") return { unread: undefined };
  if (base === "unreadable") return { unread: named };
  if ("why" in base) {
    notes.push({ linter: "ruff", text: `${named} is not read (${base.why}), so the rules only that config ignores stay on.` });
    return { unread: named };
  }
  return base;
}

/** The chain of configs `extend` names from `first`; a config Rerail cannot read ends it, named as `unread`. */
function extendChain(root: string, first: RuffFile, notes: LinterNote[]): { chain: RuffFile[]; unread: string | undefined } {
  const chain: RuffFile[] = [first];
  while (chain.at(-1)!.extend !== undefined) {
    const next = nextInChain(root, chain, notes);
    if (!("name" in next)) return { chain, unread: next.unread };
    chain.push(next);
  }
  return { chain, unread: undefined };
}

/** The per-file ignores Ruff applies: those of the nearest config that sets `per-file-ignores`, and every extension. */
function perFileIgnores(chain: readonly RuffFile[], notes: LinterNote[]): RuffPerFileIgnore[] {
  const nearest = chain.find(file => file.perFileIgnores !== undefined);
  const setting = chain.filter(file => file === nearest || file.extendPerFileIgnores.length > 0);
  // A per-file ignore's pattern is relative to its config's folder; Rerail reads those of configs at the root.
  for (const file of setting.filter(file => file.name.includes("/"))) {
    notes.push({ linter: "ruff", text: `${file.name}: its per-file ignores are not read: their patterns are relative to its folder.` });
  }
  const atRoot = setting.filter(file => !file.name.includes("/"));
  return [...atRoot.flatMap(file => (file === nearest ? file.perFileIgnores! : [])), ...atRoot.flatMap(file => file.extendPerFileIgnores)];
}

/** The nearest config's value of a flag; `false` when none sets it and the chain is read whole, undefined otherwise. */
function chainFlag(values: readonly Flag[], whole: boolean): Flag {
  return values.find(value => value !== undefined) ?? (whole ? false : undefined);
}

/** Ruff's rule selection at `root`; undefined when there is no Ruff config, or (with a note) when it cannot be read. */
export function readRuffSelection(root: string, notes: LinterNote[]): RuffSelection | undefined {
  for (const name of RUFF_FILES) {
    const first = readRuffFile(root, name, notes);
    if (first === "absent" || first === "not-ruff") continue;
    if (first === undefined) return undefined;
    const { chain, unread } = extendChain(root, first, notes);
    const whole = unread === undefined;
    // Ruff combines `lint.preview` and `preview` over the chain apart, the linter's taking `lint.preview` first.
    const preview = chain.map(file => file.lintPreview).find(value => value !== undefined) ?? chainFlag(chain.map(file => file.preview), whole);
    return {
      file: first.name,
      layers: chain.map(file => file.layer).reverse(),
      perFileIgnores: perFileIgnores(chain, notes),
      preview,
      explicitPreviewRules: chainFlag(chain.map(file => file.explicitPreviewRules), whole),
      ...(unread === undefined ? {} : { unread }),
    };
  }
  return undefined;
}

/**
 * Whether Ruff selector `selector` names rule `code`: `ALL` every rule; a category the rules in it; otherwise a prefix
 * of the code within the same linter, so `E` selects `E711` but not `ERA001`, and `PL` selects Pylint's `PLR0913`.
 * Whether Ruff then runs the rule also depends on its status (`statusAllows`).
 */
export function ruffSelects(selector: string, code: string): boolean {
  if (selector === "ALL") return true;
  if (RUFF_CATEGORIES.has(selector)) return RUFF_RULES.get(code)?.category === selector;
  if (!code.startsWith(selector)) return false;
  const selectorLetters = /^[A-Z]*/u.exec(selector)![0];
  const codeLetters = /^[A-Z]*/u.exec(code)![0];
  return selectorLetters === codeLetters || (selectorLetters === "PL" && /^PL[CERW]$/u.test(codeLetters));
}

/**
 * How specific a Ruff selector is, as Ruff orders them (`Specificity` in ruff_linter's `rule_selector.rs`): `ALL`, a
 * category, the legacy `C` and `T`, a linter (`E`, `PL`), a prefix by its length (`E7`, `PLR09`), a rule (`E711`). Two
 * code selectors that select the same code are both prefixes of it, so the longer one is the more specific.
 */
function ruffSpecificity(selector: string): number {
  if (selector === "ALL") return 0;
  return RUFF_CATEGORIES.has(selector) ? 0.5 : selector.length;
}

/** Preview mode as Ruff's selection reads it: whether preview is on, and whether preview rules need their own code. */
interface RuffMode {
  readonly preview: boolean;
  readonly explicitPreviewRules: boolean;
}

/**
 * Per status, whether a selector (exactly the rule's code, or not) selects a rule under a mode (`RuleSelector::rules`
 * in `rule_selector.rs`): a preview rule only with preview, and then by its code where preview rules need it; a
 * removed one by its code (Ruff then refuses the config). Ruff 0.16.9 has no deprecated rule, so the status type has
 * no `deprecated`: a Ruff that brings one back fails type-checking here until its arm (selected only by its code,
 * and only without preview) is added.
 */
const STATUS_ALLOWS: Readonly<Record<RuffRuleInfo["status"], (exact: boolean, mode: RuffMode) => boolean>> = {
  stable: () => true,
  preview: (exact, mode) => mode.preview && (exact || !mode.explicitPreviewRules),
  removed: exact => exact,
};

/** Whether Ruff runs a rule, and the selector that decided it; `by` is undefined for a rule a `select` leaves out. */
interface RuffDecision {
  readonly enabled: boolean;
  readonly by: RuffSelector | undefined;
  /** The config whose `select` left the rule out. */
  readonly leftOutBy?: RuffLayer;
}

/** One config's selectors that name `code` as Ruff applies them, least specific first, selects before ignores. */
function layerUpdates(layer: RuffLayer, carried: readonly RuffSelector[], code: string, applies: (selector: string) => boolean): { enabled: boolean; by: RuffSelector }[] {
  const updates = [
    ...[...(layer.select ?? []), ...layer.extendSelect].map(by => ({ enabled: true, by })),
    ...[...layer.ignore, ...carried].map(by => ({ enabled: false, by })),
  ].filter(({ by }) => ruffSelects(by.selector, code) && applies(by.selector));
  // A stable sort keeps selects before ignores of the same specificity.
  return updates.sort((left, right) => ruffSpecificity(left.by.selector) - ruffSpecificity(right.by.selector));
}

/** The ignores `select = []` with ignores and no `extend-select` carries into the next config. */
const carriedIgnores = (layer: RuffLayer): readonly RuffSelector[] =>
  (layer.select?.length === 0 && layer.extendSelect.length === 0 ? layer.ignore : []);

/**
 * What Ruff's rule selection decides for rule `code` under `mode`, replaying `as_rule_table` over the configs in order.
 * Within one config the most specific selector that names the code decides. A config that sets `select` replaces the
 * earlier configs' result (a code it does not select is off, decided by no selector); one that does not changes only
 * the codes its selectors name; `select = []` with ignores and no `extend-select` carries its ignores into the next
 * config. Undefined when no selector decides the code: Ruff's default selection does.
 */
function ruffDecision(selection: RuffSelection, code: string, info: RuffRuleInfo, mode: RuffMode): RuffDecision | undefined {
  const allows = STATUS_ALLOWS[info.status];
  let decided: RuffDecision | undefined;
  let carried: readonly RuffSelector[] = [];
  for (const layer of selection.layers) {
    const last = layerUpdates(layer, carried, code, selector => allows(selector === code, mode)).at(-1);
    carried = carriedIgnores(layer);
    const replaced = layer.select === undefined ? undefined : { enabled: false, by: undefined, leftOutBy: layer };
    decided = last ?? replaced ?? decided;
  }
  return decided;
}

const ruffText = ({ file, key, selector }: RuffSelector): string => `${file}: ${JSON.stringify(selector)} in ${key}`;

/** Ruff's default selection under `mode`: the default categories with preview, less preview rules where they need their code. */
function byDefault(info: RuffRuleInfo, mode: RuffMode): boolean {
  if (!mode.preview) return info.byDefault;
  return info.byPreviewDefault && !(mode.explicitPreviewRules && info.status === "preview");
}

const defaultText = (mode: RuffMode): string => `Ruff ${RUFF_VERSION}'s default rule selection${mode.preview ? " with preview" : ""}`;

/** Ruff's decision for `code` under one `mode`. */
function modeDecision(selection: RuffSelection, code: string, info: RuffRuleInfo, mode: RuffMode): PortDecision {
  const decision = ruffDecision(selection, code, info, mode);
  if (decision?.by !== undefined) return { on: decision.enabled, explicit: true, because: ruffText(decision.by) };
  if (decision?.leftOutBy !== undefined) return { on: false, explicit: false, because: `${decision.leftOutBy.file}: ${decision.leftOutBy.selectKey!} does not select it` };
  if (selection.unread !== undefined) return { on: "unknown", because: `${selection.unread} which Rerail does not read, may select it` };
  return byDefault(info, mode)
    ? { on: true, explicit: false, because: `${selection.file}: ${defaultText(mode)}` }
    : { on: false, explicit: false, because: `${selection.file} does not select it, nor does ${defaultText(mode)}` };
}

/** The preview modes the selection may run in: one where every config is read, both where an unread one may set them. */
function modesOf(selection: RuffSelection): RuffMode[] {
  const previews = selection.preview === undefined ? [false, true] : [selection.preview];
  const explicits = selection.explicitPreviewRules === undefined ? [false, true] : [selection.explicitPreviewRules];
  return previews.flatMap(preview => explicits.map(explicitPreviewRules => ({ preview, explicitPreviewRules })));
}

/**
 * Ruff's decision for `code` in every mode the selection may run in; unknown where they differ. The selection runs in
 * more than one mode only where a config it extends is not read, which then may set preview.
 */
function codeDecision(selection: RuffSelection, code: string, info: RuffRuleInfo): PortDecision {
  const decisions = modesOf(selection).map(mode => modeDecision(selection, code, info, mode));
  if (decisions.every(decision => decision.on === decisions[0]!.on)) return decisions[0]!;
  return { on: "unknown", because: `${selection.unread!} which Rerail does not read, may set preview, which decides it` };
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

/** The folders a per-file ignore turns the code off in, and notes for narrower patterns. */
function perFileVerdict(ruleId: string, selection: RuffSelection, code: string): { folders: FolderDecision[]; notes: string[] } {
  const folders: FolderDecision[] = [];
  const notes: string[] = [];
  for (const { pattern, folder, ignore } of selection.perFileIgnores) {
    if (!ruffSelects(ignore.selector, code)) continue;
    if (folder !== undefined) folders.push({ folder, on: false, because: ruffText(ignore) });
    else notes.push(`${ruleId}: ${ruffText(ignore)} ignores it only in the files ${JSON.stringify(pattern)} matches; it stays on.`);
  }
  return { folders, notes };
}

/** What the selection says about one of a port's codes. */
function codeVerdict(ruleId: string, selection: RuffSelection, code: string): PortVerdict & { readonly folders: readonly FolderDecision[]; readonly notes: readonly string[] } {
  const info = RUFF_RULES.get(code);
  if (info === undefined) return { folders: [], notes: [`${ruleId}: ${JSON.stringify(code)} is not a rule of Ruff ${RUFF_VERSION}; Ruff's config is not used for it.`] };
  const decision = codeDecision(selection, code, info);
  const perFile = perFileVerdict(ruleId, selection, code);
  return {
    decision,
    folders: perFile.folders,
    notes: [...previewNotes(ruleId, selection, code, info), ...overriddenNotes(ruleId, selection, code, decision), ...perFile.notes],
  };
}

/** The verdict of Ruff's selection on `port`, over its noqa codes. */
function ruffVerdict(selection: RuffSelection, port: Port): PortVerdict | undefined {
  if (port.suppress.noqa.length === 0) return undefined;
  const verdicts = port.suppress.noqa.map(code => codeVerdict(port.ruleId, selection, code));
  const decision = combineDecisions(verdicts.flatMap(verdict => (verdict.decision === undefined ? [] : [verdict.decision])));
  return {
    ...(decision === undefined ? {} : { decision }),
    folders: verdicts.flatMap(verdict => verdict.folders),
    notes: verdicts.flatMap(verdict => verdict.notes),
  };
}

export const ruffReader: LinterReader = {
  linter: "ruff",
  kinds: ["noqa"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const selection = readRuffSelection(root, notes);
    return selection === undefined ? undefined : { verdict: port => ruffVerdict(selection, port) };
  },
};
