//! Step 1 of *Flat Views and Lifecycles*: every offset form reaches its target and back; a target past the reach,
//! not at a whole unit, or outside the region is refused; a region moved by `memcpy` reads the same; the kernels'
//! declarations are the layouts.

extern crate std;

use super::offset::{Aligned, B16, B32, F32, Off, R16, R64};
use super::rel::{BitSpan, Rel, RelSpan};
use super::{Refusal, Region};
use crate::kernel::{Decls, KernelField};
use core::sync::atomic::AtomicU16;
use std::string::String;
use std::vec::Vec;

#[repr(C, align(64))]
struct Buf([u8; 1 << 16]);

fn buf() -> std::boxed::Box<Buf> {
    std::boxed::Box::new(Buf([0; 1 << 16]))
}

/// A `T` at byte `at` of the region (aligned by the caller).
fn at<'a, T: crate::Pod>(r: &'a Region<'_>, off: usize) -> &'a T {
    r.view::<T>(off).expect("a test's view")
}

fn put<T: Copy>(r: &Region<'_>, off: usize, v: T) {
    unsafe { r.ptr(off).cast::<T>().write_unaligned(v) }
}

#[test]
fn every_offset_form_reaches_its_target_and_back() {
    let mut b = buf();
    let r = Region::of(&mut b.0);

    // R32: ahead and behind, 8-byte targets.
    put(&r, 1024, 0x1122_3344_5566_7788u64);
    put(&r, 8, 0xdead_beefu64);
    let f: &Rel<u64> = at(&r, 512);
    f.set(&r, r.ptr(1024).cast()).unwrap();
    assert_eq!(f.units(), (1024 - 512) / 8);
    assert_eq!(f.get().copied(), Some(0x1122_3344_5566_7788));
    f.set(&r, r.ptr(8).cast()).unwrap();
    assert!(f.units() < 0);
    assert_eq!(f.get().copied(), Some(0xdead_beef));

    // An aligned offset in a field that is not aligned to its target: counted from the field rounded down.
    let g: &Rel<u64> = at(&r, 516);
    g.set(&r, r.ptr(1024).cast()).unwrap();
    assert_eq!(g.get().copied(), Some(0x1122_3344_5566_7788));

    // R16, R64, F32.
    let h: &Rel<u64, R16> = at(&r, 600);
    h.set(&r, r.ptr(1024).cast()).unwrap();
    assert_eq!(h.get().copied(), Some(0x1122_3344_5566_7788));
    let k: &Rel<u64, R64> = at(&r, 608);
    k.set(&r, r.ptr(8).cast()).unwrap();
    assert_eq!(k.get().copied(), Some(0xdead_beef));
    let m: &Rel<u64, F32> = at(&r, 616);
    m.set(&r, r.ptr(1024).cast()).unwrap();
    assert_eq!(m.get().copied(), Some(0x1122_3344_5566_7788));

    // B32 and B16: a u32 at an odd byte, read by value.
    put(&r, 2001, 0xa1b2_c3d4u32);
    let n: &Rel<u32, B32> = at(&r, 700);
    n.set(&r, r.ptr(2001).cast()).unwrap();
    assert_eq!(n.units(), 2001 - 700);
    assert_eq!(n.read(), Some(0xa1b2_c3d4));
    let o: &Rel<u32, B16> = at(&r, 704);
    o.set(&r, r.ptr(2001).cast()).unwrap();
    assert_eq!(o.read(), Some(0xa1b2_c3d4));
    unsafe { o.write(7) };
    assert_eq!(n.read(), Some(7));

    // Bit32: bits at any bit.
    let s: &BitSpan = at(&r, 800);
    let first = (r.addr() as u64 + 3000) * 8 + 5;
    s.set_bits(&r, first, 200).unwrap();
    unsafe { s.put(0, 13, 0x1abc) };
    assert_eq!(s.get(0, 13), 0x1abc);
    assert_eq!(s.target_bits(), Some(first));

    // None, and cleared.
    let z: &Rel<u64> = at(&r, 900);
    assert!(z.is_none() && z.get().is_none());
    f.clear();
    assert!(f.get().is_none());
}

#[test]
fn a_target_out_of_reach_is_refused() {
    let mut b = buf();
    let r = Region::of(&mut b.0);
    // u8 targets: R16 counts bytes at alignment 1, so it reaches ±32 KiB.
    let f: &Rel<u8, R16> = at(&r, 0x100);
    assert_eq!(f.set(&r, r.ptr(0x100 + 40_000)), Err(Refusal::Reach));
    assert!(f.is_none(), "a refused set leaves the reference as it was");
    f.set(&r, r.ptr(0x100 + 30_000)).unwrap();
    let g: &Rel<u8, B16> = at(&r, 0x200);
    assert_eq!(g.set(&r, r.ptr(0x200 + 40_000)), Err(Refusal::Reach));
    // F32 reaches forward only.
    let h: &Rel<u64, F32> = at(&r, 0x400);
    assert_eq!(h.set(&r, r.ptr(0x100).cast()), Err(Refusal::Reach));
    // A span longer than its length's word holds.
    let s: &RelSpan<u8, Off<core::sync::atomic::AtomicI16, Aligned>, AtomicU16> = at(&r, 0x500);
    assert_eq!(s.set(&r, r.ptr(0x600), 70_000), Err(Refusal::Reach));
}

#[test]
fn a_target_not_at_a_whole_unit_is_refused() {
    let mut b = buf();
    let r = Region::of(&mut b.0);
    let f: &Rel<u64> = at(&r, 64);
    assert_eq!(f.set(&r, r.ptr(1001).cast()), Err(Refusal::Misaligned));
    // A byte offset to a bit that is not a byte's.
    let g: &Rel<u32, B32> = at(&r, 128);
    assert_eq!(g.set_bits(&r, (r.addr() as u64 + 2000) * 8 + 3), Err(Refusal::Misaligned));
    // A target overlapping its own field is none's place, never a target.
    let h: &Rel<u8, B32> = at(&r, 256);
    assert_eq!(h.set(&r, r.ptr(256)), Err(Refusal::Outside));
}

#[test]
fn a_target_outside_the_region_is_refused() {
    let mut b = buf();
    let whole = Region::of(&mut b.0);
    let half = unsafe { Region::from_raw(whole.ptr(0), 1 << 15) };
    let f: &Rel<u64> = at(&half, 64);
    assert_eq!(f.set(&half, whole.ptr(40_000).cast()), Err(Refusal::Outside));
    // Its field outside.
    let g: &Rel<u64> = at(&whole, 40_000);
    assert_eq!(g.set(&half, whole.ptr(64).cast()), Err(Refusal::Outside));
    // A span that runs past the region's end.
    let s: &RelSpan<u64> = at(&half, 128);
    assert_eq!(s.set(&half, whole.ptr((1 << 15) - 64).cast(), 9), Err(Refusal::Outside));
    s.set(&half, whole.ptr((1 << 15) - 64).cast(), 8).unwrap();
    // A view past the region.
    assert_eq!(half.view::<u64>((1 << 15) - 4).err(), Some(Refusal::Outside));
    assert_eq!(half.view::<u64>(4).err(), Some(Refusal::Misaligned));
}

/// A structure with each kind of reference, as a block holds one.
#[repr(C)]
struct Holder {
    one: Rel<u64>,
    span: RelSpan<u32>,
    bytes: RelSpan<u32, B32>,
    bits: BitSpan,
    small: Rel<u16, R16>,
    pad: u16,
}
unsafe impl crate::Pod for Holder {}

fn fill(r: &Region<'_>, base: usize) {
    let h: &Holder = at(r, base);
    put(r, base + 256, 42u64);
    h.one.set(r, r.ptr(base + 256).cast()).unwrap();
    let words: &mut [u32] = unsafe { core::slice::from_raw_parts_mut(r.ptr(base + 512).cast(), 16) };
    for (i, w) in words.iter_mut().enumerate() {
        *w = i as u32 * 3;
    }
    h.span.set(r, r.ptr(base + 512).cast(), 16).unwrap();
    h.bytes.set(r, r.ptr(base + 1001).cast(), 4).unwrap();
    for i in 0..4 {
        unsafe { h.bytes.put(i, 1000 + i as u32) };
    }
    h.bits.set_bits(r, (r.addr() + base + 1100) as u64 * 8 + 3, 64 * 7).unwrap();
    for i in 0..64 {
        unsafe { h.bits.put(i * 7, 7, (i * 5) % 128) };
    }
    put(r, base + 1300, 0xbeefu16);
    h.small.set(r, r.ptr(base + 1300).cast()).unwrap();
}

fn check(r: &Region<'_>, base: usize) {
    let h: &Holder = at(r, base);
    assert_eq!(h.one.get().copied(), Some(42));
    assert_eq!(h.span.as_slice(), &(0..16).map(|i| i * 3).collect::<Vec<u32>>()[..]);
    assert_eq!((0..4).map(|i| h.bytes.at(i)).collect::<Vec<_>>(), [1000, 1001, 1002, 1003]);
    for i in 0..64u64 {
        assert_eq!(h.bits.packed(i, 7), (i * 5) % 128);
    }
    assert_eq!(h.small.get().copied(), Some(0xbeef));
}

#[test]
fn a_region_moved_by_memcpy_reads_the_same() {
    let (mut a, mut b) = (buf(), buf());
    let ra = Region::of(&mut a.0);
    fill(&ra, 4096);
    check(&ra, 4096);
    let rb = Region::of(&mut b.0);
    // Another place, another base: at a different offset in a different buffer, aligned as the structure needs.
    unsafe { core::ptr::copy_nonoverlapping(ra.ptr(4096), rb.ptr(8192 + 64), 2048) };
    check(&rb, 8192 + 64);
}

#[test]
fn spans_read_and_write_in_place() {
    let mut b = buf();
    let r = Region::of(&mut b.0);
    let s: &RelSpan<u32> = at(&r, 0);
    assert!(s.is_empty() && s.as_slice().is_empty());
    s.set(&r, r.ptr(64).cast(), 10).unwrap();
    unsafe { s.as_mut_slice() }.copy_from_slice(&[9, 8, 7, 6, 5, 4, 3, 2, 1, 0]);
    assert_eq!(s.as_slice()[3], 6);
    // A bit span against a model, at every width and from an odd bit.
    let bits: &BitSpan = at(&r, 16);
    bits.set_bits(&r, (r.addr() as u64 + 1024) * 8 + 5, 4000).unwrap();
    let mut model = [0u64; 70];
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    for w in [1u32, 3, 8, 13, 31, 32, 33, 57] {
        let n = (4000 / w as u64).min(70);
        for i in 0..n {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            model[i as usize] = x & ((1u64 << w) - 1);
            unsafe { bits.put(i * w as u64, w, model[i as usize]) };
        }
        for i in 0..n {
            assert_eq!(bits.packed(i, w), model[i as usize], "width {w}, entry {i}");
        }
    }
}

#[test]
#[should_panic(expected = "bits")]
fn a_read_past_a_bit_span_panics() {
    let mut b = buf();
    let r = Region::of(&mut b.0);
    let bits: &BitSpan = at(&r, 0);
    bits.set_bits(&r, (r.addr() as u64 + 64) * 8, 100).unwrap();
    bits.get(90, 11);
}

crate::structure! {
    /// A structure holding each reference form, declared for the kernels.
    pub struct Refs {
        pub wide: RelSpan<u64, R64, core::sync::atomic::AtomicU64>,
        pub one: Rel<u64>,
        pub span: RelSpan<u32>,
        pub bits: BitSpan,
        pub small: Rel<u16, R16>,
        pub pad: u16,
    }
}

struct Text(String, Vec<&'static str>);

impl Decls for Text {
    fn declare(&mut self, name: &'static str, text: &dyn Fn(&mut dyn core::fmt::Write) -> core::fmt::Result) -> core::fmt::Result {
        if !self.1.contains(&name) {
            self.1.push(name);
            text(&mut self.0)?;
        }
        Ok(())
    }
}

#[test]
fn the_kernels_declarations_are_the_layouts() {
    let mut t = Text(String::new(), Vec::new());
    Refs::deps(&mut t).unwrap();
    let s = &t.0;
    assert!(s.contains("typedef struct RelSpan_i32_u32 {\n    int32_t at;\n    uint32_t len;\n} RelSpan_i32_u32;"), "{s}");
    assert!(s.contains("UKL_STATIC_ASSERT(sizeof(RelSpan_i32_u32) == 8"), "{s}");
    assert!(s.contains("typedef struct BitSpan_i32_u32 {\n    int32_t at;\n    uint32_t bits;\n}"), "{s}");
    assert!(s.contains("typedef struct RelSpan_i64_u64 {\n    int64_t at;\n    uint64_t len;\n}"), "{s}");
    assert!(s.contains("    RelSpan_i64_u64 wide;\n    int32_t one;\n    RelSpan_i32_u32 span;\n    BitSpan_i32_u32 bits;\n    int16_t small;\n    uint16_t pad;\n"), "{s}");
    assert!(s.contains(&std::format!("UKL_STATIC_ASSERT(sizeof(Refs) == {}", size_of::<Refs>())), "{s}");
    assert_eq!(size_of::<Rel<u64>>(), 4);
    assert_eq!(size_of::<RelSpan<u32>>(), 8);
    assert_eq!(size_of::<Rel<u8, R16>>(), 2);
    // Each span declared once, before the structure that holds it.
    assert!(s.find("RelSpan_i32_u32 {").unwrap() < s.find("typedef struct Refs").unwrap());
}
