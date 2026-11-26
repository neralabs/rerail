//! The codes' directory (decision 0697, `codes`): each shipped table's kind and field atoms, written once from
//! `tables/<tag>.tsv` into the binary, by the crate's own reading of the tables (`src/codes.rs`).

#[path = "src/codes.rs"]
#[allow(dead_code)]
mod codes;

fn main() {
    let dir = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("tables");
    println!("cargo:rerun-if-changed={}", dir.display());
    println!("cargo:rerun-if-changed=src/codes.rs");
    let mut texts: Vec<(u8, String)> = std::fs::read_dir(&dir)
        .expect("the tables")
        .filter_map(|e| {
            let p = e.expect("a table").path();
            let tag = p.file_name()?.to_str()?.strip_suffix(".tsv")?.parse::<u8>().ok()?;
            println!("cargo:rerun-if-changed={}", p.display());
            Some((tag, std::fs::read_to_string(&p).expect("a table's text")))
        })
        .collect();
    texts.sort_unstable_by_key(|t| t.0);
    let tables: Vec<(u8, &str)> = texts.iter().map(|(t, s)| (*t, s.as_str())).collect();
    let image = codes::image(&tables).unwrap_or_else(|e| panic!("the codes' directory: {e}"));
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("codes.bin"), &image).expect("codes.bin");
    std::fs::write(out.join("codes_len.rs"), format!("{}\n", image.len())).expect("codes_len.rs");
}
