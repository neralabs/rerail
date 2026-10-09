import { readFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import type { UpstreamRule, UpstreamSource } from "../languages/pack.js";
import { pythonRegex, type PythonLiterals, type PythonToolkit } from "./python-toolkit.js";
import { distinctRegexes, filesUnder, run, sortedSet, treeDigest, unscoredTests, upstreamPath, type UpstreamPin } from "./upstream-source.js";

/**
 * Bandit's source facts (`upstream/bandit/source.json`), read from Bandit's checkout at the pinned commit and from the
 * pinned Bandit's own registry, run as bare `bandit` (no profile or config file, every registered test with its
 * gen_config defaults, `# nosec` honoured). A test's tests are the examples Bandit reports it on: its functional tests
 * assert only severity counts per example, so Bandit's output on the examples is the only positional answer upstream
 * gives. Ported from nera-rule-validation's src/validation/adapters/bandit.ts. An example the fixtures recording does
 * not copy is `unscored`.
 */

const DRIVER = fileURLToPath(new URL("./bandit_driver.py", import.meta.url));
/** Bandit's helpers every plugin reads the tree through. */
const SHARED_MODULES = ["bandit/core/context.py", "bandit/core/utils.py", "bandit/core/blacklisting.py", "bandit/blacklists/utils.py"];

interface RuleFact {
  kind: "plugin" | "blacklist";
  name: string;
  module: string;
  file: string;
  options: unknown;
  qualnames?: string[];
}
interface Facts {
  version: string;
  rules: Record<string, RuleFact>;
  regexes: Record<string, { pattern: string; flags: number }[]>;
  nosec: string;
}
interface LintResult {
  issues: Record<string, { test_id: string; line: number }[]>;
  skipped: Record<string, string>;
}

/**
 * Bandit's facts for the ported test ids, from `checkout` (Bandit's repository at `pin.commit`) and `python`, the
 * interpreter of the environment the pinned Bandit is installed in.
 */
export function banditSource(
  checkout: string,
  python: string,
  toolkit: PythonToolkit,
  ids: string[],
  pin: UpstreamPin & { version: string },
  configurations: string[],
  recorded: readonly string[],
): UpstreamSource {
  if (configurations.join(",") !== "default") {
    throw new Error("Bandit has no named configurations; tools[].recommended must be [default], meaning bare `bandit`");
  }
  const ask = <T>(request: object): T => JSON.parse(run(python, ["-I", "-B", DRIVER], { input: JSON.stringify(request) })) as T;
  const facts = ask<Facts>({ op: "facts" });
  if (facts.version !== pin.version) throw new Error(`${python} runs Bandit ${facts.version}; the pack pins ${pin.version}`);
  const installed = dirname(dirname(Object.values(facts.rules)[0]!.file));
  const withoutCaches = (directory: string) => treeDigest(directory, path => !path.includes("__pycache__"));
  if (withoutCaches(installed) !== withoutCaches(join(checkout, "bandit"))) {
    throw new Error(`the installed bandit package (${installed}) differs from bandit/ at ${pin.commit}`);
  }

  // The facts read upstream's code from the pinned checkout, which the check above shows is the code that runs.
  const fromCheckout = (file: string) => join(checkout, relative(dirname(installed), file));
  const examples = filesUnder(join(checkout, "examples"), path => path.endsWith(".py"));
  const examplesRun = ask<LintResult>({ op: "lint", files: examples });

  const moduleFiles = [...new Set(Object.values(facts.rules).map(rule => fromCheckout(rule.file)))];
  const sharedFiles = SHARED_MODULES.map(path => join(checkout, path));
  const literalFiles = [...moduleFiles, ...sharedFiles];
  const found = toolkit.literals(literalFiles);
  const literals = new Map<string, PythonLiterals>(literalFiles.map((file, index) => [file, found[index]!]));
  const moduleOfFile = (file: string) => upstreamPath(checkout, file).replace(/\.py$/u, "").replaceAll("/", ".");
  const importedFiles = (file: string) =>
    (literals.get(file)?.imports ?? []).map(module => join(checkout, `${module.replaceAll(".", "/")}.py`)).filter(path => path !== file && literals.has(path));
  const regexesOf = (file: string) => (facts.regexes[moduleOfFile(file)] ?? []).flatMap(({ pattern, flags }) => pythonRegex(pattern, flags) ?? []);
  const namesOf = (files: string[]) => files.flatMap(path => literals.get(path)?.names ?? []);

  const sharedNames = sortedSet(namesOf(sharedFiles));
  const everyRuleShares = new Set(sharedNames);

  // The recording's copies, by the upstream path they were copied from.
  const copied = new Set(recorded.map(file => `examples/${file}`));
  const rule = (id: string): UpstreamRule => {
    const fact = facts.rules[id]!;
    const file = fromCheckout(fact.file);
    const delegated = [...new Set([...importedFiles(file), ...sharedFiles])];
    const tests = sortedSet(examples.filter(path => (examplesRun.issues[path] ?? []).some(issue => issue.test_id === id)).map(path => upstreamPath(checkout, path)));
    return {
      implementation: [upstreamPath(checkout, file)],
      tests,
      // Bandit has no deprecation flag; a removed test is unregistered.
      deprecated: null,
      enabledBy: { configuration: "default", options: fact.options === null ? [] : [fact.options] },
      names: {
        implementation: sortedSet([
          ...namesOf([file]),
          ...(fact.qualnames ?? []).flatMap(qualname => qualname.split(".")),
          ...regexesOf(file).flatMap(regex => regex.source.match(/[A-Za-z_]\w*/gu) ?? []),
        ]),
        shared: sortedSet(namesOf(delegated).filter(name => !everyRuleShares.has(name))),
      },
      regexes: distinctRegexes([...regexesOf(file), ...delegated.flatMap(regexesOf)]),
      unscored: unscoredTests(tests.filter(test => !copied.has(test)).map(test => ({ test, case: null, reason: "not-copied" }))),
    };
  };

  // A ported id Bandit does not register gets no entry, which check 7 reports as provenance:upstream-rule.
  const known = sortedSet(ids.filter(id => facts.rules[id] !== undefined));
  return {
    tool: "bandit",
    version: pin.version,
    repository: pin.repository,
    commit: pin.commit,
    tag: pin.tag,
    configurations,
    // Bandit lints .py files, as the rule validation ladder's V1 probes it.
    extensions: [".py"],
    keywords: sortedSet(toolkit.keywords),
    // Bandit honours `# nosec`, read from its pinned manager.
    commentVocabulary: sortedSet([...toolkit.commentVocabulary, ...(facts.nosec.match(/[A-Za-z_]\w*/gu) ?? []).filter(word => word !== "P" && word !== "tests")]),
    testWords: sortedSet(toolkit.words(examples.map(path => readFileSync(path, "utf8")))),
    configurationWords: [],
    sharedNames,
    rules: Object.fromEntries(known.map(id => [id, rule(id)])),
  };
}
