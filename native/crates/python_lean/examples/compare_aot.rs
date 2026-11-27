//! The parser as Rust (`rerail-python-aot`, the Bitlex compiler's) against the lean parser, over the `.py` files under a
//! directory, each side many times: `compare_aot <dir> <seconds a side>`. Only the files both parse are timed. Each
//! side's lexer alone (the AOT's tier 1, the lean parser's tokens), then its whole parse (each to format 16), on one thread.
//! Each file is read into a buffer padded for tier 1 (`rerail_python_aot::PADDING`).
const PAD: usize = rerail_python_aot::PADDING;

fn files(dir: &std::path::Path, out: &mut Vec<Vec<u8>>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<_> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|e| e == "py")
            && let Ok(b) = std::fs::read(&p)
        {
            out.push(rerail_python_aot::padded(&b));
        }
    }
}

/// `f` over every file, in passes, for `secs`: ns a byte, and a sum that keeps the work.
fn time(fs: &[Vec<u8>], secs: f64, f: impl Fn(&[u8]) -> usize) -> (f64, usize) {
    let bytes: usize = fs.iter().map(|b| b.len() - PAD).sum();
    let t0 = std::time::Instant::now();
    let (mut n, mut passes) = (0usize, 0usize);
    while passes == 0 || t0.elapsed().as_secs_f64() < secs {
        for b in fs {
            n = n.wrapping_add(f(b));
        }
        passes += 1;
    }
    (t0.elapsed().as_secs_f64() * 1e9 / (bytes * passes) as f64, n)
}

/// The parser as Rust's keys (`rerail_python_aot::unkey`) by their names in Python's table: the lean parser's kinds,
/// fields and supertype sets. Its supertypes' ids from 0x3000 are the grammar's, in this order.
fn aot_sink() -> rerail_tape_format::Named {
    const SUPERTYPES: &[&str] = &["expression", "primary_expression", "pattern", "parameter"];
    rerail_tape_format::Named::new(rerail_python_lean::TAPE_TAG, rerail_python_lean::TERMINATORS, |key| {
        let (kind, field, flags, classes) = rerail_python_aot::unkey(key);
        let &(kind, named) = rerail_python_lean::KINDS.get(usize::from(kind))?;
        let names: Vec<&str> = classes.map(|c| c.checked_sub(0x3000).and_then(|j| SUPERTYPES.get(usize::from(j))).copied()).collect::<Option<_>>()?;
        let extra = flags & rerail_tape_format::CODE_EXTRA != 0;
        let classes = if extra { &[][..] } else { rerail_python_lean::CLASSES.iter().copied().find(|set| set.len() == names.len() && names.iter().all(|n| set.contains(n)))? };
        Some(rerail_tape_format::KeyNames { kind, named, field: rerail_python_lean::FIELDS.get(usize::from(field)).copied()?, flags, classes })
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let secs: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let mut all = Vec::new();
    files(std::path::Path::new(&args[1]), &mut all);
    let total = all.len();
    let both: Vec<Vec<u8>> = all.into_iter().filter(|b| rerail_python_aot::parse(b, b.len() - PAD, &mut aot_sink()).ok().is_some_and(|t| t.is_ok()) && rerail_python_lean::tape(&b[..b.len() - PAD]).is_ok()).collect();
    let bytes: usize = both.iter().map(|b| b.len() - PAD).sum();
    eprintln!("{} of {total} files both parse, {bytes} bytes", both.len());
    let (aot_lex, a) = time(&both, secs, |b| rerail_python_aot::lex(b, b.len() - PAD).recs.len());
    let (lean_lex, b) = time(&both, secs, |b| rerail_python_lean::token_count(&b[..b.len() - PAD]).unwrap_or(0));
    let (aot, c) = time(&both, secs, |b| rerail_python_aot::parse(b, b.len() - PAD, &mut aot_sink()).ok().and_then(Result::ok).map_or(0, |t| t.tape.len()));
    let (lean, d) = time(&both, secs, |b| rerail_python_lean::tape(&b[..b.len() - PAD]).map_or(0, |t| t.tape.len()));
    eprintln!("lexer: aot {aot_lex:.2} ns/B, lean {lean_lex:.2} ns/B ({:.2}x)   [{a} {b}]", aot_lex / lean_lex);
    eprintln!("parse: aot {aot:.2} ns/B, lean {lean:.2} ns/B ({:.2}x)   [{c} {d}]", aot / lean);
}
