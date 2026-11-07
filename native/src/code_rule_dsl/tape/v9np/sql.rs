//! Pure-SQL tape decoding on the bundled SQLite (ast-index-spike `index_spike_sqlite_puresql.py`
//! technique: fixed-stride `substr` over per-file blobs, rows from `generate_series`, structure by
//! subtree size and parent delta). No registered function: a byte is
//! `instr(X'000102..FF', substr(col, pos, 1)) - 1`.
use super::{Doc, V7, V8, V9, load, widths};
use rusqlite::{Connection, params};
use std::time::Instant;

fn byte_table() -> String {
    let mut s = String::from("X'");
    for b in 0..=255u8 {
        s.push_str(&format!("{b:02X}"));
    }
    s.push('\'');
    s
}

/// SQL: byte at 0-based offset `off` of blob `col`.
fn b(col: &str, off: &str) -> String {
    format!("(instr({}, substr({col}, ({off})+1, 1))-1)", byte_table())
}
fn u16le(col: &str, off: &str) -> String {
    format!("({} + 256*{})", b(col, off), b(col, &format!("({off})+1")))
}
fn u32le(col: &str, off: &str) -> String {
    format!(
        "({} + 256*{} + 65536*{} + 16777216*{})",
        b(col, off),
        b(col, &format!("({off})+1")),
        b(col, &format!("({off})+2")),
        b(col, &format!("({off})+3"))
    )
}
fn u24le(col: &str, off: &str) -> String {
    format!(
        "({} + 256*{} + 65536*{})",
        b(col, off),
        b(col, &format!("({off})+1")),
        b(col, &format!("({off})+2"))
    )
}

/// Field expressions for a variant, over table alias `t` and node index expression `i`.
struct Fields {
    table: &'static str,
    kind: fn(&str) -> String,
    pd: fn(&str) -> String,
    size: fn(&str) -> String,
}

fn v7_kind(i: &str) -> String {
    format!("({} & 1023)", u16le("t.rec", &format!("6*({i})")))
}
fn v7_pd(i: &str) -> String {
    u16le("t.rec", &format!("6*({i})+2"))
}
fn v7_size(i: &str) -> String {
    u16le("t.rec", &format!("6*({i})+4"))
}
fn v8_kind(i: &str) -> String {
    format!("({} & 1023)", u16le("t.kf", &format!("2*({i})")))
}
fn v8_pd(i: &str) -> String {
    u16le("t.par", &format!("2*({i})"))
}
fn v8_size(i: &str) -> String {
    u16le("t.size", &format!("2*({i})"))
}

fn v9_kind(i: &str) -> String {
    format!("({} & 1023)", u16le("t.kf", &format!("2*({i})")))
}
fn v9_pd(i: &str) -> String {
    u16le("t.par", &format!("2*({i})"))
}
fn v9_size(i: &str) -> String {
    u16le("t.size", &format!("2*({i})"))
}
fn v9_start(i: &str) -> String {
    format!("({} + {})", u32le("t.base", &format!("4*(({i})/16)")), u16le("t.off", &format!("2*({i})")))
}

fn v9r_start(i: &str) -> String {
    format!("({} + {})", u32le("t.base", &format!("4*(({i})/16)")), u16le("t.sp", &format!("4*({i})")))
}
fn v9i_kind(i: &str) -> String {
    format!("({} & 1023)", u16le("t.rec", &format!("8*({i})")))
}
fn v9i_size(i: &str) -> String {
    u16le("t.rec", &format!("8*({i})+2"))
}
fn v9i_start(i: &str) -> String {
    format!("({} + {})", u32le("t.base", &format!("4*(({i})/16)")), u16le("t.rec", &format!("8*({i})+4")))
}
fn no_parent(_: &str) -> String {
    unreachable!()
}
const V9RF: Fields = Fields { table: "t9r", kind: v7_kind, pd: v7_pd, size: v7_size };
const V9IF: Fields = Fields { table: "t9i", kind: v9i_kind, pd: no_parent, size: v9i_size };

fn q_span_v9r(kstr: u16) -> String {
    let last = format!("(s.value+{}-1)", v7_size("s.value"));
    format!(
        "SELECT count(*), sum(length(substr(t.src, {st}+1, {en}-{st}))), sum({st}) FROM t9r t, generate_series(0, t.n-1) s WHERE {ks}={kstr}",
        st = v9r_start("s.value"),
        en = format!("({} + {})", v9r_start(&last), u16le("t.sp", "4*s.value+2")),
        ks = v7_kind("s.value"),
    )
}
fn q_span_v9i(kstr: u16) -> String {
    let last = format!("(s.value+{}-1)", v9i_size("s.value"));
    format!(
        "SELECT count(*), sum(length(substr(t.src, {st}+1, {en}-{st}))), sum({st}) FROM t9i t, generate_series(0, t.n-1) s WHERE {ks}={kstr}",
        st = v9i_start("s.value"),
        en = format!("({} + {})", v9i_start(&last), u16le("t.rec", "8*s.value+6")),
        ks = v9i_kind("s.value"),
    )
}

const V9F: Fields = Fields { table: "t9", kind: v9_kind, pd: v9_pd, size: v9_size };
const V7F: Fields = Fields { table: "t7", kind: v7_kind, pd: v7_pd, size: v7_size };
const V8F: Fields = Fields { table: "t8", kind: v8_kind, pd: v8_pd, size: v8_size };

fn q_has(f: &Fields, kfn: u16, kret: u16) -> String {
    format!(
        "SELECT count(*) FROM {tb} t, generate_series(0, t.n-1) a \
         WHERE {ka}={kfn} AND EXISTS (SELECT 1 FROM generate_series(a.value+1, a.value+{sa}-1) d WHERE {kd}={kret})",
        tb = f.table,
        ka = (f.kind)("a.value"),
        sa = (f.size)("a.value"),
        kd = (f.kind)("d.value"),
    )
}

fn q_nth(f: &Fields, kcall: u16) -> String {
    format!(
        "WITH RECURSIVE a(id, i, e) AS (SELECT t.id, s.value, s.value+{ss} FROM {tb} t, generate_series(0, t.n-1) s WHERE {ks}={kcall}), \
         c(id, k, j, e) AS (SELECT id, 1, i+1, e FROM a WHERE i+1 < e \
           UNION ALL SELECT c.id, c.k+1, c.j+{sj}, c.e FROM c JOIN {tb} t ON t.id=c.id WHERE c.k < 2 AND c.j+{sj} < c.e) \
         SELECT {kj}, count(*) FROM c JOIN {tb} t ON t.id=c.id WHERE c.k=2 GROUP BY 1 ORDER BY 1",
        tb = f.table,
        ss = (f.size)("s.value"),
        ks = (f.kind)("s.value"),
        sj = (f.size)("c.j"),
        kj = (f.kind)("c.j"),
    )
}

fn q_inside(f: &Fields, kret: u16, kfn: u16) -> String {
    format!(
        "WITH RECURSIVE r(id, i, cur) AS (SELECT t.id, s.value, s.value-{ps} FROM {tb} t, generate_series(0, t.n-1) s WHERE {ks}={kret} \
           UNION ALL SELECT r.id, r.i, r.cur-{pc} FROM r JOIN {tb} t ON t.id=r.id WHERE r.cur >= 0 AND {kc} != {kfn}) \
         SELECT count(*) FROM r JOIN {tb} t ON t.id=r.id WHERE r.cur >= 0 AND {kc}={kfn}",
        tb = f.table,
        ps = (f.pd)("s.value"),
        ks = (f.kind)("s.value"),
        pc = (f.pd)("r.cur"),
        kc = (f.kind)("r.cur"),
    )
}

/// `inside` (stopBy end) as interval containment between two anchor sets: no parent column needed.
fn q_inside_interval(f: &Fields, kret: u16, kfn: u16) -> String {
    format!(
        "WITH f(id, i, e) AS MATERIALIZED (SELECT t.id, s.value, s.value+{ss} FROM {tb} t, generate_series(0, t.n-1) s WHERE {ks}={kfn}), \
         r(id, i) AS MATERIALIZED (SELECT t.id, s.value FROM {tb} t, generate_series(0, t.n-1) s WHERE {ks}={kret}) \
         SELECT count(*) FROM r WHERE EXISTS (SELECT 1 FROM f WHERE f.id=r.id AND f.i < r.i AND r.i < f.e)",
        tb = f.table,
        ss = (f.size)("s.value"),
        ks = (f.kind)("s.value"),
    )
}

fn q_span_v9(kstr: u16) -> String {
    let last = format!("(s.value+{}-1)", v9_size("s.value"));
    format!(
        "SELECT count(*), sum(length(substr(t.src, {st}+1, {en}-{st}))), sum({st}) FROM t9 t, generate_series(0, t.n-1) s WHERE {ks}={kstr}",
        st = v9_start("s.value"),
        en = format!("({} + {})", v9_start(&last), u16le("t.tail", "2*s.value")),
        ks = v9_kind("s.value"),
    )
}

fn q_span_v8(kstr: u16) -> String {
    format!(
        "SELECT count(*), sum(length(substr(t.src, {st}+1, {ln}))), sum({st}) FROM t8 t, generate_series(0, t.n-1) s WHERE {ks}={kstr}",
        st = u16le("t.start", "2*s.value"),
        ln = u16le("t.len", "2*s.value"),
        ks = v8_kind("s.value"),
    )
}

/// v7 spans in pure SQL: from the node's checkpoint, decode varints byte by byte (zigzag start delta,
/// then length) through every node of the 16-node block up to the node (`wire::index_span`).
fn q_span_v7(kstr: u16) -> String {
    let by = b("t.spans", "w.pos");
    let val = format!("(w.acc + (({by} & 127) << w.sh))");
    let zz = format!("(CASE WHEN {val} & 1 THEN -({val} >> 1)-1 ELSE {val} >> 1 END)");
    format!(
        "WITH RECURSIVE a(id, i) AS (SELECT t.id, s.value FROM t7 t, generate_series(0, t.n-1) s WHERE {ks}={kstr}), \
         w(id, i, j, pos, f, acc, sh, st, ln) AS ( \
           SELECT a.id, a.i, (a.i/16)*16, {cpo}, 0, 0, 0, {cps}, 0 FROM a JOIN t7 t ON t.id=a.id \
           UNION ALL SELECT w.id, w.i, \
             CASE WHEN {by} < 128 AND w.f=1 THEN w.j+1 ELSE w.j END, \
             w.pos+1, \
             CASE WHEN {by} < 128 THEN 1-w.f ELSE w.f END, \
             CASE WHEN {by} < 128 THEN 0 ELSE {val} END, \
             CASE WHEN {by} < 128 THEN 0 ELSE w.sh+7 END, \
             CASE WHEN {by} < 128 AND w.f=0 AND w.j != (w.i/16)*16 THEN w.st + {zz} ELSE w.st END, \
             CASE WHEN {by} < 128 AND w.f=1 THEN {val} ELSE w.ln END \
           FROM w JOIN t7 t ON t.id=w.id WHERE w.j <= w.i) \
         SELECT count(*), sum(length(substr(t.src, w.st+1, w.ln))), sum(w.st) FROM w JOIN t7 t ON t.id=w.id WHERE w.j = w.i+1",
        ks = v7_kind("s.value"),
        cpo = u24le("t.cps", "6*(a.i/16)"),
        cps = u24le("t.cps", "6*(a.i/16)+3"),
    )
}

fn time_query(conn: &Connection, sql: &str) -> (f64, String) {
    let mut best = f64::MAX;
    let mut out = String::new();
    for _ in 0..3 {
        let t = Instant::now();
        let mut stmt = conn.prepare(sql).unwrap();
        let cols = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        let mut acc = Vec::new();
        while let Some(r) = rows.next().unwrap() {
            let vals: Vec<String> = (0..cols)
                .map(|c| match r.get_ref(c).unwrap() {
                    rusqlite::types::ValueRef::Integer(v) => v.to_string(),
                    rusqlite::types::ValueRef::Null => "null".into(),
                    other => format!("{other:?}"),
                })
                .collect();
            acc.push(vals.join(","));
        }
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        out = acc.join(";");
    }
    (best, out)
}

/// Expected answers computed from the parsed nodes.
fn expected(docs: &[&Doc], kfn: u16, kret: u16, kcall: u16, kstr: u16) -> [String; 4] {
    let kind = |d: &Doc, i: usize| d.nodes[i].kf & 1023;
    let mut has = 0;
    let mut inside = 0;
    let mut nth = std::collections::BTreeMap::new();
    let (mut sc, mut sl, mut ss) = (0u64, 0u64, 0u64);
    for d in docs {
        for i in 0..d.nodes.len() {
            let k = kind(d, i);
            let size = d.nodes[i].size as usize;
            if k == kfn && (i + 1..i + size).any(|j| kind(d, j) == kret) {
                has += 1;
            }
            if k == kret {
                let mut cur = d.nodes[i].parent;
                while let Some(c) = cur {
                    if kind(d, c as usize) == kfn {
                        inside += 1;
                        break;
                    }
                    cur = d.nodes[c as usize].parent;
                }
            }
            if k == kcall && size > 1 {
                let first = i + 1;
                let second = first + d.nodes[first].size as usize;
                if second < i + size {
                    *nth.entry(kind(d, second)).or_insert(0u64) += 1;
                }
            }
            if k == kstr {
                sc += 1;
                sl += d.nodes[i].length as u64;
                ss += d.nodes[i].start as u64;
            }
        }
    }
    let nth = nth.iter().map(|(k, v)| format!("{k},{v}")).collect::<Vec<_>>().join(";");
    [has.to_string(), nth, inside.to_string(), format!("{sc},{sl},{ss}")]
}

pub fn run(lang: &str, db: &str, max_files: usize, dirs: &[String]) {
    assert_eq!(lang, "python", "the SQL queries use Python kinds");
    let (all, _, _) = load(lang, dirs, max_files);
    let docs: Vec<&Doc> = all.iter().filter(|d| { let w = widths(d.nodes.len(), d.source.len()); w.w == 2 && w.s == 2 }).collect();
    let excluded = all.len() - docs.len();
    let lang_ts = &docs[0].language;
    let id = |name: &str| lang_ts.id_for_node_kind(name, true);
    let (kfn, kret, kcall, kstr) = (id("function_definition"), id("return_statement"), id("call"), id("string"));
    assert!(!std::path::Path::new(db).exists(), "database {db} exists: pass a new path");
    let conn = Connection::open(db).unwrap();
    rusqlite::vtab::series::load_module(&conn).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA cache_size=-1000000;
         CREATE TABLE t7(id INTEGER PRIMARY KEY, n INT, rec BLOB, spans BLOB, cps BLOB, src BLOB);
         CREATE TABLE t8(id INTEGER PRIMARY KEY, n INT, kf BLOB, par BLOB, size BLOB, start BLOB, len BLOB, src BLOB);
         CREATE TABLE t9(id INTEGER PRIMARY KEY, n INT, kf BLOB, par BLOB, size BLOB, base BLOB, off BLOB, tail BLOB, src BLOB);
         CREATE TABLE t9r(id INTEGER PRIMARY KEY, n INT, rec BLOB, sp BLOB, base BLOB, src BLOB);
         CREATE TABLE t9i(id INTEGER PRIMARY KEY, n INT, rec BLOB, base BLOB, src BLOB);",
    )
    .unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    let (mut bytes7, mut bytes8, mut bytes9, mut nodes) = (0usize, 0usize, 0usize, 0usize);
    for (k, d) in docs.iter().enumerate() {
        let a = V7::encode(&d.nodes).unwrap();
        assert!(a.par_overflow.is_empty() && a.size_overflow.is_empty());
        let v = V8::encode(&d.nodes, d.source.len());
        bytes7 += a.rec.len() + a.spans.len() + a.checkpoints.len();
        bytes8 += v.kf.len() + v.par.len() + v.size.len() + v.start.len() + v.len.len();
        nodes += d.nodes.len();
        let c = V9::encode(&d.nodes);
        assert!(c.o == 2 && c.t == 2);
        bytes9 += c.kf.len() + c.size.len() + c.base.len() + c.off.len() + c.tail.len();
        tx.execute(
            "INSERT INTO t9 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![k as i64, d.nodes.len() as i64, c.kf, c.par, c.size, c.base, c.off, c.tail, d.source.as_bytes()],
        )
        .unwrap();
        let (rec, sp) = c.record_with_spans();
        tx.execute("INSERT INTO t9r VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![k as i64, d.nodes.len() as i64, rec, sp, c.base, d.source.as_bytes()]).unwrap();
        tx.execute("INSERT INTO t9i VALUES (?1, ?2, ?3, ?4, ?5)", params![k as i64, d.nodes.len() as i64, c.interleaved_np(), c.base, d.source.as_bytes()]).unwrap();
        tx.execute(
            "INSERT INTO t7 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![k as i64, d.nodes.len() as i64, a.rec, a.spans, a.checkpoints, d.source.as_bytes()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO t8 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![k as i64, d.nodes.len() as i64, v.kf, v.par, v.size, v.start, v.len, d.source.as_bytes()],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let version: String = conn.query_row("SELECT sqlite_version()", [], |r| r.get(0)).unwrap();
    let exp = expected(&docs, kfn, kret, kcall, kstr);
    let skip = || "SELECT 'n/a'".to_string();
    let queries: Vec<(&str, [String; 5], usize)> = vec![
        ("has: function_definition has return_statement (stopBy end)", [q_has(&V7F, kfn, kret), q_has(&V8F, kfn, kret), q_has(&V9F, kfn, kret), q_has(&V9RF, kfn, kret), q_has(&V9IF, kfn, kret)], 0),
        ("nthChild 2 of call (sibling chain)", [q_nth(&V7F, kcall), q_nth(&V8F, kcall), q_nth(&V9F, kcall), q_nth(&V9RF, kcall), q_nth(&V9IF, kcall)], 1),
        ("inside: return_statement inside function_definition (parent climb)", [q_inside(&V7F, kret, kfn), q_inside(&V8F, kret, kfn), q_inside(&V9F, kret, kfn), q_inside(&V9RF, kret, kfn), skip()], 2),
        ("inside as interval containment (no parent column)", [q_inside_interval(&V7F, kret, kfn), q_inside_interval(&V8F, kret, kfn), q_inside_interval(&V9F, kret, kfn), q_inside_interval(&V9RF, kret, kfn), q_inside_interval(&V9IF, kret, kfn)], 2),
        ("span text of every string node", [q_span_v7(kstr), q_span_v8(kstr), q_span_v9(kstr), q_span_v9r(kstr), q_span_v9i(kstr)], 3),
    ];
    println!(
        "sqlite {version}; files {} (excluded {excluded} with a source >= 64 KiB or >= 65535 nodes); nodes {nodes}; source {} B; \
         v7 rec+spans+cps {:.2} B/node; v8 kf+par+size+start+len {:.2} B/node; v9np kf+size+base+off+tail {:.2} B/node (+2 with par)",
        docs.len(),
        docs.iter().map(|d| d.source.len()).sum::<usize>(),
        bytes7 as f64 / nodes as f64,
        bytes8 as f64 / nodes as f64,
        bytes9 as f64 / nodes as f64
    );
    println!("| query | v7 ms | v8 (SoA) ms | v9 (SoA) ms | v9r (v7 rec6 + span rec) ms | v9i (interleaved, no parent) ms | parity v7/v8/v9/v9r/v9i | answer |\n|---|---|---|---|---|---|---|---|");
    for (name, sqls, e) in queries {
        let mut cells = Vec::new();
        let mut parity = Vec::new();
        let mut answer = String::new();
        for q in &sqls {
            let (m, r) = time_query(&conn, q);
            if r == "Text([110, 47, 97])" || q.starts_with("SELECT 'n/a'") {
                cells.push("n/a".to_string());
                parity.push("-".to_string());
            } else {
                cells.push(format!("{m:.1}"));
                parity.push((r == exp[e]).to_string());
                answer = r.chars().take(60).collect();
            }
        }
        println!("| {name} | {} | {} | {answer} |", cells.join(" | "), parity.join("/"));
    }
}
