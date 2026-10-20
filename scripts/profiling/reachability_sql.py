"""A shared request/result relation for recursive SQL and experimental Rust adapters."""
import copy
import json
from pathlib import Path
import sqlite3
import time

from sql_variants import closure_variant


def quote(value):
    return '"' + value.replace('"', '""') + '"'


def requested_roots(plan, restricted=True):
    """An explicit conservative request producer, validated against real consumers."""
    original = next(s["query"] for s in plan["stages"] if s["name"] == "descendants")
    # The conditional linter already declares its request relation explicitly.
    # Preserve that CTE, including roots that are not declarations or functions.
    if original.startswith("WITH RECURSIVE roots(__path,id) AS ("):
        return original.split(", walk(__path,root,node) AS (", 1)[0] + " SELECT DISTINCT __path,id FROM roots"
    if not restricted:
        return "SELECT DISTINCT __path,id FROM nodes"
    document = copy.deepcopy(plan)
    closure_variant(document, "roots")
    query = next(s["query"] for s in document["stages"] if s["name"] == "descendants")
    seed = query.split(" AS (", 1)[1].split(" UNION SELECT w.", 1)[0]
    return "SELECT DISTINCT __path,id FROM (" + seed + ")"


def connect(extension=None, seconds=90, work=200_000_000, analysis_limit=1000):
    db = sqlite3.connect(":memory:", cached_statements=128)
    db.execute("PRAGMA temp_store=MEMORY")
    if analysis_limit not in (0,1000): raise ValueError("analysis_limit must be 0 or 1000")
    db.execute("PRAGMA analysis_limit=" + str(analysis_limit))
    deadline = time.monotonic() + seconds
    remaining = work
    def progress():
        nonlocal remaining
        remaining -= 1000
        return int(remaining < 0 or time.monotonic() >= deadline)
    db.set_progress_handler(progress, 1000)
    if extension:
        db.enable_load_extension(True)
        try:
            db.load_extension(str(Path(extension).resolve()))
        finally:
            db.enable_load_extension(False)
    return db


def prepare(db, roots):
    # Compact keys are scoped to this immutable SQL snapshot. Include dangling
    # endpoints, because recursive SQL can traverse edges through missing nodes.
    # CTAS gives row_number() no affinity. Joining that column to INTEGER pair
    # keys prevents SQLite from using its integer lookup index in some plans.
    db.execute("CREATE TEMP TABLE reach_keys(__path TEXT NOT NULL,key,compact INTEGER NOT NULL)")
    db.execute("INSERT INTO reach_keys SELECT __path,key,row_number() OVER(ORDER BY __path,key) FROM (SELECT __path,id AS key FROM nodes UNION SELECT __path,src FROM edges UNION SELECT __path,dst FROM edges)")
    db.execute("CREATE UNIQUE INDEX reach_key_text ON reach_keys(__path,key)")
    db.execute("CREATE UNIQUE INDEX reach_key_int ON reach_keys(compact)")
    db.execute("CREATE TEMP TABLE reach_edges AS SELECT a.compact AS src,b.compact AS dst FROM edges e JOIN reach_keys a ON a.__path=e.__path AND a.key=e.src JOIN reach_keys b ON b.__path=e.__path AND b.key=e.dst")
    db.execute("CREATE INDEX reach_edges_forward ON reach_edges(src,dst)")
    db.execute("CREATE INDEX reach_edges_reverse ON reach_edges(dst,src)")
    db.execute("CREATE TEMP TABLE reach_requests(root INTEGER PRIMARY KEY)")
    db.execute("INSERT INTO reach_requests SELECT DISTINCT k.compact FROM (" + roots + ") r JOIN reach_keys k ON k.__path=r.__path AND k.key=r.id")
    db.execute("CREATE TEMP TABLE reach_batch(root INTEGER PRIMARY KEY)")
    db.execute("CREATE TEMP TABLE reach_pairs(root INTEGER,node INTEGER,PRIMARY KEY(root,node)) WITHOUT ROWID")
    for table in ["reach_keys", "reach_edges", "reach_requests"]:
        db.execute("ANALYZE " + table)
    return [row[0] for row in db.execute("SELECT root FROM reach_requests ORDER BY root")]


def tree_intervals(db):
    """Build actual DFS intervals in SQL; reject general graphs before UNION ALL.

    With at most one distinct parent, a component reachable from a root cannot
    contain a cycle. Unreachable cycle components fail the coverage check.
    """
    if db.execute("SELECT 1 FROM reach_edges GROUP BY dst HAVING count(DISTINCT src)>1 LIMIT 1").fetchone():
        return False
    db.execute("""CREATE TEMP TABLE reach_walk AS WITH RECURSIVE walk(node,path) AS (
        SELECT compact,printf('%016x/',compact) FROM reach_keys k
        WHERE NOT EXISTS(SELECT 1 FROM reach_edges e WHERE e.dst=k.compact)
        UNION ALL SELECT e.dst,w.path||printf('%016x/',e.dst)
        FROM walk w JOIN (SELECT DISTINCT src,dst FROM reach_edges) e ON e.src=w.node)
        SELECT * FROM walk""")
    if db.execute("SELECT count(*) FROM reach_walk").fetchone()[0] != db.execute("SELECT count(*) FROM reach_keys").fetchone()[0]:
        db.execute("DROP TABLE reach_walk")
        return False
    db.execute("""CREATE TEMP TABLE reach_intervals AS WITH events AS (
        SELECT node,path||'!' AS path,1 AS entering FROM reach_walk
        UNION ALL SELECT node,path||'~',0 FROM reach_walk),
        ranked AS (SELECT node,entering,row_number() OVER(ORDER BY path) AS position FROM events)
        SELECT node,max(CASE WHEN entering THEN position END) AS enter,
        max(CASE WHEN NOT entering THEN position END) AS leave FROM ranked GROUP BY node""")
    db.execute("DROP TABLE reach_walk")
    db.execute("CREATE UNIQUE INDEX reach_interval_node ON reach_intervals(node)")
    db.execute("CREATE UNIQUE INDEX reach_interval_enter ON reach_intervals(enter)")
    db.execute("ANALYZE reach_intervals")
    return True


def materialize(db, roots, backend="sql", batch_size=256, max_candidates=5_000_000,
                work=200_000_000, milliseconds=90_000, ordered_pairs=False):
    """All adapters implement the same (root,node) pairs and SQL mapping view.

    Only the request producer knows which linter root shapes matter. Adapters
    know directed graphs, integer keys, and root sets; they know no rule logic.
    """
    if not 1 <= batch_size <= 4096:
        raise ValueError("batch_size must be in 1..4096")
    started = time.process_time()
    requests = prepare(db, roots)
    prepared = time.process_time()
    requested_backend = backend
    forest = None
    if backend in ("interval", "interval-direct"):
        forest = tree_intervals(db)
        if not forest:
            backend = "sql"
    handle = None
    if backend in ("scalar", "vtab", "vtab-direct"):
        nodes = [row[0] for row in db.execute("SELECT compact FROM reach_keys")]
        edges = list(db.execute("SELECT src,dst FROM reach_edges"))
        handle = db.execute("SELECT reach_load(?,?,?,?)", (json.dumps(nodes), json.dumps(edges), work, milliseconds)).fetchone()[0]
    loaded = time.process_time()
    statements = 0
    candidates = len(requests) * db.execute("SELECT count(*) FROM reach_keys").fetchone()[0]
    if backend == "scalar" and candidates > max_candidates:
        raise ValueError(f"scalar enumeration needs {candidates} candidate pairs, above cap {max_candidates}")
    if backend == "interval-direct":
        db.execute("CREATE TEMP VIEW reach_live AS SELECT r.root,n.node FROM reach_requests r JOIN reach_intervals a ON a.node=r.root JOIN reach_intervals n ON n.enter>=a.enter AND n.enter<a.leave")
        pairs = "reach_live"
    elif backend == "vtab-direct":
        # The request set remains a table; consumers can bind root or node.
        # Keep membership inside SQL so virtual scans cannot invent extra roots.
        db.execute(f"CREATE TEMP VIEW reach_live AS SELECT v.root,v.node FROM reachability v WHERE v.graph={handle} AND v.root IN (SELECT root FROM reach_requests)")
        pairs = "reach_live"
    else:
        pairs = "reach_pairs"
        for offset in range(0, len(requests), batch_size):
            db.execute("DELETE FROM reach_batch")
            db.executemany("INSERT INTO reach_batch VALUES (?)", [(root,) for root in requests[offset:offset + batch_size]])
            if backend == "sql":
                db.execute("""WITH RECURSIVE walk(root,node) AS (
                    SELECT root,root FROM reach_batch
                    UNION SELECT w.root,e.dst FROM walk w JOIN reach_edges e ON e.src=w.node)
                    INSERT INTO reach_pairs SELECT root,node FROM walk""" + (" ORDER BY root,node" if ordered_pairs else ""))
            elif backend == "vtab":
                db.execute("INSERT INTO reach_pairs SELECT root,node FROM reachability WHERE graph=? AND root IN (SELECT root FROM reach_batch)", (handle,))
            elif backend == "scalar":
                db.execute("INSERT INTO reach_pairs SELECT r.root,k.compact FROM reach_batch r CROSS JOIN reach_keys k WHERE is_reachable(?,r.root,k.compact)", (handle,))
            elif backend == "interval":
                db.execute("INSERT INTO reach_pairs SELECT r.root,n.node FROM reach_batch r JOIN reach_intervals a ON a.node=r.root JOIN reach_intervals n ON n.enter>=a.enter AND n.enter<a.leave")
            else:
                raise ValueError(backend)
            statements += 1
        db.execute("CREATE INDEX reach_pairs_reverse ON reach_pairs(node,root)")
        db.execute("ANALYZE reach_pairs")
    traversed = time.process_time()
    db.execute("CREATE TEMP VIEW descendants AS SELECT a.__path,a.key AS root,b.key AS node FROM " + pairs + " p JOIN reach_keys a ON a.compact=p.root JOIN reach_keys b ON b.compact=p.node AND b.__path=a.__path")
    metrics = {"backend":requested_backend, "effectiveBackend":backend, "forest":forest,
        "orderedPairs":ordered_pairs,
        "requests":len(requests), "batches":statements,
        "candidatePairs":candidates, "prepareCpuMs":(prepared-started)*1000,
        "backendPrepareCpuMs":(loaded-prepared)*1000, "traverseIndexCpuMs":(traversed-loaded)*1000,
        "totalCpuMs":(time.process_time()-started)*1000, "graph":handle}
    if handle is not None:
        metrics["rust"] = json.loads(db.execute("SELECT reach_stats(?)", (handle,)).fetchone()[0])
    return metrics
