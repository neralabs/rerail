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
    let mut out = vec![0u32; 64];
    let ptr = (out.as_mut_ptr() as u64).to_le_bytes().to_vec();
    let args = [super::run::Arg::Bytes(ptr), super::run::Arg::Bytes(vec![0; 8]), super::run::Arg::Bytes(vec![0; 4])];
    super::run::run(&p, &k, &args, [1, 1, 1], [64, 1, 1], 0, 1).unwrap();
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
                Ok(vk) if std::env::var("RERAIL_UKL_VEC_TIMES").is_ok() => {
                    eprintln!("TIME {f}: {:.2} s, vector tape {} nodes, code {} words", t.elapsed().as_secs_f64(), super::k::View::new(&vk.tape).len(), vk.code.words.len());
                }
                Ok(_) => {}
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
    let mut out = vec![0u32; 64];
    let ptr = (out.as_mut_ptr() as u64).to_le_bytes().to_vec();
    let args = [super::run::Arg::Bytes(ptr), super::run::Arg::Bytes(vec![0; 8]), super::run::Arg::Bytes(vec![0; 4])];
    super::run::run(&p, &k, &args, [1, 1, 1], [64, 1, 1], 0, 1).unwrap_or_else(|e| panic!("{}", e.0));
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
