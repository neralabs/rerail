import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../src/engine/index.js";

const native = loadNativeBinding();
const capabilities = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const compile = (document: unknown) => native.CodeRuleDslRuntime.compile(JSON.stringify(document), "json", capabilities);
const literal = (value: string) => `'${value.replaceAll("'", "''")}'`;
function document(roots: string[], edges: [string, string][], batchSize?: number, recursive = false, forest = false) {
  const rootQuery = `SELECT DISTINCT __path, value AS id FROM functions CROSS JOIN json_each(${literal(JSON.stringify(roots))})`;
  const edgeQuery = `SELECT DISTINCT __path,json_extract(value,'$[0]') AS src,json_extract(value,'$[1]') AS dst FROM functions CROSS JOIN json_each(${literal(JSON.stringify(edges))}) WHERE __path='a.py'`;
  return {
    format: "rerail.policy/v1", id: "reachability", revision: 1, mode: "log",
    clauses: [{ id: "graph", relational: {
      inputs: { functions: { evidence: { as: "f", query: { ast: { language: "python", rule: { kind: "function_definition" } } }, project: { path: "file.path" } } } },
      stages: [{ name: "requested", query: rootQuery }, { name: "links", query: edgeQuery }, recursive ? {
        name: "closure", query: `WITH RECURSIVE walk(__path,root,node) AS (SELECT __path,id,id FROM requested UNION SELECT w.__path,w.root,e.dst FROM walk w JOIN links e ON e.__path=w.__path AND e.src=w.node) SELECT * FROM walk`,
      } : { name: "closure", reachability: { roots: "SELECT __path,id FROM requested", edges: "SELECT __path,src,dst FROM links", ...(forest ? { graph: "forest" } : {}), ...(batchSize === undefined ? {} : { batchSize }) } }],
      query: "SELECT f.__path AS path,f.__start AS start_byte,f.__end AS end_byte,c.root,c.node FROM functions f JOIN closure c ON c.__path=f.__path",
      key: ["root", "node"],
    }, violation: { code: "GRAPH", message: "Reachable", anchor: "witness.span" } }],
  };
}
const files = ["a.py", "b.py"].map(path => ({ path, language: "python", source: "def read(): pass\n" }));
const evaluate = (rule: ReturnType<typeof compile>, current = files) => {
  const { failures, violations } = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: current }), "report"));
  return { failures, violations };
};
it("normalizes a derived forest within a linear SQL work allowance", () => {
  const doc = document(["0"], [], 256, false, true);
  doc.clauses[0]!.relational.stages[0]!.query = "SELECT __path,0 AS id FROM functions";
  doc.clauses[0]!.relational.stages[1]!.query = "WITH RECURSIVE seq(id) AS (SELECT 0 UNION ALL SELECT id+1 FROM seq WHERE id<2999) SELECT f.__path,seq.id AS src,seq.id+1 AS dst FROM functions f CROSS JOIN seq";
  doc.clauses[0]!.relational.query = "SELECT f.__path AS path,f.__start AS start_byte,f.__end AS end_byte,count(*) AS pairs FROM functions f JOIN closure c ON c.__path=f.__path GROUP BY f.__path,f.__start,f.__end";
  doc.clauses[0]!.relational.key = ["pairs"];
  const limits = JSON.parse(capabilities);
  Object.assign(limits.limits, { maxVisitedNodes: 25_000, maxVisitsPerNode: 0 });
  const actual = evaluate(native.CodeRuleDslRuntime.compile(JSON.stringify(doc), "json", JSON.stringify(limits)));
  expect(actual.failures).toEqual([]);
  expect(actual.violations).toHaveLength(2);
  const oracle = structuredClone(doc);
  Object.assign(oracle.clauses[0]!.relational.stages[2]!, { query: "WITH RECURSIVE walk(__path,root,node) AS (SELECT __path,id,id FROM requested UNION SELECT w.__path,w.root,e.dst FROM walk w JOIN links e ON e.__path=w.__path AND e.src=w.node) SELECT * FROM walk" });
  delete oracle.clauses[0]!.relational.stages[2]!.reachability;
  expect(actual.violations).toEqual(evaluate(compile(oracle)).violations);
});
function pairCount(roots: string[], edges: [string, string][]) {
  let count = roots.length; // b.py has identical keys but no edges.
  for (const root of new Set(roots)) {
    const seen = new Set([root]); const pending = [root];
    while (pending.length) {
      const next = pending.pop();
      for (const [src, dst] of edges) if (src === next && !seen.has(dst)) { seen.add(dst); pending.push(dst); }
    }
    count += seen.size;
  }
  return count;
}

it.each([1, 2, 256, 4096])("matches recursive SQL on forests with batches of %i", batch => {
  const roots = ["r", "b", "孤立'", "r"];
  const edges: [string, string][] = [["r", "a"], ["r", "b"], ["a", "c"], ["r", "a"], ["c", "missing"], ["missing", "tail"]];
  const doc = document(roots, edges, batch, false, true);
  expect(codeRuleSchema.safeParse(doc).success).toBe(true);
  const actual = evaluate(compile(doc));
  expect(actual.failures).toEqual([]);
  expect(actual.violations).toEqual(evaluate(compile(document(roots, edges, batch, true))).violations);
});

it.each(([
  [["r", "a"], ["a", "r"]],
  [["r", "c"], ["b", "c"]],
  [["r", "r"]],
] as [string, string][][]).map(edges => ({ edges })))("rejects invalid forest shapes without partial findings: %j", ({ edges }) => {
  const actual = evaluate(compile(document(["r"], edges, 2, false, true)));
  expect(actual.violations).toEqual([]);
  expect(actual.failures).toEqual([expect.objectContaining({ code: "relational-failed", message: expect.stringContaining("forest") })]);
});

it("handles a deep forest iteratively and preserves SQLite numeric identities", () => {
  const edges = Array.from({ length: 500 }, (_, i): [string, string] => [String(i), String(i + 1)]);
  const doc = document(["0", "500", "isolated"], edges, 2, false, true);
  const actual = evaluate(compile(doc));
  expect(actual.failures).toEqual([]);
  expect(actual.violations).toEqual(evaluate(compile(document(["0", "500", "isolated"], edges, 2, true))).violations);
  const numeric = document(["r"], [], 2, false, true);
  numeric.clauses[0]!.relational.stages[0]!.query = "SELECT __path,1 AS id FROM functions UNION SELECT __path,1.0 FROM functions";
  numeric.clauses[0]!.relational.stages[1]!.query = "SELECT __path,1.0 AS src,2 AS dst FROM functions";
  const sql = structuredClone(numeric);
  delete sql.clauses[0]!.relational.stages[2]!.reachability!.graph;
  expect(evaluate(compile(numeric))).toEqual(evaluate(compile(sql)));
});

it("finishes the same derived closure within the SQL work allowance", () => {
  const roots = Array.from({ length: 450 }, (_, i) => String(i));
  const edges = roots.slice(1).map((node, i): [string, string] => [String(i), node]);
  const forest = document(roots, edges, 256, false, true);
  forest.clauses[0]!.relational.query = "SELECT f.__path AS path,f.__start AS start_byte,f.__end AS end_byte,count(*) AS pairs FROM functions f JOIN closure c ON c.__path=f.__path GROUP BY f.__path,f.__start,f.__end";
  forest.clauses[0]!.relational.key = ["pairs"];
  const sql = structuredClone(forest);
  delete sql.clauses[0]!.relational.stages[2]!.reachability!.graph;
  const limited = JSON.stringify({ ...JSON.parse(capabilities), limits: { ...JSON.parse(capabilities).limits, maxVisitedNodes: 150_000, maxVisitsPerNode: 0 } });
  const run = (doc: unknown) => evaluate(native.CodeRuleDslRuntime.compile(JSON.stringify(doc), "json", limited));
  const expected = run(sql), actual = run(forest);
  expect(expected.failures).toEqual([]);
  expect(actual.failures).toEqual([]);
  expect(actual.violations).toEqual(expected.violations);
});

it("indexes a 30,000-edge forest without recursive native calls", () => {
  const doc = document(["r"], [], 256, false, true);
  doc.clauses[0]!.relational.stages[0]!.query = "SELECT __path,0 AS id FROM functions UNION SELECT __path,30000 FROM functions";
  doc.clauses[0]!.relational.stages[1]!.query = "WITH RECURSIVE seq(i) AS (SELECT 0 UNION ALL SELECT i+1 FROM seq WHERE i<29999) SELECT __path,i AS src,i+1 AS dst FROM functions,seq WHERE __path='a.py'";
  doc.clauses[0]!.relational.query = "SELECT f.__path AS path,f.__start AS start_byte,f.__end AS end_byte,count(*) AS pairs FROM functions f JOIN closure c ON c.__path=f.__path GROUP BY f.__path,f.__start,f.__end ORDER BY f.__path";
  doc.clauses[0]!.relational.key = ["pairs"];
  const result = evaluate(compile(doc));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((v: { variables: { row: { pairs: number } } }) => v.variables.row.pairs)).toEqual([30002, 2]);
});

it.each([1, 2, 256, 4096])("matches recursive UNION on arbitrary graphs with batches of %i", batch => {
  // Cycles, diamonds, duplicate links, missing intermediate nodes and an isolated Unicode root.
  const roots = ["r", "b", "孤立'", "r"];
  const edges: [string, string][] = [["r", "a"], ["r", "b"], ["a", "c"], ["b", "c"], ["c", "r"], ["r", "a"], ["c", "missing"], ["missing", "tail"]];
  const old = evaluate(compile(document(roots, edges, batch, true)));
  const actual = evaluate(compile(document(roots, edges, batch)));
  expect(actual.failures).toEqual([]);
  expect(actual.violations).toEqual(old.violations);
  expect(actual.violations).toHaveLength(pairCount([...new Set(roots)], edges));
});

it("handles default batch boundaries, tails, empty roots, and independent stages", () => {
  const roots = Array.from({ length: 270 }, (_, i) => `n${i}`);
  const edges = roots.map((r): [string, string] => [r, "sink"]);
  const actual = evaluate(compile(document(roots, edges)));
  expect(actual.failures).toEqual([]);
  expect(actual.violations).toEqual(evaluate(compile(document(roots, edges, undefined, true))).violations);
  expect(actual.violations).toHaveLength(pairCount(roots, edges));
  const empty = document([], edges);
  const second = { name: "second", reachability: { roots: "SELECT __path,root AS id FROM closure", edges: "SELECT __path,src,dst FROM links" } };
  empty.clauses[0]!.relational.stages.push(second);
  empty.clauses[0]!.relational.query = empty.clauses[0]!.relational.query.replace("JOIN closure", "JOIN second");
  expect(evaluate(compile(empty))).toMatchObject({ failures: [], violations: [] });
});

it("matches deterministic random directed graphs", () => {
  let seed = 17;
  const random = () => (seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0);
  for (let run = 0; run < 12; run++) {
    const roots = ["0", "3", "7"];
    const edges = Array.from({ length: 30 }, (): [string, string] => [String(random() % 13), String(random() % 13)]);
    const actual = evaluate(compile(document(roots, edges, 2)));
    expect(actual.failures).toEqual([]);
    expect(actual.violations).toEqual(evaluate(compile(document(roots, edges, 2, true))).violations);
    expect(actual.violations).toHaveLength(pairCount(roots, edges));
  }
});

it("rebuilds the SQL snapshot after workspace edits, restores, and removals", async () => {
  const rule = compile(document(["r"], [["r", "child"]], 1));
  const workspace = new native.CodeRuleDslWorkspace();
  workspace.addCodeRule(rule);
  try {
    const apply = async (revision: number, current: typeof files, removed: string[] = []) => JSON.parse(await workspace.applyAsync(JSON.stringify({ revision, files: current, removed }))).evaluation;
    expect(await apply(1, files)).toMatchObject(evaluate(rule));
    const edited = { ...files[0]!, source: "value = 1\n" };
    expect(await apply(2, [edited])).toMatchObject(evaluate(rule, [edited, files[1]!]));
    expect(await apply(3, [files[0]!])).toMatchObject(evaluate(rule));
    expect(await apply(4, [], ["b.py"])).toMatchObject(evaluate(rule, [files[0]!]));
  } finally { workspace.close(); }
});

it.each([0, 4097, 1.5])("rejects invalid batch size %i in both compilers", batch => {
  const invalid = document(["r"], [], batch);
  expect(codeRuleSchema.safeParse(invalid).success).toBe(false);
  expect(() => compile(invalid)).toThrow("batchSize");
});

it.each([
  { roots: "DELETE FROM requested", edges: "SELECT __path,src,dst FROM links" },
  { roots: "SELECT __path,id FROM closure", edges: "SELECT __path,src,dst FROM links" },
  { roots: "SELECT __path,id FROM requested", edges: "SELECT __path,src FROM links" },
  { roots: "SELECT __path,id FROM requested WHERE id=?", edges: "SELECT __path,src,dst FROM links" },
  { roots: "SELECT __path,id FROM requested UNION SELECT __path,key FROM __reach_closure_keys", edges: "SELECT __path,src,dst FROM links" },
])("validates reachability queries with ordinary SQL restrictions: %j", reachability => {
  const invalid = document(["r"], []);
  invalid.clauses[0]!.relational.stages[2] = { name: "closure", reachability };
  expect(() => compile(invalid)).toThrow();
});

it("interrupts graph preparation without publishing partial findings", () => {
  const invalid = document(["r"], []);
  invalid.clauses[0]!.relational.stages[2] = { name: "closure", reachability: {
    roots: "WITH RECURSIVE forever(id) AS (SELECT 1 UNION ALL SELECT id+1 FROM forever) SELECT f.__path,id FROM functions f,forever",
    edges: "SELECT __path,src,dst FROM links",
  } };
  const actual = evaluate(compile(invalid));
  expect(actual.violations).toEqual([]);
  expect(actual.failures).toEqual([expect.objectContaining({ code: "relational-failed", message: expect.stringContaining("interrupted") })]);
});

it("ignores null roots and edges and preserves isolated reflexive pairs", () => {
  const doc = document(["isolated"], []);
  doc.clauses[0]!.relational.stages[2] = { name: "closure", reachability: {
    roots: "SELECT __path,id FROM requested UNION SELECT __path,NULL FROM requested",
    edges: "SELECT __path,NULL AS src,id AS dst FROM requested UNION SELECT __path,id,NULL FROM requested",
  } };
  const actual = evaluate(compile(doc));
  expect(actual.failures).toEqual([]);
  expect(actual.violations).toEqual(evaluate(compile(document(["isolated"], []))).violations);
  expect(actual.violations).toHaveLength(2);
});
