//! `ukl/matrix.h`'s tests (decision 0617, requirements R1-R5 and R9), on the process's device: each 64×64 block a
//! kernel's owners compute through fragments equals the host's product, exactly where the arithmetic is exact and
//! within the bound elsewhere, in every layout, from device and threadgroup memory, by level 1 and level 2, with
//! owners of one SIMD group or several. Without a device they pass, saying so.

use crate::device::{Buffer, ComputeQueue};
use crate::kernels::{alloc, buffer_of, launch, program_with, read, Program};
use crate::library_tests::{setup, Rng};
use crate::Pod;

const TESTS: &str = include_str!("matrix_tests.uglykl");

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
}

impl Ty {
    fn kernel(self) -> &'static str {
        match self {
            Ty::F16 => "t_mma_f16",
            Ty::Bf16 => "t_mma_bf16",
            Ty::I8 => "t_mma_i8",
        }
    }
    /// An element's value: an f16's or a bf16's bits, an i8.
    fn value(self, x: i32) -> f64 {
        match self {
            Ty::F16 => f16_to_f64(x as u16),
            Ty::Bf16 => f32::from_bits((x as u32) << 16) as f64,
            Ty::I8 => x as f64,
        }
    }
    /// The input type's precision in bits (R3's bound).
    fn precision(self) -> i32 {
        match self {
            Ty::F16 => 11,
            Ty::Bf16 => 8,
            Ty::I8 => 0,
        }
    }
    /// The element of a small integer `v` (|v| <= 64): exact in every type.
    fn small(self, v: i32) -> i32 {
        match self {
            Ty::F16 => f16_of_small(v) as i32,
            Ty::Bf16 => ((v as f32).to_bits() >> 16) as i32,
            Ty::I8 => v,
        }
    }
    /// A random element: a float of moderate size (f16 2^-7..2^8, bf16 2^-30..2^30), or any i8.
    fn random(self, r: &mut Rng) -> i32 {
        match self {
            Ty::F16 => ((r.below(2) as u32) << 15 | ((r.below(16) as u32 + 8) << 10) | r.below(1024) as u32) as i32,
            Ty::Bf16 => ((r.below(2) as u32) << 15 | ((r.below(61) as u32 + 97) << 7) | r.below(128) as u32) as i32,
            Ty::I8 => r.below(256) as i32 - 128,
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

/// R1's constants: per type M, N, K, groups, native; then the SIMD width, an owner's lanes, the block.
fn consts(p: &Program) -> Vec<u32> {
    let Some((_, mut q)) = setup() else { return vec![0; 18] };
    let out = alloc::<u32>(18).unwrap();
    launch(&q, p.function("t_mma_consts").unwrap()).buffer("out", &out).run(&[1], &[1]).unwrap();
    q.flush().await_().unwrap();
    read(&out, 18)
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
                if ty == Ty::I8 { v as u32 } else { (v as f32).to_bits() }
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
                let c0 = if ty == Ty::I8 { self.c[r * 64 + c] as i32 as f64 } else { f32::from_bits(self.c[r * 64 + c]) as f64 };
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

fn elements(ty: Ty, v: &[i32]) -> Buffer {
    match ty {
        Ty::I8 => buffer_of(&v.iter().map(|&x| x as i8).collect::<Vec<_>>()).unwrap(),
        _ => buffer_of(&v.iter().map(|&x| x as u16).collect::<Vec<_>>()).unwrap(),
    }
}

/// One block on the device: C's bits, logically row-major.
#[allow(clippy::too_many_arguments)]
fn block(p: &Program, q: &mut ComputeQueue, ty: Ty, inp: &Inputs, layouts: [u32; 3], tile: bool, local: bool, threads: u32) -> Vec<u32> {
    let k = inp.k as usize;
    let [la, lb, lc] = layouts;
    let a = elements(ty, &lay(&inp.a, 64, k, la));
    let b = elements(ty, &lay(&inp.b, k, 64, lb));
    let c = buffer_of(&lay(&inp.c, 64, 64, lc)).unwrap();
    let args = MmaArgs { k: inp.k, la, lb, lc, tile: tile as u32, shared_mem: local as u32 };
    launch(q, p.function(ty.kernel()).unwrap()).value("args", &args).buffer("A", &a).buffer("B", &b).buffer("Cm", &c).run(&[1], &[threads as u64]).unwrap();
    q.flush().await_().unwrap();
    let got: Vec<u32> = read(&c, 64 * 64);
    // Back to row-major: a column-major C transposed.
    lay(&got, 64, 64, lc)
}

fn value(ty: Ty, bits: u32) -> f64 {
    if ty == Ty::I8 { bits as i32 as f64 } else { f32::from_bits(bits) as f64 }
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
    eprintln!("matrix units on {}: f16 {:?}, bf16 {:?}, i8 {:?} (M, N, K, groups, native); SIMD width {}, an owner {} lanes", crate::device::backend(), &c[0..5], &c[5..10], &c[10..15], c[15], lanes);
    for t in 0..3 {
        let s = &c[t * 5..t * 5 + 5];
        assert!(s[0..3].iter().all(|&d| d > 0 && 64 % d == 0), "type {t}'s shape {:?} divides 64", &s[0..3]);
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
    for ty in [Ty::F16, Ty::Bf16, Ty::I8] {
        let big = Inputs::new(ty, 128, &mut rng, true);
        assert_exact(ty, &big, &block(p, &mut q, ty, &big, [ROW; 3], false, false, 4 * lanes), "K 128, level 1");
        assert_exact(ty, &big, &block(p, &mut q, ty, &big, [ROW; 3], true, false, 4 * lanes), "K 128, level 2");
        let inp = Inputs::new(ty, 32, &mut rng, true);
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
    for ty in [Ty::F16, Ty::Bf16, Ty::I8] {
        let k = 128u32;
        let inp = Inputs::new(ty, k, &mut rng, false);
        let got = block(p, &mut q, ty, &inp, [ROW, COL, ROW], false, false, 4 * lanes);
        if ty == Ty::I8 {
            assert_exact(ty, &inp, &got, "random bytes");
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
    let inputs: Vec<(Ty, Inputs)> = [Ty::F16, Ty::I8].into_iter().map(|ty| (ty, Inputs::new(ty, 64, &mut rng, true))).collect();
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
