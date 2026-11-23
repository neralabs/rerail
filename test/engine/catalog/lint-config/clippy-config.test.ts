import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { clippyReader } from "../../../../src/engine/catalog/lint-config/clippy-config.js";
import { portsOf, type LinterNote, type Port, type PortDecision } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { symlinkOrSkip } from "../../../support/platform/links.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-clippy-config-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
  return root;
}

const packDirectory = (id: string): string => fileURLToPath(new URL(`../../../../packs/${id}`, import.meta.url));

let ruleCatalog: RuleCatalog;
let ports: ReadonlyMap<string, Port>;
beforeAll(() => {
  ruleCatalog = readRuleCatalog([packDirectory("rust")]);
  ports = new Map(portsOf(ruleCatalog, ["rust"]).map(port => [port.ruleId, port]));
});

/** The rust pack's port `ruleId`. */
function port(ruleId: string): Port {
  const found = ports.get(ruleId);
  if (found === undefined) throw new Error(`no port ${ruleId}`);
  return found;
}

/** A `[package]` table, so Cargo takes the manifest. */
const PACKAGE = "[package]\nname = \"demo\"\n\n";

const NEEDLESS_ELSE = "clippy-needless-else"; // style
const UNNESTED_OR = "clippy-unnested-or-patterns"; // pedantic
const ELSE_IF = "clippy-else-if-without-else"; // restriction
const DOUBLE_PARENS = "clippy-double-parens"; // complexity

/** A correctness lint's port (none ships in the pack yet): its lint, `clippy::all`, its group, `warnings`. */
const CORRECTNESS: Port = {
  ...{ ruleId: "clippy-option-env-unwrap", declared: {}, startsOn: false },
  suppress: {
    eslint: [], noqa: [], nosec: [], oxlint: [], biome: [], phpstan: [], psalm: [], phpcs: [], sqlfluff: [],
    clippy: ["clippy::option_env_unwrap", "clippy::all", "clippy::correctness", "warnings"],
  },
};

/** Clippy's config read at a repository holding `files`, its notes, and a decision per port. */
async function read(files: Readonly<Record<string, string>>) {
  const root = await repository(files);
  const notes: LinterNote[] = [];
  const config = clippyReader.read(root, notes);
  const decision = (subject: string | Port): PortDecision | undefined =>
    config?.verdict(typeof subject === "string" ? port(subject) : subject, ruleCatalog)?.decision;
  return { config, notes: notes.map(note => note.text), decision };
}

describe("clippyReader", () => {
  it("has no config without a Cargo.toml, or with one naming no lint", async () => {
    expect((await read({ "src/main.rs": "fn main() {}\n" })).config).toBeUndefined();
    const plain = await read({ "Cargo.toml": "[package]\nname = \"demo\"\n\n[lints.clippy]\n\n[lints.rustdoc]\nbroken_intra_doc_links = \"deny\"\n", "src/lib.rs": "//! Docs.\n#![doc = \"x\"]\npub fn f() {}\n" });
    expect(plain.config).toBeUndefined();
    expect(plain.notes).toEqual([]);
  });

  it("turns on a pedantic lint its group turns on, and off one the config allows, explicitly", async () => {
    const { decision } = await read({ "Cargo.toml": "[package]\nname = \"demo\"\n\n[lints.clippy]\npedantic = \"warn\"\nneedless_else = \"allow\"\n" });
    expect(decision(UNNESTED_OR)).toEqual({ on: true, explicit: true, because: "Cargo.toml [lints.clippy] pedantic = \"warn\"" });
    expect(decision(NEEDLESS_ELSE)).toEqual({ on: false, explicit: true, because: "Cargo.toml [lints.clippy] needless_else = \"allow\"" });
  });

  it("decides a lint the config does not name by Clippy's default for its group", async () => {
    const { decision } = await read({ "Cargo.toml": "[package]\nname = \"demo\"\n\n[lints.clippy]\nunwrap_used = \"deny\"\n" });
    expect(decision(DOUBLE_PARENS)).toEqual({ on: true, explicit: false, because: "Clippy's default (clippy::complexity)" });
    expect(decision(CORRECTNESS)).toEqual({ on: true, explicit: false, because: "Clippy's default (clippy::correctness)" });
    expect(decision(ELSE_IF)).toEqual({ on: false, explicit: false, because: "not turned on in Cargo.toml [lints.clippy]" });
  });

  it("applies flags in Cargo's order: by priority, then by name in reverse, a later flag winning", async () => {
    const prioritised = await read({ "Cargo.toml": PACKAGE + "[lints.clippy]\nstyle = { level = \"allow\", priority = -1 }\nneedless_else = \"warn\"\n" });
    expect(prioritised.decision(NEEDLESS_ELSE)).toEqual({ on: true, explicit: true, because: "Cargo.toml [lints.clippy] needless_else = \"warn\"" });
    // At one priority, names sort in reverse: `all` comes after `needless_else` and wins; `style` comes before it.
    const unprioritised = await read({ "Cargo.toml": PACKAGE + "[lints.clippy]\nneedless_else = \"allow\"\nall = \"warn\"\n" });
    expect(unprioritised.decision(NEEDLESS_ELSE)).toEqual({ on: true, explicit: true, because: "Cargo.toml [lints.clippy] all = \"warn\"" });
    const style = await read({ "Cargo.toml": PACKAGE + "[lints.clippy]\nneedless_else = \"allow\"\nstyle = \"warn\"\n" });
    expect(style.decision(NEEDLESS_ELSE)).toEqual({ on: false, explicit: true, because: "Cargo.toml [lints.clippy] needless_else = \"allow\"" });
    const later = await read({ "Cargo.toml": PACKAGE + "[lints.clippy]\nall = { level = \"deny\", priority = 2 }\nstyle = { level = \"allow\", priority = 1 }\n" });
    expect(later.decision(NEEDLESS_ELSE)).toEqual({ on: true, explicit: true, because: "Cargo.toml [lints.clippy] all = { level = \"deny\", priority = 2 }" });
  });

  it("keeps a forbidden lint on whatever comes after", async () => {
    const { decision } = await read({ "Cargo.toml": PACKAGE + "[lints.clippy]\nneedless_else = { level = \"forbid\", priority = -5 }\nstyle = \"allow\"\n" });
    expect(decision(NEEDLESS_ELSE)).toEqual({ on: true, explicit: true, because: "Cargo.toml [lints.clippy] needless_else = { level = \"forbid\", priority = -5 }" });
  });

  it("reads a level written as a table, with or without a priority, and a lint written with dashes", async () => {
    const { decision } = await read({
      "Cargo.toml": PACKAGE + "[lints.clippy]\nneedless-else = { level = \"deny\" }\npedantic = { level = \"warn\", priority = +1_0, msrv = \"1.70\" }\nrestriction = { level = \"allow\", priority = -128 }\n",
    });
    expect(decision(NEEDLESS_ELSE)).toEqual({ on: true, explicit: true, because: "Cargo.toml [lints.clippy] needless-else = { level = \"deny\" }" });
    expect(decision(UNNESTED_OR)).toEqual({ on: true, explicit: true, because: "Cargo.toml [lints.clippy] pedantic = { level = \"warn\", priority = 10 }" });
    expect(decision(ELSE_IF)).toEqual({ on: false, explicit: true, because: "Cargo.toml [lints.clippy] restriction = { level = \"allow\", priority = -128 }" });
  });

  /** A `Cargo.toml` allowing `style` at priority 1, then setting `needless_else` to warn at priority `written`. */
  const prioritised = (written: string): string => `${PACKAGE}[lints.clippy]\nstyle = { level = "allow", priority = 1 }\nneedless_else = { level = "warn", priority = ${written} }\n`;

  it.each<[string, number]>([
    ["0", 0], ["+0", 0], ["-0", 0], ["-1", -1], ["-128", -128], ["0x0", 0], ["0b0", 0],
    ["1", 1], ["+1_0", 10], ["127", 127], ["1_2_7", 127], ["0x7f", 127], ["0x7F", 127], ["0x1_0", 16], ["0o17", 15], ["0o1_7", 15], ["0b101", 5], ["0b1_0", 2],
  ])("orders a priority written %s as %i", async (written, priority) => {
    const { decision } = await read({ "Cargo.toml": prioritised(written) });
    // At priority 1 `needless_else` still comes after `style`: names sort in reverse.
    const expected = priority >= 1
      ? { on: true, explicit: true, because: `Cargo.toml [lints.clippy] needless_else = { level = "warn", priority = ${priority} }` }
      : { on: false, explicit: true, because: "Cargo.toml [lints.clippy] style = { level = \"allow\", priority = 1 }" };
    expect(decision(NEEDLESS_ELSE)).toEqual(expected);
  });

  it.each<[string, string]>([
    ["128", "invalid value: integer `128`, expected i8"],
    ["-129", "invalid value: integer `-129`, expected i8"],
    ["0x80", "invalid value: integer `0x80`, expected i8"],
    ["0b1111_1111", "invalid value: integer `0b1111_1111`, expected i8"],
    ...["01", "+01", "00", "1__0", "_1", "1_", "0x", "0X10", "+0x10", "-0x1", "0x_1", "0xg", "0o8", "0b2", "1.5", "1e2", "inf", "nan", "true", "1979-05-27"]
      .map((written): [string, string] => [written, `${written} is not a TOML integer, expected i8`]),
    ["\"1\"", "\"1\" is not a TOML integer, expected i8"],
    ["[1]", "an array is not a TOML integer, expected i8"],
    ["{ value = 1 }", "a table is not a TOML integer, expected i8"],
  ])("refuses a priority written %s", async (written, why) => {
    const { config, notes } = await read({ "Cargo.toml": prioritised(written) });
    expect({ config, notes }).toEqual({ config: undefined, notes: [`Cargo.toml is not read: [lints.clippy] needless_else priority: ${why}.`] });
  });

  it("does not take a rustc lint for the Clippy lint written alike", async () => {
    const { decision } = await read({ "Cargo.toml": PACKAGE + "[lints.rust]\nstyle = \"deny\"\n\n[lints.clippy]\nstyle = \"allow\"\n" });
    expect(decision(NEEDLESS_ELSE)).toEqual({ on: false, explicit: true, because: "Cargo.toml [lints.clippy] style = \"allow\"" });
  });

  it("decides a lint in a Clippy group Rerail does not know only by what the config sets", async () => {
    const ungrouped: Port = { ...CORRECTNESS, ruleId: "clippy-made-up", suppress: { ...CORRECTNESS.suppress, clippy: ["clippy::made_up", "clippy::all", "warnings"] } };
    const unset = await read({ "Cargo.toml": PACKAGE + "[lints.clippy]\nunwrap_used = \"deny\"\n" });
    expect(unset.decision(ungrouped)).toEqual({ on: "unknown", because: "Rerail does not know which Clippy group it is in, so not its default level" });
    const set = await read({ "Cargo.toml": PACKAGE + "[lints.clippy]\nall = \"deny\"\n" });
    expect(set.decision(ungrouped)).toEqual({ on: true, explicit: true, because: "Cargo.toml [lints.clippy] all = \"deny\"" });
  });

  it("keeps a forbidden lint on through a crate root's later attributes, conditional or not", async () => {
    const { decision } = await read({
      "Cargo.toml": "[package]\nname = \"demo\"\n",
      "src/lib.rs": "#![forbid(clippy::style)]\n#![cfg_attr(test, allow(clippy::needless_else))]\n#![allow(clippy::all, warnings)]\n",
    });
    expect(decision(NEEDLESS_ELSE)).toEqual({ on: true, explicit: true, because: "src/lib.rs: #![forbid(clippy::style)]" });
    expect(decision(DOUBLE_PARENS)).toEqual({ on: false, explicit: true, because: "src/lib.rs: #![allow(clippy::all, warnings)]" });
  });

  it("leaves undecided a lint a conditional attribute may set, or may silence through warnings, whatever follows", async () => {
    const { decision } = await read({
      "Cargo.toml": "[package]\nname = \"demo\"\n",
      "src/lib.rs": "#![cfg_attr(test, allow(warnings))]\n#![deny(warnings)]\n#![cfg_attr(test, deny(clippy::pedantic))]\n#![deny(clippy::pedantic)]\n",
    });
    expect(decision(NEEDLESS_ELSE)).toEqual({ on: "unknown", because: "src/lib.rs: #![cfg_attr(test, allow(warnings))] may turn it off" });
    expect(decision(UNNESTED_OR)).toEqual({ on: "unknown", because: "src/lib.rs: #![cfg_attr(test, deny(clippy::pedantic))] may set it" });
    // `warnings` reaches only a lint left at warn.
    expect(decision(CORRECTNESS)).toEqual({ on: true, explicit: false, because: "Clippy's default (clippy::correctness)" });
  });

  it("turns off a lint left at warn when warnings are allowed, and keeps a deny-level one on", async () => {
    const allowed = await read({ "Cargo.toml": PACKAGE + "[lints.rust]\nwarnings = \"allow\"\n" });
    expect(allowed.decision(NEEDLESS_ELSE)).toEqual({ on: false, explicit: true, because: "Cargo.toml [lints.rust] warnings = \"allow\"" });
    expect(allowed.decision(CORRECTNESS)).toEqual({ on: true, explicit: false, because: "Clippy's default (clippy::correctness)" });
    expect(allowed.decision(ELSE_IF)).toEqual({ on: false, explicit: false, because: "not turned on in Cargo.toml [lints.rust]" });
    const denied = await read({ "Cargo.toml": PACKAGE + "[lints.rust]\nwarnings = \"deny\"\n" });
    expect(denied.decision(NEEDLESS_ELSE)).toEqual({ on: true, explicit: false, because: "Clippy's default (clippy::style)" });
  });

  it("reads the workspace's lints a package takes, and not those it does not take", async () => {
    const taken = await read({
      "Cargo.toml": "[package]\nname = \"demo\"\n\n[lints]\nworkspace = true\n\n[workspace.lints.clippy]\npedantic = { level = \"warn\", priority = -1 }\n",
    });
    expect(taken.decision(UNNESTED_OR)).toEqual({ on: true, explicit: true, because: "Cargo.toml [workspace.lints.clippy] pedantic = { level = \"warn\", priority = -1 }" });
    expect(taken.decision(ELSE_IF)).toEqual({ on: false, explicit: false, because: "not turned on in Cargo.toml [workspace.lints.clippy]" });
    // The root package does not take [workspace.lints] unless it says so; nor does a virtual root's folder.
    const untaken = await read({ "Cargo.toml": `${PACKAGE}[workspace]\n\n[workspace.lints.clippy]\nneedless_else = "allow"\n` });
    expect(untaken.config).toBeUndefined();
  });

  it("decides each member's folder by its own lints, or the workspace's when it takes them (Cargo's opt-in)", async () => {
    const { config, notes } = await read({
      "Cargo.toml": "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.lints.clippy]\nneedless_else = \"allow\"\n",
      "crates/a/Cargo.toml": "[package]\nname = \"a\"\n\n[lints]\nworkspace = true\n",
      "crates/b/Cargo.toml": "[package]\nname = \"b\"\n\n[lints.clippy]\npedantic = \"warn\"\n",
      "crates/c/Cargo.toml": "[package]\nname = \"c\"\n",
      "crates/d/Cargo.toml": "[package]\nname = \"d\"\n\n[lints]\nworkspace = false\n",
    });
    expect(config?.verdict(port(NEEDLESS_ELSE), ruleCatalog)).toEqual({
      folders: [
        { folder: "crates/a", on: false, explicit: true, because: "Cargo.toml [workspace.lints.clippy] needless_else = \"allow\"" },
        { folder: "crates/b", on: true, explicit: false, because: "Clippy's default (clippy::style)" },
        { folder: "crates/c", on: true, explicit: false, because: "Clippy's default (clippy::style)" },
        { folder: "crates/d", on: "unknown", because: "crates/d/Cargo.toml is not read: [lints] workspace is not true, the only value Cargo takes" },
      ],
    });
    expect(config?.verdict(port(UNNESTED_OR), ruleCatalog)?.folders?.slice(0, 3)).toEqual([
      { folder: "crates/a", on: false, explicit: false, because: "not turned on in Cargo.toml [workspace.lints.clippy]" },
      { folder: "crates/b", on: true, explicit: true, because: "crates/b/Cargo.toml [lints.clippy] pedantic = \"warn\"" },
      { folder: "crates/c", on: false, explicit: false, because: "Clippy's default (clippy::pedantic)" },
    ]);
    expect(notes).toEqual(["crates/d/Cargo.toml is not read: [lints] workspace is not true, the only value Cargo takes; Clippy's lints there stay as the pack has them."]);
  });

  it("leaves every folder to the pack when a member glob at the root is not expanded", async () => {
    const { config } = await read({ "Cargo.toml": `${PACKAGE}[lints.clippy]\nneedless_else = "allow"\n\n[workspace]\nmembers = ["**"]\n` });
    expect(config?.verdict(port(NEEDLESS_ELSE), ruleCatalog)).toEqual({
      decision: { on: "unknown", because: "Cargo.toml [workspace] members \"**\" is not expanded: Rerail expands *, ? and [...] in a folder's name only" },
    });
  });

  it("applies a crate root's lint attributes after Cargo's flags", async () => {
    const { decision } = await read({
      "Cargo.toml": PACKAGE + "[lints.clippy]\npedantic = \"warn\"\n",
      "src/lib.rs": "#!/usr/bin/env run-cargo-script\n//! The crate.\n/* A note. */\n#![cfg_attr(clippy, allow(clippy::unnested_or_patterns, reason = \"style\"))]\n#![deny(\n  clippy::restriction,\n)]\n\n#[derive(Debug)]\npub struct S;\n#![allow(clippy::double_parens)]\n",
    });
    expect(decision(UNNESTED_OR)).toEqual({
      on: false,
      explicit: true,
      because: "src/lib.rs: #![cfg_attr(clippy, allow(clippy::unnested_or_patterns, reason = \"style\"))]",
    });
    expect(decision(ELSE_IF)).toEqual({ on: true, explicit: true, because: "src/lib.rs: #![deny( clippy::restriction, )]" });
    // An inner attribute after the first item is not the crate's.
    expect(decision(DOUBLE_PARENS)).toEqual({ on: true, explicit: false, because: "Clippy's default (clippy::complexity)" });
  });

  it("reads crate roots' attributes alone, and leaves the pack's value where they differ or may apply", async () => {
    const { config, decision } = await read({
      "Cargo.toml": "[package]\nname = \"demo\"\n",
      "src/lib.rs": "#![warn(clippy::pedantic)]\n#![cfg_attr(test, allow(clippy::needless_else))]\n",
      "src/main.rs": "#![warn(clippy::pedantic)]\nfn main() {}\n",
    });
    expect(config).toBeDefined();
    expect(decision(UNNESTED_OR)).toEqual({ on: true, explicit: true, because: "src/lib.rs: #![warn(clippy::pedantic)]" });
    expect(decision(NEEDLESS_ELSE)).toEqual({ on: "unknown", because: "src/lib.rs: #![cfg_attr(test, allow(clippy::needless_else))] may set it" });
    expect(decision(ELSE_IF)).toEqual({ on: false, explicit: false, because: "not turned on in src/lib.rs and src/main.rs" });
    const differing = await read({ "Cargo.toml": "[package]\nname = \"demo\"\n", "src/lib.rs": "#![warn(clippy::pedantic)]\n", "src/main.rs": "fn main() {}\n" });
    expect(differing.decision(UNNESTED_OR)).toEqual({ on: "unknown", because: "src/lib.rs and src/main.rs set it differently" });
  });

  it("gives no verdict on a port that is not Clippy's", async () => {
    const { config } = await read({ "Cargo.toml": PACKAGE + "[lints.clippy]\nall = \"deny\"\n" });
    expect(config?.verdict({ ...CORRECTNESS, suppress: { ...CORRECTNESS.suppress, clippy: [] } }, ruleCatalog)).toBeUndefined();
  });

  it("reads a package's own [lints] over [workspace.lints], and none when the table it takes names no Clippy or rustc lint", async () => {
    const own = await read({ "Cargo.toml": "[package]\nname = \"demo\"\n\n[lints.clippy]\nneedless_else = \"allow\"\n\n[workspace.lints.clippy]\nneedless_else = \"warn\"\n" });
    expect(own.decision(NEEDLESS_ELSE)).toEqual({ on: false, explicit: true, because: "Cargo.toml [lints.clippy] needless_else = \"allow\"" });
    const rustdoc = await read({ "Cargo.toml": "[package]\nname = \"demo\"\n\n[lints]\nworkspace = true\n\n[workspace.lints.rustdoc]\nbroken_intra_doc_links = \"deny\"\n" });
    expect({ config: rustdoc.config, notes: rustdoc.notes }).toEqual({ config: undefined, notes: [] });
  });

  it.each<[string, string, string]>([
    ["a level Cargo does not take", "[lints.clippy]\nneedless_else = \"off\"\n", "[lints.clippy] needless_else is not \"allow\", \"warn\", \"deny\" or \"forbid\", or a table with such a level"],
    ["`expect`, which Cargo does not take", "[lints.clippy]\nneedless_else = \"expect\"\n", "[lints.clippy] needless_else is not \"allow\", \"warn\", \"deny\" or \"forbid\", or a table with such a level"],
    ["a number for a level", "[lints.clippy]\nneedless_else = 1\n", "[lints.clippy] needless_else is not \"allow\", \"warn\", \"deny\" or \"forbid\", or a table with such a level"],
    ["a table with no level", "[lints.clippy]\nstyle = { priority = 1 }\n", "[lints.clippy] style is not \"allow\", \"warn\", \"deny\" or \"forbid\", or a table with such a level"],
    ["a tool that is not a table", "[lints]\nclippy = \"deny\"\n", "[lints.clippy] is not a table"],
    ["workspace = true beside lints", "[lints]\nworkspace = true\nclippy = { all = \"deny\" }\n[workspace.lints.clippy]\nall = \"warn\"\n", "[lints] takes workspace = true and sets lints too, which Cargo refuses"],
    ["workspace = false", "[lints]\nworkspace = false\n", "[lints] workspace is not true, the only value Cargo takes"],
    ["workspace as a string", "[lints]\nworkspace = \"true\"\n[workspace.lints.clippy]\nall = \"warn\"\n", "[lints] workspace is not true, the only value Cargo takes"],
    ["workspace = true with no workspace lints", "[lints]\nworkspace = true\n", "[lints] takes workspace = true, but there is no [workspace.lints]"],
    ["[workspace.lints] that is not a table", "[workspace]\nlints = 1\n", "[workspace.lints] is not a table"],
    ["members that are not a list of strings", "[workspace]\nmembers = \"crates/*\"\n", "[workspace] members is not a list of strings"],
    ["broken TOML", "[lints.clippy\n", "line 4: a table header is not closed"],
  ])("refuses a Cargo.toml with %s, saying why", async (_, text, why) => {
    const { config, notes } = await read({ "Cargo.toml": PACKAGE + text });
    expect({ config, notes }).toEqual({ config: undefined, notes: [`Cargo.toml is not read: ${why}.`] });
  });

  it("refuses a Cargo.toml with neither [package] nor [workspace], as Cargo does", async () => {
    const { config, notes } = await read({ "Cargo.toml": "[lints.clippy]\nall = \"deny\"\n" });
    expect({ config, notes }).toEqual({ config: undefined, notes: ["Cargo.toml is not read: it has neither [package] nor [workspace], which Cargo refuses."] });
  });

  it("refuses a Cargo.toml or crate root that is not a regular file in the repository", async context => {
    const root = await repository({ "elsewhere/Cargo.toml": "[lints.clippy]\nall = \"allow\"\n" });
    symlinkOrSkip(context, join(root, "elsewhere", "Cargo.toml"), join(root, "Cargo.toml"));
    const notes: LinterNote[] = [];
    expect(clippyReader.read(root, notes)).toBeUndefined();
    expect(notes).toEqual([{ linter: "clippy", text: "Cargo.toml is not read: it is not a regular file of at most 1 MiB in the repository." }]);
    const linked = await repository({ "Cargo.toml": "[package]\nname = \"demo\"\n", "elsewhere/lib.rs": "#![allow(clippy::pedantic)]\n", "src/main.rs": "#![warn(clippy::pedantic)]\n" });
    symlinkOrSkip(context, join(linked, "elsewhere", "lib.rs"), join(linked, "src", "lib.rs"));
    const linkedNotes: LinterNote[] = [];
    const config = clippyReader.read(linked, linkedNotes);
    expect(config?.verdict(port(UNNESTED_OR), ruleCatalog)?.decision).toEqual({ on: "unknown", because: "src/lib.rs is not read and may set it" });
    expect(linkedNotes).toEqual([{ linter: "clippy", text: "src/lib.rs: its lint attributes are not read: it is not a regular file of at most 1 MiB in the repository." }]);
  });

  it("leaves what a crate root it cannot parse may set unknown", async () => {
    const { decision, notes } = await read({ "Cargo.toml": PACKAGE + "[lints.clippy]\npedantic = \"warn\"\n", "src/lib.rs": "#![allow(clippy::pedantic\n" });
    expect(decision(UNNESTED_OR)).toEqual({ on: "unknown", because: "src/lib.rs is not read and may set it" });
    expect(notes).toEqual(["src/lib.rs: its lint attributes are not read: line 2: a ( is not closed."]);
  });

  it("notes the files beside Cargo.toml that may set Clippy's lint options", async () => {
    const { notes } = await read({ "Cargo.toml": PACKAGE + "[lints.clippy]\nall = \"deny\"\n", ".cargo/config.toml": "[build]\nrustflags = [\"-Dwarnings\"]\n", "clippy.toml": "msrv = \"1.70\"\n" });
    expect(notes).toEqual(["clippy.toml is not read: Rerail does not map Clippy's lint options onto rules' options."]);
  });

  describe("crate roots", () => {
    const ALLOWED = PACKAGE + "[lints.clippy]\nneedless_else = \"allow\"\n";
    const WARN = "#![warn(clippy::needless_else)]\n";
    const explicitOn = (file: string): PortDecision => ({ on: true, explicit: true, because: `${file}: #![warn(clippy::needless_else)]` });
    const mayBe = (file: string): PortDecision => ({ on: "unknown", because: `${file}: #![warn(clippy::needless_else)] may set it` });

    it("reads the library at its [lib] path, not src/lib.rs", async () => {
      const { decision } = await read({ "Cargo.toml": `${ALLOWED}\n[lib]\npath = "lib.rs"\n`, "lib.rs": WARN, "src/lib.rs": "#![allow(clippy::needless_else)]\n" });
      expect(decision(NEEDLESS_ELSE)).toEqual(explicitOn("lib.rs"));
    });

    it("reads no library with autolib = false and no [lib], and src/lib.rs with a [lib] that names no path", async () => {
      const noLibrary = await read({ "Cargo.toml": PACKAGE + "autolib = false\n\n[lints.clippy]\nneedless_else = \"allow\"\n", "src/lib.rs": WARN });
      expect(noLibrary.decision(NEEDLESS_ELSE)).toEqual({ on: false, explicit: true, because: "Cargo.toml [lints.clippy] needless_else = \"allow\"" });
      const declared = await read({ "Cargo.toml": PACKAGE + "autolib = false\n\n[lib]\nname = \"demo\"\n\n[lints.clippy]\nneedless_else = \"allow\"\n", "src/lib.rs": WARN });
      expect(declared.decision(NEEDLESS_ELSE)).toEqual(explicitOn("src/lib.rs"));
    });

    it("reads every inferred binary: src/main.rs, src/bin/*.rs and src/bin/*/main.rs", async () => {
      const { decision } = await read({ "Cargo.toml": ALLOWED, "src/bin/a.rs": WARN, "src/bin/b/main.rs": WARN, "src/bin/README.md": "Tools.\n" });
      expect(decision(NEEDLESS_ELSE)).toEqual(explicitOn("src/bin/a.rs"));
      const differing = await read({ "Cargo.toml": ALLOWED, "src/main.rs": "fn main() {}\n", "src/bin/a.rs": WARN });
      expect(differing.decision(NEEDLESS_ELSE)).toEqual({ on: "unknown", because: "src/main.rs and src/bin/a.rs set it differently" });
    });

    it("reads a [[bin]] path; an inferred binary beside a [[bin]], or one with required-features, may or may not be checked", async () => {
      const declared = await read({ "Cargo.toml": `${ALLOWED}\n[[bin]]\nname = "x"\npath = "tools/x.rs"\n`, "tools/x.rs": WARN });
      expect(declared.decision(NEEDLESS_ELSE)).toEqual(explicitOn("tools/x.rs"));
      const beside = await read({ "Cargo.toml": `${ALLOWED}\n[[bin]]\nname = "x"\npath = "tools/x.rs"\n`, "tools/x.rs": "fn main() {}\n", "src/main.rs": WARN });
      expect(beside.decision(NEEDLESS_ELSE)).toEqual(mayBe("src/main.rs"));
      const featured = await read({ "Cargo.toml": `${ALLOWED}\n[[bin]]\nname = "x"\npath = "tools/x.rs"\nrequired-features = ["f"]\n`, "tools/x.rs": WARN });
      expect(featured.decision(NEEDLESS_ELSE)).toEqual(mayBe("tools/x.rs"));
      const same = await read({ "Cargo.toml": `${ALLOWED}\n[[bin]]\nname = "demo"\npath = "src/main.rs"\n`, "src/main.rs": WARN });
      expect(same.decision(NEEDLESS_ELSE)).toEqual(explicitOn("src/main.rs"));
    });

    it("reads no inferred binary with autobins = false, unless a [[bin]] without a path may name one", async () => {
      const off = await read({ "Cargo.toml": PACKAGE + "autobins = false\n\n[lints.clippy]\nneedless_else = \"allow\"\n", "src/main.rs": WARN });
      expect(off.decision(NEEDLESS_ELSE)).toEqual({ on: false, explicit: true, because: "Cargo.toml [lints.clippy] needless_else = \"allow\"" });
      const named = await read({ "Cargo.toml": PACKAGE + "autobins = false\n\n[lints.clippy]\nneedless_else = \"allow\"\n\n[[bin]]\nname = \"demo\"\n", "src/main.rs": WARN });
      expect(named.decision(NEEDLESS_ELSE)).toEqual(mayBe("src/main.rs"));
    });

    it("reads the build script: build.rs, package.build's path, none with build = false", async () => {
      expect((await read({ "Cargo.toml": ALLOWED, "build.rs": WARN })).decision(NEEDLESS_ELSE)).toEqual(explicitOn("build.rs"));
      expect((await read({ "Cargo.toml": PACKAGE + "build = true\n\n[lints.clippy]\nneedless_else = \"allow\"\n", "build.rs": WARN })).decision(NEEDLESS_ELSE)).toEqual(explicitOn("build.rs"));
      expect((await read({ "Cargo.toml": PACKAGE + "build = \"tools/b.rs\"\n\n[lints.clippy]\nneedless_else = \"allow\"\n", "tools/b.rs": WARN, "build.rs": "fn main() {}\n" })).decision(NEEDLESS_ELSE)).toEqual(explicitOn("tools/b.rs"));
      expect((await read({ "Cargo.toml": PACKAGE + "build = false\n\n[lints.clippy]\nneedless_else = \"allow\"\n", "build.rs": WARN })).decision(NEEDLESS_ELSE)?.on).toBe(false);
    });

    it.each<[string, string, string, string]>([
      ["a [lib] path that is no string", "", "[lib]\npath = 3\n", "[lib] path is not a path"],
      ["a package.build that is no path", "build = 3\n", "", "package.build is not a path"],
      ["a [[bin]] path outside the repository", "", "[[bin]]\nname = \"x\"\npath = \"/x.rs\"\n", "[[bin]] path \"/x.rs\" is outside the repository"],
      ["a table under a [[bin]]", "", "[[bin]]\nname = \"x\"\n[bin.test]\n", "line 9: [bin.test] is a table under an array of tables, which Rerail does not read"],
    ])("refuses a package with %s, saying why", async (_, keys, tables, why) => {
      const manifest = `[package]\nname = "demo"\n${keys}\n[lints.clippy]\nall = "deny"\n\n${tables}`;
      const { config, notes } = await read({ "Cargo.toml": manifest });
      expect({ config, notes }).toEqual({ config: undefined, notes: [`Cargo.toml is not read: ${why}.`] });
      const member = await read({ "Cargo.toml": `${PACKAGE}[lints.clippy]\nall = "deny"\n\n[workspace]\nmembers = ["a"]\n`, "a/Cargo.toml": manifest });
      expect(member.config?.verdict(port(NEEDLESS_ELSE), ruleCatalog)?.folders).toEqual([{ folder: "a", on: "unknown", because: `a/Cargo.toml is not read: ${why}` }]);
    });
  });

  describe("rustflags", () => {
    const ALLOWED = PACKAGE + "[lints.clippy]\nneedless_else = \"allow\"\n";

    it("applies the root config's [build] rustflags after [lints], as Cargo passes them", async () => {
      const { decision, notes } = await read({ "Cargo.toml": ALLOWED, ".cargo/config.toml": "[build]\nrustflags = [\"-W\", \"clippy::needless_else\"]\n" });
      expect(decision(NEEDLESS_ELSE)).toEqual({ on: true, explicit: true, because: ".cargo/config.toml [build] rustflags -W clippy::needless_else" });
      expect(notes).toEqual([]);
      const silenced = await read({ "Cargo.toml": PACKAGE + "[lints.clippy]\nneedless_else = \"warn\"\n", ".cargo/config.toml": "build.rustflags = \"-C opt-level=0 --cfg=x -L native=lib -lfoo -Awarnings\"\n" });
      expect(silenced.decision(NEEDLESS_ELSE)).toEqual({ on: false, explicit: true, because: ".cargo/config.toml [build] rustflags -A warnings" });
      const linking = await read({ "Cargo.toml": ALLOWED, ".cargo/config.toml": "[build]\nrustflags = [\"-C\", \"target-cpu=native\"]\n" });
      expect(linking.decision(ELSE_IF)).toEqual({ on: false, explicit: false, because: "not turned on in Cargo.toml [lints.clippy]" });
    });

    it("reads each form of a lint flag, a lint's - as _", async () => {
      for (const flags of ["\"-Wclippy::needless-else\"", "\"--warn=clippy::needless_else\"", "\"--warn\", \"clippy::needless_else\"", "\"-D\", \"clippy::style\""]) {
        const { decision } = await read({ "Cargo.toml": ALLOWED, ".cargo/config.toml": `[build]\nrustflags = [${flags}]\n` });
        expect(decision(NEEDLESS_ELSE)?.on).toBe(true);
      }
    });

    it("reads .cargo/config over .cargo/config.toml, as Cargo does", async () => {
      const { decision } = await read({ "Cargo.toml": ALLOWED, ".cargo/config": "[build]\nrustflags = [\"-W\", \"clippy::needless_else\"]\n", ".cargo/config.toml": "[build]\nrustflags = [\"-A\", \"clippy::needless_else\"]\n" });
      expect(decision(NEEDLESS_ELSE)).toEqual({ on: true, explicit: true, because: ".cargo/config [build] rustflags -W clippy::needless_else" });
    });

    it("reads rustflags alone as a config", async () => {
      const { decision } = await read({ "Cargo.toml": PACKAGE, ".cargo/config.toml": "[build]\nrustflags = [\"-W\", \"clippy::pedantic\"]\n" });
      expect(decision(UNNESTED_OR)).toEqual({ on: true, explicit: true, because: ".cargo/config.toml [build] rustflags -W clippy::pedantic" });
      expect(decision(ELSE_IF)).toEqual({ on: false, explicit: false, because: "not turned on in .cargo/config.toml rustflags" });
    });

    it("leaves to the pack what a [target]'s rustflags, which replace [build]'s on its platforms, may set", async () => {
      const { decision } = await read({ "Cargo.toml": ALLOWED, ".cargo/config.toml": "[build]\nrustflags = [\"-W\", \"clippy::needless_else\"]\n[target.'cfg(unix)']\nrustflags = [\"-W\", \"clippy::pedantic\"]\n" });
      expect(decision(NEEDLESS_ELSE)).toEqual({ on: "unknown", because: ".cargo/config.toml [build] rustflags -W clippy::needless_else may set it" });
      expect(decision(UNNESTED_OR)).toEqual({ on: "unknown", because: ".cargo/config.toml [target.cfg(unix)] rustflags -W clippy::pedantic may set it" });
      const empty = await read({ "Cargo.toml": ALLOWED, ".cargo/config.toml": "[build]\nrustflags = [\"-W\", \"clippy::needless_else\"]\n[target.x86_64-pc-windows-msvc]\nrustflags = []\nlinker = \"lld\"\n[target.other]\nlinker = \"ld\"\n" });
      expect(empty.decision(NEEDLESS_ELSE)?.on).toBe(true);
    });

    it("leaves to the pack, in a config's folder below the root, what its rustflags may set: Cargo reads it only when run there", async () => {
      const { config } = await read({
        "Cargo.toml": `${ALLOWED}\n[workspace]\nmembers = ["crates/a"]\n`,
        "crates/a/Cargo.toml": "[package]\nname = \"a\"\n\n[lints.clippy]\nneedless_else = \"allow\"\n",
        "crates/a/.cargo/config.toml": "[build]\nrustflags = [\"-W\", \"clippy::needless_else\"]\n",
      });
      expect(config?.verdict(port(NEEDLESS_ELSE), ruleCatalog)).toEqual({
        decision: { on: false, explicit: true, because: "Cargo.toml [lints.clippy] needless_else = \"allow\"" },
        folders: [{ folder: "crates/a", on: "unknown", because: "crates/a/.cargo/config.toml [build] rustflags -W clippy::needless_else may set it" }],
      });
    });

    it.each<[string, string, string]>([
      ["a flag Rerail does not read", "[build]\nrustflags = [\"-Zunstable-options\"]\n", ".cargo/config.toml [build] rustflags has \"-Zunstable-options\", which Rerail does not read and which may set lint levels"],
      ["--cap-lints", "[build]\nrustflags = \"--cap-lints allow\"\n", ".cargo/config.toml [build] rustflags has \"--cap-lints\", which Rerail does not read and which may set lint levels"],
      ["-W=", "[build]\nrustflags = [\"-W=clippy::needless_else\"]\n", ".cargo/config.toml [build] rustflags has \"-W=clippy::needless_else\", which Rerail does not read and which may set lint levels"],
      ["a flag missing its value", "[target.x]\nrustflags = [\"-W\"]\n", ".cargo/config.toml [target.x] rustflags ends in -W without its value"],
      ["rustflags that are no strings", "[build]\nrustflags = [1]\n", ".cargo/config.toml [build] rustflags is not a string or a list of strings"],
      ["an include", "include = [\"other.toml\"]\n", "it includes other config files, which Rerail does not follow"],
      ["broken TOML", "[build\n", "line 1: a table header is not closed"],
    ])("reads nothing where the root's config has %s, saying why", async (_, text, why) => {
      const { config, notes } = await read({ "Cargo.toml": ALLOWED, ".cargo/config.toml": text });
      expect({ config, notes }).toEqual({ config: undefined, notes: [`.cargo/config.toml is not read: ${why}.`] });
    });

    it("reads nothing where the root's .cargo/config is not a file", async () => {
      const { config, notes } = await read({ "Cargo.toml": ALLOWED, ".cargo/config/README": "x\n" });
      expect({ config, notes }).toEqual({ config: undefined, notes: [".cargo/config is not read: it is not a regular file of at most 1 MiB in the repository."] });
    });

    it("leaves to the pack every lint of the packages where Cargo reads a config below the root it cannot read", async () => {
      const { config, notes } = await read({
        "Cargo.toml": `${ALLOWED}\n[workspace]\nmembers = ["crates/a", "b"]\n`,
        "crates/a/Cargo.toml": "[package]\nname = \"a\"\n",
        "b/Cargo.toml": "[package]\nname = \"b\"\n",
        "crates/.cargo/config.toml": "include = [\"x.toml\"]\n",
      });
      const because = "crates/.cargo/config.toml is not read: it includes other config files, which Rerail does not follow";
      expect(config?.verdict(port(NEEDLESS_ELSE), ruleCatalog)).toEqual({
        decision: { on: false, explicit: true, because: "Cargo.toml [lints.clippy] needless_else = \"allow\"" },
        folders: [{ folder: "b", on: true, explicit: false, because: "Clippy's default (clippy::style)" }, { folder: "crates/a", on: "unknown", because }],
      });
      expect(notes).toEqual([`${because}; Clippy's lints there stay as the pack has them.`]);
    });
  });
});
