//! A delta executor that loads itself from one tape (decision 0686), built beside the engine.
//!
//! A program is K-form: rows of eight relations (`kform`). The executor runs it with the four delta operators over
//! relations in blocks and views over tapes (`run`), and knows nothing else of rules. The only Rust that reads a rule
//! is L1, the K0 pass (`k0`): it reads `bootstrap-l0.rerail`'s tape into P2, whose rules read any K0 file's tape into
//! K-form. Each level's output is the next level's program and facts:
//!
//! - L1: the K0 pass over `l0`: P2;
//! - L2: P2 over `l0`'s tape, its loads answered: the K-form of `l0` and the kernel files it loads;
//! - L3: that program over the user's files: their K-form, and the first lowering's (`k1_`);
//! - L4: the lowered program: the user's rules run.
//!
//! The main run is the loop's next turn.

pub mod db;
pub mod k0;
pub mod kform;
pub mod run;
pub mod sym;
pub mod tape;

#[cfg(test)]
mod tests;

use db::{Db, Text};
use kform::{Refused, Words};
use run::{Exec, World};
use std::sync::Arc;
use sym::Symbols;

/// `bootstrap-l0.rerail`: the reader of rules (P2) and the kernel's loads.
pub const L0: &str = include_str!("../rerail/bootstrap-l0.rerail");

/// The kernel files `l0` loads, by path, for a world with no disk.
pub const KERNEL: [(&str, &str); 3] = [
    ("kernel/k1-binding-1.rerail", include_str!("../rerail/kernel/k1-binding-1.rerail")),
    ("kernel/k1-binding-2.rerail", include_str!("../rerail/kernel/k1-binding-2.rerail")),
    ("kernel/k1-binding-3.rerail", include_str!("../rerail/kernel/k1-binding-3.rerail")),
];

/// The kernel's files, and any others given.
pub struct Files(pub Vec<(String, String)>);

impl Files {
    pub fn kernel() -> Files {
        Files(KERNEL.iter().map(|(p, s)| (p.to_string(), s.to_string())).collect())
    }
}

impl World for Files {
    fn read(&self, path: &str) -> Option<String> {
        // copy: load (a file's bytes, as a read from disk gives them)
        self.0.iter().find(|f| f.0 == path).map(|f| f.1.clone())
    }
}

/// The loop's levels, each a database whose changes the next level reads.
pub struct Boot<'s> {
    pub exec: Exec<'s>,
    pub k: Words,
    pub k1: Words,
    /// L1's output: P2.
    pub p2: Db,
    pub l2: Db,
    pub l3: Db,
    pub l4: Db,
}

impl<'s> Boot<'s> {
    pub fn new(sym: &'s Symbols) -> Boot<'s> {
        Boot { exec: Exec::new(sym), k: Words::new(sym), k1: Words::with_prefix(sym, "k1_"), p2: Db::new(), l2: Db::new(), l3: Db::new(), l4: Db::new() }
    }

    /// One turn: `l0` and the user's files at their new texts, each level run across its input's change. Then
    /// `settle` makes this turn's state the next one's start.
    pub fn turn(&mut self, l0: &Arc<Text>, user: &[(&str, Arc<Text>)], world: &dyn World) -> Result<(), Refused> {
        let sym = self.exec.sym;
        // L1: `l0` is module 1 at P2 and document 1 at L2.
        k0::pass(&[(1, &**l0)], sym, &mut self.p2)?;
        self.l2.give(sym.intern("bootstrap-l0.rerail"), l0.clone());
        self.exec.phase(&self.p2, &self.k, &mut self.l2, world)?;
        for (p, t) in user {
            self.l3.give(sym.intern(p), t.clone());
        }
        self.exec.phase(&self.l2, &self.k, &mut self.l3, world)?;
        self.exec.phase(&self.l3, &self.k1, &mut self.l4, world)
    }

    pub fn settle(&mut self) {
        for db in [&mut self.p2, &mut self.l2, &mut self.l3, &mut self.l4] {
            db.settle();
        }
    }
}

/// A text parsed, for callers holding a source.
pub fn text(src: &str) -> Arc<Text> {
    // copy: load (a source's bytes)
    Arc::new(Text::parse(src.to_string()).expect("an rr file that parses"))
}
