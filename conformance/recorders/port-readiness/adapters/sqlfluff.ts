import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { runChecked, type CommandRunner } from "../../../port-readiness/process.js";
import { upstreamLicenseText } from "../../../port-readiness/recorded-adapter.js";
import { sqlfluffSpan, SQLFLUFF_SPAN_PARTS, violationSpan } from "../../../port-readiness/span-units/sqlfluff.js";
import type { FixtureTextSource, PackAdapter, PackManifest, Span, UpstreamCase, UpstreamRule } from "../../../port-readiness/types.js";
import { createPythonToolkit, pythonRegex } from "../languages/python.js";
import { checkoutUpstream, existsExactly, treeDigest, uvRuntime } from "../runtimes.js";

/**
 * The SQLFluff adapter. Expected answers come from running the pinned SQLFluff itself on every case of each rule's
 * YAML test file (test/fixtures/rules/std_rule_cases/<CODE>.yml), configured as the ports are: `dialect: postgres`,
 * `templater: raw` and the port's rule configuration (sqlfluff_driver.py's PORT_RULES), since the engine reads every
 * .sql file as PostgreSQL. A case SQLFluff runs otherwise (Jinja, another rule configuration, other core settings) or
 * that its postgres dialect cannot read whole is skipped, with the reason. The YAML's own `violations` are upstream's
 * written expectation only on a case its configuration already runs as postgres.
 */

const DRIVER = fileURLToPath(new URL("./sqlfluff_driver.py", import.meta.url));
/** SQLFluff publishes its PyPI package from src/sqlfluff at the release tag, which is the bare version. */
const PACKAGE = "src/sqlfluff";
const CASES = "test/fixtures/rules/std_rule_cases";
/** SQLFluff's package and its rule test cases (checkoutUpstream adds its licence). */
const SPARSE_PATHS = [`/${PACKAGE}/`, `/${CASES}/`];
/** SQLFluff's `-- noqa` directives (src/sqlfluff/core/rules/noqa.py): `noqa`, `noqa: <rules>`, `noqa: disable=<rules>`, `noqa: enable=<rules>`. */
const NOQA_VOCABULARY = ["noqa", "disable", "enable", "all"];

interface Finding { rule: string; line: number; column: number }
type LintResult = { excluded: string } | { findings: Finding[] };
interface RuleFact { name: string; module: string; file: string; groups: string[]; aliases: string[]; configKeywords: string[] }
interface Facts {
  version: string;
  license: string;
  python: string;
  package: string;
  rules: Record<string, RuleFact>;
  defaultSelected: string[];
  regexes: Record<string, { pattern: string; flags: number }[]>;
  keywords: string[];
  /** The postgres dialect's sets of names by label, which code reads at run time (`dialect.sets("bare_functions")`). */
  dialectSets: Record<string, string[]>;
}
interface CaseEntry {
  case: string;
  dialect: string;
  sql: string;
  expect: "pass" | "fail";
  skipped: string | null;
  markupApplies: boolean;
  violations: [number, number][] | null;
  findings?: Finding[];
}
interface Cases {
  texts: string[];
  configWords: string[];
  rules: Record<string, CaseEntry[]>;
  defaultOff: Record<string, boolean>;
}

/** A word of SQL text: an identifier-shaped run, as the gates read a rule's code (policy-literals.ts's CODE_WORD). */
const SQL_WORD = /[A-Za-z_][\w$]*/gu;

/**
 * Names as SQL reads them: SQLFluff compares a name's or keyword's upper-cased text (`raw_upper`) with upper-case
 * literals (CV02's `{"IFNULL", "NVL"}`), so a name it spells in upper case is that name in any letter case.
 */
const foldedNames = (names: string[]): string[] => names.flatMap((name) => (/^[A-Z][A-Z0-9_]*$/u.test(name) ? [name, name.toLowerCase()] : [name]));

export async function createSqlfluffAdapter(pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner): Promise<PackAdapter> {
  if (pack.upstream.recommended_configs.join(",") !== "default") {
    throw new Error(`${pack.id}: SQLFluff has no named configurations; recommended_configs must be [default], meaning SQLFluff's default configuration (rules = all)`);
  }
  const upstream = await checkoutUpstream(cacheRoot, pack.upstream, pack.upstream.version, PACKAGE, SPARSE_PATHS, runner);
  const runtime = await uvRuntime(pack.upstream.runtime, cacheRoot, runner);
  const python = await createPythonToolkit(cacheRoot, runner);
  // -B: the driver's process pool imports it as a module, which would otherwise write __pycache__/ beside it.
  const ask = async <T>(request: object): Promise<T> =>
    JSON.parse(await runChecked(runner, [runtime.python, "-I", "-B", DRIVER], { input: JSON.stringify(request), cwd: cacheRoot })) as T;
  const codes = names.map((name) => name.toUpperCase());
  const facts = await ask<Facts>({ op: "facts", codes });
  const cases = await ask<Cases>({ op: "cases", checkout: upstream.directory, codes: codes.filter((code) => facts.rules[code] !== undefined) });

  // The wheel ships src/sqlfluff's Python modules and package data, not the Markdown notes it keeps for contributors.
  const installedDigest = treeDigest(facts.package, (path) => !path.includes("__pycache__"));
  const checkoutDigest = treeDigest(join(upstream.directory, PACKAGE), (path) => !path.endsWith(".md"));

  // not-overfit reads upstream's code from the pinned checkout, which the tool-pack checks show is the code that runs.
  const fromCheckout = (file: string) => join(upstream.directory, PACKAGE, relative(facts.package, file));
  // Python imports are case-sensitive: `from sqlfluff.core.parser.grammar import Sequence` names the class, not a
  // module grammar/Sequence.py, which a case-insensitive filesystem would find as grammar/sequence.py.
  const moduleFile = (module: string): string | undefined => {
    if (!module.startsWith("sqlfluff.")) return undefined;
    const base = join(upstream.directory, "src", ...module.split("."));
    return [`${base}.py`, join(base, "__init__.py")].find((path) => existsExactly(upstream.directory, path));
  };
  const ruleFiles = [...new Set(Object.values(facts.rules).map((rule) => fromCheckout(rule.file)))];
  const literals = new Map<string, { names: string[]; imports: string[] }>();
  const readLiterals = async (files: string[]) => {
    const fresh = [...new Set(files)].filter((file) => !literals.has(file));
    (await python.literals(fresh)).forEach((found, index) => literals.set(fresh[index]!, found));
  };
  await readLiterals(ruleFiles);
  /** The sqlfluff modules a file imports, a package's `__init__.py` with the modules it re-exports. */
  const importsOf = (file: string): string[] => (literals.get(file)?.imports ?? []).flatMap((module) => {
    const path = moduleFile(module);
    return path === undefined || path === file ? [] : [path];
  });
  await readLiterals(ruleFiles.flatMap(importsOf));
  const packageInits = (file: string) => importsOf(file).filter((path) => path.endsWith("__init__.py"));
  await readLiterals(ruleFiles.flatMap(packageInits).flatMap(importsOf));
  const regexesOf = (module: string) => (facts.regexes[module] ?? []).flatMap(({ pattern, flags }) => {
    const regex = pythonRegex(pattern, flags);
    return regex === undefined ? [] : [regex];
  });
  /** The members of each dialect set that code names by its label: the names that code checks against. */
  const setMembers = (names: string[]) => names.flatMap((label) => (Object.hasOwn(facts.dialectSets, label) ? facts.dialectSets[label]! : []));

  // Each case's SQL is upstream test text that is no file of its own: recorded inline, like a RuleTester case's code.
  const fixtureTexts = [...new Set(cases.texts)];
  const fixtureTextSources: FixtureTextSource[] = fixtureTexts.map((text) => ({ inline: text }));
  const fixtureWords = new Set([...fixtureTexts, ...cases.configWords].flatMap((text) => text.match(SQL_WORD) ?? []));
  // SQL keywords are case-insensitive: SQLFluff's sets are upper case, and a rule spells them in either case.
  const keywords = new Set(facts.keywords.flatMap((keyword) => [keyword, keyword.toLowerCase()]));
  const defaultSelected = new Set(facts.defaultSelected);
  const nameSet = new Set(codes);

  const rule = (name: string): UpstreamRule | undefined => {
    const code = name.toUpperCase();
    const fact = facts.rules[code];
    if (fact === undefined) return undefined;
    const on = defaultSelected.has(code) && cases.defaultOff[code] !== true;
    return {
      name: code,
      implementation: [relative(upstream.directory, fromCheckout(fact.file)).split(sep).join("/")],
      tests: cases.rules[code] === undefined ? [] : [`${CASES}/${code}.yml`],
      // SQLFluff has no deprecation flag; a removed rule leaves the registry (core/config/removed.py maps its config).
      deprecation: null,
      // PG01 and PG02 are selected by `rules = all` but hold themselves off with `force_enable = False` by default.
      recommended: on ? { config: "default", options: [] } : null,
      notOnRealCode: null,
    };
  };

  return {
    identity: {
      runtime: `sqlfluff ${facts.version}, CPython ${facts.python}, as \`dialect: postgres\`, \`templater: raw\` with the port's rule configuration (PG01 and PG02 \`force_enable = True\`, every other rule at its defaults)`,
      positions: "line and character column of each finding's start, as SQLFluff reports them (it reports no end)",
      parsers: `libpg_query (pglast, in ${pack.upstream.runtime}), on the SQL psql sends for the file: meta-commands, variables and COPY data read as psql reads them`,
    },
    packChecks: [
      ...upstream.checks,
      {
        id: "runtime-version",
        ok: facts.version === pack.upstream.version && runtime.version === `Python ${runtime.pinned}`,
        detail: `installed sqlfluff is ${facts.version} on ${runtime.version}; the pack pins ${pack.upstream.version} on Python ${runtime.pinned}`,
      },
      {
        id: "runtime-matches-checkout",
        ok: installedDigest === checkoutDigest,
        detail: `the installed sqlfluff package and ${PACKAGE}/ at the pinned commit (less its Markdown notes for contributors, which the wheel does not ship) have the same files and content`,
      },
      {
        id: "declared-license",
        ok: facts.license === pack.upstream.license,
        detail: `the installed package declares ${facts.license}; the pack records ${pack.upstream.license}`,
      },
    ],
    licenseText: upstreamLicenseText(upstream.directory),
    rule,

    implementationNames(name: string): Set<string> {
      const fact = facts.rules[name.toUpperCase()];
      if (fact === undefined) return new Set();
      const names = literals.get(fromCheckout(fact.file))?.names ?? [];
      return new Set([
        ...foldedNames([...names, ...setMembers(names)]),
        fact.name, ...fact.name.split("."), ...fact.groups, ...fact.aliases, ...fact.configKeywords,
        ...regexesOf(fact.module).flatMap((regex) => regex.source.match(/[A-Za-z_]\w*/gu) ?? []),
      ]);
    },
    sharedImplementationNames(name: string): Set<string> {
      const fact = facts.rules[name.toUpperCase()];
      if (fact === undefined) return new Set();
      const file = fromCheckout(fact.file);
      const shared = new Set([...importsOf(file), ...packageInits(file).flatMap(importsOf)]);
      const names = [...shared].filter((path) => path !== file).flatMap((path) => literals.get(path)?.names ?? []);
      return new Set(foldedNames([...names, ...setMembers(names)]));
    },
    implementationRegexes(name: string) {
      const fact = facts.rules[name.toUpperCase()];
      return fact === undefined ? [] : regexesOf(fact.module);
    },
    fixtureWords: () => fixtureWords,
    fixtureTexts: () => fixtureTexts,
    fixtureTextSources: () => fixtureTextSources,
    keywords,
    commentVocabulary: new Set(NOQA_VOCABULARY),
    inScopeSamples: ["src/example.sql"],

    cases(name: string): UpstreamCase[] {
      const code = name.toUpperCase();
      return (cases.rules[code] ?? []).map((entry) => {
        const own = (entry.findings ?? []).filter((finding) => finding.rule === code);
        const markup = !entry.markupApplies ? null
          : entry.expect === "pass" ? []
            : entry.violations === null ? null
              : entry.violations.map(([line, column]) => violationSpan(line, column));
        return {
          fixture: `${CASES}/${code}.yml#${entry.case}`,
          variant: entry.case,
          path: `${entry.dialect}/${code}/${entry.case}.sql`,
          source: entry.sql,
          skipped: entry.skipped,
          expected: entry.skipped === null ? own.map((finding) => violationSpan(finding.line, finding.column)) : [],
          markup,
        };
      });
    },

    toSpan(_name: string, source: string, startByte: number, endByte: number): Span {
      return sqlfluffSpan(source, startByte, endByte, SQLFLUFF_SPAN_PARTS);
    },

    // SQLFluff's default sql_file_exts also take .sql.j2, .dml and .ddl; the engine reads .sql alone.
    inCorpus: (path: string) => path.endsWith(".sql"),

    async lintCorpus(files, ruleNames) {
      const results = await ask<LintResult[]>({ op: "lint", texts: files.map((file) => file.source), codes });
      return files.map((file, index) => {
        const result = results[index]!;
        if ("excluded" in result) return { file, excluded: result.excluded, findings: new Map() };
        const found = result.findings.filter((finding) => nameSet.has(finding.rule));
        return {
          file,
          excluded: null,
          findings: new Map(ruleNames.map((name) => [name, found.filter((finding) => finding.rule === name.toUpperCase()).map((finding) => violationSpan(finding.line, finding.column))])),
        };
      });
    },

    parserVerdicts: (files) => ask({ op: "verdicts", texts: files.map((file) => file.source) }),
  };
}
