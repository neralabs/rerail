import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/index.js";

const required = process.env.RERAIL_REQUIRE_PRODUCER_SHAPE === "1";
const engine = new URL("../../../src/engine/index.ts", import.meta.url).href;
const capabilities = new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url).href;

it("retains finite schemas, case-insensitive name collisions, and the SQL authorizer", () => {
  const native = loadNativeBinding(), caps = readFileSync(new URL(capabilities), "utf8");
  const input = { evidence: { as: "file", query: { ast: { language: "python", rule: { kind: "module" } } }, project: { path: "file.path" } } };
  const compile = (query: string, inputs: Record<string, typeof input> = { eligible: input }) => native.CodeRuleDslRuntime.compile(JSON.stringify({
    format: "rerail.policy/v1", id: "schema-validation", revision: 1, mode: "log", clauses: [{ id: "rows",
      relational: { inputs, query, key: ["path"] }, violation: { code: "ROW", message: "Row", anchor: "witness.span" } }],
  }), "json", caps);
  const query = "SELECT __path AS path,__start AS start_byte,__end AS end_byte FROM eligible";
  expect(() => compile(query)).not.toThrow();
  for (const sql of [
    query.replace("__end", "missing"),
    query.replace("eligible", "absent"),
    "SELECT name AS path,0 AS start_byte,0 AS end_byte FROM sqlite_master",
    "SELECT random() AS path,0 AS start_byte,0 AS end_byte FROM eligible",
    "DELETE FROM eligible RETURNING __path AS path,__start AS start_byte,__end AS end_byte",
    query + " WHERE __path=?",
  ]) expect(() => compile(sql), sql).toThrow();
  expect(() => compile(query, { eligible: input, ELIGIBLE: input })).toThrow();
  expect(() => compile(query, { eligible: input, sqlite_hidden: input })).toThrow();
});

it.skipIf(!required)("validates independent input and syntax schemas without executing SQL", () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-compiler-schema-"));
  try {
    const program = `
      import { readFileSync } from 'node:fs';
      import { loadNativeBinding } from ${JSON.stringify(engine)};
      const native=loadNativeBinding(),caps=readFileSync(new URL(${JSON.stringify(capabilities)}),'utf8');
      const clause=i=>({id:'c'+i,relational:{
        inputs:{eligible:{evidence:{as:'file',query:{ast:{language:'python',rule:{kind:'module'}}},project:{path:'file.path'}}}},
        syntax:{name:'nodes',gate:'eligible',fields:['name'],edges:'edges',ownership:'ownership',owners:{fn:{kind:'function_definition'}}},
        query:'SELECT n.__path AS path,n.__start AS start_byte,n.__end AS end_byte,n.id FROM nodes n JOIN ownership o ON o.__path=n.__path AND o.id=n.id JOIN edges e ON e.__path=n.__path AND e.dst=n.id WHERE o.fn IS NOT NULL',key:['id']},
        violation:{code:'NODE',message:'Node',anchor:'witness.span'}});
      native.CodeRuleDslRuntime.compile(JSON.stringify({format:'rerail.policy/v1',id:'schemas',revision:1,mode:'log',clauses:Array.from({length:12},(_,i)=>clause(i))}),'json',caps);
      console.log('compiled');
    `;
    const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
      encoding: "utf8", timeout: 30_000,
      env: { ...process.env, RERAIL_SQL_PROFILE_DIR: directory },
    });
    expect(child.error).toBeUndefined(); expect(child.status, child.stderr).toBe(0);
    expect(child.stdout.trim()).toBe("compiled");
    const catalogues = readdirSync(directory).filter(f => /^profile-.*\.json$/.test(f));
    expect(catalogues.length, "profiling evidence is required").toBeGreaterThan(0);
    const ddl: string[] = [];
    const submitted: string[] = [];
    for (const file of catalogues) {
      const catalogue = JSON.parse(readFileSync(join(directory, file), "utf8"));
      const queries = new Map<number, { sql: string }>(catalogue.queries.map((q: { id: number; sql: string }) => [q.id, q]));
      const events = readFileSync(join(directory, file.replace(/\.json$/, ".jsonl")), "utf8").trim().split("\n").filter(Boolean).map(l => JSON.parse(l));
      expect(events.filter(e => e.kind === "sql")).toHaveLength(catalogue.executionCoverage.completed);
      for (const event of events) if (event.kind === "sql") {
        const sql = queries.get(event.query)!.sql;
        if (event.origin === "engine") submitted.push(sql);
        if (/^\s*CREATE\s+(?:TEMP\s+)?(?:VIRTUAL\s+)?TABLE\b/i.test(sql)) ddl.push(sql);
      }
    }
    expect(ddl, "compilation must register opaque schemas, without executed input/syntax CREATEs").toEqual([]);
    expect(submitted, "prepare-only compilation needs no runtime connection PRAGMAs").toEqual([]);
  } finally { rmSync(directory, { recursive: true, force: true }); }
}, 40_000);
