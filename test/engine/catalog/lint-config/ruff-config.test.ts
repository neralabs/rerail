import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { NOT_A_FILE, type LinterNote, type Port, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { readRuffSelection, ruffReader, ruffSelects } from "../../../../src/engine/catalog/lint-config/ruff-config.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

/** A repository holding `files`; a name ending in `/` is a folder. */
async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-ruff-config-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) {
    if (name.endsWith("/")) mkdirSync(join(root, name), { recursive: true });
    else {
      mkdirSync(dirname(join(root, name)), { recursive: true });
      writeFileSync(join(root, name), text);
    }
  }
  return root;
}

// The Ruff reader does not look into the catalog; an empty one stands in.
let ruleCatalog: RuleCatalog;
beforeAll(() => {
  ruleCatalog = readRuleCatalog([]);
});

const PORT = "ruff-port";

/** A port honouring noqa codes `codes`. */
const port = (codes: readonly string[]): Port => ({
  ruleId: PORT,
  suppress: { eslint: [], noqa: codes, nosec: [], oxlint: [], biome: [], phpstan: [], psalm: [], phpcs: [], clippy: [], sqlfluff: [] },
  declared: {},
  startsOn: true,
});

/** The Ruff config at `root`'s verdict on a port of `codes` (undefined without a config), with the notes reading it gave. */
function ruff(root: string, ...codes: string[]): { verdict: PortVerdict | undefined; notes: string[]; read: boolean } {
  const notes: LinterNote[] = [];
  const config = ruffReader.read(root, notes);
  return { verdict: config?.verdict(port(codes), ruleCatalog), notes: notes.map(note => note.text), read: config !== undefined };
}

const decision = async (files: Readonly<Record<string, string>>, code: string) => ruff(await repository(files), code).verdict?.decision;

// Ruff 0.16.9: E722 is stable, suspicious, in the default selection; ANN101 is removed; AIR003 is a preview rule in
// preview's default selection.
const DEFAULT_ON = { on: true, explicit: false, because: "ruff.toml: Ruff 0.16.9's default rule selection" };

describe("ruffReader: the config file Ruff uses", () => {
  it("reads no config where no file holds Ruff's, skipping a pyproject.toml that does not parse and never names Ruff", async () => {
    expect(ruff(await repository({}), "E722")).toEqual({ verdict: undefined, notes: [], read: false });
    expect(ruff(await repository({ "pyproject.toml": "[project\n" }), "E722")).toEqual({ verdict: undefined, notes: [], read: false });
  });

  it("reads none, and says so, where the config Ruff would use is not a file", async () => {
    const root = await repository({ "ruff.toml/": "", "pyproject.toml": "[tool.ruff.lint]\nignore = [\"E722\"]\n" });
    expect(ruff(root, "E722")).toEqual({ verdict: undefined, notes: [`ruff.toml is not read: ${NOT_A_FILE}`], read: false });
  });

  it("takes .ruff.toml over ruff.toml, and ruff.toml over pyproject.toml", async () => {
    const files = { "ruff.toml": "[lint]\nignore = [\"E722\"]\n", "pyproject.toml": "[tool.ruff.lint]\nextend-select = [\"E722\"]\n" };
    expect(await decision(files, "E722")).toEqual({ on: false, explicit: true, because: "ruff.toml: \"E722\" in lint.ignore" });
    expect(await decision({ ...files, ".ruff.toml": "[lint]\nextend-select = [\"E722\"]\n" }, "E722"))
      .toEqual({ on: true, explicit: true, because: ".ruff.toml: \"E722\" in lint.extend-select" });
  });

  it("says a code no Ruff rule has is not used, deciding nothing for it", async () => {
    const root = await repository({ "ruff.toml": "[lint]\nignore = [\"E722\"]\n" });
    const unknown = `${PORT}: "Z999" is not a rule of Ruff 0.16.9; Ruff's config is not used for it.`;
    expect(ruff(root, "Z999").verdict).toEqual({ folders: [], notes: [unknown] });
    expect(ruff(root, "Z999", "E722").verdict).toEqual({
      decision: { on: false, explicit: true, because: "ruff.toml: \"E722\" in lint.ignore" },
      folders: [],
      notes: [unknown],
    });
  });
});

describe("ruffReader: extend", () => {
  it("follows a chain through folders, a later config layered over the one it extends", async () => {
    expect(await decision({
      "ruff.toml": "extend = \"configs/a.toml\"\n",
      "configs/a.toml": "extend = \"./../base.toml\"\n",
      "base.toml": "[lint]\nignore = [\"E722\"]\n",
    }, "E722")).toEqual({ on: false, explicit: true, because: "base.toml: \"E722\" in lint.ignore" });
  });

  it.each([
    ["one without [tool.ruff]", "[project]\nname = \"acme\"\n"],
    ["one that does not parse and never names Ruff", "[project\n"],
  ])("ends the chain read whole at an extended pyproject.toml, %s, as Ruff reads no config from it", async (_, pyproject) => {
    const root = await repository({ "ruff.toml": "extend = \"pyproject.toml\"\n", "pyproject.toml": pyproject });
    expect(ruff(root, "E722")).toEqual({ verdict: { decision: DEFAULT_ON, folders: [], notes: [] }, notes: [], read: true });
  });

  const outside = "it is outside the repository";
  it.each([
    ["/etc/ruff.toml", outside],
    ["~/ruff.toml", outside],
    ["C:/ruff.toml", outside],
    ["configs\\base.toml", outside],
    ["../base.toml", outside],
    ["configs/../../base.toml", outside],
    ["configs/..", "it names the repository's root folder, not a config file"],
    [".", "it names the repository's root folder, not a config file"],
    ["missing.toml", "there is no such file"],
    ["ruff.toml", "its extend chain loops or runs too deep"],
  ])("leaves what the config extend %j names may decide unknown, as it is not read", async (extend, why) => {
    const root = await repository({ "ruff.toml": `extend = ${JSON.stringify(extend)}\n`, "configs/": "" });
    const named = `ruff.toml: the config it extends, ${JSON.stringify(extend)},`;
    expect(ruff(root, "E722")).toEqual({
      verdict: { decision: { on: "unknown", because: `${named} which Rerail does not read, may select it` }, folders: [], notes: [] },
      notes: [`${named} is not read (${why}), so the rules only that config ignores stay on.`],
      read: true,
    });
  });

  it("stops a loop at the config that closes it, keeping what the chain decides", async () => {
    const root = await repository({
      "ruff.toml": "extend = \"base.toml\"\n",
      "base.toml": "extend = \"ruff.toml\"\n[lint]\nignore = [\"E722\"]\n",
    });
    expect(ruff(root, "E722")).toEqual({
      verdict: { decision: { on: false, explicit: true, because: "base.toml: \"E722\" in lint.ignore" }, folders: [], notes: [] },
      notes: ["base.toml: the config it extends, \"ruff.toml\", is not read (its extend chain loops or runs too deep), so the rules only that config ignores stay on."],
      read: true,
    });
  });

  it("reads nine configs of a chain and no more", async () => {
    const files = Object.fromEntries(Array.from({ length: 9 }, (_, index) => [index === 0 ? "ruff.toml" : `c${index}.toml`, `extend = "c${index + 1}.toml"\n`]));
    const notes: LinterNote[] = [];
    const selection = readRuffSelection(await repository({ ...files, "c9.toml": "" }), notes);
    expect(selection?.layers.map(layer => layer.file)).toEqual(["c8.toml", "c7.toml", "c6.toml", "c5.toml", "c4.toml", "c3.toml", "c2.toml", "c1.toml", "ruff.toml"]);
    expect(selection?.unread).toBe("c8.toml: the config it extends, \"c9.toml\",");
    expect(notes.map(note => note.text)).toEqual(["c8.toml: the config it extends, \"c9.toml\", is not read (its extend chain loops or runs too deep), so the rules only that config ignores stay on."]);
  });

  it.each([
    ["does not parse", "[lint\n", ["base.toml is not read: line 1: a table header is not closed."]],
    ["is not a file", undefined, [`base.toml is not read: ${NOT_A_FILE}`]],
  ])("leaves what an extended config that %s may decide unknown, with only the reason it is not read", async (_, base, notes) => {
    const root = await repository({ "ruff.toml": "extend = \"base.toml\"\n", ...(base === undefined ? { "base.toml/": "" } : { "base.toml": base }) });
    expect(ruff(root, "E722")).toEqual({
      verdict: { decision: { on: "unknown", because: "ruff.toml: the config it extends, \"base.toml\", which Rerail does not read, may select it" }, folders: [], notes: [] },
      notes,
      read: true,
    });
  });
});

describe("ruffReader: selectors", () => {
  it("orders a category after ALL and before a code, as Ruff orders selectors by specificity", async () => {
    expect(await decision({ "ruff.toml": "[lint]\nignore = [\"ALL\"]\nextend-select = [\"suspicious\"]\n" }, "E722"))
      .toEqual({ on: true, explicit: true, because: "ruff.toml: \"suspicious\" in lint.extend-select" });
    expect(await decision({ "ruff.toml": "[lint]\nextend-select = [\"suspicious\"]\nignore = [\"E\"]\n" }, "E722"))
      .toEqual({ on: false, explicit: true, because: "ruff.toml: \"E\" in lint.ignore" });
    const root = await repository({ "ruff.toml": "[lint]\nignore = [\"suspicious\"]\nextend-select = [\"E722\"]\n" });
    expect(ruff(root, "E722").verdict).toEqual({
      decision: { on: true, explicit: true, because: "ruff.toml: \"E722\" in lint.extend-select" },
      folders: [],
      notes: [`${PORT}: ruff.toml: "suspicious" in lint.ignore is overridden by ruff.toml: "E722" in lint.extend-select, which Ruff applies after it; it stays on.`],
    });
  });

  it("selects a removed rule only by its code", async () => {
    expect(await decision({ "ruff.toml": "[lint]\nextend-select = [\"ANN101\"]\n" }, "ANN101"))
      .toEqual({ on: true, explicit: true, because: "ruff.toml: \"ANN101\" in lint.extend-select" });
    expect(await decision({ "ruff.toml": "[lint]\nextend-select = [\"ANN1\"]\n" }, "ANN101"))
      .toEqual({ on: false, explicit: false, because: "ruff.toml does not select it, nor does Ruff 0.16.9's default rule selection" });
  });

  it("does not take a flag that is not true or false, saying so, and runs Ruff's default without preview", async () => {
    const root = await repository({ "ruff.toml": "preview = 1\n" });
    expect(ruff(root, "AIR003")).toEqual({
      verdict: { decision: { on: false, explicit: false, because: "ruff.toml does not select it, nor does Ruff 0.16.9's default rule selection" }, folders: [], notes: [] },
      notes: ["ruff.toml: preview = 1 is not true or false; not read."],
      read: true,
    });
  });

  it.each([
    ["E7", "E711", true],
    ["E711", "E711", true],
    ["ALL", "B011", true],
    ["PL", "PLR0913", true],
    ["PLR", "PLC0414", false],
    ["E71", "E721", false],
    ["E", "ERA001", false],
    ["F", "FURB101", false],
    ["suspicious", "E722", true],
    ["pedantic", "E722", false],
    ["suspicious", "Z999", false],
  ])("takes selector %j to name %j: %j", (selector, code, selects) => {
    expect(ruffSelects(selector, code)).toBe(selects);
  });

  it("notes a list's selector Ruff does not take, reading the rest", async () => {
    const root = await repository({ "ruff.toml": "[lint]\nignore = [\"e722\", \"E722\"]\n" });
    expect(ruff(root, "E722")).toEqual({
      verdict: { decision: { on: false, explicit: true, because: "ruff.toml: \"E722\" in lint.ignore" }, folders: [], notes: [] },
      notes: ["ruff.toml: \"e722\" in lint.ignore is not a Ruff rule code or prefix."],
      read: true,
    });
  });

  it("has no verdict on a port without a noqa code", async () => {
    expect(ruff(await repository({ "ruff.toml": "" })).verdict).toBeUndefined();
  });
});

describe("ruffReader: layering", () => {
  it.each([
    ["[lint]\nselect = [\"F\"]\n", "ruff.toml: lint.select does not select it"],
    ["select = [\"F\"]\n", "ruff.toml: select does not select it"],
  ])("turns off what a select (%j) leaves out of Ruff's default", async (text, because) => {
    expect(await decision({ "ruff.toml": text }, "E722")).toEqual({ on: false, explicit: false, because });
  });

  it("carries the ignores of select = [] without extend-select into the next config's select", async () => {
    const carried = { "ruff.toml": "extend = \"base.toml\"\n[lint]\nselect = [\"E7\"]\n", "base.toml": "[lint]\nselect = []\nignore = [\"E722\"]\n" };
    expect(await decision(carried, "E722")).toEqual({ on: false, explicit: true, because: "base.toml: \"E722\" in lint.ignore" });
    expect(await decision({ ...carried, "base.toml": "[lint]\nselect = []\nextend-select = [\"F\"]\nignore = [\"E722\"]\n" }, "E722"))
      .toEqual({ on: true, explicit: true, because: "ruff.toml: \"E7\" in lint.select" });
  });
});

describe("ruffReader: preview", () => {
  const notSelected = (preview: string) => ({ on: false, explicit: false, because: `ruff.toml does not select it, nor does Ruff 0.16.9's default rule selection${preview}` });
  const previewDefault = { on: true, explicit: false, because: "ruff.toml: Ruff 0.16.9's default rule selection with preview" };

  // AIR003 is a preview rule in preview's default selection, AIR321 a preview rule outside it, AIR001 a stable rule
  // only preview's default selection enables.
  it.each([
    ["", "AIR003", notSelected("")],
    ["preview = true", "AIR003", previewDefault],
    ["preview = true", "AIR001", previewDefault],
    ["preview = false\n[lint]\npreview = true", "AIR003", previewDefault],
    ["preview = true\n[lint]\nexplicit-preview-rules = true", "AIR003", notSelected(" with preview")],
    ["preview = true\n[lint]\nexplicit-preview-rules = true", "AIR001", previewDefault],
    ["preview = true\n[lint]\nextend-select = [\"AIR3\"]", "AIR321", { on: true, explicit: true, because: "ruff.toml: \"AIR3\" in lint.extend-select" }],
    ["preview = true\n[lint]\nexplicit-preview-rules = true\nextend-select = [\"AIR3\"]", "AIR321", notSelected(" with preview")],
    ["preview = true\n[lint]\nexplicit-preview-rules = true\nextend-select = [\"AIR321\"]", "AIR321", { on: true, explicit: true, because: "ruff.toml: \"AIR321\" in lint.extend-select" }],
  ])("runs preview rules and preview's default selection only with preview: %j decides %s", async (text, code, expected) => {
    expect(await decision({ "ruff.toml": text }, code)).toEqual(expected);
  });

  it("notes a preview rule selected by its code without preview, which Ruff does not run", async () => {
    const root = await repository({ "ruff.toml": "[lint]\nselect = [\"E722\"]\nextend-select = [\"AIR321\"]\n" });
    expect(ruff(root, "AIR321").verdict).toEqual({
      decision: { on: false, explicit: false, because: "ruff.toml: lint.select does not select it" },
      folders: [],
      notes: [`${PORT}: ruff.toml: "AIR321" in lint.extend-select selects a preview rule, which Ruff runs only with preview = true; it is not on.`],
    });
    expect(ruff(root, "AIR003").verdict?.notes).toEqual([]);
  });

  it("leaves a rule preview decides unknown where a config Rerail does not read may set preview", async () => {
    const root = await repository({ "ruff.toml": "extend = \"/etc/ruff.toml\"\n[lint]\nselect = [\"AIR\"]\n" });
    expect(ruff(root, "AIR003").verdict?.decision).toEqual({
      on: "unknown",
      because: "ruff.toml: the config it extends, \"/etc/ruff.toml\", which Rerail does not read, may set preview, which decides it",
    });
    // A stable rule the select names runs whatever preview is.
    expect(ruff(root, "AIR001").verdict?.decision).toEqual({ on: true, explicit: true, because: "ruff.toml: \"AIR\" in lint.select" });
  });
});

describe("ruffReader: per-file ignores", () => {
  const ignoredIn = (folder: string, because: string) => ({ folder, on: false, because });

  it("turns the code off in a pattern's folder, noting a selector Ruff does not take and a negated pattern", async () => {
    const root = await repository({ "ruff.toml": "[lint.per-file-ignores]\n\"tests/**\" = [\"e722\", \"E722\"]\n\"!docs/**\" = [\"E7\"]\n\"src/**\" = [\"F\"]\n" });
    expect(ruff(root, "E722")).toEqual({
      verdict: {
        decision: DEFAULT_ON,
        folders: [ignoredIn("tests", "ruff.toml: \"E722\" in lint.per-file-ignores.\"tests/**\"")],
        notes: [`${PORT}: ruff.toml: "E7" in lint.per-file-ignores."!docs/**" ignores it only in the files "!docs/**" matches; it stays on.`],
      },
      notes: ["ruff.toml: \"e722\" in lint.per-file-ignores is not a Ruff rule code or prefix."],
      read: true,
    });
  });

  it("adds every config's extend-per-file-ignores to the nearest per-file-ignores", async () => {
    const root = await repository({
      "ruff.toml": "extend = \"base.toml\"\n[lint.extend-per-file-ignores]\n\"tests/**\" = [\"E722\"]\n",
      "base.toml": "[lint.per-file-ignores]\n\"docs/**\" = [\"E722\"]\n[lint.extend-per-file-ignores]\n\"scripts/**\" = [\"E7\"]\n",
    });
    expect(ruff(root, "E722").verdict?.folders).toEqual([
      ignoredIn("docs", "base.toml: \"E722\" in lint.per-file-ignores.\"docs/**\""),
      ignoredIn("tests", "ruff.toml: \"E722\" in lint.extend-per-file-ignores.\"tests/**\""),
      ignoredIn("scripts", "base.toml: \"E7\" in lint.extend-per-file-ignores.\"scripts/**\""),
    ]);
  });

  it("does not read a nested config's per-file ignores, whose patterns are relative to its folder, saying so once", async () => {
    const root = await repository({
      "ruff.toml": "extend = \"configs/base.toml\"\n",
      "configs/base.toml": "[lint.per-file-ignores]\n\"tests/**\" = [\"E722\"]\n[lint.extend-per-file-ignores]\n\"docs/**\" = [\"E722\"]\n",
    });
    expect(ruff(root, "E722")).toEqual({
      verdict: { decision: DEFAULT_ON, folders: [], notes: [] },
      notes: ["configs/base.toml: its per-file ignores are not read: their patterns are relative to its folder."],
      read: true,
    });
  });
});
