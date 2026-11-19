//! A `Program` block: modules nested whole, each its own range. A delta holds the modules an edit replaced (a
//! sparse table: null where unchanged), and names the full program. A roll-up copies every module's range,
//! from the delta where it was replaced and from the base otherwise: `memcpy` only.

use crate::block::{Block, Frozen, kind};
use crate::facts::Range;
use crate::refs::{Ptr, Span};
use crate::structure;

structure! {
    pub struct Rule { pub head: u32, pub nbody: u32, pub body: [u32; 4] }
}

structure! {
    pub struct Module {
        pub id: u32,
        pub nrules: u32,
        pub rules: Span<Rule>,
        pub text: Span<u8>,
    }
}

structure! {
    pub struct ProgramRoot {
        pub nmod: u32,
        pub pad: u32,
        pub modules: Span<Ptr<Module>>,
        pub ranges: Span<Range>,
    }
}

pub struct ModuleIn<'a> {
    pub id: u32,
    pub rules: &'a [[u32; 6]],
    pub text: &'a str,
}

/// A program block: full when `base` is `None`, else a delta whose `modules` are the replaced ones.
pub struct Program {
    pub block: Frozen,
    pub base: Option<Box<Program>>,
}

fn open(nmod: usize, delta: bool) -> Block {
    let b = crate::alloc::host(kind::PROGRAM, 256 << 20);
    let root: *mut ProgramRoot = b.alloc_t(1);
    let mods: *mut Ptr<Module> = b.alloc_t(nmod);
    let ranges: *mut Range = b.alloc_t(nmod);
    unsafe {
        (*root).nmod = nmod as u32;
        Span::set(&raw mut (*root).modules, mods, nmod);
        Span::set(&raw mut (*root).ranges, ranges, nmod);
        if delta {
            b.header_mut().delta = 1;
        }
    }
    b.set_root(root);
    b
}

fn root(b: &Block) -> &ProgramRoot {
    b.root()
}

fn set_module(b: &Block, i: usize, m: *const Module, start: usize, end: usize) {
    let r = root(b);
    unsafe {
        Ptr::set(&r.modules.as_slice()[i], m);
    }
    r.ranges.as_slice()[i].set(start, end);
}

fn write_module(b: &Block, i: usize, m: &ModuleIn) {
    let start = b.mark(64);
    let mp: *mut Module = b.alloc_t(1);
    let rp: *mut Rule = b.alloc_t(m.rules.len().max(1));
    let tp: *mut u8 = b.alloc_t(m.text.len().max(1));
    unsafe {
        for (j, r) in m.rules.iter().enumerate() {
            *rp.add(j) = Rule { head: r[0], nbody: r[1], body: [r[2], r[3], r[4], r[5]] };
        }
        std::ptr::copy_nonoverlapping(m.text.as_ptr(), tp, m.text.len());
        (*mp).id = m.id;
        (*mp).nrules = m.rules.len() as u32;
        Span::set(&raw mut (*mp).rules, rp, m.rules.len());
        Span::set(&raw mut (*mp).text, tp, m.text.len());
    }
    set_module(b, i, mp, start, b.used());
}

impl Program {
    pub fn build(modules: &[ModuleIn]) -> Program {
        let b = open(modules.len(), false);
        for (i, m) in modules.iter().enumerate() {
            write_module(&b, i, m);
        }
        Program { block: b.freeze(), base: None }
    }

    /// An edit: a delta naming this full program, holding the replaced modules.
    pub fn edit(&self, replaced: &[(usize, ModuleIn)]) -> Program {
        assert!(self.base.is_none(), "a delta names a full program");
        let b = open(self.nmod(), true);
        for (i, m) in replaced {
            write_module(&b, *i, m);
        }
        Program { block: b.freeze(), base: Some(Box::new(Program { block: self.block.clone(), base: None })) }
    }

    pub fn nmod(&self) -> usize {
        root(&self.block).nmod as usize
    }

    /// Module `i` as this version has it: the delta's if it replaced it, else the base's.
    pub fn module(&self, i: usize) -> &Module {
        if let Some(m) = root(&self.block).modules.as_slice()[i].get() {
            return m;
        }
        self.base.as_ref().expect("a module missing from a full program").module(i)
    }

    pub fn rollup(&self) -> Program {
        let base = self.base.as_ref().expect("roll up a delta");
        let b = open(self.nmod(), false);
        for i in 0..self.nmod() {
            let (src, r) = if root(&self.block).modules.as_slice()[i].is_null() { (&base.block, i) } else { (&self.block, i) };
            let range = &root(src).ranges.as_slice()[r];
            let (start, end) = range.get();
            let moff = src.offset_of(root(src).modules.as_slice()[r].get().unwrap());
            let dst = b.copy_in(src.bytes(start, end), start);
            set_module(&b, i, b.ptr::<Module>(dst + moff - start), dst, dst + end - start);
        }
        Program { block: b.freeze(), base: None }
    }

    /// A digest of every module's rules and text, as this version reads them.
    pub fn digest(&self) -> u64 {
        let mut h = crate::SEED;
        for i in 0..self.nmod() {
            let m = self.module(i);
            h = crate::mix(h, m.id as i64);
            for r in m.rules.as_slice() {
                for v in [r.head, r.nbody, r.body[0], r.body[1], r.body[2], r.body[3]] {
                    h = crate::mix(h, v as i64);
                }
            }
            for &c in m.text.as_slice() {
                h = crate::mix(h, c as i64);
            }
        }
        h
    }
}
