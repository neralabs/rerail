import { createHash } from "node:crypto";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { availableParallelism } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { runChecked, type CommandRunner } from "../../../port-readiness/process.js";
import { upstreamLicenseText } from "../../../port-readiness/upstream-files.js";
import { errorSpan, phpstanSpan, PHPSTAN_SPAN_PARTS } from "../../../port-readiness/span-units/phpstan.js";
import type { FixtureTextSource, PackAdapter, PackManifest, Span, UpstreamCase, UpstreamRule } from "../../../port-readiness/types.js";
import { createPhpToolkit, phpRegex, type PhpLiterals } from "../languages/php.js";
import { checkoutUpstream, composerRuntime, filesUnder, PHP_INI, withoutMachinePaths } from "../runtimes.js";

/**
 * The PHPStan adapter. Expected answers come from running the pinned PHPStan release (phpstan/phpstan's phar, built from
 * phpstan-src at the pinned commit) on upstream's own rule test data, as upstream's RuleTestCase analyses it: each test
 * method's `$this->analyse([files], …)` files analysed together and nothing else, the files the method requires before
 * it (definitions of the functions and classes they call) given to PHPStan to discover symbols in, read statically from
 * the test (phpstan_driver.php `tests`). PHPStan runs from its command line at a rule level
 * (conf/config.level<N>.neon): the first recommended level whose container registers the rule's class, or, for a rule no
 * recommended level turns on, the lowest level that does, with every option at that level's value. A finding is the
 * rule's when PHPStan reports it under the rule's identifier with a message the rule's class writes (`argument.type`,
 * `missingType.return` and `variable.undefined` are reported by other classes too). PHPStan reports a line and nothing
 * else, so that is the positional answer.
 *
 * A rule's cases are the data files of every ported rule's tests, not its own alone, as PHP_CodeSniffer's are: they are
 * PHP files any rule reads, so another rule's file is upstream's answer on code the rule must stay silent on. A case's
 * variant names the test method that analyses it; its markup is that method's written expectations when the method
 * analyses the file alone with the options PHPStan's level gives the rule (RuleTestCase also runs with
 * conf/bleedingEdge.neon, which the release's levels do not include, so the two may differ; the gates report where).
 */

const DRIVER = fileURLToPath(new URL("./phpstan_driver.php", import.meta.url));
/** Upstream's sources and its tests: rule tests under tests/PHPStan/Rules/, and the analyser data some of them analyse (checkoutUpstream adds its licence). */
const SPARSE_PATHS = ["/src/", "/tests/PHPStan/", "/composer.json"];
/** Where phpstan-src keeps its tests; a test file's case path is its repository path. */
const TESTS = "tests/PHPStan";
/** The rule tests: every file of them is upstream's test text. */
const RULE_TESTS = "tests/PHPStan/Rules";
/** PHPStan's rule levels, loosest first: conf/config.level0.neon to conf/config.level10.neon (`max` is level 10). */
const LEVELS = Array.from({ length: 11 }, (_, level) => `level${level}`);
/** PHPStan prints instructions for an AI agent when one of these is set (src/Internal/AgentDetector.php): no part of the answer. */
const AGENT_VARIABLES = ["AUGMENT_AGENT", "AMP_CURRENT_THREAD_ID", "AI_AGENT", "CURSOR_TRACE_ID", "CURSOR_AGENT", "GEMINI_CLI", "CODEX_SANDBOX", "CODEX_THREAD_ID", "OPENCODE_CLIENT", "OPENCODE", "CLAUDECODE", "CLAUDE_CODE", "REPL_ID"];

/**
 * The ported rules, by the identifier PHPStan reports them under: the class that reports it, the RuleTestCase that
 * tests that class (under tests/PHPStan/), and a pattern every message the class writes under the identifier matches.
 */
const RULES: Record<string, { class: string; test: string; message: RegExp }> = {
  "argument.sprintf": { class: "PHPStan\\Rules\\Functions\\PrintfParametersRule", test: "Rules/Functions/PrintfParametersRuleTest.php", message: /^Call to sprintf contains /u },
  "argument.sscanf": { class: "PHPStan\\Rules\\Functions\\PrintfParametersRule", test: "Rules/Functions/PrintfParametersRuleTest.php", message: /^Call to sscanf contains /u },
  "argument.type": { class: "PHPStan\\Rules\\Functions\\RandomIntParametersRule", test: "Rules/Functions/RandomIntParametersRuleTest.php", message: /^Parameter #1 \$min \(.*\) of function random_int expects lower number than parameter #2 \$max \(.*\)\.$/u },
  "array.duplicateKey": { class: "PHPStan\\Rules\\Arrays\\DuplicateKeysInLiteralArraysRule", test: "Rules/Arrays/DuplicateKeysInLiteralArraysRuleTest.php", message: /^Array has \d+ duplicate keys? with value /u },
  "if.alwaysFalse": { class: "PHPStan\\Rules\\Comparison\\IfConstantConditionRule", test: "Rules/Comparison/IfConstantConditionRuleTest.php", message: /^If condition is always false\.$/u },
  "if.alwaysTrue": { class: "PHPStan\\Rules\\Comparison\\IfConstantConditionRule", test: "Rules/Comparison/IfConstantConditionRuleTest.php", message: /^If condition is always true\.$/u },
  "missingType.return": { class: "PHPStan\\Rules\\Functions\\MissingFunctionReturnTypehintRule", test: "Rules/Functions/MissingFunctionReturnTypehintRuleTest.php", message: /^Function \S+\(\) has no return type specified\.$/u },
  "variable.undefined": { class: "PHPStan\\Rules\\Variables\\DefinedVariableRule", test: "Rules/Variables/DefinedVariableRuleTest.php", message: /^(?:Undefined variable: \$\S+|Variable \$\S+ might not be defined\.)$/u },
};

interface TestMethod { method: string; files: string[]; errors: [string, number][]; properties: Record<string, unknown>; required: string[] }
/** A test file as phpstan_driver.php reads it: its methods' analyse() calls, and the test methods it leaves out, with why. */
interface TestReading { methods: TestMethod[]; left: { method: string; skipped: boolean; reason: string }[] }
interface Level { rules: string[]; parameters: Record<string, unknown> }
interface Facts { name: string; version: string; reference: string; license: string; php: string; files: string[]; levels: Record<string, Level> }
interface PhpstanError { message: string; line: number; identifier?: string }
interface Report { files: Record<string, { messages: PhpstanError[] }>; errors: string[] }
/** One analysis: the files PHPStan ran over together, and per level what it reported on each, or why it could not answer. */
interface Analysis { findings: Map<string, PhpstanError[]>; internal: Map<string, string> }

/** A finding PHPStan's own machinery reports about the analysis (a parse error, a reflection failure), after which its answer on the file is not whole. */
const isInternal = (error: PhpstanError): boolean => (error.identifier ?? "").startsWith("phpstan.") || error.message.startsWith("Reflection error");

/** Run `work` over `items`, at most `limit` at a time, keeping their order. */
async function pooled<T, R>(items: T[], limit: number, work: (item: T) => Promise<R>): Promise<R[]> {
  const results: R[] = new Array<R>(items.length);
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (next < items.length) {
      const index = next;
      next += 1;
      results[index] = await work(items[index]!);
    }
  }));
  return results;
}

export async function createPhpstanAdapter(pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner): Promise<PackAdapter> {
  // phpstan/phpstan's phar is built from phpstan-src at the release tag, which is the bare version.
  const upstream = await checkoutUpstream(cacheRoot, pack.upstream, pack.upstream.version, "src", SPARSE_PATHS, runner);
  const runtime = await composerRuntime(pack.upstream.runtime, cacheRoot, runner);
  const php = await createPhpToolkit(cacheRoot, runner);
  const packageDirectory = resolve(runtime.directory, "vendor", "phpstan", "phpstan");
  const phar = join(packageDirectory, "phpstan.phar");
  // PHPStan reads register_argc_argv when it builds its container (cliArgumentsVariablesRegistered: whether $argc and
  // $argv are defined in a script's main body), so it is pinned to the CLI's own default, which a php.ini may turn off;
  // phpstan.restarted stops PHPStan re-executing itself (TurboProcessRestarter) with its own ini in place of PHP_INI.
  const ini = [...PHP_INI, "-d", "register_argc_argv=1", "-d", "phpstan.restarted=1"];
  const ask = async <T>(request: object, autoload: string): Promise<T> =>
    JSON.parse(await runChecked(runner, [runtime.php, ...ini, DRIVER, autoload], { input: JSON.stringify(request), cwd: cacheRoot })) as T;
  const parserAutoload = join(php.runtime.directory, "vendor", "autoload.php");
  const fromCheckout = (path: string) => relative(upstream.directory, path).split(sep).join("/");
  await mkdir(resolve(cacheRoot, "tmp"), { recursive: true });

  const ported = names.filter((name) => RULES[name] !== undefined).sort();
  const testFiles = [...new Set(ported.map((name) => RULES[name]!.test))].sort();
  const testPath = (test: string) => join(upstream.directory, TESTS, test);
  const readings = testFiles.length === 0 ? {} : await ask<Record<string, TestReading>>({ op: "tests", tests: testFiles.map(testPath) }, parserAutoload);
  const methodsOf = (test: string) => readings[testPath(test)]?.methods ?? [];
  const options = [...new Set(testFiles.flatMap((test) => methodsOf(test).flatMap((method) => Object.keys(method.properties))))].sort();

  const scratch = await mkdtemp(resolve(cacheRoot, "tmp", "phpstan-facts-"));
  let facts: Facts;
  try {
    facts = await ask<Facts>({ op: "facts", phar, package: packageDirectory, scratch, levels: LEVELS, rules: [...new Set(ported.map((name) => RULES[name]!.class))].sort(), parameters: options }, "");
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }

  // The level each rule runs at: the first recommended level whose container registers its class, else the lowest that does.
  const recommendedBy = new Map(ported.flatMap((name) => {
    const config = pack.upstream.recommended_configs.find((level) => facts.levels[level]?.rules.includes(RULES[name]!.class) === true);
    return config === undefined ? [] : [[name, config] as const];
  }));
  const levelOf = new Map(ported.flatMap((name) => {
    const level = recommendedBy.get(name) ?? LEVELS.find((candidate) => facts.levels[candidate]?.rules.includes(RULES[name]!.class) === true);
    return level === undefined ? [] : [[name, level] as const];
  }));

  // A test method's options against the values a level gives them: how many agree, and whether all do.
  const agreement = (method: TestMethod, level: string | undefined) => {
    const values = level === undefined ? {} : facts.levels[level]!.parameters;
    const agreeing = Object.entries(method.properties).filter(([option, value]) => values[option] !== undefined && values[option] !== null && values[option] === value).length;
    return { agreeing, all: agreeing === Object.keys(method.properties).length };
  };

  // Each data file the ported rules' tests analyse, with the method it is analysed by: of the methods that analyse it, the
  // first whose options agree most with the level its test's rule runs at. A file two tests analyse is the first test's.
  const chosen = new Map<string, { test: string; method: TestMethod }>();
  for (const test of testFiles) {
    const level = levelOf.get(ported.find((name) => RULES[name]!.test === test)!);
    for (const method of methodsOf(test)) {
      for (const file of method.files) {
        const current = chosen.get(file);
        if (current === undefined) chosen.set(file, { test, method });
        else if (current.test === test && agreement(method, level).agreeing > agreement(current.method, level).agreeing) chosen.set(file, { test, method });
      }
    }
  }
  const caseFiles = [...chosen.keys()].sort((a, b) => (fromCheckout(a) < fromCheckout(b) ? -1 : 1));

  const environment: NodeJS.ProcessEnv = Object.fromEntries(Object.entries(process.env).filter(([key]) => !AGENT_VARIABLES.includes(key)));
  /**
   * PHPStan's JSON report over `paths` at `level`, run from its command line in a directory of its own. What it says it
   * could not analyse names no directory of this machine's: not the run's, not `scratch` (where the caller wrote
   * `paths`), the checkout, the runtime or the cache root, so the recording reads the same on every machine.
   */
  const analyse = async (paths: string[], level: string, scanFiles: string[] = [], scratch: string[] = []): Promise<Report> => {
    const directory = await mkdtemp(resolve(cacheRoot, "tmp", "phpstan-run-"));
    try {
      const config = join(directory, "phpstan.neon");
      // One process: PHPStan's parallel workers would run without the pinned ini (PHP_INI) when they are spawned rather than forked.
      await writeFile(config, `${JSON.stringify({ parameters: { level: Number(level.slice("level".length)), tmpDir: join(directory, "tmp"), paths, scanFiles, parallel: { maximumNumberOfProcesses: 1 } } }, null, 1)}\n`);
      const result = await runner.run([runtime.php, ...ini, phar, "analyse", "-c", config, "--no-progress", "--error-format=json"], { cwd: directory, env: environment });
      // PHPStan exits 1 when it reports errors; anything else is a failure to analyse.
      if (result.exitCode !== 0 && result.exitCode !== 1) throw new Error(`PHPStan exited ${result.exitCode} over ${paths.length} file(s):\n${(result.stderr.trim() || result.stdout.trim()).split("\n").slice(-20).join("\n")}`);
      const report = JSON.parse(result.stdout) as Report;
      // PHPStan names a file by the path it analysed, resolved, and a trait's file once per class that uses it
      // (`<path> (in context of class X)`): key every one by the path it was given, a trait's errors in each context included.
      const given = new Map(paths.map((path) => [realpathSync(path), path]));
      const machine = [directory, ...scratch, upstream.directory, runtime.directory, cacheRoot];
      const files: Report["files"] = {};
      for (const [name, file] of Object.entries(report.files)) {
        const real = realpathSync(name.replace(/ \(in context of .*\)$/u, ""));
        const path = given.get(real) ?? real;
        const messages = file.messages.map((error) => (isInternal(error) ? { ...error, message: withoutMachinePaths(error.message, machine) } : error));
        files[path] = { messages: [...(files[path]?.messages ?? []), ...messages] };
      }
      return { files, errors: report.errors.map((error) => withoutMachinePaths(error, machine)) };
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  };

  // Every analysis unit once per level a ported rule runs at, as many at a time as the machine runs. A unit is a test
  // method's analysed files, with the files the method requires for PHPStan to discover the symbols they use.
  const levels = [...new Set(levelOf.values())].sort();
  const unitKey = (method: TestMethod) => JSON.stringify([method.files, method.required]);
  const units = [...new Map([...chosen.values()].map(({ method }) => [unitKey(method), method])).values()];
  const analyses = new Map<string, Analysis>(levels.map((level) => [level, { findings: new Map(), internal: new Map() }]));
  await pooled(levels.flatMap((level) => units.map((unit) => ({ level, unit }))), availableParallelism(), async ({ level, unit }) => {
    const report = await analyse(unit.files, level, unit.required.filter((file) => !unit.files.includes(file)));
    const analysis = analyses.get(level)!;
    for (const file of unit.files) {
      if (unitKey(chosen.get(file)!.method) !== unitKey(unit)) continue;
      const messages = report.files[file]?.messages ?? [];
      const internal = report.errors[0] ?? messages.find(isInternal)?.message;
      if (internal !== undefined) analysis.internal.set(file, internal);
      analysis.findings.set(file, messages);
    }
  });

  // Every file of upstream's rule tests, and the data files the ported tests analyse outside them: the test words and
  // texts not-overfit reads.
  const textFiles = [...new Set([...filesUnder(join(upstream.directory, RULE_TESTS), () => true), ...caseFiles])].sort((a, b) => (fromCheckout(a) < fromCheckout(b) ? -1 : 1));
  const testText = new Map(textFiles.map((path) => [path, readFileSync(path, "utf8")]));
  const fixtureTexts = [...testText.values()];
  const fixtureTextSources: FixtureTextSource[] = [...testText].map(([path, text]) => ({ path: fromCheckout(path), sha256: createHash("sha256").update(text).digest("hex") }));
  const fixtureWords = new Set((await php.words(fixtureTexts)).flat());

  // not-overfit reads upstream's code from the pinned checkout, which the tool-pack checks show is the code that runs.
  const classFile = (name: string) => (name.startsWith("PHPStan\\") ? join(upstream.directory, "src", `${name.slice("PHPStan\\".length).replaceAll("\\", "/")}.php`) : null);
  const ruleFile = (name: string) => classFile(RULES[name]!.class)!;
  const ruleFiles = [...new Set(ported.map(ruleFile))].filter(existsSync);
  const references = ruleFiles.length === 0 ? {} : await ask<Record<string, string[]>>({ op: "references", files: ruleFiles }, parserAutoload);
  // A rule's helpers: the classes of PHPStan's src/ its class names (the helpers it is built with, such as
  // RuleLevelHelper or ConstantConditionRuleHelper, the types and nodes it checks, its imports). One level only: two
  // levels down, a rule's classes reach most of PHPStan's analyser, which would explain every word.
  const helpers = new Map(ported.map((name) => [name, [...new Set((references[ruleFile(name)] ?? []).map(classFile))]
    .filter((path): path is string => path !== null && path !== ruleFile(name) && existsSync(path)).sort()]));
  const literalFiles = [...new Set([...ruleFiles, ...[...helpers.values()].flat()])];
  const literalsOf = new Map<string, PhpLiterals>((await php.literals(literalFiles)).map((found, index) => [literalFiles[index]!, found]));
  const regexesOf = (file: string) => (literalsOf.get(file)?.regexes ?? []).flatMap((pattern) => {
    const regex = phpRegex(pattern);
    return regex === undefined ? [] : [regex];
  });

  // The options PHPStan's level gives the rule, among those its test sets.
  const optionsOf = (name: string, level: string) => {
    const set = [...new Set(methodsOf(RULES[name]!.test).flatMap((method) => Object.keys(method.properties)))].sort();
    return Object.fromEntries(set.map((option) => [option, facts.levels[level]!.parameters[option] ?? null]));
  };

  // A rule no level registers is absent upstream, as its cases are skipped: it has no answer on the corpus either.
  const rule = (name: string): UpstreamRule | undefined => {
    if (!ported.includes(name) || !existsSync(ruleFile(name)) || !levelOf.has(name)) return undefined;
    const config = recommendedBy.get(name);
    const configured = config === undefined ? {} : optionsOf(name, config);
    return {
      name,
      implementation: [fromCheckout(ruleFile(name))],
      tests: [fromCheckout(testPath(RULES[name]!.test)), ...[...new Set(methodsOf(RULES[name]!.test).flatMap((method) => method.files))].map(fromCheckout).sort()],
      deprecation: null,
      recommended: config === undefined ? null : { config, options: Object.keys(configured).length > 0 ? [configured] : [] },
      notOnRealCode: null,
    };
  };

  const srcFiles = filesUnder(join(upstream.directory, "src"), () => true).map((path) => relative(join(upstream.directory, "src"), path).split(sep).join("/")).sort();
  const pharFiles = [...facts.files].sort();
  const sameFiles = srcFiles.length === pharFiles.length && srcFiles.every((path, index) => path === pharFiles[index]);

  // The test methods of the ported rules' tests the reading leaves out: those that skip themselves on this PHP, as
  // upstream's own run does, and those it cannot evaluate, for which no case is recorded.
  const left = testFiles.flatMap((test) => (readings[testPath(test)]?.left ?? []).map((entry) => ({
    ...entry,
    where: `${fromCheckout(testPath(test))}::${entry.method}`,
    reason: withoutMachinePaths(entry.reason, [upstream.directory, runtime.directory, cacheRoot]),
  })));
  const unread = left.filter((entry) => !entry.skipped);
  const readMethods = new Set(testFiles.flatMap((test) => methodsOf(test).map((method) => `${test}::${method.method}`))).size;

  return {
    identity: {
      runtime: `PHPStan ${facts.version} (phpstan/phpstan's phar, built from ${facts.name} ${facts.reference}) on PHP ${facts.php}, from its command line at the rule level of the first recommended configuration that registers the rule (${pack.upstream.recommended_configs.join(", ")}), else the lowest level that does (${ported.map((name) => `${name} ${levelOf.get(name) ?? "none"}`).join(", ")}); each upstream test method's files analysed together, as RuleTestCase analyses them, with no project, autoloader or bleeding edge; one process`,
      positions: "the line PHPStan reports, 1-based; PHPStan reports no column and no end",
      parsers: `${php.version} (php -l) and nikic/php-parser ${php.parser}`,
    },
    packChecks: [
      ...upstream.checks,
      {
        id: "runtime-version",
        ok: facts.version === pack.upstream.version && runtime.version === runtime.pinned,
        detail: `installed PHPStan is ${facts.version} on PHP ${runtime.version}; the pack pins ${pack.upstream.version} on PHP ${runtime.pinned}`,
      },
      {
        // The phar's sources are PHPStan's, downgraded to the oldest PHP it supports and reprinted, so they are compared by
        // the commit the phar records it was built from and by their files, not byte for byte.
        id: "runtime-matches-checkout",
        ok: facts.name === "phpstan/phpstan-src" && facts.reference === pack.upstream.commit && sameFiles,
        detail: `the installed phar is built from ${facts.name} ${facts.reference} (its vendor/composer/installed.php); the pack pins ${pack.upstream.commit}; its src/ has ${pharFiles.length} file(s), src/ at the pinned commit ${srcFiles.length}${sameFiles ? ", the same" : ", not the same"}`,
      },
      {
        id: "declared-license",
        ok: facts.license === pack.upstream.license,
        detail: `the installed package declares ${facts.license || "no license"}; the pack records ${pack.upstream.license}`,
      },
      {
        // Informational, so it never fails a rule: the report shows what the reading leaves out, and a re-record whose
        // reading shrank changes the count.
        id: "upstream-tests-read",
        ok: true,
        detail: `${readMethods} test method(s) of the ported rules' tests read statically; ${left.length - unread.length} skip themselves on PHP ${facts.php}; ${unread.length} cannot be evaluated, and no case is recorded for them${unread.length === 0 ? "" : `: ${unread.slice(0, 5).map((entry) => `${entry.where} (${entry.reason})`).join("; ")}${unread.length > 5 ? "; …" : ""}`}`,
      },
    ],
    licenseText: upstreamLicenseText(upstream.directory),
    rule,

    implementationNames(name: string): Set<string> {
      if (!ported.includes(name)) return new Set();
      const file = ruleFile(name);
      return new Set([
        ...(literalsOf.get(file)?.names ?? []),
        ...regexesOf(file).flatMap((regex) => regex.source.match(/[A-Za-z_]\w*/gu) ?? []),
      ]);
    },
    sharedImplementationNames(name: string): Set<string> {
      return new Set((helpers.get(name) ?? []).flatMap((path) => literalsOf.get(path)?.names ?? []));
    },
    implementationRegexes(name: string) {
      if (!ported.includes(name)) return [];
      return [ruleFile(name), ...(helpers.get(name) ?? [])].flatMap(regexesOf);
    },
    fixtureWords: () => fixtureWords,
    fixtureTexts: () => fixtureTexts,
    fixtureTextSources: () => fixtureTextSources,
    keywords: php.keywords,
    commentVocabulary: php.commentVocabulary,
    // PHPStan analyses .php files by default (conf/config.neon's fileExtensions).
    inScopeSamples: ["src/example.php"],

    cases(name: string): UpstreamCase[] {
      if (!ported.includes(name)) return [];
      const level = levelOf.get(name);
      return caseFiles.map((path) => {
        const fixture = fromCheckout(path);
        const { test, method } = chosen.get(path)!;
        const analysis = level === undefined ? undefined : analyses.get(level)!;
        const internal = analysis?.internal.get(path);
        const skipped = level === undefined
          ? `no rule level of PHPStan ${facts.version} registers ${RULES[name]!.class}`
          : internal === undefined ? null : `upstream could not analyse it: ${internal}`;
        const own = test === RULES[name]!.test && method.files.length === 1 && agreement(method, level).all;
        return {
          fixture,
          variant: `${test}::${method.method}`,
          path: fixture,
          source: readFileSync(path, "utf8"),
          skipped,
          expected: skipped === null ? (analysis!.findings.get(path) ?? []).filter((error) => error.identifier === name && RULES[name]!.message.test(error.message)).map(errorSpan) : [],
          // A test's expectations name no identifier: the rule's are those with a message its class writes.
          markup: own ? method.errors.filter(([message]) => RULES[name]!.message.test(message)).map(([, line]) => errorSpan({ line })) : null,
        };
      });
    },

    toSpan(_name: string, source: string, startByte: number, endByte: number): Span {
      return phpstanSpan(source, startByte, endByte, PHPSTAN_SPAN_PARTS);
    },

    // PHPStan analyses .php files; a Blade template (`.blade.php`) is the blade pack's, as the php pack's match excludes it.
    inCorpus: (path: string) => path.endsWith(".php") && !path.endsWith(".blade.php"),

    async lintCorpus(files, ruleNames) {
      if (files.length === 0) return [];
      const directory = await mkdtemp(resolve(cacheRoot, "tmp", "phpstan-corpus-"));
      try {
        const paths = files.map((file) => join(directory, file.task, file.path));
        await Promise.all(files.map(async (file, index) => {
          await mkdir(dirname(paths[index]!), { recursive: true });
          await writeFile(paths[index]!, file.source);
        }));
        // The corpus is one project per level its rules run at. A file PHPStan cannot parse or reflect leaves its answer
        // incomplete: it is excluded, and the rest analysed again without it.
        const excluded = new Map<string, string>();
        const found = new Map<string, Map<string, PhpstanError[]>>();
        for (const level of [...new Set(ruleNames.flatMap((name) => levelOf.get(name) ?? []))].sort()) {
          for (;;) {
            const report = await analyse(paths.filter((path) => !excluded.has(path)), level, [], [directory]);
            const severe = Object.entries(report.files).flatMap(([path, file]) => {
              const internal = file.messages.find(isInternal);
              return internal === undefined ? [] : [[path, internal.message] as const];
            });
            if (report.errors.length > 0 && severe.length === 0) throw new Error(`PHPStan could not analyse the corpus: ${report.errors[0]!}`);
            if (severe.length === 0) {
              found.set(level, new Map(Object.entries(report.files).map(([path, file]) => [path, file.messages])));
              break;
            }
            for (const [path, message] of severe) excluded.set(path, message);
          }
        }
        return files.map((file, index) => {
          const path = paths[index]!;
          const reason = excluded.get(path);
          // This machine's directories are no part of the answer (analyse strips them): the recording reads the same on every machine.
          if (reason !== undefined) return { file, excluded: `upstream could not analyse it: ${reason}`, findings: new Map() };
          return {
            file,
            excluded: null,
            findings: new Map(ruleNames.map((name) => {
              const level = levelOf.get(name);
              const messages = level === undefined ? [] : found.get(level)?.get(path) ?? [];
              return [name, messages.filter((error) => error.identifier === name && RULES[name]?.message.test(error.message) === true).map(errorSpan)];
            })),
          };
        });
      } finally {
        await rm(directory, { recursive: true, force: true });
      }
    },

    parserVerdicts: (files) => php.verdicts(files),
  };
}
