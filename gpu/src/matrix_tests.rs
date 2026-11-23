//! `ukl/matrix.h`'s tests (decision 0617, requirements R1-R6 and R9), on the process's device: each 64×64 block a
//! kernel's owners compute through fragments equals the host's product, exactly where the arithmetic is exact and
//! within the bound elsewhere, in every layout, from device and threadgroup memory, by level 1 and level 2, with
//! owners of one SIMD group or several. Without a device they pass, saying so.

use crate::device::{Buffer, ComputeQueue};
use crate::kernels::{alloc, buffer_of, launch, program_with, read, Program};
use crate::library_tests::{setup, Rng};
use crate::Pod;

const TESTS: &str = include_str!("matrix_tests.uglykl");

/// R7's program: one kernel that runs each type's product once, from device memory, the code a test reads (its
/// compiled text) apart from the other tests' kernels. (Metal's compiler failed the whole test program as a dynamic
/// library, which is how its text is read: "Compiler encountered an internal error".)
const UNITS: &str = r#"
#include <ukl/matrix.h>
#define T_UNIT(T, C, ET, CT, MEM)                                                                                        \
    {                                                                                                                   \
        ukl_mma_a_##T a;                                                                                                \
        ukl_mma_b_##T b;                                                                                                \
        ukl_mma_c_##C c;                                                                                                \
        const uint32_t k = UKL_MMA_K(T) / UKL_MMA_PACK(T);                                                              \
        ukl_mma_load_a_##T(&a, (UKL_GLOBAL const ET*)MEM, k, UKL_ROW_MAJOR, at);                                        \
        ukl_mma_load_b_##T(&b, (UKL_GLOBAL const ET*)MEM, 16u, UKL_ROW_MAJOR, at);                                      \
        ukl_mma_load_c_##C(&c, (UKL_GLOBAL const CT*)out, 16u, UKL_ROW_MAJOR, at);                                      \
        ukl_mma_##T(&c, &a, &b, at __UKL_ARGS);                                                                         \
        ukl_mma_store_c_##C(&c, (UKL_GLOBAL CT*)out, 16u, UKL_ROW_MAJOR, at);                                           \
    }
DEFINE_KERNEL2(tests, t_mma_units,
               ROBUFFER, uint64_t, mem, "[1024]",
               RWBUFFER, uint32_t, out, "[256]",
               ROBUFFER, uint32_t, sel, "[1]",
               LID0,     uint32_t, lid, )
{
    UklMmaAt at = ukl_mma_at(lid);
    const uint32_t which = sel[0];
    if (which == 0u) T_UNIT(f16, f32, uint16_t, float, mem)
    if (which == 1u) T_UNIT(bf16, f32, uint16_t, float, mem)
    if (which == 2u) T_UNIT(i8, i32, int8_t, int32_t, mem)
    if (which == 3u) T_UNIT(b1, i32, uint64_t, int32_t, mem)
}
"#;

#[repr(C)]
#[derive(Clone, Copy)]
struct MmaArgs {
    k: u32,
    la: u32,
    lb: u32,
    lc: u32,
    tile: u32,
    shared_mem: u32,
}
unsafe impl Pod for MmaArgs {}

const ROW: u32 = 0;
const COL: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Ty {
    F16,
    Bf16,
    I8,
    /// Bits: `C += popcount(A & B)`, 64 of them a word along K.
    B1,
}

impl Ty {
    fn kernel(self) -> &'static str {
        match self {
            Ty::F16 => "t_mma_f16",
            Ty::Bf16 => "t_mma_bf16",
            Ty::I8 => "t_mma_i8",
            Ty::B1 => "t_mma_b1",
        }
    }
    /// An element's value: an f16's or a bf16's bits, an i8.
    fn value(self, x: i32) -> f64 {
        match self {
            Ty::F16 => f16_to_f64(x as u16),
            Ty::Bf16 => f32::from_bits((x as u32) << 16) as f64,
            Ty::I8 | Ty::B1 => x as f64,
        }
    }
    /// Whether C is integers.
    fn int(self) -> bool {
        matches!(self, Ty::I8 | Ty::B1)
    }
    /// K for each test's sizes: the type's own multiple (b1's 256 bits) of the others' `k`.
    fn k(self, k: u32) -> u32 {
        if self == Ty::B1 { 256 * k.div_ceil(64) } else { k }
    }
    /// The input type's precision in bits (R3's bound).
    fn precision(self) -> i32 {
        match self {
            Ty::F16 => 11,
            Ty::Bf16 => 8,
            Ty::I8 | Ty::B1 => 0,
        }
    }
    /// The element of a small integer `v` (|v| <= 64): exact in every type.
    fn small(self, v: i32) -> i32 {
        match self {
            Ty::F16 => f16_of_small(v) as i32,
            Ty::Bf16 => ((v as f32).to_bits() >> 16) as i32,
            Ty::I8 => v,
            Ty::B1 => v & 1,
        }
    }
    /// A random element: a float of moderate size (f16 2^-7..2^8, bf16 2^-30..2^30), or any i8.
    fn random(self, r: &mut Rng) -> i32 {
        match self {
            Ty::F16 => ((r.below(2) as u32) << 15 | ((r.below(16) as u32 + 8) << 10) | r.below(1024) as u32) as i32,
            Ty::Bf16 => ((r.below(2) as u32) << 15 | ((r.below(61) as u32 + 97) << 7) | r.below(128) as u32) as i32,
            Ty::I8 => r.below(256) as i32 - 128,
            Ty::B1 => r.below(2) as i32,
        }
    }
}

fn f16_to_f64(h: u16) -> f64 {
    let (s, e, m) = ((h >> 15) as i32, ((h >> 10) & 31) as i32, (h & 0x3ff) as f64);
    let v = if e == 0 { m * 2f64.powi(-24) } else { (1.0 + m / 1024.0) * 2f64.powi(e - 15) };
    if s != 0 { -v } else { v }
}

fn f16_of_small(v: i32) -> u16 {
    if v == 0 {
        return 0;
    }
    let f = (v.unsigned_abs() as f32).to_bits();
    let h = (((f >> 23) - 112) << 10) | ((f >> 13) & 0x3ff);
    (h | if v < 0 { 0x8000 } else { 0 }) as u16
}

/// The matrix programs, by an emulated owner's SIMD groups (`UKL_MMA_EMULATED_GROUPS`), and their owners' lanes.
fn programs(groups: u32) -> (&'static Program, u32) {
    let g = format!("{groups}u");
    let p = program_with("matrix_tests.uglykl", TESTS, &[("UKL_MMA_EMULATED_GROUPS", g.as_str())]).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    (p, consts(p)[16])
}

/// The program with the emulation forced where a native form exists (`UKL_MMA_FORCE_EMULATED`, R6).
fn emulated() -> &'static Program {
    program_with("matrix_tests.uglykl", TESTS, &[("UKL_MMA_EMULATED_GROUPS", "1u"), ("UKL_MMA_FORCE_EMULATED", "1")]).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()))
}

/// R1's constants: per type M, N, K, groups, native (f16, bf16, i8); the SIMD width, an owner's lanes, the block;
/// then b1's five.
fn consts(p: &Program) -> Vec<u32> {
    let Some((_, mut q)) = setup() else { return vec![0; 23] };
    let out = alloc::<u32>(23).unwrap();
    launch(&q, p.function("t_mma_consts").unwrap()).buffer("out", &out).run(&[1], &[1]).unwrap();
    q.flush().await_().unwrap();
    read(&out, 23)
}

/// A block's inputs, logically row-major (`a[r * k + kk]`, `b[kk * 64 + c]`), and C's starting value's bits.
struct Inputs {
    k: u32,
    a: Vec<i32>,
    b: Vec<i32>,
    c: Vec<u32>,
}

impl Inputs {
    fn new(ty: Ty, k: u32, rng: &mut Rng, small: bool) -> Inputs {
        let el = |r: &mut Rng| if small { ty.small(r.below(17) as i32 - 8) } else { ty.random(r) };
        let a = (0..64 * k).map(|_| el(rng)).collect();
        let b = (0..64 * k).map(|_| el(rng)).collect();
        let c = (0..64 * 64)
            .map(|_| {
                let v = rng.below(201) as i32 - 100;
                if ty.int() { v as u32 } else { (v as f32).to_bits() }
            })
            .collect();
        Inputs { k, a, b, c }
    }

    /// The host's product, exact (f64 holds every sum here exactly where the floats are small integers), and each
    /// element's `Σ|a||b| + |c|` for R3's bound: in C's logical row-major order.
    fn reference(&self, ty: Ty) -> (Vec<f64>, Vec<f64>) {
        let k = self.k as usize;
        let (mut exact, mut mag) = (vec![0.0; 64 * 64], vec![0.0; 64 * 64]);
        for r in 0..64 {
            for c in 0..64 {
                let c0 = if ty.int() { self.c[r * 64 + c] as i32 as f64 } else { f32::from_bits(self.c[r * 64 + c]) as f64 };
                let (mut s, mut m) = (c0, c0.abs());
                for kk in 0..k {
                    let p = ty.value(self.a[r * k + kk]) * ty.value(self.b[kk * 64 + c]);
                    s += p;
                    m += p.abs();
                }
                exact[r * 64 + c] = s;
                mag[r * 64 + c] = m;
            }
        }
        (exact, mag)
    }
}

/// `m[r][c]` of a `rows × cols` matrix, logically row-major in `logical`, laid out in memory.
fn lay<T: Copy>(logical: &[T], rows: usize, cols: usize, layout: u32) -> Vec<T> {
    let mut out = logical.to_vec();
    if layout == COL {
        for r in 0..rows {
            for c in 0..cols {
                out[c * rows + r] = logical[r * cols + c];
            }
        }
    }
    out
}

/// A `rows × cols` matrix, logically row-major, in memory in `layout`: b1's bits packed into words along K first
/// (`along_rows`: K runs down the columns, B's).
fn elements(ty: Ty, v: &[i32], rows: usize, cols: usize, along_rows: bool, layout: u32) -> Buffer {
    match ty {
        Ty::I8 => buffer_of(&lay(&v.iter().map(|&x| x as i8).collect::<Vec<_>>(), rows, cols, layout)).unwrap(),
        Ty::B1 if along_rows => {
            let w: Vec<u64> = (0..rows / 64 * cols).map(|i| (0..64).map(|t| (v[(64 * (i / cols) + t) * cols + i % cols] as u64) << t).sum()).collect();
            buffer_of(&lay(&w, rows / 64, cols, layout)).unwrap()
        }
        Ty::B1 => {
            let w: Vec<u64> = (0..rows * (cols / 64)).map(|i| (0..64).map(|t| (v[(i / (cols / 64)) * cols + 64 * (i % (cols / 64)) + t] as u64) << t).sum()).collect();
            buffer_of(&lay(&w, rows, cols / 64, layout)).unwrap()
        }
        _ => buffer_of(&lay(&v.iter().map(|&x| x as u16).collect::<Vec<_>>(), rows, cols, layout)).unwrap(),
    }
}

/// One block on the device: C's bits, logically row-major.
#[allow(clippy::too_many_arguments)]
fn block(p: &Program, q: &mut ComputeQueue, ty: Ty, inp: &Inputs, layouts: [u32; 3], tile: bool, local: bool, threads: u32) -> Vec<u32> {
    let k = inp.k as usize;
    let [la, lb, lc] = layouts;
    let a = elements(ty, &inp.a, 64, k, false, la);
    let b = elements(ty, &inp.b, k, 64, true, lb);
    let c = buffer_of(&lay(&inp.c, 64, 64, lc)).unwrap();
    let args = MmaArgs { k: inp.k, la, lb, lc, tile: tile as u32, shared_mem: local as u32 };
    launch(q, p.function(ty.kernel()).unwrap()).value("args", &args).buffer("A", &a).buffer("B", &b).buffer("Cm", &c).run(&[1], &[threads as u64]).unwrap();
    q.flush().await_().unwrap();
    let got: Vec<u32> = read(&c, 64 * 64);
    // Back to row-major: a column-major C transposed.
    lay(&got, 64, 64, lc)
}

fn value(ty: Ty, bits: u32) -> f64 {
    if ty.int() { bits as i32 as f64 } else { f32::from_bits(bits) as f64 }
}

/// Every element equal to the exact product (R2).
fn assert_exact(ty: Ty, inp: &Inputs, got: &[u32], what: &str) {
    let (exact, _) = inp.reference(ty);
    for (i, (&g, &e)) in got.iter().zip(&exact).enumerate() {
        assert_eq!(value(ty, g), e, "{} {ty:?} {what}: C[{}][{}]", crate::device::backend(), i / 64, i % 64);
    }
}

/// R1: each type's shape divides the block, and an owner's lanes are its groups' (on every device: the constants,
/// said).
#[test]
fn matrix_shapes_divide_the_block() {
    if setup().is_none() {
        return;
    }
    let (p, lanes) = programs(1);
    let c = consts(p);
    eprintln!("matrix units on {}: f16 {:?}, bf16 {:?}, i8 {:?}, b1 {:?} (M, N, K, groups, native); SIMD width {}, an owner {} lanes", crate::device::backend(), &c[0..5], &c[5..10], &c[10..15], &c[18..23], c[15], lanes);
    for t in [0, 5, 10, 18] {
        let s = &c[t..t + 5];
        assert!(s[0..2].iter().all(|&d| d > 0 && 64 % d == 0), "the shape {:?} divides 64", &s[0..3]);
        if t == 18 {
            assert!(s[2] > 0 && s[2] % 64 == 0, "b1's K {} is whole words", s[2]);
        } else {
            assert!(s[2] > 0 && 64 % s[2] == 0, "the shape {:?} divides 64", &s[0..3]);
        }
        assert_eq!(s[3] * c[15], lanes, "an owner's lanes are its groups'");
    }
    assert_eq!(c[17], 64);
}

/// R2 and R9: small integers, every product and sum exact, in each type: K = 128 row-major by level 1 and by
/// level 2; K = 32 in all eight layouts; K = 32 from threadgroup memory, all row-major and all column-major.
#[test]
fn matrix_products_are_exact_on_small_integers() {
    let Some((_, mut q)) = setup() else { return };
    let (p, lanes) = programs(1);
    let mut rng = Rng(61);
    for ty in [Ty::F16, Ty::Bf16, Ty::I8, Ty::B1] {
        let big = Inputs::new(ty, ty.k(128), &mut rng, true);
        assert_exact(ty, &big, &block(p, &mut q, ty, &big, [ROW; 3], false, false, 4 * lanes), "K 128, level 1");
        assert_exact(ty, &big, &block(p, &mut q, ty, &big, [ROW; 3], true, false, 4 * lanes), "K 128, level 2");
        let inp = Inputs::new(ty, ty.k(32), &mut rng, true);
        for l in 0..8u32 {
            let layouts = [l & 1, (l >> 1) & 1, (l >> 2) & 1];
            assert_exact(ty, &inp, &block(p, &mut q, ty, &inp, layouts, false, false, 2 * lanes), &format!("layouts {layouts:?}"));
            if l == 0 || l == 7 {
                assert_exact(ty, &inp, &block(p, &mut q, ty, &inp, layouts, false, true, 2 * lanes), &format!("layouts {layouts:?}, threadgroup memory"));
            }
        }
    }
}

/// R3 and R9: random floats within `K · 2^-p · (Σ|a||b| + |c|)` of the exact product (p the input type's precision);
/// i8's random bytes exactly. Level 2 equals level 1 to the bit.
#[test]
fn matrix_products_are_within_the_bound_on_random_inputs() {
    let Some((_, mut q)) = setup() else { return };
    let (p, lanes) = programs(1);
    let mut rng = Rng(67);
    for ty in [Ty::F16, Ty::Bf16, Ty::I8, Ty::B1] {
        let k = ty.k(128);
        let inp = Inputs::new(ty, k, &mut rng, false);
        let got = block(p, &mut q, ty, &inp, [ROW, COL, ROW], false, false, 4 * lanes);
        if ty.int() {
            assert_exact(ty, &inp, &got, "random bytes or bits");
        } else {
            let (exact, mag) = inp.reference(ty);
            let unit = k as f64 * 2f64.powi(-ty.precision());
            for i in 0..64 * 64 {
                let (g, e) = (value(ty, got[i]), exact[i]);
                assert!((g - e).abs() <= unit * mag[i], "{} {ty:?}: C[{}][{}] = {g}, exact {e}, bound {}", crate::device::backend(), i / 64, i % 64, unit * mag[i]);
            }
        }
        let tiled = block(p, &mut q, ty, &inp, [ROW, COL, ROW], true, false, lanes);
        assert_eq!(tiled, got, "{} {ty:?}: level 2 equals level 1", crate::device::backend());
    }
}

/// R4: `UKL_MMA_EACH` visits each element of C once, at its true coordinates, with owners of 1, 2 and 4 groups.
#[test]
fn matrix_each_visits_every_element_at_its_coordinates() {
    let Some((_, mut q)) = setup() else { return };
    for groups in [1u32, 2, 4] {
        let (p, lanes) = programs(groups);
        let (cf, ci) = (alloc::<f32>(64 * 64).unwrap(), alloc::<i32>(64 * 64).unwrap());
        launch(&q, p.function("t_mma_each").unwrap()).buffer("Cf", &cf).buffer("Ci", &ci).run(&[1], &[(2 * lanes).min(256) as u64]).unwrap();
        q.flush().await_().unwrap();
        let (f, i): (Vec<f32>, Vec<i32>) = (read(&cf, 64 * 64), read(&ci, 64 * 64));
        for r in 0..64 {
            for c in 0..64 {
                assert_eq!(f[r * 64 + c], (r * 256 + c) as f32, "{} groups {groups}: f32 C[{r}][{c}]", crate::device::backend());
                assert_eq!(i[r * 64 + c], 7 + ((r % 16) * 16 + c % 16) as i32, "{} groups {groups}: i32 C[{r}][{c}]", crate::device::backend());
            }
        }
    }
}

/// R5: owners of 1, 2 and 4 SIMD groups (emulated: `UKL_MMA_EMULATED_GROUPS`), in threadgroups of 1, 2 and 4 owners,
/// give the exact product.
#[test]
fn matrix_owners_of_several_groups_agree() {
    let Some((_, mut q)) = setup() else { return };
    let mut rng = Rng(71);
    let inputs: Vec<(Ty, Inputs)> = [Ty::F16, Ty::I8, Ty::B1].into_iter().map(|ty| (ty, Inputs::new(ty, ty.k(64), &mut rng, true))).collect();
    for groups in [1u32, 2, 4] {
        let (p, lanes) = programs(groups);
        for owners in [1u32, 2, 4] {
            if owners * lanes > 256 {
                continue;
            }
            for (ty, inp) in &inputs {
                let got = block(p, &mut q, *ty, inp, [ROW, COL, ROW], false, false, owners * lanes);
                assert_exact(*ty, inp, &got, &format!("groups {groups}, owners {owners}"));
            }
        }
    }
}

/// R6: where a type is native (`UKL_MMA_NATIVE`: f16, bf16, i8 and b1 on NVIDIA sm_80 and later), its products agree
/// with the emulation's, forced in another program (`UKL_MMA_FORCE_EMULATED`), in all eight layouts, by level 1 and
/// level 2, compared as R2 and R3: on small integers both are the exact product, to the bit; on random inputs the
/// integer types' equal the emulation's, and the floats' are each within R3's bound of the exact product (a unit's
/// sums run in its own order). The forced program says it is emulated. Where nothing is native the test says so.
#[test]
fn matrix_native_forms_are_the_emulations() {
    let Some((_, mut q)) = setup() else { return };
    let (p, lanes) = programs(1);
    let (c, e) = (consts(p), consts(emulated()));
    let types = native_types(&c);
    eprintln!("matrix units on {}: native {types:?}", crate::device::backend());
    for i in [4, 9, 14, 22] {
        assert_eq!(e[i], 0, "the forced program is emulated");
    }
    let mut rng = Rng(73);
    for ty in types {
        let small = Inputs::new(ty, ty.k(64), &mut rng, true);
        let random = Inputs::new(ty, ty.k(64), &mut rng, false);
        let (exact, mag) = random.reference(ty);
        let unit = random.k as f64 * 2f64.powi(-ty.precision());
        for l in 0..8u32 {
            let layouts = [l & 1, (l >> 1) & 1, (l >> 2) & 1];
            for tile in [false, true] {
                let threads = if tile { lanes } else { 2 * lanes };
                let what = format!("layouts {layouts:?}, level {}", 1 + tile as u32);
                let native = block(p, &mut q, ty, &small, layouts, tile, false, threads);
                assert_exact(ty, &small, &native, &format!("{what}, native"));
                assert_eq!(native, block(emulated(), &mut q, ty, &small, layouts, tile, false, threads), "{} {ty:?} {what}: native equals emulated", crate::device::backend());
                let native = block(p, &mut q, ty, &random, layouts, tile, false, threads);
                if ty.int() {
                    assert_eq!(native, block(emulated(), &mut q, ty, &random, layouts, tile, false, threads), "{} {ty:?} {what}: native equals emulated", crate::device::backend());
                } else {
                    for i in 0..64 * 64 {
                        let (g, x) = (value(ty, native[i]), exact[i]);
                        assert!((g - x).abs() <= unit * mag[i], "{} {ty:?} {what}: native C[{}][{}] = {g}, exact {x}, bound {}", crate::device::backend(), i / 64, i % 64, unit * mag[i]);
                    }
                }
            }
        }
    }
}

/// The types R1's constants `c` say are native.
fn native_types(c: &[u32]) -> Vec<Ty> {
    [(Ty::F16, 4), (Ty::Bf16, 9), (Ty::I8, 14), (Ty::B1, 22)].into_iter().filter(|&(_, i)| c[i] == 1).map(|(t, _)| t).collect()
}

/// The instruction a native type's product takes, in the backend's text of the compiled kernel: CUDA's PTX, Metal's
/// AIR intrinsics (`air.`, the 8×8 product into f32).
fn unit_instruction(ty: Ty) -> &'static str {
    let metal = crate::device::backend() == "Metal";
    match ty {
        Ty::F16 if metal => "air.simdgroup_matrix_8x8_multiply_accumulate.v64f32.v64f16.v64f16.v64f32",
        Ty::Bf16 if metal => "air.simdgroup_matrix_8x8_multiply_accumulate.v64f32.v64bf16.v64bf16.v64f32",
        Ty::F16 => "mma.sync.aligned.m16n8k16.row.col.f32.f16.f16.f32",
        Ty::Bf16 => "mma.sync.aligned.m16n8k16.row.col.f32.bf16.bf16.f32",
        Ty::I8 => "mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32",
        Ty::B1 => "mma.sync.aligned.m16n8k256.row.col.s32.b1.b1.s32.and.popc",
    }
}

/// A unit's instruction, of any type: what the forced emulation's text holds none of.
fn unit_marker() -> &'static str {
    if crate::device::backend() == "Metal" { "multiply_accumulate" } else { "mma.sync" }
}

/// R7: a native type's kernel takes the unit's instruction (its compiled text holds it: CUDA's PTX), and the forced
/// emulation's takes none. Where nothing is native the test says so; a native type on a backend with no text to read
/// fails, as R7 is then unchecked.
#[test]
fn matrix_native_forms_take_the_units_instruction() {
    if setup().is_none() {
        return;
    }
    let (p, _) = programs(1);
    let types = native_types(&consts(p));
    if types.is_empty() {
        eprintln!("matrix units on {}: none native", crate::device::backend());
        return;
    }
    let code = |defines: &[(&str, &str)]| {
        crate::kernels::native_code("matrix_units.uglykl", UNITS, defines)
            .unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()))
            .unwrap_or_else(|| panic!("{}: native {types:?}, and no compiled text to read (R7)", crate::device::backend()))
    };
    let native = code(&[("UKL_MMA_EMULATED_GROUPS", "1u")]);
    for &ty in &types {
        let n = native.matches(unit_instruction(ty)).count();
        eprintln!("matrix units on {}: {ty:?}'s product, {} ({n} in the program)", crate::device::backend(), unit_instruction(ty));
        assert!(n > 0, "{} {ty:?}: the compiled kernels hold {}", crate::device::backend(), unit_instruction(ty));
    }
    let forced = code(&[("UKL_MMA_EMULATED_GROUPS", "1u"), ("UKL_MMA_FORCE_EMULATED", "1")]);
    assert!(!forced.contains(unit_marker()), "{}: the forced emulation takes no unit's instruction", crate::device::backend());
}

