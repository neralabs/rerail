//! A region that grows without moving (decision 0173 §3): at most 32 extents, each twice the one before, so an item,
//! once placed, keeps its address while the region grows. An index is found in its extent with one count of leading
//! zeros. A built image is laid out as one extent a region (`Writer`); this form is for regions growing in memory.

pub struct Extents<T> {
    /// The first extent holds `1 << shift` items.
    shift: u32,
    ext: Vec<Vec<T>>,
    len: usize,
}

pub const MAX_EXTENTS: usize = 32;

impl<T: Copy> Extents<T> {
    pub fn new(shift: u32) -> Extents<T> {
        Extents { shift, ext: Vec::new(), len: 0 }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn grow(&mut self) {
        let k = self.ext.len();
        assert!(k < MAX_EXTENTS, "a region of more than {MAX_EXTENTS} extents");
        self.ext.push(Vec::with_capacity(1 << (self.shift as usize + k)));
    }
    pub fn push(&mut self, x: T) {
        if self.ext.last().is_none_or(|e| e.len() == e.capacity()) {
            self.grow();
        }
        self.ext.last_mut().expect("an extent").push(x);
        self.len += 1;
    }
    /// Appends `xs`, filling the last extent, then new ones: never a copy of what is held.
    pub fn extend(&mut self, mut xs: &[T]) {
        while !xs.is_empty() {
            if self.ext.last().is_none_or(|e| e.len() == e.capacity()) {
                self.grow();
            }
            let last = self.ext.last_mut().expect("an extent");
            let n = (last.capacity() - last.len()).min(xs.len());
            last.extend_from_slice(&xs[..n]);
            xs = &xs[n..];
            self.len += n;
        }
    }
    /// Extent and offset of index `i`: extent k holds [(2^k − 1)·first, (2^(k+1) − 1)·first).
    #[inline]
    fn place(&self, i: usize) -> (usize, usize) {
        let q = (i >> self.shift) + 1;
        let k = (usize::BITS - 1 - q.leading_zeros()) as usize;
        (k, i - (((1usize << k) - 1) << self.shift))
    }
    #[inline]
    pub fn get(&self, i: usize) -> T {
        assert!(i < self.len, "index {i} of {}", self.len);
        let (k, at) = self.place(i);
        self.ext[k][at]
    }
    pub fn set(&mut self, i: usize, x: T) {
        assert!(i < self.len, "index {i} of {}", self.len);
        let (k, at) = self.place(i);
        self.ext[k][at] = x;
    }
    /// The extents in order: a reader walks these, not indexes.
    pub fn extents(&self) -> impl Iterator<Item = &[T]> {
        self.ext.iter().map(Vec::as_slice)
    }
    pub fn extent_count(&self) -> usize {
        self.ext.len()
    }
    /// The address of item `i`: stable for as long as the region lives.
    pub fn address(&self, i: usize) -> *const T {
        let (k, at) = self.place(i);
        &self.ext[k][at]
    }
}
