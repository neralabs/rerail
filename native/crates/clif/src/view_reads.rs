//! The views' reads as CLIF functions, written once (decision 0353 step 4c): a view's row is a read of the tape and
//! of its projection's numbering, never a relation made of it. The kernels (the UKL backend: `<ukl/reads/view.h>`) and
//! the JIT read the views through them; the host's `Viewed` answers the same rows, which a test checks.
//!
//! A read takes the document's entry (`ViewDoc`, in its batch's block beside the projection's numbering, which it
//! refers to by a run), the tape's base and its view (`view::TapeView`), then a row's local number and a column. The
//! entry holds no per-node column: what a node's row says is its tape node's (`num_select`), read off the tape
//! (`tape_number`, `tape_start`, `tape_end`); a kind and a field are its code's, in its language's run of the codes'
//! directory (`CodesLang`, decision 0697), which the binary holds and a read is passed.

// The layout, and its host reads (`rerail_reads::views`), where the reads have always been found.
pub use rerail_reads::views::*;
#[cfg(feature = "ir")]
use crate::reads::B;
#[cfg(feature = "ir")]
use cranelift_codegen::ir::{
    InstBuilder, Value,
    condcodes::IntCC,
    types::{I32, I64},
};

/// The entry in UKL: `struct ViewDoc`, each field's offset and the size asserted against this definition's.
#[cfg(feature = "ir")]
pub fn ukl() -> String {
    use std::mem::offset_of;
    let fields: [(&str, &str, usize); 27] = [
        ("uint32_t", "key", offset_of!(ViewDoc, key)),
        ("uint32_t", "tag", offset_of!(ViewDoc, tag)),
        ("uint32_t", "preorder", offset_of!(ViewDoc, preorder)),
        ("uint32_t", "nodes", offset_of!(ViewDoc, nodes)),
        ("Run", "numbering", offset_of!(ViewDoc, numbering)),
        ("uint32_t", "tokens", offset_of!(ViewDoc, tokens)),
        ("uint32_t", "own_text", offset_of!(ViewDoc, own_text)),
        ("uint32_t", "ends", offset_of!(ViewDoc, ends)),
        ("uint32_t", "source_len", offset_of!(ViewDoc, source_len)),
        ("uint32_t", "placement", offset_of!(ViewDoc, placement)),
        ("uint32_t", "file", offset_of!(ViewDoc, file)),
        ("int64_t", "delta", offset_of!(ViewDoc, delta)),
        ("uint32_t", "within_lo", offset_of!(ViewDoc, within_lo)),
        ("uint32_t", "within_hi", offset_of!(ViewDoc, within_hi)),
        ("Run", "placed", offset_of!(ViewDoc, placed)),
        ("Run", "gaps[2]", offset_of!(ViewDoc, gaps)),
        ("Run", "named", offset_of!(ViewDoc, named)),
        ("Run", "visits", offset_of!(ViewDoc, visits)),
        ("Run", "digests", offset_of!(ViewDoc, digests)),
        ("Run", "names", offset_of!(ViewDoc, names)),
        ("Run", "segments", offset_of!(ViewDoc, segments)),
        ("uint32_t", "place", offset_of!(ViewDoc, place)),
        ("uint32_t", "doc_path", offset_of!(ViewDoc, doc_path)),
        ("uint32_t", "doc_module", offset_of!(ViewDoc, doc_module)),
        ("uint32_t", "parented", offset_of!(ViewDoc, parented)),
        ("Run", "node_table", offset_of!(ViewDoc, node_table)),
        ("Run", "node_by_id", offset_of!(ViewDoc, node_by_id)),
    ];
    let mut s = format!(
        "#include <ukl/backend.h>\ntypedef struct ViewRow {{\n    uint64_t entry;\n    uint64_t tape;\n    uint64_t lines;\n    uint64_t source;\n    uint32_t homes[3];\n    uint32_t key;\n    uint32_t rows[{}];\n    uint32_t view[{}];\n    uint32_t bytes;\n    uint32_t at;\n}} ViewRow;\n\
         typedef struct ViewHome {{\n    uint64_t host;\n    UKL_GLOBAL const uint8_t* gpu;\n}} ViewHome;\n\
         typedef struct ViewDir {{
    uint64_t at;
    uint32_t ndocs;
    uint32_t nhomes;
    uint64_t rows;
    uint64_t homes;
    uint32_t cache;
    uint32_t nbatch;
    uint64_t keys;
    uint32_t ends[{}];
    uint32_t counted;
    uint32_t codes;
    uint64_t pages;
    uint64_t order;
    uint32_t first;
    uint32_t span;
}} ViewDir;
",
        ROWS,
        std::mem::size_of::<crate::view::TapeView>() / 4,
        ROWS,
    );
    s += &format!("UKL_STATIC_ASSERT(sizeof(ViewRow) == {}, \"ViewRow: size\");\n", std::mem::size_of::<ViewRow>());
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewRow, source) == {}, \"ViewRow.source\");\n", std::mem::offset_of!(ViewRow, source));
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewRow, homes) == {}, \"ViewRow.homes\");\n", std::mem::offset_of!(ViewRow, homes));
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewRow, view) == {}, \"ViewRow.view\");\n", std::mem::offset_of!(ViewRow, view));
    s += &format!("UKL_STATIC_ASSERT(sizeof(ViewHome) == {}, \"ViewHome: size\");\n", std::mem::size_of::<ViewHome>());
    s += &format!("UKL_STATIC_ASSERT(sizeof(ViewDir) == {}, \"ViewDir: size\");\n", std::mem::size_of::<ViewDir>());
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewDir, rows) == {}, \"ViewDir.rows\");\n", std::mem::offset_of!(ViewDir, rows));
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewDir, cache) == {}, \"ViewDir.cache\");\n", std::mem::offset_of!(ViewDir, cache));
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewDir, nbatch) == {}, \"ViewDir.nbatch\");\n", std::mem::offset_of!(ViewDir, nbatch));
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewDir, keys) == {}, \"ViewDir.keys\");\n", std::mem::offset_of!(ViewDir, keys));
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewDir, codes) == {}, \"ViewDir.codes\");\n", std::mem::offset_of!(ViewDir, codes));
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewDir, pages) == {}, \"ViewDir.pages\");\n", std::mem::offset_of!(ViewDir, pages));
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewDir, order) == {}, \"ViewDir.order\");\n", std::mem::offset_of!(ViewDir, order));
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewRow, at) == {}, \"ViewRow.at\");\n", std::mem::offset_of!(ViewRow, at));
    // A directory's places as a kernel reads them: the block's start, a row, and a row's tape and lines at their
    // homes' device bases.
    s += "UKL_INLINE UKL_GLOBAL const uint8_t* view_block(UKL_GLOBAL const ViewDir* d) { return (UKL_GLOBAL const uint8_t*)d - d->at; }\n";
    s += "UKL_INLINE UKL_GLOBAL const ViewRow* view_rows(UKL_GLOBAL const ViewDir* d) { return (UKL_GLOBAL const ViewRow*)(view_block(d) + d->rows); }\n";
    // The first row of view `k` past row `e`'s documents' rows: the next in the directory's order's (`ViewDir::order`),
    // `end` past the last.
    s += "UKL_INLINE uint32_t view_rows_end(UKL_GLOBAL const ViewDir* d, UKL_GLOBAL const ViewRow* e, uint32_t k, uint32_t end) { uint32_t m = e->at + 1u; return m < d->ndocs ? view_rows(d)[((UKL_GLOBAL const uint32_t*)(view_block(d) + d->order))[m]].rows[k] : end; }\n";
    s += "UKL_INLINE UKL_GLOBAL const uint8_t* view_home(UKL_GLOBAL const ViewDir* d, uint32_t h, uint64_t at) { return ((UKL_GLOBAL const ViewHome*)(view_block(d) + d->homes))[h].gpu + at; }\n";
    s += "UKL_INLINE UKL_GLOBAL const uint8_t* view_tape(UKL_GLOBAL const ViewDir* d, UKL_GLOBAL const ViewRow* e) { return view_home(d, e->homes[0], e->tape); }\n";
    // A row's source where it lies, in its own home (its third: `ViewRow::source`), a node's text its span of it
    // (`@rr_node_text_any`, no text made); and its length, its view's.
    s += "UKL_INLINE UKL_GLOBAL const uint8_t* view_source(UKL_GLOBAL const ViewDir* d, UKL_GLOBAL const ViewRow* e) { return view_home(d, e->homes[2], e->source); }\n";
    s += &format!(
        "UKL_INLINE uint32_t view_source_len(UKL_GLOBAL const ViewRow* e) {{ return e->view[{}]; }}\n",
        std::mem::offset_of!(crate::view::TapeView, source_len) / 4
    );
    // A document's lines by their home's base and their offset there (`ViewRow::lines`): a view read takes both (`ll`,
    // `la`), as a load's base must be 8-aligned and a tape's line index is 4-aligned in it (decision 0612).
    s += "UKL_INLINE UKL_GLOBAL const uint8_t* view_lines_base(UKL_GLOBAL const ViewDir* d, UKL_GLOBAL const ViewRow* e) { return view_home(d, e->homes[1], 0); }\n";
    // The codes' directory (decision 0697), every row's: the binary's, at its home's device base.
    s += "UKL_INLINE UKL_GLOBAL const uint8_t* view_codes(UKL_GLOBAL const ViewDir* d) { return view_home(d, d->codes, 0); }\n";
    s += "typedef struct ViewDoc {\n";
    for (ty, name, _) in fields.iter() {
        s += &format!("    {ty} {name};\n");
    }
    s += "} ViewDoc;\n";
    s += &format!("UKL_STATIC_ASSERT(sizeof(ViewDoc) == {}, \"ViewDoc: size\");\n", std::mem::size_of::<ViewDoc>());
    for (_, name, at) in fields.iter() {
        // An array's offset is its first element's: its name without the length.
        let name = name.split('[').next().unwrap_or(name);
        s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewDoc, {name}) == {at}, \"ViewDoc.{name}\");\n");
    }
    s
}
// The reads themselves, in CLIF: with the IR (`ir`) alone. The entry's layout above is the view's too, which the
// tape crate reads without the IR.
#[cfg(feature = "ir")]
impl B<'_, '_> {
    /// The entry's u32 at `offset`, widened.
    fn entry_u32(&mut self, offset: i32) -> Value {
        let v = self.word_at(I32, self.tape, offset);
        self.f.ins().uextend(I64, v)
    }
    /// Code `code`'s atom in the document's language's run of the codes' directory (`read`: `codes_kind` or
    /// `codes_field`), widened.
    fn code_atom(&mut self, read: &str, code: Value) -> Value {
        let tag = self.word_at(I32, self.tape, at::TAG);
        let v = self.call_codes(read, &[tag, code]);
        self.f.ins().uextend(I64, v)
    }
    /// The u32 at index `i` of the entry's run at `offset`, widened: the entry's own table (`ViewDoc::names`, the
    /// store's atoms, which the binary's directory cannot hold).
    fn entry_item(&mut self, offset: i32, i: Value) -> Value {
        let items = self.follow(self.tape, offset);
        let p = self.item(items, i, 4);
        let v = self.word_at(I32, p, 0);
        self.f.ins().uextend(I64, v)
    }
    /// Numbered node `l`'s id: the document's key, then its low bits (`l` itself where ids are local ids).
    fn node_id(&mut self, l: Value) -> Value {
        let key = self.entry_u32(at::KEY);
        let high = self.f.ins().ishl_imm(key, 32);
        let preorder = self.word_at(I32, self.tape, at::PREORDER);
        let local = self.f.ins().icmp_imm(IntCC::Equal, preorder, 1);
        let low = self.either(I32, local, |_| l, |b| {
            let tape = b.ids_tape();
            b.either(I32, tape, |b| b.tape_node(l), |b| {
                let numbering = b.follow(b.tape, at::NUMBERING);
                let node = b.c32(0);
                b.call_at("num_low_of", numbering, &[node, l])
            })
        });
        let low = self.f.ins().uextend(I64, low);
        self.f.ins().bor(high, low)
    }
    /// Whether the document's ids are tape positions (`IDS_TAPE`).
    fn ids_tape(&mut self) -> Value {
        let preorder = self.word_at(I32, self.tape, at::PREORDER);
        self.f.ins().icmp_imm(IntCC::Equal, preorder, i64::from(IDS_TAPE))
    }
    /// Numbered node `l`'s parent's id (`Projected::parent`): its tape parent's, or the root's where that is not
    /// numbered (a narrowed projection's anchor); -1 for the root.
    fn parent_id(&mut self, l: Value, x: Value) -> Value {
        let root = self.f.ins().icmp_imm(IntCC::Equal, l, 0);
        self.either(I64, root, |b| b.c64(-1), |b| {
            let q = b.call_tape("tape_parent", &[x]);
            let numbering = b.follow(b.tape, at::NUMBERING);
            let node = b.c32(0);
            let is = b.call_at("num_is", numbering, &[node, q]);
            let numbered = b.f.ins().icmp_imm(IntCC::NotEqual, is, 0);
            let local = b.either(I32, numbered, |b| {
                let numbering = b.follow(b.tape, at::NUMBERING);
                let node = b.c32(0);
                b.call_at("num_rank", numbering, &[node, q])
            }, |b| b.c32(0));
            b.node_id(local)
        })
    }
    /// Numbered node `l`'s tape node (`num_select`).
    fn tape_node(&mut self, l: Value) -> Value {
        let numbering = self.follow(self.tape, at::NUMBERING);
        let node = self.c32(0);
        self.call_at("num_select", numbering, &[node, l])
    }
}

/// `view_node(l, col)`: column `col` of the `node` view's row of numbered node `l`: the document (a program's file,
/// decision 0431 §4), the id, the kind atom, the start, the end (`Viewed::value_of`).
#[cfg(feature = "ir")]
pub fn node(b: &mut B<'_, '_>) -> Value {
    let (l, col) = (b.arg(0), b.arg(1));
    let is = |b: &mut B<'_, '_>, k: i64| b.f.ins().icmp_imm(IntCC::Equal, col, k);
    let c = is(b, 0);
    b.either(I64, c, |b| b.entry_u32(at::FILE), |b| {
        let c = is(b, 1);
        b.either(I64, c, |b| b.node_id(l), |b| {
            let x = b.tape_node(l);
            let c = is(b, 2);
            b.either(I64, c, |b| {
                let code = b.call_tape("tape_number", &[x]);
                b.code_atom("codes_kind", code)
            }, |b| {
                let c = is(b, 3);
                let v = b.either(I32, c, |b| b.call_tape("tape_start", &[x]), |b| b.call_tape("tape_end", &[x]));
                b.f.ins().uextend(I64, v)
            })
        })
    })
}

/// `view_child(i, col)`: column `col` of the document's `child` row `i`, of numbered node `i + 1` (every node but the
/// root): the parent's id, the child's id, its place among the parent's children: read in the tape's auxiliary index
/// (`tape_place`, or `tape_place_past_comments` where the entry says so), an outermost anchor's its rank among them.
#[cfg(feature = "ir")]
pub fn child(b: &mut B<'_, '_>) -> Value {
    let (i, col) = (b.arg(0), b.arg(1));
    let l = b.f.ins().iadd_imm(i, 1);
    let c = b.f.ins().icmp_imm(IntCC::Equal, col, 0);
    b.either(I64, c, |b| {
        let x = b.tape_node(l);
        b.parent_id(l, x)
    }, |b| {
        let c = b.f.ins().icmp_imm(IntCC::Equal, col, 1);
        b.either(I64, c, |b| b.node_id(l), |b| {
            let x = b.tape_node(l);
            let numbering = b.follow(b.tape, at::NUMBERING);
            let anchor = b.c32(ANCHOR_ITEM);
            let is = b.call_at("num_is", numbering, &[anchor, x]);
            let is = b.f.ins().icmp_imm(IntCC::NotEqual, is, 0);
            let place = b.either(I32, is, |b| {
                let numbering = b.follow(b.tape, at::NUMBERING);
                let anchor = b.c32(ANCHOR_ITEM);
                b.call_at("num_rank", numbering, &[anchor, x])
            }, |b| {
                let kind = b.word_at(I32, b.tape, at::PLACE);
                let past = b.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(PLACE_PAST_COMMENTS));
                b.either(I32, past, |b| b.call_tape("tape_place_past_comments", &[x]), |b| b.call_tape("tape_place", &[x]))
            });
            b.f.ins().uextend(I64, place)
        })
    })
}

/// `view_field(i, col)`: column `col` of the document's `field` row `i`, of its `i`-th node under a field
/// (`num_select` of the fielded nodes): the parent's id, the field atom, the child's id.
#[cfg(feature = "ir")]
pub fn field(b: &mut B<'_, '_>) -> Value {
    let (i, col) = (b.arg(0), b.arg(1));
    // Its tape node, and that node's number: each column's branch finds what it reads, as `child`'s does. Found once
    // before the branches, the M1 Max's compile (G13) read the tape there as zeros: the parent read `x - 1` and the
    // field atom 0, while the same MSL read right on the M3 (decision 0353 step 4d; the views' test).
    let tape_node = |b: &mut B<'_, '_>| {
        let numbering = b.follow(b.tape, at::NUMBERING);
        let fielded = b.c32(2);
        b.call_at("num_select", numbering, &[fielded, i])
    };
    let numbered = |b: &mut B<'_, '_>, x: Value| {
        let numbering = b.follow(b.tape, at::NUMBERING);
        let node = b.c32(0);
        b.call_at("num_rank", numbering, &[node, x])
    };
    let c = b.f.ins().icmp_imm(IntCC::Equal, col, 0);
    b.either(I64, c, |b| {
        let x = tape_node(b);
        let l = numbered(b, x);
        b.parent_id(l, x)
    }, |b| {
        let c = b.f.ins().icmp_imm(IntCC::Equal, col, 1);
        b.either(I64, c, |b| {
            let x = tape_node(b);
            let code = b.call_tape("tape_number", &[x]);
            b.code_atom("codes_field", code)
        }, |b| {
            let x = tape_node(b);
            let l = numbered(b, x);
            b.node_id(l)
        })
    })
}

/// `view_core(l, col)`: column `col` of `core_node`'s row of numbered node `l`: the document, its tape node, its id.
#[cfg(feature = "ir")]
pub fn core(b: &mut B<'_, '_>) -> Value {
    let (l, col) = (b.arg(0), b.arg(1));
    let c = b.f.ins().icmp_imm(IntCC::Equal, col, 0);
    b.either(I64, c, |b| b.entry_u32(at::KEY), |b| {
        let c = b.f.ins().icmp_imm(IntCC::Equal, col, 1);
        b.either(I64, c, |b| {
            let x = b.tape_node(l);
            b.f.ins().uextend(I64, x)
        }, |b| b.node_id(l))
    })
}

/// `view_line(l, col)`: column `col` of the `line` view's row of numbered node `l`: its id, the line its start is
/// on.
#[cfg(feature = "ir")]
pub fn line(b: &mut B<'_, '_>) -> Value {
    let (l, col) = (b.arg(0), b.arg(1));
    let c = b.f.ins().icmp_imm(IntCC::Equal, col, 0);
    b.either(I64, c, |b| b.node_id(l), |b| {
        let x = b.tape_node(l);
        let start = b.call_tape("tape_start", &[x]);
        let line = b.call_lines("lines_line", &[start]);
        b.f.ins().uextend(I64, line)
    })
}

/// Which place a `Loc` view's second column holds (`view_loc_*`).
#[cfg(feature = "ir")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Line,
    EndLine,
    Col,
    Docof,
}

#[cfg(feature = "ir")]
impl B<'_, '_> {
    /// `Loc` position `l`'s id: a node's or token's by its numbering (`num_low_of`), a span's in the top slots
    /// (`granules::span_low`); `l` itself where ids are local ids (`Projected::global`).
    fn loc_id(&mut self, l: Value) -> Value {
        use crate::ids::{INDEX_BITS, SLOT_BITS, SPAN_SLOTS};
        let key = self.entry_u32(at::KEY);
        let high = self.f.ins().ishl_imm(key, 32);
        let preorder = self.word_at(I32, self.tape, at::PREORDER);
        let local = self.f.ins().icmp_imm(IntCC::Equal, preorder, 1);
        let low = self.either(I32, local, |_| l, |b| {
            let nodes = b.word_at(I32, b.tape, at::NODES);
            let node = b.f.ins().icmp(IntCC::UnsignedLessThan, l, nodes);
            b.either(I32, node, |b| {
                // A node's or token's id where ids are tape positions: its tape node (`num_select`).
                let tape = b.ids_tape();
                let numbering = b.follow(b.tape, at::NUMBERING);
                let item = b.c32(0);
                b.either(I32, tape, |b| b.call_at("num_select", numbering, &[item, l]), |b| b.call_at("num_low_of", numbering, &[item, l]))
            }, |b| {
                let i = b.f.ins().isub(l, nodes);
                let tokens = b.word_at(I32, b.tape, at::TOKENS);
                let token = b.f.ins().icmp(IntCC::UnsignedLessThan, i, tokens);
                b.either(I32, token, |b| {
                    let tape = b.ids_tape();
                    let numbering = b.follow(b.tape, at::NUMBERING);
                    let item = b.c32(1);
                    b.either(I32, tape, |b| b.call_at("num_select", numbering, &[item, i]), |b| b.call_at("num_low_of", numbering, &[item, i]))
                }, |b| {
                    let j = b.f.ins().isub(i, tokens);
                    let hi = b.f.ins().ushr_imm(j, i64::from(INDEX_BITS));
                    let top = b.c32(i64::from(SPAN_SLOTS - 1));
                    let hi = b.f.ins().umin(hi, top);
                    let last = b.c32(i64::from((1u32 << SLOT_BITS) - 1));
                    let slot = b.f.ins().isub(last, hi);
                    let slot = b.f.ins().ishl_imm(slot, i64::from(INDEX_BITS));
                    let index = b.f.ins().band_imm(j, i64::from((1u32 << INDEX_BITS) - 1));
                    b.f.ins().bor(slot, index)
                })
            })
        });
        let low = self.f.ins().uextend(I64, low);
        self.f.ins().bor(high, low)
    }
    /// The document's lines (`lines_count`), read off its line index at the view's lines' base: the one definition
    /// of a line and a place (decision 0612); a guest's are its host's.
    fn loc_count(&mut self) -> Value {
        self.call_lines("lines_count", &[])
    }
    /// Line `k`'s first byte (from 0, `lines_start`).
    fn loc_start(&mut self, k: Value) -> Value {
        self.call_lines("lines_start", &[k])
    }
    /// `Loc` position `l`'s byte span, as `start << 32 | end` (`Projected::span`): a node's or token's off the tape,
    /// the whole text, a line's text, or a node's end point.
    fn loc_span(&mut self, l: Value) -> Value {
        let nodes = self.word_at(I32, self.tape, at::NODES);
        let node = self.f.ins().icmp(IntCC::UnsignedLessThan, l, nodes);
        let of_tape = |b: &mut Self, x: Value| {
            let s = b.call_tape("tape_start", &[x]);
            let e = b.call_tape("tape_end", &[x]);
            b.pair(s, e)
        };
        self.either(I64, node, |b| {
            let x = b.tape_node(l);
            of_tape(b, x)
        }, |b| {
            let i = b.f.ins().isub(l, nodes);
            let tokens = b.word_at(I32, b.tape, at::TOKENS);
            let token = b.f.ins().icmp(IntCC::UnsignedLessThan, i, tokens);
            b.either(I64, token, |b| {
                let numbering = b.follow(b.tape, at::NUMBERING);
                let item = b.c32(1);
                let x = b.call_at("num_select", numbering, &[item, i]);
                of_tape(b, x)
            }, |b| {
                let j = b.f.ins().isub(i, tokens);
                // The end points follow the whole text and the lines where the text is the document's own.
                let own = b.word_at(I32, b.tape, at::OWN_TEXT);
                let n = b.loc_count();
                let texts = b.f.ins().iadd_imm(n, 1);
                let zero = b.c32(0);
                let owned = b.f.ins().icmp_imm(IntCC::NotEqual, own, 0);
                let ends_from = b.f.ins().select(owned, texts, zero);
                let end_point = b.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, j, ends_from);
                b.either(I64, end_point, |b| {
                    let k = b.f.ins().isub(j, ends_from);
                    let x = b.tape_node(k);
                    let e = b.call_tape("tape_end", &[x]);
                    b.pair(e, e)
                }, |b| {
                    let whole = b.f.ins().icmp_imm(IntCC::Equal, j, 0);
                    b.either(I64, whole, |b| {
                        let zero = b.c32(0);
                        let len = b.word_at(I32, b.tape, at::SOURCE_LEN);
                        b.pair(zero, len)
                    }, |b| {
                        // Line `j`'s (from 1) place, from its start alone: its text's end is on it (a terminator's
                        // bytes are the line's they end), so its start stands for both ends here.
                        let k = b.f.ins().iadd_imm(j, -1);
                        let s = b.loc_start(k);
                        b.pair(s, s)
                    })
                })
            })
        })
    }
    /// A guest's span `start << 32 | end` where it stands in its host (`Placement::host`): copied, each end moved by
    /// `delta` and kept within `within`; at `within`; an embedder's, by its segments (`placed_segments`). A file's
    /// own, as it is.
    fn placed(&mut self, span: Value) -> Value {
        let placement = self.word_at(I32, self.tape, at::PLACEMENT);
        let own = self.f.ins().icmp_imm(IntCC::Equal, placement, 0);
        self.either(I64, own, |_| span, |b| {
            let segments = b.f.ins().icmp_imm(IntCC::Equal, placement, i64::from(PLACED_SEGMENTS));
            b.either(I64, segments, |b| b.placed_segments(span), |b| b.placed_stretch(placement, span))
        })
    }
    /// `placed` of a copied stretch or a point.
    fn placed_stretch(&mut self, placement: Value, span: Value) -> Value {
        let b = self;
        let lo = b.entry_u32(at::WITHIN_LO);
        let hi = b.entry_u32(at::WITHIN_HI);
        let at_ = b.f.ins().icmp_imm(IntCC::Equal, placement, i64::from(PLACED_AT));
        b.either(I64, at_, |b| {
            let s = b.f.ins().ishl_imm(lo, 32);
            b.f.ins().bor(s, hi)
        }, |b| {
            let delta = b.word_at(I64, b.tape, at::DELTA);
            let clamp = |b: &mut Self, x: Value| {
                let x = b.f.ins().iadd(x, delta);
                let x = b.f.ins().smax(x, lo);
                b.f.ins().smin(x, hi)
            };
            let s = b.f.ins().ushr_imm(span, 32);
            let e = b.f.ins().band_imm(span, 0xffff_ffff);
            let s = clamp(b, s);
            let e = clamp(b, e);
            let s = b.f.ins().ishl_imm(s, 32);
            b.f.ins().bor(s, e)
        })
    }
    /// An embedder's program's span `s..e` where it stands in its host: `SegmentMap::file_ranges(s..e).primary()`,
    /// read off its segments where they lie (`ViewDoc::segments`). The segment holding `s` (or the last before it)
    /// and those starting before `e` give one piece each, a copied one's overlapped part, a written one's whole
    /// anchor, pieces of no bytes left out; the span stands for the run of pieces, overlapping or touching, that holds
    /// the first one (`FileRanges::union`'s primary range), or, with none, for the point `s` maps to.
    fn placed_segments(&mut self, span: Value) -> Value {
        let segs = self.follow(self.tape, at::SEGMENTS);
        let n = self.word_at(I64, self.tape, at::SEGMENTS + 8);
        let n = self.f.ins().ireduce(I32, n);
        let s = self.f.ins().ushr_imm(span, 32);
        let s = self.f.ins().ireduce(I32, s);
        let e = self.f.ins().ireduce(I32, span);
        let none = self.f.ins().icmp_imm(IntCC::Equal, n, 0);
        self.either(I64, none, |_| span, |b| {
            let first = b.segments_partition(segs, n, |b, seg| {
                let end = b.word_at(I32, seg, 4);
                b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, end, s)
            });
            let last_one = b.f.ins().iadd_imm(n, -1);
            let first = b.f.ins().umin(first, last_one);
            let at = b.segment_point(segs, first, s);
            let point = b.pair(at, at);
            let empty = b.f.ins().icmp(IntCC::Equal, s, e);
            b.either(I64, empty, |_| point, |b| {
                let last = b.segments_partition(segs, n, |b, seg| {
                    let start = b.word_at(I32, seg, 0);
                    b.f.ins().icmp(IntCC::UnsignedLessThan, start, e)
                });
                b.segments_run(segs, [first, last], [s, e], point)
            })
        })
    }
    /// The first of the `n` segments at `segs` for which `pred` (of a segment's address) fails, where it holds for
    /// a prefix of them (`partition_point`).
    fn segments_partition(&mut self, segs: Value, n: Value, pred: impl Fn(&mut Self, Value) -> Value) -> Value {
        let (head, body, done) = (self.f.create_block(), self.f.create_block(), self.f.create_block());
        self.f.append_block_param(head, I32);
        self.f.append_block_param(head, I32);
        self.f.append_block_param(done, I32);
        let zero = self.c32(0);
        self.f.ins().jump(head, &[zero.into(), n.into()]);
        self.f.switch_to_block(head);
        let (lo, hi) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, lo, hi);
        self.f.ins().brif(more, body, &[], done, &[lo.into()]);
        self.f.switch_to_block(body);
        let sum = self.f.ins().iadd(lo, hi);
        let mid = self.f.ins().ushr_imm(sum, 1);
        let seg = self.item(segs, mid, 4 * SEGMENT_WORDS);
        let yes = pred(self, seg);
        let past = self.f.ins().iadd_imm(mid, 1);
        let lo = self.f.ins().select(yes, past, lo);
        let hi = self.f.ins().select(yes, hi, mid);
        self.f.ins().jump(head, &[lo.into(), hi.into()]);
        self.f.switch_to_block(done);
        self.f.block_params(done)[0]
    }
    /// The host position program byte `at` maps to in segment `i` (`SegmentMap::point`): its copy, kept within the
    /// segment, or the start of a written segment's anchor.
    fn segment_point(&mut self, segs: Value, i: Value, at_: Value) -> Value {
        let seg = self.item(segs, i, 4 * SEGMENT_WORDS);
        let (start, end) = (self.word_at(I32, seg, 0), self.word_at(I32, seg, 4));
        let (from, copied) = (self.word_at(I32, seg, 8), self.word_at(I32, seg, 16));
        let x = self.f.ins().umax(at_, start);
        let x = self.f.ins().umin(x, end);
        let x = self.f.ins().isub(x, start);
        let x = self.f.ins().iadd(from, x);
        let copied = self.f.ins().icmp_imm(IntCC::NotEqual, copied, 0);
        self.f.ins().select(copied, x, from)
    }
    /// The run of pieces holding the first one, of segments `first..last` over the program bytes `s..e`
    /// (`placed_segments`), as `start << 32 | end`; `point` where no segment gives a piece. Passes over the pieces
    /// grow the run by each one it overlaps or touches, until a pass grows it no more.
    fn segments_run(&mut self, segs: Value, [first, last]: [Value; 2], [s, e]: [Value; 2], point: Value) -> Value {
        let (head, body, after, done) = (self.f.create_block(), self.f.create_block(), self.f.create_block(), self.f.create_block());
        // `head`'s: the segment, the run's start and end, whether it has its first piece, whether this pass grew it.
        for _ in 0..5 {
            self.f.append_block_param(head, I32);
        }
        for _ in 0..4 {
            self.f.append_block_param(after, I32);
        }
        self.f.append_block_param(done, I64);
        let zero = self.c32(0);
        self.f.ins().jump(head, &[first.into(), zero.into(), zero.into(), zero.into(), zero.into()]);
        self.f.switch_to_block(head);
        let p = self.f.block_params(head).to_vec();
        let (i, cs, ce, have, grew) = (p[0], p[1], p[2], p[3], p[4]);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, i, last);
        self.f.ins().brif(more, body, &[], after, &[cs.into(), ce.into(), have.into(), grew.into()]);
        self.f.switch_to_block(body);
        let seg = self.item(segs, i, 4 * SEGMENT_WORDS);
        let (start, end) = (self.word_at(I32, seg, 0), self.word_at(I32, seg, 4));
        let (from, to, copied) = (self.word_at(I32, seg, 8), self.word_at(I32, seg, 12), self.word_at(I32, seg, 16));
        let (ps, pe) = (self.segment_point(segs, i, s), self.segment_point(segs, i, e));
        let copied = self.f.ins().icmp_imm(IntCC::NotEqual, copied, 0);
        let ps = self.f.ins().select(copied, ps, from);
        let pe = self.f.ins().select(copied, pe, to);
        let wide = self.f.ins().icmp(IntCC::NotEqual, start, end);
        let some = self.f.ins().icmp(IntCC::UnsignedLessThan, ps, pe);
        let piece = self.f.ins().band(wide, some);
        let has = self.f.ins().icmp_imm(IntCC::NotEqual, have, 0);
        let not_has = self.f.ins().icmp_imm(IntCC::Equal, have, 0);
        let first_piece = self.f.ins().band(piece, not_has);
        let before_end = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, ps, ce);
        let after_start = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, cs, pe);
        let touches = self.f.ins().band(before_end, after_start);
        let below = self.f.ins().icmp(IntCC::UnsignedLessThan, ps, cs);
        let above = self.f.ins().icmp(IntCC::UnsignedGreaterThan, pe, ce);
        let outside = self.f.ins().bor(below, above);
        let grows = self.f.ins().band(piece, has);
        let grows = self.f.ins().band(grows, touches);
        let grows = self.f.ins().band(grows, outside);
        let lo = self.f.ins().umin(cs, ps);
        let hi = self.f.ins().umax(ce, pe);
        let cs2 = self.f.ins().select(grows, lo, cs);
        let ce2 = self.f.ins().select(grows, hi, ce);
        let cs2 = self.f.ins().select(first_piece, ps, cs2);
        let ce2 = self.f.ins().select(first_piece, pe, ce2);
        let one = self.c32(1);
        let have2 = self.f.ins().select(first_piece, one, have);
        let grew2 = self.f.ins().select(grows, one, grew);
        let next = self.f.ins().iadd_imm(i, 1);
        self.f.ins().jump(head, &[next.into(), cs2.into(), ce2.into(), have2.into(), grew2.into()]);
        // A pass's end: another while it grew the run.
        self.f.switch_to_block(after);
        let p = self.f.block_params(after).to_vec();
        let (cs, ce, have, grew) = (p[0], p[1], p[2], p[3]);
        let again = self.f.ins().icmp_imm(IntCC::NotEqual, grew, 0);
        let (fin, zero) = (self.f.create_block(), self.c32(0));
        self.f.ins().brif(again, head, &[first.into(), cs.into(), ce.into(), have.into(), zero.into()], fin, &[]);
        self.f.switch_to_block(fin);
        let run = self.pair(cs, ce);
        let has = self.f.ins().icmp_imm(IntCC::NotEqual, have, 0);
        let got = self.f.ins().select(has, run, point);
        self.f.ins().jump(done, &[got.into()]);
        self.f.switch_to_block(done);
        self.f.block_params(done)[0]
    }
    /// `s << 32 | e`, of u32s.
    fn pair(&mut self, s: Value, e: Value) -> Value {
        let s = self.f.ins().uextend(I64, s);
        let s = self.f.ins().ishl_imm(s, 32);
        let e = self.f.ins().uextend(I64, e);
        self.f.ins().bor(s, e)
    }
    /// The 1-based line holding byte `at` (`LineTable::line` + 1: the `line` view's read, `lines_line`).
    fn loc_line(&mut self, at_: Value) -> Value {
        self.call_lines("lines_line", &[at_])
    }
    /// A `Loc` view's row: the id, then its place (`Viewed::loc_value`, `LineTable::place`).
    fn loc(&mut self, place: Place) -> Value {
        let (l, col) = (self.arg(0), self.arg(1));
        let c = self.f.ins().icmp_imm(IntCC::Equal, col, 0);
        self.either(I64, c, |b| b.loc_id(l), |b| {
            if place == Place::Docof {
                return b.entry_u32(at::FILE);
            }
            // Placed by its table, else its span as read, placed (`Placement::host`).
            let placement = b.word_at(I32, b.tape, at::PLACEMENT);
            let tabled = b.f.ins().icmp_imm(IntCC::Equal, placement, i64::from(PLACED_TABLE));
            let span = b.either(I64, tabled, |b| {
                let items = b.follow(b.tape, at::PLACED);
                let p = b.item(items, l, 8);
                b.word_at(I64, p, 0)
            }, |b| {
                let span = b.loc_span(l);
                b.placed(span)
            });
            let s = b.f.ins().ushr_imm(span, 32);
            let s = b.f.ins().ireduce(I32, s);
            let e = b.f.ins().ireduce(I32, span);
            let line = b.loc_line(s);
            let v = match place {
                Place::Line => line,
                // The line holding its end offset: a span ending with a terminator ends on the line after it.
                Place::EndLine => b.loc_line(e),
                Place::Col => {
                    // Its line's start (`line` from 1).
                    let k = b.f.ins().iadd_imm(line, -1);
                    let start = b.loc_start(k);
                    b.f.ins().isub(s, start)
                }
                Place::Docof => unreachable!(),
            };
            b.f.ins().uextend(I64, v)
        })
    }
}

/// `view_loc_line(l, col)`, `view_loc_end`, `view_loc_col`, `view_loc_docof`: column `col` of core.rerail's `line`,
/// `end_line`, `col` and `docof` rows of position `l` (a node, a token, the whole text, a line, an end point).
#[cfg(feature = "ir")]
pub fn loc_line(b: &mut B<'_, '_>) -> Value {
    b.loc(Place::Line)
}
#[cfg(feature = "ir")]
pub fn loc_end(b: &mut B<'_, '_>) -> Value {
    b.loc(Place::EndLine)
}
#[cfg(feature = "ir")]
pub fn loc_col(b: &mut B<'_, '_>) -> Value {
    b.loc(Place::Col)
}
#[cfg(feature = "ir")]
pub fn loc_docof(b: &mut B<'_, '_>) -> Value {
    b.loc(Place::Docof)
}

#[cfg(feature = "ir")]
mod dir_at {
    use super::{ViewDir, ViewRow};
    use std::mem::{offset_of, size_of};
    pub const AT: i32 = offset_of!(ViewDir, at) as i32;
    pub const NDOCS: i32 = offset_of!(ViewDir, ndocs) as i32;
    pub const ROWS: i32 = offset_of!(ViewDir, rows) as i32;
    pub const NBATCH: i32 = offset_of!(ViewDir, nbatch) as i32;
    pub const KEYS: i32 = offset_of!(ViewDir, keys) as i32;
    pub const PAGES: i32 = offset_of!(ViewDir, pages) as i32;
    pub const ORDER: i32 = offset_of!(ViewDir, order) as i32;
    pub const FIRST: i32 = offset_of!(ViewDir, first) as i32;
    pub const SPAN: i32 = offset_of!(ViewDir, span) as i32;
    pub const TABLE_HEADER: i64 = size_of::<super::IdPages>() as i64;
    pub const PAGE_SIZE: i64 = size_of::<super::Page>() as i64;
    pub const PAGE_NUMBER: i32 = offset_of!(super::Page, number) as i32;
    pub const PAGE_LOCAL: i32 = offset_of!(super::Page, local) as i32;
    pub const ROW_SIZE: i64 = size_of::<ViewRow>() as i64;
    pub const ROW_ROWS: i32 = offset_of!(ViewRow, rows) as i32;
    pub const ROW_KEY: i32 = offset_of!(ViewRow, key) as i32;
    pub const ROW_AT: i32 = offset_of!(ViewRow, at) as i32;
}

#[cfg(feature = "ir")]
impl B<'_, '_> {
    /// The directory's rows (`self.tape` is the directory, in its block): their address, and how many.
    fn dir_rows(&mut self) -> (Value, Value) {
        let at = self.word_at(I64, self.tape, dir_at::AT);
        let block = self.f.ins().isub(self.tape, at);
        let rows = self.word_at(I64, self.tape, dir_at::ROWS);
        let rows = self.f.ins().iadd(block, rows);
        let n = self.word_at(I32, self.tape, dir_at::NDOCS);
        (rows, n)
    }
    /// Row `m`'s first row of view `k`.
    fn dir_first(&mut self, rows: Value, m: Value, k: Value) -> Value {
        let row = self.item(rows, m, dir_at::ROW_SIZE);
        let p = self.item(row, k, 4);
        self.word_at(I32, p, dir_at::ROW_ROWS)
    }
    /// The directory's order (`order`): its address.
    fn dir_view_order(&mut self) -> Value {
        let at = self.word_at(I64, self.tape, dir_at::AT);
        let block = self.f.ins().isub(self.tape, at);
        let order = self.word_at(I64, self.tape, dir_at::ORDER);
        self.f.ins().iadd(block, order)
    }
    /// Row `m`'s place in the directory's order (`ViewRow::at`).
    fn dir_place(&mut self, rows: Value, m: Value) -> Value {
        let row = self.item(rows, m, dir_at::ROW_SIZE);
        self.word_at(I32, row, dir_at::ROW_AT)
    }
    /// The documents' keys in row order (`ViewDir::keys`): their address.
    fn dir_keys(&mut self) -> Value {
        let at = self.word_at(I64, self.tape, dir_at::AT);
        let block = self.f.ins().isub(self.tape, at);
        let keys = self.word_at(I64, self.tape, dir_at::KEYS);
        self.f.ins().iadd(block, keys)
    }
    /// The row `i`th in the directory's order (`order`).
    fn dir_row_in(&mut self, order: Value, i: Value) -> Value {
        let p = self.item(order, i, 4);
        self.word_at(I32, p, 0)
    }
    /// Document `i`'s key (`ViewDir::keys`).
    fn key_at(&mut self, keys: Value, i: Value) -> Value {
        let p = self.item(keys, i, 4);
        self.word_at(I32, p, 0)
    }
    /// `lower_bound` of `key` among the `n` ascending keys at `keys`: a fixed count of steps, `n` alone sets, each a
    /// select.
    fn lower_bound(&mut self, keys: Value, n: Value, key: Value) -> Value {
        let (head, body, last, done) = (self.f.create_block(), self.f.create_block(), self.f.create_block(), self.f.create_block());
        self.f.append_block_param(head, I32);
        self.f.append_block_param(head, I32);
        self.f.append_block_param(last, I32);
        self.f.append_block_param(done, I32);
        let zero = self.c32(0);
        self.f.ins().jump(head, &[zero.into(), n.into()]);
        self.f.switch_to_block(head);
        let (base, len) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, len, 1);
        self.f.ins().brif(more, body, &[], last, &[base.into()]);
        self.f.switch_to_block(body);
        let half = self.f.ins().ushr_imm(len, 1);
        let at = self.f.ins().iadd(base, half);
        let probe = self.f.ins().iadd_imm(at, -1);
        let k = self.key_at(keys, probe);
        let below = self.f.ins().icmp(IntCC::UnsignedLessThan, k, key);
        let base = self.f.ins().select(below, at, base);
        let len = self.f.ins().isub(len, half);
        self.f.ins().jump(head, &[base.into(), len.into()]);
        // The last span: one key, or none (`n` 0).
        self.f.switch_to_block(last);
        let base = self.f.block_params(last)[0];
        let none = self.f.ins().icmp_imm(IntCC::Equal, n, 0);
        let c = self.either(I32, none, |b| b.c32(0), |b| {
            let k = b.key_at(keys, base);
            let below = b.f.ins().icmp(IntCC::UnsignedLessThan, k, key);
            let one = b.f.ins().uextend(I32, below);
            b.f.ins().iadd(base, one)
        });
        self.f.ins().jump(done, &[c.into()]);
        self.f.switch_to_block(done);
        self.f.block_params(done)[0]
    }
    /// The first of documents `from..n` whose key is `key`, or `NO_DOC`: every one read, each a select.
    fn scan_keys(&mut self, keys: Value, from: Value, n: Value, key: Value) -> Value {
        let (head, body, done) = (self.f.create_block(), self.f.create_block(), self.f.create_block());
        self.f.append_block_param(head, I32);
        self.f.append_block_param(head, I32);
        self.f.append_block_param(done, I32);
        let no = self.c32(i64::from(NO_DOC));
        self.f.ins().jump(head, &[n.into(), no.into()]);
        self.f.switch_to_block(head);
        let (i, found) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp(IntCC::UnsignedGreaterThan, i, from);
        self.f.ins().brif(more, body, &[], done, &[found.into()]);
        // From the last down, so the first that is `key` is kept.
        self.f.switch_to_block(body);
        let i = self.f.ins().iadd_imm(i, -1);
        let k = self.key_at(keys, i);
        let same = self.f.ins().icmp(IntCC::Equal, k, key);
        let found = self.f.ins().select(same, i, found);
        self.f.ins().jump(head, &[i.into(), found.into()]);
        self.f.switch_to_block(done);
        self.f.block_params(done)[0]
    }
    /// Id `id`'s page in the directory's page table: its address.
    fn dir_page(&mut self, id: Value) -> Value {
        let at = self.word_at(I64, self.tape, dir_at::AT);
        let block = self.f.ins().isub(self.tape, at);
        let table = self.word_at(I64, self.tape, dir_at::PAGES);
        let table = self.f.ins().iadd(block, table);
        let pages = self.f.ins().iadd_imm(table, dir_at::TABLE_HEADER);
        let page = self.f.ins().ushr_imm(id, i64::from(PAGE_BITS));
        self.item(pages, page, dir_at::PAGE_SIZE)
    }
}

/// `dir_doc_of_row(k, r, memo)`: the document whose rows of view `k` hold row `r` (`ViewDir::doc_of_row`), memo
/// `memo` tried first.
#[cfg(feature = "ir")]
pub fn dir_doc_of_row(b: &mut B<'_, '_>) -> Value {
    let (k, r, memo) = (b.arg(0), b.arg(1), b.arg(2));
    let (rows, n) = b.dir_rows();
    let order = b.dir_view_order();
    let span = b.word_at(I32, b.tape, dir_at::SPAN);
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, memo, span);
    let held = b.either(I32, inside, |b| {
        let at = b.dir_place(rows, memo);
        let placed = b.f.ins().icmp(IntCC::UnsignedLessThan, at, n);
        b.either(I32, placed, |b| {
            let first = b.dir_first(rows, memo, k);
            let from = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, first, r);
            let next = b.f.ins().iadd_imm(at, 1);
            let last = b.f.ins().icmp(IntCC::Equal, next, n);
            let before = b.either(I32, last, |b| b.c32(1), |b| {
                let m = b.dir_row_in(order, next);
                let after = b.dir_first(rows, m, k);
                let past = b.f.ins().icmp(IntCC::UnsignedGreaterThan, after, r);
                b.f.ins().uextend(I32, past)
            });
            let from = b.f.ins().uextend(I32, from);
            b.f.ins().band(from, before)
        }, |b| b.c32(0))
    }, |b| b.c32(0));
    let hit = b.f.ins().icmp_imm(IntCC::NotEqual, held, 0);
    b.either(I32, hit, |_| memo, |b| {
        let none = b.f.ins().icmp_imm(IntCC::Equal, n, 0);
        b.either(I32, none, |b| b.c32(0), |b| {
            let c = b.bisect(n, &mut |b, m| {
                let row = b.dir_row_in(order, m);
                let first = b.dir_first(rows, row, k);
                b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, first, r)
            });
            let one = b.c32(1);
            let c = b.f.ins().umax(c, one);
            let c = b.f.ins().isub(c, one);
            b.dir_row_in(order, c)
        })
    })
}

/// `dir_doc_of_key(key, memo)`: the row of the document of key `key` (`ViewDir::doc_of_key`: the batch's by a lower
/// bound, the values' by a scan, in the directory's order, `keys`, its row read there), or `NO_DOC`, row `memo` tried
/// first.
#[cfg(feature = "ir")]
pub fn dir_doc_of_key(b: &mut B<'_, '_>) -> Value {
    let (key, memo) = (b.arg(0), b.arg(1));
    let (rows, n) = b.dir_rows();
    let keys = b.dir_keys();
    let order = b.dir_view_order();
    let nbatch = b.word_at(I32, b.tape, dir_at::NBATCH);
    let nbatch = b.f.ins().umin(nbatch, n);
    let span = b.word_at(I32, b.tape, dir_at::SPAN);
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, memo, span);
    let held = b.either(I32, inside, |b| {
        let row = b.item(rows, memo, dir_at::ROW_SIZE);
        let k = b.word_at(I32, row, dir_at::ROW_KEY);
        let same = b.f.ins().icmp(IntCC::Equal, k, key);
        b.f.ins().uextend(I32, same)
    }, |b| b.c32(0));
    let hit = b.f.ins().icmp_imm(IntCC::NotEqual, held, 0);
    b.either(I32, hit, |_| memo, |b| {
        // The batch's documents, in key order; then the values', in the order admitted.
        let c = b.lower_bound(keys, nbatch, key);
        let within = b.f.ins().icmp(IntCC::UnsignedLessThan, c, nbatch);
        let at = b.either(I32, within, |b| b.key_at(keys, c), |b| b.c32(0));
        let same = b.f.ins().icmp(IntCC::Equal, at, key);
        let found = b.f.ins().band(within, same);
        // The values' scan, whatever the batch's search found (none with no values): a select, not a branch.
        let scanned = b.scan_keys(keys, nbatch, n, key);
        let m = b.f.ins().select(found, c, scanned);
        let none = b.f.ins().icmp_imm(IntCC::Equal, m, i64::from(NO_DOC));
        b.either(I32, none, |b| b.c32(i64::from(NO_DOC)), |b| b.dir_row_in(order, m))
    })
}

/// `dir_doc_of_id(id)`: the row of id `id`'s document (`ViewDir::doc_of_id`): its page's number less the
/// directory's first, or `NO_DOC` past its rows. One load: a row is a number in every directory of the growth.
#[cfg(feature = "ir")]
pub fn dir_doc_of_id(b: &mut B<'_, '_>) -> Value {
    let id = b.arg(0);
    let page = b.dir_page(id);
    let number = b.word_at(I32, page, dir_at::PAGE_NUMBER);
    let first = b.word_at(I32, b.tape, dir_at::FIRST);
    let span = b.word_at(I32, b.tape, dir_at::SPAN);
    let row = b.f.ins().isub(number, first);
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, row, span);
    let no = b.c32(i64::from(NO_DOC));
    b.f.ins().select(inside, row, no)
}

/// `dir_local_of_id(id)`: id `id`'s local id in its document (`ViewDir::local_of_id`): its page's first, and its
/// place in the page.
#[cfg(feature = "ir")]
pub fn dir_local_of_id(b: &mut B<'_, '_>) -> Value {
    let id = b.arg(0);
    let page = b.dir_page(id);
    let first = b.word_at(I32, page, dir_at::PAGE_LOCAL);
    let within = b.f.ins().band_imm(id, i64::from(PAGE - 1));
    b.f.ins().iadd(first, within)
}


#[cfg(feature = "ir")]
impl B<'_, '_> {
    /// The `child` row of the first numbered node from tape node `c` on, `c` and each sibling after it within
    /// `end` (`tape_next_in`), or `NO_ROW`: numbered node `l`'s row is `l - 1`. The loop ends: each sibling is past
    /// the last, and `tape_next_in` gives none at `end` (decision 0463's reads stay within the tape).
    fn numbered_from(&mut self, c: Value, end: Value) -> Value {
        let (head, test, step, found, done) = (self.f.create_block(), self.f.create_block(), self.f.create_block(), self.f.create_block(), self.f.create_block());
        self.f.append_block_param(head, I32);
        self.f.append_block_param(done, I32);
        self.f.ins().jump(head, &[c.into()]);
        self.f.switch_to_block(head);
        let c = self.f.block_params(head)[0];
        let none = self.f.ins().icmp_imm(IntCC::Equal, c, i64::from(NO_ROW));
        let no = self.c32(i64::from(NO_ROW));
        self.f.ins().brif(none, done, &[no.into()], test, &[]);
        self.f.switch_to_block(test);
        let numbering = self.follow(self.tape, at::NUMBERING);
        let node = self.c32(0);
        let is = self.call_at("num_is", numbering, &[node, c]);
        let numbered = self.f.ins().icmp_imm(IntCC::NotEqual, is, 0);
        self.f.ins().brif(numbered, found, &[], step, &[]);
        self.f.switch_to_block(found);
        let numbering = self.follow(self.tape, at::NUMBERING);
        let node = self.c32(0);
        let l = self.call_at("num_rank", numbering, &[node, c]);
        let row = self.f.ins().iadd_imm(l, -1);
        self.f.ins().jump(done, &[row.into()]);
        self.f.switch_to_block(step);
        let next = self.call_tape("tape_next_in", &[c, end]);
        self.f.ins().jump(head, &[next.into()]);
        self.f.switch_to_block(done);
        self.f.block_params(done)[0]
    }
}

/// `view_child_first(l)`: the `child` row of numbered node `l`'s first child (its first numbered tape child), or
/// `NO_ROW` (decision 0353 step 4c: a probe of `child` by its parent, off the tape, with no index of the view).
/// For a node other than the root: a numbered node's parent is its tape parent where that is numbered, so its
/// children are its numbered tape children. The root's are also the nodes under unnumbered ones (a narrowed
/// projection): a probe of the root scans its document's rows instead.
#[cfg(feature = "ir")]
pub fn child_first(b: &mut B<'_, '_>) -> Value {
    let l = b.arg(0);
    let x = b.tape_node(l);
    let end = b.call_tape("tape_end_of", &[x]);
    let c = b.call_tape("tape_first_child", &[x]);
    b.numbered_from(c, end)
}

/// `view_child_next(i)`: the `child` row of the next child of the parent of `child` row `i` (numbered node `i +
/// 1`'s next numbered tape sibling), or `NO_ROW`. For a child of a parent other than the root (`view_child_first`).
#[cfg(feature = "ir")]
pub fn child_next(b: &mut B<'_, '_>) -> Value {
    let i = b.arg(0);
    let l = b.f.ins().iadd_imm(i, 1);
    let x = b.tape_node(l);
    let p = b.call_tape("tape_parent", &[x]);
    let root = b.f.ins().icmp_imm(IntCC::Equal, p, i64::from(NO_ROW));
    b.either(I32, root, |b| b.c32(i64::from(NO_ROW)), |b| {
        let end = b.call_tape("tape_end_of", &[p]);
        let c = b.call_tape("tape_next_in", &[x, end]);
        b.numbered_from(c, end)
    })
}

#[cfg(feature = "ir")]
/// `view_up(l)`: numbered node `l`'s nearest numbered strict tape ancestor, or `NO_ROW` (decision 0353 step 5: `has`
/// probed by its node, a climb of the tape's parents, kept where the projection numbers them, as the host's
/// `Within::containers_on`). The loop ends: each parent is above the last, and the root's is `NO_ROW`.
#[cfg(feature = "ir")]
pub fn up(b: &mut B<'_, '_>) -> Value {
    let l = b.arg(0);
    let x = b.tape_node(l);
    let first = b.call_tape("tape_parent", &[x]);
    let (head, test, step, found, done) = (b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block(), b.f.create_block());
    b.f.append_block_param(head, I32);
    b.f.append_block_param(done, I32);
    b.f.ins().jump(head, &[first.into()]);
    b.f.switch_to_block(head);
    let p = b.f.block_params(head)[0];
    let none = b.f.ins().icmp_imm(IntCC::Equal, p, i64::from(NO_ROW));
    let no = b.c32(i64::from(NO_ROW));
    b.f.ins().brif(none, done, &[no.into()], test, &[]);
    b.f.switch_to_block(test);
    let numbering = b.follow(b.tape, at::NUMBERING);
    let node = b.c32(0);
    let is = b.call_at("num_is", numbering, &[node, p]);
    let numbered = b.f.ins().icmp_imm(IntCC::NotEqual, is, 0);
    b.f.ins().brif(numbered, found, &[], step, &[]);
    b.f.switch_to_block(found);
    let numbering = b.follow(b.tape, at::NUMBERING);
    let node = b.c32(0);
    let u = b.call_at("num_rank", numbering, &[node, p]);
    b.f.ins().jump(done, &[u.into()]);
    b.f.switch_to_block(step);
    let next = b.call_tape("tape_parent", &[p]);
    b.f.ins().jump(head, &[next.into()]);
    b.f.switch_to_block(done);
    b.f.block_params(done)[0]
}

#[cfg(feature = "ir")]
/// `view_end(l)`: the number past numbered node `l`'s numbered descendants, which are `l + 1 .. view_end(l)` (the
/// numbered nodes before its subtree's end on the tape; decision 0353 step 5: `has` probed by its ancestor, as the
/// host's `Within::contained_on`).
#[cfg(feature = "ir")]
pub fn end(b: &mut B<'_, '_>) -> Value {
    let l = b.arg(0);
    let x = b.tape_node(l);
    let end = b.call_tape("tape_end_of", &[x]);
    let numbering = b.follow(b.tape, at::NUMBERING);
    let node = b.c32(0);
    b.call_at("num_rank", numbering, &[node, end])
}

/// `view_text_span(l)`: numbered node `l`'s text as its bytes' range in its document's own source, `start << 32 | end`
/// (`tape_start`, `tape_end`: a guest's in its own program, not where its host places it). The text is the source's
/// bytes there (`ViewRow::source`), read where they lie: the host's `Projected` text (`long_text`, `text_any`), and on
/// the device a text function's operand (decision 0287 §2).
#[cfg(feature = "ir")]
pub fn text_span(b: &mut B<'_, '_>) -> Value {
    let l = b.arg(0);
    let x = b.tape_node(l);
    let start = b.call_tape("tape_start", &[x]);
    let end = b.call_tape("tape_end", &[x]);
    let start = b.f.ins().uextend(I64, start);
    let high = b.f.ins().ishl_imm(start, 32);
    let end = b.f.ins().uextend(I64, end);
    b.f.ins().bor(high, end)
}

/// `view_line_span(low)`: the line whose span's id has low bits `low` (`LINE_SLOT << INDEX_BITS | l`: a `src_line`
/// span, from 1; 0 the document's `source` span), as its bytes in its document's own source, `start << 32 | stop`:
/// from its first byte (`lines_start`) to one past its terminator's last (the next line's start), or to the source's
/// end on its last line and on line 0. Its text ends `lines_terminator_length(stop - 1)` bytes before `stop` where the
/// line has a terminator (`0 < l < lines_count`), read off the source's bytes (`Doc::line_text`, as
/// `LineTable::text_of`: decision 0353 row 7). `NO_SPAN` (all ones) for another id, a line past the last, or a
/// document whose lines are its host's (a guest's, `own_text` 0).
#[cfg(feature = "ir")]
pub fn line_span(b: &mut B<'_, '_>) -> Value {
    let low = b.arg(0);
    let slot = b.f.ins().ushr_imm(low, i64::from(crate::ids::INDEX_BITS));
    let other = b.f.ins().icmp_imm(IntCC::NotEqual, slot, i64::from(crate::ids::LINE_SLOT));
    let l = b.f.ins().band_imm(low, i64::from((1u32 << crate::ids::INDEX_BITS) - 1));
    let own = b.word_at(I32, b.tape, at::OWN_TEXT);
    let count = b.loc_count();
    let past = b.f.ins().icmp(IntCC::UnsignedGreaterThan, l, count);
    let guest = b.f.ins().icmp_imm(IntCC::Equal, own, 0);
    let refused = b.f.ins().bor(guest, past);
    let refused = b.f.ins().bor(refused, other);
    let unread = b.f.ins().iconst(I64, -1);
    b.either(I64, refused, |_| unread, |b| {
        let len = b.word_at(I32, b.tape, at::SOURCE_LEN);
        let whole = b.f.ins().icmp_imm(IntCC::Equal, l, 0);
        b.either(I64, whole, |b| {
            let zero = b.c32(0);
            b.pair(zero, len)
        }, |b| {
            let k = b.f.ins().iadd_imm(l, -1);
            let start = b.loc_start(k);
            let count = b.loc_count();
            let last = b.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, l, count);
            let stop = b.either(I32, last, |_| len, |b| b.loc_start(l));
            b.pair(start, stop)
        })
    })
}

/// `view_line_terminated(low)`: 1 where the line whose span's id has low bits `low` (`view_line_span`'s) ends with a
/// terminator, so its text ends `lines_terminator_length` bytes before its span's stop: a line from 1 and before the
/// last (`l < lines_count`). 0 for line 0, the last line, and another id.
#[cfg(feature = "ir")]
pub fn line_terminated(b: &mut B<'_, '_>) -> Value {
    let low = b.arg(0);
    let slot = b.f.ins().ushr_imm(low, i64::from(crate::ids::INDEX_BITS));
    let line = b.f.ins().icmp_imm(IntCC::Equal, slot, i64::from(crate::ids::LINE_SLOT));
    let l = b.f.ins().band_imm(low, i64::from((1u32 << crate::ids::INDEX_BITS) - 1));
    let count = b.loc_count();
    let some = b.f.ins().icmp_imm(IntCC::NotEqual, l, 0);
    let before = b.f.ins().icmp(IntCC::UnsignedLessThan, l, count);
    let yes = b.f.ins().band(line, some);
    let yes = b.f.ins().band(yes, before);
    b.f.ins().uextend(I32, yes)
}

/// `view_number(low)`: the numbered node whose id's low 32 bits are `low` (its document's: the key is the caller's
/// to match), or `NO_ROW`: `low` itself where ids are local ids, else the numbering's inverse (`num_number_of`,
/// a node's, not a token's).
#[cfg(feature = "ir")]
pub fn number(b: &mut B<'_, '_>) -> Value {
    let low = b.arg(0);
    let preorder = b.word_at(I32, b.tape, at::PREORDER);
    let local = b.f.ins().icmp_imm(IntCC::Equal, preorder, 1);
    let n = b.word_at(I32, b.tape, at::NODES);
    let tape = b.ids_tape();
    let l = b.either(I32, local, |_| low, |b| b.either(I32, tape, |b| {
        // A tape position: the node there, if it is numbered (`num_is`), is its rank (`num_rank`).
        let numbering = b.follow(b.tape, at::NUMBERING);
        let item = b.c32(0);
        let is = b.call_at("num_is", numbering, &[item, low]);
        let numbered = b.f.ins().icmp_imm(IntCC::NotEqual, is, 0);
        let numbering = b.follow(b.tape, at::NUMBERING);
        let item = b.c32(0);
        let rank = b.call_at("num_rank", numbering, &[item, low]);
        let no = b.c32(i64::from(NO_ROW));
        b.f.ins().select(numbered, rank, no)
    }, |b| {
        let numbering = b.follow(b.tape, at::NUMBERING);
        let found = b.call_at("num_number_of", numbering, &[low]);
        // A node's: item 0 in the high word.
        let item = b.f.ins().ushr_imm(found, 32);
        let node = b.f.ins().icmp_imm(IntCC::Equal, item, 0);
        let number = b.f.ins().ireduce(I32, found);
        let no = b.c32(i64::from(NO_ROW));
        b.f.ins().select(node, number, no)
    }));
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, l, n);
    let no = b.c32(i64::from(NO_ROW));
    b.f.ins().select(inside, l, no)
}
#[cfg(feature = "ir")]
impl B<'_, '_> {
    /// Numbered node `l`'s depth, the root's 0: its numbered strict tape ancestors, by a climb of the tape's parents
    /// (`Projected::depth`). The loop ends: each parent is above the last, and the root's is `NO_ROW`.
    fn depth_of(&mut self, l: Value) -> Value {
        let x = self.tape_node(l);
        let first = self.call_tape("tape_parent", &[x]);
        let (head, test, done) = (self.f.create_block(), self.f.create_block(), self.f.create_block());
        self.f.append_block_param(head, I32);
        self.f.append_block_param(head, I32);
        self.f.append_block_param(done, I32);
        let zero = self.c32(0);
        self.f.ins().jump(head, &[first.into(), zero.into()]);
        self.f.switch_to_block(head);
        let (p, d) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let none = self.f.ins().icmp_imm(IntCC::Equal, p, i64::from(NO_ROW));
        self.f.ins().brif(none, done, &[d.into()], test, &[]);
        self.f.switch_to_block(test);
        let numbering = self.follow(self.tape, at::NUMBERING);
        let node = self.c32(0);
        let is = self.call_at("num_is", numbering, &[node, p]);
        let one = self.f.ins().icmp_imm(IntCC::NotEqual, is, 0);
        let one = self.f.ins().uextend(I32, one);
        let d = self.f.ins().iadd(d, one);
        let next = self.call_tape("tape_parent", &[p]);
        self.f.ins().jump(head, &[next.into(), d.into()]);
        self.f.switch_to_block(done);
        let d = self.f.block_params(done)[0];
        self.f.ins().uextend(I64, d)
    }
}

/// `view_tape(l, col)`: column `col` of the `tape` view's row of numbered node `l`: its id, its parent's id (-1 for
/// the root), the numbered nodes under it (`Projected::size`: `view_end`'s past its own), its depth.
#[cfg(feature = "ir")]
pub fn tape(b: &mut B<'_, '_>) -> Value {
    let (l, col) = (b.arg(0), b.arg(1));
    let c = b.f.ins().icmp_imm(IntCC::Equal, col, 0);
    b.either(I64, c, |b| b.node_id(l), |b| {
        let c = b.f.ins().icmp_imm(IntCC::Equal, col, 1);
        b.either(I64, c, |b| {
            let x = b.tape_node(l);
            b.parent_id(l, x)
        }, |b| {
            let c = b.f.ins().icmp_imm(IntCC::Equal, col, 2);
            b.either(I64, c, |b| {
                let x = b.tape_node(l);
                let end = b.call_tape("tape_end_of", &[x]);
                let numbering = b.follow(b.tape, at::NUMBERING);
                let node = b.c32(0);
                let past = b.call_at("num_rank", numbering, &[node, end]);
                let under = b.f.ins().isub(past, l);
                let under = b.f.ins().iadd_imm(under, -1);
                b.f.ins().uextend(I64, under)
            }, |b| b.depth_of(l))
        })
    })
}

/// `view_depth(l, col)`: column `col` of `ast_depth`'s row of numbered node `l`: its id, its depth.
#[cfg(feature = "ir")]
pub fn depth(b: &mut B<'_, '_>) -> Value {
    let (l, col) = (b.arg(0), b.arg(1));
    let c = b.f.ins().icmp_imm(IntCC::Equal, col, 0);
    b.either(I64, c, |b| b.node_id(l), |b| b.depth_of(l))
}

/// `view_order(l, col)`: column `col` of `node_order`'s row of numbered node `l`: its id, its order key under the
/// document's key (`Projected::order`: `num_order_of`, its number itself where ids are local ids).
#[cfg(feature = "ir")]
pub fn order(b: &mut B<'_, '_>) -> Value {
    let (l, col) = (b.arg(0), b.arg(1));
    let c = b.f.ins().icmp_imm(IntCC::Equal, col, 0);
    b.either(I64, c, |b| b.node_id(l), |b| {
        let key = b.entry_u32(at::KEY);
        let high = b.f.ins().ishl_imm(key, 32);
        let preorder = b.word_at(I32, b.tape, at::PREORDER);
        let local = b.f.ins().icmp_imm(IntCC::Equal, preorder, 1);
        let low = b.either(I32, local, |_| l, |b| {
            // Where ids are tape positions, the order key is the tape node: the tape is in preorder.
            let tape = b.ids_tape();
            b.either(I32, tape, |b| b.tape_node(l), |b| {
                let numbering = b.follow(b.tape, at::NUMBERING);
                b.call_at("num_order_of", numbering, &[l])
            })
        });
        let low = b.f.ins().uextend(I64, low);
        b.f.ins().bor(high, low)
    })
}

/// `view_op(i, col)`: column `col` of the `op` view's row `i`: the document's `i`-th anonymous child of a numbered node
/// (`Item::Op`, `num_select`; one of any width, a zero-width one the tape stores too), its node's id, and its name
/// atom (by its tape code, `ViewDoc::names`). Each column's branch finds the child itself, as `field`'s do (G13).
#[cfg(feature = "ir")]
pub fn op(b: &mut B<'_, '_>) -> Value {
    let (i, col) = (b.arg(0), b.arg(1));
    let child = |b: &mut B<'_, '_>| {
        let numbering = b.follow(b.tape, at::NUMBERING);
        let op = b.c32(i64::from(crate::ids::OP_ITEM));
        b.call_at("num_select", numbering, &[op, i])
    };
    let c = b.f.ins().icmp_imm(IntCC::Equal, col, 0);
    b.either(I64, c, |b| {
        let x = child(b);
        // An anonymous child is no root (`parent_id`'s first argument is not 0): its numbered node's id.
        let one = b.c32(1);
        b.parent_id(one, x)
    }, |b| {
        let x = child(b);
        let code = b.call_tape("tape_number", &[x]);
        b.entry_item(at::NAMES, code)
    })
}

/// `view_doc(l, col)`: column `col` of `doc`'s row (one a file, `l` 0): the document's key, its path's atom, its
/// module's (`ViewDoc::doc_path`, `doc_module`: interned where its program reads `doc`, `tape_rows::derive`).
#[cfg(feature = "ir")]
pub fn doc(b: &mut B<'_, '_>) -> Value {
    let col = b.arg(1);
    let k = b.f.ins().icmp_imm(IntCC::Equal, col, 0);
    let p = b.f.ins().icmp_imm(IntCC::Equal, col, 1);
    b.either(I64, k, |b| b.entry_u32(at::KEY), |b| b.either(I64, p, |b| b.entry_u32(at::DOC_PATH), |b| b.entry_u32(at::DOC_MODULE)))
}

/// `view_visit(l, col)`: column `col` of `visit_index`'s row of numbered node `l`: its id, its place in the visit
/// order (`ViewDoc::visits`, by local id: derived where its program reads `visit_index`, `tape_rows::derive`).
#[cfg(feature = "ir")]
pub fn visit(b: &mut B<'_, '_>) -> Value {
    let (l, col) = (b.arg(0), b.arg(1));
    let c = b.f.ins().icmp_imm(IntCC::Equal, col, 0);
    b.either(I64, c, |b| b.node_id(l), |b| b.entry_item(at::VISITS, l))
}

#[cfg(all(test, feature = "ir"))]
mod tests {
    use super::{NO_DOC, ViewDir, ViewRow};

    /// A directory's documents in row order need not be in key order: a chunk's tape values follow its batch's
    /// documents in the order admitted (`values::Layered`, `device_chunk::views_over`). The search by key bisects the
    /// batch's keys and scans the values' (`ViewDir::keys`, `nbatch`), on the host and in its CLIF, for every key and
    /// some of none, with a memo of the right document, a wrong one and none. It bisected every row, which found no
    /// admitted value whose key was below one before it (a Vue piece's root had no children: a policy's findings came
    /// out wrong).
    #[test]
    fn a_directorys_search_by_key_finds_documents_out_of_key_order() {
        let jit = crate::jit::Reads::new();
        type ByKey = extern "C" fn(*const u8, u32, u32) -> u32;
        // SAFETY: the read's signature: the directory, then its arguments.
        let by_key: ByKey = unsafe { std::mem::transmute(jit.function("dir_doc_of_key")) };
        // Batches of 0 to 9 documents (each lower bound's count of steps), and none or three values admitted out of
        // key order: below, among and past the batch's keys.
        for nbatch in 0..10u32 {
            for values in [&[][..], &[2_090_826_275u32, 4, 1_786_418_522][..]] {
                let keys: Vec<u32> = (0..nbatch).map(|i| 3 * i + 5).chain(values.iter().copied()).collect();
                let n = keys.len();
                // The rows are the documents' numbers (plan "Chunk node numbers"), not the order's: the `m`th in the
                // order is row `n - 1 - m` here, so a search's answer is the row its order names.
                let row_of = |m: usize| (n - 1 - m) as u32;
                // The rows, the order and the keys after the directory (its size this build's: no field of it overlaps
                // them).
                let dir_at = 0usize;
                let rows_at = size_of::<ViewDir>().next_multiple_of(8);
                let order_at = rows_at + (n * size_of::<ViewRow>()).next_multiple_of(8);
                let keys_at = order_at + (4 * n).next_multiple_of(8);
                let mut block = vec![0u64; (keys_at + 4 * n).div_ceil(8) + 1];
                let base = block.as_mut_ptr().cast::<u8>();
                // SAFETY: each write within `block`, at an offset aligned for its type.
                unsafe {
                    for (m, &key) in keys.iter().enumerate() {
                        base.add(rows_at).cast::<ViewRow>().add(row_of(m) as usize).write(ViewRow { key, at: m as u32, ..ViewRow::default() });
                        base.add(order_at).cast::<u32>().add(m).write(row_of(m));
                    }
                    std::ptr::copy_nonoverlapping(keys.as_ptr(), base.add(keys_at).cast::<u32>(), n);
                    base.add(dir_at).cast::<ViewDir>().write(ViewDir {
                        at: dir_at as u64,
                        ndocs: n as u32,
                        rows: rows_at as u64,
                        nbatch,
                        keys: keys_at as u64,
                        order: order_at as u64,
                        span: n as u32,
                        ..ViewDir::default()
                    });
                }
                // SAFETY: the directory just written, in `block`, which outlives it.
                let dir = unsafe { &*base.add(dir_at).cast::<ViewDir>() };
                let asked = keys.iter().copied().chain([0, 4, 6, 3 * nbatch + 5, u32::MAX]);
                for key in asked {
                    let want = keys.iter().position(|&k| k == key).map_or(NO_DOC, row_of);
                    for memo in [0, 3, n as u32, NO_DOC] {
                        // SAFETY: as `dir`.
                        assert_eq!(unsafe { ViewDir::doc_of_key(dir, key, memo) }, want, "the host's, batch {nbatch}, values {values:?}, key {key}, memo {memo}");
                        assert_eq!(by_key(base.cast_const(), key, memo), want, "the CLIF's, batch {nbatch}, values {values:?}, key {key}, memo {memo}");
                    }
                }
            }
        }
    }

    /// The lower bound of a fixed count of steps is the first place whose key is at least the one asked, for every
    /// length to 40 and every key around each one held.
    #[test]
    fn a_lower_bound_in_steps_its_length_sets_is_the_first_at_least_the_key() {
        for n in 0..40u32 {
            let keys: Vec<u32> = (0..n).map(|i| 2 * i + 1).collect();
            for key in 0..2 * n + 3 {
                assert_eq!(super::lower_bound(&keys, key), keys.partition_point(|&k| k < key) as u32, "{n} keys, key {key}");
            }
        }
    }
}
