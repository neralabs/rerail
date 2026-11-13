import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, posix } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { portsOf, type LinterNote, type LinterReader, type Port, type PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { phpcsReader } from "../../../../src/engine/catalog/lint-config/phpcs-config.js";
import { phpstanReader } from "../../../../src/engine/catalog/lint-config/phpstan-config.js";
import { psalmReader } from "../../../../src/engine/catalog/lint-config/psalm-config.js";
import { fixturesPath, repositoryPath } from "../../../support/oracles.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

/**
 * The PHP readers against the linters themselves: each case of `fixtures/host/lint-config/<linter>/` is a repository's
 * config and what the linter, at the version the php pack pins, reports in a probe at each probe path. The reader
 * must not say on where the linter reports nothing, or off where it reports: it may leave the pack's value (unknown)
 * only where a case below expects it, and differ only where a case below expects a note (a pattern narrower than a
 * folder, a message it matches).
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
  /** Per case, `<rule> <probe path>` where the reader differs from the linter and says so in a note. */
  readonly noted: Readonly<Record<string, readonly string[]>>;
}

/** ForbiddenFunctions reports `Found` only for a function a property forbids with no alternative; the probe reports its sibling. */
const STAND_IN: Readonly<Record<string, string>> = { "Generic.PHP.ForbiddenFunctions.FoundWithAlternative": "Generic.PHP.ForbiddenFunctions.Found" };

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
      "exclude-patterns": [
        "phpcs-squiz-php-eval lib/src/Legacy/probe.php",
        "phpcs-squiz-php-eval src/tests/probe.php",
        "phpcs-generic-php-lower-case-constant lib/src/Legacy/probe.php",
        "phpcs-generic-php-lower-case-constant src/LegacyOld/probe.php",
        "phpcs-generic-php-backtick-operator lib/src/Legacy/probe.php",
      ],
      "exclude-pattern-ruleset": ["src/tests/probe.php"],
      "exclude-pattern-ruleset-nested": ["lib/src/Legacy/probe.php"],
      "ignore-argument": ["src/tests/probe.php"],
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
      "ignore-folder": [
        "phpstan-argument-sprintf src/probe.php",
        "phpstan-array-duplicate-key lib/src/Legacy/probe.php",
        "phpstan-array-duplicate-key src/Legacy/probe.php",
      ],
      "ignore-partial": [
        "phpstan-if-always-false probe.php",
        "phpstan-if-always-false lib/src/Legacy/probe.php",
        "phpstan-if-always-false src/Legacy/probe.php",
        "phpstan-if-always-false src/probe.php",
        "phpstan-if-always-false tests/probe.php",
        "phpstan-missing-function-return-type probe.php",
        "phpstan-argument-sprintf probe.php",
        "phpstan-argument-sprintf lib/src/Legacy/probe.php",
        "phpstan-argument-sprintf src/Legacy/probe.php",
        "phpstan-argument-sprintf src/probe.php",
        "phpstan-argument-sprintf tests/probe.php",
      ],
    },
  },
  {
    reader: psalmReader,
    kind: "psalm",
    unknown: {},
    noted: {
      "handler-folders": ["psalm-forbidden-code src/probe.php"],
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

/** The reader's state for a probe path: its folder's decision (the deepest that covers it), else the whole decision. */
function readerAt(verdict: PortVerdict | undefined, path: string): boolean | "unknown" {
  if (verdict === undefined) return "unknown";
  const folder = posix.dirname(path);
  const covering = (verdict.folders ?? []).filter(decision => folder === decision.folder || folder.startsWith(`${decision.folder}/`));
  const deepest = covering.toSorted((a, b) => b.folder.length - a.folder.length)[0];
  return deepest === undefined ? verdict.decision?.on ?? "unknown" : deepest.on;
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
    expect([...Object.keys(spec.unknown), ...Object.keys(spec.noted)].filter(name => !names.has(name))).toEqual([]);
  });

  it.each(oracle.cases.map(oracleCase => [oracleCase.name, oracleCase] as const))("agrees with the linter on %s", async (name, oracleCase) => {
    const root = await repository(oracle, oracleCase);
    const notes: LinterNote[] = [];
    const config = spec.reader.read(root, notes);
    const allowedUnknown = spec.unknown[name] ?? [];
    const allowedNoted = spec.noted[name] ?? [];
    const disagreements: string[] = [];
    const usedUnknown = new Set<string>();
    const usedNoted = new Set<string>();
    const paths = oracleCase.error === undefined ? Object.keys(oracleCase.reported ?? {}) : PROBE_PATHS;
    for (const port of ports.filter(candidate => candidate.suppress[spec.kind].length > 0)) {
      const verdict = config?.verdict(port, ruleCatalog);
      for (const path of paths) {
        const reader = readerAt(verdict, path);
        const linter = linterAt(spec, port, oracleCase, path);
        if (reader === "unknown") {
          const allowed = allowedUnknown.find(entry => entry === "*" || entry === port.ruleId);
          if (allowed === undefined) disagreements.push(`${port.ruleId} ${path}: unknown, the linter is ${linter ? "on" : "off"}`);
          else usedUnknown.add(allowed);
        } else if (reader !== linter) {
          const allowed = allowedNoted.find(entry => entry === `${port.ruleId} ${path}` || entry === path);
          const said = (verdict?.notes ?? []).length > 0 || notes.length > 0;
          if (allowed === undefined || !said) disagreements.push(`${port.ruleId} ${path}: the reader is ${reader ? "on" : "off"}, the linter ${linter ? "on" : "off"}`);
          else usedNoted.add(allowed);
        }
      }
    }
    expect(disagreements).toEqual([]);
    expect(allowedUnknown.filter(entry => !usedUnknown.has(entry))).toEqual([]);
    expect(allowedNoted.filter(entry => !usedNoted.has(entry))).toEqual([]);
  });
});
