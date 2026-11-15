//! The lean `.rr` parser's cost over a list of files, for `/usr/bin/time -l` or a profiler:
//! `costs <list> <passes>`. The files are read first; each pass tapes every one.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let list = std::fs::read_to_string(&args[1]).unwrap();
    let passes: usize = args.get(2).map_or(1, |p| p.parse().unwrap());
    let root = args.get(3).map_or(".", String::as_str);
    let files: Vec<String> = list.lines().filter(|l| !l.is_empty()).filter_map(|p| std::fs::read_to_string(std::path::Path::new(root).join(p)).ok()).collect();
    let (mut n, mut bytes) = (0usize, 0usize);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for pass in 0..passes {
        for f in &files {
            let t = rerail_rr_syntax::tape(f);
            if pass == 0 {
                std::hash::Hash::hash(&t, &mut h);
            }
            n += t.len();
            bytes += f.len();
        }
    }
    eprintln!("{} files, {passes} passes, {n} tape bytes, {bytes} bytes, digest {:016x}", files.len(), std::hash::Hasher::finish(&h));
}
