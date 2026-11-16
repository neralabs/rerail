import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, posix } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import {
  portsOf,
  settleVerdicts,
  type LinterNote,
  type LinterReader,
  type Port,
  type PortVerdict,
} from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { phpcsReader } from "../../../../src/engine/catalog/lint-config/phpcs-config.js";
import { phpstanReader } from "../../../../src/engine/catalog/lint-config/phpstan-config.js";
import { psalmReader } from "../../../../src/engine/catalog/lint-config/psalm-config.js";
import { fixturesPath, repositoryPath } from "../../../support/oracles.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/**
 * The PHP readers against the linters themselves: each case of `fixtures/host/lint-config/<linter>/` is a repository's
 * config and what the linter, at the version the php pack pins, reports in a probe at each probe path. The reader
 * must not say on where the linter reports nothing, or off where it reports: it may leave the pack's value (unknown)
 * only where a case below expects it, and differ only where a case below expects a note about it (a pattern narrower
 * than a folder, a message it matches). Its verdict alone, settled for a rule starting on and for one starting off,
 * must come to the same state at each probe path, and as explicit as a case below says where it says.
 */
interface OracleCase {
  readonly name: string;
  readonly files: Readonly<Record<string, string>>;
  /** Per probe path, the ported codes, identifiers or issues the linter reports there. */
  readonly reported?: Readonly<Record<string, readonly string[]>>;
  /** The error the linter stops with, reporting nothing. */
  readonly error?: string;
}

interface Oracle {
  readonly probe: string;
  readonly cases: readonly OracleCase[];
}

/** What each linter's oracle checks, and where the reader may leave the pack's value or differ with a note. */
interface OracleSpec {
  readonly reader: LinterReader;
  readonly kind: "phpcs" | "phpstan" | "psalm";
  /** Per case, the rules (or `*`, every rule) the reader leaves at the pack's value. */
  readonly unknown: Readonly<Record<string, readonly string[]>>;
  /**
   * Per case, `<rule> <probe path>` (or a probe path, for every rule) where the reader differs from the linter, and
   * text the note that says so holds: a note of the rule's verdict or of the reader.
   */
  readonly noted: Readonly<Record<string, Readonly<Record<string, string>>>>;
  /** Per case, whether the reader's decision for a rule, wherever it decides, is explicit (the config names it). */
  readonly explicit: Readonly<Record<string, Readonly<Record<string, boolean>>>>;
}

/** ForbiddenFunctions reports `Found` only for a function a property forbids with no alternative; the probe reports its sibling. */
const STAND_IN: Readonly<Record<string, string>> = { "Generic.PHP.ForbiddenFunctions.FoundWithAlternative": "Generic.PHP.ForbiddenFunctions.Found" };

/** The same note for `rule` at each of `paths`. */
const noteAt = (rule: string, paths: readonly string[], note: string): Record<string, string> => Object.fromEntries(paths.map(path => [`${rule} ${path}`, note]));
const STAN_PATHS = ["probe.php", "lib/src/Legacy/probe.php", "src/Legacy/probe.php", "src/probe.php", "tests/probe.php"];
const LOWER_CASE_CONSTANT = "phpcs-generic-php-lower-case-constant";

const SPECS: readonly OracleSpec[] = [
  {
    reader: phpcsReader,
    kind: "phpcs",
    unknown: {
      "include-pattern": ["phpcs-squiz-php-eval"],
      "argument-severity": ["*"],
      "argument-no-warnings": ["*"],
    },
    noted: {
      "exclude-patterns": {
        "phpcs-squiz-php-eval lib/src/Legacy/probe.php": "<exclude-pattern>src/Legacy/*</exclude-pattern> also matches lib/src/Legacy/probe.php",
        "phpcs-squiz-php-eval src/tests/probe.php": "<exclude-pattern>*/tests/*</exclude-pattern> also matches src/tests/probe.php",
        [`${LOWER_CASE_CONSTANT} lib/src/Legacy/probe.php`]: "<exclude-pattern>src/Legacy</exclude-pattern> also matches lib/src/Legacy/probe.php",
        [`${LOWER_CASE_CONSTANT} src/LegacyOld/probe.php`]: "<exclude-pattern>src/Legacy</exclude-pattern> also matches lib/src/Legacy/probe.php, src/LegacyOld/probe.php",
        "phpcs-generic-php-backtick-operator lib/src/Legacy/probe.php": "<exclude-pattern>*/src/*</exclude-pattern> also matches lib/src/Legacy/probe.php",
      },
      "exclude-pattern-ruleset": { "src/tests/probe.php": "phpcs.xml: <exclude-pattern>*/tests/*</exclude-pattern> also matches src/tests/probe.php" },
      "exclude-pattern-ruleset-nested": { "lib/src/Legacy/probe.php": "build/base.xml: <exclude-pattern>src/Legacy/*</exclude-pattern> also matches lib/src/Legacy/probe.php" },
      "ignore-argument": { "src/tests/probe.php": "<arg name=\"ignore\" value=\"build/*,*/tests/*\"/> also matches src/tests/probe.php" },
    },
    explicit: {
      "standard-psr12": { "phpcs-generic-php-lower-case-constant": false },
      "settings-ruleset-three-parts": { [LOWER_CASE_CONSTANT]: true },
      "severity-ruleset-three-parts": { [LOWER_CASE_CONSTANT]: true },
      "settings-ruleset-empty-part": { [LOWER_CASE_CONSTANT]: true },
      "severity-ruleset-empty-part": { [LOWER_CASE_CONSTANT]: true, "phpcs-squiz-php-eval": true },
      "settings-ruleset-zero-part": { [LOWER_CASE_CONSTANT]: true },
    },
  },
  {
    reader: phpstanReader,
    kind: "phpstan",
    unknown: {
      "level-config-and-level": ["*"],
      "level-config-root-dir": ["*"],
      "no-level": ["*"],
      "custom-ruleset": ["*"],
      "custom-ruleset-rules": ["phpstan-argument-sprintf", "phpstan-argument-sscanf", "phpstan-array-duplicate-key", "phpstan-if-always-false", "phpstan-random-int-parameters"],
      "rules-own-class": ["phpstan-if-always-false", "phpstan-random-int-parameters"],
      "rules-other-class": ["phpstan-missing-function-return-type", "phpstan-random-int-parameters"],
      "services-tagged": ["phpstan-if-always-false", "phpstan-random-int-parameters"],
    },
    noted: {
      "ignore-folder": {
        "phpstan-argument-sprintf src/probe.php": "argument.sprintf ignores it in \"src/probe.php\", which is not a folder",
        ...noteAt("phpstan-array-duplicate-key", ["lib/src/Legacy/probe.php", "src/Legacy/probe.php"], "array.duplicateKey ignores it in \"*/Legacy/*\", which is not a folder"),
      },
      "ignore-partial": {
        ...noteAt("phpstan-if-always-false", STAN_PATHS, "if.alwaysFalse ignores only the errors its message matches"),
        "phpstan-missing-function-return-type probe.php": "missingType.return ignores only a count of its errors",
        // The entry with a message alone may match sprintf's error: the reader says it does not read such entries.
        ...noteAt("phpstan-argument-sprintf", STAN_PATHS, "1 of its ignoreErrors entry names no identifier"),
      },
    },
    explicit: {
      "level-6": { "phpstan-argument-sprintf": false, "phpstan-missing-function-return-type": false },
      "ignore-root-path": {
        "phpstan-argument-sprintf": true,
        "phpstan-random-int-parameters": true,
        "phpstan-if-always-false": true,
        "phpstan-array-duplicate-key": true,
        "phpstan-missing-function-return-type": true,
        "phpstan-argument-sscanf": false,
      },
    },
  },
  {
    reader: psalmReader,
    kind: "psalm",
    unknown: {},
    noted: {
      "handler-folders": { "psalm-forbidden-code src/probe.php": "covers <file name=\"src/probe.php\"/>, which is not a folder" },
    },
    explicit: {
      "level-default": { "psalm-forbidden-code": false },
      "handler-root-suppress": { "psalm-forbidden-code": true },
      "handler-root-after-folder": { "psalm-forbidden-code": true },
    },
  },
];

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

let ruleCatalog: RuleCatalog;
let ports: readonly Port[];
beforeAll(() => {
  ruleCatalog = readRuleCatalog([repositoryPath("packs", "php")]);
  ports = portsOf(ruleCatalog, ["php"]);
  // Settling a held-off rule turns it off whatever the linter says; the php pack holds none off.
  expect(ports.filter(port => port.heldOff !== undefined)).toEqual([]);
});

/** The case's repository: its config files and the probe at each probe path. */
async function repository(oracle: Oracle, oracleCase: OracleCase): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-php-oracle-")));
  temporaryDirectories.push(root);
  const probes = Object.keys(oracleCase.reported ?? {}).map(path => [path, oracle.probe]);
  for (const [path, text] of [...Object.entries(oracleCase.files), ...probes]) {
    mkdirSync(dirname(join(root, path!)), { recursive: true });
    writeFileSync(join(root, path!), text!);
  }
  return root;
}

/** Of `decided` (each for a folder, `""` every file), the deepest that holds probe path `path`. */
function deepestAt<T extends { readonly folder: string }>(decided: readonly T[], path: string): T | undefined {
  const folder = posix.dirname(path);
  const covering = decided.filter(decision => decision.folder === "" || folder === decision.folder || folder.startsWith(`${decision.folder}/`));
  return covering.toSorted((a, b) => b.folder.length - a.folder.length)[0];
}

/** The setting `verdict` alone settles to at a probe path, for `port` starting as `startsOn`; undefined: the pack's value. */
function settledAt(port: Port, startsOn: boolean, verdict: PortVerdict | undefined, path: string): { on: boolean; explicit: boolean } | undefined {
  const { settings } = settleVerdicts({ ...port, startsOn }, verdict === undefined ? [] : [verdict], []);
  const setting = deepestAt(settings.filter(candidate => candidate.on !== null), path);
  return setting === undefined ? undefined : { on: setting.on!, explicit: setting.explicit };
}

/** Whether the linter reports a message of `port` at `path`. */
function linterAt(spec: OracleSpec, port: Port, oracleCase: OracleCase, path: string): boolean {
  const names = port.suppress[spec.kind];
  return (oracleCase.reported?.[path] ?? []).some(reported => {
    const code = STAND_IN[reported] ?? reported;
    return names.some(name => code === name || code.startsWith(`${name}.`));
  });
}

const PROBE_PATHS = ["probe.php", "build/probe.php", "lib/src/Legacy/probe.php", "src/Legacy/Kept/probe.php", "src/Legacy/probe.php", "src/LegacyOld/probe.php", "src/probe.php", "src/tests/probe.php", "tests/probe.php"];

describe.each(SPECS)("$reader.linter against the linter's oracle", spec => {
  const oracle = JSON.parse(readFileSync(fixturesPath("host", "lint-config", spec.kind, `${spec.kind}.expected.json`), "utf8")) as Oracle;

  it("names only cases the oracle has", () => {
    const names = new Set(oracle.cases.map(oracleCase => oracleCase.name));
    expect([spec.unknown, spec.noted, spec.explicit].flatMap(Object.keys).filter(name => !names.has(name))).toEqual([]);
  });

  it.each(oracle.cases.map(oracleCase => [oracleCase.name, oracleCase] as const))("agrees with the linter on %s", async (name, oracleCase) => {
    const root = await repository(oracle, oracleCase);
    const notes: LinterNote[] = [];
    const config = spec.reader.read(root, notes);
    const allowedUnknown = spec.unknown[name] ?? [];
    const allowedNoted = spec.noted[name] ?? {};
    const explicit = spec.explicit[name] ?? {};
    const disagreements: string[] = [];
    const usedUnknown = new Set<string>();
    const usedNoted = new Set<string>();
    const usedExplicit = new Set<string>();
    const paths = oracleCase.error === undefined ? Object.keys(oracleCase.reported ?? {}) : PROBE_PATHS;
    for (const port of ports.filter(candidate => candidate.suppress[spec.kind].length > 0)) {
      const verdict = config?.verdict(port, ruleCatalog);
      const said = [...verdict?.notes ?? [], ...notes.map(note => note.text)];
      const stated = explicit[port.ruleId];
      for (const path of paths) {
        // The deepest folder decision that covers the path, else the whole decision.
        const decision = deepestAt(verdict?.folders ?? [], path) ?? verdict?.decision;
        const reader = decision?.on ?? "unknown";
        const linter = linterAt(spec, port, oracleCase, path);
        if (reader === "unknown") {
          const allowed = allowedUnknown.find(entry => entry === "*" || entry === port.ruleId);
          if (allowed === undefined) disagreements.push(`${port.ruleId} ${path}: unknown, the linter is ${linter ? "on" : "off"}`);
          else usedUnknown.add(allowed);
        } else if (reader !== linter) {
          const allowed = [`${port.ruleId} ${path}`, path].find(entry => allowedNoted[entry] !== undefined);
          if (allowed === undefined || !said.some(note => note.includes(allowedNoted[allowed]!))) disagreements.push(`${port.ruleId} ${path}: the reader is ${reader ? "on" : "off"}, the linter ${linter ? "on" : "off"}`);
          else usedNoted.add(allowed);
        }
        if (stated !== undefined && decision !== undefined && decision.on !== "unknown") {
          usedExplicit.add(port.ruleId);
          if (decision.explicit !== stated) disagreements.push(`${port.ruleId} ${path}: the reader's decision is ${decision.explicit ? "explicit" : "implicit"}`);
        }
        for (const startsOn of [true, false]) {
          const settled = settledAt(port, startsOn, verdict, path);
          const expected = reader === "unknown" ? startsOn : reader;
          if ((settled?.on ?? startsOn) !== expected) disagreements.push(`${port.ruleId} ${path}: starting ${startsOn ? "on" : "off"}, it settles ${expected ? "off" : "on"}`);
          if (settled !== undefined && stated !== undefined && settled.explicit !== stated) disagreements.push(`${port.ruleId} ${path}: starting ${startsOn ? "on" : "off"}, its setting is ${settled.explicit ? "explicit" : "implicit"}`);
        }
      }
    }
    expect(disagreements).toEqual([]);
    expect(allowedUnknown.filter(entry => !usedUnknown.has(entry))).toEqual([]);
    expect(Object.keys(allowedNoted).filter(entry => !usedNoted.has(entry))).toEqual([]);
    expect(Object.keys(explicit).filter(entry => !usedExplicit.has(entry))).toEqual([]);
  });
});
