// units: micro-measurements of unit costs the architecture's estimates depend on.
//
//   units regex <facts.sqlite> <regexdir>
//       identifier regexes evaluated once per distinct identifier text (vocab), separately and as a RegexSet
//   units flow <corpus> <filelist> <out.sqlite>
//       category-H data flow (state-flow-1 `taints` shape): native per-function worklist vs SQLite recursive CTE
//       at real row counts; also the size the DLRE `descendants` relation would have.

use regex::{Regex, RegexSet};
use rusqlite::{Connection, params};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;
use tree_sitter::{Language, Parser};

#[derive(Clone, Copy)]
struct N {
    kind: u16,
    field: u16,
    parent: u32,
    end: u32,
    sb: u32,
    eb: u32,
}

fn build(src: &[u8], lang: &Language) -> Vec<N> {
    let mut parser = Parser::new();
    parser.set_language(lang).unwrap();
    let tree = parser.parse(src, None).unwrap();
    let mut tape: Vec<N> = Vec::new();
    let mut cursor = tree.walk();
    let mut stack: Vec<u32> = Vec::new();
    loop {
        let node = cursor.node();
        let idx = tape.len() as u32;
        tape.push(N {
            kind: node.kind_id(),
            field: cursor.field_id().map(|f| f.get()).unwrap_or(0),
            parent: stack.last().copied().unwrap_or(u32::MAX),
            end: 0,
            sb: node.start_byte() as u32,
            eb: node.end_byte() as u32,
        });
        if cursor.goto_first_child() {
            stack.push(idx);
            continue;
        }
        tape[idx as usize].end = idx + 1;
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return tape;
            }
            let p = stack.pop().unwrap();
            tape[p as usize].end = tape.len() as u32;
        }
    }
}

fn regex_cmd(db: &str, dir: &str) {
    let conn = Connection::open(db).unwrap();
    let mut st = conn.prepare("SELECT DISTINCT name FROM ref_set").unwrap();
    let vocab: Vec<String> = st.query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
    let occ: i64 = conn.query_row("SELECT count(*) FROM ref", [], |r| r.get(0)).unwrap();
    let mut pats: Vec<String> = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.to_string_lossy().ends_with(".identifier.txt") {
            for l in std::fs::read_to_string(&p).unwrap().lines() {
                pats.push(l.replace("\\n", "\n"));
            }
        }
    }
    pats.sort();
    pats.dedup();
    let compiled: Vec<Regex> = pats.iter().filter_map(|p| Regex::new(p).ok()).collect();
    println!("vocab={} occurrences={} regexes={} compiled={}", vocab.len(), occ, pats.len(), compiled.len());
    let t = Instant::now();
    let mut hits = 0u64;
    for r in &compiled {
        for v in &vocab {
            if r.is_match(v) {
                hits += 1;
            }
        }
    }
    let ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "separate: {:.1} ms total, {:.1} ns per (regex,vocab) pair, hits={}",
        ms, ms * 1e6 / (compiled.len() as f64 * vocab.len() as f64), hits
    );
    let t = Instant::now();
    let ok: Vec<&String> = pats.iter().filter(|p| Regex::new(p).is_ok()).collect();
    let set = RegexSet::new(ok).unwrap();
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let mut hits2 = 0u64;
    for v in &vocab {
        hits2 += set.matches(v).iter().count() as u64;
    }
    let ms = t.elapsed().as_secs_f64() * 1e3;
    println!("regexset: build {:.1} ms, eval {:.1} ms ({:.0} ns per vocab entry), hits={}", build_ms, ms, ms * 1e6 / vocab.len() as f64, hits2);
    // Per-occurrence evaluation (what a per-node matcher does without vocab interning), extrapolated
    let per_pair_ns = 1.0; // placeholder: measured below on a sample
    let _ = per_pair_ns;
    let mut st = conn.prepare("SELECT name FROM ref LIMIT 500000").unwrap();
    let sample: Vec<String> = st.query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
    let r0 = &compiled[..compiled.len().min(64)];
    let t = Instant::now();
    let mut h = 0u64;
    for r in r0 {
        for v in &sample {
            if r.is_match(v) {
                h += 1;
            }
        }
    }
    let ns = t.elapsed().as_secs_f64() * 1e9 / (r0.len() as f64 * sample.len() as f64);
    println!("per-occurrence: {:.1} ns per (regex, identifier occurrence) [64 regexes x 500k occurrences, hits={}]", ns, h);
}

fn flow_cmd(corpus: &str, list: &str, out: &str) {
    let lang: Language = tree_sitter_python::LANGUAGE.into();
    let kid = |s: &str| lang.id_for_node_kind(s, true);
    let fid = |s: &str| lang.field_id_for_name(s).map(|f| f.get()).unwrap_or(0);
    let (k_fn, k_id, k_attr, k_call, k_sub, k_asg, k_params) =
        (kid("function_definition"), kid("identifier"), kid("attribute"), kid("call"), kid("subscript"), kid("assignment"), kid("parameters"));
    let (k_tp, k_dp, k_tdp) = (kid("typed_parameter"), kid("default_parameter"), kid("typed_default_parameter"));
    let (f_obj, f_fun, f_val, f_left, f_right, f_name, f_params) =
        (fid("object"), fid("function"), fid("value"), fid("left"), fid("right"), fid("name"), fid("parameters"));
    let files: Vec<String> = std::fs::read_to_string(list).unwrap().lines().filter(|l| !l.is_empty()).map(String::from).collect();
    let t = Instant::now();
    let docs: Vec<(Vec<u8>, Vec<N>)> = std::thread::scope(|s| {
        let chunks: Vec<Vec<String>> = (0..8).map(|t| files.iter().skip(t).step_by(8).cloned().collect()).collect();
        let hs: Vec<_> = chunks
            .into_iter()
            .map(|c| {
                let lang = lang.clone();
                s.spawn(move || {
                    c.into_iter()
                        .map(|f| {
                            let src = std::fs::read(PathBuf::from(corpus).join(&f)).unwrap();
                            let tape = build(&src, &lang);
                            (src, tape)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let nodes: usize = docs.iter().map(|d| d.1.len()).sum();
    println!("flow scope files={} nodes={} parse+tape wall {:.0} ms", docs.len(), nodes, t.elapsed().as_secs_f64() * 1e3);
    let authority: HashSet<&str> = ["txn", "read_txn", "write_txn", "conn", "connection", "sql_conn"].into_iter().collect();
    let text = |d: &(Vec<u8>, Vec<N>), i: usize| -> String {
        let n = d.1[i];
        String::from_utf8_lossy(&d.0[n.sb as usize..n.eb as usize]).into_owned()
    };

    // ---- fact rows for the flow (what the engine relations would provide)
    let t = Instant::now();
    // per doc: fn owner per node, params per fn, expression edges (src -> dst), assignments (fn, rhs range, lhs name)
    struct DF {
        owner: Vec<u32>,
        params: Vec<(u32, String)>,
        edges: Vec<(u32, u32)>,
        asg: Vec<(u32, u32, u32, String)>, // fn, rhs_start, rhs_end, lhs
        idents: Vec<(u32, u32, String)>,   // node, fn, text
    }
    let mut descendants_rows = 0u64;
    let mut dfs: Vec<DF> = Vec::with_capacity(docs.len());
    for d in &docs {
        let tp = &d.1;
        let mut owner = vec![u32::MAX; tp.len()];
        let mut stack: Vec<(u32, u32)> = Vec::new(); // (end, fn)
        let mut depth = vec![0u32; tp.len()];
        let mut df = DF { owner: Vec::new(), params: Vec::new(), edges: Vec::new(), asg: Vec::new(), idents: Vec::new() };
        for i in 0..tp.len() {
            if i > 0 {
                depth[i] = depth[tp[i].parent as usize] + 1;
                descendants_rows += depth[i] as u64;
            }
            while let Some(&(e, _)) = stack.last() {
                if i as u32 >= e { stack.pop(); } else { break; }
            }
            owner[i] = stack.last().map(|s| s.1).unwrap_or(u32::MAX);
            let n = tp[i];
            let f = owner[i];
            if n.kind == k_fn {
                stack.push((n.end, i as u32));
                // parameters
                let mut c = i + 1;
                while c < n.end as usize {
                    if tp[c].field == f_params && tp[c].kind == k_params {
                        let mut p = c + 1;
                        while p < tp[c].end as usize {
                            let pk = tp[p].kind;
                            let name = if pk == k_id {
                                Some(p)
                            } else if pk == k_tp {
                                ((p + 1)..tp[p].end as usize).find(|&q| tp[q].kind == k_id && tp[q].parent == p as u32)
                            } else if pk == k_dp || pk == k_tdp {
                                ((p + 1)..tp[p].end as usize).find(|&q| tp[q].field == f_name && tp[q].parent == p as u32)
                            } else {
                                None
                            };
                            if let Some(q) = name {
                                df.params.push((i as u32, text(d, q)));
                            }
                            p = tp[p].end as usize;
                        }
                    }
                    c = tp[c].end as usize;
                }
            } else if n.kind == k_id {
                df.idents.push((i as u32, f, text(d, i)));
            } else if n.kind == k_asg {
                let mut l = None;
                let mut r = None;
                let mut c = i + 1;
                while c < n.end as usize {
                    if tp[c].field == f_left { l = Some(c) }
                    if tp[c].field == f_right { r = Some(c) }
                    c = tp[c].end as usize;
                }
                if let (Some(l), Some(r)) = (l, r) {
                    if tp[l].kind == k_id {
                        df.asg.push((f, r as u32, tp[r].end, text(d, l)));
                    }
                }
            }
            if n.parent != u32::MAX {
                let pk = tp[n.parent as usize].kind;
                if (pk == k_attr && n.field == f_obj) || (pk == k_call && n.field == f_fun) || (pk == k_sub && n.field == f_val) {
                    df.edges.push((i as u32, n.parent));
                }
            }
        }
        df.owner = owner;
        dfs.push(df);
    }
    let fact_ms = t.elapsed().as_secs_f64() * 1e3;
    let (np, ne, na, ni): (usize, usize, usize, usize) = dfs.iter().fold((0, 0, 0, 0), |a, d| (a.0 + d.params.len(), a.1 + d.edges.len(), a.2 + d.asg.len(), a.3 + d.idents.len()));
    println!(
        "flow facts: {:.0} ms ({:.1} ns/node) params={} expr_edges={} assigns={} identifiers={} | DLRE descendants relation would hold {} rows ({:.1}x nodes)",
        fact_ms, fact_ms * 1e6 / nodes as f64, np, ne, na, ni, descendants_rows, descendants_rows as f64 / nodes as f64
    );

    // ---- native taint: per doc worklist
    let t = Instant::now();
    let mut tainted_total = 0usize;
    for (di, df) in dfs.iter().enumerate() {
        let tp = &docs[di].1;
        let mut tainted = vec![false; tp.len()];
        let mut work: Vec<u32> = Vec::new();
        let auth: HashSet<(u32, &str)> = df.params.iter().filter(|(_, n)| authority.contains(n.as_str())).map(|(f, n)| (*f, n.as_str())).collect();
        let mut by_name: HashMap<(u32, &str), Vec<u32>> = HashMap::new();
        for (node, f, t) in &df.idents {
            by_name.entry((*f, t.as_str())).or_default().push(*node);
            if auth.contains(&(*f, t.as_str())) && !tainted[*node as usize] {
                tainted[*node as usize] = true;
                work.push(*node);
            }
        }
        let mut up: HashMap<u32, Vec<u32>> = HashMap::new();
        for (s, dd) in &df.edges {
            up.entry(*s).or_default().push(*dd);
        }
        let mut done_names: HashSet<(u32, &str)> = HashSet::new();
        loop {
            while let Some(x) = work.pop() {
                if let Some(v) = up.get(&x) {
                    for &y in v {
                        if !tainted[y as usize] {
                            tainted[y as usize] = true;
                            work.push(y);
                        }
                    }
                }
            }
            // assignments: rhs root tainted (or any rhs node tainted) -> lhs name tainted in same fn
            let mut changed = false;
            for (f, rs, re, lhs) in &df.asg {
                if done_names.contains(&(*f, lhs.as_str())) {
                    continue;
                }
                if (*rs..*re).any(|j| tainted[j as usize]) {
                    done_names.insert((*f, lhs.as_str()));
                    if let Some(v) = by_name.get(&(*f, lhs.as_str())) {
                        for &y in v {
                            if !tainted[y as usize] {
                                tainted[y as usize] = true;
                                work.push(y);
                                changed = true;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        tainted_total += tainted.iter().filter(|&&b| b).count();
    }
    let native_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("native taint: {:.1} ms, tainted nodes={}", native_ms, tainted_total);

    // ---- SQLite: same relations, recursive CTE
    let _ = std::fs::rename(out, format!("{out}.old"));
    let mut db = Connection::open(out).unwrap();
    db.execute_batch(
        "PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA cache_size=-1000000; PRAGMA temp_store=MEMORY;
         CREATE TABLE ident(file INT, node INT, fn INT, text TEXT);
         CREATE TABLE param(file INT, fn INT, name TEXT);
         CREATE TABLE xedge(file INT, src INT, dst INT);
         CREATE TABLE asg(file INT, fn INT, rs INT, re INT, lhs TEXT);",
    )
    .unwrap();
    let t = Instant::now();
    {
        let tx = db.transaction().unwrap();
        {
            let mut a = tx.prepare("INSERT INTO ident VALUES(?,?,?,?)").unwrap();
            let mut b = tx.prepare("INSERT INTO param VALUES(?,?,?)").unwrap();
            let mut c = tx.prepare("INSERT INTO xedge VALUES(?,?,?)").unwrap();
            let mut e = tx.prepare("INSERT INTO asg VALUES(?,?,?,?,?)").unwrap();
            for (fi, df) in dfs.iter().enumerate() { let fi = fi as i64;
                for (n, f, t) in &df.idents { a.execute(params![fi, n, f, t]).unwrap(); }
                for (f, n) in &df.params { b.execute(params![fi, f, n]).unwrap(); }
                for (s, d) in &df.edges { c.execute(params![fi, s, d]).unwrap(); }
                for (f, rs, re, l) in &df.asg { e.execute(params![fi, f, rs, re, l]).unwrap(); }
            }
        }
        tx.commit().unwrap();
    }
    println!("sqlite flow insert: {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    db.execute_batch(
        "CREATE INDEX ident_fn_text ON ident(file, fn, text); CREATE INDEX ident_node ON ident(file, node);
         CREATE INDEX xedge_src ON xedge(file, src); CREATE INDEX asg_fn ON asg(file, fn); CREATE INDEX param_fn ON param(file, fn, name); ANALYZE;",
    )
    .unwrap();
    println!("sqlite flow indexes: {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let n: i64 = db
        .query_row(
            "WITH RECURSIVE taint(file, node) AS (
               SELECT i.file, i.node FROM ident i JOIN param p ON p.file = i.file AND p.fn = i.fn AND p.name = i.text
                 WHERE p.name IN ('txn','read_txn','write_txn','conn','connection','sql_conn')
               UNION
               SELECT x.file, x.dst FROM taint t JOIN xedge x ON x.file = t.file AND x.src = t.node
               UNION
               SELECT i.file, i.node FROM taint t JOIN ident ti ON ti.file = t.file AND ti.node = t.node
                 JOIN asg a ON a.file = t.file AND a.fn = ti.fn AND t.node >= a.rs AND t.node < a.re
                 JOIN ident i ON i.file = a.file AND i.fn = a.fn AND i.text = a.lhs
             ) SELECT count(*) FROM taint",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!("sqlite taint CTE: {:.0} ms, tainted={} (identifier-reachable subset; native counts all node kinds)", t.elapsed().as_secs_f64() * 1e3, n);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a[1].as_str() {
        "regex" => regex_cmd(&a[2], &a[3]),
        "flow" => flow_cmd(&a[2], &a[3], &a[4]),
        _ => panic!("unknown"),
    }
}
