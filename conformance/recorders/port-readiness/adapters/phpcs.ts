import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { runChecked, type CommandRunner } from "../../../port-readiness/process.js";
import { upstreamLicenseText } from "../../../port-readiness/recorded-adapter.js";
import { messageSpan, phpcsSpan, PHPCS_SPAN_PARTS } from "../../../port-readiness/span-units/phpcs.js";
import type { FixtureTextSource, PackAdapter, PackManifest, Span, UpstreamCase, UpstreamRule } from "../../../port-readiness/types.js";
import { createPhpToolkit, phpRegex, type PhpLiterals } from "../languages/php.js";
import { checkoutUpstream, composerRuntime, filesUnder, treeDigest } from "../runtimes.js";

/**
 * The PHP_CodeSniffer adapter. Expected answers come from running the pinned PHP_CodeSniffer itself on upstream's own
 * unit test files at the pinned commit, each sniff as upstream's test harness runs it (tests/Standards/
 * AbstractSniffTestCase.php: a Ruleset of the sniff alone with its default properties, a LocalFile processed, the unit
 * test's setCliValues applied), or, for a sniff a recommended standard turns on, with that standard's configuration of
 * it (`--standard=<standard> --sniffs=<sniff>`). Upstream's unit tests assert error and warning counts per line
 * (getErrorList, getWarningList), not columns, so PHP_CodeSniffer's output is the positional answer, and the
 * `upstream-tests-agree` check holds that output to the tests' counts.
 *
 * A sniff's cases are the unit test files of every sniff the pack ports, not its own alone: PHP_CodeSniffer's test
 * files are PHP files any sniff reads, so another sniff's file is upstream's answer on code the rule must stay silent
 * on, as Bandit's examples are for every Bandit test. The rule's own files have variant `.`; another's name the unit
 * test that owns it.
 */

const DRIVER = fileURLToPath(new URL("./phpcs_driver.php", import.meta.url));
/** Upstream's sources and its unit tests, under src/Standards/<Standard>/Tests/ (checkoutUpstream adds its licence). */
const SPARSE_PATHS = ["/src/", "/composer.json"];
/** A unit test's directory: release archives leave it out (`export-ignore` in upstream's .gitattributes). */
const TESTS_DIRECTORY = /(?:^|\/)Tests\//u;
/** Files in a Tests directory that are no test file of a sniff: the test classes, and the fixer's expected output (AbstractSniffTestCase's ignoreExtensions). */
const NOT_TEST_FILES = /\.(?:php|fixed|bak|orig)$/u;

interface Message { source: string; line: number; column: number; type: "error" | "warning" }
interface LintResult { messages: Record<string, Message[]>; internal: Record<string, string> }
interface SniffFact {
  class: string;
  file: string;
  deprecation: string | null;
  testClass: string | null;
  skipped: boolean;
  tests: { path: string; errors: Record<string, number>; warnings: Record<string, number> }[];
  names: string[];
}
interface StandardSetting { properties: Record<string, unknown>; excluded: string[] }
interface Facts {
  version: string;
  license: string | null;
  php: string;
  iconv: boolean;
  package: string;
  sniffs: Record<string, SniffFact>;
  standards: Record<string, Record<string, StandardSetting>>;
}

/** A message's sniff: its source without the message code (`Generic.PHP.Eval.Discouraged` is `Generic.PHP.Eval`). */
const sniffOf = (source: string): string => source.split(".").slice(0, 3).join(".");

export async function createPhpcsAdapter(pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner): Promise<PackAdapter> {
  // PHP_CodeSniffer publishes its Composer package from the repository at the release tag, which is the bare version.
  const upstream = await checkoutUpstream(cacheRoot, pack.upstream, pack.upstream.version, "src", SPARSE_PATHS, runner);
  const runtime = await composerRuntime(pack.upstream.runtime, cacheRoot, runner);
  const php = await createPhpToolkit(cacheRoot, runner);
  const ask = async <T>(request: object): Promise<T> =>
    JSON.parse(await runChecked(runner, runtime.command(DRIVER, runtime.directory), { input: JSON.stringify(request), cwd: cacheRoot })) as T;
  const facts = await ask<Facts>({ op: "facts", sniffs: names, standards: pack.upstream.recommended_configs, checkout: upstream.directory });
  const fromCheckout = (path: string) => relative(upstream.directory, path).split(sep).join("/");
  const sourceOf = (installed: string) => join(upstream.directory, relative(facts.package, installed));

  // Every unit test file of every sniff upstream ships: the test words and texts not-overfit reads.
  const testFiles = filesUnder(join(upstream.directory, "src", "Standards"), (path) => TESTS_DIRECTORY.test(fromCheckout(path)) && !NOT_TEST_FILES.test(path));
  const testText = new Map(testFiles.map((path) => [path, readFileSync(path, "utf8")]));
  const fixtureTexts = [...testText.values()];
  const fixtureTextSources: FixtureTextSource[] = [...testText].map(([path, text]) => ({ path: fromCheckout(path), sha256: createHash("sha256").update(text).digest("hex") }));
  const fixtureWords = new Set((await php.words(fixtureTexts)).flat());

  // The recommended standard that turns each sniff on, first in gates.yaml's order, and the groups each sniff runs in.
  const ported = Object.keys(facts.sniffs).sort();
  const recommendedBy = new Map(ported.flatMap((code) => {
    const standard = pack.upstream.recommended_configs.find((config) => facts.standards[config]?.[code] !== undefined);
    return standard === undefined ? [] : [[code, standard] as const];
  }));
  const groups = [
    ...pack.upstream.recommended_configs.map((standard) => ({ standard, sniffs: ported.filter((code) => recommendedBy.get(code) === standard) })),
    { standard: null, sniffs: ported.filter((code) => !recommendedBy.has(code)) },
  ].filter((group) => group.sniffs.length > 0);

  // Upstream's own unit tests, linted once as the harness runs them (every sniff alone) and once as the cases are.
  const owner = new Map(ported.flatMap((code) => facts.sniffs[code]!.tests.map((test) => [test.path, code] as const)));
  const caseFiles = [...owner.keys()].sort();
  const lintFiles = caseFiles.map((path) => ({ path, test: facts.sniffs[owner.get(path)!]!.testClass ?? undefined }));
  const asTested = await ask<LintResult>({ op: "lint", groups: [{ standard: null, sniffs: ported }], files: lintFiles });
  const asRecommended = groups.some((group) => group.standard !== null) ? await ask<LintResult>({ op: "lint", groups, files: lintFiles }) : asTested;

  const disagreements: string[] = [];
  for (const code of ported) {
    const fact = facts.sniffs[code]!;
    if (fact.skipped) continue;
    for (const test of fact.tests) {
      const found = (asTested.messages[test.path] ?? []).filter((message) => sniffOf(message.source) === code);
      for (const type of ["error", "warning"] as const) {
        const expected = type === "error" ? test.errors : test.warnings;
        const counts = new Map<number, number>();
        for (const message of found) if (message.type === type) counts.set(message.line, (counts.get(message.line) ?? 0) + 1);
        const lines = [...new Set([...Object.keys(expected).map(Number), ...counts.keys()])].sort((a, b) => a - b);
        const differing = lines.find((line) => (expected[String(line)] ?? 0) !== (counts.get(line) ?? 0));
        if (differing !== undefined) disagreements.push(`${fromCheckout(test.path)} line ${differing}: ${code}'s unit test expects ${expected[String(differing)] ?? 0} ${type}(s), PHP_CodeSniffer reports ${counts.get(differing) ?? 0}`);
      }
    }
  }

  // not-overfit reads upstream's code from the pinned checkout, which the tool-pack checks show is the code that runs.
  const sniffFile = (code: string) => sourceOf(facts.sniffs[code]!.file);
  const literalsOf = new Map<string, PhpLiterals>();
  const readLiterals = async (files: string[]) => {
    const fresh = [...new Set(files)].filter((file) => !literalsOf.has(file));
    for (const [index, found] of (await php.literals(fresh)).entries()) literalsOf.set(fresh[index]!, found);
  };
  /** The checkout file of a PHP_CodeSniffer class (`PHP_CodeSniffer\Util\Tokens` is src/Util/Tokens.php), when it is one. */
  const classFile = (name: string) => (name.startsWith("PHP_CodeSniffer\\") ? join(upstream.directory, "src", `${name.slice("PHP_CodeSniffer\\".length).replaceAll("\\", "/")}.php`) : null);
  // A sniff's helpers: the classes its file imports and the classes it extends, and theirs in turn, within upstream's src/.
  const helpers = new Map<string, string[]>();
  await readLiterals(ported.map(sniffFile));
  for (const code of ported) {
    const seen = new Set<string>([sniffFile(code)]);
    for (let frontier = [sniffFile(code)]; frontier.length > 0;) {
      const next = [...new Set(frontier.flatMap((file) => {
        const found = literalsOf.get(file);
        return [...(found?.imports ?? []), ...(found?.classes ?? []).flatMap((declared) => (declared.extends === null ? [] : [declared.extends]))];
      }).map(classFile))].filter((path): path is string => path !== null && !seen.has(path) && existsSync(path));
      for (const path of next) seen.add(path);
      await readLiterals(next);
      frontier = next;
    }
    seen.delete(sniffFile(code));
    helpers.set(code, [...seen].sort());
  }
  const regexesOf = (file: string) => (literalsOf.get(file)?.regexes ?? []).flatMap((pattern) => {
    const regex = phpRegex(pattern);
    return regex === undefined ? [] : [regex];
  });

  const rule = (name: string): UpstreamRule | undefined => {
    const fact = facts.sniffs[name];
    if (fact === undefined) return undefined;
    const standard = recommendedBy.get(name);
    const setting = standard === undefined ? undefined : facts.standards[standard]![name]!;
    const configured = setting !== undefined && (Object.keys(setting.properties).length > 0 || setting.excluded.length > 0);
    return {
      name,
      implementation: [fromCheckout(sniffFile(name))],
      tests: fact.tests.map((test) => fromCheckout(test.path)),
      deprecation: fact.deprecation,
      recommended: standard === undefined ? null : { config: standard, options: configured ? [setting] : [] },
      notOnRealCode: null,
    };
  };

  const findingsOf = (result: LintResult, path: string, code: string): Span[] =>
    (result.messages[path] ?? []).filter((message) => sniffOf(message.source) === code).map(messageSpan);

  return {
    identity: {
      runtime: `PHP_CodeSniffer ${facts.version} on PHP ${facts.php}, each sniff as upstream's unit tests run it (alone, default properties, the test's CLI values), or with the configuration of the first recommended standard that turns it on; errors and warnings alike, \`phpcs:\` directives honoured`,
      positions: "line and column as PHP_CodeSniffer reports them: 1-based, columns in characters (iconv_strlen) with a tab width of 0, and no end",
      parsers: `${php.version} (php -l) and nikic/php-parser ${php.parser}`,
    },
    packChecks: [
      ...upstream.checks,
      {
        id: "runtime-version",
        ok: facts.version === pack.upstream.version && runtime.version === runtime.pinned,
        detail: `installed PHP_CodeSniffer is ${facts.version} on PHP ${runtime.version}; the pack pins ${pack.upstream.version} on PHP ${runtime.pinned}`,
      },
      {
        id: "runtime-matches-checkout",
        ok: treeDigest(join(facts.package, "src"), (path) => !TESTS_DIRECTORY.test(path)) === treeDigest(join(upstream.directory, "src"), (path) => !TESTS_DIRECTORY.test(relative(join(upstream.directory, "src"), path))),
        detail: "the installed package's src/ and src/ at the pinned commit have the same files and content, less the unit tests the release leaves out",
      },
      {
        id: "declared-license",
        ok: facts.license === pack.upstream.license,
        detail: `the installed package declares ${facts.license ?? "no license"}; the pack records ${pack.upstream.license}`,
      },
      {
        // Without iconv PHP_CodeSniffer counts a token's length in bytes, so every column after a multibyte character moves.
        id: "columns-in-characters",
        ok: facts.iconv,
        detail: facts.iconv ? "the interpreter has iconv, so PHP_CodeSniffer counts columns in characters" : "the interpreter lacks iconv: PHP_CodeSniffer would count columns in bytes",
      },
      {
        id: "upstream-tests-agree",
        ok: disagreements.length === 0,
        detail: disagreements.length === 0
          ? `on its own unit test files, every ported sniff reports exactly the errors and warnings per line its unit test expects (${caseFiles.length} file(s))`
          : `${disagreements.length} line(s) differ from upstream's unit tests: ${disagreements.slice(0, 3).join("; ")}`,
      },
    ],
    licenseText: upstreamLicenseText(upstream.directory),
    rule,

    implementationNames(name: string): Set<string> {
      const fact = facts.sniffs[name];
      if (fact === undefined) return new Set();
      const file = sniffFile(name);
      return new Set([
        ...(literalsOf.get(file)?.names ?? []),
        // What the instantiated sniff holds: lists it builds at run time (DeprecatedFunctions asks the interpreter which
        // functions it deprecates) are read off the object, as Bandit's driver reads the patterns its plugins compile.
        ...fact.names.flatMap((value) => value.match(/[A-Za-z_]\w*/gu) ?? []),
        ...regexesOf(file).flatMap((regex) => regex.source.match(/[A-Za-z_]\w*/gu) ?? []),
      ]);
    },
    sharedImplementationNames(name: string): Set<string> {
      return new Set((helpers.get(name) ?? []).flatMap((path) => literalsOf.get(path)?.names ?? []));
    },
    implementationRegexes(name: string) {
      if (facts.sniffs[name] === undefined) return [];
      return [sniffFile(name), ...(helpers.get(name) ?? [])].flatMap(regexesOf);
    },
    fixtureWords: () => fixtureWords,
    fixtureTexts: () => fixtureTexts,
    fixtureTextSources: () => fixtureTextSources,
    keywords: php.keywords,
    commentVocabulary: php.commentVocabulary,
    // PHP_CodeSniffer checks .php and .inc files by default (Config's extensions).
    inScopeSamples: ["src/example.php", "src/example.inc"],

    cases(name: string): UpstreamCase[] {
      if (facts.sniffs[name] === undefined) return [];
      return caseFiles.map((path) => {
        const fixture = fromCheckout(path);
        const source = readFileSync(path, "utf8");
        const own = owner.get(path)!;
        const internal = asRecommended.internal[path] ?? asTested.internal[path];
        const skipped = facts.sniffs[own]!.skipped
          ? `upstream's unit test ${fromCheckout(facts.sniffs[own]!.testClass!)} skips it on PHP ${facts.php}`
          : internal === undefined ? null : `upstream could not analyse it: ${internal.replaceAll(`${upstream.directory}/`, "")}`;
        return {
          fixture,
          variant: own === name ? "." : fromCheckout(facts.sniffs[own]!.testClass ?? path),
          // PHP_CodeSniffer reads a unit test file (`.inc`) as PHP, which Rerail routes by the `.php` suffix.
          path: `${fixture}.php`,
          source,
          skipped,
          expected: skipped === null ? findingsOf(recommendedBy.has(name) ? asRecommended : asTested, path, name) : [],
          // The unit tests' expectations are counts per line, which upstream-tests-agree checks; no position to compare.
          markup: null,
        };
      });
    },

    toSpan(_name: string, source: string, startByte: number, endByte: number): Span {
      return phpcsSpan(source, startByte, endByte, PHPCS_SPAN_PARTS);
    },

    // PHP_CodeSniffer checks .php and .inc files; a Blade template (`.blade.php`) is the blade pack's, as the php pack's match excludes it.
    inCorpus: (path: string) => (path.endsWith(".php") && !path.endsWith(".blade.php")) || path.endsWith(".inc"),

    async lintCorpus(files, ruleNames) {
      await mkdir(resolve(cacheRoot, "tmp"), { recursive: true });
      const directory = await mkdtemp(resolve(cacheRoot, "tmp", "phpcs-corpus-"));
      try {
        const paths = files.map((file) => join(directory, file.task, file.path));
        await Promise.all(files.map(async (file, index) => {
          await mkdir(dirname(paths[index]!), { recursive: true });
          await writeFile(paths[index]!, file.source);
        }));
        const result = files.length === 0 ? { messages: {}, internal: {} } : await ask<LintResult>({ op: "lint", groups, files: paths.map((path) => ({ path })) });
        return files.map((file, index) => {
          const internal = result.internal[paths[index]!];
          // The temporary directory is no part of the answer: the recording must read the same on every machine.
          if (internal !== undefined) return { file, excluded: `upstream could not analyse it: ${internal.replaceAll(`${directory}/`, "")}`, findings: new Map() };
          return { file, excluded: null, findings: new Map(ruleNames.map((name) => [name, findingsOf(result, paths[index]!, name)])) };
        });
      } finally {
        await rm(directory, { recursive: true, force: true });
      }
    },

    parserVerdicts: (files) => php.verdicts(files),
  };
}
