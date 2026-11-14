// Compare Python string/comment classification: the 13-state `strings` machine against an
// automaton-free version (reflection scans + per-line segments + hidden-triple fixpoint).
use std::collections::{BTreeMap, HashSet};

fn escaped(src: &[u8]) -> Vec<bool> {
    let mut e = vec![false; src.len() + 1]; let mut run = 0usize;
    for (i, &c) in src.iter().enumerate() { if c == b'\\' { run += 1 } else { if run % 2 == 1 { e[i] = true } run = 0 } }
    e
}
fn is_q(src: &[u8], esc: &[bool], i: usize) -> bool { i < src.len() && (src[i] == b'\'' || src[i] == b'"') && !esc[i] }
fn is_nl(src: &[u8], esc: &[bool], i: usize) -> bool { src[i] == b'\n' && !esc[i] && !(i > 0 && src[i-1] == b'\r' && esc[i-1]) }

fn step(st: u8, e: u8) -> Option<u8> {
    Some(match (st, e) {
        (0, 4) => 1, (0, 0) => 2, (0, 1) => 3, (0, 2) => 6, (0, 3) => 8,
        (1, 5) => 0,
        (2, 0) | (2, 2) => 0, (3, 1) | (3, 3) => 0, (2, 5) | (3, 5) => return None,
        (6, _) => 7, (7, _) => 4, (8, _) => 9, (9, _) => 5,
        (4, 2) => 10, (5, 3) => 10, (10, _) => 11, (11, _) => 0,
        (s, _) => s,
    })
}
fn event(src: &[u8], esc: &[bool], i: usize) -> Option<u8> {
    let c = src[i];
    if is_q(src, esc, i) {
        let tri = is_q(src, esc, i+1) && is_q(src, esc, i+2) && src[i+1] == c && src[i+2] == c;
        Some(match (c, tri) { (b'\'', true) => 2, (b'"', true) => 3, (b'\'', false) => 0, _ => 1 })
    } else if c == b'#' { Some(4) } else if is_nl(src, esc, i) { Some(5) } else { None }
}
/// The machine's classes over src[a..b] from start state `st`; None on error.
fn machine_from(src: &[u8], esc: &[bool], a: usize, b: usize, mut st: u8) -> Option<(u8, Vec<u8>)> {
    let mut cls = vec![0u8; b - a];
    for i in a..b { if let Some(e) = event(src, esc, i) { st = step(st, e)? } cls[i - a] = match st { 1 => 2, 2..=9 => 1, _ => 0 }; }
    Some((st, cls))
}
fn triples(src: &[u8], esc: &[bool]) -> Vec<(usize, u8)> {
    let mut tri = vec![]; let mut i = 0;
    while i < src.len() { if is_q(src, esc, i) { let c = src[i]; let mut j = i; while j < src.len() && src[j] == c && is_q(src, esc, j) { j += 1 }
        let mut k = i; while k + 3 <= j { tri.push((k, if c == b'\'' { 1 } else { 2 })); k += 3 } i = j } else { i += 1 } }
    tri
}
/// One line, speculate-and-verify inside the line only, from triple state x0 (0 code, 1 in ''', 2 in """).
/// Returns (end triple state, classes, rounds), or None if unsettled or a short string is left open.
fn line_nofsa(src: &[u8], esc: &[bool], a: usize, b: usize, x0: i32, tri: &[(usize, u8)], kmax: usize) -> Option<(i32, Vec<u8>, usize)> {
    let tri: Vec<(usize, u8)> = tri.iter().copied().filter(|t| t.0 >= a && t.0 < b).collect();
    let cand: HashSet<usize> = tri.iter().map(|t| t.0).collect();
    let mut hidden: HashSet<usize> = HashSet::new();
    for round in 0..kmax {
        let mut x = x0; let mut tstr = vec![false; b - a]; let mut tbyte = vec![false; b - a];
        let mut openers: HashSet<usize> = HashSet::new(); let mut from: Option<usize> = if x0 != 0 { Some(a) } else { None };
        for &(p, k) in &tri { if hidden.contains(&p) { continue }
            let before = x; x = (k as i32 - x).rem_euclid(3);
            for q in p..(p + 3).min(b) { tbyte[q - a] = true }
            if before == 0 && x != 0 { from = Some(p); openers.insert(p); }
            if before != 0 && x == 0 { for q in from.take().unwrap()..p { tstr[q - a] = true } }
        }
        if let Some(f) = from { for q in f..b { tstr[q - a] = true } }
        let mut cls = vec![0u8; b - a]; let (mut s, mut cmt) = (0i32, false); let mut next_hidden = HashSet::new();
        for i in a..b {
            if cand.contains(&i) && (openers.contains(&i) || !tstr[i - a]) && (s != 0 || cmt) { next_hidden.insert(i); }
            if tstr[i - a] { cls[i - a] = 1; s = 0; cmt = false; continue }
            let c = src[i];
            if cmt { cls[i - a] = if is_nl(src, esc, i) { 0 } else { 2 }; continue }
            if is_q(src, esc, i) && !tbyte[i - a] { let k = if c == b'\'' { 1 } else { 2 }; s = (k - s).rem_euclid(3) }
            else if c == b'#' && s == 0 { cmt = true; cls[i - a] = 2; continue }
            cls[i - a] = if s != 0 { 1 } else { 0 };
        }
        if next_hidden != hidden { hidden = next_hidden; continue }
        if s != 0 && !cmt { return None }
        return Some((x, cls, round + 1));
    }
    None
}

fn lines_main(list: &str) {
    let (mut lines, mut tri_lines, mut nonid, mut noninj, mut agree, mut differ, mut unsettled) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let mut rounds: BTreeMap<usize, u64> = BTreeMap::new(); let mut shown = 0;
    for path in std::fs::read_to_string(list).unwrap().lines() {
        let Ok(src) = std::fs::read(path) else { continue }; let esc = escaped(&src); let tri = triples(&src, &esc);
        let (mut a, mut ti) = (0usize, 0usize);
        while a < src.len() {
            let mut b = a; while b < src.len() && !is_nl(&src, &esc, b) { b += 1 } let b = (b + 1).min(src.len());
            lines += 1;
            while ti < tri.len() && tri[ti].0 < a { ti += 1 }
            if ti < tri.len() && tri[ti].0 < b {
                tri_lines += 1;
                let starts = [(0i32, 0u8), (1, 4), (2, 5)];
                let act: Vec<Option<i32>> = starts.iter().map(|&(_, st)| machine_from(&src, &esc, a, b, st)
                    .and_then(|(e, _)| match e { 0 | 1 => Some(0), 4 => Some(1), 5 => Some(2), _ => None })).collect();
                if act.iter().enumerate().any(|(h, e)| *e != Some(h as i32)) { nonid += 1 }
                let dom: Vec<i32> = act.iter().filter_map(|e| *e).collect(); let set: HashSet<i32> = dom.iter().copied().collect();
                if set.len() != dom.len() { noninj += 1; if shown < 5 { eprintln!("not injective {:?}: {:?}", act, String::from_utf8_lossy(&src[a..b])); shown += 1 } }
                for &(h, st) in &starts {
                    match (machine_from(&src, &esc, a, b, st), line_nofsa(&src, &esc, a, b, h, &tri, 6)) {
                        (Some((me, mc)), Some((oe, oc, r))) => { *rounds.entry(r).or_default() += 1;
                            let me = match me { 0 | 1 => 0, 4 => 1, 5 => 2, _ => 9 };
                            if mc == oc && me == oe { agree += 1 } else { differ += 1; eprintln!("DIFFER h={h} machine end {me} ours {oe}: {:?}", String::from_utf8_lossy(&src[a..b])) } }
                        (None, None) => agree += 1,
                        (Some(_), None) => { unsettled += 1; eprintln!("UNSETTLED h={h}: {:?}", String::from_utf8_lossy(&src[a..b.min(a+160)])) }
                        (None, Some(_)) => differ += 1,
                    }
                }
            }
            a = b;
        }
    }
    println!("lines {lines}; with a triple-quote candidate {tri_lines} ({:.2}%); action not the identity {nonid}; not injective on its domain {noninj}", 100.0 * tri_lines as f64 / lines as f64);
    println!("  per (line, hypothesis): agree {agree}, differ {differ}, unsettled {unsettled}; rounds {:?}", rounds);
}

// classes: 0 code, 1 string, 2 comment
fn machine(src: &[u8], esc: &[bool]) -> Option<Vec<u8>> {
    // states: 0 code 1 comment 2 s1 3 d1 4 s3 5 d3 6 os2 7 os1 8 od2 9 od1 10 cs2 11 cs1
    let mut cls = vec![0u8; src.len()]; let mut st = 0u8;
    for i in 0..src.len() {
        let c = src[i];
        let ev: Option<u8> = if is_q(src, esc, i) {
            let tri = is_q(src, esc, i+1) && is_q(src, esc, i+2) && src[i+1] == c && src[i+2] == c;
            Some(match (c, tri) { (b'\'', true) => 2, (b'"', true) => 3, (b'\'', false) => 0, _ => 1 })
        } else if c == b'#' { Some(4) } else if is_nl(src, esc, i) { Some(5) } else { None };
        if let Some(e) = ev {
            st = match (st, e) {
                (0, 4) => 1, (0, 0) => 2, (0, 1) => 3, (0, 2) => 6, (0, 3) => 8,
                (1, 5) => 0,
                (2, 0) | (2, 2) => 0, (3, 1) | (3, 3) => 0, (2, 5) | (3, 5) => return None,
                (6, _) => 7, (7, _) => 4, (8, _) => 9, (9, _) => 5,
                (4, 2) => 10, (5, 3) => 10, (10, _) => 11, (11, _) => 0,
                (s, _) => s,
            };
        }
        cls[i] = match st { 1 => 2, 2..=9 => 1, _ => 0 };
    }
    if st != 0 && st != 1 { return None }
    Some(cls)
}

#[derive(Debug)] enum Fail { Unterminated, Iterations }

fn nofsa(src: &[u8], esc: &[bool], kmax: usize) -> Result<(Vec<u8>, usize), Fail> {
    // triple candidates: aligned from the start of each run of one unescaped quote character
    let mut tri: Vec<(usize, u8)> = vec![]; // (pos, b: 1 = ''' , 2 = """)
    let mut i = 0;
    while i < src.len() {
        if is_q(src, esc, i) { let c = src[i]; let mut j = i; while j < src.len() && src[j] == c && is_q(src, esc, j) { j += 1 }
            let mut k = i; while k + 3 <= j { tri.push((k, if c == b'\'' { 1 } else { 2 })); k += 3 } i = j } else { i += 1 }
    }
    let mut hidden: HashSet<usize> = HashSet::new();
    for iter in 0..kmax {
        // Phase T: reflection scan over active triples. state x -> (b - x) mod 3.
        // Closed form: x_n = (-1)^n * sum_j (-1)^j b_j, checked against the direct form.
        let mut x = 0i32; let (mut par, mut alt) = (0i32, 0i32);
        let mut tstr = vec![false; src.len()]; let mut tbyte = vec![false; src.len()]; // in a triple string; byte of an active triple
        let mut opener_at: Vec<usize> = vec![]; let mut open_from: Option<usize> = None;
        for &(p, b) in &tri { if hidden.contains(&p) { continue }
            let before = x; x = (b as i32 - x).rem_euclid(3);
            par ^= 1; alt = (alt + if par == 1 { -(b as i32) } else { b as i32 }).rem_euclid(3);
            let closed = if par == 1 { (-alt).rem_euclid(3) } else { alt }; assert_eq!(closed, x);
            tbyte[p] = true; tbyte[p+1] = true; tbyte[p+2] = true;
            if before == 0 && x != 0 { open_from = Some(p); opener_at.push(p) }
            if before != 0 && x == 0 { for q in open_from.take().unwrap()..p { tstr[q] = true } }
        }
        let t_open = open_from.is_some(); if let Some(f) = open_from { for q in f..src.len() { tstr[q] = true } }
        let openers: HashSet<usize> = opener_at.into_iter().collect();
        // Phase L: segments of code, reset at each unescaped newline. Every triple candidate gets
        // the status this pass implies; a pass that changes no status is self-consistent, and so
        // (by induction from the left) the only correct classification.
        let mut cls = vec![0u8; src.len()]; let (mut s, mut cmt) = (0i32, false); let mut bad = false;
        let mut next_hidden: HashSet<usize> = HashSet::new();
        let cand: HashSet<usize> = tri.iter().map(|t| t.0).collect();
        let mut i = 0;
        while i < src.len() {
            // A candidate in code (a live opener, or a hidden one) is hidden when a short string or a
            // comment is open before it. Inside a triple string a candidate is always live.
            if cand.contains(&i) && (openers.contains(&i) || !tstr[i]) && (s != 0 || cmt) { next_hidden.insert(i); }
            if tstr[i] { cls[i] = 1; s = 0; cmt = false; i += 1; continue }
            let c = src[i];
            if cmt { if is_nl(src, esc, i) { cmt = false; s = 0; cls[i] = 0 } else { cls[i] = 2 } i += 1; continue }
            if is_q(src, esc, i) && !tbyte[i] { let b = if c == b'\'' { 1 } else { 2 }; s = (b - s).rem_euclid(3) }
            else if c == b'#' && s == 0 { cmt = true; cls[i] = 2; i += 1; continue }
            else if is_nl(src, esc, i) { if s != 0 { bad = true } s = 0 }
            cls[i] = if s != 0 { 1 } else { 0 };
            i += 1;
        }
        if next_hidden != hidden { hidden = next_hidden; continue }
        if t_open || bad || (s != 0 && !cmt) { return Err(Fail::Unterminated) }
        return Ok((cls, iter + 1));
    }
    Err(Fail::Iterations)
}

fn main() {
    if std::env::args().nth(2).as_deref() == Some("lines") { lines_main(&std::env::args().nth(1).unwrap()); return }
    let a: Vec<String> = std::env::args().collect();
    let (mut files, mut agree, mut both_err, mut ref_err_only, mut ours_err_only, mut differ) = (0, 0, 0, 0, 0, 0);
    let mut iters: BTreeMap<usize, u64> = BTreeMap::new(); let mut shown = 0;
    for path in std::fs::read_to_string(&a[1]).unwrap().lines() {
        let Ok(src) = std::fs::read(path) else { continue }; files += 1;
        let esc = escaped(&src);
        let r = machine(&src, &esc); let o = nofsa(&src, &esc, 8);
        match (&r, &o) {
            (None, Err(_)) => both_err += 1,
            (None, Ok(_)) => ref_err_only += 1,
            (Some(_), Err(e)) => { ours_err_only += 1; if shown < 6 { eprintln!("ours fails ({e:?}): {path}"); shown += 1 } }
            (Some(rc), Ok((oc, it))) => { *iters.entry(*it).or_default() += 1;
                if rc == oc { agree += 1 } else { differ += 1; if shown < 6 { let k = rc.iter().zip(oc).position(|(x, y)| x != y).unwrap();
                    eprintln!("differ at {k}: {path}: {:?}", String::from_utf8_lossy(&src[k.saturating_sub(30)..(k+30).min(src.len())])); shown += 1 } } }
        }
    }
    println!("files {files}: agree {agree}, differ {differ}, both reject {both_err}, only machine rejects {ref_err_only}, only no-FSA rejects {ours_err_only}; passes needed {:?}", iters);
}
