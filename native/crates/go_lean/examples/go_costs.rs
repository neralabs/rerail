//! The parser's cost over a list of files, for `/usr/bin/time -l` or a profiler:
//! `costs <list> <passes>`. The files are read first; each pass tapes
//! every one.
/// The columns' digest, for comparing two builds' tapes.
fn digest(h: &mut std::collections::hash_map::DefaultHasher, t: &rerail_tape_format::Columns) {
    use std::hash::Hash;
    (&t.code, &t.size, &t.base, &t.off, &t.tail, t.widths, &t.keys).hash(h);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Where `main` was loaded, written to the file named, for symbolizing a profile's addresses.
    if let Some(at) = std::env::var_os("RERAIL_COSTS_MAIN") {
        std::fs::write(at, format!("{:#x}", main as fn() as usize)).unwrap();
    }
    let list = std::fs::read_to_string(&args[1]).unwrap();
    let passes: usize = args.get(2).map_or(1, |p| p.parse().unwrap());
    let files: Vec<Vec<u8>> = list.lines().filter(|l| !l.is_empty()).filter_map(|p| std::fs::read(p).ok()).collect();
    // Only the files the parser reads: a refused file's attempt is not the tape's cost.
    let all = files.len();
    let files: Vec<Vec<u8>> = files.into_iter().filter(|f| rerail_go_lean::tape(f).is_ok()).collect();
    // `RERAIL_COSTS_LEX`: the lexer alone over the same files, every token as the parser's buffer
    // lexes it.
    if std::env::var_os("RERAIL_COSTS_LEX").is_some() {
        let (mut toks, mut comments) = (0, 0);
        for _ in 0..passes {
            for f in &files {
                let (t, c) = rerail_go_lean::lex_only(f).expect("a file the parser reads is lexed");
                toks += t;
                comments += c;
            }
        }
        eprintln!("lexed alone: {toks} tokens, {comments} comments, {passes} passes");
        return;
    }
    let (mut n, mut bytes) = (0usize, 0usize);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for pass in 0..passes {
        for f in &files {
            if let Ok(t) = rerail_go_lean::tape(f) {
                if pass == 0 {
                    digest(&mut h, &t);
                }
                n += t.code.len();
                bytes += f.len();
            }
        }
    }
    eprintln!("{} files ({} read), {passes} passes, {n} nodes, {bytes} bytes taped, digest {:016x}", all, files.len(), std::hash::Hasher::finish(&h));
}
