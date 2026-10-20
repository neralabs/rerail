"""Execute generated shared SQL directly with SQLite before testing the native engine."""
import copy
from pathlib import Path
import sqlite3
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts" / "dsl"))
from data_layer_six.ast_relations import grouped_relations
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
