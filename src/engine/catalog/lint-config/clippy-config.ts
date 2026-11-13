/**
 * Clippy's config as set-up reads it (`linter-config.ts`): the lint levels the root `Cargo.toml` sets in `[lints]`
 * (or the `[workspace.lints]` it takes, or a workspace's for its members), and the inner lint attributes of the crate
 * roots `src/lib.rs` and `src/main.rs`, all read as data, never built.
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
 * (`clippy::needless_else`, `clippy::all`, `clippy::style`, `warnings`). When nothing the config sets names them,
 * Clippy's default for the lint's group decides.
 */
import { LintConfigFormatError, readFormat, readTomlTrees, type TomlValue } from "./formats/lint-config-formats.js";
import { NOT_A_FILE, readRootFile, type LinterNote, type LinterReader, type Port, type PortDecision } from "./linter-config.js";
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

/** What `Cargo.toml` sets: its flags in Cargo's order, the table they come from, and whether it names members. */
interface CargoLints {
  readonly flags: readonly LintFlag[];
  /** The table the flags come from, for the person: `Cargo.toml [lints.clippy]`; undefined when there are none. */
  readonly label: string | undefined;
  readonly members: boolean;
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

/** What `flags`, applied in order, decide for `port`; `labels` name the config for an implicit off. */
function flagsDecision(port: Port, flags: readonly LintFlag[], labels: readonly string[]): PortDecision {
  const { lint, warnings } = replay(new Set(port.suppress.clippy), flags);
  if (lint.uncertain !== undefined) return { on: "unknown", because: `${lint.uncertain.because} may set it` };
  const group = port.suppress.clippy.find(name => Object.hasOwn(GROUP_LEVELS, name)) ?? "";
  const level = lint.flag?.level ?? GROUP_LEVELS[group];
  if (level === undefined) return { on: "unknown", because: "Rerail does not know which Clippy group it is in, so not its default level" };
  return levelDecision(level, { lint: lint.flag, warnings, group }, labels);
}

/**
 * The decision for a lint at `level`, set by `lint` or else by its `group`'s default, once `warnings` applies to it;
 * `labels` name the config for an implicit off.
 */
function levelDecision(level: RustLintLevel, set: { lint: LintFlag | undefined; warnings: Slot; group: string }, labels: readonly string[]): PortDecision {
  const silenced = level === "warn" ? warningsDecision(set.warnings) : undefined;
  if (silenced !== undefined) return silenced;
  if (set.lint !== undefined) return { on: REPORTS[level], explicit: true, because: set.lint.because };
  if (REPORTS[level]) return { on: true, explicit: false, because: `Clippy's default (${set.group})` };
  return { on: false, explicit: false, because: `${labels.join(" and ")} ${labels.length > 1 ? "do" : "does"} not turn it on` };
}

/** One decision for every file from each crate root's: unknown where one is, or where they differ. */
function agreedDecision(decisions: readonly PortDecision[], files: readonly string[]): PortDecision {
  const unknown = decisions.find((decision): decision is Unknown => decision.on === "unknown");
  if (unknown !== undefined) return unknown;
  const decided = decisions.filter((decision): decision is Decided => decision.on !== "unknown");
  if (new Set(decided.map(decision => decision.on)).size > 1) return { on: "unknown", because: `${files.join(" and ")} set it differently` };
  return decided.find(decision => decision.explicit) ?? decided[0]!;
}

/** What Cargo's flags, then each crate root's attributes, decide for `port`. */
function portDecision(port: Port, cargo: readonly LintFlag[], roots: readonly CrateRoot[], labels: readonly string[]): PortDecision {
  if (roots.length === 0) return flagsDecision(port, cargo, labels);
  const decisions = roots.map((root): PortDecision => ("unread" in root
    ? { on: "unknown", because: `${root.file} is not read and may set it` }
    : flagsDecision(port, [...cargo, ...root.flags], labels)));
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

/** The flags `[<table>]` (`lints` or `workspace.lints`) sets, in the order Cargo passes them to rustc. */
function cargoFlags(table: string, lints: ReadonlyMap<string, TomlValue>): LintFlag[] {
  const flags = TOOLS.flatMap(tool => {
    const entries = lints.get(tool);
    if (entries === undefined) return [];
    if (!(entries instanceof Map)) throw new LintConfigFormatError(`[${table}.${tool}] is not a table`);
    return [...entries].map(([name, value]) => {
      const entry = cargoEntry(`[${table}.${tool}] ${name}`, value);
      const lint = (tool === "rust" ? name : `clippy::${name}`).replaceAll("-", "_");
      const flag: LintFlag = { name: lint, level: entry.level, conditional: false, because: `Cargo.toml [${table}.${tool}] ${name} = ${entry.written}` };
      return { flag, priority: entry.priority, written: name };
    });
  });
  // Cargo sorts (priority, Reverse(name), option): by priority, then by name as written in reverse. `option` only
  // orders a rustc lint and a Clippy lint written alike (`--warn=style`, `--allow=clippy::style`): different lints.
  flags.sort((a, b) => a.priority - b.priority || compare(b.written, a.written));
  return flags.map(({ flag }) => flag);
}

const compare = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** The table a package with `[lints] workspace = true` takes its lints from, refusing what Cargo refuses. */
function inheritedTable(lints: ReadonlyMap<string, TomlValue>, workspace: boolean): string {
  if (scalarText(lints.get("workspace")) !== "true") throw new LintConfigFormatError("[lints] workspace is not true, the only value Cargo takes");
  if (lints.size > 1) throw new LintConfigFormatError("[lints] takes workspace = true and sets lints too, which Cargo refuses");
  if (!workspace) throw new LintConfigFormatError("[lints] takes workspace = true, but there is no [workspace.lints]");
  return "workspace.lints";
}

const namesLints = (table: ReadonlyMap<string, TomlValue> | undefined): boolean => TOOLS.some(tool => table?.has(tool) === true);

/**
 * The table whose lints apply: the root package's `[lints]`, or the `[workspace.lints]` it takes, or, with neither,
 * a workspace's `[workspace.lints]` for its members; undefined when none names Clippy's or rustc's lints.
 */
function lintsTable(trees: ReadonlyMap<string, ReadonlyMap<string, TomlValue>>): string | undefined {
  const lints = trees.get("lints");
  if (lints?.has("workspace") === true) return inheritedTable(lints, trees.has("workspace.lints"));
  if (namesLints(lints)) return "lints";
  return namesLints(trees.get("workspace.lints")) ? "workspace.lints" : undefined;
}

/** `Cargo.toml` `text`'s lints, in the order rustc applies them. Throws `LintConfigFormatError` on what Cargo refuses. */
function cargoLints(text: string): CargoLints {
  const { trees, paths } = readTomlTrees(text, ["lints", "workspace.lints"]);
  const table = lintsTable(trees);
  const lints = table === undefined ? undefined : trees.get(table);
  if (table === undefined || lints === undefined) return { flags: [], label: undefined, members: paths.includes("workspace.members") };
  const flags = cargoFlags(table, lints);
  // A table naming no lint, `[lints.clippy]` alone, sets nothing: no config.
  const label = flags.length === 0 ? undefined : `Cargo.toml [${table}.${lints.has("clippy") ? "clippy" : "rust"}]`;
  return { flags, label, members: paths.includes("workspace.members") };
}

/** The root `Cargo.toml`'s lints; undefined when there is none or it is not read (saying why in `notes`). */
function readCargoLints(root: string, notes: LinterNote[]): CargoLints | undefined {
  const read = readRootFile(root, "Cargo.toml");
  if (!read.present) return undefined;
  if (read.text === undefined) {
    notes.push({ linter: "clippy", text: `Cargo.toml is not read: ${NOT_A_FILE}` });
    return undefined;
  }
  const text = read.text;
  const cargo = readFormat(() => cargoLints(text));
  if (cargo.ok) return cargo.value;
  notes.push({ linter: "clippy", text: `Cargo.toml is not read: ${cargo.why}.` });
  return undefined;
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

/** Files beside `Cargo.toml` that may also set Clippy's lints or their options, and why they are not read. */
const UNREAD_FILES: Readonly<Record<string, string>> = {
  ".cargo/config.toml": "rustflags it sets may turn Clippy's lints on or off",
  ".cargo/config": "rustflags it sets may turn Clippy's lints on or off",
  "clippy.toml": "Rerail does not map Clippy's lint options onto rules' options",
  ".clippy.toml": "Rerail does not map Clippy's lint options onto rules' options",
};

/** Notes what may also set Clippy's lints and is not read. */
function noteUnread(root: string, cargo: CargoLints, notes: LinterNote[]): void {
  if (cargo.members && cargo.label !== undefined) {
    notes.push({ linter: "clippy", text: `Cargo.toml: workspace members' own [lints] tables are not read; ${cargo.label} is read for every file.` });
  }
  for (const [file, why] of Object.entries(UNREAD_FILES)) {
    if (readRootFile(root, file).present) notes.push({ linter: "clippy", text: `${file} is not read: ${why}.` });
  }
}

/**
 * Clippy's config at a repository root: `Cargo.toml`'s `[lints]` and the crate roots' lint attributes. A repository
 * whose `Cargo.toml` names no Clippy or rustc lint and whose crate roots set no lint level has none.
 */
export const clippyReader: LinterReader = {
  linter: "clippy",
  kinds: ["clippy"],
  read(root, notes) {
    const cargo = readCargoLints(root, notes);
    if (cargo === undefined) return undefined;
    const roots = CRATE_ROOTS.flatMap(file => readCrateRoot(root, file, notes) ?? []);
    const setting = roots.flatMap(crate => ("flags" in crate && crate.flags.length > 0 ? [crate.file] : []));
    if (cargo.label === undefined && setting.length === 0) return undefined;
    noteUnread(root, cargo, notes);
    const labels = [...(cargo.label === undefined ? [] : [cargo.label]), ...setting];
    return {
      verdict: port => (port.suppress.clippy.length === 0 ? undefined : { decision: portDecision(port, cargo.flags, roots, labels) }),
    };
  },
};
