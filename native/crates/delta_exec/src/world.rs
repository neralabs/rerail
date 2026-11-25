//! The world a `load` is answered from (Bootstrap from one tape, §7: the `load` answerer is a World answerer: path →
//! bytes → the rr parser → a document). The answerer itself, as `Δanswer` over the `load` bistate, comes at M6 of
//! the rebuild plan.

/// What answers a `load`: a path's stamp (a file's change mark), and its bytes.
pub trait World {
    fn stamp(&self, path: &str) -> Option<u64>;
    fn read(&self, path: &str) -> Option<String>;
}

/// A world in memory: files by path. A file's stamp is its contents' hash; its reads are counted.
pub struct Files {
    pub files: Vec<(String, String)>,
    pub reads: std::cell::Cell<usize>,
}

impl Files {
    pub fn new(files: Vec<(String, String)>) -> Files {
        Files {
            files,
            reads: std::cell::Cell::new(0),
        }
    }
}

impl World for Files {
    fn stamp(&self, path: &str) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let f = self.files.iter().find(|f| f.0 == path)?;
        let mut h = rustc_hash::FxHasher::default();
        f.1.hash(&mut h);
        Some(h.finish())
    }

    fn read(&self, path: &str) -> Option<String> {
        self.reads.set(self.reads.get() + 1);
        // copy: load (a file's bytes, as a read from disk gives them)
        self.files.iter().find(|f| f.0 == path).map(|f| f.1.clone())
    }
}
