//! `clang -E` as mode (b)'s oracle (decision 0327, "Correctness").
//!
//! `pp_oracle c|cpp DIR [FIRST] [COUNT]`: each file of DIR preprocessed by clang and by
//! `rerail-c-pp` under one profile, and their tokens compared. The profile is clang's own:
//! its predefined macros (`clang -dM -E`), its search path (`clang -E -v`), and its answers to the
//! `__has_*` tests the pass asks (probed, and kept for the run).
//!
//! A corpus file includes its project's headers, which are not here. Each header the pass does not
//! find gets an empty stub, after every other directory (`-idirafter`) for both, so that both read
//! the same files.

use rerail_c_pp::files::{DirKind, Search};
use rerail_c_pp::macros::Tk;
use rerail_c_pp::sink::{At, Diag, FactSink, Sink, Tee};
use rerail_c_pp::{Config, Options, Pp};
use rerail_c_lean::pp::{P, PpLexer};
use std::collections::HashMap;
use std::process::Command;

#[derive(Default)]
struct Spellings(Vec<Vec<u8>>);

impl Sink for Spellings {
    fn token(&mut self, _t: &Tk, text: &[u8], _at: Option<At>, _off: u32) {
        self.0.push(text.to_vec());
    }
}

struct Lang {
    opts: Options,
    args: Vec<&'static str>,
}

fn clang(args: &[&str], input: Option<&[u8]>) -> (bool, Vec<u8>, Vec<u8>) {
    let mut c = Command::new("clang");
    // `__DATE__` and `__TIME__` as the configuration fixes them (the Unix epoch).
    c.args(args).env("SOURCE_DATE_EPOCH", "0");
    if input.is_some() {
        c.stdin(std::process::Stdio::piped());
    }
    c.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = c.spawn().expect("clang runs");
    if let Some(i) = input {
        use std::io::Write;
        child.stdin.take().unwrap().write_all(i).unwrap();
    }
    let o = child.wait_with_output().unwrap();
    (o.status.success(), o.stdout, o.stderr)
}

/// clang's predefined macros and search path.
fn profile(lang: &Lang) -> (Vec<(String, Option<String>)>, Vec<(String, DirKind)>) {
    let mut a = lang.args.clone();
    a.extend(["-dM", "-E", "-"]);
    let (_, out, _) = clang(&a, Some(b""));
    let mut defs = Vec::new();
    for l in String::from_utf8_lossy(&out).lines() {
        let Some(rest) = l.strip_prefix("#define ") else { continue };
        // The name runs to the first blank outside its parameter list.
        let mut depth = 0;
        let mut end = rest.len();
        for (k, c) in rest.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ' ' if depth == 0 => {
                    end = k;
                    break;
                }
                _ => {}
            }
        }
        let body = rest.get(end + 1..).unwrap_or("").to_string();
        defs.push((rest[..end].to_string(), Some(body)));
    }
    let mut a = lang.args.clone();
    a.extend(["-E", "-v", "-"]);
    let (_, _, err) = clang(&a, Some(b""));
    let mut dirs = Vec::new();
    let mut on = false;
    for l in String::from_utf8_lossy(&err).lines() {
        if l.starts_with("#include <...> search starts here") {
            on = true;
        } else if l.starts_with("End of search list") {
            on = false;
        } else if on {
            let l = l.trim();
            match l.strip_suffix(" (framework directory)") {
                Some(d) => dirs.push((d.to_string(), DirKind::Framework)),
                None => dirs.push((l.to_string(), DirKind::System)),
            }
        }
    }
    (defs, dirs)
}

/// clang's values of `tests` (`(builtin, operand)`), each line its own: a test clang does not
/// read is 0 alone.
fn probe(lang: &Lang, tests: &[(String, String)]) -> Vec<i64> {
    let mut src = String::new();
    for (k, (b, x)) in tests.iter().enumerate() {
        src.push_str(&format!("__rr_probe_{k} {b}({x})\n"));
    }
    let mut a = lang.args.clone();
    a.extend(["-E", "-P", "-w", "-"]);
    let (_, out, _) = clang(&a, Some(src.as_bytes()));
    let mut vals = vec![0; tests.len()];
    for l in String::from_utf8_lossy(&out).lines() {
        let Some(rest) = l.trim().strip_prefix("__rr_probe_") else { continue };
        let Some((k, v)) = rest.split_once(' ') else { continue };
        if let (Ok(k), Ok(v)) = (k.parse::<usize>(), v.trim().parse::<i64>()) {
            if k < vals.len() {
                vals[k] = v;
            }
        }
    }
    vals
}

fn read_fs(p: &str) -> Option<Vec<u8>> {
    std::fs::read(p).ok().filter(|_| std::path::Path::new(p).is_file())
}

/// The tokens of clang's `-E -P` text, its `#pragma` lines left out.
fn clang_tokens(text: &[u8], opts: Options) -> Vec<Vec<u8>> {
    let mut lx = PpLexer::new(text, opts);
    let mut v = Vec::new();
    loop {
        let t = lx.next();
        if t.kind == P::Eof {
            break;
        }
        if t.kind == P::Hash && t.flags & rerail_c_lean::pp::BOL != 0 {
            lx.skip_line();
            continue;
        }
        v.push(text[t.start as usize..t.end as usize].to_vec());
    }
    v
}

/// `name` under `dir`, `..` read; `None` where it leaves `root`.
fn under(root: &std::path::Path, dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let mut p = dir.to_path_buf();
    for c in name.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                if !p.pop() || !p.starts_with(root) || p == root {
                    return None;
                }
            }
            c => p.push(c),
        }
    }
    p.starts_with(root).then_some(p)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let lang = match a.get(1).map(|s| s.as_str()) {
        Some("cpp") => Lang { opts: Options::cpp(20, true), args: vec!["-x", "c++", "-std=gnu++20"] },
        _ => Lang { opts: Options::c(17, true), args: vec!["-x", "c", "-std=gnu17"] },
    };
    let dir = a.get(2).expect("pp_oracle c|cpp DIR [FIRST] [COUNT]");
    let first: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    let count: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    let (defs, dirs) = profile(&lang);
    let mut features: HashMap<(String, String), i64> = HashMap::new();
    let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    names.sort();
    let stubs = std::env::temp_dir().join(format!("rerail-pp-oracle-{}", std::process::id()));
    let (mut same, mut differ, mut clang_failed) = (0, 0, 0);
    let mut diag_kinds: HashMap<String, usize> = HashMap::new();
    for path in names.iter().skip(first).take(count) {
        let p = path.to_string_lossy().into_owned();
        let _ = std::fs::remove_dir_all(&stubs);
        std::fs::create_dir_all(&stubs).unwrap();
        // Stubs for `../x.h` too: the directory is nested, so that they stay inside.
        let nest = stubs.join("a/b/c/d");
        std::fs::create_dir_all(&nest).unwrap();
        let stub_dir = nest.to_string_lossy().into_owned();
        // Ours, until no header is missing and no test unanswered.
        let mut ours = Vec::new();
        let mut facts = FactSink::default();
        for _round in 0..4 {
            let mut cfg = Config::new(lang.opts);
            cfg.predefines = defs.clone();
            cfg.features = features.clone();
            let mut d = dirs.clone();
            d.push((stub_dir.clone(), DirKind::After));
            let mut io = Search::new(d, read_fs);
            let mut sp = Spellings::default();
            facts = FactSink::default();
            let mut tee = Tee(&mut sp, &mut facts);
            let mut pp = Pp::new(cfg, &mut io, &mut tee);
            pp.run(&p);
            let unanswered: Vec<_> = pp.unanswered.iter().filter(|k| !features.contains_key(*k)).cloned().collect();
            drop(pp);
            ours = sp.0;
            let mut again = false;
            if !unanswered.is_empty() {
                let mut u = unanswered;
                u.sort();
                u.dedup();
                for (k, v) in u.iter().zip(probe(&lang, &u)) {
                    features.insert(k.clone(), v);
                }
                again = true;
            }
            for (_, _, name, _) in &facts.missing {
                let n = String::from_utf8_lossy(name).into_owned();
                if n.starts_with('/') {
                    continue;
                }
                let Some(s) = under(&stubs, &nest, &n) else { continue };
                if !s.exists() {
                    std::fs::create_dir_all(s.parent().unwrap()).unwrap();
                    std::fs::write(&s, b"").unwrap();
                    again = true;
                }
            }
            if !again {
                break;
            }
        }
        let mut args = lang.args.clone();
        args.extend(["-E", "-P", "-w", "-ferror-limit=0", "-idirafter", &stub_dir, &p]);
        let (_ok, out, _err) = clang(&args, None);
        if out.is_empty() {
            clang_failed += 1;
            continue;
        }
        let theirs = clang_tokens(&out, lang.opts);
        for d in &facts.diags {
            if !matches!(d.2, Diag::Error | Diag::Warning) {
                let k = format!("{:?}", d.2);
                if matches!(d.2, Diag::BadExpr(_)) && !diag_kinds.contains_key(&k) {
                    let f = facts.paths.get(d.0 as usize).map_or("?", |s| s.as_str());
                    let line = std::fs::read(f).map_or(0, |b| b[..(d.1 as usize).min(b.len())].iter().filter(|&&c| c == b'\n').count() + 1);
                    println!("DIAG {k} at {f}:{line}");
                }
                *diag_kinds.entry(k).or_default() += 1;
            }
        }
        if theirs == ours {
            same += 1;
            continue;
        }
        differ += 1;
        let k = theirs.iter().zip(&ours).position(|(a, b)| a != b).unwrap_or(theirs.len().min(ours.len()));
        let show = |v: &[Vec<u8>]| v[k.saturating_sub(6)..(k + 6).min(v.len())].iter().map(|t| String::from_utf8_lossy(t).into_owned()).collect::<Vec<_>>().join(" ");
        println!("DIFF {p}: token {k} (clang {}, ours {})\n  clang: {}\n  ours:  {}", theirs.len(), ours.len(), show(&theirs), show(&ours));
    }
    let _ = std::fs::remove_dir_all(&stubs);
    println!("same {same}, differ {differ}, clang without output {clang_failed}; our diagnostics {diag_kinds:?}");
}
