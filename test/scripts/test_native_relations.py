"""Differential ladders for native row access, tree traversal, and finite-set inference."""
import json
import os
import random
import sqlite3
import unittest

EXTENSION = os.environ.get("RERAIL_REACH_EXTENSION")


def connect():
    db = sqlite3.connect(":memory:")
    db.enable_load_extension(True)
    db.load_extension(EXTENSION)
    db.enable_load_extension(False)
    db.set_progress_handler(lambda: 0, 1000)
    return db


def load_rows(db, name, columns, rows, indexes):
    db.execute("SELECT native_rows_load(?,?,?,?)", (name, json.dumps(columns), json.dumps(rows), json.dumps(indexes))).fetchone()


@unittest.skipUnless(EXTENSION, "Set RERAIL_REACH_EXTENSION to the compiled extension")
class NativeRelations(unittest.TestCase):
    def test_rows_match_sqlite_types_nulls_duplicates_and_collations(self):
        db = connect()
        rows = [[1, "ä", None], [1.0, "Ä", "dup"], ["1", "a", "text"], [None, "A", "null"], [-0.0, "a", "zero"]]
        load_rows(db, "facts", ["id", "name", "text"], rows, [0, 1])
        db.execute("CREATE TABLE reference(id,name,text)")
        db.executemany("INSERT INTO reference VALUES(?,?,?)", rows)
        for where in ["1", "id=1", "id='1'", "id IN (1,'1',1,NULL)", "id IS NULL", "id=0", "name='a' COLLATE NOCASE", "name IN ('a','Ä')", "id>=1"]:
            self.assertEqual(db.execute("SELECT * FROM facts WHERE " + where).fetchall(), db.execute("SELECT * FROM reference WHERE " + where).fetchall(), where)
        db.close()

    def test_rows_batch_requests_and_only_requested_columns(self):
        db = connect()
        load_rows(db, "facts", ["id", "text"], [[i, "x" * 1000] for i in range(1000)], [0])
        self.assertEqual(db.execute("SELECT id FROM facts WHERE id IN (1,2,2,3)").fetchall(), [(1,), (2,), (3,)])
        stats = json.loads(db.execute("SELECT native_rows_stats('facts')").fetchone()[0])
        self.assertEqual(stats["batches"], 1)
        self.assertEqual(stats["candidates"], 3)
        # SQLite rechecks the predicate as well as projecting id. Text stays unread.
        self.assertLessEqual(stats["columns"], 6)
        db.close()

    def test_rows_self_join_reverse_lookup_and_snapshot_isolation(self):
        db = connect()
        load_rows(db, "facts", ["src", "dst"], [[1, 2], [1, 3], [3, 4], [3, 4]], [0, 1])
        self.assertEqual(db.execute("SELECT a.src,b.dst FROM facts a JOIN facts b ON a.dst=b.src WHERE a.src=1").fetchall(), [(1, 4), (1, 4)])
        other = connect()
        load_rows(other, "facts", ["src", "dst"], [[9, 10]], [0, 1])
        self.assertEqual(other.execute("SELECT * FROM facts").fetchall(), [(9, 10)])
        self.assertEqual(db.execute("SELECT count(*) FROM facts").fetchone()[0], 4)
        db.close(); other.close()

    def test_tree_functions_and_batched_table_match_recursive_union(self):
        db = connect()
        nodes = [10, 20, 30, 40, 50, 60]
        edges = [[10, 30], [10, 20], [20, 40], [20, 40], [50, 60]]
        handle = db.execute("SELECT tree_load(?,?,100000,1000)", (json.dumps(nodes), json.dumps(edges))).fetchone()[0]
        db.executescript("CREATE TABLE nodes(id);CREATE TABLE edges(src,dst)")
        db.executemany("INSERT INTO nodes VALUES(?)", [(n,) for n in nodes]); db.executemany("INSERT INTO edges VALUES(?,?)", edges)
        truth = db.execute("WITH RECURSIVE walk(root,node) AS (SELECT id,id FROM nodes UNION SELECT w.root,e.dst FROM walk w JOIN edges e ON e.src=w.node) SELECT root,node FROM walk ORDER BY 1,2").fetchall()
        actual = db.execute("SELECT root,node FROM reachability WHERE graph=? AND root IN (SELECT id FROM nodes) ORDER BY 1,2", (handle,)).fetchall()
        self.assertEqual(actual, truth)
        for root in nodes + [999]:
            for node in nodes + [999]:
                self.assertEqual(db.execute("SELECT is_reachable(?,?,?)", (handle, root, node)).fetchone()[0], int((root, node) in truth))
        self.assertEqual(db.execute("SELECT root,node FROM reachability WHERE graph=? AND node=40 ORDER BY root", (handle,)).fetchall(), [(10, 40), (20, 40), (40, 40)])
        db.close()

    def test_tree_rejects_cycles_and_multiple_parents(self):
        db = connect()
        for edges in [[[1, 2], [2, 1]], [[1, 3], [2, 3]], [[1, 1]]]:
            with self.assertRaises(sqlite3.OperationalError):
                db.execute("SELECT tree_load('[1,2,3]',?,10000,1000)", (json.dumps(edges),)).fetchone()
        db.close()

    def flow(self, nodes, edges, seeds, kills=(), budget=100000, eager=False):
        db = connect()
        db.execute("SELECT flow_load('inferred',?,?,?,?,?,1000,?)", tuple(json.dumps(x) for x in [nodes, edges, seeds, list(kills)]) + (budget, eager)).fetchone()
        db.executescript("CREATE TABLE edges(src,dst);CREATE TABLE seeds(node,label);CREATE TABLE kills(node,label)")
        db.executemany("INSERT INTO edges VALUES(?,?)", edges); db.executemany("INSERT INTO seeds VALUES(?,?)", seeds); db.executemany("INSERT INTO kills VALUES(?,?)", kills)
        truth = db.execute("""WITH RECURSIVE inferred(node,label) AS (
          SELECT node,label FROM seeds UNION SELECT e.dst,i.label FROM inferred i JOIN edges e ON e.src=i.node
          WHERE NOT EXISTS(SELECT 1 FROM kills k WHERE k.node=e.dst AND k.label=i.label)
             OR EXISTS(SELECT 1 FROM seeds s WHERE s.node=e.dst AND s.label=i.label))
          SELECT node,label FROM inferred ORDER BY 1,2""").fetchall()
        return db, truth

    def test_inference_is_lazy_shared_and_matches_sql_fixed_point(self):
        db, truth = self.flow([1, 2, 3, 4], [[1, 2], [2, 3], [3, 1], [3, 4]], [[1, 9], [2, 10]], [[3, 9]])
        self.assertEqual(json.loads(db.execute("SELECT flow_stats('inferred')").fetchone()[0])["builds"], 0)
        self.assertEqual(db.execute("SELECT * FROM inferred ORDER BY 1,2").fetchall(), truth)
        for node in range(1, 6):
            for label in [9, 10, 999]:
                self.assertEqual(db.execute("SELECT flow_has('inferred',?,?)", (node, label)).fetchone()[0], int((node, label) in truth))
        self.assertEqual(db.execute("SELECT * FROM inferred WHERE node IN (1,2,2) ORDER BY 1,2").fetchall(), [p for p in truth if p[0] in (1, 2)])
        self.assertEqual(json.loads(db.execute("SELECT flow_stats('inferred')").fetchone()[0])["builds"], 1)
        db.close()

    def test_random_fixed_points_and_multiple_bitset_words(self):
        rng = random.Random(173)
        for _ in range(12):
            edges = [[rng.randrange(15), rng.randrange(15)] for _ in range(40)]
            seeds = [[rng.randrange(15), rng.randrange(140)] for _ in range(160)]
            kills = [[rng.randrange(15), rng.randrange(140)] for _ in range(20)]
            db, truth = self.flow(list(range(15)), edges, seeds, kills, budget=1000000)
            self.assertEqual(db.execute("SELECT * FROM inferred ORDER BY 1,2").fetchall(), truth)
            db.close()

    def test_inference_failure_never_publishes_partial_results(self):
        db, _ = self.flow([1, 2, 3], [[1, 2], [2, 3]], [[1, 7]], budget=1)
        for sql in ["SELECT * FROM inferred", "SELECT flow_has('inferred',3,7)"]:
            with self.assertRaises(sqlite3.OperationalError): db.execute(sql).fetchall()
        stats = json.loads(db.execute("SELECT flow_stats('inferred')").fetchone()[0])
        self.assertEqual(stats["builds"], 1); self.assertFalse(stats["complete"])
        db.close()


if __name__ == "__main__":
    unittest.main()
