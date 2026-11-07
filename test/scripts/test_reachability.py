"""Differential tests for the actual Rust SQLite extension and SQL request batches."""
import json
import os
from pathlib import Path
import random
import sqlite3
import sys
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/profiling"))
from reachability_sql import connect, materialize, requested_roots, prepare

EXTENSION = os.environ.get("RERAIL_REACH_EXTENSION")


def fixture(nodes, edges):
    db = connect(EXTENSION)
    db.executescript("CREATE TABLE nodes(__path,id); CREATE TABLE edges(__path,src,dst)")
    db.executemany("INSERT INTO nodes VALUES('ü.py',?)", [(str(n),) for n in nodes])
    db.executemany("INSERT INTO edges VALUES('ü.py',?,?)", [(str(a),str(b)) for a,b in edges])
    return db


def expected(db, roots):
    return db.execute("""WITH RECURSIVE walk(root,node) AS (
      SELECT id,id FROM nodes WHERE id IN (SELECT value FROM json_each(?))
      UNION SELECT w.root,e.dst FROM walk w JOIN edges e ON e.src=w.node)
      SELECT root,node FROM walk ORDER BY root,node""", (json.dumps([str(root) for root in roots]),)).fetchall()


class SqlPreparationTests(unittest.TestCase):
    def test_typed_integer_identities_keep_indexed_normalization(self):
        db = connect(None, seconds=10, work=300_000)
        db.executescript("CREATE TABLE nodes(__path TEXT,id INTEGER); CREATE TABLE edges(__path TEXT,src INTEGER,dst INTEGER)")
        db.executemany("INSERT INTO nodes VALUES('a.py',?)", [(n,) for n in range(1000)])
        db.executemany("INSERT INTO edges VALUES('a.py',?,?)", [(n,n+1) for n in range(999)])
        roots = prepare(db, "SELECT __path,id FROM nodes WHERE id%100=0")
        self.assertEqual(roots, [n+1 for n in range(0,1000,100)])
        self.assertEqual(db.execute("SELECT typeof(+'001'),+'001'").fetchone(), ('text','001'))
        db.close()


@unittest.skipUnless(EXTENSION, "Set RERAIL_REACH_EXTENSION to the compiled SQLite extension")
class ReachabilityTests(unittest.TestCase):
    def test_forest_diamond_cycle_duplicate_dangling_and_disconnected(self):
        graphs = [[], [(0,1),(1,2)], [(0,1),(0,2),(1,3),(2,3)],
                  [(0,1),(1,0),(1,1),(0,1)], [(0,8),(8,1)]]
        for edges in graphs:
            for backend in ["sql", "scalar", "vtab", "vtab-direct", "interval", "interval-direct"]:
                for batch in [1,7,256]:
                    with self.subTest(edges=edges, backend=backend, batch=batch):
                        db = fixture(range(5), edges)
                        truth = expected(db, [0,2,4])
                        materialize(db, "SELECT __path,id FROM nodes WHERE id IN ('0','2','4')", backend, batch)
                        self.assertEqual(db.execute("SELECT root,node FROM descendants ORDER BY root,node").fetchall(), truth)
                        db.close()

    def test_random_general_graphs_match_recursive_union(self):
        randomizer = random.Random(739)
        for graph in range(30):
            edges = [(randomizer.randrange(18),randomizer.randrange(18)) for _ in range(32)]
            for backend in ["sql", "scalar", "vtab", "vtab-direct", "interval", "interval-direct"]:
                db = fixture(range(16), edges)
                truth = expected(db, [0,3,8,15])
                materialize(db, "SELECT __path,id FROM nodes WHERE id IN ('0','3','8','15')", backend, 7)
                self.assertEqual(db.execute("SELECT root,node FROM descendants ORDER BY root,node").fetchall(), truth, (graph,backend))
                db.close()

    def test_in_is_one_filter_for_the_whole_batch_and_keeps_root_identity(self):
        db = connect(EXTENSION)
        handle = db.execute("SELECT reach_load('[1,2,3,4]','[[1,3],[2,3],[3,4]]',1000,1000)").fetchone()[0]
        rows = db.execute("SELECT root,node FROM reachability WHERE graph=? AND root IN (1,2,2,NULL) ORDER BY root,node", (handle,)).fetchall()
        self.assertEqual(rows, [(1,1),(1,3),(1,4),(2,2),(2,3),(2,4)])
        stats = json.loads(db.execute("SELECT reach_stats(?)", (handle,)).fetchone()[0])
        self.assertEqual((stats["filters"],stats["batchFilters"],stats["requestedRoots"]), (1,1,2))
        db.close()

    def test_reverse_batches_pair_probes_and_correlated_joins(self):
        db = connect(EXTENSION)
        handle = db.execute("SELECT reach_load('[1,2,3,4]','[[1,2],[2,3],[3,1]]',10000,1000)").fetchone()[0]
        self.assertEqual(db.execute("SELECT root,node FROM reachability WHERE graph=? AND node IN (2,4) ORDER BY root,node", (handle,)).fetchall(), [(1,2),(2,2),(3,2),(4,4)])
        self.assertEqual(db.execute("SELECT root,node FROM reachability WHERE graph=? AND root IN (1,4) AND node IN (2,4) ORDER BY root,node", (handle,)).fetchall(), [(1,2),(4,4)])
        db.executescript("CREATE TABLE requests(root); INSERT INTO requests VALUES(1),(4)")
        self.assertEqual(db.execute("SELECT r.root,v.node FROM requests r JOIN reachability v ON v.graph=? AND v.root=r.root ORDER BY r.root,v.node", (handle,)).fetchall(), [(1,1),(1,2),(1,3),(4,4)])
        self.assertEqual(db.execute("SELECT root,node FROM reachability WHERE graph=? AND root IN (SELECT root FROM requests WHERE 0)", (handle,)).fetchall(), [])
        with self.assertRaises(sqlite3.OperationalError):
            db.execute("SELECT * FROM reachability WHERE graph=?", (handle,)).fetchall()
        with self.assertRaises(sqlite3.OperationalError):
            db.execute("SELECT rowid FROM reachability WHERE graph=? AND root=1", (handle,)).fetchall()
        db.close()

    def test_snapshots_and_connections_do_not_share_stale_results(self):
        db = connect(EXTENSION)
        old = db.execute("SELECT reach_load('[1,2]','[[1,2]]',1000,1000)").fetchone()[0]
        new = db.execute("SELECT reach_load('[1,2]','[]',1000,1000)").fetchone()[0]
        self.assertEqual(db.execute("SELECT is_reachable(?,1,2),is_reachable(?,1,2),is_reachable(?,9,9)", (old,new,new)).fetchone(), (1,0,0))
        other = connect(EXTENSION)
        with self.assertRaises(sqlite3.OperationalError): other.execute("SELECT is_reachable(?,1,2)", (old,)).fetchone()
        other.close()
        self.assertEqual(db.execute("SELECT reach_drop(?)", (old,)).fetchone(), (1,))
        with self.assertRaises(sqlite3.OperationalError): db.execute("SELECT is_reachable(?,1,2)", (old,)).fetchone()
        db.close()

    def test_work_budget_aborts_native_traversal_without_caching_partial_results(self):
        for query in ["SELECT is_reachable(?,1,3)", "SELECT * FROM reachability WHERE graph=? AND root=1"]:
            db = connect(EXTENSION)
            # Three nodes + three edges + root lookup consume seven units.
            handle = db.execute("SELECT reach_load('[1,2,3]','[[1,2],[2,3],[3,1]]',7,1000)").fetchone()[0]
            with self.assertRaises(sqlite3.OperationalError): db.execute(query, (handle,)).fetchall()
            stats = json.loads(db.execute("SELECT reach_stats(?)", (handle,)).fetchone()[0])
            self.assertEqual(stats["cacheMembers"], 0)
            db.close()

    def test_bound_target_uses_reverse_scan_instead_of_every_requested_root(self):
        db = connect(EXTENSION)
        handle = db.execute("SELECT reach_load('[1,2,3,4]','[[1,2],[2,3]]',1000,1000)").fetchone()[0]
        rows = db.execute("SELECT root,node FROM reachability WHERE graph=? AND root IN (1,2,3,4) AND node=3 ORDER BY root", (handle,)).fetchall()
        self.assertEqual(rows, [(1,3),(2,3),(3,3)])
        stats = json.loads(db.execute("SELECT reach_stats(?)", (handle,)).fetchone()[0])
        self.assertEqual(stats["traversals"], 1)
        db.close()

    def test_memo_eviction_preserves_complete_results(self):
        db = connect(EXTENSION)
        nodes = list(range(800))
        handle = db.execute("SELECT reach_load(?,?,3000000,10000)", (json.dumps(nodes),json.dumps([(n,n+1) for n in range(799)]))).fetchone()[0]
        count = db.execute("SELECT count(*) FROM reachability WHERE graph=? AND root IN (SELECT value FROM json_each(?))", (handle,json.dumps(nodes))).fetchone()[0]
        self.assertEqual(count, 800*801//2)
        stats = json.loads(db.execute("SELECT reach_stats(?)", (handle,)).fetchone()[0])
        self.assertLessEqual(stats["cacheMembers"],250000)
        self.assertLess(stats["cacheMembers"],count)
        db.close()

    def test_native_deadline_interrupts_a_traversal(self):
        db = connect(EXTENSION)
        nodes = list(range(128))
        edges = [(n,(n+1)%128) for n in nodes]
        handle = db.execute("SELECT reach_load(?,?,10000,1)", (json.dumps(nodes),json.dumps(edges))).fetchone()[0]
        time.sleep(0.01)
        with self.assertRaises(sqlite3.OperationalError):
            db.execute("SELECT is_reachable(?,0,127)", (handle,)).fetchone()
        db.close()

    def test_sql_batch_count_and_reverse_index_are_real(self):
        db = fixture(range(19), [(n,n+1) for n in range(18)])
        truth = expected(db, range(19))
        metrics = materialize(db, "SELECT __path,id FROM nodes", "sql", 7)
        self.assertEqual(metrics["batches"], 3)
        self.assertEqual(db.execute("SELECT root,node FROM descendants ORDER BY root,node").fetchall(), truth)
        plan = db.execute("EXPLAIN QUERY PLAN SELECT root FROM reach_pairs WHERE node=3").fetchall()
        self.assertTrue(any("reach_pairs_reverse" in row[3] for row in plan))
        db.close()

    def test_ordered_sql_batches_preserve_general_graph_pairs(self):
        db = fixture(range(6), [(0,1),(0,2),(1,3),(2,3),(3,0)])
        truth = expected(db, range(6))
        materialize(db, "SELECT __path,id FROM nodes", "sql", 2, ordered_pairs=True)
        self.assertEqual(db.execute("SELECT root,node FROM descendants ORDER BY root,node").fetchall(),truth)
        db.close()

    def test_existing_explicit_root_cte_is_preserved(self):
        plan = {"stages":[{"name":"descendants", "query":"WITH RECURSIVE roots(__path,id) AS (SELECT __path,id FROM nodes WHERE id='1'), walk(__path,root,node) AS (SELECT __path,id,id FROM roots UNION SELECT w.__path,w.root,e.dst FROM walk w JOIN edges e ON e.src=w.node) SELECT * FROM walk"}]}
        db = fixture(range(4), [(0,1),(1,2)])
        self.assertEqual(db.execute(requested_roots(plan)).fetchall(), [('ü.py','1')])
        materialize(db, requested_roots(plan), "sql", 7)
        self.assertEqual(db.execute("SELECT root,node FROM descendants ORDER BY root,node").fetchall(), [('1','1'),('1','2')])
        db.close()

    def test_identical_node_ids_in_different_files_do_not_cross(self):
        for backend in ["sql", "scalar", "vtab", "vtab-direct", "interval", "interval-direct"]:
            db = fixture([1,2], [(1,2)])
            db.executemany("INSERT INTO nodes VALUES('other.py',?)", [('1',),('2',)])
            materialize(db, "SELECT __path,id FROM nodes", backend, 7)
            self.assertEqual(db.execute("SELECT __path,root,node FROM descendants ORDER BY __path,root,node").fetchall(),
                             [('other.py','1','1'),('other.py','2','2'),('ü.py','1','1'),('ü.py','1','2'),('ü.py','2','2')])
            db.close()

    def test_integer_mapping_supports_indexed_joins_for_selective_consumers(self):
        db = fixture(range(2000), [(0,n) for n in range(1,2000)])
        materialize(db, "SELECT __path,id FROM nodes", "sql", 256)
        db.execute("CREATE TABLE methods(__path,id)")
        db.executemany("INSERT INTO methods VALUES('ü.py',?)", [(str(n),) for n in range(1,201)])
        db.execute("ANALYZE methods")
        remaining = 100
        def progress():
            nonlocal remaining
            remaining -= 1
            return int(remaining < 0)
        db.set_progress_handler(progress,1000)
        self.assertEqual(db.execute("SELECT count(*) FROM methods m JOIN descendants d ON d.__path=m.__path AND d.root=m.id").fetchone(), (200,))
        db.set_progress_handler(None,0)
        compact_type = next(row[2] for row in db.execute("PRAGMA table_info(reach_keys)") if row[1] == "compact")
        self.assertEqual(compact_type, "INTEGER")
        db.close()


if __name__ == "__main__":
    if not EXTENSION:
        raise SystemExit("Set RERAIL_REACH_EXTENSION to the compiled SQLite extension")
    unittest.main()
