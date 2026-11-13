"""Synthetic shape-flip demo: pattern (P (C)) -- a P node with a direct C child.
Two corpus slices with swapped kind frequencies. Compare: anchor-on-parent (scan P, probe children),
anchor-on-child (scan C, probe parent), stats-chosen (rarer kind from kfreq), and SQLite's own planner
with and without ANALYZE."""
import random, sqlite3, time
random.seed(7)
N = 1_500_000
def build(p_frac, c_frac):
    db = sqlite3.connect(":memory:")
    db.execute("CREATE TABLE node(id INTEGER PRIMARY KEY, parent INT, kind INT)")
    rows = []
    for i in range(N):
        r = random.random()
        k = 1 if r < p_frac else 2 if r < p_frac + c_frac else 3 + int(random.random()*50)
        rows.append((i, max(0, i - 1 - int(random.random()*20)) if i else None, k))
    db.executemany("INSERT INTO node VALUES(?,?,?)", rows)
    db.execute("CREATE INDEX node_kind ON node(kind, id)")
    db.execute("CREATE INDEX node_parent ON node(parent, kind)")
    db.execute("CREATE TABLE kfreq AS SELECT kind, count(*) n FROM node GROUP BY kind")
    return db
SHAPES = {
 "anchor P (descend)": "SELECT count(*) FROM node p CROSS JOIN node c WHERE p.kind=1 AND c.parent=p.id AND c.kind=2",
 "anchor C (climb)":   "SELECT count(*) FROM node c CROSS JOIN node p WHERE c.kind=2 AND p.id=c.parent AND p.kind=1",
 "planner free":       "SELECT count(*) FROM node p JOIN node c ON c.parent=p.id WHERE p.kind=1 AND c.kind=2",
}
def tm(db, sql, reps=5):
    xs=[]
    for _ in range(reps):
        t=time.perf_counter(); r=db.execute(sql).fetchone()[0]; xs.append(time.perf_counter()-t)
    xs.sort(); return xs[reps//2]*1000, r
for name, (pf, cf) in {"slice A: P rare (0.2%), C common (30%)": (0.002, 0.30),
                       "slice B: P common (30%), C rare (0.2%)": (0.30, 0.002)}.items():
    db = build(pf, cf)
    f = dict(db.execute("SELECT kind, n FROM kfreq WHERE kind IN (1,2)"))
    print(f"{name}   kfreq P={f[1]} C={f[2]}")
    res = {k: tm(db, s) for k, s in SHAPES.items()}
    for k, (ms, r) in res.items(): print(f"  {k:22s} {ms:8.1f} ms  ({r} matches)")
    pick = "anchor P (descend)" if f[1] <= f[2] else "anchor C (climb)"
    print(f"  stats-chosen -> {pick}: {res[pick][0]:.1f} ms")
    plan = db.execute("EXPLAIN QUERY PLAN " + SHAPES["planner free"]).fetchall()
    print("  planner (no stat1):", [p[3] for p in plan])
    db.execute("ANALYZE")
    ms, r = tm(db, SHAPES["planner free"])
    plan = db.execute("EXPLAIN QUERY PLAN " + SHAPES["planner free"]).fetchall()
    print(f"  planner after ANALYZE {ms:8.1f} ms", [p[3] for p in plan])
