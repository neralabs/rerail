"""Planner statistics, built at ingestion from the facts' own indexes and persisted beside them.

usage: python3 -I stats.py build <facts.sqlite>        all documents, from scratch
       python3 -I stats.py merge <facts.sqlite> <+|-> <doc id>...
       python3 -I stats.py check <facts.sqlite>        incremental merges equal a one-shot build

This follows ast-index-spike (prior-work history index, section 2.6 and Part 3 item 15):
- **Small aggregates, maintained at load and persisted** (`t_kfreq` S178, `efreq` S277, `npost` per lexicon term S229).
  Statistics that lived only in process memory made a reopened store plan differently (S174, S178). Planning here
  previously ran a GROUP BY over every input column on every run.
- **Merged per batch, touching only the keys a batch changes.** A one-shot build and an incremental one are
  identical (DH L8706-8708). A batch is a set of documents and a sign: + adds their contribution, - removes it,
  computed from their rows while the rows are present. Replacing a document is - with the old rows, then + with
  the new.
- **Read by the planner in one statement** (`_prime_corpus_constants`, S259). A fingerprint of the facts' row
  counts is stored, and the planner refuses statistics that don't match it (the spike's attach refuses on a
  fingerprint mismatch).
- **Each aggregate is read off an index the facts already have:**
  - documents are node-id intervals (`stat_docrange`), checked here;
  - per-document rows are rowid or `child_p`/`field_p` range scans;
  - values are grouped within those ranges.

Tables:
- `stat_docrange(d, lo, hi)`: document d's nodes are lo..hi (tape preorder, ast-index-spike S012).
- `stat_val(tbl, col, val, n, docs, sqd)`: postings of each value of a value column (not a node id or offset). `n` is the
  corpus count, `docs` the documents holding it, and `sqd` the sum over documents of the per-document count squared.
- `stat_col(tbl, col, rows, ndv, sq, sqd)`:
  - `sq` is the sum over values of n squared, so `sq / rows` is the postings-weighted fanout of an equality probe;
  - `sqd` is the same within one document (`sqd / rows`).

  Node-id columns never cross documents, so their per-document sums are exact.
- `stat_kind(kind, n, size)`: nodes of each kind and the sum of their subtree sizes.
- `stat_edge(pkind, ckind, field, n)`: direct edges (efreq).
- `stat_meta(k, v)`: version, documents, nodes, and the row count of each table (the fingerprint).
"""
import sqlite3, sys

VERSION = 1
# The facts schema (exporter/src/main.rs): for each table, the node column that locates a row's document (None:
# the doc table itself), and its document-local columns: node ids, a node's document, and byte offsets, which mean
# something only within their document. Every other column is a value column, with per-value postings.
TABLES = {
    "doc":    (None, ["d"], ["path", "module"]),
    "node":   ("n", ["d", "n", "start", "end_"], ["kind"]),
    "child":  ("p", ["p", "c"], ["idx"]),
    "field":  ("p", ["p", "c"], ["f"]),
    "text":   ("n", ["n"], ["s"]),
    "intval": ("n", ["n"], ["v"]),
    "op":     ("n", ["n"], ["s"]),
    "pkg":    (None, ["d"], ["p"]),
}

def schema(db):
    db.executescript("""
        CREATE TABLE IF NOT EXISTS stat_meta(k TEXT PRIMARY KEY, v INTEGER);
        CREATE TABLE IF NOT EXISTS stat_docrange(d INTEGER PRIMARY KEY, lo INTEGER, hi INTEGER);
        CREATE TABLE IF NOT EXISTS stat_val(tbl TEXT, col TEXT, val, n INTEGER, docs INTEGER, sqd INTEGER,
                                            PRIMARY KEY (tbl, col, val)) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS stat_col(tbl TEXT, col TEXT, rows INTEGER, ndv INTEGER, sq INTEGER, sqd INTEGER,
                                            PRIMARY KEY (tbl, col)) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS stat_kind(kind TEXT PRIMARY KEY, n INTEGER, size INTEGER) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS stat_edge(pkind TEXT, ckind TEXT, field TEXT, n INTEGER,
                                             PRIMARY KEY (pkind, ckind, field)) WITHOUT ROWID;
    """)

def rows_of(tbl, loc, batch="stat_batch"):
    """The FROM clause of a table's rows in the batch's documents, by range on the locating column's index."""
    if loc is None:
        return f"{batch} b CROSS JOIN {tbl} x ON x.d = b.d"
    return f"{batch} b CROSS JOIN stat_docrange r ON r.d = b.d CROSS JOIN {tbl} x ON x.{loc} BETWEEN r.lo AND r.hi"

def merge(db, docs, sign):
    """Adds (sign = 1) or removes (sign = -1) the contribution of documents `docs`, computed from their rows."""
    db.execute("DROP TABLE IF EXISTS temp.stat_batch")
    db.execute("CREATE TEMP TABLE stat_batch(d INTEGER PRIMARY KEY)")
    db.executemany("INSERT INTO stat_batch VALUES (?)", [(d,) for d in docs])
    if sign > 0:
        # A document's interval comes from its tape root: one root per document, and the root's subtree covers it.
        db.execute("INSERT OR REPLACE INTO stat_docrange SELECT k.d, t.n, t.n + t.size FROM tape t JOIN node k ON k.n = t.n "
                   "JOIN stat_batch b ON b.d = k.d WHERE t.par < 0")
        bad = db.execute("SELECT count(*) FROM stat_batch b LEFT JOIN stat_docrange r ON r.d = b.d "
                         "WHERE r.d IS NULL OR (SELECT count(*) FROM node x WHERE x.n BETWEEN r.lo AND r.hi AND x.d = r.d) <> r.hi - r.lo + 1").fetchone()[0]
        if bad: raise SystemExit(f"{bad} documents are not node-id intervals; the planner's document ranges need them")
    for tbl, (loc, ids, vals) in TABLES.items():
        src = rows_of(tbl, loc)
        dg = "x.d" if loc is None else "r.d"
        for col in ids:
            # Document-local columns: grouped per document, so per-document sums are corpus sums (a byte offset in
            # two documents is two values).
            n, ndv, sq = db.execute(f"SELECT coalesce(sum(c), 0), count(*), coalesce(sum(c * c), 0) FROM "
                                    f"(SELECT count(*) AS c FROM {src} GROUP BY {dg}, x.{col})").fetchone()
            add_col(db, tbl, col, sign * n, sign * ndv, sign * sq, sign * sq)
        for col in vals:
            db.execute("DROP TABLE IF EXISTS temp.stat_dv")
            # Keyed, so each touched value is one probe into stat_val's primary key and back (an unkeyed temp table
            # made the UPDATE ... FROM a scan of it per stat_val row: quadratic in the vocabulary).
            db.execute("CREATE TEMP TABLE stat_dv(val PRIMARY KEY, n, docs, sqd) WITHOUT ROWID")
            db.execute(f"INSERT INTO stat_dv SELECT x.{col}, sum(c), count(*), sum(c * c) "
                       f"FROM (SELECT {dg} AS d, x.{col}, count(*) AS c FROM {src} GROUP BY {dg}, x.{col}) x GROUP BY x.{col}")
            # sq and ndv change only through the values this batch touches: sum(n^2) moves by new^2 - old^2.
            old = db.execute(f"SELECT coalesce(sum(coalesce(v.n, 0) * coalesce(v.n, 0)), 0), count(v.n) FROM stat_dv t "
                             f"LEFT JOIN stat_val v ON v.tbl = ? AND v.col = ? AND v.val = t.val", (tbl, col)).fetchone()
            db.execute(f"INSERT INTO stat_val SELECT ?, ?, val, 0, 0, 0 FROM stat_dv WHERE true "
                       f"ON CONFLICT (tbl, col, val) DO NOTHING", (tbl, col))
            db.execute(f"UPDATE stat_val AS v SET n = v.n + ? * t.n, docs = v.docs + ? * t.docs, sqd = v.sqd + ? * t.sqd "
                       f"FROM stat_dv t WHERE v.tbl = ? AND v.col = ? AND v.val = t.val", (sign, sign, sign, tbl, col))
            new = db.execute(f"SELECT coalesce(sum(v.n * v.n), 0), count(*) FROM stat_dv t JOIN stat_val v "
                             f"ON v.tbl = ? AND v.col = ? AND v.val = t.val WHERE v.n > 0", (tbl, col)).fetchone()
            db.execute("DELETE FROM stat_val WHERE tbl = ? AND col = ? AND n = 0", (tbl, col))
            n, sqd = db.execute("SELECT coalesce(sum(n), 0), coalesce(sum(sqd), 0) FROM stat_dv").fetchone()
            add_col(db, tbl, col, sign * n, new[1] - old[1], new[0] - old[0], sign * sqd)
    # Kinds with their subtree sizes, and direct edges (the exporter's kfreq and efreq, here derived in SQL).
    db.execute("DROP TABLE IF EXISTS temp.stat_dk")
    db.execute("CREATE TEMP TABLE stat_dk(kind PRIMARY KEY, n, size) WITHOUT ROWID")
    db.execute(f"INSERT INTO stat_dk SELECT x.kind, count(*), sum(t.size) FROM {rows_of('node', 'n')} "
               f"JOIN tape t ON t.n = x.n GROUP BY x.kind")
    db.execute("INSERT INTO stat_kind SELECT kind, 0, 0 FROM stat_dk WHERE true ON CONFLICT (kind) DO NOTHING")
    db.execute("UPDATE stat_kind AS k SET n = k.n + ? * t.n, size = k.size + ? * t.size FROM stat_dk t WHERE t.kind = k.kind", (sign, sign))
    db.execute("DROP TABLE IF EXISTS temp.stat_de")
    db.execute("CREATE TEMP TABLE stat_de(pkind, ckind, field, n, PRIMARY KEY (pkind, ckind, field)) WITHOUT ROWID")
    db.execute(f"INSERT INTO stat_de SELECT pk.kind, ck.kind, coalesce(f.f, '-'), count(*) "
               f"FROM {rows_of('child', 'p')} JOIN node pk ON pk.n = x.p JOIN node ck ON ck.n = x.c "
               f"LEFT JOIN field f ON f.p = x.p AND f.c = x.c GROUP BY 1, 2, 3")
    db.execute("INSERT INTO stat_edge SELECT pkind, ckind, field, 0 FROM stat_de WHERE true ON CONFLICT DO NOTHING")
    db.execute("UPDATE stat_edge AS e SET n = e.n + ? * t.n FROM stat_de t "
               "WHERE t.pkind = e.pkind AND t.ckind = e.ckind AND t.field = e.field", (sign,))
    db.execute("DELETE FROM stat_kind WHERE n = 0")
    db.execute("DELETE FROM stat_edge WHERE n = 0")
    if sign < 0:
        db.execute("DELETE FROM stat_docrange WHERE d IN (SELECT d FROM stat_batch)")
    fingerprint(db)

def add_col(db, tbl, col, rows, ndv, sq, sqd):
    db.execute("INSERT INTO stat_col VALUES (?, ?, 0, 0, 0, 0) ON CONFLICT DO NOTHING", (tbl, col))
    db.execute("UPDATE stat_col SET rows = rows + ?, ndv = ndv + ?, sq = sq + ?, sqd = sqd + ? WHERE tbl = ? AND col = ?",
               (rows, ndv, sq, sqd, tbl, col))

def fingerprint(db):
    """Row counts the statistics describe. The planner compares them with the facts and refuses a mismatch."""
    rows = [("version", VERSION), ("docs", db.execute("SELECT count(*) FROM stat_docrange").fetchone()[0])]
    rows += [(f"rows.{tbl}", db.execute(f"SELECT coalesce(rows, 0) FROM stat_col WHERE tbl = ? AND col = ?",
                                        (tbl, TABLES[tbl][1][0])).fetchone()[0]) for tbl in TABLES]
    db.executemany("INSERT OR REPLACE INTO stat_meta VALUES (?, ?)", rows)

def build(db):
    for t in ("stat_meta", "stat_docrange", "stat_val", "stat_col", "stat_kind", "stat_edge"):
        db.execute(f"DROP TABLE IF EXISTS {t}")
    schema(db)
    merge(db, [d for (d,) in db.execute("SELECT d FROM doc ORDER BY d")], 1)

def dump(db):
    return {t: sorted(db.execute(f"SELECT * FROM {t}")) for t in ("stat_meta", "stat_docrange", "stat_val", "stat_col", "stat_kind", "stat_edge")}

if __name__ == "__main__":
    import time
    cmd, path = sys.argv[1], sys.argv[2]
    db = sqlite3.connect(path)
    t = time.time()
    if cmd == "build":
        build(db)
        db.commit()
        try:
            size = db.execute("SELECT sum(pgsize) FROM dbstat WHERE name LIKE 'stat_%' OR name LIKE 'sqlite_autoindex_stat_%'").fetchone()[0] or 0
        except sqlite3.OperationalError:   # no dbstat in this build: pages from the page count before and after
            size = 0
        nodes = db.execute("SELECT rows FROM stat_col WHERE tbl = 'node' AND col = 'n'").fetchone()[0]
        per = {t: db.execute(f"SELECT count(*) FROM {t}").fetchone()[0] for t in ("stat_val", "stat_col", "stat_kind", "stat_edge", "stat_docrange")}
        print(f"built in {time.time() - t:.2f}s: {per}, {size} bytes, {size / nodes:.3f} B/node")
    elif cmd == "merge":
        schema(db)
        merge(db, [int(d) for d in sys.argv[4:]], 1 if sys.argv[3] == "+" else -1)
        db.commit()
        print(f"merged in {time.time() - t:.2f}s")
    elif cmd == "check":
        # One-shot against incremental: the documents in three batches, plus five removed and re-added. On an
        # in-memory copy, so the file is untouched.
        mem = sqlite3.connect(":memory:")
        db.backup(mem)
        db = mem
        build(db)
        one = dump(db)
        docs = [d for (d,) in db.execute("SELECT d FROM doc ORDER BY d")]
        for tb in ("stat_meta", "stat_docrange", "stat_val", "stat_col", "stat_kind", "stat_edge"):
            db.execute(f"DELETE FROM {tb}")
        k = len(docs) // 3
        for part in (docs[k:2 * k], docs[:k], docs[2 * k:]):
            merge(db, part, 1)
        merge(db, docs[k:k + 5], -1)
        merge(db, docs[k:k + 5], 1)
        inc = dump(db)
        diff = [t for t in one if one[t] != inc[t]]
        print("incremental equals one-shot" if not diff else f"DIFFERS: {diff}", f"({time.time() - t:.1f}s)")
        if diff: sys.exit(1)
