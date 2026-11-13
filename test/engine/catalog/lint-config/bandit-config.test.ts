import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { banditReader } from "../../../../src/engine/catalog/lint-config/bandit-config.js";
import { NOT_A_FILE, portsOf, settleVerdicts, type LinterNote, type Port, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const ASSERT = "bandit-b101-assert-used";
const MARSHAL = "bandit-b302-marshal";
const MD5 = "bandit-b303-md5";
const NO_SHELL = "bandit-b606-start-process-with-no-shell";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-bandit-config-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
  return root;
}

let ruleCatalog: RuleCatalog;
let ports: Map<string, Port>;
beforeAll(() => {
  ruleCatalog = readRuleCatalog([fileURLToPath(new URL("../../../../packs/python", import.meta.url))]);
  ports = new Map(portsOf(ruleCatalog, ["python"]).map(port => [port.ruleId, port]));
});

/** The verdicts on `ruleIds` of the Bandit config at `root`, with the notes reading it gave. */
function read(root: string, ...ruleIds: string[]): { verdicts: (PortVerdict | undefined)[]; notes: string[] } | undefined {
  const notes: LinterNote[] = [];
  const config = banditReader.read(root, notes);
  if (config === undefined) return undefined;
  return { verdicts: ruleIds.map(ruleId => config.verdict(ports.get(ruleId)!, ruleCatalog)), notes: notes.map(note => note.text) };
}

const decisions = (root: string, ...ruleIds: string[]) => read(root, ...ruleIds)!.verdicts.map(verdict => verdict?.decision);

describe("banditReader", () => {
  it("reads no config where the repository has none for Bandit", async () => {
    expect(read(await repository({}), ASSERT)).toBeUndefined();
    expect(read(await repository({ "pyproject.toml": "[tool.ruff]\nselect = [\"E\"]\n" }), ASSERT)).toBeUndefined();
    // A pyproject.toml Rerail cannot read is not refused when it names no Bandit config.
    expect(read(await repository({ "pyproject.toml": "[tool.ruff\n" }), ASSERT)).toBeUndefined();
  });

  it("reads no config from a .bandit without a [bandit] section, as Bandit does, and says so", async () => {
    const notes: LinterNote[] = [];
    expect(banditReader.read(await repository({ ".bandit": "[other]\nskips = B101\n" }), notes)).toBeUndefined();
    expect(notes).toEqual([{ linter: "bandit", text: ".bandit has no [bandit] section, so Bandit does not read it." }]);
  });

  it("keeps every test on where no tests are listed, turning off only the skipped ones", async () => {
    const root = await repository({ ".bandit": "[bandit]\n# Bandit's own comment\nskips: B101,B606\n" });
    expect(decisions(root, ASSERT, NO_SHELL, MD5)).toEqual([
      { on: false, explicit: true, because: ".bandit: \"B101\" in skips" },
      { on: false, explicit: true, because: ".bandit: \"B606\" in skips" },
      { on: true, explicit: false, because: ".bandit: no tests set, so Bandit runs every test" },
    ]);
  });

  it("runs only the listed tests", async () => {
    const root = await repository({ ".bandit": "[bandit]\ntests = B303,\n  B101\n" });
    expect(decisions(root, MD5, ASSERT, NO_SHELL)).toEqual([
      { on: true, explicit: true, because: ".bandit: \"B303\" in tests" },
      // configparser joins the continuation line with a newline, which Bandit keeps: "\nB101" names no test.
      { on: false, explicit: false, because: ".bandit tests does not list it" },
      { on: false, explicit: false, because: ".bandit tests does not list it" },
    ]);
    expect(read(root, ASSERT)!.notes).toEqual([".bandit: \"\\nB101\" in tests is not a Bandit test id such as \"B101\", so it names no test."]);
  });

  it("keeps the spaces Bandit keeps after commas, so a spaced id names no test", async () => {
    const root = await repository({ ".bandit": "[bandit]\nskips = B101, B606\n" });
    expect(decisions(root, NO_SHELL)).toEqual([{ on: true, explicit: false, because: ".bandit: no tests set, so Bandit runs every test" }]);
    expect(read(root, NO_SHELL)!.notes).toEqual([".bandit: \" B606\" in skips is not a Bandit test id such as \"B101\", so it names no test."]);
  });

  it("notes the targets .bandit gives, which do not change the tests Bandit runs", async () => {
    const root = await repository({ ".bandit": "[bandit]\ntargets = src\nskips = B101\n" });
    expect(read(root, ASSERT)).toEqual({
      verdicts: [{ decision: { on: false, explicit: true, because: ".bandit: \"B101\" in skips" }, folders: [] }],
      notes: [".bandit: targets \"src\" is not read: Rerail does not limit rules to the paths Bandit scans."],
    });
  });

  it("reads pyproject.toml [tool.bandit] when .bandit names no config file", async () => {
    const root = await repository({ "pyproject.toml": "[tool.bandit]\nskips = [\"B101\"]\n" });
    expect(decisions(root, ASSERT, MD5)).toEqual([
      { on: false, explicit: true, because: "pyproject.toml [tool.bandit]: \"B101\" in skips" },
      { on: true, explicit: false, because: "pyproject.toml [tool.bandit]: no tests set, so Bandit runs every test" },
    ]);
  });

  it("adds up the tests and skips of .bandit and the config file it names, as Bandit does", async () => {
    const root = await repository({
      ".bandit": "[bandit]\nconfigfile = ./bandit.toml\ntests = B101\n",
      "bandit.toml": "[tool.bandit]\ntests = [\"B303\"]\nskips = [\"B606\"]\n",
      // Not read: .bandit names another config file.
      "pyproject.toml": "[tool.bandit]\nskips = [\"B101\"]\n",
    });
    expect(decisions(root, ASSERT, MD5, MARSHAL, NO_SHELL)).toEqual([
      { on: true, explicit: true, because: ".bandit: \"B101\" in tests" },
      { on: true, explicit: true, because: "bandit.toml [tool.bandit]: \"B303\" in tests" },
      { on: false, explicit: false, because: ".bandit and bandit.toml [tool.bandit] tests does not list it" },
      { on: false, explicit: true, because: "bandit.toml [tool.bandit]: \"B606\" in skips" },
    ]);
  });

  it("names each id where it is first listed, and reads a named config file without [tool.bandit] as listing nothing", async () => {
    const listedTwice = await repository({ ".bandit": "[bandit]\nconfigfile = bandit.toml\ntests = B101\n", "bandit.toml": "[tool.bandit]\ntests = [\"B303\", \"B101\"]\n" });
    expect(decisions(listedTwice, ASSERT)).toEqual([{ on: true, explicit: true, because: ".bandit: \"B101\" in tests" }]);
    const empty = await repository({ ".bandit": "[bandit]\nconfigfile = empty.toml\nskips = B101\n", "empty.toml": "[tool.black]\nline-length = 100\n" });
    expect(read(empty, MD5)).toEqual({
      verdicts: [{ decision: { on: true, explicit: false, because: ".bandit and empty.toml [tool.bandit]: no tests set, so Bandit runs every test" }, folders: [] }],
      notes: [],
    });
  });

  it("reads B001 as every blacklist test, unless the same list names one", async () => {
    const skipped = await repository({ ".bandit": "[bandit]\nskips = B001\n" });
    expect(decisions(skipped, MD5, MARSHAL, ASSERT)).toEqual([
      { on: false, explicit: true, because: ".bandit: \"B001\" in skips (every blacklist test)" },
      { on: false, explicit: true, because: ".bandit: \"B001\" in skips (every blacklist test)" },
      { on: true, explicit: false, because: ".bandit: no tests set, so Bandit runs every test" },
    ]);
    const narrowed = await repository({ ".bandit": "[bandit]\ntests = B001,B303\n" });
    expect(decisions(narrowed, MD5, MARSHAL)).toEqual([
      { on: true, explicit: true, because: ".bandit: \"B303\" in tests" },
      { on: false, explicit: false, because: ".bandit tests does not list it" },
    ]);
  });

  it("turns every test off in the whole folders excluded, and notes other excludes", async () => {
    const root = await repository({
      ".bandit": "[bandit]\nexclude = ./tests,/docs/,*/migrations/*,../shared\n",
      "pyproject.toml": "[tool.bandit]\nexclude_dirs = [\"vendor/**\"]\n",
    });
    const result = read(root, MD5)!;
    expect(result.verdicts[0]!.folders).toEqual([
      { folder: "tests", on: false, because: ".bandit: \"./tests\" in exclude" },
      { folder: "docs", on: false, because: ".bandit: \"/docs/\" in exclude" },
      { folder: "vendor", on: false, because: "pyproject.toml [tool.bandit]: \"vendor/**\" in exclude_dirs" },
    ]);
    expect(result.notes).toEqual([
      ".bandit: \"*/migrations/*\" in exclude is not read: Rerail sets rules for whole folders only, so the tests Bandit skips there stay on.",
      ".bandit: \"../shared\" in exclude is not read: Rerail sets rules for whole folders only, so the tests Bandit skips there stay on.",
    ]);
    expect(settleVerdicts(ports.get(MD5)!, result.verdicts as PortVerdict[], [])).toEqual([
      { ruleId: MD5, folder: "tests", on: false, config: null, explicit: true, because: ".bandit: \"./tests\" in exclude" },
      { ruleId: MD5, folder: "docs", on: false, config: null, explicit: true, because: ".bandit: \"/docs/\" in exclude" },
      { ruleId: MD5, folder: "vendor", on: false, config: null, explicit: true, because: "pyproject.toml [tool.bandit]: \"vendor/**\" in exclude_dirs" },
    ]);
  });

  it("leaves the pack's values where a config Bandit takes is not read, keeping the skips it can read", async () => {
    const yaml = await repository({ "bandit.yaml": "skips: [B101]\n" });
    expect(read(yaml, ASSERT)).toEqual({
      verdicts: [{ decision: { on: "unknown", because: "bandit.yaml is not read" }, folders: [] }],
      notes: ["bandit.yaml is not read: Rerail reads Bandit's INI and TOML configs, not YAML. The rules Rerail ports from Bandit keep the pack's values."],
    });
    const profile = await repository({ ".bandit": "[bandit]\nprofile = strict\nskips = B101\n" });
    expect(decisions(profile, ASSERT, MD5)).toEqual([
      { on: false, explicit: true, because: ".bandit: \"B101\" in skips" },
      { on: "unknown", because: ".bandit selects profile \"strict\", which Rerail does not read" },
    ]);
  });

  it.each([
    ["../shared/bandit.toml", "is not read", "is not read: Rerail reads TOML configs in the repository only."],
    ["/etc/bandit.toml", "is not read", "is not read: Rerail reads TOML configs in the repository only."],
    ["bandit.yaml", "is not read", "is not read: Rerail reads TOML configs in the repository only."],
    ["missing.toml", "does not exist", "does not exist, so Bandit stops with an error."],
  ])("leaves the pack's values where .bandit names configfile %s, which Rerail does not read", async (named, because, note) => {
    const root = await repository({ ".bandit": `[bandit]\nconfigfile = ${named}\n`, "bandit.yaml": "skips: [B101]\n" });
    expect(read(root, MD5)).toEqual({
      verdicts: [{ decision: { on: "unknown", because: `.bandit: configfile ${JSON.stringify(named)} ${because}` }, folders: [] }],
      notes: [`.bandit: configfile ${JSON.stringify(named)} ${note}`],
    });
  });

  it("refuses a .bandit it cannot read as configparser does, leaving the pack's values", async () => {
    const interpolated = await repository({ ".bandit": "[bandit]\nskips = %(common)s\n" });
    expect(read(interpolated, ASSERT)).toEqual({
      verdicts: [{ decision: { on: "unknown", because: ".bandit is not read" }, folders: [] }],
      notes: [".bandit is not read: the value of skips has a \"%\", which configparser interpolates and Rerail does not."],
    });
    const duplicated = await repository({ ".bandit": "[bandit]\nskips = B101\nSKIPS = B606\n" });
    expect(read(duplicated, ASSERT)!.notes).toEqual([".bandit is not read: line 3: option skips in [bandit] is set twice."]);
    const folder = await repository({ ".bandit/x": "" });
    expect(read(folder, ASSERT)!.notes).toEqual([`.bandit is not read: ${NOT_A_FILE}`]);
  });

  it("refuses a pyproject.toml it cannot read when it may configure Bandit", async () => {
    const root = await repository({ "pyproject.toml": "[tool.bandit]\nskips = \"B101\"\n" });
    const result = read(root, ASSERT)!;
    expect(result.verdicts[0]!.decision).toEqual({ on: "unknown", because: "pyproject.toml is not read" });
    expect(result.notes).toHaveLength(1);
    expect(result.notes[0]).toMatch(/^pyproject\.toml is not read: /u);
    const folder = await repository({ "pyproject.toml/x": "" });
    expect(read(folder, ASSERT)).toEqual({
      verdicts: [{ decision: { on: "unknown", because: "pyproject.toml is not read" }, folders: [] }],
      notes: [`pyproject.toml is not read: ${NOT_A_FILE}`],
    });
  });

  it("leaves the pack's values where Bandit stops on a test both selected and skipped", async () => {
    const root = await repository({ ".bandit": "[bandit]\ntests = B101,B303\nskips = B101\n" });
    expect(decisions(root, ASSERT, MD5)).toEqual([
      { on: false, explicit: true, because: ".bandit: \"B101\" in skips" },
      { on: "unknown", because: "Bandit stops with an error: B101 in both tests and skips" },
    ]);
  });

  it("settles a port with several test ids on the weightiest decision: explicit before default, off before on", async () => {
    const md5 = ports.get(MD5)!;
    const decide = async (bandit: string, ...nosec: string[]) =>
      banditReader.read(await repository({ ".bandit": bandit }), [])!.verdict({ ...md5, suppress: { ...md5.suppress, nosec } }, ruleCatalog)!.decision;
    const listing = "[bandit]\ntests = B101,B303\nskips = B606\n";
    expect(await decide(listing, "B302", "B101", "B606")).toEqual({ on: false, explicit: true, because: ".bandit: \"B606\" in skips" });
    expect(await decide(listing, "B302", "B101")).toEqual({ on: true, explicit: true, because: ".bandit: \"B101\" in tests" });
    expect(await decide("[bandit]\nprofile = strict\nskips = B101\n", "B303", "B101")).toEqual({ on: false, explicit: true, because: ".bandit: \"B101\" in skips" });
    expect(await decide("[bandit]\nskips = B101\n", "B303", "B606")).toEqual({ on: true, explicit: false, because: ".bandit: no tests set, so Bandit runs every test" });
  });

  it("gives no verdict on a port without a Bandit test id", async () => {
    const root = await repository({ ".bandit": "[bandit]\nskips = B101\n" });
    const ruff = [...ports.values()].find(port => port.suppress.nosec.length === 0)!;
    expect(banditReader.read(root, [])!.verdict(ruff, ruleCatalog)).toBeUndefined();
  });
});
