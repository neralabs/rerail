"""Execute generated shared SQL directly with SQLite before testing the native engine."""
import copy
import json
from pathlib import Path
import sqlite3
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts" / "dsl"))
from data_layer_six.ast_relations import grouped_relations, relational, defer_node_text, typed_input_partitions, narrow_deferred_windows, shared_syntax_relations
from data_layer_six.common import E, K, S


def resolve(value, entry, oracle=False):
    if isinstance(value, dict):
        if "$option" in value:
            key = entry if value["$option"] == "entryPoint" else str(oracle).lower()
            return resolve(value.get("cases", {}).get(key, value.get("default")), entry, oracle)
        return {k: resolve(v, entry, oracle) for k, v in value.items()}
    if isinstance(value, list):
        return [resolve(v, entry, oracle) for v in value]
    return value


class SharedSqlTests(unittest.TestCase):
    def test_state_frontends_use_native_syntax_and_explicit_empty_admission(self):
        path = Path(__file__).resolve().parents[2] / 'packs/data-layer/candidates/rules/data-layer-serverplan-state-tripwires.json'
        policy = json.loads(path.read_text())
        expected = {'state-signatures-1': (1, 18, 'file'), 'state-flow-1': (5, 25, 'file'),
                    'state-projection-recovery-2': (4, 7, 'file'), 'state-flow-project-1': (5, 27, 'repository')}
        for clause in policy['clauses']:
            if clause['id'] not in expected:
                continue
            plan = clause['relational']
            inputs, stages, partition = expected[clause['id']]
            self.assertEqual(len(plan['inputs']), inputs, clause['id'])
            self.assertEqual(len(plan['stages']), stages, clause['id'])
            self.assertEqual(plan['partitionBy'], partition)
            self.assertEqual(plan['requireNonemptyInput'], 'syntax_gate')
            self.assertEqual(plan['syntax']['gate'], 'syntax_gate')
            self.assertFalse({'nodes', 'edges', 'ownership', 'syntax_rows_0', 'syntax_rows_1'} & {s['name'] for s in plan['stages']})
            descendants = next(s for s in plan['stages'] if s['name'] == 'descendants')
            self.assertEqual(descendants['reachability']['syntax'], 'nodes')
            self.assertIn('expand', descendants['reachability']['roots'])
            if 'arguments' in {s['name'] for s in plan['stages']}:
                arguments = next(s for s in plan['stages'] if s['name'] == 'arguments')
                self.assertIn('n.text', arguments['query'])

    def test_conditional_reachability_limits_graph_domains_without_losing_isolated_roots(self):
        from data_layer_six.ast_relations import batched_reachability
        clause = {'id': 'quality-conditional-1', 'relational': {'stages': [S('descendants', "WITH RECURSIVE roots(__path,id) AS (SELECT __path,id FROM nodes WHERE kind='type'), walk(__path,root,node) AS (SELECT __path,id,id FROM roots) SELECT * FROM walk")]}}
        batched_reachability(clause, forest=True)
        declared, reach = clause['relational']['stages']
        db = sqlite3.connect(':memory:')
        db.executescript('CREATE TABLE nodes (__path,id,kind); CREATE TABLE edges (__path,src,dst); CREATE INDEX forward ON edges (__path,src,dst);')
        db.executemany('INSERT INTO nodes VALUES (?,?,?)', [('a.py','r','type'),('b.py','r','identifier'),('c.py','isolated','type')])
        db.executemany('INSERT INTO edges VALUES (?,?,?)', [('a.py','r','child'),('a.py','unrelated','tail'),('b.py','r','unused')])
        db.execute('CREATE TABLE descendant_roots AS ' + declared['query'])
        self.assertEqual(db.execute(reach['reachability']['edges']).fetchall(), [('a.py','r','child')])
        closure = 'WITH RECURSIVE walk(__path,root,node) AS (SELECT __path,id,id FROM descendant_roots UNION SELECT w.__path,w.root,e.dst FROM walk w JOIN ({}) e ON e.__path=w.__path AND e.src=w.node) SELECT * FROM walk ORDER BY __path,root,node'
        expected = db.execute(closure.format('SELECT * FROM edges')).fetchall()
        self.assertEqual(db.execute(closure.format(reach['reachability']['edges'])).fetchall(), expected)
        self.assertIn(('c.py','isolated','isolated'), expected)
        db.close()

    def test_conditional_declaration_keys_preserve_shadowing_and_class_ownership(self):
        policy = json.loads((Path(__file__).resolve().parents[2] / 'packs/data-layer/candidates/rules/data-layer-serverplan-quality.json').read_text())
        clause = next(c for c in policy['clauses'] if c.get('id') == 'quality-conditional-1')
        stages = {s['name']: s['query'] for s in clause['relational']['stages'] if 'query' in s}
        db = sqlite3.connect(':memory:')
        db.executescript('CREATE TABLE classes (__path,id,name,__start); CREATE TABLE functions (__path,id,name,__start); CREATE TABLE ownership (__path,id,cls); CREATE TABLE module_names (__path,qualified);')
        db.executemany('INSERT INTO classes VALUES (?,?,?,?)', [('a.py','c1','C',1),('a.py','c2','C',10),('a.py','d1','D',20)])
        db.executemany('INSERT INTO functions VALUES (?,?,?,?)', [('a.py','m1','run',11),('a.py','m2','run',12),('a.py','m3','run',21),('a.py','g1','g',30),('a.py','g2','g',40)])
        db.executemany('INSERT INTO ownership VALUES (?,?,?)', [('a.py','m1','c2'),('a.py','m2','c2'),('a.py','m3','d1'),('a.py','g1',None),('a.py','g2',None)])
        db.execute("INSERT INTO module_names VALUES ('a.py','pkg')")
        for name in ['conditional_classes','conditional_methods','conditional_functions']:
            db.execute('CREATE TABLE ' + name + ' AS ' + stages[name])
        self.assertEqual(db.execute('SELECT id,qualified_name FROM conditional_classes ORDER BY id').fetchall(), [('c2','pkg.C'),('d1','pkg.D')])
        self.assertEqual(db.execute('SELECT id,cls FROM conditional_methods ORDER BY id').fetchall(), [('m2','c2'),('m3','d1')])
        self.assertEqual(db.execute('SELECT id,qualified_name FROM conditional_functions').fetchall(), [('g2','pkg.g')])
        db.close()

    def test_ast_ordinals_sort_narrow_rows_without_changing_node_values(self):
        from data_layer_six.ast_relations import ast_stages, narrow_ast_ordinals
        original = {'relational': {'stages': ast_stages()[:1]}}
        candidate = copy.deepcopy(original)
        narrow_ast_ordinals(candidate)
        columns = ['__path', '__start', '__end', 'kind', 'parent_start', 'parent_end', 'parent_kind', 'ancestor_start', 'ancestor_end', 'ancestor_kind']
        columns += ['payload_' + str(i) for i in range(40)]
        rows = [('a.py', i, i + 1, 'identifier', i // 10, 10000, 'block', i // 10, 10000, 'block', *['payload'] * 40) for i in range(1000)]
        def evaluate(clause):
            db = sqlite3.connect(':memory:')
            db.execute('CREATE TABLE raw_nodes (' + ','.join(columns) + ')')
            db.executemany('INSERT INTO raw_nodes VALUES (' + ','.join('?' for _ in columns) + ')', rows)
            steps = [0]
            def count():
                steps[0] += 1
                return 0
            db.set_progress_handler(count, 1)
            for stage in clause['relational']['stages']:
                db.execute('CREATE TABLE "' + stage['name'] + '" AS ' + stage['query'])
                for index, fields in enumerate(stage.get('indexes', [])):
                    db.execute('CREATE INDEX "' + stage['name'] + str(index) + '" ON "' + stage['name'] + '" (' + ','.join(fields) + ')')
            db.set_progress_handler(None, 0)
            result = db.execute('SELECT * FROM nodes ORDER BY __path,__start,__end,kind').fetchall()
            db.close()
            return result, steps[0]
        expected, old_steps = evaluate(original)
        actual, new_steps = evaluate(candidate)
        self.assertEqual(actual, expected)
        self.assertLess(new_steps, old_steps)

    def test_shared_syntax_eliminates_reconstruction_and_preserves_gates(self):
        clause = relational("quality-contracts-1", [S("calls", "SELECT n.*,f.id AS callee FROM nodes n JOIN nodes f ON f.id=n.function_kind||':'||n.function_start||':'||n.function_end"), S("descendants", "unused")], "SELECT * FROM calls")
        clause["relational"]["partitionBy"] = "file"
        clause["relational"]["inputs"]["raw_nodes"]["evidence"]["query"]["ast"]["rule"]["all"].append({"filePath": {"regex": "^eligible/"}})
        defer_node_text(clause)
        self.assertTrue(shared_syntax_relations(clause))
        plan = clause["relational"]
        self.assertEqual(list(plan["inputs"]), ["syntax_gate"])
        self.assertFalse({"nodes", "edges", "ownership"} & {s["name"] for s in plan["stages"]})
        self.assertIn("n.function_id", next(s for s in plan["stages"] if s["name"]=="calls")["query"])
        self.assertNotIn("||':'||", next(s for s in plan["stages"] if s["name"]=="calls")["query"])
        self.assertEqual(next(s for s in plan["stages"] if s["name"]=="descendants")["reachability"]["syntax"], "nodes")
        self.assertIn("type_id", next(s for s in plan["stages"] if s["name"]=="descendants")["reachability"]["roots"])
        gate = plan["inputs"]["syntax_gate"]["evidence"]["query"]["ast"]["rule"]
        self.assertIn({"filePath": {"regex": "^eligible/"}}, gate["all"])
        self.assertIn({"kind": "module"}, gate["all"])

    def test_global_declaration_projection_does_not_become_whole_file_syntax(self):
        clause = relational("quality-conditional-1", [], "SELECT * FROM nodes")
        selector = {"any": [{"kind": "module"}, {"kind": "class_definition"}, {"kind": "function_definition"}]}
        clause["relational"]["inputs"]["raw_nodes"]["evidence"]["query"]["ast"]["rule"]["all"].append(selector)
        original = copy.deepcopy(clause)
        self.assertFalse(shared_syntax_relations(clause))
        self.assertEqual(clause, original)
        typed_input_partitions(clause)
        self.assertNotIn("syntax", clause["relational"])
        def contains_selector(value):
            if value == selector:
                return True
            children = value.values() if isinstance(value, dict) else value if isinstance(value, list) else []
            return any(contains_selector(child) for child in children)
        for relation in clause["relational"]["inputs"].values():
            self.assertTrue(contains_selector(relation["evidence"]["query"]["ast"]["rule"]))

    def test_window_inputs_do_not_compute_unused_deferred_text(self):
        stage = S("last_methods", "SELECT * FROM (SELECT *,row_number() OVER(PARTITION BY __path,cls,name ORDER BY __start DESC) AS chosen FROM methods) WHERE chosen=1")
        clause = relational("quality-contracts-1", [stage], "SELECT * FROM last_methods")
        defer_node_text(clause)
        original = stage["query"]
        narrow_deferred_windows(clause)
        metadata = ["__path", "__start", "__end", *clause["relational"]["inputs"]["raw_nodes"]["evidence"]["project"], "id", "parent", "edge_parent", "ordinal", "name", "cls", "class_name"]
        db = sqlite3.connect(":memory:")
        calls = []
        db.create_function("tick", 1, lambda value: calls.append(value) or "unused payload")
        db.execute("CREATE TABLE stored(" + ",".join(metadata) + ")")
        for i in range(2):
            values = {"__path": "a.py", "__start": i, "name": "read", "cls": "c", "class_name": "C"}
            db.execute("INSERT INTO stored VALUES(" + ",".join("?" for _ in metadata) + ")", [values.get(name) for name in metadata])
        db.execute("CREATE VIEW methods AS SELECT *,tick(__path) AS text FROM stored")
        before = db.execute("SELECT __path,__start,name FROM (" + original + ")").fetchall()
        self.assertTrue(calls)
        calls.clear()
        after = db.execute("SELECT __path,__start,name FROM (" + stage["query"] + ")").fetchall()
        self.assertEqual(after, before)
        self.assertEqual(calls, [])
        db.close()

    def test_deferred_producers_remove_eager_text_before_typed_extraction(self):
        clause = relational("quality-contracts-1", [], "SELECT * FROM functions")
        defer_node_text(clause)
        typed_input_partitions(clause)
        plan = clause["relational"]
        self.assertTrue(all("text" not in r["evidence"]["project"] for r in plan["inputs"].values()))
        self.assertEqual(next(s for s in plan["stages"] if s["name"] == "nodes")["deferred"], {"text": "source_text(__path,__start,__end)"})
        self.assertEqual(next(s for s in plan["stages"] if s["name"] == "functions")["deferred"], {"text": "source_text(__path,__start,__end)"})
        conditional = relational("quality-conditional-1", [], "SELECT * FROM functions")
        before = copy.deepcopy(conditional)
        defer_node_text(conditional)
        self.assertEqual(conditional, before)

    def generate(self, shared, groups=None):
        # Oversize combined SELECTs force the old generator to split consumers.
        padding = " /* " + "x" * 8500 + " */"
        queries = [f"SELECT __path AS path,__start AS start_byte,__end AS end_byte,'{name}' AS detector,'hit' AS issue,name AS identity FROM prepared WHERE name='{name}'" + padding for name in ["read", "write"]]
        stages = [S("facts", "SELECT __path,__start,__end,tick(name) AS name FROM raw_fact", ("name",)),
                  S("prepared", "SELECT * FROM facts", ("name",))]
        rows = [((name,), query, query) for name, query in zip(["read", "write"], queries)]
        return grouped_relations("contracts", stages, rows, ["all"],
                                 {"raw_fact": E(K("function_definition"), {"name": "capture.NAME"})},
                                 row_groups={q: tag for q, tag in zip(queries, groups)} if groups else None,
                                 shared_sql=shared)

    def execute(self, clauses, entry="all", names=("read", "write")):
        rows, calls, materializations = [], [], []
        for clause in clauses:
            plan = resolve(clause["relational"], entry)
            db = sqlite3.connect(":memory:")
            db.create_function("tick", 1, lambda value: calls.append(value) or value)
            db.execute("CREATE TABLE raw_fact (__path,__start,__end,name)")
            db.executemany("INSERT INTO raw_fact VALUES (?,?,?,?)", [("a.py", i, i + 1, name) for i, name in enumerate(names)])
            for stage in plan["stages"]:
                db.execute(f'CREATE TEMP TABLE "{stage["name"]}" AS {stage["query"]}')
                materializations.append(stage["name"])
                for i, fields in enumerate(stage.get("indexes", [])):
                    db.execute(f'CREATE INDEX "idx_{stage["name"]}_{i}" ON "{stage["name"]}" ({",".join(fields)})')
            rows.extend(db.execute(plan["query"]).fetchall())
            db.close()
        return sorted(rows), calls, materializations

    def test_shares_sql_tables_and_preserves_positive_and_clean_results(self):
        before, after = self.generate(False), self.generate(True)
        self.assertEqual((len(before), len(after)), (2, 1))
        for names in [("read", "write"), ("other",), ()]:
            self.assertEqual(self.execute(before, names=names)[0], self.execute(after, names=names)[0])
        rows, calls, tables = self.execute(after)
        self.assertEqual(len(rows), 2)
        self.assertEqual(calls, ["read", "write"])
        self.assertEqual(tables.count("facts"), 1)
        self.assertEqual(len(self.execute(before)[1]), 4)

    def test_entrypoint_selection_and_repeated_generation(self):
        shared = self.generate(True)
        original = copy.deepcopy(shared)
        for entry, count in [("read", 1), ("write", 1), ("unknown", 0)]:
            self.assertEqual(self.execute(shared, entry)[0], self.execute(self.generate(False), entry)[0])
            self.assertEqual(len(self.execute(shared, entry)[0]), count)
        self.assertEqual(shared, original)
        self.assertEqual(shared, self.generate(True))

    def test_keeps_different_source_gates_separate(self):
        self.assertEqual(len(self.generate(True, ["primary", "callback"])), 2)

    def test_sqlite_materialized_cte_shares_a_producer_between_selects(self):
        db = sqlite3.connect(":memory:")
        calls = []
        db.create_function("tick", 1, lambda value: calls.append(value) or value)
        db.execute("CREATE TABLE inputs (name)")
        db.executemany("INSERT INTO inputs VALUES (?)", [("read",), ("write",)])
        sql = "WITH facts AS MATERIALIZED (SELECT tick(name) AS name FROM inputs) SELECT name FROM facts WHERE name='read' UNION ALL SELECT name FROM facts WHERE name='write'"
        self.assertEqual(sorted(db.execute(sql)), [("read",), ("write",)])
        self.assertEqual(calls, ["read", "write"])
        self.assertTrue(any("MATERIALIZE facts" in row[3] for row in db.execute("EXPLAIN QUERY PLAN " + sql)))
        db.close()


if __name__ == "__main__":
    unittest.main()
