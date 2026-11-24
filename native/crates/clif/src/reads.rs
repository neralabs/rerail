//! The tape's primitive reads as CLIF functions, written once (decision 0310): the reads
//! `rerail_tape_format::f16::View` and format 15's columns make, as `device_tape.h` ports them.
//!
//! A read takes the tape's base (the bytes its view's offsets count from), its view
//! (`view::TapeView`) and a document-local node, and returns a u32 (`NONE` for no node). Format 16's
//! value too wide for its array is all ones and lies in its side list: the node's part, its
//! 256-node block's offsets, a search for its low byte (`f16_side`, cold). A read built over another
//! calls it by name; the JIT's pass inlines the hot ones (`inline`).
//!
//! Every load here reads the tape or its view, which no code writes while it is read (`readonly`).
//! Format 16's arrays have 8 bytes of room past their last value, so a value is one word's load.

use crate::view::cache::{NOT_KEPT, PENDING, TAKEN};
use crate::view::{F15_STRIDE_SHIFT, F15_TAIL_FROM_START, F16_BLOCK, F16_K, F16_PART, F16_SUB, F16Z_BLOCK, F16Z_FORMAT, F16Z_PIECES, at, z};
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
    /// A view's (`view_*`, decision 0353 step 4): a document's entry (`view_reads::ViewDoc`, in its batch's block,
    /// a load of it aligned as a relation's), the tape's base and its view, its lines' base. A view read calls the
    /// numbering's reads with the descriptor its entry refers to, the tape's with the tape's base and view, the
    /// lines' with their base.
    View,
    /// A document's lines' (`lines_*`, decision 0353 step 4d): the base of its `LineTable`'s words (`[n, starts[n],
    /// ends[n]]`, u32s, in a slab's room beside its tape), every load aligned, an offset from it.
    Lines,
}

impl Family {
    /// The pointers a read of this family takes before its own parameters.
    pub fn pointers(self) -> usize {
        match self {
            Family::Tape => 2,
            Family::Relation | Family::Lines => 1,
            Family::View => 4,
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
    Read { family: Family::Tape, name: "tape_start_stated", params: &[I32], ret: I32, hot: true, body: start_stated },
    Read { family: Family::Tape, name: "tape_end_stated", params: &[I32], ret: I32, hot: true, body: end_stated },
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
    Read { family: Family::Tape, name: "tape_has_kind", params: &[I32, I32], ret: I32, hot: true, body: has_kind },
    Read { family: Family::Relation, name: "rows_col", params: &[I32], ret: I64, hot: true, body: crate::relation::rows_col },
    Read { family: Family::Relation, name: "col_at", params: &[I32, I32], ret: I64, hot: true, body: crate::relation::col_at },
    Read { family: Family::Relation, name: "col_get", params: &[I32, I32], ret: I64, hot: true, body: crate::relation::col_get },
    Read { family: Family::Relation, name: "rows_at", params: &[I32, I32], ret: I64, hot: true, body: crate::relation::rows_at },
    Read { family: Family::Relation, name: "rows_width", params: &[I32], ret: I32, hot: true, body: crate::relation::rows_width },
    Read { family: Family::Relation, name: "rows_get", params: &[I32, I32], ret: I64, hot: true, body: crate::relation::rows_get },
    Read { family: Family::Relation, name: "rows_len", params: &[], ret: I32, hot: true, body: crate::relation::rows_len },
    Read { family: Family::Relation, name: "rows_put", params: &[I32, I32, I64], ret: I32, hot: true, body: crate::relation::rows_put },
    Read { family: Family::Relation, name: "rel_get", params: &[I32, I32], ret: I64, hot: true, body: crate::relation::rel_get },
    Read { family: Family::Relation, name: "rel_dead", params: &[I32], ret: I32, hot: true, body: crate::relation::rel_dead },
    Read { family: Family::Relation, name: "rel_len", params: &[], ret: I32, hot: true, body: crate::relation::rel_len },
    Read { family: Family::Relation, name: "rel_group", params: &[I32, I64], ret: I64, hot: true, body: crate::relation::rel_group },
    Read { family: Family::Relation, name: "rel_group_row", params: &[I32, I32], ret: I32, hot: true, body: crate::relation::rel_group_row },
    Read { family: Family::Relation, name: "rel_key_mix", params: &[I64, I64], ret: I64, hot: true, body: crate::relation::rel_key_mix },
    // A blocked Bloom filter's (decision 0488): the descriptor is a `bloom::Bloom`.
    Read { family: Family::Relation, name: "bloom_hash", params: &[I64], ret: I64, hot: true, body: crate::relation::bloom_hash },
    Read { family: Family::Relation, name: "bloom_mask", params: &[I64], ret: I64, hot: true, body: crate::relation::bloom_mask },
    Read { family: Family::Relation, name: "bloom_may", params: &[I64], ret: I32, hot: true, body: crate::relation::bloom_may },
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
    // A view's rows (decision 0353 step 4c): row `l` of the document's rows, column `col`.
    Read { family: Family::Lines, name: "lines_line", params: &[I32], ret: I32, hot: true, body: crate::view_reads::lines_line },
    Read { family: Family::Lines, name: "lines_word", params: &[I32], ret: I32, hot: true, body: crate::view_reads::lines_word },
    Read { family: Family::View, name: "view_node", params: &[I32, I32], ret: I64, hot: true, body: crate::view_reads::node },
    Read { family: Family::View, name: "view_child", params: &[I32, I32], ret: I64, hot: true, body: crate::view_reads::child },
    Read { family: Family::View, name: "view_field", params: &[I32, I32], ret: I64, hot: true, body: crate::view_reads::field },
    Read { family: Family::View, name: "view_core", params: &[I32, I32], ret: I64, hot: true, body: crate::view_reads::core },
    Read { family: Family::View, name: "view_line", params: &[I32, I32], ret: I64, hot: true, body: crate::view_reads::line },
    Read { family: Family::View, name: "view_loc_line", params: &[I32, I32], ret: I64, hot: true, body: crate::view_reads::loc_line },
    Read { family: Family::View, name: "view_loc_end", params: &[I32, I32], ret: I64, hot: true, body: crate::view_reads::loc_end },
    Read { family: Family::View, name: "view_loc_col", params: &[I32, I32], ret: I64, hot: true, body: crate::view_reads::loc_col },
    Read { family: Family::View, name: "view_loc_docof", params: &[I32, I32], ret: I64, hot: true, body: crate::view_reads::loc_docof },
    // A 16z tape's block in the reader's slot (decision 0353 step 4b): last, so no other read's number moves.
    Read { family: Family::Tape, name: "tape_z_block", params: &[I32], ret: I64, hot: false, body: z_block },
    // LZ4's block decoder alone (`z_lz4`, which `tape_z_block` decodes with): the tape crate's `Tape16Z` decodes with
    // it, so every reader of a 16z tape has the one decoder (decision 0353 step 4b).
    Read { family: Family::Tape, name: "tape_lz4", params: &[I32, I32, I64, I64], ret: I64, hot: false, body: lz4_block },
    // `child` probed by its parent, off the tape (decision 0353 step 4c): no index of the view.
    Read { family: Family::View, name: "view_child_first", params: &[I32], ret: I32, hot: false, body: crate::view_reads::child_first },
    Read { family: Family::View, name: "view_child_next", params: &[I32], ret: I32, hot: false, body: crate::view_reads::child_next },
    Read { family: Family::View, name: "view_number", params: &[I32], ret: I32, hot: false, body: crate::view_reads::number },
    // An atom's string in a directory of runs of atoms (decision 0478 step 2b): the descriptor is an `AtomsDirRoot`.
    Read { family: Family::Relation, name: "atoms_len", params: &[I32], ret: I32, hot: true, body: crate::atoms_reads::atoms_len },
    Read { family: Family::Relation, name: "atoms_at", params: &[I32], ret: I64, hot: true, body: crate::atoms_reads::atoms_at },
    // A text test's verdict for a symbol in the one rolling cache (decision 0478 step 2d): the descriptor is the cache.
    Read { family: Family::Relation, name: "verdict_get", params: &[I32, I32], ret: I32, hot: false, body: crate::verdict_reads::verdict_get },
    Read { family: Family::Relation, name: "verdict_put", params: &[I32, I32, I32], ret: I32, hot: false, body: crate::verdict_reads::verdict_put },
    // A text function's text of a symbol in the same cache (decision 0478 step 2e): found, or taken for its length
    // and written by its caller, then completed.
    Read { family: Family::Relation, name: "text_find", params: &[I32, I32], ret: I64, hot: false, body: crate::verdict_reads::text_find },
    Read { family: Family::Relation, name: "text_open", params: &[I32, I32, I32], ret: I64, hot: false, body: crate::verdict_reads::text_open },
    Read { family: Family::Relation, name: "cache_done", params: &[I64], ret: I32, hot: false, body: crate::verdict_reads::cache_done },
    // The cache's two lookups, each written once and called (decision 0310): the verdicts' (an entry made with its
    // masks empty) and the texts' open one.
    Read { family: Family::Relation, name: "verdict_entry", params: &[I32, I32], ret: I64, hot: false, body: crate::verdict_reads::verdict_entry },
    Read { family: Family::Relation, name: "cache_open", params: &[I64, I32], ret: I64, hot: false, body: crate::verdict_reads::cache_open },
    // `has` probed off the tape (decision 0353 step 5): a node's numbered ancestors by a climb, its numbered
    // descendants as a range of numbers. Last, so no other read's number moves.
    Read { family: Family::View, name: "view_up", params: &[I32], ret: I32, hot: false, body: crate::view_reads::up },
    Read { family: Family::View, name: "view_end", params: &[I32], ret: I32, hot: false, body: crate::view_reads::end },
    // `tape`, `ast_depth` and `node_order` read in place (decision 0353 step 6: they were uploaded from the host).
    // Last, so no other read's number moves.
    Read { family: Family::View, name: "view_tape", params: &[I32, I32], ret: I64, hot: true, body: crate::view_reads::tape },
    Read { family: Family::View, name: "view_depth", params: &[I32, I32], ret: I64, hot: true, body: crate::view_reads::depth },
    Read { family: Family::View, name: "view_order", params: &[I32, I32], ret: I64, hot: true, body: crate::view_reads::order },
    // The views' directory's searches (decision 0353 step 3 on the CPU, step 2): the descriptor is the directory, in
    // its batch's block; each takes its caller's memo.
    Read { family: Family::Relation, name: "dir_doc_of_row", params: &[I32, I32, I32], ret: I32, hot: false, body: crate::view_reads::dir_doc_of_row },
    Read { family: Family::Relation, name: "dir_doc_of_key", params: &[I32, I32], ret: I32, hot: false, body: crate::view_reads::dir_doc_of_key },
    // A string's atom by its hash in a directory of runs of atoms (decision 0666): the descriptor is an
    // `AtomsDirRoot`. Last, so no other read's number moves.
    Read { family: Family::Relation, name: "atoms_find", params: &[I64, I32, I32], ret: I32, hot: false, body: crate::atoms_reads::atoms_find },
];

/// The reads of format 16 alone: a side list's value, and a node's slot (16z's parent steps read it block by
/// block).
pub const F16_ONLY: &[&str] = &["tape_f16_side", "tape_slot", "tape_line_select", "tape_lz4"];

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
    build_formats(read, call_conv, name, callee, None, true)
}

/// The read's body for tapes of `format` alone (15 or 16; `None` either, as its view says): the other's code
/// is not built, so a reader of one format (the tape crate's `F16`, `F15`) tests none.
pub fn build_for(read: &Read, call_conv: CallConv, name: UserFuncName, callee: &mut dyn Callee, format: Option<u32>) -> Function {
    build_formats(read, call_conv, name, callee, format, false)
}

/// `build_for`, and with `z` a 16z tape too (decision 0353 step 4b) where the body reads either format: its block
/// decoded into the reader's slot (`tape_z_block`) and read as format 16. The JIT's reads read 16z; the tape crate's
/// Rust reads a block at a time itself (`F16Z`). The kernels' reads of the same are written by hand (decision 0524):
/// `<ukl/reads/tape_f16z.h>` (decision 0576).
pub fn build_formats(read: &Read, call_conv: CallConv, name: UserFuncName, callee: &mut dyn Callee, format: Option<u32>, z: bool) -> Function {
    let mut func = Function::with_name_signature(name, signature(read, call_conv));
    let mut context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(&mut func, &mut context);
    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    let params = f.block_params(entry).to_vec();
    let pointers = read.family.pointers();
    let (view, doc_tape, lines) = match read.family {
        Family::Tape => (Some(params[1]), None, None),
        Family::Relation | Family::Lines => (None, None, None),
        Family::View => (None, Some((params[1], params[2])), Some(params[3])),
    };
    let mut b = B { f, tape: params[0], view, doc_tape, lines, args: params[pointers..].to_vec(), call_conv, callee, format, z, slot: None };
    let value = (read.body)(&mut b);
    b.f.ins().return_(&[value]);
    b.f.seal_all_blocks();
    b.f.finalize();
    func
}

/// Where the reads' UKL is held: the library's `ukl/reads/`, which every kernel includes them from.
#[cfg(test)]
fn ukl_reads() -> String {
    format!("{}/../ukl/lib/ukl/reads", std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
}

/// A generated header's first lines: what made it, and how to make it again.
fn generated(function: &str, decision: &str, test: &str) -> String {
    format!(
        "// Generated by native/crates/clif/src/reads.rs; do not edit. Made by rerail_clif::reads::{function} from {decision}:\n\
         // `RERAIL_WRITE_READS=1 cargo test -p rerail-clif {test}` writes it again.\n"
    )
}

/// The views' reads as UKL (decision 0353 step 4c), after their entry's declaration (`view_reads::ukl`):
/// `<ukl/reads/view.h>`, over the tape's reads that read 16z too and the numbering's (whose functions they call), and
/// the body it includes once per tape format (`<ukl/body/view_reads.h>`, decision 0576), made again by
/// `the_view_kernels_reads_are_these`.
pub fn view_ukl_source() -> Result<(String, String), String> {
    let funcs: Vec<(u32, Function, &str)> = READS
        .iter()
        .enumerate()
        .filter(|(_, read)| matches!(read.family, Family::View | Family::Lines))
        .map(|(k, read)| (k as u32, build_for(read, CallConv::SystemV, UserFuncName::user(0, k as u32), &mut ByIndex, None)))
        .map(|(k, f)| (k, f, READS[k as usize].name))
        .collect();
    let (header, body) = crate::ukl::translate_view_reads(&funcs, "clif_")?;
    let (head, rest) = header.split_at(crate::ukl::HELPERS.len());
    let made = generated("view_ukl_source", "the views' reads in CLIF (decision 0353 step 4c, 0576)", "the_view_kernels_reads_are_these");
    Ok((format!("{made}{head}#include <ukl/reads/tape_z.h>\n#include <ukl/reads/ids.h>\n{}{rest}", crate::view_reads::ukl()), format!("{made}{body}")))
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

/// A relation's reads as UKL (decision 0325): `<ukl/reads/relation.h>`, which every kernel that reads a relation in
/// place includes, made again by `the_blocks_kernels_reads_are_these`.
pub fn relation_ukl_source() -> Result<String, String> {
    let funcs: Vec<(u32, Function, &str)> = relation_funcs().into_iter().map(|(k, f, read)| (k, f, read.name)).collect();
    let body = crate::ukl::translate_reads(&funcs, "clif_", Family::Relation)?;
    Ok(format!("{}{body}", generated("relation_ukl_source", "a relation's reads in CLIF (decision 0325)", "the_blocks_kernels_reads_are_these")))
}

/// A projection's numbering's reads as UKL, after its descriptor's declaration (`ids::ukl`): `<ukl/reads/ids.h>`,
/// which a kernel that reads node ids in place includes, made again by `the_facts_id_kernels_reads_are_these`.
pub fn ids_ukl_source() -> Result<String, String> {
    let funcs: Vec<(u32, Function, &str)> = family_funcs(numbering).into_iter().map(|(k, f, read)| (k, f, read.name)).collect();
    let body = crate::ukl::translate_reads(&funcs, "clif_", Family::Relation)?;
    let (head, rest) = body.split_at(crate::ukl::HELPERS.len());
    Ok(format!(
        "{}{head}{}{rest}",
        generated("ids_ukl_source", "a projection's numbering's reads in CLIF (decision 0353 step 2)", "the_facts_id_kernels_reads_are_these"),
        crate::ids::ukl()
    ))
}

/// A body being built: the builder, the tape's base and view, the read's own parameters.
pub struct B<'a, 'c> {
    pub(crate) f: FunctionBuilder<'a>,
    /// The first pointer: the tape's base, or a relation's descriptor.
    pub(crate) tape: Value,
    /// The tape's view (the tape's family alone).
    view: Option<Value>,
    /// A view read's tape: its base and its view (the view's family alone).
    doc_tape: Option<(Value, Value)>,
    /// A view read's lines' base (the view's family alone).
    lines: Option<Value>,
    args: Vec<Value>,
    call_conv: CallConv,
    callee: &'c mut dyn Callee,
    /// The one format the body reads (`build_for`), or `None` for either.
    format: Option<u32>,
    /// Whether a 16z tape is read too (`build_formats`).
    z: bool,
    /// The 16z block being read (`in_block`): its cache entry is the tape, which the cache's writers write.
    slot: Option<Slot>,
}

/// A 16z block read in its cache entry (`B::in_block`): the reader's tape and view, the block's first node, and the
/// entry's mask (its size less one), which every load of it is masked by.
#[derive(Clone, Copy)]
struct Slot {
    tape: Value,
    view: Value,
    base: Value,
    mask: Value,
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
    /// A load of the tape's bytes: or of a 16z block's cache entry, which the cache's writers write.
    fn mem(&self) -> MemFlags {
        if self.slot.is_some() { MemFlags::new().with_notrap() } else { memory() }
    }
    /// A load of the view's fields: or of a 16z block's view in its entry, a load of the tape's (the entry is
    /// reached from the tape's base, not the view's).
    fn fields(&self) -> MemFlags {
        if self.slot.is_some() { MemFlags::new().with_notrap() } else { fields() }
    }
    /// The view's u32 at byte `offset`.
    fn view(&mut self, offset: i32) -> Value {
        let view = self.view.expect("a read of the tape's family");
        let flags = self.fields();
        self.f.ins().load(I32, flags, view, offset)
    }
    /// The view's `k`-th u32 of the array at byte `offset`.
    fn view_at(&mut self, offset: i32, k: Value) -> Value {
        let k = self.f.ins().uextend(I64, k);
        let k = self.f.ins().ishl_imm(k, 2);
        let p = self.f.ins().iadd(self.view.expect("a read of the tape's family"), k);
        let flags = self.fields();
        self.f.ins().load(I32, flags, p, offset)
    }
    /// The tape's address `base + offset` (both u32s): in a 16z block's entry, masked to the entry, so a read of
    /// one being written again reads nothing outside the cache.
    fn address(&mut self, base: Value, offset: Value) -> Value {
        let base = self.f.ins().uextend(I64, base);
        let offset = self.f.ins().uextend(I64, offset);
        if let Some(slot) = self.slot {
            let at = self.f.ins().iadd(base, offset);
            let at = self.f.ins().band(at, slot.mask);
            return self.f.ins().iadd(self.tape, at);
        }
        let p = self.f.ins().iadd(self.tape, base);
        self.f.ins().iadd(p, offset)
    }
    fn u8(&mut self, base: Value, offset: Value) -> Value {
        let p = self.address(base, offset);
        let flags = self.mem();
        self.f.ins().uload8(I32, flags, p, 0)
    }
    fn u16(&mut self, base: Value, offset: Value) -> Value {
        let p = self.address(base, offset);
        let flags = self.mem();
        self.f.ins().uload16(I32, flags, p, 0)
    }
    fn u32(&mut self, base: Value, offset: Value) -> Value {
        let p = self.address(base, offset);
        let flags = self.mem();
        self.f.ins().load(I32, flags, p, 0)
    }
    /// Eight bytes, little-endian, at any alignment.
    fn word(&mut self, base: Value, offset: Value) -> Value {
        let p = self.address(base, offset);
        let flags = self.mem();
        self.f.ins().load(I64, flags, p, 0)
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
    /// `sixteen` takes the node (its read's first argument): of a 16z tape, its place in its block, the block
    /// decoded into the reader's slot and read as format 16 (`in_block`).
    fn by_format(&mut self, ty: ir::Type, sixteen: impl Fn(&mut Self, Value) -> Value, fifteen: impl FnOnce(&mut Self) -> Value) -> Value {
        let sixteen = &sixteen;
        self.by_formats(
            ty,
            |b| {
                let i = b.arg(0);
                sixteen(b, i)
            },
            |b| b.in_block(0, sixteen),
            fifteen,
        )
    }
    /// `by_format` for a read of the tape's whole, not of a node (its newline index): a 16z tape holds it raw, where
    /// its view names it, so a 16z tape's body is format 16's, read in place of the stored bytes (decision 0434).
    fn by_format_whole(&mut self, ty: ir::Type, sixteen: impl Fn(&mut Self) -> Value, fifteen: impl FnOnce(&mut Self) -> Value) -> Value {
        let sixteen = &sixteen;
        self.by_formats(ty, |b| sixteen(b), |b| sixteen(b), fifteen)
    }
    /// `by_format` with a 16z tape's body of its own (`zed`).
    fn by_formats(&mut self, ty: ir::Type, sixteen: impl FnOnce(&mut Self) -> Value, zed: impl FnOnce(&mut Self) -> Value, fifteen: impl FnOnce(&mut Self) -> Value) -> Value {
        match self.format {
            Some(16) => sixteen(self),
            Some(F16Z_FORMAT) => zed(self),
            Some(_) => fifteen(self),
            None => {
                let format = self.view(at::FORMAT);
                let f16 = self.f.ins().icmp_imm(IntCC::Equal, format, 16);
                if !self.z {
                    return self.either(ty, f16, sixteen, fifteen);
                }
                self.either(ty, f16, sixteen, |b| {
                    let z = b.f.ins().icmp_imm(IntCC::Equal, format, i64::from(F16Z_FORMAT));
                    b.either(ty, z, zed, fifteen)
                })
            }
        }
    }
    /// A format 16 read's `body` of its argument `k`'s node, of a tape of format 16 or, read too (`z`), 16z.
    fn sixteen_or_z(&mut self, ty: ir::Type, k: usize, body: impl Fn(&mut Self, Value) -> Value) -> Value {
        if self.format == Some(F16Z_FORMAT) {
            return self.in_block(k, body);
        }
        if self.format.is_some() || !self.z {
            let i = self.arg(k);
            return body(self, i);
        }
        let format = self.view(at::FORMAT);
        let z = self.f.ins().icmp_imm(IntCC::Equal, format, i64::from(F16Z_FORMAT));
        let body = &body;
        self.either(ty, z, |b| b.in_block(k, body), |b| {
            let i = b.arg(k);
            body(b, i)
        })
    }
    /// A 16z tape's `body` of its argument `k`'s node: its block in the cache (`tape_z_block`), read as format 16
    /// with the node's place in it. Where threads share the cache (the view's `columns[4]`), the entry's completion
    /// is read again after the body, and the body read again if another thread wrote the entry meanwhile.
    fn in_block(&mut self, k: usize, body: impl Fn(&mut Self, Value) -> Value) -> Value {
        use crate::view::cache as c;
        let i = self.arg(k);
        let block = self.f.ins().ushr_imm(i, i64::from(F16Z_BLOCK.trailing_zeros()));
        let first = self.f.ins().ishl_imm(block, i64::from(F16Z_BLOCK.trailing_zeros()));
        let local = self.f.ins().band_imm(i, i64::from(F16Z_BLOCK - 1));
        let size = self.column(3);
        let size = self.f.ins().uextend(I64, size);
        let mask = self.f.ins().iadd_imm(size, -1);
        let shared = self.column(4);
        let (head, check, read, again, done) = (self.f.create_block(), self.f.create_block(), self.f.create_block(), self.f.create_block(), self.f.create_block());
        self.f.append_block_param(head, I32);
        // `read` reads an entry: the one found, or the dead entry; and whether to check it again after the body.
        self.f.append_block_param(read, I64);
        self.f.append_block_param(read, I32);
        let zero = self.c32(0);
        self.f.ins().jump(head, &[zero.into()]);
        self.f.switch_to_block(head);
        let tries = self.f.block_params(head)[0];
        let entry = self.call("tape_z_block", &[block]);
        let ready = self.entry_word(entry, c::READY);
        let before = self.f.ins().atomic_load(I32, MemFlags::trusted(), ready);
        let no = self.c32(0);
        self.f.ins().brif(shared, check, &[], read, &[entry.into(), no.into()]);
        // Shared: the entry complete, of this block (another thread may have taken it since it was found).
        self.f.switch_to_block(check);
        self.f.ins().fence();
        let lo = self.view(at::SIDE + 8);
        let hi = self.view(at::SIDE + 12);
        let id = wide(self, lo, hi);
        let id = self.f.ins().ishl_imm(id, 22);
        let block64 = self.f.ins().uextend(I64, block);
        let key = self.f.ins().bor(id, block64);
        let held = self.f.ins().load(I64, MemFlags::new().with_notrap(), entry, c::KEY);
        let ours = self.f.ins().icmp(IntCC::Equal, held, key);
        let writing = self.f.ins().band_imm(before, i64::from(c::WRITING));
        let complete = self.f.ins().icmp_imm(IntCC::Equal, writing, 0);
        let some = self.f.ins().icmp_imm(IntCC::NotEqual, before, 0);
        let ok = self.f.ins().band(ours, complete);
        let ok = self.f.ins().band(ok, some);
        // Tries spent: the dead entry is read, a failure counted (another thread holds the block). Never the entry
        // as it is, which may hold two blocks' bytes: a kernel that went on with such a value (an index, a span)
        // would reach anywhere before its run is refused (decision 0463); the dead entry's reads are its zeros.
        let (gave_up, give_up) = (self.f.create_block(), self.f.create_block());
        let yes = self.c32(1);
        self.f.ins().brif(ok, read, &[entry.into(), yes.into()], gave_up, &[]);
        self.f.switch_to_block(gave_up);
        let spent = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, tries, i64::from(c::TRIES));
        self.f.ins().brif(spent, give_up, &[], again, &[]);
        self.f.switch_to_block(give_up);
        self.count_failed();
        let dead = self.dead_entry();
        let no = self.c32(0);
        self.f.ins().jump(read, &[dead.into(), no.into()]);
        self.f.switch_to_block(read);
        let (entry, rechecking) = (self.f.block_params(read)[0], self.f.block_params(read)[1]);
        let tape = self.f.ins().iadd_imm(entry, i64::from(z::SLOT_BYTES));
        let view = self.f.ins().iadd_imm(entry, i64::from(z::SLOT_VIEW));
        let slot = Slot { tape: self.tape, view: self.view.expect("a read of the tape's family"), base: first, mask };
        let held = (std::mem::replace(&mut self.tape, tape), self.view.replace(view), self.format.replace(16), self.slot.replace(slot));
        let value = body(self, local);
        (self.tape, self.view, self.format, self.slot) = (held.0, held.1, held.2, held.3);
        let ty = self.f.func.dfg.value_type(value);
        self.f.append_block_param(done, ty);
        // Read again, a bounded number of times, if a writer took the entry while the body read it.
        let recheck = self.f.create_block();
        self.f.ins().brif(rechecking, recheck, &[], done, &[value.into()]);
        self.f.switch_to_block(recheck);
        self.f.ins().fence();
        let ready = self.entry_word(entry, c::READY);
        let after = self.f.ins().atomic_load(I32, MemFlags::trusted(), ready);
        let same = self.f.ins().icmp(IntCC::Equal, before, after);
        let (changed, kept) = (self.f.create_block(), self.f.create_block());
        self.f.ins().brif(same, done, &[value.into()], changed, &[]);
        self.f.switch_to_block(changed);
        let spent = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, tries, i64::from(c::TRIES));
        self.f.ins().brif(spent, kept, &[], again, &[]);
        // Tries spent: the dead entry read instead (above), a failure counted.
        self.f.switch_to_block(kept);
        self.count_failed();
        let dead = self.dead_entry();
        let no = self.c32(0);
        self.f.ins().jump(read, &[dead.into(), no.into()]);
        self.f.switch_to_block(again);
        let tries = self.f.ins().iadd_imm(tries, 1);
        self.f.ins().jump(head, &[tries.into()]);
        self.f.switch_to_block(done);
        self.f.block_params(done)[0]
    }
    /// The dead entry of the cache the view names: what a read with no entry of its own reads (its zeros).
    fn dead_entry(&mut self) -> Value {
        let lo = self.view(at::SIDE);
        let hi = self.view(at::SIDE + 4);
        let rel = wide(self, lo, hi);
        let cache = self.f.ins().iadd(self.tape, rel);
        let dead = cache_load(self, cache, crate::view::cache::DEAD);
        let dead = self.f.ins().uextend(I64, dead);
        self.f.ins().iadd(cache, dead)
    }
    /// One failure more in the cache the view names (`view::cache::FAILED`).
    fn count_failed(&mut self) {
        let lo = self.view(at::SIDE);
        let hi = self.view(at::SIDE + 4);
        let rel = wide(self, lo, hi);
        let cache = self.f.ins().iadd(self.tape, rel);
        let one = self.c32(1);
        cache_add(self, cache, crate::view::cache::FAILED, one);
    }
    /// The address of an entry's (or the cache's) word at `offset`.
    fn entry_word(&mut self, entry: Value, offset: i32) -> Value {
        self.f.ins().iadd_imm(entry, i64::from(offset))
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
        // A read of a 16z block calls the tape's read at the node's place in the tape, with the reader's pointers:
        // the callee finds the block itself.
        if let Some(slot) = self.slot.take() {
            let node = usize::from(name == "tape_f16_side");
            let mut args = args.to_vec();
            args[node] = self.f.ins().iadd(args[node], slot.base);
            let held = (std::mem::replace(&mut self.tape, slot.tape), self.view.replace(slot.view), self.format.take());
            let value = self.call(name, &args);
            (self.tape, self.view, self.format) = (held.0, held.1, held.2);
            self.slot = Some(slot);
            return value;
        }
        let read = read(name).expect("a read");
        let signature = signature(read, self.call_conv);
        let callee = self.callee.func_ref(name, &signature, self.f.func);
        let mut all = vec![self.tape];
        all.extend(self.view);
        all.extend_from_slice(args);
        let call = self.f.ins().call(callee, &all);
        self.f.inst_results(call)[0]
    }
    /// The tape's read `name` called by a view read, with its tape's base and view.
    pub(crate) fn call_tape(&mut self, name: &str, args: &[Value]) -> Value {
        let (base, view) = self.doc_tape.expect("a view read");
        let tape = std::mem::replace(&mut self.tape, base);
        let held = self.view.replace(view);
        let value = self.call(name, args);
        self.tape = tape;
        self.view = held;
        value
    }
    /// The lines' read `name` called by a view read, with its lines' base.
    pub(crate) fn call_lines(&mut self, name: &str, args: &[Value]) -> Value {
        let base = self.lines.expect("a view read");
        let tape = std::mem::replace(&mut self.tape, base);
        let held = self.view.take();
        let value = self.call(name, args);
        self.tape = tape;
        self.view = held;
        value
    }
    /// Format 16's column `r` (0 codes, 1 slots, 2 gaps, 3 lengths, 4 checkpoints) in block `b`: where it
    /// starts. Blocks are `F16_PART` nodes at the view's stride from block 0's start, each its columns end to
    /// end at its rows (the last block's are fewer: `nodes - b * F16_PART`), column `r` at `rows *
    /// columns[r]` (decision 0434).
    fn block_column(&mut self, r: i32, b: Value) -> Value {
        let blocks = self.column(0);
        let stride = self.view(at::STRIDE);
        let base = self.f.ins().imul(b, stride);
        let base = self.f.ins().iadd(base, blocks);
        if r == 0 {
            return base;
        }
        let nodes = self.view(at::NODES);
        let first = self.f.ins().ishl_imm(b, i64::from(F16_PART.trailing_zeros()));
        let left = self.f.ins().isub(nodes, first);
        let part = self.c32(i64::from(F16_PART));
        let rows = self.f.ins().umin(left, part);
        let before = self.column(r);
        let at = self.f.ins().imul(rows, before);
        self.f.ins().iadd(base, at)
    }
    /// Node `i`'s block and its row there.
    fn block_of(&mut self, i: Value) -> (Value, Value) {
        let b = self.f.ins().ushr_imm(i, i64::from(F16_PART.trailing_zeros()));
        let j = self.f.ins().band_imm(i, i64::from(F16_PART - 1));
        (b, j)
    }
    /// Format 16's checkpoint `c`: the start of node `c * K`, in the checkpoints of its node's block.
    fn checkpoint(&mut self, c: Value) -> Value {
        let cw = self.width(3);
        let b = self.f.ins().ushr_imm(c, i64::from((F16_PART / F16_K).trailing_zeros()));
        let checkpoints = self.block_column(4, b);
        let row = self.f.ins().band_imm(c, i64::from(F16_PART / F16_K - 1));
        let at = self.f.ins().imul(row, cw);
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
        let (b, j) = self.block_of(i);
        let array = self.block_column(1 + a as i32, b);
        let at = self.f.ins().imul(j, w);
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
    let k = b.arg(0);
    b.sixteen_or_z(I64, 1, |b, i| f16_side_of(b, k, i))
}

fn f16_side_of(b: &mut B<'_, '_>, k: Value, i: Value) -> Value {
    let side = b.view_at(at::SIDE, k);
    let parts = b.view_at(at::PARTS, k);
    let p = b.f.ins().ushr_imm(i, i64::from(F16_PART.trailing_zeros()));
    let p4 = b.f.ins().imul_imm(p, 4);
    let start = b.u32(parts, p4);
    // The part's end (the next one's start, the last the list's length: both checked where the tape is placed):
    // every offset into the part is held to it, whatever its bytes state (decision 0463).
    let next4 = b.f.ins().iadd_imm(p4, 4);
    let limit = b.u32(parts, next4);
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
    let before = b.f.ins().umin(before, limit);
    let lo = b.u16(side, before);
    let lo = b.f.ins().select(first_block, zero, lo);
    let own = b.f.ins().imul_imm(j, 2);
    let own = b.f.ins().iadd(start, own);
    let own = b.f.ins().umin(own, limit);
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
    let at = b.f.ins().umin(at, limit);
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
    let last = b.f.ins().umin(last, limit);
    let count = b.u16(side, last);
    let values = b.f.ins().iadd(offsets, count);
    let at = b.f.ins().imul(found, vw);
    let at = b.f.ins().iadd(values, at);
    let at = b.f.ins().umin(at, limit);
    let word = b.word(side, at);
    let mask = b.mask(vw);
    b.f.ins().band(word, mask)
}

/// The node's subtree's size (itself and its descendants).
fn size(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    b.by_format(
        I32,
        |b, i| {
            let v = b.get(0, i);
            let leaf = b.f.ins().band_imm(v, 1);
            let leaf = b.f.ins().icmp_imm(IntCC::NotEqual, leaf, 0);
            let inner = b.f.ins().ushr_imm(v, 1);
            let inner = b.f.ins().ireduce(I32, inner);
            let one = b.c32(1);
            // At least 1 (a subtree holds its root), whatever the slot holds: every hop by sizes moves on (0463).
            let inner = b.f.ins().umax(inner, one);
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
        |b, i| {
            let (block, j) = b.block_of(i);
            let codes = b.block_column(0, block);
            let byte = b.u8(codes, j);
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
    // Held to the source (the view's `source_len`, the source its reader reads): a span from the tape reads no
    // byte past it, whatever the tape's gaps hold (decision 0463).
    let i = b.arg(0);
    let start = b.call("tape_start_stated", &[i]);
    let source = b.view(at::SOURCE_LEN);
    b.f.ins().umin(start, source)
}

/// The node's end, held between its start and the source's end (decision 0463): a reader's span is never
/// reversed and never past the source.
fn end(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let start = b.call("tape_start", &[i]);
    let end = b.call("tape_end_stated", &[i]);
    let end = b.f.ins().umax(end, start);
    let source = b.view(at::SOURCE_LEN);
    b.f.ins().umin(end, source)
}

/// The node's start as the tape states it: what a check of the tape reads (`validated::visit`), and the CPU's
/// scans, whose slices of the source are checked. No kernel reads it: theirs is `tape_start`.
fn start_stated(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    b.by_format(
        I32,
        |b, i| {
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
            let (block, row) = b.block_of(first);
            let array = b.block_column(2, block);
            let x = b.word(array, row);
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
/// The node's end as the tape states it (`start_stated`'s readers').
fn end_stated(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    b.by_format(
        I32,
        |b, i| {
            let start = b.call("tape_start_stated", &[i]);
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
            let from = b.call("tape_start_stated", &[from]);
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
            // A 16z tape's walk reads each slot in its block (`tape_slot`), back over blocks' starts.
            b.by_formats(I32, |b| parent16(b, i, false), |b| parent16(b, i, true), |b| parent15(b, i))
        },
    )
}

fn parent16(b: &mut B<'_, '_>, i: Value, z: bool) -> Value {
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
    let v = if z { b.call("tape_slot", &[before]) } else { b.get(0, before) };
    let leaf = b.f.ins().band_imm(v, 1);
    b.f.ins().brif(leaf, sibling, &[], done, &[before.into()]);
    b.f.switch_to_block(sibling);
    let step = b.f.ins().ushr_imm(v, 2);
    let step = b.f.ins().ireduce(I32, step);
    // A step of 1 to `j` back: else the slot is no tape's, and the walk ends (each step moves back, decision 0463).
    let less = b.f.ins().iadd_imm(step, -1);
    let moves = b.f.ins().icmp(IntCC::UnsignedLessThan, less, j);
    let stepped = b.f.create_block();
    b.f.ins().brif(moves, stepped, &[], done, &[none.into()]);
    b.f.switch_to_block(stepped);
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
    let end = hop(b, before, size);
    let holds = b.f.ins().icmp(IntCC::UnsignedGreaterThan, end, i);
    let next = b.f.create_block();
    b.f.ins().brif(holds, done, &[before.into()], next, &[]);
    b.f.switch_to_block(next);
    let k = b.f.ins().iadd_imm(k, 1);
    b.f.ins().jump(head, &[before.into(), k.into()]);
    // A descent from the root: at each level, the child whose subtree holds the node.
    b.f.switch_to_block(descend);
    let (level, scan, hop_to, found) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
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
    let after = hop(b, c, size);
    let past = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, after, i);
    b.f.ins().brif(past, hop_to, &[], found, &[]);
    b.f.switch_to_block(hop_to);
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
        |b, i| {
            let length = b.get(2, i);
            b.f.ins().ireduce(I32, length)
        },
        |b| {
            let end = b.call("tape_end_stated", &[i]);
            let start = b.call("tape_start_stated", &[i]);
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
        |b, i| {
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
    b.sixteen_or_z(I64, 0, |b, i| b.get(0, i))
}

/// A 16z tape's block `block` (decision 0353 step 4b): the address of the cache entry that holds it decoded, found
/// in the cache the view names (`view::cache`) or decoded into its head; the dead entry where the cache had no room.
fn z_block(b: &mut B<'_, '_>) -> Value {
    let block = b.arg(0);
    let lo = b.view(at::SIDE);
    let hi = b.view(at::SIDE + 4);
    let rel = wide(b, lo, hi);
    let cache = b.f.ins().iadd(b.tape, rel);
    let lo = b.view(at::SIDE + 8);
    let hi = b.view(at::SIDE + 12);
    let id = wide(b, lo, hi);
    let id = b.f.ins().ishl_imm(id, 22);
    let block64 = b.f.ins().uextend(I64, block);
    let key = b.f.ins().bor(id, block64);
    let size = b.column(3);
    let size = b.f.ins().uextend(I64, size);
    // A block past the tape's last (a node read that is no node of it): the dead entry, a failure counted, so the
    // read stays in the cache and its run is refused (decision 0463). The block index is read only below its end.
    let nodes = b.view(at::NODES);
    let blocks = b.f.ins().iadd_imm(nodes, i64::from(F16Z_BLOCK - 1));
    let blocks = b.f.ins().ushr_imm(blocks, i64::from(F16Z_BLOCK.trailing_zeros()));
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, block, blocks);
    let (look, outside, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(done, I64);
    b.f.ins().brif(inside, look, &[], outside, &[]);
    b.f.switch_to_block(outside);
    cache_count(b, cache, crate::view::cache::FAILED);
    let dead = cache_load(b, cache, crate::view::cache::DEAD);
    let dead = b.f.ins().uextend(I64, dead);
    let dead = b.f.ins().iadd(cache, dead);
    b.f.ins().jump(done, &[dead.into()]);
    b.f.switch_to_block(look);
    let bytes = b.column(3);
    let entry = cache_get(b, cache, key, bytes, &|b, entry| z_decode(b, block, entry, size));
    b.f.ins().jump(done, &[entry.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

/// The u64 of two u32s, `lo` first.
fn wide(b: &mut B<'_, '_>, lo: Value, hi: Value) -> Value {
    let lo = b.f.ins().uextend(I64, lo);
    let hi = b.f.ins().uextend(I64, hi);
    let hi = b.f.ins().ishl_imm(hi, 32);
    b.f.ins().bor(lo, hi)
}

/// A cache's u32 at `p + offset`, read or written by any thread.
pub(crate) fn cache_load(b: &mut B<'_, '_>, p: Value, offset: i32) -> Value {
    let p = b.f.ins().iadd_imm(p, i64::from(offset));
    b.f.ins().atomic_load(I32, MemFlags::trusted(), p)
}
fn cache_store(b: &mut B<'_, '_>, p: Value, offset: i32, value: Value) {
    let p = b.f.ins().iadd_imm(p, i64::from(offset));
    b.f.ins().atomic_store(MemFlags::trusted(), value, p);
}
fn cache_add(b: &mut B<'_, '_>, p: Value, offset: i32, value: Value) -> Value {
    let p = b.f.ins().iadd_imm(p, i64::from(offset));
    b.f.ins().atomic_rmw(I32, MemFlags::trusted(), ir::AtomicRmwOp::Add, p, value)
}
/// The statistics mode's count at `at` (`view::cache::STATS`), added to where the cache keeps them.
fn cache_stat(b: &mut B<'_, '_>, cache: Value, at: i32) {
    let on = cache_load(b, cache, crate::view::cache::STATS);
    let (count, after) = (b.f.create_block(), b.f.create_block());
    b.f.ins().brif(on, count, &[], after, &[]);
    b.f.switch_to_block(count);
    cache_count(b, cache, at);
    b.f.ins().jump(after, &[]);
    b.f.switch_to_block(after);
}

fn cache_count(b: &mut B<'_, '_>, cache: Value, offset: i32) {
    let one = b.c32(1);
    cache_add(b, cache, offset, one);
}

/// A cache entry's filler: given the entry taken, it writes the payload and gives whether it fits.
pub(crate) type Fill<'f> = &'f dyn Fn(&mut B<'_, '_>, Value) -> Value;

/// The entry of `key` in the rolling cache at `cache` (`view::cache`): looked up in its live generations, head
/// first, a hit in the oldest copied into the head, an entry another thread writes waited for (`SPIN` times at most);
/// on a miss, an entry of `size` bytes (an I32: its header and payload) taken from the head's arena, indexed,
/// `fill`ed (its payload: `fill` gives whether it fits) and completed. A full head turns the spare into the head. After
/// `TRIES` tries without room, or for a `size` past the cache's most, a failure counted and the dead entry given.
pub(crate) fn cache_get(b: &mut B<'_, '_>, cache: Value, key: Value, size: Value, fill: Fill<'_>) -> Value {
    cache_lookup(b, cache, key, size, Some(fill))
}

/// `cache_get`, or with no `fill` an open lookup: it waits for no writer, and on a miss takes an entry of `size`
/// bytes (none for 0) that its caller writes and completes (`cache_done`); its result an entry and what it found
/// (`FOUND`, `TAKEN`, `NOT_KEPT`, `PENDING`) in the low bits.
pub(crate) fn cache_lookup(b: &mut B<'_, '_>, cache: Value, key: Value, size: Value, fill: Option<Fill<'_>>) -> Value {
    use crate::view::cache as c;
    let open = fill.is_none();
    let gens = cache_load(b, cache, c::GENS);
    let max = cache_load(b, cache, c::MAX);
    let slots = cache_load(b, cache, c::SLOTS);
    let stride = cache_load(b, cache, c::GEN_STRIDE);
    let first = cache_load(b, cache, c::FIRST_ENTRY);
    let entries = cache_load(b, cache, c::ENTRIES);
    // The key's slot: its high bits of a multiplicative hash.
    let h = b.f.ins().imul_imm(key, 0x9E37_79B9_7F4A_7C15_u64 as i64);
    let h = b.f.ins().ushr_imm(h, 32);
    let h = b.f.ins().ireduce(I32, h);
    // Its tag in a slot (`view::cache::TAG_SHIFT`): 16 bits of a second hash, never 0, so a pending slot (no entry
    // yet) is not empty.
    let t = b.f.ins().imul_imm(key, 0xC2B2_AE3D_27D4_EB4F_u64 as i64);
    let t = b.f.ins().ushr_imm(t, 48);
    let t = b.f.ins().ireduce(I32, t);
    let t = b.f.ins().bor_imm(t, 1);
    let tag = b.f.ins().ishl_imm(t, i64::from(c::TAG_SHIFT));
    // Bounded by the most slots too, so a slot's offset has a bound the compiler sees (`cache::SLOTS_MAX`).
    let slot_mask = b.f.ins().iadd_imm(slots, -1);
    let slot_mask = b.f.ins().band_imm(slot_mask, i64::from(c::SLOTS_MAX - 1));
    let done = b.f.create_block();
    b.f.append_block_param(done, I64);
    // An open lookup's entry being written by another thread: the dead entry, `PENDING`.
    let pending_exit = b.f.create_block();
    // The generation of epoch `e`: its address.
    let gen_of = |b: &mut B<'_, '_>, e: Value| {
        let g = b.f.ins().urem(e, gens);
        let at = b.f.ins().imul(g, stride);
        let at = b.f.ins().uextend(I64, at);
        let at = b.f.ins().iadd_imm(at, c::HEAD as i64);
        b.f.ins().iadd(cache, at)
    };
    // Entry `n` of a generation: its offset in the arena, read from the generation's offsets (after its index).
    let entry_of = |b: &mut B<'_, '_>, generation: Value, n: Value| {
        let n = b.f.ins().uextend(I64, n);
        let n4 = b.f.ins().ishl_imm(n, 2);
        let slots4 = b.f.ins().uextend(I64, slots);
        let slots4 = b.f.ins().ishl_imm(slots4, 2);
        let at = b.f.ins().iadd(n4, slots4);
        let at = b.f.ins().iadd(generation, at);
        let off = cache_load(b, at, c::INDEX);
        let off = b.f.ins().uextend(I64, off);
        let first = b.f.ins().uextend(I64, first);
        let at = b.f.ins().iadd(off, first);
        b.f.ins().iadd(generation, at)
    };
    // Tries: each reads the epoch and looks in the live generations.
    let (try_, look, gen_ok, probe, probe_on, next_gen, found, miss) =
        (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(try_, I32);
    let zero = b.c32(0);
    b.f.ins().jump(try_, &[zero.into()]);
    b.f.switch_to_block(try_);
    let tries = b.f.block_params(try_)[0];
    let epoch = cache_load(b, cache, c::EPOCH);
    let live = b.f.ins().iadd_imm(gens, -1);
    b.f.append_block_param(look, I32);
    b.f.ins().jump(look, &[zero.into()]);
    // Generation `k` back from the head.
    b.f.switch_to_block(look);
    let k = b.f.block_params(look)[0];
    let more = b.f.ins().icmp(IntCC::UnsignedLessThan, k, live);
    let check = b.f.create_block();
    b.f.append_block_param(miss, I32);
    b.f.ins().brif(more, check, &[], miss, &[zero.into()]);
    b.f.switch_to_block(check);
    let want = b.f.ins().isub(epoch, k);
    let generation = gen_of(b, want);
    let stamp = cache_load(b, generation, c::STAMP);
    let held = b.f.ins().icmp(IntCC::Equal, stamp, want);
    b.f.ins().brif(held, gen_ok, &[], next_gen, &[]);
    b.f.switch_to_block(gen_ok);
    b.f.append_block_param(probe, I32);
    b.f.ins().jump(probe, &[zero.into()]);
    b.f.switch_to_block(probe);
    let p = b.f.block_params(probe)[0];
    let in_range = b.f.ins().icmp_imm(IntCC::UnsignedLessThan, p, i64::from(c::PROBES));
    let read_slot = b.f.create_block();
    b.f.ins().brif(in_range, read_slot, &[], next_gen, &[]);
    b.f.switch_to_block(read_slot);
    let j = b.f.ins().iadd(h, p);
    let j = b.f.ins().band(j, slot_mask);
    let j4 = b.f.ins().imul_imm(j, 4);
    let j4 = b.f.ins().uextend(I64, j4);
    let at = b.f.ins().iadd(generation, j4);
    let x = cache_load(b, at, c::INDEX);
    let empty = b.f.ins().icmp_imm(IntCC::Equal, x, 0);
    let (tagged, numbered, candidate, pending_slot) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.ins().brif(empty, next_gen, &[], tagged, &[]);
    // Another key's tag: on. Ours: its entry, or (pending: its writer has not taken one yet) waited for.
    b.f.switch_to_block(tagged);
    let its = b.f.ins().band_imm(x, !i64::from(c::NUMBER));
    let ours = b.f.ins().icmp(IntCC::Equal, its, tag);
    b.f.ins().brif(ours, numbered, &[], probe_on, &[]);
    b.f.switch_to_block(numbered);
    let number = b.f.ins().band_imm(x, i64::from(c::NUMBER));
    let none_yet = b.f.ins().icmp_imm(IntCC::Equal, number, 0);
    b.f.ins().brif(none_yet, if open { pending_exit } else { pending_slot }, &[], candidate, &[]);
    b.f.switch_to_block(pending_slot);
    let (slot_wait, slot_waiting, slot_out) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(slot_wait, I32);
    b.f.ins().jump(slot_wait, &[zero.into()]);
    b.f.switch_to_block(slot_wait);
    let spin = b.f.block_params(slot_wait)[0];
    let left = b.f.ins().icmp_imm(IntCC::UnsignedLessThan, spin, i64::from(c::SPIN));
    b.f.ins().brif(left, slot_waiting, &[], slot_out, &[]);
    b.f.switch_to_block(slot_waiting);
    b.f.ins().fence();
    let y = cache_load(b, at, c::INDEX);
    let moved = b.f.ins().icmp(IntCC::NotEqual, y, x);
    let spin = b.f.ins().iadd_imm(spin, 1);
    b.f.ins().brif(moved, read_slot, &[], slot_wait, &[spin.into()]);
    b.f.switch_to_block(slot_out);
    cache_stat(b, cache, c::DUPLICATES);
    let own = b.c32(1);
    b.f.ins().jump(miss, &[own.into()]);
    // A fence between the slot's number and its entry's offset, key and completion: the writer's fence before it put
    // the number is half of the pair; without this half a reader on a weakly ordered device (Metal's relaxed atomics)
    // can see the number with an older offset, key or completion, take the entry for another key's, and decode the
    // block into a second entry, splitting what the block's users add to it.
    b.f.switch_to_block(candidate);
    b.f.ins().fence();
    let n = b.f.ins().iadd_imm(number, -1);
    let e = entry_of(b, generation, n);
    let k64 = b.f.ins().iadd_imm(e, i64::from(c::KEY));
    let has = b.f.ins().load(I64, MemFlags::new().with_notrap(), k64, 0);
    let same = b.f.ins().icmp(IntCC::Equal, has, key);
    let ready = cache_load(b, e, c::READY);
    let complete = b.f.ins().icmp(IntCC::Equal, ready, stamp);
    let hit = b.f.ins().band(same, complete);
    b.f.append_block_param(found, I64);
    b.f.append_block_param(found, I32);
    b.f.append_block_param(found, I32);
    let (counted_hit, not_complete) = (b.f.create_block(), b.f.create_block());
    b.f.ins().brif(hit, counted_hit, &[], not_complete, &[]);
    b.f.switch_to_block(counted_hit);
    cache_stat(b, cache, c::HITS);
    b.f.ins().jump(found, &[e.into(), k.into(), stamp.into()]);
    // Its block being written by another thread (claimed first, indexed before it is decoded): waited for, a
    // bounded number of times, else decoded here too (a duplicate).
    b.f.switch_to_block(not_complete);
    let writing = b.f.ins().bor_imm(stamp, i64::from(c::WRITING));
    let pending = b.f.ins().icmp(IntCC::Equal, ready, writing);
    let pending = b.f.ins().band(same, pending);
    let (wait, waiting, waited, run_out) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(wait, I32);
    if open {
        b.f.ins().brif(pending, pending_exit, &[], probe_on, &[]);
    } else {
        b.f.ins().brif(pending, wait, &[zero.into()], probe_on, &[]);
    }
    b.f.switch_to_block(wait);
    let spin = b.f.block_params(wait)[0];
    let left = b.f.ins().icmp_imm(IntCC::UnsignedLessThan, spin, i64::from(c::SPIN));
    b.f.ins().brif(left, waiting, &[], run_out, &[]);
    b.f.switch_to_block(waiting);
    b.f.ins().fence();
    let now = cache_load(b, e, c::READY);
    let done_now = b.f.ins().icmp(IntCC::Equal, now, stamp);
    let still = b.f.ins().icmp(IntCC::Equal, now, writing);
    let spin = b.f.ins().iadd_imm(spin, 1);
    let again = b.f.create_block();
    b.f.ins().brif(done_now, waited, &[], again, &[]);
    b.f.switch_to_block(again);
    b.f.ins().brif(still, wait, &[spin.into()], probe_on, &[]);
    b.f.switch_to_block(waited);
    cache_stat(b, cache, c::WAITED);
    b.f.ins().jump(found, &[e.into(), k.into(), stamp.into()]);
    b.f.switch_to_block(run_out);
    cache_stat(b, cache, c::DUPLICATES);
    let own = b.c32(1);
    b.f.ins().jump(miss, &[own.into()]);
    b.f.switch_to_block(probe_on);
    let p = b.f.ins().iadd_imm(p, 1);
    b.f.ins().jump(probe, &[p.into()]);
    b.f.switch_to_block(next_gen);
    let k = b.f.ins().iadd_imm(k, 1);
    b.f.ins().jump(look, &[k.into()]);
    // A hit: from the oldest live generation (of two or more), copied into the head.
    b.f.switch_to_block(found);
    let (e, k, stamp) = (b.f.block_params(found)[0], b.f.block_params(found)[1], b.f.block_params(found)[2]);
    let oldest = b.f.ins().iadd_imm(live, -1);
    let is_oldest = b.f.ins().icmp(IntCC::Equal, k, oldest);
    let some_frozen = b.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, live, 1);
    let promote = b.f.ins().band(is_oldest, some_frozen);
    let copy = b.f.create_block();
    b.f.ins().brif(promote, copy, &[], done, &[e.into()]);
    b.f.switch_to_block(copy);
    // An entry's own bytes: what its copy takes and copies.
    let len = cache_load(b, e, c::LEN);
    let not_own = b.c32(0);
    let copied = cache_take(b, cache, epoch, key, tag, &gen_of, &entry_of, entries, h, slot_mask, len, not_own, Some(&|b, to| {
        // The payload, a word at a time, and the source's completion read again after it.
        let (head, step, after) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
        b.f.append_block_param(head, I64);
        let at = b.f.ins().iconst(I64, i64::from(c::PAYLOAD));
        let end = b.f.ins().uextend(I64, len);
        b.f.ins().jump(head, &[at.into()]);
        b.f.switch_to_block(head);
        let at = b.f.block_params(head)[0];
        let more = b.f.ins().icmp(IntCC::UnsignedLessThan, at, end);
        b.f.ins().brif(more, step, &[], after, &[]);
        b.f.switch_to_block(step);
        let from = b.f.ins().iadd(e, at);
        let word = b.f.ins().load(I64, MemFlags::new().with_notrap(), from, 0);
        let into = b.f.ins().iadd(to, at);
        b.f.ins().store(MemFlags::new().with_notrap(), word, into, 0);
        let at = b.f.ins().iadd_imm(at, 8);
        b.f.ins().jump(head, &[at.into()]);
        b.f.switch_to_block(after);
        b.f.ins().fence();
        let still = cache_load(b, e, c::READY);
        b.f.ins().icmp(IntCC::Equal, still, stamp)
    }));
    let took = b.f.ins().icmp_imm(IntCC::NotEqual, copied, 0);
    let counted = b.f.create_block();
    b.f.ins().brif(took, counted, &[], done, &[e.into()]);
    b.f.switch_to_block(counted);
    cache_count(b, cache, c::PROMOTED);
    b.f.ins().jump(done, &[copied.into()]);
    // A miss: an entry of the head taken and filled, else (the head full) the spare turned and tried again. An
    // entry past the most is never taken: no generation turns for it.
    b.f.switch_to_block(miss);
    // Whether its wait ran out: an entry of its own, past the pending one's slot.
    let own = b.f.block_params(miss)[0];
    let (sized, take_it, too_big, find_only) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    let nothing = b.f.ins().icmp_imm(IntCC::Equal, size, 0);
    b.f.ins().brif(nothing, find_only, &[], sized, &[]);
    b.f.switch_to_block(sized);
    let over = b.f.ins().icmp(IntCC::UnsignedGreaterThan, size, max);
    b.f.ins().brif(over, too_big, &[], take_it, &[]);
    b.f.switch_to_block(take_it);
    let taken = cache_take(b, cache, epoch, key, tag, &gen_of, &entry_of, entries, h, slot_mask, size, own, fill);
    let got = b.f.ins().icmp_imm(IntCC::NotEqual, taken, 0);
    let (hit_miss, retry, fail) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.ins().brif(got, hit_miss, &[], retry, &[]);
    b.f.switch_to_block(hit_miss);
    cache_count(b, cache, c::MISSES);
    let taken = if open { b.f.ins().bor_imm(taken, TAKEN) } else { taken };
    b.f.ins().jump(done, &[taken.into()]);
    b.f.switch_to_block(retry);
    let tries = b.f.ins().iadd_imm(tries, 1);
    let left = b.f.ins().icmp_imm(IntCC::UnsignedLessThan, tries, i64::from(c::TRIES));
    b.f.ins().brif(left, try_, &[tries.into()], fail, &[]);
    b.f.switch_to_block(too_big);
    b.f.ins().jump(fail, &[]);
    let dead_of = |b: &mut B<'_, '_>, k: i64| {
        let dead = cache_load(b, cache, c::DEAD);
        let dead = b.f.ins().uextend(I64, dead);
        let dead = b.f.ins().iadd(cache, dead);
        if open { b.f.ins().bor_imm(dead, k) } else { dead }
    };
    b.f.switch_to_block(fail);
    cache_count(b, cache, c::FAILED);
    let dead = dead_of(b, NOT_KEPT);
    b.f.ins().jump(done, &[dead.into()]);
    // A find alone (a size of 0): no entry taken, no failure.
    b.f.switch_to_block(find_only);
    let dead = dead_of(b, NOT_KEPT);
    b.f.ins().jump(done, &[dead.into()]);
    b.f.switch_to_block(pending_exit);
    let dead = dead_of(b, PENDING);
    b.f.ins().jump(done, &[dead.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

/// Entry `e` of the cache at `cache`, taken by an open lookup (`TAKEN`) and written: completed (its epoch, the
/// writing bit cleared, after a fence) and its writer gone from its generation's count.
pub(crate) fn cache_done(b: &mut B<'_, '_>, cache: Value, e: Value) {
    use crate::view::cache as c;
    b.f.ins().fence();
    let ready = cache_load(b, e, c::READY);
    let epoch = b.f.ins().band_imm(ready, !i64::from(c::WRITING));
    cache_store(b, e, c::READY, epoch);
    b.f.ins().fence();
    let stride = cache_load(b, cache, c::GEN_STRIDE);
    let stride = b.f.ins().uextend(I64, stride);
    let at = b.f.ins().isub(e, cache);
    let at = b.f.ins().iadd_imm(at, -(c::HEAD as i64));
    let g = b.f.ins().udiv(at, stride);
    let g = b.f.ins().imul(g, stride);
    let generation = b.f.ins().iadd(cache, g);
    let generation = b.f.ins().iadd_imm(generation, c::HEAD as i64);
    let minus = b.c32(-1);
    cache_add(b, generation, c::WRITERS, minus);
}

/// An entry of `size` bytes of the head at `epoch` taken for `key` from the head's arena, `fill`ed (it gives whether
/// the payload fits), completed and indexed: its address, or 0 where there was none (the head full, its index or its
/// arena: the spare turned into the head if it was cleared, by the thread whose compare-and-swap of the epoch won,
/// which clears the next spare once its writers are done; or the payload too large).
///
/// A writer counts itself in its generation (`WRITERS`) from before it takes an entry to its completion, and checks
/// the generation is still its epoch's after: a generation is cleared only with no writer in it, so no write of an
/// older epoch's lands in an entry of a newer one, whatever the entries' sizes.
#[allow(clippy::too_many_arguments)]
fn cache_take(
    b: &mut B<'_, '_>,
    cache: Value,
    epoch: Value,
    key: Value,
    tag: Value,
    gen_of: &dyn Fn(&mut B<'_, '_>, Value) -> Value,
    entry_of: &dyn Fn(&mut B<'_, '_>, Value, Value) -> Value,
    entries: Value,
    h: Value,
    slot_mask: Value,
    size: Value,
    own: Value,
    fill: Option<Fill<'_>>,
) -> Value {
    use crate::view::cache as c;
    let rw = MemFlags::new().with_notrap();
    let done = b.f.create_block();
    b.f.append_block_param(done, I64);
    let none = b.c64(0);
    let zero = b.c32(0);
    let head = gen_of(b, epoch);
    let stamp = cache_load(b, head, c::STAMP);
    let current = b.f.ins().icmp(IntCC::Equal, stamp, epoch);
    let (take, full) = (b.f.create_block(), b.f.create_block());
    b.f.ins().brif(current, take, &[], done, &[none.into()]);
    // Its slot in the head's index claimed first, pending (its tag, no entry yet), so a thread that misses the same
    // block at once finds it and waits rather than taking an entry too. A slot of the same block already, unless
    // `own` (the lookup's wait for it ran out: an entry of its own, at a slot past it): none (the lookup, tried
    // again, finds it). No slot free in its probes: the head full.
    b.f.switch_to_block(take);
    let (probe, place, check, theirs, mine_already, next, slotted) =
        (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(probe, I32);
    b.f.ins().jump(probe, &[zero.into()]);
    b.f.switch_to_block(probe);
    let p = b.f.block_params(probe)[0];
    let in_range = b.f.ins().icmp_imm(IntCC::UnsignedLessThan, p, i64::from(c::PROBES));
    b.f.ins().brif(in_range, place, &[], full, &[]);
    b.f.switch_to_block(place);
    let j = b.f.ins().iadd(h, p);
    let j = b.f.ins().band(j, slot_mask);
    let j4 = b.f.ins().imul_imm(j, 4);
    let j4 = b.f.ins().uextend(I64, j4);
    let slot = b.f.ins().iadd(head, j4);
    let slot = b.f.ins().iadd_imm(slot, i64::from(c::INDEX));
    let was = b.f.ins().atomic_cas(MemFlags::trusted(), slot, zero, tag);
    let won = b.f.ins().icmp_imm(IntCC::Equal, was, 0);
    b.f.ins().brif(won, slotted, &[], check, &[]);
    // Taken: by this block (its tag, pending or its entry's key ours), or another's.
    b.f.switch_to_block(check);
    let its = b.f.ins().band_imm(was, !i64::from(c::NUMBER));
    let same_tag = b.f.ins().icmp(IntCC::Equal, its, tag);
    let not_own = b.f.ins().icmp_imm(IntCC::Equal, own, 0);
    let same_tag = b.f.ins().band(same_tag, not_own);
    b.f.ins().brif(same_tag, theirs, &[], next, &[]);
    b.f.switch_to_block(theirs);
    let number = b.f.ins().band_imm(was, i64::from(c::NUMBER));
    let none_yet = b.f.ins().icmp_imm(IntCC::Equal, number, 0);
    let keyed = b.f.create_block();
    b.f.ins().brif(none_yet, mine_already, &[], keyed, &[]);
    // The fence pairing with the writer's before its number (as in the lookup): its entry's offset and key as written.
    b.f.switch_to_block(keyed);
    b.f.ins().fence();
    let other = b.f.ins().iadd_imm(number, -1);
    let other = entry_of(b, head, other);
    let held = b.f.ins().load(I64, rw, other, c::KEY);
    let same = b.f.ins().icmp(IntCC::Equal, held, key);
    b.f.ins().brif(same, mine_already, &[], next, &[]);
    b.f.switch_to_block(mine_already);
    b.f.ins().jump(done, &[none.into()]);
    b.f.switch_to_block(next);
    let p = b.f.ins().iadd_imm(p, 1);
    b.f.ins().jump(probe, &[p.into()]);
    // The slot this thread's: the thread counted a writer of the head, the head checked still its epoch's, an entry
    // number and its bytes taken, else (the head turned, or full) the slot given back.
    b.f.switch_to_block(slotted);
    let (counted, gone, numbered_ok, release, write) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    let one = b.c32(1);
    let minus = b.c32(-1);
    cache_add(b, head, c::WRITERS, one);
    b.f.ins().fence();
    let still = cache_load(b, head, c::STAMP);
    let ours = b.f.ins().icmp(IntCC::Equal, still, epoch);
    b.f.ins().brif(ours, counted, &[], gone, &[]);
    b.f.switch_to_block(gone);
    b.f.ins().atomic_store(MemFlags::trusted(), zero, slot);
    cache_add(b, head, c::WRITERS, minus);
    b.f.ins().jump(done, &[none.into()]);
    b.f.switch_to_block(counted);
    let n = cache_add(b, head, c::USED, one);
    let room = b.f.ins().icmp(IntCC::UnsignedLessThan, n, entries);
    b.f.ins().brif(room, numbered_ok, &[], release, &[]);
    // Its bytes, rounded to the arena's alignment: past the arena's end, the head full.
    b.f.switch_to_block(numbered_ok);
    let rounded = b.f.ins().iadd_imm(size, (c::ALIGN - 1) as i64);
    let rounded = b.f.ins().band_imm(rounded, !((c::ALIGN - 1) as i64));
    let at = cache_add(b, head, c::BUMP, rounded);
    let stride = cache_load(b, cache, c::GEN_STRIDE);
    let first = cache_load(b, cache, c::FIRST_ENTRY);
    let arena = b.f.ins().isub(stride, first);
    let at64 = b.f.ins().uextend(I64, at);
    let rounded64 = b.f.ins().uextend(I64, rounded);
    let end = b.f.ins().iadd(at64, rounded64);
    let arena64 = b.f.ins().uextend(I64, arena);
    let fits_arena = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, end, arena64);
    b.f.ins().brif(fits_arena, write, &[], release, &[]);
    b.f.switch_to_block(release);
    b.f.ins().atomic_store(MemFlags::trusted(), zero, slot);
    cache_add(b, head, c::WRITERS, minus);
    b.f.ins().jump(full, &[]);
    // Its offset under its number, its completion marked as being written and its bytes, its key (a fence after),
    // then (a fence after the key) its number in the slot, so a waiter finds it being written; then its payload and
    // (a fence after it) its completion.
    b.f.switch_to_block(write);
    let n64 = b.f.ins().uextend(I64, n);
    let n4 = b.f.ins().ishl_imm(n64, 2);
    let slots4 = b.f.ins().iadd_imm(slot_mask, 1);
    let slots4 = b.f.ins().uextend(I64, slots4);
    let slots4 = b.f.ins().ishl_imm(slots4, 2);
    let off_at = b.f.ins().iadd(n4, slots4);
    let off_at = b.f.ins().iadd(head, off_at);
    cache_store(b, off_at, c::INDEX, at);
    let e = entry_of(b, head, n);
    let writing = b.f.ins().bor_imm(epoch, i64::from(c::WRITING));
    cache_store(b, e, c::READY, writing);
    cache_store(b, e, c::LEN, size);
    b.f.ins().fence();
    b.f.ins().store(rw, key, e, c::KEY);
    b.f.ins().fence();
    let number = b.f.ins().iadd_imm(n, 1);
    let numbered = b.f.ins().bor(tag, number);
    b.f.ins().atomic_store(MemFlags::trusted(), numbered, slot);
    // Open (no fill): the entry is its caller's to write and complete (`cache_done`), its writer still counted.
    let Some(fill) = fill else {
        b.f.ins().jump(done, &[e.into()]);
        b.f.switch_to_block(full);
        cache_full(b, cache, epoch, &gen_of, slot_mask, done, none);
        b.f.switch_to_block(done);
        return b.f.block_params(done)[0];
    };
    let fits = fill(b, e);
    let (complete, unfit) = (b.f.create_block(), b.f.create_block());
    b.f.ins().brif(fits, complete, &[], unfit, &[]);
    // Not filled (too large for an entry, or a promotion whose source turned under its copy): the entry given back,
    // its slot no key's (its number, no tag: every tag has bit 16), so it neither stops a probe nor blocks the next
    // claim of the block (decision 0540).
    b.f.switch_to_block(unfit);
    cache_store(b, e, c::READY, zero);
    b.f.ins().fence();
    b.f.ins().atomic_store(MemFlags::trusted(), number, slot);
    b.f.ins().fence();
    cache_add(b, head, c::WRITERS, minus);
    b.f.ins().jump(done, &[none.into()]);
    b.f.switch_to_block(complete);
    b.f.ins().fence();
    cache_store(b, e, c::READY, epoch);
    b.f.ins().fence();
    cache_add(b, head, c::WRITERS, minus);
    b.f.ins().jump(done, &[e.into()]);
    b.f.switch_to_block(full);
    cache_full(b, cache, epoch, &gen_of, slot_mask, done, none);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

/// The head full at `epoch`: the spare turned into the head if it was cleared, or a spare left uncleared cleared
/// now; then `done` with `none`. (In the head-full block.)
fn cache_full(b: &mut B<'_, '_>, cache: Value, epoch: Value, gen_of: &dyn Fn(&mut B<'_, '_>, Value) -> Value, slot_mask: Value, done: ir::Block, none: Value) {
    use crate::view::cache as c;
    let zero = b.c32(0);
    // The head full: the spare (cleared ahead, stamped `epoch + 1`) turned into the head; a spare left uncleared
    // (stamped 0: its writers were not done when its turn came) cleared now, if they are.
    let next_epoch = b.f.ins().iadd_imm(epoch, 1);
    let spare = gen_of(b, next_epoch);
    let ready = cache_load(b, spare, c::STAMP);
    let cleared = b.f.ins().icmp(IntCC::Equal, ready, next_epoch);
    let (turn, won, uncleared, late) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.ins().brif(cleared, turn, &[], uncleared, &[]);
    b.f.switch_to_block(uncleared);
    let waiting = b.f.ins().icmp_imm(IntCC::Equal, ready, 0);
    b.f.ins().brif(waiting, late, &[], done, &[none.into()]);
    b.f.switch_to_block(late);
    cache_clear(b, spare, next_epoch, slot_mask);
    b.f.ins().jump(done, &[none.into()]);
    b.f.switch_to_block(turn);
    let at = b.f.ins().iadd_imm(cache, i64::from(c::EPOCH));
    let was = b.f.ins().atomic_cas(MemFlags::trusted(), at, epoch, next_epoch);
    let turned = b.f.ins().icmp(IntCC::Equal, was, epoch);
    b.f.ins().brif(turned, won, &[], done, &[none.into()]);
    // The next spare: the oldest live generation of `epoch`, unstamped (no lookup or writer takes it from now), then
    // cleared and stamped once no writer is in it.
    b.f.switch_to_block(won);
    cache_count(b, cache, c::ROTATED);
    let spare_epoch = b.f.ins().iadd_imm(epoch, 2);
    let generation = gen_of(b, spare_epoch);
    cache_store(b, generation, c::STAMP, zero);
    b.f.ins().fence();
    cache_clear(b, generation, spare_epoch, slot_mask);
    b.f.ins().jump(done, &[none.into()]);
}

/// Generation `generation` (unstamped) cleared and stamped `stamp` as a spare, when no writer is in it: one thread
/// clears it (its stamp's compare-and-swap from 0 to `CLEARING`), its index, entries and arena given back. With a
/// writer still in it, it stays unstamped, and a later full head clears it (`cache_take`).
fn cache_clear(b: &mut B<'_, '_>, generation: Value, stamp: Value, slot_mask: Value) {
    use crate::view::cache as c;
    let zero = b.c32(0);
    let (try_, clear, step, cleared_all, after) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    let writers = cache_load(b, generation, c::WRITERS);
    let idle = b.f.ins().icmp_imm(IntCC::Equal, writers, 0);
    b.f.ins().brif(idle, try_, &[], after, &[]);
    b.f.switch_to_block(try_);
    let at = b.f.ins().iadd_imm(generation, i64::from(c::STAMP));
    let clearing = b.c32(i64::from(c::CLEARING));
    let was = b.f.ins().atomic_cas(MemFlags::trusted(), at, zero, clearing);
    let mine = b.f.ins().icmp_imm(IntCC::Equal, was, 0);
    b.f.append_block_param(clear, I32);
    b.f.ins().brif(mine, clear, &[zero.into()], after, &[]);
    b.f.switch_to_block(clear);
    let j = b.f.block_params(clear)[0];
    let slots = b.f.ins().iadd_imm(slot_mask, 1);
    let more = b.f.ins().icmp(IntCC::UnsignedLessThan, j, slots);
    b.f.ins().brif(more, step, &[], cleared_all, &[]);
    b.f.switch_to_block(step);
    let j4 = b.f.ins().imul_imm(j, 4);
    let j4 = b.f.ins().uextend(I64, j4);
    let at = b.f.ins().iadd(generation, j4);
    cache_store(b, at, c::INDEX, zero);
    let j = b.f.ins().iadd_imm(j, 1);
    b.f.ins().jump(clear, &[j.into()]);
    b.f.switch_to_block(cleared_all);
    cache_store(b, generation, c::USED, zero);
    cache_store(b, generation, c::BUMP, zero);
    b.f.ins().fence();
    cache_store(b, generation, c::STAMP, stamp);
    b.f.ins().jump(after, &[]);
    b.f.switch_to_block(after);
}

/// Block `block` of the reader's 16z tape decoded into the cache entry `slot` (`size` bytes), as a format 16 tape
/// of one block (decision 0434 §3's stored form read through 0353 step 4b's cache): each of its `F16Z_PIECES`
/// pieces (codes, slots, gaps, lengths) decoded where its view places it, the array's column then its side list's
/// part (an LZ4 piece by `z_lz4`, a raw one copied, a repeat as the piece it names), its checkpoints copied, each
/// side list's one part's starts after them, and its view. Column `q` lies at `nb * C[q]` (`nb` its nodes), `C[q]`
/// the least that clears the piece before and 16 bytes of room: the view's `columns` hold `C`, as a block-major
/// tape's hold the bytes a row has before each column. Whether it fits the entry (1), else 0 and nothing written.
///
/// The tape's view names its piece index (`columns[0]`: the pieces' starts in the payload, then their lengths,
/// then their kinds, a byte each), its side lists' part starts (`columns[1]`), its checkpoints (`columns[2]`) and
/// its payload (`stride`).
fn z_decode(b: &mut B<'_, '_>, block: Value, slot: Value, size: Value) -> Value {
    let rw = MemFlags::new().with_notrap();
    let nodes = b.view(at::NODES);
    let first = b.f.ins().ishl_imm(block, i64::from(F16Z_BLOCK.trailing_zeros()));
    let left = b.f.ins().isub(nodes, first);
    let most = b.c32(i64::from(F16Z_BLOCK));
    let nb = b.f.ins().umin(left, most);
    let blocks = b.f.ins().iadd_imm(nodes, i64::from(F16Z_BLOCK - 1));
    let blocks = b.f.ins().ushr_imm(blocks, i64::from(F16Z_BLOCK.trailing_zeros()));
    // Each side list's part of the block: from its start to the next's (list `q`'s starts at `4 * q * (blocks + 1)`).
    let parts = b.column(1);
    let count = b.f.ins().iadd_imm(blocks, 1);
    let side: Vec<Value> = (0..4)
        .map(|q| {
            let row = b.f.ins().imul_imm(count, q);
            let row = b.f.ins().iadd(row, block);
            let at = b.f.ins().imul_imm(row, 4);
            let from = b.u32(parts, at);
            let next = b.f.ins().iadd_imm(at, 4);
            let to = b.u32(parts, next);
            // Held to its list's order (checked where the tape is read in, decision 0463): never past it.
            let to = b.f.ins().umax(to, from);
            b.f.ins().isub(to, from)
        })
        .collect();
    // The columns' places: column `q` holds `nb * w[q]` bytes, its side part after it, then 16 bytes of room.
    let w = [b.c32(1), b.width(0), b.width(1), b.width(2)];
    let cw = b.width(3);
    let nb1 = b.f.ins().iadd_imm(nb, -1);
    let mut c = vec![b.c32(0)];
    let mut ends = Vec::new();
    for q in 0..4 {
        let at = b.f.ins().imul(nb, c[q]);
        let bytes = b.f.ins().imul(nb, w[q]);
        let end = b.f.ins().iadd(at, bytes);
        ends.push(end);
        let end = b.f.ins().iadd(end, side[q]);
        let end = b.f.ins().iadd_imm(end, 16);
        let up = b.f.ins().iadd(end, nb1);
        c.push(b.f.ins().udiv(up, nb));
    }
    let ckpt_at = b.f.ins().imul(nb, c[4]);
    let ckpts = b.f.ins().iadd_imm(nb, i64::from(F16_K) - 1);
    let ckpts = b.f.ins().ushr_imm(ckpts, i64::from(F16_K.trailing_zeros()));
    let ckpt_len = b.f.ins().imul(ckpts, cw);
    let parts_at = b.f.ins().iadd(ckpt_at, ckpt_len);
    let parts_at = b.f.ins().iadd_imm(parts_at, 16 + 7);
    let parts_at = b.f.ins().band_imm(parts_at, !7);
    let end = b.f.ins().iadd_imm(parts_at, 32 + 8);
    let need = b.f.ins().uextend(I64, end);
    let need = b.f.ins().iadd_imm(need, i64::from(z::SLOT_BYTES) + z::SLACK as i64);
    let fits = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, need, size);
    let (decode, fitted) = (b.f.create_block(), b.f.create_block());
    b.f.append_block_param(fitted, I32);
    let no = b.c32(0);
    b.f.ins().brif(fits, decode, &[], fitted, &[no.into()]);
    b.f.switch_to_block(decode);
    let dst = b.f.ins().iadd_imm(slot, i64::from(z::SLOT_BYTES));
    // The block's view: its nodes, format 16's, its columns' places, its side parts, the tape's widths and choices.
    let view = b.f.ins().iadd_imm(slot, i64::from(z::SLOT_VIEW));
    let sixteen = b.c32(16);
    let zero = b.c32(0);
    let widths = b.view(at::WIDTHS);
    let (cut, per) = (b.view(at::CUT), b.view(at::PER));
    let source = b.view(at::SOURCE_LEN);
    for (value, offset) in [(nb, at::NODES), (sixteen, at::FORMAT), (widths, at::WIDTHS), (cut, at::CUT), (per, at::PER), (source, at::SOURCE_LEN), (zero, at::STRIDE)] {
        b.f.ins().store(rw, value, view, offset);
    }
    for (k, &value) in c[..5].iter().enumerate() {
        b.f.ins().store(rw, value, view, at::COLUMNS + 4 * k as i32);
    }
    let parts_end = b.f.ins().uextend(I64, parts_at);
    let parts_end = b.f.ins().iadd(dst, parts_end);
    for q in 0..4 {
        b.f.ins().store(rw, ends[q], view, at::SIDE + 4 * q as i32);
        let part = b.f.ins().iadd_imm(parts_at, 8 * q as i64);
        b.f.ins().store(rw, part, view, at::PARTS + 4 * q as i32);
        b.f.ins().store(rw, zero, parts_end, 8 * q as i32);
        b.f.ins().store(rw, side[q], parts_end, 8 * q as i32 + 4);
    }
    // Its checkpoints, raw where the tape stores them (`Z / K` a block, `cw` bytes each).
    let ckpt = b.column(2);
    let from = b.f.ins().imul_imm(block, i64::from(F16Z_BLOCK / F16_K));
    let from = b.f.ins().imul(from, cw);
    let src = b.address(ckpt, from);
    let ckpt_at64 = b.f.ins().uextend(I64, ckpt_at);
    let to = b.f.ins().iadd(dst, ckpt_at64);
    let len = b.f.ins().uextend(I64, ckpt_len);
    z_copy_exact(b, src, to, len, memory());
    // Its pieces, one a turn: the piece index's start, length and kind (a repeat's, the piece it names).
    let pieces = b.f.ins().imul_imm(blocks, i64::from(F16Z_PIECES));
    let starts = b.column(0);
    let lens = b.f.ins().imul_imm(pieces, 4);
    let lens = b.f.ins().iadd(starts, lens);
    let kinds = b.f.ins().imul_imm(pieces, 4);
    let kinds = b.f.ins().iadd(lens, kinds);
    let payload = b.view(at::STRIDE);
    let (head, body, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.ins().jump(head, &[zero.into()]);
    b.f.switch_to_block(head);
    let q = b.f.block_params(head)[0];
    let more = b.f.ins().icmp_imm(IntCC::UnsignedLessThan, q, i64::from(F16Z_PIECES));
    b.f.ins().brif(more, body, &[], done, &[]);
    b.f.switch_to_block(body);
    let j = b.f.ins().imul_imm(block, i64::from(F16Z_PIECES));
    let j = b.f.ins().iadd(j, q);
    let kind = b.u8(kinds, j);
    let j4 = b.f.ins().imul_imm(j, 4);
    let start = b.u32(starts, j4);
    let named = b.u32(payload, start);
    // A repeat names a piece of the tape that is none (checked where the tape is read in): else itself, read raw.
    let repeat = b.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(z::REPEAT));
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, named, pieces);
    let repeat = b.f.ins().band(repeat, inside);
    let k = b.f.ins().select(repeat, named, j);
    let kind = b.u8(kinds, k);
    let k4 = b.f.ins().imul_imm(k, 4);
    let start = b.u32(starts, k4);
    let len = b.u32(lens, k4);
    let src = b.address(payload, start);
    let len = b.f.ins().uextend(I64, len);
    let src_end = b.f.ins().iadd(src, len);
    // Where it decodes: its column (`nb * C[q]`, from the view just written) to its side part's end.
    let qw = b.f.ins().uextend(I64, q);
    let qw = b.f.ins().ishl_imm(qw, 2);
    let col = b.f.ins().iadd(view, qw);
    let cq = b.f.ins().load(I32, rw, col, at::COLUMNS);
    let side_at = b.f.ins().load(I32, rw, col, at::SIDE);
    let at_q = b.f.ins().imul(nb, cq);
    let pq = b.f.ins().iadd(parts_end, qw);
    let pq = b.f.ins().iadd(pq, qw);
    let side_len = b.f.ins().load(I32, rw, pq, 4);
    let end_q = b.f.ins().iadd(side_at, side_len);
    let at64 = b.f.ins().uextend(I64, at_q);
    let to = b.f.ins().iadd(dst, at64);
    let end64 = b.f.ins().uextend(I64, end_q);
    let to_end = b.f.ins().iadd(dst, end64);
    let lz4 = b.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(z::LZ4));
    let (packed, raw, next) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.ins().brif(lz4, packed, &[], raw, &[]);
    b.f.switch_to_block(packed);
    z_lz4(b, src, src_end, to, to_end);
    b.f.ins().jump(next, &[]);
    b.f.switch_to_block(raw);
    let want = b.f.ins().isub(to_end, to);
    let n = b.f.ins().umin(len, want);
    z_copy_exact(b, src, to, n, memory());
    b.f.ins().jump(next, &[]);
    b.f.switch_to_block(next);
    let q = b.f.ins().iadd_imm(q, 1);
    b.f.ins().jump(head, &[q.into()]);
    b.f.switch_to_block(done);
    let yes = b.c32(1);
    b.f.ins().jump(fitted, &[yes.into()]);
    b.f.switch_to_block(fitted);
    b.f.block_params(fitted)[0]
}

/// LZ4's block format decoded from `src..src_end` to `dst..dst_end` (decision 0353 step 4b), in words: a
/// sequence's literals 8 bytes a copy, then 4, 2 and 1; a match 8 bytes a copy, an offset under 8 widened first as
/// LZ4's reference decoder widens it (its copies may run 8 bytes past `dst_end`, into the slot's room). A length's
/// extension bytes are read one a step, each 255 bytes of the length. A sequence that would write past `dst_end`
/// or read before `dst` (a corrupt block) ends the decode.
fn z_lz4(b: &mut B<'_, '_>, src: Value, src_end: Value, dst: Value, dst_end: Value) -> Value {
    let rw = MemFlags::new().with_notrap();
    let tape = memory();
    let (head, token, literals, after, matched, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I64);
    b.f.append_block_param(head, I64);
    // Where the decode ended in the destination.
    b.f.append_block_param(done, I64);
    b.f.ins().jump(head, &[src.into(), dst.into()]);
    b.f.switch_to_block(head);
    let (s, d) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let more = b.f.ins().icmp(IntCC::UnsignedLessThan, s, src_end);
    b.f.ins().brif(more, token, &[], done, &[d.into()]);
    b.f.switch_to_block(token);
    let t = b.f.ins().uload8(I32, tape, s, 0);
    let s = b.f.ins().iadd_imm(s, 1);
    let lit = b.f.ins().ushr_imm(t, 4);
    let (lit, s) = z_length(b, lit, s, src_end);
    let lit = b.f.ins().uextend(I64, lit);
    let d_lit = b.f.ins().iadd(d, lit);
    // The literals within the destination and the source both: a length the block states past its own bytes
    // reads nothing after them (decision 0463).
    let fits = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, d_lit, dst_end);
    let s_lit = b.f.ins().iadd(s, lit);
    let held = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, s_lit, src_end);
    let fits = b.f.ins().band(fits, held);
    b.f.ins().brif(fits, literals, &[], done, &[d.into()]);
    b.f.switch_to_block(literals);
    z_copy_exact(b, s, d, lit, memory());
    let s = b.f.ins().iadd(s, lit);
    // A match's two offset bytes within the source, else the block's last literals ended it.
    let s2 = b.f.ins().iadd_imm(s, 2);
    let more = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, s2, src_end);
    b.f.ins().brif(more, after, &[], done, &[d_lit.into()]);
    // The match: its offset back, its length (4 more than its own).
    b.f.switch_to_block(after);
    let off = b.f.ins().uload16(I32, tape, s, 0);
    let off = b.f.ins().uextend(I64, off);
    let s = b.f.ins().iadd_imm(s, 2);
    let m = b.f.ins().band_imm(t, 15);
    let (m, s) = z_length(b, m, s, src_end);
    let m = b.f.ins().iadd_imm(m, 4);
    let m = b.f.ins().uextend(I64, m);
    let back = b.f.ins().isub(d_lit, dst);
    let some = b.f.ins().icmp_imm(IntCC::NotEqual, off, 0);
    let within = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, off, back);
    let d_end = b.f.ins().iadd(d_lit, m);
    let fits = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, d_end, dst_end);
    let ok = b.f.ins().band(some, within);
    let ok = b.f.ins().band(ok, fits);
    b.f.ins().brif(ok, matched, &[], done, &[d_lit.into()]);
    b.f.switch_to_block(matched);
    let from = b.f.ins().isub(d_lit, off);
    // An offset under 8: its first 8 bytes a byte at a time and a word, and the source moved on to a multiple of
    // the offset at least 8 back (LZ4's `inc32table` and `dec64table`, a nibble each).
    let (near, far, copy) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(copy, I64);
    let small = b.f.ins().icmp_imm(IntCC::UnsignedLessThan, off, 8);
    b.f.ins().brif(small, near, &[], far, &[]);
    b.f.switch_to_block(near);
    for j in 0..4 {
        let byte = b.f.ins().uload8(I32, rw, from, j);
        b.f.ins().istore8(rw, byte, d_lit, j);
    }
    let shift = b.f.ins().ishl_imm(off, 2);
    let inc = b.c64(0x4440_1210);
    let inc = b.f.ins().ushr(inc, shift);
    let inc = b.f.ins().band_imm(inc, 15);
    let at = b.f.ins().iadd(from, inc);
    let word = b.f.ins().load(I32, rw, at, 0);
    b.f.ins().store(rw, word, d_lit, 4);
    let dec = b.c64(0x7650_3444);
    let dec = b.f.ins().ushr(dec, shift);
    let dec = b.f.ins().band_imm(dec, 15);
    let at = b.f.ins().iadd_imm(at, 4);
    let at = b.f.ins().isub(at, dec);
    b.f.ins().jump(copy, &[at.into()]);
    b.f.switch_to_block(far);
    let word = b.f.ins().load(I64, rw, from, 0);
    b.f.ins().store(rw, word, d_lit, 0);
    let at = b.f.ins().iadd_imm(from, 8);
    b.f.ins().jump(copy, &[at.into()]);
    // The rest 8 bytes a copy, at least 8 back.
    b.f.switch_to_block(copy);
    let from = b.f.block_params(copy)[0];
    let (wild, step) = (b.f.create_block(), b.f.create_block());
    b.f.append_block_param(wild, I64);
    b.f.append_block_param(wild, I64);
    let to = b.f.ins().iadd_imm(d_lit, 8);
    b.f.ins().jump(wild, &[to.into(), from.into()]);
    b.f.switch_to_block(wild);
    let (to, from) = (b.f.block_params(wild)[0], b.f.block_params(wild)[1]);
    let left = b.f.ins().icmp(IntCC::UnsignedLessThan, to, d_end);
    b.f.ins().brif(left, step, &[], head, &[s.into(), d_end.into()]);
    b.f.switch_to_block(step);
    let word = b.f.ins().load(I64, rw, from, 0);
    b.f.ins().store(rw, word, to, 0);
    let to = b.f.ins().iadd_imm(to, 8);
    let from = b.f.ins().iadd_imm(from, 8);
    b.f.ins().jump(wild, &[to.into(), from.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

/// `tape_lz4(from, to, dst, len)`: the tape's bytes `from..to`, an LZ4 block, decoded into `dst` (`len` bytes, and
/// at least 8 of room after them, which a match's copies may write) by `z_lz4`; the bytes it decoded (`len` for a
/// whole block). The tape crate's `Tape16Z` decodes a piece with it (decision 0353 step 4b: one decoder).
fn lz4_block(b: &mut B<'_, '_>) -> Value {
    let (from, to) = (b.arg(0), b.arg(1));
    let (dst, len) = (b.args[2], b.args[3]);
    let from = b.f.ins().uextend(I64, from);
    let to = b.f.ins().uextend(I64, to);
    let src = b.f.ins().iadd(b.tape, from);
    let src_end = b.f.ins().iadd(b.tape, to);
    let dst_end = b.f.ins().iadd(dst, len);
    let end = z_lz4(b, src, src_end, dst, dst_end);
    b.f.ins().isub(end, dst)
}

/// An LZ4 length of `nibble` (a u32): 15 is followed by bytes added to it, the last under 255. Returns the length and
/// the source past it.
fn z_length(b: &mut B<'_, '_>, nibble: Value, s: Value, src_end: Value) -> (Value, Value) {
    let (head, step, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(head, I64);
    b.f.append_block_param(done, I32);
    b.f.append_block_param(done, I64);
    let long = b.f.ins().icmp_imm(IntCC::Equal, nibble, 15);
    b.f.ins().brif(long, head, &[nibble.into(), s.into()], done, &[nibble.into(), s.into()]);
    b.f.switch_to_block(head);
    let (n, s) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let some = b.f.ins().icmp(IntCC::UnsignedLessThan, s, src_end);
    b.f.ins().brif(some, step, &[], done, &[n.into(), s.into()]);
    b.f.switch_to_block(step);
    let byte = b.f.ins().uload8(I32, memory(), s, 0);
    let n = b.f.ins().iadd(n, byte);
    let s = b.f.ins().iadd_imm(s, 1);
    let again = b.f.ins().icmp_imm(IntCC::Equal, byte, 255);
    b.f.ins().brif(again, head, &[n.into(), s.into()], done, &[n.into(), s.into()]);
    b.f.switch_to_block(done);
    (b.f.block_params(done)[0], b.f.block_params(done)[1])
}

/// `n` bytes from `s` to `d`: 8 a copy, then 4, 2 and 1, so no load passes `s + n` (the last block's literals end
/// the tape).
fn z_copy_exact(b: &mut B<'_, '_>, s: Value, d: Value, n: Value, load: MemFlags) {
    let rw = MemFlags::new().with_notrap();
    let (head, step, tail) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I64);
    let whole = b.f.ins().band_imm(n, !7);
    let zero = b.c64(0);
    b.f.ins().jump(head, &[zero.into()]);
    b.f.switch_to_block(head);
    let k = b.f.block_params(head)[0];
    let more = b.f.ins().icmp(IntCC::UnsignedLessThan, k, whole);
    b.f.ins().brif(more, step, &[], tail, &[]);
    b.f.switch_to_block(step);
    let from = b.f.ins().iadd(s, k);
    let word = b.f.ins().load(I64, load, from, 0);
    let to = b.f.ins().iadd(d, k);
    b.f.ins().store(rw, word, to, 0);
    let k = b.f.ins().iadd_imm(k, 8);
    b.f.ins().jump(head, &[k.into()]);
    b.f.switch_to_block(tail);
    let mut k = whole;
    for (bit, ty) in [(4, I32), (2, ir::types::I16), (1, ir::types::I8)] {
        let (copy, next) = (b.f.create_block(), b.f.create_block());
        let has = b.f.ins().band_imm(n, bit);
        b.f.ins().brif(has, copy, &[], next, &[]);
        b.f.switch_to_block(copy);
        let from = b.f.ins().iadd(s, k);
        let to = b.f.ins().iadd(d, k);
        let value = b.f.ins().load(ty, load, from, 0);
        b.f.ins().store(rw, value, to, 0);
        b.f.ins().jump(next, &[]);
        b.f.switch_to_block(next);
        let step = b.f.ins().band_imm(n, bit);
        k = b.f.ins().iadd(k, step);
    }
}

/// One past the node's subtree.
fn end_of(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let size = b.call("tape_size", &[i]);
    hop(b, i, size)
}

/// `c + size`, no further than the tape's nodes, and always past `c` however `size` wraps: a walk by sizes ends
/// whatever the tape holds, and every node it gives from one of the tape's is one of the tape's (decision 0463). A
/// walk at or past the view's last node (a short count, a node asked past it) still steps on: clamped to the nodes
/// first, a hop from there was a step of zero, and a walk stopped only past its end never stopped.
fn hop(b: &mut B<'_, '_>, c: Value, size: Value) -> Value {
    let to = b.f.ins().iadd(c, size);
    let past = b.f.ins().iadd_imm(c, 1);
    let to = b.f.ins().umax(to, past);
    let nodes = b.view(at::NODES);
    let to = b.f.ins().umin(to, nodes);
    b.f.ins().umax(to, past)
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
    let c = hop(b, c, size);
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
    let (head, body, hop_to, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
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
    b.f.ins().brif(k, hop_to, &[], done, &[c.into()]);
    b.f.switch_to_block(hop_to);
    let size = b.call("tape_size", &[c]);
    let c = hop(b, c, size);
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
/// it, a word at a time, within the index's words. Past them (a `j` the index does not hold: a line past the last),
/// the position past its last word: the walk ends there. Unbounded, a word of none of them was a step of zero, and
/// past the index lies the rest of the slab, zeroed: a select of a one there walked it all (2026-11-20, a GPU held).
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
    let words = lines_field(b, 4);
    let (head, within, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(head, I32);
    b.f.append_block_param(done, I32);
    b.f.ins().jump(head, &[w0.into(), before0.into()]);
    b.f.switch_to_block(head);
    let (w, before) = (b.f.block_params(head)[0], b.f.block_params(head)[1]);
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, w, words);
    let end = b.f.ins().imul_imm(words, 64);
    b.f.ins().brif(inside, within, &[], done, &[end.into()]);
    b.f.switch_to_block(within);
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
    b.by_format_whole(
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

/// Whether the tape's block `block` (its nodes `F16_PART * block ..`; `NONE`: the whole document) has a node of kind
/// `kind`, by its presence (the view's `presence`, decision 0434 §4, step 6): 1 where the tape has none to tell by
/// (format 15, or one stored without it), and the document's answer where its blocks' presences are not held. The
/// host's planners and the kernels ask this one read.
fn has_kind(b: &mut B<'_, '_>) -> Value {
    let (block, kind) = (b.arg(0), b.arg(1));
    let at = b.view(at::PRESENCE);
    let none = b.f.ins().icmp_imm(IntCC::Equal, at, 0);
    b.either(
        I32,
        none,
        |b| b.c32(1),
        |b| {
            let zero = b.c32(0);
            let kinds = b.u32(at, zero);
            let word = b.f.ins().ushr_imm(kind, 6);
            let outside = b.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, word, kinds);
            b.either(
                I32,
                outside,
                |b| b.c32(0),
                |b| {
                    let four = b.c32(4);
                    let fields = b.u32(at, four);
                    let eight = b.c32(8);
                    let count = b.u32(at, eight);
                    // The block's presence follows the document's; `NONE` + 1 is the document's (0).
                    let next = b.f.ins().iadd_imm(block, 1);
                    let held = b.f.ins().icmp(IntCC::UnsignedLessThan, next, count);
                    let index = b.f.ins().select(held, next, zero);
                    let words = b.f.ins().iadd(kinds, fields);
                    let k = b.f.ins().imul(index, words);
                    let k = b.f.ins().iadd(k, word);
                    let k = b.f.ins().ishl_imm(k, 3);
                    let k = b.f.ins().iadd_imm(k, 16);
                    let w = b.word(at, k);
                    let shift = b.f.ins().band_imm(kind, 63);
                    let shift = b.f.ins().uextend(I64, shift);
                    let bit = b.f.ins().ushr(w, shift);
                    let bit = b.f.ins().band_imm(bit, 1);
                    b.f.ins().ireduce(I32, bit)
                },
            )
        },
    )
}

/// Line `k`'s first byte: 0 for the first; else one past newline `k - 1` (format 16: its bucket, from its one's
/// position, and its low part); format 15's line start.
fn line_start(b: &mut B<'_, '_>) -> Value {
    let k = b.arg(0);
    b.by_format_whole(
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
    /// The numbering's kernels' reads are these, as UKL (`<ukl/reads/ids.h>`): written again with `RERAIL_WRITE_READS=1`.
    #[test]
    fn the_facts_id_kernels_reads_are_these() {
        let path = format!("{}/ids.h", super::ukl_reads());
        let source = super::ids_ukl_source().unwrap();
        if std::env::var_os("RERAIL_WRITE_READS").is_some() {
            std::fs::write(&path, &source).unwrap();
        }
        let held = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(held == source, "{path} is not the reads' UKL: RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_facts_id_kernels_reads_are_these");
    }

    /// The relation's kernels' reads are these, as UKL (`<ukl/reads/relation.h>`): written again with `RERAIL_WRITE_READS=1`.
    #[test]
    fn the_blocks_kernels_reads_are_these() {
        let path = format!("{}/relation.h", super::ukl_reads());
        let source = super::relation_ukl_source().unwrap();
        if std::env::var_os("RERAIL_WRITE_READS").is_some() {
            std::fs::write(&path, &source).unwrap();
        }
        let held = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(held == source, "{path} is not the reads' UKL: RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_blocks_kernels_reads_are_these");
    }

    /// The views' kernels' reads are these, as UKL (`<ukl/reads/view.h>`): written again with `RERAIL_WRITE_READS=1`.
    #[test]
    fn the_view_kernels_reads_are_these() {
        let (header, body) = super::view_ukl_source().unwrap();
        for (path, source) in [(format!("{}/view.h", super::ukl_reads()), header), (format!("{}/../body/view_reads.h", super::ukl_reads()), body)] {
            if std::env::var_os("RERAIL_WRITE_READS").is_some() {
                std::fs::write(&path, &source).unwrap();
            }
            let held = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(held == source, "{path} is not the reads' UKL: RERAIL_WRITE_READS=1 cargo test -p rerail-clif the_view_kernels_reads_are_these");
        }
    }
}

/// The kernels' tape reads are written by hand (decision 0524): their constants are the view's, the formats' and the
/// cache's here.
#[cfg(test)]
mod ukl_header {
    use crate::view::{F15_STRIDE_SHIFT, F15_TAIL_FROM_START, F16_BLOCK, F16_K, F16_PART, F16_SUB, F16Z_BLOCK, F16Z_FORMAT, F16Z_PIECES, at, cache as c, z};

    /// `<ukl/reads/tape_common.h>`'s `CLIF_T_*` constants are the view's field offsets, the formats' numbers, the 16z
    /// entry's layout and the rolling cache's (`view::cache`), and it defines no other.
    #[test]
    fn the_tape_headers_constants_are_the_views() {
        let header = include_str!("../../ukl/lib/ukl/reads/tape_common.h");
        let defined: std::collections::HashMap<&str, u64> = header
            .lines()
            .filter_map(|line| line.strip_prefix("#define CLIF_T_"))
            .filter_map(|rest| rest.split_once(' '))
            // The reads' argument list and the format's test, not numbers.
            .filter(|(name, _)| *name != "ARGS" && !name.contains('('))
            .map(|(name, value)| {
                let value = value.trim().trim_end_matches(['u', 'l']);
                (name, value.parse().unwrap_or_else(|_| panic!("CLIF_T_{name}: not a number")))
            })
            .collect();
        let off = |v: i32| v as u64;
        let expected: &[(&str, u64)] = &[
            ("NODES", off(at::NODES)),
            ("FORMAT", off(at::FORMAT)),
            ("WIDTHS", off(at::WIDTHS)),
            ("COLUMNS", off(at::COLUMNS)),
            ("STRIDE", off(at::STRIDE)),
            ("SIDE", off(at::SIDE)),
            ("PARTS", off(at::PARTS)),
            ("CUT", off(at::CUT)),
            ("PER", off(at::PER)),
            ("SOURCE_LEN", off(at::SOURCE_LEN)),
            ("LINES", off(at::LINES)),
            ("LINE_COUNT", off(at::LINE_COUNT)),
            ("PRESENCE", off(at::PRESENCE)),
            ("NONE", super::NONE as u64),
            ("F16Z_FORMAT", F16Z_FORMAT.into()),
            ("K", F16_K.into()),
            ("BLOCK", F16_BLOCK.into()),
            ("PART", F16_PART.into()),
            ("SUB", F16_SUB.into()),
            ("F15_STRIDE_SHIFT", F15_STRIDE_SHIFT.into()),
            ("F15_TAIL_FROM_START", F15_TAIL_FROM_START.into()),
            ("LINES_HEAD", super::HEAD_LINES as u64),
            ("LINES_SAMPLE_SHIFT", super::LINES_SAMPLE_SHIFT as u64),
            ("Z_BLOCK", F16Z_BLOCK.into()),
            ("Z_PIECES", F16Z_PIECES.into()),
            ("SLOT_VIEW", off(z::SLOT_VIEW)),
            ("SLOT_BYTES", off(z::SLOT_BYTES)),
            ("SLACK", z::SLACK as u64),
            ("RAW", z::RAW.into()),
            ("LZ4", z::LZ4.into()),
            ("REPEAT", z::REPEAT.into()),
            ("C_EPOCH", off(c::EPOCH)),
            ("C_GENS", off(c::GENS)),
            ("C_ENTRIES", off(c::ENTRIES)),
            ("C_MAX", off(c::MAX)),
            ("C_SLOTS", off(c::SLOTS)),
            ("C_GEN_STRIDE", off(c::GEN_STRIDE)),
            ("C_FIRST_ENTRY", off(c::FIRST_ENTRY)),
            ("C_DEAD", off(c::DEAD)),
            ("C_FAILED", off(c::FAILED)),
            ("C_MISSES", off(c::MISSES)),
            ("C_PROMOTED", off(c::PROMOTED)),
            ("C_ROTATED", off(c::ROTATED)),
            ("C_STATS", off(c::STATS)),
            ("C_HITS", off(c::HITS)),
            ("C_WAITED", off(c::WAITED)),
            ("C_DUPLICATES", off(c::DUPLICATES)),
            ("C_HEAD", c::HEAD as u64),
            ("G_STAMP", off(c::STAMP)),
            ("G_USED", off(c::USED)),
            ("G_BUMP", off(c::BUMP)),
            ("G_WRITERS", off(c::WRITERS)),
            ("G_INDEX", off(c::INDEX)),
            ("E_KEY", off(c::KEY)),
            ("E_READY", off(c::READY)),
            ("E_LEN", off(c::LEN)),
            ("E_PAYLOAD", off(c::PAYLOAD)),
            ("C_ALIGN", c::ALIGN as u64),
            ("C_CLEARING", c::CLEARING.into()),
            ("C_WRITING", c::WRITING.into()),
            ("C_PROBES", c::PROBES.into()),
            ("C_SLOTS_MAX", c::SLOTS_MAX.into()),
            ("C_TRIES", c::TRIES.into()),
            ("C_SPIN", c::SPIN.into()),
            ("C_TAG_SHIFT", c::TAG_SHIFT.into()),
            ("C_NUMBER", c::NUMBER.into()),
            // A probe's outcomes, the header's own.
            ("C_NOT_HERE", 0),
            ("C_FOUND", 1),
            ("C_PENDING", 2),
        ];
        for &(name, value) in expected {
            assert_eq!(defined.get(name), Some(&value), "CLIF_T_{name}");
        }
        let names: std::collections::HashSet<&str> = expected.iter().map(|(name, _)| *name).collect();
        let extra: Vec<&&str> = defined.keys().filter(|name| !names.contains(**name)).collect();
        assert!(extra.is_empty(), "constants the test does not hold: {extra:?}");
    }
}
