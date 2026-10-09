import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../src/engine/native/native.js";
import { reactComponentSource, typescriptModuleSource } from "../../support/synthetic-typescript.js";

/**
 * Scale guard for TypeScript AST evaluation: a `typescript` rule now compiles against the TypeScript and TSX
 * grammars, and the engine must still parse and walk each file exactly once with the grammar its label selects.
 */

const FILES_PER_LANGUAGE = 500;
/** Generous wall-clock ceiling; a release engine evaluates this corpus in well under a second on a laptop. */
const EVALUATION_BUDGET_MS = 5_000;

type SnapshotFile = { language: "typescript" | "tsx"; path: string; source: string };

const runtime = loadNativeBinding().CodeRuleDslRuntime;

function capabilities(): string {
  return JSON.stringify({
    format: "rerail.policy-capabilities/v1",
    ast: {
      engine: "ast-grep-core",
      engineVersion: "0.45",
      languages: { typescript: "tree-sitter-typescript@0.23" },
    },
    factRelations: {},
    limits: {
      maxAstPatternBytes: 16_384,
      maxFiles: 10_000,
      maxGraphDepth: 16,
      maxRegexPatternBytes: 16_384,
      maxSourceBytes: 32_000_000,
      maxVisitedNodes: 20_000_000,
      maxWitnesses: 10_000,
    },
  });
}

function consoleCodeRule(): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  return runtime.compile(
    JSON.stringify({
      clauses: [{
        violation: { anchor: "witness.span", code: "CONSOLE", message: "No console output" },
        forbid: { ast: { language: "typescript", rule: { pattern: "console.log($$$ARGS)" } } },
        id: "console",
      }],
      format: "rerail.policy/v1",
      id: "scale:console",
      mode: "block",
      revision: 1,
      scope: { exclude: [], include: ["**/*"] },
    }),
    "json",
    capabilities(),
  );
}

function corpus(language: SnapshotFile["language"]): SnapshotFile[] {
  return Array.from({ length: FILES_PER_LANGUAGE }, (_, index) => ({
    language,
    path: `web/src/${language}/module-${index}.${language === "tsx" ? "tsx" : "ts"}`,
    source: language === "tsx" ? reactComponentSource(index) : typescriptModuleSource(index),
  }));
}

function evaluate(compiled: NativeCompiledCodeRuleDsl, files: SnapshotFile[]) {
  const input = JSON.stringify({ facts: [], files, format: "rerail.repository-snapshot/v1", revision: 0 });
  const started = performance.now();
  const result = JSON.parse(compiled.evaluate(input, "report")) as {
    status: string;
    violations: Array<{ path: string }>;
    failures: unknown[];
    metrics: { files: number; visitedNodes: number };
  };
  return { elapsedMs: performance.now() - started, result };
}

describe("TypeScript AST evaluation at scale", () => {
  it("parses a mixed .ts/.tsx corpus once per file within budget", () => {
    const compiled = consoleCodeRule();
    const typescript = corpus("typescript");
    const tsx = corpus("tsx");

    const mixed = evaluate(compiled, [...typescript, ...tsx]);
    expect(mixed.result).toMatchObject({ failures: [], status: "completed" });
    expect(mixed.result.violations).toHaveLength(typescript.length + tsx.length);
    expect(mixed.result.metrics.files).toBe(typescript.length + tsx.length);

    const typescriptOnly = evaluate(compiled, typescript).result;
    const tsxOnly = evaluate(compiled, tsx).result;
    expect(tsxOnly.violations).toHaveLength(tsx.length);
    expect(mixed.result.metrics.visitedNodes).toBe(typescriptOnly.metrics.visitedNodes + tsxOnly.metrics.visitedNodes);

    const bytes = [...typescript, ...tsx].reduce((total, file) => total + file.source.length, 0);
    expect(bytes).toBeGreaterThan(1_000_000);
    expect(mixed.elapsedMs, `evaluated ${bytes} bytes in ${mixed.elapsedMs.toFixed(0)}ms`).toBeLessThan(EVALUATION_BUDGET_MS);
  });
});
