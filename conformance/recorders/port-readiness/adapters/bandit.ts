import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { runChecked, type CommandRunner } from "../../../port-readiness/process.js";
import { decodeSource, upstreamLicenseText } from "../../../port-readiness/recorded-adapter.js";
import { banditSpan, banditSpanParts, TROJANSOURCE_RELEASE, trojansourcePoint } from "../../../port-readiness/span-units/bandit.js";
import type { FixtureTextSource, PackAdapter, PackManifest, Span, UpstreamCase, UpstreamRule } from "../../../port-readiness/types.js";
import { createPythonToolkit, pythonRegex } from "../languages/python.js";
import { checkoutUpstream, filesUnder, treeDigest, uvRuntime } from "../runtimes.js";

/**
 * The Bandit adapter. Expected answers come from running the pinned Bandit itself, as bare `bandit <files>` runs
 * (no profile, no config file, `# nosec` honoured, every registered test with its gen_config defaults), on every one of
 * its own example files at the pinned commit. Bandit's functional tests assert only severity counts per example, so
 * Bandit's output on the examples is the only positional answer upstream gives.
 */

const DRIVER = fileURLToPath(new URL("./bandit_driver.py", import.meta.url));
const SPARSE_PATHS = ["/bandit/", "/examples/", "/setup.cfg"];
/** Bandit's helpers every plugin reads the tree through. */
const SHARED_MODULES = ["bandit/core/context.py", "bandit/core/utils.py", "bandit/core/blacklisting.py", "bandit/blacklists/utils.py"];

interface Issue { test_id: string; line: number; col: number; end_col: number; line_range: number[] }
interface LintResult { issues: Record<string, Issue[]>; skipped: Record<string, string> }
interface RuleFact { kind: "plugin" | "blacklist"; name: string; module: string; file: string; options: unknown; qualnames?: string[] }
interface Facts {
  version: string;
  license: string;
  python: string;
  rules: Record<string, RuleFact>;
  regexes: Record<string, { pattern: string; flags: number }[]>;
  nosec: string;
}

export async function createBanditAdapter(pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner): Promise<PackAdapter> {
  if (pack.upstream.recommended_configs.join(",") !== "default") {
    throw new Error(`${pack.id}: Bandit has no named configurations; recommended_configs must be [default], meaning bare \`bandit\``);
  }
  // Bandit publishes its PyPI package from bandit/ at the release tag, which is the bare version.
  const upstream = await checkoutUpstream(cacheRoot, pack.upstream, pack.upstream.version, "bandit", SPARSE_PATHS, runner);
  const runtime = await uvRuntime(pack.upstream.runtime, cacheRoot, runner);
  const python = await createPythonToolkit(cacheRoot, runner);
  const ask = async <T>(request: object): Promise<T> =>
    JSON.parse(await runChecked(runner, [runtime.python, "-I", DRIVER], { input: JSON.stringify(request), cwd: cacheRoot })) as T;
  const facts = await ask<Facts>({ op: "facts" });
  const installed = join(dirname(dirname(facts.rules.B101?.file ?? Object.values(facts.rules)[0]!.file)));
  const withoutCaches = (directory: string) => treeDigest(directory, (path) => !path.includes("__pycache__"));

  const fromCheckout = (file: string) => join(upstream.directory, relative(dirname(installed), file));
  const examples = filesUnder(join(upstream.directory, "examples"), (path) => path.endsWith(".py"));
  const exampleText = new Map(examples.map((path) => [path, readFileSync(path, "utf8")]));
  // A case's source as Bandit reads the file and as the gates read its committed copy (decodeSource): the Latin-1 a PEP
  // 263 coding line declares (trojansource_latin1.py), a leading byte order mark kept. The parsers' verdict on it is
  // keyed by that text, so the recorder and the gates find it. The fixture texts stay UTF-8, as rebuildFixtureTexts
  // reads upstream's files.
  const caseText = new Map(examples.map((path) => [path, decodeSource(readFileSync(path))]));
  const examplesRun = await ask<LintResult>({ op: "lint", files: examples });

  const idOf = (name: string) => name.toUpperCase();
  if (facts.version !== TROJANSOURCE_RELEASE && names.some((name) => idOf(name) === "B613")) {
    throw new Error(`${pack.id}: B613's line numbering is read from Bandit ${TROJANSOURCE_RELEASE}, not ${facts.version}; read bandit/plugins/trojansource.py at the pin and update trojansourcePoint`);
  }
  const toUpstreamSpan = (id: string, source: string, issue: Issue): Span => {
    if (id === "B613") {
      const point = trojansourcePoint(source, issue.line, issue.col);
      return { startLine: point.line, startCol: point.column, endLine: null, endCol: null };
    }
    // Bandit reports a start line, a start column and an end column, but no end line: line_range is the node's
    // context range, which for a string is its parent's. The end column is compared without a line. Every part is
    // compared as Bandit reports it (ADR 0006, "Bandit's keyword lines"), including a Call check's keyword argument
    // line, which bandit/core/tester.py pairs with the call's columns.
    return { startLine: issue.line - 1, startCol: issue.col, endLine: null, endCol: issue.end_col };
  };

  // not-overfit reads upstream's code from the pinned checkout, which the tool-pack checks show is the code that runs.
  const moduleFiles = [...new Set(Object.values(facts.rules).map((rule) => fromCheckout(rule.file)))];
  const sharedFiles = SHARED_MODULES.map((path) => join(upstream.directory, path));
  const literals = new Map((await python.literals([...moduleFiles, ...sharedFiles])).map((found, index) => [[...moduleFiles, ...sharedFiles][index]!, found]));
  const moduleOfFile = (file: string) => relative(upstream.directory, file).replace(/\.py$/u, "").replaceAll("/", ".");
  const importedFiles = (file: string) => (literals.get(file)?.imports ?? [])
    .map((module) => join(upstream.directory, `${module.replaceAll(".", "/")}.py`))
    .filter((path) => path !== file && literals.has(path));
  const regexesOf = (file: string) => (facts.regexes[moduleOfFile(file)] ?? []).flatMap(({ pattern, flags }) => {
    const regex = pythonRegex(pattern, flags);
    return regex === undefined ? [] : [regex];
  });

  const fixtureTexts = [...exampleText.values()];
  const fixtureTextSources: FixtureTextSource[] = [...exampleText].map(([path, text]) => ({
    path: relative(upstream.directory, path).split(sep).join("/"),
    sha256: createHash("sha256").update(text).digest("hex"),
  }));
  const fixtureWords = new Set((await python.words(fixtureTexts)).flat());

  const rule = (name: string): UpstreamRule | undefined => {
    const id = idOf(name);
    const fact = facts.rules[id];
    if (fact === undefined) return undefined;
    return {
      name: id,
      implementation: [relative(upstream.directory, fromCheckout(fact.file))],
      tests: examples.filter((path) => (examplesRun.issues[path] ?? []).some((issue) => issue.test_id === id)).map((path) => relative(upstream.directory, path)),
      // Bandit has no deprecation flag; a removed test is unregistered.
      deprecation: null,
      recommended: { config: "default", options: fact.options === null ? [] : [fact.options] },
      notOnRealCode: null,
    };
  };

  return {
    identity: {
      // How Bandit ran and how its positions are read both decide verdicts a port cannot follow (ADR 0006).
      runtime: `bandit ${facts.version}, CPython ${facts.python}, run as bare \`bandit\` (no profile or config file, every registered test with its defaults, \`# nosec\` honoured)`,
      positions: "line, column and end column exactly as Bandit reports them, including a Call check that reports a keyword argument's line with the call's columns",
      parsers: `CPython ${python.version.replace(/^Python /u, "")} and libcst`,
    },
    packChecks: [
      ...upstream.checks,
      {
        id: "runtime-version",
        ok: facts.version === pack.upstream.version && runtime.version === `Python ${runtime.pinned}`,
        detail: `installed bandit is ${facts.version} on ${runtime.version}; the pack pins ${pack.upstream.version} on Python ${runtime.pinned}`,
      },
      {
        id: "runtime-matches-checkout",
        ok: withoutCaches(installed) === withoutCaches(join(upstream.directory, "bandit")),
        detail: "the installed bandit package and bandit/ at the pinned commit have the same files and content",
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
      const fact = facts.rules[idOf(name)];
      if (fact === undefined) return new Set();
      const file = fromCheckout(fact.file);
      return new Set([
        ...(literals.get(file)?.names ?? []),
        ...(fact.qualnames ?? []).flatMap((qualname) => qualname.split(".")),
        ...regexesOf(file).flatMap((regex) => regex.source.match(/[A-Za-z_]\w*/gu) ?? []),
      ]);
    },
    sharedImplementationNames(name: string): Set<string> {
      const fact = facts.rules[idOf(name)];
      if (fact === undefined) return new Set();
      const file = fromCheckout(fact.file);
      return new Set([...new Set([...importedFiles(file), ...sharedFiles])].flatMap((path) => literals.get(path)?.names ?? []));
    },
    implementationRegexes(name: string) {
      const fact = facts.rules[idOf(name)];
      if (fact === undefined) return [];
      const file = fromCheckout(fact.file);
      return [...regexesOf(file), ...[...new Set([...importedFiles(file), ...sharedFiles])].flatMap(regexesOf)];
    },
    fixtureWords: () => fixtureWords,
    fixtureTexts: () => fixtureTexts,
    fixtureTextSources: () => fixtureTextSources,
    keywords: python.keywords,
    // Bandit honours `# nosec`, read from its pinned manager.
    commentVocabulary: new Set([...python.commentVocabulary, ...(facts.nosec.match(/[A-Za-z_]\w*/gu) ?? []).filter((word) => word !== "P" && word !== "tests")]),
    inScopeSamples: ["src/example.py"],

    cases(name: string): UpstreamCase[] {
      const id = idOf(name);
      return examples.map((path) => {
        const fixture = relative(upstream.directory, path);
        const source = caseText.get(path)!;
        const skipped = examplesRun.skipped[path];
        return {
          fixture,
          variant: ".",
          path: fixture,
          source,
          skipped: skipped === undefined ? null : `upstream skips it: ${skipped}`,
          expected: skipped === undefined ? (examplesRun.issues[path] ?? []).filter((issue) => issue.test_id === id).map((issue) => toUpstreamSpan(id, source, issue)) : [],
          // Bandit's tests assert severity counts per file, not positions.
          markup: null,
        };
      });
    },

    toSpan(name: string, source: string, startByte: number, endByte: number): Span {
      return banditSpan(source, startByte, endByte, banditSpanParts(idOf(name)));
    },

    parserVerdicts: (files) => python.verdicts(files),
  };
}
