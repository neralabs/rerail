import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { portsOf, type LinterNote, type LinterReader, type Port, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { phpstanReader } from "../../../../src/engine/catalog/lint-config/phpstan-config.js";
import { psalmReader } from "../../../../src/engine/catalog/lint-config/psalm-config.js";
import { phpcsReader } from "../../../../src/engine/catalog/lint-config/phpcs-config.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

/** A repository with `files`; a name ending in `/` is an empty folder. */
async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-php-lint-config-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) {
    if (name.endsWith("/")) {
      mkdirSync(join(root, name), { recursive: true });
      continue;
    }
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
  return root;
}

const packDirectory = (id: string): string => fileURLToPath(new URL(`../../../../packs/${id}`, import.meta.url));

let ruleCatalog: RuleCatalog;
let ports: Map<string, Port>;
beforeAll(() => {
  ruleCatalog = readRuleCatalog([packDirectory("php")]);
  ports = new Map(portsOf(ruleCatalog, ["php"]).map(port => [port.ruleId, port]));
});

const port = (ruleId: string): Port => {
  const found = ports.get(ruleId);
  if (found === undefined) throw new Error(`no port ${ruleId}`);
  return found;
};

/** What `reader` reads at `root`: the verdict per rule id asked for, and the notes. */
function read(reader: LinterReader, root: string, ruleIds: readonly string[]): { verdicts: Record<string, PortVerdict | undefined>; notes: string[]; read: boolean } {
  const notes: LinterNote[] = [];
  const config = reader.read(root, notes);
  const verdicts = Object.fromEntries(ruleIds.map(ruleId => [ruleId, config?.verdict(port(ruleId), ruleCatalog)]));
  expect(notes.every(note => note.linter === reader.linter)).toBe(true);
  return { verdicts, notes: notes.map(note => note.text), read: config !== undefined };
}

/** A port of another pack than php's, whose only names of `kind` are `names`. */
function customPort(kind: "phpstan" | "psalm", names: readonly string[]): Port {
  const base = port(SPRINTF);
  return { ...base, ruleId: "custom-port", suppress: { ...base.suppress, phpstan: [], [kind]: names } };
}

describe.each([
  { reader: phpstanReader, used: "phpstan.neon", shadowed: "phpstan.dist.neon", tool: "PHPStan" },
  { reader: psalmReader, used: "psalm.xml", shadowed: "psalm.xml.dist", tool: "Psalm" },
  { reader: phpcsReader, used: ".phpcs.xml", shadowed: "phpcs.xml.dist", tool: "PHP_CodeSniffer" },
])("$reader.linter's config at the root", ({ reader, used, shadowed, tool }) => {
  it("is not read when the one the linter uses is not a regular file, and the ones it shadows are noted", async () => {
    const root = await repository({ [`${used}/`]: "", [shadowed]: "" });
    const notes: LinterNote[] = [];
    expect(reader.read(root, notes)).toBeUndefined();
    expect(notes.map(note => note.text)).toEqual([
      `${shadowed} is not read: ${tool} uses ${used} instead.`,
      `${used} is not read: it is not a regular file of at most 1 MiB in the repository.`,
    ]);
  });
});

const SPRINTF = "phpstan-argument-sprintf";
const RETURN_TYPE = "phpstan-missing-function-return-type";
const IF_FALSE = "phpstan-if-always-false";
const RANDOM_INT = "phpstan-random-int-parameters";
const PHPSTAN_PORTS = [SPRINTF, RETURN_TYPE, IF_FALSE, RANDOM_INT];

describe("phpstanReader", () => {
  it("gives no verdict without a config, and none for a port of another linter", async () => {
    expect(read(phpstanReader, await repository({ "composer.json": "{}" }), PHPSTAN_PORTS)).toEqual({ verdicts: Object.fromEntries(PHPSTAN_PORTS.map(id => [id, undefined])), notes: [], read: false });
    const root = await repository({ "phpstan.neon": "parameters:\n\tlevel: 5\n" });
    expect(read(phpstanReader, root, ["psalm-forbidden-code"]).verdicts["psalm-forbidden-code"]).toBeUndefined();
  });

  it("turns on the rules of the level and below, and leaves the rules of higher levels off", async () => {
    const { verdicts, notes } = read(phpstanReader, await repository({ "phpstan.neon": "parameters:\n\tlevel: 5\n\tpaths: [src]\n" }), PHPSTAN_PORTS);
    expect(notes).toEqual([]);
    expect(verdicts[SPRINTF]).toEqual({ decision: { on: true, explicit: false, because: "phpstan.neon: level 5" } });
    expect(verdicts[IF_FALSE]?.decision).toEqual({ on: true, explicit: false, because: "phpstan.neon: level 5" });
    expect(verdicts[RANDOM_INT]?.decision).toEqual({ on: true, explicit: false, because: "phpstan.neon: level 5" });
    expect(verdicts[RETURN_TYPE]?.decision).toEqual({
      on: false,
      explicit: false,
      because: "phpstan.neon: level 5 does not turn it on (PHPStan reports missingType.return from level 6)",
    });
  });

  it("reads max as level 10 and a level 0 config as level 0", async () => {
    expect(read(phpstanReader, await repository({ "phpstan.neon.dist": "parameters:\n  level: max\n" }), [RETURN_TYPE]).verdicts[RETURN_TYPE]?.decision).toMatchObject({ on: true });
    const zero = read(phpstanReader, await repository({ "phpstan.dist.neon": "parameters:\n  level: 0\n" }), [SPRINTF, IF_FALSE]).verdicts;
    expect(zero[SPRINTF]?.decision).toMatchObject({ on: true });
    expect(zero[IF_FALSE]?.decision).toMatchObject({ on: false, explicit: false });
  });

  it("uses the first config in PHPStan's order", async () => {
    const root = await repository({ "phpstan.neon": "parameters:\n\tlevel: 6\n", "phpstan.neon.dist": "parameters:\n\tlevel: 1\n" });
    const { verdicts, notes } = read(phpstanReader, root, [RETURN_TYPE]);
    expect(verdicts[RETURN_TYPE]?.decision).toMatchObject({ on: true, because: "phpstan.neon: level 6" });
    expect(notes).toEqual(["phpstan.neon.dist is not read: PHPStan uses phpstan.neon instead."]);
  });

  it("turns a rule off for an identifier ignored everywhere, or in a folder, and notes narrower ignores", async () => {
    const root = await repository({
      "phpstan.neon": [
        "parameters:",
        "\tlevel: 6",
        "\tignoreErrors:",
        "\t\t- identifier: missingType.return",
        "\t\t-",
        "\t\t\tidentifiers: [argument.sprintf]",
        "\t\t\tpaths: [src/Legacy, src/one.php]",
        "\t\t- identifier: if.alwaysFalse",
        "\t\t  message: '#^If condition#'",
        "\t\t- '#^Some message#'",
      ].join("\n"),
      "src/Legacy/": "",
      "src/one.php": "<?php\n",
    });
    const { verdicts, notes } = read(phpstanReader, root, [RETURN_TYPE, SPRINTF, IF_FALSE]);
    expect(verdicts[RETURN_TYPE]?.decision).toEqual({ on: false, explicit: true, because: "phpstan.neon: ignoreErrors identifier missingType.return" });
    expect(verdicts[SPRINTF]).toEqual({
      decision: { on: true, explicit: false, because: "phpstan.neon: level 6" },
      folders: [{ folder: "src/Legacy", on: false, explicit: true, because: "phpstan.neon: ignoreErrors identifier argument.sprintf in path src/Legacy" }],
      notes: [`${SPRINTF}: phpstan.neon: ignoreErrors identifier argument.sprintf ignores it in "src/one.php", which is not a folder of the repository; Rerail sets rules for whole folders only.`],
    });
    expect(verdicts[IF_FALSE]).toEqual({
      decision: { on: true, explicit: false, because: "phpstan.neon: level 6" },
      notes: [`${IF_FALSE}: phpstan.neon: ignoreErrors identifier if.alwaysFalse ignores only the errors its message matches, so it does not turn the rule off.`],
    });
    expect(notes).toEqual(["phpstan.neon: 1 of its ignoreErrors entry names no identifier; Rerail reads the identifiers an entry names, not the messages it matches."]);
  });

  it("follows the repository's includes in nette/di's order, the including config winning", async () => {
    const root = await repository({
      "phpstan.neon": "includes:\n\t- build/phpstan-level.neon\n\t- phpstan-baseline.neon\nparameters:\n\tlevel: 6\n",
      "build/phpstan-level.neon": "parameters:\n\tlevel: 2\n\tignoreErrors:\n\t\t- identifier: argument.type\n\t\t  path: ../src\n",
      "phpstan-baseline.neon": "parameters:\n\tignoreErrors:\n\t\t-\n\t\t\tidentifier: missingType.return\n\t\t\tcount: 3\n\t\t\tpath: src/a.php\n",
      "src/a.php": "<?php\n",
    });
    const { verdicts, notes } = read(phpstanReader, root, [RETURN_TYPE, RANDOM_INT]);
    expect(notes).toEqual([]);
    expect(verdicts[RETURN_TYPE]?.decision).toEqual({ on: true, explicit: false, because: "phpstan.neon: level 6" });
    expect(verdicts[RETURN_TYPE]?.notes).toEqual([
      `${RETURN_TYPE}: phpstan-baseline.neon: ignoreErrors identifier missingType.return ignores only a count of its errors, so it does not turn the rule off.`,
      `${RETURN_TYPE}: phpstan-baseline.neon: ignoreErrors identifier missingType.return ignores it in "src/a.php", which is not a folder of the repository; Rerail sets rules for whole folders only.`,
    ]);
    expect(verdicts[RANDOM_INT]?.folders).toEqual([{ folder: "src", on: false, explicit: true, because: "build/phpstan-level.neon: ignoreErrors identifier argument.type in path src" }]);
  });

  it("drops the included ignores that ignoreErrors! replaces", async () => {
    const root = await repository({
      "phpstan.neon": "includes:\n\t- base.neon\nparameters:\n\tlevel: 6\n\tignoreErrors!:\n\t\t- identifier: argument.type\n",
      "base.neon": "parameters:\n\tignoreErrors:\n\t\t- identifier: missingType.return\n",
    });
    const { verdicts } = read(phpstanReader, root, [RETURN_TYPE, RANDOM_INT]);
    expect(verdicts[RETURN_TYPE]?.decision).toMatchObject({ on: true });
    expect(verdicts[RANDOM_INT]?.decision).toMatchObject({ on: false, explicit: true });
  });

  it("reads the level of a level config the phar ships, and reads past bleeding edge", async () => {
    const root = await repository({ "phpstan.neon": "includes:\n\t- phar://phpstan.phar/conf/config.level4.neon\n\t- vendor/phpstan/phpstan/conf/bleedingEdge.neon\n\t- '%rootDir%/conf/bleedingEdge.neon'\n" });
    const { verdicts, notes } = read(phpstanReader, root, [IF_FALSE, RANDOM_INT]);
    expect(notes).toEqual([]);
    expect(verdicts[IF_FALSE]?.decision).toEqual({ on: true, explicit: false, because: "phpstan.neon: includes phar://phpstan.phar/conf/config.level4.neon" });
    expect(verdicts[RANDOM_INT]?.decision).toMatchObject({ on: false, explicit: false });
  });

  it("does not take a level config through the Composer package, which has none", async () => {
    const root = await repository({ "phpstan.neon": "includes:\n\t- '%rootDir%/conf/config.level4.neon'\n" });
    expect(read(phpstanReader, root, [IF_FALSE]).verdicts[IF_FALSE]?.decision).toEqual({
      on: "unknown",
      because: "phpstan.neon's include \"%rootDir%/conf/config.level4.neon\", which Rerail does not read, may register or ignore it",
    });
  });

  it("leaves the pack's values where an extension it includes may decide, keeping what the config ignores", async () => {
    const root = await repository({
      "phpstan.neon": "includes:\n\t- vendor/phpstan/phpstan-strict-rules/rules.neon\nparameters:\n\tlevel: 8\n\tignoreErrors:\n\t\t- identifier: missingType.return\n",
    });
    const { verdicts, notes } = read(phpstanReader, root, [SPRINTF, RETURN_TYPE]);
    expect(verdicts[SPRINTF]?.decision).toEqual({
      on: "unknown",
      because: "phpstan.neon's include \"vendor/phpstan/phpstan-strict-rules/rules.neon\", which Rerail does not read, may register or ignore it",
    });
    expect(verdicts[RETURN_TYPE]?.decision).toMatchObject({ on: false, explicit: true });
    expect(notes).toEqual(["phpstan.neon's include \"vendor/phpstan/phpstan-strict-rules/rules.neon\" is not read (it is an installed package's config), so the PHPStan rules Rerail ports keep the pack's values unless the config ignores them."]);
  });

  it("leaves the pack's values when the config sets no level, which then comes from the command line", async () => {
    const { verdicts, notes } = read(phpstanReader, await repository({ "phpstan.neon": "parameters:\n\tpaths: [src]\n" }), [SPRINTF]);
    expect(verdicts[SPRINTF]?.decision).toEqual({ on: "unknown", because: "the config sets no level, so PHPStan takes it from --level on its command line, which Rerail does not read" });
    expect(notes).toHaveLength(1);
  });

  it("does not read a config that is not NEON, or a level PHPStan does not have", async () => {
    const broken = read(phpstanReader, await repository({ "phpstan.neon": "parameters:\n\tlevel: 5\n\tlevel: 6\n" }), [SPRINTF]);
    expect(broken.read).toBe(false);
    expect(broken.notes).toEqual(["phpstan.neon is not read: not NEON: duplicated key 'level' on line 3."]);
    const eleven = read(phpstanReader, await repository({ "phpstan.neon": "parameters:\n\tlevel: 11\n" }), [SPRINTF]);
    expect(eleven.verdicts[SPRINTF]?.decision).toEqual({ on: "unknown", because: "phpstan.neon: level 11 is not a level PHPStan has" });
  });

  it("reads the identifiers of each ignoreErrors shape, noting the entries and lists it does not read", async () => {
    const root = await repository({
      "phpstan.neon": [
        "parameters:",
        "\tlevel: 6",
        "\tignoreErrors:",
        "\t\t- identifiers: [argument.sprintf, if.alwaysFalse]",
        "\t\t- identifier: [1]",
        "\t\t- identifiers: [2]",
      ].join("\n"),
    });
    const { verdicts, notes } = read(phpstanReader, root, [SPRINTF, IF_FALSE, RETURN_TYPE]);
    const because = "phpstan.neon: ignoreErrors identifiers argument.sprintf, if.alwaysFalse";
    expect(verdicts[SPRINTF]?.decision).toEqual({ on: false, explicit: true, because });
    expect(verdicts[IF_FALSE]?.decision).toEqual({ on: false, explicit: true, because });
    expect(verdicts[RETURN_TYPE]?.decision).toMatchObject({ on: true });
    expect(notes).toEqual(["phpstan.neon: 2 of its ignoreErrors entries name no identifier; Rerail reads the identifiers an entry names, not the messages it matches."]);
    const scalar = read(phpstanReader, await repository({ "phpstan.neon": "parameters:\n\tlevel: 6\n\tignoreErrors: '#^Some message#'\n" }), [SPRINTF]);
    expect(scalar.verdicts[SPRINTF]?.decision).toMatchObject({ on: true });
    expect(scalar.notes).toEqual(["phpstan.neon: its ignoreErrors are not read: they are not a list."]);
  });

  it("limits an ignore to the paths that are folders of the repository, and notes every other path", async () => {
    const others = ["src/*/Generated", "%rootDir%/../src", "src/Missing", "/abs/src", "../outside", "."];
    const root = await repository({
      "phpstan.neon": [
        "parameters:",
        "\tlevel: 6",
        "\tignoreErrors:",
        "\t\t-",
        "\t\t\tidentifier: argument.sprintf",
        "\t\t\tpaths:",
        "\t\t\t\t- '%currentWorkingDirectory%/src/Legacy'",
        ...others.map(path => `\t\t\t\t- '${path}'`),
      ].join("\n"),
      "src/Legacy/": "",
    });
    const ignore = "phpstan.neon: ignoreErrors identifier argument.sprintf";
    expect(read(phpstanReader, root, [SPRINTF]).verdicts[SPRINTF]).toEqual({
      decision: { on: true, explicit: false, because: "phpstan.neon: level 6" },
      folders: [{ folder: "src/Legacy", on: false, explicit: true, because: `${ignore} in path src/Legacy` }],
      notes: others.map(path => `${SPRINTF}: ${ignore} ignores it in ${JSON.stringify(path)}, which is not a folder of the repository; Rerail sets rules for whole folders only.`),
    });
  });

  it("reads the repository's includes, and notes each include it does not read and why", async () => {
    const root = await repository({
      "phpstan.neon": [
        "includes:",
        "\t- 1",
        "\t- ../outside.neon",
        "\t- '%rootDir%/../extra.neon'",
        "\t- '%currentWorkingDirectory%/build/base.neon'",
        "\t- build/rules.php",
        "\t- missing.neon",
        "\t- folder.neon",
        "\t- broken.neon",
        "parameters:",
        "\tlevel: 6",
      ].join("\n"),
      "build/base.neon": "parameters:\n\tignoreErrors:\n\t\t- identifier: missingType.return\n",
      "folder.neon/": "",
      "broken.neon": "a: [1",
    });
    const { verdicts, notes } = read(phpstanReader, root, [SPRINTF, RETURN_TYPE]);
    expect(verdicts[SPRINTF]?.decision).toEqual({ on: "unknown", because: "phpstan.neon's include 1, which Rerail does not read, may register or ignore it" });
    expect(verdicts[RETURN_TYPE]?.decision).toEqual({ on: false, explicit: true, because: "build/base.neon: ignoreErrors identifier missingType.return" });
    const keep = "so the PHPStan rules Rerail ports keep the pack's values unless the config ignores them.";
    expect(notes).toEqual([
      expect.stringMatching(/^broken\.neon is not read: not NEON: /u),
      `phpstan.neon's include 1 is not read (it is not a path), ${keep}`,
      `phpstan.neon's include "../outside.neon" is not read (it is outside the repository or names a parameter), ${keep}`,
      `phpstan.neon's include "%rootDir%/../extra.neon" is not read (it is outside the repository or names a parameter), ${keep}`,
      `phpstan.neon's include "build/rules.php" is not read (it is PHP, which Rerail never runs), ${keep}`,
      `phpstan.neon's include "missing.neon" is not read (it does not exist), ${keep}`,
      `phpstan.neon's include "folder.neon" is not read (it is not a regular file of at most 1 MiB in the repository), ${keep}`,
    ]);
  });

  it("follows includes 8 deep, so a config including itself is read", async () => {
    const { verdicts, notes } = read(phpstanReader, await repository({ "phpstan.neon": "includes:\n\t- phpstan.neon\nparameters:\n\tlevel: 6\n" }), [SPRINTF]);
    expect(verdicts[SPRINTF]?.decision).toEqual({ on: "unknown", because: "phpstan.neon's include \"phpstan.neon\", which Rerail does not read, may register or ignore it" });
    expect(notes).toEqual(["phpstan.neon's include \"phpstan.neon\" is not read (includes are followed 8 deep), so the PHPStan rules Rerail ports keep the pack's values unless the config ignores them."]);
  });

  it.each([
    ["a level and the level config of the same level", "includes:\n\t- phar://phpstan.phar/conf/config.level4.neon\nparameters:\n\tlevel: 4\n", { on: true, explicit: false, because: "phpstan.neon: level 4" }],
    [
      "a level and the level config of another",
      "includes:\n\t- phar://phpstan.phar/conf/config.level4.neon\nparameters:\n\tlevel: 5\n",
      { on: "unknown", because: "phpstan.neon sets level 5 and phpstan.neon: includes phar://phpstan.phar/conf/config.level4.neon" },
    ],
    ["the max level config", "includes:\n\t- phar://phpstan.phar/conf/config.levelmax.neon\n", { on: true, explicit: false, because: "phpstan.neon: includes phar://phpstan.phar/conf/config.levelmax.neon" }],
    [
      "a custom ruleset and no level",
      "parameters:\n\tcustomRulesetUsed: true\n",
      { on: "unknown", because: "the config is a custom ruleset (customRulesetUsed), whose rules Rerail does not read" },
    ],
  ])("decides by the level of %s", async (_, text, decision) => {
    expect(read(phpstanReader, await repository({ "phpstan.neon": text }), [IF_FALSE]).verdicts[IF_FALSE]?.decision).toEqual(decision);
  });

  it("turns on a port whose own rule class rules: or a tagged service registers, whatever the level", async () => {
    const root = await repository({
      "phpstan.neon": [
        "parameters:",
        "\tlevel: 0",
        "rules:",
        "\t- \\phpstan\\rules\\functions\\MissingFunctionReturnTypehintRule",
        "\t- 1",
        "services:",
        "\t-",
        "\t\tfactory: PHPStan\\Rules\\Comparison\\IfConstantConditionRule",
        "\t\ttags: {phpstan.rules.rule: true}",
        "\t- PHPStan\\Rules\\Functions\\RandomIntParametersRule",
      ].join("\n"),
    });
    const { verdicts, notes } = read(phpstanReader, root, [RETURN_TYPE, IF_FALSE, SPRINTF, RANDOM_INT]);
    expect(verdicts[RETURN_TYPE]?.decision).toEqual({ on: true, explicit: true, because: "phpstan.neon: rules \\phpstan\\rules\\functions\\MissingFunctionReturnTypehintRule" });
    expect(verdicts[IF_FALSE]?.decision).toEqual({ on: true, explicit: true, because: "phpstan.neon: services tagged phpstan.rules.rule PHPStan\\Rules\\Comparison\\IfConstantConditionRule" });
    expect(verdicts[SPRINTF]?.decision).toEqual({ on: true, explicit: false, because: "phpstan.neon: level 0" });
    expect(verdicts[RANDOM_INT]?.decision).toEqual({
      on: "unknown",
      because: "phpstan.neon: rules \\phpstan\\rules\\functions\\MissingFunctionReturnTypehintRule registers a rule Rerail does not read, which may report argument.type",
    });
    const every = "registers a rule at every level, so the PHPStan rules Rerail ports that the level leaves off keep the pack's values unless it is the port's own rule.";
    expect(notes).toEqual([
      `phpstan.neon: rules \\phpstan\\rules\\functions\\MissingFunctionReturnTypehintRule ${every}`,
      `phpstan.neon: rules a rule ${every}`,
      `phpstan.neon: services tagged phpstan.rules.rule PHPStan\\Rules\\Comparison\\IfConstantConditionRule ${every}`,
    ]);
  });

  it("leaves the pack's value for a port the level leaves off where a conditional or unnamed rule may report it", async () => {
    const root = await repository({
      "phpstan.neon": [
        "parameters:",
        "\tcustomRulesetUsed: true",
        "services:",
        "\t-",
        "\t\tclass: Acme\\Rule()",
        "\t\ttags: [phpstan.rules.rule]",
        "conditionalTags:",
        "\tPHPStan\\Rules\\Functions\\MissingFunctionReturnTypehintRule:",
        "\t\tphpstan.rules.rule: %featureToggles.example%",
      ].join("\n"),
    });
    const { verdicts } = read(phpstanReader, root, [RETURN_TYPE]);
    expect(verdicts[RETURN_TYPE]?.decision).toEqual({ on: "unknown", because: "the config is a custom ruleset (customRulesetUsed), whose rules Rerail does not read" });
    const level = await repository({ "phpstan.neon": "parameters:\n\tlevel: 1\nconditionalTags:\n\tPHPStan\\Rules\\Functions\\MissingFunctionReturnTypehintRule:\n\t\tphpstan.rules.rule: %featureToggles.example%\n\t- {phpstan.rules.rule: true}\n" });
    expect(read(phpstanReader, level, [RETURN_TYPE]).verdicts[RETURN_TYPE]?.decision).toEqual({
      on: "unknown",
      because: "phpstan.neon: conditionalTags PHPStan\\Rules\\Functions\\MissingFunctionReturnTypehintRule a rule registers a rule Rerail does not read, which may report missingType.return",
    });
  });

  it("leaves the pack's value for a port of an identifier whose level Rerail does not know", async () => {
    const config = phpstanReader.read(await repository({ "phpstan.neon": "parameters:\n\tlevel: 9\n" }), []);
    expect(config?.verdict(customPort("phpstan", ["method.notFound"]), ruleCatalog)).toEqual({
      decision: { on: "unknown", because: "the level PHPStan reports method.notFound from is not known to Rerail" },
    });
  });
});

const FORBIDDEN = "psalm-forbidden-code";

/** A psalm.xml with `attributes` on its root and `body` inside it. */
const psalmXml = (attributes: string, body = ""): string =>
  `<?xml version="1.0"?>\n<psalm xmlns="https://getpsalm.org/schema/config" ${attributes}>\n<projectFiles><directory name="src"/></projectFiles>\n${body}\n</psalm>\n`;

describe("psalmReader", () => {
  it("gives no verdict without a config", async () => {
    expect(read(psalmReader, await repository({}), [FORBIDDEN])).toEqual({ verdicts: { [FORBIDDEN]: undefined }, notes: [], read: false });
  });

  it("turns an issue on at the levels Psalm reports it as an error, level 2 without errorLevel", async () => {
    expect(read(psalmReader, await repository({ "psalm.xml": psalmXml("") }), [FORBIDDEN]).verdicts[FORBIDDEN]).toEqual({
      decision: { on: true, explicit: false, because: "psalm.xml sets no errorLevel, so Psalm runs at level 2" },
    });
    expect(read(psalmReader, await repository({ "psalm.xml.dist": psalmXml("errorLevel=\"4\"") }), [FORBIDDEN]).verdicts[FORBIDDEN]?.decision)
      .toEqual({ on: true, explicit: false, because: "psalm.xml.dist: errorLevel 4" });
    expect(read(psalmReader, await repository({ "psalm.xml": psalmXml("errorLevel=\"5\"") }), [FORBIDDEN]).verdicts[FORBIDDEN]?.decision).toEqual({
      on: false,
      explicit: false,
      because: "psalm.xml: errorLevel 5, where ForbiddenCode is info, not an error (Psalm reports it as an error at levels 1 to 4)",
    });
  });

  it("follows an issue handler over the level: error on, info and suppress off", async () => {
    const handler = (level: string): string => `<issueHandlers><ForbiddenCode${level}/></issueHandlers>`;
    const verdict = async (attributes: string, level: string): Promise<PortVerdict | undefined> =>
      read(psalmReader, await repository({ "psalm.xml": psalmXml(attributes, handler(level)) }), [FORBIDDEN]).verdicts[FORBIDDEN];
    expect((await verdict("errorLevel=\"1\"", " errorLevel=\"suppress\""))?.decision).toEqual({ on: false, explicit: true, because: "psalm.xml: <ForbiddenCode errorLevel=\"suppress\"/>" });
    expect((await verdict("errorLevel=\"1\"", " errorLevel=\"info\""))?.decision).toMatchObject({ on: false, explicit: true });
    expect((await verdict("errorLevel=\"8\"", ""))?.decision).toEqual({ on: true, explicit: true, because: "psalm.xml: <ForbiddenCode/>" });
  });

  it("sets folders from an issue handler's errorLevel directories, the first covering a folder winning", async () => {
    const root = await repository({
      "psalm.xml": psalmXml("errorLevel=\"1\"", [
        "<issueHandlers>",
        "  <ForbiddenCode>",
        "    <errorLevel type=\"suppress\"><directory name=\"src/Legacy\"/><file name=\"src/one.php\"/></errorLevel>",
        "    <errorLevel type=\"error\"><directory name=\"src/Legacy/Kept\"/><directory name=\"tests/**\"/></errorLevel>",
        "  </ForbiddenCode>",
        "</issueHandlers>",
      ].join("\n")),
      "src/Legacy/Kept/": "",
      "src/one.php": "<?php\n",
      "tests/": "",
    });
    expect(read(psalmReader, root, [FORBIDDEN]).verdicts[FORBIDDEN]).toEqual({
      decision: { on: true, explicit: true, because: "psalm.xml: <ForbiddenCode/>" },
      folders: [
        { folder: "src/Legacy", on: false, explicit: true, because: "psalm.xml: <ForbiddenCode> <errorLevel type=\"suppress\"> <directory name=\"src/Legacy\"/>" },
        { folder: "tests", on: true, explicit: true, because: "psalm.xml: <ForbiddenCode> <errorLevel type=\"error\"> <directory name=\"tests\"/>" },
      ],
      notes: [`${FORBIDDEN}: psalm.xml: <ForbiddenCode> <errorLevel type="suppress"> covers <file name="src/one.php"/>, which is not a folder of the repository; Rerail sets rules for whole folders only.`],
    });
  });

  it("leaves the pack's value where a plugin may suppress an issue the level turns on", async () => {
    const plugins = "<plugins><pluginClass class=\"Psalm\\LaravelPlugin\\Plugin\"/></plugins>";
    const on = read(psalmReader, await repository({ "psalm.xml": psalmXml("errorLevel=\"3\"", plugins) }), [FORBIDDEN]);
    expect(on.verdicts[FORBIDDEN]?.decision).toEqual({ on: "unknown", because: "psalm.xml loads plugin Psalm\\LaravelPlugin\\Plugin, which Rerail never runs and which may suppress ForbiddenCode" });
    expect(on.notes).toHaveLength(1);
    const off = read(psalmReader, await repository({ "psalm.xml": psalmXml("errorLevel=\"7\"", plugins) }), [FORBIDDEN]);
    expect(off.verdicts[FORBIDDEN]?.decision).toMatchObject({ on: false, explicit: false });
  });

  it("does not read a config Psalm refuses, one that includes others, or a document type declaration", async () => {
    for (const [text, why] of [
      [psalmXml("errorLevel=\"9\""), "errorLevel \"9\" is not a level Psalm takes"],
      [psalmXml("", "<issueHandlers><ForbiddenCode errorLevel=\"loud\"/></issueHandlers>"), "<ForbiddenCode> errorLevel \"loud\" is not error, info or suppress"],
      [psalmXml("xmlns:xi=\"http://www.w3.org/2001/XInclude\"", "<xi:include href=\"handlers.xml\"/>"), "it includes other files (XInclude), which Rerail does not follow"],
      ["<!DOCTYPE psalm><psalm/>", "not XML Rerail reads: a document type declaration is not read (line 1)"],
      ["<config/>", "its root element is not <psalm>"],
      [psalmXml("", "<issueHandlers><ForbiddenCode><errorLevel><directory name=\"src\"/></errorLevel></ForbiddenCode></issueHandlers>"), "an <errorLevel> type \"\" is not error, info or suppress"],
    ] as const) {
      expect(read(psalmReader, await repository({ "psalm.xml": text }), [FORBIDDEN])).toEqual({ verdicts: { [FORBIDDEN]: undefined }, notes: [`psalm.xml is not read: ${why}.`], read: false });
    }
  });

  it("notes what an errorLevel covers that is not a folder of the repository", async () => {
    const unread = ["<directory name=\"src/*/Legacy\"/>", "<directory name=\"src/Missing\"/>", "<directory name=\".\"/>", "<directory name=\"/abs\"/>", "<referencedClass name=\"\"/>"];
    const root = await repository({
      "psalm.xml": psalmXml("", `<issueHandlers><ForbiddenCode><errorLevel type="info"><directory name="src/*/Legacy"/><directory name="src/Missing"/><directory name="."/><directory name="/abs"/><referencedClass/></errorLevel></ForbiddenCode></issueHandlers>`),
      "src/": "",
    });
    expect(read(psalmReader, root, [FORBIDDEN]).verdicts[FORBIDDEN]).toEqual({
      decision: { on: true, explicit: true, because: "psalm.xml: <ForbiddenCode/>" },
      notes: unread.map(what => `${FORBIDDEN}: psalm.xml: <ForbiddenCode> <errorLevel type="info"> covers ${what}, which is not a folder of the repository; Rerail sets rules for whole folders only.`),
    });
  });

  it("takes a PluginIssue handler as the handler of the issue it names, and notes each plugin by its class, file or element", async () => {
    const handlers = "<issueHandlers><PluginIssue errorLevel=\"error\"/><PluginIssue name=\"ForbiddenCode\" errorLevel=\"suppress\"/></issueHandlers>";
    const plugins = "<plugins><pluginClass class=\"Acme\\Plugin\"/><plugin filename=\"build/Plugin.php\"/><plugin/></plugins>";
    const { verdicts, notes } = read(psalmReader, await repository({ "psalm.xml": psalmXml("", handlers + plugins) }), [FORBIDDEN]);
    expect(verdicts[FORBIDDEN]?.decision).toEqual({ on: false, explicit: true, because: "psalm.xml: <ForbiddenCode errorLevel=\"suppress\"/>" });
    expect(notes).toEqual(["Acme\\Plugin", "build/Plugin.php", "plugin"].map(plugin => `psalm.xml: plugin ${plugin} is not run, so the Psalm rules Rerail ports that only its level turns on keep the pack's values.`));
  });

  it.each([
    ["an issue whose level Rerail does not know", "", { on: "unknown", because: "the level Psalm reports TaintedSql at is not known to Rerail" }],
    [
      "issues the config turns on and off",
      "<issueHandlers><ForbiddenCode errorLevel=\"suppress\"/><TaintedSql/></issueHandlers>",
      { on: "unknown", because: "the config turns some of its issues on and others off (psalm.xml: <ForbiddenCode errorLevel=\"suppress\"/>; psalm.xml: <TaintedSql/>)" },
    ],
    [
      "issues the config turns off alike",
      "<issueHandlers><ForbiddenCode errorLevel=\"suppress\"/><TaintedSql errorLevel=\"info\"/></issueHandlers>",
      { on: false, explicit: true, because: "psalm.xml: <ForbiddenCode errorLevel=\"suppress\"/>; psalm.xml: <TaintedSql errorLevel=\"info\"/>" },
    ],
  ])("decides a port of several issues as one: %s", async (_, body, decision) => {
    const config = psalmReader.read(await repository({ "psalm.xml": psalmXml("", body) }), []);
    expect(config?.verdict(customPort("psalm", ["ForbiddenCode", "TaintedSql"]), ruleCatalog)).toEqual({ decision });
  });

  it("gives no verdict on a port of another linter", async () => {
    expect(read(psalmReader, await repository({ "psalm.xml": psalmXml("") }), [SPRINTF]).verdicts[SPRINTF]).toBeUndefined();
  });
});

const EVAL = "phpcs-squiz-php-eval";
const LOWER_CASE = "phpcs-generic-php-lower-case-constant";
const BACKTICK = "phpcs-generic-php-backtick-operator";
const EMPTY = "phpcs-generic-code-analysis-empty-statement";
const SILENCED = "phpcs-generic-php-no-silenced-errors";
const SIZE = "phpcs-squiz-php-disallow-size-functions-in-loops";

/** A ruleset with `body`. */
const ruleset = (body: string): string => `<?xml version="1.0"?>\n<ruleset name="Project">\n${body}\n</ruleset>\n`;

describe("phpcsReader", () => {
  it("gives no verdict without a ruleset", async () => {
    expect(read(phpcsReader, await repository({ "composer.json": "{}" }), [EVAL])).toEqual({ verdicts: { [EVAL]: undefined }, notes: [], read: false });
  });

  it("turns on the sniffs of the standards it references, and leaves the others off", async () => {
    const psr12 = read(phpcsReader, await repository({ "phpcs.xml.dist": ruleset("<rule ref=\"PSR12\"/>") }), [LOWER_CASE, EVAL]);
    expect(psr12.verdicts[LOWER_CASE]).toEqual({ decision: { on: true, explicit: false, because: "phpcs.xml.dist: <rule ref=\"PSR12\">" } });
    expect(psr12.verdicts[EVAL]).toEqual({ decision: { on: false, explicit: false, because: "phpcs.xml.dist does not turn it on" } });
    const squiz = read(phpcsReader, await repository({ "phpcs.xml": ruleset("<rule ref=\"Squiz\"/>") }), [EVAL, EMPTY, BACKTICK]);
    expect(squiz.verdicts[EVAL]?.decision).toEqual({ on: true, explicit: false, because: "phpcs.xml: <rule ref=\"Squiz\">" });
    // Squiz's own <severity>0 for DetectedCATCH names no message the sniff reports (it reports DetectedCatch).
    expect(squiz.verdicts[EMPTY]).toEqual({ decision: { on: true, explicit: false, because: "phpcs.xml: <rule ref=\"Squiz\">" } });
    expect(squiz.verdicts[BACKTICK]?.decision).toMatchObject({ on: false, explicit: false });
  });

  it("uses the first ruleset in PHP_CodeSniffer's order", async () => {
    const root = await repository({ ".phpcs.xml": ruleset("<rule ref=\"Squiz\"/>"), "phpcs.xml.dist": ruleset("<rule ref=\"PSR12\"/>") });
    const { verdicts, notes } = read(phpcsReader, root, [EVAL]);
    expect(verdicts[EVAL]?.decision).toMatchObject({ on: true, because: ".phpcs.xml: <rule ref=\"Squiz\">" });
    expect(notes).toEqual(["phpcs.xml.dist is not read: PHP_CodeSniffer uses .phpcs.xml instead."]);
  });

  it("turns on a category, sniff or message the ruleset names, and off what it excludes or sets to severity 0", async () => {
    const root = await repository({
      "phpcs.xml": ruleset([
        "<rule ref=\"Generic.PHP\"><exclude name=\"Generic.PHP.NoSilencedErrors\"/></rule>",
        "<rule ref=\"Generic.PHP.BacktickOperator\"><severity>0</severity></rule>",
        "<rule ref=\"Squiz\"><exclude name=\"Squiz.PHP.Eval\"/></rule>",
        "<rule ref=\"Generic.CodeAnalysis.EmptyStatement.DetectedCatch\"><severity>0</severity></rule>",
      ].join("\n")),
    });
    const { verdicts } = read(phpcsReader, root, [LOWER_CASE, BACKTICK, SILENCED, EVAL, EMPTY]);
    expect(verdicts[LOWER_CASE]?.decision).toEqual({ on: true, explicit: true, because: "phpcs.xml: <rule ref=\"Generic.PHP\">" });
    expect(verdicts[BACKTICK]?.decision).toEqual({ on: false, explicit: true, because: "phpcs.xml: <rule ref=\"Generic.PHP.BacktickOperator\"><severity>0</severity>" });
    expect(verdicts[SILENCED]?.decision).toEqual({ on: false, explicit: true, because: "phpcs.xml: <rule ref=\"Generic.PHP\"><exclude name=\"Generic.PHP.NoSilencedErrors\"/>" });
    expect(verdicts[EVAL]?.decision).toEqual({ on: false, explicit: true, because: "phpcs.xml: <rule ref=\"Squiz\"><exclude name=\"Squiz.PHP.Eval\"/>" });
    expect(verdicts[EMPTY]).toEqual({
      decision: { on: true, explicit: false, because: "phpcs.xml: <rule ref=\"Squiz\">" },
      notes: [`${EMPTY}: phpcs.xml: <rule ref="Generic.CodeAnalysis.EmptyStatement.DetectedCatch"><severity>0</severity> turns off Generic.CodeAnalysis.EmptyStatement.DetectedCatch; the rule stays on for its other messages.`],
    });
  });

  it("includes only the message a message code names, of a sniff not included before", async () => {
    const { verdicts } = read(phpcsReader, await repository({ "phpcs.xml": ruleset("<rule ref=\"Generic.PHP.BacktickOperator.Found\"/>") }), [BACKTICK]);
    expect(verdicts[BACKTICK]?.decision).toEqual({ on: true, explicit: true, because: "phpcs.xml: <rule ref=\"Generic.PHP.BacktickOperator.Found\">" });
  });

  it("turns a rule off in the folders its exclude patterns leave out, noting the files they also match elsewhere and other patterns", async () => {
    const root = await repository({
      "phpcs.xml": ruleset([
        "<rule ref=\"Squiz.PHP.Eval\">",
        "  <exclude-pattern>src/Legacy/*</exclude-pattern>",
        "  <exclude-pattern>*/tests/*</exclude-pattern>",
        "  <exclude-pattern>*.inc</exclude-pattern>",
        "</rule>",
      ].join("\n")),
      "src/Legacy/a.php": "<?php\n",
      "tests/a.php": "<?php\n",
      "lib/src/Legacy/a.php": "<?php\n",
      "lib/tests/a.php": "<?php\n",
      "lib/tests/b.php": "<?php\n",
      "lib/tests/notes.md": "",
      "src/tests/a.php": "<?php\n",
      "src/tests/b.php": "<?php\n",
    });
    const rule = "phpcs.xml: <rule ref=\"Squiz.PHP.Eval\">";
    expect(read(phpcsReader, root, [EVAL]).verdicts[EVAL]).toEqual({
      decision: { on: true, explicit: true, because: rule },
      folders: [
        { folder: "src/Legacy", on: false, explicit: true, because: `${rule}<exclude-pattern>src/Legacy/*</exclude-pattern>` },
        { folder: "tests", on: false, explicit: true, because: `${rule}<exclude-pattern>*/tests/*</exclude-pattern>` },
      ],
      notes: [
        `${EVAL}: ${rule}<exclude-pattern>src/Legacy/*</exclude-pattern> also matches lib/src/Legacy/a.php outside src/Legacy; Rerail sets rules for whole folders only, so the rule stays on there.`,
        `${EVAL}: ${rule}<exclude-pattern>*/tests/*</exclude-pattern> also matches lib/tests/a.php, lib/tests/b.php, src/tests/a.php and 1 more outside tests; Rerail sets rules for whole folders only, so the rule stays on there.`,
        `${EVAL}: ${rule}<exclude-pattern>*.inc</exclude-pattern> leaves out files that are not one folder of the repository; Rerail sets rules for whole folders only.`,
      ],
    });
  });

  it("turns every rule off in the folders a ruleset's own exclude patterns and the ignore argument leave out", async () => {
    const root = await repository({
      "phpcs.xml": ruleset("<exclude-pattern>src/Legacy/*</exclude-pattern>\n<exclude-pattern phpcbf-only=\"true\">tests/*</exclude-pattern>\n<arg name=\"ignore\" value=\"build/*, ,generated\\,old/*\"/>\n<rule ref=\"./base.xml\"/>"),
      "base.xml": ruleset("<exclude-pattern>docs/*</exclude-pattern>\n<rule ref=\"Squiz.PHP.Eval\"/>"),
      "src/Legacy/": "",
      "build/": "",
      "docs/": "",
      "tests/": "",
    });
    expect(read(phpcsReader, root, [EVAL]).verdicts[EVAL]).toEqual({
      decision: { on: true, explicit: true, because: "phpcs.xml: <rule ref=\"./base.xml\">: base.xml: <rule ref=\"Squiz.PHP.Eval\">" },
      folders: [
        { folder: "src/Legacy", on: false, explicit: true, because: "phpcs.xml: <exclude-pattern>src/Legacy/*</exclude-pattern>" },
        { folder: "docs", on: false, explicit: true, because: "base.xml: <exclude-pattern>docs/*</exclude-pattern>" },
        { folder: "build", on: false, explicit: true, because: "phpcs.xml: <arg name=\"ignore\" value=\"build/*, ,generated\\,old/*\"/>" },
      ],
      notes: [`${EVAL}: phpcs.xml: <arg name="ignore" value="build/*, ,generated\\,old/*"/> leaves out files that are not one folder of the repository; Rerail sets rules for whole folders only.`],
    });
  });

  it("reads a ref that is a file at the root as a ruleset, whatever its name, as PHP_CodeSniffer checks for a file first", async () => {
    const root = await repository({
      ".phpcs.xml": ruleset("<rule ref=\"phpcs.xml.dist\"/>\n<rule ref=\"Acme.Rules\"/>\n<rule ref=\"./build/base.xml\"/>"),
      "phpcs.xml.dist": ruleset("<rule ref=\"Squiz.PHP.Eval\"/>"),
      "Acme.Rules": ruleset("<rule ref=\"Generic.PHP.BacktickOperator\"/>"),
      "build/base.xml": ruleset("<rule ref=\"./near.xml\"/>\n<rule ref=\"far.xml\"/>\n<rule ref=\"./root.xml\"/>\n<rule ref=\"build\"/>\n<rule ref=\"Sniffs\"/>"),
      "build/near.xml": ruleset("<rule ref=\"Generic.PHP.NoSilencedErrors\"/>"),
      "far.xml": ruleset("<rule ref=\"Generic.PHP.LowerCaseConstant\"/>"),
      "build/far.xml": ruleset("<rule ref=\"Generic.CodeAnalysis.EmptyStatement\"/>"),
      "root.xml": ruleset("<rule ref=\"Squiz.PHP.DisallowSizeFunctionsInLoops\"/>"),
      "build/ruleset.xml": ruleset(""),
      "Sniffs/": "",
    });
    const { verdicts, notes } = read(phpcsReader, root, [EVAL, BACKTICK, SILENCED, LOWER_CASE, EMPTY, SIZE]);
    expect(notes).toEqual(["build/base.xml: \"Sniffs\" is not read (it is a folder of sniffs, which Rerail never runs), so the PHP_CodeSniffer rules Rerail ports that only a standard turns on keep the pack's values."]);
    expect(verdicts[EVAL]?.decision).toEqual({ on: true, explicit: true, because: ".phpcs.xml: <rule ref=\"phpcs.xml.dist\">: phpcs.xml.dist: <rule ref=\"Squiz.PHP.Eval\">" });
    expect(verdicts[BACKTICK]?.decision).toMatchObject({ on: true, explicit: true });
    expect(verdicts[SILENCED]?.decision).toMatchObject({ on: true, explicit: true });
    expect(verdicts[LOWER_CASE]?.decision).toMatchObject({ on: true, explicit: true });
    expect(verdicts[EMPTY]?.decision).toEqual({ on: "unknown", because: "build/base.xml: <rule ref=\"Sniffs\"> (it is a folder of sniffs, which Rerail never runs) is not read and may include it" });
    expect(verdicts[SIZE]?.decision).toEqual({
      on: true,
      explicit: true,
      because: ".phpcs.xml: <rule ref=\"./build/base.xml\">: build/base.xml: <rule ref=\"./root.xml\">: root.xml: <rule ref=\"Squiz.PHP.DisallowSizeFunctionsInLoops\">",
    });
  });

  it("follows a ruleset file in the repository, and leaves the pack's values where a standard it does not read may decide", async () => {
    const root = await repository({
      "phpcs.xml": ruleset("<rule ref=\"./build/base.xml\"/>\n<rule ref=\"WordPress\"/>\n<rule ref=\"PSR12\"/>"),
      "build/base.xml": ruleset("<rule ref=\"Generic.PHP.NoSilencedErrors\"/>"),
    });
    const { verdicts, notes } = read(phpcsReader, root, [SILENCED, LOWER_CASE, EVAL]);
    expect(verdicts[SILENCED]?.decision).toEqual({ on: true, explicit: true, because: "phpcs.xml: <rule ref=\"./build/base.xml\">: build/base.xml: <rule ref=\"Generic.PHP.NoSilencedErrors\">" });
    expect(verdicts[LOWER_CASE]?.decision).toEqual({ on: "unknown", because: "phpcs.xml: <rule ref=\"WordPress\"> (it is a coding standard PHP_CodeSniffer does not ship, which Rerail does not read) is not read and may set its severity" });
    expect(verdicts[EVAL]?.decision).toMatchObject({ on: "unknown" });
    expect(notes).toEqual(["phpcs.xml: \"WordPress\" is not read (it is a coding standard PHP_CodeSniffer does not ship, which Rerail does not read), so the PHP_CodeSniffer rules Rerail ports that only a standard turns on keep the pack's values."]);
  });

  it("reads the exclude and sniffs arguments, skips phpcbf-only elements, and leaves the pack's values under a severity threshold", async () => {
    const excluded = read(phpcsReader, await repository({
      "phpcs.xml": ruleset("<arg name=\"exclude\" value=\"Squiz.PHP.Eval\"/>\n<rule ref=\"Squiz\"/>\n<rule ref=\"Generic.PHP.BacktickOperator\" phpcbf-only=\"true\"/>"),
    }), [EVAL, BACKTICK]).verdicts;
    expect(excluded[EVAL]?.decision).toEqual({ on: false, explicit: true, because: "phpcs.xml: <arg name=\"exclude\" value=\"Squiz.PHP.Eval\"/>" });
    expect(excluded[BACKTICK]?.decision).toMatchObject({ on: false, explicit: false });
    const only = read(phpcsReader, await repository({ "phpcs.xml": ruleset("<arg name=\"sniffs\" value=\"Generic.PHP.LowerCaseConstant\"/>\n<rule ref=\"Squiz\"/>") }), [EVAL, LOWER_CASE]).verdicts;
    expect(only[EVAL]?.decision).toMatchObject({ on: false, explicit: true });
    expect(only[LOWER_CASE]?.decision).toMatchObject({ on: true });
    const threshold = read(phpcsReader, await repository({ "phpcs.xml": ruleset("<arg value=\"sn\"/>\n<rule ref=\"Squiz\"><exclude name=\"Squiz.PHP.Eval\"/></rule>") }), [EVAL, LOWER_CASE]);
    expect(threshold.verdicts[LOWER_CASE]?.decision).toEqual({ on: "unknown", because: "phpcs.xml: <arg value=\"sn\"/> changes which severities are reported" });
    expect(threshold.verdicts[EVAL]?.decision).toMatchObject({ on: false, explicit: true });
  });

  it("does not read a ruleset with a document type declaration, or one that is not a ruleset", async () => {
    const doctype = read(phpcsReader, await repository({ "phpcs.xml": "<!DOCTYPE ruleset><ruleset/>" }), [EVAL]);
    expect(doctype).toEqual({ verdicts: { [EVAL]: undefined }, notes: ["phpcs.xml is not read: not XML Rerail reads: a document type declaration is not read (line 1)."], read: false });
    expect(read(phpcsReader, await repository({ "phpcs.xml": "<psalm/>" }), [EVAL]).notes).toEqual(["phpcs.xml is not read: its root element is not <ruleset>."]);
  });

  it("follows ruleset files and folders in the repository, noting each reference it does not read and why", async () => {
    const unread = [
      ["~/standards/acme.xml", "it is outside the repository"],
      ["../outside.xml", "it is outside the repository"],
      ["vendor/acme/ruleset.xml", "it is an installed package's ruleset"],
      ["./Sniffs/MySniff.php", "it is a sniff, which Rerail never runs"],
      ["./missing.xml", "it does not exist"],
      ["./standard", "it is not a regular file of at most 1 MiB in the repository"],
      ["constructor", "it is a coding standard PHP_CodeSniffer does not ship, which Rerail does not read"],
      ["./broken.xml", "not XML Rerail reads: </ruleset> closes <rule> (line 1)"],
      ["./other.xml", "its root element is not <ruleset>"],
    ];
    const root = await repository({
      "phpcs.xml": ruleset([...unread.map(([ref]) => ref), "."].map(ref => `<rule ref="${ref}"/>`).join("\n")),
      "standard/ruleset.xml/": "",
      "Sniffs/MySniff.php": "<?php\n",
      "broken.xml": "<ruleset><rule></ruleset>",
      "other.xml": "<psalm/>",
      "ruleset.xml": ruleset("<rule ref=\"Generic.PHP.NoSilencedErrors\"/>"),
    });
    const { verdicts, notes } = read(phpcsReader, root, [SILENCED, EVAL]);
    expect(verdicts[SILENCED]?.decision).toEqual({ on: true, explicit: true, because: "phpcs.xml: <rule ref=\".\">: ruleset.xml: <rule ref=\"Generic.PHP.NoSilencedErrors\">" });
    expect(verdicts[EVAL]?.decision).toEqual({ on: "unknown", because: "phpcs.xml: <rule ref=\"~/standards/acme.xml\"> (it is outside the repository) is not read and may include it" });
    expect(notes).toEqual(unread.map(([ref, why]) => `phpcs.xml: ${JSON.stringify(ref)} is not read (${why}), so the PHP_CodeSniffer rules Rerail ports that only a standard turns on keep the pack's values.`));
  });

  it("follows ruleset files 8 deep, so a ruleset referencing itself is read", async () => {
    const { verdicts, notes } = read(phpcsReader, await repository({ "phpcs.xml": ruleset("<rule ref=\"./phpcs.xml\"/>") }), [EVAL]);
    expect(verdicts[EVAL]?.decision).toEqual({ on: "unknown", because: "phpcs.xml: <rule ref=\"./phpcs.xml\"> (rulesets are followed 8 deep) is not read and may include it" });
    expect(notes).toEqual(["phpcs.xml: \"./phpcs.xml\" is not read (rulesets are followed 8 deep), so the PHP_CodeSniffer rules Rerail ports that only a standard turns on keep the pack's values."]);
  });

  it("skips a rule without a ref and an exclude without a name, leaves the pack's value under an include pattern, and notes properties it does not map", async () => {
    const { verdicts, notes } = read(phpcsReader, await repository({
      "phpcs.xml": ruleset([
        "<exclude-pattern>src/Legacy/*</exclude-pattern>",
        "<rule><severity>0</severity></rule>",
        "<rule ref=\"Squiz.PHP\"><exclude/><exclude name=\"Squiz.PHP.Eval.Discouraged\"/><include-pattern>src/*</include-pattern></rule>",
        "<rule ref=\"Generic.PHP.LowerCaseConstant\"><include-pattern>src/*</include-pattern><exclude-pattern>src/Old/*</exclude-pattern><properties><property name=\"a\" value=\"b\"/></properties></rule>",
      ].join("\n")),
      "src/Legacy/": "",
      "src/Old/": "",
    }), [EVAL, LOWER_CASE]);
    expect(notes).toEqual([]);
    expect(verdicts[EVAL]).toEqual({
      decision: { on: false, explicit: true, because: "phpcs.xml: <rule ref=\"Squiz.PHP\"><exclude name=\"Squiz.PHP.Eval.Discouraged\"/>" },
      folders: [{ folder: "src/Legacy", on: false, explicit: true, because: "phpcs.xml: <exclude-pattern>src/Legacy/*</exclude-pattern>" }],
    });
    expect(verdicts[LOWER_CASE]).toEqual({
      decision: { on: "unknown", because: "phpcs.xml: <rule ref=\"Generic.PHP.LowerCaseConstant\"><include-pattern>src/*</include-pattern> limits it to the files it matches; Rerail sets rules for whole folders only" },
      folders: [{ folder: "src/Legacy", on: false, explicit: true, because: "phpcs.xml: <exclude-pattern>src/Legacy/*</exclude-pattern>" }],
      notes: [`${LOWER_CASE}: the properties of phpcs.xml: <rule ref="Generic.PHP.LowerCaseConstant"> are not mapped; it keeps its default options.`],
    });
  });

  it("turns a message back on that a later rule includes after its sniff's severity 0", async () => {
    const root = await repository({ "phpcs.xml": ruleset("<rule ref=\"Generic.PHP.BacktickOperator\"><severity>0</severity></rule>\n<rule ref=\"Generic.PHP.BacktickOperator.Found\"/>") });
    expect(read(phpcsReader, root, [BACKTICK]).verdicts[BACKTICK]).toEqual({ decision: { on: true, explicit: true, because: "phpcs.xml: <rule ref=\"Generic.PHP.BacktickOperator\">" } });
  });

  it("notes a folder exclude pattern that leaves out only some of a port's messages", async () => {
    const root = await repository({
      "phpcs.xml": ruleset("<rule ref=\"Generic.CodeAnalysis\"/>\n<rule ref=\"Generic.CodeAnalysis.EmptyStatement.DetectedCatch\"><exclude-pattern>src/Legacy/*</exclude-pattern></rule>"),
      "src/Legacy/": "",
    });
    expect(read(phpcsReader, root, [EMPTY]).verdicts[EMPTY]).toEqual({
      decision: { on: true, explicit: true, because: "phpcs.xml: <rule ref=\"Generic.CodeAnalysis\">" },
      notes: [`${EMPTY}: phpcs.xml: <rule ref="Generic.CodeAnalysis.EmptyStatement.DetectedCatch"><exclude-pattern>src/Legacy/*</exclude-pattern> leaves out only some of its messages; Rerail sets rules for whole folders only.`],
    });
  });

  it("takes each argument from the first that sets it, a ruleset's own over those of the rulesets it references", async () => {
    const root = await repository({
      "phpcs.xml": ruleset([
        "<arg/>",
        "<arg name=\"exclude\" value=\"Squiz.PHP.Eval\"/>",
        "<arg name=\"exclude\" value=\"Generic.PHP.LowerCaseConstant\"/>",
        "<rule ref=\"./build/base.xml\"/>",
        "<rule ref=\"PSR12\"/>",
        "<rule ref=\"Generic.PHP.BacktickOperator\"/>",
        "<rule ref=\"Squiz.PHP.Eval\"/>",
      ].join("\n")),
      "build/base.xml": ruleset("<arg name=\"exclude\" value=\"Generic.PHP.BacktickOperator\"/>\n<arg name=\"sniffs\" value=\"Squiz.PHP.Eval, Generic.PHP.LowerCaseConstant\"/>"),
    });
    const { verdicts, notes } = read(phpcsReader, root, [EVAL, LOWER_CASE, BACKTICK]);
    expect(notes).toEqual([]);
    expect(verdicts[EVAL]?.decision).toEqual({ on: false, explicit: true, because: "phpcs.xml: <arg name=\"exclude\" value=\"Squiz.PHP.Eval\"/>" });
    expect(verdicts[LOWER_CASE]?.decision).toEqual({ on: true, explicit: false, because: "phpcs.xml: <rule ref=\"PSR12\">" });
    expect(verdicts[BACKTICK]?.decision).toEqual({ on: false, explicit: true, because: "build/base.xml: <arg name=\"sniffs\" value=\"Squiz.PHP.Eval, Generic.PHP.LowerCaseConstant\"/>" });
  });

  it("gives no verdict on a port of another linter", async () => {
    expect(read(phpcsReader, await repository({ "phpcs.xml": ruleset("<rule ref=\"PSR12\"/>") }), [SPRINTF]).verdicts[SPRINTF]).toBeUndefined();
  });
});
