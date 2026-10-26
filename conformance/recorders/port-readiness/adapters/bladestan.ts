import { createHash } from "node:crypto";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { runChecked, type CommandRunner } from "../../../port-readiness/process.js";
import { upstreamLicenseText } from "../../../port-readiness/recorded-adapter.js";
import { bladestanSpan, templateLineSpan, BLADESTAN_SPAN_PARTS } from "../../../port-readiness/span-units/bladestan.js";
import type { FixtureTextSource, PackAdapter, PackManifest, Span, UpstreamCase, UpstreamRule } from "../../../port-readiness/types.js";
import { createBladeToolkit } from "../languages/blade.js";
import { phpRegex, type PhpLiterals } from "../languages/php.js";
import { checkoutUpstream, composerRuntime, filesUnder, PHP_INI, treeDigest, withoutMachinePaths } from "../runtimes.js";

/**
 * The Bladestan adapter. Bladestan runs PHPStan's rules over the PHP a template compiles to, where PHP code renders the
 * template (`view('name', [...])`), and reports each error at the template file and line it maps back to. Expected
 * answers come from running the pinned Bladestan through its own rule test (tests/Rules/BladeRuleTest.php: PHPStan's
 * RuleTestCase at level 8 with tests/Rules/config/configured_extension.neon, in the Testbench application its
 * testbench.yaml boots) on the templates of that test's application (tests/skeleton/resources), each rendered alone
 * with no view data, as `view('<its name>')` renders it: a variable no path of the template defines is then "might not
 * be defined", which no ported rule claims, so Bladestan's reading and Rerail's, which reads view data as unknown,
 * agree on what a template alone says. Only the errors Bladestan places at the rendered template itself are its
 * answer; an included template's are the included template's, which its own render answers.
 *
 * Upstream's test renders the templates from PHP fixtures with data (tests/Rules/Fixture/*.php), and asserts each
 * error's message and caller line; the `upstream-tests-agree` check holds the installed Bladestan to every one of those
 * expectations. The template lines are Bladestan's metadata, so its output is the positional answer.
 *
 * A ported rule is one PHPStan identifier, and the ported finding the error of that identifier whose message is the
 * rule the pack ports (`PORTED`): `argument.type` is every rule's parameter type error, the port random_int's. Every
 * template is every rule's case, as Bandit's examples are for every Bandit test, with variant `.`: one test owns them all.
 */

const DRIVER = fileURLToPath(new URL("./bladestan_driver.php", import.meta.url));
/** Upstream's sources, its configuration and bootstrap, and its tests with their application (checkoutUpstream adds its licence). */
const SPARSE_PATHS = ["/src/", "/config/", "/tests/", "/bootstrap.php", "/testbench.yaml", "/composer.json"];
/** What the package runs: its sources, its PHPStan configuration and the bootstrap that configuration loads. */
const RUNNING_CODE = /^(?:src|config)\/|^bootstrap\.php$/u;
/** The templates of the test's application, and the fixtures and classes BladeRuleTest analyses them with. */
const TEST_INPUTS = ["tests/skeleton", "tests/Rules"];
/** The packages whose versions decide Bladestan's answers: it, the Laravel it compiles with, and the extensions and application it runs in. */
const PACKAGES = ["tomasvotruba/bladestan", "laravel/framework", "larastan/larastan", "livewire/livewire", "orchestra/testbench-core", "nikic/php-parser"];
/** Where the runtime keeps PHPStan's sources, inside its phar. */
const PHAR = "vendor/phpstan/phpstan/phpstan.phar";

/**
 * Each ported identifier: the PHPStan rule that reports it and the message of the ported rule (PHPStan 2.2.16), as
 * upstream/bladestan/generate.py records them.
 */
const PORTED: Record<string, { rule: string; message: RegExp }> = {
  // DefinedVariableRule: "Undefined variable: $x"; not "Variable $x might not be defined." (some paths).
  "variable.undefined": { rule: "PHPStan\\Rules\\Variables\\DefinedVariableRule", message: /^Undefined variable: \$/u },
  "argument.sprintf": { rule: "PHPStan\\Rules\\Functions\\PrintfParametersRule", message: /^Call to sprintf contains /u },
  "array.duplicateKey": { rule: "PHPStan\\Rules\\Arrays\\DuplicateKeysInLiteralArraysRule", message: /^Array has \d+ duplicate keys? with value /u },
  "argument.type": { rule: "PHPStan\\Rules\\Functions\\RandomIntParametersRule", message: /^Parameter #1 \$min \(.*\) of function random_int expects lower number than parameter #2 \$max/u },
  "if.alwaysTrue": { rule: "PHPStan\\Rules\\Comparison\\IfConstantConditionRule", message: /^If condition is always true\.$/u },
  "if.alwaysFalse": { rule: "PHPStan\\Rules\\Comparison\\IfConstantConditionRule", message: /^If condition is always false\.$/u },
  "missingType.return": { rule: "PHPStan\\Rules\\Functions\\MissingFunctionReturnTypehintRule", message: /^Function .+ has no return type specified\.$/u },
  "identical.alwaysTrue": { rule: "PHPStan\\Rules\\Comparison\\StrictComparisonOfDifferentTypesRule", message: /^Strict comparison using === between .+ will always evaluate to true\.$/u },
};

interface BladestanError { identifier: string | null; message: string; line: number; template: string | null; templateLine: number | null }
interface Rendered { path: string; view: string | null; errors?: BladestanError[]; crash?: string }
interface Facts {
  versions: Record<string, string | null>;
  phpstan: string;
  php: string;
  package: string;
  phar: string;
  pharSources: string[];
  license: string | null;
  rules: Record<string, { file: string; registered: boolean }>;
  tests: { file: string; expected: { message: string; line: number }[]; errors: BladestanError[] }[];
  templates: Rendered[];
}

/** Whether an error is the ported finding of `name`. */
const isPorted = (name: string, error: BladestanError): boolean => {
  const ported = PORTED[name];
  return ported !== undefined && error.identifier === name && ported.message.test(error.message);
};

export async function createBladestanAdapter(pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner): Promise<PackAdapter> {
  // Bladestan publishes its Composer package from the repository at the release tag, which is the bare version.
  const upstream = await checkoutUpstream(cacheRoot, pack.upstream, pack.upstream.version, "src", SPARSE_PATHS, runner);
  const runtime = await composerRuntime(pack.upstream.runtime, cacheRoot, runner);
  const blade = await createBladeToolkit(cacheRoot, runner);
  const fromCheckout = (path: string) => relative(upstream.directory, path).split(sep).join("/");

  /**
   * The driver's answer, run in a scratch directory of its own (the test application and TMPDIR live there). Every
   * machine's path is stripped from the answer, the longest first: the scratch
   * directory, upstream's checkout, the runtime and anything else under the cache root, each as given and resolved. The
   * recording must read the same on every machine, so a template's or test file's path in the answer is relative to the
   * checkout, the package's and the phar's to the runtime, and a message (a crash, a Laravel `(View: …)` suffix) names
   * no cache root. `register_argc_argv` is the CLI's default, set explicitly since php.ini files differ on it and
   * PHPStan reads it (DefinedVariableRule's $argc and $argv).
   */
  async function ask<T>(request: object): Promise<T> {
    await mkdir(resolve(cacheRoot, "tmp"), { recursive: true });
    const directory = realpathSync(await mkdtemp(resolve(cacheRoot, "tmp", "bladestan-")));
    try {
      await mkdir(join(directory, "tmp"));
      const command = [runtime.php, ...PHP_INI, "-d", "register_argc_argv=1", DRIVER, runtime.directory];
      const output = await runChecked(runner, command, { input: JSON.stringify(request), cwd: directory, env: { ...process.env, TMPDIR: join(directory, "tmp") } });
      return JSON.parse(withoutMachinePaths(output, [directory, upstream.directory, runtime.directory, cacheRoot])) as T;
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  }

  const ported = names.filter((name) => PORTED[name] !== undefined).sort();
  const ruleClasses = [...new Set(ported.map((name) => PORTED[name]!.rule))].sort();
  const levels = await ask<Record<string, number | null>>({ op: "levels", rules: ruleClasses });
  const facts = await ask<Facts>({ op: "facts", checkout: upstream.directory, rules: ruleClasses, packages: PACKAGES });
  // The driver's paths are machine-free (ask): the runtime's are resolved against it again where a file is read.
  const installedPackage = resolve(runtime.directory, facts.package);

  // BladeRuleTest's inputs (the templates, the fixtures rendering them, the classes they use): the test words and
  // texts not-overfit reads.
  const testFiles = TEST_INPUTS.flatMap((directory) => filesUnder(join(upstream.directory, directory), () => true));
  const testText = new Map(testFiles.map((path) => [path, readFileSync(path, "utf8")]));
  const fixtureTexts = [...testText.values()];
  const fixtureTextSources: FixtureTextSource[] = [...testText].map(([path, text]) => ({ path: fromCheckout(path), sha256: createHash("sha256").update(text).digest("hex") }));
  const phpFiles = testFiles.filter((path) => path.endsWith(".php") && !path.endsWith(".blade.php"));
  const otherFiles = testFiles.filter((path) => !phpFiles.includes(path));
  const fixtureWords = new Set([
    ...(await blade.words(otherFiles.map((path) => testText.get(path)!))).flat(),
    ...(await blade.php.words(phpFiles.map((path) => testText.get(path)!))).flat(),
  ]);

  // Upstream's own test, every expectation (message and caller line) against what the installed Bladestan reports.
  const disagreements: string[] = [];
  for (const test of facts.tests) {
    const reported = test.errors.map((error) => `${error.line}: ${error.message}`).sort();
    const expected = test.expected.map((error) => `${error.line}: ${error.message}`).sort();
    const missing = expected.filter((entry, index) => expected.indexOf(entry) === index && expected.filter((other) => other === entry).length > reported.filter((other) => other === entry).length);
    const extra = reported.filter((entry, index) => reported.indexOf(entry) === index && reported.filter((other) => other === entry).length > expected.filter((other) => other === entry).length);
    if (missing.length > 0 || extra.length > 0) disagreements.push(`${test.file}: ${[...missing.map((entry) => `expects ${entry}`), ...extra.map((entry) => `reports ${entry}`)].slice(0, 2).join(", ")}`);
  }

  // not-overfit reads upstream's code: Bladestan's from the pinned checkout, which the tool-pack checks show is the code
  // that runs, and PHPStan's rules from the phar the runtime runs.
  const pharFile = (source: string) => `phar://${resolve(runtime.directory, facts.phar)}/${source}`;
  const pharSources = new Set(facts.pharSources);
  const literalsOf = new Map<string, PhpLiterals>();
  const readLiterals = async (files: string[]) => {
    const fresh = [...new Set(files)].filter((file) => !literalsOf.has(file));
    for (const [index, found] of (await blade.literals(fresh)).entries()) literalsOf.set(fresh[index]!, found);
  };
  /** A class's file within `namespace`: Bladestan's under the checkout's src/, PHPStan's rules under the phar's src/Rules/. */
  const classFile = (name: string): string | null => {
    if (name.startsWith("Bladestan\\")) {
      const path = join(upstream.directory, "src", `${name.slice("Bladestan\\".length).replaceAll("\\", "/")}.php`);
      return existsSync(path) ? path : null;
    }
    const source = name.startsWith("PHPStan\\Rules\\") ? `src/${name.slice("PHPStan\\".length).replaceAll("\\", "/")}.php` : null;
    return source !== null && pharSources.has(source) ? pharFile(source) : null;
  };
  /** The classes a file imports and extends, and theirs in turn, within Bladestan and PHPStan's rules. */
  const closure = async (start: string): Promise<string[]> => {
    const seen = new Set<string>([start]);
    await readLiterals([start]);
    for (let frontier = [start]; frontier.length > 0;) {
      const next = [...new Set(frontier.flatMap((file) => {
        const found = literalsOf.get(file);
        return [...(found?.imports ?? []), ...(found?.classes ?? []).flatMap((declared) => (declared.extends === null ? [] : [declared.extends]))];
      }).map(classFile))].filter((path): path is string => path !== null && !seen.has(path));
      for (const path of next) seen.add(path);
      await readLiterals(next);
      frontier = next;
    }
    seen.delete(start);
    return [...seen].sort();
  };
  const bladeRule = join(upstream.directory, "src", "Rules", "BladeRule.php");
  const bladeRuleHelpers = [bladeRule, ...(await closure(bladeRule))];
  const ruleFile = (name: string) => pharFile(facts.rules[PORTED[name]!.rule]!.file);
  const helpers = new Map<string, string[]>();
  for (const name of ported) helpers.set(name, [...new Set([...(await closure(ruleFile(name))), ...bladeRuleHelpers])].sort());
  const regexesOf = (file: string) => (literalsOf.get(file)?.regexes ?? []).flatMap((pattern) => {
    const regex = phpRegex(pattern);
    return regex === undefined ? [] : [regex];
  });

  // The templates, and the case of each: what Bladestan placed at the template itself when it rendered it alone.
  const templates = facts.templates.map((rendered) => {
    const fixture = rendered.path;
    const placed = (rendered.errors ?? []).filter((error) => error.template === null);
    const skipped = rendered.view === null
      ? "no view name addresses it: its name holds a dot besides .blade.php"
      : rendered.crash !== undefined
        ? `upstream could not analyse it: ${rendered.crash}`
        : placed.length > 0
          ? `upstream could not analyse it: ${[...new Set(placed.map((error) => error.message))].join("; ")}`
          : null;
    return { fixture, source: readFileSync(join(upstream.directory, fixture), "utf8"), skipped, errors: (rendered.errors ?? []).filter((error) => error.template === rendered.path) };
  });

  const rule = (name: string): UpstreamRule | undefined => {
    const ported = PORTED[name];
    if (ported === undefined || facts.rules[ported.rule]?.registered !== true) return undefined;
    const level = levels[ported.rule] ?? null;
    const config = level === null ? undefined : pack.upstream.recommended_configs.find((candidate) => Number(/^level(\d+)$/u.exec(candidate)?.[1] ?? Number.NaN) >= level);
    return {
      name,
      implementation: ["src/Rules/BladeRule.php", `${PHAR}/${facts.rules[ported.rule]!.file}`],
      // The templates upstream's own test reports the ported finding at.
      tests: [...new Set(facts.tests.flatMap((test) => test.errors.filter((error) => error.template !== null && isPorted(name, error)).map((error) => error.template!)))].sort(),
      // PHPStan deprecates rules by removing them in a major release; none of these carries a deprecation.
      deprecation: null,
      // A PHPStan level is a configuration file; the ported rules take no option there.
      recommended: config === undefined ? null : { config, options: [] },
      notOnRealCode: null,
    };
  };

  const findingsOf = (errors: BladestanError[], name: string): Span[] =>
    errors.filter((error) => isPorted(name, error) && error.templateLine !== null).map((error) => templateLineSpan({ templateLine: error.templateLine! }));

  return {
    identity: {
      runtime: `Bladestan ${facts.versions["tomasvotruba/bladestan"]} running PHPStan ${facts.phpstan} on PHP ${facts.php} (laravel/framework ${facts.versions["laravel/framework"]}, larastan ${facts.versions["larastan/larastan"]}, livewire ${facts.versions["livewire/livewire"]}, orchestra/testbench-core ${facts.versions["orchestra/testbench-core"]}, nikic/php-parser ${facts.versions["nikic/php-parser"]}), as its own rule test runs it (tests/Rules/BladeRuleTest.php: PHPStan's RuleTestCase at level 8 with tests/Rules/config/configured_extension.neon, in the Testbench application of its testbench.yaml); each template rendered alone with no view data (\`view('<name>')\`), its answer the errors Bladestan places at the template itself; a ported finding is the error of the identifier whose message is the ported rule's`,
      positions: "the template line Bladestan reports (its template_line metadata: 1-based, lines split at \\n), and no column or end",
      parsers: blade.version,
    },
    packChecks: [
      ...upstream.checks,
      {
        id: "runtime-version",
        ok: facts.versions["tomasvotruba/bladestan"] === pack.upstream.version && runtime.version === runtime.pinned,
        detail: `installed Bladestan is ${facts.versions["tomasvotruba/bladestan"] ?? "absent"} on PHP ${runtime.version}; the pack pins ${pack.upstream.version} on PHP ${runtime.pinned}`,
      },
      {
        id: "runtime-matches-checkout",
        ok: treeDigest(installedPackage, (path) => RUNNING_CODE.test(relative(installedPackage, path).split(sep).join("/"))) === treeDigest(upstream.directory, (path) => RUNNING_CODE.test(fromCheckout(path))),
        detail: "the installed package's src/, config/ and bootstrap.php and those at the pinned commit have the same files and content",
      },
      {
        id: "declared-license",
        ok: facts.license === pack.upstream.license,
        detail: `the installed package declares ${facts.license ?? "no license"}; the pack records ${pack.upstream.license}`,
      },
      {
        id: "upstream-tests-agree",
        ok: disagreements.length === 0 && facts.tests.length > 0,
        detail: disagreements.length === 0
          ? `every fixture of tests/Rules/BladeRuleTest.php (${facts.tests.length}) reports exactly the errors (message and line) the test expects`
          : `${disagreements.length} fixture(s) differ from upstream's test: ${disagreements.slice(0, 3).join("; ")}`,
      },
    ],
    licenseText: upstreamLicenseText(upstream.directory),
    rule,

    implementationNames(name: string): Set<string> {
      if (rule(name) === undefined) return new Set();
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
      if (rule(name) === undefined) return [];
      return [ruleFile(name), ...(helpers.get(name) ?? [])].flatMap(regexesOf);
    },
    fixtureWords: () => fixtureWords,
    fixtureTexts: () => fixtureTexts,
    fixtureTextSources: () => fixtureTextSources,
    keywords: blade.keywords,
    commentVocabulary: blade.commentVocabulary,
    // Bladestan analyses Blade templates (`.blade.php`), the views PHP code renders.
    inScopeSamples: ["resources/views/example.blade.php"],

    cases(name: string): UpstreamCase[] {
      if (rule(name) === undefined) return [];
      return templates.map((template) => ({
        fixture: template.fixture,
        variant: ".",
        path: template.fixture,
        source: template.source,
        skipped: template.skipped,
        expected: template.skipped === null ? findingsOf(template.errors, name) : [],
        // The test asserts messages at caller lines, which upstream-tests-agree checks; no template position to compare.
        markup: null,
      }));
    },

    toSpan(_name: string, source: string, startByte: number, endByte: number): Span {
      return bladestanSpan(source, startByte, endByte, BLADESTAN_SPAN_PARTS);
    },

    parserVerdicts: (files) => blade.verdicts(files),
  };
}
