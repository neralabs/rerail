//! `clang -E` as mode (b)'s oracle (decision 0327, "Correctness").
//!
//! `pp_oracle c|cpp DIR [FIRST] [COUNT]`: each file of DIR preprocessed by clang and by
//! `rerail-c-pp` under one profile, and their tokens compared. The profile and the stubs of the
//! headers a corpus file's project would have are `oracle_profile`'s.

#[path = "../src/oracle_profile.rs"]
mod oracle_profile;

use oracle_profile::{Lang, Profile, Settled, clang, clang_tokens};
use rerail_c_pp::sink::Diag;
use std::collections::HashMap;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let lang = match a.get(1).map(|s| s.as_str()) {
        Some("cpp") => Lang::cpp(),
        _ => Lang::c(),
    };
    let dir = a.get(2).expect("pp_oracle c|cpp DIR [FIRST] [COUNT]");
    let first: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    let count: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    let mut prof = Profile::new(lang);
    let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    names.sort();
    let (mut same, mut differ, mut clang_failed) = (0, 0, 0);
    let mut diag_kinds: HashMap<String, usize> = HashMap::new();
    for path in names.iter().skip(first).take(count) {
        let p = path.to_string_lossy().into_owned();
        // Ours, until no header is missing and no test unanswered.
        let Settled { tokens: ours, facts, stub_dir, .. } = prof.settle(&p);
        let lang = &prof.lang;
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
    drop(prof);
    println!("same {same}, differ {differ}, clang without output {clang_failed}; our diagnostics {diag_kinds:?}");
}
