import sqlite3, time
db = sqlite3.connect(":memory:")
db.execute("CREATE TABLE kfreq(code INTEGER PRIMARY KEY, n INTEGER)")
db.executemany("INSERT INTO kfreq VALUES(?,?)", [(1, 10), (2, 5_000_000)])
db.execute("CREATE TABLE t(x INTEGER)")
db.executemany("INSERT INTO t VALUES(?)", ((i,) for i in range(300_000)))
G_F = "(SELECT n FROM kfreq WHERE code=1) > (SELECT n FROM kfreq WHERE code=2)"
G_T = "(SELECT n FROM kfreq WHERE code=1) < (SELECT n FROM kfreq WHERE code=2)"
def tm(sql, reps=5):
    xs=[]
    for _ in range(reps):
        t=time.perf_counter(); rows=db.execute(sql).fetchall(); xs.append(time.perf_counter()-t)
    xs.sort(); return xs[len(xs)//2]*1000, rows
HEAVY = "heavy(i) AS {m}(SELECT 0 {seedguard} UNION ALL SELECT i+1 FROM heavy WHERE i<2000000)"
def shape(kind, g, mat):
    m = "MATERIALIZED " if mat else ""
    if kind == "outer-where":   # guard on the consuming arm
        return f"WITH RECURSIVE {HEAVY.format(m='',seedguard='')} SELECT 'heavy', count(*) FROM heavy WHERE ({g}) UNION ALL SELECT 'light', count(*) FROM t"
    if kind == "gate-join":     # 0/1-row gate CTE drives the join
        return f"WITH RECURSIVE gate(ok) AS {m}(SELECT 1 WHERE {g}), {HEAVY.format(m='',seedguard='')} SELECT 'heavy', count(*) FROM gate CROSS JOIN heavy UNION ALL SELECT 'light', count(*) FROM t"
    if kind == "seed-guard":    # guard in the recursive seed
        return f"WITH RECURSIVE {HEAVY.format(m='',seedguard='WHERE '+g)} SELECT 'heavy', count(*) FROM heavy UNION ALL SELECT 'light', count(*) FROM t"
    if kind == "case-scalar":   # CASE WHEN guard THEN (expensive scalar subquery)
        return f"WITH RECURSIVE {HEAVY.format(m='',seedguard='')} SELECT 'heavy', CASE WHEN {g} THEN (SELECT count(*) FROM heavy) END UNION ALL SELECT 'light', count(*) FROM t"
print("sqlite", sqlite3.sqlite_version)
print("light alone %.1f ms" % tm("SELECT count(*) FROM t")[0])
for kind in ("outer-where", "gate-join", "seed-guard", "case-scalar"):
    for mat in ((False, True) if kind=="gate-join" else (False,)):
        f = tm(shape(kind, G_F, mat)); t = tm(shape(kind, G_T, mat))
        print(f"{kind:12s} {'mat' if mat else '   '} guard false {f[0]:7.1f} ms   guard true {t[0]:7.1f} ms   {f[1]} | {t[1]}")
