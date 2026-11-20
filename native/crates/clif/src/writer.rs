//! The tape's writer in CLIF (decision 0434 §6): format 16's ops (`open`, `close`, `leaf`, `commit`, `retag`,
//! `set_start`, `finish`) written once, for the Rust backend, the JIT and the kernels, over one region of memory
//! whose layout is fixed when it is made: the writer's `State`, then its parts, each named by its offset from the
//! state and its capacity. The reference is `rerail_tape_format::f16::reference::Writer`, written by hand, which these
//! ops write the same bytes as; the tape crate's writer (`f16::Writer`) is these ops.
//!
//! An op returns 0, or its failure (`fail`): the first is kept in the state, and a full part names itself, its
//! op having written nothing past the part's capacity. A cell of the tape is written as a word: its bytes in a
//! 64-bit load and store, the bytes past it kept, so a part's capacity has 8 bytes of room past its last cell.

/// Nodes a block (format 16's `Z`).
pub const Z: u32 = 1024;
/// Nodes a checkpoint (format 16's `K`).
pub const K: u32 = 8;
/// Nodes a side-list part's sub-block (an entry's offset is a byte; format 16's `BLOCK`).
pub const BLOCK: u32 = 256;
/// A format 16 tape's header (format 16's `HEAD16`).
pub const HEAD16: u32 = 120;
/// The most nodes a tape holds (format 16's `MAX_NODES`).
pub const MAX_NODES: u32 = 1 << 30;
/// No node.
pub const NONE: u32 = u32::MAX;
/// Format 16's magic, as a little-endian word.
pub const MAGIC16: u64 = u64::from_le_bytes(*b"TMPTAPB6");

/// An open node: its index, its children so far, its last child, its start.
pub const OPEN: u32 = 24;
/// A block's entry: its room (from the tape's start), its side area (`NONE` once final), whether it is final.
pub const TABLE: u32 = 16;
/// A block's laid parts: four (start in `laid`, length), u32s.
pub const LAID_AT: u32 = 32;
/// A side area: four lists' entry counts (u32s), then each list's node offsets (u16s, `Z`) and values (u64s, `Z`).
pub const SIDE_LIST: u32 = 2 * Z + 8 * Z;
pub const SIDE_AREA: u32 = 16 + 4 * SIDE_LIST;

/// An op's failures. A full part's is `FULL` plus the part's number (`State`'s order of parts).
pub mod fail {
    /// A start or end past the tape's bounds (before the last start, past the source, too many nodes).
    pub const BOUNDS: u32 = 1;
    /// A node after the root's subtree closed.
    pub const OUTSIDE: u32 = 2;
    /// An edit of a node in a block already final.
    pub const FINAL: u32 = 3;
    /// A start set of a node not open.
    pub const NOT_OPEN: u32 = 4;
    /// A close with no node open.
    pub const NO_OPEN: u32 = 5;
    /// A finish with nodes open.
    pub const UNCLOSED: u32 = 6;
    /// A node whose key the language's table does not hold (tier 2's, decision 0434 §6, R17).
    pub const NUMBER: u32 = 7;
    /// An edit of a node tier 2 already passed to the ops (R17): a commit point that was not final.
    pub const PASSED: u32 = 8;
    /// A part with no room: `FULL + its number`.
    pub const FULL: u32 = 0x100;
}

/// One part of the writer's region: where it lies (an offset from the state's address), its capacity and what it
/// holds, in bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Part {
    pub at: i64,
    pub cap: u64,
    pub len: u64,
}

/// A piece's entry in 16z's piece index (`part::PIECES`): its start in the payload, its stored length, its raw
/// length, its kind (`RAW`, `LZ4`, `REPEAT`), the next piece in its bucket of the seen pieces (`NONE` for none),
/// u32s from byte 0; its bytes' hash, a u64 at byte 24.
pub const PIECE: u32 = 32;
/// 16z's piece kinds.
pub const RAW: u32 = 0;
pub const LZ4: u32 = 1;
pub const REPEAT: u32 = 2;
/// The smallest piece a repeat names, and the smallest LZ4 is tried on (format 16's `REPEAT_MIN`, `LZ4_MIN`).
pub const REPEAT_MIN: u32 = 8;
pub const LZ4_MIN: u32 = 16;
/// 16z's magic, as a little-endian word.
pub const MAGIC16Z: u64 = u64::from_le_bytes(*b"TMPTAPBZ");
/// The LZ4 compressor's tables (`part::LZ`): a hash of `1 << LZ_HASH_LOG` u32s, then a chain of 64 Ki u16s.
pub const LZ_HASH_LOG: u32 = 15;
pub const LZ_TABLES: u32 = (4 << LZ_HASH_LOG) + 2 * 65536;
/// LZ4 HC's level: its chain entries searched a position (decision 0434 §6).
pub const LZ_ATTEMPTS: u32 = 8;
/// The most bytes LZ4 writes for `n` (`LZ4_compressBound`).
pub const fn lz4_bound(n: u64) -> u64 {
    n + n / 255 + 16
}

/// The parts, in their order in `State` (a full part's failure is `fail::FULL` plus its number).
pub mod part {
    pub const KINDS: u32 = 0;
    pub const OPEN: u32 = 1;
    pub const OUT: u32 = 2;
    pub const TABLE: u32 = 3;
    pub const SIDES: u32 = 4;
    pub const FREE: u32 = 5;
    pub const LAID: u32 = 6;
    pub const LAID_AT: u32 = 7;
    pub const PRESENCE: u32 = 8;
    pub const LINES: u32 = 9;
    /// 16z's: the blocks' rooms, and the ones given back (u32 offsets).
    pub const ROOMS: u32 = 10;
    pub const ROOM_FREE: u32 = 11;
    /// 16z's piece index (`PIECE` bytes an entry), the seen pieces' buckets (u32s, `NONE` for none), the
    /// checkpoints, the LZ4 compressor's tables.
    pub const PIECES: u32 = 12;
    pub const SEEN: u32 = 13;
    pub const CKPT: u32 = 14;
    pub const LZ: u32 = 15;
    /// How many.
    pub const COUNT: usize = 16;
}

/// The writer's state, at the start of its region: the document's constants, the writer's variables, its parts.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct State {
    /// Each array's escape: its widest value, which says the value is its side list's.
    pub esc: [u64; 3],
    pub source_len: u64,
    /// The last node's start.
    pub prev_start: u64,
    /// The slot, gap and length widths; the checkpoints'.
    pub w: [u32; 3],
    pub cw: u32,
    /// The side lists' value widths.
    pub vw: [u32; 4],
    /// The code byte's cutoff, and the numbers a prefix byte covers.
    pub cut: u32,
    pub per: u32,
    /// A block's bytes, and where each of its columns starts in it.
    pub stride: u32,
    pub at: [u32; 5],
    /// 1: a presence a block is kept; 2: the tape is 16z.
    pub flags: u32,
    /// The presence's words of kinds and of fields.
    pub kw: u32,
    pub fw: u32,
    /// The header's language tag, revision mark, and the source's newlines.
    pub tag: u32,
    pub mark: u32,
    pub newlines: u32,
    /// The nodes so far; the first not yet committed.
    pub n: u32,
    pub frontier: u32,
    /// The first failure, and its node.
    pub bad: u32,
    pub bad_node: u32,
    /// The open nodes; the blocks begun.
    pub open_len: u32,
    pub blocks: u32,
    /// 16z: the seen pieces' buckets (a power of two).
    pub zbuckets: u32,
    /// The LZ4 compressor's: the last piece's first index and its length (a reused liblz4 context's `dictLimit`
    /// and its input), the next index to insert, the last match's offset and back count.
    pub zbase: u32,
    pub zprev: u32,
    pub znext: u32,
    pub zoff: u32,
    pub zback: u32,
    /// Each number's kind and field (a u32: kind, then field << 16), read only.
    pub kinds: Part,
    pub open: Part,
    /// The tape: its header, its blocks, then (at `finish`) its tail.
    pub out: Part,
    pub table: Part,
    /// The side areas made; the ones given back (u32s).
    pub sides: Part,
    pub free: Part,
    /// Each final block's side-list parts as stored, and where they lie.
    pub laid: Part,
    pub laid_at: Part,
    /// The document's presence (u64s), then a block's each where kept.
    pub presence: Part,
    /// The source's newline index, given.
    pub lines: Part,
    pub rooms: Part,
    pub room_free: Part,
    pub pieces: Part,
    pub seen: Part,
    pub ckpt: Part,
    pub lz: Part,
}

/// Byte `k`'s offset of `State`'s fields.
pub mod at {
    use super::{Part, State};
    use std::mem::offset_of;
    pub const ESC: i32 = offset_of!(State, esc) as i32;
    pub const SOURCE_LEN: i32 = offset_of!(State, source_len) as i32;
    pub const PREV_START: i32 = offset_of!(State, prev_start) as i32;
    pub const W: i32 = offset_of!(State, w) as i32;
    pub const CW: i32 = offset_of!(State, cw) as i32;
    pub const VW: i32 = offset_of!(State, vw) as i32;
    pub const CUT: i32 = offset_of!(State, cut) as i32;
    pub const PER: i32 = offset_of!(State, per) as i32;
    pub const STRIDE: i32 = offset_of!(State, stride) as i32;
    pub const AT: i32 = offset_of!(State, at) as i32;
    pub const FLAGS: i32 = offset_of!(State, flags) as i32;
    pub const KW: i32 = offset_of!(State, kw) as i32;
    pub const FW: i32 = offset_of!(State, fw) as i32;
    pub const TAG: i32 = offset_of!(State, tag) as i32;
    pub const MARK: i32 = offset_of!(State, mark) as i32;
    pub const NEWLINES: i32 = offset_of!(State, newlines) as i32;
    pub const N: i32 = offset_of!(State, n) as i32;
    pub const FRONTIER: i32 = offset_of!(State, frontier) as i32;
    pub const BAD: i32 = offset_of!(State, bad) as i32;
    pub const BAD_NODE: i32 = offset_of!(State, bad_node) as i32;
    pub const OPEN_LEN: i32 = offset_of!(State, open_len) as i32;
    pub const BLOCKS: i32 = offset_of!(State, blocks) as i32;
    pub const ZBUCKETS: i32 = offset_of!(State, zbuckets) as i32;
    pub const ZBASE: i32 = offset_of!(State, zbase) as i32;
    pub const ZPREV: i32 = offset_of!(State, zprev) as i32;
    pub const ZNEXT: i32 = offset_of!(State, znext) as i32;
    pub const ZOFF: i32 = offset_of!(State, zoff) as i32;
    pub const ZBACK: i32 = offset_of!(State, zback) as i32;
    pub const PARTS: i32 = offset_of!(State, kinds) as i32;
    pub const PART: i32 = std::mem::size_of::<Part>() as i32;
    pub const P_AT: i32 = offset_of!(Part, at) as i32;
    pub const P_CAP: i32 = offset_of!(Part, cap) as i32;
    pub const P_LEN: i32 = offset_of!(Part, len) as i32;
}

const _: () = assert!(at::PARTS % 8 == 0 && at::PARTS + part::COUNT as i32 * at::PART == std::mem::size_of::<State>() as i32);

/// The bytes a row has before column `r` (0 codes, 1 slots, 2 gaps, 3 lengths, 4 checkpoints).
pub fn before(w: [u32; 3], r: usize) -> u32 {
    [0, 1, 1 + w[0], 1 + w[0] + w[1], 1 + w[0] + w[1] + w[2]][r]
}

/// A block's bytes.
pub fn stride_of(w: [u32; 3], cw: u32) -> u32 {
    Z * before(w, 4) + Z / K * cw
}

#[cfg(feature = "ir")]
pub use ops::*;

#[cfg(feature = "ir")]
mod ops {
    use super::*;
    use crate::reads::{Callee, Family};
    use cranelift_codegen::ir::condcodes::IntCC;
    use cranelift_codegen::ir::types::{I32, I64};
    use cranelift_codegen::ir::{self, AbiParam, Function, InstBuilder, MemFlags, Signature, UserFuncName, Value};
    use cranelift_codegen::isa::CallConv;
    use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};

    /// An op: its name, its parameters after the state's address, its body. Each returns an i32: 0, a failure, or
    /// (`w_value`'s) a value's low word, which `w_value_hi` is not needed for (values are u64s, returned as i64).
    pub struct Op {
        pub name: &'static str,
        pub params: &'static [ir::Type],
        pub ret: ir::Type,
        pub public: bool,
        /// An op a public one calls at every node (or the compressor at every position): inlined where it is
        /// called. Another (a failure, a block's or a piece's work) stays a call, out of the per-node path.
        pub hot: bool,
        body: fn(&mut W<'_, '_>),
    }

    /// Every op: the public ones, then the ones they call.
    pub const OPS: &[Op] = &[
        Op { name: "w_open", params: &[I32, I64], ret: I32, public: true, hot: false, body: open },
        Op { name: "w_close", params: &[I64], ret: I32, public: true, hot: false, body: close },
        Op { name: "w_leaf", params: &[I32, I64, I64], ret: I32, public: true, hot: false, body: leaf },
        Op { name: "w_commit", params: &[], ret: I32, public: true, hot: false, body: commit },
        Op { name: "w_retag", params: &[I32, I32], ret: I32, public: true, hot: false, body: retag },
        Op { name: "w_set_start", params: &[I32, I64], ret: I32, public: true, hot: false, body: set_start },
        Op { name: "w_finish", params: &[], ret: I32, public: true, hot: false, body: finish },
        Op { name: "w_finish_blocks", params: &[], ret: I32, public: true, hot: false, body: finish_blocks },
        Op { name: "w_finish_tail", params: &[], ret: I32, public: true, hot: false, body: finish_tail },
        Op { name: "w_fail", params: &[I32, I32], ret: I32, public: false, hot: false, body: fail_op },
        Op { name: "w_put", params: &[I32, I32, I64], ret: I32, public: false, hot: true, body: put },
        Op { name: "w_value", params: &[I32, I32], ret: I64, public: false, hot: true, body: value },
        Op { name: "w_put_code", params: &[I32, I32], ret: I32, public: false, hot: true, body: put_code },
        Op { name: "w_new_block", params: &[I32], ret: I32, public: false, hot: false, body: new_block },
        Op { name: "w_settle", params: &[I32], ret: I32, public: false, hot: true, body: settle },
        Op { name: "w_finalize", params: &[I32, I32], ret: I32, public: false, hot: false, body: finalize },
        Op { name: "w_lay", params: &[I32, I32, I32], ret: I32, public: false, hot: false, body: lay },
        Op { name: "w_find", params: &[I64, I32, I32], ret: I32, public: false, hot: false, body: find },
        Op { name: "w_copy", params: &[I64, I64, I64], ret: I32, public: false, hot: false, body: copy },
        Op { name: "w_zero", params: &[I64, I64], ret: I32, public: false, hot: false, body: zero },
        Op { name: "w_fill", params: &[I64, I64, I64], ret: I32, public: false, hot: false, body: fill },
        Op { name: "w_zblock", params: &[I32, I32], ret: I32, public: false, hot: false, body: zblock },
        Op { name: "w_zpiece", params: &[I32, I64, I64], ret: I32, public: false, hot: false, body: zpiece },
        Op { name: "w_zfinish", params: &[I32], ret: I32, public: false, hot: false, body: zfinish },
        Op { name: "w_lz4", params: &[I64, I64, I64], ret: I64, public: true, hot: false, body: lz4 },
        Op { name: "w_lz4_find", params: &[I64, I64, I64, I64, I64], ret: I64, public: false, hot: true, body: lz4_find },
        Op { name: "w_hash", params: &[I64, I64], ret: I64, public: false, hot: false, body: hash },
        Op { name: "w_eq", params: &[I64, I64, I64], ret: I32, public: false, hot: false, body: eq },
    ];

    pub fn op(name: &str) -> Option<&'static Op> {
        OPS.iter().find(|op| op.name == name)
    }

    /// Every op declared in `module` with `linkage` and defined there, each calling the others by its id there (as
    /// `declare_func_in_func`): the JIT's (`jit::Writer`), and a module whose own code calls the ops (tier 2's,
    /// decision 0434 §6, R17). Each op's id and signature, by name.
    pub fn define_ops<M: cranelift_module::Module>(module: &mut M, linkage: cranelift_module::Linkage) -> Result<std::collections::HashMap<&'static str, (cranelift_module::FuncId, Signature)>, String> {
        use cranelift_module::FuncId;
        // An op's call of another: the module's function of that name, by its id.
        struct ByName<'m> {
            ids: &'m std::collections::HashMap<&'static str, (FuncId, Signature)>,
        }
        impl Callee for ByName<'_> {
            fn func_ref(&mut self, name: &str, signature: &Signature, func: &mut Function) -> ir::FuncRef {
                let name = func.declare_imported_user_function(ir::UserExternalName::new(0, self.ids[name].0.as_u32()));
                let signature = func.import_signature(signature.clone());
                func.import_function(ir::ExtFuncData { name: ir::ExternalName::User(name), signature, colocated: true })
            }
        }
        let cc = module.isa().default_call_conv();
        let mut ids = std::collections::HashMap::new();
        for op in OPS {
            let s = signature(op, cc);
            let id = module.declare_function(op.name, linkage, &s).map_err(|e| format!("{}: {e:?}", op.name))?;
            ids.insert(op.name, (id, s));
        }
        for op in OPS {
            let id = ids[op.name].0;
            let func = build(op, cc, UserFuncName::user(0, id.as_u32()), &mut ByName { ids: &ids });
            let mut context = module.make_context();
            context.func = func;
            module.define_function(id, &mut context).map_err(|e| format!("{}: {e:?}", op.name))?;
        }
        Ok(ids)
    }

    /// An op's signature: the state's address, then its parameters.
    pub fn signature(op: &Op, call_conv: CallConv) -> Signature {
        let mut s = Signature::new(call_conv);
        s.params.push(AbiParam::new(I64));
        s.params.extend(op.params.iter().map(|&t| AbiParam::new(t)));
        s.returns.push(AbiParam::new(op.ret));
        s
    }

    /// The op's body as a function named `name`, its calls of other ops through `callee`.
    pub fn build(op: &Op, call_conv: CallConv, name: UserFuncName, callee: &mut dyn Callee) -> Function {
        let mut func = Function::with_name_signature(name, signature(op, call_conv));
        let mut context = FunctionBuilderContext::new();
        let mut f = FunctionBuilder::new(&mut func, &mut context);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        f.switch_to_block(entry);
        let params = f.block_params(entry).to_vec();
        let mut w = W { f, p: params[0], args: params[1..].to_vec(), call_conv, callee };
        (op.body)(&mut w);
        w.f.seal_all_blocks();
        w.f.finalize();
        func
    }

    /// The ops' function numbers start here, past the reads' (`reads::READS`, which the backends look a
    /// function's family up in by its number).
    pub const ID_BASE: u32 = 1 << 20;

    /// The ops, each named by `ID_BASE` plus its place in `OPS`, with whether it is public.
    pub fn funcs() -> Vec<(u32, Function, &'static Op)> {
        OPS.iter().enumerate().map(|(k, op)| (ID_BASE + k as u32, build(op, CallConv::SystemV, UserFuncName::user(0, ID_BASE + k as u32), &mut ByIndex), op)).collect()
    }

    struct ByIndex;
    impl Callee for ByIndex {
        fn func_ref(&mut self, name: &str, signature: &Signature, func: &mut Function) -> ir::FuncRef {
            let index = ID_BASE + OPS.iter().position(|op| op.name == name).expect("an op") as u32;
            let name = func.declare_imported_user_function(ir::UserExternalName::new(0, index));
            let signature = func.import_signature(signature.clone());
            func.import_function(ir::ExtFuncData { name: ir::ExternalName::User(name), signature, colocated: true })
        }
    }

    /// The ops as Rust: the tape crate's `generated_writer.rs`, made again by `the_tape_crates_writer_is_this`.
    pub fn rust_source() -> Result<String, String> {
        let funcs = funcs();
        let entries: Vec<(u32, &str)> = funcs.iter().filter(|(_, _, op)| op.public).map(|(k, _, op)| (*k, op.name)).collect();
        let funcs: Vec<(u32, Function, bool)> = funcs.into_iter().map(|(k, f, op)| (k, f, op.hot)).collect();
        let body = crate::rust::translate_ops(&funcs, &entries)?;
        let mut out = String::from(
            "// Generated by rerail_clif::writer::rust_source from the tape's writer in CLIF (decision 0434 §6). Do not\n\
             // edit: `RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_tape_crates_writer_is_this` writes it again.\n\
             #![allow(dead_code, unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_parens, clippy::all)]\n\n",
        );
        out.push_str(&body);
        let h = crate::rust::fingerprint(&out);
        out.push_str(&format!("\n/// This source's fingerprint (`rust::fingerprint` of what comes before).\npub const SOURCE: u64 = {h:#x};\n"));
        Ok(out)
    }

    /// The ops as UKL (decisions 0434 §6 R15, 0440 §3.3): the library's `<ukl/writer.h>`, which a kernel that writes a
    /// tape includes, made again by `the_tape_crates_writer_kernels_are_these`. Each takes the region's buffer and the
    /// state's offset in it, as a relation's reads take theirs.
    pub fn ukl_source() -> Result<String, String> {
        let funcs: Vec<(u32, Function, &str)> = funcs().into_iter().map(|(k, f, op)| (k, f, op.name)).collect();
        let body = crate::ukl::translate_reads(&funcs, "clif_", Family::Relation)?;
        Ok(format!(
            "// Generated by rerail_clif::writer::ukl_source from the tape's writer in CLIF (decision 0434 §6). Do not\n\
             // edit: `RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_tape_crates_writer_kernels_are_these` writes\n\
             // it again.\n#pragma once\n#include <ukl/memory.h>\n#include <ukl/clif.h>\n{body}"
        ))
    }

    /// A body's builder: the state's address `p`, the op's arguments.
    pub struct W<'a, 'b> {
        f: FunctionBuilder<'a>,
        p: Value,
        args: Vec<Value>,
        call_conv: CallConv,
        callee: &'b mut dyn Callee,
    }

    /// The state's own words: aligned.
    fn state() -> MemFlags {
        MemFlags::new().with_notrap().with_aligned()
    }
    /// The parts' bytes: at any alignment.
    fn bytes() -> MemFlags {
        MemFlags::new().with_notrap()
    }

    impl W<'_, '_> {
        fn arg(&self, k: usize) -> Value {
            self.args[k]
        }
        fn c32(&mut self, v: i64) -> Value {
            self.f.ins().iconst(I32, v)
        }
        fn c64(&mut self, v: i64) -> Value {
            self.f.ins().iconst(I64, v)
        }
        fn u64of(&mut self, v: Value) -> Value {
            self.f.ins().uextend(I64, v)
        }
        /// The state's u32 at `off`.
        fn get32(&mut self, off: i32) -> Value {
            self.f.ins().load(I32, state(), self.p, off)
        }
        fn set32(&mut self, off: i32, v: Value) {
            self.f.ins().store(state(), v, self.p, off);
        }
        fn get64(&mut self, off: i32) -> Value {
            self.f.ins().load(I64, state(), self.p, off)
        }
        fn set64(&mut self, off: i32, v: Value) {
            self.f.ins().store(state(), v, self.p, off);
        }
        /// The state's u32 `k` of the array at `off`.
        fn get32_at(&mut self, off: i32, k: Value) -> Value {
            let k = self.u64of(k);
            let k = self.f.ins().ishl_imm(k, 2);
            let a = self.f.ins().iadd(self.p, k);
            self.f.ins().load(I32, state(), a, off)
        }
        fn get64_at(&mut self, off: i32, k: Value) -> Value {
            let k = self.u64of(k);
            let k = self.f.ins().ishl_imm(k, 3);
            let a = self.f.ins().iadd(self.p, k);
            self.f.ins().load(I64, state(), a, off)
        }
        /// Part `k`'s field at `field` (`at::P_AT`, `P_CAP`, `P_LEN`).
        fn part_field(&mut self, k: u32, field: i32) -> i32 {
            at::PARTS + k as i32 * at::PART + field
        }
        /// Part `k`'s address.
        fn part(&mut self, k: u32) -> Value {
            let off = self.part_field(k, at::P_AT);
            let rel = self.get64(off);
            self.f.ins().iadd(self.p, rel)
        }
        fn part_cap(&mut self, k: u32) -> Value {
            let off = self.part_field(k, at::P_CAP);
            self.get64(off)
        }
        fn part_len(&mut self, k: u32) -> Value {
            let off = self.part_field(k, at::P_LEN);
            self.get64(off)
        }
        fn set_part_len(&mut self, k: u32, v: Value) {
            let off = self.part_field(k, at::P_LEN);
            self.set64(off, v);
        }
        fn ld(&mut self, ty: ir::Type, a: Value, off: i32) -> Value {
            self.f.ins().load(ty, bytes(), a, off)
        }
        fn ld8(&mut self, a: Value, off: i32) -> Value {
            self.f.ins().uload8(I32, bytes(), a, off)
        }
        fn ld16(&mut self, a: Value, off: i32) -> Value {
            self.f.ins().uload16(I32, bytes(), a, off)
        }
        fn st(&mut self, v: Value, a: Value, off: i32) {
            self.f.ins().store(bytes(), v, a, off);
        }
        fn st8(&mut self, v: Value, a: Value, off: i32) {
            self.f.ins().istore8(bytes(), v, a, off);
        }
        fn st16(&mut self, v: Value, a: Value, off: i32) {
            self.f.ins().istore16(bytes(), v, a, off);
        }
        /// `base + k * size` (k a u32).
        fn index(&mut self, base: Value, k: Value, size: i64) -> Value {
            let k = self.u64of(k);
            let k = self.f.ins().imul_imm(k, size);
            self.f.ins().iadd(base, k)
        }
        /// The low `w` bytes' mask (`w` an i64, all ones at 8).
        fn mask(&mut self, w: Value) -> Value {
            let bits = self.f.ins().ishl_imm(w, 3);
            let eight = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, w, 8);
            let one = self.c64(1);
            let m = self.f.ins().ishl(one, bits);
            let m = self.f.ins().iadd_imm(m, -1);
            let all = self.c64(-1);
            self.f.ins().select(eight, all, m)
        }
        /// The cell of `w` bytes (an i64) at `a`, written as a word: its bytes `v`'s, the bytes past it kept.
        fn cell_put(&mut self, a: Value, w: Value, v: Value) {
            let m = self.mask(w);
            let old = self.ld(I64, a, 0);
            let keep = self.f.ins().band_not(old, m);
            let v = self.f.ins().band(v, m);
            let new = self.f.ins().bor(keep, v);
            self.st(new, a, 0);
        }
        fn cell_get(&mut self, a: Value, w: Value) -> Value {
            let m = self.mask(w);
            let v = self.ld(I64, a, 0);
            self.f.ins().band(v, m)
        }
        fn var(&mut self, ty: ir::Type, init: Value) -> Variable {
            let v = self.f.declare_var(ty);
            self.f.def_var(v, init);
            v
        }
        fn get(&mut self, v: Variable) -> Value {
            self.f.use_var(v)
        }
        fn set(&mut self, v: Variable, x: Value) {
            self.f.def_var(v, x);
        }
        /// `then` where `cond` holds.
        fn if_(&mut self, cond: Value, then: impl FnOnce(&mut Self)) {
            let (yes, done) = (self.f.create_block(), self.f.create_block());
            self.f.ins().brif(cond, yes, &[], done, &[]);
            self.f.switch_to_block(yes);
            then(self);
            self.f.ins().jump(done, &[]);
            self.f.switch_to_block(done);
        }
        fn if_else(&mut self, cond: Value, then: impl FnOnce(&mut Self), otherwise: impl FnOnce(&mut Self)) {
            let (yes, no, done) = (self.f.create_block(), self.f.create_block(), self.f.create_block());
            self.f.ins().brif(cond, yes, &[], no, &[]);
            self.f.switch_to_block(yes);
            then(self);
            self.f.ins().jump(done, &[]);
            self.f.switch_to_block(no);
            otherwise(self);
            self.f.ins().jump(done, &[]);
            self.f.switch_to_block(done);
        }
        /// `body` while `cond` holds.
        fn while_(&mut self, cond: impl Fn(&mut Self) -> Value, body: impl FnOnce(&mut Self)) {
            let (head, inner, done) = (self.f.create_block(), self.f.create_block(), self.f.create_block());
            self.f.ins().jump(head, &[]);
            self.f.switch_to_block(head);
            let c = cond(self);
            self.f.ins().brif(c, inner, &[], done, &[]);
            self.f.switch_to_block(inner);
            body(self);
            self.f.ins().jump(head, &[]);
            self.f.switch_to_block(done);
        }
        /// Returns `v`; what follows is unreached until a block jumps to it.
        fn ret(&mut self, v: Value) {
            self.f.ins().return_(&[v]);
            let after = self.f.create_block();
            self.f.switch_to_block(after);
        }
        /// Returns `v` where it is not 0.
        fn ret_if(&mut self, v: Value) {
            let c = self.f.ins().icmp_imm(IntCC::NotEqual, v, 0);
            self.if_(c, |w| w.ret(v));
        }
        /// The op `name` called with `args` after the state.
        fn call(&mut self, name: &str, args: &[Value]) -> Value {
            let op = op(name).expect("an op");
            let signature = signature(op, self.call_conv);
            let callee = self.callee.func_ref(name, &signature, self.f.func);
            let mut all = vec![self.p];
            all.extend_from_slice(args);
            let call = self.f.ins().call(callee, &all);
            self.f.inst_results(call)[0]
        }
        /// The failure `code` at `node`, the first kept: returns it.
        fn fail(&mut self, code: u32, node: Value) {
            let code = self.c32(i64::from(code));
            let r = self.call("w_fail", &[code, node]);
            self.ret(r);
        }
        fn fail_if(&mut self, cond: Value, code: u32, node: Value) {
            self.if_(cond, |w| w.fail(code, node));
        }
        /// Fails as full unless part `k` holds `need` bytes (an i64).
        fn room(&mut self, k: u32, need: Value) {
            let cap = self.part_cap(k);
            let over = self.f.ins().icmp(IntCC::UnsignedGreaterThan, need, cap);
            let none = self.c32(i64::from(NONE));
            self.fail_if(over, fail::FULL + k, none);
        }
        /// Block `b`'s table entry.
        fn entry(&mut self, b: Value) -> Value {
            let table = self.part(part::TABLE);
            self.index(table, b, i64::from(TABLE))
        }
        /// Block `b`'s room's address in the tape.
        fn room_of(&mut self, b: Value) -> Value {
            let e = self.entry(b);
            let room = self.ld(I32, e, 0);
            let room = self.u64of(room);
            let base = self.rooms();
            self.f.ins().iadd(base, room)
        }
        /// Whether the tape is 16z.
        fn z(&mut self) -> Value {
            let flags = self.get32(at::FLAGS);
            let z = self.f.ins().band_imm(flags, 2);
            self.f.ins().icmp_imm(IntCC::NotEqual, z, 0)
        }
        /// Where the blocks' rooms lie: in 16, the tape; in 16z, their own part.
        fn rooms(&mut self) -> Value {
            let z = self.z();
            let rooms = self.part(part::ROOMS);
            let out = self.part(part::OUT);
            self.f.ins().select(z, rooms, out)
        }
        /// Block `b`'s side area's address.
        fn area_of(&mut self, b: Value) -> Value {
            let e = self.entry(b);
            let area = self.ld(I32, e, 4);
            let sides = self.part(part::SIDES);
            self.index(sides, area, i64::from(SIDE_AREA))
        }
        /// List `q` of the area at `area`: its count's address, its offsets', its values'.
        fn list(&mut self, area: Value, q: Value) -> (Value, Value, Value) {
            let count = self.index(area, q, 4);
            let base = self.index(area, q, i64::from(SIDE_LIST));
            let js = self.f.ins().iadd_imm(base, 16);
            let vs = self.f.ins().iadd_imm(base, 16 + 2 * i64::from(Z));
            (count, js, vs)
        }
        /// The state's `w[a]` (a u32 index), as an i64.
        fn width(&mut self, a: Value) -> Value {
            let w = self.get32_at(at::W, a);
            self.u64of(w)
        }
        /// Column `r`'s offset in a room (`at[r]`), as an i64.
        fn col(&mut self, r: Value) -> Value {
            let c = self.get32_at(at::AT, r);
            self.u64of(c)
        }
        fn node_block(&mut self, i: Value) -> (Value, Value) {
            let b = self.f.ins().ushr_imm(i, Z.trailing_zeros() as i64);
            let j = self.f.ins().band_imm(i, i64::from(Z - 1));
            (b, j)
        }
        fn zero32(&mut self) -> Value {
            self.c32(0)
        }
        fn ok(&mut self) {
            let z = self.zero32();
            self.f.ins().return_(&[z]);
            let after = self.f.create_block();
            self.f.switch_to_block(after);
            let z = self.zero32();
            self.f.ins().return_(&[z]);
        }
    }

    // ---- the ops --------------------------------------------------------------------------------------------

    /// `w_fail(code, node)`: the first failure kept; returns `code`.
    fn fail_op(w: &mut W<'_, '_>) {
        let (code, node) = (w.arg(0), w.arg(1));
        let bad = w.get32(at::BAD);
        let first = w.f.ins().icmp_imm(IntCC::Equal, bad, 0);
        w.if_(first, |w| {
            w.set32(at::BAD, code);
            w.set32(at::BAD_NODE, node);
        });
        w.f.ins().return_(&[code]);
        let after = w.f.create_block();
        w.f.switch_to_block(after);
        w.f.ins().return_(&[code]);
    }

    /// `w_find(js, count, j)`: the first entry of the offsets at `js` (u16s, `count` of them, ascending) not below
    /// `j`.
    fn find(w: &mut W<'_, '_>) {
        let (js, count, j) = (w.arg(0), w.arg(1), w.arg(2));
        let zero = w.zero32();
        let lo = w.var(I32, zero);
        let hi = w.var(I32, count);
        w.while_(
            |w| {
                let (l, h) = (w.get(lo), w.get(hi));
                w.f.ins().icmp(IntCC::UnsignedLessThan, l, h)
            },
            |w| {
                let (l, h) = (w.get(lo), w.get(hi));
                let s = w.f.ins().iadd(l, h);
                let mid = w.f.ins().ushr_imm(s, 1);
                let a = w.index(js, mid, 2);
                let e = w.ld16(a, 0);
                let below = w.f.ins().icmp(IntCC::UnsignedLessThan, e, j);
                let up = w.f.ins().iadd_imm(mid, 1);
                let l2 = w.f.ins().select(below, up, l);
                let h2 = w.f.ins().select(below, h, mid);
                w.set(lo, l2);
                w.set(hi, h2);
            },
        );
        let l = w.get(lo);
        w.f.ins().return_(&[l]);
    }

    /// `w_copy(to, from, len)`: `len` bytes moved down or apart (`to <= from`, or not overlapping), a word at a
    /// time, the last word's bytes past `len` kept.
    fn copy(w: &mut W<'_, '_>) {
        let (to, from, len) = (w.arg(0), w.arg(1), w.arg(2));
        let zero = w.c64(0);
        let k = w.var(I64, zero);
        w.while_(
            |w| {
                let k = w.get(k);
                let k8 = w.f.ins().iadd_imm(k, 8);
                w.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, k8, len)
            },
            |w| {
                let kv = w.get(k);
                let s = w.f.ins().iadd(from, kv);
                let d = w.f.ins().iadd(to, kv);
                let v = w.ld(I64, s, 0);
                w.st(v, d, 0);
                let k8 = w.f.ins().iadd_imm(kv, 8);
                w.set(k, k8);
            },
        );
        let kv = w.get(k);
        let rest = w.f.ins().isub(len, kv);
        let some = w.f.ins().icmp_imm(IntCC::NotEqual, rest, 0);
        w.if_(some, |w| {
            let s = w.f.ins().iadd(from, kv);
            let d = w.f.ins().iadd(to, kv);
            let v = w.ld(I64, s, 0);
            w.cell_put(d, rest, v);
        });
        let z = w.zero32();
        w.f.ins().return_(&[z]);
    }

    /// `w_zero(to, len)`: `len` bytes (a multiple of 8) zeroed, a word at a time.
    fn zero(w: &mut W<'_, '_>) {
        let (to, len) = (w.arg(0), w.arg(1));
        let zero = w.c64(0);
        let k = w.var(I64, zero);
        w.while_(
            |w| {
                let k = w.get(k);
                w.f.ins().icmp(IntCC::UnsignedLessThan, k, len)
            },
            |w| {
                let kv = w.get(k);
                let d = w.f.ins().iadd(to, kv);
                let z = w.c64(0);
                w.st(z, d, 0);
                let k8 = w.f.ins().iadd_imm(kv, 8);
                w.set(k, k8);
            },
        );
        let z = w.zero32();
        w.f.ins().return_(&[z]);
    }

    /// `w_put(a, i, v)`: value `v` of node `i`'s cell in array `a` (0 slots, 1 gaps, 2 lengths), or its escape and
    /// its entry in its block's side list `a + 1`.
    fn put(w: &mut W<'_, '_>) {
        let (a, i, v) = (w.arg(0), w.arg(1), w.arg(2));
        let (b, j) = w.node_block(i);
        let esc = w.get64_at(at::ESC, a);
        let area = w.area_of(b);
        let q = w.f.ins().iadd_imm(a, 1);
        let (count_at, js, vs) = w.list(area, q);
        let count = w.ld(I32, count_at, 0);
        let escaped = w.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, v, esc);
        let cellv = w.f.ins().select(escaped, esc, v);
        w.if_else(
            escaped,
            |w| {
                // Entries come in row order but for an ancestor's late close.
                let k = w.call("w_find", &[js, count, j]);
                let inside = w.f.ins().icmp(IntCC::UnsignedLessThan, k, count);
                let ka = w.index(js, k, 2);
                let found = w.ld16(ka, 0);
                let same = w.f.ins().icmp(IntCC::Equal, found, j);
                let hit = w.f.ins().band(inside, same);
                w.if_else(
                    hit,
                    |w| {
                        let va = w.index(vs, k, 8);
                        w.st(v, va, 0);
                    },
                    |w| insert(w, count_at, js, vs, count, k, j, v),
                );
            },
            |w| {
                // A cell written again (`set_start`) drops the entry it had.
                let some = w.f.ins().icmp_imm(IntCC::NotEqual, count, 0);
                w.if_(some, |w| {
                    let last = w.f.ins().iadd_imm(count, -1);
                    let la = w.index(js, last, 2);
                    let lj = w.ld16(la, 0);
                    let reach = w.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, lj, j);
                    w.if_(reach, |w| {
                        let k = w.call("w_find", &[js, count, j]);
                        let inside = w.f.ins().icmp(IntCC::UnsignedLessThan, k, count);
                        let ka = w.index(js, k, 2);
                        let found = w.ld16(ka, 0);
                        let same = w.f.ins().icmp(IntCC::Equal, found, j);
                        let hit = w.f.ins().band(inside, same);
                        w.if_(hit, |w| remove(w, count_at, js, vs, count, k));
                    });
                });
            },
        );
        let wa = w.width(a);
        let room = w.room_of(b);
        let col = w.col(q);
        let j64 = w.u64of(j);
        let off = w.f.ins().imul(j64, wa);
        let cell = w.f.ins().iadd(room, col);
        let cell = w.f.ins().iadd(cell, off);
        w.cell_put(cell, wa, cellv);
        w.ok();
    }

    /// Entry `(j, v)` inserted at `k` of a list of `count`, the ones from `k` moved up one.
    #[allow(clippy::too_many_arguments)]
    fn insert(w: &mut W<'_, '_>, count_at: Value, js: Value, vs: Value, count: Value, k: Value, j: Value, v: Value) {
        let m = w.var(I32, count);
        w.while_(
            |w| {
                let m = w.get(m);
                w.f.ins().icmp(IntCC::UnsignedGreaterThan, m, k)
            },
            |w| {
                let mv = w.get(m);
                let below = w.f.ins().iadd_imm(mv, -1);
                let from = w.index(js, below, 2);
                let e = w.ld16(from, 0);
                let to = w.index(js, mv, 2);
                w.st16(e, to, 0);
                let from = w.index(vs, below, 8);
                let x = w.ld(I64, from, 0);
                let to = w.index(vs, mv, 8);
                w.st(x, to, 0);
                w.set(m, below);
            },
        );
        let ka = w.index(js, k, 2);
        w.st16(j, ka, 0);
        let va = w.index(vs, k, 8);
        w.st(v, va, 0);
        let c = w.f.ins().iadd_imm(count, 1);
        w.st(c, count_at, 0);
    }

    /// Entry `k` of a list of `count` removed, the ones after it moved down one.
    fn remove(w: &mut W<'_, '_>, count_at: Value, js: Value, vs: Value, count: Value, k: Value) {
        let last = w.f.ins().iadd_imm(count, -1);
        let m = w.var(I32, k);
        w.while_(
            |w| {
                let m = w.get(m);
                w.f.ins().icmp(IntCC::UnsignedLessThan, m, last)
            },
            |w| {
                let mv = w.get(m);
                let above = w.f.ins().iadd_imm(mv, 1);
                let from = w.index(js, above, 2);
                let e = w.ld16(from, 0);
                let to = w.index(js, mv, 2);
                w.st16(e, to, 0);
                let from = w.index(vs, above, 8);
                let x = w.ld(I64, from, 0);
                let to = w.index(vs, mv, 8);
                w.st(x, to, 0);
                w.set(m, above);
            },
        );
        w.st(last, count_at, 0);
    }

    /// `w_value(a, i)`: node `i`'s value in array `a` as written: its cell, or its side-list entry where the cell
    /// is the escape.
    fn value(w: &mut W<'_, '_>) {
        let (a, i) = (w.arg(0), w.arg(1));
        let (b, j) = w.node_block(i);
        let wa = w.width(a);
        let room = w.room_of(b);
        let q = w.f.ins().iadd_imm(a, 1);
        let col = w.col(q);
        let j64 = w.u64of(j);
        let off = w.f.ins().imul(j64, wa);
        let cell = w.f.ins().iadd(room, col);
        let cell = w.f.ins().iadd(cell, off);
        let v = w.cell_get(cell, wa);
        let esc = w.get64_at(at::ESC, a);
        let plain = w.f.ins().icmp(IntCC::UnsignedLessThan, v, esc);
        w.if_(plain, |w| w.ret(v));
        let area = w.area_of(b);
        let (count_at, js, vs) = w.list(area, q);
        let count = w.ld(I32, count_at, 0);
        let k = w.call("w_find", &[js, count, j]);
        let inside = w.f.ins().icmp(IntCC::UnsignedLessThan, k, count);
        w.if_(inside, |w| {
            let va = w.index(vs, k, 8);
            let x = w.ld(I64, va, 0);
            w.ret(x);
        });
        w.f.ins().return_(&[v]);
    }

    /// `w_put_code(i, number)`: node `i`'s code byte, and its side-list entry for `number`.
    fn put_code(w: &mut W<'_, '_>) {
        let (i, number) = (w.arg(0), w.arg(1));
        let (b, j) = w.node_block(i);
        let cut = w.get32(at::CUT);
        let room = w.room_of(b);
        let j64 = w.u64of(j);
        let code_at = w.f.ins().iadd(room, j64);
        let low = w.f.ins().icmp(IntCC::UnsignedLessThan, number, cut);
        w.if_else(
            low,
            |w| w.st8(number, code_at, 0),
            |w| {
                let per = w.get32(at::PER);
                let t = w.f.ins().isub(number, cut);
                let r = w.f.ins().urem(t, per);
                let r = w.u64of(r);
                let d = w.f.ins().udiv(t, per);
                let byte = w.f.ins().iadd(cut, d);
                w.st8(byte, code_at, 0);
                let area = w.area_of(b);
                let zero = w.zero32();
                let (count_at, js, vs) = w.list(area, zero);
                let count = w.ld(I32, count_at, 0);
                // Entries come in row order but for a retag.
                let k = w.call("w_find", &[js, count, j]);
                insert(w, count_at, js, vs, count, k, j, r);
            },
        );
        w.ok();
    }

    /// `w_new_block(b)`: room for block `b` at its place in the tape (zeroed), a side area, its table entry.
    fn new_block(w: &mut W<'_, '_>) {
        let b = w.arg(0);
        let stride = w.get32(at::STRIDE);
        let stride = w.u64of(stride);
        let b64 = w.u64of(b);
        // The block's room: in 16 its place in the tape; in 16z one given back, else a new one.
        let zero64 = w.c64(0);
        let roomv = w.var(I64, zero64);
        let z = w.z();
        w.if_else(
            z,
            |w| {
                let free = w.part_len(part::ROOM_FREE);
                let none = w.f.ins().icmp_imm(IntCC::Equal, free, 0);
                w.if_else(
                    none,
                    |w| {
                        let made = w.part_len(part::ROOMS);
                        let next = w.f.ins().iadd(made, stride);
                        w.room(part::ROOMS, next);
                        w.set_part_len(part::ROOMS, next);
                        w.set(roomv, made);
                    },
                    |w| {
                        let top = w.f.ins().iadd_imm(free, -4);
                        w.set_part_len(part::ROOM_FREE, top);
                        let f = w.part(part::ROOM_FREE);
                        let at = w.f.ins().iadd(f, top);
                        let r = w.ld(I32, at, 0);
                        let r = w.u64of(r);
                        w.set(roomv, r);
                    },
                );
            },
            |w| {
                let room = w.f.ins().imul(b64, stride);
                let room = w.f.ins().iadd_imm(room, i64::from(HEAD16));
                let end = w.f.ins().iadd(room, stride);
                w.room(part::OUT, end);
                let len = w.part_len(part::OUT);
                let grow = w.f.ins().icmp(IntCC::UnsignedLessThan, len, end);
                w.if_(grow, |w| w.set_part_len(part::OUT, end));
                w.set(roomv, room);
            },
        );
        let room = w.get(roomv);
        let te = w.f.ins().iadd_imm(b64, 1);
        let te = w.f.ins().imul_imm(te, i64::from(TABLE));
        w.room(part::TABLE, te);
        // A side area given back, else a new one.
        let free = w.part_len(part::FREE);
        let none = w.f.ins().icmp_imm(IntCC::Equal, free, 0);
        let zero = w.zero32();
        let area = w.var(I32, zero);
        w.if_else(
            none,
            |w| {
                let made = w.part_len(part::SIDES);
                let next = w.f.ins().iadd_imm(made, i64::from(SIDE_AREA));
                w.room(part::SIDES, next);
                w.set_part_len(part::SIDES, next);
                let d = w.c64(i64::from(SIDE_AREA));
                let k = w.f.ins().udiv(made, d);
                let k = w.f.ins().ireduce(I32, k);
                w.set(area, k);
            },
            |w| {
                let top = w.f.ins().iadd_imm(free, -4);
                w.set_part_len(part::FREE, top);
                let f = w.part(part::FREE);
                let at = w.f.ins().iadd(f, top);
                let k = w.ld(I32, at, 0);
                w.set(area, k);
            },
        );
        let base = w.rooms();
        let at = w.f.ins().iadd(base, room);
        w.call("w_zero", &[at, stride]);
        let tl = w.part_len(part::TABLE);
        let grow = w.f.ins().icmp(IntCC::UnsignedLessThan, tl, te);
        w.if_(grow, |w| w.set_part_len(part::TABLE, te));
        let e = w.entry(b);
        let room32 = w.f.ins().ireduce(I32, room);
        w.st(room32, e, 0);
        let a = w.get(area);
        w.st(a, e, 4);
        let z = w.zero32();
        w.st(z, e, 8);
        let ad = w.area_of(b);
        let z64 = w.c64(0);
        w.st(z64, ad, 0);
        w.st(z64, ad, 8);
        let nb = w.f.ins().iadd_imm(b, 1);
        w.set32(at::BLOCKS, nb);
        // The block's presence, where kept.
        let flags = w.get32(at::FLAGS);
        let keep = w.f.ins().band_imm(flags, 1);
        w.if_(keep, |w| {
            let kw = w.get32(at::KW);
            let fw = w.get32(at::FW);
            let words = w.f.ins().iadd(kw, fw);
            let words = w.u64of(words);
            let bytes = w.f.ins().ishl_imm(words, 3);
            let nb64 = w.f.ins().iadd_imm(b64, 2);
            let need = w.f.ins().imul(nb64, bytes);
            w.room(part::PRESENCE, need);
            let pl = w.part_len(part::PRESENCE);
            let grow = w.f.ins().icmp(IntCC::UnsignedLessThan, pl, need);
            w.if_(grow, |w| w.set_part_len(part::PRESENCE, need));
            let pres = w.part(part::PRESENCE);
            let b1 = w.f.ins().iadd_imm(b64, 1);
            let off = w.f.ins().imul(b1, bytes);
            let at = w.f.ins().iadd(pres, off);
            w.call("w_zero", &[at, bytes]);
        });
        w.ok();
    }

    /// `w_open(number, start)`: a node at the end, in preorder, open.
    fn open(w: &mut W<'_, '_>) {
        let (number, start) = (w.arg(0), w.arg(1));
        let i = w.get32(at::N);
        let prev = w.get64(at::PREV_START);
        let len = w.get64(at::SOURCE_LEN);
        let before = w.f.ins().icmp(IntCC::UnsignedLessThan, start, prev);
        let past = w.f.ins().icmp(IntCC::UnsignedGreaterThan, start, len);
        let i1 = w.f.ins().iadd_imm(i, 1);
        let many = w.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, i1, i64::from(MAX_NODES));
        let bad = w.f.ins().bor(before, past);
        let bad = w.f.ins().bor(bad, many);
        w.fail_if(bad, fail::BOUNDS, i);
        let open_len = w.get32(at::OPEN_LEN);
        let some = w.f.ins().icmp_imm(IntCC::NotEqual, i, 0);
        let none_open = w.f.ins().icmp_imm(IntCC::Equal, open_len, 0);
        let outside = w.f.ins().band(some, none_open);
        w.fail_if(outside, fail::OUTSIDE, i);
        let need = w.u64of(open_len);
        let need = w.f.ins().iadd_imm(need, 1);
        let need = w.f.ins().imul_imm(need, i64::from(OPEN));
        w.room(part::OPEN, need);
        let (b, j) = w.node_block(i);
        let first = w.f.ins().icmp_imm(IntCC::Equal, j, 0);
        w.if_(first, |w| {
            let r = w.call("w_new_block", &[b]);
            w.ret_if(r);
        });
        // The node before is a leaf unless it is the parent: its slot describes this node.
        let any = w.f.ins().icmp_imm(IntCC::NotEqual, open_len, 0);
        w.if_(any, |w| {
            let opens = w.part(part::OPEN);
            let top = w.f.ins().iadd_imm(open_len, -1);
            let t = w.index(opens, top, i64::from(OPEN));
            let p = w.ld(I32, t, 0);
            let idx = w.ld(I32, t, 4);
            let last = w.ld(I32, t, 8);
            let idx1 = w.f.ins().iadd_imm(idx, 1);
            w.st(idx1, t, 4);
            w.st(i, t, 8);
            let im1 = w.f.ins().iadd_imm(i, -1);
            let leaf = w.f.ins().icmp(IntCC::NotEqual, im1, p);
            w.if_(leaf, |w| {
                let mk = w.f.ins().band_imm(idx, i64::from(K - 1));
                let checkpoint = w.f.ins().icmp_imm(IntCC::Equal, mk, 0);
                let dp = w.f.ins().isub(i, p);
                let dp = w.u64of(dp);
                let a = w.f.ins().ishl_imm(dp, 2);
                let a = w.f.ins().bor_imm(a, 3);
                let dl = w.f.ins().isub(i, last);
                let dl = w.u64of(dl);
                let c = w.f.ins().ishl_imm(dl, 2);
                let c = w.f.ins().bor_imm(c, 1);
                let v = w.f.ins().select(checkpoint, a, c);
                let zero = w.zero32();
                let r = w.call("w_put", &[zero, im1, v]);
                w.ret_if(r);
            });
        });
        let r = w.call("w_put_code", &[i, number]);
        w.ret_if(r);
        let gap = w.f.ins().isub(start, prev);
        let root = w.f.ins().icmp_imm(IntCC::Equal, i, 0);
        let z64 = w.c64(0);
        let gap = w.f.ins().select(root, z64, gap);
        let one = w.c32(1);
        let r = w.call("w_put", &[one, i, gap]);
        w.ret_if(r);
        let mk = w.f.ins().band_imm(j, i64::from(K - 1));
        let checkpoint = w.f.ins().icmp_imm(IntCC::Equal, mk, 0);
        w.if_(checkpoint, |w| put_checkpoint(w, b, j, start));
        w.set64(at::PREV_START, start);
        w.set32(at::N, i1);
        let opens = w.part(part::OPEN);
        let t = w.index(opens, open_len, i64::from(OPEN));
        w.st(i, t, 0);
        let z = w.zero32();
        w.st(z, t, 4);
        let none = w.c32(i64::from(NONE));
        w.st(none, t, 8);
        w.st(start, t, 16);
        let ol = w.f.ins().iadd_imm(open_len, 1);
        w.set32(at::OPEN_LEN, ol);
        let later = w.f.ins().icmp_imm(IntCC::NotEqual, b, 0);
        let settle = w.f.ins().band(first, later);
        w.if_(settle, |w| {
            // The block before has its last slot now.
            let pb = w.f.ins().iadd_imm(b, -1);
            let r = w.call("w_settle", &[pb]);
            w.ret_if(r);
        });
        w.ok();
    }

    /// Node `j` of block `b`'s checkpoint: `start` at the checkpoints' width.
    fn put_checkpoint(w: &mut W<'_, '_>, b: Value, j: Value, start: Value) {
        let room = w.room_of(b);
        let four = w.c32(4);
        let col = w.col(four);
        let cw = w.get32(at::CW);
        let cw = w.u64of(cw);
        let c = w.f.ins().ushr_imm(j, K.trailing_zeros() as i64);
        let c = w.u64of(c);
        let off = w.f.ins().imul(c, cw);
        let a = w.f.ins().iadd(room, col);
        let a = w.f.ins().iadd(a, off);
        w.cell_put(a, cw, start);
    }

    /// `w_close(end)`: the innermost open node's end: its length, and an inner node's slot.
    fn close(w: &mut W<'_, '_>) {
        let end = w.arg(0);
        let open_len = w.get32(at::OPEN_LEN);
        let none = w.f.ins().icmp_imm(IntCC::Equal, open_len, 0);
        let n = w.get32(at::N);
        w.fail_if(none, fail::NO_OPEN, n);
        let top = w.f.ins().iadd_imm(open_len, -1);
        w.set32(at::OPEN_LEN, top);
        let opens = w.part(part::OPEN);
        let t = w.index(opens, top, i64::from(OPEN));
        let i = w.ld(I32, t, 0);
        let start = w.ld(I64, t, 16);
        let len = w.get64(at::SOURCE_LEN);
        let before = w.f.ins().icmp(IntCC::UnsignedLessThan, end, start);
        let past = w.f.ins().icmp(IntCC::UnsignedGreaterThan, end, len);
        let bad = w.f.ins().bor(before, past);
        w.fail_if(bad, fail::BOUNDS, i);
        let two = w.c32(2);
        let l = w.f.ins().isub(end, start);
        let r = w.call("w_put", &[two, i, l]);
        w.ret_if(r);
        let size = w.f.ins().isub(n, i);
        let inner = w.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, size, 1);
        w.if_(inner, |w| {
            let s = w.u64of(size);
            let s = w.f.ins().ishl_imm(s, 1);
            let zero = w.zero32();
            let r = w.call("w_put", &[zero, i, s]);
            w.ret_if(r);
        });
        let (b, _) = w.node_block(i);
        let r = w.call("w_settle", &[b]);
        w.f.ins().return_(&[r]);
        let after = w.f.create_block();
        w.f.switch_to_block(after);
        let z = w.zero32();
        w.f.ins().return_(&[z]);
    }

    /// `w_leaf(number, start, end)`: a closed node with no children.
    fn leaf(w: &mut W<'_, '_>) {
        let (number, start, end) = (w.arg(0), w.arg(1), w.arg(2));
        let r = w.call("w_open", &[number, start]);
        w.ret_if(r);
        let r = w.call("w_close", &[end]);
        w.f.ins().return_(&[r]);
        let after = w.f.create_block();
        w.f.switch_to_block(after);
        let z = w.zero32();
        w.f.ins().return_(&[z]);
    }

    /// `w_commit()`: the nodes so far final but for the open ones.
    fn commit(w: &mut W<'_, '_>) {
        let frontier = w.get32(at::FRONTIER);
        let was = w.f.ins().ushr_imm(frontier, Z.trailing_zeros() as i64);
        let n = w.get32(at::N);
        w.set32(at::FRONTIER, n);
        let to = w.f.ins().ushr_imm(n, Z.trailing_zeros() as i64);
        let b = w.var(I32, was);
        w.while_(
            |w| {
                let b = w.get(b);
                w.f.ins().icmp(IntCC::UnsignedLessThan, b, to)
            },
            |w| {
                let bv = w.get(b);
                let r = w.call("w_settle", &[bv]);
                w.ret_if(r);
                let nb = w.f.ins().iadd_imm(bv, 1);
                w.set(b, nb);
            },
        );
        w.ok();
    }

    /// `w_settle(b)`: block `b` made final, if it is: every node in it closed, the node after its last opened, the
    /// frontier past it.
    fn settle(w: &mut W<'_, '_>) {
        let b = w.arg(0);
        let e = w.entry(b);
        let done = w.ld(I32, e, 8);
        let b1 = w.f.ins().iadd_imm(b, 1);
        let end = w.f.ins().imul_imm(b1, i64::from(Z));
        let n = w.get32(at::N);
        let frontier = w.get32(at::FRONTIER);
        let is_done = w.f.ins().icmp_imm(IntCC::NotEqual, done, 0);
        let short = w.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, n, end);
        let behind = w.f.ins().icmp(IntCC::UnsignedLessThan, frontier, end);
        let not = w.f.ins().bor(is_done, short);
        let not = w.f.ins().bor(not, behind);
        let z = w.zero32();
        w.if_(not, |w| w.ret(z));
        // The open nodes' indices ascend: the last below the block's end is the one that may be in it.
        let open_len = w.get32(at::OPEN_LEN);
        let opens = w.part(part::OPEN);
        let lo = w.var(I32, z);
        let hi = w.var(I32, open_len);
        w.while_(
            |w| {
                let (l, h) = (w.get(lo), w.get(hi));
                w.f.ins().icmp(IntCC::UnsignedLessThan, l, h)
            },
            |w| {
                let (l, h) = (w.get(lo), w.get(hi));
                let s = w.f.ins().iadd(l, h);
                let mid = w.f.ins().ushr_imm(s, 1);
                let t = w.index(opens, mid, i64::from(OPEN));
                let node = w.ld(I32, t, 0);
                let below = w.f.ins().icmp(IntCC::UnsignedLessThan, node, end);
                let up = w.f.ins().iadd_imm(mid, 1);
                let l2 = w.f.ins().select(below, up, l);
                let h2 = w.f.ins().select(below, h, mid);
                w.set(lo, l2);
                w.set(hi, h2);
            },
        );
        let k = w.get(lo);
        let some = w.f.ins().icmp_imm(IntCC::NotEqual, k, 0);
        w.if_(some, |w| {
            let km = w.f.ins().iadd_imm(k, -1);
            let t = w.index(opens, km, i64::from(OPEN));
            let node = w.ld(I32, t, 0);
            let first = w.f.ins().imul_imm(b, i64::from(Z));
            let inside = w.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, node, first);
            let z = w.zero32();
            w.if_(inside, |w| w.ret(z));
        });
        let rows = w.c32(i64::from(Z));
        let r = w.call("w_finalize", &[b, rows]);
        w.f.ins().return_(&[r]);
        let after = w.f.create_block();
        w.f.switch_to_block(after);
        let z = w.zero32();
        w.f.ins().return_(&[z]);
    }

    /// `w_finalize(b, rows)`: block `b`, of `rows` rows, final: its presence taken, its side lists' parts laid,
    /// its side area given back.
    fn finalize(w: &mut W<'_, '_>) {
        let (b, rows) = (w.arg(0), w.arg(1));
        let e = w.entry(b);
        let one = w.c32(1);
        w.st(one, e, 8);
        let room = w.room_of(b);
        let area = w.area_of(b);
        let zero = w.zero32();
        let (count_at, js, vs) = w.list(area, zero);
        let count = w.ld(I32, count_at, 0);
        let cut = w.get32(at::CUT);
        let per = w.get32(at::PER);
        let kinds = w.part(part::KINDS);
        let numbers = w.part_len(part::KINDS);
        let numbers = w.f.ins().ushr_imm(numbers, 2);
        let pres = w.part(part::PRESENCE);
        let kw = w.get32(at::KW);
        let fw = w.get32(at::FW);
        let flags = w.get32(at::FLAGS);
        let keep = w.f.ins().band_imm(flags, 1);
        let words = w.f.ins().iadd(kw, fw);
        let words = w.u64of(words);
        let bytes = w.f.ins().ishl_imm(words, 3);
        let b64 = w.u64of(b);
        let b1 = w.f.ins().iadd_imm(b64, 1);
        let off = w.f.ins().imul(b1, bytes);
        let mine = w.f.ins().iadd(pres, off);
        let low = w.var(I32, zero);
        let jv = w.var(I32, zero);
        w.while_(
            |w| {
                let j = w.get(jv);
                w.f.ins().icmp(IntCC::UnsignedLessThan, j, rows)
            },
            |w| {
                let j = w.get(jv);
                let j64 = w.u64of(j);
                let ca = w.f.ins().iadd(room, j64);
                let byte = w.ld8(ca, 0);
                let number = w.var(I32, byte);
                let escaped = w.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, byte, cut);
                w.if_(escaped, |w| {
                    // The next entry of the codes' side list, where it is this node's.
                    let k = w.get(low);
                    let inside = w.f.ins().icmp(IntCC::UnsignedLessThan, k, count);
                    let lv = w.var(I32, zero);
                    w.if_(inside, |w| {
                        let ka = w.index(js, k, 2);
                        let ej = w.ld16(ka, 0);
                        let mine = w.f.ins().icmp(IntCC::Equal, ej, j);
                        w.if_(mine, |w| {
                            let va = w.index(vs, k, 8);
                            let x = w.ld(I64, va, 0);
                            let x = w.f.ins().ireduce(I32, x);
                            w.set(lv, x);
                        });
                        let k1 = w.f.ins().iadd_imm(k, 1);
                        w.set(low, k1);
                    });
                    let d = w.f.ins().isub(byte, cut);
                    let d = w.f.ins().imul(d, per);
                    let x = w.f.ins().iadd(cut, d);
                    let l = w.get(lv);
                    let x = w.f.ins().iadd(x, l);
                    w.set(number, x);
                });
                let num = w.get(number);
                let known = w.f.ins().uextend(I64, num);
                let known = w.f.ins().icmp(IntCC::UnsignedLessThan, known, numbers);
                let kf = w.var(I32, zero);
                w.if_(known, |w| {
                    let a = w.index(kinds, num, 4);
                    let x = w.ld(I32, a, 0);
                    w.set(kf, x);
                });
                let kfv = w.get(kf);
                let kind = w.f.ins().band_imm(kfv, 0xffff);
                let field = w.f.ins().ushr_imm(kfv, 16);
                let kw64 = w.u64of(kw);
                let set_bit = |w: &mut W<'_, '_>, base: Value, word0: Value, bit: Value| {
                    let word = w.f.ins().ushr_imm(bit, 6);
                    let word = w.u64of(word);
                    let word = w.f.ins().iadd(word, word0);
                    let at = w.f.ins().ishl_imm(word, 3);
                    let at = w.f.ins().iadd(base, at);
                    let sh = w.f.ins().band_imm(bit, 63);
                    let sh = w.u64of(sh);
                    let one = w.c64(1);
                    let m = w.f.ins().ishl(one, sh);
                    let x = w.ld(I64, at, 0);
                    let x = w.f.ins().bor(x, m);
                    w.st(x, at, 0);
                };
                let z64 = w.c64(0);
                set_bit(w, pres, z64, kind);
                set_bit(w, pres, kw64, field);
                w.if_(keep, |w| {
                    let z64 = w.c64(0);
                    set_bit(w, mine, z64, kind);
                    set_bit(w, mine, kw64, field);
                });
                let j1 = w.f.ins().iadd_imm(j, 1);
                w.set(jv, j1);
            },
        );
        // Its side lists' parts, laid.
        let q = w.var(I32, zero);
        w.while_(
            |w| {
                let q = w.get(q);
                w.f.ins().icmp_imm(IntCC::UnsignedLessThan, q, 4)
            },
            |w| {
                let qv = w.get(q);
                let r = w.call("w_lay", &[b, qv, rows]);
                w.ret_if(r);
                let q1 = w.f.ins().iadd_imm(qv, 1);
                w.set(q, q1);
            },
        );
        // Its side area, given back.
        let fl = w.part_len(part::FREE);
        let need = w.f.ins().iadd_imm(fl, 4);
        w.room(part::FREE, need);
        let fp = w.part(part::FREE);
        let at = w.f.ins().iadd(fp, fl);
        let e = w.entry(b);
        let a = w.ld(I32, e, 4);
        w.st(a, at, 0);
        w.set_part_len(part::FREE, need);
        let none = w.c32(i64::from(NONE));
        w.st(none, e, 4);
        // 16z: its pieces compressed, its room given back.
        let z = w.z();
        w.if_(z, |w| {
            let r = w.call("w_zblock", &[b, rows]);
            w.ret_if(r);
        });
        w.ok();
    }

    /// `w_lay(b, q, rows)`: block `b`'s side list `q` as stored, appended to `laid` (format 16's `part_into`): a
    /// u16 count a sub-block, each entry's offset in its sub-block, then the values at `vw[q]` bytes.
    fn lay(w: &mut W<'_, '_>) {
        let (b, q, rows) = (w.arg(0), w.arg(1), w.arg(2));
        let area = w.area_of(b);
        let (count_at, js, vs) = w.list(area, q);
        let count = w.ld(I32, count_at, 0);
        let laid = w.part(part::LAID);
        let start = w.part_len(part::LAID);
        let vw = w.get32_at(at::VW, q);
        let vw = w.u64of(vw);
        let nsub = w.f.ins().iadd_imm(rows, i64::from(BLOCK - 1));
        let nsub = w.f.ins().ushr_imm(nsub, BLOCK.trailing_zeros() as i64);
        let c64 = w.u64of(count);
        let ns64 = w.u64of(nsub);
        let some = w.f.ins().icmp_imm(IntCC::NotEqual, count, 0);
        let zero64 = w.c64(0);
        let ends = w.f.ins().ishl_imm(ns64, 1);
        let vals = w.f.ins().imul(c64, vw);
        let len = w.f.ins().iadd(ends, c64);
        let len = w.f.ins().iadd(len, vals);
        let len = w.f.ins().select(some, len, zero64);
        let end = w.f.ins().iadd(start, len);
        w.room(part::LAID, end);
        w.if_(some, |w| {
            let at = w.f.ins().iadd(laid, start);
            let zero = w.zero32();
            let k = w.var(I32, zero);
            let s = w.var(I32, zero);
            w.while_(
                |w| {
                    let s = w.get(s);
                    w.f.ins().icmp(IntCC::UnsignedLessThan, s, nsub)
                },
                |w| {
                    let sv = w.get(s);
                    w.while_(
                        |w| {
                            let kv = w.get(k);
                            let inside = w.f.ins().icmp(IntCC::UnsignedLessThan, kv, count);
                            let ka = w.index(js, kv, 2);
                            // A read one past the list's last entry lies in its room (`Z` entries).
                            let ej = w.ld16(ka, 0);
                            let sb = w.f.ins().ushr_imm(ej, BLOCK.trailing_zeros() as i64);
                            let mine = w.f.ins().icmp(IntCC::Equal, sb, sv);
                            w.f.ins().band(inside, mine)
                        },
                        |w| {
                            let kv = w.get(k);
                            let k1 = w.f.ins().iadd_imm(kv, 1);
                            w.set(k, k1);
                        },
                    );
                    let kv = w.get(k);
                    let ea = w.index(at, sv, 2);
                    w.st16(kv, ea, 0);
                    let s1 = w.f.ins().iadd_imm(sv, 1);
                    w.set(s, s1);
                },
            );
            let offs = w.f.ins().iadd(at, ends);
            let m = w.var(I32, zero);
            w.while_(
                |w| {
                    let m = w.get(m);
                    w.f.ins().icmp(IntCC::UnsignedLessThan, m, count)
                },
                |w| {
                    let mv = w.get(m);
                    let ka = w.index(js, mv, 2);
                    let ej = w.ld16(ka, 0);
                    let o = w.index(offs, mv, 1);
                    w.st8(ej, o, 0);
                    let m1 = w.f.ins().iadd_imm(mv, 1);
                    w.set(m, m1);
                },
            );
            let values = w.f.ins().iadd(offs, c64);
            let zero = w.zero32();
            w.set(m, zero);
            w.while_(
                |w| {
                    let m = w.get(m);
                    w.f.ins().icmp(IntCC::UnsignedLessThan, m, count)
                },
                |w| {
                    let mv = w.get(m);
                    let va = w.index(vs, mv, 8);
                    let x = w.ld(I64, va, 0);
                    let m64 = w.u64of(mv);
                    let o = w.f.ins().imul(m64, vw);
                    let o = w.f.ins().iadd(values, o);
                    w.cell_put(o, vw, x);
                    let m1 = w.f.ins().iadd_imm(mv, 1);
                    w.set(m, m1);
                },
            );
        });
        w.set_part_len(part::LAID, end);
        let la = w.part(part::LAID_AT);
        let b64 = w.u64of(b);
        let need = w.f.ins().iadd_imm(b64, 1);
        let need = w.f.ins().imul_imm(need, i64::from(LAID_AT));
        w.room(part::LAID_AT, need);
        let ll = w.part_len(part::LAID_AT);
        let grow = w.f.ins().icmp(IntCC::UnsignedLessThan, ll, need);
        w.if_(grow, |w| w.set_part_len(part::LAID_AT, need));
        let e = w.index(la, b, i64::from(LAID_AT));
        let q8 = w.u64of(q);
        let q8 = w.f.ins().ishl_imm(q8, 3);
        let e = w.f.ins().iadd(e, q8);
        let s32 = w.f.ins().ireduce(I32, start);
        let l32 = w.f.ins().ireduce(I32, len);
        w.st(s32, e, 0);
        w.st(l32, e, 4);
        w.ok();
    }

    /// `w_retag(i, number)`: node `i`'s code given again; its block not final.
    fn retag(w: &mut W<'_, '_>) {
        let (i, number) = (w.arg(0), w.arg(1));
        changeable(w, i);
        let (b, j) = w.node_block(i);
        let area = w.area_of(b);
        let zero = w.zero32();
        let (count_at, js, vs) = w.list(area, zero);
        let count = w.ld(I32, count_at, 0);
        let k = w.call("w_find", &[js, count, j]);
        let inside = w.f.ins().icmp(IntCC::UnsignedLessThan, k, count);
        let ka = w.index(js, k, 2);
        let found = w.ld16(ka, 0);
        let same = w.f.ins().icmp(IntCC::Equal, found, j);
        let hit = w.f.ins().band(inside, same);
        w.if_(hit, |w| remove(w, count_at, js, vs, count, k));
        let r = w.call("w_put_code", &[i, number]);
        w.f.ins().return_(&[r]);
        let after = w.f.create_block();
        w.f.switch_to_block(after);
        let z = w.zero32();
        w.f.ins().return_(&[z]);
    }

    /// Fails unless node `i` is written and its block not final.
    fn changeable(w: &mut W<'_, '_>, i: Value) {
        let n = w.get32(at::N);
        let written = w.f.ins().icmp(IntCC::UnsignedLessThan, i, n);
        w.if_else(
            written,
            |w| {
                let (b, _) = w.node_block(i);
                let e = w.entry(b);
                let done = w.ld(I32, e, 8);
                let fin = w.f.ins().icmp_imm(IntCC::NotEqual, done, 0);
                w.fail_if(fin, fail::FINAL, i);
            },
            |w| w.fail(fail::FINAL, i),
        );
    }

    /// `w_set_start(i, start)`: open node `i`'s start given again: its gap, its checkpoint and the next node's gap.
    fn set_start(w: &mut W<'_, '_>) {
        let (i, start) = (w.arg(0), w.arg(1));
        let open_len = w.get32(at::OPEN_LEN);
        let opens = w.part(part::OPEN);
        // The innermost open entry of node `i`.
        let k = w.var(I32, open_len);
        let none = w.c32(i64::from(NONE));
        let found = w.var(I32, none);
        w.while_(
            |w| {
                let kv = w.get(k);
                let more = w.f.ins().icmp_imm(IntCC::NotEqual, kv, 0);
                let f = w.get(found);
                let unseen = w.f.ins().icmp_imm(IntCC::Equal, f, i64::from(NONE));
                w.f.ins().band(more, unseen)
            },
            |w| {
                let kv = w.get(k);
                let km = w.f.ins().iadd_imm(kv, -1);
                let t = w.index(opens, km, i64::from(OPEN));
                let node = w.ld(I32, t, 0);
                let is = w.f.ins().icmp(IntCC::Equal, node, i);
                let f = w.get(found);
                let f2 = w.f.ins().select(is, km, f);
                w.set(found, f2);
                w.set(k, km);
            },
        );
        let fk = w.get(found);
        let missing = w.f.ins().icmp_imm(IntCC::Equal, fk, i64::from(NONE));
        w.fail_if(missing, fail::NOT_OPEN, i);
        changeable(w, i);
        let n = w.get32(at::N);
        let i1 = w.f.ins().iadd_imm(i, 1);
        let has_next = w.f.ins().icmp(IntCC::UnsignedLessThan, i1, n);
        w.if_(has_next, |w| changeable(w, i1));
        let t = w.index(opens, fk, i64::from(OPEN));
        let old = w.ld(I64, t, 16);
        let one = w.c32(1);
        let z64 = w.c64(0);
        let prev = w.var(I64, z64);
        let some = w.f.ins().icmp_imm(IntCC::NotEqual, i, 0);
        w.if_(some, |w| {
            let g = w.call("w_value", &[one, i]);
            let p = w.f.ins().isub(old, g);
            w.set(prev, p);
        });
        let len = w.get64(at::SOURCE_LEN);
        let next = w.var(I64, len);
        w.if_(has_next, |w| {
            let g = w.call("w_value", &[one, i1]);
            let x = w.f.ins().iadd(old, g);
            w.set(next, x);
        });
        let pv = w.get(prev);
        let nv = w.get(next);
        let lo = w.f.ins().icmp(IntCC::UnsignedLessThan, start, pv);
        let hi = w.f.ins().icmp(IntCC::UnsignedGreaterThan, start, nv);
        let bad = w.f.ins().bor(lo, hi);
        w.fail_if(bad, fail::BOUNDS, i);
        w.if_(some, |w| {
            let g = w.f.ins().isub(start, pv);
            let r = w.call("w_put", &[one, i, g]);
            w.ret_if(r);
        });
        w.if_else(
            has_next,
            |w| {
                let g = w.f.ins().isub(nv, start);
                let r = w.call("w_put", &[one, i1, g]);
                w.ret_if(r);
            },
            |w| w.set64(at::PREV_START, start),
        );
        let (b, j) = w.node_block(i);
        let mk = w.f.ins().band_imm(j, i64::from(K - 1));
        let checkpoint = w.f.ins().icmp_imm(IntCC::Equal, mk, 0);
        w.if_(checkpoint, |w| put_checkpoint(w, b, j, start));
        w.st(start, t, 16);
        w.ok();
    }

    /// `w_finish()`: the tape: the last node's slot, the blocks not yet final, the last block's columns moved down
    /// to its rows, the tail (each side list's parts in block order, their starts, the newline index), the header.
    fn finish(w: &mut W<'_, '_>) {
        let r = w.call("w_finish_blocks", &[]);
        w.ret_if(r);
        let r = w.call("w_finish_tail", &[]);
        w.f.ins().return_(&[r]);
        let after = w.f.create_block();
        w.f.switch_to_block(after);
        let z = w.zero32();
        w.f.ins().return_(&[z]);
    }

    /// `w_finish_blocks()`: `finish`'s first half: the last node's slot, every block final. A host reserves the
    /// tail's room after it, knowing the side lists' laid parts (`w_finish_tail`).
    fn finish_blocks(w: &mut W<'_, '_>) {
        // The first failure, kept, is the op's.
        let bad = w.get32(at::BAD);
        w.ret_if(bad);
        let open_len = w.get32(at::OPEN_LEN);
        let unclosed = w.f.ins().icmp_imm(IntCC::NotEqual, open_len, 0);
        let n = w.get32(at::N);
        w.fail_if(unclosed, fail::UNCLOSED, n);
        let some = w.f.ins().icmp_imm(IntCC::NotEqual, n, 0);
        w.if_(some, |w| {
            // The last node in preorder is a leaf with nothing after it.
            let last = w.f.ins().iadd_imm(n, -1);
            let zero = w.zero32();
            let one = w.c64(1);
            let r = w.call("w_put", &[zero, last, one]);
            w.ret_if(r);
        });
        let count = w.f.ins().iadd_imm(n, i64::from(Z - 1));
        let count = w.f.ins().ushr_imm(count, Z.trailing_zeros() as i64);
        let zero = w.zero32();
        let b = w.var(I32, zero);
        w.while_(
            |w| {
                let b = w.get(b);
                w.f.ins().icmp(IntCC::UnsignedLessThan, b, count)
            },
            |w| {
                let bv = w.get(b);
                let e = w.entry(bv);
                let done = w.ld(I32, e, 8);
                let open = w.f.ins().icmp_imm(IntCC::Equal, done, 0);
                w.if_(open, |w| {
                    let first = w.f.ins().imul_imm(bv, i64::from(Z));
                    let rows = w.f.ins().isub(n, first);
                    let zc = w.c32(i64::from(Z));
                    let rows = w.f.ins().umin(rows, zc);
                    let r = w.call("w_finalize", &[bv, rows]);
                    w.ret_if(r);
                });
                let b1 = w.f.ins().iadd_imm(bv, 1);
                w.set(b, b1);
            },
        );
        w.ok();
    }

    /// `w_finish_tail()`: `finish`'s second half: 16z's tail (`w_zfinish`), or 16's: the last block's columns moved
    /// down to its rows, then the side lists' parts in block order, their starts, the newline index, the header.
    fn finish_tail(w: &mut W<'_, '_>) {
        let bad = w.get32(at::BAD);
        w.ret_if(bad);
        let n = w.get32(at::N);
        let count = w.f.ins().iadd_imm(n, i64::from(Z - 1));
        let count = w.f.ins().ushr_imm(count, Z.trailing_zeros() as i64);
        let z = w.z();
        w.if_(z, |w| {
            let r = w.call("w_zfinish", &[count]);
            w.ret(r);
        });
        let out = w.part(part::OUT);
        let w0 = w.get32(at::W);
        let w1 = w.get32(at::W + 4);
        let w2 = w.get32(at::W + 8);
        let cw = w.get32(at::CW);
        let cw64 = w.u64of(cw);
        let befores = {
            let one = w.c32(1);
            let b2 = w.f.ins().iadd_imm(w0, 1);
            let b3 = w.f.ins().iadd(b2, w1);
            let b4 = w.f.ins().iadd(b3, w2);
            [one, b2, b3, b4]
        };
        let head = w.c64(i64::from(HEAD16));
        let ends = w.var(I64, head);
        let any = w.f.ins().icmp_imm(IntCC::NotEqual, count, 0);
        w.if_(any, |w| {
            // The last block's columns down to its rows.
            let lb = w.f.ins().iadd_imm(count, -1);
            let first = w.f.ins().imul_imm(lb, i64::from(Z));
            let rows = w.f.ins().isub(n, first);
            let rows64 = w.u64of(rows);
            let room = w.room_of(lb);
            for r in 1..5u32 {
                let len = if r == 4 {
                    let c = w.f.ins().iadd_imm(rows64, i64::from(K - 1));
                    let c = w.f.ins().ushr_imm(c, K.trailing_zeros() as i64);
                    w.f.ins().imul(c, cw64)
                } else {
                    let wr = [w0, w1, w2][r as usize - 1];
                    let wr = w.u64of(wr);
                    w.f.ins().imul(rows64, wr)
                };
                let rc = w.c32(i64::from(r));
                let from = w.col(rc);
                let from = w.f.ins().iadd(room, from);
                let br = w.u64of(befores[r as usize - 1]);
                let to = w.f.ins().imul(rows64, br);
                let to = w.f.ins().iadd(room, to);
                w.call("w_copy", &[to, from, len]);
                if r == 4 {
                    let e = w.f.ins().iadd(to, len);
                    let e = w.f.ins().isub(e, out);
                    w.set(ends, e);
                }
            }
        });
        // The tail: each side list's parts in block order, their starts, the newline index.
        let blocks_end = w.get(ends);
        let lines_len = w.part_len(part::LINES);
        let laid_len = w.part_len(part::LAID);
        let c64 = w.u64of(count);
        let starts = w.f.ins().iadd_imm(c64, 1);
        let starts = w.f.ins().imul_imm(starts, 16);
        let need = w.f.ins().iadd(blocks_end, laid_len);
        let need = w.f.ins().iadd(need, starts);
        let need = w.f.ins().iadd(need, lines_len);
        let need = w.f.ins().iadd_imm(need, 8 * 6 + 8);
        w.room(part::OUT, need);
        let at = w.f.ins().iadd(out, blocks_end);
        let z64 = w.c64(0);
        w.st(z64, at, 0);
        let pos = w.var(I64, blocks_end);
        let p8 = w.f.ins().iadd_imm(blocks_end, 8);
        w.set(pos, p8);
        let laid = w.part(part::LAID);
        let la = w.part(part::LAID_AT);
        let mut side = Vec::new();
        for q in 0..4i64 {
            let s = w.get(pos);
            side.push(s);
            let zero = w.zero32();
            let bb = w.var(I32, zero);
            w.while_(
                |w| {
                    let b = w.get(bb);
                    w.f.ins().icmp(IntCC::UnsignedLessThan, b, count)
                },
                |w| {
                    let bv = w.get(bb);
                    let e = w.index(la, bv, i64::from(LAID_AT));
                    let st = w.ld(I32, e, (8 * q) as i32);
                    let ln = w.ld(I32, e, (8 * q + 4) as i32);
                    let st = w.u64of(st);
                    let ln = w.u64of(ln);
                    let from = w.f.ins().iadd(laid, st);
                    let p = w.get(pos);
                    let to = w.f.ins().iadd(out, p);
                    w.call("w_copy", &[to, from, ln]);
                    let p2 = w.f.ins().iadd(p, ln);
                    w.set(pos, p2);
                    let b1 = w.f.ins().iadd_imm(bv, 1);
                    w.set(bb, b1);
                },
            );
            let p = w.get(pos);
            let to = w.f.ins().iadd(out, p);
            let z64 = w.c64(0);
            w.st(z64, to, 0);
            let p8 = w.f.ins().iadd_imm(p, 8);
            w.set(pos, p8);
        }
        // The padding to the part starts' alignment is zeros too (the bytes there were the last block's columns).
        let p = w.get(pos);
        let to = w.f.ins().iadd(out, p);
        let z64 = w.c64(0);
        w.st(z64, to, 0);
        let p = w.f.ins().iadd_imm(p, 3);
        let p = w.f.ins().band_imm(p, !3);
        w.set(pos, p);
        let mut parts = Vec::new();
        for q in 0..4i64 {
            let s = w.get(pos);
            parts.push(s);
            let zero = w.zero32();
            let acc = w.var(I32, zero);
            let bb = w.var(I32, zero);
            w.while_(
                |w| {
                    let b = w.get(bb);
                    w.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, b, count)
                },
                |w| {
                    let bv = w.get(bb);
                    let a = w.get(acc);
                    let p = w.get(pos);
                    let to = w.f.ins().iadd(out, p);
                    w.st(a, to, 0);
                    let p4 = w.f.ins().iadd_imm(p, 4);
                    w.set(pos, p4);
                    let inside = w.f.ins().icmp(IntCC::UnsignedLessThan, bv, count);
                    w.if_(inside, |w| {
                        let e = w.index(la, bv, i64::from(LAID_AT));
                        let ln = w.ld(I32, e, (8 * q + 4) as i32);
                        let a2 = w.f.ins().iadd(a, ln);
                        w.set(acc, a2);
                    });
                    let b1 = w.f.ins().iadd_imm(bv, 1);
                    w.set(bb, b1);
                },
            );
        }
        let lines_at = w.get(pos);
        let lines = w.part(part::LINES);
        let to = w.f.ins().iadd(out, lines_at);
        w.call("w_copy", &[to, lines, lines_len]);
        let le = w.f.ins().iadd(lines_at, lines_len);
        let to = w.f.ins().iadd(out, le);
        let z64 = w.c64(0);
        w.st(z64, to, 0);
        let end = w.f.ins().iadd_imm(le, 8);
        w.set_part_len(part::OUT, end);
        let tail: Vec<Value> = side.iter().chain(&parts).copied().chain([lines_at, end]).collect();
        write_head(w, out, MAGIC16, &tail);
        w.ok();
    }

    /// The header at `out`: `magic`, the document's constants, and the tail's offsets (`tail`, ten i64s: the side
    /// lists', their part starts', the newline index's, the end; for 16z, the tape's it decodes to).
    fn write_head(w: &mut W<'_, '_>, out: Value, magic: u64, tail: &[Value]) {
        let h = w.c64(i64::from(HEAD16));
        w.call("w_zero", &[out, h]);
        let magic = w.c64(magic as i64);
        w.st(magic, out, 0);
        let tag = w.get32(at::TAG);
        w.st8(tag, out, 8);
        let mark = w.get32(at::MARK);
        w.st8(mark, out, 9);
        let cw = w.get32(at::CW);
        w.st8(cw, out, 10);
        let nl = w.get32(at::NEWLINES);
        w.st(nl, out, 12);
        let sl = w.get64(at::SOURCE_LEN);
        w.st(sl, out, 16);
        let n = w.get32(at::N);
        let n64 = w.u64of(n);
        w.st(n64, out, 24);
        for k in 0..4 {
            let v = w.get32(at::VW + 4 * k);
            w.st8(v, out, 32 + k);
        }
        for k in 0..3 {
            let v = w.get32(at::W + 4 * k);
            w.st8(v, out, 36 + k);
        }
        let stride = w.get32(at::STRIDE);
        w.st(stride, out, 40);
        let blocks = w.c32(i64::from(HEAD16));
        w.st(blocks, out, 44);
        for (t, &v) in tail.iter().enumerate() {
            let v = w.f.ins().ireduce(I32, v);
            w.st(v, out, 48 + 4 * t as i32);
        }
    }

    /// `w_zblock(b, rows)`: 16z's block `b`, final: its four pieces (each column at its rows, then its side list's
    /// part as laid) stored in turn at the payload's end (`w_zpiece`), its checkpoints kept, its room given back.
    fn zblock(w: &mut W<'_, '_>) {
        let (b, rows) = (w.arg(0), w.arg(1));
        let room = w.room_of(b);
        let laid = w.part(part::LAID);
        let la = w.part(part::LAID_AT);
        let la = w.index(la, b, i64::from(LAID_AT));
        let rows64 = w.u64of(rows);
        let zero = w.zero32();
        let q = w.var(I32, zero);
        w.while_(
            |w| {
                let q = w.get(q);
                w.f.ins().icmp_imm(IntCC::UnsignedLessThan, q, 4)
            },
            |w| {
                let qv = w.get(q);
                // A column's cell: a byte (codes), else its array's width.
                let first = w.f.ins().icmp_imm(IntCC::Equal, qv, 0);
                let one = w.c32(1);
                let qm = w.f.ins().iadd_imm(qv, -1);
                let qm = w.f.ins().select(first, zero, qm);
                let wq = w.get32_at(at::W, qm);
                let cell = w.f.ins().select(first, one, wq);
                let cell = w.u64of(cell);
                let clen = w.f.ins().imul(rows64, cell);
                let e = w.index(la, qv, 8);
                let lst = w.ld(I32, e, 0);
                let lst = w.u64of(lst);
                let llen = w.ld(I32, e, 4);
                let llen = w.u64of(llen);
                let raw = w.f.ins().iadd(clen, llen);
                let pos = w.part_len(part::OUT);
                let need = w.f.ins().iadd(pos, raw);
                let need = w.f.ins().iadd(need, raw);
                let extra = w.c64(255);
                let extra = w.f.ins().udiv(raw, extra);
                let need = w.f.ins().iadd(need, extra);
                let need = w.f.ins().iadd_imm(need, 16);
                w.room(part::OUT, need);
                let j = w.f.ins().ishl_imm(b, 2);
                let j = w.f.ins().iadd(j, qv);
                let j1 = w.u64of(j);
                let j1 = w.f.ins().iadd_imm(j1, 1);
                let pneed = w.f.ins().imul_imm(j1, i64::from(PIECE));
                w.room(part::PIECES, pneed);
                let pl = w.part_len(part::PIECES);
                let grow = w.f.ins().icmp(IntCC::UnsignedLessThan, pl, pneed);
                w.if_(grow, |w| w.set_part_len(part::PIECES, pneed));
                let out = w.part(part::OUT);
                let src = w.f.ins().iadd(out, pos);
                let col = w.col(qv);
                let col = w.f.ins().iadd(room, col);
                w.call("w_copy", &[src, col, clen]);
                let to = w.f.ins().iadd(src, clen);
                let from = w.f.ins().iadd(laid, lst);
                w.call("w_copy", &[to, from, llen]);
                let r = w.call("w_zpiece", &[j, pos, raw]);
                w.ret_if(r);
                let q1 = w.f.ins().iadd_imm(qv, 1);
                w.set(q, q1);
            },
        );
        // Its checkpoints, at their place among the blocks'.
        let cw = w.get32(at::CW);
        let cw = w.u64of(cw);
        let b64 = w.u64of(b);
        let c0 = w.f.ins().imul_imm(b64, i64::from(Z / K));
        let c0 = w.f.ins().imul(c0, cw);
        let c = w.f.ins().iadd_imm(rows64, i64::from(K - 1));
        let c = w.f.ins().ushr_imm(c, K.trailing_zeros() as i64);
        let clen = w.f.ins().imul(c, cw);
        let end = w.f.ins().iadd(c0, clen);
        w.room(part::CKPT, end);
        let ck = w.part(part::CKPT);
        let to = w.f.ins().iadd(ck, c0);
        let four = w.c32(4);
        let col = w.col(four);
        let from = w.f.ins().iadd(room, col);
        w.call("w_copy", &[to, from, clen]);
        let cl = w.part_len(part::CKPT);
        let grow = w.f.ins().icmp(IntCC::UnsignedLessThan, cl, end);
        w.if_(grow, |w| w.set_part_len(part::CKPT, end));
        // The laid parts are in its pieces; its room serves the next block.
        let z64 = w.c64(0);
        w.set_part_len(part::LAID, z64);
        let fl = w.part_len(part::ROOM_FREE);
        let need = w.f.ins().iadd_imm(fl, 4);
        w.room(part::ROOM_FREE, need);
        let fp = w.part(part::ROOM_FREE);
        let at = w.f.ins().iadd(fp, fl);
        let e = w.entry(b);
        let r = w.ld(I32, e, 0);
        w.st(r, at, 0);
        w.set_part_len(part::ROOM_FREE, need);
        w.ok();
    }

    /// `w_zpiece(j, pos, raw)`: piece `j`, its `raw` bytes at the payload's end (`pos` in the tape), stored as 16z
    /// stores it: a repeat of an earlier piece of the same bytes (found by their hash, then compared: a raw
    /// piece's bytes with these, an LZ4 one's with these compressed), else one LZ4 block where that saves a tenth,
    /// else as they are. Its entry in the piece index; the payload past it.
    fn zpiece(w: &mut W<'_, '_>) {
        let (j, pos, raw) = (w.arg(0), w.arg(1), w.arg(2));
        let out = w.part(part::OUT);
        let src = w.f.ins().iadd(out, pos);
        let dst = w.f.ins().iadd(src, raw);
        let pieces = w.part(part::PIECES);
        let pe = w.index(pieces, j, i64::from(PIECE));
        let start = w.f.ins().iadd_imm(pos, -i64::from(HEAD16));
        let start = w.f.ins().ireduce(I32, start);
        let raw32 = w.f.ins().ireduce(I32, raw);
        let unset = w.c64(-1);
        let zl = w.var(I64, unset);
        let zero64 = w.c64(0);
        let h = w.var(I64, zero64);
        let none = w.c32(i64::from(NONE));
        let next = w.var(I32, none);
        let compress = |w: &mut W<'_, '_>| -> Value {
            let z = w.get(zl);
            let todo = w.f.ins().icmp_imm(IntCC::SignedLessThan, z, 0);
            w.if_(todo, |w| {
                let l = w.call("w_lz4", &[src, raw, dst]);
                w.set(zl, l);
            });
            w.get(zl)
        };
        let entry = |w: &mut W<'_, '_>, len: Value, kind: u32, next: Value, h: Value| {
            w.st(start, pe, 0);
            let len32 = w.f.ins().ireduce(I32, len);
            w.st(len32, pe, 4);
            w.st(raw32, pe, 8);
            let k = w.c32(i64::from(kind));
            w.st(k, pe, 12);
            w.st(next, pe, 16);
            w.st(h, pe, 24);
            let end = w.f.ins().iadd(pos, len);
            w.set_part_len(part::OUT, end);
        };
        let big = w.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, raw, i64::from(REPEAT_MIN));
        w.if_(big, |w| {
            let hv = w.call("w_hash", &[src, raw]);
            w.set(h, hv);
            let nb = w.get32(at::ZBUCKETS);
            let mask = w.f.ins().iadd_imm(nb, -1);
            let h32 = w.f.ins().ireduce(I32, hv);
            let bucket = w.f.ins().band(h32, mask);
            let seen = w.part(part::SEEN);
            let slot = w.index(seen, bucket, 4);
            let head = w.ld(I32, slot, 0);
            let e = w.var(I32, head);
            w.while_(
                |w| {
                    let ev = w.get(e);
                    w.f.ins().icmp_imm(IntCC::NotEqual, ev, i64::from(NONE))
                },
                |w| {
                    let ev = w.get(e);
                    let ee = w.index(pieces, ev, i64::from(PIECE));
                    let eh = w.ld(I64, ee, 24);
                    let er = w.ld(I32, ee, 8);
                    let same_h = w.f.ins().icmp(IntCC::Equal, eh, hv);
                    let same_r = w.f.ins().icmp(IntCC::Equal, er, raw32);
                    let cand = w.f.ins().band(same_h, same_r);
                    w.if_(cand, |w| {
                        let kind = w.ld(I32, ee, 12);
                        let est = w.ld(I32, ee, 0);
                        let est = w.u64of(est);
                        let stored = w.f.ins().iadd(out, est);
                        let stored = w.f.ins().iadd_imm(stored, i64::from(HEAD16));
                        let zero = w.zero32();
                        let same = w.var(I32, zero);
                        let is_raw = w.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(RAW));
                        w.if_else(
                            is_raw,
                            |w| {
                                let r = w.call("w_eq", &[stored, src, raw]);
                                w.set(same, r);
                            },
                            |w| {
                                let z = compress(w);
                                let elen = w.ld(I32, ee, 4);
                                let elen = w.u64of(elen);
                                let fits = w.f.ins().icmp(IntCC::Equal, z, elen);
                                w.if_(fits, |w| {
                                    let r = w.call("w_eq", &[stored, dst, z]);
                                    w.set(same, r);
                                });
                            },
                        );
                        let sv = w.get(same);
                        let hit = w.f.ins().icmp_imm(IntCC::NotEqual, sv, 0);
                        w.if_(hit, |w| {
                            w.st(ev, src, 0);
                            let four = w.c64(4);
                            let none = w.c32(i64::from(NONE));
                            entry(w, four, REPEAT, none, hv);
                            let z = w.zero32();
                            w.ret(z);
                        });
                    });
                    let nx = w.ld(I32, ee, 16);
                    w.set(e, nx);
                },
            );
            // The first piece of these bytes: into its bucket.
            w.set(next, head);
            w.st(j, slot, 0);
        });
        let kind = w.c32(i64::from(RAW));
        let kindv = w.var(I32, kind);
        let lenv = w.var(I64, raw);
        let try_lz = w.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, raw, i64::from(LZ4_MIN));
        w.if_(try_lz, |w| {
            let z = compress(w);
            let z10 = w.f.ins().imul_imm(z, 10);
            let r9 = w.f.ins().imul_imm(raw, 9);
            let saves = w.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, z10, r9);
            w.if_(saves, |w| {
                w.call("w_copy", &[src, dst, z]);
                let k = w.c32(i64::from(LZ4));
                w.set(kindv, k);
                w.set(lenv, z);
            });
        });
        let len = w.get(lenv);
        let k = w.get(kindv);
        let nx = w.get(next);
        let hv = w.get(h);
        // `entry` with the kind as a value.
        w.st(start, pe, 0);
        let len32 = w.f.ins().ireduce(I32, len);
        w.st(len32, pe, 4);
        w.st(raw32, pe, 8);
        w.st(k, pe, 12);
        w.st(nx, pe, 16);
        w.st(hv, pe, 24);
        let end = w.f.ins().iadd(pos, len);
        w.set_part_len(part::OUT, end);
        w.ok();
    }

    /// `w_zfinish(count)`: 16z's tail after its payload (format 16's `ZOut::finish`): each side list's part
    /// starts by block, the checkpoints, the newline index, the piece index (starts, lengths, kinds); then the
    /// header, which says the tape it decodes to, and where each of these lies.
    fn zfinish(w: &mut W<'_, '_>) {
        let count = w.arg(0);
        let out = w.part(part::OUT);
        let pos0 = w.part_len(part::OUT);
        let la = w.part(part::LAID_AT);
        let ck = w.part(part::CKPT);
        let clen = w.part_len(part::CKPT);
        let lines = w.part(part::LINES);
        let llen = w.part_len(part::LINES);
        let pieces = w.part(part::PIECES);
        let c64 = w.u64of(count);
        let np = w.f.ins().ishl_imm(c64, 2);
        let starts_len = w.f.ins().iadd_imm(c64, 1);
        let starts_len = w.f.ins().imul_imm(starts_len, 16);
        let need = w.f.ins().iadd(pos0, starts_len);
        let need = w.f.ins().iadd(need, clen);
        let need = w.f.ins().iadd(need, llen);
        let idx = w.f.ins().imul_imm(np, 9);
        let need = w.f.ins().iadd(need, idx);
        let need = w.f.ins().iadd_imm(need, 3 + 8 + 8 + 3 + 8);
        w.room(part::OUT, need);
        let z64 = w.c64(0);
        let at0 = w.f.ins().iadd(out, pos0);
        w.st(z64, at0, 0);
        let p = w.f.ins().iadd_imm(pos0, 3);
        let p = w.f.ins().band_imm(p, !3);
        let pos = w.var(I64, p);
        let parts_at = p;
        let mut totals = Vec::new();
        for q in 0..4i32 {
            let zero = w.zero32();
            let acc = w.var(I32, zero);
            let bb = w.var(I32, zero);
            w.while_(
                |w| {
                    let b = w.get(bb);
                    w.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, b, count)
                },
                |w| {
                    let bv = w.get(bb);
                    let a = w.get(acc);
                    let p = w.get(pos);
                    let to = w.f.ins().iadd(out, p);
                    w.st(a, to, 0);
                    let p4 = w.f.ins().iadd_imm(p, 4);
                    w.set(pos, p4);
                    let inside = w.f.ins().icmp(IntCC::UnsignedLessThan, bv, count);
                    w.if_(inside, |w| {
                        let e = w.index(la, bv, i64::from(LAID_AT));
                        let ln = w.ld(I32, e, 8 * q + 4);
                        let a2 = w.f.ins().iadd(a, ln);
                        w.set(acc, a2);
                    });
                    let b1 = w.f.ins().iadd_imm(bv, 1);
                    w.set(bb, b1);
                },
            );
            let a = w.get(acc);
            let a = w.u64of(a);
            totals.push(a);
        }
        let ckpt_at = w.get(pos);
        let to = w.f.ins().iadd(out, ckpt_at);
        w.call("w_copy", &[to, ck, clen]);
        let e = w.f.ins().iadd(ckpt_at, clen);
        let to = w.f.ins().iadd(out, e);
        w.st(z64, to, 0);
        let lines_at = w.f.ins().iadd_imm(e, 8);
        let to = w.f.ins().iadd(out, lines_at);
        w.call("w_copy", &[to, lines, llen]);
        let e = w.f.ins().iadd(lines_at, llen);
        let to = w.f.ins().iadd(out, e);
        w.st(z64, to, 0);
        w.st(z64, to, 8);
        let p = w.f.ins().iadd_imm(e, 8 + 3);
        let starts = w.f.ins().band_imm(p, !3);
        let lens = w.f.ins().ishl_imm(np, 2);
        let lens = w.f.ins().iadd(starts, lens);
        let kinds = w.f.ins().ishl_imm(np, 2);
        let kinds = w.f.ins().iadd(lens, kinds);
        let end = w.f.ins().iadd(kinds, np);
        let zero = w.c64(0);
        let k = w.var(I64, zero);
        w.while_(
            |w| {
                let kv = w.get(k);
                w.f.ins().icmp(IntCC::UnsignedLessThan, kv, np)
            },
            |w| {
                let kv = w.get(k);
                let pe = w.f.ins().imul_imm(kv, i64::from(PIECE));
                let pe = w.f.ins().iadd(pieces, pe);
                let k4 = w.f.ins().ishl_imm(kv, 2);
                let st = w.ld(I32, pe, 0);
                let a = w.f.ins().iadd(out, starts);
                let a = w.f.ins().iadd(a, k4);
                w.st(st, a, 0);
                let ln = w.ld(I32, pe, 4);
                let a = w.f.ins().iadd(out, lens);
                let a = w.f.ins().iadd(a, k4);
                w.st(ln, a, 0);
                let kd = w.ld(I32, pe, 12);
                let a = w.f.ins().iadd(out, kinds);
                let a = w.f.ins().iadd(a, kv);
                w.st8(kd, a, 0);
                let k1 = w.f.ins().iadd_imm(kv, 1);
                w.set(k, k1);
            },
        );
        w.set_part_len(part::OUT, end);
        // The tape it decodes to: its blocks' end, then its tail as format 16 places it (`Placement::of`).
        let n = w.get32(at::N);
        let stride = w.get32(at::STRIDE);
        let stride = w.u64of(stride);
        let cw = w.get32(at::CW);
        let cw = w.u64of(cw);
        let head16 = w.c64(i64::from(HEAD16));
        let be = w.var(I64, head16);
        let any = w.f.ins().icmp_imm(IntCC::NotEqual, count, 0);
        w.if_(any, |w| {
            let lb = w.f.ins().iadd_imm(c64, -1);
            let n64 = w.u64of(n);
            let first = w.f.ins().imul_imm(lb, i64::from(Z));
            let rows = w.f.ins().isub(n64, first);
            let at = w.f.ins().imul(lb, stride);
            let at = w.f.ins().iadd_imm(at, i64::from(HEAD16));
            let w0 = w.get32(at::W);
            let w1 = w.get32(at::W + 4);
            let w2 = w.get32(at::W + 8);
            let b4 = w.f.ins().iadd(w0, w1);
            let b4 = w.f.ins().iadd(b4, w2);
            let b4 = w.f.ins().iadd_imm(b4, 1);
            let b4 = w.u64of(b4);
            let cols = w.f.ins().imul(rows, b4);
            let c = w.f.ins().iadd_imm(rows, i64::from(K - 1));
            let c = w.f.ins().ushr_imm(c, K.trailing_zeros() as i64);
            let c = w.f.ins().imul(c, cw);
            let x = w.f.ins().iadd(at, cols);
            let x = w.f.ins().iadd(x, c);
            w.set(be, x);
        });
        let be = w.get(be);
        let mut tail = Vec::new();
        let mut at = w.f.ins().iadd_imm(be, 8);
        for t in &totals {
            tail.push(at);
            let x = w.f.ins().iadd(at, *t);
            at = w.f.ins().iadd_imm(x, 8);
        }
        let x = w.f.ins().iadd_imm(at, 3);
        at = w.f.ins().band_imm(x, !3);
        let starts_each = w.f.ins().iadd_imm(c64, 1);
        let starts_each = w.f.ins().ishl_imm(starts_each, 2);
        for _ in 0..4 {
            tail.push(at);
            at = w.f.ins().iadd(at, starts_each);
        }
        tail.push(at);
        let x = w.f.ins().iadd(at, llen);
        let tend = w.f.ins().iadd_imm(x, 8);
        tail.push(tend);
        write_head(w, out, MAGIC16Z, &tail);
        for (t, v) in [parts_at, ckpt_at, lines_at, starts, lens, kinds, end].into_iter().enumerate() {
            let v = w.f.ins().ireduce(I32, v);
            w.st(v, out, 88 + 4 * t as i32);
        }
        w.ok();
    }

    // ---- LZ4 HC, level 4 (decision 0434 §6) -------------------------------------------------------------------
    //
    // liblz4 1.10.0's `LZ4HC_compress_hashChain` with 8 attempts, no dictionary, no pattern analysis and no chain
    // swap, as `LZ4_compress_HC(.., 4)` runs it: the same sequences, so the same bytes. Positions are i64 offsets
    // from the piece's first byte; an index is a position plus the piece's base (`zbase`).

    const MINMATCH: i64 = 4;
    const MFLIMIT: i64 = 12;
    const LASTLITERALS: i64 = 5;
    const OPTIMAL_ML: i64 = 18;
    const DISTANCE_MAX: i64 = 65535;
    const CHAIN_AT: i64 = 4 << LZ_HASH_LOG;

    impl W<'_, '_> {
        fn r16(&mut self, src: Value, pos: Value) -> Value {
            let a = self.f.ins().iadd(src, pos);
            let v = self.ld16(a, 0);
            self.u64of(v)
        }
        fn r32(&mut self, src: Value, pos: Value) -> Value {
            let a = self.f.ins().iadd(src, pos);
            self.ld(I32, a, 0)
        }
        fn r64(&mut self, src: Value, pos: Value) -> Value {
            let a = self.f.ins().iadd(src, pos);
            self.ld(I64, a, 0)
        }
        /// LZ4 HC's hash of a 4-byte word: its hash table's slot, times 4 (an i64 byte offset).
        fn lz_hash(&mut self, x: Value) -> Value {
            let m = self.f.ins().imul_imm(x, 2654435761u32 as i32 as i64);
            let h = self.f.ins().ushr_imm(m, i64::from(32 - LZ_HASH_LOG));
            let h = self.u64of(h);
            self.f.ins().ishl_imm(h, 2)
        }
        fn slt(&mut self, a: Value, b: Value) -> Value {
            self.f.ins().icmp(IntCC::SignedLessThan, a, b)
        }
        fn sle(&mut self, a: Value, b: Value) -> Value {
            self.f.ins().icmp(IntCC::SignedLessThanOrEqual, a, b)
        }
        fn jump(&mut self, to: ir::Block) {
            self.f.ins().jump(to, &[]);
        }
        fn branch(&mut self, cond: Value, yes: ir::Block, no: ir::Block) {
            self.f.ins().brif(cond, yes, &[], no, &[]);
        }
        /// `k` bytes of 255 at `*op`, then the byte `rest`: LZ4's length past 15 (`len / 255` 255s, `len % 255`).
        fn lz_length(&mut self, op: Variable, len: Value) {
            let d = self.c64(255);
            let k = self.f.ins().udiv(len, d);
            let rest = self.f.ins().urem(len, d);
            let o = self.get(op);
            let ones = self.c64(-1);
            self.call("w_fill", &[o, k, ones]);
            let o = self.f.ins().iadd(o, k);
            self.st8(rest, o, 0);
            let o = self.f.ins().iadd_imm(o, 1);
            self.set(op, o);
        }
        /// `LZ4HC_encodeSequence`: the literals from `anchor` to `ip`, then a match of `ml` at `off`; `ip` past the
        /// match, `anchor` at it.
        fn lz_encode(&mut self, src: Value, ip: Variable, anchor: Variable, op: Variable, ml: Value, off: Value) {
            let (i, a) = (self.get(ip), self.get(anchor));
            let len = self.f.ins().isub(i, a);
            let token = self.get(op);
            let o = self.f.ins().iadd_imm(token, 1);
            self.set(op, o);
            let tok = self.var(I64, len);
            let long = self.f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, len, 15);
            self.if_(long, |w| {
                let ext = w.f.ins().iadd_imm(len, -15);
                w.lz_length(op, ext);
                let t = w.c64(15);
                w.set(tok, t);
            });
            let t = self.get(tok);
            let t = self.f.ins().ishl_imm(t, 4);
            self.set(tok, t);
            let o = self.get(op);
            let from = self.f.ins().iadd(src, a);
            self.call("w_copy", &[o, from, len]);
            let o = self.f.ins().iadd(o, len);
            self.st16(off, o, 0);
            let o = self.f.ins().iadd_imm(o, 2);
            self.set(op, o);
            let l = self.f.ins().iadd_imm(ml, -MINMATCH);
            let long = self.f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, l, 15);
            self.if_else(
                long,
                |w| {
                    let ext = w.f.ins().iadd_imm(l, -15);
                    w.lz_length(op, ext);
                    let t = w.get(tok);
                    let t = w.f.ins().iadd_imm(t, 15);
                    w.set(tok, t);
                },
                |w| {
                    let t = w.get(tok);
                    let t = w.f.ins().iadd(t, l);
                    w.set(tok, t);
                },
            );
            let t = self.get(tok);
            self.st8(t, token, 0);
            let i = self.f.ins().iadd(i, ml);
            self.set(ip, i);
            self.set(anchor, i);
        }
        /// `LZ4HC_InsertAndGetWiderMatch` (by `w_lz4_find`): the match's length, its offset and back count in the
        /// state.
        fn lz_find(&mut self, src: Value, ip: Value, low: Value, high: Value, longest: Value) -> (Value, Value, Value) {
            let len = self.call("w_lz4_find", &[src, ip, low, high, longest]);
            let off = self.get32(at::ZOFF);
            let off = self.u64of(off);
            let back = self.get32(at::ZBACK);
            let back = self.f.ins().sextend(I64, back);
            (len, off, back)
        }
    }

    /// `w_fill(to, len, v)`: `len` bytes of `v`'s, a word at a time, the bytes past `len` kept.
    fn fill(w: &mut W<'_, '_>) {
        let (to, len, v) = (w.arg(0), w.arg(1), w.arg(2));
        let zero = w.c64(0);
        let k = w.var(I64, zero);
        w.while_(
            |w| {
                let k = w.get(k);
                let k8 = w.f.ins().iadd_imm(k, 8);
                w.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, k8, len)
            },
            |w| {
                let kv = w.get(k);
                let d = w.f.ins().iadd(to, kv);
                w.st(v, d, 0);
                let k8 = w.f.ins().iadd_imm(kv, 8);
                w.set(k, k8);
            },
        );
        let kv = w.get(k);
        let rest = w.f.ins().isub(len, kv);
        let some = w.f.ins().icmp_imm(IntCC::NotEqual, rest, 0);
        w.if_(some, |w| {
            let d = w.f.ins().iadd(to, kv);
            w.cell_put(d, rest, v);
        });
        let z = w.zero32();
        w.f.ins().return_(&[z]);
    }

    /// `w_hash(src, n)`: a 64-bit hash of `n` bytes, a word at a time (the seen pieces').
    fn hash(w: &mut W<'_, '_>) {
        let (src, n) = (w.arg(0), w.arg(1));
        let zero = w.c64(0);
        let k = w.var(I64, zero);
        let h = w.var(I64, n);
        let mix = |w: &mut W<'_, '_>, x: Value| {
            let hv = w.get(h);
            let x = w.f.ins().bxor(hv, x);
            let x = w.f.ins().imul_imm(x, 0x9e37_79b9_7f4a_7c15u64 as i64);
            let s = w.f.ins().ushr_imm(x, 29);
            let x = w.f.ins().bxor(x, s);
            w.set(h, x);
        };
        w.while_(
            |w| {
                let k = w.get(k);
                let k8 = w.f.ins().iadd_imm(k, 8);
                w.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, k8, n)
            },
            |w| {
                let kv = w.get(k);
                let x = w.r64(src, kv);
                mix(w, x);
                let k8 = w.f.ins().iadd_imm(kv, 8);
                w.set(k, k8);
            },
        );
        let kv = w.get(k);
        let rest = w.f.ins().isub(n, kv);
        let some = w.f.ins().icmp_imm(IntCC::NotEqual, rest, 0);
        w.if_(some, |w| {
            let a = w.f.ins().iadd(src, kv);
            let x = w.cell_get(a, rest);
            mix(w, x);
        });
        let hv = w.get(h);
        w.f.ins().return_(&[hv]);
    }

    /// `w_eq(a, b, n)`: 1 where `n` bytes at `a` and at `b` are the same, a word at a time.
    fn eq(w: &mut W<'_, '_>) {
        let (a, b, n) = (w.arg(0), w.arg(1), w.arg(2));
        let zero = w.c64(0);
        let k = w.var(I64, zero);
        w.while_(
            |w| {
                let k = w.get(k);
                let k8 = w.f.ins().iadd_imm(k, 8);
                w.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, k8, n)
            },
            |w| {
                let kv = w.get(k);
                let x = w.r64(a, kv);
                let y = w.r64(b, kv);
                let ne = w.f.ins().icmp(IntCC::NotEqual, x, y);
                let z = w.zero32();
                w.if_(ne, |w| w.ret(z));
                let k8 = w.f.ins().iadd_imm(kv, 8);
                w.set(k, k8);
            },
        );
        let kv = w.get(k);
        let rest = w.f.ins().isub(n, kv);
        let pa = w.f.ins().iadd(a, kv);
        let pb = w.f.ins().iadd(b, kv);
        let x = w.cell_get(pa, rest);
        let y = w.cell_get(pb, rest);
        let same = w.f.ins().icmp(IntCC::Equal, x, y);
        let one = w.c32(1);
        let z = w.zero32();
        let r = w.f.ins().select(same, one, z);
        w.f.ins().return_(&[r]);
    }

    /// `w_lz4_find(src, ip, low, high, longest)`: `LZ4HC_InsertAndGetWiderMatch` at level 4 for the piece at
    /// `src`: the positions before `ip` inserted, then up to 8 entries of `ip`'s chain tried for a match longer
    /// than `longest`, reaching back to `low` and on to `high`. Returns its length (`longest` if none), its offset
    /// and back count in the state.
    fn lz4_find(w: &mut W<'_, '_>) {
        let (src, ip, low, high, longest0) = (w.arg(0), w.arg(1), w.arg(2), w.arg(3), w.arg(4));
        let lz = w.part(part::LZ);
        let chain = w.f.ins().iadd_imm(lz, CHAIN_AT);
        let g = w.get32(at::ZBASE);
        let g = w.u64of(g);
        let ip_index = w.f.ins().iadd(g, ip);
        // The positions before `ip` in the hash table and the chain (`LZ4HC_Insert`).
        let next = w.get32(at::ZNEXT);
        let next = w.u64of(next);
        let nu = w.var(I64, next);
        w.while_(
            |w| {
                let v = w.get(nu);
                w.f.ins().icmp(IntCC::UnsignedLessThan, v, ip_index)
            },
            |w| {
                let v = w.get(nu);
                let pos = w.f.ins().isub(v, g);
                let x = w.r32(src, pos);
                let h = w.lz_hash(x);
                let slot = w.f.ins().iadd(lz, h);
                let old = w.ld(I32, slot, 0);
                let old = w.u64of(old);
                let delta = w.f.ins().isub(v, old);
                let max = w.c64(DISTANCE_MAX);
                let delta = w.f.ins().umin(delta, max);
                let c = w.f.ins().band_imm(v, 0xffff);
                let c = w.f.ins().ishl_imm(c, 1);
                let ca = w.f.ins().iadd(chain, c);
                w.st16(delta, ca, 0);
                let v32 = w.f.ins().ireduce(I32, v);
                w.st(v32, slot, 0);
                let v1 = w.f.ins().iadd_imm(v, 1);
                w.set(nu, v1);
            },
        );
        let i32_index = w.f.ins().ireduce(I32, ip_index);
        w.set32(at::ZNEXT, i32_index);
        let within = w.f.ins().iadd_imm(g, DISTANCE_MAX + 1);
        let within = w.f.ins().icmp(IntCC::UnsignedGreaterThan, within, ip_index);
        let far = w.f.ins().iadd_imm(ip_index, -DISTANCE_MAX);
        let lowest = w.f.ins().select(within, g, far);
        let look_back = w.f.ins().isub(ip, low);
        let pattern = w.r32(src, ip);
        let h = w.lz_hash(pattern);
        let slot = w.f.ins().iadd(lz, h);
        let first = w.ld(I32, slot, 0);
        let first = w.u64of(first);
        let mi = w.var(I64, first);
        let attempts = w.c64(i64::from(LZ_ATTEMPTS));
        let tries = w.var(I64, attempts);
        let longest = w.var(I64, longest0);
        let zero = w.c64(0);
        let offset = w.var(I64, zero);
        let s_back = w.var(I64, zero);
        w.while_(
            |w| {
                let m = w.get(mi);
                let t = w.get(tries);
                let a = w.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, m, lowest);
                let b = w.f.ins().icmp_imm(IntCC::SignedGreaterThan, t, 0);
                w.f.ins().band(a, b)
            },
            |w| {
                let t = w.get(tries);
                let t = w.f.ins().iadd_imm(t, -1);
                w.set(tries, t);
                let m = w.get(mi);
                let mp = w.f.ins().isub(m, g);
                let l = w.get(longest);
                let at_low = w.f.ins().iadd(low, l);
                let at_low = w.f.ins().iadd_imm(at_low, -1);
                let a = w.r16(src, at_low);
                let at_m = w.f.ins().isub(mp, look_back);
                let at_m = w.f.ins().iadd(at_m, l);
                let at_m = w.f.ins().iadd_imm(at_m, -1);
                let b = w.r16(src, at_m);
                let same16 = w.f.ins().icmp(IntCC::Equal, a, b);
                let mv = w.r32(src, mp);
                let same32 = w.f.ins().icmp(IntCC::Equal, mv, pattern);
                let cand = w.f.ins().band(same16, same32);
                w.if_(cand, |w| {
                    let back = lz_count_back(w, src, ip, mp, low);
                    let ip4 = w.f.ins().iadd_imm(ip, MINMATCH);
                    let mp4 = w.f.ins().iadd_imm(mp, MINMATCH);
                    let c = lz_count(w, src, ip4, mp4, high);
                    let ml = w.f.ins().iadd_imm(c, MINMATCH);
                    let ml = w.f.ins().isub(ml, back);
                    let l = w.get(longest);
                    let better = w.f.ins().icmp(IntCC::SignedGreaterThan, ml, l);
                    w.if_(better, |w| {
                        w.set(longest, ml);
                        let o = w.f.ins().isub(ip_index, m);
                        w.set(offset, o);
                        w.set(s_back, back);
                    });
                });
                let c = w.f.ins().band_imm(m, 0xffff);
                let c = w.f.ins().ishl_imm(c, 1);
                let ca = w.f.ins().iadd(chain, c);
                let d = w.ld16(ca, 0);
                let d = w.u64of(d);
                let m2 = w.f.ins().isub(m, d);
                w.set(mi, m2);
            },
        );
        let o = w.get(offset);
        let o = w.f.ins().ireduce(I32, o);
        w.set32(at::ZOFF, o);
        let b = w.get(s_back);
        let b = w.f.ins().ireduce(I32, b);
        w.set32(at::ZBACK, b);
        let l = w.get(longest);
        w.f.ins().return_(&[l]);
    }

    /// `LZ4HC_countBack`: minus the bytes before `ip` and `mp` that are the same, down to `low` and to the piece's
    /// first byte; a word at a time, the last word's top bytes by its leading zeros.
    fn lz_count_back(w: &mut W<'_, '_>, src: Value, ip: Value, mp: Value, low: Value) -> Value {
        let a = w.f.ins().isub(low, ip);
        let zero = w.c64(0);
        let b = w.f.ins().isub(zero, mp);
        let min = w.f.ins().smax(a, b);
        let back = w.var(I64, zero);
        let done = w.var(I64, zero);
        w.while_(
            |w| {
                let bk = w.get(back);
                let d = w.f.ins().isub(bk, min);
                let room = w.f.ins().icmp_imm(IntCC::SignedGreaterThan, d, 3);
                let dn = w.get(done);
                let go = w.f.ins().icmp_imm(IntCC::Equal, dn, 0);
                w.f.ins().band(room, go)
            },
            |w| {
                let bk = w.get(back);
                let p = w.f.ins().iadd(ip, bk);
                let p = w.f.ins().iadd_imm(p, -4);
                let q = w.f.ins().iadd(mp, bk);
                let q = w.f.ins().iadd_imm(q, -4);
                let x = w.r32(src, p);
                let y = w.r32(src, q);
                let v = w.f.ins().bxor(x, y);
                let differ = w.f.ins().icmp_imm(IntCC::NotEqual, v, 0);
                let common = w.f.ins().clz(v);
                let common = w.f.ins().ushr_imm(common, 3);
                let common = w.u64of(common);
                let at_diff = w.f.ins().isub(bk, common);
                let step = w.f.ins().iadd_imm(bk, -4);
                let nb = w.f.ins().select(differ, at_diff, step);
                w.set(back, nb);
                let one = w.c64(1);
                let zero = w.c64(0);
                let dn = w.f.ins().select(differ, one, zero);
                w.set(done, dn);
            },
        );
        let dn = w.get(done);
        let open = w.f.ins().icmp_imm(IntCC::Equal, dn, 0);
        w.if_(open, |w| {
            // The remainder: fewer than 4 bytes, the top ones of the word that ends at `back`.
            let bk = w.get(back);
            let rest = w.f.ins().isub(bk, min);
            let some = w.f.ins().icmp_imm(IntCC::SignedGreaterThan, rest, 0);
            w.if_(some, |w| {
                let p = w.f.ins().iadd(ip, bk);
                let p = w.f.ins().iadd_imm(p, -4);
                let q = w.f.ins().iadd(mp, bk);
                let q = w.f.ins().iadd_imm(q, -4);
                let x = w.r32(src, p);
                let y = w.r32(src, q);
                let v = w.f.ins().bxor(x, y);
                let common = w.f.ins().clz(v);
                let common = w.f.ins().ushr_imm(common, 3);
                let common = w.u64of(common);
                let common = w.f.ins().umin(common, rest);
                let nb = w.f.ins().isub(bk, common);
                w.set(back, nb);
            });
        });
        w.get(back)
    }

    /// `LZ4_count`: the bytes from `p` and `m` that are the same, up to `limit`, a word at a time.
    fn lz_count(w: &mut W<'_, '_>, src: Value, p: Value, m: Value, limit: Value) -> Value {
        let zero = w.c64(0);
        let k = w.var(I64, zero);
        let found = w.c64(-1);
        let result = w.var(I64, found);
        w.while_(
            |w| {
                let kv = w.get(k);
                let at = w.f.ins().iadd(p, kv);
                let at8 = w.f.ins().iadd_imm(at, 8);
                let room = w.sle(at8, limit);
                let r = w.get(result);
                let open = w.f.ins().icmp_imm(IntCC::SignedLessThan, r, 0);
                w.f.ins().band(room, open)
            },
            |w| {
                let kv = w.get(k);
                let a = w.f.ins().iadd(p, kv);
                let b = w.f.ins().iadd(m, kv);
                let x = w.r64(src, a);
                let y = w.r64(src, b);
                let v = w.f.ins().bxor(x, y);
                let differ = w.f.ins().icmp_imm(IntCC::NotEqual, v, 0);
                w.if_(differ, |w| {
                    let t = w.f.ins().ctz(v);
                    let t = w.f.ins().ushr_imm(t, 3);
                    let r = w.f.ins().iadd(kv, t);
                    w.set(result, r);
                });
                let k8 = w.f.ins().iadd_imm(kv, 8);
                w.set(k, k8);
            },
        );
        let r = w.get(result);
        let open = w.f.ins().icmp_imm(IntCC::SignedLessThan, r, 0);
        w.if_(open, |w| {
            let kv = w.get(k);
            let at = w.f.ins().iadd(p, kv);
            let rest = w.f.ins().isub(limit, at);
            let zero = w.c64(0);
            let rest = w.f.ins().smax(rest, zero);
            let a = w.f.ins().iadd(src, at);
            let b = w.f.ins().iadd(m, kv);
            let b = w.f.ins().iadd(src, b);
            let x = w.cell_get(a, rest);
            let y = w.cell_get(b, rest);
            let v = w.f.ins().bxor(x, y);
            let t = w.f.ins().ctz(v);
            let t = w.f.ins().ushr_imm(t, 3);
            let t = w.f.ins().umin(t, rest);
            let r = w.f.ins().iadd(kv, t);
            w.set(result, r);
        });
        w.get(result)
    }

    /// `w_lz4(src, n, dst)`: the `n` bytes at `src` as one LZ4 block at `dst` (`LZ4_compress_HC(.., 4)`'s bytes);
    /// returns its length. `dst` has room for `lz4_bound(n)` bytes, and the 8 past a cell; `src`'s 3 bytes before
    /// it and 8 after are readable.
    fn lz4(w: &mut W<'_, '_>) {
        let (src, n, dst) = (w.arg(0), w.arg(1), w.arg(2));
        // `LZ4HC_init_internal`, the context reused: this piece's indices start 64 KB past the last one's end.
        let base = w.get32(at::ZBASE);
        let base = w.u64of(base);
        let prev = w.get32(at::ZPREV);
        let prev = w.u64of(prev);
        let start = w.f.ins().iadd(base, prev);
        let startv = w.var(I64, start);
        let over = w.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, start, 1 << 30);
        w.if_(over, |w| {
            let lz = w.part(part::LZ);
            let len = w.c64(i64::from(4 << LZ_HASH_LOG));
            w.call("w_zero", &[lz, len]);
            let z = w.c64(0);
            w.set(startv, z);
        });
        let start = w.get(startv);
        let g = w.f.ins().iadd_imm(start, 65536);
        let g32 = w.f.ins().ireduce(I32, g);
        w.set32(at::ZBASE, g32);
        w.set32(at::ZNEXT, g32);
        let n32 = w.f.ins().ireduce(I32, n);
        w.set32(at::ZPREV, n32);
        let zero = w.c64(0);
        let ip = w.var(I64, zero);
        let anchor = w.var(I64, zero);
        let op = w.var(I64, dst);
        let start0 = w.var(I64, zero);
        let start2 = w.var(I64, zero);
        let start3 = w.var(I64, zero);
        let (m0l, m0o) = (w.var(I64, zero), w.var(I64, zero));
        let (m1l, m1o) = (w.var(I64, zero), w.var(I64, zero));
        let (m2l, m2o) = (w.var(I64, zero), w.var(I64, zero));
        let (m3l, m3o) = (w.var(I64, zero), w.var(I64, zero));
        let mflimit = w.f.ins().iadd_imm(n, -MFLIMIT);
        let matchlimit = w.f.ins().iadd_imm(n, -LASTLITERALS);
        let main = w.f.create_block();
        let body = w.f.create_block();
        let search2 = w.f.create_block();
        let search3 = w.f.create_block();
        let last = w.f.create_block();
        let small = w.f.ins().icmp_imm(IntCC::SignedLessThan, n, MFLIMIT + 1);
        w.branch(small, last, main);

        // The main loop.
        w.f.switch_to_block(main);
        let i = w.get(ip);
        let go = w.sle(i, mflimit);
        w.branch(go, body, last);

        w.f.switch_to_block(body);
        let i = w.get(ip);
        let three = w.c64(MINMATCH - 1);
        let (l, o, _) = w.lz_find(src, i, i, matchlimit, three);
        let none = w.f.ins().icmp_imm(IntCC::SignedLessThan, l, MINMATCH);
        let found = w.f.create_block();
        let skip = w.f.create_block();
        w.branch(none, skip, found);
        w.f.switch_to_block(skip);
        let i1 = w.f.ins().iadd_imm(i, 1);
        w.set(ip, i1);
        w.jump(main);
        w.f.switch_to_block(found);
        w.set(start0, i);
        w.set(m0l, l);
        w.set(m0o, o);
        w.set(m1l, l);
        w.set(m1o, o);
        w.jump(search2);

        // _Search2.
        w.f.switch_to_block(search2);
        let i = w.get(ip);
        let l1 = w.get(m1l);
        let end1 = w.f.ins().iadd(i, l1);
        let room = w.sle(end1, mflimit);
        w.if_else(
            room,
            |w| {
                let s2 = w.f.ins().iadd_imm(end1, -2);
                let (l, o, b) = w.lz_find(src, s2, i, matchlimit, l1);
                let s2 = w.f.ins().iadd(s2, b);
                w.set(start2, s2);
                w.set(m2l, l);
                w.set(m2o, o);
            },
            |w| {
                let z = w.c64(0);
                w.set(m2l, z);
                w.set(m2o, z);
            },
        );
        let l2 = w.get(m2l);
        let l1 = w.get(m1l);
        let no_better = w.sle(l2, l1);
        let encode1 = w.f.create_block();
        let better = w.f.create_block();
        w.branch(no_better, encode1, better);
        w.f.switch_to_block(encode1);
        let o1 = w.get(m1o);
        w.lz_encode(src, ip, anchor, op, l1, o1);
        w.jump(main);
        w.f.switch_to_block(better);
        let i = w.get(ip);
        let s0 = w.get(start0);
        let s2 = w.get(start2);
        let l0 = w.get(m0l);
        let skipped = w.slt(s0, i);
        let e0 = w.f.ins().iadd(i, l0);
        let squeeze = w.slt(s2, e0);
        let restore = w.f.ins().band(skipped, squeeze);
        w.if_(restore, |w| {
            w.set(ip, s0);
            let (l, o) = (w.get(m0l), w.get(m0o));
            w.set(m1l, l);
            w.set(m1o, o);
        });
        let i = w.get(ip);
        let gap = w.f.ins().isub(s2, i);
        let tiny = w.f.ins().icmp_imm(IntCC::SignedLessThan, gap, 3);
        let drop1 = w.f.create_block();
        w.branch(tiny, drop1, search3);
        w.f.switch_to_block(drop1);
        w.set(ip, s2);
        let (l, o) = (w.get(m2l), w.get(m2o));
        w.set(m1l, l);
        w.set(m1o, o);
        w.jump(search2);

        // _Search3.
        w.f.switch_to_block(search3);
        let i = w.get(ip);
        let s2 = w.get(start2);
        let gap = w.f.ins().isub(s2, i);
        let short = w.f.ins().icmp_imm(IntCC::SignedLessThan, gap, OPTIMAL_ML);
        w.if_(short, |w| {
            let l1 = w.get(m1l);
            let cap = w.c64(OPTIMAL_ML);
            let new_ml = w.f.ins().smin(l1, cap);
            let nm = w.var(I64, new_ml);
            let l2 = w.get(m2l);
            let lim = w.f.ins().iadd(s2, l2);
            let lim = w.f.ins().iadd_imm(lim, -MINMATCH);
            let reach = w.f.ins().iadd(i, new_ml);
            let past = w.f.ins().icmp(IntCC::SignedGreaterThan, reach, lim);
            w.if_(past, |w| {
                let x = w.f.ins().iadd(gap, l2);
                let x = w.f.ins().iadd_imm(x, -MINMATCH);
                w.set(nm, x);
            });
            let nmv = w.get(nm);
            let corr = w.f.ins().isub(nmv, gap);
            let pos = w.f.ins().icmp_imm(IntCC::SignedGreaterThan, corr, 0);
            w.if_(pos, |w| {
                let s = w.f.ins().iadd(s2, corr);
                w.set(start2, s);
                let l = w.f.ins().isub(l2, corr);
                w.set(m2l, l);
            });
        });
        let s2 = w.get(start2);
        let l2 = w.get(m2l);
        let end2 = w.f.ins().iadd(s2, l2);
        let room = w.sle(end2, mflimit);
        w.if_else(
            room,
            |w| {
                let s3 = w.f.ins().iadd_imm(end2, -3);
                let (l, o, b) = w.lz_find(src, s3, s2, matchlimit, l2);
                let s3 = w.f.ins().iadd(s3, b);
                w.set(start3, s3);
                w.set(m3l, l);
                w.set(m3o, o);
            },
            |w| {
                let z = w.c64(0);
                w.set(m3l, z);
                w.set(m3o, z);
            },
        );
        let l3 = w.get(m3l);
        let l2 = w.get(m2l);
        let no_better = w.sle(l3, l2);
        let encode2 = w.f.create_block();
        let three_m = w.f.create_block();
        w.branch(no_better, encode2, three_m);
        // No better third: the first two encoded.
        w.f.switch_to_block(encode2);
        let i = w.get(ip);
        let s2 = w.get(start2);
        let l1 = w.get(m1l);
        let e1 = w.f.ins().iadd(i, l1);
        let cut = w.slt(s2, e1);
        w.if_(cut, |w| {
            let x = w.f.ins().isub(s2, i);
            w.set(m1l, x);
        });
        let (l1, o1) = (w.get(m1l), w.get(m1o));
        w.lz_encode(src, ip, anchor, op, l1, o1);
        w.set(ip, s2);
        let (l2, o2) = (w.get(m2l), w.get(m2o));
        w.lz_encode(src, ip, anchor, op, l2, o2);
        w.jump(main);

        w.f.switch_to_block(three_m);
        let i = w.get(ip);
        let s2 = w.get(start2);
        let s3 = w.get(start3);
        let l1 = w.get(m1l);
        let e1 = w.f.ins().iadd(i, l1);
        let e13 = w.f.ins().iadd_imm(e1, 3);
        let no_room = w.slt(s3, e13);
        let crowded = w.f.create_block();
        let ascending = w.f.create_block();
        w.branch(no_room, crowded, ascending);
        w.f.switch_to_block(crowded);
        let can = w.f.ins().icmp(IntCC::SignedGreaterThanOrEqual, s3, e1);
        let write1 = w.f.create_block();
        let swap = w.f.create_block();
        w.branch(can, write1, swap);
        // The first written now; the second dropped, the third becomes the first.
        w.f.switch_to_block(write1);
        let over = w.slt(s2, e1);
        w.if_(over, |w| {
            let corr = w.f.ins().isub(e1, s2);
            let s = w.f.ins().iadd(s2, corr);
            w.set(start2, s);
            let l2 = w.get(m2l);
            let l = w.f.ins().isub(l2, corr);
            w.set(m2l, l);
            let short = w.f.ins().icmp_imm(IntCC::SignedLessThan, l, MINMATCH);
            w.if_(short, |w| {
                w.set(start2, s3);
                let (l, o) = (w.get(m3l), w.get(m3o));
                w.set(m2l, l);
                w.set(m2o, o);
            });
        });
        let o1 = w.get(m1o);
        w.lz_encode(src, ip, anchor, op, l1, o1);
        w.set(ip, s3);
        let (l, o) = (w.get(m3l), w.get(m3o));
        w.set(m1l, l);
        w.set(m1o, o);
        let s2 = w.get(start2);
        w.set(start0, s2);
        let (l, o) = (w.get(m2l), w.get(m2o));
        w.set(m0l, l);
        w.set(m0o, o);
        w.jump(search2);
        w.f.switch_to_block(swap);
        w.set(start2, s3);
        let (l, o) = (w.get(m3l), w.get(m3o));
        w.set(m2l, l);
        w.set(m2o, o);
        w.jump(search3);

        // Three ascending matches: the first written, the second becomes the first, the third the second.
        w.f.switch_to_block(ascending);
        let i = w.get(ip);
        let s2 = w.get(start2);
        let l1 = w.get(m1l);
        let e1 = w.f.ins().iadd(i, l1);
        let over = w.slt(s2, e1);
        w.if_(over, |w| {
            let gap = w.f.ins().isub(s2, i);
            let short = w.f.ins().icmp_imm(IntCC::SignedLessThan, gap, OPTIMAL_ML);
            w.if_else(
                short,
                |w| {
                    let l1 = w.get(m1l);
                    let cap = w.c64(OPTIMAL_ML);
                    let l1 = w.f.ins().smin(l1, cap);
                    w.set(m1l, l1);
                    let l2 = w.get(m2l);
                    let lim = w.f.ins().iadd(s2, l2);
                    let lim = w.f.ins().iadd_imm(lim, -MINMATCH);
                    let reach = w.f.ins().iadd(i, l1);
                    let past = w.f.ins().icmp(IntCC::SignedGreaterThan, reach, lim);
                    w.if_(past, |w| {
                        let x = w.f.ins().iadd(gap, l2);
                        let x = w.f.ins().iadd_imm(x, -MINMATCH);
                        w.set(m1l, x);
                    });
                    let l1 = w.get(m1l);
                    let corr = w.f.ins().isub(l1, gap);
                    let pos = w.f.ins().icmp_imm(IntCC::SignedGreaterThan, corr, 0);
                    w.if_(pos, |w| {
                        let s = w.f.ins().iadd(s2, corr);
                        w.set(start2, s);
                        let l = w.f.ins().isub(l2, corr);
                        w.set(m2l, l);
                    });
                },
                |w| w.set(m1l, gap),
            );
        });
        let (l1, o1) = (w.get(m1l), w.get(m1o));
        w.lz_encode(src, ip, anchor, op, l1, o1);
        let s2 = w.get(start2);
        w.set(ip, s2);
        let (l, o) = (w.get(m2l), w.get(m2o));
        w.set(m1l, l);
        w.set(m1o, o);
        let s3 = w.get(start3);
        w.set(start2, s3);
        let (l, o) = (w.get(m3l), w.get(m3o));
        w.set(m2l, l);
        w.set(m2o, o);
        w.jump(search3);

        // _last_literals.
        w.f.switch_to_block(last);
        let a = w.get(anchor);
        let run = w.f.ins().isub(n, a);
        let token = w.get(op);
        let o = w.f.ins().iadd_imm(token, 1);
        w.set(op, o);
        let long = w.f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, run, 15);
        w.if_else(
            long,
            |w| {
                let t = w.c32(0xf0);
                w.st8(t, token, 0);
                let ext = w.f.ins().iadd_imm(run, -15);
                w.lz_length(op, ext);
            },
            |w| {
                let t = w.f.ins().ishl_imm(run, 4);
                w.st8(t, token, 0);
            },
        );
        let o = w.get(op);
        let from = w.f.ins().iadd(src, a);
        w.call("w_copy", &[o, from, run]);
        let o = w.f.ins().iadd(o, run);
        let len = w.f.ins().isub(o, dst);
        w.f.ins().return_(&[len]);
    }
}


#[cfg(all(test, feature = "ir"))]
mod tests {
    /// The tape crate's writer is the ops' Rust (decision 0434 §6).
    #[test]
    fn the_tape_crates_writer_is_this() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../tape_format/src/generated_writer.rs");
        let source = super::rust_source().unwrap();
        if std::env::var_os("RERAIL_WRITE_READS").is_some() {
            std::fs::write(path, &source).unwrap();
        }
        let held = std::fs::read_to_string(path).unwrap_or_default();
        assert!(held == source, "{path} is not the writer's Rust: RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_tape_crates_writer_is_this");
    }

    /// The kernels' writer is the ops' UKL, the library's `<ukl/writer.h>` (decisions 0434 §6 R15, 0440 §3.3).
    #[test]
    fn the_tape_crates_writer_kernels_are_these() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../ukl/lib/ukl/writer.h");
        let source = super::ukl_source().unwrap();
        if std::env::var_os("RERAIL_WRITE_READS").is_some() {
            std::fs::write(path, &source).unwrap();
        }
        let held = std::fs::read_to_string(path).unwrap_or_default();
        assert!(held == source, "{path} is not the writer's UKL: RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_tape_crates_writer_kernels_are_these");
    }

    /// Every op's function passes Cranelift's verifier.
    #[test]
    fn the_ops_verify() {
        let flags = cranelift_codegen::settings::Flags::new(cranelift_codegen::settings::builder());
        for (_, f, op) in super::funcs() {
            if let Err(e) = cranelift_codegen::verify_function(&f, &flags) {
                panic!("{}: {e}\n{}", op.name, f.display());
            }
        }
    }
}
