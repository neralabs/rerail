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
  it("orders a category after ALL and before a code, as Ruff orders selectors by specificity, with preview", async () => {
    const preview = "preview = true\n";
    expect(await decision({ "ruff.toml": `${preview}[lint]\nignore = ["ALL"]\nextend-select = ["suspicious"]\n` }, "E722"))
      .toEqual({ on: true, explicit: true, because: "ruff.toml: \"suspicious\" in lint.extend-select" });
    expect(await decision({ "ruff.toml": `${preview}[lint]\nextend-select = ["suspicious"]\nignore = ["E"]\n` }, "E722"))
      .toEqual({ on: false, explicit: true, because: "ruff.toml: \"E\" in lint.ignore" });
    const root = await repository({ "ruff.toml": `${preview}[lint]\nignore = ["suspicious"]\nextend-select = ["E722"]\n` });
    expect(ruff(root, "E722").verdict).toEqual({
      decision: { on: true, explicit: true, because: "ruff.toml: \"E722\" in lint.extend-select" },
      folders: [],
      notes: [`${PORT}: ruff.toml: "suspicious" in lint.ignore is overridden by ruff.toml: "E722" in lint.extend-select, which Ruff applies after it; it stays on.`],
    });
  });

  // rule_redirects.rs at the pin: TCH is TC, TRY200 is B904, M is RUF100, R5 is RET5.
  it.each([
    ["select = [\"E\", \"F\", \"TCH\"]", "TC005", true, "\"TCH\" in lint.select (Ruff reads it as \"TC\")"],
    ["select = [\"TRY200\"]", "B904", true, "\"TRY200\" in lint.select (Ruff reads it as \"B904\")"],
    ["select = [\"M\"]", "RUF100", true, "\"M\" in lint.select (Ruff reads it as \"RUF100\")"],
    ["select = [\"ALL\"]\nignore = [\"R5\"]", "RET504", false, "\"R5\" in lint.ignore (Ruff reads it as \"RET5\")"],
    ["select = [\"TC\"]\nignore = [\"TCH005\"]", "TC005", false, "\"TCH005\" in lint.ignore (Ruff reads it as \"TC005\")"],
  ])("reads a selector Ruff remaps as its target: %j decides %s", async (lint, code, on, because) => {
    expect(await decision({ "ruff.toml": `[lint]\n${lint}\n` }, code)).toEqual({ on, explicit: true, because: `ruff.toml: ${because}` });
  });

  it("takes a rule name and a category only with preview, and then skips a selector it does not know, saying so", async () => {
    const root = await repository({ "ruff.toml": "preview = true\n[lint]\nselect = [\"E\", \"bare-except\", \"pytest-fixture-autouse\", \"XYZ\"]\nignore = [\"E7\"]\n[lint.per-file-ignores]\n\"tests/**\" = [\"XYZ\"]\n" });
    const byName = "ruff.toml: \"bare-except\" in lint.select (Ruff reads it as \"E722\")";
    expect(ruff(root, "E722")).toEqual({
      verdict: {
        decision: { on: true, explicit: true, because: byName },
        folders: [],
        notes: [`${PORT}: ruff.toml: "E7" in lint.ignore is overridden by ${byName}, which Ruff applies after it; it stays on.`],
      },
      notes: ["ruff.toml: \"XYZ\" in lint.select is not a Ruff rule code or prefix.", "ruff.toml: \"XYZ\" in lint.per-file-ignores.\"tests/**\" is not a Ruff rule code or prefix."],
      read: true,
    });
  });

  const refused = (why: string, first = "ruff.toml") =>
    `${first}: Ruff refuses this config (${why}), so Rerail reads none of it: the rules Rerail ports from Ruff keep the pack's values.`;

  it.each([
    ["[lint]\nselect = [\"E\", \"suspicious\"]", "ruff.toml: \"suspicious\" in lint.select selects by category or rule name, which Ruff takes only with preview"],
    ["[lint]\nselect = [\"bare-except\"]", "ruff.toml: \"bare-except\" in lint.select selects by category or rule name, which Ruff takes only with preview"],
    ["[lint]\nignore = [\"e722\"]", "ruff.toml: \"e722\" in lint.ignore is not a selector Ruff knows"],
    ["preview = true\n[lint]\nextend-select = [\"ANN101\"]", "ruff.toml: \"ANN101\" in lint.extend-select selects a removed rule"],
    ["[lint.per-file-ignores]\n\"tests/**\" = [\"E7XX\"]", "ruff.toml: \"E7XX\" in lint.per-file-ignores.\"tests/**\" is not a selector Ruff knows"],
    ["[lint.pydocstyle]\nconvention = \"sphinx\"", "ruff.toml: lint.pydocstyle.convention = \"sphinx\" is not a convention Ruff knows"],
  ])("reads nothing of a config Ruff refuses, saying why: %j", async (text, why) => {
    expect(ruff(await repository({ "ruff.toml": text }), "E722")).toEqual({ verdict: undefined, notes: [refused(why)], read: false });
  });

  it("reads nothing of a config Ruff may refuse, where a config Rerail does not read may leave preview off", async () => {
    const root = await repository({ "ruff.toml": "extend = \"/etc/ruff.toml\"\n[lint]\nselect = [\"suspicious\"]\n" });
    expect(ruff(root, "E722").notes).toEqual([
      "ruff.toml: the config it extends, \"/etc/ruff.toml\", is not read (it is outside the repository), so the rules only that config ignores stay on.",
      refused("ruff.toml: \"suspicious\" in lint.select selects by category or rule name, which Ruff takes only with preview"),
    ]);
  });

  it("selects a removed rule by no prefix, and lets a config ignore it", async () => {
    expect(await decision({ "ruff.toml": "[lint]\nextend-select = [\"ANN1\"]\n" }, "ANN101"))
      .toEqual({ on: false, explicit: false, because: "ruff.toml does not select it, nor does Ruff 0.16.9's default rule selection" });
    expect(await decision({ "ruff.toml": "[lint]\nignore = [\"ANN101\"]\n" }, "ANN101"))
      .toEqual({ on: false, explicit: true, because: "ruff.toml: \"ANN101\" in lint.ignore" });
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
    ["C", "C901", true],
    ["C", "C401", true],
    ["C", "CPY001", false],
    ["T", "T201", true],
    ["T", "TC001", false],
    ["suspicious", "E722", true],
    ["pedantic", "E722", false],
    ["suspicious", "Z999", false],
  ])("takes selector %j to name %j: %j", (selector, code, selects) => {
    expect(ruffSelects(selector, code)).toBe(selects);
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

  it("leaves a rule preview or a convention decides unknown where a config Rerail does not read may set them", async () => {
    const root = await repository({ "ruff.toml": "extend = \"/etc/ruff.toml\"\n[lint]\nselect = [\"AIR\", \"D\"]\n" });
    const unknown = {
      on: "unknown",
      because: "ruff.toml: the config it extends, \"/etc/ruff.toml\", which Rerail does not read, may set preview or a docstring convention, which decide it",
    };
    expect(ruff(root, "AIR003").verdict?.decision).toEqual(unknown);
    // google and pep257 turn D404 off; no convention leaves it on.
    expect(ruff(root, "D404").verdict?.decision).toEqual(unknown);
    // A stable rule the select names runs whatever preview is.
    expect(ruff(root, "AIR001").verdict?.decision).toEqual({ on: true, explicit: true, because: "ruff.toml: \"AIR\" in lint.select" });
  });
});

describe("ruffReader: docstring convention and incompatible rules", () => {
  const off = (because: string) => ({ on: false, explicit: false, because });
  it.each([
    ["[lint]\nselect = [\"D\"]\n[lint.pydocstyle]\nconvention = \"google\"", "D213", off("ruff.toml: lint.pydocstyle.convention = \"google\" turns it off")],
    ["select = [\"D\"]\n[pydocstyle]\nconvention = \"google\"", "D404", off("ruff.toml: pydocstyle.convention = \"google\" turns it off")],
    ["[lint]\nselect = [\"D\"]\nextend-select = [\"D404\"]\n[lint.pydocstyle]\nconvention = \"google\"", "D404", { on: true, explicit: true, because: "ruff.toml: \"D404\" in lint.extend-select" }],
    ["[lint]\nselect = [\"D\"]\n[lint.pydocstyle]\nconvention = \"google\"", "D212", { on: true, explicit: true, because: "ruff.toml: \"D\" in lint.select" }],
    ["[lint]\nselect = [\"D\"]", "D203", off("Ruff never runs it with D211, which is on (ruff.toml: \"D\" in lint.select)")],
    ["[lint]\nselect = [\"D\"]\nignore = [\"D211\"]", "D203", { on: true, explicit: true, because: "ruff.toml: \"D\" in lint.select" }],
  ])("turns off what the convention or a rule Ruff prefers turns off, but what a config selects by its code: %j decides %s", async (text, code, expected) => {
    expect(await decision({ "ruff.toml": text }, code)).toEqual(expected);
  });
});

describe("ruffReader: per-file ignores", () => {
  const ignoredIn = (folder: string, because: string) => ({ folder, on: false, explicit: true, because });

  it("turns the code off in the folder a pattern covers whole, `*` crossing folders, noting narrower patterns", async () => {
    const root = await repository({
      "ruff.toml": "[lint.per-file-ignores]\n\"tests/**\" = [\"E722\"]\n\"lib/*\" = [\"E7\"]\n\"./docs/**/*\" = [\"E722\"]\n\"src/../x/**\" = [\"E722\"]\n\"**/test_*.py\" = [\"E7\"]\n",
    });
    expect(ruff(root, "E722").verdict).toEqual({
      decision: DEFAULT_ON,
      folders: [
        ignoredIn("tests", "ruff.toml: \"E722\" in lint.per-file-ignores.\"tests/**\""),
        ignoredIn("lib", "ruff.toml: \"E7\" in lint.per-file-ignores.\"lib/*\""),
        ignoredIn("docs", "ruff.toml: \"E722\" in lint.per-file-ignores.\"./docs/**/*\""),
      ],
      notes: [
        `${PORT}: ruff.toml: "E722" in lint.per-file-ignores."src/../x/**" ignores it only in the files "src/../x/**" matches; it stays on.`,
        `${PORT}: ruff.toml: "E7" in lint.per-file-ignores."**/test_*.py" ignores it only in the files "**/test_*.py" matches; it stays on.`,
      ],
    });
  });

  it("turns the code off for every file where a pattern matches every file's name", async () => {
    expect(await decision({ "ruff.toml": "[lint.per-file-ignores]\n\"*\" = [\"E722\"]\n" }, "E722"))
      .toEqual({ on: false, explicit: true, because: "ruff.toml: \"E722\" in lint.per-file-ignores.\"*\"" });
  });

  describe("a pattern starting `!`, which ignores the code in every file it does not match", () => {
    const negated = (pattern: string) => `ruff.toml: "E722" in lint.per-file-ignores.${JSON.stringify(pattern)}`;

    it("turns the code off, back to the selection in the one folder the patterns leave out", async () => {
      expect(ruff(await repository({ "ruff.toml": "[lint.per-file-ignores]\n\"!tests/**\" = [\"E722\"]\n\"!tests/unit/**\" = [\"E7\"]\n" }), "E722").verdict).toEqual({
        decision: { on: false, explicit: true, because: `${negated("!tests/**")}; ruff.toml: "E7" in lint.per-file-ignores."!tests/unit/**" ignores it in every file outside tests/, tests/unit/` },
        folders: [{ folder: "tests/unit", on: true, explicit: false, because: `${negated("!tests/**")}; ruff.toml: "E7" in lint.per-file-ignores."!tests/unit/**" leaves it to ${DEFAULT_ON.because}` }],
        notes: [],
      });
    });

    it.each([
      ["patterns leave out folders apart", "\"!tests/**\" = [\"E722\"]\n\"!docs/**\" = [\"E722\"]\n"],
      ["a plain pattern covers the folder left out", "\"!tests/**\" = [\"E722\"]\n\"tests/**\" = [\"E722\"]\n"],
    ])("turns the code off for every file, and back in no folder, where %s", async (_, table) => {
      const verdict = ruff(await repository({ "ruff.toml": `[lint.per-file-ignores]\n${table}` }), "E722").verdict;
      expect(verdict?.decision).toMatchObject({ on: false, explicit: true });
      expect(verdict?.folders?.filter(folder => folder.on !== false)).toEqual([]);
    });

    it("leaves a code the selection turns off off, with no folder back on", async () => {
      expect(ruff(await repository({ "ruff.toml": "[lint]\nselect = [\"F\"]\n[lint.per-file-ignores]\n\"!tests/**\" = [\"E722\"]\n" }), "E722").verdict).toEqual({
        decision: { on: false, explicit: true, because: `${negated("!tests/**")} ignores it in every file outside tests/` },
        folders: [],
        notes: [],
      });
    });

    it("leaves the code unknown where a pattern leaves out narrower files, and alone where it leaves out every file", async () => {
      expect(await decision({ "ruff.toml": "[lint.per-file-ignores]\n\"!**/test_*.py\" = [\"E722\"]\n" }, "E722"))
        .toEqual({ on: "unknown", because: `${negated("!**/test_*.py")} ignores it in every file but some, which Rerail does not place in whole folders` });
      expect(await decision({ "ruff.toml": "[lint.per-file-ignores]\n\"!*\" = [\"E722\"]\n" }, "E722")).toEqual(DEFAULT_ON);
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

  it("reads an extended config's patterns from its own folder, but a pattern of file names everywhere", async () => {
    const root = await repository({
      "ruff.toml": "extend = \"configs/base.toml\"\n",
      "configs/base.toml": "[lint.per-file-ignores]\n\"tests/**\" = [\"E722\"]\n\"./*\" = [\"E722\"]\n[lint.extend-per-file-ignores]\n\"**\" = [\"E7\"]\n",
    });
    expect(ruff(root, "E722").verdict).toEqual({
      decision: { on: false, explicit: true, because: "configs/base.toml: \"E7\" in lint.extend-per-file-ignores.\"**\"" },
      folders: [
        ignoredIn("configs/tests", "configs/base.toml: \"E722\" in lint.per-file-ignores.\"tests/**\""),
        ignoredIn("configs", "configs/base.toml: \"E722\" in lint.per-file-ignores.\"./*\""),
      ],
      notes: [],
    });
  });
});

describe("ruffReader: a Ruff config below the root", () => {
  it("leaves every rule unknown in a folder holding one, which Ruff uses there, and drops the root's decisions in it", async () => {
    const root = await repository({
      "ruff.toml": "[lint.per-file-ignores]\n\"sub/tests/**\" = [\"E722\"]\n\"tests/**\" = [\"E722\"]\n",
      "sub/pyproject.toml": "[tool.ruff]\n",
      "sub/ruff.toml": "",
      "lib/pyproject.toml": "[project]\nname = \"lib\"\n",
      "docs/pyproject.toml": "[tool.ruff.lint]\nselect = [\"F\"]\n",
      "notes.txt": "",
    });
    const unread = (file: string, folder: string) => `${file} is not read: Ruff uses it, not ruff.toml, for the files in ${folder}/, so the rules Rerail ports from Ruff keep the pack's values there.`;
    expect(ruff(root, "E722")).toEqual({
      verdict: {
        decision: DEFAULT_ON,
        folders: [
          { folder: "tests", on: false, explicit: true, because: "ruff.toml: \"E722\" in lint.per-file-ignores.\"tests/**\"" },
          { folder: "docs", on: "unknown", because: "docs/pyproject.toml, which Rerail does not read, is Ruff's config for docs/" },
          { folder: "sub", on: "unknown", because: "sub/ruff.toml, which Rerail does not read, is Ruff's config for sub/" },
        ],
        notes: [],
      },
      notes: [unread("docs/pyproject.toml", "docs"), unread("sub/ruff.toml", "sub")],
      read: true,
    });
    // A code no Ruff rule has decides nothing, there either.
    expect(ruff(root, "Z999").verdict?.folders).toEqual([]);
  });
});
