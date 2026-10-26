import { existsSync, readFileSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { availableParallelism } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { runChecked, type CommandRunner } from "../../../port-readiness/process.js";
import { upstreamLicenseText } from "../../../port-readiness/recorded-adapter.js";
import { psalmSpan, PSALM_SPAN_PARTS } from "../../../port-readiness/span-units/psalm.js";
import type { FixtureTextSource, PackAdapter, PackManifest, Span, UpstreamCase, UpstreamRule } from "../../../port-readiness/types.js";
import { createPhpToolkit, phpRegex, type PhpLiterals } from "../languages/php.js";
import { checkoutUpstream, composerRuntime, filesUnder, treeDigest, withoutMachinePaths } from "../runtimes.js";

/**
 * The Psalm adapter. Expected answers come from running the pinned Psalm itself, as its command line
 * (`vendor/bin/psalm --output-format=json`), on the cases of upstream's own tests at the pinned commit: each case of a
 * test class's providerValidCodeParse() and providerInvalidCodeParse() is a PHP source written inline, which upstream's
 * test harness (tests/TestCase.php, tests/Traits/*CodeAnalysisTestTrait.php) analyses alone as src/somefile.php, for
 * the PHP version the case names (7.4 by default, 8.0 for a `PHP80-` case, 8.1 for a valid `PHP81-` one), with the
 * issues it ignores suppressed. The answer is Psalm configured as upstream recommends: the psalm.xml `psalm --init`
 * writes (Config\Creator) for a project in which Psalm finds no issue, level 1 with unused code (and so unused
 * variables) found.
 *
 * The tests assert, by matching the message of the first error Psalm throws, an invalid case's issue type (sometimes
 * with its line and column), and that a valid case raises no error; they do not write every finding's start and end, so
 * no case has markup. The `upstream-tests-agree` check runs each case once more as the tests configure Psalm
 * (tests/TestConfig.php) and holds Psalm's output to those assertions.
 *
 * An issue's cases are the cases of every ported issue's own test class (tests/<Issue>Test.php), not its own alone,
 * as PHP_CodeSniffer's are: another issue's case is upstream's answer on code the rule must stay silent on. The rule's
 * own cases have variant `.`; another's name the test class that owns it.
 */

const DRIVER = fileURLToPath(new URL("./psalm_driver.php", import.meta.url));
/** Upstream's sources, the stubs and call maps its analysis reads, and its tests (checkoutUpstream adds its licence). */
const SPARSE_PATHS = ["/src/", "/stubs/", "/dictionaries/", "/tests/", "/composer.json"];
/** What the release carries of Psalm's analysis; `dictionaries/scripts` is left out of it (`export-ignore` in upstream's .gitattributes). */
const RELEASE_TREES = ["src", "stubs", "dictionaries"];
const RELEASE_IGNORED = /^dictionaries\/scripts\//u;
/** Issues after which Psalm's answer on a file is not its whole answer: it could not parse it, or analysed another declaration of its class instead. */
const UNANALYSED = new Set(["ParseError", "DuplicateClass"]);

interface ProviderCase {
  provider: "valid" | "invalid";
  name: string;
  code: string;
  error_message: string | null;
  ignored_issues: string[];
  php_version: string | null;
}
/** A test class as psalm_driver.php reads it: its cases, and the provider items (or whole providers) it cannot evaluate, with why. */
interface TestClass { path: string; unusedVariables: boolean; cases: ProviderCase[]; left: { provider: "valid" | "invalid"; item: string | null; reason: string }[] }
interface IssueFact { file: string; deprecation: string | null; recommended: boolean }
interface Facts {
  version: string;
  license: string | null;
  php: string;
  package: string;
  recommended: string;
  issues: Record<string, IssueFact>;
  tests: TestClass[];
}
/** One issue of Psalm's JSON report. */
interface Issue { type: string; message: string; file_name: string; line_from: number; line_to: number; column_from: number; from: number; to: number }

/** A Psalm configuration with the issues a case ignores suppressed, as the test harness sets them (Config::REPORT_SUPPRESS). */
function withIgnored(config: string, ignored: string[]): string {
  const bad = ignored.find((issue) => !/^[A-Z][A-Za-z0-9]*$/u.test(issue));
  if (bad !== undefined) throw new Error(`a case ignores ${JSON.stringify(bad)}, which is no Psalm issue type`);
  if (ignored.length === 0) return config;
  const handlers = `    <issueHandlers>\n${ignored.map((issue) => `        <${issue} errorLevel="suppress" />\n`).join("")}    </issueHandlers>\n`;
  return config.replace(/<\/psalm>\s*$/u, `${handlers}</psalm>\n`);
}

/**
 * The configuration upstream's tests run Psalm with (tests/TestConfig.php), as psalm.xml: level 1, docblock types read,
 * internal functions' false and null returns ignored, no override attribute required, loose binary operands, no unused
 * code, baseline entries or suppressions sought; unused variables found where the test class's setUp asks for them
 * (`reportUnusedVariables()`, as UnusedVariableTest does). A case's run suppresses the issues it ignores and HARNESS_UNREPORTED.
 */
function testedConfig(unusedVariables: boolean): string {
  return `<?xml version="1.0"?>
<psalm errorLevel="1" useDocblockTypes="true" ignoreInternalFunctionFalseReturn="true" ignoreInternalFunctionNullReturn="true" ensureOverrideAttribute="false" strictBinaryOperands="false" findUnusedCode="false" findUnusedVariablesAndParams="${unusedVariables}" findUnusedBaselineEntry="false" findUnusedPsalmSuppress="false">
    <projectFiles>
        <directory name="src" />
    </projectFiles>
</psalm>
`;
}

/**
 * The issues Psalm's command line raises at TestConfig's settings that the test harness never does: TestCase::analyzeFile
 * analyses the one file (FileAnalyzer) and never runs the project-wide pass after it (ClassLikes::consolidateAnalyzedData),
 * whose only issue without unused code sought is ClassMustBeFinal. Run as the tests run, they are suppressed.
 */
const HARNESS_UNREPORTED = ["ClassMustBeFinal"];

/**
 * The PHP version upstream's test harness analyses a case for: the case's own, else 8.0 for a test named `PHP80-`, 8.1
 * for a valid one named `PHP81-`, else 7.4 (testValidCode, testInvalidCode).
 */
function phpVersionOf(testCase: ProviderCase): string {
  if (testCase.php_version !== null) return testCase.php_version;
  if (testCase.name.includes("PHP80-")) return "8.0";
  if (testCase.name.includes("PHP81-") && testCase.provider === "valid") return "8.1";
  return "7.4";
}

/** Whether upstream's test harness skips a case: a `SKIPPED-` name, unless a version prefix it checks first claims the name. */
function skippedByTest(testCase: ProviderCase): boolean {
  if (testCase.name.includes("PHP80-") || (testCase.name.includes("PHP81-") && testCase.provider === "valid")) return false;
  return testCase.name.includes("SKIPPED-");
}

/**
 * Whether an invalid case's `error_message` matches the error Psalm throws for `issue`, as testInvalidCode matches it
 * (`/\b<error_message>\b/` on the CodeException message: the type, the location, then the message, to which a Mixed*
 * issue's exception appends where the mixed value came from).
 */
function asserted(issue: Issue, errorMessage: string): boolean {
  const thrown = `${issue.type} - src/somefile.php:${issue.line_from}:${issue.column_from} - ${issue.message}`;
  const pattern = new RegExp(`\\b${errorMessage.replace(/[.*+?^${}()|[\]\\/]/gu, "\\$&")}\\b`, "u");
  return pattern.test(thrown) || (issue.type.startsWith("Mixed") && errorMessage.includes(thrown));
}

/** Run `work` over `items` with at most `limit` at a time, keeping their order. */
async function pooled<T, R>(items: T[], limit: number, work: (item: T) => Promise<R>): Promise<R[]> {
  const results: R[] = new Array<R>(items.length);
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, async () => {
    for (let index = next++; index < items.length; index = next++) results[index] = await work(items[index]!);
  }));
  return results;
}

export async function createPsalmAdapter(pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner): Promise<PackAdapter> {
  // Psalm publishes its Composer package from the repository at the release tag, which is the bare version.
  const upstream = await checkoutUpstream(cacheRoot, pack.upstream, pack.upstream.version, "src", SPARSE_PATHS, runner);
  const runtime = await composerRuntime(pack.upstream.runtime, cacheRoot, runner);
  const php = await createPhpToolkit(cacheRoot, runner);
  const fromCheckout = (path: string) => relative(upstream.directory, path).split(sep).join("/");
  const psalm = runtime.vendorBin("psalm");
  await mkdir(resolve(cacheRoot, "tmp"), { recursive: true });

  const facts = await (async () => {
    const scratch = await mkdtemp(resolve(cacheRoot, "tmp", "psalm-init-"));
    try {
      return JSON.parse(await runChecked(runner, runtime.command(DRIVER, runtime.directory), {
        input: JSON.stringify({ op: "facts", issues: names, checkout: upstream.directory, scratch }),
        cwd: cacheRoot,
      })) as Facts;
    } finally {
      await rm(scratch, { recursive: true, force: true });
    }
  })();

  /** Psalm's report on a project of `files` (relative to its src/) configured by `config`, in a temporary directory. */
  const analyse = async (config: string, files: { path: string; source: string }[], phpVersion: string | null): Promise<Issue[]> => {
    const directory = await mkdtemp(resolve(cacheRoot, "tmp", "psalm-"));
    try {
      await Promise.all(files.map(async (file) => {
        await mkdir(dirname(join(directory, "src", file.path)), { recursive: true });
        await writeFile(join(directory, "src", file.path), file.source);
      }));
      await writeFile(join(directory, "psalm.xml"), config);
      const argv = runtime.command(psalm, `--config=${join(directory, "psalm.xml")}`, `--root=${directory}`, "--no-cache", "--output-format=json", "--no-progress", "--monochrome", "--threads=1", ...(phpVersion === null ? [] : [`--php-version=${phpVersion}`]));
      const result = await runner.run(argv, { cwd: directory });
      const machine = [directory, upstream.directory, runtime.directory, cacheRoot];
      // Psalm exits 2 when it reports an error and 0 when it reports none; anything else is a failure to run.
      if (result.exitCode !== 0 && result.exitCode !== 2) {
        throw new Error(`psalm exited ${result.exitCode}:\n${withoutMachinePaths((result.stderr.trim() || result.stdout.trim()).split("\n").slice(-20).join("\n"), machine)}`);
      }
      // This machine's directories are no part of the answer: the recording must read the same on every machine.
      return (JSON.parse(result.stdout) as Issue[]).map((issue) => ({ ...issue, message: withoutMachinePaths(issue.message, machine), file_name: issue.file_name.split(sep).join("/") }));
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  };

  const ported = names.filter((name) => facts.issues[name] !== undefined).sort();
  const testOf = new Map(ported.flatMap((name) => {
    const test = facts.tests.find((candidate) => candidate.path === `tests/${name}Test.php`);
    return test === undefined ? [] : [[name, test] as const];
  }));
  const owned = [...new Set(testOf.values())].sort((a, b) => (a.path < b.path ? -1 : 1));
  const caseList = owned.flatMap((test) => test.cases.map((testCase) => ({ test, testCase, fixture: `${test.path}#${testCase.provider}/${testCase.name}` })));
  // What the reading leaves out of the ported issues' tests, for which no case is recorded, and of the rest of upstream's
  // tests, whose cases not-overfit reads.
  const unread = owned.flatMap((test) => test.left.map((entry) => `${test.path}#${entry.provider}${entry.item === null ? "" : `/${entry.item}`} (${entry.reason})`));
  const unreadElsewhere = facts.tests.filter((test) => !owned.includes(test)).reduce((count, test) => count + test.left.length, 0);

  // Every case once as the tests configure Psalm and once as upstream recommends.
  const runs = await pooled(caseList, availableParallelism(), async ({ test, testCase }) => {
    const files = [{ path: "somefile.php", source: testCase.code }];
    return {
      tested: await analyse(withIgnored(testedConfig(test.unusedVariables), [...new Set([...testCase.ignored_issues, ...HARNESS_UNREPORTED])]), files, phpVersionOf(testCase)),
      recommended: await analyse(withIgnored(facts.recommended, testCase.ignored_issues), files, phpVersionOf(testCase)),
    };
  });

  const disagreements: string[] = [];
  const misplaced: string[] = [];
  for (const [index, { test, testCase, fixture }] of caseList.entries()) {
    const { tested, recommended } = runs[index]!;
    for (const issue of [...tested, ...recommended]) {
      const span = psalmSpan(testCase.code, issue.from, issue.to);
      if (span.startLine !== issue.line_from - 1 || span.endLine !== issue.line_to - 1) misplaced.push(`${fixture}: ${issue.type} at lines ${issue.line_from}-${issue.line_to}, offsets ${issue.from}-${issue.to}`);
    }
    if (skippedByTest(testCase)) continue;
    if (testCase.provider === "invalid") {
      if (testCase.error_message === null || !tested.some((issue) => asserted(issue, testCase.error_message!))) {
        disagreements.push(`${fixture}: ${test.path} expects ${JSON.stringify(testCase.error_message)}, Psalm reports ${tested.map((issue) => `${issue.type} ${issue.line_from}:${issue.column_from}`).join(", ") || "nothing"}`);
      }
    } else if (tested.length > 0) {
      disagreements.push(`${fixture}: valid, Psalm reports ${tested.map((issue) => `${issue.type} ${issue.line_from}:${issue.column_from}`).join(", ")}`);
    }
  }

  // An issue the recommended configuration turns on: reported as an error at its level, and reported on its own test's
  // cases (an issue of an opt-in feature, such as UnusedVariable, is only reported when the configuration turns the
  // feature on).
  const turnedOn = new Set(ported.filter((name) => facts.issues[name]!.recommended
    && caseList.some(({ test }, index) => testOf.get(name) === test && runs[index]!.recommended.some((issue) => issue.type === name))));

  // Every case of every test class upstream ships: the test words and texts not-overfit reads.
  const fixtureTexts = [...new Set(facts.tests.flatMap((test) => test.cases.map((testCase) => testCase.code)))];
  const fixtureTextSources: FixtureTextSource[] = fixtureTexts.map((text) => ({ inline: text }));
  const fixtureWords = new Set([
    ...(await php.words(fixtureTexts)).flat(),
    // The issues a case ignores are its test configuration's option values.
    ...facts.tests.flatMap((test) => test.cases.flatMap((testCase) => testCase.ignored_issues)),
  ]);

  // An issue's implementation: its class, and every file of upstream's src/ that raises it.
  const sources = filesUnder(join(upstream.directory, "src"), (path) => path.endsWith(".php"));
  const implementationOf = new Map(ported.map((name) => {
    const raises = new RegExp(`\\bnew\\s+(?:\\\\?Psalm\\\\Issue\\\\)?${name}\\s*\\(`, "u");
    const own = join(upstream.directory, relative(facts.package, facts.issues[name]!.file));
    return [name, [...new Set([own, ...sources.filter((path) => raises.test(readFileSync(path, "utf8")))])].sort()] as const;
  }));
  const literalsOf = new Map<string, PhpLiterals>();
  const readLiterals = async (files: string[]) => {
    const fresh = [...new Set(files)].filter((file) => !literalsOf.has(file));
    for (const [index, found] of (await php.literals(fresh)).entries()) literalsOf.set(fresh[index]!, found);
  };
  await readLiterals([...implementationOf.values()].flat());
  /** The checkout file of a Psalm class (`Psalm\Issue\CodeIssue` is src/Psalm/Issue/CodeIssue.php), when it is one. */
  const classFile = (name: string) => (name.startsWith("Psalm\\") ? join(upstream.directory, "src", `${name.replaceAll("\\", "/")}.php`) : null);
  // An issue's helpers: the classes its implementation's classes extend, and theirs in turn, within upstream's src/.
  // Imports are not followed: Psalm's analyzers import most of Psalm, which would explain any name Psalm mentions.
  const helpers = new Map<string, string[]>();
  for (const name of ported) {
    const seen = new Set<string>(implementationOf.get(name));
    for (let frontier = implementationOf.get(name)!; frontier.length > 0;) {
      const next = [...new Set(frontier.flatMap((file) => (literalsOf.get(file)?.classes ?? []).flatMap((declared) => (declared.extends === null ? [] : [declared.extends]))).map(classFile))]
        .filter((path): path is string => path !== null && !seen.has(path) && existsSync(path));
      for (const path of next) seen.add(path);
      await readLiterals(next);
      frontier = next;
    }
    for (const file of implementationOf.get(name)!) seen.delete(file);
    helpers.set(name, [...seen].sort());
  }
  const regexesOf = (file: string) => (literalsOf.get(file)?.regexes ?? []).flatMap((pattern) => {
    const regex = phpRegex(pattern);
    return regex === undefined ? [] : [regex];
  });

  const recommendedConfig = pack.upstream.recommended_configs[0]!;
  const rule = (name: string): UpstreamRule | undefined => {
    const fact = facts.issues[name];
    if (fact === undefined) return undefined;
    const test = testOf.get(name);
    return {
      name,
      implementation: implementationOf.get(name)!.map(fromCheckout),
      tests: test === undefined ? [] : [test.path],
      deprecation: fact.deprecation,
      recommended: turnedOn.has(name) ? { config: recommendedConfig, options: [] } : null,
      notOnRealCode: null,
    };
  };

  const findingsOf = (issues: Issue[], source: string, name: string): Span[] =>
    issues.filter((issue) => issue.type === name).map((issue) => psalmSpan(source, issue.from, issue.to));

  return {
    identity: {
      runtime: `Psalm ${facts.version} on PHP ${facts.php}, configured by the psalm.xml \`psalm --init\` writes for a project in which it finds no issue (level 1, findUnusedCode); each upstream test case analysed alone as src/somefile.php, for the PHP version its test names, with the issues it ignores suppressed`,
      positions: "Psalm's selection, from its byte offsets (`from`, `to`): start and end, 0-based lines as PHP's tokenizer breaks them, columns in UTF-8 bytes",
      parsers: `${php.version} (php -l) and nikic/php-parser ${php.parser}`,
    },
    packChecks: [
      ...upstream.checks,
      {
        id: "runtime-version",
        ok: facts.version === pack.upstream.version && runtime.version === runtime.pinned,
        detail: `installed Psalm is ${facts.version} on PHP ${runtime.version}; the pack pins ${pack.upstream.version} on PHP ${runtime.pinned}`,
      },
      {
        id: "runtime-matches-checkout",
        ok: RELEASE_TREES.every((tree) => treeDigest(join(facts.package, tree)) === treeDigest(join(upstream.directory, tree), (path) => !RELEASE_IGNORED.test(fromCheckout(path)))),
        detail: `the installed package's ${RELEASE_TREES.join("/, ")}/ and the same trees at the pinned commit have the same files and content, less dictionaries/scripts/, which the release leaves out`,
      },
      {
        id: "declared-license",
        ok: facts.license === pack.upstream.license,
        detail: `the installed package declares ${facts.license ?? "no license"}; the pack records ${pack.upstream.license}`,
      },
      {
        id: "positions-from-offsets",
        ok: misplaced.length === 0,
        detail: misplaced.length === 0
          ? "every issue Psalm reports on the cases starts and ends on the lines its byte offsets fall on, so the offsets are its position"
          : `${misplaced.length} issue(s) report lines their byte offsets do not fall on: ${misplaced.slice(0, 3).join("; ")}`,
      },
      {
        id: "upstream-tests-agree",
        ok: disagreements.length === 0,
        detail: disagreements.length === 0
          ? `configured as its tests configure it, Psalm raises each invalid case's expected error and no error on a valid case (${caseList.filter(({ testCase }) => !skippedByTest(testCase)).length} case(s); \`SKIPPED-\` cases, which the tests skip as known bugs, left out)`
          : `${disagreements.length} case(s) differ from upstream's tests: ${disagreements.slice(0, 3).join("; ")}`,
      },
      {
        // Informational, so it never fails a rule: the report shows what the reading leaves out, and a re-record whose
        // reading shrank changes the count.
        id: "upstream-tests-read",
        ok: true,
        detail: `${caseList.length} case(s) of the ported issues' tests read statically; ${unread.length} provider item(s) cannot be evaluated, and no case is recorded for them${unread.length === 0 ? "" : `: ${unread.slice(0, 5).join("; ")}${unread.length > 5 ? "; …" : ""}`}; ${unreadElsewhere} more in upstream's other tests`,
      },
    ],
    licenseText: upstreamLicenseText(upstream.directory),
    rule,

    implementationNames(name: string): Set<string> {
      const files = implementationOf.get(name);
      if (files === undefined) return new Set();
      return new Set(files.flatMap((file) => [
        ...(literalsOf.get(file)?.names ?? []),
        ...regexesOf(file).flatMap((regex) => regex.source.match(/[A-Za-z_]\w*/gu) ?? []),
      ]));
    },
    sharedImplementationNames(name: string): Set<string> {
      return new Set((helpers.get(name) ?? []).flatMap((path) => literalsOf.get(path)?.names ?? []));
    },
    implementationRegexes(name: string) {
      return [...(implementationOf.get(name) ?? []), ...(helpers.get(name) ?? [])].flatMap(regexesOf);
    },
    fixtureWords: () => fixtureWords,
    fixtureTexts: () => fixtureTexts,
    fixtureTextSources: () => fixtureTextSources,
    keywords: php.keywords,
    commentVocabulary: php.commentVocabulary,
    // Psalm analyses .php files unless a config adds extensions (Config's file_extensions).
    inScopeSamples: ["src/example.php"],

    cases(name: string): UpstreamCase[] {
      if (facts.issues[name] === undefined) return [];
      return caseList.map(({ test, testCase, fixture }, index) => {
        const { tested, recommended } = runs[index]!;
        const answer = turnedOn.has(name) ? recommended : tested;
        const unanalysed = answer.find((issue) => UNANALYSED.has(issue.type));
        const skipped = skippedByTest(testCase)
          ? `upstream's test ${test.path} skips it as a known Psalm bug (SKIPPED-)`
          : unanalysed === undefined ? null : `upstream could not analyse it: ${unanalysed.type}: ${unanalysed.message}`;
        return {
          fixture,
          variant: testOf.get(name) === test ? "." : test.path,
          path: `${test.path.replace(/^tests\//u, "").replace(/\.php$/u, "")}/${testCase.provider}/${testCase.name}.php`,
          source: testCase.code,
          skipped,
          expected: skipped === null ? findingsOf(answer, testCase.code, name) : [],
          // The tests assert the first error's message, not every finding's start and end; upstream-tests-agree checks them.
          markup: null,
        };
      });
    },

    toSpan(_name: string, source: string, startByte: number, endByte: number): Span {
      return psalmSpan(source, startByte, endByte, PSALM_SPAN_PARTS);
    },

    parserVerdicts: (files) => php.verdicts(files),
  };
}
