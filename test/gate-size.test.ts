/**
 * Check 9 (size, conformance/gates/size.ts): V1 Gate 4's tests from the rule validation ladder, each a form of
 * expansion or repetition a port took, or one it must not be accused of.
 */
import { describe, expect, it } from "vitest";
import { NESTING_RUN_LIMIT, REPEATED_SHARE_LIMIT, SIZE_FLAG_BYTES, sizeGate } from "../conformance/gates/size.js";

const FLAGGED = SIZE_FLAG_BYTES + 1;

function rule(body: Record<string, unknown>, extra: Record<string, unknown> = {}): { clauses: unknown[] } {
  return { clauses: [{ id: "c", forbid: { ast: { language: "typescript", rule: body, ...extra } } }] };
}

function share(document: { clauses?: unknown }): number {
  return sizeGate(FLAGGED, document).metrics.repeatedShare;
}

function passes(document: { clauses?: unknown }, id: string): boolean {
  return sizeGate(FLAGGED, document).checks.find(check => check.id === id)!.ok;
}

describe("check 9: size", () => {
  it("passes small rules and flagged rules that write nothing out over and over", () => {
    const small = sizeGate(100, rule({ kind: "debugger_statement" }));
    expect(small.checks.map(check => [check.id, check.ok])).toEqual([
      ["size:combinatorial-expansion", true],
      ["size:written-out-repetition", true],
    ]);
    expect(small.metrics.flagged).toBe(false);
    // Two variants of one helper (break and continue) are a pairing, not repetition over and over.
    const variant = (keyword: string) => ({
      all: [{ kind: `${keyword}_statement` }, { not: { has: { kind: "statement_identifier", stopBy: "end" } } }, { inside: { kind: "switch_body", stopBy: "end" } }],
    });
    const paired = rule({ any: [{ matches: "collects-break" }, { matches: "collects-continue" }] }, { utils: { "collects-break": variant("break"), "collects-continue": variant("continue") } });
    expect(share(paired)).toBe(0);
    expect(sizeGate(FLAGGED, paired).checks.map(check => check.ok)).toEqual([true, true]);
    expect(sizeGate(FLAGGED, paired).metrics.flagged).toBe(true);
  });

  it("sends back a cartesian product written out cell by cell", () => {
    const cells = [];
    for (const receiver of ["window", "self", "globalThis", "top", "parent"]) {
      for (const method of ["setTimeout", "setInterval", "setImmediate", "requestIdleCallback", "queueMicrotask", "postMessage"]) {
        for (const enclosing of ["class_declaration", "function_declaration", "arrow_function", "method_definition", "program"]) {
          cells.push({ kind: "call_expression", regex: `^${receiver}\\.${method}\\b`, inside: { kind: enclosing, stopBy: "end" } });
        }
      }
    }
    const result = sizeGate(FLAGGED, rule({ any: cells }));
    expect(result.metrics.repeatedShare, `repeated share ${result.metrics.repeatedShare}`).toBeGreaterThan(0.9);
    expect(result.checks.find(check => check.id === "size:combinatorial-expansion")!.ok).toBe(false);
  });

  it("sends back a product whose cells differ in shape but repeat their parts verbatim", () => {
    const receivers = ["window", "self", "globalThis", "top", "parent"];
    const methods = ["setTimeout", "setInterval", "setImmediate", "requestIdleCallback", "queueMicrotask", "postMessage"];
    const wrappers = (count: number) => ({ inside: { any: Array.from({ length: count }, (_, index) => ({ kind: `wrapper_kind_${index}` })), stopBy: "end" } });
    const argumentsOf = (count: number) => ({ has: { field: "arguments", any: Array.from({ length: count }, (_, index) => ({ kind: `argument_kind_${index}` })) } });
    const cells = receivers.flatMap((receiver, r) => methods.map((method, m) => ({ all: [{ regex: `^${receiver}\\.${method}$` }, wrappers(r + 1), argumentsOf(m + 1)] })));
    expect(share(rule({ any: cells })), `repeated share ${share(rule({ any: cells }))}`).toBeGreaterThanOrEqual(REPEATED_SHARE_LIMIT);
  });

  it("sends back a product hidden in a utility map or written in small cells", () => {
    const receivers = ["window", "self", "globalThis", "top", "parent"];
    const methods = ["setTimeout", "setInterval", "setImmediate", "requestIdleCallback", "queueMicrotask", "postMessage"];
    const cells = receivers.flatMap(receiver => methods.map(method => ({ kind: "call_expression", regex: `^${receiver}\\.${method}\\b`, inside: { kind: "program", stopBy: "end" } })));
    const utils = Object.fromEntries(cells.map((cell, index) => [`c${index}`, cell]));
    const inUtils = rule({ any: cells.map((_, index) => ({ matches: `c${index}` })) }, { utils });
    expect(share(inUtils), `utility map share ${share(inUtils)}`).toBeGreaterThanOrEqual(REPEATED_SHARE_LIMIT);
    const small = rule({ any: receivers.flatMap(receiver => methods.map(method => ({ pattern: `${receiver}.${method}($$$A)` }))) });
    expect(share(small), `small cells share ${share(small)}`).toBeGreaterThanOrEqual(REPEATED_SHARE_LIMIT);
  });

  it("does not treat a table of distinct cases as expansion", () => {
    const apis = ["util.isArray", "util.isBoolean", "fs.exists", "path._makeLong", "crypto.createCredentials", "domain.create", "os.tmpDir", "buffer.SlowBuffer"];
    const table = {
      clauses: apis.map((api, index) => ({
        id: `api-${index}`,
        forbid: { ast: { language: "typescript", rule: { kind: "call_expression", pattern: `${api}($$$A)` } } },
        violation: { anchor: "witness.span", code: "DEPRECATED", message: `'${api}' was deprecated; use its replacement instead.` },
      })),
    };
    expect(share(table)).toBe(0);
  });

  it("finds repetition written out along one nesting path, through wrappers and beside other relations", () => {
    const chain = (depth: number, level: (inner: Record<string, unknown>) => Record<string, unknown>): Record<string, unknown> => {
      let node: Record<string, unknown> = { kind: "identifier" };
      for (let index = 0; index < depth; index += 1) node = level(node);
      return node;
    };
    const elseIf = chain(30, inner => ({ kind: "if_statement", has: { field: "alternative", kind: "else_clause", has: inner } }));
    const wrapped = chain(30, inner => ({ kind: "parenthesized_expression", all: [{ has: inner }] }));
    const beside = chain(50, inner => ({ kind: "parenthesized_expression", has: inner, not: { kind: "comment" } }));
    for (const [name, body] of Object.entries({ elseIf, wrapped, beside })) {
      const result = sizeGate(FLAGGED, rule(body));
      expect(result.metrics.nestingRun, `${name}: ${result.metrics.nestingRun}`).toBeGreaterThanOrEqual(30);
      expect(passes(rule(body), "size:written-out-repetition"), name).toBe(false);
    }
    // A(A(A(x))) is a node kind three times along one path, whether or not the last copy has a child.
    expect(sizeGate(FLAGGED, rule(chain(3, inner => ({ kind: "parenthesized_expression", has: inner })))).metrics.nestingRun).toBe(NESTING_RUN_LIMIT);
    // The all-list style, utilities and count hide nothing.
    const allList = chain(14, inner => ({ all: [{ kind: "if_statement" }, { has: { field: "alternative", all: [{ kind: "else_clause" }, { has: inner }] } }] }));
    expect(sizeGate(FLAGGED, rule(allList)).metrics.nestingRun).toBe(14);
    const viaUtilities = rule(
      { matches: "level0" },
      { utils: Object.fromEntries(Array.from({ length: 10 }, (_, index) => [`level${index}`, { kind: "parenthesized_expression", has: index < 9 ? { matches: `level${index + 1}` } : { kind: "identifier" } }])) },
    );
    expect(sizeGate(FLAGGED, viaUtilities).metrics.nestingRun).toBe(10);
    const viaCount = chain(6, inner => ({ kind: "parenthesized_expression", count: { has: inner, min: 1 } }));
    expect(sizeGate(FLAGGED, rule(viaCount)).metrics.nestingRun).toBe(6);
    // Saying more about the same node is not nesting, nor are alternatives, nor what a node is not.
    expect(sizeGate(FLAGGED, rule({ kind: "call_expression", all: [{ kind: "call_expression" }], not: { kind: "call_expression" } })).metrics.nestingRun).toBe(1);
    expect(sizeGate(FLAGGED, rule({ any: [{ kind: "block", has: { any: [{ kind: "block" }, { kind: "x" }] } }, { kind: "y" }] })).metrics.nestingRun).toBe(2);
    // A recursive utility walks a tree; it is reported, not counted as written-out repetition.
    const recursive = rule({ matches: "walk" }, { utils: { walk: { any: [{ kind: "identifier" }, { kind: "parenthesized_expression", has: { matches: "walk" } }] } } });
    expect(sizeGate(FLAGGED, recursive).metrics.nestingRun).toBe(1);
  });

  it("reports recursive utilities called by id or with parameters, per query", () => {
    const callForm = rule({ matches: "walk" }, { utils: { walk: { params: ["N"], rule: { any: [{ kind: "call_expression" }, { has: { matches: { walk: { N: "$N" } } } }] } } } });
    expect(sizeGate(FLAGGED, callForm).metrics.recursiveUtils).toEqual(["walk"]);
    const separate = {
      clauses: [
        {
          id: "c",
          forbid: {
            select_when: {
              select: { ast: { language: "typescript", rule: { matches: "walk" }, utils: { walk: { any: [{ kind: "x" }, { has: { matches: "walk" } }] } } } },
              exists: { ast: { language: "typescript", rule: { matches: "walk" }, utils: { walk: { kind: "y" } } } },
            },
          },
        },
      ],
    };
    expect(sizeGate(FLAGGED, separate).metrics.recursiveUtils).toEqual(["walk"]);
  });

  it("measures the canonical form core stores, whatever the key order the document was written in", () => {
    const written = { id: "r", clauses: [{ violation: { message: "m", code: "C" }, id: "c", forbid: { ast: { rule: { kind: "x" }, language: "python" } } }] };
    const reordered = { clauses: [{ id: "c", forbid: { ast: { language: "python", rule: { kind: "x" } } }, violation: { code: "C", message: "m" } }], id: "r" };
    expect(sizeGate(100, written).metrics).toEqual(sizeGate(100, reordered).metrics);
    expect(sizeGate(100, written).metrics.canonicalBytes).toBe(JSON.stringify(reordered).length);
  });
});
