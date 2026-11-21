import assert from "node:assert/strict";
import { test } from "vitest";

import { NESTING_RUN_LIMIT, REPEATED_SHARE_LIMIT, SIZE_FLAG_BYTES, compactGate, type RepetitionExemption } from "../../conformance/languages/checks/compact.js";

const FLAGGED = SIZE_FLAG_BYTES + 1;

function rule(body: Record<string, unknown>, extra: Record<string, unknown> = {}): { clauses: unknown[] } {
  return { clauses: [{ id: "c", forbid: { ast: { language: "typescript", rule: body, ...extra } } }] };
}

function share(document: { clauses?: unknown }): number {
  return compactGate(FLAGGED, document).metrics.repeatedShare;
}

test("compact passes small rules and flagged rules that write nothing out over and over", () => {
  assert.deepEqual(compactGate(100, rule({ kind: "debugger_statement" })).checks.map((check) => check.ok), [true]);
  // Two variants of one helper (break and continue) are a pairing, not repetition over and over.
  const variant = (keyword: string) => ({ all: [{ kind: `${keyword}_statement` }, { not: { has: { kind: "statement_identifier", stopBy: "end" } } }, { inside: { kind: "switch_body", stopBy: "end" } }] });
  const paired = rule({ any: [{ matches: "collects-break" }, { matches: "collects-continue" }] }, { utils: { "collects-break": variant("break"), "collects-continue": variant("continue") } });
  assert.equal(share(paired), 0);
  assert.deepEqual(compactGate(FLAGGED, paired).checks.map((check) => check.ok), [true, true, true]);
});

test("compact sends back a cartesian product written out cell by cell", () => {
  const cells = [];
  for (const receiver of ["window", "self", "globalThis", "top", "parent"]) {
    for (const method of ["setTimeout", "setInterval", "setImmediate", "requestIdleCallback", "queueMicrotask", "postMessage"]) {
      for (const enclosing of ["class_declaration", "function_declaration", "arrow_function", "method_definition", "program"]) {
        cells.push({ kind: "call_expression", regex: `^${receiver}\\.${method}\\b`, inside: { kind: enclosing, stopBy: "end" } });
      }
    }
  }
  const result = compactGate(FLAGGED, rule({ any: cells }));
  assert.ok(result.metrics.repeatedShare > 0.9, `repeated share ${result.metrics.repeatedShare}`);
  assert.equal(result.checks.find((check) => check.id === "no-combinatorial-expansion")!.ok, false);
});

test("compact sends back a product whose cells differ in shape but repeat their parts verbatim", () => {
  const receivers = ["window", "self", "globalThis", "top", "parent"];
  const methods = ["setTimeout", "setInterval", "setImmediate", "requestIdleCallback", "queueMicrotask", "postMessage"];
  const wrappers = (count: number) => ({ inside: { any: Array.from({ length: count }, (_, index) => ({ kind: `wrapper_kind_${index}` })), stopBy: "end" } });
  const argumentsOf = (count: number) => ({ has: { field: "arguments", any: Array.from({ length: count }, (_, index) => ({ kind: `argument_kind_${index}` })) } });
  const cells = receivers.flatMap((receiver, r) => methods.map((method, m) => ({ all: [{ regex: `^${receiver}\\.${method}$` }, wrappers(r + 1), argumentsOf(m + 1)] })));
  assert.ok(share(rule({ any: cells })) >= REPEATED_SHARE_LIMIT, `repeated share ${share(rule({ any: cells }))}`);
});

test("compact sends back a product hidden in a utility map or written in small cells", () => {
  const receivers = ["window", "self", "globalThis", "top", "parent"];
  const methods = ["setTimeout", "setInterval", "setImmediate", "requestIdleCallback", "queueMicrotask", "postMessage"];
  const cells = receivers.flatMap((receiver) => methods.map((method) => ({ kind: "call_expression", regex: `^${receiver}\\.${method}\\b`, inside: { kind: "program", stopBy: "end" } })));
  const utils = Object.fromEntries(cells.map((cell, index) => [`c${index}`, cell]));
  const inUtils = rule({ any: cells.map((_, index) => ({ matches: `c${index}` })) }, { utils });
  assert.ok(share(inUtils) >= REPEATED_SHARE_LIMIT, `utility map share ${share(inUtils)}`);
  const small = rule({ any: receivers.flatMap((receiver) => methods.map((method) => ({ pattern: `${receiver}.${method}($$$A)` }))) });
  assert.ok(share(small) >= REPEATED_SHARE_LIMIT, `small cells share ${share(small)}`);
});

test("compact does not treat a table of distinct cases as expansion", () => {
  const apis = ["util.isArray", "util.isBoolean", "fs.exists", "path._makeLong", "crypto.createCredentials", "domain.create", "os.tmpDir", "buffer.SlowBuffer"];
  const table = { clauses: apis.map((api, index) => ({
    id: `api-${index}`,
    forbid: { ast: { language: "typescript", rule: { kind: "call_expression", pattern: `${api}($$$A)` } } },
    violation: { anchor: "witness.span", code: "DEPRECATED", message: `'${api}' was deprecated; use its replacement instead.` },
  })) };
  assert.equal(share(table), 0);
});

test("compact finds repetition written out along one nesting path, through wrappers and beside other relations", () => {
  const chain = (depth: number, level: (inner: Record<string, unknown>) => Record<string, unknown>): Record<string, unknown> => {
    let node: Record<string, unknown> = { kind: "identifier" };
    for (let index = 0; index < depth; index += 1) node = level(node);
    return node;
  };
  const elseIf = chain(30, (inner) => ({ kind: "if_statement", has: { field: "alternative", kind: "else_clause", has: inner } }));
  const wrapped = chain(30, (inner) => ({ kind: "parenthesized_expression", all: [{ has: inner }] }));
  const beside = chain(50, (inner) => ({ kind: "parenthesized_expression", has: inner, not: { kind: "comment" } }));
  for (const [name, body] of Object.entries({ elseIf, wrapped, beside })) {
    const result = compactGate(FLAGGED, rule(body));
    assert.ok(result.metrics.nestingRun >= 30, `${name}: ${result.metrics.nestingRun}`);
    assert.equal(result.checks.find((check) => check.id === "no-written-out-repetition")!.ok, false, name);
  }
  // A(A(A(x))) is a node kind three times along one path, whether or not the last copy has a child.
  assert.equal(compactGate(FLAGGED, rule(chain(3, (inner) => ({ kind: "parenthesized_expression", has: inner })))).metrics.nestingRun, NESTING_RUN_LIMIT);
  // The all-list style, utilities and count hide nothing.
  const allList = chain(14, (inner) => ({ all: [{ kind: "if_statement" }, { has: { field: "alternative", all: [{ kind: "else_clause" }, { has: inner }] } }] }));
  assert.equal(compactGate(FLAGGED, rule(allList)).metrics.nestingRun, 14);
  const viaUtilities = rule({ matches: "level0" }, { utils: Object.fromEntries(Array.from({ length: 10 }, (_, index) => [`level${index}`, { kind: "parenthesized_expression", has: index < 9 ? { matches: `level${index + 1}` } : { kind: "identifier" } }])) });
  assert.equal(compactGate(FLAGGED, viaUtilities).metrics.nestingRun, 10);
  const viaCount = chain(6, (inner) => ({ kind: "parenthesized_expression", count: { has: inner, min: 1 } }));
  assert.equal(compactGate(FLAGGED, rule(viaCount)).metrics.nestingRun, 6);
  // Saying more about the same node is not nesting, nor are alternatives, nor what a node is not.
  assert.equal(compactGate(FLAGGED, rule({ kind: "call_expression", all: [{ kind: "call_expression" }], not: { kind: "call_expression" } })).metrics.nestingRun, 1);
  assert.equal(compactGate(FLAGGED, rule({ any: [{ kind: "block", has: { any: [{ kind: "block" }, { kind: "x" }] } }, { kind: "y" }] })).metrics.nestingRun, 2);
  // A recursive utility walks a tree; it is reported, not counted as written-out repetition.
  const recursive = rule({ matches: "walk" }, { utils: { walk: { any: [{ kind: "identifier" }, { kind: "parenthesized_expression", has: { matches: "walk" } }] } } });
  assert.equal(compactGate(FLAGGED, recursive).metrics.nestingRun, 1);
});

test("compact follows a query into its language's shared define utilities", () => {
  const nested = (depth: number): Record<string, unknown> => Object.fromEntries(Array.from({ length: depth }, (_, index) => [`level${index}`, { kind: "parenthesized_expression", has: index < depth - 1 ? { matches: `level${index + 1}` } : { kind: "identifier" } }]));
  const shared = (language: string, utils: Record<string, unknown>) => ({ id: "shared", define: { language, utils } });
  const query = (extra: Record<string, unknown> = {}) => ({ id: "c", forbid: { ast: { language: "typescript", rule: { matches: "level0" }, ...extra } } });
  // A parenthesized expression three deep, written out through shared utilities, is written-out repetition.
  const result = compactGate(FLAGGED, { clauses: [shared("typescript", nested(3)), query()] });
  assert.equal(result.metrics.nestingRun, NESTING_RUN_LIMIT);
  assert.equal(result.checks.find((check) => check.id === "no-written-out-repetition")!.ok, false);
  // Another language's define is invisible to the query.
  assert.equal(compactGate(FLAGGED, { clauses: [shared("python", nested(3)), query()] }).metrics.nestingRun, 0);
  // A query's own utilities and its shared ones join along one path.
  const ownFirst = query({ utils: { level0: { kind: "parenthesized_expression", has: { matches: "level1" } } } });
  assert.equal(compactGate(FLAGGED, { clauses: [shared("typescript", { level1: nested(3).level1, level2: nested(3).level2 }), ownFirst] }).metrics.nestingRun, NESTING_RUN_LIMIT);
  // Shared utilities that reach each other walk a tree and end.
  const cycle = { a: { kind: "parenthesized_expression", has: { matches: "b" } }, b: { kind: "parenthesized_expression", has: { matches: "a" } } };
  const cyclic = compactGate(FLAGGED, { clauses: [shared("typescript", cycle), { id: "c", forbid: { ast: { language: "typescript", rule: { matches: "a" } } } }] });
  assert.equal(cyclic.metrics.nestingRun, 2);
});

test("compact passes the runs a rule declares are not copies, fails a run it does not declare, and fails the declaration once they are gone", () => {
  const chain = (depth: number, kind = "parenthesized_expression"): Record<string, unknown> => (depth === 1 ? { kind } : { kind, has: chain(depth - 1, kind) });
  const repetition = (bytes: number, body: Record<string, unknown>, exempt?: RepetitionExemption) => compactGate(bytes, rule(body), exempt).checks.find((check) => check.id === "no-written-out-repetition");
  const reason = "each step is the next enclosing expression";
  const exempt = { runs: { parenthesized_expression: 3 }, reason };
  const measured = (most: number, long: string) => `the most times one node kind occurs along a single nesting path: ${most} (limit ${NESTING_RUN_LIMIT})${long === "" ? "" : `; runs of ${NESTING_RUN_LIMIT} or more: ${long}`}`;
  assert.deepEqual(repetition(FLAGGED, chain(3), exempt), { id: "no-written-out-repetition", ok: true, detail: `exempt (${reason}); ${measured(3, "parenthesized_expression 3")}` });
  // Without the declaration the same run still fails.
  assert.equal(repetition(FLAGGED, chain(3))!.ok, false);
  // A run the declaration does not name, or a longer one than it names, was not judged: the declaration covers neither.
  const notDeclared = (most: number, long: string, excused = "parenthesized_expression 3") => `the exemption (${reason}) excuses ${excused}, not the runs measured; ${measured(most, long)}; set its runs to those measured if they are not copies either, or rewrite the repetition`;
  assert.deepEqual(repetition(FLAGGED, { all: [chain(3), { has: chain(6, "call_expression") }] }, exempt), { id: "no-written-out-repetition", ok: false, detail: notDeclared(6, "call_expression 6, parenthesized_expression 3") });
  assert.deepEqual(repetition(FLAGGED, chain(4), exempt), { id: "no-written-out-repetition", ok: false, detail: notDeclared(4, "parenthesized_expression 4") });
  // A run it names that went away no longer needs excusing, though another run still does.
  const tooMany = { runs: { call_expression: 3, parenthesized_expression: 3 }, reason };
  assert.deepEqual(repetition(FLAGGED, chain(3), tooMany), { id: "no-written-out-repetition", ok: false, detail: notDeclared(3, "parenthesized_expression 3", "call_expression 3, parenthesized_expression 3") });
  // Once every run is under the limit, or the document under the flag, the declaration is stale.
  assert.deepEqual(repetition(FLAGGED, chain(2), exempt), { id: "no-written-out-repetition", ok: false, detail: `stale exemption (${reason}): ${measured(2, "")}; remove the exemption` });
  assert.deepEqual(repetition(100, chain(3), exempt), { id: "no-written-out-repetition", ok: false, detail: `stale exemption (${reason}): 100 bytes, under the ${SIZE_FLAG_BYTES}-byte flag; remove the exemption` });
  assert.equal(repetition(100, chain(3)), undefined);
});

test("compact reports recursive utilities called by id or with parameters, per query", () => {
  const callForm = rule({ matches: "walk" }, { utils: { walk: { params: ["N"], rule: { any: [{ kind: "call_expression" }, { has: { matches: { walk: { N: "$N" } } } }] } } } });
  assert.deepEqual(compactGate(FLAGGED, callForm).metrics.recursiveUtils, ["walk"]);
  const separate = {
    clauses: [{
      id: "c",
      forbid: {
        select_when: {
          select: { ast: { language: "typescript", rule: { matches: "walk" }, utils: { walk: { any: [{ kind: "x" }, { has: { matches: "walk" } }] } } } },
          exists: { ast: { language: "typescript", rule: { matches: "walk" }, utils: { walk: { kind: "y" } } } },
        },
      },
    }],
  };
  assert.deepEqual(compactGate(FLAGGED, separate).metrics.recursiveUtils, ["walk"]);
});
