/**
 * Ruff's rule selection, read as data from the config Ruff uses at the root (`.ruff.toml`, `ruff.toml` or
 * `pyproject.toml` `[tool.ruff]`) and the configs its `extend` names in the repository: `select`, `extend-select`,
 * `ignore`, `extend-ignore`, `per-file-ignores`, `extend-per-file-ignores`, `preview`, `explicit-preview-rules` and
 * `pydocstyle.convention`, each from the `lint` section where it is set there, else from the top level, as Ruff reads
 * them.
 *
 * A port's noqa codes are decided as the pinned Ruff decides them (`as_rule_table` in ruff_workspace's
 * `configuration.rs`): every selector read as Ruff reads it (a remapped one, `TCH` or `TRY200`, as its target; a
 * category or rule name only with preview), Ruff's default selection (`DEFAULT_SELECTORS` in ruff_linter's
 * `settings/mod.rs`, or the default categories with preview), then each config's selectors from the extended one to the
 * root's, then the docstring convention and the pairs of rules Ruff never runs together. A code a selector decides is
 * explicit; one none decides follows the default, an implicit on or off. A preview rule runs only with preview (and,
 * with `explicit-preview-rules`, only when selected by its code). What an extended config Rerail does not read may
 * decide is unknown, as is every rule in a folder below the root holding a Ruff config of its own, which Ruff uses
 * there instead. A config Ruff refuses (a selector it does not know, a removed rule selected) is not read at all.
 */
import { combineDecisions } from "./linter-decisions.js";
import {
  NOT_A_FILE,
  readRootFile,
  type FolderDecision,
  inFolder,
  type LinterConfig,
  type LinterNote,
  type LinterReader,
  type Port,
  type PortDecision,
  type PortVerdict,
} from "./linter-config.js";
import { readFormat, readTomlStringLists, type TomlStringLists } from "./formats/lint-config-formats.js";
import {
  RUFF_CATEGORIES,
  RUFF_CONVENTIONS,
  RUFF_INCOMPATIBLE,
  RUFF_LINTER_PREFIXES,
  RUFF_NAMES_WITHOUT_CODE,
  RUFF_REDIRECTS,
  RUFF_RULES,
  UPSTREAM_VERSIONS,
  type RuffRuleInfo,
} from "./generated/ruff-rules.generated.js";
import { repositoryFiles } from "./repository-files.js";

/** The Ruff release whose rules and defaults the generated tables hold (packs/python/upstream/ruff/gates.yaml). */
const RUFF_VERSION = UPSTREAM_VERSIONS["python/ruff"]!;
/** Ruff's config files in the order of precedence when one folder has several. */
const RUFF_FILES = [".ruff.toml", "ruff.toml", "pyproject.toml"];
/** Ruff's rule selection lists; each is read from the `lint` section, else from the top level as Ruff does. */
const RUFF_LISTS = ["select", "extend-select", "ignore", "extend-ignore"] as const;
const RUFF_TABLES = ["per-file-ignores", "extend-per-file-ignores"] as const;
const RUFF_SCALARS = ["preview", "explicit-preview-rules"] as const;
const CONVENTION = "pydocstyle.convention";
const CATEGORIES = new Set(RUFF_CATEGORIES);
/** Each rule's name to its code; undefined for a rule without one. */
const NAMES = new Map<string, string | undefined>([
  ...[...RUFF_RULES].map(([code, info]): [string, string] => [info.name, code]),
  ...RUFF_NAMES_WITHOUT_CODE.map((name): [string, undefined] => [name, undefined]),
]);
const LINTERS_LONGEST_FIRST = [...RUFF_LINTER_PREFIXES].sort((left, right) => right.length - left.length);
/** The linter whose code prefix starts `selector`, the longest that does, as Ruff's `Linter::parse_code` finds it. */
const linterOf = (selector: string): string | undefined => LINTERS_LONGEST_FIRST.find(prefix => selector.startsWith(prefix));
/** Every code selector Ruff takes: each prefix of a rule's code from its linter's prefix on (`E`, `E7`, `E71`, `E711`; `PL`, `PLR`). */
const CODE_SELECTORS = new Set([...RUFF_RULES.keys()].flatMap(code => {
  const linter = linterOf(code)!;
  return Array.from({ length: code.length - linter.length + 1 }, (_, extra) => code.slice(0, linter.length + extra));
}));
/** The legacy selectors `C` and `T` (`RuleSelector::C` and `T` in ruff_linter's `rule_selector.rs`) and the linters each selects. */
const LEGACY_GROUPS: Readonly<Record<string, readonly string[]>> = { C: ["C4", "C90"], T: ["T10", "T20"] };
/** A per-file pattern every file's name matches, as Ruff matches it (`*` crosses `/`, `**\/` may match nothing). */
const EVERY_FILE = new Set(["*", "**", "**/*", "**/**"]);

/** A Ruff selector in one of the rule selection options, with the file and key path that hold it. */
export interface RuffSelector {
  readonly file: string;
  readonly key: string;
  /** The selector as Ruff reads it: a remapped one as its target, a rule name as its code. */
  readonly selector: string;
  /** The selector as the config writes it, where Ruff reads it as another. */
  readonly written?: string;
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

/**
 * A per-file ignore: `ignore` in the files `pattern` matches, or, for a pattern starting `!`, in every file it does not.
 * `folder` is the folder whose every file the pattern (without its `!`) matches, relative to the root (`""` for every
 * file); undefined where it matches narrower files.
 */
type RuffPerFileIgnore = { readonly pattern: string; readonly folder: string | undefined; readonly ignore: RuffSelector };

/** The docstring convention a config sets, and where. */
interface RuffConvention {
  readonly name: string;
  readonly because: string;
}

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
  /** `pydocstyle.convention`; null where no config sets it, undefined where one not read may. */
  readonly convention: RuffConvention | null | undefined;
  /** The extended config Rerail does not read, which may select or ignore any rule: how the person knows it. */
  readonly unread?: string;
}

/** A Ruff setting a config sets: `true`, `false`, or undefined where it does not set it. */
type Flag = boolean | undefined;

/** One Ruff config file as read: its rule selection, its per-file ignores, its settings, and the config it extends. */
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
  readonly convention: RuffConvention | undefined;
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

/** The key path of `option`: in the lint section where it is set there, else the top level; undefined when unset. */
const keyOf = (toml: RuffToml, option: string, set: ReadonlyMap<string, unknown>): string | undefined =>
  sectionsOf(toml.prefix).map(section => `${section}${option}`).find(path => set.has(path));

/** The selectors of `option` as written, from the lint section where it is set there, else the top level; undefined when unset. */
function selectorsOf(toml: RuffToml, option: string): { key: string; selectors: RuffSelector[] } | undefined {
  const key = keyOf(toml, option, toml.found.lists);
  if (key === undefined) return undefined;
  return { key, selectors: toml.found.lists.get(key)!.map(selector => ({ file: toml.name, key, selector })) };
}

/** The repository folder (`""` the root) of config `file`. */
const folderOf = (file: string): string => file.split("/").slice(0, -1).join("/");

/**
 * The folder, relative to the root, whose every file Ruff's per-file pattern `pattern` (without a `!`) in config `file`
 * matches: `""` for every file, undefined for narrower files. Ruff matches a pattern against a file's name and against
 * its path from the config's folder (`PerFile` in ruff_linter's `settings/types.rs`), `*` crossing `/`, so `tests/*`
 * is every file under `tests`.
 */
function patternFolder(pattern: string, file: string): string | undefined {
  if (EVERY_FILE.has(pattern)) return "";
  const path = pattern.replace(/^\.\//u, "");
  const match = /^((?:[A-Za-z0-9_@.-]+\/)*[A-Za-z0-9_@.-]+)\/\*\*?(?:\/\*\*?)?$/u.exec(path);
  const parts = [folderOf(file), EVERY_FILE.has(path) ? "" : match?.[1]];
  if (parts[1] === undefined || parts[1].split("/").some(part => part === "." || part === "..")) return undefined;
  return parts.filter(part => part !== "").join("/");
}

/** `per-file-ignores` or `extend-per-file-ignores` as written; undefined when unset. */
function perFileIgnoresOf(toml: RuffToml, option: string): RuffPerFileIgnore[] | undefined {
  const key = keyOf(toml, option, toml.found.tables);
  if (key === undefined) return undefined;
  return [...toml.found.tables.get(key)!].flatMap(([pattern, ignored]) => ignored.map(selector => ({
    pattern,
    folder: patternFolder(pattern.replace(/^!/u, ""), toml.name),
    ignore: { file: toml.name, key: `${key}.${JSON.stringify(pattern)}`, selector },
  })));
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

/** The file's `pydocstyle.convention`, from the lint section where it is set there, else the top level. */
function conventionOf(toml: RuffToml): RuffConvention | undefined {
  const key = keyOf(toml, CONVENTION, toml.found.strings);
  if (key === undefined) return undefined;
  const name = toml.found.strings.get(key)!;
  return { name, because: `${toml.name}: ${key} = ${JSON.stringify(name)}` };
}

/** What `readTomlStringLists` makes of a Ruff file whose Ruff section starts at `prefix`, or why it cannot be read. */
function parseRuffToml(text: string, prefix: string) {
  const sections = sectionsOf(prefix);
  const paths = (options: readonly string[]): string[] => options.flatMap(option => sections.map(section => `${section}${option}`));
  return readFormat(() => readTomlStringLists(text, paths(RUFF_LISTS), paths(RUFF_TABLES), [`${prefix}extend`, ...paths([CONVENTION])], paths(RUFF_SCALARS)));
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
    convention: conventionOf(toml),
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
function perFileIgnores(chain: readonly RuffFile[]): RuffPerFileIgnore[] {
  return [...chain.find(file => file.perFileIgnores !== undefined)?.perFileIgnores ?? [], ...chain.flatMap(file => file.extendPerFileIgnores)];
}

/** The nearest config's value of a setting; `none` when none sets it and the chain is read whole, undefined otherwise. */
function chainValue<T, N>(values: readonly (T | undefined)[], whole: boolean, none: N): T | N | undefined {
  return values.find(value => value !== undefined) ?? (whole ? none : undefined);
}

/** What Ruff reads selector `written` as: the selector it selects by (undefined for a rule without a code), and whether Ruff takes it only with preview; undefined for one Ruff does not know. */
function resolveSelector(written: string): { readonly selector: string | undefined; readonly previewOnly: boolean } | undefined {
  if (written === "ALL" || Object.hasOwn(LEGACY_GROUPS, written)) return { selector: written, previewOnly: false };
  const redirected = RUFF_REDIRECTS.get(written) ?? written;
  if (CODE_SELECTORS.has(redirected)) return { selector: redirected, previewOnly: false };
  if (CATEGORIES.has(written)) return { selector: written, previewOnly: true };
  return NAMES.has(written) ? { selector: NAMES.get(written), previewOnly: true } : undefined;
}

const ruffText = ({ file, key, selector, written }: RuffSelector): string =>
  `${file}: ${JSON.stringify(written ?? selector)} in ${key}${written === undefined ? "" : ` (Ruff reads it as ${JSON.stringify(selector)})`}`;

/**
 * Why Ruff refuses selector `by` with preview `preview` (undefined where a config not read may set it), if it does
 * (`RuleSelection::resolve` and `as_rule_table`): one it does not know or takes only with preview (a category or a rule
 * name), without preview; a removed rule selected by its code. With preview, Ruff skips one it does not know.
 */
function refusal(by: RuffSelector, selects: boolean, preview: boolean | undefined): string | undefined {
  const resolved = resolveSelector(by.selector);
  if (selects && RUFF_RULES.get(resolved?.selector ?? "")?.status === "removed") return `${ruffText(by)} selects a removed rule`;
  if (preview === true || resolved?.previewOnly === false) return undefined;
  return resolved === undefined ? `${ruffText(by)} is not a selector Ruff knows` : `${ruffText(by)} selects by category or rule name, which Ruff takes only with preview`;
}

/** Why Ruff refuses the selection read, in every case it does. */
function refusals(chain: readonly RuffFile[], perFile: readonly RuffPerFileIgnore[], preview: boolean | undefined): string[] {
  const selects = chain.flatMap(file => [...file.layer.select ?? [], ...file.layer.extendSelect]);
  const others = [...chain.flatMap(file => file.layer.ignore), ...perFile.map(entry => entry.ignore)];
  const conventions = chain.flatMap(file => (file.convention === undefined || Object.hasOwn(RUFF_CONVENTIONS, file.convention.name) ? [] : [`${file.convention.because} is not a convention Ruff knows`]));
  return [
    ...selects.flatMap(by => refusal(by, true, preview) ?? []),
    ...others.flatMap(by => refusal(by, false, preview) ?? []),
    ...conventions,
  ];
}

/** Selector `by` as Ruff reads it; undefined, with a note, for one Ruff does not know (and skips, with preview), or a rule without a code. */
function resolved(by: RuffSelector, notes: LinterNote[]): RuffSelector | undefined {
  const reading = resolveSelector(by.selector);
  if (reading === undefined) notes.push({ linter: "ruff", text: `${by.file}: ${JSON.stringify(by.selector)} in ${by.key} is not a Ruff rule code or prefix.` });
  if (reading?.selector === undefined) return undefined;
  return reading.selector === by.selector ? by : { ...by, selector: reading.selector, written: by.selector };
}

/** Layer `layer` with each selector as Ruff reads it. */
function resolvedLayer(layer: RuffLayer, notes: LinterNote[]): RuffLayer {
  const each = (selectors: readonly RuffSelector[]): RuffSelector[] => selectors.flatMap(by => resolved(by, notes) ?? []);
  return { ...layer, select: layer.select === undefined ? undefined : each(layer.select), extendSelect: each(layer.extendSelect), ignore: each(layer.ignore) };
}

/** The selection `chain` (the config Ruff uses at the root first) makes, with each selector as Ruff reads it; undefined, with a note, where Ruff refuses it. */
function selectionOf(chain: readonly RuffFile[], unread: string | undefined, notes: LinterNote[]): RuffSelection | undefined {
  const whole = unread === undefined;
  // Ruff combines `lint.preview` and `preview` over the chain apart, the linter's taking `lint.preview` first.
  const preview = chain.map(file => file.lintPreview).find(value => value !== undefined) ?? chainValue(chain.map(file => file.preview), whole, false);
  const perFile = perFileIgnores(chain);
  const refused = refusals(chain, perFile, preview);
  if (refused.length > 0) {
    notes.push({ linter: "ruff", text: `${chain[0]!.name}: Ruff refuses this config (${refused.join("; ")}), so Rerail reads none of it: the rules Rerail ports from Ruff keep the pack's values.` });
    return undefined;
  }
  return {
    file: chain[0]!.name,
    layers: chain.map(file => resolvedLayer(file.layer, notes)).reverse(),
    perFileIgnores: perFile.flatMap(entry => {
      const ignore = resolved(entry.ignore, notes);
      return ignore === undefined ? [] : [{ ...entry, ignore }];
    }),
    preview,
    explicitPreviewRules: chainValue(chain.map(file => file.explicitPreviewRules), whole, false),
    convention: chainValue(chain.map(file => file.convention), whole, null),
    ...(unread === undefined ? {} : { unread }),
  };
}

/** Ruff's rule selection at `root`; undefined when there is no Ruff config, or (with a note) when it cannot be read or Ruff refuses it. */
export function readRuffSelection(root: string, notes: LinterNote[]): RuffSelection | undefined {
  for (const name of RUFF_FILES) {
    const first = readRuffFile(root, name, notes);
    if (first === "absent" || first === "not-ruff") continue;
    if (first === undefined) return undefined;
    const { chain, unread } = extendChain(root, first, notes);
    return selectionOf(chain, unread, notes);
  }
  return undefined;
}

/**
 * Whether Ruff selector `selector` (as Ruff reads it) names rule `code`: `ALL` every rule; a category the rules in it;
 * `C` and `T` their linters; otherwise a prefix of the code within the same linter, so `E` selects `E711` but not
 * `ERA001`, and `PL` selects Pylint's `PLR0913`. Whether Ruff then runs the rule also depends on its status.
 */
export function ruffSelects(selector: string, code: string): boolean {
  if (selector === "ALL") return true;
  if (CATEGORIES.has(selector)) return RUFF_RULES.get(code)?.category === selector;
  if (Object.hasOwn(LEGACY_GROUPS, selector)) return LEGACY_GROUPS[selector]!.includes(linterOf(code)!);
  return code.startsWith(selector) && linterOf(selector) === linterOf(code);
}

/**
 * How specific a Ruff selector is, as Ruff orders them (`Specificity` in ruff_linter's `rule_selector.rs`): `ALL`, a
 * category, the legacy `C` and `T`, a linter (`E`, `PL`), a prefix by its length (`E7`, `PLR09`), a rule (`E711`). Two
 * code selectors that select the same code are both prefixes of it in one linter, so the longer is the more specific.
 */
function ruffSpecificity(selector: string): number {
  if (selector === "ALL") return 0;
  if (CATEGORIES.has(selector)) return 1;
  return Object.hasOwn(LEGACY_GROUPS, selector) ? 2 : 2 + selector.length;
}

/** The mode Ruff's selection runs in: preview, whether preview rules need their own code, and the docstring convention. */
interface RuffMode {
  readonly preview: boolean;
  readonly explicitPreviewRules: boolean;
  readonly convention: string | undefined;
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
 * Whether the configs select `code` by its code since the last `select`, which keeps the docstring convention from
 * turning it off (`docstring_overrides` in `as_rule_table`): a config that sets `select` starts it afresh.
 */
function selectedByCode(exact: boolean, layer: RuffLayer, code: string, selects: boolean): boolean {
  const named = selects && [...layer.select ?? [], ...layer.extendSelect].some(by => by.selector === code);
  return layer.select === undefined ? exact || named : named;
}

/**
 * What Ruff's rule selection decides for rule `code` under `mode`, replaying `as_rule_table` over the configs in order.
 * Within one config the most specific selector that names the code decides. A config that sets `select` replaces the
 * earlier configs' result (a code it does not select is off, decided by no selector); one that does not changes only
 * the codes its selectors name; `select = []` with ignores and no `extend-select` carries its ignores into the next
 * config. `decided` is undefined when no selector decides the code: Ruff's default selection does.
 */
function ruffDecision(selection: RuffSelection, code: string, info: RuffRuleInfo, mode: RuffMode): { decided: RuffDecision | undefined; exact: boolean } {
  const allows = STATUS_ALLOWS[info.status];
  let decided: RuffDecision | undefined;
  let carried: readonly RuffSelector[] = [];
  let exact = false;
  for (const layer of selection.layers) {
    const last = layerUpdates(layer, carried, code, selector => allows(selector === code, mode)).at(-1);
    carried = carriedIgnores(layer);
    exact = selectedByCode(exact, layer, code, allows(true, mode));
    const replaced = layer.select === undefined ? undefined : { enabled: false, by: undefined, leftOutBy: layer };
    decided = last ?? replaced ?? decided;
  }
  return { decided, exact };
}

/** Ruff's default selection under `mode`: the default categories with preview, less preview rules where they need their code. */
function byDefault(info: RuffRuleInfo, mode: RuffMode): boolean {
  if (!mode.preview) return info.byDefault;
  return info.byPreviewDefault && !(mode.explicitPreviewRules && info.status === "preview");
}

const defaultText = (mode: RuffMode): string => `Ruff ${RUFF_VERSION}'s default rule selection${mode.preview ? " with preview" : ""}`;

/** What the selectors and Ruff's default selection decide for `code` under `mode`, and whether a config selects it by its code. */
function selectedDecision(selection: RuffSelection, code: string, info: RuffRuleInfo, mode: RuffMode): { decision: PortDecision; exact: boolean } {
  const { decided, exact } = ruffDecision(selection, code, info, mode);
  const decision = ((): PortDecision => {
    if (decided?.by !== undefined) return { on: decided.enabled, explicit: true, because: ruffText(decided.by) };
    if (decided?.leftOutBy !== undefined) return { on: false, explicit: false, because: `${decided.leftOutBy.file}: ${decided.leftOutBy.selectKey!} does not select it` };
    if (selection.unread !== undefined) return { on: "unknown", because: `${selection.unread} which Rerail does not read, may select it` };
    return byDefault(info, mode)
      ? { on: true, explicit: false, because: `${selection.file}: ${defaultText(mode)}` }
      : { on: false, explicit: false, because: `${selection.file} does not select it, nor does ${defaultText(mode)}` };
  })();
  return { decision, exact };
}

/** Where the docstring convention turns `code` off: it is on, in the convention's list, and not selected by its code. */
function conventionOff(selection: RuffSelection, code: string, mode: RuffMode, exact: boolean): PortDecision | undefined {
  if (mode.convention === undefined || exact || !RUFF_CONVENTIONS[mode.convention]!.includes(code)) return undefined;
  return { on: false, explicit: false, because: `${selection.convention?.because ?? `pydocstyle.convention = ${JSON.stringify(mode.convention)}`} turns it off` };
}

/** Where Ruff turns `code` off as the second of a pair of rules it never runs together, the first being on. */
function incompatibleOff(selection: RuffSelection, code: string, mode: RuffMode): PortDecision | undefined {
  const preferred = RUFF_INCOMPATIBLE.find(([, expendable]) => expendable === code)?.[0];
  if (preferred === undefined) return undefined;
  const decision = modeDecision(selection, preferred, RUFF_RULES.get(preferred)!, mode);
  return decision.on === true ? { on: false, explicit: false, because: `Ruff never runs it with ${preferred}, which is on (${decision.because})` } : undefined;
}

/** Ruff's decision for `code` under one `mode`: its selection, then the docstring convention and the incompatible pairs. */
function modeDecision(selection: RuffSelection, code: string, info: RuffRuleInfo, mode: RuffMode): PortDecision {
  const { decision, exact } = selectedDecision(selection, code, info, mode);
  if (decision.on !== true) return decision;
  return conventionOff(selection, code, mode, exact) ?? incompatibleOff(selection, code, mode) ?? decision;
}

/** The modes the selection may run in: one where every config is read, every one an unread config may set otherwise. */
function modesOf(selection: RuffSelection): RuffMode[] {
  const previews = selection.preview === undefined ? [false, true] : [selection.preview];
  const explicits = selection.explicitPreviewRules === undefined ? [false, true] : [selection.explicitPreviewRules];
  const conventions = selection.convention === undefined ? [undefined, ...Object.keys(RUFF_CONVENTIONS)] : [selection.convention?.name];
  return previews.flatMap(preview => explicits.flatMap(explicitPreviewRules => conventions.map(convention => ({ preview, explicitPreviewRules, convention }))));
}

/**
 * Ruff's decision for `code` in every mode the selection may run in; unknown where they differ. The selection runs in
 * more than one mode only where a config it extends is not read, which then may set preview or a convention.
 */
function codeDecision(selection: RuffSelection, code: string, info: RuffRuleInfo): PortDecision {
  const decisions = modesOf(selection).map(mode => modeDecision(selection, code, info, mode));
  if (decisions.every(decision => decision.on === decisions[0]!.on)) return decisions[0]!;
  return { on: "unknown", because: `${selection.unread!} which Rerail does not read, may set preview or a docstring convention, which decide it` };
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
    const file = RUFF_FILES.map(name => `${folder}/${name}`).find(path => paths.includes(path) && readRuffFile(root, path, []) !== "not-ruff");
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
