"""Column-demand and lifetime ladders, including dead SQL references and multiplicity."""
import sqlite3
import sys
from pathlib import Path
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/profiling"))
from native_sql_plan import plan, expose, materialize, release


class DemandPlan(unittest.TestCase):
    def run_program(self, stages, query, rows, optimized):
        schema = ["id", "name", "unused", "text"]
        db = sqlite3.connect(":memory:", cached_statements=0)
        compiled = None
        if optimized:
            compiled = plan(db, {"input": schema}, stages, query)
            db.close(); db = sqlite3.connect(":memory:")
            columns = compiled["columns"]["input"]
            db.execute("CREATE TABLE __physical_input(" + ",".join('"'+c+'"' for c in columns) + ")")
            db.executemany("INSERT INTO __physical_input VALUES(" + ",".join("?" for _ in columns) + ")", [[row[schema.index(c)] for c in columns] for row in rows])
            expose(db, "input", "__physical_input", schema, columns)
        else:
            db.execute("CREATE TABLE input(id,name,unused,text)"); db.executemany("INSERT INTO input VALUES(?,?,?,?)", rows)
        dropped = []
        for i, stage in enumerate(stages):
            if optimized:
                materialize(db, stage["name"], stage["query"], compiled["schemas"][stage["name"]], compiled["columns"][stage["name"]], stage.get("indexes", []))
                dropped.extend(release(db, compiled, i))
            else:
                db.execute(f'CREATE TABLE "{stage["name"]}" AS {stage["query"]}')
        result = db.execute(query).fetchall(); db.close()
        return result, compiled, dropped

    def test_demand_crosses_aliases_expressions_and_dead_schema_references(self):
        stages = [{"name": "a", "query": "SELECT *, upper(name) AS normalized,unused||text AS dead FROM input"}, {"name": "b", "query": "SELECT id,normalized FROM a WHERE id>0"}]
        query = "SELECT * FROM b ORDER BY id"
        rows = [[1, "one", None, "x"], [2, "two", "dead", "y"], [0, "zero", "z", "w"]]
        before, _, _ = self.run_program(stages, query, rows, False)
        after, compiled, dropped = self.run_program(stages, query, rows, True)
        self.assertEqual(before, after)
        self.assertEqual(compiled["columns"]["input"], ["id", "name"])
        self.assertIn("input", dropped); self.assertIn("a", dropped)

    def test_distinct_multiplicity_aggregate_and_null_semantics(self):
        programs = [
            ([{"name": "a", "query": "SELECT DISTINCT id,unused FROM input"}], "SELECT id,count(*) FROM a GROUP BY id ORDER BY id"),
            ([{"name": "a", "query": "SELECT id,sum(unused) AS total FROM input GROUP BY id"}], "SELECT id,total FROM a ORDER BY id"),
            ([{"name": "a", "query": "SELECT id,name FROM input WHERE unused IS NULL"}], "SELECT count(*) FROM a"),
            ([{"name": "a", "query": "SELECT id,row_number() OVER(PARTITION BY name ORDER BY unused) AS ordinal FROM input"}], "SELECT id,ordinal FROM a ORDER BY id,ordinal"),
        ]
        rows = [[1, "a", 3, "unused"], [1, "a", 4, "unused"], [2, "b", None, "unused"], [2, "b", None, "unused"]]
        for stages, query in programs:
            self.assertEqual(self.run_program(stages, query, rows, False)[0], self.run_program(stages, query, rows, True)[0], query)

    def test_recursive_and_self_join_dependencies_remain_live(self):
        stages = [{"name": "a", "query": "SELECT id,unused AS parent FROM input"}, {"name": "b", "query": "WITH RECURSIVE walk(id) AS (SELECT id FROM a WHERE parent IS NULL UNION SELECT a.id FROM a JOIN walk w ON a.parent=w.id) SELECT * FROM walk"}]
        query = "SELECT a.id,b.id FROM a JOIN b ON a.id=b.id ORDER BY a.id"
        rows = [[1, "root", None, "x"], [2, "child", 1, "y"], [3, "other", 3, "z"]]
        before, _, _ = self.run_program(stages, query, rows, False)
        after, compiled, dropped = self.run_program(stages, query, rows, True)
        self.assertEqual(before, after); self.assertEqual(compiled["lastUse"]["a"], 2)
        self.assertNotIn("a", dropped)


if __name__ == "__main__":
    unittest.main()
