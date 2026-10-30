import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { loadRuleCatalog } from "../../../src/engine/catalog/rule-catalog.js";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { loadNativeBinding } from "../../../src/engine/native/native.js";
import { bundledCodeRuleDslDirectory } from "../../../src/engine/code-rule/code-rule-compile.js";
import {
  CheckoutCatalogs,
  CompiledRuleCache,
  loadCodeRuleCapabilities,
} from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { decisionFor, type RepositoryAuditRuleProvenance } from "../../../src/engine/audit/audit.js";
import {
  CODE_RULE_LATEST_ECMA_VERSION,
  CODE_RULE_PARSE_GRAMMARS,
  CODE_RULE_PARSE_KINDS,
  CODE_RULE_PYTHON_VERSIONS,
  codeRuleSchema,
  type CodeRule,
} from "../../../src/engine/code-rule/code-rule-schema.js";
import { applyCodeRuleConfig } from "../../../src/engine/code-rule/code-rule-options.js";
import { setRepositoryStarted } from "../../../src/runtime/index.js";
import { addAttachment, setRuleSetting } from "../../../src/engine/catalog/rule-configuration.js";
import { upsertLibraryRule } from "../../../src/engine/catalog/rule-library.js";
import { setUpTestRepository } from "../../support/repository-rules.js";
import { startTestRepository } from "../../support/started-repository.js";
import { validateCodeRule } from "../../../src/engine/rules.js";

const temporaryDirectories: string[] = [];

afterEach(() => {
  for (const path of temporaryDirectories.splice(0)) rmSync(path, { force: true, recursive: true });
});

function temporaryRoot(): string {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-policy-dsl-catalog-")));
  temporaryDirectories.push(root);
  return root;
}

/** A Git checkout with its own database. */
function checkout(): { root: string; databasePath: string } {
  const root = temporaryRoot();
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  return { root, databasePath: join(root, ".rerail", "rerail.sqlite") };
}

function writeCapabilities(root: string): string {
  const capabilitiesPath = join(root, "capabilities.json");
  writeFileSync(
    capabilitiesPath,
    JSON.stringify({
      ast: {
        engine: "ast-grep-core",
        engineVersion: "0.45",
        languages: { python: "tree-sitter-python@0.23", typescript: "tree-sitter-typescript@0.23" },
      },
      factRelations: {},
      format: "rerail.policy-capabilities/v1",
      limits: {
        maxAstPatternBytes: 16_384,
        maxFiles: 10_000,
        maxGraphDepth: 16,
        maxRegexPatternBytes: 16_384,
        maxSourceBytes: 1_000_000,
        maxVisitedNodes: 100_000,
        maxWitnesses: 10_000,
      },
    }),
  );
  return capabilitiesPath;
}

function rule(id: string, revision = 1): CodeRule {
  return codeRuleSchema.parse({
    clauses: [
      {
        id: `${id}:check`,
        violation: { anchor: "witness.span", code: "FORBIDDEN_PRINT", message: "No print" },
        forbid: { ast: { language: "python", rule: { pattern: "print($ARG)" } } },
      },
    ],
    format: "rerail.policy/v1",
    id,
    mode: "block",
    revision,
    scope: { exclude: [], include: ["**/*.py"] },
  });
}

describe("checkout code-rule catalog", () => {
  it("compiles only the rules a set-up, started repository adds, ordered by id", () => {
    const { root, databasePath } = checkout();
    const capabilities = loadCodeRuleCapabilities(writeCapabilities(temporaryRoot()));
    const key = startTestRepository(root, databasePath);
    const catalogs = new CheckoutCatalogs(databasePath, { capabilities });
    try {
      const catalog = catalogs.catalog(root);
      expect(catalogs.eligibility(root)).toBe("not-set-up");
      expect(catalog.status.ruleCount).toBe(0);

      setUpTestRepository(root, { library: [{ kind: "code", rule: rule("test:zeta") }, { kind: "code", rule: rule("test:alpha") }] }, databasePath);
      expect(catalog.refresh()).toBe(true);
      expect(catalog.snapshot().codeRules.map(({ id }) => id)).toEqual(["test:alpha", "test:zeta"]);
      expect(JSON.parse(catalog.snapshot().codeRules[0]!.executable.canonicalJson())).toMatchObject({ id: "test:alpha" });
      expect(catalog.refresh()).toBe(false);

      const database = openRerailDatabase(databasePath);
      try {
        setRepositoryStarted(database, key, false);
      } finally {
        database.$client.close();
      }
      expect(catalog.refresh()).toBe(true);
      expect(catalogs.eligibility(root)).toBe("stopped");
      expect(catalog.status.ruleCount).toBe(0);
    } finally {
      catalogs.close();
    }
  });

  it("places a rule per folder from its attachments and settings, the deepest folder deciding", () => {
    const { root, databasePath } = checkout();
    const capabilities = loadCodeRuleCapabilities(writeCapabilities(temporaryRoot()));
    const key = setUpTestRepository(root, { library: [{ kind: "code", rule: rule("test:everywhere") }] }, databasePath);
    const database = openRerailDatabase(databasePath);
    const catalogs = new CheckoutCatalogs(databasePath, { capabilities });
    try {
      upsertLibraryRule(database, loadRuleCatalog(), rule("test:api-only"), "code", { approve: true });
      addAttachment(database, { repositoryKey: key, folder: "api", sourceKind: "library_rule", sourceId: "test:api-only" });
      setRuleSetting(database, { owner: { kind: "repository", repositoryKey: key }, folder: "legacy", targetKind: "rule", targetId: "test:everywhere", on: false, mode: null });
      setRuleSetting(database, { owner: { kind: "checkout", checkoutPath: root }, folder: "web", targetKind: "rule", targetId: "test:everywhere", on: null, mode: "log" });
      const snapshot = catalogs.catalog(root).snapshot();
      const placement = Object.fromEntries(snapshot.codeRules.map(codeRule => [codeRule.id, codeRule.placement]));
      expect(placement).toEqual({
        "test:api-only": [{ folder: "api", mode: "block" }],
        "test:everywhere": [{ folder: "", mode: "block" }, { folder: "legacy", mode: null }, { folder: "web", mode: "log" }],
      });
      const audit = { ruleProvenance: [...snapshot.provenance] };
      expect(decisionFor(audit, { ruleId: "test:everywhere", path: "web/view.py" })).toMatchObject({
        folder: "web", on: true, mode: "log", modeFrom: { by: "checkout", checkoutPath: root, folder: "web" },
      });
      expect(decisionFor(audit, { ruleId: "test:everywhere", path: "web2/view.py" })).toMatchObject({ folder: "", mode: "block" });
      expect(decisionFor(audit, { ruleId: "test:everywhere", path: "legacy/old.py" })).toMatchObject({ on: false, onFrom: { by: "repository", folder: "legacy" } });
      expect(snapshot.provenance.find(entry => entry.ruleId === "test:api-only")).toMatchObject({
        repositoryKey: key, checkoutPath: root, source: { kind: "library_rule", id: "test:api-only", packId: null },
      } satisfies Partial<RepositoryAuditRuleProvenance>);
    } finally {
      catalogs.close();
      database.$client.close();
    }
  });

  it("changes its version with a placement but reuses the compiled body", () => {
    const { root, databasePath } = checkout();
    const capabilities = loadCodeRuleCapabilities(writeCapabilities(temporaryRoot()));
    const key = setUpTestRepository(root, { library: [{ kind: "code", rule: rule("test:placed") }] }, databasePath);
    const database = openRerailDatabase(databasePath);
    const catalogs = new CheckoutCatalogs(databasePath, { capabilities });
    try {
      const catalog = catalogs.catalog(root);
      const before = catalog.snapshot();
      setRuleSetting(database, { owner: { kind: "repository", repositoryKey: key }, folder: "docs", targetKind: "rule", targetId: "test:placed", on: false, mode: null });
      expect(catalog.refresh()).toBe(true);
      const after = catalog.snapshot();
      expect(after.version).not.toBe(before.version);
      expect(after.codeRules[0]!.executable).toBe(before.codeRules[0]!.executable);
      // Switched off everywhere, the rule leaves the compiled set.
      setRuleSetting(database, { owner: { kind: "repository", repositoryKey: key }, folder: "", targetKind: "rule", targetId: "test:placed", on: false, mode: null });
      expect(catalog.refresh()).toBe(true);
      expect(catalog.snapshot().codeRules).toEqual([]);
    } finally {
      catalogs.close();
      database.$client.close();
    }
  });

  it("places where suppression comments are ignored, changing its version but not the compiled body", () => {
    const { root, databasePath } = checkout();
    const capabilities = loadCodeRuleCapabilities(writeCapabilities(temporaryRoot()));
    const key = setUpTestRepository(root, { library: [{ kind: "code", rule: rule("test:placed") }] }, databasePath);
    const database = openRerailDatabase(databasePath);
    const catalogs = new CheckoutCatalogs(databasePath, { capabilities });
    try {
      const catalog = catalogs.catalog(root);
      const before = catalog.snapshot();
      expect(before.codeRules[0]!.placement).toEqual([{ folder: "", mode: "block" }]);

      const owner = { kind: "repository" as const, repositoryKey: key };
      setRuleSetting(database, { owner, folder: "web", targetKind: "rule", targetId: "test:placed", on: null, mode: null, suppressions: "ignore" });
      setRuleSetting(database, { owner, folder: "web/legacy", targetKind: "rule", targetId: "test:placed", on: false, mode: null });
      setRuleSetting(database, { owner, folder: "web/vendor", targetKind: "rule", targetId: "test:placed", on: null, mode: null, suppressions: "honour" });
      expect(catalog.refresh()).toBe(true);
      const after = catalog.snapshot();
      expect(after.codeRules[0]!.placement).toEqual([
        { folder: "", mode: "block" },
        { folder: "web", mode: "block", suppressions: "ignore" },
        { folder: "web/legacy", mode: null, suppressions: "ignore" },
        { folder: "web/vendor", mode: "block" },
      ]);
      expect(after.version).not.toBe(before.version);
      expect(after.codeRules[0]!.executable).toBe(before.codeRules[0]!.executable);
      expect(decisionFor({ ruleProvenance: [...after.provenance] }, { ruleId: "test:placed", path: "web/a.py" })).toMatchObject({
        suppressions: "ignore", suppressionsFrom: { by: "repository", folder: "web", target: "rule" },
      });
    } finally {
      catalogs.close();
      database.$client.close();
    }
  });

  it("quarantines a library rule the engine rejects while other rules stay executable", () => {
    const { root, databasePath } = checkout();
    const capabilitiesPath = writeCapabilities(temporaryRoot());
    writeFileSync(
      capabilitiesPath,
      JSON.stringify({ ...JSON.parse(readFileSync(capabilitiesPath, "utf8")), ast: { engine: "ast-grep-core", engineVersion: "0.45", languages: {} } }),
    );
    const regexRule = codeRuleSchema.parse({
      clauses: [{ id: "regex", violation: { anchor: "witness.span", code: "R", message: "r" }, forbid: { regex: { source: "file.content", pattern: "print" } } }],
      format: "rerail.policy/v1", id: "test:regex", mode: "steer", revision: 1, scope: { exclude: [], include: ["**/*.py"] },
    });
    setUpTestRepository(root, { library: [{ kind: "code", rule: rule("test:requires-python") }, { kind: "code", rule: regexRule }] }, databasePath);
    const catalogs = new CheckoutCatalogs(databasePath, { capabilities: loadCodeRuleCapabilities(capabilitiesPath) });
    try {
      const catalog = catalogs.catalog(root);
      expect(catalog.snapshot().codeRules.map(({ id }) => id)).toEqual(["test:regex"]);
      expect(catalog.quarantined()).toEqual([
        expect.objectContaining({ kind: "code", ruleId: "test:requires-python", owner: "library", reason: expect.stringMatching(/missing.*python.*capability/iu) }),
      ]);
    } finally {
      catalogs.close();
    }
  });
});

describe("code rule schema and engine agree on unsupported fields", () => {
  const bundledCapabilities = readFileSync(join(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");
  const violation = { anchor: "witness.span", code: "X", message: "x" };
  const regex = { regex: { pattern: "x", source: "file.content" } };
  const document = (clause: Record<string, unknown>, rule: Record<string, unknown> = {}) => ({
    clauses: [{ id: "x", violation: violation, forbid: regex, ...clause }],
    format: "rerail.policy/v1",
    id: "x",
    mode: "block",
    revision: 1,
    ...rule,
  });
  const ast = (rule: unknown, extra: Record<string, unknown> = {}, spec: Record<string, unknown> = {}) =>
    document({ forbid: { ast: { language: "typescript", rule, ...extra } }, violation: { ...violation, ...spec } });
  const evidenceQuery = (where?: unknown) => ({
    evidence: { as: "s", query: { ast: { language: "typescript", rule: { kind: "identifier" } } }, project: { path: "file.path" }, ...(where === undefined ? {} : { where }) },
  });
  const where = (predicate: unknown) => document({ forbid: undefined, budget: { query: evidenceQuery(predicate), count: { lte: 1 } } });
  const py = (rule: unknown, extra: Record<string, unknown> = {}, spec: Record<string, unknown> = {}) =>
    document({ forbid: { ast: { language: "python", rule, ...extra } }, violation: { ...violation, ...spec } });
  const php = (rule: unknown, extra: Record<string, unknown> = {}, spec: Record<string, unknown> = {}) =>
    document({ forbid: { ast: { language: "php", rule, ...extra } }, violation: { ...violation, ...spec } });
  const relation = (form: "require_each" | "budget", spec: Record<string, unknown>) => document({
    forbid: undefined,
    ...(form === "require_each" ? { require_each: { subject: evidenceQuery(), evidence: evidenceQuery() } } : { budget: { query: evidenceQuery(), count: { lte: 1 } } }),
    violation: { ...violation, ...spec },
  });

  it.each([
    ["a string stopBy other than neighbor or end", ast({ kind: "identifier", inside: { kind: "call_expression", stopBy: "far" } }), /expected one of \\"neighbor\\"\|\\"end\\"/],
    ["a malformed stopBy rule", ast({ kind: "identifier", inside: { kind: "call_expression", stopBy: { kind: "program", field: "body" } } }), /Unrecognized key/],
    ["an undefined utility in a stopBy rule", ast({ kind: "identifier", inside: { kind: "call_expression", stopBy: { matches: "missing" } } }), /Utility rule missing is not defined/],
    ["a field on precedes", ast({ kind: "identifier", precedes: { kind: "identifier", field: "name" } }), /field is only supported in has\/inside/],
    ["a field on follows", ast({ kind: "identifier", follows: { kind: "identifier", field: "name" } }), /field is only supported in has\/inside/],
    ["a field in a count", ast({ kind: "identifier", count: { inside: { kind: "variable_declarator", field: "name" }, min: 1 } }), /field is not supported in count/],
    ["an empty field", ast({ kind: "identifier", inside: { kind: "variable_declarator", field: " " } }), /Must be a non-empty string/],
    ["a field on a rule", ast({ kind: "identifier", field: "name" }), /Unrecognized key: \\"field\\"/],
    ["an nthChild with an illegal character", ast({ kind: "identifier", nthChild: "2x" }), /nthChild: illegal character `x`/],
    ["an nthChild that is no An+B", ast({ kind: "identifier", nthChild: "n+" }), /nthChild: invalid syntax/],
    ["a negative nthChild", ast({ kind: "identifier", nthChild: -1 }), /Too small/],
    ["an nthChild without a position", ast({ kind: "identifier", nthChild: { ofRule: { kind: "identifier" } } }), /Invalid input/],
    ["an nthChild with an unknown key", ast({ kind: "identifier", nthChild: { position: 1, of: {} } }), /Unrecognized key: \\"of\\"/],
    ["an nthChild with a malformed ofRule", ast({ kind: "identifier", nthChild: { position: 1, ofRule: { kinds: "identifier" } } }), /Unrecognized key: \\"kinds\\"/],
    ["an nthChild ofRule with an undefined utility", ast({ kind: "identifier", nthChild: { position: 1, ofRule: { matches: "missing" } } }), /Utility rule missing is not defined/],
    ["an empty layout", ast({ kind: "identifier", layout: {} }), /layout must hold span, start, before or after/],
    ["a line of a transform variable", ast({ pattern: "f($A)" }, { transform: { T: { substring: { source: "$A" } }, L: { line: { source: "$T" } } } }), /\$T is a transform variable, text; line, lines and count read a capture/],
    ["a line edge other than start or end", ast({ pattern: "f($A)" }, { transform: { L: { line: { source: "$A", edge: "middle" } } } }), /Invalid input/],
    ["a count with settings", ast({ pattern: "f($$$A)" }, { transform: { N: { count: { source: "$$$A", nodes: true } } } }), /Unrecognized key: \\"nodes\\"/],
    ["the value of a line", ast({ pattern: "f($A)" }, { transform: { L: { line: { source: "$A", value: true } } } }), /Unrecognized key: \\"value\\"/],
    ["an unknown layout key", ast({ kind: "identifier", layout: { lines: { eq: 0 } } }), /Unrecognized key: \\"lines\\"/],
    ["an empty line count", ast({ kind: "identifier", layout: { span: {} } }), /Set eq, min or max/],
    ["a negative line count", ast({ kind: "identifier", layout: { span: { eq: -1 } } }), /Too small/],
    ["eq beside max", ast({ kind: "identifier", layout: { span: { eq: 1, max: 2 } } }), /Invalid input/],
    ["a line count whose min exceeds its max", ast({ kind: "identifier", layout: { start: { min: 3, max: 2 } } }), /min must not exceed max/],
    ["a gap without to", ast({ kind: "identifier", layout: { before: { breaks: { eq: 0 } } } }), /expected string/],
    ["a gap to a parent", ast({ kind: "identifier", layout: { before: { to: "parent", breaks: { eq: 0 } } } }), /to must be \\"token\\", \\"sibling\\" or \\"capture.<NAME>\\"/],
    ["a gap to a $-prefixed capture", ast({ kind: "identifier", layout: { before: { to: "capture.$A", breaks: { eq: 0 } } } }), /to must be \\"token\\", \\"sibling\\" or \\"capture.<NAME>\\"/],
    ["a gap that counts nothing", ast({ kind: "identifier", layout: { after: { to: "token" } } }), /A gap must count breaks, blank or both/],
    ["a non-boolean comments", ast({ kind: "identifier", layout: { after: { to: "token", comments: 1, breaks: { eq: 0 } } } }), /expected boolean/],
    ["an edge on a capture", ast({ pattern: "f($A)", layout: { after: { to: "capture.A", edge: true, breaks: { eq: 0 } } } }), /a capture names its node/],
    ["a layout inside a parsed tree", py({ kind: "string", parse: { grammar: "percentFormat", has: { kind: "placeholder", layout: { span: { eq: 0 } } } } }), /layout is not supported inside parse \(percentFormat trees\)/],
    ["an empty suppress", ast({ kind: "identifier" }, {}, { suppress: {} }), /suppress must hold eslint, noqa, nosec, oxlint, biome, phpstan, psalm, phpcs, clippy, or sqlfluff/],
    ["an unknown suppress key", ast({ kind: "identifier" }, {}, { suppress: { jshint: {} } }), /Unrecognized key: \\"jshint\\"/],
    ["an empty eslint rule list", ast({ kind: "identifier" }, {}, { suppress: { eslint: { rules: [] } } }), /Too small/],
    ["an eslint rule name with a comma", ast({ kind: "identifier" }, {}, { suppress: { eslint: { rules: ["a, b"] } } }), /Must be a rule name/],
    ["an empty oxlint rule list", ast({ kind: "identifier" }, {}, { suppress: { oxlint: { rules: [] } } }), /Too small/],
    ["an oxlint rule name with a plugin prefix", ast({ kind: "identifier" }, {}, { suppress: { oxlint: { rules: ["react-hooks/rules-of-hooks"] } } }), /Must be an oxlint rule name/],
    ["a Biome rule name without its lint/ category", ast({ kind: "identifier" }, {}, { suppress: { biome: { rules: ["correctness/useHookAtTopLevel"] } } }), /Must be a Biome lint rule name/],
    ["a lowercase noqa code", py({ kind: "identifier" }, {}, { suppress: { noqa: { codes: ["f401"] } } }), /Must be a rule code/],
    ["a noqa parent other than statement", py({ kind: "identifier" }, {}, { suppress: { noqa: { codes: ["F401"], parent: "import" } } }), /Invalid input/],

    ["suppression on a regex query", document({ violation: { ...violation, suppress: { eslint: { rules: ["strict"] } } } }), /suppress removes an ast query's violations/],
    ["a cyclic transform", ast({ pattern: "f($A)" }, { transform: { B: { substring: { source: "$C" } }, C: { substring: { source: "$B" } } } }), /B has a cyclic dependency/],
    ["a transform in the string form", ast({ pattern: "f($A)" }, { transform: { B: "substring($A, startChar=1)" } }), /Invalid input/],
    ["a rewrite transform", ast({ pattern: "f($A)" }, { transform: { B: { rewrite: { source: "$A", rewriters: ["r"] } } } }), /Invalid input/],
    ["a transform source without $", ast({ pattern: "f($A)" }, { transform: { B: { substring: { source: "A" } } } }), /A transform source is a \$-prefixed metavariable/],
    ["an unknown case", ast({ pattern: "f($A)" }, { transform: { B: { convert: { source: "$A", toCase: "titleCase" } } } }), /Invalid input/],
    ["a lower-case transform name", ast({ pattern: "f($A)" }, { transform: { b: { substring: { source: "$A" } } } }), /Must be a metavariable name/],
    ["two transformations in one variable", ast({ pattern: "f($A)" }, { transform: { B: { substring: { source: "$A" }, convert: { source: "$A", toCase: "lowerCase" } } } }), /Invalid input/],
    ["a transform default that is no string", ast({ pattern: "f($A)" }, { transform: { B: { substring: { source: "$A", default: 1 } } } }), /Invalid input/],
    ["a transform value that is no boolean", ast({ pattern: "f($A)" }, { transform: { B: { substring: { source: "$A", value: "yes" } } } }), /Invalid input/],
    ["the value of a list capture", ast({ pattern: "f($$$A)" }, { transform: { B: { substring: { source: "$$$A", value: true } } } }), /A value is read from a single capture/],
    ["the value of a transform variable", ast({ pattern: "f($A)" }, { transform: { B: { substring: { source: "$A" } }, C: { substring: { source: "$B", value: true } } } }), /A transform variable is text/],
    ["a fix range on a regex query", document({ violation: { ...violation, fix: { template: "y", range: "capture.X" } } }), /A fix range names a capture of an ast query/],
    ["an unknown fix range", ast({ pattern: "f($A)" }, {}, { fix: { template: "y", range: "witness.start" } }), /A fix range is witness.span or capture/],
    ["a fix object without a template", ast({ pattern: "f($A)" }, {}, { fix: { range: "capture.A" } }), /Invalid input/],
    ["each on a regex query", document({ violation: { ...violation, each: "X" } }), /each enumerates the captures of an ast query/],
    ["each with a $", ast({ pattern: "f($A)" }, {}, { each: "$A" }), /each is a metavariable name/],
    ["an array kind", ast({ kind: ["identifier"], pattern: "f()" }), /expected string/],
    ["a non-string regex", ast({ kind: "identifier", regex: 1 }), /expected string/],
    ["the removed ast capture field", ast({ pattern: "f($A)" }, { capture: "A" }), /Unrecognized key: \\"capture\\"/],
    ["the removed regex captures field", document({ forbid: { regex: { ...regex.regex, captures: ["x"] } } }), /Unrecognized key: \\"captures\\"/],
    ["an unknown anchor", document({ violation: { ...violation, anchor: "subject.id" } }), /Anchor must be witness\.span, witness\.start, witness\.end, file\.start, or capture\.<NAME>/],
    ["an empty capture anchor", document({ violation: { ...violation, anchor: "capture." } }), /Anchor must be/],
    ["a non-string fix", document({ violation: { ...violation, fix: { template: "y" } } }), /expected string/],
    ["finding variables", document({ violation: { ...violation, variables: { name: "x" } } }), /Violation variables are not supported yet/],
    ["an unknown predicate operator", where({ gt: ["s.size", 1] }), /Invalid input/],
    ["an eq with one operand", where({ eq: ["s.size"] }), /Invalid input/],
    ["an undefined utility", ast({ kind: "identifier", not: { matches: "missing" } }), /Utility rule missing is not defined/],
    ["a call with a rule for an argument", ast({ matches: { wrap: { X: { kind: "number" } } } }, { utils: { wrap: { params: ["X"], rule: { kind: "number" } } } }), /matches must be the id of a utility rule or a call/],
    ["a reserved utility id", ast({ kind: "number" }, { utils: { "a(b)": { kind: "number" } } }), /Utility ids must not contain/],
    ["a malformed utility", ast({ matches: "u" }, { utils: { u: { kind: "number", stopBy: "end" } } }), /Unrecognized key/],
    ["a malformed constraint", ast({ pattern: "$F()" }, { constraints: { F: { regex: 1 } } }), /expected string/],
    ["a comparison with an unknown operator", ast({ pattern: "f($A, $B)", compare: { left: "A", op: "before", right: "B" } }), /Invalid option/],
    ["a comparison with a $-prefixed name", ast({ pattern: "f($A, $B)", compare: { left: "$A", op: "lt", right: "B" } }), /metavariable name without \$/],
    ["a comparison with an unknown collation", ast({ pattern: "f($A, $B)", has: { kind: "identifier", compare: { left: "A", op: "lt", right: "B", collation: "locale" } } }), /Invalid option/],
    ["a list of comparisons", ast({ pattern: "f($A, $B)", compare: [{ left: "A", op: "lt", right: "B" }] }), /expected object/],
    ["an unknown comparison text", ast({ pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "cooked" } }), /Invalid option/],
    ["an ordering over tokens", ast({ pattern: "f($A, $B)", compare: { left: "A", op: "lt", right: "B", text: "tokens" } }), /tokens compares for equality only/],
    ["a collation over tokens", ast({ pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "tokens", collation: "fold-lower" } }), /tokens compares tokens exactly/],
    ["commute over source text", ast({ pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", commute: ["||"] } }), /commute applies to text tokens only/],
    ["an empty commute", ast({ pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "tokens", commute: [] } }), /Too small/],
    ["commute over ESLint's references", ast({ pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "reference", commute: ["||"] } }), /commute applies to text tokens only/],
    ["commute over Ruff's structure", py({ pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "structure", commute: ["or"] } }), /commute applies to text tokens only/],
    ["a field inside a parsed tree", py({ kind: "string", parse: { grammar: "percentFormat", has: { kind: "placeholder", field: "key" } } }), /field is not supported inside parse/],
    ["an nthChild inside a parsed tree", py({ kind: "string", parse: { grammar: "braceFormat", has: { kind: "field", nthChild: 1 } } }), /nthChild is not supported inside parse \(braceFormat trees\)/],
    ["a collation over indices", ast({ pattern: "f($A, $B)", compare: { left: "A", op: "lt", right: "B", text: "index", collation: "natural" } }), /index compares positions exactly/],
    ["an ordering over nodes", ast({ pattern: "f($A, $B)", compare: { left: "A", op: "gt", right: "B", text: "node" } }), /node compares for equality only/],
    ["a collation over nodes", ast({ pattern: "f($A, $B)", compare: { left: "A", op: "ne", right: "B", text: "node", collation: "natural" } }), /node compares nodes exactly/],
    ["an ordering over structures", document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op: "lt", right: "B", text: "structure-hash" } } } } }), /structure-hash compares for equality only/],
    ["a collation over structures", document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "structure", collation: "natural" } } } } }), /structure compares structures exactly/],
    ["an ordering over references", ast({ pattern: "f($A, $B)", compare: { left: "A", op: "lt", right: "B", text: "reference-no-static-keys" } }), /reference-no-static-keys compares for equality only/],
    ["a count without bounds", ast({ kind: "program", count: { has: { kind: "if_statement" } } }), /min to at least 1, or set max/],
    ["a count with two relations", ast({ kind: "program", count: { has: { kind: "if_statement" }, inside: { kind: "module" }, min: 1 } }), /exactly one relation/],
    ["a count whose min exceeds its max", ast({ kind: "program", count: { has: { kind: "if_statement" }, min: 2, max: 1 } }), /min must not exceed/],
    ["a count with an undefined utility", ast({ kind: "program", count: { has: { matches: "missing" }, min: 1 } }), /Utility rule missing is not defined/],
    ["a missing mode", document({}, { mode: undefined }), /expected one of \\"log\\"\|\\"steer\\"\|\\"block\\"/],
    ["a mode outside log, steer, and block", document({}, { mode: "error" }), /expected one of \\"log\\"\|\\"steer\\"\|\\"block\\"/],
    ["a finding severity", document({ violation: { ...violation, severity: "error" } }), /Unrecognized key: \\"severity\\"/],
    ["an empty resolves list", ast({ kind: "identifier", resolves: { global: [] } }), /Too small/],
    ["a dotted global name", ast({ kind: "identifier", resolves: { global: ["a.b"] } }), /Must be an identifier/],
    ["a resolves without names", ast({ kind: "identifier", resolves: {} }), /global names, import names, imported names, this, unresolved, a binding, write, or beforeLocal/],
    ["unresolved other than true or star", ast({ kind: "identifier", resolves: { unresolved: false } }), /Invalid input/],
    ["unresolved beside other resolves terms", ast({ kind: "identifier", resolves: { unresolved: true, global: ["x"] } }), /write other resolves terms in a resolves of their own/],
    ["beforeLocal other than true", ast({ kind: "identifier", resolves: { beforeLocal: false } }), /Invalid input/],
    ["beforeLocal beside other resolves terms", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { beforeLocal: true, global: ["x"] } } } } }), /write other resolves terms in a resolves of their own/],
    ["seenBefore without modules", document({ forbid: { ast: { language: "python", rule: { kind: "call", resolves: { seenBefore: true } } } } }), /seenBefore needs imported modules/],
    ["seenBefore over a dotted module", document({ forbid: { ast: { language: "python", rule: { kind: "call", resolves: { imported: ["os.path"], seenBefore: true } } } } }), /top-level module/],
    ["seenBefore beside other resolves terms", document({ forbid: { ast: { language: "python", rule: { kind: "call", resolves: { imported: ["os"], seenBefore: true, prefix: true } } } } }), /write other resolves terms in a resolves of their own/],
    ["global other than true or names", ast({ kind: "identifier", resolves: { global: false } }), /Invalid input/],
    ["a prefix on import: true", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { import: true, prefix: true } } } } }), /lists none to prefix/],
    ["declared on global: true", ast({ kind: "identifier", resolves: { global: true, declared: false } }, { languageOptions: {} }), /no names to undeclare/],
    ["an unknown binding form", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { form: "assign" } } } } }), /Invalid option/],
    ["an empty scope list", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { scope: [] } } } } }), /Too small/],
    ["a non-boolean used filter", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { used: "no" } } } } }), /expected boolean/],
    ["an unknown binds key", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { shadows: true } } } } }), /Unrecognized key/],
    ["a nested redefines", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { redefines: { redefines: {} } } } } } }), /Unrecognized key/],
    ["a non-boolean sameScope", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { redefines: { sameScope: "yes" } } } } } }), /expected boolean/],
    ["only in binds", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { only: true } } } } }), /Unrecognized key/],
    ["stopAtGlobal in binds", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { stopAtGlobal: true } } } } }), /Unrecognized key/],
    ["a binding count without bounds", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { bindings: { min: 0 } } } } } }), /set min to at least 1, or set max/],
    ["a binding count whose min exceeds its max", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { bindings: { min: 2, max: 1 } } } } } }), /min must not exceed/],
    ["a binding count within a $-prefixed name", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { bindings: { within: "$LOOP", max: 1 } } } } } }), /without `\$`/],
    ["an occurrence in redefines", ast({ kind: "identifier", binds: { redefines: { occurrence: { kind: "identifier" } } } }), /Unrecognized key/],
    ["write other than last", ast({ kind: "identifier", resolves: { write: "first" } }), /Invalid input/],
    ["write beside global names", ast({ kind: "identifier", resolves: { write: "last", global: ["a"] } }), /write asks where ESLint reports a variable/],
    ["a filter's directives other than eslint", ast({ kind: "identifier", binds: { directives: "jshint" } }), /Invalid input/],
    ["a dotted JSX pragma", ast({ kind: "identifier" }, { languageOptions: { jsxPragma: "a.b" } }), /Must be an identifier/],
    ["an unknown binding form in a filter", ast({ kind: "identifier", resolves: { binding: { form: "let" } } }), /Invalid option/],
    ["an unknown resolves key", ast({ kind: "identifier", resolves: { global: ["a"], scope: "file" } }), /Unrecognized key: \\"scope\\"/],
    ["directives other than eslint", ast({ kind: "identifier", resolves: { global: ["a"], directives: "jshint" } }), /Invalid input/],
    ["an empty value test", ast({ kind: "string", value: {} }), /value must hold a predicate/],
    ["arguments without a signature", ast({ kind: "call_expression", arguments: { absent: ["a"] } }), /expected array/],
    ["arguments that neither bind nor assert absence", ast({ kind: "call_expression", arguments: { signature: ["a"] } }), /must bind a slot or list absent ones/],
    ["a slot listed twice", ast({ kind: "call_expression", arguments: { signature: ["a", "a"], absent: ["a"] } }), /listed twice/],
    ["a star before a slash", document({ forbid: { ast: { language: "python", rule: { kind: "call", arguments: { signature: ["*", "a", "/"], absent: ["a"] } } } } }), /before \*/],
    ["an absent slot the signature lacks", ast({ kind: "call_expression", arguments: { signature: ["a"], absent: ["b"] } }), /slot of the signature/],
    ["a bound slot that is absent", ast({ kind: "call_expression", arguments: { signature: ["a"], bind: { A: "a" }, absent: ["a"] } }), /cannot be absent/],
    ["a bind name with a dollar", ast({ kind: "call_expression", arguments: { signature: ["a"], bind: { $A: "a" } } }), /metavariable name/],
    ["a malformed slot name", ast({ kind: "call_expression", arguments: { signature: ["a-b"], absent: ["a-b"] } }), /slot name/],
    ["an unknown arguments mode", document({ forbid: { ast: { language: "python", rule: { kind: "call", arguments: { signature: ["a"], absent: ["a"], mode: "ruff" } } } } }), /Invalid option/],
    ["a non-boolean follow", ast({ kind: "identifier", value: { eq: 1, follow: 1 } }), /expected boolean/],
    ["a value test that only follows", ast({ kind: "identifier", value: { follow: true } }), /value must hold a predicate/],
    ["a binding beside other resolves terms", ast({ kind: "identifier", resolves: { binding: {}, global: ["x"] } }), /write other resolves terms in a resolves of their own/],
    ["an unknown binding node", ast({ kind: "identifier", resolves: { binding: { values: { kind: "number" } } } }), /Unrecognized key: \\"values\\"/],
    ["a malformed binding sub-rule", ast({ kind: "identifier", resolves: { binding: { value: { kinds: "number" } } } }), /Unrecognized key: \\"kinds\\"/],
    ["a non-boolean only", ast({ kind: "identifier", resolves: { binding: { only: 1 } } }), /expected boolean/],
    ["a non-boolean destructuredValue", ast({ kind: "identifier", resolves: { binding: { destructuredValue: 0 } } }), /expected boolean/],
    ["an unknown value key", ast({ kind: "string", value: { equals: "a" } }), /Unrecognized key: \\"equals\\"/],
    ["an array eq", ast({ kind: "array", value: { eq: [1] } }), /Invalid input/],
    ["a zero bit mask", ast({ kind: "number", value: { bitsAny: 0 } }), /Too small/],
    ["a string bound", ast({ kind: "number", value: { gt: "1" } }), /expected number/],
    ["an empty prefix list", ast({ kind: "string", value: { startsWith: [] } }), /Too small/],
    ["a non-boolean truthy", ast({ kind: "string", value: { truthy: 1 } }), /expected boolean/],
    ["a malformed import name", document({ forbid: { ast: { language: "python", rule: { kind: "attribute", resolves: { import: ["os..path"] } } } } }), /Must be a dotted name/],
    ["global comments without global names", document({ forbid: { ast: { language: "python", rule: { kind: "attribute", resolves: { import: ["os"], directives: "eslint" } } } } }), /only add global names/],
    ["more than 1024 global names", ast({ resolves: { global: Array.from({ length: 1025 }, (_, index) => `n${index}`) } }), /Too big/],
    ["an empty imported list", document({ forbid: { ast: { language: "python", rule: { kind: "dotted_name", resolves: { imported: [] } } } } }), /Too small/],
    ["a prefix other than true", document({ forbid: { ast: { language: "python", rule: { kind: "dotted_name", resolves: { imported: ["a"], prefix: false } } } } }), /Invalid input/],
    ["a prefix without dotted names", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { global: ["a"], prefix: true } } } } }), /prefix needs some/],
    ["an unknown relative reading", document({ forbid: { ast: { language: "python", rule: { kind: "dotted_name", resolves: { imported: ["a"], relative: "level" } } } } }), /Invalid option/],
    ["relative without imported names", document({ forbid: { ast: { language: "python", rule: { kind: "attribute", resolves: { import: ["a"], relative: "text" } } } } }), /relative needs some/],
    ["an unknown pythonVersion", document({ forbid: { ast: { language: "python", languageOptions: { pythonVersion: "3.16" }, rule: { kind: "identifier" } } } }), /Invalid option/],
    ["a numeric pythonVersion", document({ forbid: { ast: { language: "python", languageOptions: { pythonVersion: 3.13 }, rule: { kind: "identifier" } } } }), /Invalid option/],
    ["a typing module that is no dotted name", document({ forbid: { ast: { language: "python", languageOptions: { typingModules: ["a..b"] }, rule: { kind: "identifier" } } } }), /Must be a dotted name/],
    ["a generic with a $", document({ forbid: { ast: { language: "python", languageOptions: { extendGenerics: ["lib.$Box"] }, rule: { kind: "identifier" } } } }), /Must be a dotted name/],
    ["more than 256 extra generics", document({ forbid: { ast: { language: "python", languageOptions: { extendGenerics: Array.from({ length: 257 }, () => "a") }, rule: { kind: "identifier" } } } }), /Too big/],
    ["an unknown ecmaVersion", ast({ kind: "identifier" }, { languageOptions: { ecmaVersion: 4 } }), /ecmaVersion must be .+, 3, 5, 6 to 17, or 2015 to 2026/],
    ["an ecmaVersion past the table", ast({ kind: "identifier" }, { languageOptions: { ecmaVersion: 2027 } }), /ecmaVersion must be/],
    ["a module before ES2015", ast({ kind: "identifier" }, { languageOptions: { ecmaVersion: 5 } }), /needs ecmaVersion 2015 or later/],
    ["an unknown sourceType", ast({ kind: "identifier" }, { languageOptions: { sourceType: "esm" } }), /Invalid option/],
    ["an unknown global setting", ast({ kind: "identifier" }, { languageOptions: { globals: { a: "on" } } }), /Invalid input/],
    ["a global that is no identifier", ast({ kind: "identifier" }, { languageOptions: { globals: { "a-b": "off" } } }), /Must be an identifier/],
    ["more than 16384 configured globals", ast({ kind: "identifier" }, { languageOptions: { globals: Object.fromEntries(Array.from({ length: 16385 }, (_, index) => [`n${index}`, "off"])) } }), /At most 16384 globals/],
    ["parser options other than ecmaFeatures", ast({ kind: "identifier" }, { languageOptions: { parserOptions: { ecmaVersion: 5 } } }), /Unrecognized key: \\"ecmaVersion\\"/],
    ["an ecmaFeature other than globalReturn or impliedStrict", ast({ kind: "identifier" }, { languageOptions: { parserOptions: { ecmaFeatures: { jsx: true } } } }), /Unrecognized key: \\"jsx\\"/],
    ["a non-boolean ecmaFeature", ast({ kind: "identifier" }, { languageOptions: { parserOptions: { ecmaFeatures: { globalReturn: 1 } } } }), /expected boolean/],
    ["sloppyFunctions other than true", ast({ kind: "this", resolves: { this: "global", sloppyFunctions: false } }, { languageOptions: {} }), /Invalid input/],
    ["sloppyFunctions without this: global", ast({ kind: "this", resolves: { this: "globalScope", sloppyFunctions: true } }, { languageOptions: {} }), /sloppyFunctions needs it/],
    ["a when that is no object", ast({ kind: "identifier" }, { when: [] }), /expected object/],
    ["a version gate without bounds", document({ forbid: { ast: { language: "python", when: { pythonVersion: {} }, rule: { kind: "identifier" } } } }), /Set min, max or both/],
    ["a version gate with an unknown bound", document({ forbid: { ast: { language: "python", when: { pythonVersion: { exact: "3.11" } }, rule: { kind: "identifier" } } } }), /Unrecognized key/],
    ["a version gate past the table", document({ forbid: { ast: { language: "python", when: { pythonVersion: { min: "3.16" } }, rule: { kind: "identifier" } } } }), /Invalid option/],
    ["a version gate whose min is after its max", document({ forbid: { ast: { language: "python", when: { pythonVersion: { min: "3.12", max: "3.10" } }, rule: { kind: "identifier" } } } }), /min is after max/],
    ["an unknown sourceType gate", ast({ kind: "identifier" }, { when: { sourceType: "esm" } }), /Invalid/],
    ["an empty sourceType gate list", ast({ kind: "identifier" }, { when: { sourceType: [] } }), /Too small|Invalid/],
    ["an empty ecmaFeatures gate", ast({ kind: "identifier" }, { when: { ecmaFeatures: {} } }), /Set globalReturn, impliedStrict or both/],
    ["an unknown ecmaFeatures gate", ast({ kind: "identifier" }, { when: { ecmaFeatures: { jsx: true } } }), /Unrecognized key/],
    ["an ecmaVersion gate outside the table", ast({ kind: "identifier" }, { when: { ecmaVersion: { max: 4 } } }), /ecmaVersion must be/],
    ["an ecmaVersion gate whose min is after its max", ast({ kind: "identifier" }, { when: { ecmaVersion: { min: 2020, max: 6 } } }), /min is after max/],
    ["a lib that is no list", ast({ kind: "identifier" }, { languageOptions: { lib: "dom" } }), /expected array/],
    ["a lib name that is no name", ast({ kind: "identifier" }, { languageOptions: { lib: ["dom/iterable"] } }), /Must be a TypeScript lib name/],
    ["a non-boolean declaration", ast({ kind: "identifier" }, { file: { declaration: "yes" } }), /expected boolean/],
    ["a path file condition that is no object", ast({ kind: "identifier" }, { file: { path: "^src/" } }), /expected object/],
    ["an empty file condition", document({ forbid: { ast: { language: "python", file: {}, rule: { kind: "module" } } } }), /Set path, declaration, module, moduleName, package, inPackage or jsxRuntime/],
    ["an unknown file condition", document({ forbid: { ast: { language: "python", file: { paths: { regex: "a" } }, rule: { kind: "module" } } } }), /Unrecognized key/],
    ["a file module that is no object", document({ forbid: { ast: { language: "python", file: { module: "^a$" }, rule: { kind: "module" } } } }), /expected object/],
    ["a non-boolean inPackage", document({ forbid: { ast: { language: "python", file: { inPackage: "yes" }, rule: { kind: "module" } } } }), /expected boolean/],
    ["a file text condition without regex or notRegex", document({ forbid: { ast: { language: "python", file: { moduleName: {} }, rule: { kind: "module" } } } }), /Set regex, notRegex or both/],
    ["an empty notRegex", document({ forbid: { ast: { language: "python", file: { module: { notRegex: "" } }, rule: { kind: "module" } } } }), /Must be a non-empty string/],
    ["this other than global or globalScope", ast({ kind: "this", resolves: { this: true } }, { languageOptions: {} }), /Invalid option/],
    ["an unknown this form", ast({ kind: "this", resolves: { this: "module" } }, { languageOptions: {} }), /Invalid option/],
    ["declared other than a boolean", ast({ kind: "identifier", resolves: { global: ["a"], declared: "no" } }, { languageOptions: {} }), /expected boolean/],
    ["declared without global names", ast({ kind: "this", resolves: { this: "global", declared: false } }, { languageOptions: {} }), /declared needs some/],
    ["declared false with global comments", ast({ kind: "identifier", resolves: { global: ["a"], declared: false, directives: "eslint" } }, { languageOptions: {} }), /which declared: false ignores/],
    ["scriptDeclarations other than true", ast({ kind: "identifier", resolves: { global: ["a"], scriptDeclarations: false } }, { languageOptions: {} }), /Invalid input/],
    ["scriptDeclarations without global names", ast({ kind: "this", resolves: { this: "global", scriptDeclarations: true } }, { languageOptions: {} }), /scriptDeclarations needs some/],
    ["a fix on a require_each check", relation("require_each", { fix: "y" }), /Only forbid clauses over an ast or file.content regex query can suggest a fix/],
    ["a start anchor on a budget check", relation("budget", { anchor: "witness.start" }), /support only witness.span/],
    ["a file start anchor on a budget check", relation("budget", { anchor: "file.start" }), /support only witness.span/],
    ["a capture anchor on an llm check", document({ forbid: undefined, llm: {}, violation: { ...violation, anchor: "capture.X" } }), /support only witness.span/],
    ["a host fact query", document({ forbid: { fact: { as: "s", relation: "symbols" } } }), /Unrecognized key|Invalid input/],
    ["a fix on a file.path regex", document({ forbid: { regex: { pattern: "x", source: "file.path" } }, violation: { ...violation, fix: "y" } }), /file.path regex match is not source text/],
    ["a capture anchor on a regex without that group", document({ violation: { ...violation, anchor: "capture.X" } }), /capture.X names no capture of the query/],
    ["a message variable on a group-less regex", document({ violation: { ...violation, message: "set $HOME" } }), /\$HOME names no capture/],
    ["a capture outside parse", py({ kind: "string", capture: "S" }), /capture binds a node of a parsed tree; use it inside parse/],
    ["an unknown grammar", py({ kind: "string", parse: { grammar: "yaml", kind: "format" } }), /grammar must be one of percentFormat, braceFormat/],
    ["a parse without a rule", py({ kind: "string", parse: { grammar: "percentFormat" } }), /parse must hold a rule over the parsed tree/],
    ["a kind the grammar has not", py({ parse: { grammar: "percentFormat", kind: "field" } }), /field is not a percentFormat kind/],
    ["a pattern inside a format tree", py({ parse: { grammar: "braceFormat", has: { pattern: "x" } } }), /pattern is not supported inside parse \(braceFormat trees\)/],
    ["resolves inside a format tree", py({ parse: { grammar: "percentFormat", has: { kind: "placeholder", resolves: { global: ["x"] } } } }), /resolves is not supported inside parse/],
    ["follow inside a parsed tree", py({ parse: { grammar: "percentFormat", has: { kind: "mapping_key", value: { eq: "a", follow: true } } } }), /no binding to follow/],
    ["a token comparison inside a parsed tree", py({ parse: { grammar: "braceFormat", compare: { left: "A", op: "eq", right: "B", text: "tokens" } } }), /compares by source, unquoted, value or node/],
    ["an unknown directive kind", ast({ kind: "comment", parse: { grammar: "eslintDirective", has: { kind: "name" } } }), /name is not a eslintDirective kind/],
    ["a pattern over a directive", ast({ kind: "comment", parse: { grammar: "eslintDirective", pattern: "$A" } }), /pattern is not supported inside parse \(eslintDirective trees\)/],
    ["a layout inside a directive", ast({ kind: "comment", parse: { grammar: "eslintDirective", has: { kind: "item", layout: { span: { eq: 0 } } } } }), /layout is not supported inside parse \(eslintDirective trees\)/],
    ["a host rule outside parse", ast({ kind: "comment", host: { kind: "program" } }), /host tests the literal a parse holds; use it inside parse/],
    ["a host rule in an annotation", py({ kind: "string", parse: { grammar: "annotation", kind: "identifier", host: { kind: "module" } } }), /host is not supported inside parse \(annotation trees\)/],
    ["a host rule in formatArgs' fields", py({ kind: "call", formatArgs: { style: "brace", string: "S", args: "A", missing: { kind: "field", host: { kind: "module" } } } }), /formatArgs' fields have none/],
    ["an item's resolves other than global", ast({ kind: "comment", parse: { grammar: "eslintDirective", has: { kind: "item", resolves: { unresolved: true } } } }), /resolves by global only/],
    ["resolves in a format string", py({ kind: "string", parse: { grammar: "percentFormat", kind: "placeholder", resolves: { global: true } } }), /resolves is not supported inside parse \(percentFormat trees\)/],
    ["a capture name with a dollar", py({ parse: { grammar: "percentFormat", has: { kind: "mapping_key", capture: "$K" } } }), /metavariable name without \$/],
    ["an unknown key inside parse", py({ parse: { grammar: "percentFormat", kinds: "format" } }), /Unrecognized key/],
    ["formatArgs without unused or missing", py({ pattern: "$S.format($$$A)", formatArgs: { style: "brace", string: "S", args: "A" } }), /formatArgs must hold unused, missing or both/],
    ["an unknown formatArgs style", py({ pattern: "$S.format($$$A)", formatArgs: { style: "c", string: "S", args: "A", unused: { kind: "identifier" } } }), /Invalid option/],
    ["a missing kind of the other grammar", py({ pattern: "$S.format($$$A)", formatArgs: { style: "brace", string: "S", args: "A", missing: { kind: "placeholder" } } }), /placeholder is not a braceFormat kind/],
    ["a capture in formatArgs' unused", py({ pattern: "$S.format($$$A)", formatArgs: { style: "brace", string: "S", args: "A", unused: { kind: "identifier", capture: "U" } } }), /capture binds a node of a parsed tree/],
    ["a binding inside an annotation", py({ parse: { grammar: "annotation", has: { kind: "identifier", resolves: { binding: { form: "import" } } } } }), /bindings' nodes are the host's/],
    ["a format grammar inside an annotation", py({ parse: { grammar: "annotation", has: { kind: "string", parse: { grammar: "percentFormat", kind: "format" } } } }), /parses as an annotation/],
    ["a calls through other than sync or any", py({ kind: "call", calls: { through: "async", kind: "identifier" } }), /Invalid option/],
    ["a calls chain with a dollar", py({ kind: "call", calls: { chain: "$C", kind: "identifier" } }), /metavariable name/],
    ["a calls rule holding calls", py({ kind: "call", calls: { has: { kind: "call", calls: { kind: "identifier" } } } }), /A calls rule cannot contain calls/],
    ["a calls inner rule with an unknown key", py({ kind: "call", calls: { kinds: "identifier" } }), /Unrecognized key/],
    ["a PHP import name with ASCII punctuation", php({ kind: "name", resolves: { import: ["@Foo"] } }), /Must be a dotted name/],
    ["a PHP import name with an empty segment", php({ kind: "name", resolves: { import: ["Foo\\\\Bar"] } }), /Must be a dotted name/],
    ["a PHP import name over 256 UTF-8 bytes", php({ kind: "name", resolves: { import: ["é".repeat(129)] } }), /Must be a dotted name/],
    ["a noqa code ending in a line feed", py({ kind: "identifier" }, {}, { suppress: { noqa: { codes: ["F401\n"] } } }), /Must be a rule code/],
    ["a PHPStan identifier ending in a line feed", php({ kind: "name" }, {}, { suppress: { phpstan: { identifiers: ["echo.nonString\n"] } } }), /Must be a PHPStan identifier/],
    ["a Psalm issue name ending in a carriage return", php({ kind: "name" }, {}, { suppress: { psalm: { issues: ["UndefinedVariable\r"] } } }), /Must be a Psalm issue name/],
    ["a PHP_CodeSniffer code ending in a line separator", php({ kind: "name" }, {}, { suppress: { phpcs: { sniffs: ["Squiz.PHP.DiscouragedFunctions\u2028"] } } }), /Must be a PHP_CodeSniffer sniff or message code/],
  ])("both reject %s", (_name, input, schemaError) => {
    const parsed = codeRuleSchema.safeParse(input);
    expect(parsed.success).toBe(false);
    expect(parsed.error?.message).toMatch(schemaError);
    expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).toThrow();
  });

  it.each([
    ["eslint suppression on a Python query", py({ kind: "identifier" }, {}, { suppress: { eslint: { rules: ["strict"] } } }), /eslint-disable comments are ESLint's; a python query has none/],
    ["oxlint suppression on a Python query", py({ kind: "identifier" }, {}, { suppress: { oxlint: { rules: ["no-console"] } } }), /oxlint-disable comments are Oxlint's; a python query has none/],
    ["biome suppression on a Python query", py({ kind: "identifier" }, {}, { suppress: { biome: { rules: ["lint/suspicious/noConsole"] } } }), /biome-ignore comments are Biome's; a python query has none/],
    ["noqa suppression on a TypeScript query", ast({ kind: "identifier" }, {}, { suppress: { noqa: { codes: ["F401"] } } }), /noqa comments are Ruff's; a typescript query has none/],
    ["psalm suppression on a Python query", py({ kind: "identifier" }, {}, { suppress: { psalm: { issues: ["UndefinedVariable"] } } }), /@psalm-suppress tags are Psalm's; a python query has none/],
    ["phpcs suppression on a TypeScript query", ast({ kind: "identifier" }, {}, { suppress: { phpcs: { sniffs: ["Squiz.PHP.DiscouragedFunctions"] } } }), /phpcs: annotations are PHP_CodeSniffer's; a typescript query has none/],
    ["a structural comparison in a TypeScript rule", ast({ pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "structure-parts" } }), /compare Python and PHP syntax/],
    ["property names in a Python rule", document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "key" } } } } }), /Python has none/],
    ["member names in a Python rule", document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "member" } } } } }), /Python has none/],
    ["ESTree names in a Python rule", document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "name" } } } } }), /use unquoted for Python/],
    ["references in a Python rule", document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "reference" } } } } }), /use structure for Python/],
    ["star imports in a TypeScript rule", ast({ kind: "identifier", resolves: { unresolved: "star" } }), /TypeScript has none/],
    ["beforeLocal in a TypeScript rule", ast({ kind: "identifier", resolves: { beforeLocal: true } }), /TypeScript has none/],
    ["import: true in a TypeScript rule", ast({ kind: "identifier", resolves: { import: true } }), /import resolution is Python-only/],
    ["sameImport in a TypeScript rule", ast({ kind: "identifier", binds: { redefines: { sameImport: true } } }), /qualified import names are Python's/],
    ["every in a Python rule", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { redefines: { every: true } } } } } }), /`every` is TypeScript's/],
    ["builtin in a Python filter", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { builtin: true } } } } }), /configured globals are ESLint's/],
    ["an occurrence in a Python rule", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { occurrence: { kind: "identifier" } } } } } }), /occurrences are TypeScript's/],
    ["write in a Python rule", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { write: "last" } } } } }), /where ESLint reports a variable is TypeScript's/],
    ["a Python binding flag in TypeScript binds", ast({ kind: "identifier", binds: { outer: false } }), /TypeScript has none/],
    ["a Python scope kind in a TypeScript filter", ast({ kind: "identifier", binds: { scope: "lambda" } }), /`"lambda"` is not a scope kind/],
    ["a TypeScript scope kind in a Python filter", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { scope: ["module", "block"] } } } } }), /`"block"` is not a scope kind/],
    ["exported comments in a Python filter", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { directives: "eslint" } } } } }), /comments are ESLint's; Python has none/],
    ["a JSX pragma on a Python query", document({ forbid: { ast: { language: "python", languageOptions: { jsxPragma: "h" }, rule: { kind: "identifier" } } } }), /unknown field `jsxPragma`/],
    ["a Python binding flag on a TypeScript binding", ast({ kind: "identifier", resolves: { binding: { latest: true } } }), /TypeScript has none/],
    ["global comments on unresolved Python names", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { unresolved: true, directives: "eslint" } } } } }), /comments are ESLint's; Python has none/],
    ["global comments in a Python rule", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { global: ["a"], directives: "eslint" } } } } }), /comments are ESLint's; Python has none/],
    ["import resolution in a TypeScript rule", ast({ kind: "identifier", resolves: { import: ["react.useState"] } }), /import resolution is Python-only/],
    ["keyword-only slots in TypeScript", ast({ kind: "call_expression", arguments: { signature: ["a", "*", "b"], absent: ["b"] } }), /JavaScript passes arguments by position only/],
    ["syntactic arguments in TypeScript", ast({ kind: "call_expression", arguments: { signature: ["a"], absent: ["a"], mode: "syntactic" } }), /TypeScript has no keywords/],
    ["ESTree arguments in Python", document({ forbid: { ast: { language: "python", rule: { kind: "call", arguments: { signature: ["a"], absent: ["a"], mode: "estree" } } } } }), /use language or syntactic for Python/],
    ["stopAtGlobal in a TypeScript rule", ast({ kind: "identifier", resolves: { binding: { stopAtGlobal: true } } }), /TypeScript has none/],
    ["destructuredValue in a Python rule", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { binding: { destructuredValue: false } } } } } }), /TypeScript's/],
    ["a Python type name in a binding's value test", ast({ kind: "identifier", resolves: { binding: { value: { value: { type: "str" } } } } }), /`str` is not a TypeScript type name/],
    ["a Python type name in a TypeScript rule", ast({ kind: "string", value: { type: "str" } }), /`str` is not a TypeScript type name/],
    ["a TypeScript type name in a Python rule", document({ forbid: { ast: { language: "python", rule: { kind: "string", value: { type: ["str", "string"] } } } } }), /`string` is not a Python type name/],
    ["imported names in a TypeScript rule", ast({ kind: "identifier", resolves: { imported: ["react"] } }), /import resolution is Python-only/],
    ["an ecmaVersion on a Python query", document({ forbid: { ast: { language: "python", languageOptions: { ecmaVersion: 5 }, rule: { kind: "identifier" } } } }), /needs ecmaVersion 2015 or later/],
    ["a pythonVersion on a TypeScript query", ast({ kind: "identifier" }, { languageOptions: { pythonVersion: "3.13" } }), /unknown field `pythonVersion`/],
    ["typingModules on a TypeScript query", ast({ kind: "identifier" }, { languageOptions: { typingModules: ["compat"] } }), /unknown field `typingModules`/],
    ["parser options on a Python query", document({ forbid: { ast: { language: "python", languageOptions: { parserOptions: {} }, rule: { kind: "identifier" } } } }), /unknown field `parserOptions`/],
    ["an ecmaVersion gate on a Python query", document({ forbid: { ast: { language: "python", when: { ecmaVersion: { min: 5 } }, rule: { kind: "identifier" } } } }), /unknown field `ecmaVersion`/],
    ["a pythonVersion gate on a TypeScript query", ast({ kind: "identifier" }, { when: { pythonVersion: { min: "3.10" } } }), /unknown field `pythonVersion`/],
    ["a sourceType gate on a Python query", document({ forbid: { ast: { language: "python", when: { sourceType: "script" }, rule: { kind: "identifier" } } } }), /unknown field `sourceType`/],
    ["a file condition on a TypeScript query", ast({ kind: "identifier" }, { file: { inPackage: true } }), /module paths are Python's/],
    ["a lib on a Python query", document({ forbid: { ast: { language: "python", languageOptions: { lib: ["dom"] }, rule: { kind: "identifier" } } } }), /unknown field `lib`/],
    ["a module file condition on a TypeScript query", ast({ kind: "identifier" }, { file: { module: { regex: "a" } } }), /module paths are Python's/],
    ["a declaration file condition on a Python query", document({ forbid: { ast: { language: "python", file: { declaration: true }, rule: { kind: "module" } } } }), /declaration files are TypeScript's/],
    ["a JSX runtime on python", document({ forbid: { ast: { language: "python", file: { jsxRuntime: ["react"] }, rule: { kind: "module" } } } }), /JSX runtimes are TypeScript's/],
    ["a package manifest condition on python", document({ forbid: { ast: { language: "python", file: { package: { bin: true } }, rule: { kind: "module" } } } }), /python's package is a regex or notRegex/],
    ["a text package condition on typescript", ast({ kind: "identifier" }, { file: { package: { regex: "a" } } }), /typescript's package reads the nearest package.json/],
    ["this without language options", ast({ kind: "this", resolves: { this: "global" } }), /declare languageOptions/],
    ["declared without language options", ast({ kind: "identifier", resolves: { global: ["a"], declared: false } }), /declare languageOptions/],
    ["scriptDeclarations without language options", ast({ kind: "identifier", resolves: { global: ["a"], scriptDeclarations: true } }), /declare languageOptions/],
    ["scriptDeclarations in a Python rule", document({ forbid: { ast: { language: "python", languageOptions: {}, rule: { kind: "identifier", resolves: { global: ["a"], scriptDeclarations: true } } } } }), /scripts are ESLint's/],
    ["this in a Python rule", document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { this: "global" } } } } }), /`this` is TypeScript's/],
    ["a parse in a TypeScript rule", ast({ kind: "string", parse: { grammar: "percentFormat", kind: "format" } }), /`percentFormat` parses Python literals/],
    ["a directive parse in a Python rule", py({ kind: "comment", parse: { grammar: "eslintDirective", kind: "directive" } }), /`eslintDirective` parses TypeScript comments/],
    ["formatArgs in a TypeScript rule", ast({ pattern: "f($$$A)", formatArgs: { style: "brace", string: "S", args: "A", unused: { kind: "identifier" } } }), /`formatArgs` is not available for typescript policies/],
    ["calls in a TypeScript rule", ast({ kind: "call_expression", calls: { kind: "function_declaration" } }), /`calls` is not available for typescript policies/],
    ["a PHP-qualified Python import", py({ kind: "attribute", resolves: { import: ["App\\Client"] } }), /is a PHP name; Python imports are dotted names/],
    ["a PHP-qualified Python imported module", py({ kind: "dotted_name", resolves: { imported: ["App\\Client"] } }), /is a PHP name; Python imports are dotted names/],
  ])("refuses %s in the engine, which the rule's language lacks", (_name, input, reason) => {
    const validation = validateCodeRule(input);
    expect(validation).toMatchObject({ ok: false, code: "invalid-rule" });
    expect(validation.ok ? undefined : validation.reason).toMatch(reason);
  });

  it("both accept PHP names of non-ASCII labels up to 256 UTF-8 bytes, and PHP suppression names", () => {
    for (const input of [
      php({ kind: "name", resolves: { import: ["\\Ünïcödé\\Klasse", "é".repeat(128), "_\u{1F600}9"] } }),
      php({ kind: "name", resolves: { imported: ["App\\Http"], prefix: true } }),
      php({ kind: "name" }, {}, { suppress: { phpstan: { identifiers: ["echo.nonString"] }, psalm: { issues: ["Undefined-Variable_2"] }, phpcs: { sniffs: ["Squiz.PHP.DiscouragedFunctions.Found"] } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).error).toBeUndefined();
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  const shared = (utils: Record<string, unknown>, language = "typescript") => ({ id: `shared-${language}`, define: { language, utils } });
  const clauses = (...list: unknown[]) => ({ clauses: list, format: "rerail.policy/v1", id: "x", mode: "steer", revision: 1 });
  const tsQuery = (id: string, rule: unknown, extra: Record<string, unknown> = {}) => ({ id, violation: violation, forbid: { ast: { language: "typescript", rule, ...extra } } });

  it.each([
    ["a define with a finding", clauses({ ...shared({ u: { kind: "identifier" } }), violation: violation }, tsQuery("q", { matches: "u" })), /A define clause reports nothing, so it has no violation/],
    ["a define without a language", clauses({ id: "d", define: { utils: { u: { kind: "identifier" } } } }, tsQuery("q", { kind: "identifier" })), /language is required/],
    ["a define without utils", clauses({ id: "d", define: { language: "typescript" } }, tsQuery("q", { kind: "identifier" })), /define must hold utils/],
    ["a define beside another form", clauses({ ...shared({ u: { kind: "identifier" } }), forbid: regex }, tsQuery("q", { kind: "identifier" })), /exactly one constraint form/],
    ["a document of defines alone", clauses(shared({ u: { kind: "identifier" } })), /clauses must hold a clause besides define/],
    ["a check without a finding", clauses({ id: "q", forbid: regex }), /violation is required/],
    ["a utility defined twice", clauses(shared({ u: { kind: "identifier" } }), { id: "e", define: { language: "typescript", utils: { u: { kind: "number" } } } }, tsQuery("q", { matches: "u" })), /already defined by another define of typescript/],
    ["a shared utility naming a query's", clauses(shared({ u: { matches: "local" } }), tsQuery("q", { matches: "u" }, { utils: { local: { kind: "identifier" } } })), /Utility rule local is not defined/],
    ["a query utility a define also names", clauses(shared({ u: { kind: "identifier" } }), tsQuery("q", { matches: "u" }, { utils: { u: { kind: "number" } } })), /also defined by a define of typescript; rename one/],
    ["a shared utility of another language", clauses(shared({ u: { kind: "identifier" } }), { id: "q", violation: violation, forbid: { ast: { language: "python", rule: { matches: "u" } } } }), /Utility rule u is not defined/],
    ["a malformed shared utility", clauses(shared({ u: { kind: "identifier", stopBy: "end" } }), tsQuery("q", { kind: "identifier" })), /Unrecognized key/],
  ])("both reject %s", (_name, input, schemaError) => {
    const parsed = codeRuleSchema.safeParse(input);
    expect(parsed.success).toBe(false);
    expect(parsed.error?.message).toMatch(schemaError);
    expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).toThrow();
  });

  const callee = { params: ["F"], rule: { has: { field: "function", pattern: "$F" } } };
  it.each([
    ["a parameterized utility named without arguments", ast({ matches: "callee" }, { utils: { callee } }), /takes parameters \(F\)/],
    ["a call of a utility without parameters", ast({ matches: { plain: { F: "$C" } } }, { utils: { plain: { kind: "call_expression" } } }), /takes no parameters/],
    ["a call with an unknown parameter", ast({ matches: { callee: { G: "$C" } } }, { utils: { callee } }), /G is not a parameter of callee/],
    ["a call that leaves a parameter out", ast({ matches: { callee: {} } }, { utils: { callee } }), /Parameter F of callee is not passed/],
    ["an argument that names no capture", ast({ matches: { callee: { F: "C" } } }, { utils: { callee } }), /A parameter names a capture/],
    ["a multi-capture argument", ast({ matches: { callee: { F: "$$$C" } } }, { utils: { callee } }), /A parameter names a capture/],
    ["a call of two utilities", ast({ matches: { callee: { F: "$C" }, other: { F: "$D" } } }, { utils: { callee } }), /matches must call one utility/],
    ["empty parameters", ast({ kind: "number" }, { utils: { u: { params: [], rule: { kind: "number" } } } }), /Too small/],
    ["a parameter listed twice", ast({ kind: "number" }, { utils: { u: { params: ["A", "A"], rule: { kind: "number" } } } }), /A parameter is listed twice/],
    ["a lower-case parameter", ast({ kind: "number" }, { utils: { u: { params: ["a"], rule: { kind: "number" } } } }), /Must be a metavariable name/],
    ["parameters without a rule", ast({ kind: "number" }, { utils: { u: { params: ["A"] } } }), /Unrecognized key: \\"params\\"/],
    ["a shared call that leaves a parameter out", clauses(shared({ callee }), tsQuery("q", { matches: { callee: {} } })), /Parameter F of callee is not passed/],
  ])("both reject %s", (_name, input, schemaError) => {
    const parsed = codeRuleSchema.safeParse(input);
    expect(parsed.success).toBe(false);
    expect(parsed.error?.message).toMatch(schemaError);
    expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).toThrow();
  });

  it("both accept parameterized utilities, recursive and shared", () => {
    const pair = {
      params: ["X", "Y"],
      rule: {
        pattern: "$X",
        any: [
          { inside: { field: "right", kind: "assignment_expression", has: { field: "left", pattern: "$Y" } } },
          {
            all: [
              { inside: { kind: "array", matches: { pair: { X: "$XP", Y: "$YP" } } } },
              { inside: { stopBy: "end", kind: "assignment_expression", has: { field: "left", has: { stopBy: "end", pattern: "$Y", compare: { left: "Y", op: "eq", right: "X", text: "index" }, inside: { pattern: "$YPC", compare: { left: "YPC", op: "eq", right: "YP", text: "node" } } } } } },
            ],
          },
        ],
      },
    };
    const selfAssign = { kind: "identifier", pattern: "$R", matches: { pair: { X: "$R", Y: "$L" } }, compare: { left: "L", op: "eq", right: "R", text: "name" } };
    for (const input of [
      ast({ kind: "call_expression", matches: { callee: { F: "$C" } } }, { utils: { callee } }, { message: "$C" }),
      ast(selfAssign, { utils: { pair } }, { message: "$L", anchor: "capture.L" }),
      clauses(shared({ pair }), tsQuery("q", selfAssign)),
      ast({ kind: "call_expression", not: { matches: { callee: { F: "$C" } } } }, { utils: { callee } }),
      ast({ pattern: "$A($B)", matches: { u: { P: "$A" } } }, { utils: { u: { params: ["P"], rule: { has: { field: "arguments", has: { matches: "v" } } } }, v: { pattern: "$Q", compare: { left: "P", op: "ne", right: "Q", text: "tokens" } } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).error).toBeUndefined();
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  const optionsClause = (options: Record<string, unknown>, config: Record<string, unknown> = {}) => ({ id: "options", define: { options, config } });
  const unusedOptions = {
    vars: { type: "string", enum: ["all", "local"], default: "all" },
    args: { type: "string", enum: ["all", "after-used", "none"], default: "after-used" },
    varsIgnorePattern: { type: "string", nullable: true, default: null },
    allowed: { type: "array", default: ["a.b", "c"] },
  };
  const unusedClauses = [
    tsQuery("vars", {
      kind: "identifier",
      inside: { kind: "variable_declarator", field: "name" },
      all: [
        { any: [{ inside: { kind: "function_declaration", stopBy: "end" } }, { $option: "vars", cases: { all: { kind: "identifier" }, local: { regex: "[^\\s\\S]" } } }] },
        { $option: "varsIgnorePattern", cases: { null: [] }, default: [{ not: { regex: { $option: "varsIgnorePattern" } } }], spread: true },
      ],
    }),
    tsQuery("args", { kind: "identifier", inside: { kind: "required_parameter" } }, { when: { option: { args: { in: ["all", "after-used"] } } } }),
    tsQuery("allowed", { kind: "identifier", regex: { $option: "allowed", escape: "regex", join: "|", template: "^(?:{})$" } }),
  ];
  const snapshotOf = (source: string) =>
    JSON.stringify({ facts: [], files: [{ language: "typescript", path: "a.ts", source }], format: "rerail.repository-snapshot/v1", revision: 0 });

  it("both resolve options identically, and a variant is a config patch", () => {
    const source = "const a = 1;\nconst _b = 2;\nfunction f(p) { const c = 3; }\na.b; ab; c;\n";
    for (const config of [{}, { vars: "local", varsIgnorePattern: "^_", args: "none" }, { varsIgnorePattern: "^_", allowed: [] }]) {
      const written = clauses(optionsClause(unusedOptions), ...unusedClauses);
      const patched = applyCodeRuleConfig(written, config);
      const parsed = codeRuleSchema.parse(patched);
      expect(JSON.stringify(parsed)).not.toContain("$option");
      expect(parsed.clauses[0]?.define?.config).toEqual(config);
      const runtime = loadNativeBinding().CodeRuleDslRuntime;
      const violations = (document: unknown) => JSON.parse(runtime.compile(JSON.stringify(document), "json", bundledCapabilities).evaluate(snapshotOf(source), "report")).violations;
      expect(violations(parsed)).toEqual(violations(patched));
    }
    expect(() => applyCodeRuleConfig(clauses(optionsClause(unusedOptions), ...unusedClauses), { argz: "none" })).toThrow(/argz` is not an option this document declares/);
    expect(() => applyCodeRuleConfig(clauses(optionsClause(unusedOptions), ...unusedClauses), { args: "some" })).toThrow(/not one of the option's enum values/);
  });

  it("both spread a reference among the clauses into them", () => {
    const on = { on: { type: "boolean", default: true } };
    const spread = (list: unknown[]) => ({ $option: "on", cases: { false: [] }, default: list, spread: true });
    const classes = tsQuery("c", { kind: "class_declaration" });
    const numbers = tsQuery("n", { kind: "number" });
    const runtime = loadNativeBinding().CodeRuleDslRuntime;
    const violations = (document: unknown) =>
      (JSON.parse(runtime.compile(JSON.stringify(document), "json", bundledCapabilities).evaluate(snapshotOf("class A {}\n1;\n"), "report")).violations as { clauseId: string }[]).map(violation => violation.clauseId);
    for (const [written, expected] of [
      [clauses(optionsClause(on), spread([classes])), ["c"]],
      [clauses(optionsClause(on), spread([classes]), numbers), ["c", "n"]],
      [clauses(optionsClause(on, { on: false }), spread([classes]), numbers), ["n"]],
      [clauses(optionsClause(on), { $option: "on", cases: { true: classes, false: numbers } }), ["c"]],
    ] as const) {
      const parsed = codeRuleSchema.parse(written);
      expect(JSON.stringify(parsed)).not.toContain("$option");
      expect(violations(written)).toEqual(expected);
      expect(violations(parsed)).toEqual(expected);
    }
    for (const [input, schemaError, engineError] of [
      [clauses(optionsClause(on, { on: false }), spread([classes])), /clauses must hold a clause besides define/, /clauses must hold a clause besides define/],
      [
        clauses(optionsClause(on), spread([{ id: "d", define: { options: { x: { type: "boolean", default: true } } } }]), classes),
        /a define is written among the clauses, not made by a \$option reference/,
        /\/clauses\/1: a define is written among the clauses, not made by a \$option reference/,
      ],
      [clauses(optionsClause(on), { $option: "on", define: { options: {} } }, classes), /unknown field `define`/, /unknown field `define` at \/clauses\/1/],
    ] as const) {
      const parsed = codeRuleSchema.safeParse(input);
      expect(parsed.success).toBe(false);
      expect(parsed.error?.message).toMatch(schemaError);
      expect(() => runtime.compile(JSON.stringify(input), "json", bundledCapabilities)).toThrow(engineError);
    }
  });

  it.each([
    ["an undeclared option", clauses(optionsClause({ s: { type: "string", default: "a" } }), tsQuery("q", { regex: { $option: "t" } })), /`t` is not an option this document declares/],
    ["a default of another type", clauses(optionsClause({ s: { type: "string", default: 1 } }), tsQuery("q", { kind: "number" })), /is not a value of the option's type/],
    ["a default outside the enum", clauses(optionsClause({ s: { type: "string", enum: ["a"], default: "b" } }), tsQuery("q", { kind: "number" })), /not one of the option's enum values/],
    ["a null default of an option that is not nullable", clauses(optionsClause({ s: { type: "string", default: null } }), tsQuery("q", { kind: "number" })), /null is not allowed/],
    ["an array item of another type", clauses(optionsClause({ s: { type: "array", items: "integer", default: [1.5] } }), tsQuery("q", { kind: "number" })), /is not an allowed item of the option/],
    ["an option without a default", clauses(optionsClause({ s: { type: "string" } }), tsQuery("q", { kind: "number" })), /default/],
    ["an option name with a digit first", clauses(optionsClause({ "1s": { type: "string", default: "a" } }), tsQuery("q", { kind: "number" })), /starts with a letter or _/],
    ["a config for an undeclared option", clauses(optionsClause({ s: { type: "string", default: "a" } }, { t: "a" }), tsQuery("q", { kind: "number" })), /`t` is not an option this document declares/],
    ["a config of another type", clauses(optionsClause({ s: { type: "string", default: "a" } }, { s: true }), tsQuery("q", { kind: "number" })), /is not a value of the option's type/],
    ["an option declared twice", clauses({ id: "a", define: { options: { s: { type: "string", default: "a" } } } }, { id: "b", define: { options: { s: { type: "string", default: "a" } } } }, tsQuery("q", { kind: "number" })), /already declared by another define/],
    ["a spread outside an array", clauses(optionsClause({ s: { type: "array", default: [] } }), tsQuery("q", { regex: { $option: "s", spread: true } })), /only an array's item may spread/],
    ["a case missing without a default", clauses(optionsClause({ s: { type: "string", default: "a" } }), tsQuery("q", { regex: { $option: "s", cases: { b: "x" } } })), /no case for `a` and no default/],
    ["a join of a string", clauses(optionsClause({ s: { type: "string", default: "a" } }), tsQuery("q", { regex: { $option: "s", join: "|" } })), /the option's value is not an array/],
    ["an unknown reference key", clauses(optionsClause({ s: { type: "string", default: "a" } }), tsQuery("q", { regex: { $option: "s", suffix: "x" } })), /unknown field `suffix`/],
    ["a when.option naming no option", clauses(optionsClause({ s: { type: "string", default: "a" } }), tsQuery("q", { kind: "number" }, { when: { option: { t: "a" } } })), /`t` is not an option this document declares/],
    ["a when.option with an unknown condition", clauses(optionsClause({ s: { type: "string", default: "a" } }), tsQuery("q", { kind: "number" }, { when: { option: { s: { startsWith: "a" } } } })), /Invalid input/],
    ["a reference that resolves to an invalid rule", clauses(optionsClause({ s: { type: "string", default: "" } }), tsQuery("q", { regex: { $option: "s" } })), /Must be a non-empty string/],
  ])("both reject %s", (_name, input, schemaError) => {
    const parsed = codeRuleSchema.safeParse(input);
    expect(parsed.success).toBe(false);
    expect(parsed.error?.message).toMatch(schemaError);
    expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).toThrow();
  });

  it("both accept options in scopes, shared utilities and a define of options alone", () => {
    const shared = { id: "shared", define: { language: "typescript", utils: { named: { kind: "identifier", regex: { $option: "name", template: "^{}$" } } } } };
    for (const input of [
      { ...clauses(optionsClause({ roots: { type: "array", default: ["src"] } }), tsQuery("q", { kind: "number" })), scope: { include: { $option: "roots", map: "{}/**" }, exclude: [] } },
      clauses(optionsClause({ name: { type: "string", default: "x" } }, { name: "y" }), shared, tsQuery("q", { matches: "named" })),
      clauses(optionsClause({ strict: { type: "boolean", default: false } }), tsQuery("q", { kind: "number" }, { when: { option: { strict: { not: true } } } })),
      clauses(optionsClause({ strict: { type: "boolean", default: false } }), tsQuery("q", { kind: "number" }, { when: { option: {} } })),
      clauses(optionsClause({ strict: { type: "boolean", default: false } }), tsQuery("q", { kind: "number" }, { when: { option: { $option: "strict", cases: { true: { strict: true } }, default: {} } } })),
      clauses(optionsClause({ s: { type: "string", default: "$option" } }), tsQuery("q", { kind: "number" })),
    ]) {
      expect(codeRuleSchema.safeParse(input).error).toBeUndefined();
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both resolve options inside suppress, layout, transforms and languageOptions", () => {
    const options = optionsClause({
      disable: { type: "array", default: ["names"] },
      codes: { type: "array", default: ["F401"] },
      maxBreaks: { type: "integer", default: 0 },
      missing: { type: "string", default: "none" },
      browser: { type: "boolean", default: false },
    });
    const names = {
      id: "names",
      forbid: {
        ast: {
          language: "typescript",
          rule: {
            kind: "identifier",
            pattern: "$X",
            regex: "^[a-d]$",
            inside: { kind: "array" },
            any: [{ pattern: "$Y", regex: "^never$" }, { kind: "identifier" }],
            layout: { before: { to: "token", breaks: { max: { $option: "maxBreaks" } } } },
          },
          transform: { T: { replace: { source: "$X", replace: "^", by: { $option: "missing" } } }, D: { substring: { source: "$Y", default: { $option: "missing" } } } },
        },
      },
      violation: { ...violation, anchor: "capture.X", message: "$X ${T} ${D}", suppress: { eslint: { rules: { $option: "disable" } } } },
    };
    const window = tsQuery("window", { kind: "identifier", regex: "^window$", resolves: { global: ["window"] } }, { languageOptions: { globals: { $option: "browser", cases: { true: { window: "readonly" } }, default: {} } } });
    const noqa = { id: "noqa", forbid: { ast: { language: "python", rule: { kind: "import_statement" } } }, violation: { ...violation, suppress: { noqa: { codes: { $option: "codes" } } } } };
    const files = [
      { language: "typescript", path: "a.ts", source: "const v = [a,\n  // eslint-disable-next-line names\n  b,\n\n  c, d];\nwindow.x;\n" },
      { language: "python", path: "a.py", source: "import os  # noqa: F401\n" },
    ];
    const snapshot = JSON.stringify({ facts: [], files, format: "rerail.repository-snapshot/v1", revision: 0 });
    const runtime = loadNativeBinding().CodeRuleDslRuntime;
    const reported = (document: unknown) =>
      (JSON.parse(runtime.compile(JSON.stringify(document), "json", bundledCapabilities).evaluate(snapshot, "report")).violations as { clauseId: string; message: string }[]).map(
        found => `${found.clauseId}: ${found.message}`,
      );
    const configurations: [Parameters<typeof applyCodeRuleConfig>[1], string[]][] = [
      [{}, ["names: a nonea none", "names: d noned none"]],
      [{ disable: ["other"], codes: ["E501"], maxBreaks: 2, missing: "-", browser: true }, ["names: a -a -", "names: b -b -", "names: c -c -", "names: d -d -", "noqa: x", "window: x"]],
    ];
    for (const [config, expected] of configurations) {
      const written = applyCodeRuleConfig(clauses(options, names, window, noqa), config);
      const parsed = codeRuleSchema.parse(written);
      expect(JSON.stringify(parsed)).not.toContain("$option");
      expect(reported(parsed).sort()).toEqual(expected);
      expect(reported(written).sort()).toEqual(expected);
    }
  });

  it("both accept parameterized utilities over layouts, parsed captures, transforms and suppression", () => {
    const near = { params: ["P"], rule: { layout: { before: { to: "capture.P", breaks: { eq: 0 } } } } };
    const items = { params: ["N"], rule: { kind: "comment", parse: { grammar: "eslintDirective", value: { regex: "^globals?$" }, has: { kind: "item", capture: "N" } } } };
    const arg = { params: ["X"], rule: { has: { field: "arguments", has: { kind: "number", pattern: "$X" } } } };
    const callee = { stopBy: "end", kind: "call_expression", has: { field: "function", pattern: "$C" } };
    for (const input of [
      ast({ kind: "identifier", inside: callee, not: { matches: { near: { P: "$C" } } } }, { utils: { near } }),
      ast({ kind: "identifier", inside: callee, all: [{ matches: { near: { P: "$C" } } }] }, { utils: { near } }),
      ast({ matches: { items: { N: "$G" } } }, { utils: { items } }, { anchor: "capture.G", each: "G", message: "global ${G}!", fix: { template: "${G}2", range: "capture.G" }, suppress: { eslint: { rules: ["globals"] } } }),
      ast(
        { kind: "call_expression", any: [{ pattern: "f()" }, { pattern: "f($$$)", matches: { arg: { X: "$A" } } }] },
        { utils: { arg }, transform: { V: { substring: { source: "$A", value: true, default: "none" } }, L: { line: { source: "$A", default: "?" } } } },
        { message: "${V} on ${L}" },
      ),
    ]) {
      expect(codeRuleSchema.safeParse(input).error).toBeUndefined();
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept utilities a define shares with the queries of its language", () => {
    for (const input of [
      clauses(shared({ u: { kind: "identifier" } }), tsQuery("q", { matches: "u" })),
      clauses(shared({ u: { kind: "identifier", resolves: { global: ["window"] } } }), tsQuery("q", { matches: "u" }, { languageOptions: { globals: { window: "readonly" } } })),
      clauses(tsQuery("q", { kind: "arguments", has: { matches: "local" } }, { utils: { local: { matches: "u" } } }), shared({ u: { kind: "identifier", not: { matches: "v" } }, v: { regex: "^_" } })),
      clauses(shared({ tail: { any: [{ kind: "number" }, { kind: "parenthesized_expression", has: { matches: "tail" } }] } }), tsQuery("q", { kind: "return_statement", has: { matches: "tail" } })),
      clauses(shared({ u: { kind: "identifier" } }), shared({ u: { kind: "call" } }, "python"), tsQuery("q", { matches: "u" }), { id: "p", violation: violation, forbid: { ast: { language: "python", rule: { matches: "u" } } } }),
      clauses(shared({ u: { kind: "identifier" } }), { id: "sw", violation: violation, forbid: { select_when: { select: { ast: { language: "typescript", rule: { matches: "u" } } }, exists: { ast: { language: "typescript", rule: { kind: "number", inside: { matches: "u", stopBy: "end" } } } } } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).error).toBeUndefined();
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept a field on inside and has, in rules, utilities and constraints", () => {
    for (const input of [
      ast({ kind: "identifier", inside: { kind: "variable_declarator", field: "name" } }),
      ast({ kind: "function_declaration", has: { field: "body", stopBy: "end", kind: "call_expression" } }),
      ast({ kind: "function_declaration", not: { has: { field: "body", stopBy: { kind: "return_statement" }, kind: "expression_statement" } } }),
      ast({ matches: "u" }, { utils: { u: { kind: "identifier", inside: { kind: "if_statement", field: "condition", stopBy: "end" } } } }),
      ast({ pattern: "f($A)" }, { constraints: { A: { has: { field: "function", kind: "identifier" } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "yield", not: { inside: { kind: "function_definition", field: "body", stopBy: "end" } } } } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept rules over ESLint's directive comments", () => {
    for (const input of [
      ast({ kind: "comment", parse: { grammar: "eslintDirective", kind: "directive" } }),
      ast(
        { kind: "comment", parse: { grammar: "eslintDirective", value: { regex: "^globals?$" }, has: { kind: "item", capture: "N", not: { has: { kind: "setting", value: { eq: "off" } } } } } },
        {},
        { anchor: "capture.N", message: "'$N' is already defined.", each: "N" },
      ),
      ast({ kind: "comment", not: { parse: { grammar: "eslintDirective", any: [{ has: { kind: "description" } }, { value: { type: "string" } }] } } }),
      ast(
        {
          kind: "comment",
          parse: {
            grammar: "eslintDirective",
            value: { regex: "^globals?$" },
            has: {
              kind: "item",
              capture: "N",
              any: [
                { resolves: { global: true } },
                { host: { inside: { kind: "program", has: { stopBy: "end", kind: "variable_declarator", has: { field: "name", pattern: "$V" }, compare: { left: "V", op: "eq", right: "N", text: "name" } } } } },
              ],
            },
          },
        },
        { languageOptions: { globals: { conf: "readonly" } } },
        { anchor: "capture.N", message: "'$N' is already defined.", each: "N" },
      ),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept layouts and opt-in suppression", () => {
    for (const input of [
      ast({ pattern: "function $F($$$P) { $$$ }" }, { transform: { N: { count: { source: "$$$P" } }, L: { line: { source: "$F", edge: "end" } }, S: { lines: { source: "$$$P" } } } }, { message: "$F has $N parameters from line $L over $S lines" }),
      ast({ any: [{ pattern: "f($A)" }, { pattern: "g()" }] }, { transform: { L: { line: { source: "$A", default: "none" } }, S: { lines: { source: "$A", default: "0" } }, N: { count: { source: "$A", default: "0" } } } }, { message: "line ${L}, ${S} lines, ${N} nodes" }),
      ast({ kind: "identifier", layout: { span: { eq: 0 }, start: { min: 0, max: 0 } } }),
      ast({ kind: "pair", layout: { before: { to: "sibling", blank: { min: 1 } } } }),
      ast({ kind: "comment", layout: { before: { to: "token", comments: true, edge: true, breaks: { eq: 0 } }, after: { to: "token", breaks: { min: 1 }, blank: { max: 0 } } } }),
      ast({ kind: "string", follows: { kind: "string", pattern: "$L" }, layout: { before: { to: "capture.L", breaks: { eq: 0 } } } }),
      ast({ matches: "u" }, { utils: { u: { kind: "identifier", not: { layout: { after: { to: "token", breaks: { eq: 0 } } } } } } }),
      ast({ kind: "class_declaration" }, {}, { suppress: { eslint: { rules: ["max-classes-per-file", "rule-to-test/max-classes-per-file"] } } }),
      py({ kind: "identifier" }, {}, { suppress: { noqa: { codes: ["F401", "TC002", "TCH002"], parent: "statement" } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept nthChild as a number, An+B, or with ofRule and reverse", () => {
    for (const input of [
      ast({ kind: "identifier", nthChild: 2 }),
      ast({ kind: "identifier", nthChild: " -n + 3 " }),
      ast({ kind: "identifier", nthChild: { position: "2n+1", reverse: true } }),
      ast({ kind: "identifier", nthChild: { position: 1, ofRule: { kind: "identifier", pattern: "$X" } } }),
      ast({ kind: "call_expression", has: { kind: "arguments", has: { kind: "identifier", nthChild: 1 } } }),
      ast({ matches: "u" }, { utils: { u: { kind: "identifier", nthChild: { position: 1, ofRule: { matches: "v" } } }, v: { kind: "identifier" } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept transforms read by messages, fixes and comparisons", () => {
    for (const input of [
      ast({ pattern: "f($A)" }, { transform: { B: { replace: { source: "$A", replace: "^\\s+|\\s+$", by: "" } } } }, { message: "$B", fix: "g($B)" }),
      ast({ pattern: "f($$$A)" }, { transform: { B: { substring: { source: "$$$A", startChar: 1, endChar: -1 } }, C: { convert: { source: "$B", toCase: "snakeCase", separatedBy: ["caseChange", "dash"] } } } }, { message: "$C" }),
      ast({ pattern: "f($A, $C)", compare: { left: "B", op: "eq", right: "C", text: "unquoted" } }, { transform: { B: { convert: { source: "$A", toCase: "lowerCase" } } } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "f($A)" }, transform: { B: { convert: { source: "$A", toCase: "pascalCase" } } } } }, violation: { ...violation, message: "use $B" } }),
      ast({ any: [{ pattern: "f($A)" }, { pattern: "g()" }] }, { transform: { B: { replace: { source: "$A", default: "", replace: "^$", by: "none" } }, C: { convert: { source: "$A", default: "x", toCase: "upperCase" } }, D: { substring: { source: "$A", default: "", startChar: 1 } } } }, { message: "$B $C $D" }),
      ast({ pattern: "f($A, $C)", compare: { left: "B", op: "eq", right: "C", text: "source" } }, { transform: { B: { replace: { source: "$A", value: true, replace: "^\\s+|\\s+$", by: "" } }, D: { substring: { source: "$A", value: false } } } }, { message: "$B $D" }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "f($A)" }, transform: { B: { convert: { source: "$A", value: true, default: "", toCase: "upperCase" } } } } }, violation: { ...violation, message: "$B" } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept each over a capture the query binds", () => {
    for (const input of [
      ast({ kind: "call_expression", has: { kind: "arguments", has: { kind: "identifier", pattern: "$X" } } }, {}, { each: "X", anchor: "capture.X" }),
      ast({ kind: "regex", regex: "^/(?<EQ>=)" }, {}, { fix: { template: "[=]", range: "capture.EQ" } }),
      ast({ pattern: "f($A)" }, {}, { fix: { template: "g($A)", range: "witness.span" } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "comparison_operator", has: { kind: "true", pattern: "$OP" } } } }, violation: { ...violation, each: "OP" } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept counts along any relation", () => {
    const input = ast({
      kind: "function_declaration",
      all: [
        { count: { has: { kind: "if_statement", stopBy: { matches: "function" } }, min: 2 } },
        { count: { inside: { kind: "if_statement", stopBy: "end" }, max: 3 } },
        { count: { follows: { kind: "function_declaration", stopBy: "end" }, min: 0, max: 1 } },
      ],
    }, { utils: { function: { kind: "function_declaration" } } });
    expect(codeRuleSchema.safeParse(input).success).toBe(true);
    expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
  });

  it("both accept comparisons between captures", () => {
    const input = ast({
      pattern: "f($A, $B)",
      all: [{ compare: { left: "A", op: "gt", right: "B", collation: "fold-lower", text: "unquoted" } }, { compare: { left: "A", op: "ne", right: "B" } }],
      follows: { pattern: "f($C, $$$)", stopBy: "end", compare: { left: "C", op: "lt", right: "A" } },
    });
    expect(codeRuleSchema.safeParse(input).success).toBe(true);
    expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
  });

  it("both accept a stopBy rule that uses a utility", () => {
    const input = ast(
      { kind: "this", inside: { kind: "class_body", stopBy: { matches: "function" } } },
      { utils: { function: { any: [{ kind: "function_declaration" }, { kind: "function_expression" }] } } },
    );
    expect(codeRuleSchema.safeParse(input).success).toBe(true);
    expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
  });

  it("both accept recursive utilities and constraints that use them", () => {
    const input = ast(
      { pattern: "$F($$$)", has: { matches: "tail" } },
      {
        utils: { tail: { any: [{ kind: "number" }, { kind: "parenthesized_expression", has: { matches: "tail" } }] }, hook: { regex: "^use" } },
        constraints: { F: { matches: "hook" } },
      },
    );
    expect(codeRuleSchema.safeParse(input).success).toBe(true);
    expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
  });

  it("both accept name resolution in rules, relations, negations and constraints", () => {
    for (const input of [
      ast({ kind: "identifier", resolves: { global: ["alert", "confirm", "$", "_private"] } }),
      ast({ kind: "identifier", resolves: { global: true } }),
      ast({ kind: "identifier", resolves: { unresolved: true, directives: "eslint" } }, { languageOptions: { globals: { jQuery: "readonly" } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { unresolved: "star" } } } } }),
      document({ forbid: { ast: { language: "python", languageOptions: { pythonVersion: "3.12" }, rule: { kind: "identifier", resolves: { unresolved: true } } } } }),
      document({ forbid: { ast: { language: "python", languageOptions: { pythonVersion: "3.12", typingModules: ["compat", "foo.typical"], extendGenerics: ["django.db.models.ForeignKey"] }, rule: { kind: "identifier", resolves: { unresolved: true } } } } }),
      document({ forbid: { ast: { language: "python", rule: { any: [{ kind: "identifier" }, { kind: "attribute" }], resolves: { import: true, global: true } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { form: ["assignment", "named"], scope: "function", latest: true, outer: false, used: false, nestedUse: false, exported: false, site: { kind: "assignment" } } } } } }),
      document({ forbid: { ast: { language: "python", rule: { any: [{ kind: "dotted_name" }, { kind: "identifier" }], binds: { form: "import", scope: ["module", "function"] } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { form: "annotation", scope: "function", latest: true, used: false } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { beforeLocal: true } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "subscript", resolves: { imported: ["os"], seenBefore: true } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { binding: { form: "import", scope: "module", nestedUse: true, latest: true, outer: false, value: { kind: "call" } } } } } } }),
      ast({ kind: "identifier", resolves: { binding: { form: ["import", "definition"] } } }),
      ast({ kind: "identifier", resolves: { binding: { scope: "global", used: true, referenced: true, nestedUse: false, exported: false, directives: "eslint" } } }),
      ast({ kind: "identifier", binds: { builtin: true, redefines: { every: true, sameScope: true, form: "definition", site: { kind: "interface_declaration" } } } }),
      ast({ kind: "identifier", resolves: { write: "last" } }),
      ast({ kind: "identifier", resolves: { write: "last", binding: { used: false, occurrence: { inside: { kind: "array_pattern" } } } } }),
      ast({ kind: "identifier", binds: { builtin: true, directives: "eslint", occurrence: { kind: "identifier" } } }),
      ast({ kind: "identifier", resolves: { write: "last", binding: { used: false, exported: false, site: { kind: "variable_declarator" } } } }),
      ast({ any: [{ kind: "identifier" }, { kind: "type_identifier" }], binds: { form: "parameter", scope: ["function", "functionName"], used: false, referenced: false, exported: false } }, { languageOptions: { jsxPragma: null, jsxFragmentName: "Fragment" } }),
      ast({ any: [{ kind: "identifier" }, { kind: "type_identifier" }], binds: { builtin: true } }, { languageOptions: { sourceType: "script", lib: ["DOM", "es2022", "lib"] } }),
      ast({ kind: "private_property_identifier", binds: { form: ["field", "method", "accessor"], used: false, referenced: true, scope: "class", redefines: { every: true, form: "accessor" } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "identifier", binds: { form: "for", redefines: { form: "import", used: false, sameScope: true, sameImport: false, exported: false, site: { kind: "dotted_name" } } } } } } }),
      ast({ pattern: "$F($$$)", has: { stopBy: "end", resolves: { global: ["window"], directives: "eslint" } } }, { constraints: { F: { not: { resolves: { global: ["fetch"] } } } } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "$F($$$)" }, constraints: { F: { resolves: { global: ["print"] } } } } } }),
      document({ forbid: { ast: { language: "python", rule: { any: [{ kind: "identifier" }, { kind: "attribute" }], resolves: { global: ["open"], import: ["io.open", "os.path.join", "os"] } } } } }),
      ast({ kind: "identifier", resolves: { global: ["Reflect"] } }, { languageOptions: {} }),
      document({ forbid: { ast: { language: "python", languageOptions: {}, rule: { kind: "identifier", resolves: { global: ["print"] } } } } }),
      document({ forbid: { ast: { language: "python", languageOptions: { pythonVersion: "3.14" }, rule: { kind: "identifier", resolves: { global: ["print"], import: ["typing.Sequence"] } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "dotted_name", resolves: { imported: ["pickle", "xml.etree"], prefix: true } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "dotted_name", resolves: { imported: ["pdb.set_trace"], relative: "text" } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "dotted_name", resolves: { imported: ["pdb"], relative: "never", prefix: true } } } } }),
      document({ forbid: { ast: { language: "python", rule: { any: [{ kind: "identifier" }, { kind: "attribute" }], resolves: { import: ["subprocess"], imported: ["subprocess"], prefix: true } } } } }),
      ast({ kind: "identifier", resolves: { global: ["Reflect"], directives: "eslint" } }, { languageOptions: { ecmaVersion: 5, sourceType: "commonjs", globals: { String: "off", a: "readonly", b: "writable", c: "readable", d: "writeable", e: true, f: false, g: "true", h: "false", i: null } } }),
      ast({ kind: "identifier", resolves: { global: ["Reflect"] } }, { languageOptions: { ecmaVersion: "latest", sourceType: "module" } }),
      ast({ kind: "identifier", resolves: { global: ["Reflect"] } }, { languageOptions: { ecmaVersion: 17 } }),
      ast({ kind: "identifier", resolves: { global: ["Reflect"] } }, { languageOptions: { ecmaVersion: 3, sourceType: "script" } }),
      ast({ kind: "identifier", resolves: { global: ["Reflect"] } }, { languageOptions: { globals: Object.fromEntries(Array.from({ length: 16384 }, (_, index) => [`n${index}`, "readonly"])) } }),
      ast({ any: [{ kind: "this" }, { kind: "identifier" }], resolves: { this: "global", global: ["globalThis"] } }, { languageOptions: { sourceType: "script", ecmaVersion: 2020 } }),
      ast({ kind: "identifier", resolves: { global: ["window", "alert"], declared: false } }, { languageOptions: { ecmaVersion: 5, sourceType: "script" } }),
      ast({ kind: "this", resolves: { this: "globalScope" } }, { languageOptions: { ecmaVersion: 5, sourceType: "script" } }),
      ast({ kind: "this", resolves: { this: "global", sloppyFunctions: true } }, { languageOptions: { ecmaVersion: 5, sourceType: "script", parserOptions: { ecmaFeatures: { globalReturn: true, impliedStrict: false } } } }),
      ast({ kind: "identifier", resolves: { global: ["Reflect"] } }, { languageOptions: { parserOptions: {} } }),
      ast({ kind: "identifier", resolves: { global: ["Reflect"] } }, { languageOptions: { parserOptions: { ecmaFeatures: {} } } }),
      ast({ kind: "identifier", resolves: { global: ["window"], declared: true, directives: "eslint" } }, { languageOptions: {} }),
      ast({ kind: "identifier", resolves: { global: ["Array", "window"], directives: "eslint", scriptDeclarations: true } }, { languageOptions: { sourceType: "script" } }),
      document({ forbid: { ast: { language: "python", languageOptions: { pythonVersion: "3.9" }, rule: { kind: "identifier", resolves: { global: ["aiter"], declared: false } } } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept version gates and file conditions", () => {
    for (const input of [
      document({ forbid: { ast: { language: "python", file: { inPackage: false }, rule: { kind: "module" } } } }),
      document({ forbid: { ast: { language: "python", file: { module: { regex: "^pkg\\." }, moduleName: { regex: "^[a-z_]+$" }, inPackage: true }, rule: { kind: "module" } } } }),
      document({ forbid: { ast: { language: "python", file: { module: { notRegex: "\\.migrations\\." }, moduleName: { regex: "[A-Z]", notRegex: "^setUp$" } }, rule: { kind: "module" } } } }),
      document({ forbid: { ast: { language: "python", file: { package: { regex: "\\." }, moduleName: { regex: "^__init__$" } }, rule: { kind: "module" } } } }),
      ast({ kind: "identifier" }, { file: { declaration: true, path: { regex: "^src/", notRegex: "\\.test\\.ts$" } } }),
      document({ forbid: { ast: { language: "python", file: { path: { notRegex: "/tests/" }, inPackage: true }, rule: { kind: "module" } } } }),
      document({ forbid: { ast: { language: "python", languageOptions: { pythonVersion: "3.9" }, when: { pythonVersion: { min: "3.10" } }, rule: { kind: "call" } } } }),
      document({ forbid: { ast: { language: "python", when: { pythonVersion: { min: "3.8", max: "3.11" } }, rule: { kind: "call" } } } }),
      ast({ kind: "identifier" }, { when: { ecmaVersion: { min: 6, max: 2020 } } }),
      ast({ kind: "identifier" }, { languageOptions: { ecmaVersion: 5, sourceType: "script" }, when: { ecmaVersion: { min: "latest" } } }),
      ast({ kind: "identifier" }, { when: {} }),
      ast({ kind: "identifier" }, { languageOptions: { sourceType: "commonjs" }, when: { sourceType: ["script", "commonjs"], ecmaFeatures: { impliedStrict: false } } }),
      ast({ kind: "identifier" }, { when: { sourceType: "module", ecmaVersion: { min: 6 }, ecmaFeatures: { globalReturn: true, impliedStrict: true } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept comparisons over values, property names, tokens, structures and nodes", () => {
    for (const input of [
      ast({ pattern: "f($A, $B)", compare: { left: "A", op: "gt", right: "B", text: "value" } }),
      ast({ pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "value", collation: "fold-lower" } }),
      ast({ kind: "pair", pattern: "$P", follows: { stopBy: "end", kind: "pair", pattern: "$Q" }, compare: { left: "Q", op: "gt", right: "P", text: "key", collation: "natural" } }),
      ast({ pattern: "f($A, $B)", compare: { left: "A", op: "lt", right: "B", collation: "natural-fold-lower" } }),
      ast({ kind: "pair", pattern: "$P", follows: { stopBy: "end", kind: "pair", pattern: "$Q", compare: { left: "Q", op: "eq", right: "P", text: "key" } } }),
      ast({ kind: "method_definition", pattern: "$P", follows: { stopBy: "end", kind: "method_definition", pattern: "$Q", compare: { left: "Q", op: "eq", right: "P", text: "member" } } }),
      ast({ pattern: "export { $A as $B }", compare: { left: "A", op: "eq", right: "B", text: "name" } }),
      ast({ pattern: "$A = $B", compare: { left: "A", op: "eq", right: "B", text: "reference" } }),
      ast({ pattern: "$A = $B", not: { compare: { left: "A", op: "ne", right: "B", text: "reference-no-static-keys", collation: "ordinal" } } }),
      ast({ pattern: "f($A, $B)", not: { compare: { left: "A", op: "ne", right: "B", text: "tokens" } } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op: "le", right: "B", text: "value" } } } } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "tokens" } } } } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "structure" } } } } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", not: { compare: { left: "A", op: "ne", right: "B", text: "structure-hash" } } } } } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "structure-parts", collation: "ordinal" } } } } }),
      ast({ kind: "for_statement", pattern: "$A", follows: { stopBy: "end", kind: "for_statement", pattern: "$B", compare: { left: "A", op: "ne", right: "B", text: "node" } } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op: "eq", right: "B", text: "node" } } } } }),
      ast({ pattern: "f($A, $B)", compare: { left: "A", op: "lt", right: "B", text: "index" } }),
      ast({ pattern: "f($A, $B)", compare: { left: "A", op: "ne", right: "B", text: "tokens", commute: ["||", "&&"] } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "f($A, $B)", compare: { left: "A", op: "ge", right: "B", text: "index", collation: "ordinal" } } } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept value predicates in rules, relations, negations and constraints", () => {
    for (const input of [
      ast({ kind: "string", value: { eq: "0.0.0.0" } }),
      ast({ kind: "expression_statement", not: { has: { value: { type: ["number", "bigint"], constant: true } } } }),
      ast({ pattern: "f($A)" }, { constraints: { A: { value: { startsWith: ["/tmp", "/var/tmp"], regex: "^/" } } } }),
      ast({ kind: "binary_expression", has: { stopBy: "end", value: { truthy: false } } }),
      ast({ kind: "number", value: { bitsAny: 18, gt: 1, ge: 1, lt: 1e6, le: 1e6 } }),
      ast({ kind: "number", value: { eq: null } }, { languageOptions: { globals: { NaN: "off" } } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "os.chmod($P, $M)" }, constraints: { M: { value: { bitsAny: 18 } } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "integer", value: { type: ["int", "NoneType", "ellipsis"], eq: 1.5, constant: false } } } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept binding sub-rules and followed values", () => {
    for (const input of [
      ast({ kind: "identifier", resolves: { binding: {} } }),
      ast({ pattern: "$F()" }, { constraints: { F: { matches: "alias" } }, utils: { alias: { any: [{ kind: "identifier", resolves: { global: ["JSON"] } }, { kind: "identifier", resolves: { binding: { value: { matches: "alias" } } } }] } } }),
      ast({ kind: "identifier", not: { resolves: { binding: { site: { kind: "variable_declarator" }, annotation: { kind: "predefined_type" } } } } }),
      ast({ pattern: "setTimeout($S)" }, { constraints: { S: { value: { type: "string", follow: true } } } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "$D.get($K, None)" }, constraints: { D: { resolves: { binding: { value: { kind: "dictionary" }, annotation: { regex: "^dict" } } } } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { binding: { value: { pattern: "$F($$$)" } } } }, constraints: { F: { kind: "identifier" } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "integer", value: { eq: 1, follow: false } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { binding: { stopAtGlobal: true, only: true, value: { kind: "attribute" } } } } } } }),
      ast({ kind: "identifier", resolves: { binding: { only: true } } }),
      ast({ kind: "identifier", resolves: { binding: { destructuredValue: false, value: { kind: "identifier" } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { binding: { stopAtGlobal: true, only: true, form: "global", outer: true, latest: false, scope: "function" } } } } } }),
      document({ forbid: { ast: { language: "python", rule: { kind: "identifier", inside: { kind: "for_statement", pattern: "$LOOP", stopBy: "end" }, binds: { form: "augmented", bindings: { within: "LOOP", min: 1, max: 1 } } } } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept arguments bound to signature slots", () => {
    for (const input of [
      ast({ pattern: "setTimeout($$$)", arguments: { signature: ["handler", "timeout"], bind: { H: "handler" } } }),
      ast({ kind: "new_expression", not: { arguments: { signature: ["a"], absent: ["a"] } } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "requests.get($$$)", arguments: { signature: ["url", "params", "*", "timeout"], absent: ["timeout"] } } } } }),
      document({ forbid: { ast: { language: "python", rule: { pattern: "zip($$$)", arguments: { signature: ["*", "strict"], absent: ["strict"], mode: "syntactic" } } } } }),
      ast({ pattern: "setTimeout($$$)", arguments: { signature: ["code", "delay"], bind: { S: "code" }, mode: "estree" } }),
      ast({ pattern: "setTimeout($$$)", arguments: { signature: ["handler"], bind: { H: "handler" }, mode: "language" } }),
      document({
        forbid: {
          ast: {
            language: "python",
            rule: { pattern: "$Q.extra($$$)", arguments: { signature: ["select", "where", "/", "params"], bind: { W: "where", S: "select" } } },
            constraints: { W: { not: { kind: "list" } } },
          },
        },
      }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept rules over parsed literals, with captures in messages, anchors and fixes", () => {
    for (const input of [
      py({ kind: "string", parse: { grammar: "percentFormat", has: { kind: "placeholder", stopBy: "end", has: { kind: "mapping_key" } } } }),
      py({ kind: "string", parse: { grammar: "percentFormat", has: { kind: "error", value: { regex: "^unsupported" }, capture: "C" } } }, {}, { anchor: "capture.C", message: "bad $C" }),
      py({ pattern: "$S.format($$$)", has: { kind: "string", parse: { grammar: "braceFormat", has: { kind: "field", stopBy: "end", capture: "F", not: { has: { kind: "conversion" } } } } } }, {}, { message: "field $F", fix: "$S" }),
      py({ kind: "string", parse: { grammar: "braceFormat", all: [{ has: { kind: "field", has: { kind: "field_name", has: { kind: "auto" } } } }, { has: { kind: "field", has: { kind: "field_name", has: { kind: "index" } } } }] } }),
      py({ kind: "string", parse: { grammar: "braceFormat", count: { has: { kind: "field" }, min: 2 }, has: { kind: "field", capture: "A", precedes: { kind: "field", capture: "B" } }, compare: { left: "A", op: "eq", right: "B" } } }),
      py({ pattern: "$S.format($$$A)", formatArgs: { style: "brace", string: "S", args: "A", unused: { kind: "keyword_argument", pattern: "$K=$V" } } }, {}, { message: "unused $K", fix: "$S" }),
      py({ pattern: "$S % $R", formatArgs: { style: "percent", string: "S", args: "R", missing: { kind: "placeholder", has: { kind: "mapping_key", capture: "M" } } } }, {}, { anchor: "capture.M", message: "no $M" }),
      py({ any: [{ kind: "string" }, { kind: "concatenated_string" }], parse: { grammar: "annotation", has: { kind: "identifier", resolves: { unresolved: true }, stopBy: "end", capture: "N" } } }, {}, { anchor: "capture.N", message: "undefined $N" }),
      py({ kind: "string", parse: { grammar: "annotation", kind: "ERROR" } }),
      py({ kind: "string", parse: { grammar: "annotation", has: { pattern: "Optional[$T]", stopBy: "end", has: { kind: "string", parse: { grammar: "annotation", has: { kind: "identifier", capture: "N" } } } } } }),
      py({ kind: "identifier", binds: { form: "annotation", scope: "function", used: false } }),
      py({ pattern: "$O.strip($A)" }, { constraints: { A: { kind: "string", value: { regex: "(?s)(.).*\\1" } } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept calls terms, with a chain in the message and in utilities", () => {
    const blocking = { stopBy: { kind: "function_definition" }, kind: "call", has: { field: "function", resolves: { import: ["time.sleep"] } } };
    for (const input of [
      py({ kind: "call", calls: { through: "sync", has: blocking } }),
      py({ kind: "function_definition", not: { has: { stopBy: { kind: "function_definition" }, kind: "call", calls: { has: blocking } } } }),
      py({ kind: "call", calls: { through: "sync", chain: "CHAIN", has: blocking } }, {}, { message: "blocks via $CHAIN" }),
      py({ kind: "call", matches: "blocks" }, { utils: { blocks: { calls: { through: "any", has: blocking } } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("both accept M12b's keys over parsed literals: each, fix ranges and transforms of exported captures", () => {
    const keys = { kind: "string", parse: { grammar: "percentFormat", has: { kind: "mapping_key", stopBy: "end", capture: "K" } } };
    for (const input of [
      py(keys, {}, { anchor: "capture.K", each: "K", message: "key $K" }),
      py(keys, {}, { fix: { template: "b", range: "capture.K" } }),
      py(keys, { transform: { U: { convert: { source: "$K", toCase: "upperCase" } } } }, { message: "key $U" }),
      py({ pattern: "$S.format($$$A)", formatArgs: { style: "brace", string: "S", args: "A", missing: { kind: "field", capture: "F" } } }, { transform: { N: { replace: { source: "$F", replace: "^\\{|\\}$", by: "" } } } }, { anchor: "capture.F", each: "F", message: "missing $N" }),
      py({ kind: "string", parse: { grammar: "percentFormat", all: [{ has: { kind: "mapping_key", stopBy: "end", capture: "K" } }, { has: { kind: "mapping_key", stopBy: "end", capture: "L", compare: { left: "L", op: "eq", right: "U", text: "source" } } }] } }, { transform: { U: { convert: { source: "$K", toCase: "upperCase" } } } }),
    ]) {
      expect(codeRuleSchema.safeParse(input).success).toBe(true);
      expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
    }
  });

  it("pins parse's grammars and kinds to the engine's", () => {
    const literal = (file: string) => readFileSync(new URL(`../../../native/src/code_rule_dsl/literal/${file}`, import.meta.url), "utf8");
    const grammars = [...literal("mod.rs").matchAll(/^\s+"(\w+)" => Some\(Self::\w+\),$/gm)].map(match => match[1]).filter(name => name !== "testChars");
    expect(grammars).toEqual([...CODE_RULE_PARSE_GRAMMARS]);
    const kinds = (file: string) => [...(literal(file).split("KINDS: &[&str] = &[")[1]!.split("];")[0]!.matchAll(/"(\w+)"/g))].map(match => match[1]);
    expect(kinds("percent.rs")).toEqual([...CODE_RULE_PARSE_KINDS.percentFormat]);
    expect(kinds("brace.rs")).toEqual([...CODE_RULE_PARSE_KINDS.braceFormat]);
    expect(kinds("eslint_directive.rs")).toEqual([...CODE_RULE_PARSE_KINDS.eslintDirective]);
    expect(kinds("phpdoc.rs")).toEqual([...CODE_RULE_PARSE_KINDS.phpDoc]);
    expect(kinds("rust_format.rs")).toEqual([...CODE_RULE_PARSE_KINDS.rustFormat]);
  });


  it("pins the latest ecmaVersion and the Python versions of the engine's builtins tables", () => {
    const table = JSON.parse(readFileSync(new URL("../../../native/src/code_rule_dsl/scope/data/es_globals.generated.json", import.meta.url), "utf8")) as { latest: number; versions: Record<string, unknown> };
    expect(table.latest).toBe(CODE_RULE_LATEST_ECMA_VERSION);
    expect(Object.keys(table.versions)).toContain(String(CODE_RULE_LATEST_ECMA_VERSION));
    const python = JSON.parse(readFileSync(new URL("../../../native/src/code_rule_dsl/scope/data/python_builtins.generated.json", import.meta.url), "utf8")) as { versions: Record<string, unknown> };
    expect(Object.keys(python.versions)).toEqual([...CODE_RULE_PYTHON_VERSIONS]);
  });

  it("both accept empty violation variables and the witness anchor", () => {
    const input = document({ violation: { ...violation, variables: {} } });
    expect(codeRuleSchema.safeParse(input).success).toBe(true);
    expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
  });

  it.each([
    ["a start anchor", document({ violation: { ...violation, anchor: "witness.start" } })],
    ["a file start anchor", document({ violation: { ...violation, anchor: "file.start" } })],
    ["a file start anchor on an ast query", ast({ kind: "program" }, {}, { anchor: "file.start" })],
    ["an end anchor with a regex fix", document({ violation: { ...violation, anchor: "witness.end", fix: "y" } })],
    ["a capture anchor, interpolation, and an ast fix", ast({ pattern: "f($A)" }, {}, { anchor: "capture.A", message: "f of $A", fix: "g($A)" })],
    ["a named regex group", document({ forbid: { regex: { pattern: "(?<X>x)", source: "file.content" } }, violation: { ...violation, anchor: "capture.X", message: "$X", fix: "[$X]" } })],
    ["escaped and digit dollars in a message", document({ violation: { ...violation, message: "set \\$HOME, pay $100" } })],
    ["a comparison of names its own negation binds", ast({
      kind: "pair",
      all: [
        { has: { kind: "property_identifier", pattern: "$K" } },
        { not: { precedes: { stopBy: "end", kind: "pair", has: { kind: "property_identifier", pattern: "$J" }, compare: { left: "J", op: "eq", right: "K" } } } },
      ],
    })],
  ])("both accept %s", (_name, input) => {
    expect(codeRuleSchema.safeParse(input).success).toBe(true);
    expect(() => loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(input), "json", bundledCapabilities)).not.toThrow();
  });
});

describe("compiled rule cache", () => {
  function cachedRule(id: string, revision: number): { text: string; digest: string } {
    const text = JSON.stringify({
      format: "rerail.policy/v1", id, revision, mode: "steer",
      scope: { include: ["**/*.py"], exclude: [] },
      clauses: [{ id: "c", violation: { code: "X", anchor: "witness.span", message: "x" }, forbid: { regex: { pattern: `N${revision}`, source: "file.content" } } }],
    });
    return { text, digest: `sha256:${id}:${revision}` };
  }

  it("keeps one compiled version per rule, so editing a rule does not keep its earlier versions in memory", () => {
    const cache = new CompiledRuleCache();
    const capabilities = loadCodeRuleCapabilities();
    const first = cachedRule("cache:a", 1);
    const outcome = cache.compile(capabilities, "cache:a", first.text, first.digest, "null");
    expect(outcome.ok).toBe(true);
    expect(cache.compile(capabilities, "cache:a", first.text, first.digest, "null")).toBe(outcome);
    for (let revision = 2; revision <= 5; revision += 1) {
      const edited = cachedRule("cache:a", revision);
      expect(cache.compile(capabilities, "cache:a", edited.text, edited.digest, "null").ok).toBe(true);
    }
    const other = cachedRule("cache:b", 1);
    cache.compile(capabilities, "cache:b", other.text, other.digest, "null");
    expect(cache.size).toBe(2);
  });

  it("keeps one compiled version per config of a rule, so two repositories configuring it differently do not recompile in turn", () => {
    const cache = new CompiledRuleCache();
    const capabilities = loadCodeRuleCapabilities();
    const plain = cachedRule("cache:c", 1);
    const configured = cachedRule("cache:c", 2);
    const plainOutcome = cache.compile(capabilities, "cache:c", plain.text, plain.digest, "null");
    const configuredOutcome = cache.compile(capabilities, "cache:c", configured.text, configured.digest, "{\"option\":\"type\"}");
    expect(cache.compile(capabilities, "cache:c", plain.text, plain.digest, "null")).toBe(plainOutcome);
    expect(cache.compile(capabilities, "cache:c", configured.text, configured.digest, "{\"option\":\"type\"}")).toBe(configuredOutcome);
    expect(cache.size).toBe(2);
  });
});
