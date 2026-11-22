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
        ssa_of(&p, &vk);
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

/// The interpreter runs a kernel's code tape a whole register an instruction: 32-bit arithmetic, comparisons and a
/// branch that varies across the lanes are fused opcodes, never the generic ones that go lane by lane (decision 0567
/// §6), and the result is each thread's.
#[test]
fn the_interpreter_runs_fused_opcodes_and_each_thread_gets_its_value() {
    use super::code::{HEADER, I, WORDS};
    let body = "    uint32_t x = lid * 3u + (lid >> 1);\n    if ((lid & 1u) != 0u)\n        x = x ^ 0xffu;\n    else if (lid < 40u)\n        x = x - 7u;\n    out[lid] = x;";
    let text = format!(
        "#include <ukl/memory.h>\n#include <ukl/group.h>\n\nDEFINE_KERNEL2(rule, t_rule,\n               RWBUFFER, uint32_t, out, \"[64]\",\n               LID0,     uint32_t, lid, ,\n               LANE,     uint16_t, lane, )\n{{\n{body}\n}}\n"
    );
    let src = super::source("/<kernels>/rule.uglykl", &text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap();
    let k = super::vector::vectorize(&p, p.kernel("t_rule").unwrap()).unwrap();
    let v = super::k::View::new(&k.code);
    let w = v.p(0);
    let code = &w[HEADER + 2 * w[2] as usize..][..w[7] as usize];
    // SAFETY: the words are `compile`'s instructions.
    let ops: Vec<I> = code.chunks(WORDS).map(|i| unsafe { I::of(i[0]) }).collect();
    for want in [I::Mul32, I::Add32, I::ShrU32, I::Xor32, I::MAnd] {
        assert!(ops.contains(&want), "{want:?} in {ops:?}");
    }
    assert!(!ops.iter().any(|o| matches!(o, I::VBinGen | I::VUnGen | I::VCastGen)), "a generic opcode in {ops:?}");
    let out = run_on(&p, &k, super::run::Exec::Interpret);
    assert_eq!(run_on(&p, &k, super::run::Exec::Jit), out, "compiled against interpreted");
    ssa_runs_equal(&p, &k, &out);
    let want: Vec<u32> = (0..64u32)
        .map(|l| {
            let x = l * 3 + (l >> 1);
            if l & 1 != 0 { x ^ 0xff } else if l < 40 { x.wrapping_sub(7) } else { x }
        })
        .collect();
    assert_eq!(out, want);
}

/// A kernel kept as bytes (a store's, `Kernel::to_bytes`) is the same kernel read back: its tapes, name and flag; and
/// bytes that are not one's are refused.
#[test]
fn a_kernel_kept_as_bytes_reads_back_the_same() {
    let text = "#include <ukl/memory.h>\n#include <ukl/group.h>\n\nDEFINE_KERNEL2(rule, t_rule,\n               RWBUFFER, uint32_t, out, \"[64]\",\n               LID0,     uint32_t, lid, ,\n               LANE,     uint16_t, lane, )\n{\n    out[lid] = lid * 3u + ukl_sg_sum(lid __UKL_ARGS);\n}\n";
    let src = super::source("/<kernels>/rule.uglykl", text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap();
    let k = super::vector::vectorize(&p, p.kernel("t_rule").unwrap()).unwrap();
    let bytes = k.to_bytes();
    let back = super::vector::Kernel::from_bytes(&bytes).expect("the kernel's bytes");
    assert_eq!((back.name.as_str(), back.lid), (k.name.as_str(), k.lid));
    assert_eq!((&back.tape.image, &back.tape.words, &back.code.image, &back.code.words), (&k.tape.image, &k.tape.words, &k.code.image, &k.code.words));
    assert!(super::vector::Kernel::from_bytes(&bytes[..bytes.len() - 1]).is_none(), "a kernel cut short");
    let mut other = bytes.clone();
    other[7] ^= 1;
    assert!(super::vector::Kernel::from_bytes(&other).is_none(), "another version's");
}

/// Every kernel of each source in the directory `RERAIL_UKL_VEC_SOURCES` names (the device's libraries, written by
/// `rerail-gpu`'s `vec::library` with the same variable set) lowers and vectorizes: for the kernels beyond the
/// library's tests (decision 0567 step 3's gate: `join_stages`, `pp_grid`).
#[test]
fn the_written_sources_lower_and_vectorize() {
    let Ok(dir) = std::env::var("RERAIL_UKL_VEC_SOURCES") else { return };
    let mut failed = Vec::new();
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "uglykl")).collect();
    paths.sort();
    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap();
        let name = format!("/<kernels>/{}", path.file_name().unwrap().to_string_lossy());
        let p = match super::program(&super::preprocess().file(&name, text.as_str()), &name) {
            Ok(p) => p,
            Err(e) => {
                failed.push(format!("{name}: {e}"));
                continue;
            }
        };
        let kernels: Vec<_> = p.funcs.iter().enumerate().filter(|(_, f)| f.kernel).map(|(k, f)| (k, f.name.clone())).collect();
        let only = std::env::var("RERAIL_UKL_VEC_ONLY").ok();
        for (k, f) in kernels {
            if only.as_ref().is_some_and(|o| *o != f) {
                continue;
            }
            if std::env::var("RERAIL_UKL_VEC_TRACE").is_ok() {
                eprintln!("{f}: {} (the kernel tape {} nodes)", super::vector::inlined_size(&p, k as u32), super::k::View::new(&p.tape).len());
            }
            let t = std::time::Instant::now();
            match super::vector::vectorize(&p, k as u32) {
                Err(e) => failed.push(format!("{name} {f}: {e}")),
                Ok(vk) => {
                    // The kernel's SSA, compiled at a width of 8 (32 if it is the group's).
                    let vt = t.elapsed().as_secs_f64();
                    let t = std::time::Instant::now();
                    let names = |id: u32| if id == u32::MAX { vk.name.clone() } else { p.funcs[id as usize].name.clone() };
                    let ssa = super::ssa::build(&vk.tape, &names, &|id| p.funcs[id as usize].called);
                    if let (Ok(dir), Ok(s)) = (std::env::var("RERAIL_UKL_VEC_SSA"), &ssa) {
                        std::fs::write(format!("{dir}/{f}.ssa"), s.listing()).unwrap();
                    }
                    let st = t.elapsed().as_secs_f64();
                    let t = std::time::Instant::now();
                    let c = ssa.as_ref().map_err(|e| e.0.clone()).and_then(|s| super::cl::compile(s, &vk.name, std::env::var("RERAIL_UKL_WIDTH").ok().and_then(|w| w.parse().ok()).unwrap_or(8), &super::exec::places()).map(|_| ()).map_err(|e| e.0));
                    if std::env::var("RERAIL_UKL_VEC_TIMES").is_ok() {
                        let values = ssa.as_ref().map_or(0, |s| s.funcs.iter().map(|f| f.insts.len()).sum::<usize>());
                        eprintln!("TIME {f}: vectorized {vt:.2} s ({} nodes), SSA {st:.3} s ({values} values), compiled {:.3} s", super::k::View::new(&vk.tape).len(), t.elapsed().as_secs_f64());
                    }
                    if let Err(e) = c {
                        failed.push(format!("{name} {f}: {e}"));
                    }
                }
            }
        }
        eprintln!("{name}: {} kernels", p.funcs.iter().filter(|f| f.kernel).count());
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// Each thread's `out[0..64)` from a kernel of `body` (`out`, `lid`, `lane` its parameters) run over one group of 64.
fn ran(body: &str) -> Vec<u32> {
    ran_with("", body)
}

/// The same, with `prelude` (functions, types) before the kernel.
fn ran_with(prelude: &str, body: &str) -> Vec<u32> {
    let text = format!(
        "#include <ukl/memory.h>\n#include <ukl/group.h>\n{prelude}\nDEFINE_KERNEL2(rule, t_rule,\n               RWBUFFER, uint32_t, out, \"[64]\",\n               LID0,     uint32_t, lid, ,\n               LANE,     uint16_t, lane, )\n{{\n{body}\n}}\n"
    );
    let src = super::source("/<kernels>/rule.uglykl", &text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("t_rule").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    ssa_of(&p, &k);
    let interpreted = run_on(&p, &k, super::run::Exec::Interpret);
    assert_eq!(run_on(&p, &k, super::run::Exec::Jit), interpreted, "compiled against interpreted");
    ssa_runs_equal(&p, &k, &interpreted);
    interpreted
}

/// Each thread's `out[0..64)` from kernel `k` of `p` run over one group of 64 by `exec`.
fn run_on(p: &super::lower::Program, k: &super::vector::Kernel, exec: super::run::Exec) -> Vec<u32> {
    let mut out = vec![0u32; 64];
    let ptr = (out.as_mut_ptr() as u64).to_le_bytes().to_vec();
    let args = [super::run::Arg::Bytes(ptr), super::run::Arg::Bytes(vec![0; 8]), super::run::Arg::Bytes(vec![0; 4])];
    super::run::run(p, k, &args, super::run::Launch { grid: [1, 1, 1], block: [64, 1, 1], local: 0, resident: 1, exec }).unwrap_or_else(|e| panic!("{exec:?}: {}", e.0));
    out
}

/// 64-bit unsigned division and comparison are unsigned, on a uniform value and on one that varies: the overflow
/// check of a decimal constant (`pp`'s `if_number`: `v > (0xffff_ffff_ffff_ffff - d) / 10`).
#[test]
fn sixty_four_bit_unsigned_division_is_unsigned() {
    let out = ran("    uint64_t d = (uint64_t)(lid % 10u);\n    uint64_t q = (0xfffffffffffffffful - d) / (uint64_t)(10u);\n    uint64_t u = (0xfffffffffffffffful - 5ul) / 10ul;\n    uint64_t v = 1844674407370955161ul;\n    out[lid] = (uint32_t)(q >> 32) ^ (uint32_t)(q) ^ (v > u ? 0x80000000u : 0u) ^ (v > q ? 0x40000000u : 0u);");
    let want: Vec<u32> = (0..64u64)
        .map(|l| {
            let q = (u64::MAX - l % 10) / 10;
            let (u, v) = ((u64::MAX - 5) / 10, 1844674407370955161u64);
            (q >> 32) as u32 ^ q as u32 ^ if v > u { 0x8000_0000 } else { 0 } ^ if v > q { 0x4000_0000 } else { 0 }
        })
        .collect();
    assert_eq!(out, want);
}

/// Lanes of a SIMD group exchange values through memory at a SIMD group barrier (`ukl_sg_barrier`, as `ukl/lz4.h`'s
/// copies do): compiled at a width of fewer lanes, the barrier ends a chunk, so each lane reads its neighbour's write.
#[test]
fn a_simd_group_barrier_shows_each_lanes_write_to_the_others() {
    let out = ran("    out[lid] = lid * 7u + 1u;\n    ukl_sg_barrier();\n    uint32_t v = out[lid ^ 5u];\n    ukl_sg_barrier();\n    out[lid] = v;");
    let want: Vec<u32> = (0..64u32).map(|l| (l ^ 5) * 7 + 1).collect();
    assert_eq!(out, want);
}

/// A division whose divisor is 0 only in lanes that do not run it divides in the lanes that do: the kernel does not
/// fail (a uniform divisor too).
#[test]
fn a_division_by_zero_in_lanes_that_do_not_run_it_is_none() {
    let out = ran("    uint32_t d = lid % 4u;\n    uint32_t q = 0u;\n    if (d != 0u) q = (1000u + lid) / d + (77u % d);\n    uint64_t e = (uint64_t)(lid & 3u);\n    if (e != 0ul) q += (uint32_t)(((uint64_t)(lid) << 33) / e);\n    uint32_t u = out[0] * 0u;\n    if (lid > 100u) q += 5u / u;\n    out[lid] = q;");
    let want: Vec<u32> = (0..64u32)
        .map(|l| {
            let d = l % 4;
            let mut q = if d != 0 { (1000 + l) / d + 77 % d } else { 0 };
            let e = u64::from(l & 3);
            if e != 0 {
                q = q.wrapping_add(((u64::from(l) << 33) / e) as u32);
            }
            q
        })
        .collect();
    assert_eq!(out, want);
}

/// A `switch` on a 64-bit value that varies: each lane's case.
#[test]
fn a_switch_on_a_sixty_four_bit_value_runs_each_lanes_case() {
    let out = ran("    uint64_t v = ((uint64_t)(lid % 5u) << 40) | 3ul;\n    uint32_t x = 1u;\n    switch (v) {\n    case 3ul: x = 10u; break;\n    case (1ul << 40) | 3ul: x = 20u; break;\n    case (3ul << 40) | 3ul: x = 40u;\n    default: x += 2u;\n    }\n    out[lid] = x;");
    let want: Vec<u32> = (0..64u32).map(|l| [10, 20, 3, 42, 3][(l % 5) as usize]).collect();
    assert_eq!(out, want);
}

/// After some lanes return, a value the SIMD group shares (`lid / 32`) addresses a load: a chunk of fewer lanes than
/// the group's whose lanes have all returned reads nothing, and takes the value from no lane past its own.
#[test]
fn a_shared_address_after_lanes_return_reads_for_the_lanes_left() {
    // Each SIMD group reads the other's word 3 (the first group's, written before the second runs; the second's,
    // not yet written when the first runs): no lane reads what another of its own group writes.
    let out = ran("    out[lid] = lid + 100u;\n    if (lid % 8u >= 3u) return;\n    uint32_t g = lid / 32u;\n    uint32_t v = out[(1u - g) * 32u + 3u];\n    out[lid] = v + lid;");
    let want: Vec<u32> = (0..64u32).map(|l| if l % 8 >= 3 { l + 100 } else if l < 32 { l } else { 103 + l }).collect();
    assert_eq!(out, want);
}

/// `pp`'s decimal reader (`if_number`, `pb_if_text`'s) on a token each lane holds: the largest 64-bit value's 20
/// digits is one, a 20-digit value past it is not, a hex constant with a suffix is one.
#[test]
fn a_constant_reader_reads_each_lanes_token() {
    const IF_NUMBER: &str = "#define VF_U (1u)\n#define SFX1(a) ((uint32_t)(a))\n#define SFX2(a, b) ((uint32_t)(a) | ((uint32_t)(b) << 8))\n#define SFX3(a, b, c) ((uint32_t)(a) | ((uint32_t)(b) << 8) | ((uint32_t)(c) << 16))\nUKL_INLINE uint32_t if_digit(uint8_t c) {\n    if (c >= '0' && c <= '9') return (uint32_t)(c - '0');\n    if (c >= 'a' && c <= 'z') return (uint32_t)(c - 'a') + 10u;\n    if (c >= 'A' && c <= 'Z') return (uint32_t)(c - 'A') + 10u;\n    return 99u;\n}\ntypedef struct IfVal { uint64_t bits; uint32_t f; bool ok; } IfVal;\n\nUKL_INLINE IfVal if_val(bool ok, uint64_t bits, uint32_t f) {\n    IfVal r;\n    r.ok = ok; r.bits = bits; r.f = f;\n    return r;\n}\n\n// An integer constant's value (`expr::number`): none for a floating constant, a bad suffix, or a\n// value past 64 bits.\nUKL_INLINE IfVal if_number(UKL_PRIVATE const uint8_t* t, uint32_t n) {\n    // The first two characters, the radix's prefix.\n    uint8_t c0 = 0u, c1 = 0u;\n    uint32_t m = 0u;\n    for (uint32_t k = 0u; k < n && m < 2u; k++) {\n        if (t[k] == ((uint8_t)('\\''))) continue;\n        if (m == 0u) c0 = t[k]; else c1 = t[k];\n        m++;\n    }\n    uint32_t radix = 10u, skip = 0u;\n    if (m >= 2u && c0 == '0' && (c1 | 0x20u) == 'x') { radix = 16u; skip = 2u; }\n    else if (m >= 2u && c0 == '0' && (c1 | 0x20u) == 'b') { radix = 2u; skip = 2u; }\n    else if (m >= 1u && c0 == '0') { radix = 8u; skip = 1u; }\n    // The digits, then the suffix (at most three characters).\n    uint64_t v = 0ul;\n    uint32_t j = 0u, digits = 0u, k = 0u, sfx = 0u;\n    for (uint32_t p = 0u; p < n; p++) {\n        uint8_t c = t[p];\n        if (c == ((uint8_t)('\\''))) continue;\n        if (j++ < skip) continue;\n        if (k == 0u) {\n            uint32_t d = if_digit(c);\n            if (d < radix) {\n                if (v > (0xfffffffffffffffful - (uint64_t)(d)) / (uint64_t)(radix)) return if_val(false, 0ul, 0u);\n                v = v * (uint64_t)(radix) + (uint64_t)(d);\n                digits++;\n                continue;\n            }\n        }\n        if (k >= 3u) return if_val(false, 0ul, 0u);\n        if (c >= 'A' && c <= 'Z') c = (uint8_t)(c | 0x20u);\n        sfx |= ((uint32_t)(c)) << (8u * k);\n        k++;\n    }\n    if (radix != 10u && radix != 8u && digits == 0u) return if_val(false, 0ul, 0u);\n    bool u;\n    if (sfx == 0u || sfx == SFX1('l') || sfx == SFX2('l', 'l') || sfx == SFX1('z') || sfx == SFX2('w', 'b')) u = false;\n    else if (sfx == SFX1('u') || sfx == SFX2('u', 'l') || sfx == SFX2('l', 'u') || sfx == SFX3('u', 'l', 'l') || sfx == SFX3('l', 'l', 'u') || sfx == SFX2('u', 'z') || sfx == SFX2('z', 'u')\n             || sfx == SFX3('u', 'w', 'b') || sfx == SFX3('w', 'b', 'u')) u = true;\n    else return if_val(false, 0ul, 0u);\n    // A decimal past `intmax_t` is unsigned.\n    if (radix == 10u && v > 0x7ffffffffffffffful) u = true;\n    return if_val(true, v, u ? VF_U : 0u);\n}\n";
    let body = "    uint8_t t[24];\n    uint8_t r[24];\n    uint64_t x = lid == 0u ? 0xfffffffffffffffful : (lid == 1u ? 9999999999999999999ul : 1000ul + (uint64_t)(lid));\n    uint32_t n = 0u;\n    do { r[n] = (uint8_t)('0' + (uint32_t)(x % 10ul)); n++; x /= 10ul; } while (x != 0ul && n < 24u);\n    if (lid == 1u) { r[n] = '9'; n++; }\n    for (uint32_t k = 0u; k < n; k++) t[k] = r[n - 1u - k];\n    IfVal c = if_number(t, n);\n    out[lid] = (c.ok ? 1u : 0u) | ((uint32_t)(c.bits) & 0xfffffff0u);";
    let out = ran_with(IF_NUMBER, body);
    assert_eq!(out[0], 1 | 0xfffffff0, "the largest 64-bit value");
    assert_eq!(out[1], 0, "past it");
    for l in 2..64u32 {
        assert_eq!(out[l as usize], 1 | ((1000 + l) & 0xfffffff0), "lane {l}");
    }
    // Beside a lane that reads a hex constant with a suffix (`pb_if_text`'s texts 9 and 10, which failed together).
    let hex = "    uint8_t t[24];\n    uint32_t n = 0u;\n    if (lid == 0u) {\n        t[0] = '0'; t[1] = 'x';\n        for (uint32_t k = 2u; k < 18u; k++) t[k] = 'F';\n        t[18] = 'u'; t[19] = 'l'; t[20] = 'l'; n = 21u;\n    } else {\n        uint8_t r[24];\n        uint64_t x = 0xfffffffffffffffful;\n        do { r[n] = (uint8_t)('0' + (uint32_t)(x % 10ul)); n++; x /= 10ul; } while (x != 0ul);\n        for (uint32_t k = 0u; k < n; k++) t[k] = r[n - 1u - k];\n    }\n    IfVal c = if_number(t, n);\n    out[lid] = (c.ok ? 1u : 0u) | ((uint32_t)(c.bits) & 0xfffffff0u);";
    let out = ran_with(IF_NUMBER, hex);
    assert_eq!(out[0], 1 | 0xfffffff0, "the hex constant");
    assert_eq!(out[1], 1 | 0xfffffff0, "the decimal beside it");
}

/// A chain of `else if`s that varies across the lanes: each lane's assignments are its branch's, and a lane that
/// takes none keeps its value (`if_number`'s radix, which a lane that took the first branch gave to the lane
/// beside it).
#[test]
fn an_else_if_chain_assigns_each_lanes_branch() {
    let out = ran("    uint32_t c0 = lid == 0u ? 48u : (lid == 1u ? 49u : (lid % 3u == 0u ? 48u : 50u));\n    uint32_t c1 = lid == 0u ? 120u : (lid % 5u == 0u ? 98u : 51u);\n    uint32_t m = 2u;\n    uint32_t radix = 10u, skip = 0u;\n    if (m >= 2u && c0 == 48u && (c1 | 32u) == 120u) { radix = 16u; skip = 2u; }\n    else if (m >= 2u && c0 == 48u && (c1 | 32u) == 98u) { radix = 2u; skip = 2u; }\n    else if (m >= 1u && c0 == 48u) { radix = 8u; skip = 1u; }\n    out[lid] = radix | skip << 8;");
    let want: Vec<u32> = (0..64u32)
        .map(|l| {
            let c0 = if l == 0 { 48 } else if l == 1 { 49 } else if l % 3 == 0 { 48 } else { 50 };
            let c1 = if l == 0 { 120 } else if l % 5 == 0 { 98 } else { 51 };
            let (r, s) = if c0 == 48 && (c1 | 32) == 120 { (16, 2) } else if c0 == 48 && (c1 | 32) == 98 { (2, 2) } else if c0 == 48 { (8, 1) } else { (10, 0) };
            r | s << 8
        })
        .collect();
    assert_eq!(out, want);
}

/// `||` and `&&` that short-circuit in some lanes and go on in others: each lane's value (`if_number`'s suffix
/// chains, where the lanes whose suffix was none were refused beside one whose suffix was `ull`).
#[test]
fn short_circuits_that_vary_are_each_lanes() {
    let out = ran("    uint32_t s = lid == 0u ? 0x6c6c75u : (lid % 7u == 3u ? 0x6cu : (lid % 5u == 1u ? 0x75u : 0u));\n    bool u;\n    uint32_t r;\n    if (s == 0u || s == 0x6cu || s == 0x6c6cu || s == 0x7au) { u = false; r = 1u; }\n    else if (s == 0x75u || s == 0x6c75u || s == 0x6c6c75u) { u = true; r = 2u; }\n    else { u = false; r = 3u; }\n    bool a = s != 0u && (s & 1u) == 1u && s > 0x70u;\n    out[lid] = r | (u ? 16u : 0u) | (a ? 256u : 0u);");
    let want: Vec<u32> = (0..64u32)
        .map(|l| {
            let s = if l == 0 { 0x6c6c75 } else if l % 7 == 3 { 0x6c } else if l % 5 == 1 { 0x75 } else { 0 };
            let (u, r) = if [0, 0x6c, 0x6c6c, 0x7a].contains(&s) { (false, 1) } else if [0x75, 0x6c75, 0x6c6c75].contains(&s) { (true, 2) } else { (false, 3) };
            let a = s != 0 && s & 1 == 1 && s > 0x70;
            r | if u { 16 } else { 0 } | if a { 256 } else { 0 }
        })
        .collect();
    assert_eq!(out, want);
}

/// A `continue` under nested branches that vary, beside lanes that go on in the body: the lanes that continued
/// leave the rest of the body (`if_number`'s digits loop, whose decimal lanes took lane 0's suffix).
#[test]
fn a_continue_that_varies_leaves_the_rest_of_the_body() {
    let out = ran("    uint32_t n = lid == 0u ? 21u : 20u, skip = lid == 0u ? 2u : 0u;\n    uint32_t j = 0u, k = 0u, sfx = 0u, digits = 0u;\n    for (uint32_t p = 0u; p < n; p++) {\n        uint32_t c = lid == 0u ? (p < 18u ? 70u : 117u) : 49u;\n        if (c == 39u) continue;\n        if (j++ < skip) continue;\n        if (k == 0u) {\n            uint32_t d = c - 48u;\n            if (d < (lid == 0u ? 23u : 10u)) {\n                digits++;\n                continue;\n            }\n        }\n        if (k >= 3u) break;\n        sfx |= c << (8u * k);\n        k++;\n    }\n    out[lid] = sfx ^ digits << 24;");
    let want: Vec<u32> = (0..64u32).map(|l| if l == 0 { 0x757575 ^ 16 << 24 } else { 20 << 24 }).collect();
    assert_eq!(out, want);
}

/// A `switch` on a value that varies runs each lane's case, falling through, `break`, `continue` and `default` as C
/// does; one on a value the SIMD group shares goes straight to its case (decision 0567: a round for each value the
/// running lanes have, each through a jump to its case).
#[test]
fn a_switch_runs_each_lanes_case() {
    let out = ran(
        "    uint32_t x = 0u;\n    for (uint32_t k = 0u; k < 3u; k++) {\n        switch ((lid + k) % 7u) {\n        case 0u: x += 1u;\n        case 1u: x += 10u; break;\n        case 3u: x += 100u; continue;\n        case 5u: if ((lid & 1u) != 0u) break; x += 1000u;\n        default: x += 7u;\n        }\n        x += 3u;\n        switch (k) {\n        case 0u: x ^= 0x10u; break;\n        case 2u: x ^= 0x200u;\n        default: x += 1u;\n        }\n    }\n    out[lid] = x;",
    );
    let want: Vec<u32> = (0..64u32)
        .map(|lid| {
            let mut x = 0u32;
            for k in 0..3u32 {
                match (lid + k) % 7 {
                    0 => x += 11,
                    1 => x += 10,
                    3 => {
                        x += 100;
                        continue;
                    }
                    5 if lid & 1 != 0 => {}
                    5 => x += 1007,
                    _ => x += 7,
                }
                x += 3;
                match k {
                    0 => x ^= 0x10,
                    2 => x = (x ^ 0x200) + 1,
                    _ => x += 1,
                }
            }
            x
        })
        .collect();
    assert_eq!(out, want);
}

/// A kernel's compiled sections kept as bytes (a store's, `jit::compile`'s) load into code that runs as compiled: the
/// code needs no relocation, so the bytes are the code (decision 0567 step 3).
#[test]
fn compiled_code_kept_as_bytes_runs_the_same() {
    let text = "#include <ukl/memory.h>\n#include <ukl/group.h>\n\nstatic uint32_t twice(uint32_t x) { return x + x; }\n\nDEFINE_KERNEL2(rule, t_rule,\n               RWBUFFER, uint32_t, out, \"[64]\",\n               LID0,     uint32_t, lid, ,\n               LANE,     uint16_t, lane, )\n{\n    uint32_t x = twice(lid);\n    switch (lid % 3u) { case 0u: x += 5u; break; default: x ^= 1u; }\n    out[lid] = x + ukl_sg_sum(lid __UKL_ARGS);\n}\n";
    let src = super::source("/<kernels>/rule.uglykl", text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap();
    let k = super::vector::vectorize(&p, p.kernel("t_rule").unwrap()).unwrap();
    let mut kept = None;
    super::run::compiled_kept(&k, || None, |b| kept = Some(b.to_vec())).unwrap();
    let kept = kept.expect("the compiled code needs no relocation");
    let want = run_on(&p, &k, super::run::Exec::Interpret);
    assert_eq!(run_on(&p, &k, super::run::Exec::Jit), want);
    let again = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
    super::run::compiled_kept(&again, || Some(kept.clone()), |_| panic!("compiled again, not loaded")).unwrap();
    assert_eq!(run_on(&p, &again, super::run::Exec::Jit), want);
    let mut bad = kept.clone();
    bad.truncate(bad.len() - 1);
    let other = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
    let mut compiled = false;
    super::run::compiled_kept(&other, || Some(bad), |_| compiled = true).unwrap();
    assert!(compiled, "a kept form cut short is compiled again");
}

/// Kernel `k`'s SSA (`ssa`), built and checked, or the test fails with why; its listing to `RERAIL_UKL_VEC_SSA`'s
/// directory when set.
fn ssa_of(p: &super::lower::Program, k: &super::vector::Kernel) -> super::ssa::Ssa {
    let s = super::ssa::build(&k.tape, &|id| if id == u32::MAX { k.name.clone() } else { p.funcs[id as usize].name.clone() }, &|id| p.funcs[id as usize].called).unwrap_or_else(|e| panic!("{}: {e}", k.name));
    if let Ok(dir) = std::env::var("RERAIL_UKL_VEC_SSA") {
        std::fs::write(format!("{dir}/{}.ssa", k.name), s.listing()).unwrap();
    }
    s
}


/// The chunked runs `ssa_runs_equal` made (a cross-lane kernel below 32 lanes).
static CHUNKED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Kernel `k`'s SSA compiled (`cl`) at each width runs as the interpreter did (`want`).
fn ssa_runs_equal(p: &super::lower::Program, k: &super::vector::Kernel, want: &[u32]) {
    for w in [1u32, 2, 4, 8, 16, 32] {
        let ssa = ssa_of(p, k);
        let (c, bytes) = super::cl::compile(&ssa, &k.name, w, &super::exec::places()).unwrap_or_else(|e| panic!("{}: {e}", k.name));
        assert_eq!(c.chunked, ssa.funcs[0].cross && w < 32, "{} at width {w}: its SIMD groups as chunks", k.name);
        if c.chunked {
            CHUNKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        // Its kept form, loaded, runs the same.
        let loaded = super::cl::load(&bytes.expect("a kept form")).expect("the kept form loads");
        let back = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
        let _ = back.cl.set(Ok(loaded));
        assert_eq!(run_on(p, &back, super::run::Exec::Ssa), want, "{} at width {w}: the kept form against the interpreter", k.name);
        let again = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
        let _ = again.cl.set(Ok(c));
        assert_eq!(run_on(p, &again, super::run::Exec::Ssa), want, "{} at width {w}: the compiled SSA against the interpreter", k.name);
    }
}

/// Kernel `t_rule` of a source whose body is `body`, lowered and vectorized.
fn kernel_of(body: &str) -> (super::lower::Program, super::vector::Kernel) {
    let text = format!(
        "#include <ukl/memory.h>\n#include <ukl/group.h>\nDEFINE_KERNEL2(rule, t_rule,\n               RWBUFFER, uint32_t, out, \"[64]\",\n               LID0,     uint32_t, lid, ,\n               LANE,     uint16_t, lane, )\n{{\n{body}\n}}\n"
    );
    let src = super::source("/<kernels>/rule.uglykl", &text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("t_rule").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    (p, k)
}

/// The register pressure a width gives: a kernel of few values live runs wide, one of many (each live to the end)
/// narrower, where its values fit the registers; and the pressure model's counts are a lanes value's parts.
#[test]
fn the_width_is_the_one_whose_registers_hold_the_values_live() {
    let files = super::pressure::Files { vec: 28, int: 24, float: 0 };
    let (p, k) = kernel_of("    uint32_t a = out[(lid + 1u) % 64u] * 3u + lid;\n    out[lid] = a ^ (a >> 2);");
    let few = super::pressure::of(&ssa_of(&p, &k));
    let w_few = super::pressure::width(&few, files);
    let names: Vec<String> = (0..20).map(|j| format!("a{j}")).collect();
    let mut body = String::new();
    for (j, n) in names.iter().enumerate() {
        body += &format!("    uint32_t {n} = out[(lid + {j}u) % 64u] * {}u + lid;\n", 2 * j + 3);
    }
    body += &format!("    out[lid] = {};", names.join(" ^ "));
    let (p2, k2) = kernel_of(&body);
    let many = super::pressure::of(&ssa_of(&p2, &k2));
    let w_many = super::pressure::width(&many, files);
    // At width 8 each of the 20 lanes values is two vector registers: at least 40 live at the last.
    let at8 = many.iter().map(|f| f.at[3].vec).max().unwrap();
    assert!(at8 >= 40, "the 20 values at width 8: {at8} vector registers");
    let ats = |ps: &[super::pressure::FuncPressure]| ps.iter().map(|f| (f.lanes, f.scalar, f.other, f.at[0], f.at[2], f.at[5])).collect::<Vec<_>>();
    assert!(w_few >= 8, "a kernel of few values runs wide: {w_few} ({:?})", ats(&few));
    assert!(w_many < w_few, "one of many values runs narrower: {w_many} against {w_few} ({:?})", ats(&many));
    // Both run as the interpreter does at the width chosen.
    for (p, k) in [(&p, &k), (&p2, &k2)] {
        let want = run_on(p, k, super::run::Exec::Interpret);
        let ssa = ssa_of(p, k);
        let (c, _) = super::cl::compile(&ssa, &k.name, 0, &super::exec::places()).unwrap();
        let again = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
        let _ = again.cl.set(Ok(c));
        assert_eq!(run_on(p, &again, super::run::Exec::Ssa), want);
    }
}

/// A development measurement (`RERAIL_UKL_VEC_BENCH`): the 20-values kernel's launch time at each width.
#[test]
fn width_times() {
    if std::env::var("RERAIL_UKL_VEC_BENCH").is_err() {
        return;
    }
    let names: Vec<String> = (0..20).map(|j| format!("a{j}")).collect();
    let mut body = String::new();
    for (j, n) in names.iter().enumerate() {
        body += &format!("    uint32_t {n} = out[(lid + {j}u) % 64u] * {}u + lid;\n", 2 * j + 3);
    }
    body += &format!("    out[lid] = {};", names.join(" ^ "));
    for b in [&body, &"    uint32_t a = out[(lid + 1u) % 64u] * 3u + lid;\n    out[lid] = a ^ (a >> 2);".to_string()] {
        let (p, k) = kernel_of(b);
        let ssa = ssa_of(&p, &k);
        let mut out = vec![0u32; 64];
        let ptr = (out.as_mut_ptr() as u64).to_le_bytes().to_vec();
        let args = [super::run::Arg::Bytes(ptr), super::run::Arg::Bytes(vec![0; 8]), super::run::Arg::Bytes(vec![0; 4])];
        let ps = super::pressure::of(&ssa);
        let chosen = super::pressure::width(&ps, super::pressure::Files::host());
        for w in [1u32, 2, 4, 8, 16, 32] {
            let (c, _) = super::cl::compile(&ssa, &k.name, w, &super::exec::places()).unwrap();
            let kk = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
            let _ = kk.cl.set(Ok(c));
            let go = || super::run::run(&p, &kk, &args, super::run::Launch { grid: [256, 1, 1], block: [64, 1, 1], local: 0, resident: 1, exec: super::run::Exec::Ssa }).unwrap();
            go();
            let t = std::time::Instant::now();
            for _ in 0..20 {
                go();
            }
            eprintln!("BENCH w{w}: {:.1} us a launch (chosen w{chosen})", t.elapsed().as_secs_f64() * 1e6 / 20.0);
        }
    }
}

/// Division and remainder of 32-bit and narrower lanes, as vectors: unsigned and signed, by a constant (a power of
/// two among them), by a uniform divisor and by one that varies, negative operands and `i32::MIN / -1` (which
/// wraps) among them; each lane's as Rust's wrapping operations.
#[test]
fn thirty_two_bit_lanes_divide_as_each_lanes_integer() {
    let out = ran(concat!(
        "    uint32_t u = lid * 2654435761u;\n",
        "    int32_t s = (int32_t)u;\n",
        "    uint32_t d = (lid % 7u) + 1u;\n",
        "    int32_t sd = lid % 3u == 0u ? -(int32_t)d : (int32_t)d;\n",
        "    uint32_t k = out[63] + 10u;\n",
        "    int32_t mn = lid == 5u ? (int32_t)0x80000000u : s;\n",
        "    int32_t m1 = lid == 5u ? -1 : sd;\n",
        "    uint8_t b = (uint8_t)u;\n",
        "    uint32_t r = 0u;\n",
        "    switch (lid % 8u) {\n",
        "    case 0u: r = u / 10u + u % 64u; break;\n",
        "    case 1u: r = u % 10u ^ u / 64u; break;\n",
        "    case 2u: r = (uint32_t)(s / 7) ^ (uint32_t)(s % 7); break;\n",
        "    case 3u: r = (uint32_t)(s / sd) + (uint32_t)(s % sd); break;\n",
        "    case 4u: r = u / d ^ u % d; break;\n",
        "    case 5u: r = (uint32_t)(mn / m1) ^ (uint32_t)(s % 16); break;\n",
        "    case 6u: r = u / k + u % k; break;\n",
        "    default: r = (uint32_t)(b / (uint8_t)d) + (uint32_t)(b % (uint8_t)3u); break;\n",
        "    }\n",
        "    out[lid] = r;"
    ));
    let want: Vec<u32> = (0..64u32)
        .map(|l| {
            let u = l.wrapping_mul(2654435761);
            let s = u as i32;
            let d = l % 7 + 1;
            let sd = if l % 3 == 0 { -(d as i32) } else { d as i32 };
            let k = 10u32;
            let mn = if l == 5 { i32::MIN } else { s };
            let m1 = if l == 5 { -1 } else { sd };
            let b = u as u8;
            match l % 8 {
                0 => u / 10 + u % 64,
                1 => u % 10 ^ u / 64,
                2 => (s / 7) as u32 ^ (s % 7) as u32,
                3 => (s.wrapping_div(sd) as u32).wrapping_add(s.wrapping_rem(sd) as u32),
                4 => u / d ^ u % d,
                5 => mn.wrapping_div(m1) as u32 ^ (s % 16) as u32,
                6 => u / k + u % k,
                _ => u32::from(b / d as u8) + u32::from(b % 3),
            }
        })
        .collect();
    // Lane 6's `k` read `out[63]` before any lane wrote it.
    assert_eq!(out, want);
}

/// A cross-lane kernel run as chunks of its SIMD groups' lanes (every width below 32, `ran`'s), as the interpreter
/// runs it: lanes that end before the group's operations, a branch that varies between them (each chunk its own
/// way), a loop whose count is a group's maximum (its test the group's decision), shuffles across chunks in it, a
/// ballot and a sum of the lanes left.
#[test]
fn a_cross_lane_kernel_runs_as_chunks_of_its_lanes() {
    let before = CHUNKED.load(std::sync::atomic::Ordering::Relaxed);
    let out = ran(concat!(
        "    if (lid % 5u == 3u) { out[lid] = 1000u + lid; return; }\n",
        "    uint32_t r = 0u;\n",
        "    if ((lid & 1u) != 0u) { r = lid * lid; } else { r = r - 3u; }\n",
        "    uint32_t s = ukl_sg_sum(r __UKL_ARGS);\n",
        "    uint32_t n = ukl_sg_max(lid % 6u __UKL_ARGS);\n",
        "    uint32_t acc = 0u;\n",
        "    for (uint32_t k = 0u; k < n; k++) acc += ukl_sg_shuffle(lid + k, (lane + k * 9u) % 32u __UKL_ARGS);\n",
        "    uint32_t b = ukl_sg_ballot(lid % 3u == 0u __UKL_ARGS);\n",
        "    out[lid] = s ^ acc * 3u ^ b;"
    ));
    assert!(CHUNKED.load(std::sync::atomic::Ordering::Relaxed) > before, "the kernel ran as chunks");
    for l in 0..64u32 {
        if l % 5 == 3 {
            assert_eq!(out[l as usize], 1000 + l, "lane {l} ended first");
        }
    }
    // The SIMD group's sum of the lanes left: lane 0's is in its value (b and acc beside it).
    let left: Vec<u32> = (0..32u32).filter(|l| l % 5 != 3).collect();
    let s = left.iter().fold(0u32, |a, &l| a.wrapping_add(if l & 1 != 0 { l * l } else { 0u32.wrapping_sub(3) }));
    let b = left.iter().filter(|&&l| l % 3 == 0).fold(0u32, |a, &l| a | 1 << l);
    let n = left.iter().map(|l| l % 6).max().unwrap();
    let acc = (0..n).fold(0u32, |a, k| {
        let src = (k * 9) % 32;
        a.wrapping_add(if src % 5 != 3 { src + k } else { 0 })
    });
    assert_eq!(out[0], s ^ acc.wrapping_mul(3) ^ b, "lane 0");
}

/// A call keeps the caller's registers where the callee takes fewer than the caller keeps across it: a small function
/// called while twelve lanes values live in its caller keeps its callers' registers (`CallConv::PreserveAll`), and the kernel runs as the interpreter does at every width.
#[test]
fn a_small_callee_keeps_its_callers_registers() {
    let prelude = "static __attribute__((noinline)) uint32_t mix(uint32_t x) { return (x ^ (x >> 3)) * 2654435761u; }\n";
    let names: Vec<String> = (0..12).map(|j| format!("a{j}")).collect();
    let mut body = String::new();
    for (j, n) in names.iter().enumerate() {
        body += &format!("    uint32_t {n} = out[(lid + {j}u) % 64u] * {}u + lid;\n", 2 * j + 3);
    }
    body += "    uint32_t m = mix(lid);\n";
    body += &format!("    out[lid] = m ^ {};", names.join(" ^ "));
    let text = format!(
        "#include <ukl/memory.h>\n#include <ukl/group.h>\n{prelude}\nDEFINE_KERNEL2(rule, t_rule,\n               RWBUFFER, uint32_t, out, \"[64]\",\n               LID0,     uint32_t, lid, ,\n               LANE,     uint16_t, lane, )\n{{\n{body}\n}}\n"
    );
    let src = super::source("/<kernels>/rule.uglykl", &text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap();
    let k = super::vector::vectorize(&p, p.kernel("t_rule").unwrap()).unwrap();
    let ssa = ssa_of(&p, &k);
    let ps = super::pressure::of(&ssa);
    let mix = ssa.funcs.iter().position(|f| f.name.contains("mix")).expect("mix, a function of its own");
    let keeps = super::pressure::keeps_callers(&ps, 3);
    assert!(keeps[mix], "mix at width 8 keeps its callers' registers: {:?}", ps.iter().map(|f| (&f.across, f.at[3])).collect::<Vec<_>>());
    ran_with(prelude, &body);
}

/// The element sum: each lane loads its value, the SIMD group sums them (a horizontal sum), and the group's first lane
/// adds the sum to the result atomically. One SIMD group of 32 lanes, `out[i] = 3i + 1` to start: `out[0]` ends
/// `1 + (1 + 4 + ... + 94)`, on the interpreter and on the compiled SSA at every width.
#[test]
fn an_element_sum_is_a_horizontal_sum_and_one_atomic() {
    let (p, k) = kernel_of("    uint32_t v = out[lid];\n    uint32_t s = ukl_sg_sum(v __UKL_ARGS);\n    if (lane == 0u) ukl_atomic_add(&out[0], s);");
    let run = |k: &super::vector::Kernel, exec| {
        let mut out: Vec<u32> = (0..64u32).map(|i| 3 * i + 1).collect();
        let ptr = (out.as_mut_ptr() as u64).to_le_bytes().to_vec();
        let args = [super::run::Arg::Bytes(ptr), super::run::Arg::Bytes(vec![0; 8]), super::run::Arg::Bytes(vec![0; 4])];
        super::run::run(&p, k, &args, super::run::Launch { grid: [1, 1, 1], block: [32, 1, 1], local: 0, resident: 1, exec }).unwrap_or_else(|e| panic!("{exec:?}: {}", e.0));
        out
    };
    let mut want: Vec<u32> = (0..64u32).map(|i| 3 * i + 1).collect();
    want[0] += (0..32u32).map(|i| 3 * i + 1).sum::<u32>();
    assert_eq!(run(&k, super::run::Exec::Interpret), want, "the interpreter");
    for w in [1u32, 2, 4, 8, 16, 32] {
        let (c, _) = super::cl::compile(&ssa_of(&p, &k), &k.name, w, &super::exec::places()).unwrap();
        let at = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
        let _ = at.cl.set(Ok(c));
        assert_eq!(run(&at, super::run::Exec::Ssa), want, "the compiled SSA at width {w}");
    }
}

/// A per-lane atomic (each lane its own address, under a branch that varies) of a SIMD group's sum: every lane that
/// runs adds the group's sum to its own word, as the interpreter does (the sum computed once, not again for each
/// lane's atomic: Cranelift's elaboration places a pure value no deeper than its uses' common dominator).
#[test]
fn a_per_lane_atomic_of_a_sum_adds_it_in_each_lane() {
    let out = ran("    uint32_t s = ukl_sg_sum(lid * 3u + 1u __UKL_ARGS);\n    if (lid % 3u == 0u) ukl_atomic_add(&out[lid], s);");
    for l in 0..64u32 {
        let g = l / 32 * 32;
        let s: u32 = (g..g + 32).map(|i| i * 3 + 1).sum();
        assert_eq!(out[l as usize], if l % 3 == 0 { s } else { 0 }, "lane {l}");
    }
}

/// A matrix product: `c = a b` of 32 by 32 matrices of `float`, one group of 32 by 32 threads, each thread its row's
/// (`i`, its id in the second dimension) and column's (`j`, the first) element: a SIMD group a row, its lanes the
/// row's columns. The values are small integers, so each sum is exact: the same on the interpreter and on the compiled
/// SSA at every width.
#[test]
fn a_matrix_product_is_each_rows_columns_in_lanes() {
    const N: usize = 32;
    let text = "#include <ukl/memory.h>\n#include <ukl/group.h>\nDEFINE_KERNEL2(rule, t_mm,\n               ROBUFFER, float, a, \"[1024]\",\n               ROBUFFER, float, b, \"[1024]\",\n               RWBUFFER, float, c, \"[1024]\",\n               LID0,     uint32_t, j, ,\n               LID1,     uint32_t, i, )\n{\n    float s = 0.0f;\n    for (uint32_t k = 0; k < 32u; ++k)\n        s += a[i * 32u + k] * b[k * 32u + j];\n    c[i * 32u + j] = s;\n}\n";
    let src = super::source("/<kernels>/rule.uglykl", text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("t_mm").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    let a: Vec<f32> = (0..N * N).map(|x| (x % 7) as f32 - 3.0).collect();
    let b: Vec<f32> = (0..N * N).map(|x| (x % 5) as f32 + 1.0).collect();
    let mut want = vec![0f32; N * N];
    for i in 0..N {
        for j in 0..N {
            want[i * N + j] = (0..N).map(|k| a[i * N + k] * b[k * N + j]).sum();
        }
    }
    let run = |k: &super::vector::Kernel, exec| {
        let mut c = vec![0f32; N * N];
        let ptr = |p: *const f32| super::run::Arg::Bytes((p as u64).to_le_bytes().to_vec());
        let args = [ptr(a.as_ptr()), ptr(b.as_ptr()), ptr(c.as_mut_ptr()), super::run::Arg::Bytes(vec![0; 8]), super::run::Arg::Bytes(vec![0; 4])];
        super::run::run(&p, k, &args, super::run::Launch { grid: [1, 1, 1], block: [32, 32, 1], local: 0, resident: 1, exec }).unwrap_or_else(|e| panic!("{exec:?}: {}", e.0));
        c
    };
    assert_eq!(run(&k, super::run::Exec::Interpret), want, "the interpreter");
    for w in [1u32, 2, 4, 8, 16, 32] {
        let (c, _) = super::cl::compile(&ssa_of(&p, &k), &k.name, w, &super::exec::places()).unwrap();
        let at = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
        let _ = at.cl.set(Ok(c));
        assert_eq!(run(&at, super::run::Exec::Ssa), want, "the compiled SSA at width {w}");
    }
}

/// A floating `a * a - b` is contracted to one multiply-add, rounded once (OpenCL C's `FP_CONTRACT`), on the
/// interpreter and the compiled SSA alike: with `a = 1 + 2^-12` and `b = 1 + 2^-11` it is `2^-24`, where rounding the
/// product first (to `b`) would give 0. Also `+=` of a product, and `fma` called by name.
#[test]
fn a_floating_multiply_add_is_rounded_once() {
    let text = "#include <ukl/memory.h>\n#include <ukl/group.h>\nDEFINE_KERNEL2(rule, t_fma,\n               ROBUFFER, float, a, \"[32]\",\n               ROBUFFER, float, b, \"[32]\",\n               RWBUFFER, float, c, \"[96]\",\n               LID0,     uint32_t, j, )\n{\n    c[j] = a[j] * a[j] - b[j];\n    float s = -b[j];\n    s += a[j] * a[j];\n    c[32u + j] = s;\n    c[64u + j] = fma(a[j], a[j], -b[j]);\n}\n";
    let src = super::source("/<kernels>/rule.uglykl", text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("t_fma").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    let a = vec![1.0f32 + 2f32.powi(-12); 32];
    let b = vec![1.0f32 + 2f32.powi(-11); 32];
    assert_eq!(a[0] * a[0] - b[0], 0.0, "rounded twice");
    let want = vec![2f32.powi(-24); 96];
    let run = |k: &super::vector::Kernel, exec| {
        let mut c = vec![0f32; 96];
        let ptr = |p: *const f32| super::run::Arg::Bytes((p as u64).to_le_bytes().to_vec());
        let args = [ptr(a.as_ptr()), ptr(b.as_ptr()), ptr(c.as_mut_ptr()), super::run::Arg::Bytes(vec![0; 8]), super::run::Arg::Bytes(vec![0; 4])];
        super::run::run(&p, k, &args, super::run::Launch { grid: [1, 1, 1], block: [32, 1, 1], local: 0, resident: 1, exec }).unwrap_or_else(|e| panic!("{exec:?}: {}", e.0));
        c
    };
    assert_eq!(run(&k, super::run::Exec::Interpret), want, "the interpreter");
    assert_eq!(run(&k, super::run::Exec::Jit), want, "the JIT");
    for w in [1u32, 2, 4, 8, 16, 32] {
        let (c, _) = super::cl::compile(&ssa_of(&p, &k), &k.name, w, &super::exec::places()).unwrap();
        let at = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
        let _ = at.cl.set(Ok(c));
        assert_eq!(run(&at, super::run::Exec::Ssa), want, "the compiled SSA at width {w}");
    }
}
