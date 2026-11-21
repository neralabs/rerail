/// The UKL library's test kernels (`gpu/src/library_tests.uglykl`), the first gate (decision 0567, step 1).
const LIBRARY_TESTS: &str = include_str!("../../../../gpu/src/library_tests.uglykl");

/// The library's test kernels preprocess as OpenCL C with 32-lane sub-groups, and C's parser reads them whole.
#[test]
fn the_library_tests_are_one_c_tape() {
    let src = super::source("/<kernels>/library_tests.uglykl", LIBRARY_TESTS, &[]).unwrap();
    assert!(src.text().contains("sub_group_ballot"), "the sub-groups' builtins");
    let tape = super::c_tape(&src).unwrap();
    assert!(tape.code.len() / 2 > 10_000, "a tape of the whole source");
}

/// Writes the preprocessed library tests to `RERAIL_UKL_VEC_DUMP`, when set: for reading what the parser sees.
#[test]
fn the_library_tests_source_is_dumped_when_asked() {
    if let Ok(path) = std::env::var("RERAIL_UKL_VEC_DUMP") {
        std::fs::write(path, super::source("/<kernels>/library_tests.uglykl", LIBRARY_TESTS, &[]).unwrap().text()).unwrap();
    }
}

