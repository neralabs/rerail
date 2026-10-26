import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { runChecked, type CommandRunner } from "../../../port-readiness/process.js";
import { upstreamLicenseText } from "../../../port-readiness/recorded-adapter.js";
import { bladeParserSpan, BLADE_PARSER_SPAN_PARTS, validationSpan } from "../../../port-readiness/span-units/blade-parser.js";
import type { FixtureTextSource, PackAdapter, PackManifest, Span, UpstreamCase, UpstreamRule } from "../../../port-readiness/types.js";
import { createBladeToolkit } from "../languages/blade.js";
import { phpRegex, type PhpLiterals } from "../languages/php.js";
import { checkoutUpstream, filesUnder, treeDigest, withoutMachinePaths } from "../runtimes.js";

/**
 * The stillat/blade-parser adapter. Expected answers come from running the pinned stillat/blade-parser itself on the
 * templates upstream's own validator tests (tests/Validation/<X>Test.php, Pest files) hand the parser, each validator
 * as the package's default configuration loads it (config/validation.php's core_validators, with the options
 * ValidatorServiceProvider gives it) on a document of its own, as its test runs it. The templates are read by running
 * the tests themselves (blade_parser_tests.php, which the pack's conformance recorder runs too: a test with Pest stubbed
 * and every template it passes Document::fromText() captured, every dataset row included);
 * upstream's tests assert a count of errors and their messages, not positions, so the validator's output is the
 * positional answer, and the `upstream-tests-agree` check holds that output to the tests' counts and messages.
 *
 * A validator's cases are the templates of every ported validator's test, not its own alone: they are Blade templates
 * any validator reads, so another validator's template is upstream's answer on a template the rule must stay silent
 * on, as Bandit's examples are for every Bandit test. The rule's own templates have variant `.`; another's name the
 * test that owns it.
 */

const DRIVER = fileURLToPath(new URL("./blade_parser_driver.php", import.meta.url));
/** The reader of upstream's validator tests, shared with the Blade pack's conformance recorder (generate.py). */
const READ_TESTS = fileURLToPath(new URL("./blade_parser_tests.php", import.meta.url));
/** Upstream's sources, its default configuration, and the validator tests with the Pest file they load (checkoutUpstream adds its licence). */
const SPARSE_PATHS = ["/src/", "/config/", "/tests/Pest.php", "/tests/Validation/", "/composer.json"];
const VALIDATOR_TESTS = "tests/Validation";
/** The namespace of upstream's classes, under src/ (composer.json's psr-4). */
const NAMESPACE = "Stillat\\BladeParser\\";
/** The configuration a project's validators are loaded from: the package's config/validation.php, merged under this key. */
const DEFAULT_CONFIG = "blade.validation";

interface Finding { line: number; column: number; message: string }
interface LintResult { findings: Record<string, Record<string, Finding[]>>; errors: Record<string, string> }
interface ValidatorFact {
  class: string;
  file: string;
  deprecation: string | null;
  test: string | null;
  names: string[];
  options: Record<string, unknown> | null;
}
interface Facts {
  version: string;
  /** The installed laravel/framework, whose container, collections and Blade compiler stillat/blade-parser runs on. */
  framework: string;
  license: string | null;
  php: string;
  package: string;
  validators: Record<string, ValidatorFact>;
}
/** One test (and dataset row) as blade_parser_tests.php runs it: the templates it parsed and the expectations it made. */
interface TestRun {
  test: string;
  dataset: string | null;
  templates: string[];
  expectations: ({ count: number } | { be: unknown })[];
}
/** One template a validator's test hands the parser, numbered as the pack's fixtures/<X>Test/<n>.blade.php are. */
interface Template { validator: string; test: string; number: number; source: string }

export async function createBladeParserAdapter(pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner): Promise<PackAdapter> {
  // stillat/blade-parser publishes its Composer package from the repository at the release tag, `v<version>`.
  const upstream = await checkoutUpstream(cacheRoot, pack.upstream, `v${pack.upstream.version}`, "src", SPARSE_PATHS, runner);
  const blade = await createBladeToolkit(cacheRoot, runner);
  const runtime = blade.runtime;
  const ask = async <T>(request: object): Promise<T> =>
    JSON.parse(await runChecked(runner, runtime.command(DRIVER, runtime.directory), { input: JSON.stringify(request), cwd: cacheRoot })) as T;
  const facts = await ask<Facts>({ op: "facts", validators: names, checkout: upstream.directory });
  /** The driver's lint, with this machine's directories taken out of what it says it could not parse, so the recording reads the same on every machine. */
  const lint = async (request: object): Promise<LintResult> => {
    const result = await ask<LintResult>({ op: "lint", ...request });
    const machine = [upstream.directory, runtime.directory, cacheRoot];
    return { ...result, errors: Object.fromEntries(Object.entries(result.errors).map(([path, error]) => [path, withoutMachinePaths(error, machine)])) };
  };
  const fromCheckout = (path: string) => relative(upstream.directory, path).split(sep).join("/");
  const ported = Object.keys(facts.validators).sort();

  // Every ported validator's test, run: each distinct template of a test file, in Pest's order, and every run's expectations.
  const templates: Template[] = [];
  const runs: { validator: string; where: string; template: Template; expectations: TestRun["expectations"] }[] = [];
  for (const validator of ported) {
    const test = facts.validators[validator]!.test;
    if (test === null) continue;
    const read = JSON.parse(await runChecked(runner, runtime.command(READ_TESTS, runtime.directory, upstream.directory, join(upstream.directory, test)), { cwd: cacheRoot })) as TestRun[];
    const own = new Map<string, Template>();
    for (const run of read) {
      const where = `${test}: ${run.test}${run.dataset === null ? "" : ` ${run.dataset}`}`;
      if (run.templates.length !== 1) throw new Error(`${where} hands Document::fromText() ${run.templates.length} templates`);
      const source = run.templates[0]!;
      if (!own.has(source)) {
        const template = { validator, test, number: own.size + 1, source };
        own.set(source, template);
        templates.push(template);
      }
      runs.push({ validator, where, template: own.get(source)!, expectations: run.expectations });
    }
  }
  const fixtureOf = (template: Template) => `${template.test}#${template.number}`;

  // The templates, linted as the default configuration loads each validator (as its test constructs one it does not list).
  const configured = await lint({
    validators: ported.map((validator) => ({ name: validator, options: facts.validators[validator]!.options })),
    files: templates.map((template) => ({ path: fixtureOf(template), source: template.source })),
  });

  const disagreements: string[] = [];
  for (const { validator, where, template, expectations } of runs) {
    const reported = configured.findings[fixtureOf(template)]?.[validator] ?? [];
    const error = configured.errors[fixtureOf(template)];
    if (error !== undefined) {
      disagreements.push(`${where}: stillat/blade-parser throws on its template: ${error}`);
      continue;
    }
    for (const expectation of expectations) {
      if ("count" in expectation && expectation.count !== reported.length) disagreements.push(`${where} expects ${expectation.count} error(s); ${validator} reports ${reported.length}`);
    }
    const messages = reported.map((finding) => finding.message);
    for (const expectation of expectations) {
      if (!("be" in expectation) || typeof expectation.be !== "string") continue;
      const index = messages.indexOf(expectation.be);
      if (index === -1) disagreements.push(`${where} expects the message ${JSON.stringify(expectation.be)}; ${validator} reports ${JSON.stringify(reported.map((finding) => finding.message))}`);
      else messages.splice(index, 1);
    }
  }

  // Every validator test upstream ships, and the templates the ported validators' tests build: the test words and
  // texts not-overfit reads. A dataset row's template is built at run time (DebugDirectiveTest's from the parser's
  // directive names), so it is in no test file's text.
  const testFiles = filesUnder(join(upstream.directory, VALIDATOR_TESTS), (path) => path.endsWith("Test.php"));
  const testText = new Map(testFiles.map((path) => [path, readFileSync(path, "utf8")]));
  const fixtureTexts = [...testText.values(), ...templates.map((template) => template.source)];
  const fixtureTextSources: FixtureTextSource[] = [
    ...[...testText].map(([path, text]) => ({ path: fromCheckout(path), sha256: createHash("sha256").update(text).digest("hex") })),
    ...templates.map((template) => ({ inline: template.source })),
  ];
  const fixtureWords = new Set((await blade.words(fixtureTexts)).flat());

  // not-overfit reads upstream's code from the pinned checkout, which the tool-pack checks show is the code that runs.
  const sourceOf = (installed: string) => join(upstream.directory, relative(facts.package, installed));
  const validatorFile = (name: string) => sourceOf(facts.validators[name]!.file);
  const literalsOf = new Map<string, PhpLiterals>();
  const readLiterals = async (files: string[]) => {
    const fresh = [...new Set(files)].filter((file) => !literalsOf.has(file));
    for (const [index, found] of (await blade.literals(fresh)).entries()) literalsOf.set(fresh[index]!, found);
  };
  /** The checkout file of a stillat/blade-parser class (`Stillat\BladeParser\Nodes\DirectiveNode` is src/Nodes/DirectiveNode.php), when it is one. */
  const classFile = (name: string) => (name.startsWith(NAMESPACE) ? join(upstream.directory, "src", `${name.slice(NAMESPACE.length).replaceAll("\\", "/")}.php`) : null);
  // A validator's helpers: the classes its file imports and the classes it extends, and theirs in turn, within upstream's src/.
  const helpers = new Map<string, string[]>();
  await readLiterals(ported.map(validatorFile));
  for (const name of ported) {
    const seen = new Set<string>([validatorFile(name)]);
    for (let frontier = [validatorFile(name)]; frontier.length > 0;) {
      const next = [...new Set(frontier.flatMap((file) => {
        const found = literalsOf.get(file);
        return [...(found?.imports ?? []), ...(found?.classes ?? []).flatMap((declared) => (declared.extends === null ? [] : [declared.extends]))];
      }).map(classFile))].filter((path): path is string => path !== null && !seen.has(path) && existsSync(path));
      for (const path of next) seen.add(path);
      await readLiterals(next);
      frontier = next;
    }
    seen.delete(validatorFile(name));
    helpers.set(name, [...seen].sort());
  }
  const regexesOf = (file: string) => (literalsOf.get(file)?.regexes ?? []).flatMap((pattern) => {
    const regex = phpRegex(pattern);
    return regex === undefined ? [] : [regex];
  });

  const rule = (name: string): UpstreamRule | undefined => {
    const fact = facts.validators[name];
    if (fact === undefined) return undefined;
    const configuredOptions = fact.options !== null && Object.keys(fact.options).length > 0;
    return {
      name,
      implementation: [fromCheckout(validatorFile(name))],
      tests: fact.test === null ? [] : [fact.test],
      deprecation: fact.deprecation,
      recommended: fact.options === null ? null : { config: DEFAULT_CONFIG, options: configuredOptions ? [fact.options] : [] },
      notOnRealCode: null,
    };
  };

  const findingsOf = (result: LintResult, path: string, name: string): Span[] => (result.findings[path]?.[name] ?? []).map(validationSpan);

  return {
    identity: {
      runtime: `stillat/blade-parser ${facts.version} on PHP ${facts.php} with laravel/framework ${facts.framework}, each validator on a document of its own as its test runs it, loaded with the options the package's default configuration (${DEFAULT_CONFIG}, config/validation.php) gives it, on a bare container: Blade's core directives and no custom one`,
      positions: "the start line and column of the node a validation error reports, as stillat/blade-parser numbers them: lines break at \\r\\n, \\r and \\n, and a node's character offset is looked up among the line ends' byte offsets; no end",
      parsers: blade.version,
    },
    packChecks: [
      ...upstream.checks,
      {
        id: "runtime-version",
        ok: facts.version === pack.upstream.version && runtime.version === runtime.pinned,
        detail: `installed stillat/blade-parser is ${facts.version} on PHP ${runtime.version}; the pack pins ${pack.upstream.version} on PHP ${runtime.pinned}`,
      },
      {
        id: "runtime-matches-checkout",
        ok: ["src", "config"].every((directory) => treeDigest(join(facts.package, directory)) === treeDigest(join(upstream.directory, directory))),
        detail: "the installed package's src/ and config/ and those at the pinned commit have the same files and content",
      },
      {
        id: "declared-license",
        ok: facts.license === pack.upstream.license,
        detail: `the installed package declares ${facts.license ?? "no license"}; the pack records ${pack.upstream.license}`,
      },
      {
        id: "upstream-tests-agree",
        ok: disagreements.length === 0,
        detail: disagreements.length === 0
          ? `on every template its test hands the parser, every ported validator, as the default configuration loads it, reports exactly the errors and messages its test expects (${runs.length} test run(s), ${templates.length} template(s))`
          : `${disagreements.length} expectation(s) of upstream's tests differ from the validators' output: ${disagreements.slice(0, 3).join("; ")}`,
      },
    ],
    licenseText: upstreamLicenseText(upstream.directory),
    rule,

    implementationNames(name: string): Set<string> {
      const fact = facts.validators[name];
      if (fact === undefined) return new Set();
      const file = validatorFile(name);
      return new Set([
        ...(literalsOf.get(file)?.names ?? []),
        // What the instantiated validator holds: lists it reads at construction (DebugDirectiveValidator asks the
        // parser's CoreDirectiveRetriever for the debug directives) are read off the object, as Bandit's driver reads
        // the patterns its plugins compile.
        ...fact.names.flatMap((value) => value.match(/[A-Za-z_]\w*/gu) ?? []),
        ...regexesOf(file).flatMap((regex) => regex.source.match(/[A-Za-z_]\w*/gu) ?? []),
      ]);
    },
    sharedImplementationNames(name: string): Set<string> {
      return new Set((helpers.get(name) ?? []).flatMap((path) => literalsOf.get(path)?.names ?? []));
    },
    implementationRegexes(name: string) {
      if (facts.validators[name] === undefined) return [];
      return [validatorFile(name), ...(helpers.get(name) ?? [])].flatMap(regexesOf);
    },
    fixtureWords: () => fixtureWords,
    fixtureTexts: () => fixtureTexts,
    fixtureTextSources: () => fixtureTextSources,
    keywords: blade.keywords,
    commentVocabulary: blade.commentVocabulary,
    // stillat/blade-parser validates Blade templates, `.blade.php` files (`blade:validate` walks the views for them).
    inScopeSamples: ["resources/views/example.blade.php"],

    cases(name: string): UpstreamCase[] {
      if (facts.validators[name] === undefined) return [];
      return templates.map((template) => {
        const fixture = fixtureOf(template);
        const error = configured.errors[fixture];
        const skipped = error === undefined ? null : `upstream could not analyse it: ${error}`;
        return {
          fixture,
          variant: template.validator === name ? "." : template.test,
          // Each template is a Blade template, read where the pack's copy of it is: <test file less .php>/<n>.blade.php.
          path: `${template.test.replace(/\.php$/u, "")}/${template.number}.blade.php`,
          source: template.source,
          skipped,
          expected: skipped === null ? findingsOf(configured, fixture, name) : [],
          // The tests' expectations are counts and messages, which upstream-tests-agree checks; no position to compare.
          markup: null,
        };
      });
    },

    toSpan(_name: string, source: string, startByte: number, endByte: number): Span {
      return bladeParserSpan(source, startByte, endByte, BLADE_PARSER_SPAN_PARTS);
    },

    parserVerdicts: (files) => blade.verdicts(files),
  };
}
