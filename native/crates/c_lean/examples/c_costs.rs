//! The parser's cost over a list of files, for `/usr/bin/time -l` or a profiler:
//! `c_costs <c|cpp> <list> <passes>`. The files are read first; each pass tapes every one.

/// The tape's digest, for comparing two builds' tapes.
fn digest(h: &mut std::collections::hash_map::DefaultHasher, t: &rerail_tape_format::f16::Finished) {
    use std::hash::Hash;
    t.tape.as_bytes().hash(h);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Where `main` was loaded, written to the file named, for symbolizing a profile's addresses.
    if let Some(at) = std::env::var_os("RERAIL_COSTS_MAIN") {
        std::fs::write(at, format!("{:#x}", main as fn() as usize)).unwrap();
    }
    let dialect = if args[1] == "c" { rerail_c_lean::Dialect::C } else { rerail_c_lean::Dialect::Cpp };
    let list = std::fs::read_to_string(&args[2]).unwrap();
    let passes: usize = args.get(3).map_or(1, |p| p.parse().unwrap());
    let files: Vec<Vec<u8>> = list.lines().filter(|l| !l.is_empty()).filter_map(|p| std::fs::read(p).ok()).collect();
    // Only the files the parser reads: a refused file's attempt is not the tape's cost.
    let all = files.len();
    // `RERAIL_COSTS_ALL`: every file, read or not (texts made to measure one shape of token).
    let every = std::env::var_os("RERAIL_COSTS_ALL").is_some();
    let files: Vec<Vec<u8>> = files.into_iter().filter(|f| every || rerail_c_lean::tape(f, dialect).is_ok()).collect();
    // `RERAIL_COSTS_LEX`: the lexer alone over the same files, every token as the parser's buffer
    // lexes it.
    // `RERAIL_COSTS_LEX=stage1`: stage 1 alone (its masks).
    if std::env::var_os("RERAIL_COSTS_LEX").is_some_and(|v| v == "stage1") {
        let mut nul = 0;
        for _ in 0..passes {
            for f in &files {
                nul += usize::from(rerail_c_lean::stage1_only(f).is_some());
            }
        }
        eprintln!("stage 1 alone: {nul} texts with a NUL, {passes} passes");
        return;
    }
    if std::env::var_os("RERAIL_COSTS_LEX").is_some() {
        let (mut toks, mut comments) = (0, 0);
        for _ in 0..passes {
            for f in &files {
                // A file read only by lexing a directive's argument again (a raw string in a `#define`) is left out.
                let Some((t, c)) = rerail_c_lean::lex_only(f, dialect) else { continue };
                toks += t;
                comments += c;
            }
        }
        eprintln!("lexed alone: {toks} tokens, {comments} comments, {passes} passes");
        return;
    }
    let (mut n, mut bytes, mut read) = (0usize, 0usize, 0usize);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for pass in 0..passes {
        for f in &files {
            if let Ok(t) = rerail_c_lean::tape(f, dialect) {
                if pass == 0 {
                    digest(&mut h, &t);
                    read += 1;
                }
                n += t.tape.len();
                bytes += f.len();
            }
        }
    }
    eprintln!("{} files ({read} read), {passes} passes, {n} nodes, {bytes} bytes taped, digest {:016x}", all, std::hash::Hasher::finish(&h));
}
