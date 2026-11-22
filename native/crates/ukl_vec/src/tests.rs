/// The UKL library's test kernels (`gpu/src/library_tests.uglykl`), the first gate (decision 0567, step 1).
const LIBRARY_TESTS: &str = include_str!("../../../../gpu/src/library_tests.uglykl");

/// The library's test kernels preprocess for the `Vec` backend, its intrinsics `__ukl_*`, and C's parser reads them whole.
#[test]
fn the_library_tests_are_one_c_tape() {
    let src = super::source("/<kernels>/library_tests.uglykl", LIBRARY_TESTS, &[]).unwrap();
    assert!(src.text().contains("__ukl_sg_ballot") && !src.text().contains("sub_group_"), "the SIMD group's intrinsics, not OpenCL's");
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

/// Uniform 32-bit values whose readers read only their low 32 bits are held with the bits past 32 not made
/// (`Cg::low_only`): products and sums of them, a left shift of one, a store of one, its narrowings, its bits as a
/// float; and the values made of them that other readers read (a right shift, a compare, a division, a widening to 64
/// bits, signed and unsigned) are each the 32-bit value.
#[test]
fn uniform_values_read_for_their_low_bits_are_each_32_bit_value() {
    let out = ran(concat!(
        "    uint32_t a = (out[63] + 0x9e3779b9u) * 0x85ebca6bu;\n",
        "    uint32_t b = a * 7u + a;\n",
        "    uint32_t c = (b << 5) ^ a;\n",
        "    uint32_t g = c * 3u;\n",
        "    out[40] = g;\n",
        "    uint8_t d = (uint8_t)g;\n",
        "    uint16_t h = (uint16_t)(g + 9u);\n",
        "    float f = __ukl_as_float(g | 1u);\n",
        "    int32_t s = (int32_t)(out[62] + 3u) * -0x7654321;\n",
        "    int32_t t = s - 1;\n",
        "    uint32_t r = lid == 0u ? b >> 3 : lid == 1u ? (uint32_t)d : lid == 2u ? __ukl_as_uint32_t(f) : lid == 3u ? (uint32_t)(t >> 2)\n",
        "        : lid == 4u ? (uint32_t)(t / 3) : lid == 5u ? (c < b ? 1u : 2u) : lid == 6u ? (uint32_t)((int64_t)t >> 40)\n",
        "        : lid == 7u ? (uint32_t)h : lid == 8u ? (uint32_t)((uint64_t)c >> 31) : lid;\n",
        "    if (lid < 40u)\n",
        "        out[lid] = r;\n",
    ));
    let a = 0x9e37_79b9u32.wrapping_mul(0x85eb_ca6b);
    let b = a.wrapping_mul(7).wrapping_add(a);
    let c = (b << 5) ^ a;
    let g = c.wrapping_mul(3);
    let t = 3i32.wrapping_mul(-0x765_4321).wrapping_sub(1);
    let mut want: Vec<u32> = (0..64).collect();
    want[..9].copy_from_slice(&[
        b >> 3,
        u32::from(g as u8),
        g | 1,
        (t >> 2) as u32,
        (t / 3) as u32,
        if c < b { 1 } else { 2 },
        (i64::from(t) >> 40) as u32,
        u32::from(g.wrapping_add(9) as u16),
        (u64::from(c) >> 31) as u32,
    ]);
    want[40] = g;
    want[62] = 0;
    want[63] = 0;
    for l in 41..62 {
        want[l] = 0;
    }
    assert_eq!(out, want);
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

/// A `&&` and a `||` whose second side has no effect, made with no branch: its load is only the lanes' that read
/// it, so the lanes where the pointer is null (the first side false, or true for `||`) never read through it.
#[test]
fn a_short_circuit_with_no_effect_reads_only_in_the_lanes_it_runs() {
    let out = ran("    UKL_GLOBAL uint32_t* p = lid < 8u ? out : (UKL_GLOBAL uint32_t*)0;\n    uint32_t r = (lid < 8u && p[lid] == 0u) ? 1u : 2u;\n    uint32_t q = (lid >= 8u || p[lid] == 0u) ? 4u : 8u;\n    out[lid] = r | q;");
    let want: Vec<u32> = (0..64u32).map(|l| if l < 8 { 1 | 4 } else { 2 | 4 }).collect();
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

/// A kernel of many values live in a loop: 20 accumulators, each updated each time round and read at the end (at 32
/// lanes 160 vector registers, at 4 lanes 20).
fn many_values_body() -> String {
    let names: Vec<String> = (0..20).map(|j| format!("a{j}")).collect();
    let mut body = String::new();
    for (j, n) in names.iter().enumerate() {
        body += &format!("    uint32_t {n} = out[lid] + {j}u;\n");
    }
    body += "    for (uint32_t k = 0u; k < 16u; k++) {\n";
    for (j, n) in names.iter().enumerate() {
        body += &format!("        {n} = {n} * {}u + (k ^ lid);\n", 2 * j + 3);
    }
    body += "    }\n";
    body += &format!("    out[lid] = {};", names.join(" ^ "));
    body
}

/// The register pressure a width gives: a kernel of few values live runs wide, one of many live in a loop narrower,
/// where its values fit the registers; and the pressure model's counts are a lanes value's parts.
#[test]
fn the_width_is_the_one_whose_registers_hold_the_values_live() {
    let files = super::pressure::Files { vec: 28, int: 24, float: 0 };
    let (p, k) = kernel_of("    uint32_t a = out[(lid + 1u) % 64u] * 3u + lid;\n    out[lid] = a ^ (a >> 2);");
    let few = super::pressure::of(&ssa_of(&p, &k));
    let w_few = super::pressure::width(&few, files);
    let (p2, k2) = kernel_of(&many_values_body());
    let many = super::pressure::of(&ssa_of(&p2, &k2));
    let w_many = super::pressure::width(&many, files);
    // At width 8 each of the 20 lanes values is two vector registers: at least 40 live at the last.
    let at8 = many.iter().map(|f| f.at[3].vec).max().unwrap();
    assert!(at8 >= 40, "the 20 values at width 8: {at8} vector registers");
    let ats = |ps: &[super::pressure::FuncPressure]| ps.iter().map(|f| (f.lanes, f.scalar, f.other, f.at[0], f.at[2], f.at[5])).collect::<Vec<_>>();
    assert!(w_few >= 8, "a kernel of few values runs wide: {w_few} ({:?})", ats(&few));
    // The widths their launches are fastest at (`width_times`, and the split-float product's measurement): the 20
    // accumulators at 4 lanes, where they fit the registers; the split-float product at 32.
    assert_eq!(w_many, 4, "one of many values runs at 4 lanes ({:?})", ats(&many));
    let (p3, k3) = split_floats();
    let w_split = super::pressure::width(&super::pressure::of(&ssa_of(&p3, &k3)), files);
    assert_eq!(w_split, 32, "the split-float product runs at 32 lanes");
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

/// A development measurement (`RERAIL_UKL_VEC_BENCH`): the many-values kernel's launch time at each width.
#[test]
fn width_times() {
    if std::env::var("RERAIL_UKL_VEC_BENCH").is_err() {
        return;
    }
    let body = many_values_body();
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
            // The least of 50: a loaded machine's other work only adds.
            let best = (0..50)
                .map(|_| {
                    let t = std::time::Instant::now();
                    go();
                    t.elapsed().as_secs_f64() * 1e6
                })
                .fold(f64::MAX, f64::min);
            eprintln!("BENCH w{w}: {best:.1} us a launch (chosen w{chosen})");
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
    let fibers = super::fiber::prepared();
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
    // A SIMD group's chunks are a loop of its code, between its exchanges: no fiber runs them, nor the SIMD groups of
    // a group with no barrier.
    assert_eq!(super::fiber::prepared(), fibers, "fibers prepared for a group with no barrier");
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

/// A threadgroup barrier in a cross-lane kernel run as chunks: its SIMD group's chunks all reach it, the group then
/// waits for the other SIMD group (a fiber each: the only waits that need one), and each lane reads what the other
/// SIMD group wrote before it; the ballot before and the sum after are their SIMD group's.
#[test]
fn a_threadgroup_barrier_in_a_chunked_kernel_waits_for_the_other_simd_group() {
    let fibers = super::fiber::prepared();
    let out = ran(concat!(
        "    uint32_t b = ukl_sg_ballot(lid % 3u == 0u __UKL_ARGS);\n",
        "    out[lid] = lid * 5u + b;\n",
        "    ukl_tg_barrier();\n",
        "    uint32_t v = out[lid ^ 32u];\n",
        "    ukl_tg_barrier();\n",
        "    out[lid] = v ^ ukl_sg_sum(lid __UKL_ARGS);"
    ));
    let ballot = |g: u32| (0..32u32).filter(|l| (g * 32 + l) % 3 == 0).fold(0u32, |a, l| a | 1 << l);
    for l in 0..64u32 {
        let (g, o) = (l / 32, (l ^ 32) / 32);
        let sum: u32 = (g * 32..g * 32 + 32).sum();
        assert_eq!(out[l as usize], ((l ^ 32) * 5 + ballot(o)) ^ sum, "lane {l}");
    }
    assert!(super::fiber::prepared() > fibers, "the SIMD groups wait for each other as fibers");
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

/// A matrix product of 32 by 32 floats stored apart: each entry's sign a bit of a word, its exponent a byte, its
/// fraction 3 bytes; its result stored the same way, each row's signs one SIMD group's ballot.
fn split_floats() -> (super::lower::Program, super::vector::Kernel) {
    let text = "#include <ukl/memory.h>\n#include <ukl/group.h>\n\
#define UNPACK(sg, ex, mt, x) __ukl_as_float(((sg[(x) >> 5u] >> ((x) & 31u)) << 31) | ((uint32_t)ex[x] << 23) | (uint32_t)mt[3u * (x)] | ((uint32_t)mt[3u * (x) + 1u] << 8) | ((uint32_t)mt[3u * (x) + 2u] << 16))\n\
DEFINE_KERNEL2(rule, t_mms,\n\
               ROBUFFER, uint32_t, asg, \"[32]\",\n\
               ROBUFFER, uint8_t, aex, \"[1024]\",\n\
               ROBUFFER, uint8_t, amt, \"[3072]\",\n\
               ROBUFFER, uint32_t, bsg, \"[32]\",\n\
               ROBUFFER, uint8_t, bex, \"[1024]\",\n\
               ROBUFFER, uint8_t, bmt, \"[3072]\",\n\
               RWBUFFER, uint32_t, csg, \"[32]\",\n\
               RWBUFFER, uint8_t, cex, \"[1024]\",\n\
               RWBUFFER, uint8_t, cmt, \"[3072]\",\n\
               LID0,     uint32_t, j, ,\n\
               LID1,     uint32_t, i, )\n\
{\n\
    float s = 0.0f;\n\
    for (uint32_t k = 0; k < 32u; ++k) {\n\
        uint32_t x = i * 32u + k;\n\
        uint32_t y = k * 32u + j;\n\
        s += UNPACK(asg, aex, amt, x) * UNPACK(bsg, bex, bmt, y);\n\
    }\n\
    uint32_t r = __ukl_as_uint32_t(s);\n\
    uint32_t z = i * 32u + j;\n\
    cex[z] = (uint8_t)(r >> 23);\n\
    cmt[3u * z] = (uint8_t)r;\n\
    cmt[3u * z + 1u] = (uint8_t)(r >> 8);\n\
    cmt[3u * z + 2u] = (uint8_t)((r >> 16) & 0x7fu);\n\
    uint32_t sign = ukl_sg_ballot((r >> 31) != 0u __UKL_ARGS);\n\
    if (j == 0u)\n\
        csg[i] = sign;\n\
}\n";
    let src = super::source("/<kernels>/rule.uglykl", text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("t_mms").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    (p, k)
}

/// A matrix product of floats stored apart: each entry's sign a bit (32 to a word, entry `e` bit `e % 32` of word
/// `e / 32`), its exponent a byte, its fraction 3 bytes (low byte first, the top bit 0). Each lane unpacks its
/// operands to floats, sums their products as `fma` (each rounded once), and packs its result the same way, the
/// sign bits of a row a ballot of its 32 lanes. The result's bits are a 32-bit `fma` product's.
#[test]
fn a_matrix_product_of_split_floats_is_a_floating_one() {
    const N: usize = 32;
    let (p, k) = split_floats();
    // Fractions that use all 23 bits, both signs, exponents around 1.
    let float = |x: usize, salt: u32| {
        let h = (x as u32).wrapping_mul(0x9e37_79b9).wrapping_add(salt).rotate_left(13).wrapping_mul(0x85eb_ca6b);
        f32::from_bits((h & 0x807f_ffff) | ((124 + (h >> 23) % 6) << 23))
    };
    let a: Vec<f32> = (0..N * N).map(|x| float(x, 1)).collect();
    let b: Vec<f32> = (0..N * N).map(|x| float(x, 2)).collect();
    let mut want = vec![0u32; N * N];
    for i in 0..N {
        for j in 0..N {
            want[i * N + j] = (0..N).fold(0f32, |s, k| a[i * N + k].mul_add(b[k * N + j], s)).to_bits();
        }
    }
    let split = |v: &[f32]| {
        let mut sg = vec![0u32; v.len() / 32];
        let (mut ex, mut mt) = (Vec::new(), Vec::new());
        for (e, f) in v.iter().enumerate() {
            let r = f.to_bits();
            sg[e / 32] |= (r >> 31) << (e % 32);
            ex.push((r >> 23) as u8);
            mt.extend_from_slice(&(r & 0x7f_ffff).to_le_bytes()[..3]);
        }
        (sg, ex, mt)
    };
    let ((asg, aex, amt), (bsg, bex, bmt)) = (split(&a), split(&b));
    let run = |k: &super::vector::Kernel, exec| {
        let (mut csg, mut cex, mut cmt) = (vec![0u32; N], vec![0u8; N * N], vec![0u8; 3 * N * N]);
        let ptr = |p: *const u8| super::run::Arg::Bytes((p as u64).to_le_bytes().to_vec());
        let args = [
            ptr(asg.as_ptr().cast()), ptr(aex.as_ptr()), ptr(amt.as_ptr()),
            ptr(bsg.as_ptr().cast()), ptr(bex.as_ptr()), ptr(bmt.as_ptr()),
            ptr(csg.as_mut_ptr().cast()), ptr(cex.as_mut_ptr()), ptr(cmt.as_mut_ptr()),
            super::run::Arg::Bytes(vec![0; 8]), super::run::Arg::Bytes(vec![0; 4]),
        ];
        super::run::run(&p, k, &args, super::run::Launch { grid: [1, 1, 1], block: [32, 32, 1], local: 0, resident: 1, exec }).unwrap_or_else(|e| panic!("{exec:?}: {}", e.0));
        (0..N * N)
            .map(|e| ((csg[e / 32] >> (e % 32)) & 1) << 31 | u32::from(cex[e]) << 23 | u32::from_le_bytes([cmt[3 * e], cmt[3 * e + 1], cmt[3 * e + 2], 0]))
            .collect::<Vec<u32>>()
    };
    assert_eq!(run(&k, super::run::Exec::Interpret), want, "the interpreter");
    let fibers = super::fiber::prepared();
    // 0: the width its registers' pressure chooses.
    for w in [0u32, 1, 2, 4, 8, 16, 32] {
        let (c, _) = super::cl::compile(&ssa_of(&p, &k), &k.name, w, &super::exec::places()).unwrap();
        let at = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
        let _ = at.cl.set(Ok(c));
        assert_eq!(run(&at, super::run::Exec::Ssa), want, "the compiled SSA at width {w}");
        // A development measurement (`RERAIL_UKL_VEC_BENCH`): the least of 50 launches.
        if std::env::var("RERAIL_UKL_VEC_BENCH").is_ok() {
            let best = (0..50)
                .map(|_| {
                    let t = std::time::Instant::now();
                    run(&at, super::run::Exec::Ssa);
                    t.elapsed().as_secs_f64() * 1e6
                })
                .fold(f64::MAX, f64::min);
            eprintln!("BENCH t_mms w{w}: {best:.1} us a launch");
        }
    }
    // One SIMD group's code, its chunks a loop of it: no fiber.
    assert_eq!(super::fiber::prepared(), fibers, "fibers prepared for a group with no barrier");
}

/// A compiled function's counted loop (machine code as text, `compile_listed`): from the block of its first fused
/// multiply-add to its back edge, the jumps between blocks left out.
#[cfg(target_arch = "aarch64")]
fn fma_loop(code: &str) -> Vec<&str> {
    let lines: Vec<&str> = code.lines().collect();
    let fm = lines.iter().position(|l| l.trim_start().starts_with("fmadd ") || l.trim_start().starts_with("fmla ")).expect("a multiply-add");
    let start = (0..fm).rev().find(|&i| lines[i].starts_with("block")).expect("its block");
    let end = (fm..lines.len()).find(|&i| lines[i].trim_start().starts_with("cbnz ")).expect("its back edge");
    lines[start + 1..=end].iter().copied().filter(|l| l.starts_with("  ") && !l.trim_start().starts_with("b label")).collect()
}

/// The split-float matrix product's loop at each width is vector code as written by hand:
/// - nothing in the stack: its constants and accumulators are in registers (one sign multiplier for every 4 lanes,
///   one swizzle pattern for each window offset, no pattern for exponent bytes);
/// - B's sign word (one for the SIMD group: its lanes' index over 32) loaded once and shifted as a scalar;
/// - each 4 lanes' fraction one swizzle (`tbl`) of a window of B's bytes, its exponents a widening (`uxtl`) of a
///   window of 16 (under 16 lanes, where registers are left, a swizzle): B's bytes are never loaded one at a time;
/// - A's fraction bytes loads at offsets 1 apart from one register;
/// - at 1 lane, each sign bit's shift a shift of the count the source masks (`x & 31`), with no mask of its own;
/// - no zero-extension of a uniform value already zero-extended (`mov w, w`): A's bytes, as loaded, widened and
///   shifted to their places;
/// - the counter up to 0, tested at the loop's end with no compare;
/// - fewer instructions a lane at each wider width.
/// AArch64's code.
#[test]
#[cfg(target_arch = "aarch64")]
fn the_split_float_products_loop_is_vector_code() {
    let (p, k) = split_floats();
    let ssa = ssa_of(&p, &k);
    let mut last = f64::MAX;
    for w in [1u32, 2, 4, 8, 16, 32] {
        let (c, _) = super::cl::compile_listed(&ssa, &k.name, w, &super::exec::places()).unwrap();
        let code = c.listing.last().expect("the kernel's code for a chunk whose lanes all run");
        let body = fma_loop(code);
        let count = |op: &str| body.iter().filter(|l| l.trim_start().starts_with(op)).count();
        let text = body.join("\n");
        assert!(!text.contains("[sp"), "w{w}: the loop reads or writes the stack:\n{text}");
        let movs: Vec<&&str> = body.iter().filter(|l| l.trim_start().starts_with("mov w")).collect();
        assert!(movs.is_empty(), "w{w}: zero-extensions {movs:?}:\n{text}");
        let per = body.len() as f64 / f64::from(w);
        assert!(per < last, "w{w}: {per:.2} instructions a lane, against {last:.2} at half the width:\n{text}");
        last = per;
        // A's bytes: its exponent and its 3 fraction bytes, the fraction's from one register at offsets k, k+1, k+2.
        let at: Vec<(String, i64)> = body
            .iter()
            .filter_map(|l| {
                let r = l.trim_start().strip_prefix("ldrb ")?;
                let (base, off) = r.split_once("[")?.1.trim_end_matches(']').split_once(", #")?;
                Some((base.to_string(), off.parse().ok()?))
            })
            .collect();
        assert!(
            at.iter().any(|(b, o)| at.contains(&(b.clone(), o + 1)) && at.contains(&(b.clone(), o + 2))),
            "w{w}: A's fraction bytes are not loaded at offsets from one register:\n{text}"
        );
        if w == 1 {
            let masks = body.iter().filter(|l| l.trim_start().starts_with("and ") && l.ends_with(", #31")).count();
            assert_eq!(masks, 2, "w1: the shifts' counts masked again:\n{text}");
        }
        if w >= 2 {
            assert_eq!(count("ldrb "), 4, "w{w}: B's bytes loaded one at a time:\n{text}");
        }
        if w >= 4 {
            let tbl = if w >= 16 { w / 4 } else { 2 * w / 4 };
            assert_eq!(count("tbl "), tbl as usize, "w{w}: a swizzle for each 4 lanes' fraction (and exponents, under 16 lanes):\n{text}");
            assert_eq!(count("uxtl") > 0, w >= 16, "w{w}: the exponent bytes a widening at 16 lanes and more:\n{text}");
            assert_eq!(count("mul "), w as usize / 4, "w{w}: the sign bits' products:\n{text}");
            assert_eq!(count("ushl "), 0, "w{w}: the sign word (one for the SIMD group) shifted as a vector:\n{text}");
        }
        // The back edge: the counter's increment, then a test of it against 0.
        let (inc, br) = (body[body.len() - 2].trim_start(), body[body.len() - 1].trim_start());
        let reg = br.strip_prefix("cbnz ").and_then(|r| r.split(',').next()).unwrap();
        assert!(inc.starts_with(&format!("add {reg}, {reg}, #1")), "w{w}: the loop's end:\n{inc}\n{br}");
    }
}

/// Bytes and 16-bit lanes next to each other (each lane's the next), unsigned and signed, and `or`s of a record's
/// bytes each shifted to its place by a multiple of 8 (a 32-bit word of 4 bytes, 24 bits of 3 in another order, a
/// word of two 16-bit halves), and an `or` whose bytes overlap (one byte twice, at 0 and 8; two at 0; one shifted
/// by 4): at every width, as the interpreter's.
#[test]
fn next_bytes_and_records_bytes_ored_are_each_lanes() {
    let text = "#include <ukl/memory.h>\n#include <ukl/group.h>\n\
DEFINE_KERNEL2(rule, t_rec,\n\
               ROBUFFER, uint8_t, ub, \"[256]\",\n\
               ROBUFFER, int8_t, sb, \"[256]\",\n\
               ROBUFFER, uint16_t, uh, \"[128]\",\n\
               ROBUFFER, int16_t, sh, \"[128]\",\n\
               ROBUFFER, uint8_t, rec, \"[256]\",\n\
               RWBUFFER, uint32_t, out, \"[512]\",\n\
               LID0,     uint32_t, lid, )\n\
{\n\
    out[lid] = (uint32_t)ub[lid] * 3u;\n\
    out[64u + lid] = (uint32_t)((int32_t)sb[lid] * 5);\n\
    out[128u + lid] = (uint32_t)uh[lid] + 1u;\n\
    out[192u + lid] = (uint32_t)((int32_t)sh[lid] - 7);\n\
    out[256u + lid] = (uint32_t)rec[4u * lid] | ((uint32_t)rec[4u * lid + 1u] << 8) | ((uint32_t)rec[4u * lid + 2u] << 16) | ((uint32_t)rec[4u * lid + 3u] << 24);\n\
    out[320u + lid] = ((uint32_t)rec[3u * lid + 2u] << 16) | (uint32_t)rec[3u * lid] | ((uint32_t)rec[3u * lid + 1u] << 8);\n\
    out[384u + lid] = (uint32_t)rec[2u * lid] | ((uint32_t)rec[2u * lid] << 8) | ((uint32_t)rec[2u * lid + 1u] << 4) | (uint32_t)rec[2u * lid + 1u];\n\
    out[448u + lid] = (uint32_t)uh[2u * lid] | ((uint32_t)uh[2u * lid + 1u] << 16);\n\
}\n";
    let src = super::source("/<kernels>/rule.uglykl", text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("t_rec").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    let h = |x: u32, salt: u32| x.wrapping_mul(0x9e37_79b9).wrapping_add(salt).rotate_left(11).wrapping_mul(0x85eb_ca6b);
    let ub: Vec<u8> = (0..256).map(|x| (h(x, 1) >> 24) as u8).collect();
    let sb: Vec<i8> = (0..256).map(|x| (h(x, 2) >> 24) as i8).collect();
    let uh: Vec<u16> = (0..128).map(|x| (h(x, 3) >> 16) as u16).collect();
    let sh: Vec<i16> = (0..128).map(|x| (h(x, 4) >> 16) as i16).collect();
    let rec: Vec<u8> = (0..256).map(|x| (h(x, 5) >> 24) as u8).collect();
    let mut want = vec![0u32; 512];
    for l in 0..64usize {
        let r = |i: usize| u32::from(rec[i]);
        want[l] = u32::from(ub[l]) * 3;
        want[64 + l] = (i32::from(sb[l]) * 5) as u32;
        want[128 + l] = u32::from(uh[l]) + 1;
        want[192 + l] = (i32::from(sh[l]) - 7) as u32;
        want[256 + l] = u32::from_le_bytes([rec[4 * l], rec[4 * l + 1], rec[4 * l + 2], rec[4 * l + 3]]);
        want[320 + l] = r(3 * l + 2) << 16 | r(3 * l) | r(3 * l + 1) << 8;
        want[384 + l] = r(2 * l) | r(2 * l) << 8 | r(2 * l + 1) << 4 | r(2 * l + 1);
        want[448 + l] = u32::from(uh[2 * l]) | u32::from(uh[2 * l + 1]) << 16;
    }
    let run = |k: &super::vector::Kernel, exec| {
        let mut out = vec![0u32; 512];
        let ptr = |p: *const u8| super::run::Arg::Bytes((p as u64).to_le_bytes().to_vec());
        let args = [
            ptr(ub.as_ptr()), ptr(sb.as_ptr().cast()), ptr(uh.as_ptr().cast()), ptr(sh.as_ptr().cast()), ptr(rec.as_ptr()),
            ptr(out.as_mut_ptr().cast()), super::run::Arg::Bytes(vec![0; 8]), super::run::Arg::Bytes(vec![0; 4]),
        ];
        super::run::run(&p, k, &args, super::run::Launch { grid: [1, 1, 1], block: [64, 1, 1], local: 0, resident: 1, exec }).unwrap_or_else(|e| panic!("{exec:?}: {}", e.0));
        out
    };
    assert_eq!(run(&k, super::run::Exec::Interpret), want, "the interpreter");
    let ssa = ssa_of(&p, &k);
    for w in [1u32, 2, 4, 8, 16, 32] {
        let (c, _) = super::cl::compile_listed(&ssa, &k.name, w, &super::exec::places()).unwrap();
        // At 16 lanes and more, each 4 lanes' swizzles: one for each record's word (the 4 bytes', the 3 bytes', the
        // two halves'), and the overlapping bytes' three (the byte at 0 and 8 one; the other byte's two loads, the one
        // shifted by 4 and the one at 0, each its own). The lanes next to each other are widenings, no swizzle.
        if w >= 16 && cfg!(target_arch = "aarch64") {
            let code = c.listing.last().unwrap();
            let tbl = code.lines().filter(|l| l.trim_start().starts_with("tbl ")).count();
            assert_eq!(tbl, 6 * w as usize / 4, "w{w}: the swizzles:\n{code}");
        }
        let at = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
        let _ = at.cl.set(Ok(c));
        assert_eq!(run(&at, super::run::Exec::Ssa), want, "the compiled SSA at width {w}");
    }
}

/// Lanes of 1 and 2 bytes some bytes apart, as windows of their bytes and shuffles: signed bytes 3 apart, 2-byte
/// lanes 4 apart; and stores of two of each 3 bytes, which are never whole elements, so each lane's go as they are
/// (the third byte of each kept). At every width, as a lane at a time would.
#[test]
fn byte_lanes_some_bytes_apart_are_read_and_written_as_each_lanes() {
    let text = "#include <ukl/memory.h>\n#include <ukl/group.h>\n\
DEFINE_KERNEL2(rule, t_bytes,\n\
               ROBUFFER, int8_t, src, \"[256]\",\n\
               ROBUFFER, int16_t, hs, \"[128]\",\n\
               RWBUFFER, uint8_t, dst, \"[256]\",\n\
               RWBUFFER, uint32_t, out, \"[64]\",\n\
               LID0,     uint32_t, lid, )\n\
{\n\
    int32_t a = src[3u * lid + 1u];\n\
    int32_t h = hs[2u * lid];\n\
    dst[3u * lid] = (uint8_t)(a + h);\n\
    dst[3u * lid + 2u] = (uint8_t)(h >> 8);\n\
    out[lid] = (uint32_t)(a * 65536 + h);\n\
}\n";
    let src = super::source("/<kernels>/rule.uglykl", text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("t_bytes").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    let bytes: Vec<i8> = (0..256u32).map(|x| (x.wrapping_mul(0x9e37_79b9) >> 24) as i8).collect();
    let halves: Vec<i16> = (0..128u32).map(|x| (x.wrapping_mul(0x85eb_ca6b) >> 16) as i16).collect();
    let mut want_dst = vec![0xa5u8; 256];
    let mut want_out = vec![0u32; 64];
    for l in 0..64usize {
        let (a, h) = (i32::from(bytes[3 * l + 1]), i32::from(halves[2 * l]));
        want_dst[3 * l] = (a + h) as u8;
        want_dst[3 * l + 2] = (h >> 8) as u8;
        want_out[l] = (a * 65536 + h) as u32;
    }
    let run = |k: &super::vector::Kernel, exec| {
        let (mut dst, mut out) = (vec![0xa5u8; 256], vec![0u32; 64]);
        let ptr = |p: *const u8| super::run::Arg::Bytes((p as u64).to_le_bytes().to_vec());
        let args = [ptr(bytes.as_ptr().cast()), ptr(halves.as_ptr().cast()), ptr(dst.as_mut_ptr()), ptr(out.as_mut_ptr().cast()), super::run::Arg::Bytes(vec![0; 8]), super::run::Arg::Bytes(vec![0; 4])];
        super::run::run(&p, k, &args, super::run::Launch { grid: [1, 1, 1], block: [64, 1, 1], local: 0, resident: 1, exec }).unwrap_or_else(|e| panic!("{exec:?}: {}", e.0));
        (dst, out)
    };
    assert_eq!(run(&k, super::run::Exec::Interpret), (want_dst.clone(), want_out.clone()), "the interpreter");
    for w in [1u32, 2, 4, 8, 16, 32] {
        let (c, _) = super::cl::compile(&ssa_of(&p, &k), &k.name, w, &super::exec::places()).unwrap();
        let at = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
        let _ = at.cl.set(Ok(c));
        assert_eq!(run(&at, super::run::Exec::Ssa), (want_dst.clone(), want_out.clone()), "the compiled SSA at width {w}");
    }
}

/// A floating `a * a - b` is contracted to one multiply-add, rounded once (OpenCL C's `FP_CONTRACT`), on the
/// interpreter and the compiled SSA alike: with `a = 1 + 2^-12` and `b = 1 + 2^-11` it is `2^-24`, where rounding the
/// product first (to `b`) would give 0. Also `+=` of a product, and `fma` called by name.
#[test]
fn a_floating_multiply_add_is_rounded_once() {
    let text = "#include <ukl/memory.h>\n#include <ukl/group.h>\nDEFINE_KERNEL2(rule, t_fma,\n               ROBUFFER, float, a, \"[32]\",\n               ROBUFFER, float, b, \"[32]\",\n               RWBUFFER, float, c, \"[96]\",\n               LID0,     uint32_t, j, )\n{\n    c[j] = a[j] * a[j] - b[j];\n    float s = -b[j];\n    s += a[j] * a[j];\n    c[32u + j] = s;\n    c[64u + j] = __ukl_fma(a[j], a[j], -b[j]);\n}\n";
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

/// A compiled function's reads and writes back of one address (machine code as text): in a block, an `ldr` from a
/// register's address and a `str` to it with no write of the register between.
#[cfg(target_arch = "aarch64")]
fn read_modify_writes(code: &str) -> usize {
    let reg = |r: &str| r.trim().trim_start_matches(['x', 'w']).to_string();
    let mut n = 0;
    for block in code.split("\nblock") {
        let mut loaded = std::collections::HashSet::new();
        for l in block.lines().map(str::trim_start) {
            let Some((op, rest)) = l.split_once(' ') else { continue };
            let addr = rest.split_once(", [").and_then(|(_, a)| a.strip_suffix(']')).filter(|a| !a.starts_with("sp") && !a.contains(',')).map(reg);
            match (op, addr) {
                ("str", Some(a)) if loaded.contains(&a) => n += 1,
                ("str", _) => {}
                (_, a) => {
                    // A load's address, then its destination written (its own address too, if the same register).
                    if let (Some(a), true) = (a, op == "ldr") {
                        loaded.insert(a);
                    }
                    if let Some(d) = rest.split(',').next() {
                        loaded.remove(&reg(d));
                    }
                }
            }
        }
    }
    n
}

/// The nearest ancestors' kernel (`nearest_jump`, the packed transducer's, decisions 0038 and 0040), the simplest of
/// the engine's kernels, compiled as written by hand: its `ukl_atomic_load` (OpenCL 1.2's `atomic_or(p, 0)`) is each
/// lane's load, its `ukl_atomic_store` (an `atomic_xchg` whose old value no one reads) each lane's store, and its
/// `ukl_atomic_max` of the lanes' steps at one address the lanes' maximum and one atomic: the code reads and writes
/// back one address once, the maximum's. Each cell's answer, on the interpreter and the compiled SSA at every width,
/// is its nearest strict ancestor in its set.
#[test]
fn the_nearest_kernels_atomics_are_loads_stores_and_one_maximum() {
    let text = include_str!("../../../src/code_rule_dsl/transducer/packed/nearest.uglykl");
    let src = super::source("/<kernels>/nearest.uglykl", text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("nearest_jump").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    // 4 sets of a binary tree of 16 nodes (node i's parent (i - 1) / 2); set s holds the nodes i with i % (s + 2) == 0.
    const NODES: u32 = 16;
    const FINAL: u32 = 0x8000_0000;
    let parent = |i: u32| (i > 0).then(|| (i - 1) / 2);
    let member = |s: u32, i: u32| i % (s + 2) == 0;
    let (mut start, mut want) = (Vec::new(), Vec::new());
    for s in 0..4u32 {
        for i in 0..NODES {
            start.push(match parent(i) {
                None => u32::MAX,
                Some(q) if member(s, q) => FINAL | q,
                Some(q) => q,
            });
            let mut a = parent(i);
            while let Some(q) = a.filter(|&q| !member(s, q)) {
                a = parent(q);
            }
            want.push(a.map_or(u32::MAX, |q| FINAL | q));
        }
    }
    let run = |k: &super::vector::Kernel, exec| {
        let mut state = start.clone();
        let mut stats = [0u32; 1];
        let ptr = |p: *mut u32| super::run::Arg::Bytes((p as u64).to_le_bytes().to_vec());
        let args_of: Vec<u8> = [NODES, 4 * NODES, 0, 0].iter().flat_map(|x| x.to_le_bytes()).collect();
        let args = [ptr(state.as_mut_ptr()), ptr(stats.as_mut_ptr()), super::run::Arg::Bytes(args_of), super::run::Arg::Bytes(vec![0; 8]), super::run::Arg::Bytes(vec![0; 4])];
        super::run::run(&p, k, &args, super::run::Launch { grid: [2, 1, 1], block: [32, 1, 1], local: 0, resident: 1, exec }).unwrap_or_else(|e| panic!("{exec:?}: {}", e.0));
        assert!((1..=4).contains(&stats[0]), "{exec:?}: the most steps a cell took, {}", stats[0]);
        state
    };
    assert_eq!(run(&k, super::run::Exec::Interpret), want, "the interpreter");
    let ssa = ssa_of(&p, &k);
    // Its arguments' struct (read only, as its inner function's `const` parameter) read where the launch put it.
    let frame: Vec<_> = ssa.funcs[0].insts.iter().filter(|i| matches!(i.op, super::ssa::O::FrameAddr | super::ssa::O::Copy)).map(|i| i.op).collect();
    assert!(frame.is_empty(), "the kernel reads its lanes' frames: {frame:?}\n{}", ssa.listing());
    // Its fields read once, at the entry, as values: the struct is the third argument.
    let f = &ssa.funcs[0];
    let mut of_args: Vec<super::ssa::V> = (0..f.insts.len() as super::ssa::V).filter(|&x| f.inst(x).op == super::ssa::O::Arg && f.inst(x).a == 2).collect();
    for x in 0..f.insts.len() as super::ssa::V {
        if f.inst(x).op == super::ssa::O::Bin && of_args.contains(&f.inst(x).a) {
            of_args.push(x);
        }
    }
    let fields: Vec<super::ssa::V> = (0..f.insts.len() as super::ssa::V).filter(|&x| f.inst(x).op == super::ssa::O::Load && of_args.contains(&f.inst(x).a)).collect();
    assert!(!fields.is_empty() && fields.iter().all(|x| f.blocks[0].insts.contains(x)), "the struct's fields {fields:?} read past the entry:\n{}", ssa.listing());
    // An inlined function's parameter is its call's argument in every lane: no address is chosen by lanes (the
    // source chooses none), so `stats`' address is uniform.
    let chosen: Vec<_> = (0..f.insts.len() as super::ssa::V).filter(|&x| f.inst(x).op == super::ssa::O::Select && f.inst(x).m == super::k::M::PTR).collect();
    assert!(chosen.is_empty(), "addresses chosen by lanes {chosen:?}:\n{}", ssa.listing());
    // The loop's `steps < args.nodes`, which every running lane agrees on, an operand of `&&` with a lane's own
    // `(v & FINAL) == 0`: a compare in lanes, not the first running lane's `steps` taken (a horizontal operation each
    // round) and the compare's bool made a mask again.
    let firsts: Vec<_> = (0..f.insts.len() as super::ssa::V).filter(|&x| f.inst(x).op == super::ssa::O::First).collect();
    assert!(firsts.is_empty(), "values taken from the first running lane {firsts:?}:\n{}", ssa.listing());
    // `w`'s load set in the running lanes, then `v` to it: one select each, neither reading through the other.
    let sel = |x: super::ssa::V| f.inst(x).op == super::ssa::O::Select;
    let chained: Vec<_> = (0..f.insts.len() as super::ssa::V).filter(|&x| sel(x) && [f.inst(x).b, f.inst(x).c].iter().any(|&y| sel(y) && f.inst(y).a == f.inst(x).a)).collect();
    assert!(chained.is_empty(), "selects of selects under one mask {chained:?}:\n{}", ssa.listing());
    for w in [1u32, 2, 4, 8, 16, 32] {
        let (c, _) = super::cl::compile(&ssa, &k.name, w, &super::exec::places()).unwrap();
        let at = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
        let _ = at.cl.set(Ok(c));
        assert_eq!(run(&at, super::run::Exec::Ssa), want, "the compiled SSA at width {w}");
        #[cfg(target_arch = "aarch64")]
        {
            let (c, _) = super::cl::compile_listed(&ssa, &k.name, w, &super::exec::places()).unwrap();
            let code = c.listing.last().expect("the kernel's code for a chunk whose lanes all run");
            assert!(read_modify_writes(code) <= 1, "w{w}: {} addresses read and written back:\n{code}", read_modify_writes(code));
            // A branch on whether a lane is on: a test of the mask's lanes, not of a bool made of them.
            let lines: Vec<&str> = code.lines().map(str::trim_start).collect();
            let remade = lines.windows(3).filter(|l| l[0].starts_with("cset ") && l[1].starts_with("uxtb ") && l[2].starts_with("cbnz ")).count();
            assert!(w < 4 || remade == 0, "w{w}: {remade} branches on a bool made of the mask's lanes:\n{code}");
            // A compare's lanes as a mask: the compare's own, not one made again of its bool.
            let again = lines.windows(2).filter(|l| l[0].starts_with("ushr v") && l[0].ends_with("#31") && l[1].starts_with("cmeq v") && l[1].ends_with("#0")).count();
            assert!(w < 4 || again == 0, "w{w}: {again} masks made again of a compare's bools:\n{code}");
            // A gather of `state[base + v]` as written by hand: each lane's 32-bit index out of its vector, and the
            // load scales and extends it (`ldr w, [x, w, uxtw #2]`), and a store's lanes the same: no lane's 64-bit
            // address made in vectors.
            let scaled = lines.iter().filter(|l| l.starts_with("ldr w") && l.to_lowercase().ends_with(", uxtw #2]")).count();
            let widened = lines.iter().filter(|l| l.starts_with("shl v") && l.contains(".2d")).count();
            assert!(w < 4 || (scaled >= w as usize && widened == 0), "w{w}: {scaled} scaled lane loads, {widened} indexes scaled to addresses in vectors:\n{code}");
            // The thread's `cell` (its global id, a `uint32_t`) made as 32-bit lanes: no 64-bit lanes narrowed to it.
            let wide = lines.iter().filter(|l| l.starts_with("add v") && l.contains(".2d")).count();
            assert!(w < 4 || wide == 0, "w{w}: {wide} sums in 64-bit lanes:\n{code}");
        }
    }
}

/// A kernel's struct of arguments passed by value to inlined functions: one that only reads its copy reads the
/// launch's bytes where they are, one that writes its copy gets a copy (in each lane's frame), so the struct the
/// others read is as launched. On the interpreter and the compiled SSA at every width.
#[test]
fn a_struct_argument_is_read_in_place_unless_its_copy_is_written() {
    let text = "#include <ukl/backend.h>\n#include <ukl/memory.h>\n\
        typedef struct P { uint32_t a, b; } P;\nUKL_REFLECT_STRUCT(P, a, b)\n\
        UKL_INLINE uint32_t bumped(P p, uint32_t x) { p.a += x; return p.a * p.b; }\n\
        UKL_INLINE uint32_t total(P p) { return p.a + p.b; }\n\
        DEFINE_KERNEL2(rule, t_args,\n               RWBUFFER, uint32_t, out, \"[64]\",\n               ARGS,     P,        args, ,\n               GID0,     uint32_t, lid, )\n\
        {\n    out[lid] = bumped(args, lid) + total(args) + args.a;\n}\n";
    let src = super::source("/<kernels>/rule.uglykl", text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("t_args").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    let want: Vec<u32> = (0..64u32).map(|l| (5 + l) * 7 + 12 + 5).collect();
    let run = |k: &super::vector::Kernel, exec| {
        let mut out = vec![0u32; 64];
        let args = [
            super::run::Arg::Bytes((out.as_mut_ptr() as u64).to_le_bytes().to_vec()),
            super::run::Arg::Bytes([5u32, 7].iter().flat_map(|x| x.to_le_bytes()).collect()),
            super::run::Arg::Bytes(vec![0; 8]),
            super::run::Arg::Bytes(vec![0; 4]),
        ];
        super::run::run(&p, k, &args, super::run::Launch { grid: [2, 1, 1], block: [32, 1, 1], local: 0, resident: 1, exec }).unwrap_or_else(|e| panic!("{exec:?}: {}", e.0));
        out
    };
    assert_eq!(run(&k, super::run::Exec::Interpret), want, "the interpreter");
    let ssa = ssa_of(&p, &k);
    let copies = ssa.funcs[0].insts.iter().filter(|i| i.op == super::ssa::O::Copy).count();
    assert_eq!(copies, 1, "one copy, the one `bumped` writes:\n{}", ssa.listing());
    for w in [1u32, 2, 4, 8, 16, 32] {
        let (c, _) = super::cl::compile(&ssa, &k.name, w, &super::exec::places()).unwrap();
        let at = super::vector::Kernel::from_bytes(&k.to_bytes()).unwrap();
        let _ = at.cl.set(Ok(c));
        assert_eq!(run(&at, super::run::Exec::Ssa), want, "the compiled SSA at width {w}");
    }
}

/// The nearest ancestors' kernel as Rust (`rs.rs`), at 8 lanes: written from its vector tape, it is the checked-in
/// module of `rerail-ukl-aot` (whose tests run it against a scalar and a hand-vectorized version). `UKL_RS_WRITE=1`
/// writes it again.
#[test]
fn the_nearest_kernels_rust_is_the_checked_in_one() {
    let text = include_str!("../../../src/code_rule_dsl/transducer/packed/nearest.uglykl");
    let src = super::source("/<kernels>/nearest.uglykl", text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("nearest_jump").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    let rust = super::rs::kernel(&k, 8).unwrap_or_else(|e| panic!("{}\n{}", e.0, super::k::View::new(&k.tape).listing()));
    let at = concat!(env!("CARGO_MANIFEST_DIR"), "/../ukl_aot/src/nearest_jump_w8.rs");
    if std::env::var_os("UKL_RS_WRITE").is_some() {
        std::fs::write(at, &rust).unwrap();
    }
    let kept = std::fs::read_to_string(at).unwrap_or_default();
    assert!(kept == rust, "{at} is not the kernel's Rust (UKL_RS_WRITE=1 writes it):\n{rust}");
}

/// The nearest kernel's Rust masks only where its lanes part (decision 0567 step 3):
/// - a group's chunks of 8 threads all run: a chunk past the group's last thread is the one slow path (1);
/// - `cell >= args.cells`, of consecutive lanes against one value, is decided for the chunk at once, its one chunk
///   whose lanes are on both sides the slow path (2);
/// - a test of a mask's lanes is a comparison of its bits, which the compiler folds where the mask is known (3);
/// - in the loop, whose lanes part (each its own rounds), a variable no lane reads after it leaves is set in every
///   lane, `steps` counted by its mask, and the maximum of `steps > 0` lanes is the lanes' maximum, unmasked (4).
///
/// So where a function's lanes all run (its `_f` instance), no mask is read outside the loop and the slow paths.
#[test]
fn the_nearest_kernels_rust_masks_only_where_its_lanes_part() {
    let text = include_str!("../../../src/code_rule_dsl/transducer/packed/nearest.uglykl");
    let src = super::source("/<kernels>/nearest.uglykl", text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("nearest_jump").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    let rust = super::rs::kernel(&k, 8).unwrap_or_else(|e| panic!("{}", e.0));
    let lines: Vec<&str> = rust.lines().collect();
    // (1) Full chunks, then the group's last threads.
    assert!(rust.contains("while lid0 + 8 <= n {") && rust.contains("if lid0 < n {"), "no chunks of all lanes:\n{rust}");
    // The functions where all lanes run: outside the slow paths (marked, to the end of their block), no mask.
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut full = 0;
    let mut i = 0;
    while i < lines.len() {
        if !(lines[i].starts_with("unsafe fn ") && lines[i].contains("_f(")) {
            i += 1;
            continue;
        }
        full += 1;
        let name = lines[i];
        i += 1;
        while i < lines.len() && lines[i] != "}" {
            let l = lines[i];
            if l.trim_start().starts_with("// Lanes part") {
                let d = indent(l);
                while i < lines.len() && !(indent(lines[i]) < d && !lines[i].trim().is_empty()) {
                    i += 1;
                }
                continue;
            }
            // A full round's end, where some lane's condition fails, is the one test of a mask.
            let round = l.trim_start().starts_with("if !all(&") && l.ends_with("{ break; }");
            let masked = ["vbslq_u32", "vbicq_u32", "select_unpredictable", "any(&", "all(&", "live[", "vcgtq_u32"].iter().find(|t| l.contains(*t) && !round);
            assert!(masked.is_none(), "{name}: a mask ({}) where all lanes run: {l}\n{rust}", masked.unwrap());
            i += 1;
        }
    }
    // `nearest_jump`'s own, its load of `state[cell]` and its maximum: the gather and the store run in the loop only.
    assert!(full == 3, "{full} functions of all lanes:\n{rust}");
    // Its loop's lanes part early, and it says nothing of them: no full rounds, whose test would be a second branch.
    assert!(!rust.contains("// Full rounds"), "full rounds of a loop not said to go on together:\n{rust}");
    // A chunk whose lanes all return runs nothing after.
    let after: Vec<&[&str]> = lines.windows(2).filter(|w| w[0].trim() == "return;" && !w[1].trim().starts_with('}')).collect();
    assert!(after.is_empty(), "statements after a return: {after:?}\n{rust}");
    // (2) The bounds check decided for a chunk at once.
    assert!(lines.iter().any(|l| l.contains("as u64 + 7")), "`cell >= args.cells` not decided for the chunk:\n{rust}");
    // (3) Mask tests of the mask's bits.
    assert!(rust.contains("fn any(") && !rust.contains("vminvq_u32(") && !rust.contains("vmaxvq_u32(vorrq_u32"), "mask tests by reductions:\n{rust}");
    // (4) No variable set by a mask's select; `steps` counted by its mask.
    let selected: Vec<&&str> = lines.iter().filter(|l| { let t = l.trim_start(); t.starts_with('x') && t.contains(" = [vbslq_u32(") }).collect();
    assert!(selected.is_empty(), "variables set by a select: {selected:?}\n{rust}");
    assert!(lines.iter().any(|l| { let t = l.trim_start(); t.starts_with('x') && t.contains(" = [vsubq_u32(x") }), "`steps` not counted by its mask:\n{rust}");
    // (5) In the loop, after every lane read its own cell of `state` (`_v0`, its address valid in each): the gather
    // of a lane that does not run reads its own cell, not a running lane's (no reduction for one); and the store of
    // `state[cell]`, whose every write is a thread's own cell (no other thread writes a lane's), writes back what is
    // there in a lane that does not run: a load, a select and the registers' store, no store of each lane.
    let funcs: Vec<(&str, String)> = rust.split("\nunsafe fn ").skip(1).map(|f| (f.split('(').next().unwrap(), f.split("\n}\n").next().unwrap().to_string())).collect();
    let owned: Vec<&(&str, String)> = funcs.iter().filter(|(n, _)| n.contains("_v0")).collect();
    assert!(owned.len() >= 2, "functions of lanes whose own cells are valid: {:?}\n{rust}", funcs.iter().map(|f| f.0).collect::<Vec<_>>());
    for (n, f) in &owned {
        assert!(!f.contains("vdupq_n_u32(vmaxvq_u32(") && !f.contains("select_unpredictable"), "{n}: a running lane's address for one that does not run, or a store of each lane:\n{f}");
    }
    assert!(owned.iter().any(|(_, f)| f.contains("vst1q_u32(") && f.contains("vbslq_u32(") && f.contains("vld1q_u32(")), "no store of the registers, its lanes that do not run what is there:\n{rust}");
}

/// `mix.uglykl` (rerail-ukl-aot's test kernel) as Rust at 8 lanes, checked in as `nearest` is.
fn mix_rust() -> String {
    let text = include_str!("../../ukl_aot/kernels/mix.uglykl");
    let src = super::source("/<kernels>/mix.uglykl", text, &[]).unwrap();
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("mix_lanes").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    super::rs::kernel(&k, 8).unwrap_or_else(|e| panic!("{}\n{}", e.0, super::k::View::new(&k.tape).listing()))
}

#[test]
fn the_mix_kernels_rust_is_the_checked_in_one() {
    let rust = mix_rust();
    let at = concat!(env!("CARGO_MANIFEST_DIR"), "/../ukl_aot/src/mix_lanes_w8.rs");
    if std::env::var_os("UKL_RS_WRITE").is_some() {
        std::fs::write(at, &rust).unwrap();
    }
    let kept = std::fs::read_to_string(at).unwrap_or_default();
    assert!(kept == rust, "{at} is not the kernel's Rust (UKL_RS_WRITE=1 writes it):\n{rust}");
}

/// Where the lanes of `mix` part (decision 0567 step 3):
/// - an `if` that rejoins runs both sides, each masked, with no test of whether any lane takes it;
/// - `UKL_LIKELY(c)` runs its side unmasked in a chunk whose lanes all take it, `UKL_UNLIKELY(c)` the other side
///   in a chunk none of whose lanes do; any other chunk runs both sides masked;
/// - a loop of each lane's own bound, said to go on together (`UKL_LIKELY`), runs unmasked rounds while every lane
///   goes on, then masked rounds; a loop of one bound runs unmasked rounds, its test the loop's own.
#[test]
fn the_mix_kernels_rust_runs_both_sides_and_full_rounds() {
    let rust = mix_rust();
    let lines: Vec<&str> = rust.lines().map(str::trim).collect();
    // The tests of whether any lane runs: those that return (`i >= args.n`), and the rest after them.
    let guarded: Vec<&[&str]> = lines.windows(2).filter(|w| w[0].starts_with("if any(") && w[0] != "if any(&live) {" && !w[1].starts_with("live = [vbicq_u32(live[0]")).collect();
    assert!(guarded.is_empty(), "a side guarded by whether any lane takes it: {guarded:?}\n{rust}");
    assert!(lines.iter().any(|l| l.starts_with("if all(&") && l.ends_with('{')), "no unmasked side of the likely `if`:\n{rust}");
    assert!(lines.iter().any(|l| l.starts_with("if !any(&") && l.ends_with('{')), "no unmasked side of the unlikely `if`:\n{rust}");
    let f = rust.split("\nunsafe fn ").find(|f| f.starts_with("b0_") && f.contains("_f(")).unwrap_or_else(|| panic!("no kernel function of all lanes:\n{rust}"));
    let lines: Vec<&str> = f.lines().map(str::trim).collect();
    let full: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| **l == "// Full rounds: every lane goes on.").map(|(i, _)| i).collect();
    assert!(full.len() == 1, "{} loops of full rounds, not 1:\n{rust}", full.len());
    // The hinted loop: its full rounds end where a lane's condition fails, then each lane its own.
    let part = lines[full[0]..].iter().position(|l| l.starts_with("// Lanes part: each its own rounds.")).unwrap_or_else(|| panic!("no masked rounds after the full ones:\n{rust}"));
    assert!(lines[full[0]..full[0] + part].iter().any(|l| l.starts_with("if !all(&") && l.ends_with("{ break; }")), "full rounds that do not end where a lane does:\n{rust}");
    assert!(!lines[full[0]..full[0] + part].iter().any(|l| l.contains("vbslq_u32(")), "a masked write in a full round:\n{rust}");
    // The loop of one bound, in every instance: its test the condition itself, its counter one value.
    for f in rust.split("\nunsafe fn ").filter(|f| f.starts_with("b0_")) {
        let lines: Vec<&str> = f.lines().map(str::trim).collect();
        let one = lines.iter().position(|l| *l == "// Rounds of one condition: every lane goes on.").unwrap_or_else(|| panic!("no rounds of one condition:\n{f}"));
        let end = lines[one..].iter().position(|l| *l == "}").unwrap() + one;
        assert!(lines[one..end].iter().any(|l| l.starts_with("if !(") && l.ends_with("{ break; }") && !l.contains("any(") && !l.contains("all(")), "the loop of one bound tests its lanes:\n{f}");
        assert!(lines[one..end].iter().any(|l| l.contains("wrapping_add(1") || l.contains("+ 1")), "the loop of one bound counts in lanes:\n{f}");
    }
}
