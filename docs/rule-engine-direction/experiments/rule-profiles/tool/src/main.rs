// ruleprof: corpus statistics, fact-family extraction cost, and SQLite
// cross-file join costs at real row counts for the data-layer gate corpus.
//
// usage: ruleprof <corpus_dir> <out_dir> [threads]

use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tree_sitter::{Language, Parser};

#[derive(Clone, Copy)]
struct N {
    kind: u16,
    field: u16, // 0 = none
    named: bool,
    parent: u32,
    end: u32, // exclusive preorder end
    sb: u32,
    eb: u32,
}

struct Doc {
    path: String,
    src: Vec<u8>,
    tape: Vec<N>,
    parse_ns: u64,
    tape_ns: u64,
    max_depth: u32,
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        let ft = e.file_type().unwrap();
        if ft.is_dir() {
            collect(&p, out);
        } else if p.extension().map(|x| x == "py").unwrap_or(false) {
            out.push(p);
        }
    }
}

fn build(src: &[u8], lang: &Language) -> (Vec<N>, u64, u64, u32) {
    let mut parser = Parser::new();
    parser.set_language(lang).unwrap();
    let t0 = Instant::now();
    let tree = parser.parse(src, None).unwrap();
    let parse_ns = t0.elapsed().as_nanos() as u64;
    let t1 = Instant::now();
    let mut tape: Vec<N> = Vec::new();
    let mut cursor = tree.walk();
    let mut stack: Vec<u32> = Vec::new();
    let mut max_depth = 0u32;
    loop {
        let node = cursor.node();
        let idx = tape.len() as u32;
        let parent = stack.last().copied().unwrap_or(u32::MAX);
        tape.push(N {
            kind: node.kind_id(),
            field: cursor.field_id().map(|f| f.get()).unwrap_or(0),
            named: node.is_named(),
            parent,
            end: 0,
            sb: node.start_byte() as u32,
            eb: node.end_byte() as u32,
        });
        if cursor.goto_first_child() {
            stack.push(idx);
            max_depth = max_depth.max(stack.len() as u32);
            continue;
        }
        tape[idx as usize].end = idx + 1;
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                let t = t1.elapsed().as_nanos() as u64;
                return (tape, parse_ns, t, max_depth);
            }
            let p = stack.pop().unwrap();
            tape[p as usize].end = tape.len() as u32;
        }
    }
}

struct K {
    module: u16,
    function_definition: u16,
    class_definition: u16,
    call: u16,
    attribute: u16,
    identifier: u16,
    import_statement: u16,
    import_from_statement: u16,
    dotted_name: u16,
    aliased_import: u16,
    assignment: u16,
    expression_statement: u16,
    string: u16,
    string_content: u16,
    list: u16,
    decorated_definition: u16,
    relative_import: u16,
    f_name: u16,
    f_function: u16,
    f_superclasses: u16,
    f_left: u16,
    f_right: u16,
    f_module_name: u16,
    f_alias: u16,
    f_attribute: u16,
    f_object: u16,
    f_body: u16,
}

fn kid(lang: &Language, s: &str) -> u16 {
    lang.id_for_node_kind(s, true)
}
fn fid(lang: &Language, s: &str) -> u16 {
    lang.field_id_for_name(s).map(|f| f.get()).unwrap_or(0)
}

fn children(t: &[N], i: usize) -> impl Iterator<Item = usize> + '_ {
    let end = t[i].end as usize;
    let mut c = i + 1;
    std::iter::from_fn(move || {
        if c >= end {
            return None;
        }
        let r = c;
        c = t[c].end as usize;
        Some(r)
    })
}

fn child_field(t: &[N], i: usize, f: u16) -> Option<usize> {
    children(t, i).find(|&c| t[c].field == f)
}

fn text<'a>(d: &'a Doc, i: usize) -> &'a str {
    let n = &d.tape[i];
    std::str::from_utf8(&d.src[n.sb as usize..n.eb as usize]).unwrap_or("")
}

// Dotted name if the expression is an identifier/attribute chain, else None.
fn dotted(d: &Doc, k: &K, i: usize) -> Option<String> {
    let n = d.tape[i];
    if n.kind == k.identifier {
        return Some(text(d, i).to_string());
    }
    if n.kind == k.attribute {
        let o = child_field(&d.tape, i, k.f_object)?;
        let a = child_field(&d.tape, i, k.f_attribute)?;
        let base = dotted(d, k, o)?;
        return Some(format!("{}.{}", base, text(d, a)));
    }
    None
}

#[derive(Default)]
struct Facts {
    defs: Vec<(u32, u32, u8, String, String, String, bool)>, // file,node,kind(0 fn,1 class),name,qual,scope,is_async
    calls: Vec<(u32, u32, String, String, String)>,        // file,node,scope,callee_full,callee_last
    imports: Vec<(u32, String, String, String, u8)>,       // file,module,name,alias,level
    bases: Vec<(u32, String, String)>,                     // file,class_qual,base
    refs: Vec<(u32, String)>,                              // file,name (raw occurrences)
    assigns: Vec<(u32, String, String, u16)>,              // file,scope,target,rhs kind
    all_names: Vec<(u32, String)>,
}

fn extract(d: &Doc, fileid: u32, k: &K, f: &mut Facts) {
    let t = &d.tape;
    let mut scope: Vec<(u32, String)> = Vec::new(); // (end, qual)
    let mut def_name_nodes: Vec<u32> = Vec::new();
    for i in 0..t.len() {
        while let Some((e, _)) = scope.last() {
            if (i as u32) >= *e {
                scope.pop();
            } else {
                break;
            }
        }
        let n = t[i];
        let cur = scope.last().map(|s| s.1.as_str()).unwrap_or("");
        if n.kind == k.function_definition || n.kind == k.class_definition {
            if let Some(nm) = child_field(t, i, k.f_name) {
                def_name_nodes.push(nm as u32);
                let name = text(d, nm).to_string();
                let qual = if cur.is_empty() { name.clone() } else { format!("{}.{}", cur, name) };
                let is_async = n.kind == k.function_definition && d.src[n.sb as usize..].starts_with(b"async");
                let kind = if n.kind == k.class_definition { 1 } else { 0 };
                if kind == 1 {
                    if let Some(sc) = child_field(t, i, k.f_superclasses) {
                        for c in children(t, sc) {
                            if t[c].named {
                                f.bases.push((fileid, qual.clone(), text(d, c).to_string()));
                            }
                        }
                    }
                }
                f.defs.push((fileid, i as u32, kind, name, qual.clone(), cur.to_string(), is_async));
                scope.push((n.end, qual));
            }
        } else if n.kind == k.call {
            if let Some(fnode) = child_field(t, i, k.f_function) {
                if let Some(full) = dotted(d, k, fnode) {
                    let last = full.rsplit('.').next().unwrap_or("").to_string();
                    f.calls.push((fileid, i as u32, cur.to_string(), full, last));
                }
            }
        } else if n.kind == k.import_statement {
            for c in children(t, i) {
                if t[c].kind == k.dotted_name {
                    f.imports.push((fileid, text(d, c).to_string(), String::new(), String::new(), 0));
                } else if t[c].kind == k.aliased_import {
                    let nm = child_field(t, c, k.f_name).map(|x| text(d, x).to_string()).unwrap_or_default();
                    let al = child_field(t, c, k.f_alias).map(|x| text(d, x).to_string()).unwrap_or_default();
                    f.imports.push((fileid, nm, String::new(), al, 0));
                }
            }
        } else if n.kind == k.import_from_statement {
            let mut module = String::new();
            let mut level = 0u8;
            if let Some(m) = child_field(t, i, k.f_module_name) {
                let s = text(d, m);
                level = s.bytes().take_while(|&b| b == b'.').count() as u8;
                module = s.trim_start_matches('.').to_string();
            }
            for c in children(t, i) {
                if t[c].field != k.f_name {
                    continue;
                }
                if t[c].kind == k.dotted_name {
                    f.imports.push((fileid, module.clone(), text(d, c).to_string(), String::new(), level));
                } else if t[c].kind == k.aliased_import {
                    let nm = child_field(t, c, k.f_name).map(|x| text(d, x).to_string()).unwrap_or_default();
                    let al = child_field(t, c, k.f_alias).map(|x| text(d, x).to_string()).unwrap_or_default();
                    f.imports.push((fileid, module.clone(), nm, al, level));
                }
            }
        } else if n.kind == k.assignment {
            if let Some(l) = child_field(t, i, k.f_left) {
                if t[l].kind == k.identifier {
                    let rk = child_field(t, i, k.f_right).map(|r| t[r].kind).unwrap_or(0);
                    let target = text(d, l).to_string();
                    if target == "__all__" && cur.is_empty() {
                        if let Some(r) = child_field(t, i, k.f_right) {
                            for j in r..t[r].end as usize {
                                if t[j].kind == k.string_content {
                                    f.all_names.push((fileid, text(d, j).to_string()));
                                }
                            }
                        }
                    }
                    f.assigns.push((fileid, cur.to_string(), target, rk));
                }
            }
        } else if n.kind == k.identifier {
            f.refs.push((fileid, text(d, i).to_string()));
        }
    }
    let _ = def_name_nodes;
}

fn module_name(path: &str) -> String {
    let p = path.trim_end_matches(".py");
    let p = p.strip_suffix("/__init__").unwrap_or(p);
    p.replace('/', ".")
}

fn time<T>(label: &str, log: &mut Vec<String>, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let line = format!("{label}\t{ms:.1} ms");
    println!("{line}");
    log.push(line);
    r
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let corpus = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let threads: usize = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(8);
    std::fs::create_dir_all(&out).unwrap();
    let lang: Language = tree_sitter_python::LANGUAGE.into();
    let mut log = Vec::new();

    let mut files = Vec::new();
    collect(&corpus, &mut files);
    println!("files {}", files.len());

    // ---- parse + tape build (parallel); per-file times are thread-local CPU-ish wall
    let wall = Instant::now();
    let chunks: Vec<Vec<PathBuf>> = (0..threads)
        .map(|t| files.iter().skip(t).step_by(threads).cloned().collect())
        .collect();
    let mut docs: Vec<Doc> = std::thread::scope(|s| {
        let hs: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                let lang = lang.clone();
                let corpus = corpus.clone();
                s.spawn(move || {
                    chunk
                        .into_iter()
                        .map(|p| {
                            let src = std::fs::read(&p).unwrap();
                            let (tape, parse_ns, tape_ns, max_depth) = build(&src, &lang);
                            let rel = p.strip_prefix(&corpus).unwrap().to_string_lossy().to_string();
                            Doc { path: rel, src, tape, parse_ns, tape_ns, max_depth }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    docs.sort_by(|a, b| a.path.cmp(&b.path));
    let wall_ms = wall.elapsed().as_secs_f64() * 1e3;
    let nodes: u64 = docs.iter().map(|d| d.tape.len() as u64).sum();
    let named: u64 = docs.iter().map(|d| d.tape.iter().filter(|n| n.named).count() as u64).sum();
    let bytes: u64 = docs.iter().map(|d| d.src.len() as u64).sum();
    let parse_ns: u64 = docs.iter().map(|d| d.parse_ns).sum();
    let tape_ns: u64 = docs.iter().map(|d| d.tape_ns).sum();
    let line = format!(
        "corpus files={} bytes={} nodes={} named={} parse_cpu_ms={:.0} tape_build_cpu_ms={:.0} wall_ms={:.0} threads={} parse_ns_per_node={:.1} tape_ns_per_node={:.1}",
        docs.len(), bytes, nodes, named, parse_ns as f64 / 1e6, tape_ns as f64 / 1e6, wall_ms, threads,
        parse_ns as f64 / nodes as f64, tape_ns as f64 / nodes as f64
    );
    println!("{line}");
    log.push(line);

    // ---- kind histogram, edge triples, per-file stats
    let mut kinds: HashMap<u16, u64> = HashMap::new();
    let mut edges: HashMap<(u16, u16, u16), u64> = HashMap::new();
    for d in &docs {
        for n in &d.tape {
            *kinds.entry(n.kind).or_default() += 1;
            if n.parent != u32::MAX && n.named {
                let p = d.tape[n.parent as usize];
                *edges.entry((p.kind, n.field, n.kind)).or_default() += 1;
            }
        }
    }
    let mut kv: Vec<_> = kinds.iter().collect();
    kv.sort_by(|a, b| b.1.cmp(a.1));
    let mut kf = std::fs::File::create(out.join("kinds.tsv")).unwrap();
    for (k, c) in &kv {
        writeln!(kf, "{}\t{}\t{}", lang.node_kind_for_id(**k).unwrap_or("?"), lang.node_kind_is_named(**k), c).unwrap();
    }
    let mut ev: Vec<_> = edges.iter().collect();
    ev.sort_by(|a, b| b.1.cmp(a.1));
    let mut ef = std::fs::File::create(out.join("edges.tsv")).unwrap();
    for ((p, f, c), n) in ev.iter().take(400) {
        let fname = if *f == 0 { "-" } else { lang.field_name_for_id(*f).unwrap_or("?") };
        writeln!(ef, "{}\t{}\t{}\t{}", lang.node_kind_for_id(*p).unwrap_or("?"), fname, lang.node_kind_for_id(*c).unwrap_or("?"), n).unwrap();
    }
    let interesting = [
        "call", "function_definition", "class_definition", "attribute", "assignment", "string", "identifier",
        "decorated_definition", "comparison_operator", "import_from_statement", "import_statement", "argument_list",
        "return_statement", "raise_statement", "await", "with_statement", "expression_statement",
    ];
    let ids: Vec<u16> = interesting.iter().map(|s| kid(&lang, s)).collect();
    let mut pf = std::fs::File::create(out.join("files.tsv")).unwrap();
    write!(pf, "path\tbytes\tnodes\tnamed\tmax_depth\tparse_us").unwrap();
    for s in interesting {
        write!(pf, "\t{s}").unwrap();
    }
    writeln!(pf).unwrap();
    for d in &docs {
        let mut cnt = vec![0u64; ids.len()];
        for n in &d.tape {
            for (j, id) in ids.iter().enumerate() {
                if n.kind == *id {
                    cnt[j] += 1;
                }
            }
        }
        write!(pf, "{}\t{}\t{}\t{}\t{}\t{}", d.path, d.src.len(), d.tape.len(), d.tape.iter().filter(|n| n.named).count(), d.max_depth, d.parse_ns / 1000).unwrap();
        for c in cnt {
            write!(pf, "\t{c}").unwrap();
        }
        writeln!(pf).unwrap();
    }

    // ---- subtree-size distribution for common anchor kinds (has-scan cost)
    let mut sub = std::fs::File::create(out.join("subtree.tsv")).unwrap();
    for s in ["module", "class_definition", "function_definition", "call", "assignment", "attribute", "decorated_definition"] {
        let id = kid(&lang, s);
        let mut sizes: Vec<u32> = Vec::new();
        for d in &docs {
            for (i, n) in d.tape.iter().enumerate() {
                if n.kind == id {
                    sizes.push(n.end - i as u32);
                }
            }
        }
        sizes.sort();
        let sum: u64 = sizes.iter().map(|&x| x as u64).sum();
        let q = |p: f64| sizes[((sizes.len() - 1) as f64 * p) as usize];
        writeln!(sub, "{s}\tcount={}\tsum_subtree={}\tp50={}\tp90={}\tp99={}\tmax={}", sizes.len(), sum, q(0.5), q(0.9), q(0.99), q(1.0)).unwrap();
    }

    // ---- inside cost: ancestors per node (depth)
    let mut depth_sum = 0u64;
    for d in &docs {
        let mut depth = vec![0u32; d.tape.len()];
        for i in 1..d.tape.len() {
            depth[i] = depth[d.tape[i].parent as usize] + 1;
            depth_sum += depth[i] as u64;
        }
    }
    let line = format!("mean_depth {:.2}", depth_sum as f64 / nodes as f64);
    println!("{line}");
    log.push(line);

    // ---- fact extraction (single thread, timed)
    let k = K {
        module: kid(&lang, "module"),
        function_definition: kid(&lang, "function_definition"),
        class_definition: kid(&lang, "class_definition"),
        call: kid(&lang, "call"),
        attribute: kid(&lang, "attribute"),
        identifier: kid(&lang, "identifier"),
        import_statement: kid(&lang, "import_statement"),
        import_from_statement: kid(&lang, "import_from_statement"),
        dotted_name: kid(&lang, "dotted_name"),
        aliased_import: kid(&lang, "aliased_import"),
        assignment: kid(&lang, "assignment"),
        expression_statement: kid(&lang, "expression_statement"),
        string: kid(&lang, "string"),
        string_content: kid(&lang, "string_content"),
        list: kid(&lang, "list"),
        decorated_definition: kid(&lang, "decorated_definition"),
        relative_import: kid(&lang, "relative_import"),
        f_name: fid(&lang, "name"),
        f_function: fid(&lang, "function"),
        f_superclasses: fid(&lang, "superclasses"),
        f_left: fid(&lang, "left"),
        f_right: fid(&lang, "right"),
        f_module_name: fid(&lang, "module_name"),
        f_alias: fid(&lang, "alias"),
        f_attribute: fid(&lang, "attribute"),
        f_object: fid(&lang, "object"),
        f_body: fid(&lang, "body"),
    };
    let _ = (k.module, k.expression_statement, k.string, k.list, k.decorated_definition, k.relative_import, k.f_body);
    let mut facts = Facts::default();
    let t = Instant::now();
    for (fi, d) in docs.iter().enumerate() {
        extract(d, fi as u32, &k, &mut facts);
    }
    let ex_ms = t.elapsed().as_secs_f64() * 1e3;
    let line = format!(
        "extract ms={:.0} ns_per_node={:.2} defs={} calls={} imports={} bases={} refs={} assigns={} all_names={}",
        ex_ms, ex_ms * 1e6 / nodes as f64, facts.defs.len(), facts.calls.len(), facts.imports.len(), facts.bases.len(),
        facts.refs.len(), facts.assigns.len(), facts.all_names.len()
    );
    println!("{line}");
    log.push(line);

    // ---- largest file per-edit cost: re-extract only that file
    let big = docs.iter().enumerate().max_by_key(|(_, d)| d.tape.len()).unwrap();
    let mut tmp = Facts::default();
    let t = Instant::now();
    extract(big.1, big.0 as u32, &k, &mut tmp);
    let line = format!(
        "largest_file {} nodes={} parse_ms={:.1} extract_ms={:.2}",
        big.1.path, big.1.tape.len(), big.1.parse_ns as f64 / 1e6, t.elapsed().as_secs_f64() * 1e3
    );
    println!("{line}");
    log.push(line);

    // ---- SQLite
    let dbpath = out.join("facts.sqlite");
    let _ = std::fs::rename(&dbpath, out.join("facts.sqlite.old"));
    let mut db = Connection::open(&dbpath).unwrap();
    db.execute_batch(
        "PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA cache_size=-1000000; PRAGMA temp_store=MEMORY;
         CREATE TABLE file(id INTEGER PRIMARY KEY, path TEXT, module TEXT);
         CREATE TABLE def(id INTEGER PRIMARY KEY, file INT, node INT, kind INT, name TEXT, qual TEXT, scope TEXT, is_async INT);
         CREATE TABLE call(id INTEGER PRIMARY KEY, file INT, node INT, scope TEXT, callee TEXT, last TEXT);
         CREATE TABLE imp(id INTEGER PRIMARY KEY, file INT, module TEXT, name TEXT, alias TEXT, level INT);
         CREATE TABLE base(file INT, cls TEXT, base TEXT);
         CREATE TABLE ref(file INT, name TEXT);
         CREATE TABLE asg(file INT, scope TEXT, target TEXT, rk INT);
         CREATE TABLE all_name(file INT, name TEXT);",
    )
    .unwrap();
    time("sqlite_insert_all", &mut log, || {
        let tx = db.transaction().unwrap();
        {
            let mut s = tx.prepare("INSERT INTO file VALUES(?,?,?)").unwrap();
            for (i, d) in docs.iter().enumerate() {
                s.execute(params![i as i64, d.path, module_name(&d.path)]).unwrap();
            }
            let mut s = tx.prepare("INSERT INTO def(file,node,kind,name,qual,scope,is_async) VALUES(?,?,?,?,?,?,?)").unwrap();
            for r in &facts.defs {
                s.execute(params![r.0, r.1, r.2, r.3, r.4, r.5, r.6]).unwrap();
            }
            let mut s = tx.prepare("INSERT INTO call(file,node,scope,callee,last) VALUES(?,?,?,?,?)").unwrap();
            for r in &facts.calls {
                s.execute(params![r.0, r.1, r.2, r.3, r.4]).unwrap();
            }
            let mut s = tx.prepare("INSERT INTO imp(file,module,name,alias,level) VALUES(?,?,?,?,?)").unwrap();
            for r in &facts.imports {
                s.execute(params![r.0, r.1, r.2, r.3, r.4]).unwrap();
            }
            let mut s = tx.prepare("INSERT INTO base VALUES(?,?,?)").unwrap();
            for r in &facts.bases {
                s.execute(params![r.0, r.1, r.2]).unwrap();
            }
            let mut s = tx.prepare("INSERT INTO ref VALUES(?,?)").unwrap();
            for r in &facts.refs {
                s.execute(params![r.0, r.1]).unwrap();
            }
            let mut s = tx.prepare("INSERT INTO asg VALUES(?,?,?,?)").unwrap();
            for r in &facts.assigns {
                s.execute(params![r.0, r.1, r.2, r.3]).unwrap();
            }
            let mut s = tx.prepare("INSERT INTO all_name VALUES(?,?)").unwrap();
            for r in &facts.all_names {
                s.execute(params![r.0, r.1]).unwrap();
            }
        }
        tx.commit().unwrap();
    });
    let q = |db: &Connection, label: &str, log: &mut Vec<String>, sql: &str| {
        let t = Instant::now();
        let v: i64 = db.query_row(sql, [], |r| r.get(0)).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        let line = format!("{label}\t{ms:.1} ms\tresult={v}");
        println!("{line}");
        log.push(line);
    };
    let x = |db: &Connection, label: &str, log: &mut Vec<String>, sql: &str| {
        let t = Instant::now();
        db.execute_batch(sql).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        let line = format!("{label}\t{ms:.1} ms");
        println!("{line}");
        log.push(line);
    };
    // Deduped ref set (fact_ref family as a set per (file,name)).
    x(&db, "ref_dedup_materialize", &mut log, "CREATE TABLE ref_set AS SELECT DISTINCT file, name FROM ref;");
    q(&db, "ref_set_rows", &mut log, "SELECT count(*) FROM ref_set");
    x(&db, "indexes", &mut log,
        "CREATE INDEX def_name ON def(name); CREATE INDEX def_file_qual ON def(file, qual);
         CREATE INDEX call_last ON call(last); CREATE INDEX call_file_scope ON call(file, scope);
         CREATE INDEX ref_set_name ON ref_set(name); CREATE INDEX file_module ON file(module);
         CREATE INDEX imp_file ON imp(file); ANALYZE;");
    // B1 call resolution by unique last name (category D-lite)
    x(&db, "B1_call_resolve_unique_name", &mut log,
        "CREATE TABLE edge AS
           WITH u AS (SELECT name, min(id) AS id FROM def GROUP BY name HAVING count(*) = 1)
           SELECT c.id AS call, cd.id AS caller, u.id AS callee
           FROM call c JOIN u ON u.name = c.last
           LEFT JOIN def cd ON cd.file = c.file AND cd.qual = c.scope;");
    q(&db, "edge_rows", &mut log, "SELECT count(*) FROM edge");
    // B2 import resolution: from M import N -> def in module M named N at top level (category E)
    x(&db, "B2_import_resolve", &mut log,
        "CREATE TABLE imp_res AS
           SELECT i.id AS imp, d.id AS def FROM imp i
           JOIN file f ON f.module = i.module
           JOIN def d ON d.file = f.id AND d.qual = i.name
           WHERE i.level = 0 AND i.name <> '';");
    q(&db, "imp_res_rows", &mut log, "SELECT count(*) FROM imp_res");
    // B2b: module suffix resolution (data-layer imports often relative to package root)
    x(&db, "B2b_import_resolve_suffix", &mut log,
        "CREATE TABLE imp_res2 AS
           SELECT i.id AS imp, d.id AS def FROM imp i
           JOIN file f ON (f.module = i.module OR f.module LIKE '%.' || i.module)
           JOIN def d ON d.file = f.id AND d.qual = i.name
           WHERE i.name <> '';");
    q(&db, "imp_res2_rows", &mut log, "SELECT count(*) FROM imp_res2");
    // B3 reachability (category G): roots = public top-level defs in public modules + __all__
    x(&db, "B3_edges_index", &mut log, "CREATE INDEX edge_caller ON edge(caller);");
    x(&db, "B3_reach", &mut log,
        "CREATE TABLE reach AS
           WITH RECURSIVE r(id) AS (
             SELECT id FROM def WHERE scope = '' AND name NOT LIKE '\\_%' ESCAPE '\\'
             UNION
             SELECT e.callee FROM edge e JOIN r ON e.caller = r.id WHERE e.callee IS NOT NULL
             UNION
             SELECT d.id FROM def d JOIN r ON d.file = (SELECT file FROM def WHERE id = r.id) AND d.scope = (SELECT qual FROM def WHERE id = r.id)
           ) SELECT id FROM r;");
    q(&db, "reach_rows", &mut log, "SELECT count(*) FROM reach");
    // B4 dead internal symbols (category G/I anti-join): private defs never referenced by name outside own def
    q(&db, "B4_dead_internal_antijoin", &mut log,
        "SELECT count(*) FROM def d WHERE d.name LIKE '\\_%' ESCAPE '\\' AND d.name NOT LIKE '\\_\\_%' ESCAPE '\\'
           AND NOT EXISTS (SELECT 1 FROM ref_set r WHERE r.name = d.name AND r.file <> d.file)
           AND NOT EXISTS (SELECT 1 FROM call c WHERE c.last = d.name AND c.file = d.file)");
    // B5 per-source label propagation (category H-like): K sources, closure over call edges
    for kk in [16, 128, 1024] {
        x(&db, &format!("B5_label_prop_K{kk}"), &mut log, &format!(
            "DROP TABLE IF EXISTS lp; CREATE TABLE lp AS
               WITH RECURSIVE src(id) AS (SELECT id FROM def WHERE kind = 0 ORDER BY id LIMIT {kk}),
               p(s, id, d) AS (SELECT id, id, 0 FROM src
                 UNION SELECT p.s, e.callee, p.d + 1 FROM p JOIN edge e ON e.caller = p.id WHERE e.callee IS NOT NULL AND p.d < 64)
               SELECT DISTINCT s, id FROM p;"));
        q(&db, &format!("B5_rows_K{kk}"), &mut log, "SELECT count(*) FROM lp");
    }
    // B6 hierarchy closure (category F): class -> base by unique name, transitive
    x(&db, "B6_hierarchy_closure", &mut log,
        "CREATE TABLE hier AS
           WITH ub AS (SELECT name, min(id) AS id FROM def WHERE kind = 1 GROUP BY name HAVING count(*) = 1),
           be AS (SELECT c.id AS cls, ub.id AS base FROM base b JOIN def c ON c.file = b.file AND c.qual = b.cls AND c.kind = 1
                  JOIN ub ON ub.name = CASE WHEN instr(b.base, '.') > 0 THEN substr(b.base, length(rtrim(b.base, replace(b.base, '.', ''))) + 1) ELSE b.base END),
           RECURSIVE_DUMMY AS (SELECT 1)
           SELECT * FROM be;
         CREATE INDEX hier_cls ON hier(cls);
         CREATE TABLE hier_tc AS WITH RECURSIVE t(cls, anc) AS (SELECT cls, base FROM hier UNION SELECT t.cls, h.base FROM t JOIN hier h ON h.cls = t.anc) SELECT * FROM t;");
    q(&db, "hier_rows", &mut log, "SELECT count(*) FROM hier");
    q(&db, "hier_tc_rows", &mut log, "SELECT count(*) FROM hier_tc");
    // B7 count/dedupe (categories A/B): per-file per-callee counts, group-by
    q(&db, "B7_groupby_file_callee_count", &mut log,
        "SELECT count(*) FROM (SELECT file, callee, count(*) FROM call GROUP BY file, callee HAVING count(*) > 1)");
    // Per-edit: delete+reinsert facts of the largest file, then re-resolve edges of that file
    let bf = big.0 as i64;
    x(&db, "edit_delete_reinsert_largest", &mut log, &format!(
        "BEGIN; CREATE TEMP TABLE c2 AS SELECT * FROM call WHERE file = {bf}; DELETE FROM call WHERE file = {bf};
         INSERT INTO call SELECT * FROM c2; DROP TABLE c2;
         CREATE TEMP TABLE r2 AS SELECT * FROM ref_set WHERE file = {bf}; DELETE FROM ref_set WHERE file = {bf};
         INSERT INTO ref_set SELECT * FROM r2; DROP TABLE r2; COMMIT;"));
    q(&db, "edit_reresolve_edges_largest", &mut log, &format!(
        "WITH u AS (SELECT name, min(id) AS id FROM def GROUP BY name HAVING count(*) = 1)
         SELECT count(*) FROM call c JOIN u ON u.name = c.last WHERE c.file = {bf}"));
    q(&db, "edit_reach_full_recompute", &mut log,
        "WITH RECURSIVE r(id) AS (
             SELECT id FROM def WHERE scope = '' AND name NOT LIKE '\\_%' ESCAPE '\\'
             UNION SELECT e.callee FROM edge e JOIN r ON e.caller = r.id WHERE e.callee IS NOT NULL
           ) SELECT count(*) FROM r");
    let sz = std::fs::metadata(&dbpath).map(|m| m.len()).unwrap_or(0);
    let line = format!("sqlite_file_bytes {sz}");
    println!("{line}");
    log.push(line);
    std::fs::write(out.join("measure.txt"), log.join("\n") + "\n").unwrap();
}
