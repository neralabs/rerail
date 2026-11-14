//! `RERAIL_VERSION_TRACE=<dir>`: a measurement aid for decision 0173's version simulation. Each place a program is
//! made from another (the survey's E1 to E6) notes the two: each distinct program's text, once, as `<digest>.rr`
//! (or its `Debug` form, `<digest>.dbg`, where the text has no form for it), and a line `site\tfrom\tto` in
//! `trace.tsv`. Off, it costs one environment read a making.
use super::front::Program;
use std::hash::BuildHasher;
use std::io::Write;
use std::path::Path;

pub fn note(site: &str, from: &Program, to: &Program) {
    let Some(dir) = std::env::var_os("RERAIL_VERSION_TRACE") else { return };
    let dir = Path::new(&dir);
    let _ = std::fs::create_dir_all(dir);
    let (f, t) = (save(dir, from), save(dir, to));
    if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("trace.tsv")) {
        let _ = out.write_all(format!("{site}\t{f}\t{t}\n").as_bytes());
    }
}

fn save(dir: &Path, prog: &Program) -> String {
    let (text, ext) = match super::program_text::print(prog) {
        Ok(t) => (t, "rr"),
        Err(_) => (format!("{prog:?}"), "dbg"),
    };
    let digest = format!("{:016x}", foldhash::quality::FixedState::with_seed(0x0173).hash_one(&text));
    let path = dir.join(format!("{digest}.{ext}"));
    if !path.exists() {
        let tmp = dir.join(format!("{digest}.{}.tmp", std::process::id()));
        if std::fs::write(&tmp, &text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
    digest
}
