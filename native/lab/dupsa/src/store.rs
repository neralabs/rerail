//! Files read in place: a file is mapped, and an array in it is a slice of the mapping, not a copy.
//!
//! An array is written as its length (u64) then its values, little-endian, starting at a multiple of 8 bytes, so that
//! a later run can take it as a slice of the mapped file (`view`) without reading it.

use std::ops::Deref;

/// A file's bytes, mapped. The mapping is kept for the whole run (the spike's index and tapes live as long as it).
pub fn map(path: &std::path::Path) -> Result<&'static [u8], String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if f.metadata().map_err(|e| format!("{}: {e}", path.display()))?.len() == 0 {
        return Ok(&[]);
    }
    // SAFETY: the file is the run's own input, not changed while it runs.
    let m = unsafe { memmap2::Mmap::map(&f) }.map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(&Box::leak(Box::new(m))[..])
}

/// Values this module writes and views: plain numbers.
pub trait Plain: Copy + 'static {
    fn put(self, out: &mut Vec<u8>);
}
impl Plain for u8 {
    fn put(self, out: &mut Vec<u8>) {
        out.push(self);
    }
}
impl Plain for u32 {
    fn put(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}
impl Plain for u64 {
    fn put(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}

fn align(out: &mut Vec<u8>) {
    while out.len() % 8 != 0 {
        out.push(0);
    }
}

pub fn put<T: Plain>(out: &mut Vec<u8>, v: &[T]) {
    align(out);
    out.extend_from_slice(&(v.len() as u64).to_le_bytes());
    for &x in v {
        x.put(out);
    }
}

/// The array `put` wrote at `*at`, as a slice of the mapped bytes.
pub fn view<T: Plain>(b: &'static [u8], at: &mut usize) -> &'static [T] {
    assert!(cfg!(target_endian = "little"), "the index is little-endian");
    *at = at.next_multiple_of(8);
    let n = u64::from_le_bytes(b[*at..*at + 8].try_into().unwrap()) as usize;
    *at += 8;
    let size = n * std::mem::size_of::<T>();
    let bytes = &b[*at..*at + size];
    *at += size;
    if n == 0 {
        return &[];
    }
    assert!(std::mem::align_of::<T>() <= 8);
    assert_eq!(bytes.as_ptr() as usize % std::mem::align_of::<T>(), 0, "an array not aligned in the index");
    // SAFETY: `T` is a plain number, the bytes are in bounds and aligned, and the mapping lives for the run.
    unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<T>(), n) }
}

/// An array owned, or read in place from a mapped file.
pub enum Arr<T: 'static> {
    Owned(Vec<T>),
    Mapped(&'static [T]),
}

impl<T> Deref for Arr<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        match self {
            Arr::Owned(v) => v,
            Arr::Mapped(s) => s,
        }
    }
}

impl<T> From<Vec<T>> for Arr<T> {
    fn from(v: Vec<T>) -> Self {
        Arr::Owned(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrays_read_back_in_place() {
        let mut out = vec![7u8];
        put(&mut out, &[1u32, 2, 3]);
        put(&mut out, b"abc".as_slice());
        put(&mut out, &[u64::MAX, 5]);
        put::<u32>(&mut out, &[]);
        // Leaked at an 8-byte-aligned address, as a mapping is.
        let words: &'static mut [u64] = Box::leak(vec![0u64; out.len().div_ceil(8)].into_boxed_slice());
        let b: &'static mut [u8] = unsafe { std::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), out.len()) };
        b.copy_from_slice(&out);
        let mut at = 1;
        assert_eq!(view::<u32>(b, &mut at), &[1, 2, 3]);
        assert_eq!(view::<u8>(b, &mut at), b"abc");
        assert_eq!(view::<u64>(b, &mut at), &[u64::MAX, 5]);
        assert!(view::<u32>(b, &mut at).is_empty());
    }
}
