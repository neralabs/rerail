use rayon::prelude::*;
use regex_automata::dfa::{dense, Automaton};
use regex_automata::{Anchored, Input, MatchKind};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;

fn read_json(p: &str) -> Value { serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap() }
fn files_of(corpus: &str, list: &str) -> (Vec<String>, Vec<Vec<u8>>) {
    let names: Vec<String> = std::fs::read_to_string(list).unwrap().lines().filter(|l| !l.is_empty()).map(String::from).collect();
    let texts: Vec<Vec<u8>> = names.par_iter().map(|n| std::fs::read(format!("{corpus}/{n}")).unwrap_or_default()).collect();
    (names, texts)
}

// ---------------- features ----------------
fn hir_props(p: &str) -> Value {
    use regex_syntax::hir::Look;
    let hir = match regex_syntax::ParserBuilder::new().build().parse(p) { Ok(h) => h, Err(e) => return json!({"parse_error": e.to_string()}) };
    let props = hir.properties();
    let ls = props.look_set();
    let mut looks = vec![];
    for (l, n) in [(Look::Start, "\\A"), (Look::End, "\\z"), (Look::StartLF, "(?m)^"), (Look::EndLF, "(?m)$"), (Look::WordAscii, "(?-u)\\b"), (Look::WordAsciiNegate, "(?-u)\\B"), (Look::WordUnicode, "\\b"), (Look::WordUnicodeNegate, "\\B")] {
        if ls.contains(l) { looks.push(n); }
    }
    json!({"looks": looks, "captures": props.explicit_captures_len(), "min_len": props.minimum_len(), "max_len": props.maximum_len(), "literal": props.is_literal(), "alt_literal": props.is_alternation_literal(), "utf8": props.is_utf8()})
}
fn dfa_one(p: &str, uwb: bool, limit: usize) -> Result<(usize, f64), String> {
    let t = Instant::now();
    let d = dense::Builder::new()
        .configure(dense::Config::new().unicode_word_boundary(uwb).dfa_size_limit(Some(limit)).determinize_size_limit(Some(limit)))
        .build(p).map_err(|e| e.to_string())?;
    Ok((d.memory_usage(), t.elapsed().as_secs_f64()))
}
fn features(pats_path: &str) {
    let v = read_json(pats_path);
    let mut out = vec![];
    for item in v.as_array().unwrap() {
        let p = item["pattern"].as_str().unwrap();
        let re_ok = regex::Regex::new(p).map(|_| true).map_err(|e| e.to_string());
        let strict = dfa_one(p, false, 64 << 20);
        let heur = dfa_one(p, true, 64 << 20);
        let ascii = dfa_one(&format!("(?-u:\\b)|{}", ""), false, 1 << 20).ok(); let _ = ascii;
        let ascii_wb = dfa_one(&p.replace("\\b", "(?-u:\\b)").replace("\\B", "(?-u:\\B)"), false, 64 << 20);
        out.push(json!({"id": item["id"], "host": item["host"], "pattern": p, "regex_ok": re_ok.is_ok(), "regex_err": re_ok.err(),
            "hir": hir_props(p),
            "dfa_strict": strict.as_ref().map(|x| x.0).ok(), "dfa_strict_err": strict.as_ref().err(),
            "dfa_uwb_heuristic": heur.as_ref().map(|x| x.0).ok(), "dfa_uwb_err": heur.as_ref().err(),
            "dfa_ascii_wb": ascii_wb.as_ref().map(|x| x.0).ok(), "dfa_ascii_err": ascii_wb.err(),
            "dfa_build_s": 0}));
    }
    println!("{}", serde_json::to_string(&out).unwrap());
}

// ---------------- nodes ----------------
struct DocNodes { idents: Vec<(u32, u32)>, large: Vec<(u16, u32, u32)> }
const LARGE: [&str; 6] = ["module", "function_definition", "conditional_expression", "assignment", "string", "dotted_name"];
fn collect(src: &[u8], parser: &mut tree_sitter::Parser) -> DocNodes {
    let tree = parser.parse(src, None).unwrap();
    let mut c = tree.walk();
    let mut d = DocNodes { idents: vec![], large: vec![] };
    loop {
        let n = c.node();
        let k = n.kind();
        if k == "identifier" { d.idents.push((n.start_byte() as u32, n.end_byte() as u32)); }
        if let Some(i) = LARGE.iter().position(|x| *x == k) { d.large.push((i as u16, n.start_byte() as u32, n.end_byte() as u32)); }
        if c.goto_first_child() { continue; }
        loop {
            if c.goto_next_sibling() { break; }
            if !c.goto_parent() { return d; }
        }
    }
}

struct MultiDfa { dfa: dense::DFA<Vec<u32>>, words: usize }
impl MultiDfa {
    fn build(pats: &[String], uwb: bool) -> Result<MultiDfa, String> {
        let ascii = !uwb;
        let d = dense::Builder::new()
            .syntax(regex_automata::util::syntax::Config::new().unicode(!ascii).utf8(!ascii))
            .configure(dense::Config::new().match_kind(MatchKind::All).unicode_word_boundary(uwb).dfa_size_limit(Some(2 << 30)).determinize_size_limit(Some(2 << 30)))
            .build_many(pats).map_err(|e| e.to_string())?;
        Ok(MultiDfa { dfa: d, words: (pats.len() + 63) / 64 })
    }
    /// bitset of patterns that match anywhere in hay; None on quit (non-ASCII under \b heuristic)
    fn which(&self, hay: &[u8], out: &mut [u64], cache: &mut HashMap<u32, Vec<u64>>) -> bool {
        let d = &self.dfa;
        let mut s = d.start_state_forward(&Input::new(hay).anchored(Anchored::No)).unwrap();
        let mut acc = |s: regex_automata::util::primitives::StateID, out: &mut [u64]| {
            let m = cache.entry(s.as_u32()).or_insert_with(|| {
                let mut v = vec![0u64; self.words];
                for i in 0..d.match_len(s) { let p = d.match_pattern(s, i).as_usize(); v[p / 64] |= 1 << (p % 64); }
                v
            });
            for (o, x) in out.iter_mut().zip(m.iter()) { *o |= *x; }
        };
        for &b in hay {
            s = d.next_state(s, b);
            if d.is_special_state(s) {
                if d.is_match_state(s) { acc(s, out); }
                else if d.is_quit_state(s) { return false; }
                else if d.is_dead_state(s) { return true; }
            }
        }
        s = d.next_eoi_state(s);
        if d.is_match_state(s) { acc(s, out); }
        true
    }
}

fn nodes(corpus: &str, list: &str, hosts_path: &str, sample_every: usize) {
    let hosts = read_json(hosts_path);
    let get = |k: &str| -> Vec<String> { hosts[k].as_array().map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect()).unwrap_or_default() };
    let ident = get("node:identifier");
    let none = get("node:none");
    let module = get("node:module");
    let fdef = get("node:function_definition");
    let (names, texts) = files_of(corpus, list);
    let t = Instant::now();
    let docs: Vec<DocNodes> = texts.par_iter().map_init(|| { let mut p = tree_sitter::Parser::new(); p.set_language(&tree_sitter_python::LANGUAGE.into()).unwrap(); p }, |p, s| collect(s, p)).collect();
    let parse_s = t.elapsed().as_secs_f64();
    let n_occ: usize = docs.iter().map(|d| d.idents.len()).sum();
    let occ_bytes: usize = docs.iter().flat_map(|d| d.idents.iter()).map(|(a, b)| (b - a) as usize).sum();
    let mut large_stats = vec![(0usize, 0usize); LARGE.len()];
    for d in &docs { for (k, a, b) in &d.large { large_stats[*k as usize].0 += 1; large_stats[*k as usize].1 += (b - a) as usize; } }
    eprintln!("parsed {} files in {parse_s:.2}s (parallel); ident occ {n_occ}, bytes {occ_bytes}", names.len());

    // global interning (single thread)
    let t = Instant::now();
    let mut intern: HashMap<&[u8], u32> = HashMap::with_capacity(1 << 20);
    let mut doc_ids: Vec<Vec<u32>> = Vec::with_capacity(docs.len());
    for (d, s) in docs.iter().zip(texts.iter()) {
        let mut ids = Vec::with_capacity(d.idents.len());
        for (a, b) in &d.idents { let k = &s[*a as usize..*b as usize]; let n = intern.len() as u32; ids.push(*intern.entry(k).or_insert(n)); }
        doc_ids.push(ids);
    }
    let intern_s = t.elapsed().as_secs_f64();
    let mut distinct: Vec<&[u8]> = vec![&[][..]; intern.len()];
    for (k, v) in &intern { distinct[*v as usize] = k; }
    let per_doc_distinct: usize = doc_ids.iter().map(|ids| { let mut v = ids.clone(); v.sort_unstable(); v.dedup(); v.len() }).sum();
    let distinct_bytes: usize = distinct.iter().map(|k| k.len()).sum();
    eprintln!("distinct {} ({} bytes), per-doc distinct sum {per_doc_distinct}, intern {intern_s:.3}s", distinct.len(), distinct_bytes);

    let mut res = json!({"files": names.len(), "parse_s_parallel": parse_s, "ident_occurrences": n_occ, "ident_occ_bytes": occ_bytes,
        "distinct_names": distinct.len(), "distinct_bytes": distinct_bytes, "per_doc_distinct_sum": per_doc_distinct, "intern_global_s": intern_s,
        "large": LARGE.iter().zip(large_stats.iter()).map(|(k, (n, b))| json!({"kind": k, "nodes": n, "bytes": b})).collect::<Vec<_>>()});

    let shapes = read_json(&format!("{}/pattern_shapes.json", std::path::Path::new(hosts_path).parent().unwrap().display()));
    for (label, pats) in [("identifier", ident.clone()), ("identifier+none", [ident.clone(), none.clone()].concat())] {
        let res_re: Vec<regex::Regex> = pats.iter().map(|p| regex::Regex::new(p).unwrap()).collect();
        // A. per occurrence (sampled files)
        let t = Instant::now();
        let mut evals = 0usize; let mut hitsa = 0usize;
        for (i, (d, s)) in docs.iter().zip(texts.iter()).enumerate() {
            if i % sample_every != 0 { continue; }
            for (a, b) in &d.idents {
                let txt = unsafe { std::str::from_utf8_unchecked(&s[*a as usize..*b as usize]) };
                for r in &res_re { evals += 1; if r.is_match(txt) { hitsa += 1; } }
            }
        }
        let a_s = t.elapsed().as_secs_f64();
        let ns_per_eval = a_s * 1e9 / evals as f64;
        // B. per distinct, one regex at a time
        let t = Instant::now();
        let mut bits = vec![vec![0u64; (pats.len() + 63) / 64]; distinct.len()];
        for (pi, r) in res_re.iter().enumerate() {
            for (ni, k) in distinct.iter().enumerate() {
                if r.is_match(unsafe { std::str::from_utf8_unchecked(k) }) { bits[ni][pi / 64] |= 1 << (pi % 64); }
            }
        }
        let b_s = t.elapsed().as_secs_f64();
        let nonzero = bits.iter().filter(|r| r.iter().any(|w| *w != 0)).count();
        let set_bits: u32 = bits.iter().flat_map(|r| r.iter()).map(|w| w.count_ones()).sum();
        // C. RegexSet
        let t = Instant::now();
        let set = regex::RegexSet::new(&pats).unwrap();
        let set_build = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let mut set_mismatch = 0usize;
        for (ni, k) in distinct.iter().enumerate() {
            let m = set.matches(unsafe { std::str::from_utf8_unchecked(k) });
            let mut row = vec![0u64; (pats.len() + 63) / 64];
            for p in m.iter() { row[p / 64] |= 1 << (p % 64); }
            if row != bits[ni] { set_mismatch += 1; }
        }
        let c_s = t.elapsed().as_secs_f64();
        // C'. RegexSet parallel
        let t = Instant::now();
        let _cnt: usize = distinct.par_iter().map(|k| set.matches(unsafe { std::str::from_utf8_unchecked(k) }).len()).sum();
        let c_par = t.elapsed().as_secs_f64();
        // D. multi-pattern dense DFA
        let mut dfa_res = json!(null);
        let mut dfa_all = serde_json::Map::new();
        for uwb in if label == "identifier" { vec![false, true] } else { vec![] } {
            let t = Instant::now();
            match MultiDfa::build(&pats, uwb) {
                Ok(m) => {
                    let build = t.elapsed().as_secs_f64();
                    let mut cache = HashMap::new();
                    let t = Instant::now();
                    let mut mism = 0usize; let mut quits = 0usize;
                    for (ni, k) in distinct.iter().enumerate() {
                        let mut row = vec![0u64; m.words];
                        if !m.which(k, &mut row, &mut cache) { quits += 1; continue; }
                        if row != bits[ni] { mism += 1; }
                    }
                    let run = t.elapsed().as_secs_f64();
                    dfa_res = json!({"build_s": build, "memory_bytes": m.dfa.memory_usage(), "run_s": run, "mismatch": mism, "quits_nonascii": quits, "match_state_masks": cache.len()});
                }
                Err(e) => { dfa_res = json!({"error": e, "build_s": t.elapsed().as_secs_f64()}); }
            }
            dfa_all.insert(if uwb { "unicode".into() } else { "ascii".into() }, dfa_res.clone());
            eprintln!("multi dfa uwb={uwb}: {dfa_res}");
        }
        let dfa_res = Value::Object(dfa_all);
        // E. hybrid: exact literal sets via hash on the name, the rest via one multi DFA
        let t = Instant::now();
        let mut lit: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut rest_idx = vec![]; let mut rest = vec![];
        for (pi, p) in pats.iter().enumerate() {
            let sh = &shapes[p.as_str()];
            if sh["shape"].as_str() == Some("exact") { for l in sh["literals"].as_array().unwrap() { lit.entry(l.as_str().unwrap()).or_default().push(pi); } }
            else { rest_idx.push(pi); rest.push(p.clone()); }
        }
        let md = MultiDfa::build(&rest, false).unwrap();
        let hyb_build = t.elapsed().as_secs_f64();
        let t = Instant::now(); let mut cache = HashMap::new(); let mut hmis = 0usize;
        for (ni, k) in distinct.iter().enumerate() {
            let mut row = vec![0u64; (pats.len() + 63) / 64];
            if let Some(v) = lit.get(unsafe { std::str::from_utf8_unchecked(k) }) { for &p in v { row[p / 64] |= 1 << (p % 64); } }
            let mut r2 = vec![0u64; md.words];
            md.which(k, &mut r2, &mut cache);
            for (j, &pi) in rest_idx.iter().enumerate() { if r2[j / 64] >> (j % 64) & 1 == 1 { row[pi / 64] |= 1 << (pi % 64); } }
            if row != bits[ni] { hmis += 1; }
        }
        let hyb_run = t.elapsed().as_secs_f64();
        let hybrid = json!({"exact_patterns": pats.len() - rest.len(), "literals": lit.len(), "dfa_patterns": rest.len(), "build_s": hyb_build, "dfa_memory_bytes": md.dfa.memory_usage(), "run_s": hyb_run, "mismatch": hmis});
        eprintln!("hybrid {label}: {hybrid}");
        // per-pattern match frequency (top 10)
        let mut freq: Vec<(usize, usize)> = (0..pats.len()).map(|pi| (bits.iter().filter(|r| r[pi / 64] >> (pi % 64) & 1 == 1).count(), pi)).collect();
        freq.sort_unstable_by(|a, b| b.cmp(a));
        let top: Vec<Value> = freq.iter().take(10).map(|(n, pi)| json!({"pattern": pats[*pi], "names": n})).collect();
        res[label] = json!({"hybrid": hybrid, "top_patterns": top, "patterns": pats.len(), "per_occurrence": {"sample_every": sample_every, "evals": evals, "seconds": a_s, "ns_per_eval": ns_per_eval, "hits": hitsa,
                "extrapolated_full_s": ns_per_eval * 1e-9 * (n_occ * pats.len()) as f64},
            "per_distinct_one_at_a_time_s": b_s, "names_with_any_match": nonzero, "set_bits": set_bits,
            "bitset_table_bytes_dense": distinct.len() * ((pats.len() + 63) / 64) * 8,
            "regexset": {"build_s": set_build, "run_s": c_s, "run_parallel_s": c_par, "mismatch": set_mismatch},
            "multi_dfa": dfa_res});
        eprintln!("{label}: {}", res[label]);
    }
    // module-hosted regexes over whole file text: one at a time vs set
    let mres: Vec<regex::Regex> = module.iter().map(|p| regex::Regex::new(p).unwrap()).collect();
    let t = Instant::now();
    let mut mh = 0usize;
    for s in &texts { let s = String::from_utf8_lossy(s); for r in &mres { if r.is_match(&s) { mh += 1; } } }
    let m1 = t.elapsed().as_secs_f64();
    let mset = regex::RegexSet::new(&module).unwrap();
    let t = Instant::now();
    let mut mh2 = 0usize;
    for s in &texts { let s = String::from_utf8_lossy(s); mh2 += mset.matches(&s).len(); }
    let m2 = t.elapsed().as_secs_f64();
    // function_definition anchored patterns per node
    let fres: Vec<regex::Regex> = fdef.iter().map(|p| regex::Regex::new(p).unwrap()).collect();
    let t = Instant::now();
    let mut fh = 0usize; let mut fn_nodes = 0usize;
    for (d, s) in docs.iter().zip(texts.iter()) {
        for (k, a, b) in &d.large { if *k == 1 { fn_nodes += 1; let txt = String::from_utf8_lossy(&s[*a as usize..*b as usize]); for r in &fres { if r.is_match(&txt) { fh += 1; } } } }
    }
    let f1 = t.elapsed().as_secs_f64();
    res["module_hosted"] = json!({"patterns": module.len(), "one_at_a_time_s": m1, "hits": mh, "regexset_s": m2, "hits_set": mh2});
    res["function_definition_hosted"] = json!({"patterns": fdef.len(), "nodes": fn_nodes, "seconds": f1, "hits": fh});
    println!("{}", serde_json::to_string_pretty(&res).unwrap());
}

// ---------------- text ----------------
struct Clause { pat: regex::Regex, lines: bool, first: bool, lf: bool, li: Vec<regex::Regex>, le: Vec<regex::Regex>, ci: Vec<regex::Regex>, ce: Vec<regex::Regex>, files: Vec<u32>, path: bool }
fn rv(v: &Value) -> Vec<regex::Regex> { v.as_array().map(|a| a.iter().map(|x| regex::Regex::new(x.as_str().unwrap()).unwrap()).collect()).unwrap_or_default() }
fn load_clauses(p: &str) -> (Vec<Clause>, Vec<String>) {
    let v = read_json(p);
    let mut out = vec![]; let mut pats = vec![];
    for c in v.as_array().unwrap() {
        let pat = c["pattern"].as_str().unwrap().to_string();
        let l = &c["lines"];
        out.push(Clause { pat: regex::Regex::new(&pat).unwrap(), lines: !l.is_null(), first: l["first"].as_bool().unwrap_or(false), lf: l["terminators"].as_str() == Some("lf"),
            li: rv(&l["include"]), le: rv(&l["exclude"]), ci: rv(&c["content"]["include"]), ce: rv(&c["content"]["exclude"]),
            files: c["files"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect(), path: c["source"].as_str() == Some("file.path") });
        pats.push(pat);
    }
    (out, pats)
}
/// today's per-clause semantics (no line-state / indentation filters); returns number of hits
fn run_clause(c: &Clause, hay: &str) -> usize {
    if !c.ci.iter().all(|r| r.is_match(hay)) || c.ce.iter().any(|r| r.is_match(hay)) { return 0; }
    let mut n = 0;
    let mut scan = |text: &str| {
        if c.lines && (!c.li.iter().all(|r| r.is_match(text)) || c.le.iter().any(|r| r.is_match(text))) { return; }
        for _m in c.pat.find_iter(text) { n += 1; if c.first { break; } }
    };
    if c.lines {
        let mut start = 0; let mut chars = hay.char_indices().peekable();
        while let Some((at, ch)) = chars.next() {
            if ch == '\n' || (!c.lf && matches!(ch, '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}')) {
                scan(&hay[start..at]); start = at + ch.len_utf8();
                if ch == '\r' && chars.peek().is_some_and(|(_, c)| *c == '\n') { start += 1; chars.next(); }
            }
        }
        if start < hay.len() { scan(&hay[start..]); }
    } else { scan(hay); }
    n
}
fn varint(mut x: u32, out: &mut Vec<u8>) { while x >= 0x80 { out.push((x as u8) | 0x80); x >>= 7; } out.push(x as u8); }
fn eval_q(q: &Value, post: &HashMap<u32, Vec<u32>>, universe: &[u32]) -> Vec<u32> {
    // returns sorted file ids; universe = all files
    if q.get("all").is_some() { return universe.to_vec(); }
    if let Some(tris) = q.get("tri").and_then(|v| v.as_array()) {
        let mut lists: Vec<&Vec<u32>> = vec![];
        for t in tris { let b = t.as_str().unwrap().as_bytes(); let k = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32; match post.get(&k) { Some(l) => lists.push(l), None => return vec![] } }
        lists.sort_by_key(|l| l.len());
        let mut cur = lists[0].clone();
        for l in &lists[1..] { cur.retain(|x| l.binary_search(x).is_ok()); if cur.is_empty() { break; } }
        return cur;
    }
    let args: Vec<Vec<u32>> = q["args"].as_array().unwrap().iter().map(|a| eval_q(a, post, universe)).collect();
    match q["op"].as_str().unwrap() {
        "and" => { let mut it = args.into_iter(); let mut cur = it.next().unwrap(); for l in it { cur.retain(|x| l.binary_search(x).is_ok()); } cur }
        _ => { let mut cur: Vec<u32> = args.into_iter().flatten().collect(); cur.sort_unstable(); cur.dedup(); cur }
    }
}
fn text(corpus: &str, list: &str, clauses_p: &str, queries_p: &str, reps: usize) {
    let (names, raw) = files_of(corpus, list);
    let texts: Vec<String> = raw.iter().map(|b| String::from_utf8_lossy(b).into_owned()).collect();
    let total_bytes: usize = texts.iter().map(|t| t.len()).sum();
    let (cl, pats) = load_clauses(clauses_p);
    let content: Vec<usize> = (0..cl.len()).filter(|i| !cl[*i].path).collect();
    // A. today: per clause, per in-scope file (single thread)
    let mut a_best = f64::MAX; let mut hits_a = vec![0usize; cl.len()]; let mut bytes_a = 0usize; let mut hitfiles_a = vec![vec![]; cl.len()];
    for _ in 0..reps {
        let t = Instant::now(); bytes_a = 0;
        for &i in &content { let c = &cl[i]; hits_a[i] = 0; hitfiles_a[i].clear();
            for &f in &c.files { bytes_a += texts[f as usize].len(); let h = run_clause(c, &texts[f as usize]); if h > 0 { hits_a[i] += h; hitfiles_a[i].push(f); } } }
        a_best = a_best.min(t.elapsed().as_secs_f64());
    }
    // A'. today parallel over files (per clause loop inside)
    let t = Instant::now();
    let _x: usize = content.par_iter().map(|&i| cl[i].files.par_iter().map(|&f| run_clause(&cl[i], &texts[f as usize])).sum::<usize>()).sum();
    let a_par = t.elapsed().as_secs_f64();
    // B. one multi-pattern pass per file (RegexSet over whole file, (?m) so ^ works per line), then exact per-line confirm only for set hits
    let cpats: Vec<String> = content.iter().map(|&i| format!("(?m){}", pats[i])).collect();
    let t = Instant::now(); let set = regex::RegexSet::new(&cpats).unwrap(); let set_build = t.elapsed().as_secs_f64();
    let mut file_clauses: Vec<Vec<usize>> = vec![vec![]; texts.len()];
    for (j, &i) in content.iter().enumerate() { for &f in &cl[i].files { file_clauses[f as usize].push(j); } }
    let mut b_best = f64::MAX; let mut b_miss = 0usize; let mut b_bytes = 0usize; let mut b_confirm = 0usize;
    for _ in 0..reps {
        let t = Instant::now(); let mut found = vec![0usize; cl.len()]; b_bytes = 0; b_confirm = 0;
        for (f, cs) in file_clauses.iter().enumerate() {
            if cs.is_empty() { continue; }
            b_bytes += texts[f].len();
            let m = set.matches(&texts[f]);
            for &j in cs { if m.matched(j) { b_confirm += 1; let i = content[j]; found[i] += run_clause(&cl[i], &texts[f]); } }
        }
        b_best = b_best.min(t.elapsed().as_secs_f64());
        b_miss = content.iter().filter(|&&i| found[i] != hits_a[i]).count();
    }
    // C. trigram index build
    let t = Instant::now();
    let per_file: Vec<Vec<u32>> = raw.par_iter().map(|b| { let mut v: Vec<u32> = b.windows(3).map(|w| ((w[0] as u32) << 16) | ((w[1] as u32) << 8) | w[2] as u32).collect(); v.sort_unstable(); v.dedup(); v }).collect();
    let extract_par = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let _ser: Vec<Vec<u32>> = raw.iter().map(|b| { let mut v: Vec<u32> = b.windows(3).map(|w| ((w[0] as u32) << 16) | ((w[1] as u32) << 8) | w[2] as u32).collect(); v.sort_unstable(); v.dedup(); v }).collect();
    let extract_ser = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let mut post: HashMap<u32, Vec<u32>> = HashMap::new();
    for (f, v) in per_file.iter().enumerate() { for k in v { post.entry(*k).or_default().push(f as u32); } }
    let merge = t.elapsed().as_secs_f64();
    let mut enc = vec![]; let n_post: usize = post.values().map(|l| l.len()).sum();
    for l in post.values() { let mut prev = 0u32; for (i, x) in l.iter().enumerate() { varint(if i == 0 { *x } else { x - prev }, &mut enc); prev = *x; } }
    let dict_bytes = post.len() * (3 + 4 + 4);
    // also a per-file bitmap alternative is not built; dictionary = key(3) + offset(4) + count(4)
    // D. trigram-prefiltered verify
    let qs = read_json(queries_p);
    // raw (gated) pattern hits per clause: files where content gates pass and the main pattern matches anywhere ((?m))
    let raw_hits: Vec<Vec<u32>> = (0..cl.len()).map(|i| {
        if cl[i].path { return vec![]; }
        let r = regex::Regex::new(&format!("(?m){}", pats[i])).unwrap(); let c = &cl[i];
        c.files.par_iter().filter(|&&f| { let t = &texts[f as usize]; c.ci.iter().all(|g| g.is_match(t)) && !c.ce.iter().any(|g| g.is_match(t)) && r.is_match(t) }).cloned().collect()
    }).collect();
    let universe: Vec<u32> = (0..texts.len() as u32).collect();
    let mut per_clause = vec![];
    let mut c_best = f64::MAX; let mut c_miss = 0usize; let mut cand_total = 0usize; let mut cand_bytes = 0usize; let mut q_time_best = f64::MAX;
    for rep in 0..reps {
        let t = Instant::now(); let mut qt = 0f64; cand_total = 0; cand_bytes = 0; c_miss = 0;
        for &i in &content {
            let c = &cl[i];
            let tq = Instant::now();
            let cand = eval_q(&qs[i]["query"], &post, &universe);
            let scope = &c.files; // sorted
            let cand: Vec<u32> = cand.into_iter().filter(|f| scope.binary_search(f).is_ok()).collect();
            qt += tq.elapsed().as_secs_f64();
            let mut h = 0; let mut hf = 0;
            for &f in &cand { cand_bytes += texts[f as usize].len(); let x = run_clause(c, &texts[f as usize]); h += x; if x > 0 { hf += 1; } }
            cand_total += cand.len();
            if h != hits_a[i] { c_miss += 1; }
            let unsound = raw_hits[i].iter().filter(|f| cand.binary_search(f).is_err()).count();
            if rep == 0 { per_clause.push(json!({"idx": i, "raw_hit_files": raw_hits[i].len(), "unsound": unsound, "scope_files": scope.len(), "candidates": cand.len(), "hit_files": hitfiles_a[i].len(), "hits": hits_a[i], "hits_trigram": h, "hit_files_trigram": hf})); }
        }
        c_best = c_best.min(t.elapsed().as_secs_f64()); q_time_best = q_time_best.min(qt);
    }
    let scope_pairs: usize = content.iter().map(|&i| cl[i].files.len()).sum();
    let res = json!({"files": names.len(), "corpus_bytes": total_bytes, "content_clauses": content.len(), "scope_file_pairs": scope_pairs,
        "today_per_clause": {"seconds_1thread": a_best, "seconds_parallel": a_par, "bytes_scanned": bytes_a, "GBps": bytes_a as f64 / a_best / 1e9},
        "one_pass_regexset": {"build_s": set_build, "seconds_1thread": b_best, "bytes_scanned": b_bytes, "confirm_pairs": b_confirm, "clauses_mismatch": b_miss},
        "trigram_index": {"extract_parallel_s": extract_par, "extract_1thread_s": extract_ser, "merge_s": merge, "distinct_trigrams": post.len(), "postings": n_post,
            "postings_varint_bytes": enc.len(), "dictionary_bytes": dict_bytes, "bytes_per_source_byte": (enc.len() + dict_bytes) as f64 / total_bytes as f64},
        "trigram_prefilter": {"seconds_1thread": c_best, "query_eval_s": q_time_best, "candidate_pairs": cand_total, "candidate_bytes": cand_bytes, "clauses_mismatch": c_miss},
        "per_clause": per_clause});
    println!("{}", serde_json::to_string_pretty(&res).unwrap());
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a[1].as_str() {
        "features" => features(&a[2]),
        "nodes" => nodes(&a[2], &a[3], &a[4], a[5].parse().unwrap()),
        "text" => text(&a[2], &a[3], &a[4], &a[5], a[6].parse().unwrap()),
        "split" => {
            let (_n, raw) = files_of(&a[2], &a[3]);
            let texts: Vec<String> = raw.iter().map(|b| String::from_utf8_lossy(b).into_owned()).collect();
            let (cl, pats) = load_clauses(&a[4]);
            let content: Vec<usize> = (0..cl.len()).filter(|i| !cl[*i].path).collect();
            let wh: Vec<regex::Regex> = pats.iter().map(|p| regex::Regex::new(&format!("(?m){p}")).unwrap()).collect();
            let t = Instant::now(); let mut h = 0usize; let mut by = 0usize;
            for &i in &content { for &f in &cl[i].files { by += texts[f as usize].len(); if wh[i].is_match(&texts[f as usize]) { h += 1; } } }
            let whole = t.elapsed().as_secs_f64();
            // gates only
            let t = Instant::now(); let mut g = 0usize;
            for &i in &content { for &f in &cl[i].files { let tx = &texts[f as usize]; if cl[i].ci.iter().all(|r| r.is_match(tx)) { g += 1; } } }
            let gates = t.elapsed().as_secs_f64();
            // line split only (char_indices, python terminators), once per (clause,file)
            let t = Instant::now(); let mut nl = 0usize;
            for &i in &content { for &f in &cl[i].files { for (_, ch) in texts[f as usize].char_indices() { if matches!(ch, '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}') { nl += 1; } } } }
            let split = t.elapsed().as_secs_f64();
            let t = Instant::now(); let mut nl2 = 0usize;
            for &i in &content { for &f in &cl[i].files { nl2 += memchr_count(texts[f as usize].as_bytes()); } }
            let split2 = t.elapsed().as_secs_f64();
            println!("{}", json!({"bytes": by, "whole_file_is_match_s": whole, "whole_hits": h, "content_gates_s": gates, "gate_pass": g, "char_line_split_s": split, "lines": nl, "byte_newline_count_s": split2, "lf": nl2}));
        }
        _ => panic!(),
    }
}

fn memchr_count(b: &[u8]) -> usize { b.iter().filter(|&&c| c == b'\n').count() }
