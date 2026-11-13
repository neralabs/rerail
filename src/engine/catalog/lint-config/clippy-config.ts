/**
 * Clippy's config as set-up reads it (`linter-config.ts`): per package Cargo lints, the lint levels its `Cargo.toml`
 * sets in `[lints]` (or the root's `[workspace.lints]` it takes with `[lints] workspace = true`), and the inner lint
 * attributes of its crate roots `src/lib.rs` and `src/main.rs`, all read as data, never built. The root package
 * decides for every file; each member of the root's workspace (`cargo-workspace.ts`) decides for its folder. Cargo
 * gives a package `[workspace.lints]` only when it takes them: a member with its own `[lints]`, or none, does not get
 * them, and neither does the root package.
 *
 * Cargo passes `[lints]` to rustc as `--allow`/`--warn`/`--deny`/`--forbid` flags ordered by `priority`, then by
 * lint name in reverse (cargo `src/cargo/workspace/parser/mod.rs` `lints_to_rustflags`, Rust 1.99, lines 2826-2880);
 * rustc applies them in order, a later flag overriding an earlier one for every lint it names, a group naming each
 * of its lints, except that nothing lowers `forbid` (`rustc_lint/src/levels.rs` `add_command_line`). A crate root's
 * inner attributes apply after the flags. A lint left at `warn` takes the level of the `warnings` group when that is
 * set otherwise (`rustc_middle/src/lint.rs` `reveal_actual_level_spec`): `-A warnings` silences it, so it is off
 * here, as no finding of it reaches the person; `-D warnings` keeps it on.
 *
 * A port is matched to Clippy through its `suppress.clippy.lints` names: its lint and its groups
 * (`clippy::needless_else`, `clippy::all`, `clippy::style`, `warnings`). When nothing a package sets names them,
 * Clippy's default for the lint's group decides. `fixtures/host/lint-config/clippy/` holds what `cargo clippy` itself
 * decides over a corpus of workspaces, which the reader is tested against.
 */
import { manifestFile, readCargoWorkspace, readManifest, type CargoManifest, type CargoTable, type UnreadFolder } from "./cargo-workspace.js";
import { LintConfigFormatError, readFormat, type TomlValue } from "./formats/lint-config-formats.js";
import { inFolder, NOT_A_FILE, readRootFile, type FolderDecision, type LinterNote, type LinterReader, type Port, type PortDecision, type PortVerdict } from "./linter-config.js";
import { readRustLintAttributes, type RustLintLevel } from "./rust-lint-attributes.js";

/**
 * Each Clippy group's default level at the pinned clippy 0.1.99 (rust-lang/rust-clippy commit
 * 49e2f89a2605a9a340e04955fdbefa9803e6027a, `declare_clippy_lint/src/lib.rs` lines 152-280, the `declare_clippy_lint!`
 * arms): a lint takes its group's level. `clippy::all` is correctness, suspicious, style, complexity and perf
 * (lines 30-41 there).
 */
const GROUP_LEVELS: Readonly<Record<string, RustLintLevel>> = {
  "clippy::correctness": "deny",
  "clippy::suspicious": "warn",
  "clippy::style": "warn",
  "clippy::complexity": "warn",
  "clippy::perf": "warn",
  "clippy::pedantic": "allow",
  "clippy::restriction": "allow",
  "clippy::nursery": "allow",
  "clippy::cargo": "allow",
};

/** Whether a lint at each level reports findings. */
const REPORTS: Readonly<Record<RustLintLevel, boolean>> = { allow: false, expect: false, warn: true, deny: true, forbid: true };

/** The levels Cargo's `[lints]` takes (cargo-util-schemas `TomlLintLevel`). */
const CARGO_LEVELS: Readonly<Record<string, RustLintLevel>> = { allow: "allow", warn: "warn", deny: "deny", forbid: "forbid" };

/** The tools of `[lints]` whose levels reach Clippy: Clippy's own and rustc's (`warnings`). */
const TOOLS = ["clippy", "rust"] as const;

/** The crate roots whose inner attributes are read. */
const CRATE_ROOTS = ["src/lib.rs", "src/main.rs"] as const;

/** One lint level the config sets, in the order rustc applies it. */
interface LintFlag {
  /** The lint or group, as rustc names it: `clippy::needless_else`, `clippy::all`, `warnings`. */
  readonly name: string;
  readonly level: RustLintLevel;
  /** A `cfg_attr` Rerail cannot decide wraps it. */
  readonly conditional: boolean;
  /** Where it is set, for the person: `Cargo.toml [lints.clippy] needless_else = "allow"`. */
  readonly because: string;
}

/** A package Cargo lints: its folder, its flags in Cargo's order, its crate roots, and what sets a level there. */
interface PackageLints {
  /** Relative to the repository root; `""` for the root package. */
  readonly folder: string;
  readonly flags: readonly LintFlag[];
  readonly roots: readonly CrateRoot[];
  /** Why a lint nothing turns on is off, naming what sets levels there; undefined when nothing does (Clippy's defaults). */
  readonly unset: string | undefined;
}

/** A crate root: its lint attributes as flags, or that it is present and not read. */
type CrateRoot = { readonly file: string; readonly flags: readonly LintFlag[] } | { readonly file: string; readonly unread: true };

type Decided = Extract<PortDecision, { on: boolean }>;
type Unknown = Extract<PortDecision, { on: "unknown" }>;

/** Where the flags leave one lint, or one group: the flag that last set it, and one Rerail cannot decide, if any. */
interface Slot {
  readonly flag?: LintFlag;
  readonly uncertain?: LintFlag;
}

/** `slot` once `flag` applies: nothing lowers `forbid`; after an undecidable flag, the level stays undecided. */
function applyFlag(slot: Slot, flag: LintFlag): Slot {
  if (slot.uncertain !== undefined || slot.flag?.level === "forbid") return slot;
  return flag.conditional ? { ...slot, uncertain: flag } : { flag };
}

/** The flags in `flags` that set a lint named by `names` (its own name and its groups), and those that set `warnings`. */
function replay(names: ReadonlySet<string>, flags: readonly LintFlag[]): { lint: Slot; warnings: Slot } {
  let lint: Slot = {};
  let warnings: Slot = {};
  for (const flag of flags) {
    if (flag.name === "warnings") warnings = applyFlag(warnings, flag);
    else if (names.has(flag.name)) lint = applyFlag(lint, flag);
  }
  return { lint, warnings };
}

/** Whether `warnings` turns off a lint left at `warn`: an explicit off, or unknown; undefined when it leaves it on. */
function warningsDecision(warnings: Slot): PortDecision | undefined {
  if (warnings.uncertain !== undefined) return { on: "unknown", because: `${warnings.uncertain.because} may turn it off` };
  if (warnings.flag === undefined || REPORTS[warnings.flag.level]) return undefined;
  return { on: false, explicit: true, because: warnings.flag.because };
}

/** What `flags`, applied in order, decide for `port`; `unset` says why it is off where nothing turns it on. */
function flagsDecision(port: Port, flags: readonly LintFlag[], unset: string | undefined): PortDecision {
  const { lint, warnings } = replay(new Set(port.suppress.clippy), flags);
  if (lint.uncertain !== undefined) return { on: "unknown", because: `${lint.uncertain.because} may set it` };
  const group = port.suppress.clippy.find(name => Object.hasOwn(GROUP_LEVELS, name)) ?? "";
  const level = lint.flag?.level ?? GROUP_LEVELS[group];
  if (level === undefined) return { on: "unknown", because: "Rerail does not know which Clippy group it is in, so not its default level" };
  return levelDecision(level, { lint: lint.flag, warnings, group }, unset);
}

/**
 * The decision for a lint at `level`, set by `lint` or else by its `group`'s default, once `warnings` applies to it;
 * `unset` says why it is off where nothing turns it on, undefined where nothing sets a level (Clippy's defaults).
 */
function levelDecision(level: RustLintLevel, set: { lint: LintFlag | undefined; warnings: Slot; group: string }, unset: string | undefined): PortDecision {
  const silenced = level === "warn" ? warningsDecision(set.warnings) : undefined;
  if (silenced !== undefined) return silenced;
  if (set.lint !== undefined) return { on: REPORTS[level], explicit: true, because: set.lint.because };
  if (REPORTS[level] || unset === undefined) return { on: REPORTS[level], explicit: false, because: `Clippy's default (${set.group})` };
  return { on: false, explicit: false, because: unset };
}

/** One decision for every file from each crate root's: unknown where one is, or where they differ. */
function agreedDecision(decisions: readonly PortDecision[], files: readonly string[]): PortDecision {
  const unknown = decisions.find((decision): decision is Unknown => decision.on === "unknown");
  if (unknown !== undefined) return unknown;
  const decided = decisions.filter((decision): decision is Decided => decision.on !== "unknown");
  if (new Set(decided.map(decision => decision.on)).size > 1) return { on: "unknown", because: `${files.join(" and ")} set it differently` };
  return decided.find(decision => decision.explicit) ?? decided[0]!;
}

/** What a package's Cargo flags, then each of its crate roots' attributes, decide for `port`. */
function portDecision(port: Port, { flags, roots, unset }: PackageLints): PortDecision {
  if (roots.length === 0) return flagsDecision(port, flags, unset);
  const decisions = roots.map((root): PortDecision => ("unread" in root
    ? { on: "unknown", because: `${root.file} is not read and may set it` }
    : flagsDecision(port, [...flags, ...root.flags], unset)));
  return agreedDecision(decisions, roots.map(root => root.file));
}

/** A number's, boolean's or date's text as TOML writes it; undefined for a string, array or table. */
const scalarText = (value: TomlValue | undefined): string | undefined => (typeof value === "object" && "scalar" in value ? value.scalar : undefined);

/**
 * TOML 1.0's integers: decimal with an optional sign and no leading zero, or unsigned `0x` (either case of digit),
 * `0o` and `0b`; an underscore only between two digits.
 */
const TOML_INTEGER = /^(?:[+-]?(?:0|[1-9](?:_?\d)*)|0x[\dA-Fa-f](?:_?[\dA-Fa-f])*|0o[0-7](?:_?[0-7])*|0b[01](?:_?[01])*)$/u;

/** `value` as the person reads it: a scalar as TOML writes it, a string quoted, else what it is. */
function tomlText(value: TomlValue): string {
  if (typeof value === "string") return JSON.stringify(value);
  if (Array.isArray(value)) return "an array";
  return value instanceof Map ? "a table" : value.scalar;
}

/**
 * Cargo's `priority`: an `i8` (-128 to 127) in any of TOML's integer forms (`-1`, `+1_0`, `0x7f`, `0o17`, `0b101`).
 * One out of range is refused in serde's words, "invalid value: integer `300`, expected i8".
 */
function cargoPriority(where: string, value: TomlValue | undefined): number {
  if (value === undefined) return 0;
  const written = tomlText(value);
  if (!TOML_INTEGER.test(written)) throw new LintConfigFormatError(`${where} priority: ${written} is not a TOML integer, expected i8`);
  const priority = Number(written.replaceAll("_", ""));
  if (priority < -128 || priority > 127) throw new LintConfigFormatError(`${where} priority: invalid value: integer \`${written}\`, expected i8`);
  return priority;
}

function cargoLevel(where: string, value: TomlValue | undefined): RustLintLevel {
  const level = typeof value === "string" && Object.hasOwn(CARGO_LEVELS, value) ? CARGO_LEVELS[value] : undefined;
  if (level === undefined) throw new LintConfigFormatError(`${where} is not "allow", "warn", "deny" or "forbid", or a table with such a level`);
  return level;
}

/** One `[lints.<tool>]` entry: a level, or `{ level = …, priority = … }` (other keys are the lint's own config). */
function cargoEntry(where: string, value: TomlValue): { level: RustLintLevel; priority: number; written: string } {
  if (!(value instanceof Map)) return { level: cargoLevel(where, value), priority: 0, written: JSON.stringify(value) };
  const level = cargoLevel(where, value.get("level"));
  const priority = cargoPriority(where, value.get("priority"));
  return { level, priority, written: `{ level = "${level}"${value.has("priority") ? `, priority = ${priority}` : ""} }` };
}

/** The flags `[<table>]` of `file` (`lints`, or the root's `workspace.lints`) sets, in the order Cargo passes them to rustc. */
function cargoFlags(file: string, table: string, lints: CargoTable): LintFlag[] {
  const flags = TOOLS.flatMap(tool => {
    const entries = lints.get(tool);
    if (entries === undefined) return [];
    if (!(entries instanceof Map)) throw new LintConfigFormatError(`[${table}.${tool}] is not a table`);
    return [...entries].map(([name, value]) => {
      const entry = cargoEntry(`[${table}.${tool}] ${name}`, value);
      const lint = (tool === "rust" ? name : `clippy::${name}`).replaceAll("-", "_");
      const flag: LintFlag = { name: lint, level: entry.level, conditional: false, because: `${file} [${table}.${tool}] ${name} = ${entry.written}` };
      return { flag, priority: entry.priority, written: name };
    });
  });
  // Cargo sorts (priority, Reverse(name), option): by priority, then by name as written in reverse. `option` only
  // orders a rustc lint and a Clippy lint written alike (`--warn=style`, `--allow=clippy::style`): different lints.
  flags.sort((a, b) => a.priority - b.priority || compare(b.written, a.written));
  return flags.map(({ flag }) => flag);
}

const compare = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** The root's `[workspace.lints]` a package with `[lints] workspace = true` takes, refusing what Cargo refuses. */
function inheritedTable(lints: CargoTable, workspaceLints: CargoTable | undefined): CargoTable {
  if (scalarText(lints.get("workspace")) !== "true") throw new LintConfigFormatError("[lints] workspace is not true, the only value Cargo takes");
  if (lints.size > 1) throw new LintConfigFormatError("[lints] takes workspace = true and sets lints too, which Cargo refuses");
  if (workspaceLints === undefined) throw new LintConfigFormatError("[lints] takes workspace = true, but there is no [workspace.lints]");
  return workspaceLints;
}

/** A package's lint flags and the table they come from, for the person; refuses what Cargo refuses. */
function cargoLints(manifest: CargoManifest, workspaceLints: CargoTable | undefined): { flags: LintFlag[]; label: string | undefined } {
  const lints = manifest.tables.get("lints");
  if (lints === undefined) return { flags: [], label: undefined };
  const inherits = lints.has("workspace");
  const [file, table, entries] = inherits ? ["Cargo.toml", "workspace.lints", inheritedTable(lints, workspaceLints)] : [manifestFile(manifest.folder), "lints", lints];
  const flags = cargoFlags(file, table, entries);
  // A table naming no lint, `[lints.clippy]` alone, sets nothing: no config.
  return { flags, label: flags.length === 0 ? undefined : `${file} [${table}.${entries.has("clippy") ? "clippy" : "rust"}]` };
}

/** Crate root `file`'s lint attributes as flags; undefined when it is absent. */
function readCrateRoot(root: string, file: string, notes: LinterNote[]): CrateRoot | undefined {
  const read = readRootFile(root, file);
  if (!read.present) return undefined;
  if (read.text === undefined) {
    notes.push({ linter: "clippy", text: `${file}: its lint attributes are not read: ${NOT_A_FILE}` });
    return { file, unread: true };
  }
  const text = read.text;
  const attributes = readFormat(() => readRustLintAttributes(text));
  if (!attributes.ok) {
    notes.push({ linter: "clippy", text: `${file}: its lint attributes are not read: ${attributes.why}.` });
    return { file, unread: true };
  }
  return { file, flags: attributes.value.flatMap(({ level, names, conditional, text: written }) => names.map(name => ({ name, level, conditional, because: `${file}: ${written}` }))) };
}

/** The package `manifest` describes: its lints, then its crate roots'. Throws `LintConfigFormatError` on lints Cargo refuses. */
function readPackage(root: string, manifest: CargoManifest, workspaceLints: CargoTable | undefined, notes: LinterNote[]): PackageLints {
  const cargo = cargoLints(manifest, workspaceLints);
  const prefix = manifest.folder === "" ? "" : `${manifest.folder}/`;
  const roots = CRATE_ROOTS.flatMap(file => readCrateRoot(root, `${prefix}${file}`, notes) ?? []);
  const setting = roots.flatMap(crate => ("flags" in crate && crate.flags.length > 0 ? [crate.file] : []));
  const labels = [...(cargo.label === undefined ? [] : [cargo.label]), ...setting];
  const unset = labels.length === 0 ? undefined : `${labels.join(" and ")} ${labels.length > 1 ? "do" : "does"} not turn it on`;
  return { folder: manifest.folder, flags: cargo.flags, roots, unset };
}

/** A member: its lints, or, where Cargo refuses them, its folder as one Rerail cannot tell (noted). */
function readMemberPackage(root: string, member: CargoManifest, workspaceLints: CargoTable | undefined, notes: LinterNote[]): PackageLints | UnreadFolder {
  const read = readFormat(() => readPackage(root, member, workspaceLints, notes));
  if (read.ok) return read.value;
  const because = `${manifestFile(member.folder)} is not read: ${read.why}`;
  notes.push({ linter: "clippy", text: `${because}; Clippy's lints there stay as the pack has them.` });
  return { folder: member.folder, because };
}

/** The root's `[workspace.lints]`, if it has one; Cargo refuses one that is not a table. */
function workspaceLintsOf(manifest: CargoManifest): CargoTable | undefined {
  const lints = manifest.tables.get("workspace")?.get("lints");
  if (lints !== undefined && !(lints instanceof Map)) throw new LintConfigFormatError("[workspace.lints] is not a table");
  return lints;
}

/**
 * The packages Cargo lints from root manifest `manifest`, and the folders Rerail cannot tell. Throws
 * `LintConfigFormatError` on a root manifest Cargo refuses: then Cargo lints nothing.
 */
function readPackages(root: string, manifest: CargoManifest, notes: LinterNote[]): { packages: PackageLints[]; unread: UnreadFolder[] } {
  if (!manifest.tables.has("package") && !manifest.tables.has("workspace")) throw new LintConfigFormatError("it has neither [package] nor [workspace], which Cargo refuses");
  const workspaceLints = workspaceLintsOf(manifest);
  const packages = manifest.tables.has("package") ? [readPackage(root, manifest, workspaceLints, notes)] : [];
  const workspace = readCargoWorkspace(root, manifest, notes);
  const unread = [...workspace.unread];
  for (const member of workspace.members) {
    const read = readMemberPackage(root, member, workspaceLints, notes);
    if ("because" in read) unread.push(read);
    else packages.push(read);
  }
  return { packages, unread };
}

/** Files beside `Cargo.toml` that may also set Clippy's lints or their options, and why they are not read. */
const UNREAD_FILES: Readonly<Record<string, string>> = {
  ".cargo/config.toml": "rustflags it sets may turn Clippy's lints on or off",
  ".cargo/config": "rustflags it sets may turn Clippy's lints on or off",
  "clippy.toml": "Rerail does not map Clippy's lint options onto rules' options",
  ".clippy.toml": "Rerail does not map Clippy's lint options onto rules' options",
};

/** The verdict on `port`: the root package's decision for every file, each member's and each unread folder's for its folder. */
function packagesVerdict(port: Port, packages: readonly PackageLints[], unread: readonly UnreadFolder[]): PortVerdict {
  const decided = packages.map(found => ({ folder: found.folder, decision: portDecision(port, found) }));
  // A glob Rerail does not expand at the root leaves every folder to the pack.
  const everywhere = unread.find(found => found.folder === "");
  const decision = everywhere === undefined ? decided.find(found => found.folder === "")?.decision : { on: "unknown" as const, because: everywhere.because };
  const folders: FolderDecision[] = [
    ...decided.flatMap(({ folder, decision }) => (folder === "" ? [] : [inFolder(folder, decision)])),
    ...unread.flatMap(({ folder, because }) => (folder === "" ? [] : [{ folder, on: "unknown" as const, because }])),
  ];
  return { ...(decision === undefined ? {} : { decision }), ...(folders.length > 0 ? { folders } : {}) };
}

/** The root manifest, read; undefined when there is none or Cargo refuses it (saying why in `notes`). */
function readRoot(root: string, notes: LinterNote[]): { packages: PackageLints[]; unread: UnreadFolder[] } | undefined {
  const manifest = readManifest(root, "");
  if (manifest === undefined) return undefined;
  if ("why" in manifest) {
    notes.push({ linter: "clippy", text: `Cargo.toml is not read: ${manifest.why}.` });
    return undefined;
  }
  const read = readFormat(() => readPackages(root, manifest, notes));
  if (read.ok) return read.value;
  notes.push({ linter: "clippy", text: `Cargo.toml is not read: ${read.why}.` });
  return undefined;
}

/**
 * Clippy's config at a repository root: each package's `[lints]` and crate roots' lint attributes. A repository none
 * of whose packages names a Clippy or rustc lint, in `Cargo.toml` or a crate root, has none.
 */
export const clippyReader: LinterReader = {
  linter: "clippy",
  kinds: ["clippy"],
  read(root, notes) {
    const found = readRoot(root, notes);
    if (found === undefined || found.packages.every(({ unset }) => unset === undefined)) return undefined;
    for (const [file, why] of Object.entries(UNREAD_FILES)) {
      if (readRootFile(root, file).present) notes.push({ linter: "clippy", text: `${file} is not read: ${why}.` });
    }
    return { verdict: port => (port.suppress.clippy.length === 0 ? undefined : packagesVerdict(port, found.packages, found.unread)) };
  },
};
