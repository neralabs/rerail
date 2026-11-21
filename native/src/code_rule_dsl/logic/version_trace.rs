//! `RERAIL_VERSION_TRACE=<dir>`: a measurement aid for decision 0173's version simulation. Each place a program is
//! made from another (the survey's E1 to E6) notes the two: each distinct program's text, printed from its version,
//! once, as `<digest>.vrr`, and a line `site\tfrom\tto` in `trace.tsv`. Off, it costs one environment read a making.
use std::hash::BuildHasher;
use std::io::Write;
use std::path::Path;

/// Whether the trace is on.
pub fn on() -> bool {
    std::env::var_os("RERAIL_VERSION_TRACE").is_some()
}

/// Notes program `from` made into `to` at `site`, each printed from its handles (`Version::show`, 0276 §6) as
/// `<digest>.vrr`.
pub fn note(site: &str, from: &super::program_image::Version, to: &super::program_image::Version) {
    let Some(dir) = std::env::var_os("RERAIL_VERSION_TRACE") else { return };
    let dir = Path::new(&dir);
    let _ = std::fs::create_dir_all(dir);
    let (f, t) = (save_text(dir, from.show(), "vrr"), save_text(dir, to.show(), "vrr"));
    if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("trace.tsv")) {
        let _ = out.write_all(format!("{site}\t{f}\t{t}\n").as_bytes());
    }
}

fn save_text(dir: &Path, text: String, ext: &str) -> String {
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
