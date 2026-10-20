/** Profile-only API: real tape access, structural algorithms, and existing lexical analysis. */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/index.js";
type Row = (string | number | null)[];
type Reply = { rows: Row[]; buildMs: number; queryMs: number; nodes: number; ownerLookupWork: number; ownerIndexEntries: number; stats: { scans: number; candidates: number; columns: number; batches: number } };
type Spike = { query(sql: string): string };
const native = loadNativeBinding() as ReturnType<typeof loadNativeBinding> & { NativeSqlSpike?: new(path: string, source: string, backend: string) => Spike };
const query = (db: Spike, sql: string): Reply => JSON.parse(db.query(sql)) as Reply;
const create = (source: string, backend = "vtab", path = "a.py") => new native.NativeSqlSpike!(path, source, backend);
const source = "x = 1\ndef read(x):\n    def nested():\n        return x\n    return nested()\nvalue = read(x)\n";

describe.skipIf(native.NativeSqlSpike === undefined)("profile native SQL interfaces", () => {
  it("matches materialized tables for node projections, kinds, parents and scalar predicates", () => {
    const direct = create(source), stored = create(source, "materialized");
    const checks = [
      "SELECT __path,id,parent,kind,start_byte,end_byte,text,field,named,fn,cls,top_fn FROM nodes ORDER BY id",
      "SELECT id,text FROM nodes WHERE kind IN ('identifier','function_definition') ORDER BY id",
      "SELECT c.id,c.kind,p.kind FROM nodes c JOIN nodes p ON p.id=c.parent ORDER BY c.id",
      "SELECT a.id,b.id FROM nodes a JOIN nodes b ON b.id=a.parent WHERE a.id IN (2,3,4,5,6) ORDER BY a.id",
      "SELECT id,owner(id),is_ancestor(0,id),node_text(id) FROM nodes WHERE kind='identifier' ORDER BY id",
      "SELECT id FROM nodes WHERE id='2' OR id=3.0 ORDER BY id",
      "SELECT id FROM nodes WHERE kind='IDENTIFIER' COLLATE NOCASE ORDER BY id",
    ];
    for (const sql of checks) expect(query(direct, sql).rows).toEqual(query(stored, sql).rows);
    expect(query(direct, "SELECT is_ancestor(-1,0),owner(-1),node_text(-1)").rows).toEqual([[null, null, null]]);
  });

  it("exposes the same named-node facts as the native matcher", () => {
    const caps = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
    const policy = { format: "rerail.policy/v1", id: "capture", revision: 1, mode: "log", clauses: [{ id: "facts", relational: {
      inputs: { syntax: { evidence: { as: "n", query: { ast: { language: "python", rule: { pattern: "$NODE" } } }, project: { kind: "capture.NODE.kind", text: "capture.NODE" } } } },
      query: "SELECT __path AS path,__start AS start_byte,__end AS end_byte,kind,text FROM syntax", key: ["kind"],
    }, violation: { code: "FACT", message: "Fact", anchor: "witness.span" } }] };
    const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", caps);
    const evaluation = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [{ path: "a.py", source, language: "python" }] }), "report")) as { failures: unknown[]; violations: { variables: { row: { start_byte: number; end_byte: number; kind: string; text: string } } }[] };
    expect(evaluation.failures).toEqual([]);
    const rows = evaluation.violations.map(v => { const r = v.variables.row; return [r.start_byte, r.end_byte, r.kind, r.text]; });
    const actual = query(create(source), "SELECT start_byte,end_byte,kind,text FROM nodes WHERE named=1").rows;
    expect(actual.map(r => JSON.stringify(r)).sort()).toEqual(rows.map(r => JSON.stringify(r)).sort());
  });

  it("batches indexed point requests and avoids unused text projections", () => {
    const db = create(source);
    const result = query(db, "SELECT id FROM nodes WHERE id IN (0,1,1,2)");
    expect(result.rows).toEqual([[0], [1], [2]]);
    expect(result.stats.batches).toBe(1); expect(result.stats.candidates).toBe(3);
    expect(result.stats.columns).toBeLessThanOrEqual(6);
  });

  it("uses owner postings with work proportional to requests plus returned memberships", () => {
    const text = Array.from({ length: 200 }, (_, i) => `def f${i}():\n    return ${i}\n`).join("\n");
    const db = create(text);
    const owners = query(db, "SELECT id FROM nodes WHERE kind='function_definition'").rows;
    const before = query(db, "SELECT 1");
    let memberships = 0;
    for (const [id] of owners) {
      memberships += Number(query(db, `SELECT count(*) FROM nodes WHERE fn=${id}`).rows[0]![0]);
    }
    const after = query(db, "SELECT 1");
    expect(after.ownerLookupWork - before.ownerLookupWork).toBe(owners.length + memberships);
    expect(after.ownerLookupWork).toBeLessThan(after.nodes * 2);
    expect(after.ownerIndexEntries).toBeLessThanOrEqual(after.nodes * 3);
    const truth = query(create(text, "materialized"), "SELECT fn,count(*) FROM nodes WHERE fn IS NOT NULL GROUP BY fn ORDER BY fn").rows;
    expect(query(db, "SELECT fn,count(*) FROM nodes WHERE fn IN (SELECT id FROM nodes WHERE kind='function_definition') GROUP BY fn ORDER BY fn").rows).toEqual(truth);
  });

  it("enumerates descendants and ancestors with the same structural snapshot as point functions", () => {
    const db = create(source);
    const truth = query(db, "WITH RECURSIVE walk(root,node) AS (SELECT id,id FROM nodes WHERE kind='function_definition' UNION SELECT w.root,n.id FROM walk w JOIN nodes n ON n.parent=w.node) SELECT root,node FROM walk ORDER BY 1,2").rows;
    const actual = query(db, "SELECT root,node FROM reachability WHERE graph=tree_handle() AND root IN (SELECT id FROM nodes WHERE kind='function_definition') ORDER BY 1,2").rows;
    expect(actual).toEqual(truth);
    expect(query(db, "SELECT count(*) FROM reachability WHERE graph=tree_handle() AND root=0").rows).toEqual([[query(db, "SELECT count(*) FROM nodes").rows[0]![0]]]);
    expect(query(db, "SELECT is_reachable(tree_handle(),0,2),is_ancestor(0,2)").rows).toEqual([[1, 1]]);
  });

  it("reuses existing lexical analysis for binding lookups and resolved calls", () => {
    const db = create(source);
    const rows = query(db, "SELECT name,reference_start,reference_end,target_start,target_end FROM native_bindings ORDER BY reference_start").rows;
    expect(rows.length).toBeGreaterThan(0);
    expect(query(db, "SELECT b.reference_start,b.target_start,binding_start(b.reference_start,b.reference_end) FROM native_bindings b ORDER BY b.reference_start").rows.every(row => row[1] === row[2])).toBe(true);
    const outer = rows.find(row => row[0] === "x" && row[1] === source.lastIndexOf("x"));
    expect(outer?.[3]).toBe(0);
    const inner = rows.find(row => row[0] === "x" && row[1] === source.indexOf("return x") + 7);
    expect(inner?.[3]).toBe(source.indexOf("(x)") + 1);
    const calls = query(db, "SELECT c.start_byte,b.target_start FROM nodes c JOIN nodes n ON n.parent=c.id AND n.kind='identifier' AND n.start_byte=c.start_byte JOIN native_bindings b ON b.reference_start=n.start_byte AND b.reference_end=n.end_byte WHERE c.kind='call' ORDER BY c.start_byte").rows;
    expect(calls).toEqual([[source.indexOf("nested()", source.indexOf("return nested")), source.indexOf("def nested")], [source.lastIndexOf("read(x)"), source.indexOf("def read")]]);
  });

  it("keeps source revisions separate and supports lazy text with narrow materialization", () => {
    const a = create(source), b = create("def other(): pass\n", "narrow");
    expect(query(a, "SELECT text FROM nodes WHERE kind='function_definition'").rows).toHaveLength(2);
    expect(query(b, "SELECT node_text(id) FROM nodes WHERE kind='function_definition'").rows).toEqual([["def other(): pass"]]);
    expect(() => query(a, "DELETE FROM nodes")).toThrow(/read-only|cannot modify/);
  });
});
