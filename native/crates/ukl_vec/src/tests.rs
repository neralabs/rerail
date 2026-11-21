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


/// The library's test kernels lower to one kernel tape, and each kernel vectorizes: inlined, its uniformity found,
/// the barrier rule proven (decision 0567, step 2).
#[test]
fn the_library_tests_lower_and_vectorize() {
    let src = super::source("/<kernels>/library_tests.uglykl", LIBRARY_TESTS, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    for name in ["t_simd", "t_group", "t_lz4", "t_grid_sort", "t_workqueue_sg", "ukl_radix_u64_scatter"] {
        let k = p.kernel(name).unwrap_or_else(|| panic!("no kernel {name}"));
        assert!(p.stacks[k as usize] > 0, "{name}'s stack");
    }
    let mut cut = 0;
    for (k, f) in p.funcs.iter().enumerate().filter(|(_, f)| f.kernel) {
        let vk = super::vector::vectorize(&p, k as u32).unwrap_or_else(|e| panic!("{}: {e}", f.name));
        let v = super::k::View::new(&vk.tape);
        cut += usize::from(v.gsync(0));
    }
    assert!(cut > 0, "some kernels have threadgroup barriers: the cut");
}

/// A kernel's vectorizing: its error, if it is refused.
fn vectorized(body: &str) -> Result<(), String> {
    let text = format!(
        "#include <ukl/memory.h>\n#include <ukl/group.h>\n\nDEFINE_KERNEL2(rule, t_rule,\n               RWBUFFER, uint32_t, out, \"[64]\",\n               LID0,     uint32_t, lid, ,\n               LANE,     uint16_t, lane, )\n{{\n{body}\n}}\n"
    );
    let src = super::source("/<kernels>/rule.uglykl", &text, &[]).map_err(|e| e.to_string())?;
    let tape = super::c_tape(&src).map_err(|e| e.to_string())?;
    let p = super::lower::lower(&src, &tape).map_err(|e| e.to_string())?;
    let k = p.kernel("t_rule").expect("the kernel");
    super::vector::vectorize(&p, k).map(|_| ()).map_err(|e| e.0)
}

/// A barrier some of the group's threads skip, which the rest then meet again, breaks the barrier rule: the kernel
/// is refused, naming the barrier and the branch (decision 0567 §3).
#[test]
fn a_barrier_some_threads_skip_is_refused_with_its_place() {
    let e = vectorized("    if (lid < 16u)\n        ukl_tg_barrier();\n    ukl_tg_barrier();\n    out[lid] = 1u;").unwrap_err();
    assert!(e.contains("0567 §3"), "{e}");
    assert!(e.contains("rule.uglykl:9"), "the branch's place: {e}");
}

/// A SIMD-group operation under a branch that varies across its lanes, whose other lanes meet another one, is
/// refused too.
#[test]
fn a_simd_group_operation_some_lanes_skip_is_refused() {
    let e = vectorized("    uint32_t x = 0u;\n    if ((lid & 1u) != 0u)\n        x = ukl_sg_sum(lid __UKL_ARGS);\n    out[lid] = ukl_sg_sum(x __UKL_ARGS);").unwrap_err();
    assert!(e.contains("SIMD group"), "{e}");
}

/// Threads that leave and end without reaching another sync point are let off: an early return before the
/// barriers, a barrier in a branch whose other threads then end.
#[test]
fn threads_that_end_are_let_off() {
    vectorized("    if (lid >= 48u)\n        return;\n    ukl_tg_barrier();\n    out[lid] = 1u;\n    ukl_tg_barrier();").unwrap();
    vectorized("    if (lid < 16u) {\n        ukl_tg_barrier();\n        out[lid] = 2u;\n    }").unwrap();
}
