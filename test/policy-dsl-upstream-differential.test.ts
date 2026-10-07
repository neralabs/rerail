import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";
import { afterAll, describe, expect, it } from "vitest";
import { loadNativeBinding } from "../src/native.js";
import { UPSTREAM_FIELD_CASES } from "./fixtures/upstream-field-cases.js";
import { UPSTREAM_NTH_CHILD_CASES } from "./fixtures/upstream-nth-child-cases.js";
import { UPSTREAM_RELATIONAL_CASES, type UpstreamCase } from "./fixtures/upstream-relational-cases.js";
import { UPSTREAM_STOP_BY_CASES } from "./fixtures/upstream-stop-by-cases.js";
import { UPSTREAM_UTILS_CASES } from "./fixtures/upstream-utils-cases.js";

// The pinned devDependency is the oracle; ast-grep-core in native/Cargo.toml is pinned to the same version.
const UPSTREAM_VERSION = "0.45.3";
const cli = join(dirname(createRequire(import.meta.url).resolve("@ast-grep/cli/package.json")), "ast-grep");
const binding = loadNativeBinding();
const scratch = mkdtempSync(join(tmpdir(), "rerail-upstream-"));
afterAll(() => rmSync(scratch, { recursive: true, force: true }));

/** Matched byte range plus the single-node captures, as `start-end` strings. */
interface Match {
  range: string;
  captures: Record<string, string>;
}

const capabilities = JSON.stringify({
  format: "rerail.policy-capabilities/v1",
  ast: {
    engine: "ast-grep-core",
    engineVersion: "0.45",
    languages: { python: "tree-sitter-python@0.23", typescript: "tree-sitter-typescript@0.23" },
  },
  factRelations: {},
  limits: {
    maxAstPatternBytes: 16_384,
    maxFiles: 100,
    maxGraphDepth: 16,
    maxRegexPatternBytes: 16_384,
    maxSourceBytes: 1_000_000,
    maxVisitedNodes: 100_000,
    maxWitnesses: 10_000,
  },
});

function fileName(testCase: UpstreamCase): string {
  return `${testCase.id}.${testCase.language === "python" ? "py" : "ts"}`;
}

function upstream(testCase: UpstreamCase): Match[] {
  const path = join(scratch, fileName(testCase));
  writeFileSync(path, testCase.source);
  const rule = JSON.stringify({
    id: testCase.id,
    language: testCase.language === "python" ? "Python" : "TypeScript",
    severity: "hint",
    rule: testCase.rule,
    ...(testCase.utils === undefined ? {} : { utils: testCase.utils }),
    ...(testCase.constraints === undefined ? {} : { constraints: testCase.constraints }),
    ...(testCase.languageOptions === undefined ? {} : { languageOptions: testCase.languageOptions }),
  });
  const output = execFileSync(cli, ["scan", "--inline-rules", rule, "--json=compact", path], { encoding: "utf8" });
  const matches = JSON.parse(output) as Array<{
    range: { byteOffset: { start: number; end: number } };
    metaVariables?: { single?: Record<string, { range: { byteOffset: { start: number; end: number } } }> };
  }>;
  return matches.map(match => ({
    range: `${match.range.byteOffset.start}-${match.range.byteOffset.end}`,
    captures: Object.fromEntries(
      Object.entries(match.metaVariables?.single ?? {}).map(([name, value]) => [
        name,
        `${value.range.byteOffset.start}-${value.range.byteOffset.end}`,
      ]),
    ),
  }));
}

type RerailFinding = {
  span: { startByte: number; endByte: number };
  captures: Record<string, Array<{ startByte: number; endByte: number }>>;
};

function rerailMatches(findings: RerailFinding[], singles: Set<string>): Match[] {
  return findings.map(finding => ({
    range: `${finding.span.startByte}-${finding.span.endByte}`,
    captures: Object.fromEntries(
      Object.entries(finding.captures)
        .filter(([name]) => singles.has(name))
        .map(([name, [span]]) => [name, `${span!.startByte}-${span!.endByte}`]),
    ),
  }));
}

function compile(testCase: UpstreamCase) {
  return binding.PolicyDslRuntime.compile(
    JSON.stringify({
      format: "rerail.policy/v1",
      id: `upstream:${testCase.id}`,
      revision: 1,
      scope: { include: ["**/*"], exclude: [] },
      checks: [
        {
          id: "case",
          forbid: {
            ast: {
              language: testCase.language,
              rule: testCase.rule,
              ...(testCase.utils === undefined ? {} : { utils: testCase.utils }),
              ...(testCase.constraints === undefined ? {} : { constraints: testCase.constraints }),
              ...(testCase.languageOptions === undefined ? {} : { languageOptions: testCase.languageOptions }),
            },
          },
          finding: { anchor: "witness.span", code: "CASE", severity: "error", message: "case" },
        },
      ],
    }),
    "json",
    capabilities,
  );
}

const files = (testCase: UpstreamCase) => [{ path: fileName(testCase), language: testCase.language, source: testCase.source }];

async function rerail(testCase: UpstreamCase, singles: Set<string>): Promise<{ stateless: Match[]; indexed: Match[] }> {
  const compiled = compile(testCase);
  const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: files(testCase) });
  const stateless = JSON.parse(compiled.evaluate(snapshot, "diagnostic")) as { findings: RerailFinding[] };
  const workspace = new binding.PolicyDslWorkspace();
  try {
    workspace.addPolicy(compiled);
    const payload = JSON.stringify({ revision: 1, files: files(testCase), removed: [] });
    const raw = workspace.tryApply(payload) ?? (await workspace.applyAsync(payload));
    const indexed = (JSON.parse(raw) as { evaluation: { findings: RerailFinding[] } }).evaluation.findings;
    return { stateless: rerailMatches(stateless.findings, singles), indexed: rerailMatches(indexed, singles) };
  } finally {
    workspace.close();
  }
}

const sorted = (matches: Match[]) => [...matches].sort((a, b) => a.range.localeCompare(b.range));

describe(`relational semantics match ast-grep ${UPSTREAM_VERSION}`, () => {
  it("uses the pinned ast-grep CLI", () => {
    expect(execFileSync(cli, ["--version"], { encoding: "utf8" }).trim()).toBe(`ast-grep ${UPSTREAM_VERSION}`);
  });

  it.each(UPSTREAM_RELATIONAL_CASES.map(testCase => [testCase.id, testCase] as const))("%s", async (_id, testCase) => {
    await expectUpstreamAgreement(testCase);
  });
});

/**
 * `message` and `fix` rendered by the CLI (`message`, `replacement`, `replacementOffsets`) against Rerail's finding
 * `message` and `fix`. Messages are compared where every interpolated capture is one line of at most 80 characters;
 * past that Rerail cuts the capture (see native/src/policy_dsl/findings.rs) where upstream inlines it whole.
 */
interface OutputCase {
  id: string;
  language: "typescript" | "tsx" | "python";
  source: string;
  rule: Record<string, unknown>;
  message: string;
  fix: string;
  /** Upstream renders the message with a multi-line capture; compare only the fix. */
  fixOnly?: boolean;
  /** The rule's `transform`, beside `rule` upstream and in the ast query in Rerail. */
  transform?: Record<string, unknown>;
}

const indented = "function f(debug: boolean) {\n  if (debug) {\n    console.log(a, {\n      b,\n    });\n  }\n  console.log(user.name);\n}\n";

const OUTPUT_CASES: OutputCase[] = [
  { id: "multi-capture-reindents", language: "typescript", source: indented, rule: { pattern: "console.log($$$ARGS)" }, message: "log $$$ARGS", fix: "logger.info($$$ARGS)", fixOnly: true },
  { id: "single-line-multi-capture", language: "typescript", source: "console.log(a, b + 1, 'c');\n", rule: { pattern: "console.log($$$ARGS)" }, message: "log of $$$ARGS!", fix: "logger.info($$$ARGS)" },
  { id: "two-singles", language: "typescript", source: "if (a == b) {}\nconst c = x.y == f(z);\n", rule: { pattern: "$A == $B" }, message: "compare $A with $B using ===", fix: "$A === $B" },
  { id: "multi-line-template", language: "typescript", source: indented, rule: { pattern: "console.log($A)" }, message: "guard $A", fix: "if (verbose) {\n  console.log($A);\n}" },
  { id: "relation-capture", language: "typescript", source: "foo(first, second);\nbar(second);\n", rule: { kind: "identifier", regex: "^second$", inside: { pattern: "foo($$$ARGS)", stopBy: "end" } }, message: "inside foo($$$ARGS)", fix: "$$$ARGS" },
  { id: "unbound-any-branch", language: "typescript", source: "foo(x);\nbar();\n", rule: { any: [{ pattern: "foo($A)" }, { pattern: "bar()" }] }, message: "call [$A]", fix: "baz($A)" },
  { id: "doubled-dollar-and-literal-dollar", language: "typescript", source: "foo(x);\n", rule: { pattern: "foo($A)" }, message: "$$A costs $ and $lower", fix: "`$${$A}`" },
  { id: "deletion", language: "typescript", source: "function g() {\n  debugger;\n}\n", rule: { kind: "debugger_statement" }, message: "no debugger", fix: "" },
  { id: "tsx-jsx-fix", language: "tsx", source: "export const V = () => (\n  <div>\n    <button type=\"button\" onClick={go}>Go</button>\n  </div>\n);\n", rule: { pattern: "<button $$$ATTRS>$$$BODY</button>" }, message: "raw button with $$$ATTRS", fix: "<Button $$$ATTRS>$$$BODY</Button>" },
  { id: "python-indented", language: "python", source: "def f():\n    if x:\n        print(\n            a,\n            b,\n        )\n", rule: { pattern: "print($$$ARGS)" }, message: "print", fix: "logger.info($$$ARGS)" },
  // ESLint report E3: the capture keeps its comments and loses its indentation, so `)` lands in `// trail`. Upstream's
  // TemplateFix does the same; Rerail keeps it.
  { id: "multi-capture-with-comments", language: "typescript", source: "f(\n  // lead\n  a,\n  b // trail\n)\n", rule: { pattern: "f($$$A)" }, message: "f", fix: "h($$$A)" },
  // M12b: `transform` (replace, substring, convert) in messages and fixes.
  {
    id: "transform-replace-trims",
    language: "typescript",
    source: "import a from ' m ';\nimport b from 'n';\n",
    rule: { kind: "string_fragment", pattern: "$M" },
    transform: { T: { replace: { source: "$M", replace: "^\\s+|\\s+$", by: "" } } },
    message: "'$T' import",
    fix: "$T",
  },
  {
    id: "transform-replace-expands-groups",
    language: "typescript",
    source: "setUserName(x);\nsetAge(y);\n",
    rule: { pattern: "$F($A)", kind: "call_expression" },
    transform: { G: { replace: { source: "$F", replace: "^set(?<rest>\\w+)$", by: "get${rest}" } } },
    message: "use $G",
    fix: "$G() ?? $A",
  },
  {
    id: "transform-substring-negative",
    language: "typescript",
    source: "log('héllo wörld');\nlog('ab');\n",
    rule: { pattern: "log($S)" },
    transform: { INNER: { substring: { source: "$S", startChar: 1, endChar: -1 } }, LAST: { substring: { source: "$S", startChar: -3 } } },
    message: "[$INNER] [$LAST]",
    fix: "log(`$INNER`)",
  },
  {
    id: "transform-convert-cases",
    language: "typescript",
    source: "const userAccountName = 1;\nconst XMLHttpRequest2 = 2;\nconst snake_case_name = 3;\n",
    rule: { pattern: "const $N = $V" },
    transform: {
      SNAKE: { convert: { source: "$N", toCase: "snakeCase" } },
      KEBAB: { convert: { source: "$N", toCase: "kebabCase" } },
      PASCAL: { convert: { source: "$N", toCase: "pascalCase" } },
      CAMEL: { convert: { source: "$N", toCase: "camelCase", separatedBy: ["underscore"] } },
      UPPER: { convert: { source: "$N", toCase: "upperCase" } },
      CAP: { convert: { source: "$N", toCase: "capitalize" } },
    },
    message: "$SNAKE $KEBAB $PASCAL $CAMEL $UPPER $CAP",
    fix: "const $SNAKE = $V",
  },
  {
    id: "transform-chain",
    language: "typescript",
    source: "console.log('  Hello World  ');\n",
    rule: { pattern: "console.log($A)" },
    transform: {
      BARE: { substring: { source: "$A", startChar: 1, endChar: -1 } },
      TRIM: { replace: { source: "$BARE", replace: "^\\s+|\\s+$", by: "" } },
      KEY: { convert: { source: "$TRIM", toCase: "snakeCase", separatedBy: ["space", "caseChange"] } },
    },
    message: "$KEY",
    fix: "logger.info(t('$KEY'))",
  },
  {
    id: "transform-multi-line-source",
    language: "typescript",
    source: indented,
    rule: { pattern: "console.log($$$ARGS)" },
    transform: { SAME: { substring: { source: "$$$ARGS" } } },
    message: "log",
    fix: "logger.info(\n  $SAME\n)",
    fixOnly: true,
  },
  {
    id: "transform-unbound-source",
    language: "typescript",
    source: "f(abcd);\ng();\n",
    rule: { any: [{ pattern: "f($A)" }, { pattern: "g()" }] },
    transform: { B: { substring: { source: "$A", startChar: 1, endChar: -1 } } },
    message: "[$B]",
    fix: "h($B)",
  },
  // Upstream leaves an unbound source empty and runs no operation: `replace: "^$"` supplies nothing there.
  {
    id: "transform-unbound-source-runs-no-replace",
    language: "typescript",
    source: "f(abcd);\ng();\n",
    rule: { any: [{ pattern: "f($A)" }, { pattern: "g()" }] },
    transform: { B: { replace: { source: "$A", replace: "^$", by: "none" } } },
    message: "[$B]",
    fix: "h($B)",
  },
  {
    id: "transform-python",
    language: "python",
    source: "def fooBar():\n    pass\n",
    rule: { pattern: "def $F(): pass", kind: "function_definition" },
    transform: { S: { convert: { source: "$F", toCase: "snakeCase" } } },
    message: "rename to $S",
    fix: "def $S(): pass",
  },
];

interface Output {
  range: string;
  message: string;
  replacement: string;
  replacementRange: string;
}

const extension = { typescript: "ts", tsx: "tsx", python: "py" } as const;
const upstreamLanguage = { typescript: "TypeScript", tsx: "Tsx", python: "Python" } as const;

function upstreamOutput(testCase: OutputCase): Output[] {
  const path = join(scratch, `${testCase.id}.${extension[testCase.language]}`);
  writeFileSync(path, testCase.source);
  const rule = JSON.stringify({
    id: testCase.id,
    language: upstreamLanguage[testCase.language],
    severity: "hint",
    message: testCase.message,
    rule: testCase.rule,
    fix: testCase.fix,
    ...(testCase.transform === undefined ? {} : { transform: testCase.transform }),
  });
  const output = execFileSync(cli, ["scan", "--inline-rules", rule, "--json=compact", path], { encoding: "utf8" });
  const matches = JSON.parse(output) as Array<{
    range: { byteOffset: { start: number; end: number } };
    message: string;
    replacement: string;
    replacementOffsets: { start: number; end: number };
  }>;
  return matches.map(match => ({
    range: `${match.range.byteOffset.start}-${match.range.byteOffset.end}`,
    message: match.message,
    replacement: match.replacement,
    replacementRange: `${match.replacementOffsets.start}-${match.replacementOffsets.end}`,
  }));
}

type OutputFinding = {
  span: { startByte: number; endByte: number };
  message: string;
  fix?: { span: { startByte: number; endByte: number }; text: string };
};

async function rerailOutput(testCase: OutputCase): Promise<{ stateless: Output[]; indexed: Output[] }> {
  const compiled = binding.PolicyDslRuntime.compile(
    JSON.stringify({
      format: "rerail.policy/v1",
      id: `upstream:${testCase.id}`,
      revision: 1,
      scope: { include: ["**/*"], exclude: [] },
      checks: [{
        id: "case",
        forbid: {
          ast: {
            language: testCase.language === "python" ? "python" : "typescript",
            rule: testCase.rule,
            ...(testCase.transform === undefined ? {} : { transform: testCase.transform }),
          },
        },
        finding: { anchor: "witness.span", code: "CASE", severity: "error", message: testCase.message, fix: testCase.fix },
      }],
    }),
    "json",
    capabilities,
  );
  const input = [{ path: `${testCase.id}.${extension[testCase.language]}`, language: testCase.language, source: testCase.source }];
  const outputs = (findings: OutputFinding[]) => findings.map(finding => ({
    range: `${finding.span.startByte}-${finding.span.endByte}`,
    message: finding.message,
    replacement: finding.fix!.text,
    replacementRange: `${finding.fix!.span.startByte}-${finding.fix!.span.endByte}`,
  }));
  const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: input });
  const stateless = JSON.parse(compiled.evaluate(snapshot, "diagnostic")) as { findings: OutputFinding[] };
  const workspace = new binding.PolicyDslWorkspace();
  try {
    workspace.addPolicy(compiled);
    const payload = JSON.stringify({ revision: 1, files: input, removed: [] });
    const raw = workspace.tryApply(payload) ?? (await workspace.applyAsync(payload));
    const indexed = (JSON.parse(raw) as { evaluation: { findings: OutputFinding[] } }).evaluation.findings;
    return { stateless: outputs(stateless.findings), indexed: outputs(indexed) };
  } finally {
    workspace.close();
  }
}

describe(`messages and fixes match ast-grep ${UPSTREAM_VERSION}`, () => {
  const byRange = (outputs: Output[]) => [...outputs].sort((a, b) => a.range.localeCompare(b.range));
  const comparable = (testCase: OutputCase, outputs: Output[]) =>
    byRange(outputs).map(output => (testCase.fixOnly ? { ...output, message: "" } : output));

  it.each(OUTPUT_CASES.map(testCase => [testCase.id, testCase] as const))("%s", async (_id, testCase) => {
    const expected = comparable(testCase, upstreamOutput(testCase));
    expect(expected.length).toBeGreaterThan(0);
    const { stateless, indexed } = await rerailOutput(testCase);
    expect(comparable(testCase, stateless)).toEqual(expected);
    expect(comparable(testCase, indexed)).toEqual(expected);
  });

  it("reads a transform's default where upstream leaves an unbound source empty", async () => {
    const plain = OUTPUT_CASES.find(testCase => testCase.id === "transform-unbound-source-runs-no-replace")!;
    const withDefault = { ...plain, transform: { B: { replace: { source: "$A", default: "", replace: "^$", by: "none" } } } };
    const { stateless, indexed } = await rerailOutput(withDefault);
    const upstream = upstreamOutput(plain);
    expect(byRange(upstream).map(output => output.message)).toEqual(["[abcd]", "[]"]);
    expect(byRange(stateless).map(output => [output.message, output.replacement])).toEqual([["[abcd]", "h(abcd)"], ["[none]", "h(none)"]]);
    expect(indexed).toEqual(stateless);
  });

  it("reads ${NAME} where upstream renders it literally", async () => {
    const braced = { id: "braced-variables", language: "typescript" as const, source: "f(abcd);\n", rule: { pattern: "f($A)" }, message: "[${A}_X]", fix: "g_${A}_Y()" };
    const [upstream] = upstreamOutput(braced);
    expect([upstream!.message, upstream!.replacement]).toEqual(["[${A}_X]", "g_${A}_Y()"]);
    const { stateless, indexed } = await rerailOutput(braced);
    expect(stateless.map(output => [output.message, output.replacement])).toEqual([["[abcd_X]", "g_abcd_Y()"]]);
    expect(indexed).toEqual(stateless);
  });

  it("cuts a multi-line capture in a message where upstream inlines it", async () => {
    const [upstreamFirst] = upstreamOutput(OUTPUT_CASES[0]!);
    const { stateless } = await rerailOutput(OUTPUT_CASES[0]!);
    expect(upstreamFirst!.message).toBe("log a, {\n      b,\n    }");
    expect(stateless[0]!.message).toBe("log a, {…");
  });
});

describe(`utils, matches, and constraints match ast-grep ${UPSTREAM_VERSION}`, () => {
  it.each(UPSTREAM_UTILS_CASES.map(testCase => [testCase.id, testCase] as const))("%s", async (_id, testCase) => {
    // Every case matches somewhere upstream, so agreement is never vacuous.
    expect(upstream(testCase).length).toBeGreaterThan(0);
    await expectUpstreamAgreement(testCase);
  });
});

describe(`stopBy rules match ast-grep ${UPSTREAM_VERSION}`, () => {
  it.each(UPSTREAM_STOP_BY_CASES.map(testCase => [testCase.id, testCase] as const))("%s", async (_id, testCase) => {
    expect(upstream(testCase).length).toBeGreaterThan(0);
    await expectUpstreamAgreement(testCase);
  });
});

describe(`field on inside and has matches ast-grep ${UPSTREAM_VERSION}`, () => {
  it.each(UPSTREAM_FIELD_CASES.map(testCase => [testCase.id, testCase] as const))("%s", async (_id, testCase) => {
    expect(upstream(testCase).length).toBeGreaterThan(0);
    await expectUpstreamAgreement(testCase);
  });
});

describe(`nthChild matches ast-grep ${UPSTREAM_VERSION}`, () => {
  it.each(UPSTREAM_NTH_CHILD_CASES.map(testCase => [testCase.id, testCase] as const))("%s", async (_id, testCase) => {
    expect(upstream(testCase).length).toBeGreaterThan(0);
    await expectUpstreamAgreement(testCase);
  });
});

async function expectUpstreamAgreement(testCase: UpstreamCase): Promise<void> {
  const expected = sorted(upstream(testCase));
  const singles = new Set(expected.flatMap(match => Object.keys(match.captures)));
  const { stateless, indexed } = await rerail(testCase, singles);
  expect(sorted(stateless)).toEqual(expected);
  expect(sorted(indexed)).toEqual(expected);
}

/**
 * M6 compile-time decisions, each against what the pinned CLI does with the same rule text: where upstream rejects
 * a rule Rerail rejects it too, and where upstream accepts a rule that can never match Rerail rejects it (a Rerail
 * extension of "no silent ignores"). Kindless rules are the one deliberate acceptance.
 */
interface CompileCase {
  id: string;
  language: "typescript" | "python";
  source: string;
  /** The ast query body as raw JSON text, so repeated keys survive. */
  query: string;
  upstream: "rejects" | "matches-nothing" | "matches";
  rerail: "rejects" | "agrees";
  rerailError?: RegExp;
}

const COMPILE_CASES: CompileCase[] = [
  { id: "repeated-key", language: "python", source: "a\nb\nc\n", query: `{"kind": "identifier", "not": {"regex": "^a$"}, "not": {"regex": "^b$"}}`, upstream: "rejects", rerail: "rejects", rerailError: /^duplicate key `not` at \/checks\/0\/forbid\/ast\/rule/ },
  { id: "abstract-kind", language: "typescript", source: "f(a + 1);\n", query: `{"kind": "expression"}`, upstream: "matches-nothing", rerail: "rejects", rerailError: /`expression` is an abstract kind/ },
  { id: "abstract-kind-python", language: "python", source: "f(a + 1)\n", query: `{"kind": "call", "has": {"kind": "primary_expression"}}`, upstream: "matches-nothing", rerail: "rejects", rerailError: /`primary_expression` is an abstract kind/ },
  { id: "python-dollar-name", language: "python", source: "import time\ntime.sleep(1)\ntime.At\n", query: `{"pattern": "time.$At"}`, upstream: "matches-nothing", rerail: "rejects", rerailError: /`\$At` is not a metavariable/ },
  { id: "typescript-dollar-identifier", language: "typescript", source: "time.At;\ntime.$At;\n", query: `{"pattern": "time.$At"}`, upstream: "matches", rerail: "agrees" },
];

function upstreamCompile(testCase: CompileCase): { error: boolean; matches: number } {
  const path = join(scratch, `compile-${testCase.id}.${testCase.language === "python" ? "py" : "ts"}`);
  writeFileSync(path, testCase.source);
  const rule = `{"id": "${testCase.id}", "language": "${testCase.language === "python" ? "Python" : "TypeScript"}", "severity": "hint", "rule": ${testCase.query}}`;
  try {
    const output = execFileSync(cli, ["scan", "--inline-rules", rule, "--json=compact", path], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    return { error: false, matches: (JSON.parse(output) as unknown[]).length };
  } catch {
    return { error: true, matches: 0 };
  }
}

describe(`M6 compile-time decisions against ast-grep ${UPSTREAM_VERSION}`, () => {
  it.each(COMPILE_CASES.map(testCase => [testCase.id, testCase] as const))("%s", async (_id, testCase) => {
    const upstreamResult = upstreamCompile(testCase);
    expect(upstreamResult.error).toBe(testCase.upstream === "rejects");
    expect(upstreamResult.matches > 0).toBe(testCase.upstream === "matches");
    const source = `{"format": "rerail.policy/v1", "id": "upstream:${testCase.id}", "revision": 1, "scope": {"include": ["**/*"], "exclude": []},
      "checks": [{"id": "case", "forbid": {"ast": {"language": "${testCase.language}", "rule": ${testCase.query}}},
      "finding": {"anchor": "witness.span", "code": "CASE", "severity": "error", "message": "case"}}]}`;
    if (testCase.rerail === "rejects") {
      expect(() => binding.PolicyDslRuntime.compile(source, "json", capabilities)).toThrow(testCase.rerailError);
      return;
    }
    const compiled = binding.PolicyDslRuntime.compile(source, "json", capabilities);
    const files = [{ path: `compile-${testCase.id}.${testCase.language === "python" ? "py" : "ts"}`, language: testCase.language, source: testCase.source }];
    const snapshot = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files });
    const found = (JSON.parse(compiled.evaluate(snapshot, "diagnostic")) as { findings: unknown[] }).findings;
    expect(found).toHaveLength(upstreamResult.matches);
  });

  it("keeps kindless rules that upstream rejects", () => {
    const testCase: CompileCase = { id: "kindless", language: "python", source: "(1)\n", query: `{"inside": {"kind": "parenthesized_expression"}, "not": {"kind": "integer"}}`, upstream: "rejects", rerail: "agrees" };
    expect(upstreamCompile(testCase).error).toBe(true);
    const source = `{"format": "rerail.policy/v1", "id": "kindless", "revision": 1, "checks": [{"id": "case",
      "forbid": {"ast": {"language": "python", "rule": ${testCase.query}}},
      "finding": {"anchor": "witness.span", "code": "CASE", "severity": "error", "message": "case"}}]}`;
    expect(() => binding.PolicyDslRuntime.compile(source, "json", capabilities)).not.toThrow();
  });
});

/**
 * M7 `resolves`: upstream has no scope model, so it rejects the key. The same rule without `resolves` matches
 * identically in both engines; with it, Rerail keeps exactly the matches whose name is not shadowed (or, for `import`,
 * that denote a listed qualified name). Name resolution itself is pinned against Ruff and typescript-eslint by the
 * kernel's offline oracle fixtures (`rerail-fixtures/python/scope`, `rerail-fixtures/ecma/scope`).
 *
 * M8 `languageOptions`: upstream has no language options; its rule config ignores an unknown `languageOptions` field
 * beside `rule` (and rejects one inside `rule`). Rerail's options change nothing without `resolves`, so the base cases
 * pass them to both engines and still agree.
 */
interface ResolveCase {
  base: UpstreamCase;
  /** The base case's query with `resolves` added. */
  resolved: Pick<UpstreamCase, "rule" | "constraints" | "languageOptions" | "utils">;
  /** Texts Rerail keeps, in document order. */
  kept: string[];
}

const RESOLVE_CASES: ResolveCase[] = [
  {
    base: {
      id: "resolves-python-builtin",
      language: "python",
      source: "print(len(x))\nlen = 1\nprint(len)\ndef f(print):\n    print(len)\n",
      rule: { kind: "identifier", regex: "^(print|len)$" },
    },
    resolved: { rule: { kind: "identifier", regex: "^(print|len)$", resolves: { global: ["print", "len"] } } },
    kept: ["print", "len", "print"],
  },
  {
    base: {
      id: "resolves-typescript-global",
      language: "typescript",
      source: "alert(1);\nfunction f(alert: () => void) { alert(); }\nconst o = { alert };\nlet w: Window;\n",
      rule: { any: [{ kind: "identifier" }, { kind: "shorthand_property_identifier" }, { kind: "type_identifier" }], regex: "^(alert|Window)$" },
    },
    resolved: {
      rule: { any: [{ kind: "identifier" }, { kind: "shorthand_property_identifier" }, { kind: "type_identifier" }], regex: "^(alert|Window)$", resolves: { global: ["alert", "Window"] } },
    },
    kept: ["alert", "alert", "Window"],
  },
  {
    base: {
      id: "resolves-python-import",
      language: "python",
      source: "import subprocess as sp\nfrom os import path\nsp.run(a)\npath.join(b)\nsp.call(c)\nrun(d)\n",
      rule: { pattern: "$F($$$)" },
    },
    resolved: { rule: { pattern: "$F($$$)" }, constraints: { F: { resolves: { import: ["subprocess.run", "os.path.join"] } } } },
    kept: ["sp.run(a)", "path.join(b)"],
  },
  // M8: upstream ignores `languageOptions` beside `rule`, and Rerail's options change nothing without `resolves`.
  {
    base: {
      id: "resolves-typescript-language-options",
      language: "typescript",
      source: "Reflect();\nnew String(1);\nthis.eval(2);\nfunction f(String) { String; }\n",
      rule: { any: [{ kind: "identifier" }, { kind: "this" }], regex: "^(Reflect|String|this)$" },
      languageOptions: { ecmaVersion: 5, sourceType: "script", globals: { String: "off" } },
    },
    resolved: {
      rule: { any: [{ kind: "identifier" }, { kind: "this" }], regex: "^(Reflect|String|this)$", resolves: { global: ["Reflect", "String"], this: "global" } },
      languageOptions: { ecmaVersion: 2015, sourceType: "script", globals: { String: "off" } },
    },
    kept: ["Reflect", "this"],
  },
  {
    base: {
      id: "resolves-python-version",
      language: "python",
      source: "aiter(x)\nprint(y)\ndef f(x: Sequence): pass\nfrom typing import Sequence\n",
      rule: { kind: "identifier", regex: "^(aiter|print|Sequence)$" },
      languageOptions: { pythonVersion: "3.9" },
    },
    resolved: {
      rule: { kind: "identifier", regex: "^(aiter|print|Sequence)$", resolves: { global: ["aiter", "print"], import: ["typing.Sequence"] } },
      languageOptions: { pythonVersion: "3.14" },
    },
    kept: ["aiter", "print", "Sequence"],
  },
  {
    base: {
      id: "resolves-python-imported",
      language: "python",
      source: "import pickle\nfrom xml.etree import ElementTree\nfrom . import pickle as p\nimport os.path\n",
      rule: { kind: "dotted_name" },
    },
    resolved: { rule: { kind: "dotted_name", resolves: { imported: ["pickle", "xml.etree"], prefix: true } } },
    kept: ["pickle", "xml.etree", "ElementTree"],
  },
];

describe(`resolves (M7, M8) against ast-grep ${UPSTREAM_VERSION}`, () => {
  it("is a key upstream rejects: it has no scope model", () => {
    const path = join(scratch, "resolves-unknown.py");
    writeFileSync(path, "print(1)\n");
    const rule = JSON.stringify({ id: "resolves", language: "Python", severity: "hint", rule: { kind: "identifier", resolves: { global: ["print"] } } });
    const run = () => execFileSync(cli, ["scan", "--inline-rules", rule, "--json=compact", path], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    expect(run).toThrow(/unknown field `resolves`/);
  });

  it("ignores languageOptions beside a rule and rejects it inside one (M8)", () => {
    const path = join(scratch, "language-options.ts");
    writeFileSync(path, "Reflect();\n");
    const scan = (rule: Record<string, unknown>) =>
      execFileSync(cli, ["scan", "--inline-rules", JSON.stringify({ id: "options", language: "TypeScript", severity: "hint", ...rule }), "--json=compact", path], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      });
    expect(JSON.parse(scan({ languageOptions: { ecmaVersion: 5 }, rule: { kind: "identifier" } }))).toHaveLength(1);
    expect(() => scan({ rule: { kind: "identifier", languageOptions: {} } })).toThrow(/unknown field `languageOptions`/);
  });

  it.each(RESOLVE_CASES.map(testCase => [testCase.base.id, testCase] as const))("%s: the rule without resolves agrees, and resolves keeps a subset", async (_id, testCase) => {
    await expectUpstreamAgreement(testCase.base);
    const all = new Set(upstream(testCase.base).map(match => match.range));
    const { stateless, indexed } = await rerail({ ...testCase.base, ...testCase.resolved }, new Set());
    const texts = (matches: Match[]) =>
      matches
        .map(match => match.range.split("-").map(Number) as [number, number])
        .sort(([left], [right]) => left - right)
        .map(([start, end]) => testCase.base.source.slice(start, end));
    expect(texts(stateless)).toEqual(testCase.kept);
    expect(texts(indexed)).toEqual(testCase.kept);
    for (const match of stateless) expect(all.has(match.range)).toBe(true);
  });
});

/**
 * M9a `value` and `compare`'s new text modes: upstream has neither key and rejects both. The same rule without them
 * matches identically in both engines; with them, Rerail keeps the expected subset. Decoding and folding are pinned
 * against CPython and eslint-utils by the kernel's offline oracle fixtures (`rerail-fixtures/python/value`,
 * `rerail-fixtures/ecma/value`). On
 * comment-free input, `text: "tokens"` equality agrees with upstream's repeated-metavariable equality.
 */
const VALUE_CASES: ResolveCase[] = [
  {
    base: {
      id: "value-python-bind",
      language: "python",
      source: "s.bind(('0.0.0.0', 80))\ns.bind(('0.0.' '0.0', 81))\ns.bind(('127.0.0.1', 82))\ns.bind((host, 83))\n",
      rule: { pattern: "$S.bind(($H, $P))" },
    },
    resolved: { rule: { pattern: "$S.bind(($H, $P))" }, constraints: { H: { value: { eq: "0.0.0.0" } } } },
    kept: ["s.bind(('0.0.0.0', 80))", "s.bind(('0.0.' '0.0', 81))"],
  },
  {
    base: {
      id: "value-typescript-numbers",
      language: "typescript",
      source: "f(1e3);\nf(0x10);\nf(2 ** 10);\nf(x);\nf('2000');\n",
      rule: { pattern: "f($N)" },
    },
    resolved: { rule: { pattern: "f($N)" }, constraints: { N: { value: { gt: 100 } } } },
    kept: ["f(1e3)", "f(2 ** 10)"],
  },
  {
    base: {
      id: "compare-value-python",
      language: "python",
      source: "f(1, 1.0)\nf('a', \"a\")\nf(1, 2)\nf(x, x)\n",
      rule: { pattern: "f($A, $B)" },
    },
    resolved: { rule: { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "value" } } },
    kept: ["f(1, 1.0)", "f('a', \"a\")"],
  },
  {
    base: {
      id: "compare-key-typescript",
      language: "typescript",
      source: "const o = { a: 1, 'a': 2, 0x1: 3, 1: 4, b: 5 };\n",
      rule: { kind: "pair", pattern: "$P", follows: { stopBy: "end", kind: "pair", pattern: "$Q" } },
    },
    resolved: { rule: { kind: "pair", pattern: "$P", follows: { stopBy: "end", kind: "pair", pattern: "$Q", compare: { left: "Q", op: "eq", right: "P", text: "key" } } } },
    kept: ["'a': 2", "1: 4"],
  },
];

describe(`value and compare modes (M9a) against ast-grep ${UPSTREAM_VERSION}`, () => {
  it("are keys upstream rejects: it has neither", () => {
    const path = join(scratch, "value-unknown.py");
    writeFileSync(path, "f('a', 'a')\n");
    const scan = (rule: Record<string, unknown>) => () =>
      execFileSync(cli, ["scan", "--inline-rules", JSON.stringify({ id: "value", language: "Python", severity: "hint", rule }), "--json=compact", path], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      });
    expect(scan({ kind: "string", value: { eq: "a" } })).toThrow(/unknown field `value`/);
    expect(scan({ pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "tokens" } })).toThrow(/unknown field `compare`/);
  });

  it.each([
    ["typescript" as const, "f(a + b, a+b);\nf(x.y, x . y);\nf((a), a);\nf('a', \"a\");\nf(1, 1.0);\nf(a, b);\nf([1, 2], [1,2]);\n"],
    ["python" as const, "f(a + b, a+b)\nf(x[1], x[ 1 ])\nf((a), a)\nf('a', \"a\")\nf(1, 1.0)\nf(a, b)\n"],
  ])("tokens equality is upstream's repeated-metavariable equality on comment-free %s", async (language, source) => {
    const repeated: UpstreamCase = { id: `tokens-${language}`, language, source, rule: { pattern: "f($A, $A)" } };
    const expected = sorted(upstream(repeated)).map(match => match.range);
    expect(expected.length).toBeGreaterThan(1);
    const { stateless, indexed } = await rerail(
      { ...repeated, rule: { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "tokens" } } },
      new Set(),
    );
    expect(sorted(stateless).map(match => match.range)).toEqual(expected);
    expect(sorted(indexed).map(match => match.range)).toEqual(expected);
  });

  it.each(VALUE_CASES.map(testCase => [testCase.base.id, testCase] as const))("%s: the rule without them agrees, and they keep a subset", async (_id, testCase) => {
    await expectKeptSubset(testCase);
  });
});

async function expectKeptSubset(testCase: ResolveCase): Promise<void> {
  await expectUpstreamAgreement(testCase.base);
  const all = new Set(upstream(testCase.base).map(match => match.range));
  const { stateless, indexed } = await rerail({ ...testCase.base, ...testCase.resolved }, new Set());
  const texts = (matches: Match[]) =>
    matches
      .map(match => match.range.split("-").map(Number) as [number, number])
      .sort(([left], [right]) => left - right)
      .map(([start, end]) => testCase.base.source.slice(start, end));
  expect(texts(stateless)).toEqual(testCase.kept);
  expect(texts(indexed)).toEqual(testCase.kept);
  for (const match of stateless) expect(all.has(match.range)).toBe(true);
}

/**
 * M9b `resolves.binding`, `value.follow` and `arguments`: upstream has none of them and rejects `resolves`, `value` and
 * `arguments` as unknown fields. The same rule without them matches identically in both engines; with them, Rerail
 * keeps the expected subset. Binding values are pinned against eslint-utils (`rerail-fixtures/ecma/value`) and
 * the scope oracles; on calls without starred or keyword arguments, a bound slot is upstream's positional capture.
 */
const BINDING_CASES: ResolveCase[] = [
  {
    base: {
      id: "binding-python-dict",
      language: "python",
      source: "ages = {'Tom': 23}\nages.get('Cat', None)\nages = ['Tom']\nages.get('Cat', None)\ndef f(d: dict, o: object):\n    d.get(1, None)\n    o.get(1, None)\n",
      rule: { pattern: "$D.get($K, None)" },
    },
    resolved: {
      rule: { pattern: "$D.get($K, None)" },
      constraints: { D: { any: [{ resolves: { binding: { value: { kind: "dictionary" } } } }, { resolves: { binding: { annotation: { regex: "^dict" } } } }] } },
    },
    kept: ["ages.get('Cat', None)", "d.get(1, None)"],
  },
  {
    base: {
      id: "binding-typescript-alias",
      language: "typescript",
      source: "var foo = JSON; function f(foo) { foo(); }\nvar a = JSON, b = a; b();\nlet c = JSON; c = Math; c();\n",
      rule: { pattern: "$F()" },
    },
    resolved: {
      rule: { pattern: "$F()" },
      constraints: { F: { matches: "json" } },
      utils: { json: { any: [{ kind: "identifier", resolves: { global: ["JSON"] } }, { kind: "identifier", resolves: { binding: { value: { matches: "json" } } } }] } },
    },
    kept: ["b()"],
  },
  {
    base: {
      id: "follow-typescript-string",
      language: "typescript",
      source: "const s = 'alert(1)'; setTimeout(s);\nlet t = 'a'; t = 'b'; setTimeout(t);\nsetTimeout(() => 1);\n",
      rule: { pattern: "setTimeout($S)" },
    },
    resolved: { rule: { pattern: "setTimeout($S)" }, constraints: { S: { value: { type: "string", follow: true } } } },
    kept: ["setTimeout(s)"],
  },
  {
    base: {
      id: "arguments-python-absent",
      language: "python",
      source: "requests.get(url)\nrequests.get(url, timeout=5)\nrequests.get(url, **kw)\nrequests.get(url, **{'verify': False})\n",
      rule: { pattern: "requests.get($$$)" },
    },
    resolved: { rule: { pattern: "requests.get($$$)", arguments: { signature: ["url", "params", "*", "timeout"], absent: ["timeout"] } } },
    kept: ["requests.get(url)", "requests.get(url, **{'verify': False})"],
  },
  {
    base: {
      id: "arguments-python-bind",
      language: "python",
      source: "qs.extra(None, 'raw')\nqs.extra(where=['a'])\nqs.extra(select={}, where=x)\nqs.extra(*args)\n",
      rule: { pattern: "$Q.extra($$$)" },
    },
    resolved: {
      rule: { pattern: "$Q.extra($$$)", arguments: { signature: ["select", "where", "params", "tables", "order_by", "select_params"], bind: { W: "where" } } },
      constraints: { W: { not: { kind: "list" } } },
    },
    kept: ["qs.extra(None, 'raw')", "qs.extra(select={}, where=x)"],
  },
];

describe(`binding values and arguments (M9b) against ast-grep ${UPSTREAM_VERSION}`, () => {
  it("are keys upstream rejects: it has none of them", () => {
    const path = join(scratch, "binding-unknown.py");
    writeFileSync(path, "x = 1\nf(x)\n");
    const scan = (rule: Record<string, unknown>) => () =>
      execFileSync(cli, ["scan", "--inline-rules", JSON.stringify({ id: "binding", language: "Python", severity: "hint", rule }), "--json=compact", path], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      });
    expect(scan({ kind: "identifier", resolves: { binding: { value: { kind: "integer" } } } })).toThrow(/unknown field `resolves`/);
    expect(scan({ kind: "identifier", value: { eq: 1, follow: true } })).toThrow(/unknown field `value`/);
    expect(scan({ kind: "call", arguments: { signature: ["a"], bind: { A: "a" } } })).toThrow(/unknown field `arguments`/);
  });

  it.each([
    ["typescript" as const, "f(a, b);\nf(1);\nf();\nf(g(x), [y]);\n"],
    ["python" as const, "f(a, b)\nf(1)\nf()\nf(g(x), [y])\n"],
  ])("a bound slot is upstream's positional capture on %s calls without starred or keyword arguments", async (language, source) => {
    // Upstream's `f($A, $$$)` needs the comma, so one argument is `f($A)`.
    const positional: UpstreamCase = { id: `arguments-${language}`, language, source, rule: { any: [{ pattern: "f($A)" }, { pattern: "f($A, $$$)" }] } };
    const expected = sorted(upstream(positional));
    expect(expected.length).toBeGreaterThan(1);
    const { stateless, indexed } = await rerail(
      { ...positional, rule: { pattern: "f($$$)", arguments: { signature: ["first"], bind: { A: "first" } } } },
      new Set(["A"]),
    );
    expect(sorted(stateless)).toEqual(expected);
    expect(sorted(indexed)).toEqual(expected);
  });

  it.each(BINDING_CASES.map(testCase => [testCase.base.id, testCase] as const))("%s: the rule without them agrees, and they keep a subset", async (_id, testCase) => {
    await expectKeptSubset(testCase);
  });
});

/**
 * M10a any-mode resolution and Python usedness: upstream rejects `resolves` (so `unresolved`, `global: true` and
 * `import: true`) and `binds` as unknown fields. The same rule without them matches identically in both engines; with
 * them, Rerail keeps the expected subset. Their semantics are pinned against Ruff 0.16.9 (`rerail-fixtures/python/scope/
 * usage`).
 */
const USAGE_CASES: ResolveCase[] = [
  {
    base: {
      id: "unresolved-python",
      language: "python",
      source: "import os\nx = 1\ndel x\nprint(x, missing, os)\nfrom m import *\nstarred\n",
      rule: { kind: "identifier" },
    },
    resolved: { rule: { kind: "identifier", resolves: { unresolved: true } } },
    kept: ["x", "missing"],
  },
  {
    base: {
      id: "unresolved-star-python",
      language: "python",
      source: "before\nfrom m import *\nafter\ndef f():\n    return anywhere\n",
      rule: { kind: "identifier" },
    },
    resolved: { rule: { kind: "identifier", resolves: { unresolved: "star" } } },
    kept: ["after", "anywhere"],
  },
  {
    base: {
      id: "global-any-typescript",
      language: "typescript",
      source: "const Map = 1;\nf(Map, Set, window, Array);\n",
      rule: { kind: "identifier" },
    },
    resolved: { rule: { kind: "identifier", resolves: { global: true } } },
    kept: ["Set", "Array"],
  },
  {
    base: {
      id: "import-any-python",
      language: "python",
      source: "import os.path\nv = 1\nos.path.join(v)\nlen(v)\n",
      rule: { pattern: "$F($$$)" },
    },
    resolved: { rule: { pattern: "$F($$$)" }, constraints: { F: { resolves: { import: true } } } },
    kept: ["os.path.join(v)", "len(v)"],
  },
  {
    base: {
      id: "binds-unused-python",
      language: "python",
      source: "import os\nimport sys\nprint(sys)\ndef f():\n    a = 1\n    b = 2\n    return b\n",
      rule: { any: [{ kind: "identifier" }, { kind: "dotted_name" }] },
    },
    resolved: { rule: { any: [{ kind: "identifier" }, { kind: "dotted_name" }], binds: { form: ["import", "assignment"], latest: true, used: false } } },
    kept: ["os", "a"],
  },
  {
    base: {
      id: "binds-redefines-python",
      language: "python",
      source: "import os\nimport os\nif c:\n    import re\nelse:\n    import re\n",
      rule: { kind: "dotted_name" },
    },
    resolved: { rule: { kind: "dotted_name", binds: { redefines: { form: "import", used: false } } } },
    kept: ["os"],
  },
];

describe(`any-mode resolution and usedness (M10a) against ast-grep ${UPSTREAM_VERSION}`, () => {
  it("are keys upstream rejects: it has none of them", () => {
    const path = join(scratch, "usage-unknown.py");
    writeFileSync(path, "import os\nx = 1\n");
    const scan = (rule: Record<string, unknown>) => () =>
      execFileSync(cli, ["scan", "--inline-rules", JSON.stringify({ id: "usage", language: "Python", severity: "hint", rule }), "--json=compact", path], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      });
    expect(scan({ kind: "identifier", resolves: { unresolved: true } })).toThrow(/unknown field `resolves`/);
    expect(scan({ kind: "identifier", binds: { used: false } })).toThrow(/unknown field `binds`/);
    expect(scan({ kind: "dotted_name", binds: { redefines: { form: "import" } } })).toThrow(/unknown field `binds`/);
  });

  it.each(USAGE_CASES.map(testCase => [testCase.base.id, testCase] as const))("%s: the rule without them agrees, and they keep a subset", async (_id, testCase) => {
    await expectKeptSubset(testCase);
  });
});

/**
 * M10b TypeScript usedness: upstream rejects `binds` and `resolves` (so `write`, `occurrence`, `builtin`) as unknown
 * fields. The same rule without them matches identically in both engines; with them, Rerail keeps the expected subset.
 * Their semantics are pinned against ESLint 10.11.0 and typescript-eslint 8.70.1 (`rerail-fixtures/ecma/scope/
 * eslint-usage`).
 */
const TYPESCRIPT_USAGE_CASES: ResolveCase[] = [
  {
    base: {
      id: "binds-unused-typescript",
      language: "typescript",
      source: "import { a, b } from 'm';\nlet c = 0;\nc++;\nexport const d = a;\n",
      rule: { kind: "identifier" },
    },
    resolved: { rule: { kind: "identifier", binds: { used: false, exported: false } } },
    kept: ["b", "c"],
  },
  {
    base: {
      id: "write-last-typescript",
      language: "typescript",
      source: "let a;\na = 1;\nlet b = 2;\nuse(b);\n",
      rule: { kind: "identifier" },
    },
    resolved: { rule: { kind: "identifier", resolves: { write: "last", binding: { used: false } } } },
    kept: ["a"],
  },
  {
    base: {
      id: "binds-redefines-typescript",
      language: "typescript",
      source: "var a = 1;\nvar a = 2;\nvar b = 3;\n",
      rule: { kind: "identifier" },
    },
    resolved: { rule: { kind: "identifier", binds: { redefines: {} } } },
    kept: ["a"],
  },
];

describe(`TypeScript usedness (M10b) against ast-grep ${UPSTREAM_VERSION}`, () => {
  it("are keys upstream rejects: it has none of them", () => {
    const path = join(scratch, "usage-unknown.ts");
    writeFileSync(path, "let a = 1;\na = 2;\n");
    const scan = (rule: Record<string, unknown>) => () =>
      execFileSync(cli, ["scan", "--inline-rules", JSON.stringify({ id: "usage", language: "TypeScript", severity: "hint", rule }), "--json=compact", path], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      });
    expect(scan({ kind: "identifier", binds: { used: false, occurrence: { kind: "identifier" } } })).toThrow(/unknown field `binds`/);
    expect(scan({ kind: "identifier", resolves: { write: "last" } })).toThrow(/unknown field `resolves`/);
    expect(scan({ kind: "identifier", binds: { builtin: true, redefines: { every: true } } })).toThrow(/unknown field `binds`/);
  });

  it.each(TYPESCRIPT_USAGE_CASES.map(testCase => [testCase.base.id, testCase] as const))("%s: the rule without them agrees, and they keep a subset", async (_id, testCase) => {
    await expectKeptSubset(testCase);
  });
});

/**
 * M11a literal sub-grammars: upstream rejects `parse` (and so `capture`) as an unknown field. The same rule without it
 * matches identically in both engines; with it, Rerail keeps the expected subset. The grammars are pinned against Ruff
 * 0.16.9 and CPython 3.13 (`rerail-fixtures/python/literal`).
 */
const LITERAL_CASES: ResolveCase[] = [
  {
    base: {
      id: "parse-percent-python",
      language: "python",
      source: "a = '%(a)s' % d\nb = '%s' % t\nc = '%(a' % d\nd = f'{x}%(a)s' % d\n",
      rule: { kind: "string" },
    },
    resolved: { rule: { kind: "string", parse: { grammar: "percentFormat", has: { kind: "placeholder", has: { kind: "mapping_key", capture: "K" } } } } },
    kept: ["'%(a)s'"],
  },
  {
    base: {
      id: "parse-brace-python",
      language: "python",
      source: "'{0}{}'.format(1, 2)\n'{}{}'.format(1, 2)\n'{'.format(1)\n",
      rule: { kind: "string" },
    },
    resolved: {
      rule: {
        kind: "string",
        parse: { grammar: "braceFormat", all: [{ has: { kind: "field", has: { kind: "field_name", has: { kind: "auto" } } } }, { has: { kind: "field", has: { kind: "field_name", has: { kind: "index" } } } }] },
      },
    },
    kept: ["'{0}{}'"],
  },
  {
    base: {
      id: "format-args-python",
      language: "python",
      source: "'{a}'.format(a=1, b=2)\n'{a}'.format(a=1)\n'{a}'.format(b=1, **kw)\n",
      rule: { pattern: "$S.format($$$A)" },
    },
    resolved: { rule: { pattern: "$S.format($$$A)", formatArgs: { style: "brace", string: "S", args: "A", unused: { kind: "keyword_argument" } } } },
    kept: ["'{a}'.format(a=1, b=2)"],
  },
  {
    base: {
      id: "parse-annotation-python",
      language: "python",
      source: "from typing import List\nx: \"List[Missing]\"\ny: \"List[int]\"\nz = \"Missing\"\n",
      rule: { kind: "string" },
    },
    resolved: { rule: { kind: "string", parse: { grammar: "annotation", has: { kind: "identifier", resolves: { unresolved: true }, stopBy: "end" } } } },
    kept: ['"List[Missing]"'],
  },
];

describe(`literal sub-grammars (M11a) against ast-grep ${UPSTREAM_VERSION}`, () => {
  it("are keys upstream rejects: it has none of them", () => {
    const path = join(scratch, "literal-unknown.py");
    writeFileSync(path, "'%s' % x\n");
    const scan = (rule: Record<string, unknown>) => () =>
      execFileSync(cli, ["scan", "--inline-rules", JSON.stringify({ id: "literal", language: "Python", severity: "hint", rule }), "--json=compact", path], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      });
    expect(scan({ kind: "string", parse: { grammar: "percentFormat", kind: "format" } })).toThrow(/unknown field `parse`/);
    expect(scan({ kind: "string", capture: "S" })).toThrow(/unknown field `capture`/);
    expect(scan({ pattern: "$S % $R", formatArgs: { style: "percent", string: "S", args: "R", unused: { kind: "pair" } } })).toThrow(/unknown field `formatArgs`/);
  });

  it.each(LITERAL_CASES.map(testCase => [testCase.base.id, testCase] as const))("%s: the rule without them agrees, and they keep a subset", async (_id, testCase) => {
    await expectKeptSubset(testCase);
  });
});
