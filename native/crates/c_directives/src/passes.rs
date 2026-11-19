//! The preprocessor as array programs (decision 0421, placement (a)): the spike's passes
//! (`rerail_c_pp::stream::parallel`) as the transducer's side program over integer columns
//! (`rerail_compiler::array`).
//!
//! Two programs:
//! - **The round** (`ROUND`), one of P1's: from the include tree and the activity the round
//!   before left, the unit's layout (sizes bottom-up, first positions top-down: fixed points
//!   over the tree), the definitions by position (rows sorted by name and position, pushes and
//!   pops matched by depth), every arm's test, each chain's first true arm and the groups'
//!   activity (a fixed point over the nesting), the includes found, the pragmas `_Pragma`
//!   makes, and what `#pragma once` and `#import` skip.
//! - **The text** (`TEXT`), P2 to P4 under the settled state: the active text's tokens, their
//!   parentheses matched by depth, the candidates (a search of the definitions), the outermost
//!   invocations (a prefix maximum), each expanded alone, and the output assembled by counts,
//!   a prefix sum and a scatter.
//!
//! The host (`rerail_c_pp::stream::parallel::host::Unit`) runs the rounds until one leaves the
//! state it read: between two, it reads the files the round found and numbers the instances
//! its includes enter. The functions the programs `call` are its: an arm's test, an include's
//! file, a stretch's `_Pragma`s and an invocation's expansion, each one element alone.

use rerail_c_pp::Config;
use rerail_c_pp::files::Files;
use rerail_c_pp::stream::parallel::host::{Unit, code, ev, tok};
use rerail_c_pp::stream::parallel::{Outcome, Stats};
use rerail_compiler::array::{ArrayOp, ArrayProgram, Calls, Combine, Fun, MapOp, Place};
use std::collections::HashMap;
use std::sync::LazyLock;

/// A program being built: its operations, and its inputs and outputs by name.
struct B {
    p: ArrayProgram,
    inputs: Vec<&'static str>,
    outputs: Vec<&'static str>,
}

type V = u16;

impl B {
    fn new() -> B {
        B { p: ArrayProgram::default(), inputs: Vec::new(), outputs: Vec::new() }
    }
    fn op(&mut self, op: ArrayOp) -> V {
        self.p.ops.push(op);
        (self.p.ops.len() - 1) as V
    }
    fn input(&mut self, name: &'static str) -> V {
        let k = self.p.inputs;
        self.p.inputs += 1;
        self.inputs.push(name);
        self.op(ArrayOp::Input(k))
    }
    fn output(&mut self, name: &'static str, v: V) {
        self.p.outputs.push(v);
        self.outputs.push(name);
    }
    fn c(&mut self, x: i64) -> V {
        self.op(ArrayOp::Const(x))
    }
    fn map(&mut self, op: MapOp, a: V, b: V) -> V {
        self.op(ArrayOp::Map(op, a, b))
    }
    fn add(&mut self, a: V, b: V) -> V {
        self.map(MapOp::Add, a, b)
    }
    fn sub(&mut self, a: V, b: V) -> V {
        self.map(MapOp::Sub, a, b)
    }
    fn and(&mut self, a: V, b: V) -> V {
        self.map(MapOp::And, a, b)
    }
    fn or(&mut self, a: V, b: V) -> V {
        self.map(MapOp::Or, a, b)
    }
    fn eq(&mut self, a: V, b: V) -> V {
        self.map(MapOp::Eq, a, b)
    }
    fn lt(&mut self, a: V, b: V) -> V {
        self.map(MapOp::Lt, a, b)
    }
    fn le(&mut self, a: V, b: V) -> V {
        self.map(MapOp::Le, a, b)
    }
    fn shl(&mut self, a: V, k: u32) -> V {
        let k = self.c(i64::from(k));
        self.map(MapOp::Shl, a, k)
    }
    fn shr(&mut self, a: V, k: u32) -> V {
        let k = self.c(i64::from(k));
        self.map(MapOp::Shr, a, k)
    }
    fn addc(&mut self, a: V, x: i64) -> V {
        let x = self.c(x);
        self.add(a, x)
    }
    fn eqc(&mut self, a: V, x: i64) -> V {
        let x = self.c(x);
        self.eq(a, x)
    }
    fn select(&mut self, m: V, a: V, b: V) -> V {
        self.op(ArrayOp::Select(m, a, b))
    }
    fn gather(&mut self, s: V, i: V) -> V {
        self.op(ArrayOp::Gather(s, i))
    }
    /// `s[i]` where `i` is in `s`, else `none`: a gather from `s` with `none` after it.
    fn gather_or(&mut self, s: V, i: V, none: i64) -> V {
        let zero = self.c(0);
        let n = self.len(s);
        let lo = self.le(zero, i);
        let hi = self.lt(i, n);
        let ok = self.and(lo, hi);
        let at = self.select(ok, i, n);
        let none = self.c(none);
        let ext = self.concat(s, none);
        self.gather(ext, at)
    }
    fn scan(&mut self, how: Combine, excl: bool, a: V, seg: Option<V>) -> V {
        self.op(ArrayOp::Scan { how, excl, a, seg })
    }
    fn sort(&mut self, keys: &[V]) -> V {
        self.op(ArrayOp::Sort(keys.to_vec()))
    }
    fn scatter(&mut self, how: Combine, src: V, idx: V, n: V, init: i64) -> V {
        let init = self.c(init);
        self.op(ArrayOp::Scatter { how, src, idx, n, init })
    }
    fn search(&mut self, le: bool, sorted: V, q: V) -> V {
        self.op(ArrayOp::Search { le, sorted, q })
    }
    fn compact(&mut self, m: V) -> V {
        self.op(ArrayOp::Compact(m))
    }
    fn expand(&mut self, c: V) -> V {
        self.op(ArrayOp::Expand(c))
    }
    fn iota(&mut self, a: V) -> V {
        self.op(ArrayOp::Iota(a))
    }
    fn len(&mut self, a: V) -> V {
        self.op(ArrayOp::Len(a))
    }
    fn concat(&mut self, a: V, b: V) -> V {
        self.op(ArrayOp::Concat(a, b))
    }
    fn fun(&mut self, name: &str, place: Place, fields: u8) -> u16 {
        self.p.funs.push(Fun { name: name.to_owned(), place, fields });
        (self.p.funs.len() - 1) as u16
    }
    fn call(&mut self, f: u16, args: &[V], env: &[V]) -> V {
        self.op(ArrayOp::Call { f, args: args.to_vec(), env: env.to_vec() })
    }
    fn part(&mut self, c: V, k: u8) -> V {
        self.op(ArrayOp::Part(c, k))
    }
    fn carry(&mut self, init: V) -> V {
        self.op(ArrayOp::Carry(init))
    }
    fn rounds(&mut self, pairs: &[(V, V)]) {
        self.p.rounds.push(pairs.to_vec());
    }
    /// Ones (zeros) as long as `a`.
    fn ones(&mut self, a: V) -> V {
        let one = self.c(1);
        self.select(a, one, one)
    }
    fn zeros(&mut self, a: V) -> V {
        let zero = self.c(0);
        self.select(a, zero, zero)
    }
    /// The maximum of `a` (0 when empty): a scalar.
    fn max_of(&mut self, a: V) -> V {
        let zero = self.c(0);
        let one = self.c(1);
        self.scatter(Combine::Max, a, zero, one, 0)
    }
    /// The element before each in `a`, `none` for the first.
    fn before(&mut self, a: V, none: i64) -> V {
        let i = self.iota(a);
        let p = self.addc(i, -1);
        self.gather_or(a, p, none)
    }
    /// Each element's index in its segment, segments by counts `n` (`expand`'s output order).
    fn within(&mut self, owner: V, n: V) -> V {
        let base = self.scan(Combine::Add, true, n, None);
        let i = self.iota(owner);
        let b = self.gather(base, owner);
        self.sub(i, b)
    }
    fn built(self) -> Prog {
        self.p.verify().expect("the passes' program verifies");
        Prog { p: self.p, inputs: self.inputs, outputs: self.outputs }
    }
}

/// A built program, its inputs and outputs by name.
pub struct Prog {
    pub p: ArrayProgram,
    inputs: Vec<&'static str>,
    outputs: Vec<&'static str>,
}

/// The program's functions, in the order both programs declare them.
const F_TEST: u16 = 0;
const F_INCLUDE: u16 = 1;
const F_EVENTS: u16 = 2;
const F_EXPAND: u16 = 3;

fn funs(b: &mut B) {
    assert_eq!(b.fun("test", Place::Words, 3), F_TEST);
    assert_eq!(b.fun("include", Place::Host, 2), F_INCLUDE);
    assert_eq!(b.fun("events", Place::Words, 3), F_EVENTS);
    assert_eq!(b.fun("expand", Place::Words, 2), F_EXPAND);
}

/// The shape's own refusals (`shape`'s codes).
const SHAPE: &[&str] = &["", "a pop_macro with no push", "a call past its stretch of text", "past the positions' bits"];

/// The unit's instances, their tree and layout, and the definitions by position: what both
/// programs read first.
struct Base {
    i_file: V,
    f_dirbase: V,
    f_ndirs: V,
    // By instance-directive (an instance's directives, instance by instance).
    id_inst: V,
    id_dir: V,
    id_pos: V,
    id_gidx: V,
    // By instance.
    intree: V,
    bs: V,
    i_dbase: V,
    i_gbase: V,
    i_ndirs: V,
    // By instance-group.
    ig_inst: V,
    ig_group: V,
    // The definitions by position.
    keys: V,
    vals: V,
    // Inputs both read.
    f_textbase: V,
    f_groupbase: V,
    d_before: V,
    d_after: V,
    d_code: V,
    d_val: V,
    f_ntext: V,
    id_shift_incl: V,
    a_prev: V,
    shape: V,
}

fn base(b: &mut B) -> Base {
    let f_dirbase = b.input("f_dirbase");
    let f_ndirs = b.input("f_ndirs");
    let f_groupbase = b.input("f_groupbase");
    let f_ngroups = b.input("f_ngroups");
    let f_textbase = b.input("f_textbase");
    let f_ntext = b.input("f_ntext");
    let d_code = b.input("d_code");
    let d_sym = b.input("d_sym");
    let d_val = b.input("d_val");
    let d_group = b.input("d_group");
    let d_after = b.input("d_after");
    let d_before = b.input("d_before");
    let i_file = b.input("i_file");
    let l_parent = b.input("l_parent");
    let l_dir = b.input("l_dir");
    let l_child = b.input("l_child");
    let a_prev = b.input("a_prev");
    let e_inst = b.input("e_inst");
    let e_tok = b.input("e_tok");
    let e_ev = b.input("e_ev");
    let e_sym = b.input("e_sym");
    let b_sym = b.input("b_sym");
    let b_val = b.input("b_val");
    let zero = b.c(0);
    let one = b.c(1);
    // Instance-directives and instance-groups.
    let i_ndirs = b.gather(f_ndirs, i_file);
    let i_ngroups = b.gather(f_ngroups, i_file);
    let i_ntext = b.gather(f_ntext, i_file);
    let i_dbase = b.scan(Combine::Add, true, i_ndirs, None);
    let i_gbase = b.scan(Combine::Add, true, i_ngroups, None);
    let id_inst = b.expand(i_ndirs);
    let id_off = b.within(id_inst, i_ndirs);
    let i_fdb = b.gather(f_dirbase, i_file);
    let id_fdb = b.gather(i_fdb, id_inst);
    let id_dir = b.add(id_fdb, id_off);
    let ig_inst = b.expand(i_ngroups);
    let ig_off = b.within(ig_inst, i_ngroups);
    let i_fgb = b.gather(f_groupbase, i_file);
    let ig_fgb = b.gather(i_fgb, ig_inst);
    let ig_group = b.add(ig_fgb, ig_off);
    let n_i = b.len(i_file);
    let n_id = b.len(id_inst);
    // The links: each one's instance-directive, and each directive's child.
    let l_db = b.gather(i_dbase, l_parent);
    let l_pf = b.gather(i_file, l_parent);
    let l_fdb = b.gather(f_dirbase, l_pf);
    let l_off = b.sub(l_dir, l_fdb);
    let l_idx = b.add(l_db, l_off);
    let id_child = b.scatter(Combine::Set, l_child, l_idx, n_id, -1);
    let i_parent = b.scatter(Combine::Set, l_parent, l_child, n_i, -1);
    // In the tree: the roots, and what a link from the tree enters.
    let ii = b.iota(i_file);
    let two = b.c(2);
    let root = b.lt(ii, two);
    let t0 = b.carry(root);
    let tp = b.gather_or(t0, i_parent, 0);
    let t1 = b.or(root, tp);
    b.rounds(&[(t0, t1)]);
    let intree = t1;
    // Sizes bottom-up: each instance's directives and tokens, and its children's sizes.
    let own = b.add(i_ndirs, i_ntext);
    let s0 = b.carry(own);
    let ls = b.gather(s0, l_child);
    let lt = b.gather(t0, l_child);
    let lsz = b.select(lt, ls, zero);
    let kids = b.scatter(Combine::Add, lsz, l_parent, n_i, 0);
    let s1 = b.add(own, kids);
    b.rounds(&[(s0, s1)]);
    let sz = s1;
    // Each directive's slot after its instance's first: its place, the text before it, and the
    // sizes of the includes before it.
    let id_ct = b.gather_or(intree, id_child, 0);
    let id_cs0 = b.gather_or(sz, id_child, 0);
    let id_cs = b.select(id_ct, id_cs0, zero);
    let id_first = b.eqc(id_off, 0);
    let id_shift = b.scan(Combine::Add, true, id_cs, Some(id_first));
    let id_shift_incl = b.add(id_shift, id_cs);
    let id_before = b.gather(d_before, id_dir);
    let rel0 = b.add(id_off, id_before);
    let id_rel = b.add(rel0, id_shift);
    // First slots top-down: the predefines' at 1, the main file's after them, a child's after
    // its include.
    let s_first = b.gather(sz, zero);
    let main_base = b.add(s_first, one);
    let is0 = b.eqc(ii, 0);
    let root_base = b.select(is0, one, main_base);
    let b0 = b.carry(root_base);
    let lpb = b.gather(b0, l_parent);
    let lrel = b.gather(id_rel, l_idx);
    let lb0 = b.add(lpb, lrel);
    let lb = b.addc(lb0, 1);
    let scattered = b.scatter(Combine::Set, lb, l_child, n_i, 0);
    let b1 = b.select(root, root_base, scattered);
    b.rounds(&[(b0, b1)]);
    let bs = b1;
    let id_b = b.gather(bs, id_inst);
    let id_slot = b.add(id_b, id_rel);
    let id_pos = b.shl(id_slot, 8);
    // Positions fit 32 bits (a key is a symbol's 32 above them).
    let pm = b.max_of(id_pos);
    let big = b.c(1 << 32);
    let over = b.le(big, pm);
    let three = b.c(3);
    let shape_bits = b.select(over, three, zero);
    // Each directive's group, as an instance-group, and its activity the round before.
    let id_g = b.gather(d_group, id_dir);
    let i_fgb2 = b.gather(f_groupbase, i_file);
    let id_fgb = b.gather(i_fgb2, id_inst);
    let id_gl = b.sub(id_g, id_fgb);
    let id_ib = b.gather(i_gbase, id_inst);
    let id_gidx = b.add(id_ib, id_gl);
    let id_tree = b.gather(intree, id_inst);
    let id_ap = b.gather(a_prev, id_gidx);
    let id_act = b.and(id_ap, id_tree);
    // The rows: active definitions, undefinitions, pushes and pops; the pragmas' pushes and
    // pops; the built-ins at 0. A row's kind: 0 sets, 1 pushes, 2 pops.
    let id_code = b.gather(d_code, id_dir);
    let c_def = b.c(code::DEFINE);
    let c_pop = b.c(code::POP);
    let ge = b.le(c_def, id_code);
    let le = b.le(id_code, c_pop);
    let rowlike = b.and(ge, le);
    let rm = b.and(rowlike, id_act);
    let ri = b.compact(rm);
    let r_dir = b.gather(id_dir, ri);
    let r_sym1 = b.gather(d_sym, r_dir);
    let r_pos1 = b.gather(id_pos, ri);
    let r_code = b.gather(d_code, r_dir);
    let r_val1 = b.gather(d_val, r_dir);
    let is_push = b.eqc(r_code, code::PUSH);
    let is_pop = b.eqc(r_code, code::POP);
    let pp1 = b.select(is_pop, two, zero);
    let r_kind1 = b.select(is_push, one, pp1);
    // The pragmas' rows: the event's token's position, one past, and the event's place.
    let e_file = b.gather(i_file, e_inst);
    let e_pos0 = token_pos(b, e_inst, e_file, e_tok, &TokenPlace { f_dirbase, f_ndirs, d_before, i_dbase, id_shift_incl, bs });
    let e_pi = b.before(e_inst, -1);
    let e_seg = b.map(MapOp::Ne, e_pi, e_inst);
    let e_ones = b.ones(e_inst);
    let e_k = b.scan(Combine::Add, true, e_ones, Some(e_seg));
    let e_pos1 = b.addc(e_pos0, 1);
    let e_pos = b.add(e_pos1, e_k);
    let e_tree = b.gather(intree, e_inst);
    let e_once = b.eqc(e_ev, ev::ONCE);
    let e_row0 = b.map(MapOp::Ne, e_once, one);
    let e_row = b.and(e_row0, e_tree);
    let ei = b.compact(e_row);
    let er_sym = b.gather(e_sym, ei);
    let er_pos = b.gather(e_pos, ei);
    let er_ev = b.gather(e_ev, ei);
    let er_push = b.eqc(er_ev, ev::PUSH);
    let er_kind = b.select(er_push, one, two);
    let er_val = b.zeros(er_push);
    let b_pos = b.zeros(b_sym);
    let b_kind = b_pos;
    let r_sym2 = b.concat(r_sym1, er_sym);
    let r_sym = b.concat(r_sym2, b_sym);
    let r_pos2 = b.concat(r_pos1, er_pos);
    let r_pos = b.concat(r_pos2, b_pos);
    let r_kind2 = b.concat(r_kind1, er_kind);
    let r_kind_u = b.concat(r_kind2, b_kind);
    let r_val2 = b.concat(r_val1, er_val);
    let r_val_u = b.concat(r_val2, b_val);
    let r_hi = b.shl(r_sym, 32);
    let r_key_u = b.or(r_hi, r_pos);
    let perm = b.sort(&[r_key_u]);
    let key = b.gather(r_key_u, perm);
    let kind = b.gather(r_kind_u, perm);
    let val = b.gather(r_val_u, perm);
    let sym = b.shr(key, 32);
    // Each name's rows a segment: the stack's depth after each row.
    let ri2 = b.iota(key);
    let prev = b.addc(ri2, -1);
    let psym = b.gather_or(sym, prev, -1);
    let seg = b.map(MapOp::Ne, psym, sym);
    let k_push = b.eqc(kind, 1);
    let k_pop = b.eqc(kind, 2);
    let neg = b.sub(zero, k_pop);
    let delta = b.select(k_push, one, neg);
    let dep_after = b.scan(Combine::Add, false, delta, Some(seg));
    let dep_before = b.sub(dep_after, delta);
    let unmatched0 = b.le(dep_before, zero);
    let unmatched = b.and(unmatched0, k_pop);
    let um = b.scatter(Combine::Max, unmatched, zero, one, 0);
    let um_code = b.select(um, one, zero);
    let shape = b.map(MapOp::Max, shape_bits, um_code);
    // Each pop's push: the latest push of its name before it with the depth it pops.
    let pi = b.compact(k_push);
    let p_sym = b.gather(sym, pi);
    let p_dep = b.gather(dep_after, pi);
    let p_hi = b.shl(p_sym, 40);
    let p_mid = b.shl(p_dep, 32);
    let p_k0 = b.or(p_hi, p_mid);
    let p_key_u = b.or(p_k0, pi);
    let pperm = b.sort(&[p_key_u]);
    let p_key = b.gather(p_key_u, pperm);
    let q_hi = b.shl(sym, 40);
    let q_mid = b.shl(dep_before, 32);
    let q0 = b.or(q_hi, q_mid);
    let q = b.or(q0, ri2);
    let at = b.search(false, p_key, q);
    let atm = b.addc(at, -1);
    let cand = b.gather_or(p_key, atm, -1);
    let cand_hi = b.shr(cand, 32);
    let q_top = b.shr(q, 32);
    let same = b.eq(cand_hi, q_top);
    let mask32 = b.c(0xffff_ffff);
    let cand_row = b.and(cand, mask32);
    let none = b.c(-1);
    let matched = b.select(same, cand_row, none);
    // Each row's source: a set row itself; a push the row before it in its segment; a pop its
    // push. Followed until a set (or none: undefined).
    let is_set = b.eqc(kind, 0);
    let before = b.select(seg, none, prev);
    let src_push = b.select(k_push, before, matched);
    let src = b.select(is_set, ri2, src_push);
    let q0c = b.carry(src);
    let qk = b.gather_or(kind, q0c, 0);
    let neg_q = b.lt(q0c, zero);
    let at_set = b.eqc(qk, 0);
    let term = b.or(neg_q, at_set);
    let qn = b.gather_or(src, q0c, -1);
    let q1 = b.select(term, q0c, qn);
    b.rounds(&[(q0c, q1)]);
    let cur = b.gather_or(val, q1, 0);
    // The table: every row but the pushes.
    let not_push = b.map(MapOp::Ne, kind, one);
    let kept = b.compact(not_push);
    let keys = b.gather(key, kept);
    let vals = b.gather(cur, kept);
    Base { i_file, f_dirbase, f_ndirs, id_inst, id_dir, id_pos, id_gidx, intree, bs, i_dbase, i_gbase, i_ndirs, ig_inst, ig_group, keys, vals, f_textbase, f_groupbase, d_before, d_after, d_code, d_val, f_ntext, id_shift_incl, a_prev, shape }
}

/// What a text token's position reads.
struct TokenPlace {
    f_dirbase: V,
    f_ndirs: V,
    d_before: V,
    i_dbase: V,
    id_shift_incl: V,
    bs: V,
}

/// The position of text token `tok` (its index in its file) of instance `inst`: the directives
/// before it in the file (a search), the sizes of their includes, and the instance's first slot.
fn token_pos(b: &mut B, inst: V, file: V, tok: V, w: &TokenPlace) -> V {
    // The directives' files, and each directive's (file, text before) as one sorted key.
    let d_file = b.expand(w.f_ndirs);
    let dhi = b.shl(d_file, 32);
    let dkey = b.or(dhi, w.d_before);
    let qhi = b.shl(file, 32);
    let q = b.or(qhi, tok);
    let kg = b.search(true, dkey, q);
    let fdb = b.gather(w.f_dirbase, file);
    let k = b.sub(kg, fdb);
    token_pos_after(b, inst, tok, k, w)
}

/// `token_pos` of a token with `k` directives before it in its file.
fn token_pos_after(b: &mut B, inst: V, tok: V, k: V, w: &TokenPlace) -> V {
    // The sizes of the includes before its `k` directives.
    let ib = b.gather(w.i_dbase, inst);
    let at0 = b.add(ib, k);
    let at = b.addc(at0, -1);
    let zero = b.c(0);
    let has = b.lt(zero, k);
    let sh0 = b.gather_or(w.id_shift_incl, at, 0);
    let sh = b.select(has, sh0, zero);
    let rel0 = b.add(tok, k);
    let rel = b.add(rel0, sh);
    let base = b.gather(w.bs, inst);
    let slot = b.add(base, rel);
    b.shl(slot, 8)
}

/// Program 0: one of P1's rounds.
fn round() -> Prog {
    let mut b = B::new();
    funs(&mut b);
    let x = base(&mut b);
    let zero = b.c(0);
    let one = b.c(1);
    let big = b.c(i64::MAX);
    let d_chain = b.input("d_chain");
    let d_arm = b.input("d_arm");
    let f_chainbase = b.input("f_chainbase");
    let f_nchains = b.input("f_nchains");
    let g_parent = b.input("g_parent");
    let g_chain = b.input("g_chain");
    let g_arm = b.input("g_arm");
    let t_reach = b.input("t_reach");
    // The arms read: of an instance in the tree, in a group active in the state the round
    // starts from (an arm under an inactive group is never read), each tested at its position.
    // One a group made active this round is read in the next.
    let id_code = b.gather(x.d_code, x.id_dir);
    let is_arm = b.eqc(id_code, code::TEST);
    let id_tree = b.gather(x.intree, x.id_inst);
    let id_pact = b.gather_or(x.a_prev, x.id_gidx, 0);
    let am0 = b.and(is_arm, id_tree);
    let am = b.and(am0, id_pact);
    let ai = b.compact(am);
    let a_inst = b.gather(x.id_inst, ai);
    let a_dir = b.gather(x.id_dir, ai);
    let a_pos = b.gather(x.id_pos, ai);
    // The tests the rounds before read: by arm (instance << 32 | directive, sorted), its value
    // and refusal, and the definitions it looked up (by memo entry). An arm is read again where
    // it is new or one of those definitions is another at its position now.
    let m_key = b.input("m_key");
    let m_val = b.input("m_val");
    let m_ref = b.input("m_ref");
    let k_owner = b.input("k_owner");
    let k_sym = b.input("k_sym");
    let k_val = b.input("k_val");
    let a_hi = b.shl(a_inst, 32);
    let a_key = b.or(a_hi, a_dir);
    let mk0 = b.search(false, m_key, a_key);
    let m_at = b.gather_or(m_key, mk0, -1);
    let hit = b.eq(m_at, a_key);
    let none = b.c(-1);
    let mk = b.select(hit, mk0, none);
    let n_m = b.len(m_key);
    let a_i = b.iota(a_key);
    let m_arm = b.scatter(Combine::Set, a_i, mk, n_m, -1);
    let p_arm = b.gather_or(m_arm, k_owner, -1);
    let p_ok = b.le(zero, p_arm);
    let p_pos = b.gather_or(a_pos, p_arm, 0);
    let p_hi = b.shl(k_sym, 32);
    let p_q = b.or(p_hi, p_pos);
    let p_at0 = b.search(false, x.keys, p_q);
    let p_at = b.addc(p_at0, -1);
    let p_rk = b.gather_or(x.keys, p_at, -1);
    let p_rs = b.shr(p_rk, 32);
    let p_same = b.eq(p_rs, k_sym);
    let p_v0 = b.gather_or(x.vals, p_at, 0);
    let p_cur = b.select(p_same, p_v0, zero);
    let p_diff = b.map(MapOp::Ne, p_cur, k_val);
    let p_changed = b.and(p_ok, p_diff);
    let m_changed = b.scatter(Combine::Max, p_changed, k_owner, n_m, 0);
    let a_changed = b.gather_or(m_changed, mk, 1);
    let not_hit = b.map(MapOp::Ne, hit, one);
    let need = b.or(not_hit, a_changed);
    let ei = b.compact(need);
    let e_inst = b.gather(a_inst, ei);
    let e_dir = b.gather(a_dir, ei);
    let e_pos = b.gather(a_pos, ei);
    let tc = b.call(F_TEST, &[e_inst, e_dir, e_pos], &[x.keys, x.vals]);
    let t_tag = b.part(tc, 0);
    let t_x = b.part(tc, 1);
    let t_y = b.part(tc, 2);
    let t_owner0 = b.expand(tc);
    let t_owner = b.gather(ei, t_owner0);
    let t_head = b.eqc(t_tag, 0);
    let hi_ = b.compact(t_head);
    let e_val = b.gather(t_x, hi_);
    let e_ref = b.gather(t_y, hi_);
    let n_a = b.len(a_key);
    let ev_all = b.scatter(Combine::Set, e_val, ei, n_a, 0);
    let er_all = b.scatter(Combine::Set, e_ref, ei, n_a, 0);
    let kv = b.gather_or(m_val, mk, 0);
    let kr = b.gather_or(m_ref, mk, 0);
    let a_val = b.select(need, ev_all, kv);
    let a_ref = b.select(need, er_all, kr);
    // The memo the next round reads: every arm now, the pairs of those read again, and the
    // kept pairs of the others.
    let t_pair = b.eqc(t_tag, 1);
    let tpi = b.compact(t_pair);
    let np_owner = b.gather(t_owner, tpi);
    let np_sym = b.gather(t_x, tpi);
    let np_val = b.gather(t_y, tpi);
    let p_need = b.gather_or(need, p_arm, 1);
    let p_keep0 = b.map(MapOp::Ne, p_need, one);
    let p_keep = b.and(p_ok, p_keep0);
    let kpi = b.compact(p_keep);
    let kp_owner = b.gather(p_arm, kpi);
    let kp_sym = b.gather(k_sym, kpi);
    let kp_val = b.gather(k_val, kpi);
    let nk_owner = b.concat(np_owner, kp_owner);
    let nk_sym = b.concat(np_sym, kp_sym);
    let nk_val = b.concat(np_val, kp_val);
    // Each chain's first true arm.
    let i_nch = b.gather(f_nchains, x.i_file);
    let i_cbase = b.scan(Combine::Add, true, i_nch, None);
    let n_ch = {
        let s = b.scan(Combine::Add, false, i_nch, None);
        let l = b.len(s);
        let lm = b.addc(l, -1);
        b.gather_or(s, lm, 0)
    };
    let a_chain = b.gather(d_chain, a_dir);
    let a_file = b.gather(x.i_file, a_inst);
    let a_fcb = b.gather(f_chainbase, a_file);
    let a_cl = b.sub(a_chain, a_fcb);
    let a_icb = b.gather(i_cbase, a_inst);
    let a_ich = b.add(a_icb, a_cl);
    let a_arm = b.gather(d_arm, a_dir);
    let a_src = b.select(a_val, a_arm, big);
    let taken = b.scatter(Combine::Min, a_src, a_ich, n_ch, i64::MAX);
    // Each group's own: a file's group, or an arm whose chain took it.
    let ig_chain = b.gather(g_chain, x.ig_group);
    let ig_arm = b.gather(g_arm, x.ig_group);
    let ig_file = b.gather(x.i_file, x.ig_inst);
    let ig_fcb = b.gather(f_chainbase, ig_file);
    let ig_cl = b.sub(ig_chain, ig_fcb);
    let ig_icb = b.gather(i_cbase, x.ig_inst);
    let ig_ich = b.add(ig_icb, ig_cl);
    let ig_taken = b.gather_or(taken, ig_ich, i64::MAX);
    let ig_is_arm = b.le(zero, ig_chain);
    let ig_took = b.eq(ig_taken, ig_arm);
    let not_arm = b.map(MapOp::Ne, ig_is_arm, one);
    let own = b.or(not_arm, ig_took);
    // Active where its parent is: a fixed point over the nesting.
    let ig_par = b.gather(g_parent, x.ig_group);
    let ig_fgb = b.gather(x.f_groupbase, ig_file);
    let ig_pl = b.sub(ig_par, ig_fgb);
    let ig_ib = b.gather(x.i_gbase, x.ig_inst);
    let ig_pidx0 = b.add(ig_ib, ig_pl);
    let has_par = b.le(zero, ig_par);
    let none = b.c(-1);
    let ig_pidx = b.select(has_par, ig_pidx0, none);
    let act0 = b.carry(own);
    let pa = b.gather_or(act0, ig_pidx, 1);
    let act1 = b.and(own, pa);
    b.rounds(&[(act0, act1)]);
    let ig_tree = b.gather(x.intree, x.ig_inst);
    let a_next = b.select(ig_tree, act1, x.a_prev);
    // A refusal counts where its arm is read: its group active, and no arm before it true.
    let a_g = b.gather(x.id_gidx, ai);
    let a_gact = b.gather(act1, a_g);
    let a_taken = b.gather(taken, a_ich);
    let a_upto = b.le(a_arm, a_taken);
    let a_read = b.and(a_gact, a_upto);
    let a_refd = b.select(a_read, a_ref, zero);
    let refused0 = b.scatter(Combine::Max, a_refd, zero, one, 0);
    // The active includes, each resolved at its position.
    let id_gact = b.gather(act1, x.id_gidx);
    let id_live = b.and(id_gact, id_tree);
    let c_inc = b.c(code::INCLUDE);
    let c_imp = b.c(code::IMPORT);
    let is_inc0 = b.eq(id_code, c_inc);
    let is_imp = b.eq(id_code, c_imp);
    let is_inc = b.or(is_inc0, is_imp);
    let im = b.and(is_inc, id_live);
    let ii = b.compact(im);
    let i_inst = b.gather(x.id_inst, ii);
    let i_dir = b.gather(x.id_dir, ii);
    let i_pos = b.gather(x.id_pos, ii);
    let ic = b.call(F_INCLUDE, &[i_inst, i_dir, i_pos], &[x.keys, x.vals]);
    let i_found = b.part(ic, 0);
    let i_ref = b.part(ic, 1);
    let refused1 = b.scatter(Combine::Max, i_ref, zero, one, 0);
    let refused = b.map(MapOp::Max, refused0, refused1);
    // The pragmas `_Pragma` makes: the active stretches that hold a name that can reach it.
    let tp = TokenPlace { f_dirbase: x.f_dirbase, f_ndirs: x.f_ndirs, d_before: x.d_before, i_dbase: x.i_dbase, id_shift_incl: x.id_shift_incl, bs: x.bs };
    let st = stretches(&mut b, &x, act1, &tp);
    let reach_sum = b.scan(Combine::Add, false, t_reach, None);
    let r_end = b.addc(st.g_end, -1);
    let r_hi = b.gather_or(reach_sum, r_end, 0);
    let r_start = b.addc(st.g_start, -1);
    let r_lo = b.gather_or(reach_sum, r_start, 0);
    let r_n = b.sub(r_hi, r_lo);
    let has_reach = b.lt(zero, r_n);
    let sm = b.and(st.active, has_reach);
    let si = b.compact(sm);
    let s_inst = b.gather(st.inst, si);
    let s_k = b.gather(st.k, si);
    let s_pos = b.gather(st.pos, si);
    let ec = b.call(F_EVENTS, &[s_inst, s_k, s_pos], &[x.keys, x.vals]);
    let owner = b.expand(ec);
    let ne_inst = b.gather(s_inst, owner);
    let ne_tok = b.part(ec, 0);
    let ne_ev = b.part(ec, 1);
    let ne_sym = b.part(ec, 2);
    // `#pragma once`, `_Pragma("once")` and `#import`: from their position on, their file is
    // not entered again.
    let c_once = b.c(code::ONCE);
    let is_once = b.eq(id_code, c_once);
    let om = b.and(is_once, id_live);
    let oi = b.compact(om);
    let o_inst = b.gather(x.id_inst, oi);
    let o_file = b.gather(x.i_file, o_inst);
    let o_pos = b.gather(x.id_pos, oi);
    let ne_once = b.eqc(ne_ev, ev::ONCE);
    let neoi = b.compact(ne_once);
    let neo_inst = b.gather(ne_inst, neoi);
    let neo_file = b.gather(x.i_file, neo_inst);
    let neo_tok = b.gather(ne_tok, neoi);
    let neo_pos0 = token_pos(&mut b, neo_inst, neo_file, neo_tok, &tp);
    // Its place among its instance's events: as the events' own (`e_k`), counted in order.
    let ne_pi = b.before(ne_inst, -1);
    let ne_seg = b.map(MapOp::Ne, ne_pi, ne_inst);
    let ne_one = b.ones(ne_inst);
    let ne_k = b.scan(Combine::Add, true, ne_one, Some(ne_seg));
    let neo_k = b.gather(ne_k, neoi);
    let neo_p1 = b.addc(neo_pos0, 1);
    let neo_pos = b.add(neo_p1, neo_k);
    let found = b.le(zero, i_found);
    let fi = b.compact(found);
    let f_inst = b.gather(i_inst, fi);
    let f_dir = b.gather(i_dir, fi);
    let f_pos = b.gather(i_pos, fi);
    let f_file = b.gather(i_found, fi);
    let f_code = b.gather(x.d_code, f_dir);
    let f_imp = b.eqc(f_code, code::IMPORT);
    let fii = b.compact(f_imp);
    let imp_file = b.gather(f_file, fii);
    let imp_pos = b.gather(f_pos, fii);
    let of2 = b.concat(o_file, neo_file);
    let ofile = b.concat(of2, imp_file);
    let op2 = b.concat(o_pos, neo_pos);
    let opos = b.concat(op2, imp_pos);
    let ohi = b.shl(ofile, 32);
    let okey_u = b.or(ohi, opos);
    let operm = b.sort(&[okey_u]);
    let okey = b.gather(okey_u, operm);
    let fhi = b.shl(f_file, 32);
    let fq = b.or(fhi, f_pos);
    let fat = b.search(false, okey, fq);
    let fatm = b.addc(fat, -1);
    let oc = b.gather_or(okey, fatm, -1);
    let oc_file = b.shr(oc, 32);
    let skipped = b.eq(oc_file, f_file);
    let kept_m = b.map(MapOp::Ne, skipped, one);
    let ki = b.compact(kept_m);
    let n_parent = b.gather(f_inst, ki);
    let n_dir = b.gather(f_dir, ki);
    let n_file = b.gather(f_file, ki);
    // The unsupported directives that are active.
    let c_uns = b.c(code::UNSUPPORTED);
    let is_uns = b.eq(id_code, c_uns);
    let um = b.and(is_uns, id_live);
    let uv0 = b.gather(x.d_val, x.id_dir);
    let uv = b.addc(uv0, 1);
    let ux = b.select(um, uv, zero);
    let unsupported = b.scatter(Combine::Max, ux, zero, one, 0);
    b.output("a_next", a_next);
    b.output("n_parent", n_parent);
    b.output("n_dir", n_dir);
    b.output("n_file", n_file);
    b.output("e_inst", ne_inst);
    b.output("e_tok", ne_tok);
    b.output("e_ev", ne_ev);
    b.output("e_sym", ne_sym);
    b.output("refused", refused);
    b.output("unsupported", unsupported);
    b.output("shape", x.shape);
    b.output("m_key", a_key);
    b.output("m_val", a_val);
    b.output("m_ref", a_ref);
    b.output("k_owner", nk_owner);
    b.output("k_sym", nk_sym);
    b.output("k_val", nk_val);
    b.built()
}

/// The stretches of text: each instance's text after each of its directives (and before the
/// first), its group's activity, its tokens' range in the unit's text, and its position.
struct Stretches {
    inst: V,
    k: V,
    active: V,
    g_start: V,
    g_end: V,
    pos: V,
}

fn stretches(b: &mut B, x: &Base, act: V, tp: &TokenPlace) -> Stretches {
    let zero = b.c(0);
    let n = b.addc(x.i_ndirs, 1);
    let inst = b.expand(n);
    let k = b.within(inst, n);
    let file = b.gather(x.i_file, inst);
    let fdb = b.gather(x.f_dirbase, file);
    let first = b.eqc(k, 0);
    // The directive before it: its group after it, and the text before it.
    let dk0 = b.add(fdb, k);
    let dprev = b.addc(dk0, -1);
    let gafter = b.gather_or(x.d_after, dprev, -1);
    let fgb = b.gather(x.f_groupbase, file);
    let group = b.select(first, fgb, gafter);
    let gl = b.sub(group, fgb);
    let ib = b.gather(x.i_gbase, inst);
    let gidx = b.add(ib, gl);
    let ga = b.gather(act, gidx);
    let tree = b.gather(x.intree, inst);
    let active = b.and(ga, tree);
    let sb = b.gather_or(x.d_before, dprev, 0);
    let start = b.select(first, zero, sb);
    let ndirs = b.gather(x.i_ndirs, inst);
    let last = b.eq(k, ndirs);
    // The directive after it (none after the last stretch, and a unit may have no directive).
    let dk = b.gather_or(x.d_before, dk0, 0);
    let ntext = b.gather(x.f_ntext, file);
    let end = b.select(last, ntext, dk);
    let tb = b.gather(x.f_textbase, file);
    let g_start = b.add(tb, start);
    let g_end = b.add(tb, end);
    // Its first token has its `k` directives before it (a nonempty stretch's; an empty one is
    // never active).
    let pos = token_pos_after(b, inst, start, k, tp);
    let nonempty = b.lt(start, end);
    let active = b.and(active, nonempty);
    Stretches { inst, k, active, g_start, g_end, pos }
}

/// Program 1: the text, P2 to P4.
fn text() -> Prog {
    let mut b = B::new();
    funs(&mut b);
    let x = base(&mut b);
    let zero = b.c(0);
    let one = b.c(1);
    let t_kind = b.input("t_kind");
    let t_sym = b.input("t_sym");
    let fnlike = b.input("fnlike");
    let tp = TokenPlace { f_dirbase: x.f_dirbase, f_ndirs: x.f_ndirs, d_before: x.d_before, i_dbase: x.i_dbase, id_shift_incl: x.id_shift_incl, bs: x.bs };
    let st = stretches(&mut b, &x, x.a_prev, &tp);
    // The active stretches, in the unit's order, and their tokens.
    let sa_u = b.compact(st.active);
    let sa_pos = b.gather(st.pos, sa_u);
    let sperm = b.sort(&[sa_pos]);
    let sa = b.gather(sa_u, sperm);
    let s_start = b.gather(st.g_start, sa);
    let s_end = b.gather(st.g_end, sa);
    let s_pos = b.gather(st.pos, sa);
    let s_inst = b.gather(st.inst, sa);
    let s_len = b.sub(s_end, s_start);
    let tx_s = b.expand(s_len);
    let tx_j = b.within(tx_s, s_len);
    let tx_gs = b.gather(s_start, tx_s);
    let tx_t = b.add(tx_gs, tx_j);
    let tx_sp = b.gather(s_pos, tx_s);
    let tx_jp = b.shl(tx_j, 8);
    let tx_pos = b.add(tx_sp, tx_jp);
    let tx_kind = b.gather(t_kind, tx_t);
    let tx_sym = b.gather(t_sym, tx_t);
    let tx_first = b.eqc(tx_j, 0);
    let tx_i = b.iota(tx_t);
    // Parentheses by depth, a `)` with none open changing nothing: the depth less its least.
    let lp = b.eqc(tx_kind, tok::LPAREN);
    let rp = b.eqc(tx_kind, tok::RPAREN);
    let neg = b.sub(zero, rp);
    let delta = b.select(lp, one, neg);
    let d = b.scan(Combine::Add, false, delta, Some(tx_first));
    let dmin = b.scan(Combine::Min, false, d, Some(tx_first));
    let dmin0 = b.map(MapOp::Min, dmin, zero);
    let depth = b.sub(d, dmin0);
    // The depth before each token: the one after the token before it in its stretch.
    let db0 = b.before(depth, 0);
    let dprev = b.select(tx_first, zero, db0);
    // A `)` that closes: it was open before it. Keyed (stretch, depth after, index).
    let closes0 = b.lt(zero, dprev);
    let closes = b.and(rp, closes0);
    let ci = b.compact(closes);
    let c_s = b.gather(tx_s, ci);
    let c_d = b.gather(depth, ci);
    let c_hi = b.shl(c_s, 36);
    let c_mid = b.shl(c_d, 24);
    let c_j = b.gather(tx_j, ci);
    let ck0 = b.or(c_hi, c_mid);
    let ck_u = b.or(ck0, c_j);
    let cperm = b.sort(&[ck_u]);
    let ck = b.gather(ck_u, cperm);
    // Each `(`'s `)`: the first after it in its stretch at the depth before it.
    let lq_hi = b.shl(tx_s, 36);
    let ld = b.addc(depth, -1);
    let lq_mid = b.shl(ld, 24);
    let lq0 = b.or(lq_hi, lq_mid);
    let lq = b.or(lq0, tx_j);
    let lat = b.search(true, ck, lq);
    let lc = b.gather_or(ck, lat, -1);
    let lc_top = b.shr(lc, 24);
    let lq_top = b.shr(lq, 24);
    let lsame = b.eq(lc_top, lq_top);
    let mask24 = b.c(0xff_ffff);
    let lcj = b.and(lc, mask24);
    let ldiff = b.sub(lcj, tx_j);
    let lclose0 = b.add(tx_i, ldiff);
    let none = b.c(-1);
    let lclose1 = b.select(lsame, lclose0, none);
    let close = b.select(lp, lclose1, none);
    // Positions' bits: a stretch, a depth and an index fit their fields.
    let big_j = b.c(1 << 24);
    let jover = b.le(big_j, tx_j);
    let dlim = b.c(1 << 12);
    let dover = b.le(dlim, depth);
    let over0 = b.or(jover, dover);
    let over_m = b.max_of(over0);
    let three = b.c(3);
    let over_code = b.select(over_m, three, zero);
    // The candidates: identifiers naming a macro where they stand.
    let qs = b.shl(tx_sym, 32);
    let q = b.or(qs, tx_pos);
    let at = b.search(false, x.keys, q);
    let atm = b.addc(at, -1);
    let rk = b.gather_or(x.keys, atm, -1);
    let rsym = b.shr(rk, 32);
    let is_id = b.eqc(tx_kind, tok::IDENT);
    let same = b.eq(rsym, tx_sym);
    let hit = b.and(is_id, same);
    let def0 = b.gather_or(x.vals, atm, 0);
    let def = b.select(hit, def0, zero);
    let cand = b.lt(zero, def);
    let fl = b.gather(fnlike, def);
    // An invocation's last token: its own, or its call's `)`.
    let nxt = b.addc(tx_i, 1);
    let nk = b.gather_or(tx_kind, nxt, -1);
    let ns = b.gather_or(tx_s, nxt, -1);
    let same_s = b.eq(ns, tx_s);
    let n_lp0 = b.eqc(nk, tok::LPAREN);
    let n_lp = b.and(n_lp0, same_s);
    let ncl = b.gather_or(close, nxt, -1);
    let call_reach = b.select(n_lp, ncl, none);
    let fl_reach = b.select(fl, call_reach, tx_i);
    let reach0 = b.select(cand, fl_reach, none);
    // A function-like name before a `(` with no `)` in its stretch: the shape refuses it.
    let fl_c = b.and(cand, fl);
    let open_call0 = b.and(fl_c, n_lp);
    let ncl_none = b.lt(ncl, zero);
    let open_call = b.and(open_call0, ncl_none);
    let oc_m = b.max_of(open_call);
    let two = b.c(2);
    let oc_code = b.select(oc_m, two, zero);
    let reach = reach0;
    // The outermost: past every earlier one's reach (a prefix maximum).
    let far = b.scan(Combine::Max, true, reach, None);
    let past = b.lt(far, tx_i);
    let is_inv0 = b.le(zero, reach);
    let outer = b.and(is_inv0, past);
    let vi = b.compact(outer);
    let v_last_i = b.gather(reach, vi);
    let v_first = b.gather(tx_t, vi);
    let v_last = b.gather(tx_t, v_last_i);
    let v_s = b.gather(tx_s, vi);
    let v_stop0 = b.gather(s_end, v_s);
    let v_stop = b.addc(v_stop0, -1);
    let v_pos = b.gather(tx_pos, vi);
    let v_inst = b.gather(s_inst, v_s);
    let ec = b.call(F_EXPAND, &[v_inst, v_first, v_last, v_stop, v_pos, zero], &[x.keys, x.vals]);
    let e_h = b.part(ec, 0);
    let e_tag = b.part(ec, 1);
    let e_owner = b.expand(ec);
    // `__COUNTER__`: each invocation's count, its base the counts before it; those that count
    // and have one, expanded again from it.
    let is_count = b.eqc(e_tag, 1);
    let cnt_i = b.compact(is_count);
    let v_counted = b.gather(e_h, cnt_i);
    let v_base = b.scan(Combine::Add, true, v_counted, None);
    let vc0 = b.lt(zero, v_counted);
    let vc1 = b.lt(zero, v_base);
    let again_m = b.and(vc0, vc1);
    let ai = b.compact(again_m);
    let a_inst = b.gather(v_inst, ai);
    let a_first = b.gather(v_first, ai);
    let a_last = b.gather(v_last, ai);
    let a_stop = b.gather(v_stop, ai);
    let a_pos = b.gather(v_pos, ai);
    let a_base = b.gather(v_base, ai);
    let ac = b.call(F_EXPAND, &[a_inst, a_first, a_last, a_stop, a_pos, a_base], &[x.keys, x.vals]);
    let a_h = b.part(ac, 0);
    let a_tag = b.part(ac, 1);
    let a_owner0 = b.expand(ac);
    let a_owner = b.gather(ai, a_owner0);
    // Each invocation's tokens: the second expansion's where there is one.
    let n_v = b.len(vi);
    let redone = b.scatter(Combine::Set, one, ai, n_v, 0);
    let e_red = b.gather(redone, e_owner);
    let e_tok0 = b.eqc(e_tag, 0);
    let e_not_red = b.map(MapOp::Ne, e_red, one);
    let e_keep = b.and(e_tok0, e_not_red);
    let a_keep = b.eqc(a_tag, 0);
    let eki = b.compact(e_keep);
    let aki = b.compact(a_keep);
    let k_owner1 = b.gather(e_owner, eki);
    let k_owner2 = b.gather(a_owner, aki);
    let k_h1 = b.gather(e_h, eki);
    let k_h2 = b.gather(a_h, aki);
    let k_owner_u = b.concat(k_owner1, k_owner2);
    let k_h_u = b.concat(k_h1, k_h2);
    let k_ord = b.iota(k_owner_u);
    let kperm = b.sort(&[k_owner_u, k_ord]);
    let k_owner = b.gather(k_owner_u, kperm);
    let k_h = b.gather(k_h_u, kperm);
    let v_n = b.scatter(Combine::Add, one, k_owner, n_v, 0);
    // P4: each token's count: an invocation's first its tokens, one inside an invocation none,
    // any other itself.
    let n_tx = b.len(tx_t);
    let starts = b.scatter(Combine::Set, v_n, vi, n_tx, -1);
    let is_start = b.le(zero, starts);
    let inv_reach = b.select(outer, reach, none);
    let cover = b.scan(Combine::Max, false, inv_reach, None);
    let covered = b.le(tx_i, cover);
    let plain0 = b.select(covered, zero, one);
    let count = b.select(is_start, starts, plain0);
    let offset = b.scan(Combine::Add, true, count, None);
    let total0 = b.scan(Combine::Add, false, count, None);
    let tl = b.len(total0);
    let tlm = b.addc(tl, -1);
    let total = b.gather_or(total0, tlm, 0);
    let copied0 = b.map(MapOp::Ne, is_start, one);
    let copied1 = b.eqc(count, 1);
    let copied = b.and(copied0, copied1);
    let pi = b.compact(copied);
    let p_at = b.gather(offset, pi);
    let p_h = b.gather(tx_t, pi);
    let k_vstart = b.gather(vi, k_owner);
    let k_off = b.gather(offset, k_vstart);
    let k_within = b.within(k_owner, v_n);
    let k_at = b.add(k_off, k_within);
    let all_at = b.concat(p_at, k_at);
    let all_h = b.concat(p_h, k_h);
    let out = b.scatter(Combine::Set, all_h, all_at, total, 0);
    let shape0 = b.map(MapOp::Max, x.shape, over_code);
    let shape = b.map(MapOp::Max, shape0, oc_code);
    b.output("out", out);
    b.output("shape", shape);
    b.built()
}

pub static ROUND: LazyLock<Prog> = LazyLock::new(round);
pub static TEXT: LazyLock<Prog> = LazyLock::new(text);

/// The functions, the unit's.
struct Host<'u, 'a> {
    unit: &'u mut Unit<'a>,
    /// A function's refusal: its reason's code plus one.
    refused: i64,
    /// The tests and expansions on the words (`words`), else on the spike's lean reading.
    words: Option<&'u mut words::Words>,
}

impl Calls for Host<'_, '_> {
    fn call(&mut self, f: u16, _: &Fun, a: &[i64], env: &[&[i64]], out: &mut Vec<i64>) -> Result<(), &'static str> {
        let (keys, vals) = (env[0], env[1]);
        match f {
            F_TEST => {
                // Its value and refusal, then each definition it looked up.
                let mut looked = Vec::new();
                let (v, r) = match self.words.as_deref_mut() {
                    Some(w) => w.test(self.unit, a[0], a[1], a[2], keys, vals, &mut looked),
                    None => self.unit.test(a[0], a[1], a[2], keys, vals, &mut looked),
                };
                out.extend([0, i64::from(v), r]);
                for (s, d) in looked {
                    out.extend([1, s, d]);
                }
            }
            F_INCLUDE => match self.unit.include(a[0], a[1], a[2], keys, vals) {
                Ok(file) => out.extend([file, 0]),
                Err(r) => out.extend([-1, r]),
            },
            F_EVENTS => self.unit.events(a[0], a[1], a[2], keys, vals, out),
            F_EXPAND => {
                let mut toks = Vec::new();
                let r = match self.words.as_deref_mut() {
                    Some(w) => w.expand(self.unit, a[0], a[1], a[2], a[3], a[4], a[5], keys, vals, &mut toks),
                    None => self.unit.expand(a[0], a[1], a[2], a[3], a[4], a[5], keys, vals, &mut toks),
                };
                match r {
                    Ok(counted) => {
                        for h in toks {
                            out.extend([h, 0]);
                        }
                        out.extend([counted, 1]);
                    }
                    Err(r) => {
                        self.refused = self.refused.max(r);
                        out.extend([0, 1]);
                    }
                }
            }
            _ => unreachable!("a function the programs declare"),
        }
        Ok(())
    }

    /// Tests and expansions on the words read together (`Words::test_many`, `expand_many`): a
    /// wave of readings in one batch. Every other call one by one.
    fn call_many(&mut self, f: u16, fun: &Fun, args: &[&[i64]], w: usize, env: &[&[i64]], out: &mut Vec<Vec<i64>>) -> Result<(), &'static str> {
        let a = |k: usize, j: usize| if args[k].len() == 1 { args[k][0] } else { args[k][j] };
        match (f, self.words.as_deref_mut()) {
            (F_TEST, Some(words)) => {
                let (keys, vals) = (env[0], env[1]);
                let arms: Vec<(i64, i64, i64)> = (0..w).map(|j| (a(0, j), a(1, j), a(2, j))).collect();
                let mut looked: Vec<Vec<(i64, i64)>> = vec![Vec::new(); w];
                let vs = words.test_many(self.unit, &arms, keys, vals, &mut looked);
                for ((v, r), l) in vs.into_iter().zip(looked) {
                    let mut rows = vec![0, i64::from(v), r];
                    for (s, d) in l {
                        rows.extend([1, s, d]);
                    }
                    out.push(rows);
                }
                Ok(())
            }
            (F_EXPAND, Some(words)) => {
                let (keys, vals) = (env[0], env[1]);
                let calls: Vec<[i64; 6]> = (0..w).map(|j| std::array::from_fn(|k| a(k, j))).collect();
                let mut toks: Vec<Vec<i64>> = vec![Vec::new(); w];
                let rs = words.expand_many(self.unit, &calls, keys, vals, &mut toks);
                for (r, t) in rs.into_iter().zip(toks) {
                    let mut rows = Vec::with_capacity(2 * t.len() + 2);
                    match r {
                        Ok(counted) => {
                            for h in t {
                                rows.extend([h, 0]);
                            }
                            rows.extend([counted, 1]);
                        }
                        Err(r) => {
                            self.refused = self.refused.max(r);
                            rows.extend([0, 1]);
                        }
                    }
                    out.push(rows);
                }
                Ok(())
            }
            _ => {
                let mut one = Vec::with_capacity(args.len());
                for j in 0..w {
                    one.clear();
                    one.extend((0..args.len()).map(|k| a(k, j)));
                    let mut rows = Vec::new();
                    self.call(f, fun, &one, env, &mut rows)?;
                    out.push(rows);
                }
                Ok(())
            }
        }
    }
}

/// Which executor runs the programs: the interpreter (their meaning), or the device's launches
/// (`rerail_metal::array_program`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exec {
    Interpreter,
    #[cfg(target_os = "macos")]
    Metal,
    /// The programs on Metal, and the words' readings too: a wave of a call's readings in one
    /// lockstep batch (`expand::on_metal_batch`, 0408).
    #[cfg(target_os = "macos")]
    MetalWords,
}

/// The device's account of a unit's runs (`Exec::Metal`).
#[derive(Clone, Debug, Default)]
pub struct DeviceStats {
    pub runs: usize,
    pub gpu_seconds: f64,
    pub dispatches: usize,
    pub syncs: usize,
    /// Elements whose calls the host answered.
    pub calls: usize,
    /// The syncs by what read (`DeviceRun::syncs_by`).
    pub syncs_by: std::collections::BTreeMap<&'static str, usize>,
    /// The most device bytes a run held at once (`DeviceRun::peak_bytes`).
    pub peak_bytes: usize,
    pub by_kernel: std::collections::BTreeMap<&'static str, usize>,
}

impl DeviceStats {
    pub fn add(&mut self, o: &DeviceStats) {
        (self.runs, self.gpu_seconds, self.dispatches, self.syncs, self.calls) = (self.runs + o.runs, self.gpu_seconds + o.gpu_seconds, self.dispatches + o.dispatches, self.syncs + o.syncs, self.calls + o.calls);
        self.peak_bytes = self.peak_bytes.max(o.peak_bytes);
        for (k, n) in &o.by_kernel {
            *self.by_kernel.entry(k).or_default() += n;
        }
        for (k, n) in &o.syncs_by {
            *self.syncs_by.entry(k).or_default() += n;
        }
    }
}

/// The executor of a unit's runs, and its account.
struct Runner {
    exec: Exec,
    device: DeviceStats,
}

impl Prog {
    /// The first operation whose value for unit `u` of a batch's device run is not its
    /// interpreter's, printed with its operands (`RERAIL_PP_ORACLE_OPS`).
    #[cfg(target_os = "macos")]
    fn first_difference(&self, inputs: &[Vec<&[i64]>], hosts: &mut [Host], u: usize) {
        let want = self.p.values_at(&inputs[u], &[], &mut hosts[u]).ok().expect("the interpreter's run");
        let mut jobs: Vec<rerail_metal::array_program::Job> = inputs.iter().zip(hosts.iter_mut()).map(|(i, h)| rerail_metal::array_program::Job { inputs: i.clone(), params: Vec::new(), calls: h }).collect();
        let got = rerail_metal::array_program::values_batch(&self.p, &mut jobs).expect("a device");
        let show = |v: &[i64]| format!("{} long {:?}", v.len(), &v[..v.len().min(24)]);
        for (i, w) in want.arrays.iter().enumerate() {
            let g = got[u][i].as_deref().unwrap_or(&[]);
            if w.as_slice() != g {
                eprintln!("oracle: unit {u}: %{i} = {} differs\n  interpreter {}\n  device      {}", self.p.ops[i].text(), show(w), show(g));
                for o in self.p.ops[i].operands() {
                    let o = usize::from(o);
                    let same = want.arrays[o].as_slice() == got[u][o].as_deref().unwrap_or(&[]);
                    eprintln!("  %{o} = {} ({}): {}", self.p.ops[o].text(), if same { "the same" } else { "DIFFERS" }, show(&want.arrays[o]));
                }
                return;
            }
        }
        eprintln!("oracle: unit {u}: every value the same");
    }

    /// A run for each unit of a batch on its named columns, its calls answered by its host: its
    /// outputs by name.
    fn run(&self, cols: &[HashMap<&'static str, Vec<i64>>], hosts: &mut [Host], runner: &mut Runner) -> Vec<Result<HashMap<&'static str, Vec<i64>>, &'static str>> {
        let inputs: Vec<Vec<&[i64]>> = cols.iter().map(|c| self.inputs.iter().map(|n| c.get(n).unwrap_or_else(|| panic!("no column {n}")).as_slice()).collect()).collect();
        let outputs: Vec<Result<Vec<Vec<i64>>, (usize, &'static str)>> = match runner.exec {
            Exec::Interpreter => inputs.iter().zip(hosts.iter_mut()).map(|(i, h)| self.p.values_at(i, &[], h).map(|v| self.p.outputs.iter().map(|&o| v.arrays[usize::from(o)].clone()).collect())).collect(),
            #[cfg(target_os = "macos")]
            Exec::Metal | Exec::MetalWords => {
                let mut jobs: Vec<rerail_metal::array_program::Job> = inputs.iter().zip(hosts.iter_mut()).map(|(i, h)| rerail_metal::array_program::Job { inputs: i.clone(), params: Vec::new(), calls: h }).collect();
                match rerail_metal::array_program::run_batch(&self.p, &mut jobs) {
                    Ok((out, r)) => {
                        runner.device.add(&DeviceStats { runs: 1, gpu_seconds: r.gpu_seconds, dispatches: r.dispatches, syncs: r.syncs, calls: r.calls, syncs_by: r.syncs_by, peak_bytes: r.peak_bytes, by_kernel: r.by_kernel });
                        if std::env::var_os("RERAIL_PP_ORACLE").is_some() {
                            // Each unit's run against the interpreter's, output by output.
                            drop(jobs);
                            for (u, (i, h)) in inputs.iter().zip(hosts.iter_mut()).enumerate() {
                                let want = self.p.values_at(i, &[], h).map(|v| self.p.outputs.iter().map(|&o| v.arrays[usize::from(o)].clone()).collect::<Vec<_>>());
                                match (&want, &out[u]) {
                                    (Ok(w), Ok(g)) => {
                                        for (k, (w, g)) in w.iter().zip(g).enumerate() {
                                            if w != g {
                                                eprintln!("oracle: unit {u} of {}: output {} differs ({} and {} long): {:?} / {:?}", out.len(), self.outputs[k], w.len(), g.len(), &w[..w.len().min(8)], &g[..g.len().min(8)]);
                                            }
                                        }
                                        if w != g && std::env::var_os("RERAIL_PP_ORACLE_OPS").is_some() {
                                            self.first_difference(&inputs, hosts, u);
                                            std::process::exit(3);
                                        }
                                    }
                                    _ if want.as_ref().err() != out[u].as_ref().err() => eprintln!("oracle: unit {u}: {:?} / {:?}", want.as_ref().err(), out[u].as_ref().err()),
                                    _ => {}
                                }
                            }
                        }
                        out
                    }
                    Err(_) => return cols.iter().map(|_| Err("the device failed")).collect(),
                }
            }
        };
        outputs
            .into_iter()
            .map(|o| {
                let o = o.map_err(|(i, why)| {
                    if std::env::var_os("RERAIL_PASSES_TRACE").is_some() {
                        for k in i.saturating_sub(20)..i {
                            eprintln!("passes:   %{k} = {}", self.p.ops[k].text());
                        }
                        eprintln!("passes: %{i} = {} refused: {why}", self.p.ops[i].text());
                    }
                    why
                })?;
                Ok(self.outputs.iter().copied().zip(o).collect())
            })
            .collect()
    }
}

/// The unit's state between rounds: the links of the include tree, each instance-group's
/// activity, and the pragmas `_Pragma` made.
#[derive(Clone, PartialEq, Eq, Default)]
struct State {
    l_parent: Vec<i64>,
    l_dir: Vec<i64>,
    l_child: Vec<i64>,
    a: Vec<i64>,
    e: [Vec<i64>; 4],
}

/// The tests read so far (`ROUND`'s memo): carried from round to round, not part of the state
/// the fixpoint compares.
#[derive(Default)]
struct Memo([Vec<i64>; 6]);

const MEMO: [&str; 6] = ["m_key", "m_val", "m_ref", "k_owner", "k_sym", "k_val"];

/// The unit whose main file is `path`, read by the programs.
pub fn run(cfg: Config, io: &mut dyn Files, path: &str) -> (Outcome, Stats) {
    let mut unit = match Unit::new(cfg, io, path) {
        Ok(u) => u,
        Err(why) => return (Err(why), Stats::default()),
    };
    let mut r = [Reading::new(&mut unit, None)];
    read(&mut r, &mut Runner { exec: Exec::Interpreter, device: DeviceStats::default() });
    let [r] = r;
    let out = r.outcome();
    (out, unit.stats.clone())
}

/// `run`, its tests and expansions on the words (decision 0430), run by `run_words`
/// (`expand::native`, or the interpreter).
pub fn run_words(cfg: Config, io: &mut dyn Files, path: &str, run_words: words::Run) -> (Outcome, Stats, words::WordStats) {
    let (out, stats, w, _) = run_on(cfg, io, path, run_words, Exec::Interpreter);
    (out, stats, w)
}

/// `run_words`, the programs run by `exec`.
pub fn run_on(cfg: Config, io: &mut dyn Files, path: &str, run_words: words::Run, exec: Exec) -> (Outcome, Stats, words::WordStats, DeviceStats) {
    let (mut out, device) = run_batch(vec![(cfg, io, path)], run_words, exec);
    let (o, s, w) = out.pop().unwrap();
    (o, s, w, device)
}

/// Each unit of a batch read by the programs (`run_on`), in the same runs: the rounds of every
/// unit still moving in one run of `ROUND` a round, then the text of all in one run of `TEXT`.
pub fn run_batch(units: Vec<(Config, &mut dyn Files, &str)>, run_words: words::Run, exec: Exec) -> (Vec<(Outcome, Stats, words::WordStats)>, DeviceStats) {
    let mut made: Vec<Result<(Unit, words::Words), &'static str>> = units
        .into_iter()
        .map(|(cfg, io, path)| {
            let w = words::Words::new(&cfg, run_words)?;
            #[cfg(target_os = "macos")]
            let w = if exec == Exec::MetalWords { w.with_batch(crate::expand::on_metal_counted) } else { w };
            Ok((Unit::new(cfg, io, path)?, w))
        })
        .collect();
    let mut runner = Runner { exec, device: DeviceStats::default() };
    let mut readings: Vec<Reading> = made.iter_mut().filter_map(|m| m.as_mut().ok()).map(|(u, w)| Reading::new(u, Some(w))).collect();
    read(&mut readings, &mut runner);
    let outcomes: Vec<Outcome> = readings.into_iter().map(Reading::outcome).collect();
    let mut outcomes = outcomes.into_iter();
    let out = made
        .into_iter()
        .map(|m| match m {
            Ok((u, w)) => (outcomes.next().unwrap(), u.stats.clone(), w.stats.clone()),
            Err(why) => (Err(why), Stats::default(), Default::default()),
        })
        .collect::<Vec<_>>();
    (out, runner.device)
}

/// A unit's reading: its state between rounds, its memo, and where it is.
struct Reading<'u, 'a> {
    unit: &'u mut Unit<'a>,
    words: Option<&'u mut words::Words>,
    state: State,
    memo: Memo,
    b_sym: Vec<i64>,
    b_val: Vec<i64>,
    /// Its outcome once it has one; until then, whether its rounds are done (its text next).
    done: Option<Outcome>,
    settled: bool,
}

impl<'u, 'a> Reading<'u, 'a> {
    fn new(unit: &'u mut Unit<'a>, words: Option<&'u mut words::Words>) -> Self {
        let mut state = State::default();
        fresh(unit, 0, &mut state.a);
        let (b_sym, b_val) = unit.builtin_rows();
        Reading { unit, words, state, memo: Memo::default(), b_sym, b_val, done: None, settled: false }
    }

    fn outcome(self) -> Outcome {
        self.done.unwrap_or(Err("past the rounds' bound"))
    }
}

/// Each instance from `from` as first entered: its own group active, nothing else.
fn fresh(unit: &Unit, from: usize, a: &mut Vec<i64>) {
    let inst = unit.instances();
    for &f in &inst[from..] {
        let n = unit.cols.f_ngroups[f as usize];
        a.push(1);
        a.extend(std::iter::repeat_n(0, (n - 1) as usize));
    }
}

fn read(rs: &mut [Reading], runner: &mut Runner) {
    for round in 1..=rerail_compiler::array::MAX_ROUNDS as u32 {
        let moving: Vec<usize> = (0..rs.len()).filter(|&k| rs[k].done.is_none() && !rs[k].settled).collect();
        if moving.is_empty() {
            break;
        }
        let mut cols = Vec::with_capacity(moving.len());
        for &k in &moving {
            let r = &mut rs[k];
            r.unit.generation += 1;
            r.unit.stats.rounds = round;
            let mut c = columns(r.unit, &r.state, &r.b_sym, &r.b_val);
            for (j, n) in MEMO.iter().enumerate() {
                c.insert(n, std::mem::take(&mut r.memo.0[j]));
            }
            cols.push(c);
        }
        let outs = {
            let mut hosts: Vec<Host> = rs.iter_mut().enumerate().filter(|(k, _)| moving.contains(k)).map(|(_, r)| Host { unit: &mut *r.unit, refused: 0, words: r.words.as_deref_mut() }).collect();
            ROUND.run(&cols, &mut hosts, runner)
        };
        for (&k, o) in moving.iter().zip(outs) {
            let r = &mut rs[k];
            let mut o = match o {
                Ok(o) => o,
                Err(why) => {
                    r.done = Some(Err(why));
                    continue;
                }
            };
            for (j, n) in MEMO.iter().enumerate() {
                r.memo.0[j] = o.remove(n).expect("the memo's outputs");
            }
            if o["refused"][0] > 0 {
                r.done = Some(Err(r.unit.reason(o["refused"][0] - 1)));
                continue;
            }
            if o["shape"][0] > 0 {
                r.done = Some(Err(SHAPE[o["shape"][0] as usize]));
                continue;
            }
            let n_inst = r.unit.instances().len();
            let mut next = State { a: o["a_next"].clone(), e: [o["e_inst"].clone(), o["e_tok"].clone(), o["e_ev"].clone(), o["e_sym"].clone()], ..State::default() };
            for j in 0..o["n_parent"].len() {
                let (p, d, f) = (o["n_parent"][j], o["n_dir"][j], o["n_file"][j]);
                let c = r.unit.enter(p, d, f);
                next.l_parent.push(p);
                next.l_dir.push(d);
                next.l_child.push(c);
            }
            fresh(r.unit, n_inst, &mut next.a);
            if next == r.state {
                if o["unsupported"][0] > 0 {
                    r.done = Some(Err(r.unit.reason(o["unsupported"][0] - 1)));
                } else {
                    r.settled = true;
                }
                continue;
            }
            r.state = next;
        }
    }
    text_of(rs, runner);
}

/// The text of each unit whose rounds are done, in one run of `TEXT`.
fn text_of(rs: &mut [Reading], runner: &mut Runner) {
    let ready: Vec<usize> = (0..rs.len()).filter(|&k| rs[k].done.is_none() && rs[k].settled).collect();
    if ready.is_empty() {
        return;
    }
    let mut cols = Vec::with_capacity(ready.len());
    for &k in &ready {
        let r = &mut rs[k];
        r.unit.generation += 1;
        let mut c = columns(r.unit, &r.state, &r.b_sym, &r.b_val);
        c.insert("t_kind", r.unit.cols.t_kind.clone());
        c.insert("t_sym", r.unit.cols.t_sym.clone());
        c.insert("fnlike", r.unit.fnlike());
        cols.push(c);
    }
    let (outs, refused): (Vec<_>, Vec<i64>) = {
        let mut hosts: Vec<Host> = rs.iter_mut().enumerate().filter(|(k, _)| ready.contains(k)).map(|(_, r)| Host { unit: &mut *r.unit, refused: 0, words: r.words.as_deref_mut() }).collect();
        let outs = TEXT.run(&cols, &mut hosts, runner);
        (outs, hosts.iter().map(|h| h.refused).collect())
    };
    for ((&k, o), refused) in ready.iter().zip(outs).zip(refused) {
        let r = &mut rs[k];
        r.done = Some((|| {
            let o = o?;
            if refused > 0 {
                return Err(r.unit.reason(refused - 1));
            }
            if o["shape"][0] > 0 {
                return Err(SHAPE[o["shape"][0] as usize]);
            }
            r.unit.stats.output = o["out"].len() as u64;
            Ok(o["out"].iter().map(|&h| if h < 0 && let Some(w) = r.words.as_deref() { w.made(h).to_vec() } else { r.unit.spelling(h) }).collect())
        })());
    }
}

/// The columns a run reads.
fn columns(unit: &mut Unit, s: &State, b_sym: &[i64], b_val: &[i64]) -> HashMap<&'static str, Vec<i64>> {
    let t_reach = unit.reaching();
    let c = &unit.cols;
    let mut m: HashMap<&'static str, Vec<i64>> = HashMap::new();
    for (n, v) in [
        ("f_dirbase", &c.f_dirbase),
        ("f_ndirs", &c.f_ndirs),
        ("f_groupbase", &c.f_groupbase),
        ("f_ngroups", &c.f_ngroups),
        ("f_chainbase", &c.f_chainbase),
        ("f_nchains", &c.f_nchains),
        ("f_textbase", &c.f_textbase),
        ("f_ntext", &c.f_ntext),
        ("d_code", &c.d_code),
        ("d_sym", &c.d_sym),
        ("d_val", &c.d_val),
        ("d_group", &c.d_group),
        ("d_after", &c.d_after),
        ("d_before", &c.d_before),
        ("d_chain", &c.d_chain),
        ("d_arm", &c.d_arm),
        ("g_parent", &c.g_parent),
        ("g_chain", &c.g_chain),
        ("g_arm", &c.g_arm),
    ] {
        m.insert(n, v.clone());
    }
    m.insert("i_file", unit.instances());
    m.insert("l_parent", s.l_parent.clone());
    m.insert("l_dir", s.l_dir.clone());
    m.insert("l_child", s.l_child.clone());
    m.insert("a_prev", s.a.clone());
    m.insert("e_inst", s.e[0].clone());
    m.insert("e_tok", s.e[1].clone());
    m.insert("e_ev", s.e[2].clone());
    m.insert("e_sym", s.e[3].clone());
    m.insert("b_sym", b_sym.to_vec());
    m.insert("b_val", b_val.to_vec());
    m.insert("t_reach", t_reach);
    m
}

#[cfg(test)]
mod tests;

pub mod words;
