//! Readers for the extractor's files, and the thread CPU clock.
use std::path::Path;

pub struct Csr<T> { pub off: Vec<u64>, pub val: Vec<T> }
impl<T> Csr<T> {
    pub fn row(&self, i: usize) -> &[T] { &self.val[self.off[i] as usize..self.off[i + 1] as usize] }
}

fn u32_at(b: &[u8], i: usize) -> u32 { u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) }
fn u64_at(b: &[u8], i: usize) -> u64 { u64::from_le_bytes(b[i..i + 8].try_into().unwrap()) }

/// A CSR file: u32 n, u64 offsets[n+1], then values of `w` bytes; returns the CSR and the bytes after it.
pub fn csr<T>(b: &[u8], w: usize, get: impl Fn(&[u8]) -> T) -> (Csr<T>, &[u8]) {
    let n = u32_at(b, 0) as usize;
    let off: Vec<u64> = (0..=n).map(|i| u64_at(b, 4 + 8 * i)).collect();
    let start = 4 + 8 * (n + 1);
    let m = off[n] as usize;
    let val = (0..m).map(|i| get(&b[start + w * i..start + w * (i + 1)])).collect();
    (Csr { off, val }, &b[start + w * m..])
}

pub struct Data { pub files: Vec<String>, pub names: Vec<String>, pub doc_names: Csr<u32>, pub occ: Csr<u32>, pub role: Vec<u8>, pub kinds: Csr<u16>, pub edges: Vec<(u32, u32)> }

pub fn load(dir: &Path) -> Data {
    let rd = |f: &str| std::fs::read(dir.join(f)).unwrap();
    let lines = |f: &str| String::from_utf8(rd(f)).unwrap().lines().map(str::to_owned).collect::<Vec<_>>();
    let u32v = |x: &[u8]| u32::from_le_bytes(x.try_into().unwrap());
    let dn = rd("doc_names.bin");
    let (doc_names, _) = csr(&dn, 4, u32v);
    let ob = rd("occ.bin");
    let (occ, rest) = csr(&ob, 4, u32v);
    let role = rest.to_vec();
    let kb = rd("kinds.bin");
    let (kinds, _) = csr(&kb, 2, |x| u16::from_le_bytes(x.try_into().unwrap()));
    let ib = rd("imports.bin");
    let m = u32_at(&ib, 0) as usize;
    let edges = (0..m).map(|i| (u32_at(&ib, 4 + 8 * i), u32_at(&ib, 8 + 8 * i))).collect();
    Data { files: lines("files.txt"), names: lines("names.txt"), doc_names, occ, role, kinds, edges }
}

/// This thread's CPU time in seconds.
pub fn cpu() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}

/// The minimum CPU time of `reps` runs of `f`, and its last result.
pub fn best<R>(reps: usize, mut f: impl FnMut() -> R) -> (f64, R) {
    let mut best = f64::MAX;
    let mut out = None;
    for _ in 0..reps {
        let t = cpu();
        let r = std::hint::black_box(f());
        best = best.min(cpu() - t);
        out = Some(r);
    }
    (best, out.unwrap())
}

pub fn load_average() -> String {
    let mut l = [0f64; 3];
    unsafe { libc::getloadavg(l.as_mut_ptr(), 3) };
    format!("{:.0}", l[0])
}
