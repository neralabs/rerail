//! The views' reads as CLIF functions, written once (decision 0353 step 4c): a view's row is a read of the tape and
//! of its projection's numbering, never a relation made of it. The kernels (the MSL backend: `view_reads.metal`) and
//! the JIT read the views through them; the host's `Viewed` answers the same rows, which a test checks.
//!
//! A read takes the document's entry (`ViewDoc`, in its batch's block beside the projection's numbering, which it
//! refers to by a run), the tape's base and its view (`view::TapeView`), then a row's local number and a column. The
//! entry holds no per-node column: what a node's row says is its tape node's (`num_select`), read off the tape
//! (`tape_number`, `tape_start`, `tape_end`); a kind is its code's, by the derived table of the tape's codes.

use crate::ids::Run;
use crate::reads::B;
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
    /// core.rr's positions past the nodes (`Locs`), for the `Loc` views: the tokens, then where the text is the
    /// document's own (1) the whole text and each line, then the end points (`ends`, one a node, or none).
    pub tokens: u32,
    pub own_text: u32,
    pub ends: u32,
    pub source_len: u32,
    /// core.rr's lines (`core_rr::Lines`' words, `[ns, starts[ns], ne, ends[ne]]`), in the block: 0 items where the
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
    /// Device addresses: `device const uchar*` in the kernels.
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
/// `line`, then core.rr's places (`Loc`): `line`, `end_line`, `col`, `docof`.
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

/// The entry's MSL: `struct ViewDoc`, each field's offset and the size asserted against this definition's.
pub fn msl() -> String {
    use std::mem::offset_of;
    let fields: [(&str, &str, usize); 18] = [
        ("uint", "key", offset_of!(ViewDoc, key)),
        ("uint", "tag", offset_of!(ViewDoc, tag)),
        ("uint", "preorder", offset_of!(ViewDoc, preorder)),
        ("uint", "nodes", offset_of!(ViewDoc, nodes)),
        ("Run", "numbering", offset_of!(ViewDoc, numbering)),
        ("Run", "kinds", offset_of!(ViewDoc, kinds)),
        ("Run", "fields", offset_of!(ViewDoc, fields)),
        ("Run", "places", offset_of!(ViewDoc, places)),
        ("uint", "tokens", offset_of!(ViewDoc, tokens)),
        ("uint", "own_text", offset_of!(ViewDoc, own_text)),
        ("uint", "ends", offset_of!(ViewDoc, ends)),
        ("uint", "source_len", offset_of!(ViewDoc, source_len)),
        ("Run", "lines", offset_of!(ViewDoc, lines)),
        ("uint", "placement", offset_of!(ViewDoc, placement)),
        ("uint", "file", offset_of!(ViewDoc, file)),
        ("long", "delta", offset_of!(ViewDoc, delta)),
        ("uint", "within_lo", offset_of!(ViewDoc, within_lo)),
        ("uint", "within_hi", offset_of!(ViewDoc, within_hi)),
    ];
    let mut s = format!(
        "struct ViewRow {{\n    ulong entry;\n    device const uchar* tape;\n    device const uchar* lines;\n    uint rows[{}];\n    uint view[{}];\n}};\n\
         struct ViewDir {{\n    device const uchar* chunk;\n    uint ndocs;\n    uint pad;\n    device const ViewRow* rows;\n}};\n",
        VIEWS.len(),
        std::mem::size_of::<crate::view::TapeView>() / 4,
    );
    s += &format!("static_assert(sizeof(ViewRow) == {}, \"ViewRow: size\");\n", std::mem::size_of::<ViewRow>());
    s += &format!("static_assert(__builtin_offsetof(ViewRow, view) == {}, \"ViewRow.view\");\n", std::mem::offset_of!(ViewRow, view));
    s += &format!("static_assert(sizeof(ViewDir) == {}, \"ViewDir: size\");\n", std::mem::size_of::<ViewDir>());
    s += &format!("static_assert(__builtin_offsetof(ViewDir, rows) == {}, \"ViewDir.rows\");\n", std::mem::offset_of!(ViewDir, rows));
    s += "struct ViewDoc {\n";
    for (ty, name, _) in fields.iter() {
        s += &format!("    {ty} {name};\n");
    }
    s += "};\n";
    s += &format!("static_assert(sizeof(ViewDoc) == {}, \"ViewDoc: size\");\n", std::mem::size_of::<ViewDoc>());
    for (_, name, at) in fields.iter() {
        s += &format!("static_assert(__builtin_offsetof(ViewDoc, {name}) == {at}, \"ViewDoc.{name}\");\n");
    }
    s
}

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

/// `view_node(l, col)`: column `col` of the `node` view's row of numbered node `l`: the document, the id, the kind
/// atom, the start, the end (`Viewed::value_of`).
pub fn node(b: &mut B<'_, '_>) -> Value {
    let (l, col) = (b.arg(0), b.arg(1));
    let is = |b: &mut B<'_, '_>, k: i64| b.f.ins().icmp_imm(IntCC::Equal, col, k);
    let c = is(b, 0);
    b.either(I64, c, |b| b.entry_u32(at::KEY), |b| {
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
pub fn field(b: &mut B<'_, '_>) -> Value {
    let (i, col) = (b.arg(0), b.arg(1));
    let numbering = b.follow(b.tape, at::NUMBERING);
    let fielded = b.c32(2);
    let x = b.call_at("num_select", numbering, &[fielded, i]);
    let numbering = b.follow(b.tape, at::NUMBERING);
    let node = b.c32(0);
    let l = b.call_at("num_rank", numbering, &[node, x]);
    let c = b.f.ins().icmp_imm(IntCC::Equal, col, 0);
    b.either(I64, c, |b| b.parent_id(l, x), |b| {
        let c = b.f.ins().icmp_imm(IntCC::Equal, col, 1);
        b.either(I64, c, |b| {
            let code = b.call_tape("tape_number", &[x]);
            b.entry_item(at::FIELDS, code)
        }, |b| b.node_id(l))
    })
}

/// `view_core(l, col)`: column `col` of `core_node`'s row of numbered node `l`: the document, its tape node, its id.
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Line,
    EndLine,
    Col,
    Docof,
}

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
    /// A word of core.rr's lines (`ViewDoc::lines`) at index `i`.
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
    /// The 1-based line of core.rr's lines holding byte `at` (`core_rr::Lines::line`).
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

/// `view_loc_line(l, col)`, `view_loc_end`, `view_loc_col`, `view_loc_docof`: column `col` of core.rr's `line`,
/// `end_line`, `col` and `docof` rows of position `l` (a node, a token, the whole text, a line, an end point).
pub fn loc_line(b: &mut B<'_, '_>) -> Value {
    b.loc(Place::Line)
}
pub fn loc_end(b: &mut B<'_, '_>) -> Value {
    b.loc(Place::EndLine)
}
pub fn loc_col(b: &mut B<'_, '_>) -> Value {
    b.loc(Place::Col)
}
pub fn loc_docof(b: &mut B<'_, '_>) -> Value {
    b.loc(Place::Docof)
}
