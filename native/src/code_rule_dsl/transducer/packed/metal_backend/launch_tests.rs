    use super::launch::*;
    use super::*;

    #[test]
    fn production_encoders_cannot_bypass_the_launch_guard() {
        // The engine's Metal files, and every file of the Metal crate (its runtime is the old
        // `metal_backend.rs`, its launch layer `launch.rs`).
        let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let engine = std::fs::read_dir(root.join("src/code_rule_dsl/transducer/packed")).unwrap();
        let crate_files = std::fs::read_dir(root.join("crates/metal/src")).unwrap();
        for (in_crate, entry) in engine.map(|e| (false, e)).chain(crate_files.map(|e| (true, e))) {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            // `*_tests.rs` files are test modules (mounted under `#[cfg(test)]`), not encoders.
            if !(in_crate || name.starts_with("metal"))
                || !name.ends_with(".rs")
                || name.ends_with("_tests.rs")
                || (in_crate && name == "launch.rs")
            {
                continue;
            }
            let name = if in_crate && name == "runtime.rs" { "metal_backend.rs" } else { name };
            let source = std::fs::read_to_string(&path).unwrap();
            // Up to its first test module (`mod` or, since the crate split, `pub mod`).
            let cut = ["#[cfg(test)]\nmod ", "#[cfg(test)]\npub mod "].iter().filter_map(|m| source.find(m)).min();
            let source = &source[..cut.unwrap_or(source.len())];
            for forbidden in [
                ".dispatch_threads(",
                ".dispatch_thread_groups(",
                ".dispatch_thread_groups_indirect(",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{name} bypasses the launch guard: {forbidden}"
                );
            }
            if name != "metal_backend.rs" {
                assert!(
                    !source.contains(".commit()"),
                    "{name} bypasses checked submission"
                );
            } else {
                // Only the upload/readback blit commands can submit without a launch check.
                for prefix in source.split(".commit()").collect::<Vec<_>>().windows(2) {
                    let before = prefix[0];
                    assert!(
                        before.rfind("new_blit_command_encoder")
                            > before.rfind("new_compute_command_encoder"),
                        "unchecked compute submission in {name}"
                    );
                }
            }
        }
    }
