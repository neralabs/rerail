use rerail_c_pp::tape::{IDENTIFIER, image};
use rerail_metal::pp_eval::{Unit, name_key, names};
use rerail_tape_format::View;

/// `a == b`, or the difference printed and a panic (Relax keeps no assert's message).
#[track_caller]
fn same<T: PartialEq + std::fmt::Debug>(a: T, b: T, what: impl std::fmt::Display) {
    if a != b {
        eprintln!("MISMATCH {what}: {a:?} for {b:?}");
        panic!("{what}");
    }
}

/// Files of the corpora (`RERAIL_PP_COUNT` of each language, 40 by default), each with its tape.
fn corpus() -> Vec<(Vec<u8>, Vec<u8>)> {
    let root = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus");
    let count = std::env::var("RERAIL_PP_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(40);
    let mut out = Vec::new();
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths.into_iter().take(count) {
            let src = std::fs::read(&p).unwrap();
            let tape = image(&src, crate::pp::options(cpp));
            out.push((src, tape));
        }
    }
    out
}

/// The device's reader gives every node of every file as `View` does, and each identifier leaf's
/// key is its bytes' (`name_key`), every other node's 0; the unit's node bases are the files'
/// counts' prefix sum.
#[test]
fn the_names_pass_reads_the_tapes_in_place() {
    let mut files = vec![
        (b"#define F(a) a + x\nint y = F(abcdefghijklmnopq);\n".to_vec(), Vec::new()),
        (b"".to_vec(), Vec::new()),
        (b"x\n".to_vec(), Vec::new()),
    ];
    for f in &mut files {
        f.1 = image(&f.0, crate::pp::options(true));
    }
    files.extend(corpus());
    let unit = Unit::load(&files.iter().map(|(s, t)| (s.as_slice(), t.as_slice())).collect::<Vec<_>>()).unwrap();
    let got = names(&unit, true).unwrap();
    let mut g = 0usize;
    for (f, (src, tape)) in files.iter().enumerate() {
        let v = View::new(tape).unwrap();
        same(got.bases[f] as usize, g, format!("file {f}'s first node"));
        for i in 0..v.len() {
            let want = [v.key(i), v.start(i), v.end(i), v.size(i) as u32];
            same(got.nodes[g], want, format!("file {f} node {i}"));
            let key = if (v.key(i) & 0x3ff) as u16 == IDENTIFIER { name_key(&src[v.start(i) as usize..v.end(i) as usize]) } else { 0 };
            same(got.keys[g], key, format!("file {f} node {i}'s key"));
            g += 1;
        }
    }
    same(got.total as usize, g, "the total");
    eprintln!("{} files, {g} nodes", files.len());
}
