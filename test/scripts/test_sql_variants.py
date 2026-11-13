"""Check graph rewrites with SQLite, including non-tree counterexamples."""
import copy
import importlib.util
from pathlib import Path
import sqlite3
import unittest

spec = importlib.util.spec_from_file_location("sql_variants", Path(__file__).resolve().parents[2] / "scripts/profiling/sql_variants.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def plan():
    return {"partitionBy": "file", "inputs": {}, "stages": [{"name": "descendants", "query": "WITH RECURSIVE walk(__path,root,node) AS (SELECT __path,id,id FROM nodes UNION SELECT w.__path,w.root,e.dst FROM walk w JOIN edges e ON e.__path=w.__path AND e.src=w.node) SELECT * FROM walk", "indexes": [["__path", "root", "node"], ["__path", "node", "root"]]}], "query": "SELECT __path,root,node FROM descendants ORDER BY root,node"}


def execute(document, edges, path="a.py"):
    db = sqlite3.connect(":memory:")
    db.executescript("CREATE TABLE nodes (__path,id); CREATE TABLE edges (__path,src,dst)")
    db.executemany("INSERT INTO nodes VALUES (?,?)", [(path, name) for name in ["a", "b", "c", "d"]])
    db.executemany("INSERT INTO edges VALUES (?,?,?)", [(path, a, b) for a,b in edges])
    db.set_progress_handler(lambda: 1, 100_000)
    for stage in document["stages"]:
        db.execute(f'CREATE TEMP TABLE "{stage["name"]}" AS {stage["query"]}')
        for i, fields in enumerate(stage["indexes"]):
            db.execute(f'CREATE INDEX "idx_{stage["name"]}_{i}" ON "{stage["name"]}" ({",".join(fields)})')
    rows = db.execute(document["query"]).fetchall()
    db.close()
    return rows


class VariantTests(unittest.TestCase):
    def test_scope_guard_skips_work_and_preserves_oracle_entrypoints(self):
        stages = [{"name":"nodes","query":"SELECT 1 AS value"},
                  {"name":"expensive","query":"SELECT tick(value) FROM nodes"},
                  {"name":"files","query":"SELECT 0 AS primary_file,0 AS fallback_file"}]
        reference = {"clauses":[{"id":f"quality-contracts-{i}","relational":{
            "query":"_check_compile_direct_io" if i == 1 else "secondary",
            "stages":stages if i == 1 else [stages[0],stages[2]]}} for i in [1,2,3]]}
        document = {"stages":copy.deepcopy(stages)}
        self.assertEqual(module.scope_stages(document, reference), ["expensive"])
        self.assertEqual([stage["name"] for stage in document["stages"]], ["nodes","files","expensive"])
        query = document["stages"][2]["query"]
        def resolve(value, entry, oracle):
            if isinstance(value,str): return value
            key = entry if value["$option"] == "entryPoint" else str(oracle).lower()
            return resolve(value["cases"].get(key,value["default"]),entry,oracle)
        calls = []; db = sqlite3.connect(":memory:")
        db.create_function("tick",1,lambda value:calls.append(value) or value)
        db.executescript("CREATE TABLE nodes (value); INSERT INTO nodes VALUES(1); CREATE TABLE files(primary_file,fallback_file); INSERT INTO files VALUES(0,0)")
        self.assertEqual(db.execute(resolve(query,"all",False)).fetchall(), [])
        self.assertEqual(calls, [])
        self.assertEqual(db.execute(resolve(query,"all",True)).fetchall(), [(1,)])
        self.assertEqual(db.execute(resolve(query,"_check_compile_direct_io",False)).fetchall(), [(1,)])
        db.execute("UPDATE files SET fallback_file=1")
        self.assertEqual(db.execute(resolve(query,"all",False)).fetchall(), [(1,)])
        db.close()

    def test_path_and_integer_rewrites_preserve_forests_diamonds_cycles_and_dangling_edges(self):
        graphs = [[("a","b"),("b","c")], [("a","b"),("a","c"),("b","d"),("c","d")],
                  [("a","b"),("b","a")], [("a","outside"),("outside","b")]]
        for graph in graphs:
            for path in ["a.py", "other/ü.py"]:
                expected = execute(plan(), graph, path)
                for name in ["no-path", "compact-keys", "compact-no-path"]:
                    document = plan(); module.closure_variant(document, name)
                    self.assertEqual(execute(document, graph, path), expected, (name, graph, path))

    def test_union_all_requires_a_unique_path_forest(self):
        document = plan(); module.closure_variant(document, "union-all")
        forest = [("a","b"),("b","c")]
        self.assertEqual(execute(document, forest), execute(plan(), forest))
        diamond = [("a","b"),("a","c"),("b","d"),("c","d")]
        self.assertNotEqual(execute(document, diamond), execute(plan(), diamond))
        with self.assertRaises(sqlite3.OperationalError): execute(document, [("a","b"),("b","a")])

    def test_compact_rewrites_preserve_an_explicit_seed_restriction(self):
        graph = [("a", "b"), ("b", "c"), ("c", "d")]
        original = plan()
        original["stages"][0]["query"] = original["stages"][0]["query"].replace("FROM nodes UNION SELECT", "FROM (SELECT * FROM nodes WHERE id='b') UNION SELECT")
        for name in ["no-path", "compact-keys", "compact-no-path"]:
            document = copy.deepcopy(original); module.closure_variant(document, name)
            self.assertEqual(execute(document, graph), execute(original, graph))
            module.closure_variant(document, "union-all")
            self.assertEqual(execute(document, graph), execute(original, graph))

    def test_root_restriction_keeps_comprehensions_and_unfielded_comparison_operands(self):
        def run(document):
            db = sqlite3.connect(":memory:")
            db.executescript("CREATE TABLE nodes (__path,id,kind,parent); CREATE TABLE edges (__path,src,dst)")
            nodes = [("a.py", "comp", "list_comprehension", None), ("a.py", "loop", "for_in_clause", "comp"),
                     ("a.py", "cmp", "comparison_operator", None), ("a.py", "operand", "attribute", "cmp"),
                     ("a.py", "name", "identifier", "operand")]
            db.executemany("INSERT INTO nodes VALUES (?,?,?,?)", nodes)
            db.executemany("INSERT INTO edges VALUES (?,?,?)", [(path, parent, node) for path,node,_,parent in nodes if parent])
            db.execute("CREATE TEMP TABLE descendants AS " + document["stages"][0]["query"])
            rows = db.execute("SELECT root,node FROM descendants WHERE root IN ('comp','operand') ORDER BY root,node").fetchall()
            db.close()
            return rows
        document = plan(); module.closure_variant(document, "roots")
        self.assertEqual(run(document), run(plan()))
        self.assertEqual(len(run(document)), 4)

    def test_optional_field_pruning_keeps_projected_captures(self):
        optional = lambda name: {"any": [{"has": {"field":name.lower(),"pattern":"$"+name}}, {"pattern":"$_"}]}
        relation = {"evidence":{"project":{"name":"capture.NAME"},"query":{"ast":{"rule":{"all":[optional("NAME"), optional("BODY")]}}}}}
        document = {"inputs":{"syntax_0":relation}};before = copy.deepcopy(document)
        self.assertEqual(module.narrow_selectors(document), 1)
        self.assertEqual(len(document["inputs"]["syntax_0"]["evidence"]["query"]["ast"]["rule"]["all"]), 1)
        self.assertEqual(before["inputs"]["syntax_0"]["evidence"]["project"], relation["evidence"]["project"])

    def test_pruning_does_not_treat_an_option_case_named_all_as_an_ast_list(self):
        relation = {"evidence":{"project":{"name":"capture.NAME"},"query":{"ast":{"rule":{"all":[
            {"$option":"entryPoint","cases":{"all":{"kind":"identifier"}},"default":{"kind":"identifier"}},
            {"any":[{"has":{"field":"body","pattern":"$BODY"}},{"pattern":"$_"}]},
        ]}}}}}
        module.narrow_selectors({"inputs":{"syntax_0":relation}})
        self.assertEqual(relation["evidence"]["query"]["ast"]["rule"]["all"], [
            {"$option":"entryPoint","cases":{"all":{"kind":"identifier"}},"default":{"kind":"identifier"}},
        ])


if __name__ == "__main__": unittest.main()
