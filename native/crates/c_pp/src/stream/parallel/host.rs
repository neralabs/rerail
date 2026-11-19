//! The host's side of the passes as an array program (decision 0421, placement (a)): a unit's
//! files as columns, its instances, and the functions an array program's `call`s name, each
//! the spike's own reading of one element.
//!
//! The program (`rerail_c_directives::passes`) holds the shape: positions, the definitions by
//! position, activity, the include tree's layout, the invocations and the assembly. This module
//! holds what is not the program's:
//! - **P0**: each file's tables (`files::tabulate`), as columns, numbered across the unit: a
//!   file's directives, groups, chains and text tokens follow the files read before it;
//! - **the instances**: the instance an include enters, by (instance, directive, file), new
//!   ones numbered as they are found (the import fixpoint's split: reading files is the host's);
//! - **the functions**, each of one element, read where the element stands (`in_context`) under
//!   the definitions the program gives (an `env` table): an arm's test, an include's file, a
//!   stretch's `_Pragma`s, and an invocation's expansion.
//!
//! The positional table crosses as two columns: `key = symbol << 32 | position` (sorted) and
//! `value` (a definition's index plus one; 0, none).

use super::files::Act;
use super::rounds::{NONE, Tu, reaching, resolve, test};
use super::text::{Piece, Stretch, isolated, stretch};
use super::{Ev, SUB, Stats, Vis, in_context};
use super::super::{Config, Pp};
use crate::directives::Kind;
use crate::files::Files;
use crate::macros::*;
use rerail_c_lean::T;
use rerail_c_lean::pp::P;
use std::collections::HashSet;

/// A directive's code (`Cols::d_code`).
pub mod code {
    pub const NOTHING: i64 = 0;
    /// A chain's arm: `#if…`, `#elif…`, `#else`.
    pub const TEST: i64 = 1;
    pub const INCLUDE: i64 = 2;
    pub const IMPORT: i64 = 3;
    /// `d_sym` its name, `d_val` its definition plus one.
    pub const DEFINE: i64 = 4;
    pub const UNDEF: i64 = 5;
    pub const PUSH: i64 = 6;
    pub const POP: i64 = 7;
    pub const ONCE: i64 = 8;
    /// `d_val` the reason's code (`Unit::reason`).
    pub const UNSUPPORTED: i64 = 9;
}

/// A text token's kind (`Cols::t_kind`), what the invocations' passes read.
pub mod tok {
    pub const OTHER: i64 = 0;
    pub const IDENT: i64 = 1;
    pub const LPAREN: i64 = 2;
    pub const RPAREN: i64 = 3;
}

/// A pragma event's code: `#pragma once`, `push_macro`, `pop_macro` made by `_Pragma`.
pub mod ev {
    pub const ONCE: i64 = 0;
    pub const PUSH: i64 = 1;
    pub const POP: i64 = 2;
}

/// The columns of every file read, numbered across the unit (`-1`: none).
#[derive(Default, Clone)]
pub struct Cols {
    /// By file, in the order read: its first directive, group, chain and text token, its
    /// counts, and its guard's symbol.
    pub f_dirbase: Vec<i64>,
    pub f_ndirs: Vec<i64>,
    pub f_groupbase: Vec<i64>,
    pub f_ngroups: Vec<i64>,
    pub f_chainbase: Vec<i64>,
    pub f_nchains: Vec<i64>,
    pub f_textbase: Vec<i64>,
    pub f_ntext: Vec<i64>,
    pub f_guard: Vec<i64>,
    /// By directive: its code, symbol and value (`code`), the group it is read in and the group
    /// after it, the text tokens before it in its file, and an arm's chain and place in it.
    pub d_code: Vec<i64>,
    pub d_sym: Vec<i64>,
    pub d_val: Vec<i64>,
    pub d_group: Vec<i64>,
    pub d_after: Vec<i64>,
    pub d_before: Vec<i64>,
    pub d_chain: Vec<i64>,
    pub d_arm: Vec<i64>,
    /// By group: its parent group (`-1` for a file's own), and an arm's chain and place.
    pub g_parent: Vec<i64>,
    pub g_chain: Vec<i64>,
    pub g_arm: Vec<i64>,
    /// By text token: its kind (`tok`) and symbol.
    pub t_kind: Vec<i64>,
    pub t_sym: Vec<i64>,
}

/// A unit read as an array program: the serial pass's state the functions read in.
pub struct Unit<'a> {
    pp: Pp<'a>,
    tu: Tu,
    builtins: Vec<(u32, u64, u32)>,
    /// By the pass's file id: its number in the unit's columns (`NONE`: not read).
    number: Vec<u32>,
    /// By number: the file id.
    pub files: Vec<u32>,
    pub cols: Cols,
    /// The generation and length of the positional table `pp` holds.
    vis: Option<(u64, usize)>,
    pub generation: u64,
    /// Tokens the expansions made: handle `-1 - k` is token `k`.
    made: Vec<Tk>,
    reach: (usize, HashSet<u32>),
    pragma: u32,
    reasons: Vec<&'static str>,
    /// By definition: the file and directive that define it (a built-in's: none).
    def_dir: std::collections::HashMap<u32, (u32, u32)>,
    pub stats: Stats,
}

impl<'a> Unit<'a> {
    /// The unit whose main file is `path`: its predefines and main file read, as `run` reads them.
    pub fn new(cfg: Config, io: &'a mut dyn Files, path: &str) -> Result<Unit<'a>, &'static str> {
        let sink: &'a mut crate::sink::Null = Box::leak(Box::new(crate::sink::Null));
        let mut pp = Pp::new(cfg, io, sink);
        let src = pp.io.read(path).ok_or("the main file is not read")?;
        let mut pre = String::new();
        for (n, b) in &pp.cfg.predefines {
            match b {
                Some(b) => pre.push_str(&format!("#define {n} {b}\n")),
                None => pre.push_str(&format!("#undef {n}\n")),
            }
        }
        let builtin = pp.add_file("<built-in>".into(), pre.into_bytes(), None);
        let main = pp.add_file(path.to_string(), src, None);
        pp.base = main;
        let builtins: Vec<(u32, u64, u32)> = pp.macros.iter().enumerate().filter(|&(_, &d)| d != 0).map(|(s, &d)| (s as u32, 0, d)).collect();
        let tu = Tu::new(&mut pp, builtin, main);
        let pragma = pp.syms.get(b"_Pragma");
        let mut unit = Unit { pp, tu, builtins, number: Vec::new(), files: Vec::new(), cols: Cols::default(), vis: None, generation: 0, made: Vec::new(), reach: (usize::MAX, HashSet::new()), pragma, reasons: Vec::new(), def_dir: Default::default(), stats: Stats::default() };
        unit.sync();
        Ok(unit)
    }

    /// The built-ins' rows of the positional table: (symbol, definition plus one), at position 0.
    pub fn builtin_rows(&self) -> (Vec<i64>, Vec<i64>) {
        (self.builtins.iter().map(|b| i64::from(b.0)).collect(), self.builtins.iter().map(|b| i64::from(b.2)).collect())
    }

    /// By instance: its file's number. Instance 0 is the predefines', 1 the main file's; the
    /// predefines' come first in the unit.
    pub fn instances(&self) -> Vec<i64> {
        self.tu.insts.iter().map(|i| i64::from(self.number[i.file as usize])).collect()
    }

    /// By definition (its index plus one, 0 none): whether it is function-like.
    pub fn fnlike(&self) -> Vec<i64> {
        std::iter::once(0).chain(self.pp.defs.iter().map(|d| i64::from(d.fnlike))).collect()
    }

    /// The names that can reach `_Pragma`, as a text token's column: whether its symbol is one.
    pub fn reaching(&mut self) -> Vec<i64> {
        if self.reach.0 != self.pp.defs.len() {
            self.reach = (self.pp.defs.len(), reaching(&self.pp, self.pragma));
        }
        self.cols.t_sym.iter().map(|&s| i64::from(self.reach.1.contains(&(s as u32)))).collect()
    }

    /// Reason `k`'s text (`code::UNSUPPORTED`'s value, a function's refusal).
    pub fn reason(&self, k: i64) -> &'static str {
        self.reasons.get(k as usize).copied().unwrap_or("an unknown reason")
    }

    /// A refusal's reason as a code (`reason`).
    pub fn reason_code(&mut self, why: &'static str) -> i64 {
        match self.reasons.iter().position(|&r| r == why) {
            Some(k) => k as i64,
            None => {
                self.reasons.push(why);
                self.reasons.len() as i64 - 1
            }
        }
    }

    /// The instance that include `dir` (a directive's number) of instance `inst` enters when it
    /// finds file `file` (a number): a new one the first time.
    pub fn enter(&mut self, inst: i64, dir: i64, file: i64) -> i64 {
        let d = (dir - self.cols.f_dirbase[self.number[self.tu.insts[inst as usize].file as usize] as usize]) as u32;
        let f = self.files[file as usize];
        let key = (inst as u32, d, f);
        if let Some(&c) = self.tu.key.get(&key) {
            return i64::from(c);
        }
        let c = self.tu.insts.len() as u32;
        let mut chain = self.tu.insts[inst as usize].chain.clone();
        chain.push(f);
        self.tu.insts.push(super::rounds::Inst { file: f, chain });
        self.tu.key.insert(key, c);
        i64::from(c)
    }

    /// The tables of every file read since, appended to the columns.
    fn sync(&mut self) {
        for file in 0..self.tu.tabs.len() {
            if self.tu.tabs[file].is_none() || self.number.get(file).is_some_and(|&n| n != NONE) {
                continue;
            }
            if self.number.len() <= file {
                self.number.resize(file + 1, NONE);
            }
            self.number[file] = self.files.len() as u32;
            self.files.push(file as u32);
            let tab = self.tu.tabs[file].as_ref().unwrap();
            let c = &mut self.cols;
            let (db, gb, cb, tb) = (c.d_code.len() as i64, c.g_parent.len() as i64, c.f_chainbase.iter().zip(&c.f_nchains).map(|(a, b)| a + b).max().unwrap_or(0), c.t_kind.len() as i64);
            c.f_dirbase.push(db);
            c.f_ndirs.push(tab.dirs.len() as i64);
            c.f_groupbase.push(gb);
            c.f_ngroups.push(tab.groups.len() as i64);
            c.f_chainbase.push(cb);
            c.f_nchains.push(tab.chains.len() as i64);
            c.f_textbase.push(tb);
            c.f_ntext.push(tab.text.len() as i64);
            c.f_guard.push(tab.guard.map_or(-1, i64::from));
            // Each arm's chain and place, by directive and by the group after it.
            let mut arm = vec![(-1i64, -1i64); tab.dirs.len()];
            let mut garm = vec![(-1i64, -1i64); tab.groups.len()];
            for (k, arms) in tab.chains.iter().enumerate() {
                for (a, &d) in arms.iter().enumerate() {
                    arm[d as usize] = (cb + k as i64, a as i64);
                    garm[tab.dirs[d as usize].after as usize] = (cb + k as i64, a as i64);
                }
            }
            let mut reasons = Vec::new();
            for (d, dir) in tab.dirs.iter().enumerate() {
                let (code, sym, val) = match dir.act {
                    Act::Nothing => (code::NOTHING, -1, 0),
                    Act::Test => (code::TEST, -1, 0),
                    Act::Include if dir.kind == Kind::Import => (code::IMPORT, -1, 0),
                    Act::Include => (code::INCLUDE, -1, 0),
                    Act::Define(k) => {
                        self.def_dir.insert(k, (file as u32, d as u32));
                        (code::DEFINE, i64::from(self.pp.defs[k as usize].name), i64::from(k) + 1)
                    }
                    Act::Undef(s) => (code::UNDEF, i64::from(s), 0),
                    Act::Push(s) => (code::PUSH, i64::from(s), 0),
                    Act::Pop(s) => (code::POP, i64::from(s), 0),
                    Act::Once => (code::ONCE, -1, 0),
                    Act::Unsupported(why) => {
                        reasons.push((d, why));
                        (code::UNSUPPORTED, -1, 0)
                    }
                };
                c.d_code.push(code);
                c.d_sym.push(sym);
                c.d_val.push(val);
                c.d_group.push(gb + i64::from(dir.group));
                c.d_after.push(gb + i64::from(dir.after));
                c.d_before.push(i64::from(dir.text_before));
                c.d_chain.push(arm[d].0);
                c.d_arm.push(arm[d].1);
            }
            for (g, group) in tab.groups.iter().enumerate() {
                c.g_parent.push(if g == 0 { -1 } else { gb + i64::from(group.parent) });
                c.g_chain.push(garm[g].0);
                c.g_arm.push(garm[g].1);
            }
            for t in &tab.text {
                let kind = if t.kind == P::Ident {
                    tok::IDENT
                } else if t.is(T::LParen) {
                    tok::LPAREN
                } else if t.is(T::RParen) {
                    tok::RPAREN
                } else {
                    tok::OTHER
                };
                c.t_kind.push(kind);
                c.t_sym.push(if t.kind == P::Ident { i64::from(t.sym) } else { -1 });
            }
            for (d, why) in reasons {
                let k = self.reason_code(why);
                self.cols.d_val[(db as usize) + d] = k;
            }
        }
    }

    /// The positional table of `keys` and `values`, made once a generation.
    fn table(&mut self, keys: &[i64], values: &[i64]) {
        if self.vis == Some((self.generation, keys.len())) {
            return;
        }
        let rows = keys.iter().zip(values).map(|(&k, &v)| ((k >> 32) as u32, (k & 0xffff_ffff) as u64, v as u32)).collect();
        self.pp.vis = Some(Vis { rows });
        self.vis = Some((self.generation, keys.len()));
    }

    /// Arm `dir`'s test in instance `inst` at `pos`: its value, and a refusal's code plus one
    /// (0: none). A refusal counts only where the arm is read; the program decides. The
    /// definitions it looked up go to `looked`, as (symbol, the table's value: definition plus
    /// one, 0 none): the arm is read again only where one of them changes.
    pub fn test(&mut self, inst: i64, dir: i64, pos: i64, keys: &[i64], values: &[i64], looked: &mut Vec<(i64, i64)>) -> (bool, i64) {
        self.table(keys, values);
        self.stats.tests += 1;
        let chain = self.tu.insts[inst as usize].chain.clone();
        let file = self.tu.insts[inst as usize].file;
        let d = (dir - self.cols.f_dirbase[self.number[file as usize] as usize]) as usize;
        let tab = self.tu.tabs[file as usize].as_ref().unwrap();
        self.pp.looked = Some(Default::default());
        let r = in_context(&mut self.pp, &chain, pos as u64, |pp| test(pp, tab, d));
        let l = self.pp.looked.take().expect("the record of lookups").into_inner();
        looked.extend(l.into_iter().map(|(s, d)| (i64::from(s), d.map_or(0, |d| i64::from(d) + 1))));
        match r {
            Ok(v) => (v, 0),
            Err(why) => (false, self.reason_code(why) + 1),
        }
    }

    /// Include `dir`'s file in instance `inst` at `pos` (a number; -1: none), with its file's
    /// guard read there. A refusal is the reason's code plus one.
    pub fn include(&mut self, inst: i64, dir: i64, pos: i64, keys: &[i64], values: &[i64]) -> Result<i64, i64> {
        self.table(keys, values);
        let chain = self.tu.insts[inst as usize].chain.clone();
        let file = self.tu.insts[inst as usize].file;
        let d = (dir - self.cols.f_dirbase[self.number[file as usize] as usize]) as usize;
        let tab = self.tu.tabs[file as usize].as_ref().unwrap();
        let literal = tab.dirs[d].line.first().is_some_and(|t| matches!(t.kind, P::Header | P::Str) || t.is(T::Lt));
        let deep = chain.len() as u32 > self.pp.cfg.max_depth;
        let f = match self.tu.found.get(&(file, d as u32)) {
            Some(&f) if literal => (f != NONE).then_some(f),
            _ => {
                self.stats.includes += 1;
                let f = match in_context(&mut self.pp, &chain, pos as u64, |pp| resolve(pp, tab, d)) {
                    Ok(f) => f,
                    Err(why) => return Err(self.reason_code(why) + 1),
                };
                if literal && !deep {
                    self.tu.found.insert((file, d as u32), f.unwrap_or(NONE));
                }
                f
            }
        };
        let Some(f) = f.filter(|_| !deep) else { return Ok(-1) };
        self.tu.tab(&mut self.pp, f);
        self.sync();
        let guard = self.tu.tabs[f as usize].as_ref().unwrap().guard;
        if guard.is_some_and(|g| self.pp.vis.as_ref().unwrap().at(g, pos as u64).is_some()) {
            return Ok(-1);
        }
        Ok(i64::from(self.number[f as usize]))
    }

    /// The pragmas `_Pragma` makes in the stretch of instance `inst`'s text after its `k`th
    /// directive (its first token at `pos`): `(text token in its file, event, symbol)` rows.
    pub fn events(&mut self, inst: i64, k: i64, pos: i64, keys: &[i64], values: &[i64], out: &mut Vec<i64>) {
        self.table(keys, values);
        let tab = self.tu.tab_of(inst as u32);
        let k = k as usize;
        let start = if k == 0 { 0 } else { tab.dirs[k - 1].text_before };
        let end = if k < tab.dirs.len() { tab.dirs[k].text_before } else { tab.text.len() as u32 };
        let s = Stretch { pos: pos as u64, inst: inst as u32, start, end };
        let mut scratch = Stats::default();
        let mut none = HashSet::new();
        let Ok(pieces) = stretch(&mut self.pp, &self.tu, tab, &s, Some(&self.reach.1), &mut scratch, &mut none) else { return };
        for p in pieces {
            if let Piece::Expanded { at, events, .. } = p {
                for (e, sym) in events {
                    let e = match e {
                        Ev::Once => ev::ONCE,
                        Ev::Push => ev::PUSH,
                        Ev::Pop => ev::POP,
                    };
                    out.extend([i64::from(s.start + at), e, i64::from(sym)]);
                }
            }
        }
    }

    /// The invocation of text tokens `first..=last` (numbers) of instance `inst`, in a stretch
    /// whose last token is `stop`, at `pos`, `__COUNTER__` from `counter`: its tokens' handles
    /// (`spelling`) to `out`, and its count of `__COUNTER__`s. One that asks past its end (a
    /// merge) is refused, with the reason's code plus one.
    pub fn expand(&mut self, inst: i64, first: i64, last: i64, stop: i64, pos: i64, counter: i64, keys: &[i64], values: &[i64], out: &mut Vec<i64>) -> Result<i64, i64> {
        self.table(keys, values);
        let chain = self.tu.insts[inst as usize].chain.clone();
        let file = self.tu.insts[inst as usize].file;
        let base = self.cols.f_textbase[self.number[file as usize] as usize];
        let tab = self.tu.tabs[file as usize].as_ref().unwrap();
        let (a, b, z) = ((first - base) as usize, (last - base) as usize, (stop - base) as usize);
        let list = tab.text[a..=b].to_vec();
        let r = in_context(&mut self.pp, &chain, pos as u64, |pp| isolated(pp, list, counter as u64));
        if r.cut || (r.peeked && b < z && tab.text[b + 1].is(T::LParen)) {
            return Err(self.reason_code("a merge") + 1);
        }
        self.stats.invocations += 1;
        self.stats.expanded += r.out.len() as u64;
        for t in r.out {
            self.made.push(t);
            out.push(-(self.made.len() as i64));
        }
        Ok(r.counted as i64)
    }

    /// A handle's spelling: a text token's number, or a token an expansion made (`-1 - k`).
    pub fn spelling(&self, handle: i64) -> Vec<u8> {
        if handle < 0 {
            return self.pp.text(&self.made[(-handle - 1) as usize]).to_vec();
        }
        let k = self.cols.f_textbase.partition_point(|&b| b <= handle) - 1;
        let tab = self.tu.tabs[self.files[k] as usize].as_ref().unwrap();
        self.pp.text(&tab.text[(handle - self.cols.f_textbase[k]) as usize]).to_vec()
    }

    // The views the words' readings of one element take (`rerail_c_directives::passes::words`).

    /// The pass's files (a token's `buf` less one): each one's path and phase-2 text.
    pub fn file(&self, k: usize) -> (&str, &[u8]) {
        let f = &self.pp.files[k];
        (&f.path, f.text())
    }

    pub fn nfiles(&self) -> usize {
        self.pp.files.len()
    }

    /// File `k`'s line table in phase-2 positions, as the words read one (`pp_words::File::lines`):
    /// for each newline of its source, the last phase-2 position before the line after it.
    pub fn lines(&self, k: usize) -> Vec<u32> {
        let f = &self.pp.files[k];
        let n = f.text().len() as u32;
        let mut out = Vec::new();
        let mut i = 0;
        let src = &f.src;
        while let Some(nl) = rerail_simd8::micro::find(src, i, b'\n') {
            // The first phase-2 position past the newline, less one.
            let (mut lo, mut hi) = (0u32, n + 1);
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if mid < n && f.orig(mid) <= nl as u32 { lo = mid + 1 } else { hi = mid }
            }
            out.push(lo.wrapping_sub(1));
            i = nl + 1;
        }
        out
    }

    /// Instance `inst`'s file (the pass's number) and the files it is included through.
    pub fn instance(&self, inst: i64) -> (u32, &[u32]) {
        let i = &self.tu.insts[inst as usize];
        (i.file, &i.chain)
    }

    /// Text tokens `first..=last` (numbers) of instance `inst`'s file, and the one after them
    /// up to `stop` (`None` past it).
    pub fn text_toks(&self, inst: i64, first: i64, last: i64, stop: i64) -> (&[Tk], Option<&Tk>) {
        let file = self.tu.insts[inst as usize].file;
        let base = self.cols.f_textbase[self.number[file as usize] as usize];
        let tab = self.tu.tabs[file as usize].as_ref().unwrap();
        let (a, b) = ((first - base) as usize, (last - base) as usize);
        (&tab.text[a..=b], (last < stop).then(|| &tab.text[b + 1]))
    }

    /// Directive `dir` (a number) of instance `inst`'s file: its kind and its line's tokens.
    pub fn directive(&self, inst: i64, dir: i64) -> (Kind, &[Tk]) {
        let file = self.tu.insts[inst as usize].file;
        let d = (dir - self.cols.f_dirbase[self.number[file as usize] as usize]) as usize;
        let tab = self.tu.tabs[file as usize].as_ref().unwrap();
        (tab.dirs[d].kind, &tab.dirs[d].line)
    }

    /// Definition `d`'s `#define` line's tokens after `define` (a built-in's: none), and the
    /// symbols of the identifiers its body holds.
    pub fn definition(&self, d: u32) -> Option<(&[Tk], impl Iterator<Item = u32> + '_)> {
        let &(file, k) = self.def_dir.get(&d)?;
        let tab = self.tu.tabs[file as usize].as_ref().unwrap();
        let names = self.pp.defs[d as usize].body.iter().filter(|t| t.kind == P::Ident).map(|t| t.sym);
        Some((&tab.dirs[k as usize].line[..], names))
    }

    /// A symbol's spelling, and the symbol of a spelling.
    pub fn sym_text(&self, s: u32) -> &[u8] {
        self.pp.syms.text(s)
    }

    pub fn sym(&mut self, b: &[u8]) -> u32 {
        self.pp.syms.get(b)
    }

    /// A token's spelling.
    pub fn tok_text(&self, t: &Tk) -> &[u8] {
        self.pp.text(t)
    }

    /// Whether a header name is found from file `from` (`__has_include`, `__has_include_next`
    /// where `next`), as the lean pass's `Files` finds it.
    pub fn found(&mut self, from: u32, name: &[u8], angle: bool, next: bool) -> bool {
        let f = &self.pp.files[from as usize];
        let (includer, next_after) = (f.path.clone(), next.then_some(f.found_by));
        self.pp.io.find(name, angle, &includer, next_after).is_some()
    }

    /// `text` lexed as a file of its own (a `_Pragma`'s, as `pragma_text` lexes one): the file's
    /// number and its tokens.
    pub fn lex(&mut self, path: String, text: Vec<u8>) -> (u32, Vec<Tk>) {
        let pf = self.pp.add_file(path, text, None);
        let super::super::Src::File { mut lx, .. } = self.pp.file_src(pf) else { unreachable!() };
        let mut toks = Vec::new();
        loop {
            let t = lx.next();
            if t.kind == P::Eof {
                break;
            }
            toks.push(self.pp.file_tok(t, pf));
        }
        (pf, toks)
    }

    /// Positions' bits below a token's.
    pub const SUB: u32 = SUB;
}
