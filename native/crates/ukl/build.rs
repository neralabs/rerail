//! Computes the carried files' fingerprint (`rerail_ukl::fingerprint`) once, when the crate is built: FNV-1a over
//! each file's name and text, each followed by a 0, in `carried.rs`'s order.

include!("src/carried.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/carried.rs");
    // The files `carried.rs` includes: the headers under `forest/src/`, the library under `lib/`.
    for (name, _) in HEADERS {
        println!("cargo:rerun-if-changed=../../../forest/src/{name}");
    }
    for (name, _) in LIBRARY {
        println!("cargo:rerun-if-changed=lib/{name}");
    }
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for (name, text) in HEADERS.iter().chain(LIBRARY) {
        for b in name.bytes().chain([0]).chain(text.bytes()).chain([0]) {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    std::fs::write(out.join("fingerprint.rs"), format!("const FINGERPRINT: u64 = {h:#018x};\n")).expect("the fingerprint written");
}
