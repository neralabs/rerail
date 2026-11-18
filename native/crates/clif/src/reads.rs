//! The tape's primitive reads as CLIF functions, written once (decision 0310): the reads
//! `rerail_tape_format::f16::View` and format 15's columns make, as `metal_tape.metal` ports them.
//!
//! A read takes the tape's base (the bytes its view's offsets count from), its view
//! (`view::TapeView`) and a document-local node, and returns a u32 (`NONE` for no node). Format 16's
//! value too wide for its array is all ones and lies in its side list: the node's part, its
//! 256-node block's offsets, a search for its low byte (`f16_side`, cold). A read built over another
//! calls it by name; the JIT's pass inlines the hot ones (`inline`).
//!
//! Every load here reads the tape or its view, which no code writes while it is read (`readonly`).
//! Format 16's arrays have 8 bytes of room past their last value, so a value is one word's load.

use crate::view::{F15_STRIDE_SHIFT, F15_TAIL_FROM_START, F16_BLOCK, F16_K, F16_PART, F16_SUB, at};
use cranelift_codegen::ir::{
    self, AbiParam, Function, InstBuilder, MemFlags, Signature, UserFuncName, Value,
    condcodes::IntCC,
    types::{I32, I64},
};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

/// No node.
pub const NONE: i64 = u32::MAX as i64;

/// A family of reads: what a read's first parameters are (decision 0325). Each is a pointer its loads count from;
/// a family's reads call only each other, passing the same pointers on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// The tape's (`tape_*`, decision 0310): the tape's base (the bytes the view's offsets count from) and its view
    /// (`view::TapeView`, whose loads are aligned: its fields).
    Tape,
    /// A relation's (`rows_*`, `rel_*`; `crate::relation`): the descriptor's address (a `rerail_blocks`
    /// `Relation`, or its `Rows`), every load an offset from it through the block's self-relative references, and
    /// aligned (the block's own words: the Rust backend checks none against a length).
    Relation,
}

impl Family {
    /// The pointers a read of this family takes before its own parameters.
    pub fn pointers(self) -> usize {
        match self {
            Family::Tape => 2,
            Family::Relation => 1,
        }
    }
}

/// A read: its family, its name, its parameters after its family's pointers, its result, whether the JIT's pass
/// inlines it (a search, or the rare case of a read, is cold: it stays a call), and its body.
pub struct Read {
    pub family: Family,
    pub name: &'static str,
    pub params: &'static [ir::Type],
    pub ret: ir::Type,
    pub hot: bool,
    body: fn(&mut B<'_, '_>) -> Value,
}

/// Every read.
pub const READS: &[Read] = &[
    Read { family: Family::Tape, name: "tape_f16_side", params: &[I32, I32], ret: I64, hot: false, body: f16_side },
    Read { family: Family::Tape, name: "tape_size", params: &[I32], ret: I32, hot: true, body: size },
    Read { family: Family::Tape, name: "tape_number", params: &[I32], ret: I32, hot: true, body: number },
    Read { family: Family::Tape, name: "tape_start", params: &[I32], ret: I32, hot: true, body: start },
    Read { family: Family::Tape, name: "tape_end", params: &[I32], ret: I32, hot: true, body: end },
    Read { family: Family::Tape, name: "tape_parent", params: &[I32], ret: I32, hot: false, body: parent },
    Read { family: Family::Tape, name: "tape_end_of", params: &[I32], ret: I32, hot: true, body: end_of },
    Read { family: Family::Tape, name: "tape_next_in", params: &[I32, I32], ret: I32, hot: true, body: next_in },
    Read { family: Family::Tape, name: "tape_next", params: &[I32], ret: I32, hot: true, body: next },
    Read { family: Family::Tape, name: "tape_first_child", params: &[I32], ret: I32, hot: true, body: first_child },
    Read { family: Family::Tape, name: "tape_child_count", params: &[I32], ret: I32, hot: true, body: child_count },
    Read { family: Family::Tape, name: "tape_child", params: &[I32, I32], ret: I32, hot: true, body: child },
    Read { family: Family::Tape, name: "tape_length", params: &[I32], ret: I32, hot: true, body: length },
    Read { family: Family::Tape, name: "tape_start_next", params: &[I32, I32], ret: I32, hot: true, body: start_next },
    Read { family: Family::Tape, name: "tape_slot", params: &[I32], ret: I64, hot: true, body: slot },
    Read { family: Family::Tape, name: "tape_line_select", params: &[I32, I32], ret: I32, hot: false, body: line_select },
    Read { family: Family::Tape, name: "tape_line", params: &[I32], ret: I32, hot: true, body: line },
    Read { family: Family::Tape, name: "tape_line_start", params: &[I32], ret: I32, hot: true, body: line_start },
    Read { family: Family::Relation, name: "rows_col", params: &[I32], ret: I64, hot: true, body: crate::relation::rows_col },
    Read { family: Family::Relation, name: "col_at", params: &[I32, I32], ret: I64, hot: true, body: crate::relation::col_at },
    Read { family: Family::Relation, name: "col_get", params: &[I32, I32], ret: I64, hot: true, body: crate::relation::col_get },
    Read { family: Family::Relation, name: "rows_at", params: &[I32, I32], ret: I64, hot: true, body: crate::relation::rows_at },
    Read { family: Family::Relation, name: "rows_width", params: &[I32], ret: I32, hot: true, body: crate::relation::rows_width },
    Read { family: Family::Relation, name: "rows_get", params: &[I32, I32], ret: I64, hot: true, body: crate::relation::rows_get },
    Read { family: Family::Relation, name: "rel_get", params: &[I32, I32], ret: I64, hot: true, body: crate::relation::rel_get },
    Read { family: Family::Relation, name: "rel_dead", params: &[I32], ret: I32, hot: true, body: crate::relation::rel_dead },
    Read { family: Family::Relation, name: "rel_group", params: &[I32, I64], ret: I64, hot: true, body: crate::relation::rel_group },
    Read { family: Family::Relation, name: "rel_group_row", params: &[I32, I32], ret: I32, hot: true, body: crate::relation::rel_group_row },
    Read { family: Family::Relation, name: "rel_key_mix", params: &[I64, I64], ret: I64, hot: true, body: crate::relation::rel_key_mix },
    // A projection's numbering's (decision 0353 step 2): the descriptor is an `ids::Numbering`.
    Read { family: Family::Relation, name: "num_is", params: &[I32, I32], ret: I32, hot: true, body: crate::id_reads::is },
    Read { family: Family::Relation, name: "num_rank", params: &[I32, I32], ret: I32, hot: true, body: crate::id_reads::rank },
    Read { family: Family::Relation, name: "num_select", params: &[I32, I32], ret: I32, hot: true, body: crate::id_reads::select },
    Read { family: Family::Relation, name: "num_granule", params: &[I32, I32], ret: I64, hot: false, body: crate::id_reads::granule },
    Read { family: Family::Relation, name: "num_low_of", params: &[I32, I32], ret: I32, hot: true, body: crate::id_reads::low_of },
    Read { family: Family::Relation, name: "num_number_in", params: &[I32, I32, I32, I32], ret: I32, hot: false, body: crate::id_reads::number_in },
    Read { family: Family::Relation, name: "num_number_of", params: &[I32], ret: I64, hot: true, body: crate::id_reads::number_of },
    Read { family: Family::Relation, name: "num_order_of", params: &[I32], ret: I32, hot: true, body: crate::id_reads::order_of },
    Read { family: Family::Relation, name: "num_number_of_order", params: &[I32], ret: I32, hot: true, body: crate::id_reads::number_of_order },
];

/// The reads of format 16 alone: a side list's value, and a node's slot (16z's parent steps read it block by
/// block).
pub const F16_ONLY: &[&str] = &["tape_f16_side", "tape_slot", "tape_line_select"];

/// The reads a reader of the tape crate calls (the rest are the reads' own: a side list's value takes a list's
/// number, which is the reads' to give).
pub fn public(name: &str) -> bool {
    name != "tape_f16_side" && name != "tape_line_select"
}

/// The read named `name`.
pub fn read(name: &str) -> Option<&'static Read> {
    READS.iter().find(|read| read.name == name)
}

/// A read's signature: its family's pointers (the tape's base and its view; a relation's descriptor), then its own
/// parameters.
pub fn signature(read: &Read, call_conv: CallConv) -> Signature {
    let mut signature = Signature::new(call_conv);
    signature.params.extend((0..read.family.pointers()).map(|_| AbiParam::new(I64)));
    signature.params.extend(read.params.iter().map(|&ty| AbiParam::new(ty)));
    signature.returns.push(AbiParam::new(read.ret));
    signature
}

/// Where a read's calls of other reads go: the caller's reference to the read named so.
pub trait Callee {
    fn func_ref(&mut self, name: &str, signature: &Signature, func: &mut Function) -> ir::FuncRef;
}

/// The read's body as a function named `name`, its calls of other reads through `callee`: for a tape of
/// either format, as its view says (the JIT's, which runs one program over documents of both).
pub fn build(read: &Read, call_conv: CallConv, name: UserFuncName, callee: &mut dyn Callee) -> Function {
    build_for(read, call_conv, name, callee, None)
}

/// The read's body for tapes of `format` alone (15 or 16; `None` either, as its view says): the other's code
/// is not built, so a reader of one format (the tape crate's `F16`, `F15`) tests none.
pub fn build_for(read: &Read, call_conv: CallConv, name: UserFuncName, callee: &mut dyn Callee, format: Option<u32>) -> Function {
    let mut func = Function::with_name_signature(name, signature(read, call_conv));
    let mut context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(&mut func, &mut context);
    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    let params = f.block_params(entry).to_vec();
    let pointers = read.family.pointers();
    let mut b = B { f, tape: params[0], view: (pointers > 1).then(|| params[1]), args: params[pointers..].to_vec(), call_conv, callee, format };
    let value = (read.body)(&mut b);
    b.f.ins().return_(&[value]);
    b.f.seal_all_blocks();
    b.f.finalize();
    func
}

/// The reads as Rust (decision 0310's Rust backend): a module a format (`f16`, `f15`), each read built for
/// it (`build_for`) and translated (`rust::translate_reads`), a hot one inlined where it is called, as the
/// JIT's pass inlines it; the source ends with its fingerprint. The tape crate holds it, made again by its
/// test (`the_tape_crates_reads_are_these`).
pub fn rust_source() -> Result<String, String> {
    let mut out = String::from(
        "// Generated by rerail_clif::reads::rust_source from the tape's reads in CLIF (decision 0310). Do not edit:\n\
         // `RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_tape_crates_reads_are_these` writes it again.\n\
         #![allow(dead_code, unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_parens, clippy::all)]\n",
    );
    for (module, format) in [("f16", 16), ("f15", 15)] {
        // Format 15 has no side lists.
        let reads: Vec<(u32, &Read)> = READS
            .iter()
            .enumerate()
            .filter(|(_, read)| read.family == Family::Tape && (format == 16 || !F16_ONLY.contains(&read.name)))
            .map(|(k, read)| (k as u32, read))
            .collect();
        let funcs: Vec<(u32, Function, bool)> =
            reads.iter().map(|&(k, read)| (k, build_for(read, CallConv::SystemV, UserFuncName::user(0, k), &mut ByIndex, Some(format)), read.hot)).collect();
        let entries: Vec<(u32, &str)> = reads.iter().filter(|(_, read)| public(read.name)).map(|&(k, read)| (k, read.name)).collect();
        let body = crate::rust::translate_reads(&funcs, &entries, Family::Tape).map_err(|e| format!("format {format}: {e}"))?;
        out.push_str(&format!("\n/// The reads of a format {format} tape.\npub mod {module} {{\n"));
        for line in body.lines() {
            if line.is_empty() {
                out.push('\n');
            } else {
                out.push_str(&format!("    {line}\n"));
            }
        }
        out.push_str("}\n");
    }
    let h = crate::rust::fingerprint(&out);
    out.push_str(&format!("\n/// This source's fingerprint (`rust::fingerprint` of what comes before).\npub const SOURCE: u64 = {h:#x};\n"));
    Ok(out)
}

/// The reads as MSL (decision 0310's Metal backend): each read built for either format, as its view says (a batch
/// holds documents of both), `clif_` before its name, for `metal_tape.metal`'s readers to call. Made again by
/// `the_kernels_reads_are_these`.
pub fn msl_source() -> Result<String, String> {
    let funcs: Vec<(u32, Function, &str)> =
        READS
            .iter()
            .enumerate()
            .filter(|(_, read)| read.family == Family::Tape)
            .map(|(k, read)| (k as u32, build(read, CallConv::SystemV, UserFuncName::user(0, k as u32), &mut ByIndex), read.name))
            .collect();
    let body = crate::msl::translate_reads(&funcs, "clif_", Family::Tape)?;
    Ok(format!(
        "// Generated by rerail_clif::reads::msl_source from the tape's reads in CLIF (decision 0310). Do not edit:\n\
         // `RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_kernels_reads_are_these` writes it again.\n{body}"
    ))
}

/// Where a read's calls of other reads go when it is translated: by its place in `READS`, as the translators name
/// functions.
struct ByIndex;
impl Callee for ByIndex {
    fn func_ref(&mut self, name: &str, signature: &Signature, func: &mut Function) -> ir::FuncRef {
        let index = READS.iter().position(|read| read.name == name).expect("a read") as u32;
        let name = func.declare_imported_user_function(ir::UserExternalName::new(0, index));
        let signature = func.import_signature(signature.clone());
        func.import_function(ir::ExtFuncData { name: ir::ExternalName::User(name), signature, colocated: true })
    }
}

/// Whether a read of the relation's family is a projection's numbering's (`id_reads`), which the facts crate holds.
fn numbering(read: &Read) -> bool {
    read.name.starts_with("num_")
}

/// The relation's reads (decision 0325), each built and named by its place in `READS`, with whether it is hot.
fn relation_funcs() -> Vec<(u32, Function, &'static Read)> {
    family_funcs(|read| !numbering(read))
}

/// The reads of the relation's family that `keep` keeps, each built and named by its place in `READS`.
fn family_funcs(keep: impl Fn(&Read) -> bool) -> Vec<(u32, Function, &'static Read)> {
    READS
        .iter()
        .enumerate()
        .filter(|(_, read)| read.family == Family::Relation && keep(read))
        .map(|(k, read)| (k as u32, build(read, CallConv::SystemV, UserFuncName::user(0, k as u32), &mut ByIndex), read))
        .collect()
}

/// A relation's reads as Rust (decision 0325): `rerail_blocks`' `generated_reads.rs`, which its `Rows` and
/// `Relation` read through, made again by `the_blocks_reads_are_these`.
pub fn relation_rust_source() -> Result<String, String> {
    let funcs = relation_funcs();
    let entries: Vec<(u32, &str)> = funcs.iter().map(|(k, _, read)| (*k, read.name)).collect();
    let funcs: Vec<(u32, Function, bool)> = funcs.into_iter().map(|(k, f, read)| (k, f, read.hot)).collect();
    let body = crate::rust::translate_reads(&funcs, &entries, Family::Relation)?;
    let mut out = String::from(
        "// Generated by rerail_clif::reads::relation_rust_source from a relation's reads in CLIF (decision 0325). Do not\n\
         // edit: `RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_blocks_reads_are_these` writes it again.\n\
         #![allow(dead_code, unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_parens, clippy::all)]\n\n",
    );
    out.push_str(&body);
    let h = crate::rust::fingerprint(&out);
    out.push_str(&format!("\n/// This source's fingerprint (`rust::fingerprint` of what comes before).\npub const SOURCE: u64 = {h:#x};\n"));
    Ok(out)
}

/// A relation's reads as MSL (decision 0325): `rerail_blocks`' `relation_reads.metal`, which every kernel that
/// reads a relation in place includes, made again by `the_blocks_kernels_reads_are_these`.
pub fn relation_msl_source() -> Result<String, String> {
    let funcs: Vec<(u32, Function, &str)> = relation_funcs().into_iter().map(|(k, f, read)| (k, f, read.name)).collect();
    let body = crate::msl::translate_reads(&funcs, "clif_", Family::Relation)?;
    Ok(format!(
        "// Generated by rerail_clif::reads::relation_msl_source from a relation's reads in CLIF (decision 0325). Do not\n\
         // edit: `RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_blocks_kernels_reads_are_these` writes it again.\n{body}"
    ))
}

/// A projection's numbering's reads as Rust (decision 0353 step 2): the facts crate's `id_reads.rs`, which its
/// `Ranks` and `granules::Table` read through, made again by `the_facts_id_reads_are_these`.
pub fn ids_rust_source() -> Result<String, String> {
    let funcs = family_funcs(numbering);
    let entries: Vec<(u32, &str)> = funcs.iter().map(|(k, _, read)| (*k, read.name)).collect();
    let funcs: Vec<(u32, Function, bool)> = funcs.into_iter().map(|(k, f, read)| (k, f, read.hot)).collect();
    let body = crate::rust::translate_reads(&funcs, &entries, Family::Relation)?;
    let mut out = String::from(
        "// Generated by rerail_clif::reads::ids_rust_source from a projection's numbering's reads in CLIF (decision 0353
         // step 2). Do not edit: `RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_facts_id_reads_are_these` writes it
         // again.
         #![allow(dead_code, unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_parens, clippy::all)]

",
    );
    out.push_str(&body);
    let h = crate::rust::fingerprint(&out);
    out.push_str(&format!("\n/// This source's fingerprint (`rust::fingerprint` of what comes before).\npub const SOURCE: u64 = {h:#x};\n"));
    Ok(out)
}

/// A projection's numbering's reads as MSL, after its descriptor's declaration (`ids::msl`): `id_reads.metal`, which
/// a kernel that reads node ids in place includes, made again by `the_facts_id_kernels_reads_are_these`.
pub fn ids_msl_source() -> Result<String, String> {
    let funcs: Vec<(u32, Function, &str)> = family_funcs(numbering).into_iter().map(|(k, f, read)| (k, f, read.name)).collect();
    let body = crate::msl::translate_reads(&funcs, "clif_", Family::Relation)?;
    Ok(format!(
        "// Generated by rerail_clif::reads::ids_msl_source from a projection's numbering's reads in CLIF (decision 0353\n\
         // step 2). Do not edit: `RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_facts_id_kernels_reads_are_these`\n\
         // writes it again.\n{}{body}",
        crate::ids::msl()
    ))
}

/// A body being built: the builder, the tape's base and view, the read's own parameters.
pub struct B<'a, 'c> {
    pub(crate) f: FunctionBuilder<'a>,
    /// The first pointer: the tape's base, or a relation's descriptor.
    pub(crate) tape: Value,
    /// The tape's view (the tape's family alone).
    view: Option<Value>,
    args: Vec<Value>,
    call_conv: CallConv,
    callee: &'c mut dyn Callee,
    /// The one format the body reads (`build_for`), or `None` for either.
    format: Option<u32>,
}

/// A load of the tape's bytes.
fn memory() -> MemFlags {
    MemFlags::new().with_notrap().with_readonly()
}

/// A load of the view's fields: aligned, which tells the Rust backend it is no load of the tape (whose loads it
/// checks against the tape's bytes).
pub(crate) fn fields() -> MemFlags {
    memory().with_aligned()
}

impl B<'_, '_> {
    pub(crate) fn arg(&self, k: usize) -> Value {
        self.args[k]
    }
    pub(crate) fn c32(&mut self, value: i64) -> Value {
        self.f.ins().iconst(I32, value)
    }
    pub(crate) fn c64(&mut self, value: i64) -> Value {
        self.f.ins().iconst(I64, value)
    }
    /// The view's u32 at byte `offset`.
    fn view(&mut self, offset: i32) -> Value {
        let view = self.view.expect("a read of the tape's family");
        self.f.ins().load(I32, fields(), view, offset)
    }
    /// The view's `k`-th u32 of the array at byte `offset`.
    fn view_at(&mut self, offset: i32, k: Value) -> Value {
        let k = self.f.ins().uextend(I64, k);
        let k = self.f.ins().ishl_imm(k, 2);
        let p = self.f.ins().iadd(self.view.expect("a read of the tape's family"), k);
        self.f.ins().load(I32, fields(), p, offset)
    }
    /// The tape's address `base + offset` (both u32s).
    fn address(&mut self, base: Value, offset: Value) -> Value {
        let base = self.f.ins().uextend(I64, base);
        let offset = self.f.ins().uextend(I64, offset);
        let p = self.f.ins().iadd(self.tape, base);
        self.f.ins().iadd(p, offset)
    }
    fn u8(&mut self, base: Value, offset: Value) -> Value {
        let p = self.address(base, offset);
        self.f.ins().uload8(I32, memory(), p, 0)
    }
    fn u16(&mut self, base: Value, offset: Value) -> Value {
        let p = self.address(base, offset);
        self.f.ins().uload16(I32, memory(), p, 0)
    }
    fn u32(&mut self, base: Value, offset: Value) -> Value {
        let p = self.address(base, offset);
        self.f.ins().load(I32, memory(), p, 0)
    }
    /// Eight bytes, little-endian, at any alignment.
    fn word(&mut self, base: Value, offset: Value) -> Value {
        let p = self.address(base, offset);
        self.f.ins().load(I64, memory(), p, 0)
    }
    /// The low `w` bytes' mask (all ones at 8 or more).
    fn mask(&mut self, w: Value) -> Value {
        let bits = self.f.ins().imul_imm(w, 8);
        let one = self.c64(1);
        let bits = self.f.ins().uextend(I64, bits);
        let shifted = self.f.ins().ishl(one, bits);
        let low = self.f.ins().iadd_imm(shifted, -1);
        let wide = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, w, 8);
        let all = self.c64(-1);
        self.f.ins().select(wide, all, low)
    }
    /// The width at nibble `k` of the view's widths.
    fn width(&mut self, k: u32) -> Value {
        let widths = self.view(at::WIDTHS);
        let shifted = self.f.ins().ushr_imm(widths, i64::from(4 * k));
        self.f.ins().band_imm(shifted, 15)
    }
    fn column(&mut self, k: i32) -> Value {
        self.view(at::COLUMNS + 4 * k)
    }
    /// `sixteen`'s value for a format 16 tape, else `fifteen`'s: the view's format tested, unless the body is
    /// built for one (`build_for`).
    fn by_format(&mut self, ty: ir::Type, sixteen: impl FnOnce(&mut Self) -> Value, fifteen: impl FnOnce(&mut Self) -> Value) -> Value {
        match self.format {
            Some(16) => sixteen(self),
            Some(_) => fifteen(self),
            None => {
                let format = self.view(at::FORMAT);
                let f16 = self.f.ins().icmp_imm(IntCC::Equal, format, 16);
                self.either(ty, f16, sixteen, fifteen)
            }
        }
    }
    /// `then` where `cond`, else `otherwise`: a value of type `ty`.
    pub(crate) fn either(&mut self, ty: ir::Type, cond: Value, then: impl FnOnce(&mut Self) -> Value, otherwise: impl FnOnce(&mut Self) -> Value) -> Value {
        let (yes, no, done) = (self.f.create_block(), self.f.create_block(), self.f.create_block());
        self.f.append_block_param(done, ty);
        self.f.ins().brif(cond, yes, &[], no, &[]);
        self.f.switch_to_block(yes);
        let value = then(self);
        self.f.ins().jump(done, &[value.into()]);
        self.f.switch_to_block(no);
        let value = otherwise(self);
        self.f.ins().jump(done, &[value.into()]);
        self.f.switch_to_block(done);
        self.f.block_params(done)[0]
    }
    /// The read `name` called with `args` after the tape and view.
    pub(crate) fn call(&mut self, name: &str, args: &[Value]) -> Value {
        let read = read(name).expect("a read");
        let signature = signature(read, self.call_conv);
        let callee = self.callee.func_ref(name, &signature, self.f.func);
        let mut all = vec![self.tape];
        all.extend(self.view);
        all.extend_from_slice(args);
        let call = self.f.ins().call(callee, &all);
        self.f.inst_results(call)[0]
    }
    /// Format 16's checkpoint `c`: the start of node `c * K`.
    fn checkpoint(&mut self, c: Value) -> Value {
        let cw = self.width(3);
        let checkpoints = self.column(4);
        let at = self.f.ins().imul(c, cw);
        let word = self.word(checkpoints, at);
        let mask = self.mask(cw);
        self.f.ins().band(word, mask)
    }
    /// Format 15's cell of row `i` of a column of width 2 or 4.
    fn cell(&mut self, column: Value, width: Value, i: Value) -> Value {
        let two = self.f.ins().icmp_imm(IntCC::Equal, width, 2);
        self.either(
            I32,
            two,
            |b| {
                let at = b.f.ins().imul_imm(i, 2);
                b.u16(column, at)
            },
            |b| {
                let at = b.f.ins().imul_imm(i, 4);
                b.u32(column, at)
            },
        )
    }
    /// Format 16's value `i` of array `a` (0 slots, 1 gaps, 2 lengths): one word's load, or (all
    /// ones) its side list's.
    fn get(&mut self, a: u32, i: Value) -> Value {
        let w = self.width(a);
        let array = self.column(1 + a as i32);
        let at = self.f.ins().imul(i, w);
        let word = self.word(array, at);
        let mask = self.mask(w);
        let value = self.f.ins().band(word, mask);
        let escaped = self.f.ins().icmp(IntCC::Equal, value, mask);
        self.either(
            I64,
            escaped,
            |b| {
                let list = b.c32(i64::from(a) + 1);
                b.call("tape_f16_side", &[list, i])
            },
            |_| value,
        )
    }
}

/// A node's value in side list `k` (0 codes' low parts, 1 slots, 2 gaps, 3 lengths), the node listed.
fn f16_side(b: &mut B<'_, '_>) -> Value {
    let (k, i) = (b.arg(0), b.arg(1));
    let side = b.view_at(at::SIDE, k);
    let parts = b.view_at(at::PARTS, k);
    let p = b.f.ins().ushr_imm(i, i64::from(F16_PART.trailing_zeros()));
    let p4 = b.f.ins().imul_imm(p, 4);
    let start = b.u32(parts, p4);
    let nodes = b.view(at::NODES);
    let blocks = b.f.ins().iadd_imm(nodes, i64::from(F16_BLOCK - 1));
    let blocks = b.f.ins().ushr_imm(blocks, i64::from(F16_BLOCK.trailing_zeros()));
    let first = b.f.ins().imul_imm(p, i64::from(F16_SUB));
    let left = b.f.ins().isub(blocks, first);
    let sub = b.c32(i64::from(F16_SUB));
    let nsub = b.f.ins().umin(sub, left);
    let j = b.f.ins().ushr_imm(i, i64::from(F16_BLOCK.trailing_zeros()));
    let j = b.f.ins().band_imm(j, i64::from(F16_SUB - 1));
    // The block's run of offsets: from the end of the one before (0 for the part's first) to its own end.
    let first_block = b.f.ins().icmp_imm(IntCC::Equal, j, 0);
    let before = b.f.ins().iadd_imm(j, -1);
    let zero = b.c32(0);
    let before = b.f.ins().select(first_block, zero, before);
    let before = b.f.ins().imul_imm(before, 2);
    let before = b.f.ins().iadd(start, before);
    let lo = b.u16(side, before);
    let lo = b.f.ins().select(first_block, zero, lo);
    let own = b.f.ins().imul_imm(j, 2);
    let own = b.f.ins().iadd(start, own);
    let hi = b.u16(side, own);
    let offsets = b.f.ins().imul_imm(nsub, 2);
    let offsets = b.f.ins().iadd(start, offsets);
    let low = b.f.ins().band_imm(i, i64::from(F16_BLOCK - 1));
    // The first offset at or past the node's low byte.
    let (head, body, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(head, I32);
    b.f.append_block_param(done, I32);
    b.f.ins().jump(head, &[lo.into(), hi.into()]);
    b.f.switch_to_block(head);
    let (lo, hi) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let more = b.f.ins().icmp(IntCC::UnsignedLessThan, lo, hi);
    b.f.ins().brif(more, body, &[], done, &[lo.into()]);
    b.f.switch_to_block(body);
    let sum = b.f.ins().iadd(lo, hi);
    let middle = b.f.ins().ushr_imm(sum, 1);
    let at = b.f.ins().iadd(offsets, middle);
    let byte = b.u8(side, at);
    let below = b.f.ins().icmp(IntCC::UnsignedLessThan, byte, low);
    let next = b.f.ins().iadd_imm(middle, 1);
    let lo = b.f.ins().select(below, next, lo);
    let hi = b.f.ins().select(below, hi, middle);
    b.f.ins().jump(head, &[lo.into(), hi.into()]);
    b.f.switch_to_block(done);
    let found = b.f.block_params(done)[0];
    // The list's values follow the part's offsets.
    let shift = b.f.ins().imul_imm(k, 4);
    let shift = b.f.ins().iadd_imm(shift, 16);
    let widths = b.view(at::WIDTHS);
    let vw = b.f.ins().ushr(widths, shift);
    let vw = b.f.ins().band_imm(vw, 15);
    let last = b.f.ins().iadd_imm(nsub, -1);
    let last = b.f.ins().imul_imm(last, 2);
    let last = b.f.ins().iadd(start, last);
    let count = b.u16(side, last);
    let values = b.f.ins().iadd(offsets, count);
    let at = b.f.ins().imul(found, vw);
    let at = b.f.ins().iadd(values, at);
    let word = b.word(side, at);
    let mask = b.mask(vw);
    b.f.ins().band(word, mask)
}

/// The node's subtree's size (itself and its descendants).
fn size(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    b.by_format(
        I32,
        |b| {
            let v = b.get(0, i);
            let leaf = b.f.ins().band_imm(v, 1);
            let leaf = b.f.ins().icmp_imm(IntCC::NotEqual, leaf, 0);
            let inner = b.f.ins().ushr_imm(v, 1);
            let inner = b.f.ins().ireduce(I32, inner);
            let one = b.c32(1);
            b.f.ins().select(leaf, one, inner)
        },
        |b| {
            let column = b.column(1);
            let width = b.width(0);
            let cell = b.cell(column, width, i);
            let one = b.c32(1);
            b.f.ins().umax(cell, one)
        },
    )
}

/// The node's code's number in its table (format 16: its language's).
fn number(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    b.by_format(
        I32,
        |b| {
            let codes = b.column(0);
            let byte = b.u8(codes, i);
            let cut = b.view(at::CUT);
            let below = b.f.ins().icmp(IntCC::UnsignedLessThan, byte, cut);
            b.either(
                I32,
                below,
                |_| byte,
                |b| {
                    let per = b.view(at::PER);
                    let over = b.f.ins().isub(byte, cut);
                    let over = b.f.ins().imul(over, per);
                    let list = b.c32(0);
                    let low = b.call("tape_f16_side", &[list, i]);
                    let low = b.f.ins().ireduce(I32, low);
                    let number = b.f.ins().iadd(cut, over);
                    b.f.ins().iadd(number, low)
                },
            )
        },
        |b| {
            let codes = b.column(0);
            let at = b.f.ins().imul_imm(i, 2);
            b.u16(codes, at)
        },
    )
}

/// The node's start (a document-local byte).
fn start(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    b.by_format(
        I32,
        |b| {
            // Its checkpoint, then the gaps of the nodes after it, to the node's own.
            let c = b.f.ins().ushr_imm(i, i64::from(F16_K.trailing_zeros()));
            let from = b.checkpoint(c);
            let first = b.f.ins().imul_imm(c, i64::from(F16_K));
            let r = b.f.ins().isub(i, first);
            let (gaps, folded, each, head, body, done) =
                (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
            b.f.append_block_param(head, I32);
            b.f.append_block_param(head, I64);
            b.f.append_block_param(done, I64);
            b.f.ins().brif(r, gaps, &[], done, &[from.into()]);
            // At a byte's width, the gaps of nodes `first + 1 ..= i` are bytes 1 to `r` of the word at the
            // checkpoint's node: summed by folding the word, unless one is escaped (all ones: its side list's).
            b.f.switch_to_block(gaps);
            let gw = b.width(1);
            let byte = b.f.ins().icmp_imm(IntCC::Equal, gw, 1);
            b.f.ins().brif(byte, folded, &[], each, &[]);
            b.f.switch_to_block(folded);
            let array = b.column(2);
            let x = b.word(array, first);
            let all = b.c64(-1);
            let keep = b.f.ins().irsub_imm(r, i64::from(F16_K) - 1);
            let shift = b.f.ins().imul_imm(keep, 8);
            let shift = b.f.ins().uextend(I64, shift);
            let m = b.f.ins().ushr(all, shift);
            let m = b.f.ins().band_imm(m, !0xff);
            let x = b.f.ins().band(x, m);
            let nx = b.f.ins().bnot(x);
            let t = b.f.ins().band_imm(nx, 0x7f7f_7f7f_7f7f_7f7f);
            let t = b.f.ins().iadd_imm(t, 0x7f7f_7f7f_7f7f_7f7f);
            let t = b.f.ins().bor(t, nx);
            let t = b.f.ins().bnot(t);
            let t = b.f.ins().band_imm(t, 0x8080_8080_8080_8080_u64 as i64);
            let escaped = b.f.ins().band(t, m);
            let sum = b.f.create_block();
            b.f.ins().brif(escaped, each, &[], sum, &[]);
            b.f.switch_to_block(sum);
            const LO: i64 = 0x00ff_00ff_00ff_00ff;
            const LO16: i64 = 0x0000_ffff_0000_ffff;
            let low = b.f.ins().band_imm(x, LO);
            let high = b.f.ins().ushr_imm(x, 8);
            let high = b.f.ins().band_imm(high, LO);
            let x = b.f.ins().iadd(low, high);
            let low = b.f.ins().band_imm(x, LO16);
            let high = b.f.ins().ushr_imm(x, 16);
            let high = b.f.ins().band_imm(high, LO16);
            let x = b.f.ins().iadd(low, high);
            let low = b.f.ins().band_imm(x, 0xffff_ffff);
            let high = b.f.ins().ushr_imm(x, 32);
            let x = b.f.ins().iadd(low, high);
            let total = b.f.ins().iadd(from, x);
            b.f.ins().jump(done, &[total.into()]);
            // Else (a wider gap, or an escaped one) gap by gap: at most `K - 1` values.
            b.f.switch_to_block(each);
            let next = b.f.ins().iadd_imm(first, 1);
            b.f.ins().jump(head, &[next.into(), from.into()]);
            b.f.switch_to_block(head);
            let (j, sum) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
            let more = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, j, i);
            b.f.ins().brif(more, body, &[], done, &[sum.into()]);
            b.f.switch_to_block(body);
            let gap = b.get(1, j);
            let sum = b.f.ins().iadd(sum, gap);
            let j = b.f.ins().iadd_imm(j, 1);
            b.f.ins().jump(head, &[j.into(), sum.into()]);
            b.f.switch_to_block(done);
            let sum = b.f.block_params(done)[0];
            b.f.ins().ireduce(I32, sum)
        },
        |b| {
            let base = b.column(2);
            let c = b.f.ins().ushr_imm(i, i64::from(F15_STRIDE_SHIFT));
            let c = b.f.ins().imul_imm(c, 4);
            let checkpoint = b.u32(base, c);
            let off = b.column(3);
            let width = b.width(1);
            let cell = b.cell(off, width, i);
            b.f.ins().iadd(checkpoint, cell)
        },
    )
}

/// The node's end (a document-local byte).
fn end(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    b.by_format(
        I32,
        |b| {
            let start = b.call("tape_start", &[i]);
            let length = b.get(2, i);
            let length = b.f.ins().ireduce(I32, length);
            b.f.ins().iadd(start, length)
        },
        |b| {
            // A tail from the node's own start, or from its last descendant's.
            let widths = b.view(at::WIDTHS);
            let own = b.f.ins().band_imm(widths, i64::from(F15_TAIL_FROM_START));
            let own = b.f.ins().icmp_imm(IntCC::NotEqual, own, 0);
            let size = b.call("tape_size", &[i]);
            let last = b.f.ins().iadd(i, size);
            let last = b.f.ins().iadd_imm(last, -1);
            let from = b.f.ins().select(own, i, last);
            let from = b.call("tape_start", &[from]);
            let tail = b.column(4);
            let width = b.width(2);
            let cell = b.cell(tail, width, i);
            b.f.ins().iadd(from, cell)
        },
    )
}

/// The node's parent, or `NONE` at the root. Format 16: the node before is the parent (an inner
/// node), or a leaf naming the previous sibling or, every K-th sibling, the parent. Format 15: the
/// nearest node before whose subtree holds the node, back over at most 64 nodes, else a descent
/// from the root over the siblings' subtrees.
fn parent(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let root = b.f.ins().icmp_imm(IntCC::Equal, i, 0);
    b.either(
        I32,
        root,
        |b| b.c32(NONE),
        |b| {
            b.by_format(I32, |b| parent16(b, i), |b| parent15(b, i))
        },
    )
}

fn parent16(b: &mut B<'_, '_>, i: Value) -> Value {
    let (head, body, sibling, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(done, I32);
    b.f.ins().jump(head, &[i.into()]);
    b.f.switch_to_block(head);
    let j = b.f.block_params(head)[0];
    let none = b.c32(NONE);
    b.f.ins().brif(j, body, &[], done, &[none.into()]);
    b.f.switch_to_block(body);
    let before = b.f.ins().iadd_imm(j, -1);
    let v = b.get(0, before);
    let leaf = b.f.ins().band_imm(v, 1);
    b.f.ins().brif(leaf, sibling, &[], done, &[before.into()]);
    b.f.switch_to_block(sibling);
    let step = b.f.ins().ushr_imm(v, 2);
    let step = b.f.ins().ireduce(I32, step);
    let back = b.f.ins().isub(j, step);
    let to_parent = b.f.ins().band_imm(v, 2);
    let next = b.f.create_block();
    b.f.ins().brif(to_parent, done, &[back.into()], next, &[]);
    b.f.switch_to_block(next);
    b.f.ins().jump(head, &[back.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

fn parent15(b: &mut B<'_, '_>, i: Value) -> Value {
    let done = b.f.create_block();
    b.f.append_block_param(done, I32);
    // Back over at most 64 nodes.
    let (head, body, descend) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(head, I32);
    let zero = b.c32(0);
    b.f.ins().jump(head, &[i.into(), zero.into()]);
    b.f.switch_to_block(head);
    let (j, k) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let some = b.f.ins().icmp_imm(IntCC::NotEqual, j, 0);
    let near = b.f.ins().icmp_imm(IntCC::UnsignedLessThan, k, 64);
    let go = b.f.ins().band(some, near);
    b.f.ins().brif(go, body, &[], descend, &[]);
    b.f.switch_to_block(body);
    let before = b.f.ins().iadd_imm(j, -1);
    let size = b.call("tape_size", &[before]);
    let end = b.f.ins().iadd(before, size);
    let holds = b.f.ins().icmp(IntCC::UnsignedGreaterThan, end, i);
    let next = b.f.create_block();
    b.f.ins().brif(holds, done, &[before.into()], next, &[]);
    b.f.switch_to_block(next);
    let k = b.f.ins().iadd_imm(k, 1);
    b.f.ins().jump(head, &[before.into(), k.into()]);
    // A descent from the root: at each level, the child whose subtree holds the node.
    b.f.switch_to_block(descend);
    let (level, scan, hop, found) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(level, I32);
    b.f.append_block_param(scan, I32);
    b.f.append_block_param(scan, I32);
    let zero = b.c32(0);
    b.f.ins().jump(level, &[zero.into()]);
    b.f.switch_to_block(level);
    let at = b.f.block_params(level)[0];
    let c = b.f.ins().iadd_imm(at, 1);
    b.f.ins().jump(scan, &[at.into(), c.into()]);
    b.f.switch_to_block(scan);
    let (at, c) = (b.f.block_params(scan)[0], b.f.block_params(scan)[1]);
    let size = b.call("tape_size", &[c]);
    let after = b.f.ins().iadd(c, size);
    let past = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, after, i);
    b.f.ins().brif(past, hop, &[], found, &[]);
    b.f.switch_to_block(hop);
    b.f.ins().jump(scan, &[at.into(), after.into()]);
    b.f.switch_to_block(found);
    let is = b.f.ins().icmp(IntCC::Equal, c, i);
    b.f.ins().brif(is, done, &[at.into()], level, &[c.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

/// The node's length in bytes.
fn length(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    b.by_format(
        I32,
        |b| {
            let length = b.get(2, i);
            b.f.ins().ireduce(I32, length)
        },
        |b| {
            let end = b.call("tape_end", &[i]);
            let start = b.call("tape_start", &[i]);
            b.f.ins().isub(end, start)
        },
    )
}

/// The node's start, given the node before's (`prev`; any value for the first node): in a scan, format 16's from
/// the one before, a gap's add, or its checkpoint.
fn start_next(b: &mut B<'_, '_>) -> Value {
    let (i, prev) = (b.arg(0), b.arg(1));
    b.by_format(
        I32,
        |b| {
            let r = b.f.ins().band_imm(i, i64::from(F16_K - 1));
            b.either(
                I32,
                r,
                |b| {
                    let gap = b.get(1, i);
                    let gap = b.f.ins().ireduce(I32, gap);
                    b.f.ins().iadd(prev, gap)
                },
                |b| {
                    let c = b.f.ins().ushr_imm(i, i64::from(F16_K.trailing_zeros()));
                    let start = b.checkpoint(c);
                    b.f.ins().ireduce(I32, start)
                },
            )
        },
        |b| b.call("tape_start", &[i]),
    )
}

/// Format 16's slot of the node: an inner node's subtree size, shifted up by one; a leaf's step back to its previous
/// sibling or (bit 1) its parent, shifted up by two, with bit 0 set.
fn slot(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    b.get(0, i)
}

/// One past the node's subtree.
fn end_of(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let size = b.call("tape_size", &[i]);
    b.f.ins().iadd(i, size)
}

/// The node after the node's subtree while it is before `end` (its parent's subtree's end), else `NONE`.
fn next_in(b: &mut B<'_, '_>) -> Value {
    let (i, end) = (b.arg(0), b.arg(1));
    let after = b.call("tape_end_of", &[i]);
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, after, end);
    let none = b.c32(NONE);
    b.f.ins().select(inside, after, none)
}

/// The node's next sibling, or `NONE`.
fn next(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let parent = b.call("tape_parent", &[i]);
    let root = b.f.ins().icmp_imm(IntCC::Equal, parent, NONE);
    b.either(
        I32,
        root,
        |b| b.c32(NONE),
        |b| {
            let end = b.call("tape_end_of", &[parent]);
            b.call("tape_next_in", &[i, end])
        },
    )
}

/// The node's first child, or `NONE`.
fn first_child(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let size = b.call("tape_size", &[i]);
    let inner = b.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, size, 1);
    let child = b.f.ins().iadd_imm(i, 1);
    let none = b.c32(NONE);
    b.f.ins().select(inner, child, none)
}

/// The node's children, by hops over their subtrees.
fn child_count(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let end = b.call("tape_end_of", &[i]);
    let first = b.f.ins().iadd_imm(i, 1);
    let (head, body, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(head, I32);
    b.f.append_block_param(done, I32);
    let zero = b.c32(0);
    b.f.ins().jump(head, &[first.into(), zero.into()]);
    b.f.switch_to_block(head);
    let (c, n) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let more = b.f.ins().icmp(IntCC::UnsignedLessThan, c, end);
    b.f.ins().brif(more, body, &[], done, &[n.into()]);
    b.f.switch_to_block(body);
    let size = b.call("tape_size", &[c]);
    let c = b.f.ins().iadd(c, size);
    let n = b.f.ins().iadd_imm(n, 1);
    b.f.ins().jump(head, &[c.into(), n.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

/// The node's `k`-th child (from 0), by hops over its elder siblings' subtrees, or `NONE`.
fn child(b: &mut B<'_, '_>) -> Value {
    let (i, k) = (b.arg(0), b.arg(1));
    let end = b.call("tape_end_of", &[i]);
    let first = b.f.ins().iadd_imm(i, 1);
    let (head, body, hop, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(head, I32);
    b.f.append_block_param(done, I32);
    b.f.ins().jump(head, &[first.into(), k.into()]);
    b.f.switch_to_block(head);
    let (c, k) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let more = b.f.ins().icmp(IntCC::UnsignedLessThan, c, end);
    let none = b.c32(NONE);
    b.f.ins().brif(more, body, &[], done, &[none.into()]);
    b.f.switch_to_block(body);
    b.f.ins().brif(k, hop, &[], done, &[c.into()]);
    b.f.switch_to_block(hop);
    let size = b.call("tape_size", &[c]);
    let c = b.f.ins().iadd(c, size);
    let k = b.f.ins().iadd_imm(k, -1);
    b.f.ins().jump(head, &[c.into(), k.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

// ---- lines: format 16's newline index (`rerail_tape_format::newlines`), format 15's line starts ----------------

/// The newline index's u32 field `k` (0 the newlines' count, 1 the low parts' width, 2 and 3 the zero and one
/// samples' counts, 4 the high bits' words).
fn lines_field(b: &mut B<'_, '_>, k: i64) -> Value {
    let lines = b.view(at::LINES);
    let k = b.c32(4 * k);
    b.u32(lines, k)
}

/// Where the newline index's part at `offset` bytes past its first fields lies (tape-relative).
fn lines_part(b: &mut B<'_, '_>, offset: Value) -> Value {
    let lines = b.view(at::LINES);
    let at = b.f.ins().iadd(lines, offset);
    b.f.ins().iadd_imm(at, HEAD_LINES)
}

/// The newline index's first fields' bytes (`newlines::HEAD`).
const HEAD_LINES: i64 = 20;
/// Every 256th zero and one of the high bits is sampled (`newlines::SAMPLE_SHIFT`).
const LINES_SAMPLE_SHIFT: i64 = 8;

/// Newline `i`'s low part (`l` bits).
fn line_low(b: &mut B<'_, '_>, i: Value, l: Value) -> Value {
    let zs = lines_field(b, 2);
    let os = lines_field(b, 3);
    let words = lines_field(b, 4);
    let samples = b.f.ins().iadd(zs, os);
    let before = b.f.ins().iadd(samples, words);
    let before = b.f.ins().imul_imm(before, 8);
    let lows = lines_part(b, before);
    let i64v = b.f.ins().uextend(I64, i);
    let l64 = b.f.ins().uextend(I64, l);
    let bit = b.f.ins().imul(i64v, l64);
    let byte = b.f.ins().ushr_imm(bit, 3);
    let byte = b.f.ins().ireduce(I32, byte);
    let word = b.word(lows, byte);
    let shift = b.f.ins().band_imm(bit, 7);
    let word = b.f.ins().ushr(word, shift);
    let one = b.c64(1);
    let mask = b.f.ins().ishl(one, l64);
    let mask = b.f.ins().iadd_imm(mask, -1);
    b.f.ins().band(word, mask)
}

/// The position of the `r`-th set bit of `x` (one is there): six halvings, each a popcount.
fn select_in_word(b: &mut B<'_, '_>, x: Value, r: Value) -> Value {
    let (mut x, mut r, mut pos) = (x, r, b.c32(0));
    for s in [32i64, 16, 8, 4, 2, 1] {
        let low = b.f.ins().band_imm(x, (1i64 << s) - 1);
        let c = b.f.ins().popcnt(low);
        let c = b.f.ins().ireduce(I32, c);
        let past = b.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, r, c);
        let less = b.f.ins().isub(r, c);
        r = b.f.ins().select(past, less, r);
        let shifted = b.f.ins().ushr_imm(x, s);
        x = b.f.ins().select(past, shifted, x);
        let moved = b.f.ins().iadd_imm(pos, s);
        pos = b.f.ins().select(past, moved, pos);
    }
    pos
}

/// The position in the newline index's high bits of its `j`-th one (`ones` 1) or zero (0): from the sample before
/// it, a word at a time.
fn line_select(b: &mut B<'_, '_>) -> Value {
    let (ones, j) = (b.arg(0), b.arg(1));
    let zs = lines_field(b, 2);
    let os = lines_field(b, 3);
    let zeros_bytes = b.f.ins().imul_imm(zs, 8);
    let zero = b.c32(0);
    let skip = b.f.ins().select(ones, zeros_bytes, zero);
    let k = b.f.ins().ushr_imm(j, LINES_SAMPLE_SHIFT);
    let k = b.f.ins().imul_imm(k, 8);
    let sample = b.f.ins().iadd(skip, k);
    let sample = lines_part(b, sample);
    let w0 = b.u32(sample, zero);
    let four = b.c32(4);
    let before0 = b.u32(sample, four);
    let samples = b.f.ins().iadd(zs, os);
    let samples = b.f.ins().imul_imm(samples, 8);
    let high = lines_part(b, samples);
    let (head, done) = (b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(head, I32);
    b.f.append_block_param(done, I32);
    b.f.ins().jump(head, &[w0.into(), before0.into()]);
    b.f.switch_to_block(head);
    let (w, before) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let at = b.f.ins().imul_imm(w, 8);
    let word = b.word(high, at);
    let flipped = b.f.ins().bnot(word);
    let x = b.f.ins().select(ones, word, flipped);
    let c = b.f.ins().popcnt(x);
    let c = b.f.ins().ireduce(I32, c);
    let through = b.f.ins().iadd(before, c);
    let here = b.f.ins().icmp(IntCC::UnsignedGreaterThan, through, j);
    let (found, next) = (b.f.create_block(), b.f.create_block());
    b.f.ins().brif(here, found, &[], next, &[]);
    b.f.switch_to_block(next);
    let w1 = b.f.ins().iadd_imm(w, 1);
    b.f.ins().jump(head, &[w1.into(), through.into()]);
    b.f.switch_to_block(found);
    let r = b.f.ins().isub(j, before);
    let bit = select_in_word(b, x, r);
    let base = b.f.ins().imul_imm(w, 64);
    let pos = b.f.ins().iadd(base, bit);
    b.f.ins().jump(done, &[pos.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

/// The line byte `x` lies in (from 0): format 16's newlines before it, by the index (the zeros before and after its
/// bucket, then a search of the bucket's low parts); format 15's line starts searched.
fn line(b: &mut B<'_, '_>) -> Value {
    let x = b.arg(0);
    b.by_format(
        I32,
        |b| {
            let m = lines_field(b, 0);
            let none = b.f.ins().icmp_imm(IntCC::Equal, m, 0);
            b.either(
                I32,
                none,
                |b| b.c32(0),
                |b| {
                    let l = lines_field(b, 1);
                    let h = b.f.ins().ushr(x, l);
                    let one = b.c32(1);
                    let lbits = b.f.ins().ishl(one, l);
                    let lmask = b.f.ins().iadd_imm(lbits, -1);
                    let xl = b.f.ins().band(x, lmask);
                    let xl = b.f.ins().uextend(I64, xl);
                    let zero = b.c32(0);
                    // The newlines in buckets before `h`, and through it.
                    let first = b.f.ins().icmp_imm(IntCC::Equal, h, 0);
                    let lo = b.either(
                        I32,
                        first,
                        |b| b.c32(0),
                        |b| {
                            let h1 = b.f.ins().iadd_imm(h, -1);
                            let p = b.call("tape_line_select", &[zero, h1]);
                            b.f.ins().isub(p, h1)
                        },
                    );
                    let p = b.call("tape_line_select", &[zero, h]);
                    let hi = b.f.ins().isub(p, h);
                    // The bucket's first newline whose low part is at or past `x`'s.
                    let (head, body, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
                    b.f.append_block_param(head, I32);
                    b.f.append_block_param(head, I32);
                    b.f.append_block_param(done, I32);
                    b.f.ins().jump(head, &[lo.into(), hi.into()]);
                    b.f.switch_to_block(head);
                    let (lo, hi) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
                    let more = b.f.ins().icmp(IntCC::UnsignedLessThan, lo, hi);
                    b.f.ins().brif(more, body, &[], done, &[lo.into()]);
                    b.f.switch_to_block(body);
                    let sum = b.f.ins().iadd(lo, hi);
                    let middle = b.f.ins().ushr_imm(sum, 1);
                    let low = line_low(b, middle, l);
                    let below = b.f.ins().icmp(IntCC::UnsignedLessThan, low, xl);
                    let next = b.f.ins().iadd_imm(middle, 1);
                    let lo = b.f.ins().select(below, next, lo);
                    let hi = b.f.ins().select(below, hi, middle);
                    b.f.ins().jump(head, &[lo.into(), hi.into()]);
                    b.f.switch_to_block(done);
                    b.f.block_params(done)[0]
                },
            )
        },
        |b| {
            // The last line start at or before `x`.
            let lines = b.view(at::LINES);
            let count = b.view(at::LINE_COUNT);
            let (head, body, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
            b.f.append_block_param(head, I32);
            b.f.append_block_param(head, I32);
            b.f.append_block_param(done, I32);
            let one = b.c32(1);
            b.f.ins().jump(head, &[one.into(), count.into()]);
            b.f.switch_to_block(head);
            let (lo, hi) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
            let more = b.f.ins().icmp(IntCC::UnsignedLessThan, lo, hi);
            b.f.ins().brif(more, body, &[], done, &[lo.into()]);
            b.f.switch_to_block(body);
            let sum = b.f.ins().iadd(lo, hi);
            let middle = b.f.ins().ushr_imm(sum, 1);
            let at = b.f.ins().imul_imm(middle, 4);
            let start = b.u32(lines, at);
            let past = b.f.ins().icmp(IntCC::UnsignedGreaterThan, start, x);
            let next = b.f.ins().iadd_imm(middle, 1);
            let lo = b.f.ins().select(past, lo, next);
            let hi = b.f.ins().select(past, middle, hi);
            b.f.ins().jump(head, &[lo.into(), hi.into()]);
            b.f.switch_to_block(done);
            let found = b.f.block_params(done)[0];
            b.f.ins().iadd_imm(found, -1)
        },
    )
}

/// Line `k`'s first byte: 0 for the first; else one past newline `k - 1` (format 16: its bucket, from its one's
/// position, and its low part); format 15's line start.
fn line_start(b: &mut B<'_, '_>) -> Value {
    let k = b.arg(0);
    b.by_format(
        I32,
        |b| {
            let first = b.f.ins().icmp_imm(IntCC::Equal, k, 0);
            b.either(
                I32,
                first,
                |b| b.c32(0),
                |b| {
                    let i = b.f.ins().iadd_imm(k, -1);
                    let one = b.c32(1);
                    let p = b.call("tape_line_select", &[one, i]);
                    let bucket = b.f.ins().isub(p, i);
                    let l = lines_field(b, 1);
                    let high = b.f.ins().ishl(bucket, l);
                    let low = line_low(b, i, l);
                    let low = b.f.ins().ireduce(I32, low);
                    let at = b.f.ins().bor(high, low);
                    b.f.ins().iadd_imm(at, 1)
                },
            )
        },
        |b| {
            let lines = b.view(at::LINES);
            let at = b.f.ins().imul_imm(k, 4);
            b.u32(lines, at)
        },
    )
}

#[cfg(test)]
mod tests {
    /// The facts crate's numbering reads are these, as Rust: written again with `RERAIL_WRITE_READS=1`.
    #[test]
    fn the_facts_id_reads_are_these() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/code_rule_dsl/facts/vtab/syntax/id_reads.rs");
        let source = super::ids_rust_source().unwrap();
        if std::env::var_os("RERAIL_WRITE_READS").is_some() {
            std::fs::write(path, &source).unwrap();
        }
        let held = std::fs::read_to_string(path).unwrap_or_default();
        assert!(held == source, "{path} is not the reads' Rust: RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_facts_id_reads_are_these");
    }

    /// The numbering's kernels' reads are these, as MSL: written again with `RERAIL_WRITE_READS=1`.
    #[test]
    fn the_facts_id_kernels_reads_are_these() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/code_rule_dsl/facts/vtab/syntax/id_reads.metal");
        let source = super::ids_msl_source().unwrap();
        if std::env::var_os("RERAIL_WRITE_READS").is_some() {
            std::fs::write(path, &source).unwrap();
        }
        let held = std::fs::read_to_string(path).unwrap_or_default();
        assert!(held == source, "{path} is not the reads' MSL: RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_facts_id_kernels_reads_are_these");
    }

    /// The blocks crate's reads are these, as Rust: written again with `RERAIL_WRITE_READS=1`.
    #[test]
    fn the_blocks_reads_are_these() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../blocks/src/generated_reads.rs");
        let source = super::relation_rust_source().unwrap();
        if std::env::var_os("RERAIL_WRITE_READS").is_some() {
            std::fs::write(path, &source).unwrap();
        }
        let held = std::fs::read_to_string(path).unwrap_or_default();
        assert!(held == source, "{path} is not the reads' Rust: RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_blocks_reads_are_these");
    }

    /// The blocks crate's kernels' reads are these, as MSL: written again with `RERAIL_WRITE_READS=1`.
    #[test]
    fn the_blocks_kernels_reads_are_these() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../blocks/src/relation_reads.metal");
        let source = super::relation_msl_source().unwrap();
        if std::env::var_os("RERAIL_WRITE_READS").is_some() {
            std::fs::write(path, &source).unwrap();
        }
        let held = std::fs::read_to_string(path).unwrap_or_default();
        assert!(held == source, "{path} is not the reads' MSL: RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_blocks_kernels_reads_are_these");
    }

    /// The kernels' reads are these, as MSL: written again with `RERAIL_WRITE_READS=1`.
    #[test]
    fn the_kernels_reads_are_these() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/code_rule_dsl/transducer/packed/metal_tape_reads.metal");
        let source = super::msl_source().unwrap();
        if std::env::var_os("RERAIL_WRITE_READS").is_some() {
            std::fs::write(path, &source).unwrap();
        }
        let held = std::fs::read_to_string(path).unwrap_or_default();
        assert!(held == source, "{path} is not the reads' MSL: RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_kernels_reads_are_these");
    }

    /// The tape crate's reads are these, as Rust: written again with `RERAIL_WRITE_READS=1`.
    #[test]
    fn the_tape_crates_reads_are_these() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../tape_format/src/generated_reads.rs");
        let source = super::rust_source().unwrap();
        if std::env::var_os("RERAIL_WRITE_READS").is_some() {
            std::fs::write(path, &source).unwrap();
        }
        let held = std::fs::read_to_string(path).unwrap_or_default();
        assert!(held == source, "{path} is not the reads' Rust: RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_tape_crates_reads_are_these");
    }
}
