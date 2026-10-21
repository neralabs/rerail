import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { runChecked, type CommandRunner } from "../../../port-readiness/process.js";
import { upstreamLicenseText } from "../../../port-readiness/recorded-adapter.js";
import { lintSpan, tlintSpan, TLINT_SPAN_PARTS } from "../../../port-readiness/span-units/tlint.js";
import type { FixtureTextSource, PackAdapter, PackManifest, Span, UpstreamCase, UpstreamRule } from "../../../port-readiness/types.js";
import { createBladeToolkit } from "../languages/blade.js";
import { phpRegex, type PhpLiterals } from "../languages/php.js";
import { checkoutUpstream, composerRuntime, filesUnder, treeDigest } from "../runtimes.js";

/**
 * The tlint adapter. Expected answers come from running the pinned tlint itself on the templates upstream's own linter
 * tests (tests/Linting/Linters/<Lint>Test.php) hand each Blade linter, each lint with the first recommended preset that
 * turns it on, as `tlint lint <file>` runs a file (LintCommand::lintFile: every linter of the preset that applies to the
 * path, in the preset's order). The tests assert lines (`$lints[0]->getNode()->getLine()`) and `assertEmpty`, never
 * columns, and tlint reports a line only, so tlint's output is the positional answer, and the `upstream-tests-agree`
 * check holds every assertion of every Blade linter's test to the real linter's output, as PHPUnit checks it.
 *
 * A lint's cases are the templates of every Blade linter's test (each linter that uses LintsBladeTemplates), not its
 * own alone: they are Blade templates any Blade lint reads, so another lint's template is upstream's answer on code
 * the rule must stay silent on, as Bandit's examples are for every Bandit test. A template is no file of upstream's:
 * it is the string a test method hands `new <Lint>(...)`, captured as the linter receives it (the driver rewrites the
 * call), so each case is its own source, named `<test file>#<test method>`, as an ESLint case is its RuleTester code.
 * The rule's own templates have variant `.`; another's name the test that owns it.
 */

const DRIVER = fileURLToPath(new URL("./tlint_driver.php", import.meta.url));
/** Upstream's sources and its tests (checkoutUpstream adds its licence). */
const SPARSE_PATHS = ["/src/", "/tests/", "/composer.json"];
/** Where tlint keeps its linter tests, one `<Lint>Test.php` per linter. */
const LINTER_TESTS = "tests/Linting/Linters";

interface LinterTest {
  test: string;
  templates: string[];
  assertions: { assertion: string; ok: boolean }[];
  error: string | null;
}
interface LinterFact {
  file: string;
  blade: boolean;
  names: string[];
  patterns: string[];
  test: string | null;
  tests: LinterTest[];
}
interface Facts {
  version: string;
  license: string | null;
  php: string;
  parser: string;
  package: string;
  linters: Record<string, LinterFact>;
  presets: Record<string, string[]>;
}
interface Group { preset: string | null; lints: string[] }
interface LintResult { lints: Record<string, { lint: string; line: number }[]>; internal: Record<string, string> }

/** A Blade linter's test case before it is linted: its template, the lint whose test hands it over, and where it is read. */
interface Template { fixture: string; owner: string; test: string; path: string; source: string }

export async function createTlintAdapter(pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner): Promise<PackAdapter> {
  // tlint publishes its Composer package from the repository at the release tag, `v<version>`.
  const upstream = await checkoutUpstream(cacheRoot, pack.upstream, `v${pack.upstream.version}`, "src", SPARSE_PATHS, runner);
  const runtime = await composerRuntime(pack.upstream.runtime, cacheRoot, runner);
  const blade = await createBladeToolkit(cacheRoot, runner);
  const ask = async <T>(request: object): Promise<T> =>
    JSON.parse(await runChecked(runner, runtime.command(DRIVER, runtime.directory), { input: JSON.stringify(request), cwd: cacheRoot })) as T;
  const facts = await ask<Facts>({ op: "facts", lints: names, presets: pack.upstream.recommended_configs, checkout: upstream.directory });
  const fromCheckout = (path: string) => relative(upstream.directory, path).split(sep).join("/");

  // Every PHP file of upstream's tests (the linter tests, whose templates are nowdoc strings in them, and the Laravel
  // application its preset test lints): the test words and texts not-overfit reads.
  const testFiles = filesUnder(join(upstream.directory, "tests"), (path) => path.endsWith(".php"));
  const testText = new Map(testFiles.map((path) => [path, readFileSync(path, "utf8")]));
  const fixtureTexts = [...testText.values()];
  const fixtureTextSources: FixtureTextSource[] = [...testText].map(([path, text]) => ({ path: fromCheckout(path), sha256: createHash("sha256").update(text).digest("hex") }));
  const templateFiles = [...testText.keys()].filter((path) => path.endsWith(".blade.php"));
  const phpFiles = [...testText.keys()].filter((path) => !path.endsWith(".blade.php"));
  const fixtureWords = new Set([
    ...(await blade.words(templateFiles.map((path) => testText.get(path)!))).flat(),
    ...(await blade.php.words(phpFiles.map((path) => testText.get(path)!))).flat(),
  ]);

  // The recommended preset that turns each lint on, first in gates.yaml's order, and the groups each lint runs in.
  const ported = Object.keys(facts.linters).filter((lint) => names.includes(lint)).sort();
  const recommendedBy = new Map(ported.flatMap((lint) => {
    const preset = pack.upstream.recommended_configs.find((config) => facts.presets[config]?.includes(lint) === true);
    return preset === undefined ? [] : [[lint, preset] as const];
  }));
  const groups: Group[] = [
    ...pack.upstream.recommended_configs.map((preset) => ({ preset, lints: ported.filter((lint) => recommendedBy.get(lint) === preset) })),
    { preset: null, lints: ported.filter((lint) => !recommendedBy.has(lint)) },
  ].filter((group) => group.lints.length > 0);

  // Every template every Blade linter's test hands its linter, one per test method, in the linters' and methods' order.
  const bladeLinters = Object.keys(facts.linters).filter((lint) => facts.linters[lint]!.blade).sort();
  const disagreements: string[] = [];
  const templates: Template[] = bladeLinters.flatMap((lint) => {
    const fact = facts.linters[lint]!;
    if (fact.test === null) {
      disagreements.push(`the Blade linter ${lint} has no test at ${LINTER_TESTS}/${lint}Test.php`);
      return [];
    }
    const test = fromCheckout(fact.test);
    return fact.tests.flatMap((method) => {
      const failed = method.assertions.filter((assertion) => !assertion.ok).map((assertion) => assertion.assertion);
      if (method.error !== null) disagreements.push(`${test}::${method.test} throws ${method.error}`);
      else if (method.assertions.length === 0) disagreements.push(`${test}::${method.test} asserts nothing`);
      else if (failed.length > 0) disagreements.push(`${test}::${method.test} fails ${failed.join(", ")}`);
      // A method that threw before handing its linter a template has no case; its disagreement above is reported.
      if (method.error !== null && method.templates.length === 0) return [];
      // A case is named by its test method, which hands the linter one template; one handing it more names none.
      if (method.templates.length !== 1) throw new Error(`${test}::${method.test} hands ${lint} ${method.templates.length} templates; a case is one template per test method`);
      return [{ fixture: `${test}#${method.test}`, owner: lint, test, path: `${lint}Test/${method.test}.blade.php`, source: method.templates[0]! }];
    });
  });

  // The templates, written where their cases are read and linted once, each lint with its group.
  const linted = await lintFiles(templates.map((template) => ({ path: template.path, source: template.source })));

  // not-overfit reads upstream's code from the pinned checkout, which the tool-pack checks show is the code that runs.
  // A linter's file at the pinned checkout: the installed file's path within the package, which the checks show is the same tree.
  const linterFile = (lint: string) => join(upstream.directory, relative(facts.package, facts.linters[lint]!.file));
  const literalsOf = new Map<string, PhpLiterals>();
  const readLiterals = async (files: string[]) => {
    const fresh = [...new Set(files)].filter((file) => !literalsOf.has(file));
    for (const [index, found] of (await blade.literals(fresh)).entries()) literalsOf.set(fresh[index]!, found);
  };
  /** The checkout file of a tlint class (`Tighten\TLint\BaseLinter` is src/BaseLinter.php), when it is one. */
  const classFile = (name: string) => (name.startsWith("Tighten\\TLint\\") ? join(upstream.directory, "src", `${name.slice("Tighten\\TLint\\".length).replaceAll("\\", "/")}.php`) : null);
  // A lint's helpers: the classes and traits its file imports and the classes it extends, and theirs in turn, within upstream's src/.
  const helpers = new Map<string, string[]>();
  await readLiterals(ported.map(linterFile));
  for (const lint of ported) {
    const seen = new Set<string>([linterFile(lint)]);
    for (let frontier = [linterFile(lint)]; frontier.length > 0;) {
      const next = [...new Set(frontier.flatMap((file) => {
        const found = literalsOf.get(file);
        return [...(found?.imports ?? []), ...(found?.classes ?? []).flatMap((declared) => (declared.extends === null ? [] : [declared.extends]))];
      }).map(classFile))].filter((path): path is string => path !== null && !seen.has(path) && existsSync(path));
      for (const path of next) seen.add(path);
      await readLiterals(next);
      frontier = next;
    }
    seen.delete(linterFile(lint));
    helpers.set(lint, [...seen].sort());
  }
  const regexesOf = (patterns: string[]) => patterns.flatMap((pattern) => {
    const regex = phpRegex(pattern);
    return regex === undefined ? [] : [regex];
  });
  // The patterns a linter matches with: those it passes to preg_* as literals, and those its class holds (the
  // LintsBladeTemplates searches it passes as `self::$directiveSearch`).
  const ownRegexes = (lint: string) => [...regexesOf(literalsOf.get(linterFile(lint))?.regexes ?? []), ...regexesOf(facts.linters[lint]!.patterns)];

  const rule = (name: string): UpstreamRule | undefined => {
    const fact = facts.linters[name];
    if (fact === undefined || !ported.includes(name)) return undefined;
    const preset = recommendedBy.get(name);
    return {
      name,
      implementation: [fromCheckout(linterFile(name))],
      tests: fact.test === null ? [] : [fromCheckout(fact.test)],
      // tlint deprecates no linter: it removes them in a major release (readme: the 7.x release removed those Duster covers).
      deprecation: null,
      // A preset is a list of linters; none takes an option.
      recommended: preset === undefined ? null : { config: preset, options: [] },
      notOnRealCode: null,
    };
  };

  const findingsOf = (result: LintResult, path: string, lint: string): Span[] =>
    (result.lints[path] ?? []).filter((found) => found.lint === lint).map(lintSpan);

  /** Upstream's answer on each file, written under a temporary directory at its relative path and linted from there. */
  async function lintFiles(files: { path: string; source: string }[]): Promise<LintResult> {
    if (files.length === 0 || groups.length === 0) return { lints: {}, internal: {} };
    await mkdir(resolve(cacheRoot, "tmp"), { recursive: true });
    const directory = await mkdtemp(resolve(cacheRoot, "tmp", "tlint-"));
    try {
      await Promise.all(files.map(async (file) => {
        await mkdir(dirname(join(directory, file.path)), { recursive: true });
        await writeFile(join(directory, file.path), file.source);
      }));
      // Linted by relative path from the directory, as `tlint lint` lints a project's files: the path a linter's
      // appliesToPath reads and every message are the file's own, never this machine's.
      return await ask<LintResult>({ op: "lint", cwd: directory, groups, files: files.map((file) => file.path) });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  }

  return {
    identity: {
      runtime: `tlint ${facts.version} on PHP ${facts.php} (nikic/php-parser ${facts.parser}), each lint with the first recommended preset that turns it on, as \`tlint lint <file>\` runs a file (every linter of the preset that applies to the path, in its order)`,
      positions: "the line tlint reports (its node's start line: 1-based, lines split at \\r\\n, \\r and \\n), and no column or end",
      parsers: blade.version,
    },
    packChecks: [
      ...upstream.checks,
      {
        id: "runtime-version",
        ok: facts.version === pack.upstream.version && runtime.version === runtime.pinned,
        detail: `installed tlint is ${facts.version} on PHP ${runtime.version}; the pack pins ${pack.upstream.version} on PHP ${runtime.pinned}`,
      },
      {
        id: "runtime-matches-checkout",
        ok: treeDigest(join(facts.package, "src")) === treeDigest(join(upstream.directory, "src")),
        detail: "the installed package's src/ and src/ at the pinned commit have the same files and content",
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
          ? `every test method of every Blade linter's test (${bladeLinters.length} linter(s), ${templates.length} template(s)) passes against the installed tlint, each assertion checked as PHPUnit checks it`
          : `${disagreements.length} test method(s) fail: ${disagreements.slice(0, 3).join("; ")}`,
      },
    ],
    licenseText: upstreamLicenseText(upstream.directory),
    rule,

    implementationNames(name: string): Set<string> {
      if (rule(name) === undefined) return new Set();
      return new Set([
        ...(literalsOf.get(linterFile(name))?.names ?? []),
        // What the linter class holds: its constants (SpaceAfterBladeDirectives::SPACE_AFTER lists the directives it
        // checks) and its static properties (the searches LintsBladeTemplates gives it).
        ...facts.linters[name]!.names.flatMap((value) => value.match(/[A-Za-z_]\w*/gu) ?? []),
        ...ownRegexes(name).flatMap((regex) => regex.source.match(/[A-Za-z_]\w*/gu) ?? []),
      ]);
    },
    sharedImplementationNames(name: string): Set<string> {
      return new Set((helpers.get(name) ?? []).flatMap((path) => literalsOf.get(path)?.names ?? []));
    },
    implementationRegexes(name: string) {
      if (rule(name) === undefined) return [];
      return [...ownRegexes(name), ...(helpers.get(name) ?? []).flatMap((path) => regexesOf(literalsOf.get(path)?.regexes ?? []))];
    },
    fixtureWords: () => fixtureWords,
    fixtureTexts: () => fixtureTexts,
    fixtureTextSources: () => fixtureTextSources,
    keywords: blade.keywords,
    commentVocabulary: blade.commentVocabulary,
    // tlint's Blade linters lint a file whose path holds `.blade.php` (LintsBladeTemplates::appliesToPath).
    inScopeSamples: ["resources/views/example.blade.php"],

    cases(name: string): UpstreamCase[] {
      if (rule(name) === undefined) return [];
      return templates.map((template) => {
        const internal = linted.internal[template.path];
        const skipped = internal === undefined ? null : `upstream could not analyse it: ${internal}`;
        return {
          fixture: template.fixture,
          variant: template.owner === name ? "." : template.test,
          path: template.path,
          source: template.source,
          skipped,
          expected: skipped === null ? findingsOf(linted, template.path, name) : [],
          // The tests assert lines of the lints they index, which upstream-tests-agree checks; none is a complete list.
          markup: null,
        };
      });
    },

    toSpan(_name: string, source: string, startByte: number, endByte: number): Span {
      return tlintSpan(source, startByte, endByte, TLINT_SPAN_PARTS);
    },

    // `tlint lint <directory>` lints its .php files, and the Blade linters those whose path holds `.blade.php`.
    inCorpus: (path: string) => path.endsWith(".blade.php"),

    async lintCorpus(files, ruleNames) {
      const paths = files.map((file) => `${file.task}/${file.path}`);
      const result = await lintFiles(files.map((file, index) => ({ path: paths[index]!, source: file.source })));
      return files.map((file, index) => {
        const internal = result.internal[paths[index]!];
        if (internal !== undefined) return { file, excluded: `upstream could not analyse it: ${internal}`, findings: new Map() };
        return { file, excluded: null, findings: new Map(ruleNames.map((name) => [name, findingsOf(result, paths[index]!, name)])) };
      });
    },

    parserVerdicts: (files) => blade.verdicts(files),
  };
}
