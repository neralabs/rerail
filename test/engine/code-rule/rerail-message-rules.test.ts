/**
 * The rerail pack (packs/rerail/): its rules over rule pack messages, each on the messages it must catch, with the word
 * its steer names, and the near misses it must let through. pack.json's examples, which the pack tests prove, give
 * one of each; these are the rest: quoted code, option names, the rest of a document, nesting, minified ports, probes.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { languageFor } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../src/engine/native/native.js";
import { loadPackRuleDocuments } from "../../../src/engine/catalog/rule-documents.js";
import { repositoryPath } from "../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = readFileSync(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const documents = loadPackRuleDocuments([repositoryPath("packs", "rerail")]);

const evaluationSchema = z.object({
  violations: z.array(z.object({ code: z.string(), path: z.string(), message: z.string() })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function compile(id: string): NativeCompiledCodeRuleDsl {
  const document = documents.find(candidate => candidate.rule.id === id);
  if (document === undefined) throw new Error(`${id} is not in packs/rerail`);
  return runtime.compile(JSON.stringify(document.rule), "json", capabilities);
}

/** The messages of `compiled`'s violations in a one-file repository holding `content` at `path`. */
function steers(compiled: NativeCompiledCodeRuleDsl, path: string, content: string): string[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: languageFor(path) ?? "text", path, source: content }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.message);
}

interface Extra {
  minified?: boolean;
  code?: string;
  pattern?: string;
  description?: string;
  suppress?: Record<string, unknown>;
  options?: Record<string, unknown>;
}

/** A rule document of another pack whose one violation carries `message`, written as `minified` ports are or pretty. */
function document(id: string, message: unknown, extra: Extra = {}): string {
  const violation = { code: extra.code ?? "X", anchor: "witness.span", message, ...(extra.suppress === undefined ? {} : { suppress: extra.suppress }) };
  const clauses: unknown[] = [{ id: "c", forbid: { regex: { source: "file.content", pattern: extra.pattern ?? "requests\\.get\\(" } }, violation }];
  if (extra.options !== undefined) clauses.unshift({ id: "options", define: { options: extra.options } });
  const rule = {
    format: "rerail.policy/v1",
    id,
    revision: 1,
    mode: "steer",
    description: extra.description ?? "Pass a timeout to every requests call.",
    scope: { include: ["**/*.py"], exclude: [] },
    clauses,
  };
  return extra.minified === true ? JSON.stringify(rule) : `${JSON.stringify(rule, null, 2)}\n`;
}

/** eslint-sort-keys' message shape: a `natural` choice inside an `order` choice inside a `caseSensitive` one. */
function sortKeys(last: string): unknown {
  const order = (direction: string, text: string) => ({ $option: "natural", cases: { true: `Sort these keys in natural ${direction} order.` }, default: text });
  return {
    $option: "caseSensitive",
    cases: {
      false: { $option: "order", cases: { asc: order("ascending", "Sort these keys in ascending order.") }, default: order("descending", "Sort these keys in descending order.") },
    },
    default: { $option: "order", cases: { desc: order("descending", "Sort these keys in descending order.") }, default: order("ascending", last) },
  };
}

/** Each rule's cases: a caught one names the word its steer quotes; a probe path is outside the rule's scope. */
const RULES: Record<string, { caught: [string, string, string][]; passed: [string, string][] }> = {
  "rerail-message-tool-names": {
    caught: [
      // The old bare message: a tool name and a rule code, nothing to do.
      ["packs/python/rules/bandit-b101-assert-used.json", document("bandit-b101-assert-used", "Bandit B101 (assert-used)"), "Bandit"],
      // The rule's own id, pasted after the message as the block would show it.
      ["packs/python/rules/bandit-b113-request-without-timeout.json",
        document("bandit-b113-request-without-timeout", "Pass a timeout to this requests call [bandit-b113-request-without-timeout]."), "bandit"],
      // A case of a message an option chooses, in a minified port.
      ["packs/javascript/rules/eslint-no-console.json",
        document("eslint-no-console", { $option: "allow", cases: { '["warn"]': "Use the logger; ESLint allows only console.warn here." }, default: "Use the logger instead of console." }, { minified: true }),
        "ESLint"],
      // The last default of eslint-sort-keys' message, after a case three objects deep.
      ["packs/javascript/rules/eslint-sort-keys.json", document("eslint-sort-keys", sortKeys("Sort these keys in ascending order, as Biome does."), { minified: true }), "Biome"],
      // A tool name after two quoted spans.
      ["packs/javascript/rules/eslint-no-console-levels.json", document("eslint-no-console-levels", "Pass 'warn' or 'error', the levels ESLint accepts."), "ESLint"],
      // A hedge that names the tool: the steer asks for "this rule", not for the hedge to go.
      ["packs/python/rules/ruff-up007-non-pep604-annotation.json",
        document("ruff-up007-non-pep604-annotation", "Ruff can't see your target version; if it is below 3.10, keep this Union."), "Ruff"],
      // The tool's full name, and a name right after a JSON escape.
      ["packs/php/rules/phpcs-no-silenced-errors.json",
        document("phpcs-no-silenced-errors", "PHP_CodeSniffer flags this silenced call; handle the error instead of using @."), "PHP_CodeSniffer"],
      ["packs/javascript/rules/eslint-max-len.json", document("eslint-max-len", "Keep this on one line.\nESLint reports the rest."), "ESLint"],
    ],
    passed: [
      // The same hedge as "this rule", and a tool name in code the change removes.
      ["packs/python/rules/ruff-up007-non-pep604-annotation.json",
        document("ruff-up007-non-pep604-annotation", "This rule can't see your target version; if it is below 3.10, keep this `Union`.")],
      ["packs/php/rules/phpcs-no-unused-ignore.json", document("phpcs-no-unused-ignore", "Remove this unused `phpcs:ignore` comment; this rule found nothing on the line it silences.")],
      // Tool names everywhere but the message: the id, the clause's pattern, the code and suppress, and an option's
      // default holding a "message" of its own.
      ["packs/javascript/rules/eslint-no-restricted-imports.json",
        document("eslint-no-restricted-imports", "Import this from the module the project allows instead.", {
          minified: true,
          code: "ESLINT_NO_RESTRICTED_IMPORTS",
          pattern: "eslint|biome",
          suppress: { eslint: { rules: ["no-restricted-imports"] }, biome: { rules: ["lint/style/noRestrictedImports"] } },
          options: { paths: { type: "array", default: [{ name: "lodash", message: "ESLint users import lodash-es." }] } },
        })],
      // An option named after the tool's setting.
      ["packs/python/candidates/rules/ruff-a005-stdlib-module-shadowing.json",
        document("ruff-a005-stdlib-module-shadowing", {
          $option: "flake8-builtins.strict-checking",
          cases: { true: "Rename this module; at any depth it shadows a standard-library module." },
          default: "Rename this top-level module; it shadows a standard-library module.",
        })],
      // Quoted code may hold any word.
      ["packs/javascript/rules/eslint-no-unused-vars.json",
        document("eslint-no-unused-vars", "Prefix `$NAME` with `_`, or remove it; keep an `// eslint-disable-next-line` only for generated code.")],
      ["packs/python/rules/ruff-pgh004-blanket-noqa.json", document("ruff-pgh004-blanket-noqa", "Name the code in this `# noqa: <code>` comment, such as `# noqa: E501` for `ruff`.")],
      // The near miss on main: an option value in single quotes, and the same word as a case key.
      ["packs/javascript/rules/eslint-no-warning-comments.json",
        document("eslint-no-warning-comments", { $option: "terms", cases: { '["eslint"]': "Unexpected 'eslint' comment: '$CT'." }, default: "Unexpected 'fixme' comment: '$CT'." }, { minified: true })],
      // An apostrophe before a quoted span.
      ["packs/javascript/rules/eslint-no-restricted-syntax.json", document("eslint-no-restricted-syntax", "Don't add 'eslint' to this list; it's a reserved key.")],
      // A lone apostrophe ('static) does not pair with the quote that opens 'biome'.
      ["packs/rust/candidates/rules/clippy-redundant-static-lifetimes.json", document("clippy-redundant-static-lifetimes", "Remove this 'static; keep 'biome' as the option value.")],
      // A probe is not a rule people read.
      ["packs/python/candidates/probes/rules/bandit-b101-assert-used.json", document("bandit-b101-assert-used", "Bandit B101 (assert-used)")],
    ],
  },
  "rerail-message-rerail-name": {
    caught: [
      ["packs/python/rules/ruff-s113-request-without-timeout.json",
        document("ruff-s113-request-without-timeout", "Rerail found a request without a timeout; pass `timeout=`."), "Rerail"],
      // In lower case, in the default of a message an option chooses.
      ["packs/python/rules/bandit-b602-subprocess-popen-with-shell-equals-true.json",
        document("bandit-b602-subprocess-popen-with-shell-equals-true", {
          $option: "strict",
          cases: { true: "Pass the command as a list of arguments instead of `shell=True`." },
          default: "Pass a list of arguments instead of `shell=True`, so rerail stops reporting it.",
        }), "rerail"],
      // A hedge that names Rerail: the steer asks for "this rule", not for the hedge to go.
      ["packs/php/candidates/rules/php-count-in-loop-condition.json",
        document("php-count-in-loop-condition", "Rerail found no write to `$V` in this loop; if `$V` grows inside it, keep the call here, otherwise hoist it."), "Rerail"],
      // Code left unquoted: the steer asks to quote it.
      ["packs/php/rules/phpcs-no-silenced-errors.json",
        document("phpcs-no-silenced-errors", "Add a rerail-ignore comment with a reason where the silenced call is intended."), "rerail"],
    ],
    passed: [
      ["packs/php/candidates/rules/php-count-in-loop-condition.json",
        document("php-count-in-loop-condition", "This rule found no write to `$V` in this loop; if `$V` grows inside it, keep the call here, otherwise hoist it.")],
      // Quoted code may hold it.
      ["packs/php/rules/phpcs-no-silenced-errors.json", document("phpcs-no-silenced-errors", "Add a `// rerail-ignore <id> -- <reason>` comment only where silencing is intended.")],
      // The name everywhere but the message: the description, and an option's default holding a "message" of its own.
      ["packs/complexity/rules/complexity-cyclomatic-python.json",
        document("complexity-cyclomatic-python", "Split `$NAME` into smaller functions, each with one job.", {
          description: "Keep Python functions at cyclomatic complexity 6 or lower; Rerail counts a match once.",
          options: { notes: { type: "array", default: [{ message: "Rerail counts each match once." }] } },
        })],
      // A probe is not a rule people read.
      ["packs/python/candidates/probes/rules/ruff-s113-request-without-timeout.json", document("ruff-s113-request-without-timeout", "Rerail found a request without a timeout.")],
    ],
  },
};

describe("rerail rule pack", () => {
  it("has a test for each rule", () => {
    expect(documents.map(candidate => candidate.rule.id).sort()).toEqual(Object.keys(RULES).sort());
  });

  describe.each(Object.entries(RULES))("%s", (id, { caught, passed }) => {
    const compiled = compile(id);

    it.each(caught)("catches %s, naming the word", (path, content, word) => {
      expect(compiled.inScope(path)).toBe(true);
      const found = steers(compiled, path, content);
      expect(found).toHaveLength(1);
      expect(found[0]).toContain(`\`${word}\``);
    });

    it.each(passed)("lets %s through", (path, content) => {
      expect(steers(compiled, path, content)).toEqual([]);
    });
  });

  it("scopes out the probes under candidates/probes/", () => {
    for (const id of Object.keys(RULES)) expect(compile(id).inScope("packs/python/candidates/probes/rules/bandit-b101-assert-used.json")).toBe(false);
  });
});
