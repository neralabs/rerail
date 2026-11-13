import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { biomeReader } from "../../../../src/engine/catalog/lint-config/biome-config.js";
import { portsOf, settleVerdicts, type LinterNote, type Port, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-biome-config-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
  return root;
}

const packDirectory = (id: string): string => decodeURIComponent(new URL(`../../../../packs/${id}`, import.meta.url).pathname);

let ruleCatalog: RuleCatalog;
let ports: ReadonlyMap<string, Port>;
beforeAll(() => {
  ruleCatalog = readRuleCatalog([packDirectory("javascript")]);
  ports = new Map(portsOf(ruleCatalog, ["javascript"]).map(port => [port.ruleId, port]));
});

/** The javascript pack's port `ruleId`. */
function port(ruleId: string): Port {
  const found = ports.get(ruleId);
  if (found === undefined) throw new Error(`no port ${ruleId}`);
  return found;
}

/** A port of Biome rule `name` (`lint/<group>/<rule>`) alone, for rules no shipped port names. */
const biomePort = (name: string, startsOn = true): Port => ({
  ruleId: `biome-${name.split("/").at(-1)!}`,
  suppress: { eslint: [], noqa: [], nosec: [], oxlint: [], biome: [name], phpstan: [], psalm: [], phpcs: [], clippy: [], sqlfluff: [] },
  declared: {},
  startsOn,
});

const NO_DEBUGGER = "eslint-no-debugger"; // suspicious/noDebugger: recommended
const CURLY = "eslint-curly"; // style/useBlockStatements: not recommended
const EQEQEQ = "eslint-eqeqeq"; // suspicious/noDoubleEquals: recommended
const HOOKS = biomePort("lint/correctness/useHookAtTopLevel"); // recommended, react and next domains
const UNKNOWN_ATTRIBUTE = biomePort("lint/suspicious/noUnknownAttribute"); // not recommended, react domain
const THIS_IN_METHODS = biomePort("lint/nursery/useThisInClassMethods"); // nursery

/** Biome's config read at a repository holding `files`, its notes, and a verdict per port. */
async function read(files: Readonly<Record<string, string>>) {
  const root = await repository(files);
  const notes: LinterNote[] = [];
  const config = biomeReader.read(root, notes);
  const verdict = (subject: string | Port): PortVerdict | undefined => config?.verdict(typeof subject === "string" ? port(subject) : subject, ruleCatalog);
  return { config, notes: notes.map(note => note.text), verdict, decision: (subject: string | Port) => verdict(subject)?.decision };
}

const json = (value: unknown): string => JSON.stringify(value, null, 2);

describe("biomeReader", () => {
  it("has no config without a Biome config file, and none for a port without a Biome name", async () => {
    expect((await read({ "package.json": "{}" })).config).toBeUndefined();
    const { config } = await read({ "biome.json": "{}" });
    expect(config?.verdict(biomePort("lint/suspicious/noDebugger"), ruleCatalog)).toBeDefined();
    expect(config?.verdict({ ...biomePort("x"), suppress: { ...biomePort("x").suppress, biome: [] } }, ruleCatalog)).toBeUndefined();
  });

  it("keeps Biome's recommended rules on and leaves the others off when the config names none", async () => {
    const { decision } = await read({ "biome.json": json({ $schema: "https://biomejs.dev/schemas/2.5.15/schema.json" }) });
    expect(decision(NO_DEBUGGER)).toEqual({ on: true, explicit: false, because: "biome.json: recommended" });
    expect(decision(CURLY)).toEqual({ on: false, explicit: false, because: "biome.json does not turn it on" });
  });

  it("turns a named rule on or off explicitly, whatever the preset", async () => {
    const { decision } = await read({
      "biome.json": json({ linter: { rules: { recommended: false, style: { useBlockStatements: "warn" }, suspicious: { noDebugger: { level: "off" } } } } }),
    });
    expect(decision(CURLY)).toEqual({ on: true, explicit: true, because: "biome.json: \"useBlockStatements\": \"warn\" in linter.rules.style" });
    expect(decision(NO_DEBUGGER)).toEqual({ on: false, explicit: true, because: "biome.json: \"noDebugger\": \"off\" in linter.rules.suspicious" });
    expect(decision(EQEQEQ)).toEqual({ on: false, explicit: false, because: "biome.json does not turn it on" });
  });

  it("reads groups: a severity turns the group on or off, and a group's recommended is a preset", async () => {
    const plain = await read({ "biome.json": json({ linter: { rules: { style: "error", suspicious: "off" } } }) });
    expect(plain.decision(CURLY)).toEqual({ on: true, explicit: true, because: "biome.json: \"style\": \"error\" in linter.rules" });
    expect(plain.decision(NO_DEBUGGER)).toEqual({ on: false, explicit: true, because: "biome.json: \"suspicious\": \"off\" in linter.rules" });
    const grouped = await read({ "biome.jsonc": "// Biome\n{ \"linter\": { \"rules\": { \"suspicious\": { \"recommended\": false, }, }, }, }\n" });
    expect(grouped.decision(NO_DEBUGGER)).toEqual({ on: false, explicit: false, because: "biome.jsonc does not turn it on" });
    const all = await read({ "biome.json": json({ linter: { rules: { preset: "all" } } }) });
    expect(all.decision(CURLY)).toEqual({ on: true, explicit: true, because: "biome.json: \"preset\": \"all\" in linter.rules" });
  });

  it("turns nursery rules on only by name or by the group", async () => {
    const all = await read({ "biome.json": json({ linter: { rules: { preset: "all" } } }) });
    expect(all.decision(THIS_IN_METHODS)).toEqual({ on: false, explicit: false, because: "biome.json does not turn it on" });
    const named = await read({ "biome.json": json({ linter: { rules: { nursery: { useThisInClassMethods: "on" } } } }) });
    expect(named.decision(THIS_IN_METHODS)).toEqual({ on: true, explicit: true, because: "biome.json: \"useThisInClassMethods\": \"on\" in linter.rules.nursery" });
  });

  it("reads domains, and leaves a recommended domain rule to the pack when only package.json could turn it on", async () => {
    const unset = await read({ "biome.json": "{}" });
    expect(unset.decision(HOOKS)).toEqual({ on: "unknown", because: "biome.json: Biome turns the recommended \"react\" rules on when package.json depends on the domain's packages" });
    expect(unset.decision(UNKNOWN_ATTRIBUTE)).toEqual({ on: false, explicit: false, because: "biome.json does not turn it on" });
    const recommended = await read({ "biome.json": json({ linter: { domains: { react: "recommended", next: "none" } } }) });
    expect(recommended.decision(HOOKS)).toEqual({ on: true, explicit: false, because: "biome.json: \"react\": \"recommended\" in linter.domains" });
    expect(recommended.decision(UNKNOWN_ATTRIBUTE)).toEqual({ on: false, explicit: false, because: "biome.json does not turn it on" });
    const all = await read({ "biome.json": json({ linter: { domains: { react: "all" } } }) });
    expect(all.decision(UNKNOWN_ATTRIBUTE)).toEqual({ on: true, explicit: true, because: "biome.json: \"react\": \"all\" in linter.domains" });
    const none = await read({ "biome.json": json({ linter: { domains: { react: "none", next: "none" } } }) });
    expect(none.decision(HOOKS)).toEqual({ on: false, explicit: true, because: "biome.json: \"react\": \"none\" in linter.domains" });
    // A rule named on stays on in a domain set to none.
    const named = await read({ "biome.json": json({ linter: { domains: { react: "none" }, rules: { correctness: { useHookAtTopLevel: "error" } } } }) });
    expect(named.decision(HOOKS)).toEqual({ on: true, explicit: true, because: "biome.json: \"useHookAtTopLevel\": \"error\" in linter.rules.correctness" });
  });

  it("leaves every rule off when the linter is off, below what another linter turns on", async () => {
    const { verdict } = await read({ "biome.json": json({ linter: { enabled: false, rules: { style: { useBlockStatements: "error" } } } }) });
    const off = verdict(CURLY)!;
    expect(off.decision).toEqual({ on: false, explicit: false, because: "biome.json: \"enabled\": false in linter" });
    const eslintOn: PortVerdict = { decision: { on: true, explicit: true, because: ".eslintrc.json: \"curly\": \"error\"" } };
    expect(settleVerdicts({ ...port(CURLY), startsOn: false }, [off, eslintOn], []).map(setting => setting.on)).toEqual([true]);
  });

  it("sets a rule per whole folder an override is for, and notes an override for other files", async () => {
    const { verdict, notes } = await read({
      "biome.json": json({
        linter: { rules: { suspicious: { noDebugger: "error" } } },
        overrides: [
          { includes: ["scripts/**"], linter: { rules: { suspicious: { noDebugger: "off" } } } },
          { includes: ["scripts/strict/**"], linter: { rules: { suspicious: { noDebugger: "on" } } } },
          { includes: ["**/*.test.ts"], linter: { rules: { suspicious: { noDebugger: "off" } } } },
          { includes: ["vendor/**"], formatter: { enabled: false } },
        ],
      }),
    });
    expect(verdict(NO_DEBUGGER)).toEqual({
      decision: { on: true, explicit: true, because: "biome.json: \"noDebugger\": \"error\" in linter.rules.suspicious" },
      folders: [{ folder: "scripts", on: false, because: "biome.json: \"noDebugger\": \"off\" in overrides[0].linter.rules.suspicious" }],
    });
    expect(notes).toEqual(["biome.json: overrides[2] is not read: its includes [\"**/*.test.ts\"] are not whole folders (<folder>/**)."]);
    const disabled = await read({ "biome.json": json({ overrides: [{ includes: ["legacy/**"], linter: { enabled: false } }] }) });
    expect(disabled.verdict(NO_DEBUGGER)?.folders).toEqual([{ folder: "legacy", on: false, because: "biome.json: \"enabled\": false in overrides[0].linter" }]);
  });

  it("merges a config it extends in the repository under its own, and the extended overrides first", async () => {
    const { decision, verdict } = await read({
      "biome.json": json({ extends: ["./configs/base.jsonc"], linter: { rules: { suspicious: { noDoubleEquals: "off" } } } }),
      "configs/base.jsonc": json({
        linter: { rules: { recommended: false, suspicious: { noDebugger: { level: "warn", options: {} } }, style: "on" } },
        overrides: [{ includes: ["src/**"], linter: { rules: { style: "off" } } }],
      }),
    });
    expect(decision(NO_DEBUGGER)).toEqual({ on: true, explicit: true, because: "configs/base.jsonc: \"noDebugger\": \"warn\" in linter.rules.suspicious" });
    expect(decision(EQEQEQ)).toEqual({ on: false, explicit: true, because: "biome.json: \"noDoubleEquals\": \"off\" in linter.rules.suspicious" });
    expect(verdict(CURLY)?.folders).toEqual([{ folder: "src", on: false, because: "configs/base.jsonc: \"style\": \"off\" in overrides[0].linter.rules" }]);
  });

  it("leaves the rules a package it extends may set to the pack, deciding only those the config names", async () => {
    const { decision, notes } = await read({
      "biome.json": json({ extends: ["ultracite/biome/core"], linter: { rules: { suspicious: { noDebugger: "off" }, style: { useBlockStatements: "error" } } } }),
    });
    expect(decision(EQEQEQ)).toEqual({ on: "unknown", because: "biome.json extends \"ultracite/biome/core\", which Rerail does not read" });
    expect(decision(NO_DEBUGGER)).toEqual({ on: false, explicit: true, because: "biome.json: \"noDebugger\": \"off\" in linter.rules.suspicious" });
    expect(decision(CURLY)).toEqual({ on: true, explicit: true, because: "biome.json: \"useBlockStatements\": \"error\" in linter.rules.style" });
    expect(notes).toEqual(["biome.json: extends \"ultracite/biome/core\" is not read: it is a package or a file outside the repository; the rules it may set keep the pack's values."]);
  });

  it("leaves every rule to the pack when the root config is not Biome's root", async () => {
    const { decision, notes } = await read({ "biome.json": json({ extends: "//", linter: { rules: { suspicious: { noDebugger: "off" } } } }) });
    expect(decision(NO_DEBUGGER)).toEqual({ on: "unknown", because: "biome.json is not Biome's root config" });
    expect(notes).toEqual(["biome.json is not read: it is not a root config (\"root\": false, or \"extends\": \"//\"), so Biome takes a config outside the repository."]);
  });

  it.each([
    { files: ["biome.json", "biome.jsonc", ".biome.json", ".biome.jsonc"], takes: "biome.json" },
    { files: ["biome.jsonc", ".biome.json", ".biome.jsonc"], takes: "biome.jsonc" },
    { files: [".biome.json", ".biome.jsonc"], takes: ".biome.json" },
    { files: [".biome.jsonc"], takes: ".biome.jsonc" },
  ])("takes $takes first of $files", async ({ files, takes }) => {
    const config = (file: string): string => json({ linter: { rules: { style: { useBlockStatements: file === takes ? "error" : "off" } } } });
    const { decision } = await read(Object.fromEntries(files.map(file => [file, config(file)])));
    expect(decision(CURLY)).toEqual({ on: true, explicit: true, because: `${takes}: "useBlockStatements": "error" in linter.rules.style` });
  });

  it("reads a .jsonc config as JSONC and a .json one only as plain JSON", async () => {
    const commentedJsonc = "// Biome\n{ \"linter\": { \"rules\": { \"style\": \"error\", }, }, }\n";
    expect((await read({ ".biome.jsonc": commentedJsonc })).decision(CURLY)).toEqual({ on: true, explicit: true, because: ".biome.jsonc: \"style\": \"error\" in linter.rules" });
    for (const file of ["biome.json", ".biome.json"]) {
      const { config, notes } = await read({ [file]: "// comment\n{}\n" });
      expect(config).toBeUndefined();
      expect(notes).toEqual([`${file} is not read: it is not plain JSON (Biome reads a .json config without comments or trailing commas).`]);
    }
    expect((await read({ "biome.json": "\uFEFF{ \"linter\": { \"rules\": { \"style\": \"error\" } } }" })).decision(CURLY)).toMatchObject({ on: true });
  });

  it.each([
    { config: [], why: "it is not an object" },
    { config: { linter: [] }, why: "linter is not an object" },
    { config: { linter: { enabled: "yes" } }, why: "linter.enabled is not true or false" },
    { config: { linter: { rules: { suspicious: { noSuchRule: "off" } } } }, why: "linter.rules.suspicious has no rule \"noSuchRule\" in Biome 2.5.15" },
    { config: { linter: { rules: { typos: "off" } } }, why: "linter.rules has no group \"typos\" in Biome 2.5.15" },
    { config: { linter: { rules: { style: "loud" } } }, why: "linter.rules.style is not one of \"off\", \"on\", \"info\", \"warn\", \"error\"" },
    { config: { linter: { rules: { style: 1 } } }, why: "linter.rules.style is not an object" },
    { config: { linter: { rules: { preset: "most" } } }, why: "linter.rules.preset is not one of \"recommended\", \"all\", \"none\"" },
    { config: { linter: { rules: { recommended: "yes" } } }, why: "linter.rules.recommended is not true or false" },
    { config: { linter: { rules: { suspicious: { recommended: 1 } } } }, why: "linter.rules.suspicious.recommended is not true or false" },
    { config: { linter: { rules: { suspicious: { noDebugger: { level: "off", severity: 1 } } } } }, why: "linter.rules.suspicious.noDebugger has an unknown key \"severity\"" },
    { config: { linter: { rules: { suspicious: { noDebugger: { level: "loud" } } } } }, why: "linter.rules.suspicious.noDebugger.level is not one of \"off\", \"on\", \"info\", \"warn\", \"error\"" },
    { config: { linter: { rules: { suspicious: { noDebugger: true } } } }, why: "linter.rules.suspicious.noDebugger is not an object" },
    { config: { linter: { domains: { angular: "all" } } }, why: "linter.domains has no domain \"angular\" in Biome 2.5.15" },
    { config: { linter: { domains: { react: "some" } } }, why: "linter.domains.react is not one of \"all\", \"none\", \"recommended\"" },
    { config: { root: "yes" }, why: "root is not true or false" },
    { config: { extends: "./base.json" }, why: "extends is not \"//\" or a list of paths" },
    { config: { extends: [1] }, why: "extends is not \"//\" or a list of paths" },
    { config: { overrides: {} }, why: "overrides is not a list" },
    { config: { overrides: ["scripts/**"] }, why: "overrides[0] is not an object" },
    { config: { overrides: [{ includes: "scripts/**", linter: {} }] }, why: "overrides[0].includes is not a list of globs" },
    { config: { overrides: [{ includes: [1], linter: {} }] }, why: "overrides[0].includes is not a list of globs" },
  ])("refuses a config whose $why", async ({ config, why }) => {
    const { config: read_, notes } = await read({ "biome.json": json(config) });
    expect(read_).toBeUndefined();
    expect(notes).toEqual([`biome.json is not read: ${why}.`]);
  });

  it("refuses a .jsonc config that is not JSONC, with the parser's reason", async () => {
    const { config, notes } = await read({ "biome.jsonc": "{ /* open" });
    expect(config).toBeUndefined();
    expect(notes).toEqual(["biome.jsonc is not read: a /* comment is not closed."]);
  });

  it("reads a root config, and a config that says it is root over its extends \"//\"", async () => {
    const notRoot = await read({ "biome.json": json({ root: false }) });
    expect(notRoot.decision(NO_DEBUGGER)).toEqual({ on: "unknown", because: "biome.json is not Biome's root config" });
    expect(notRoot.verdict({ ...biomePort("x"), suppress: { ...biomePort("x").suppress, biome: [] } })).toBeUndefined();
    const root = await read({ "biome.json": json({ root: true, extends: "//", linter: { rules: { suspicious: { noDebugger: "off" } } } }) });
    expect(root.decision(NO_DEBUGGER)).toEqual({ on: false, explicit: true, because: "biome.json: \"noDebugger\": \"off\" in linter.rules.suspicious" });
    expect(root.notes).toEqual([]);
  });

  it("notes linter.includes, applying the rules to every file", async () => {
    const { decision, notes } = await read({ "biome.json": json({ linter: { includes: ["src/**"], rules: { style: "error" } } }) });
    expect(decision(CURLY)).toMatchObject({ on: true });
    expect(notes).toEqual(["biome.json: linter.includes is not read; the rules Rerail sets apply to every file."]);
  });

  it("notes an override with a linter section but no includes", async () => {
    const { verdict, notes } = await read({ "biome.json": json({ overrides: [{ linter: { rules: { style: "error" } } }] }) });
    expect(verdict(CURLY)).toEqual({ decision: { on: false, explicit: false, because: "biome.json does not turn it on" } });
    expect(notes).toEqual(["biome.json: overrides[0] is not read: its includes [] are not whole folders (<folder>/**)."]);
  });

  it("resolves extends: a relative path or a bare file in the repository is read, as normalised", async () => {
    const base = json({ linter: { rules: { style: { useBlockStatements: "error" } } } });
    for (const entry of ["./configs/base.json", "configs/base.json", "./configs/../configs/base.json"]) {
      const { decision, notes } = await read({ "biome.json": json({ extends: [entry] }), "configs/base.json": base });
      expect(decision(CURLY)).toEqual({ on: true, explicit: true, because: "configs/base.json: \"useBlockStatements\": \"error\" in linter.rules.style" });
      expect(notes).toEqual([]);
    }
    // An extended config without a linter section leaves Biome's defaults.
    const noLinter = await read({ "biome.json": json({ extends: ["base.json"] }), "base.json": json({ formatter: { enabled: false } }) });
    expect(noLinter.decision(NO_DEBUGGER)).toEqual({ on: true, explicit: false, because: "biome.json: recommended" });
  });

  it.each([
    { entry: "../outside.json", note: "biome.json: extends \"../outside.json\" is not read: it is a package or a file outside the repository; the rules it may set keep the pack's values." },
    { entry: "/etc/biome.json", note: "biome.json: extends \"/etc/biome.json\" is not read: it is a package or a file outside the repository; the rules it may set keep the pack's values." },
    { entry: "shared/missing.json", note: "biome.json: extends \"shared/missing.json\" is not read: it is a package or a file outside the repository; the rules it may set keep the pack's values." },
    { entry: "./missing.json", note: "missing.json is not read: it is not in the repository." },
    { entry: "./configs", note: "configs is not read: it is not a regular file of at most 1 MiB in the repository." },
    { entry: "./bad.json", note: "bad.json is not read: it is not plain JSON (Biome reads a .json config without comments or trailing commas)." },
  ])("leaves the rules to the pack when it cannot read extends $entry", async ({ entry, note }) => {
    const { decision, notes } = await read({ "biome.json": json({ extends: [entry] }), "configs/base.json": "{}", "bad.json": "{ /* */ }" });
    expect(decision(NO_DEBUGGER)).toEqual({ on: "unknown", because: `biome.json extends ${JSON.stringify(entry)}, which Rerail does not read` });
    expect(notes).toEqual([note]);
  });

  it("leaves to the pack what an unread extended config may set, unless the config sets it after", async () => {
    const extendsPackage = { extends: ["@acme/biome"] };
    const unread = "biome.json extends \"@acme/biome\", which Rerail does not read";
    const bare = await read({ "biome.json": json(extendsPackage) });
    expect(bare.decision(HOOKS)).toEqual({ on: "unknown", because: unread });
    // A domain the config sets to none may be undone by the unread config naming the rule on.
    const none = await read({ "biome.json": json({ ...extendsPackage, linter: { domains: { react: "none", next: "none" } } }) });
    expect(none.decision(HOOKS)).toEqual({ on: "unknown", because: unread });
    const reactNone = await read({ "biome.json": json({ ...extendsPackage, linter: { domains: { react: "none" }, rules: { correctness: { useHookAtTopLevel: "off" } } } }) });
    expect(reactNone.decision(HOOKS)).toEqual({ on: false, explicit: true, because: "biome.json: \"useHookAtTopLevel\": \"off\" in linter.rules.correctness" });
    const allDomains = await read({ "biome.json": json({ ...extendsPackage, linter: { domains: { react: "all", next: "all" } } }) });
    expect(allDomains.decision(UNKNOWN_ATTRIBUTE)).toEqual({ on: "unknown", because: unread });
    // A group the config names after the unread config keeps the unread preset and entries it does not set.
    const group = await read({ "biome.json": json({ ...extendsPackage, linter: { rules: { suspicious: { recommended: true } } } }) });
    expect(group.decision(NO_DEBUGGER)).toEqual({ on: "unknown", because: unread });
    const preset = await read({ "biome.json": json({ ...extendsPackage, linter: { rules: { preset: "none", suspicious: { preset: "all" } } } }) });
    expect(preset.decision(NO_DEBUGGER)).toEqual({ on: "unknown", because: unread });
    expect(preset.decision(CURLY)).toEqual({ on: "unknown", because: unread });
  });

  it("merges a group set to a severity in an extended config into the group object over it", async () => {
    const { decision } = await read({
      "biome.json": json({ extends: ["./base.json"], linter: { rules: { suspicious: { recommended: false }, style: "off" } } }),
      "base.json": json({ linter: { rules: { suspicious: "error", style: { useBlockStatements: "error" } } } }),
    });
    expect(decision(NO_DEBUGGER)).toEqual({ on: true, explicit: true, because: "base.json: \"suspicious\": \"error\" in linter.rules" });
    expect(decision(CURLY)).toEqual({ on: false, explicit: true, because: "biome.json: \"style\": \"off\" in linter.rules" });
  });

  it("reads a group's own recommended over the rules' preset", async () => {
    const { decision } = await read({ "biome.json": json({ linter: { rules: { recommended: false, suspicious: { recommended: true } } } }) });
    expect(decision(NO_DEBUGGER)).toEqual({ on: true, explicit: false, because: "biome.json: \"recommended\": true in linter.rules.suspicious" });
    expect(decision(HOOKS)).toEqual({ on: false, explicit: false, because: "biome.json does not turn it on" });
  });

  it("does not turn a domain rule on by its group's severity, nor by package.json once rules set no preset", async () => {
    const plain = await read({ "biome.json": json({ linter: { rules: { correctness: "error", suspicious: "error" } } }) });
    expect(plain.decision(HOOKS)).toEqual({ on: false, explicit: false, because: "biome.json does not turn it on" });
    expect(plain.decision(UNKNOWN_ATTRIBUTE)).toEqual({ on: false, explicit: false, because: "biome.json does not turn it on" });
    const recommended = await read({ "biome.json": json({ linter: { rules: { preset: "recommended" } } }) });
    expect(recommended.decision(HOOKS)).toEqual({ on: "unknown", because: "biome.json: Biome turns the recommended \"react\" rules on when package.json depends on the domain's packages" });
  });

  it("decides a port of a name Biome does not have as unknown", async () => {
    const { decision } = await read({ "biome.json": "{}" });
    expect(decision(biomePort("lint/suspicious/noSuchRule"))).toEqual({ on: "unknown", because: "lint/suspicious/noSuchRule is not a Biome 2.5.15 lint rule" });
  });

  it.each([
    { names: ["lint/style/useBlockStatements", "lint/suspicious/noDebugger"], expected: { on: true, explicit: true, because: "biome.json: \"useBlockStatements\": \"error\" in linter.rules.style" } },
    { names: ["lint/suspicious/noSuchRule", "lint/suspicious/noDoubleEquals"], expected: { on: true, explicit: false, because: "biome.json: recommended" } },
    { names: ["lint/complexity/noVoid", "lint/suspicious/noSuchRule"], expected: { on: "unknown", because: "lint/suspicious/noSuchRule is not a Biome 2.5.15 lint rule" } },
    { names: ["lint/complexity/noVoid", "lint/suspicious/noDebugger"], expected: { on: false, explicit: true, because: "biome.json: \"noDebugger\": \"off\" in linter.rules.suspicious" } },
    { names: ["lint/complexity/noVoid", "lint/style/noNegationElse"], expected: { on: false, explicit: false, because: "biome.json does not turn it on" } },
  ])("decides a port of several Biome rules $names: on, else unknown, else explicitly off", async ({ names, expected }) => {
    const { decision } = await read({ "biome.json": json({ linter: { rules: { suspicious: { noDebugger: "off" }, style: { useBlockStatements: "error" } } } }) });
    expect(decision({ ...biomePort(names[0]!), suppress: { ...biomePort(names[0]!).suppress, biome: names } })).toEqual(expected);
  });

  it("notes rule options it does not map, keeping the rule on", async () => {
    const { decision } = await read({ "biome.json": json({ linter: { rules: { style: { useBlockStatements: { level: "error", options: { strict: true } } } } } }) });
    expect(decision(CURLY)).toEqual({
      on: true,
      explicit: true,
      because: "biome.json: \"useBlockStatements\": \"error\" in linter.rules.style",
      config: { ok: false, reason: "Biome's rule options are not read" },
    });
  });
});
