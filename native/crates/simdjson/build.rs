//! Compiles simdjson's amalgamation with the shim (one translation unit: the shim's stage 2 uses
//! simdjson's internal walker), optimized in every profile: simdjson is a
//! dependency's code, as the `[profile.dev.package."*"]` setting has the others (a workspace
//! member is not covered by it).
fn main() {
    let vendor = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/simdjson-5.0.5");
    let shim = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shim.cpp");
    for file in [vendor.join("simdjson.cpp"), vendor.join("simdjson.h"), shim.clone()] {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .opt_level(3)
        .debug(false)
        .define("NDEBUG", None)
        // The shim reads errors as codes; nothing throws.
        .define("SIMDJSON_EXCEPTIONS", "0")
        .include(&vendor)
        .file(&shim)
        .warnings(false)
        .compile("rerail_simdjson");
}
