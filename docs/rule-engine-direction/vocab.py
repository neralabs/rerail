import sqlite3, random, time
print("sqlite", sqlite3.sqlite_version)
c = sqlite3.connect(":memory:")
c.executescript("""
CREATE VIRTUAL TABLE search_kind_fts USING fts5(token, content='', contentless_delete=1, detail=full, tokenize='ascii');
CREATE VIRTUAL TABLE kv_row USING fts5vocab(search_kind_fts, 'row');
CREATE VIRTUAL TABLE kv_inst USING fts5vocab(search_kind_fts, 'instance');
""")
random.seed(1)
kinds = [f"k{i:04x}" for i in range(300)]
w = [1/(i+1) for i in range(300)]  # zipf-ish
N=2000; total=0
for t in range(1, N+1):
    n = random.randint(200, 3000); total += n
    c.execute("INSERT INTO search_kind_fts(rowid, token) VALUES(?,?)", (t, " ".join(random.choices(kinds, w, k=n))))
c.commit()
print("nodes", total)
t0=time.perf_counter(); rows=c.execute("SELECT term, doc, cnt FROM kv_row ORDER BY cnt LIMIT 3").fetchall(); print("rarest", rows, f"{(time.perf_counter()-t0)*1e3:.1f} ms full vocab scan")
t0=time.perf_counter(); r=c.execute("SELECT term, doc, cnt FROM kv_row WHERE term IN ('k0005','k012c')").fetchall(); print("point", r, f"{(time.perf_counter()-t0)*1e3:.2f} ms")
# per-subset (workspace) counts via instance table
c.execute("CREATE TEMP TABLE ws(tape_id INTEGER PRIMARY KEY)"); c.executemany("INSERT INTO ws VALUES(?)", [(i,) for i in range(1, N+1, 4)])
t0=time.perf_counter(); r=c.execute("SELECT term, count(*) FROM kv_inst JOIN ws ON ws.tape_id=kv_inst.doc WHERE term IN ('k0005','k012c') GROUP BY term").fetchall(); print("subset via instance", r, f"{(time.perf_counter()-t0)*1e3:.1f} ms")
# deletion keeps vocab exact?
c.execute("DELETE FROM search_kind_fts WHERE rowid=1"); print("after delete", c.execute("SELECT doc,cnt FROM kv_row WHERE term='k0000'").fetchone())
