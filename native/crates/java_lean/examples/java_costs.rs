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
    let list = std::fs::read_to_string(&args[1]).unwrap();
    let passes: usize = args.get(2).map_or(1, |p| p.parse().unwrap());
    let files: Vec<Vec<u8>> = list.lines().filter(|l| !l.is_empty()).filter_map(|p| std::fs::read(p).ok()).collect();
    let (mut n, mut bytes) = (0usize, 0usize);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for pass in 0..passes {
        for f in &files {
            if let Ok(t) = rerail_java_lean::tape(f) {
                if pass == 0 {
                    digest(&mut h, &t);
                }
                n += t.keys.len();
                bytes += f.len();
            }
        }
    }
    eprintln!("{} files, {passes} passes, {n} nodes, {bytes} bytes taped, digest {:016x}", files.len(), std::hash::Hasher::finish(&h));
}
