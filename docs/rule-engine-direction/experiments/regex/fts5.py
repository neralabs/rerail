import sqlite3, time, os, sys, json
CORPUS = sys.argv[1]; FILES = [l.strip() for l in open(sys.argv[2]) if l.strip()]
texts = [open(os.path.join(CORPUS, f), encoding='utf-8', errors='replace').read() for f in FILES]
total = sum(len(t.encode()) for t in texts)
res = {'source_bytes': total}
for detail in ['none', 'full']:
    db = f'out/fts5_{detail}.db'
    if os.path.exists(db): os.rename(db, f'/tmp/ai-trash-{os.path.basename(db)}-{time.time()}')
    con = sqlite3.connect(db); con.execute('PRAGMA journal_mode=OFF'); con.execute('PRAGMA synchronous=OFF')
    con.execute(f"CREATE VIRTUAL TABLE t USING fts5(c, tokenize='trigram case_sensitive 1', detail={detail}, content='')")
    t0 = time.time()
    with con:
        con.executemany('INSERT INTO t(rowid, c) VALUES (?, ?)', ((i + 1, x) for i, x in enumerate(texts)))
    con.execute("INSERT INTO t(t) VALUES('optimize')"); con.commit()
    b = time.time() - t0
    con.close()
    res[detail] = {'build_s': b, 'db_bytes': os.path.getsize(db), 'bytes_per_source_byte': os.path.getsize(db) / total}
    print(detail, res[detail], flush=True)
# query timing with AND of trigrams on detail=none
Q = json.load(open('out/tri_queries.json'))
def fts(q):
    if q.get('all'): return None
    if 'tri' in q: return ' AND '.join('"' + t.replace('"', '""') + '"' for t in q['tri'])
    parts = [fts(a) for a in q['args']]
    if q['op'] == 'and':
        parts = [p for p in parts if p]
        return ' AND '.join('(' + p + ')' for p in parts) if parts else None
    if any(p is None for p in parts): return None
    return ' OR '.join('(' + p + ')' for p in parts) if parts else '"\x01\x01\x01"'
con = sqlite3.connect('out/fts5_none.db')
t0 = time.time(); n = 0; cands = []
for q in Q:
    e = fts(q['query'])
    if e is None: cands.append(None); continue
    rows = con.execute('SELECT rowid FROM t WHERE t MATCH ?', (e,)).fetchall(); n += len(rows); cands.append(len(rows))
res['query_s_all_clauses'] = time.time() - t0; res['cands'] = cands
print(res['query_s_all_clauses'], n)
json.dump(res, open('out/fts5.json', 'w'))
