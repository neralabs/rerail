/**
 * One Ruff config file read as data (`.ruff.toml`, `ruff.toml` or `pyproject.toml` `[tool.ruff]`), and the configs
 * its `extend` names in the repository: `select`, `extend-select`, `ignore`, `extend-ignore`, `per-file-ignores`,
 * `extend-per-file-ignores`, `preview`, `explicit-preview-rules` and `pydocstyle.convention`, each from the `lint`
 * section where it is set there, else from the top level, as Ruff reads them. A file Ruff refuses (a key that names no
 * Ruff option, an `extend` it fails to load) is not read; an extended config Ruff may load but Rerail cannot place or
 * read ends the chain as unread, so what it may decide stays unknown.
 */
import { lstatSync } from "node:fs";
import { join } from "node:path";
import { NOT_A_FILE, readRootFile, type LinterNote } from "../linter-config.js";
import { readFormat } from "../formats/lint-config-formats.js";
import { readTomlStringLists, type TomlStringLists } from "../formats/toml-format.js";
import { RUFF_OPTIONS, type RuffOptionKeys } from "../generated/ruff-rules.generated.js";
import type { RuffSelector } from "./ruff-selectors.js";

/** Ruff's config files in the order of precedence when one folder has several. */
export const RUFF_FILES = [".ruff.toml", "ruff.toml", "pyproject.toml"];
/** Ruff's rule selection lists; each is read from the `lint` section, else from the top level as Ruff does. */
const RUFF_LISTS = ["select", "extend-select", "ignore", "extend-ignore"] as const;
const RUFF_TABLES = ["per-file-ignores", "extend-per-file-ignores"] as const;
const RUFF_SCALARS = ["preview", "explicit-preview-rules"] as const;
const CONVENTION = "pydocstyle.convention";
/** A per-file pattern every file's name matches, as Ruff matches it (`*` crosses `/`, `**\/` may match nothing). */
const EVERY_FILE = new Set(["*", "**", "**/*", "**/**"]);

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
export type RuffPerFileIgnore = { readonly pattern: string; readonly folder: string | undefined; readonly ignore: RuffSelector };

/** The docstring convention a config sets, and where. */
export interface RuffConvention {
  readonly name: string;
  readonly because: string;
}

/** A Ruff setting a config sets: `true`, `false`, or undefined where it does not set it. */
type Flag = boolean | undefined;

/** One Ruff config file as read: its rule selection, its per-file ignores, its settings, and the config it extends. */
export interface RuffFile {
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
export const folderOf = (file: string): string => file.split("/").slice(0, -1).join("/");

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

/** What `readTomlStringLists` makes of a Ruff file whose Ruff section starts at `prefix` (TOML 1.1, as Ruff parses it), or why it cannot be read. */
function parseRuffToml(text: string, prefix: string) {
  const sections = sectionsOf(prefix);
  const paths = (options: readonly string[]): string[] => options.flatMap(option => sections.map(section => `${section}${option}`));
  return readFormat(() => readTomlStringLists(text, paths(RUFF_LISTS), paths(RUFF_TABLES), [`${prefix}extend`, ...paths([CONVENTION])], paths(RUFF_SCALARS), "1.1"));
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

/** A Ruff config file Rerail does not read, and why; `loads` is whether Ruff loads it, or fails to as Rerail does. */
interface UnreadFile {
  readonly why: string;
  readonly loads: boolean;
}

/**
 * Ruff config file `name` (a path relative to the root), as Ruff reads it: from `[tool.ruff]` in a `pyproject.toml`,
 * from the top in any other file. `absent` when there is no such file, `not-ruff` for a `pyproject.toml` without
 * `[tool.ruff]` (or, not `extended`, one that does not parse and never names Ruff), or why it is not read.
 */
export function readRuffFile(root: string, name: string, notes: LinterNote[], extended: boolean): RuffFile | "absent" | "not-ruff" | UnreadFile {
  const read = readRootFile(root, name);
  if (!read.present) return "absent";
  if (read.text === undefined) return notAFile(root, name);
  const prefix = name.split("/").at(-1) === "pyproject.toml" ? "tool.ruff." : "";
  const found = parseRuffToml(read.text, prefix);
  if (!found.ok) return unparsedRuffFile(read.text, prefix, found.why, extended);
  return parsedRuffFile({ name, prefix, found: found.value, notes });
}

/** Why the path `name` that is no regular file Rerail reads is not read: a folder, which Ruff fails to load, or what Ruff may load (a link, a large file). */
function notAFile(root: string, name: string): UnreadFile {
  return lstatSync(join(root, name)).isDirectory() ? { why: "it is a folder", loads: false } : { why: NOT_A_FILE.slice(0, -1), loads: true };
}

/**
 * The Ruff file a parsed file holds: `not-ruff` for a pyproject.toml without a [tool.ruff] section, as Ruff skips one;
 * one with a key in its Ruff section that names no Ruff option, which Ruff refuses (`deny_unknown_fields`), is not read.
 */
function parsedRuffFile(toml: RuffToml): RuffFile | "not-ruff" | UnreadFile {
  if (toml.prefix !== "" && !toml.found.paths.some(path => path === "tool.ruff" || path.startsWith("tool.ruff."))) return "not-ruff";
  const unknown = toml.found.paths.find(path => path.startsWith(toml.prefix) && !isRuffOption(path.slice(toml.prefix.length)));
  return unknown === undefined ? ruffFileOf(toml) : { why: `Ruff refuses its ${JSON.stringify(unknown)}, which is not a Ruff option`, loads: false };
}

/** One part of a dotted key path as `readTomlStringLists` writes it: bare, or quoted as JSON. */
const KEY_PART = /"(?:[^"\\]|\\.)*"|[^.]+/gu;

/** Whether Ruff's options take the dotted key path `path` (`lint.isort.known-first-party`), as `RUFF_OPTIONS` holds them. */
function isRuffOption(path: string): boolean {
  let keys: RuffOptionKeys | undefined = RUFF_OPTIONS;
  for (const [part] of path.matchAll(KEY_PART)) {
    const key = part.startsWith("\"") ? (JSON.parse(part) as string) : part;
    keys = Object.hasOwn(keys, key) ? keys[key] : keys["*"];
    if (keys === undefined) return false;
  }
  return true;
}

/**
 * A Ruff file that does not parse, which Ruff fails to load; a pyproject.toml found, not extended, that never names
 * Ruff is skipped as one that may hold no Ruff config.
 */
function unparsedRuffFile(text: string, prefix: string, why: string, extended: boolean): UnreadFile | "not-ruff" {
  return prefix === "" || extended || text.includes("ruff") ? { why, loads: false } : "not-ruff";
}

/** Whether `extend` names a path Rerail cannot place: absolute, from home, with a drive or backslash, or through an environment variable Ruff expands. */
const isOutsidePath = (extend: string): boolean => /^(?:[/~]|[A-Za-z]:)/u.test(extend) || /[\\$]/u.test(extend);

/**
 * The config `extend` names from config `from`, as a path relative to the root: `""` for the root folder itself,
 * undefined where Rerail cannot place it.
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

/** Why the extended config of `chain`'s last is not read before reading it, if it is not: `target` is its path, undefined where Rerail cannot place it. */
function unreadExtension(chain: readonly RuffFile[], target: string | undefined): UnreadFile | undefined {
  if (target === undefined) return { why: "it is outside the repository or names an environment variable", loads: true };
  if (chain.some(file => file.name === target)) return { why: "its extend chain loops", loads: false };
  return chain.length > MAX_RUFF_EXTENDS ? { why: "its extend chain runs too deep", loads: true } : undefined;
}

/** The config the last of `chain` extends, as read: a Ruff file, none, or why it is not read. */
function extendedFile(root: string, chain: readonly RuffFile[], notes: LinterNote[]): RuffFile | "not-ruff" | UnreadFile {
  const current = chain.at(-1)!;
  const target = extendedPath(current.name, current.extend!);
  const skipped = unreadExtension(chain, target);
  if (skipped !== undefined) return skipped;
  const base = readRuffFile(root, target!, notes, true);
  return base === "absent" ? { why: "there is no such file", loads: false } : base;
}

/** Where the chain Rerail reads ends: at a config it does not read (`unread`, named), or one Ruff fails to load (`refused`, why). */
export type ChainEnd = { readonly unread: string | undefined; readonly refused?: never } | { readonly unread?: never; readonly refused: string };

/** The next config in the chain after `current`, or where the chain ends. */
function nextInChain(root: string, chain: readonly RuffFile[], notes: LinterNote[]): RuffFile | ChainEnd {
  const current = chain.at(-1)!;
  const named = `${current.name}: the config it extends, ${JSON.stringify(current.extend)},`;
  const base = extendedFile(root, chain, notes);
  if (base === "not-ruff") return { unread: undefined };
  if ("name" in base) return base;
  if (!base.loads) return { refused: `${named} fails to load: ${base.why}` };
  notes.push({ linter: "ruff", text: `${named} is not read (${base.why}), so the rules only that config ignores stay on.` });
  return { unread: named };
}

/** The chain of configs `extend` names from `first`, and where it ends. */
export function extendChain(root: string, first: RuffFile, notes: LinterNote[]): { chain: RuffFile[]; end: ChainEnd } {
  const chain: RuffFile[] = [first];
  while (chain.at(-1)!.extend !== undefined) {
    const next = nextInChain(root, chain, notes);
    if (!("name" in next)) return { chain, end: next };
    chain.push(next);
  }
  return { chain, end: { unread: undefined } };
}
