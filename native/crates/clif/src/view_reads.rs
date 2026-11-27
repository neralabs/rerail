//! The views' reads as CLIF functions, written once (decision 0353 step 4c): a view's row is a read of the tape and
//! of its projection's numbering, never a relation made of it. The kernels (the UKL backend: `<ukl/reads/view.h>`) and
//! the JIT read the views through them; the host's `Viewed` answers the same rows, which a test checks.
//!
//! A read takes the document's entry (`ViewDoc`, in its batch's block beside the projection's numbering, which it
//! refers to by a run), the tape's base and its view (`view::TapeView`), then a row's local number and a column. The
//! entry holds no per-node column: what a node's row says is its tape node's (`num_select`), read off the tape
//! (`tape_number`, `tape_start`, `tape_end`); a kind is its code's, by the derived table of the tape's codes.

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
        ("Run", "kinds", offset_of!(ViewDoc, kinds)),
        ("Run", "fields", offset_of!(ViewDoc, fields)),
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
        ("uint32_t", "place", offset_of!(ViewDoc, place)),
        ("uint32_t", "doc_path", offset_of!(ViewDoc, doc_path)),
        ("uint32_t", "doc_module", offset_of!(ViewDoc, doc_module)),
        ("uint32_t", "pad", offset_of!(ViewDoc, pad)),
        ("Run", "node_table", offset_of!(ViewDoc, node_table)),
        ("Run", "node_by_id", offset_of!(ViewDoc, node_by_id)),
    ];
    let mut s = format!(
        "#include <ukl/backend.h>\ntypedef struct ViewRow {{\n    uint64_t entry;\n    uint64_t tape;\n    uint64_t lines;\n    uint64_t source;\n    uint32_t homes[3];\n    uint32_t key;\n    uint32_t rows[{}];\n    uint32_t view[{}];\n    uint32_t bytes;\n}} ViewRow;\n\
         typedef struct ViewHome {{\n    uint64_t host;\n    UKL_GLOBAL const uint8_t* gpu;\n}} ViewHome;\n\
         typedef struct ViewDir {{\n    uint64_t at;\n    uint32_t ndocs;\n    uint32_t nhomes;\n    uint64_t rows;\n    uint64_t homes;\n    uint64_t cache;\n    uint64_t by_key;\n    uint32_t ends[{}];\n    uint32_t counted;\n    uint32_t pad;\n}} ViewDir;\n",
        ROWS,
        std::mem::size_of::<crate::view::TapeView>() / 4,
        ROWS,
    );
    s += &format!("UKL_STATIC_ASSERT(sizeof(ViewRow) == {}, \"ViewRow: size\");\n", std::mem::size_of::<ViewRow>());
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewRow, homes) == {}, \"ViewRow.homes\");\n", std::mem::offset_of!(ViewRow, homes));
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewRow, view) == {}, \"ViewRow.view\");\n", std::mem::offset_of!(ViewRow, view));
    s += &format!("UKL_STATIC_ASSERT(sizeof(ViewHome) == {}, \"ViewHome: size\");\n", std::mem::size_of::<ViewHome>());
    s += &format!("UKL_STATIC_ASSERT(sizeof(ViewDir) == {}, \"ViewDir: size\");\n", std::mem::size_of::<ViewDir>());
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewDir, rows) == {}, \"ViewDir.rows\");\n", std::mem::offset_of!(ViewDir, rows));
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewDir, cache) == {}, \"ViewDir.cache\");\n", std::mem::offset_of!(ViewDir, cache));
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewDir, by_key) == {}, \"ViewDir.by_key\");\n", std::mem::offset_of!(ViewDir, by_key));
    // A directory's places as a kernel reads them: the block's start, a row, and a row's tape and lines at their
    // homes' device bases.
    s += "UKL_INLINE UKL_GLOBAL const uint8_t* view_block(UKL_GLOBAL const ViewDir* d) { return (UKL_GLOBAL const uint8_t*)d - d->at; }\n";
    s += "UKL_INLINE UKL_GLOBAL const ViewRow* view_rows(UKL_GLOBAL const ViewDir* d) { return (UKL_GLOBAL const ViewRow*)(view_block(d) + d->rows); }\n";
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
    /// The u32 at index `i` of the entry's run at `offset`, widened.
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
        let local = self.f.ins().icmp_imm(IntCC::NotEqual, preorder, 0);
        let low = self.either(I32, local, |_| l, |b| {
            let numbering = b.follow(b.tape, at::NUMBERING);
            let node = b.c32(0);
            b.call_at("num_low_of", numbering, &[node, l])
        });
        let low = self.f.ins().uextend(I64, low);
        self.f.ins().bor(high, low)
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
                b.entry_item(at::KINDS, code)
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
            b.entry_item(at::FIELDS, code)
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
        let local = self.f.ins().icmp_imm(IntCC::NotEqual, preorder, 0);
        let low = self.either(I32, local, |_| l, |b| {
            let nodes = b.word_at(I32, b.tape, at::NODES);
            let node = b.f.ins().icmp(IntCC::UnsignedLessThan, l, nodes);
            b.either(I32, node, |b| {
                let numbering = b.follow(b.tape, at::NUMBERING);
                let item = b.c32(0);
                b.call_at("num_low_of", numbering, &[item, l])
            }, |b| {
                let i = b.f.ins().isub(l, nodes);
                let tokens = b.word_at(I32, b.tape, at::TOKENS);
                let token = b.f.ins().icmp(IntCC::UnsignedLessThan, i, tokens);
                b.either(I32, token, |b| {
                    let numbering = b.follow(b.tape, at::NUMBERING);
                    let item = b.c32(1);
                    b.call_at("num_low_of", numbering, &[item, i])
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
    /// `delta` and kept within `within`; at `within`. A file's own, as it is.
    fn placed(&mut self, span: Value) -> Value {
        let placement = self.word_at(I32, self.tape, at::PLACEMENT);
        let own = self.f.ins().icmp_imm(IntCC::Equal, placement, 0);
        self.either(I64, own, |_| span, |b| {
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
        })
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
    pub const BY_KEY: i32 = offset_of!(ViewDir, by_key) as i32;
    pub const ROW_SIZE: i64 = size_of::<ViewRow>() as i64;
    pub const ROW_ROWS: i32 = offset_of!(ViewRow, rows) as i32;
    pub const ROW_KEY: i32 = offset_of!(ViewRow, key) as i32;
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
    /// The directory's order by key (`by_key`): its address.
    fn dir_order(&mut self) -> Value {
        let at = self.word_at(I64, self.tape, dir_at::AT);
        let block = self.f.ins().isub(self.tape, at);
        let order = self.word_at(I64, self.tape, dir_at::BY_KEY);
        self.f.ins().iadd(block, order)
    }
    /// The row of the document `i`th by key.
    fn dir_by_key(&mut self, order: Value, i: Value) -> Value {
        let p = self.item(order, i, 4);
        self.word_at(I32, p, 0)
    }
    /// Row `m`'s document's key.
    fn dir_key(&mut self, rows: Value, m: Value) -> Value {
        let row = self.item(rows, m, dir_at::ROW_SIZE);
        self.word_at(I32, row, dir_at::ROW_KEY)
    }
}

/// `dir_doc_of_row(k, r, memo)`: the document whose rows of view `k` hold row `r` (`ViewDir::doc_of_row`), memo
/// `memo` tried first.
#[cfg(feature = "ir")]
pub fn dir_doc_of_row(b: &mut B<'_, '_>) -> Value {
    let (k, r, memo) = (b.arg(0), b.arg(1), b.arg(2));
    let (rows, n) = b.dir_rows();
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, memo, n);
    let held = b.either(I32, inside, |b| {
        let first = b.dir_first(rows, memo, k);
        let from = b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, first, r);
        let next = b.f.ins().iadd_imm(memo, 1);
        let last = b.f.ins().icmp(IntCC::Equal, next, n);
        let before = b.either(I32, last, |b| b.c32(1), |b| {
            let after = b.dir_first(rows, next, k);
            let past = b.f.ins().icmp(IntCC::UnsignedGreaterThan, after, r);
            b.f.ins().uextend(I32, past)
        });
        let from = b.f.ins().uextend(I32, from);
        b.f.ins().band(from, before)
    }, |b| b.c32(0));
    let hit = b.f.ins().icmp_imm(IntCC::NotEqual, held, 0);
    b.either(I32, hit, |_| memo, |b| {
        let c = b.bisect(n, &mut |b, m| {
            let first = b.dir_first(rows, m, k);
            b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, first, r)
        });
        let one = b.c32(1);
        let c = b.f.ins().umax(c, one);
        b.f.ins().isub(c, one)
    })
}

/// `dir_doc_of_key(key, memo)`: the document of key `key` (`ViewDir::doc_of_key`: a bisection of the order by key),
/// or `NO_DOC`, memo `memo` (a row) tried first.
#[cfg(feature = "ir")]
pub fn dir_doc_of_key(b: &mut B<'_, '_>) -> Value {
    let (key, memo) = (b.arg(0), b.arg(1));
    let (rows, n) = b.dir_rows();
    let inside = b.f.ins().icmp(IntCC::UnsignedLessThan, memo, n);
    let held = b.either(I32, inside, |b| {
        let k = b.dir_key(rows, memo);
        let same = b.f.ins().icmp(IntCC::Equal, k, key);
        b.f.ins().uextend(I32, same)
    }, |b| b.c32(0));
    let hit = b.f.ins().icmp_imm(IntCC::NotEqual, held, 0);
    b.either(I32, hit, |_| memo, |b| {
        let order = b.dir_order();
        let c = b.bisect(n, &mut |b, i| {
            let m = b.dir_by_key(order, i);
            let k = b.dir_key(rows, m);
            b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, k, key)
        });
        let any = b.f.ins().icmp_imm(IntCC::NotEqual, c, 0);
        b.either(I32, any, |b| {
            let i = b.f.ins().iadd_imm(c, -1);
            let m = b.dir_by_key(order, i);
            let k = b.dir_key(rows, m);
            let same = b.f.ins().icmp(IntCC::Equal, k, key);
            let no = b.c32(i64::from(NO_DOC));
            b.f.ins().select(same, m, no)
        }, |b| b.c32(i64::from(NO_DOC)))
    })
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

/// `view_number(low)`: the numbered node whose id's low 32 bits are `low` (its document's: the key is the caller's
/// to match), or `NO_ROW`: `low` itself where ids are local ids, else the numbering's inverse (`num_number_of`,
/// a node's, not a token's).
#[cfg(feature = "ir")]
pub fn number(b: &mut B<'_, '_>) -> Value {
    let low = b.arg(0);
    let preorder = b.word_at(I32, b.tape, at::PREORDER);
    let local = b.f.ins().icmp_imm(IntCC::NotEqual, preorder, 0);
    let n = b.word_at(I32, b.tape, at::NODES);
    let l = b.either(I32, local, |_| low, |b| {
        let numbering = b.follow(b.tape, at::NUMBERING);
        let found = b.call_at("num_number_of", numbering, &[low]);
        // A node's: item 0 in the high word.
        let item = b.f.ins().ushr_imm(found, 32);
        let node = b.f.ins().icmp_imm(IntCC::Equal, item, 0);
        let number = b.f.ins().ireduce(I32, found);
        let no = b.c32(i64::from(NO_ROW));
        b.f.ins().select(node, number, no)
    });
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
        let local = b.f.ins().icmp_imm(IntCC::NotEqual, preorder, 0);
        let low = b.either(I32, local, |_| l, |b| {
            let numbering = b.follow(b.tape, at::NUMBERING);
            b.call_at("num_order_of", numbering, &[l])
        });
        let low = b.f.ins().uextend(I64, low);
        b.f.ins().bor(high, low)
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

#[cfg(all(test, feature = "ir"))]
mod tests {
    use super::{NO_DOC, ViewDir, ViewRow};

    /// A directory's documents in row order need not be in key order: a chunk's tape values follow its batch's
    /// documents (`values::Layered`, `device_chunk::views_over`). The search by key bisects the order by key
    /// (`ViewDir::by_key`), on the host and in its CLIF, for every key and some of none, with a memo of the right
    /// document, a wrong one and none. A search of the rows as if in key order found no admitted value whose key was
    /// below one before it (a Vue piece's root had no children: a policy's findings came out wrong).
    #[test]
    fn a_directorys_search_by_key_finds_documents_out_of_key_order() {
        let jit = crate::jit::Reads::new();
        type ByKey = extern "C" fn(*const u8, u32, u32) -> u32;
        // SAFETY: the read's signature: the directory, then its arguments.
        let by_key: ByKey = unsafe { std::mem::transmute(jit.function("dir_doc_of_key")) };
        // Batches of 0 to 9 documents, and none or three values admitted out of key order: below, among and past the
        // batch's keys.
        for nbatch in 0..10u32 {
            for values in [&[][..], &[2_090_826_275u32, 4, 1_786_418_522][..]] {
                let keys: Vec<u32> = (0..nbatch).map(|i| 3 * i + 5).chain(values.iter().copied()).collect();
                let n = keys.len();
                let mut order: Vec<u32> = (0..n as u32).collect();
                order.sort_by_key(|&i| keys[i as usize]);
                let row = std::mem::size_of::<ViewRow>();
                let (dir_at, rows_at) = (0usize, std::mem::size_of::<ViewDir>().next_multiple_of(8));
                let order_at = rows_at + row * n;
                let mut block = vec![0u64; (order_at + 4 * n).div_ceil(8) + 1];
                let base = block.as_mut_ptr().cast::<u8>();
                // SAFETY: each write within `block`, at an offset aligned for its type.
                unsafe {
                    for (i, &k) in keys.iter().enumerate() {
                        base.add(rows_at + row * i).cast::<ViewRow>().write(ViewRow { key: k, ..ViewRow::default() });
                    }
                    std::ptr::copy_nonoverlapping(order.as_ptr(), base.add(order_at).cast::<u32>(), n);
                    base.add(dir_at).cast::<ViewDir>().write(ViewDir { at: dir_at as u64, ndocs: n as u32, rows: rows_at as u64, by_key: order_at as u64, ..ViewDir::default() });
                }
                // SAFETY: within `block`.
                let dir = unsafe { base.add(dir_at) }.cast::<ViewDir>().cast_const();
                let asked = keys.iter().copied().chain([0, 4, 6, 3 * nbatch + 5, u32::MAX]);
                for key in asked {
                    let want = keys.iter().position(|&k| k == key).map_or(NO_DOC, |i| i as u32);
                    for memo in [0, 3, n as u32, NO_DOC] {
                        // SAFETY: the directory just written, in `block`, which outlives it.
                        assert_eq!(unsafe { ViewDir::doc_of_key(dir, key, memo) }, want, "the host's, batch {nbatch}, values {values:?}, key {key}, memo {memo}");
                        assert_eq!(by_key(base.cast_const(), key, memo), want, "the CLIF's, batch {nbatch}, values {values:?}, key {key}, memo {memo}");
                    }
                }
            }
        }
    }
}
