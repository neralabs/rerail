//! The views' reads as CLIF functions, written once (decision 0353 step 4c): a view's row is a read of the tape and
//! of its projection's numbering, never a relation made of it. The kernels (the UKL backend: `<ukl/reads/view.h>`) and
//! the JIT read the views through them; the host's `Viewed` answers the same rows, which a test checks.
//!
//! A read takes the document's entry (`ViewDoc`, in its batch's block beside the projection's numbering, which it
//! refers to by a run), the tape's base and its view (`view::TapeView`), then a row's local number and a column. The
//! entry holds no per-node column: what a node's row says is its tape node's (`num_select`), read off the tape
//! (`tape_number`, `tape_start`, `tape_end`); a kind is its code's, by the derived table of the tape's codes.

use crate::ids::Run;
#[cfg(feature = "ir")]
use crate::reads::B;
#[cfg(feature = "ir")]
use cranelift_codegen::ir::{
    InstBuilder, Value,
    condcodes::IntCC,
    types::{I32, I64},
};

/// A document's entry for its views' reads (`ViewDoc` in the kernels).
#[repr(C)]
#[derive(Debug, Default)]
pub struct ViewDoc {
    /// The document's key: an id's high 32 bits.
    pub key: u32,
    /// Its dialect's tape tag.
    pub tag: u32,
    /// 1 when ids are local ids (`RERAIL_NODE_IDS=preorder`: no granule table).
    pub preorder: u32,
    /// The projection's numbered nodes.
    pub nodes: u32,
    /// The projection's numbering (`ids::Numbering`, one).
    pub numbering: Run,
    /// By the tape's code (its table's number, `tape_number`): the kind atom its `kind` column holds (u32s, its
    /// name's canonical code's), and the field atom (`tag << 16 | field`).
    pub kinds: Run,
    pub fields: Run,
    /// Each numbered node's place among its parent's children (u32s, by local id): the walk's, derived once
    /// (`Projected::idx`, decision 0353 step 3), as the tape holds no sibling's place.
    pub places: Run,
    /// core.rerail's positions past the nodes (`Locs`), for the `Loc` views: the tokens, then where the text is the
    /// document's own (1) the whole text and each line, then the end points (`ends`, one a node, or none).
    pub tokens: u32,
    pub own_text: u32,
    pub ends: u32,
    pub source_len: u32,
    /// core.rerail's lines (`core_rr::Lines`' words, `[ns, starts[ns], ne, ends[ne]]`), in the block: 0 items where the
    /// document has no `Loc` rows on the device. A guest's are its host's (decision 0055).
    pub lines: Run,
    /// A guest's placement in its host (`Placement`): 0 none (a file's own tree), `PLACED_COPIED` (host byte = guest
    /// byte + `delta`, clamped to `within`), `PLACED_AT` (every span at `within`).
    pub placement: u32,
    /// The key of the file the document is part of (decision 0431 §4): its own, or a program's file's. Its
    /// `docof`; its ids keep `key`.
    pub file: u32,
    pub delta: i64,
    pub within_lo: u32,
    pub within_hi: u32,
}
const _: () = assert!(std::mem::size_of::<ViewDoc>() == 16 + 16 * 4 + 16 + 16 + 24);

/// `ViewDoc::placement`'s kinds.
pub const PLACED_COPIED: u32 = 1;
pub const PLACED_AT: u32 = 2;

/// A document of a batch, as a kernel finds its views' rows (decision 0353 step 4d): its entry's offset in the
/// batch's block, its tape's and its lines' GPU addresses, its tape's view (the reads' own), and its first row in
/// each view (`VIEWS`).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct ViewRow {
    pub entry: u64,
    /// Device addresses: `UKL_GLOBAL const uint8_t*` in the kernels.
    pub tape: u64,
    pub lines: u64,
    pub rows: [u32; VIEWS.len()],
    pub view: crate::view::TapeView,
}
const _: () = assert!(std::mem::size_of::<ViewRow>() == (8 + 8 + 8 + 36 + std::mem::size_of::<crate::view::TapeView>()).next_multiple_of(8));

/// A batch's documents as its views' reads find them: the batch's block's GPU address (the entries' base), its
/// documents' rows (their GPU address), how many.
#[repr(C)]
#[derive(Debug, Default)]
pub struct ViewDir {
    pub chunk: u64,
    pub ndocs: u32,
    pub pad: u32,
    pub rows: u64,
}
const _: () = assert!(std::mem::size_of::<ViewDir>() == 24);

/// A view's number in a kernel's reads (`ViewDir`'s rows' and `vread`'s): `node`, `child`, `field`, `core_node`,
/// `line`, then core.rerail's places (`Loc`): `line`, `end_line`, `col`, `docof`.
pub const VIEWS: [&str; 9] = ["view_node", "view_child", "view_field", "view_core", "view_line", "view_loc_line", "view_loc_end", "view_loc_col", "view_loc_docof"];

/// A field's byte offset in the entry.
pub mod at {
    use super::ViewDoc;
    use std::mem::offset_of;
    pub const KEY: i32 = offset_of!(ViewDoc, key) as i32;
    pub const TAG: i32 = offset_of!(ViewDoc, tag) as i32;
    pub const PREORDER: i32 = offset_of!(ViewDoc, preorder) as i32;
    pub const NODES: i32 = offset_of!(ViewDoc, nodes) as i32;
    pub const NUMBERING: i32 = offset_of!(ViewDoc, numbering) as i32;
    pub const KINDS: i32 = offset_of!(ViewDoc, kinds) as i32;
    pub const FIELDS: i32 = offset_of!(ViewDoc, fields) as i32;
    pub const PLACES: i32 = offset_of!(ViewDoc, places) as i32;
    pub const TOKENS: i32 = offset_of!(ViewDoc, tokens) as i32;
    pub const OWN_TEXT: i32 = offset_of!(ViewDoc, own_text) as i32;
    pub const ENDS: i32 = offset_of!(ViewDoc, ends) as i32;
    pub const SOURCE_LEN: i32 = offset_of!(ViewDoc, source_len) as i32;
    pub const LINES: i32 = offset_of!(ViewDoc, lines) as i32;
    pub const PLACEMENT: i32 = offset_of!(ViewDoc, placement) as i32;
    pub const FILE: i32 = offset_of!(ViewDoc, file) as i32;
    pub const DELTA: i32 = offset_of!(ViewDoc, delta) as i32;
    pub const WITHIN_LO: i32 = offset_of!(ViewDoc, within_lo) as i32;
    pub const WITHIN_HI: i32 = offset_of!(ViewDoc, within_hi) as i32;
}

/// The `node` view's columns (`Viewed::columns`): the document, the node's id, its kind, start and end.
pub const NODE_COLUMNS: u32 = 5;
/// `child`'s: the parent's id, the child's, its place; `field`'s: the parent's id, the field atom, the child's id;
/// `core_node`'s: the document, the tape node, the id.
pub const CHILD_COLUMNS: u32 = 3;
pub const FIELD_COLUMNS: u32 = 3;
pub const CORE_COLUMNS: u32 = 3;
/// `line`'s: the node's id, the line its start is on (from 1); each `Loc` view's: the node's or span's id, its place.
pub const LINE_COLUMNS: u32 = 2;
pub const LOC_COLUMNS: u32 = 2;

/// The entry in UKL: `struct ViewDoc`, each field's offset and the size asserted against this definition's.
#[cfg(feature = "ir")]
pub fn ukl() -> String {
    use std::mem::offset_of;
    let fields: [(&str, &str, usize); 18] = [
        ("uint32_t", "key", offset_of!(ViewDoc, key)),
        ("uint32_t", "tag", offset_of!(ViewDoc, tag)),
        ("uint32_t", "preorder", offset_of!(ViewDoc, preorder)),
        ("uint32_t", "nodes", offset_of!(ViewDoc, nodes)),
        ("Run", "numbering", offset_of!(ViewDoc, numbering)),
        ("Run", "kinds", offset_of!(ViewDoc, kinds)),
        ("Run", "fields", offset_of!(ViewDoc, fields)),
        ("Run", "places", offset_of!(ViewDoc, places)),
        ("uint32_t", "tokens", offset_of!(ViewDoc, tokens)),
        ("uint32_t", "own_text", offset_of!(ViewDoc, own_text)),
        ("uint32_t", "ends", offset_of!(ViewDoc, ends)),
        ("uint32_t", "source_len", offset_of!(ViewDoc, source_len)),
        ("Run", "lines", offset_of!(ViewDoc, lines)),
        ("uint32_t", "placement", offset_of!(ViewDoc, placement)),
        ("uint32_t", "file", offset_of!(ViewDoc, file)),
        ("int64_t", "delta", offset_of!(ViewDoc, delta)),
        ("uint32_t", "within_lo", offset_of!(ViewDoc, within_lo)),
        ("uint32_t", "within_hi", offset_of!(ViewDoc, within_hi)),
    ];
    let mut s = format!(
        "#include <ukl/backend.h>\ntypedef struct ViewRow {{\n    uint64_t entry;\n    UKL_GLOBAL const uint8_t* tape;\n    UKL_GLOBAL const uint8_t* lines;\n    uint32_t rows[{}];\n    uint32_t view[{}];\n}} ViewRow;\n\
         typedef struct ViewDir {{\n    UKL_GLOBAL const uint8_t* chunk;\n    uint32_t ndocs;\n    uint32_t pad;\n    UKL_GLOBAL const ViewRow* rows;\n}} ViewDir;\n",
        VIEWS.len(),
        std::mem::size_of::<crate::view::TapeView>() / 4,
    );
    s += &format!("UKL_STATIC_ASSERT(sizeof(ViewRow) == {}, \"ViewRow: size\");\n", std::mem::size_of::<ViewRow>());
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewRow, view) == {}, \"ViewRow.view\");\n", std::mem::offset_of!(ViewRow, view));
    s += &format!("UKL_STATIC_ASSERT(sizeof(ViewDir) == {}, \"ViewDir: size\");\n", std::mem::size_of::<ViewDir>());
    s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF(ViewDir, rows) == {}, \"ViewDir.rows\");\n", std::mem::offset_of!(ViewDir, rows));
    s += "typedef struct ViewDoc {\n";
    for (ty, name, _) in fields.iter() {
        s += &format!("    {ty} {name};\n");
    }
    s += "} ViewDoc;\n";
    s += &format!("UKL_STATIC_ASSERT(sizeof(ViewDoc) == {}, \"ViewDoc: size\");\n", std::mem::size_of::<ViewDoc>());
    for (_, name, at) in fields.iter() {
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
/// root): the parent's id, the child's id, its place among the parent's children.
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
        b.either(I64, c, |b| b.node_id(l), |b| b.entry_item(at::PLACES, l))
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

/// `lines_line(at)`: the lines starting at or before byte `at` (`Facts::start_line`: `LineTable::line` + 1), by
/// bisection of the starts.
#[cfg(feature = "ir")]
pub fn lines_line(b: &mut B<'_, '_>) -> Value {
    let at_ = b.arg(0);
    let n = b.word_at(I32, b.tape, 0);
    b.bisect(n, &mut |b, i| {
        let starts = b.f.ins().iadd_imm(b.tape, 4);
        let p = b.item(starts, i, 4);
        let start = b.word_at(I32, p, 0);
        b.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, start, at_)
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
    /// A word of core.rerail's lines (`ViewDoc::lines`) at index `i`.
    fn loc_word(&mut self, i: Value) -> Value {
        let lines = self.follow(self.tape, at::LINES);
        let p = self.item(lines, i, 4);
        self.word_at(I32, p, 0)
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
                let ns = b.loc_word_at(0);
                let ns1 = b.f.ins().iadd_imm(ns, 1);
                let ne = b.loc_word(ns1);
                let texts = b.f.ins().iadd_imm(ne, 1);
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
                        // Line `j`'s text: its start (word `j`) and its end (word `ns + 1 + j`).
                        let s = b.loc_word(j);
                        let ns = b.loc_word_at(0);
                        let at_ = b.f.ins().iadd(ns, j);
                        let at_ = b.f.ins().iadd_imm(at_, 1);
                        let e = b.loc_word(at_);
                        b.pair(s, e)
                    })
                })
            })
        })
    }
    fn loc_word_at(&mut self, i: i64) -> Value {
        let i = self.c32(i);
        self.loc_word(i)
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
    /// The 1-based line of core.rerail's lines holding byte `at` (`core_rr::Lines::line`).
    fn loc_line(&mut self, at_: Value) -> Value {
        let lines = self.follow(self.tape, at::LINES);
        let n = self.call_at("loc_line", lines, &[at_]);
        let one = self.c32(1);
        self.f.ins().umax(n, one)
    }
    /// A `Loc` view's row: the id, then its place (`Viewed::loc_value`, `core_rr::Lines::place`).
    fn loc(&mut self, place: Place) -> Value {
        let (l, col) = (self.arg(0), self.arg(1));
        let c = self.f.ins().icmp_imm(IntCC::Equal, col, 0);
        self.either(I64, c, |b| b.loc_id(l), |b| {
            if place == Place::Docof {
                return b.entry_u32(at::FILE);
            }
            let span = b.loc_span(l);
            let span = b.placed(span);
            let s = b.f.ins().ushr_imm(span, 32);
            let s = b.f.ins().ireduce(I32, s);
            let e = b.f.ins().ireduce(I32, span);
            let line = b.loc_line(s);
            let v = match place {
                Place::Line => line,
                Place::EndLine => {
                    let past = b.f.ins().icmp(IntCC::UnsignedGreaterThan, e, s);
                    b.either(I32, past, |b| {
                        let last = b.f.ins().iadd_imm(e, -1);
                        b.loc_line(last)
                    }, |_| line)
                }
                Place::Col => {
                    // Its line's start: word `line` (the starts from word 1).
                    let start = b.loc_word(line);
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

/// No row: a parent probe's end (`view_child_first`, `view_child_next`, `view_number`).
pub const NO_ROW: u32 = u32::MAX;

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
