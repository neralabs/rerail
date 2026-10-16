import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  loadNativeBinding,
  type NativeCompiledCodeRuleDsl,
  type NativeCodeRuleDslWorkspace,
} from "../../../src/engine/native/native.js";
import { applyCodeRuleConfig, resolveCodeRuleOptions } from "../../../src/engine/code-rule/code-rule-options.js";
import { contentStorePath } from "../../../src/engine/native/rerail-build.js";

/** The reference binary intentionally retains these reviewed defects. */
const REFERENCE_BUGS: Record<string, true> = { B1: true, B2: true, B3: true, B5: true, B6: true, B7: true, B11: true, B12: true, B13: true, B14: true, B15: true, B19: true };
const USING_REFERENCE_ADDON = process.env.RERAIL_ENGINE_PATH?.includes("jeff-baseline") === true;

const capabilities = JSON.stringify({
  format: "rerail.policy-capabilities/v1",
  ast: { engine: "ast-grep-core", engineVersion: "0.45", languages: { python: "tree-sitter-python@0.23", typescript: "tree-sitter-typescript@0.23" } },
  factRelations: {},
  limits: { maxAstPatternBytes: 16384, maxFiles: 100, maxGraphDepth: 16, maxRegexPatternBytes: 16384, maxSourceBytes: 1_000_000, maxVisitedNodes: 100_000, maxWitnesses: 10_000 },
});

type File = { path: string; language: "python" | "typescript"; source: string; module?: string | string[] };
type Violation = { id: string; clauseId: string; message: string; path?: string; span?: { startByte: number; endByte: number; startLine: number; startColumn: number; endLine: number; endColumn: number }; captures: Record<string, Array<{ startByte: number; endByte: number }> > };
type Result = { status: "completed" | "failed"; violations: Violation[]; failures: Array<{ code: string; clauseId: string; path?: string }> };

function document(clauses: unknown[], extra: Record<string, unknown> = {}) {
  return JSON.stringify({ format: "rerail.policy/v1", id: "semantic-acceptance", revision: 1, mode: "steer", scope: { include: ["**/*"], exclude: [] }, clauses: clauses, ...extra });
}
function clause(id: string, rule: Record<string, unknown>, language: "python" | "typescript" = "python", extra: Record<string, unknown> = {}, query: Record<string, unknown> = {}) {
  const { capture: _capture, ...hostRule } = rule;
  return { id, forbid: { ast: { language, rule: hostRule, ...query } }, violation: { anchor: "witness.span", code: id, message: id }, ...extra };
}
function compile(clauses: unknown[], extra: Record<string, unknown> = {}) {
  return loadNativeBinding().CodeRuleDslRuntime!.compile(document(clauses, extra), "json", capabilities);
}
function evaluate(compiled: NativeCompiledCodeRuleDsl, files: File[]): Result {
  return JSON.parse(compiled.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 1, facts: [], files }), "report")) as Result;
}
async function workspace(compiled: NativeCompiledCodeRuleDsl, revision: number, files: File[], removed: string[] = []): Promise<Result> {
  const native = new (loadNativeBinding().CodeRuleDslWorkspace!)();
  try {
    native.addCodeRule(compiled);
    return JSON.parse(await native.applyAsync(JSON.stringify({ revision, files, removed }))).evaluation as Result;
  } finally { native.close(); }
}
/** Index the attached repository chunk by chunk, as the enforcer does; the workspace revision after it. */
async function indexRepository(native: NativeCodeRuleDslWorkspace): Promise<number> {
  const options = { maxFiles: 100_000, maxSourceBytes: 512 * 1024 * 1024, maxFileBytes: 1_000_000, ignoredDirectories: [], sourceLanguages: { ".py": "python" }, nestedCheckouts: [] };
  for (;;) {
    const chunk = JSON.parse(await native.indexRepositoryAsync(options)) as { done: boolean; revision: number };
    if (chunk.done) return chunk.revision;
  }
}
function texts(result: Result, source: string) { return result.violations.map(item => source.slice(item.span!.startByte, item.span!.endByte)); }
/** Each violation as `clauseId text`, sorted: what each clause reported, whatever the output order. */
function reported(result: Result, files: readonly File[]) {
  return result.violations.map(item => `${item.clauseId} ${files.find(file => file.path === item.path)!.source.slice(item.span!.startByte, item.span!.endByte)}`).sort();
}
function specOrReference<T>(bug: string, spec: T, legacy: T): T { return USING_REFERENCE_ADDON && REFERENCE_BUGS[bug] === true ? legacy : spec; }

// These tests deliberately use only the N-API code-rule boundary.  Thus the same file is
// executable against the pinned reference addon and every candidate native build.
// A06 (first-binding unknown alternatives) and A11 (lone-surrogate value comparisons) have no
// scenario here.
describe("semantic acceptance", () => {
  describe("A01 builtin shadowing and import aliases", () => {
    it("matches only lexically resolved builtin references and qualified aliases", () => {
      const source = "import os as operating_system\nopen('a')\ndef f(open):\n    open('b')\noperating_system.path.join('a', 'b')\n";
      const compiled = compile([
        clause("builtin", { kind: "identifier", resolves: { global: ["open"] }, capture: "X" }),
        clause("alias", { kind: "attribute", resolves: { import: ["os.path.join"] }, capture: "X" }),
      ]);
      expect(texts(evaluate(compiled, [{ path: "a.py", language: "python", source }]), source)).toEqual(["open", "operating_system.path.join"]);
    });
  });

  describe("A02 configuration-aware facts", () => {
    it("does not share configured globals between otherwise identical inputs", () => {
      const source = "alert(value);\n";
      const configured = clause("configured", { kind: "identifier", resolves: { global: ["alert"] }, capture: "X" }, "typescript");
      const absent = clause("absent", { kind: "identifier", resolves: { global: ["alert"] }, capture: "X" }, "typescript");
      expect(evaluate(compile([configured]), [{ path: "a.ts", language: "typescript", source }]).violations).toHaveLength(1);
      expect(evaluate(compile([absent]), [{ path: "a.ts", language: "typescript", source }]).violations).toHaveLength(1);
    });
  });

  describe("A03 bounded value following", () => {
    it("does not claim branch-mutated values are an all-path proof", () => {
      const source = "let value = 1; if (flag) value = 2; consume(value);\n";
      const result = evaluate(compile([clause("value", { pattern: "consume($X)" }, "typescript")]), [{ path: "a.ts", language: "typescript", source }]);
      expect(result.status).toBe("completed");
      expect(result.violations).toHaveLength(1);
    });
  });

  describe("A04 negated unknown", () => {
    it("does not manufacture a match from an unresolved value", () => {
      const source = "unknown_value;\n";
      const result = evaluate(compile([clause("unknown", { kind: "expression_statement", has: { kind: "identifier", regex: "^unknown_value$", not: { value: { eq: 1 } } } }, "typescript")]), [{ path: "a.ts", language: "typescript", source }]);
      expect(result.status).toBe("completed");
      expect(result.violations).toHaveLength(0);
    });
  });

  describe("A05 transactional unknown captures", () => {
    it("keeps an independent false conjunction false without leaking captures", () => {
      const source = "consume(value);\n";
      const result = evaluate(compile([clause("transaction", { all: [{ pattern: "consume($X)" }, { kind: "class" }], capture: "X" }, "typescript")]), [{ path: "a.ts", language: "typescript", source }]);
      expect(result.violations).toEqual([]);
    });
  });

  describe("A07 aliased utility outputs", () => {
    it("unifies parameter outputs mapped to one capture, whatever the declaration or mapping order", () => {
      const files: File[] = [{ path: "a.ts", language: "typescript", source: "f(g);\nh(h);\n" }];
      const orders = [
        [["P", "Q"], { P: "$X", Q: "$X" }],
        [["Q", "P"], { P: "$X", Q: "$X" }],
        [["P", "Q"], { Q: "$X", P: "$X" }],
        [["Q", "P"], { Q: "$X", P: "$X" }],
      ] as const;
      for (const [params, mapping] of orders) {
        const compiled = compile([{
          id: "pair",
          forbid: { ast: { language: "typescript", rule: { kind: "call_expression", matches: { pair: mapping } }, utils: { pair: { params, rule: { pattern: "$P($Q)" } } } } },
          violation: { anchor: "capture.X", code: "pair", message: "$X" },
        }]);
        const result = evaluate(compiled, files);
        // `f(g)` returns conflicting outputs, so the call fails without binding `$X`; `h(h)`'s agree.
        expect(result.violations.map(item => [item.span!.startByte, item.message]), params.join()).toEqual([[6, "h"]]);
        expect(result.failures).toEqual([]);
      }
    });
  });

  describe("A08 ordering, cycles, and gated clauses", () => {
    it("rejects a capture read before its explicit binding with a source-path error", () => {
      const invalid = () => compile([clause("ordered", { all: [{ compare: { left: "X", op: "eq", right: "X" } }, { pattern: "f($X)" }] }, "typescript")]);
      expect(invalid).toThrow(/reads `\$X`, which only .* binds, after it/);
    });
  });

  describe("A09 numeric option canonicalization", () => {
    const optionDocument = (config: Record<string, unknown>) => ({
      format: "rerail.policy/v1", id: "numeric-option", revision: 1, mode: "steer", scope: { include: ["**/*"], exclude: [] },
      clauses: [
        { id: "options", define: { options: { level: { type: "number", default: 0 } }, config } },
        { id: "selected", forbid: { ast: { language: "typescript", rule: { $option: "level", cases: { "1": { kind: "number" } }, default: { kind: "string" } } } }, violation: { anchor: "witness.span", code: "selected", message: "selected" } },
      ],
    });
    /** The document as raw JSON text with the configured `level` spelled exactly as given. */
    const raw = (level: string) => JSON.stringify(optionDocument({ level: "LEVEL" })).replace('"LEVEL"', level);
    const files: File[] = [{ path: "a.ts", language: "typescript", source: "f(1, 'a');\n" }];
    const selected = (codeRule: string) => reported(evaluate(loadNativeBinding().CodeRuleDslRuntime!.compile(codeRule, "json", capabilities), files), files);

    it("selects one case for raw JSON 1.0, raw JSON 1 and the TypeScript number 1", () => {
      const configured = applyCodeRuleConfig(optionDocument({}), { level: 1 });
      for (const codeRule of [raw("1.0"), raw("1"), JSON.stringify(configured), JSON.stringify(resolveCodeRuleOptions(configured))]) {
        expect(selected(codeRule)).toEqual(["selected 1"]);
      }
      // A different number selects the default in every spelling.
      expect(selected(raw("2.0"))).toEqual(["selected 'a'"]);
      expect(selected(JSON.stringify(resolveCodeRuleOptions(applyCodeRuleConfig(optionDocument({}), { level: 2 }))))).toEqual(["selected 'a'"]);
    });

    it("rejects a number JavaScript cannot represent instead of selecting a different case", () => {
      // 2^53 + 1: native JSON could read it exactly, while JavaScript rounds it to 2^53.
      expect(() => loadNativeBinding().CodeRuleDslRuntime!.compile(raw("9007199254740993"), "json", capabilities)).toThrow(/define\/config\/level: \d+ is not exactly representable as a JavaScript number/);
    });
  });

  describe("A10 parsed literal host ranges", () => {
    it("returns host byte and line/column spans for multibyte CRLF source", () => {
      const source = "value = 'é\\n'\r\n";
      const result = evaluate(compile([clause("literal", { kind: "string", capture: "X" })]), [{ path: "a.py", language: "python", source }]);
      const span = result.violations[0]!.span!;
      expect(Buffer.from(source).subarray(span.startByte, span.endByte).toString("utf8")).toBe("'é\\n'");
      expect(span.startLine).toBe(0);
    });
  });

  describe("A12 each binding enumeration", () => {
    it("reports each reachable selected capture once", () => {
      const source = "f(a); f(b);\n";
      const result = evaluate(compile([clause("each", { pattern: "f($X)", capture: "X" }, "typescript", { violation: { anchor: "capture.X", code: "each", message: "each $X", each: "X" } })]), [{ path: "a.ts", language: "typescript", source }]);
      expect(texts(result, source)).toEqual(specOrReference("B2", ["a", "b"], ["a", "b"]));
      expect(new Set(result.violations.map(item => item.id)).size).toBe(2);
    });
  });

  describe("A13 suppression and enumeration", () => {
    it("does not treat a suppression comment as a code rule result unless suppression is opted in", () => {
      const source = "// eslint-disable-next-line\nf(a);\nf(b);\n";
      const result = evaluate(compile([clause("suppression", { pattern: "f($X)", capture: "X" }, "typescript", { violation: { anchor: "capture.X", code: "suppression", message: "s $X", each: "X" } })]), [{ path: "a.ts", language: "typescript", source }]);
      expect(texts(result, source)).toEqual(["a", "b"]);
    });
  });

  describe("A14 witness allowance", () => {
    it("returns an explicit bounded outcome rather than silently truncating", () => {
      const limited = JSON.stringify({ ...JSON.parse(capabilities), limits: { ...JSON.parse(capabilities).limits, maxWitnesses: 1 } });
      const compiled = loadNativeBinding().CodeRuleDslRuntime!.compile(document([clause("limit", { pattern: "f($X)", capture: "X" }, "typescript", { violation: { anchor: "capture.X", code: "limit", message: "$X", each: "X" } })]), "json", limited);
      const result = evaluate(compiled, [{ path: "a.ts", language: "typescript", source: "f(a); f(b);" }]);
      expect(result.failures.map(item => item.code)).toContain("witness-limit");
    });
  });

  describe("A15 resource bounds", () => {
    it("fails a too-small deterministic visit budget instead of converting it through negation", () => {
      const limited = JSON.stringify({ ...JSON.parse(capabilities), limits: { ...JSON.parse(capabilities).limits, maxVisitedNodes: 1 } });
      const compiled = loadNativeBinding().CodeRuleDslRuntime!.compile(document([{ ...clause("budget", { not: { kind: "identifier" } }), violation: { anchor: "witness.span", code: "budget", message: "budget" } }]), "json", limited);
      expect(evaluate(compiled, [{ path: "a.py", language: "python", source: "a = b + c\n" }]).failures.map(item => item.code)).toContain(specOrReference("B6", "budget-exceeded", "source-incomplete"));
    });
  });

  describe("A16 package-marker invalidation", () => {
    it("attaches storage asynchronously without blocking the JavaScript continuation", async () => {
      const root = mkdtempSync(join(tmpdir(), "rerail-semantic-async-storage-"));
      const native = new (loadNativeBinding().CodeRuleDslWorkspace!)();
      try {
        const attachment = native.enableStorageAsync(root, contentStorePath());
        expect(attachment).toBeInstanceOf(Promise);
        let continued = false;
        void Promise.resolve().then(() => { continued = true; });
        await Promise.resolve();
        expect(continued).toBe(true);
        await attachment;
      } finally {
        native.close();
        rmSync(root, { force: true, recursive: true });
      }
    });

    const files: File[] = [
      { path: "pkg/helper.py", language: "python", source: "def helper():\n    pass\n" },
      { path: "pkg/mod.py", language: "python", source: "from . import helper\nhelper()\n" },
      { path: "loose/x.py", language: "python", source: "x = 1\n" },
    ];
    const marker: File = { path: "pkg/__init__.py", language: "python", source: "" };
    /** One module check per side of the package boundary, so a stale resident reports the wrong side. */
    const packageCodeRule = () => compile([
      clause("packaged", { kind: "module" }, "python", {}, { file: { inPackage: true } }),
      clause("loose", { kind: "module" }, "python", {}, { file: { inPackage: false } }),
    ]);
    const unpackaged = ["loose loose/x.py", "loose pkg/helper.py", "loose pkg/mod.py"];
    const packaged = ["loose loose/x.py", "packaged pkg/__init__.py", "packaged pkg/helper.py", "packaged pkg/mod.py"];
    const located = (violations: readonly Violation[]) => violations.map(item => `${item.clauseId} ${item.path}`).sort();
    /** The unchanged loose sibling's violation identity, which no marker change may disturb. */
    const siblingId = (violations: readonly Violation[]) => violations.find(item => item.path === "loose/x.py")?.id;
    async function apply(native: NativeCodeRuleDslWorkspace, revision: number, changed: File[], removed: string[]) {
      return (JSON.parse(await native.applyAsync(JSON.stringify({ revision, files: changed, removed }))) as { evaluation: Result }).evaluation.violations;
    }
    async function allPages(native: NativeCodeRuleDslWorkspace) {
      const violations: Violation[] = [];
      for (let cursor = 0, done = false; !done;) {
        const page = JSON.parse(await native.resultPageAsync(cursor, 1 << 20)) as { cursor: number; done: boolean; violations: Violation[] };
        violations.push(...page.violations);
        ({ cursor, done } = page);
      }
      return violations;
    }

    it("refreshes unchanged residents on unsaved marker create and delete overlays", async () => {
      const native = new (loadNativeBinding().CodeRuleDslWorkspace!)();
      try {
        native.addCodeRule(packageCodeRule());
        const results = [await apply(native, 1, files, []), await apply(native, 2, [marker], []), await apply(native, 3, [], [marker.path])];
        expect(results.map(located)).toEqual([unpackaged, packaged, unpackaged]);
        expect(new Set(results.map(siblingId)).size).toBe(1);
      } finally { native.close(); }
    });

    it("refreshes indexed residents when a marker is created and deleted over an attached store", async () => {
      const root = mkdtempSync(join(tmpdir(), "rerail-semantic-a16-"));
      const native = new (loadNativeBinding().CodeRuleDslWorkspace!)();
      try {
        for (const file of files) {
          mkdirSync(join(root, file.path, ".."), { recursive: true });
          writeFileSync(join(root, file.path), file.source);
        }
        execFileSync("git", ["init", "--quiet"], { cwd: root });
        execFileSync("git", ["add", "."], { cwd: root });
        execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
        await native.enableStorageAsync(root, contentStorePath());
        native.addCodeRule(packageCodeRule());
        const revision = await indexRepository(native);
        const results = [await allPages(native)];
        writeFileSync(join(root, marker.path), marker.source);
        // The touched-files result of each change holds the marker and the residents it moved.
        expect(located(await apply(native, revision + 1, [marker], []))).toEqual(packaged.filter(item => item.startsWith("packaged")));
        results.push(await allPages(native));
        rmSync(join(root, marker.path));
        expect(located(await apply(native, revision + 2, [], [marker.path]))).toEqual(unpackaged.filter(item => item.startsWith("loose pkg/")));
        results.push(await allPages(native));
        expect(results.map(located)).toEqual([unpackaged, packaged, unpackaged]);
        expect(new Set(results.map(siblingId)).size).toBe(1);
      } finally {
        native.close();
        rmSync(root, { force: true, recursive: true });
      }
    });

    it("rediscovers a committed clean file for a new storage attachment", async () => {
      const root = mkdtempSync(join(tmpdir(), "rerail-semantic-fresh-storage-"));
      const first = new (loadNativeBinding().CodeRuleDslWorkspace!)();
      const second = new (loadNativeBinding().CodeRuleDslWorkspace!)();
      if (typeof first.enableStorageAsync !== "function") {
        expect(USING_REFERENCE_ADDON).toBe(true);
        first.close();
        second.close();
        rmSync(root, { force: true, recursive: true });
        return;
      }
      try {
        mkdirSync(join(root, "pkg"));
        writeFileSync(join(root, "pkg", "a.py"), "value = 1\n");
        execFileSync("git", ["init", "--quiet"], { cwd: root });
        execFileSync("git", ["add", "."], { cwd: root });
        execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "first"], { cwd: root });
        await first.enableStorageAsync(root, contentStorePath());
        writeFileSync(join(root, "pkg", "a.py"), "value = 2\n");
        execFileSync("git", ["add", "."], { cwd: root });
        execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "second"], { cwd: root });
        await second.enableStorageAsync(root, contentStorePath());
        second.addCodeRule(compile([clause("fresh", { regex: "2" })]));
        const revision = await indexRepository(second);
        const result = JSON.parse(await second.applyAsync(JSON.stringify({ revision: revision + 1, files: [], removed: [] }))).evaluation as Result;
        expect(result.violations.some((violation) => violation.path === "pkg/a.py")).toBe(true);
      } finally {
        first.close();
        second.close();
        rmSync(root, { force: true, recursive: true });
      }
    });
  });

  describe("A17 context changes and moves", () => {
    it("publishes only the current revision after a move", async () => {
      const compiled = compile([clause("move", { kind: "identifier", capture: "X" })]);
      const native = new (loadNativeBinding().CodeRuleDslWorkspace!)();
      try {
        native.addCodeRule(compiled);
        await native.applyAsync(JSON.stringify({ revision: 1, files: [{ path: "a.py", language: "python", source: "x = 1\n" }], removed: [] }));
        const moved = JSON.parse(await native.applyAsync(JSON.stringify({ revision: 2, files: [{ path: "pkg/a.py", language: "python", source: "y = 1\n" }], removed: ["a.py"] }))).evaluation as Result;
        expect(moved.violations.every(item => item.path === "pkg/a.py")).toBe(true);
      } finally { native.close(); }
    });
  });

  describe("A18 stateless and workspace agreement", () => {
    it("returns identical ordered violations and failures for the same snapshot", async () => {
      const source = "f(a);\n"; const compiled = compile([clause("agreement", { pattern: "f($X)", capture: "X" }, "typescript")]);
      const full = evaluate(compiled, [{ path: "a.ts", language: "typescript", source }]);
      const indexed = await workspace(compiled, 1, [{ path: "a.ts", language: "typescript", source }]);
      expect({ violations: indexed.violations, failures: indexed.failures }).toEqual({ violations: full.violations, failures: full.failures });
    });
  });

  describe("A19 capability isolation", () => {
    it("rejects unsupported capabilities before an invalid code rule can run", () => {
      expect(() => loadNativeBinding().CodeRuleDslRuntime!.compile(document([clause("valid", { kind: "identifier" })], { requires: ["not-supported"] }), "json", capabilities)).toThrow();
    });
  });

  describe("A20 select_when evidence", () => {
    it("opens only on a definite existence witness", () => {
      const source = "f(a); g(b);\n";
      const compiled = compile([{
        id: "gate",
        forbid: {
          select_when: {
            exists: { ast: { language: "typescript", rule: { pattern: "f($X)" } } },
            select: { ast: { language: "typescript", rule: { pattern: "g($Y)" } } },
          },
        },
        violation: { anchor: "witness.span", code: "gate", message: "gate" },
      }]);
      expect(texts(evaluate(compiled, [{ path: "a.ts", language: "typescript", source }]), source)).toEqual(["g(b)"]);
    });
  });

  describe("B11 select_when existence ignores reporting enumeration", () => {
    it("opens a workspace gate even when only the select side has a regex-group each", async () => {
      const source = "enable(); report(alpha);\n";
      const compiled = compile([{
        id: "gate",
        forbid: { select_when: {
          exists: { ast: { language: "typescript", rule: { pattern: "enable()" } } },
          select: { ast: { language: "typescript", rule: { kind: "call_expression", regex: "^report\\((?<ITEM>\\w+)\\)$" } } },
        } },
        violation: { anchor: "witness.span", code: "gate", message: "$ITEM", each: "ITEM" },
      }]);
      const files = [{ path: "a.ts", language: "typescript" as const, source }];
      expect(texts(evaluate(compiled, files), source)).toEqual(["report(alpha)"]);
      expect(texts(await workspace(compiled, 1, files), source)).toEqual(specOrReference("B11", ["report(alpha)"], []));
    });
  });

  describe("B12 parsed sub-rules order capture dependencies", () => {
    it("matches whether a parsed-tree comparison is authored before its bindings or after", () => {
      const source = "value = '{x}{x}'\n";
      const parsed = (items: unknown[]) => clause("parsed-order", {
        kind: "string",
        parse: { grammar: "braceFormat", all: items },
      });
      const bindings = [{
        has: { kind: "field", capture: "A", precedes: { kind: "field", capture: "B" } },
      }];
      const comparison = { compare: { left: "A", op: "eq", right: "B", text: "source" } };
      const files = [{ path: "a.py", language: "python" as const, source }];
      expect(texts(evaluate(compile([parsed([...bindings, comparison])]), files), source)).toEqual(["'{x}{x}'"]);
      expect(texts(evaluate(compile([parsed([comparison, ...bindings])]), files), source)).toEqual(specOrReference("B12", ["'{x}{x}'"], []));
    });
  });

  describe("B13 evidence budget group identities", () => {
    it("keeps distinct over-budget groups distinct and stable under file reordering", () => {
      const codeRule = document([{
        id: "groups",
        budget: {
          query: { evidence: {
            as: "symbol",
            query: { ast: { language: "python", rule: { pattern: "symbol($GROUP)" } } },
            project: { group: "capture.GROUP" },
          } },
          group_by: ["symbol.group"],
          count: { lte: 1 },
        },
        violation: { anchor: "witness.span", code: "groups", message: "groups" },
      }]);
      const compiled = loadNativeBinding().CodeRuleDslRuntime!.compile(codeRule, "json", capabilities);
      const files = ["alpha", "alpha", "beta", "beta"].map((group, index) => ({
        path: `${index}.py`, language: "python", source: `symbol(${group})\n`,
      }));
      const snapshot = (inputs: typeof files) => ({ format: "rerail.repository-snapshot/v1", revision: 1, files: inputs });
      const first = (JSON.parse(compiled.evaluate(JSON.stringify(snapshot(files)), "report")) as Result).violations.map(item => item.id);
      const reversed = (JSON.parse(compiled.evaluate(JSON.stringify(snapshot([...files].reverse())), "report")) as Result).violations.map(item => item.id);
      expect(new Set(first).size).toBe(2);
      expect(reversed.toSorted()).toEqual(first.toSorted());
    });
  });

  describe("B14 evidence spans", () => {
    it("reports source coordinates from an evidence witness", () => {
      const codeRule = document([{
        id: "evidence-span",
        budget: {
          query: { evidence: {
            as: "symbol",
            query: { ast: { language: "python", rule: { pattern: "symbol($GROUP)" } } },
            project: { group: "capture.GROUP" },
          } },
          count: { lte: 0 },
        },
        violation: { anchor: "witness.span", code: "evidence-span", message: "evidence-span" },
      }]);
      const compiled = loadNativeBinding().CodeRuleDslRuntime!.compile(codeRule, "json", capabilities);
      const result = JSON.parse(compiled.evaluate(JSON.stringify({
        format: "rerail.repository-snapshot/v1", revision: 1,
        files: [{ path: "a.py", language: "python", source: "symbol(alpha)\n" }],
      }), "report")) as Result;
      expect(result.violations[0]!.span).toMatchObject({ startByte: 0, startLine: 0, startColumn: 0 });
    });
  });

  describe("B15 parameterized utility regex groups", () => {
    it("exports a called utility's witness regex group to each reporting", () => {
      const source = "report(alpha);\n";
      const create = () => compile([{
        id: "called-group",
        forbid: { ast: {
          language: "typescript",
          rule: { pattern: "$X($$$ARGS)", matches: { report: { P: "$X" } } },
          utils: { report: { params: ["P"], rule: { regex: "^report\\((?<ITEM>\\w+)\\)$" } } },
        } },
        violation: { anchor: "witness.span", code: "called-group", message: "$ITEM", each: "ITEM" },
      }]);
      if (USING_REFERENCE_ADDON) {
        expect(create).toThrow(/`\$ITEM` names no capture/);
        return;
      }
      const files = [{ path: "a.ts", language: "typescript" as const, source }];
      expect(texts(evaluate(create(), files), source)).toEqual(["report(alpha)"]);
    });
  });

  describe("A21 violation identity lifecycle", () => {
    it("keeps unchanged violation identities stable across an unrelated edit", async () => {
      const compiled = compile([clause("identity", { pattern: "f($X)", capture: "X" }, "typescript", { violation: { anchor: "capture.X", code: "identity", message: "$X", each: "X" } })]);
      const native = new (loadNativeBinding().CodeRuleDslWorkspace!)();
      try {
        native.addCodeRule(compiled);
        const one = JSON.parse(await native.applyAsync(JSON.stringify({ revision: 1, files: [{ path: "a.ts", language: "typescript", source: "f(a); f(b);" }], removed: [] }))).evaluation as Result;
        const two = JSON.parse(await native.applyAsync(JSON.stringify({ revision: 2, files: [{ path: "unrelated.py", language: "python", source: "x = 1" }], removed: [] }))).evaluation as Result;
        expect(new Set(one.violations.map(item => item.id)).size).toBe(2);
        expect(two.violations.filter(item => item.path === "a.ts").map(item => item.id).sort()).toEqual(one.violations.map(item => item.id).sort());
      } finally { native.close(); }
    });
  });

  describe("A22 nested evaluation cleanup", () => {
    it("allows a later code rule to run after a parsed-tree evaluation", () => {
      const parsed = compile([clause("parsed", { kind: "string", parse: { grammar: "annotation", kind: "identifier" } })]);
      const later = compile([clause("later", { kind: "identifier", capture: "X" })]);
      expect(evaluate(parsed, [{ path: "a.py", language: "python", source: "x: 'Thing'\n" }]).status).toBe("completed");
      expect(evaluate(later, [{ path: "b.py", language: "python", source: "after = 1\n" }]).violations).not.toHaveLength(0);
    });
  });
  describe("A23 line model agreement", () => {
    it("reports the same line that layout and transform.line use for bare CR and U+2028", async () => {
      const source = "const first = 1;\rconst second = 2;\u2028const target = 3;\n";
      const compiled = compile([{
        id: "line-model",
        forbid: {
          ast: {
            language: "typescript",
            rule: { pattern: "const target = $VALUE;", layout: { span: { eq: 0 } } },
            transform: { LINE: { line: { source: "$VALUE" } } },
          },
        },
        violation: { anchor: "capture.VALUE", code: "line-model", message: "line $LINE" },
      }]);
      const files = [{ path: "a.ts", language: "typescript" as const, source }];
      for (const result of [evaluate(compiled, files), await workspace(compiled, 1, files)]) {
        expect(result.violations).toHaveLength(1);
        expect(result.violations[0]!.span!.startLine).toBe(2);
        expect(result.violations[0]!.message).toBe("line 3");
      }
    });
  });
});
