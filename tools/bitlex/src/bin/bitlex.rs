//! bitlex tokens LEXER FILE        print the tokens and errors of one file
//! bitlex diff LEXER LIST          tokens against tree-sitter's leaves, for every file in LIST
//! bitlex strings LEXER LIST       Python: string and comment bytes against the 13-state machine
//! bitlex fstr LEXER LIST          Python: interpolations, format expressions and specs against
//!                                 tree-sitter and the f-string oracle
//!
//! LIST is a file of paths, one per line. Files run in parallel; times are not measured here.

use bitlex::eval::{load_table, Ev, Output, Val};
use bitlex::mask::Mask;
use bitlex::oracle;
use bitlex::syntax::{parse, Stmt, UClass};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tree_sitter::{Language, Node, Parser};

fn tables(prog: &[Stmt]) -> HashMap<String, Vec<(u32, u32)>> {
    let mut out = HashMap::new();
    for s in prog {
        if let Stmt::Ucode { classes, .. } = s {
            for (_, c) in classes {
                if let UClass::Table(file, sym) = c {
                    let path = std::env::var("BITLEX_PARSER_C").unwrap_or_else(|_| {
                        format!("{}/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tree-sitter-python-0.25.0/src/{file}", std::env::var("HOME").unwrap())
                    });
                    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
                    out.insert(format!("{file}:{sym}"), load_table(&text, sym).unwrap());
                }
            }
        }
    }
    out
}

fn lang_of(prog: &[Stmt]) -> Language {
    match prog.iter().find_map(|s| if let Stmt::Lexer(n) = s { Some(n.as_str()) } else { None }) {
        Some("json") => tree_sitter_json::LANGUAGE.into(),
        _ => tree_sitter_python::LANGUAGE.into(),
    }
}

/// Runs `f` over every path in `list` on all cores.
fn par_files<T: Send>(list: &str, f: impl Fn(&str, &[u8]) -> T + Sync) -> Vec<(String, T)> {
    let paths: Vec<String> = std::fs::read_to_string(list).unwrap().lines().map(String::from).collect();
    let next = AtomicUsize::new(0);
    let out = Mutex::new(vec![]);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= paths.len() {
                    break;
                }
                let Ok(src) = std::fs::read(&paths[i]) else { continue };
                let r = f(&paths[i], &src);
                out.lock().unwrap().push((paths[i].clone(), r));
            });
        }
    });
    let mut v = out.into_inner().unwrap();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn rounds_hist(results: &[&Output]) -> BTreeMap<String, BTreeMap<usize, u64>> {
    let mut h: BTreeMap<String, BTreeMap<usize, u64>> = BTreeMap::new();
    for o in results {
        let mut per: BTreeMap<String, usize> = BTreeMap::new();
        for (l, r) in &o.rounds {
            let l = l.rsplit(": ").next().unwrap().to_string();
            let e = per.entry(l).or_default();
            *e = (*e).max(*r);
        }
        for (l, r) in per {
            *h.entry(l).or_default().entry(r).or_default() += 1;
        }
    }
    h
}

/// tree-sitter's leaves as (start, end, kind). A string_content with escape children becomes the
/// pieces between its children, as the lexer emits them.
fn ts_leaves(n: Node, out: &mut Vec<(usize, usize, String)>) {
    if n.kind() == "string_content" && n.child_count() > 0 {
        let mut at = n.start_byte();
        let mut c = n.walk();
        for ch in n.children(&mut c) {
            if ch.start_byte() > at {
                out.push((at, ch.start_byte(), "string_content".into()));
            }
            ts_leaves(ch, out);
            at = ch.end_byte();
        }
        if n.end_byte() > at {
            out.push((at, n.end_byte(), "string_content".into()));
        }
        return;
    }
    if n.child_count() == 0 {
        if n.end_byte() > n.start_byte() {
            out.push((n.start_byte(), n.end_byte(), n.kind().to_string()));
        }
        return;
    }
    let mut c = n.walk();
    for ch in n.children(&mut c) {
        ts_leaves(ch, out);
    }
}

#[derive(Default)]
struct DiffRes {
    ts_error: bool,
    lex_errors: Vec<String>,
    mismatch: Option<String>,
    sigs: Vec<String>,
    eval_error: Option<String>,
}

fn diff(prog: &[Stmt], list: &str) {
    let lang = lang_of(prog);
    let tabs = tables(prog);
    let res = par_files(list, |_, src| {
        let mut r = DiffRes::default();
        let mut p = Parser::new();
        p.set_language(&lang).unwrap();
        let t = p.parse(src, None).unwrap();
        r.ts_error = t.root_node().has_error();
        let out = match Ev::run(prog, src, &tabs) {
            Ok(o) => o,
            Err(e) => {
                r.eval_error = Some(e);
                return r;
            }
        };
        r.lex_errors = out.errors.clone();
        if r.ts_error || !r.lex_errors.is_empty() {
            return r;
        }
        let mut ts = vec![];
        ts_leaves(t.root_node(), &mut ts);
        let mut i = 0;
        let mut j = 0;
        let lx = &out.tokens;
        let same = |a: &(usize, usize, String), b: &bitlex::eval::Token| {
            a.0 == b.start && a.1 == b.end && (a.2 == b.kind || (b.soft && a.2.as_bytes() == &src[b.start..b.end]))
        };
        while i < ts.len() || j < lx.len() {
            if i < ts.len() && j < lx.len() && same(&ts[i], &lx[j]) {
                i += 1;
                j += 1;
                continue;
            }
            let tsd = ts.get(i).map(|a| format!("{}@{}..{}", a.2, a.0, a.1)).unwrap_or("-".into());
            let lxd = lx.get(j).map(|b| format!("{}@{}..{}", b.kind, b.start, b.end)).unwrap_or("-".into());
            let tk = ts.get(i).map(|a| a.2.clone()).unwrap_or("-".into());
            let lk = lx.get(j).map(|b| b.kind.clone()).unwrap_or("-".into());
            r.sigs.push(format!("tree-sitter {tk} / lexer {lk}"));
            if r.mismatch.is_none() {
                let at = ts.get(i).map(|a| a.0).or(lx.get(j).map(|b| b.start)).unwrap_or(0);
                let ctx = String::from_utf8_lossy(&src[at.saturating_sub(20)..(at + 30).min(src.len())]).into_owned();
                r.mismatch = Some(format!("tree-sitter {tsd}, lexer {lxd}: {ctx:?}"));
            }
            // Resynchronise on the smaller start.
            let ts_at = ts.get(i).map(|a| a.0).unwrap_or(usize::MAX);
            let lx_at = lx.get(j).map(|b| b.start).unwrap_or(usize::MAX);
            if ts_at <= lx_at {
                i += 1
            }
            if lx_at <= ts_at {
                j += 1
            }
        }
        r
    });
    let (mut files, mut ts_err, mut both, mut lex_only, mut equal, mut differ, mut evalerr) = (0, 0, 0, 0, 0, 0, 0);
    let mut why: BTreeMap<String, u64> = BTreeMap::new();
    let mut sigs: BTreeMap<String, u64> = BTreeMap::new();
    let mut shown = 0;
    for (path, r) in &res {
        files += 1;
        if let Some(e) = &r.eval_error {
            evalerr += 1;
            if evalerr <= 3 {
                eprintln!("EVAL {path}: {e}");
            }
            continue;
        }
        let lex_err = !r.lex_errors.is_empty();
        match (r.ts_error, lex_err) {
            (true, true) => both += 1,
            (true, false) => ts_err += 1,
            (false, true) => {
                lex_only += 1;
                let w = r.lex_errors[0].split(" (byte").next().unwrap().to_string();
                *why.entry(w).or_default() += 1;
                if std::env::var("SHOW_FALLBACK").is_ok() {
                    eprintln!("FALLBACK {path}: {:?}", r.lex_errors);
                }
            }
            (false, false) => {
                if let Some(m) = &r.mismatch {
                    differ += 1;
                    if shown < 12 {
                        eprintln!("DIFFER {path}: {m}");
                        shown += 1;
                    }
                    for s in &r.sigs {
                        *sigs.entry(s.clone()).or_default() += 1;
                    }
                } else {
                    equal += 1
                }
            }
        }
    }
    println!("files {files}: tree-sitter and lexer agree on every token in {equal}; differ in {differ}");
    println!("  tree-sitter has errors: {} (the lexer falls back too in {both}); only the lexer falls back: {lex_only}; evaluator errors: {evalerr}", ts_err + both);
    for (w, c) in &why {
        println!("  fallback: {c:6}  {w}");
    }
    let mut sv: Vec<_> = sigs.into_iter().collect();
    sv.sort_by(|a, b| b.1.cmp(&a.1));
    for (s, c) in sv.iter().take(25) {
        println!("  mismatch: {c:6}  {s}");
    }
}

fn mask_of<'a>(o: &'a Output, n: &str) -> Option<&'a Mask> {
    match o.vars.get(n) {
        Some(Val::M(m)) => Some(m),
        _ => None,
    }
}

fn strings(prog: &[Stmt], list: &str) {
    let tabs = tables(prog);
    let res = par_files(list, |_, src| {
        let out = Ev::run(prog, src, &tabs);
        let m = oracle::machine(src);
        (out, m)
    });
    let (mut agree, mut differ, mut both_reject, mut machine_rejects, mut mask_rejects, mut evalerr) = (0, 0, 0, 0, 0, 0);
    let mut shown = 0;
    let mut outs = vec![];
    for (path, (out, m)) in &res {
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                evalerr += 1;
                if evalerr <= 3 {
                    eprintln!("EVAL {path}: {e}")
                }
                continue;
            }
        };
        outs.push(out);
        let strings_err = out.errors.iter().any(|e| e.contains("line:") || e.contains("rounds(icode)"));
        match (m, strings_err) {
            (None, true) => both_reject += 1,
            (None, false) => machine_rejects += 1,
            (Some(_), true) => {
                mask_rejects += 1;
                if shown < 8 {
                    eprintln!("MASKS REJECT {path}: {:?}", out.errors);
                    shown += 1
                }
            }
            (Some(cls), false) => {
                let (s, c) = (mask_of(out, "in_str").unwrap(), mask_of(out, "in_cmt").unwrap());
                let k = (0..cls.len()).find(|&i| cls[i] != if c.get(i) { 2 } else if s.get(i) { 1 } else { 0 });
                match k {
                    None => agree += 1,
                    Some(k) => {
                        differ += 1;
                        if shown < 8 {
                            let src = std::fs::read(path).unwrap();
                            eprintln!("DIFFER {path} at {k}: {:?}", String::from_utf8_lossy(&src[k.saturating_sub(30)..(k + 30).min(src.len())]));
                            shown += 1
                        }
                    }
                }
            }
        }
    }
    println!("files {}: classes equal to the 13-state machine in {agree}; differ in {differ}", res.len());
    println!("  both reject {both_reject}; only the machine rejects {machine_rejects}; only the masks reject {mask_rejects}; evaluator errors {evalerr}");
    for (l, h) in rounds_hist(&outs) {
        println!("  {l}: files by rounds {h:?}");
    }
}

fn ts_fields(n: Node, f: &mut oracle::Fields) {
    match n.kind() {
        "interpolation" => f.interp.push((n.start_byte(), n.end_byte())),
        "format_expression" => f.fexpr.push((n.start_byte(), n.end_byte())),
        "format_specifier" => f.spec.push((n.start_byte(), n.end_byte())),
        _ => {}
    }
    let mut c = n.walk();
    for ch in n.children(&mut c) {
        ts_fields(ch, f);
    }
}

/// The fields of one f-string, not those of strings nested in it.
fn ts_fields_own(n: Node, f: &mut oracle::Fields) {
    let mut c = n.walk();
    for ch in n.children(&mut c) {
        if ch.kind() == "string" {
            continue;
        }
        match ch.kind() {
            "interpolation" => f.interp.push((ch.start_byte(), ch.end_byte())),
            "format_expression" => f.fexpr.push((ch.start_byte(), ch.end_byte())),
            "format_specifier" => f.spec.push((ch.start_byte(), ch.end_byte())),
            _ => {}
        }
        ts_fields_own(ch, f);
    }
}

fn mask_fields(o: &Output) -> oracle::Fields {
    let mut f = oracle::Fields::default();
    let get = |n: &str| -> Mask {
        let mut m: Option<Mask> = None;
        for inst in ["f1", "f2"] {
            if let Some(x) = mask_of(o, &format!("{inst}.{n}")) {
                m = Some(match m {
                    Some(y) => y.or(x),
                    None => x.clone(),
                })
            }
        }
        m.unwrap()
    };
    let (iopen, xopen, fclose, colon) = (get("iopen"), get("xopen"), get("fclose"), get("colon"));
    let all = iopen.or(&xopen).or(&fclose).or(&colon);
    let mut stack: Vec<(usize, bool, Option<usize>)> = vec![];
    for i in all.ones_iter() {
        if iopen.get(i) || xopen.get(i) {
            stack.push((i, iopen.get(i), None))
        }
        if colon.get(i) {
            if let Some(t) = stack.last_mut() {
                t.2 = Some(i)
            }
        }
        if fclose.get(i) {
            if let Some((s, top, c)) = stack.pop() {
                if top { f.interp.push((s, i + 1)) } else { f.fexpr.push((s, i + 1)) }
                if let Some(c) = c {
                    f.spec.push((c, i))
                }
            }
        }
    }
    f.interp.sort();
    f.fexpr.sort();
    f.spec.sort();
    f
}

fn fstr(prog: &[Stmt], list: &str) {
    let tabs = tables(prog);
    let lang: Language = tree_sitter_python::LANGUAGE.into();
    #[derive(Default)]
    struct FR {
        skip: bool,
        out: Option<Output>,
        eval_error: Option<String>,
        masks_eq_ts: Option<bool>,
        oracle_ok: u64,
        oracle_bad: u64,
        oracle_unsup: BTreeMap<&'static str, u64>,
        fstrings: u64,
        detail: Option<String>,
    }
    let res = par_files(list, |_, src| {
        let mut r = FR::default();
        let mut p = Parser::new();
        p.set_language(&lang).unwrap();
        let t = p.parse(src, None).unwrap();
        if t.root_node().has_error() {
            r.skip = true;
            return r;
        }
        let mut truth = oracle::Fields::default();
        ts_fields(t.root_node(), &mut truth);
        truth.interp.sort();
        truth.fexpr.sort();
        truth.spec.sort();
        // The oracle, per f-string tree-sitter found.
        let esc = oracle::escaped(src);
        let mut stack = vec![t.root_node()];
        while let Some(n) = stack.pop() {
            let mut c = n.walk();
            for ch in n.children(&mut c) {
                stack.push(ch)
            }
            if n.kind() != "string" {
                continue;
            }
            let (Some(st), Some(en)) = (n.child(0), n.child(n.child_count() - 1)) else { continue };
            let pre = &src[st.byte_range()];
            if !pre.iter().any(|&b| b == b'f' || b == b'F') {
                continue;
            }
            r.fstrings += 1;
            let triple = pre.len() >= 3 && pre[pre.len() - 3..].iter().all(|&b| b == pre[pre.len() - 1]);
            let raw = pre.iter().any(|&b| b == b'r' || b == b'R');
            let mut want = oracle::Fields::default();
            ts_fields_own(n, &mut want);
            want.interp.sort();
            want.fexpr.sort();
            want.spec.sort();
            match oracle::fstring(src, &esc, st.end_byte(), en.start_byte(), triple, raw, 12) {
                oracle::FOut::Ok(f, _) if f == want => r.oracle_ok += 1,
                o @ (oracle::FOut::Ok(..) | oracle::FOut::Unsettled) => {
                    r.oracle_bad += 1;
                    if std::env::var("SHOW_ORACLE").is_ok() {
                        eprintln!("ORACLE {:?}\n    tree-sitter {want:?}\n    oracle {o:?}", String::from_utf8_lossy(&src[n.byte_range()]));
                    }
                }
                oracle::FOut::Unsupported(w) => *r.oracle_unsup.entry(w).or_default() += 1,
            }
        }
        match Ev::run(prog, src, &tabs) {
            Ok(o) => {
                if o.errors.is_empty() {
                    let f = mask_fields(&o);
                    r.masks_eq_ts = Some(f == truth);
                    if f != truth {
                        r.detail = Some(format!("tree-sitter {truth:?}\n    masks {f:?}"));
                    }
                }
                r.out = Some(o);
            }
            Err(e) => r.eval_error = Some(e),
        }
        r
    });
    let (mut files, mut skipped, mut eq, mut ne, mut fallback, mut evalerr) = (0, 0, 0, 0, 0, 0);
    let (mut ook, mut obad, mut fs) = (0, 0, 0);
    let mut unsup: BTreeMap<&str, u64> = BTreeMap::new();
    let mut why: BTreeMap<String, u64> = BTreeMap::new();
    let mut outs = vec![];
    let mut shown = 0;
    for (path, r) in &res {
        if r.skip {
            skipped += 1;
            continue;
        }
        files += 1;
        ook += r.oracle_ok;
        obad += r.oracle_bad;
        fs += r.fstrings;
        for (k, v) in &r.oracle_unsup {
            *unsup.entry(k).or_default() += v
        }
        if let Some(e) = &r.eval_error {
            evalerr += 1;
            if evalerr <= 3 {
                eprintln!("EVAL {path}: {e}")
            }
            continue;
        }
        let o = r.out.as_ref().unwrap();
        outs.push(o);
        match r.masks_eq_ts {
            Some(true) => eq += 1,
            Some(false) => {
                ne += 1;
                if shown < 8 {
                    eprintln!("DIFFER {path}\n    {}", r.detail.as_ref().unwrap());
                    shown += 1
                }
            }
            None => {
                fallback += 1;
                let w = o.errors[0].split(" (byte").next().unwrap().to_string();
                *why.entry(w).or_default() += 1;
            }
        }
    }
    println!("files without tree-sitter errors {files} (skipped {skipped}); f-strings {fs}");
    println!("  oracle, per f-string: equal to tree-sitter {ook}; different or unsettled {obad}; unsupported {unsup:?}");
    println!("  masks, per file: fields equal to tree-sitter {eq}; different {ne}; lexer falls back {fallback}; evaluator errors {evalerr}");
    for (w, c) in &why {
        println!("    fallback: {c:6}  {w}");
    }
    for (l, h) in rounds_hist(&outs) {
        println!("  {l}: files by rounds {h:?}");
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 {
        eprintln!("usage: bitlex tokens|diff|strings|fstr LEXER FILE|LIST");
        std::process::exit(2);
    }
    let prog = parse(&std::fs::read_to_string(&a[2]).unwrap()).unwrap_or_else(|e| {
        eprintln!("{}: {e}", a[2]);
        std::process::exit(1)
    });
    match a[1].as_str() {
        "tokens" => {
            let src = std::fs::read(&a[3]).unwrap();
            let tabs = tables(&prog);
            match Ev::run(&prog, &src, &tabs) {
                Ok(o) => {
                    for t in &o.tokens {
                        println!("{:6} {:6} {:20} {:?}{}", t.start, t.end, t.kind, String::from_utf8_lossy(&src[t.start..t.end]), if t.soft { " soft" } else { "" });
                    }
                    for e in &o.errors {
                        println!("ERROR {e}");
                    }
                    println!("rounds {:?}", o.rounds);
                    let ops = bitlex::eval::OPS.load(std::sync::atomic::Ordering::Relaxed);
                    println!("ops per block {ops} (each op runs over all {} blocks)", src.len().div_ceil(64));
                }
                Err(e) => println!("EVAL {e}"),
            }
        }
        "diff" => diff(&prog, &a[3]),
        "strings" => strings(&prog, &a[3]),
        "fstr" => fstr(&prog, &a[3]),
        c => eprintln!("unknown command {c}"),
    }
}
