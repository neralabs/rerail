//! The operations on values, once: a value is its machine type's bits in a `u64` (an integer's zero- or
//! sign-extended past its width, a float's bits), as the kernel tape's nodes give them (`crate::k`).

use crate::k::{M, Op};

/// `x` as type `m` holds it: an integer's bits past its width extended by its sign, a `bool` 0 or 1.
#[inline(always)]
pub fn norm(m: M, x: u64) -> u64 {
    match m {
        M::B => u64::from(x != 0),
        M::I8 => x as i8 as i64 as u64,
        M::U8 => x as u8 as u64,
        M::I16 => x as i16 as i64 as u64,
        M::U16 => x as u16 as u64,
        M::I32 => x as i32 as i64 as u64,
        M::U32 | M::F32 => x as u32 as u64,
        _ => x,
    }
}

#[inline(always)]
fn f32_of(x: u64) -> f32 {
    f32::from_bits(x as u32)
}

#[inline(always)]
fn f64_of(x: u64) -> f64 {
    f64::from_bits(x)
}

/// `a op b` on two values of `m`: an `m`, or a `bool` for a comparison. `Err` for a division by zero.
#[inline(always)]
pub fn bin(op: Op, m: M, a: u64, b: u64) -> Result<u64, &'static str> {
    if m.float() {
        let r = if m == M::F32 {
            let (x, y) = (f32_of(a), f32_of(b));
            match op {
                Op::Add => (x + y).to_bits() as u64,
                Op::Sub => (x - y).to_bits() as u64,
                Op::Mul => (x * y).to_bits() as u64,
                Op::Div => (x / y).to_bits() as u64,
                Op::Eq => u64::from(x == y),
                Op::Ne => u64::from(x != y),
                Op::Lt => u64::from(x < y),
                Op::Le => u64::from(x <= y),
                Op::Gt => u64::from(x > y),
                Op::Ge => u64::from(x >= y),
                _ => return Err("an operation floats do not have"),
            }
        } else {
            let (x, y) = (f64_of(a), f64_of(b));
            match op {
                Op::Add => (x + y).to_bits(),
                Op::Sub => (x - y).to_bits(),
                Op::Mul => (x * y).to_bits(),
                Op::Div => (x / y).to_bits(),
                Op::Eq => u64::from(x == y),
                Op::Ne => u64::from(x != y),
                Op::Lt => u64::from(x < y),
                Op::Le => u64::from(x <= y),
                Op::Gt => u64::from(x > y),
                Op::Ge => u64::from(x >= y),
                _ => return Err("an operation floats do not have"),
            }
        };
        return Ok(r);
    }
    let bits = m.bits().max(1);
    let signed = m.signed();
    let (sa, sb) = (a as i64, b as i64);
    let r = match op {
        Op::Add => a.wrapping_add(b),
        Op::Sub => a.wrapping_sub(b),
        Op::Mul => a.wrapping_mul(b),
        Op::Div => {
            if norm(m, b) == 0 {
                return Err("a division by zero");
            }
            if signed { sa.wrapping_div(sb) as u64 } else { norm(m, a) / norm(m, b) }
        }
        Op::Rem => {
            if norm(m, b) == 0 {
                return Err("a remainder by zero");
            }
            if signed { sa.wrapping_rem(sb) as u64 } else { norm(m, a) % norm(m, b) }
        }
        // A shift by the type's width or more is undefined in C: the hardware's (the count masked), as GPUs do.
        Op::Shl => a.wrapping_shl((b as u32) & (bits - 1)),
        Op::Shr => {
            let s = (b as u32) & (bits - 1);
            if signed { (sa >> s) as u64 } else { norm(m, a) >> s }
        }
        Op::And => a & b,
        Op::Or => a | b,
        Op::Xor => a ^ b,
        Op::Eq => return Ok(u64::from(norm(m, a) == norm(m, b))),
        Op::Ne => return Ok(u64::from(norm(m, a) != norm(m, b))),
        Op::Lt | Op::Le | Op::Gt | Op::Ge => {
            let (x, y) = (norm(m, a), norm(m, b));
            let o = if signed { (x as i64).cmp(&(y as i64)) } else { x.cmp(&y) };
            return Ok(u64::from(match op {
                Op::Lt => o.is_lt(),
                Op::Le => o.is_le(),
                Op::Gt => o.is_gt(),
                _ => o.is_ge(),
            }));
        }
        _ => return Err("not a binary operation"),
    };
    Ok(norm(m, r))
}

/// `op a` on a value of `m`.
#[inline(always)]
pub fn un(op: Op, m: M, a: u64) -> u64 {
    match op {
        Op::Neg if m == M::F32 => (-f32_of(a)).to_bits() as u64,
        Op::Neg if m == M::F64 => (-f64_of(a)).to_bits(),
        Op::Neg => norm(m, a.wrapping_neg()),
        Op::Not => norm(m, !a),
        Op::LNot => u64::from(truth(m, a) == 0),
        _ => a,
    }
}

/// Whether `a` of `m` is non-zero (a condition's test), as 0 or 1.
#[inline(always)]
pub fn truth(m: M, a: u64) -> u64 {
    match m {
        M::F32 => u64::from(f32_of(a) != 0.0),
        M::F64 => u64::from(f64_of(a) != 0.0),
        _ => u64::from(norm(m, a) != 0),
    }
}

/// `a` of `from` converted to `to` (C's conversions: an integer truncated or extended, a float rounded toward
/// zero to an integer, a pointer as its 64 bits).
#[inline(always)]
pub fn cast(from: M, to: M, a: u64) -> u64 {
    if from == to {
        return a;
    }
    if to == M::B {
        return truth(from, a);
    }
    match (from.float(), to.float()) {
        (false, false) => norm(to, norm(from, a)),
        (true, true) => {
            if to == M::F32 {
                (f64_of(a) as f32).to_bits() as u64
            } else {
                f64::from(f32_of(a)).to_bits()
            }
        }
        (false, true) => {
            let v = norm(from, a);
            let f = if from.signed() { v as i64 as f64 } else { v as f64 };
            if to == M::F32 { (f as f32).to_bits() as u64 } else { f.to_bits() }
        }
        (true, false) => {
            let f = if from == M::F32 { f64::from(f32_of(a)) } else { f64_of(a) };
            norm(to, if to.signed() { f as i64 as u64 } else { f as u64 })
        }
    }
}

/// The value of `m` at `p`.
///
/// # Safety
///
/// `p` is readable for `m`'s bytes.
#[inline(always)]
pub unsafe fn load(m: M, p: u64) -> u64 {
    let p = p as usize as *const u8;
    // SAFETY: the caller's.
    unsafe {
        match m.bytes() {
            1 => norm(m, u64::from(p.read())),
            2 => norm(m, u64::from(p.cast::<u16>().read_unaligned())),
            4 => norm(m, u64::from(p.cast::<u32>().read_unaligned())),
            _ => p.cast::<u64>().read_unaligned(),
        }
    }
}

/// `v` of `m` stored at `p`.
///
/// # Safety
///
/// `p` is writable for `m`'s bytes.
#[inline(always)]
pub unsafe fn store(m: M, p: u64, v: u64) {
    let p = p as usize as *mut u8;
    // SAFETY: the caller's.
    unsafe {
        match m.bytes() {
            1 => p.write(norm(m, v) as u8),
            2 => p.cast::<u16>().write_unaligned(v as u16),
            4 => p.cast::<u32>().write_unaligned(v as u32),
            _ => p.cast::<u64>().write_unaligned(v),
        }
    }
}
