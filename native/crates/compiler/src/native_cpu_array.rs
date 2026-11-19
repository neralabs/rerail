//! Array programs on the CPU JIT (decision 0430): each operation's work over its elements as a
//! function Cranelift emits, one for each shape of operation a program has (its map, its
//! combination, its flags), all `fn(args: *const u64) -> i64` over a block of words: lengths,
//! pointers, and a mask a pointer's index is taken by (0, a scalar broadcast; -1, element `j`).
//!
//! The host's driver is the interpreter's (`ArrayProgram::values_at`): the operations in order,
//! the fixed points' rounds, the calls. It chooses each value's length and lays out its memory;
//! every loop over elements is emitted code:
//! - maps, selects, iotas and copies: a loop an element;
//! - scans: a loop with the running combination, reset where a flag is set;
//! - sorts: under 64 elements an insertion sort; else a stable LSD radix sort, a byte a pass,
//!   the keys from the least significant, only the bytes where a key's elements differ;
//! - gathers, scatters (a `set` scatter's written flags), searches (a check that the array is
//!   sorted, then a binary search a query), compacts, expands;
//! - a fixed point's verdict: whether a carry is its next.
//!
//! A call's rows are the host's (`Calls::call_many`), laid out into its parts as the
//! interpreter lays them.
//!
//! A refusal is the interpreter's: its operation and its reason.

use crate::Error;
use crate::array::{ArrayOp, ArrayProgram, Calls, Combine, MAX_ROUNDS, MapOp, Values};
use cranelift_codegen::ir::{AbiParam, BlockArg, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::{Linkage, Module};
use std::collections::HashMap;

/// An emitted operation: its words in, its answer (0, or a refusal's code; a count for
/// `Compact` and `ExpandCount`).
type Kernel = unsafe extern "C" fn(*const u64) -> i64;

/// The shapes of work the functions are emitted for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Shape {
    /// `[n, a, am, b, bm, out]`.
    Map(MapOp),
    /// `[n, m, mm, a, am, b, bm, out]`.
    Select,
    /// `[n, out]`.
    Iota,
    /// `[n, src, out]`.
    Copy,
    /// `[n, a, flags, out]`.
    Scan { how: Combine, excl: bool, seg: bool },
    /// `[n, src, len, idx, out]`: 1 where an index is outside.
    Gather,
    /// `[w, src, sm, idx, im, len, init, out, written]`: 2 where a `set` writes twice.
    Scatter(Combine),
    /// `[ns, sorted, nq, q, out]`: 3 where `sorted` is not.
    Search { le: bool },
    /// `[n, m, out]`: the count.
    Compact,
    /// `[n, c]`: the sum, or -1 at a negative count.
    ExpandCount,
    /// `[n, c, out]`.
    ExpandFill,
    /// `[n, nkeys, keys, perm, tmp, hist]`.
    Sort,
    /// `[n, a, b]`: 1 where they differ.
    Differ,
}

/// A program's operations compiled: each operation's function, and the verdict's.
pub struct CompiledArray {
    _module: JITModule,
    kernels: HashMap<Shape, Kernel>,
}

/// The shape operation `op` runs as, where its work is over elements.
fn shape_of(op: &ArrayOp) -> Vec<Shape> {
    match op {
        ArrayOp::Map(m, ..) => vec![Shape::Map(*m)],
        ArrayOp::Select(..) => vec![Shape::Select],
        ArrayOp::Iota(_) => vec![Shape::Iota],
        ArrayOp::Input(_) | ArrayOp::Concat(..) | ArrayOp::Part(..) | ArrayOp::Carry(_) => vec![Shape::Copy],
        ArrayOp::Scan { how, excl, seg, .. } => vec![Shape::Scan { how: *how, excl: *excl, seg: seg.is_some() }],
        ArrayOp::Gather(..) => vec![Shape::Gather],
        ArrayOp::Scatter { how, .. } => vec![Shape::Scatter(*how)],
        ArrayOp::Search { le, .. } => vec![Shape::Search { le: *le }],
        ArrayOp::Compact(_) => vec![Shape::Compact],
        ArrayOp::Expand(_) => vec![Shape::ExpandCount, Shape::ExpandFill],
        ArrayOp::Sort(_) => vec![Shape::Sort],
        ArrayOp::Param(_) | ArrayOp::Const(_) | ArrayOp::Len(_) | ArrayOp::Call { .. } => vec![],
    }
}

/// `p`'s operations compiled, each shape once.
pub fn compile(p: &ArrayProgram) -> Result<CompiledArray, Error> {
    let mut shapes: Vec<Shape> = vec![Shape::Differ, Shape::Copy];
    for op in &p.ops {
        for s in shape_of(op) {
            if !shapes.contains(&s) {
                shapes.push(s);
            }
        }
    }
    let builder = crate::native_cpu_codegen::tier1_builder().map_err(|_| Error::BackendFailure)?;
    let mut module = JITModule::new(builder);
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(types::I64));
    signature.returns.push(AbiParam::new(types::I64));
    let mut ids = Vec::with_capacity(shapes.len());
    let mut context: Context = module.make_context();
    let mut fbc = FunctionBuilderContext::new();
    for (k, &s) in shapes.iter().enumerate() {
        let id = module.declare_function(&format!("array_{k}"), Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
        context.func.signature = signature.clone();
        {
            let mut f = FunctionBuilder::new(&mut context.func, &mut fbc);
            emit(&mut f, s);
            f.seal_all_blocks();
            f.finalize();
        }
        module.define_function(id, &mut context).map_err(|_| Error::BackendFailure)?;
        module.clear_context(&mut context);
        ids.push(id);
    }
    module.finalize_definitions().map_err(|_| Error::BackendFailure)?;
    let kernels: HashMap<Shape, Kernel> = shapes
        .iter()
        .zip(ids)
        .map(|(&s, id)| {
            // SAFETY: each function was defined with `signature`, which `Kernel` is.
            (s, unsafe { std::mem::transmute::<*const u8, Kernel>(module.get_finalized_function(id)) })
        })
        .collect();
    if std::env::var_os("RERAIL_ARRAY_KERNELS").is_some() {
        // Each function's address, for a sampling profiler's unnamed frames.
        let mut at: Vec<(usize, Shape)> = kernels.iter().map(|(s, k)| (*k as usize, *s)).collect();
        at.sort_unstable_by_key(|a| a.0);
        for (a, s) in at {
            eprintln!("array kernel {a:#x} {s:?}");
        }
    }
    Ok(CompiledArray { _module: module, kernels })
}

const MEM: MemFlags = MemFlags::trusted();

/// A loop while `cond` over carried values: `body` gives the next ones; the last ones out.
fn while_loop(f: &mut FunctionBuilder<'_>, init: &[Value], cond: &mut dyn FnMut(&mut FunctionBuilder<'_>, &[Value]) -> Value, body: &mut dyn FnMut(&mut FunctionBuilder<'_>, &[Value]) -> Vec<Value>) -> Vec<Value> {
    let (head, inner, exit) = (f.create_block(), f.create_block(), f.create_block());
    for _ in init {
        f.append_block_param(head, types::I64);
        f.append_block_param(exit, types::I64);
    }
    let a: Vec<BlockArg> = init.iter().map(|&v| v.into()).collect();
    f.ins().jump(head, &a);
    f.switch_to_block(head);
    let cs = f.block_params(head).to_vec();
    let c = cond(f, &cs);
    let a: Vec<BlockArg> = cs.iter().map(|&v| v.into()).collect();
    f.ins().brif(c, inner, &[], exit, &a);
    f.switch_to_block(inner);
    let next = body(f, &cs);
    let a: Vec<BlockArg> = next.iter().map(|&v| v.into()).collect();
    f.ins().jump(head, &a);
    f.switch_to_block(exit);
    f.block_params(exit).to_vec()
}

/// `j in 0..n` with carried values: `body` gives the next ones.
fn count_loop(f: &mut FunctionBuilder<'_>, n: Value, init: &[Value], body: &mut dyn FnMut(&mut FunctionBuilder<'_>, Value, &[Value]) -> Vec<Value>) -> Vec<Value> {
    let zero = f.ins().iconst(types::I64, 0);
    let mut all = vec![zero];
    all.extend_from_slice(init);
    let out = while_loop(f, &all, &mut |f, c| f.ins().icmp(IntCC::UnsignedLessThan, c[0], n), &mut |f, c| {
        let mut next = body(f, c[0], &c[1..]);
        let j1 = f.ins().iadd_imm(c[0], 1);
        next.insert(0, j1);
        next
    });
    out[1..].to_vec()
}

/// Return `code` where `bad`, else go on.
fn fail_if(f: &mut FunctionBuilder<'_>, bad: Value, code: i64) {
    let (fail, cont) = (f.create_block(), f.create_block());
    f.ins().brif(bad, fail, &[], cont, &[]);
    f.switch_to_block(fail);
    let c = f.ins().iconst(types::I64, code);
    f.ins().return_(&[c]);
    f.switch_to_block(cont);
}

/// Element `j` of the array at `base` by `mask` (0: its first, broadcast).
fn at(f: &mut FunctionBuilder<'_>, base: Value, mask: Value, j: Value) -> Value {
    let off = f.ins().ishl_imm(j, 3);
    let off = f.ins().band(off, mask);
    let addr = f.ins().iadd(base, off);
    f.ins().load(types::I64, MEM, addr, 0)
}

fn elem(f: &mut FunctionBuilder<'_>, base: Value, j: Value) -> Value {
    let off = f.ins().ishl_imm(j, 3);
    let addr = f.ins().iadd(base, off);
    f.ins().load(types::I64, MEM, addr, 0)
}

fn put(f: &mut FunctionBuilder<'_>, base: Value, j: Value, v: Value) {
    let off = f.ins().ishl_imm(j, 3);
    let addr = f.ins().iadd(base, off);
    f.ins().store(MEM, v, addr, 0);
}

fn flag(f: &mut FunctionBuilder<'_>, cc: IntCC, a: Value, b: Value) -> Value {
    let c = f.ins().icmp(cc, a, b);
    f.ins().uextend(types::I64, c)
}

fn combine(f: &mut FunctionBuilder<'_>, how: Combine, a: Value, b: Value) -> Value {
    match how {
        Combine::Set => b,
        Combine::Add => f.ins().iadd(a, b),
        Combine::Max => f.ins().smax(a, b),
        Combine::Min => f.ins().smin(a, b),
    }
}

fn map(f: &mut FunctionBuilder<'_>, op: MapOp, a: Value, b: Value) -> Value {
    match op {
        MapOp::Add => f.ins().iadd(a, b),
        MapOp::Sub => f.ins().isub(a, b),
        MapOp::Mul => f.ins().imul(a, b),
        MapOp::And => f.ins().band(a, b),
        MapOp::Or => f.ins().bor(a, b),
        MapOp::Xor => f.ins().bxor(a, b),
        MapOp::Min => f.ins().smin(a, b),
        MapOp::Max => f.ins().smax(a, b),
        MapOp::Eq => flag(f, IntCC::Equal, a, b),
        MapOp::Ne => flag(f, IntCC::NotEqual, a, b),
        MapOp::Lt => flag(f, IntCC::SignedLessThan, a, b),
        MapOp::Le => flag(f, IntCC::SignedLessThanOrEqual, a, b),
        // Cranelift's shifts take the amount modulo 64, as `wrapping_shl` does.
        MapOp::Shl => f.ins().ishl(a, b),
        MapOp::Shr => f.ins().sshr(a, b),
    }
}

fn emit(f: &mut FunctionBuilder<'_>, s: Shape) {
    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    let words = f.block_params(entry)[0];
    let w: Vec<Value> = (0..9).map(|k| f.ins().load(types::I64, MEM.with_readonly(), words, 8 * k)).collect();
    let zero = f.ins().iconst(types::I64, 0);
    let mut ret = zero;
    match s {
        Shape::Map(op) => {
            count_loop(f, w[0], &[], &mut |f, j, _| {
                let a = at(f, w[1], w[2], j);
                let b = at(f, w[3], w[4], j);
                let r = map(f, op, a, b);
                put(f, w[5], j, r);
                vec![]
            });
        }
        Shape::Select => {
            count_loop(f, w[0], &[], &mut |f, j, _| {
                let m = at(f, w[1], w[2], j);
                let a = at(f, w[3], w[4], j);
                let b = at(f, w[5], w[6], j);
                let r = f.ins().select(m, a, b);
                put(f, w[7], j, r);
                vec![]
            });
        }
        Shape::Iota => {
            count_loop(f, w[0], &[], &mut |f, j, _| {
                put(f, w[1], j, j);
                vec![]
            });
        }
        Shape::Copy => {
            count_loop(f, w[0], &[], &mut |f, j, _| {
                let x = elem(f, w[1], j);
                put(f, w[2], j, x);
                vec![]
            });
        }
        Shape::Scan { how, excl, seg } => {
            let id = f.ins().iconst(types::I64, how.identity());
            count_loop(f, w[0], &[id], &mut |f, j, c| {
                let mut acc = c[0];
                if seg {
                    let s = elem(f, w[2], j);
                    acc = f.ins().select(s, id, acc);
                }
                let e = elem(f, w[1], j);
                let next = combine(f, how, acc, e);
                put(f, w[3], j, if excl { acc } else { next });
                vec![next]
            });
        }
        Shape::Gather => {
            count_loop(f, w[0], &[], &mut |f, j, _| {
                let k = elem(f, w[3], j);
                let bad = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, k, w[2]);
                fail_if(f, bad, 1);
                let x = elem(f, w[1], k);
                put(f, w[4], j, x);
                vec![]
            });
        }
        Shape::Scatter(how) => {
            let (len, init, out, written) = (w[5], w[6], w[7], w[8]);
            count_loop(f, len, &[], &mut |f, t, _| {
                put(f, out, t, init);
                vec![]
            });
            count_loop(f, w[0], &[], &mut |f, j, _| {
                let t = at(f, w[3], w[4], j);
                let inside = f.ins().icmp(IntCC::UnsignedLessThan, t, len);
                let (write, next) = (f.create_block(), f.create_block());
                f.ins().brif(inside, write, &[], next, &[]);
                f.switch_to_block(write);
                let x = at(f, w[1], w[2], j);
                if how == Combine::Set {
                    let addr = f.ins().iadd(written, t);
                    let was = f.ins().uload8(types::I64, MEM, addr, 0);
                    fail_if(f, was, 2);
                    let one = f.ins().iconst(types::I64, 1);
                    f.ins().istore8(MEM, one, addr, 0);
                    put(f, out, t, x);
                } else {
                    let old = elem(f, out, t);
                    let r = combine(f, how, old, x);
                    put(f, out, t, r);
                }
                f.ins().jump(next, &[]);
                f.switch_to_block(next);
                vec![]
            });
        }
        Shape::Search { le } => {
            let (ns, sorted, nq, q, out) = (w[0], w[1], w[2], w[3], w[4]);
            // Sorted: each element at most the next.
            let one = f.ins().iconst(types::I64, 1);
            let pairs = f.ins().umax(ns, one);
            let pairs = f.ins().iadd_imm(pairs, -1);
            count_loop(f, pairs, &[], &mut |f, j, _| {
                let a = elem(f, sorted, j);
                let j1 = f.ins().iadd_imm(j, 1);
                let b = elem(f, sorted, j1);
                let bad = f.ins().icmp(IntCC::SignedGreaterThan, a, b);
                fail_if(f, bad, 3);
                vec![]
            });
            count_loop(f, nq, &[], &mut |f, j, _| {
                let x = elem(f, q, j);
                let z = f.ins().iconst(types::I64, 0);
                let found = while_loop(f, &[z, ns], &mut |f, c| f.ins().icmp(IntCC::UnsignedLessThan, c[0], c[1]), &mut |f, c| {
                    let sum = f.ins().iadd(c[0], c[1]);
                    let mid = f.ins().ushr_imm(sum, 1);
                    let e = elem(f, sorted, mid);
                    let below = f.ins().icmp(if le { IntCC::SignedLessThanOrEqual } else { IntCC::SignedLessThan }, e, x);
                    let up = f.ins().iadd_imm(mid, 1);
                    let lo = f.ins().select(below, up, c[0]);
                    let hi = f.ins().select(below, c[1], mid);
                    vec![lo, hi]
                });
                put(f, out, j, found[0]);
                vec![]
            });
        }
        Shape::Compact => {
            let n = count_loop(f, w[0], &[zero], &mut |f, j, c| {
                let m = elem(f, w[1], j);
                // Each index at the count so far, kept where its flag is set.
                put(f, w[2], c[0], j);
                let set = flag(f, IntCC::NotEqual, m, zero);
                vec![f.ins().iadd(c[0], set)]
            });
            ret = n[0];
        }
        Shape::ExpandCount => {
            let n = count_loop(f, w[0], &[zero], &mut |f, j, c| {
                let k = elem(f, w[1], j);
                let bad = f.ins().icmp(IntCC::SignedLessThan, k, zero);
                fail_if(f, bad, -1);
                vec![f.ins().iadd(c[0], k)]
            });
            ret = n[0];
        }
        Shape::ExpandFill => {
            count_loop(f, w[0], &[zero], &mut |f, j, c| {
                let k = elem(f, w[1], j);
                let base = c[0];
                count_loop(f, k, &[], &mut |f, r, _| {
                    let at = f.ins().iadd(base, r);
                    put(f, w[2], at, j);
                    vec![]
                });
                vec![f.ins().iadd(base, k)]
            });
        }
        Shape::Sort => emit_sort(f, &w),
        Shape::Differ => {
            count_loop(f, w[0], &[], &mut |f, j, _| {
                let a = elem(f, w[1], j);
                let b = elem(f, w[2], j);
                let bad = f.ins().icmp(IntCC::NotEqual, a, b);
                fail_if(f, bad, 1);
                vec![]
            });
        }
    }
    f.ins().return_(&[ret]);
}

/// Arrays shorter than this sort by insertion: a radix pass's histogram is 256 counts.
const INSERTION: i64 = 64;

/// The stable sort, the order into `perm` (from `0..n`):
/// - shorter than `INSERTION`: an insertion sort, each element moved down past those whose keys
///   (compared in order) are greater;
/// - else an LSD radix sort, for each key from the last, each byte from the lowest where the
///   key's elements differ (one pass ORs each one's difference from the first): a histogram
///   of the byte over the elements in their order so far, its exclusive sums, each element to
///   its place in the other buffer. The order ends in `perm`.
fn emit_sort(f: &mut FunctionBuilder<'_>, w: &[Value]) {
    let (n, nkeys, keys, perm, tmp, hist) = (w[0], w[1], w[2], w[3], w[4], w[5]);
    count_loop(f, n, &[], &mut |f, j, _| {
        put(f, perm, j, j);
        vec![]
    });
    let (small, radix, end) = (f.create_block(), f.create_block(), f.create_block());
    let short = f.ins().icmp_imm(IntCC::SignedLessThan, n, INSERTION);
    f.ins().brif(short, small, &[], radix, &[]);

    // Insertion: `x` down past each element before it whose keys are greater.
    f.switch_to_block(small);
    let one = f.ins().iconst(types::I64, 1);
    // -1, 0 or 1: element `a`'s keys against `b`'s, in order.
    let order = |f: &mut FunctionBuilder<'_>, a: Value, b: Value| -> Value {
        let z = f.ins().iconst(types::I64, 0);
        count_loop(f, nkeys, &[z], &mut |f, k, c| {
            let key = elem(f, keys, k);
            let ka = elem(f, key, a);
            let kb = elem(f, key, b);
            let gt = f.ins().icmp(IntCC::SignedGreaterThan, ka, kb);
            let lt = f.ins().icmp(IntCC::SignedLessThan, ka, kb);
            let p1 = f.ins().iconst(types::I64, 1);
            let m1 = f.ins().iconst(types::I64, -1);
            let z = f.ins().iconst(types::I64, 0);
            let here = f.ins().select(lt, m1, z);
            let here = f.ins().select(gt, p1, here);
            let decided = f.ins().icmp_imm(IntCC::NotEqual, c[0], 0);
            vec![f.ins().select(decided, c[0], here)]
        })[0]
    };
    let zero = f.ins().iconst(types::I64, 0);
    let rest = f.ins().isub(n, one);
    let rest = f.ins().smax(rest, zero);
    count_loop(f, rest, &[], &mut |f, i0, _| {
        let i = f.ins().iadd_imm(i0, 1);
        let x = elem(f, perm, i);
        let at = while_loop(f, &[i], &mut |f, c| {
            // `j > 0` and the element before greater (read at `max(j, 1) - 1`, in bounds).
            let j = c[0];
            let one = f.ins().iconst(types::I64, 1);
            let jm = f.ins().smax(j, one);
            let jm = f.ins().iadd_imm(jm, -1);
            let before = elem(f, perm, jm);
            let o = order(f, before, x);
            let greater = f.ins().icmp_imm(IntCC::SignedGreaterThan, o, 0);
            let more = f.ins().icmp_imm(IntCC::SignedGreaterThan, j, 0);
            f.ins().band(more, greater)
        }, &mut |f, c| {
            let jm = f.ins().iadd_imm(c[0], -1);
            let before = elem(f, perm, jm);
            put(f, perm, c[0], before);
            vec![jm]
        });
        put(f, perm, at[0], x);
        vec![]
    });
    f.ins().jump(end, &[]);

    f.switch_to_block(radix);
    let k256 = f.ins().iconst(types::I64, 256);
    let k8 = f.ins().iconst(types::I64, 8);
    let min = f.ins().iconst(types::I64, i64::MIN);
    let digit = |f: &mut FunctionBuilder<'_>, key: Value, idx: Value, shift: Value| -> Value {
        let e = elem(f, key, idx);
        let u = f.ins().bxor(e, min);
        let d = f.ins().ushr(u, shift);
        f.ins().band_imm(d, 255)
    };
    let ends = count_loop(f, nkeys, &[perm, tmp], &mut |f, kk, c| {
        // The keys from the least significant.
        let last = f.ins().iadd_imm(nkeys, -1);
        let k = f.ins().isub(last, kk);
        let key = elem(f, keys, k);
        // The bits where its elements differ from the first.
        let z = f.ins().iconst(types::I64, 0);
        let first = elem(f, key, z);
        let differ = count_loop(f, n, &[z], &mut |f, j, d| {
            let e = elem(f, key, j);
            let x = f.ins().bxor(e, first);
            vec![f.ins().bor(d[0], x)]
        })[0];
        count_loop(f, k8, &[c[0], c[1]], &mut |f, b, c| {
            let (cur, alt) = (c[0], c[1]);
            let shift = f.ins().ishl_imm(b, 3);
            let (pass, done) = (f.create_block(), f.create_block());
            f.append_block_param(done, types::I64);
            f.append_block_param(done, types::I64);
            // One byte for all: this pass keeps the order.
            let varies = f.ins().ushr(differ, shift);
            let varies = f.ins().band_imm(varies, 255);
            f.ins().brif(varies, pass, &[], done, &[cur.into(), alt.into()]);
            f.switch_to_block(pass);
            count_loop(f, k256, &[], &mut |f, d, _| {
                let z = f.ins().iconst(types::I64, 0);
                put(f, hist, d, z);
                vec![]
            });
            count_loop(f, n, &[], &mut |f, j, _| {
                let idx = elem(f, cur, j);
                let d = digit(f, key, idx, shift);
                let h = elem(f, hist, d);
                let h1 = f.ins().iadd_imm(h, 1);
                put(f, hist, d, h1);
                vec![]
            });
            let z = f.ins().iconst(types::I64, 0);
            count_loop(f, k256, &[z], &mut |f, d, s| {
                let h = elem(f, hist, d);
                put(f, hist, d, s[0]);
                vec![f.ins().iadd(s[0], h)]
            });
            count_loop(f, n, &[], &mut |f, j, _| {
                let idx = elem(f, cur, j);
                let d = digit(f, key, idx, shift);
                let at = elem(f, hist, d);
                put(f, alt, at, idx);
                let at1 = f.ins().iadd_imm(at, 1);
                put(f, hist, d, at1);
                vec![]
            });
            f.ins().jump(done, &[alt.into(), cur.into()]);
            f.switch_to_block(done);
            f.block_params(done).to_vec()
        })
    });
    // The order in the other buffer: copied into `perm`.
    let copy = f.create_block();
    let other = f.ins().icmp(IntCC::NotEqual, ends[0], perm);
    f.ins().brif(other, copy, &[], end, &[]);
    f.switch_to_block(copy);
    let from = ends[0];
    count_loop(f, n, &[], &mut |f, j, _| {
        let x = elem(f, from, j);
        put(f, perm, j, x);
        vec![]
    });
    f.ins().jump(end, &[]);
    f.switch_to_block(end);
}

/// A new array of `n` elements, for a function to write whole.
fn room(n: usize) -> Vec<i64> {
    Vec::with_capacity(n)
}

impl CompiledArray {
    fn call(&self, s: Shape, words: &[u64]) -> i64 {
        let mut w = [0u64; 9];
        w[..words.len()].copy_from_slice(words);
        // SAFETY: each shape's function reads its words as `Shape` lays them out, and the
        // driver gives each pointer room for what the function writes.
        unsafe { self.kernels[&s](w.as_ptr()) }
    }

    /// A copy of `x` by the emitted copy.
    fn copied(&self, x: &[i64]) -> Vec<i64> {
        let mut out = room(x.len());
        self.call(Shape::Copy, &[x.len() as u64, x.as_ptr() as u64, out.as_mut_ptr() as u64]);
        // SAFETY: the copy wrote `x.len()` elements.
        unsafe { out.set_len(x.len()) };
        out
    }

    /// The program's values as `ArrayProgram::values_at` gives them.
    pub fn values_at(&self, p: &ArrayProgram, inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls) -> Result<Values, (usize, &'static str)> {
        if inputs.len() != usize::from(p.inputs) || params.len() != usize::from(p.params) {
            return Err((0, "the run's columns or parameters are not the program's"));
        }
        let n = p.ops.len();
        let mut v = Values { arrays: vec![Vec::new(); n], parts: vec![Vec::new(); n], rounds: 1 };
        let mut ends: Vec<(usize, &Vec<(u16, u16)>)> = p.rounds.iter().map(|g| (usize::from(g.iter().map(|q| q.1).max().unwrap()), g)).collect();
        ends.sort_unstable_by_key(|e| e.0);
        let mut next_end = 0;
        for i in 0..n {
            self.step(p, i, &mut v, inputs, params, calls, None).map_err(|e| (i, e))?;
            while next_end < ends.len() && ends[next_end].0 == i {
                self.settle(p, ends[next_end].1, &mut v, inputs, params, calls)?;
                next_end += 1;
            }
        }
        Ok(v)
    }

    /// Whether arrays `a` and `b` differ, by the emitted verdict.
    fn differ(&self, a: &[i64], b: &[i64]) -> bool {
        a.len() != b.len() || self.call(Shape::Differ, &[a.len() as u64, a.as_ptr() as u64, b.as_ptr() as u64]) != 0
    }

    fn settle(&self, p: &ArrayProgram, group: &[(u16, u16)], v: &mut Values, inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls) -> Result<(), (usize, &'static str)> {
        let lo = usize::from(group.iter().map(|q| q.0).min().unwrap());
        let hi = usize::from(group.iter().map(|q| q.1).max().unwrap());
        let mut moved = vec![false; hi + 1];
        for i in lo..=hi {
            moved[i] = group.iter().any(|q| usize::from(q.0) == i) || p.ops[i].operands().iter().any(|&o| usize::from(o) >= lo && moved[usize::from(o)]);
        }
        let mut rounds = 1;
        loop {
            if group.iter().all(|&(c, x)| !self.differ(&v.arrays[usize::from(c)], &v.arrays[usize::from(x)])) {
                v.rounds = v.rounds.max(rounds);
                return Ok(());
            }
            if rounds >= MAX_ROUNDS {
                return Err((hi, "past the rounds' bound"));
            }
            rounds += 1;
            let nexts: Vec<(usize, Vec<i64>)> = group.iter().map(|&(c, x)| (usize::from(c), self.copied(&v.arrays[usize::from(x)]))).collect();
            for i in (lo..=hi).filter(|&i| moved[i]) {
                let carried = nexts.iter().find(|q| q.0 == i).map(|q| self.copied(&q.1));
                self.step(p, i, v, inputs, params, calls, carried).map_err(|e| (i, e))?;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn step(&self, p: &ArrayProgram, i: usize, v: &mut Values, inputs: &[&[i64]], params: &[i64], calls: &mut dyn Calls, carried: Option<Vec<i64>>) -> Result<(), &'static str> {
        let arrays = &v.arrays;
        let a = |k: u16| -> &[i64] { &arrays[usize::from(k)] };
        let scalar = |k: u16| -> Result<i64, &'static str> {
            match a(k) {
                [x] => Ok(*x),
                _ => Err("a scalar operand is not one element"),
            }
        };
        let width = |xs: &[&[i64]]| -> Result<usize, &'static str> {
            let long: Vec<usize> = xs.iter().map(|x| x.len()).filter(|&l| l != 1).collect();
            match long.first() {
                None => Ok(1),
                Some(&l) if long.iter().all(|&m| m == l) => Ok(l),
                _ => Err("operands of unequal lengths"),
            }
        };
        // An operand's pointer and its mask: a scalar is its first element for every `j`.
        let ptr = |x: &[i64]| x.as_ptr() as u64;
        let mask = |x: &[i64]| if x.len() == 1 { 0u64 } else { u64::MAX };
        let made = |out: &mut Vec<i64>, len: usize| {
            // SAFETY: the function wrote `len` elements.
            unsafe { out.set_len(len) }
        };
        let out: Vec<i64> = match &p.ops[i] {
            ArrayOp::Input(k) => self.copied(inputs[usize::from(*k)]),
            ArrayOp::Param(k) => vec![params[usize::from(*k)]],
            ArrayOp::Const(c) => vec![*c],
            ArrayOp::Len(x) => vec![a(*x).len() as i64],
            ArrayOp::Iota(x) => {
                let n = a(*x).len();
                let mut out = room(n);
                self.call(Shape::Iota, &[n as u64, out.as_mut_ptr() as u64]);
                made(&mut out, n);
                out
            }
            ArrayOp::Map(op, x, y) => {
                let (x, y) = (a(*x), a(*y));
                let n = width(&[x, y])?;
                let mut out = room(n);
                self.call(Shape::Map(*op), &[n as u64, ptr(x), mask(x), ptr(y), mask(y), out.as_mut_ptr() as u64]);
                made(&mut out, n);
                out
            }
            ArrayOp::Select(m, x, y) => {
                let (m, x, y) = (a(*m), a(*x), a(*y));
                let n = width(&[m, x, y])?;
                let mut out = room(n);
                self.call(Shape::Select, &[n as u64, ptr(m), mask(m), ptr(x), mask(x), ptr(y), mask(y), out.as_mut_ptr() as u64]);
                made(&mut out, n);
                out
            }
            ArrayOp::Scan { how, excl, a: x, seg } => {
                let x = a(*x);
                let flags = match seg {
                    Some(s) if a(*s).len() != x.len() => return Err("a scan's flags are not as long as its values"),
                    Some(s) => ptr(a(*s)),
                    None => 0,
                };
                let n = x.len();
                let mut out = room(n);
                self.call(Shape::Scan { how: *how, excl: *excl, seg: seg.is_some() }, &[n as u64, ptr(x), flags, out.as_mut_ptr() as u64]);
                made(&mut out, n);
                out
            }
            ArrayOp::Sort(keys) => {
                let keys: Vec<&[i64]> = keys.iter().map(|&k| a(k)).collect();
                let n = keys[0].len();
                if keys.iter().any(|k| k.len() != n) {
                    return Err("a sort's keys of unequal lengths");
                }
                let ptrs: Vec<u64> = keys.iter().map(|k| ptr(k)).collect();
                let (mut out, mut tmp, mut hist) = (room(n), room(n), room(256));
                self.call(Shape::Sort, &[n as u64, keys.len() as u64, ptrs.as_ptr() as u64, out.as_mut_ptr() as u64, tmp.as_mut_ptr() as u64, hist.as_mut_ptr() as u64]);
                made(&mut out, n);
                out
            }
            ArrayOp::Gather(s, idx) => {
                let (s, idx) = (a(*s), a(*idx));
                let n = idx.len();
                let mut out = room(n);
                if self.call(Shape::Gather, &[n as u64, ptr(s), s.len() as u64, ptr(idx), out.as_mut_ptr() as u64]) != 0 {
                    return Err("a gather's index outside its array");
                }
                made(&mut out, n);
                out
            }
            ArrayOp::Scatter { how, src, idx, n, init } => {
                let len = usize::try_from(scalar(*n)?).map_err(|_| "a scatter's negative length")?;
                let (src, idx) = (a(*src), a(*idx));
                let w = width(&[src, idx])?;
                let init = scalar(*init)?;
                let mut out = room(len);
                let mut written = vec![0u8; if *how == Combine::Set { len } else { 0 }];
                if self.call(Shape::Scatter(*how), &[w as u64, ptr(src), mask(src), ptr(idx), mask(idx), len as u64, init as u64, out.as_mut_ptr() as u64, written.as_mut_ptr() as u64]) != 0 {
                    return Err("two writes of a set scatter to one element");
                }
                made(&mut out, len);
                out
            }
            ArrayOp::Search { le, sorted, q } => {
                let (s, q) = (a(*sorted), a(*q));
                let mut out = room(q.len());
                if self.call(Shape::Search { le: *le }, &[s.len() as u64, ptr(s), q.len() as u64, ptr(q), out.as_mut_ptr() as u64]) != 0 {
                    return Err("a search of an array not sorted");
                }
                made(&mut out, q.len());
                out
            }
            ArrayOp::Compact(m) => {
                let m = a(*m);
                let mut out = room(m.len());
                let k = self.call(Shape::Compact, &[m.len() as u64, ptr(m), out.as_mut_ptr() as u64]);
                made(&mut out, k as usize);
                out
            }
            ArrayOp::Expand(c) => {
                let c = a(*c);
                let total = self.call(Shape::ExpandCount, &[c.len() as u64, ptr(c)]);
                if total < 0 {
                    return Err("a negative count");
                }
                let mut out = room(total as usize);
                self.call(Shape::ExpandFill, &[c.len() as u64, ptr(c), out.as_mut_ptr() as u64]);
                made(&mut out, total as usize);
                out
            }
            ArrayOp::Concat(x, y) => {
                let (x, y) = (a(*x), a(*y));
                let mut out = room(x.len() + y.len());
                self.call(Shape::Copy, &[x.len() as u64, ptr(x), out.as_mut_ptr() as u64]);
                // SAFETY: `out` has room for both.
                let second = unsafe { out.as_mut_ptr().add(x.len()) };
                self.call(Shape::Copy, &[y.len() as u64, ptr(y), second as u64]);
                made(&mut out, x.len() + y.len());
                out
            }
            ArrayOp::Call { f, args, env } => {
                let fun = &p.funs[usize::from(*f)];
                let xs: Vec<&[i64]> = args.iter().map(|&x| a(x)).collect();
                let env: Vec<&[i64]> = env.iter().map(|&x| a(x)).collect();
                let w = if xs.is_empty() { 1 } else { width(&xs)? };
                let fields = usize::from(fun.fields);
                let mut each = Vec::with_capacity(w);
                calls.call_many(*f, fun, &xs, w, &env, &mut each)?;
                let mut counts = Vec::with_capacity(w);
                let mut rows = Vec::new();
                for made in each {
                    if fields == 0 || made.len() % fields != 0 {
                        if fields == 0 && made.is_empty() {
                            counts.push(0);
                            continue;
                        }
                        return Err("a call's results are not whole rows");
                    }
                    counts.push((made.len() / fields) as i64);
                    rows.extend(made);
                }
                v.parts[i] = (0..fields).map(|k| rows.iter().skip(k).step_by(fields).copied().collect()).collect();
                counts
            }
            ArrayOp::Part(c, k) => self.copied(&v.parts[usize::from(*c)][usize::from(*k)]),
            ArrayOp::Carry(init) => match carried {
                Some(x) => x,
                None => self.copied(a(*init)),
            },
        };
        v.arrays[i] = out;
        Ok(())
    }
}
