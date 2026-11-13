import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { NOT_A_FILE, portsOf, type LinterNote, type Port, type PortDecision } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { sqlfluffReader } from "../../../../src/engine/catalog/lint-config/sqlfluff-config.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const IS_NULL = "sqlfluff-cv05-convention-is-null";
const COALESCE = "sqlfluff-cv02-convention-coalesce";
const UNIQUE_TABLE = "sqlfluff-al04-aliasing-unique-table";
const DISTINCT = "sqlfluff-am01-ambiguous-distinct";
const REFERENCES_FROM = "sqlfluff-rf01-references-from";
const LOCKS = "sqlfluff-pg01-postgres-excessive-locks";
const FOREIGN_KEY = "sqlfluff-pg02-postgres-not-valid-foreign-key";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-sqlfluff-config-")));
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
  ruleCatalog = readRuleCatalog([fileURLToPath(new URL("../../../../packs/sql", import.meta.url))]);
  ports = new Map(portsOf(ruleCatalog, ["sql"]).map(port => [port.ruleId, port]));
});

/** The decisions on `ruleIds` of the SQLFluff config at `root`, with the notes reading it gave; undefined with no config. */
function read(root: string, ...ruleIds: string[]): { decisions: (PortDecision | undefined)[]; notes: string[] } | undefined {
  const notes: LinterNote[] = [];
  const config = sqlfluffReader.read(root, notes);
  if (config === undefined) return undefined;
  return { decisions: ruleIds.map(ruleId => config.verdict(ports.get(ruleId)!, ruleCatalog)?.decision), notes: notes.map(note => note.text) };
}

const decisions = (root: string, ...ruleIds: string[]) => read(root, ...ruleIds)!.decisions;

const EVERY_RULE_RUNS = { on: true, explicit: false, because: ".sqlfluff: no rules set, so SQLFluff runs every rule" };
const NOT_SELECTED = { on: false, explicit: false, because: ".sqlfluff: rules does not select it" };

describe("sqlfluffReader", () => {
  it("reads no config where the repository has none for SQLFluff", async () => {
    expect(read(await repository({}), IS_NULL)).toBeUndefined();
    expect(read(await repository({ "setup.cfg": "[metadata]\nname = app\n", "tox.ini": "[tox]\nenvlist = py312\n" }), IS_NULL)).toBeUndefined();
    expect(read(await repository({ "pyproject.toml": "[tool.ruff]\nline-length = 100\n" }), IS_NULL)).toBeUndefined();
    // A shared file SQLFluff may not read is not refused when it names no SQLFluff config.
    expect(read(await repository({ "setup.cfg": "no section here\n" }), IS_NULL)).toBeUndefined();
    expect(read(await repository({ "pyproject.toml": "[tool.black\n" }), IS_NULL)).toBeUndefined();
  });

  it("runs every rule SQLFluff runs by default where no rules are set", async () => {
    const root = await repository({ ".sqlfluff": "[sqlfluff]\ndialect = postgres\n" });
    expect(decisions(root, IS_NULL, LOCKS)).toEqual([
      EVERY_RULE_RUNS,
      // PG01 reports nothing, selected or not, without force_enable.
      { on: false, explicit: false, because: ".sqlfluff: no force_enable = True for postgres.excessive_locks, without which SQLFluff reports nothing for it" },
    ]);
    // An empty .sqlfluff is still SQLFluff's config.
    expect(decisions(await repository({ ".sqlfluff": "" }), IS_NULL)).toEqual([EVERY_RULE_RUNS]);
    // So is an empty list in pyproject.toml.
    expect(decisions(await repository({ "pyproject.toml": "[tool.sqlfluff.core]\nrules = []\n" }), IS_NULL)).toEqual([
      { on: true, explicit: false, because: "pyproject.toml [tool.sqlfluff.core]: no rules set, so SQLFluff runs every rule" },
    ]);
  });

  it.each([
    // Values `coerce_value` makes falsy set the default selection.
    ["", EVERY_RULE_RUNS],
    ["None", EVERY_RULE_RUNS],
    ["false", EVERY_RULE_RUNS],
    ["0", EVERY_RULE_RUNS],
    ["0.0", EVERY_RULE_RUNS],
    ["-0e0", EVERY_RULE_RUNS],
    ["0_0", EVERY_RULE_RUNS],
    // `_` is no number to Python, so it is a reference naming no rule.
    ["_", NOT_SELECTED],
    // fnmatch globs, matched case-sensitively.
    ["CV0?", { on: true, explicit: true, because: ".sqlfluff: \"CV0?\" in rules" }],
    ["CV0[4-6]", { on: true, explicit: true, because: ".sqlfluff: \"CV0[4-6]\" in rules" }],
    ["[]C]V05", { on: true, explicit: true, because: ".sqlfluff: \"[]C]V05\" in rules" }],
    // fnmatch reads a leading `^` in a class as itself, not as negation.
    ["[^C]V05", { on: true, explicit: true, because: ".sqlfluff: \"[^C]V05\" in rules" }],
    ["[!C]V05", NOT_SELECTED],
    // An unclosed `[` is literal.
    ["[CV05", NOT_SELECTED],
    ["cv05", NOT_SELECTED],
    // A `-` that names no range is literal, inside a class or out.
    ["CV0[5-]", { on: true, explicit: true, because: ".sqlfluff: \"CV0[5-]\" in rules" }],
    ["CV0[-5]", { on: true, explicit: true, because: ".sqlfluff: \"CV0[-5]\" in rules" }],
    ["convention-is_null", NOT_SELECTED],
    // fnmatch takes no `-` as a range just after one: `1-3`, then `-` and `5`.
    ["CV0[1-3-5]", { on: true, explicit: true, because: ".sqlfluff: \"CV0[1-3-5]\" in rules" }],
    // A range running backwards is dropped: the class left empty matches nothing, or anything when negated.
    ["CV0[9-1]", NOT_SELECTED],
    ["CV0[!9-1]", { on: true, explicit: true, because: ".sqlfluff: \"CV0[!9-1]\" in rules" }],
    ["CV0[49-1]", NOT_SELECTED],
  ])("reads rules = %j as SQLFluff does", async (value, expected) => {
    expect(decisions(await repository({ ".sqlfluff": `[sqlfluff]\nrules = ${value}\n` }), IS_NULL)).toEqual([expected]);
  });

  it("turns off what exclude_rules names by code, alias, group or glob", async () => {
    const root = await repository({ ".sqlfluff": "[sqlfluff]\nexclude_rules = L049, aliasing,\n    RF0[!2-9]\n" });
    expect(decisions(root, IS_NULL, UNIQUE_TABLE, REFERENCES_FROM, DISTINCT)).toEqual([
      { on: false, explicit: true, because: ".sqlfluff: \"L049\" in exclude_rules" },
      { on: false, explicit: true, because: ".sqlfluff: \"aliasing\" in exclude_rules" },
      { on: false, explicit: true, because: ".sqlfluff: \"RF0[!2-9]\" in exclude_rules" },
      EVERY_RULE_RUNS,
    ]);
  });

  it("runs only what rules selects, by name glob or group", async () => {
    const root = await repository({ ".sqlfluff": "[sqlfluff]\nrules = convention.is_*,core\n" });
    expect(decisions(root, IS_NULL, DISTINCT, COALESCE)).toEqual([
      { on: true, explicit: true, because: ".sqlfluff: \"convention.is_*\" in rules" },
      { on: true, explicit: true, because: ".sqlfluff: \"core\" in rules" },
      NOT_SELECTED,
    ]);
    const every = await repository({ ".sqlfluff": "[sqlfluff]\nrules = all\n" });
    expect(decisions(every, COALESCE)).toEqual([{ on: true, explicit: false, because: ".sqlfluff: \"all\" in rules" }]);
  });

  it("lets exclude_rules win over rules", async () => {
    const root = await repository({ ".sqlfluff": "[sqlfluff]\nrules = CV05,CV02\nexclude_rules = convention.is_null\n" });
    expect(decisions(root, IS_NULL, COALESCE)).toEqual([
      { on: false, explicit: true, because: ".sqlfluff: \"convention.is_null\" in exclude_rules" },
      { on: true, explicit: true, because: ".sqlfluff: \"CV02\" in rules" },
    ]);
  });

  it("takes each key from the last file that sets it: setup.cfg, tox.ini, pep8.ini, .sqlfluff, pyproject.toml", async () => {
    const root = await repository({
      "setup.cfg": "[sqlfluff]\nrules = CV05\nexclude_rules = AL04\n",
      "tox.ini": "[sqlfluff]\nexclude_rules = AM01\n",
      ".sqlfluff": "[sqlfluff]\nrules = AL04,AM01\n",
      "pyproject.toml": "[tool.sqlfluff.core]\nrules = [\"AL04\", \"CV02\"]\n",
    });
    expect(decisions(root, UNIQUE_TABLE, COALESCE, IS_NULL)).toEqual([
      { on: true, explicit: true, because: "pyproject.toml [tool.sqlfluff.core]: \"AL04\" in rules" },
      { on: true, explicit: true, because: "pyproject.toml [tool.sqlfluff.core]: \"CV02\" in rules" },
      { on: false, explicit: false, because: "pyproject.toml [tool.sqlfluff.core]: rules does not select it" },
    ]);
    expect(decisions(root, DISTINCT)).toEqual([{ on: false, explicit: true, because: "tox.ini: \"AM01\" in exclude_rules" }]);
    // `None` sets the default again over an earlier file's rules.
    const reset = await repository({ "setup.cfg": "[sqlfluff]\nrules = CV05\n", ".sqlfluff": "[sqlfluff]\nrules = None\n" });
    expect(decisions(reset, COALESCE)).toEqual([EVERY_RULE_RUNS]);
  });

  it("reads a comma-separated string in pyproject.toml as SQLFluff does", async () => {
    const root = await repository({ "pyproject.toml": "[tool.sqlfluff.core]\nexclude_rules = \"CV05, AM01\"\n" });
    expect(decisions(root, IS_NULL, DISTINCT)).toEqual([
      { on: false, explicit: true, because: "pyproject.toml [tool.sqlfluff.core]: \"CV05\" in exclude_rules" },
      { on: false, explicit: true, because: "pyproject.toml [tool.sqlfluff.core]: \"AM01\" in exclude_rules" },
    ]);
  });

  it("turns on a rule that needs force_enable only where its own section sets it", async () => {
    const root = await repository({
      ".sqlfluff": "[sqlfluff]\n\n[sqlfluff:rules:postgres.excessive_locks]\nforce_enable = True\n",
      "pyproject.toml": "[tool.sqlfluff.rules.postgres.not_valid_foreign_key]\nforce_enable = false\n",
    });
    expect(decisions(root, LOCKS, FOREIGN_KEY)).toEqual([
      { on: true, explicit: true, because: ".sqlfluff [sqlfluff:rules:postgres.excessive_locks]: force_enable = True" },
      { on: false, explicit: true, because: "pyproject.toml [tool.sqlfluff.rules.\"postgres.not_valid_foreign_key\"]: force_enable = false" },
    ]);
    const zero = await repository({ ".sqlfluff": "[sqlfluff:rules:postgres.excessive_locks]\nforce_enable = 0\n" });
    expect(decisions(zero, LOCKS)).toEqual([{ on: false, explicit: true, because: ".sqlfluff [sqlfluff:rules:postgres.excessive_locks]: force_enable = 0" }]);
    // force_enable does not run a rule the selection leaves out.
    const excluded = await repository({ ".sqlfluff": "[sqlfluff]\nexclude_rules = postgres\n[sqlfluff:rules:postgres.excessive_locks]\nforce_enable = True\n" });
    expect(decisions(excluded, LOCKS)).toEqual([{ on: false, explicit: true, because: ".sqlfluff: \"postgres\" in exclude_rules" }]);
  });

  it.each([
    ["true", true],
    ["1", true],
    ["0x1", true],
    ["0", false],
    ["false", false],
    // Only True, False, 1 and 0 pass SQLFluff's check: on anything else, a TOML string or another number included, it stops.
    ["1_0", "unknown"],
    ["\"False\"", "unknown"],
    ["\"\"", "unknown"],
    ["inf", "unknown"],
  ])("reads force_enable = %s in pyproject.toml's quoted rule table as SQLFluff does", async (value, on) => {
    const root = await repository({ "pyproject.toml": `[tool.sqlfluff.rules."postgres.excessive_locks"]\nforce_enable = ${value}\n` });
    const where = `pyproject.toml [tool.sqlfluff.rules."postgres.excessive_locks"]: force_enable = ${value}`;
    const because = on === "unknown" ? `${where}: SQLFluff stops with an error where it selects the rule, as force_enable must be True or False` : where;
    expect(decisions(root, LOCKS, IS_NULL)).toEqual([
      on === "unknown" ? { on, because } : { on, explicit: true, because },
      on === "unknown" ? { on, because } : { on: true, explicit: false, because: "pyproject.toml: no rules set, so SQLFluff runs every rule" },
    ]);
  });

  it.each([
    ["True", true],
    ["1.0", true],
    ["-0", false],
    ["None", false],
    ["yes", "unknown"],
    ["2", "unknown"],
    ["nan", "unknown"],
  ])("reads force_enable = %s in .sqlfluff as configparser and coerce_value do", async (value, on) => {
    const root = await repository({ ".sqlfluff": `[sqlfluff:rules:postgres.excessive_locks]\nforce_enable = ${value}\n` });
    const where = `.sqlfluff [sqlfluff:rules:postgres.excessive_locks]: force_enable = ${value}`;
    expect(decisions(root, LOCKS)).toEqual([
      on === "unknown" ? { on, because: `${where}: SQLFluff stops with an error where it selects the rule, as force_enable must be True or False` } : { on, explicit: true, because: where },
    ]);
  });

  it("leaves the pack's values where rules is a value SQLFluff stops on, and reads false ones as unset", async () => {
    const number = await repository({ ".sqlfluff": "[sqlfluff]\nrules = 1\nexclude_rules = CV05\n" });
    expect(decisions(number, IS_NULL, COALESCE)).toEqual([
      { on: false, explicit: true, because: ".sqlfluff: \"CV05\" in exclude_rules" },
      { on: "unknown", because: ".sqlfluff: SQLFluff stops with an error on rules = 1, which is not a list" },
    ]);
    const word = await repository({ ".sqlfluff": "[sqlfluff]\nrules = TRUE\n" });
    expect(decisions(word, IS_NULL)).toEqual([{ on: "unknown", because: ".sqlfluff: SQLFluff stops with an error on rules = TRUE, which is not a list" }]);
    const unset = await repository({ ".sqlfluff": "[sqlfluff]\nrules = 0.0\nexclude_rules = none\n" });
    expect(decisions(unset, IS_NULL)).toEqual([{ on: true, explicit: false, because: ".sqlfluff: no rules set, so SQLFluff runs every rule" }]);
  });

  it("leaves the pack's values where a file SQLFluff reads is not read, as far as it may change them", async () => {
    const colon = await repository({ "setup.cfg": "[sqlfluff]\nexclude_rules = CV05\n", ".sqlfluff": "[sqlfluff]\nrules: CV05\n" });
    expect(read(colon, IS_NULL, COALESCE)).toEqual({
      decisions: [{ on: "unknown", because: ".sqlfluff is not read" }, { on: "unknown", because: ".sqlfluff is not read" }],
      notes: [".sqlfluff is not read: line 2: \"rules: CV05\" is not a section header or a \"key = value\" option."],
    });
    // A file before the one that sets rules can only have excluded what they select.
    const earlier = await repository({ "tox.ini": "[sqlfluff]\nrules = AL04\nrules = CV05\n", ".sqlfluff": "[sqlfluff]\nrules = CV05\n" });
    expect(read(earlier, IS_NULL, COALESCE)).toEqual({
      decisions: [{ on: "unknown", because: "tox.ini is not read" }, NOT_SELECTED],
      notes: ["tox.ini is not read: line 3: option rules in [sqlfluff] is set twice."],
    });
    // Nor can it set force_enable where a later file does not.
    const forced = await repository({ "setup.cfg": "[sqlfluff]\nrules: CV05\n", ".sqlfluff": "[sqlfluff]\nrules = postgres\nexclude_rules = None\n" });
    expect(read(forced, LOCKS, IS_NULL)).toEqual({
      decisions: [{ on: "unknown", because: "setup.cfg is not read" }, NOT_SELECTED],
      notes: ["setup.cfg is not read: line 2: \"rules: CV05\" is not a section header or a \"key = value\" option."],
    });
    const toml = await repository({ "pyproject.toml": "[tool.sqlfluff.core]\nrules = 3\n" });
    expect(read(toml, IS_NULL)).toEqual({
      decisions: [{ on: "unknown", because: "pyproject.toml is not read" }],
      notes: ["pyproject.toml is not read: rules in [tool.sqlfluff.core] is not a string or a list of strings."],
    });
    const list = await repository({ "pyproject.toml": "[tool.sqlfluff.rules.postgres.excessive_locks]\nforce_enable = [true]\n" });
    expect(read(list, LOCKS)).toEqual({
      decisions: [{ on: "unknown", because: "pyproject.toml is not read" }],
      notes: ["pyproject.toml is not read: force_enable of postgres.excessive_locks in [tool.sqlfluff.rules] is not a boolean."],
    });
    const folder = await repository({ ".sqlfluff/x": "" });
    expect(read(folder, IS_NULL)).toEqual({ decisions: [{ on: "unknown", because: ".sqlfluff is not read" }], notes: [`.sqlfluff is not read: ${NOT_A_FILE}`] });
  });

  it("decides each folder holding a config by the configs from the root down to it", async () => {
    const root = await repository({
      ".sqlfluff": "[sqlfluff]\nrules = AL04,CV05\n",
      "models/.sqlfluff": "[sqlfluff]\nexclude_rules = CV05\n",
      "models/staging/setup.cfg": "[sqlfluff]\nrules = CV02\n",
      "models/staging/x.sql": "select 1\n",
      "other/setup.cfg": "[metadata]\nname = x\n",
    });
    expect(sqlfluffReader.read(root, [])!.verdict(ports.get(IS_NULL)!, ruleCatalog)).toEqual({
      decision: { on: true, explicit: true, because: ".sqlfluff: \"CV05\" in rules" },
      folders: [
        { folder: "models", on: false, explicit: true, because: "models/.sqlfluff: \"CV05\" in exclude_rules" },
        { folder: "models/staging", on: false, explicit: true, because: "models/.sqlfluff: \"CV05\" in exclude_rules" },
      ],
    });
    // A config only below the root: SQLFluff's defaults for every other file.
    const nestedOnly = await repository({ "models/.sqlfluff": "[sqlfluff]\nrules = CV05\n" });
    expect(sqlfluffReader.read(nestedOnly, [])!.verdict(ports.get(COALESCE)!, ruleCatalog)).toEqual({
      decision: { on: true, explicit: false, because: "SQLFluff's defaults: no rules set, so SQLFluff runs every rule" },
      folders: [{ folder: "models", on: false, explicit: false, because: "models/.sqlfluff: rules does not select it" }],
    });
  });

  it("turns every rule off in the folders SQLFluff ignores by literal path, noting files and leaving other patterns' folders to the pack", async () => {
    const root = await repository({
      ".sqlfluffignore": "# built\n\n/generated/\nlegacy.sql\nseeds  \n",
      ".sqlfluff": "[sqlfluff]\nignore_paths = models/staging\n",
      "generated/a.sql": "",
      "seeds/b.sql": "",
      "dbt/seeds/c.sql": "",
      "models/staging/d.sql": "",
      "models/staging/.sqlfluff": "[sqlfluff]\nrules = AL04\n",
      "other/legacy.sql": "",
      "macros/.sqlfluffignore": "*.tmp.sql\n",
      "macros/e.sql": "",
    });
    const notes: LinterNote[] = [];
    expect(sqlfluffReader.read(root, notes)!.verdict(ports.get(IS_NULL)!, ruleCatalog)).toEqual({
      decision: { on: true, explicit: false, because: ".sqlfluff: no rules set, so SQLFluff runs every rule" },
      folders: [
        { folder: "generated", on: false, explicit: true, because: ".sqlfluffignore: \"/generated/\"" },
        { folder: "dbt/seeds", on: false, explicit: true, because: ".sqlfluffignore: \"seeds  \"" },
        { folder: "seeds", on: false, explicit: true, because: ".sqlfluffignore: \"seeds  \"" },
        { folder: "macros", on: "unknown", because: "macros/.sqlfluffignore is not read" },
        { folder: "models/staging", on: false, explicit: true, because: ".sqlfluff: ignore_paths: \"models/staging\"" },
      ],
    });
    expect(notes.map(note => note.text)).toEqual([
      "macros/.sqlfluffignore is not read: \"*.tmp.sql\" names no path literally, and Rerail reads only those. The rules keep the pack's values where it applies.",
      ".sqlfluffignore: SQLFluff ignores other/legacy.sql, which Rerail cannot set rules for alone (it sets them for whole folders), so the rules stay on there.",
    ]);
  });

  it("leaves every file to the pack where an ignore spec at the root is not read, or a .sqlfluff SQLFluff reads everywhere", async () => {
    const negation = await repository({ ".sqlfluffignore": "models/\n!models/staging/\n" });
    expect(read(negation, IS_NULL)!.decisions).toEqual([{ on: "unknown", because: ".sqlfluffignore is not read" }]);
    const folder = await repository({ ".sqlfluffignore/x": "" });
    expect(read(folder, IS_NULL)).toEqual({
      decisions: [{ on: "unknown", because: ".sqlfluffignore is not read" }],
      notes: [`.sqlfluffignore is not read: ${NOT_A_FILE} The rules keep the pack's values where it applies.`],
    });
    const nested = await repository({ ".sqlfluff": "[sqlfluff]\nrules = CV05\n", "models/.sqlfluff": "[sqlfluff]\nrules = A\nrules = B\n" });
    expect(sqlfluffReader.read(nested, [])!.verdict(ports.get(IS_NULL)!, ruleCatalog)).toEqual({ decision: { on: "unknown", because: "models/.sqlfluff is not read" }, folders: [] });
  });

  it.each([
    ["[sqlfluff]\nignore_paths = 0\n", ".sqlfluff"],
    ["[sqlfluff]\nignore_paths =\n", ".sqlfluff"],
    ["[tool.sqlfluff.core]\nignore_paths = []\n", "pyproject.toml"],
  ])("reads %j as ignoring nothing, as discovery does", async (text, name) => {
    const root = await repository({ [name]: text, "models/a.sql": "" });
    expect(sqlfluffReader.read(root, [])!.verdict(ports.get(IS_NULL)!, ruleCatalog)!.folders).toEqual([]);
  });

  it("reads pyproject.toml's ignore_paths as a comma-separated string, refusing other values", async () => {
    const text = await repository({ "pyproject.toml": "[tool.sqlfluff.core]\nignore_paths = \"models,seeds\"\n", "models/a.sql": "", "seeds/b.sql": "" });
    expect(sqlfluffReader.read(text, [])!.verdict(ports.get(IS_NULL)!, ruleCatalog)!.folders).toEqual([
      { folder: "models", on: false, explicit: true, because: "pyproject.toml: ignore_paths: \"models\"" },
      { folder: "seeds", on: false, explicit: true, because: "pyproject.toml: ignore_paths: \"seeds\"" },
    ]);
    const number = await repository({ "pyproject.toml": "[tool.sqlfluff.core]\nignore_paths = 1\n" });
    expect(read(number, IS_NULL)!.notes).toEqual(["pyproject.toml is not read: ignore_paths in [tool.sqlfluff.core] is not a string or a list of strings."]);
  });

  it("gives no verdict on a port without a SQLFluff reference", async () => {
    const root = await repository({ ".sqlfluff": "[sqlfluff]\n" });
    const port = ports.get(IS_NULL)!;
    expect(sqlfluffReader.read(root, [])!.verdict({ ...port, suppress: { ...port.suppress, sqlfluff: [] } }, ruleCatalog)).toBeUndefined();
  });
});
