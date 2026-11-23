//! simdjson (the C++ library, vendored at `native/vendor/simdjson-5.0.5`) and the engine's JSON tape.
//!
//! The portable side: the native code (simdjson, the kernels) is `rerail-simdjson-native`'s.
//!
//! `tape_with` is a kernel's: simdjson's stage 1, the source hashed in it, and a stage 2 of ours, which
//! walks the index itself, validates the document as simdjson's does, and writes the v9np tape into the
//! tape's own buffers. `structurals` is simdjson's DOM parser and the structural index its stage 1
//! found (`tape_from_index`, the kernels' oracle and the tape on a CPU no kernel runs on, writes a tape
//! from it).
//!
//! The index holds, in order, the offset of each structural byte (`{ } [ ] : ,`) and of each
//! scalar's first byte (a string's opening quote, a number's or a literal's first byte). Each
//! thread parses with a parser of its own.

pub use rerail_simdjson_native::{Error, Kernel, implementation, structurals};
use rerail_simdjson_native::{KEEP, Sink, release_above};

/// `grow`: each buffer holds `need[i]` bytes, its first `len[i]` kept.
unsafe extern "C" fn grow(sink: *mut Sink, need: *const usize) {
    // SAFETY: the shim passes the sink it was given, whose `ctx` is the tape's buffers, and
    // `need` of 5; it has written the first `len[i]` bytes of each.
    unsafe {
        let sink = &mut *sink;
        let columns = &mut *sink.ctx.cast::<[Vec<u8>; 5]>();
        for (i, v) in columns.iter_mut().enumerate() {
            v.set_len(sink.len[i]);
            let need = *need.add(i);
            v.reserve(need.saturating_sub(v.len()));
            sink.col[i] = v.as_mut_ptr();
            sink.cap[i] = v.capacity();
        }
    }
}

/// The kinds and fields a JSON tape is written with: tree-sitter-json's ids, which the caller looks up.
#[derive(Clone, Copy, Debug)]
pub struct Kinds {
    pub document: u16,
    pub object: u16,
    pub pair: u16,
    pub array: u16,
    pub string: u16,
    pub string_content: u16,
    pub escape_sequence: u16,
    pub number: u16,
    pub true_: u16,
    pub false_: u16,
    pub null: u16,
    /// tree-sitter-json's `comment`: a JSONC file's (`tape_jsonc_with`).
    pub comment: u16,
    pub key_field: u16,
    pub value_field: u16,
}

/// A JSON document's tape, written straight from simdjson's index in the engine's v9np columns
/// (`tape::TapeImage`): tree-sitter-json's nodes in preorder (`document`, `object`, `pair` with its `key` and
/// `value`, `array`, `string` with its `string_content` and `escape_sequence`, `number`, `true`, `false`,
/// `null`), without the tokens each node implies.
///
/// One pass over the index writes every column at the node's preorder slot `ord`:
/// - at a node's start, its `code` (numbered by first use in the file), its `off` from its block's
///   `base`, and the `base` of each block of 16 nodes (starts never decrease in preorder);
/// - at its end, its `size` (the nodes written since it) and its `tail` (its end less the start of
///   the last node written, its last descendant).
///
/// `size`, `off` and `tail` are written as `u32`s, then narrowed to 2 bytes where the file's largest
/// value fits, as the tape stores them.
#[derive(Clone, Debug, Default)]
pub struct Tape {
    /// Each node's code, an index into `codes`, 2 bytes little-endian.
    pub code: Vec<u8>,
    /// The file's codes, in first-use order: kind, field, and whether the node is in a value's place
    /// (tree-sitter-json's `_value` supertype: the document's child, an array element, a pair's value).
    pub codes: Vec<(u16, u16, bool)>,
    pub size: Vec<u8>,
    pub base: Vec<u8>,
    pub off: Vec<u8>,
    pub tail: Vec<u8>,
    /// `size`, `off` and `tail`'s widths in bytes (2 or 4); every tail is from the last descendant's start.
    pub widths: [u8; 3],
    /// The source's SHA-256, where the pass that read it computed it (`tape`, not `tape_from_index`).
    pub digest: Option<[u8; 32]>,
}

impl Tape {
    pub fn node_count(&self) -> usize {
        self.code.len() / 2
    }
}

/// A block of the tape: `base` holds one start per this many nodes (`tape::CHECKPOINT_STRIDE`).
pub const STRIDE: usize = 16;

/// Why a document has no tape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// simdjson refused it.
    Simdjson(Error),
    /// Its index is not one a valid document has (never, for a document simdjson accepts).
    Shape(&'static str),
    /// It is past the tape's offsets (4 GB; 2 GB for a kernel).
    TooLarge,
    /// This CPU does not run the kernel asked for.
    NotHere,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Simdjson(e) => write!(f, "simdjson: {e}"),
            Refusal::Shape(why) => f.write_str(why),
            Refusal::TooLarge => f.write_str("past 4 GB"),
            Refusal::NotHere => f.write_str("a kernel this CPU does not run"),
        }
    }
}

/// `src`'s tape and its SHA-256 by `kernel`, in one pass of stage 1 and one of a stage 2 of ours:
/// - stage 1 is simdjson's (the kernel's implementation's), each 64-byte block of the source also
///   hashed (with the CPU's SHA-256 instructions, where the kernel has them);
/// - stage 2 walks the index itself, as simdjson's walker does, checks each scalar as simdjson's tape
///   builder does (escapes, numbers, atoms), and writes the columns at each node's preorder slot, into
///   the returned `Tape`'s own buffers, at their stored widths, with the tape's codes. No DOM tape, no
///   unescaped strings, no number values, nothing copied or converted after.
///
/// Each column is 2 bytes wide unless a value needs 4: `size`'s width is predicted from the structural
/// count, `off` and `tail` start at 2, and a file a prediction fails is walked again at the right widths
/// (7 of the repository's 2,200). `Refusal::NotHere` where this CPU does not run the kernel.
///
/// The nodes are `tape_from_index`'s, byte for byte (`every_kernels_tape_is_the_index_writers`).
pub fn tape_with(kernel: Kernel, src: &[u8], kinds: &Kinds) -> Result<Tape, Refusal> {
    run(kernel, src, kinds, false)
}

/// `src`'s tape by `kernel`, read as JSONC: as `tape_with`, each comment (`//` to the line's end, its
/// `\n` not included; `/* … */`) also a node, a `comment` leaf of the lowest node holding the tokens on
/// either side of it, as tree-sitter-json places it (an extra). Stage 1 reads the source with its
/// comments blanked; the digest is the source's as given. A trailing comma (before `]` or `}`) is passed
/// over, as TypeScript reads JSONC: the tape is the one the file has without it. For a file the strict
/// read refused: a strict file's tape is `tape_with`'s.
pub fn tape_jsonc_with(kernel: Kernel, src: &[u8], kinds: &Kinds) -> Result<Tape, Refusal> {
    run(kernel, src, kinds, true)
}

/// `src`'s tape by `kernel`, the source not hashed (`digest` is `None`): for a measurement, against
/// a reader that does not hash (`rerail-json-lean`).
pub fn tape_unhashed_with(kernel: Kernel, src: &[u8], kinds: &Kinds) -> Result<Tape, Refusal> {
    let mut t = run_as(kernel, src, kinds, Read::Unhashed)?;
    t.digest = None;
    Ok(t)
}
#[derive(Clone, Copy, PartialEq)]
enum Read {
    Strict,
    Jsonc,
    Unhashed,
}
fn run(kernel: Kernel, src: &[u8], kinds: &Kinds, jsonc: bool) -> Result<Tape, Refusal> {
    run_as(kernel, src, kinds, if jsonc { Read::Jsonc } else { Read::Strict })
}
fn run_as(kernel: Kernel, src: &[u8], kinds: &Kinds, read: Read) -> Result<Tape, Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    let mut columns: [Vec<u8>; 5] = Default::default();
    let mut sink = sink(kinds, (&raw mut columns).cast());
    // SAFETY: the sink's buffers are `columns`, which `grow` grows; the shim writes them only within
    // what `grow` gave.
    let code = unsafe {
        match read {
            Read::Strict => kernel.tape(src, &mut sink),
            Read::Jsonc => kernel.tape_jsonc(src, &mut sink),
            Read::Unhashed => kernel.tape_unhashed(src, &mut sink),
        }
    };
    if src.len() > KEEP {
        release_above(KEEP);
    }
    match code {
        -1 => Err(Refusal::NotHere),
        0 => {
            for (v, &len) in columns.iter_mut().zip(&sink.len) {
                // SAFETY: the shim wrote `len` bytes of each, within its capacity.
                unsafe { v.set_len(len) };
                // The buffers were made for the most nodes a file of its length can have: what the
                // tape does not use is given back.
                if v.capacity() > len + len / 2 + 64 {
                    v.shrink_to_fit();
                }
            }
            let codes = (0..sink.codes as usize).map(|i| (sink.code_kind[i], sink.code_field[i], sink.code_value[i] != 0)).collect();
            let [code, size, off, tail, base] = columns;
            Ok(Tape { code, codes, size, base, off, tail, widths: sink.width, digest: Some(sink.digest) })
        }
        error => Err(Refusal::Simdjson(Error(error))),
    }
}

/// A sink for `kinds` whose `ctx` is `ctx`: the columns grown by `grow`, no events.
fn sink(k: &Kinds, ctx: *mut std::ffi::c_void) -> Sink {
    Sink {
        col: [std::ptr::null_mut(); 5],
        len: [0; 5],
        cap: [0; 5],
        width: [2; 3],
        ctx,
        grow,
        kind: [k.document, k.pair, k.string_content, k.escape_sequence, k.object, k.array, k.string, k.number, k.true_, k.false_, k.null, k.comment],
        key_field: k.key_field,
        value_field: k.value_field,
        codes: 0,
        code_kind: [0; 36],
        code_field: [0; 36],
        code_value: [0; 36],
        digest: [0; 32],
        ev: std::ptr::null_mut(),
        ev_len: 0,
        ev_cap: 0,
        flush: flush_events,
    }
}

/// A format 16 walk's events (`events_with`), the low byte of an event's first word; the code id above it.
pub const EV_OPEN: u32 = 0;
pub const EV_LEAF: u32 = 1;
pub const EV_CLOSE: u32 = 2;

/// The events a walk holds before it passes them on: 4,096 nodes'.
const EVENT_WORDS: usize = 3 * 4096;

/// The codes a walk has numbered so far, by id (`events_with`, `events_from_index`): kind, field, and whether in a
/// value's place.
pub struct Codes<'a> {
    kind: &'a [u16],
    field: &'a [u16],
    value: &'a [u8],
}

impl Codes<'_> {
    pub fn len(&self) -> usize {
        self.kind.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kind.is_empty()
    }

    pub fn get(&self, id: usize) -> (u16, u16, bool) {
        (self.kind[id], self.field[id], self.value[id] != 0)
    }
}

/// What `flush_events` passes the events to: the caller's.
type OnEvents<'a> = &'a mut dyn FnMut(&Codes, &[u32]);

/// `flush`: the window's events given to the caller's `on`, which `ctx` points to.
unsafe extern "C" fn flush_events(sink: *mut Sink) {
    // SAFETY: the shim passes the sink `events_with` made, whose `ctx` is its `OnEvents` and whose `ev` holds
    // `ev_len` words it wrote; `on` is apart from the sink and its buffer.
    unsafe {
        let sink: &Sink = &*sink;
        if sink.ctx.is_null() {
            return;
        }
        let on = &mut *sink.ctx.cast::<OnEvents>();
        let words = std::slice::from_raw_parts(sink.ev, sink.ev_len as usize);
        let n = sink.codes as usize;
        on(&Codes { kind: &sink.code_kind[..n], field: &sink.code_field[..n], value: &sink.code_value[..n] }, words);
    }
}

/// `src` walked by `kernel` for format 16 (decision 0434): no column is written; each node's event (`EV_OPEN`, its
/// code id and start; `EV_LEAF`, its code id, start and end; `EV_CLOSE` and its end; three words each) is given to
/// `on` in preorder, a window of them at a time, with the codes numbered so far. The walk never edits a node it
/// gave. `jsonc` reads the file as JSONC (`tape_jsonc_with`). The source's SHA-256, or why it has no tape: then `on`
/// may have had some of its events.
pub fn events_with(kernel: Kernel, src: &[u8], kinds: &Kinds, jsonc: bool, mut on: impl FnMut(&Codes, &[u32])) -> Result<[u8; 32], Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    let mut on: OnEvents = &mut on;
    let mut buf = vec![0u32; EVENT_WORDS];
    let mut sink = sink(kinds, (&raw mut on).cast());
    sink.ev = buf.as_mut_ptr();
    sink.ev_cap = EVENT_WORDS as u32;
    // SAFETY: the sink writes no column; its events go to `buf`, `ev_cap` words, and `flush_events` gives them to `on`.
    let code = unsafe { if jsonc { kernel.tape_jsonc(src, &mut sink) } else { kernel.tape(src, &mut sink) } };
    if src.len() > KEEP {
        release_above(KEEP);
    }
    match code {
        -1 => Err(Refusal::NotHere),
        0 => Ok(sink.digest),
        error => Err(Refusal::Simdjson(Error(error))),
    }
}

/// `src`'s tape by the fastest kernel this CPU runs, else from the index (`tape_from_index`, no digest).
pub fn tape(src: &[u8], kinds: &Kinds) -> Result<Tape, Refusal> {
    match Kernel::best() {
        Some(kernel) => tape_with(kernel, src, kinds),
        None => tape_from_index(src, kinds),
    }
}

/// `src`'s tape, written from simdjson's structural index after its DOM parser validated the whole document.
pub fn tape_from_index(src: &[u8], kinds: &Kinds) -> Result<Tape, Refusal> {
    if src.len() >= u32::MAX as usize {
        return Err(Refusal::TooLarge);
    }
    // simdjson skips a UTF-8 byte order mark, and its offsets start after it.
    let shift = if src.starts_with(b"\xef\xbb\xbf") { 3 } else { 0 };
    structurals(src, |index| {
        // A node an index entry, and half again for pairs, contents and escapes (the vectors grow past it).
        let guess = index.len() + index.len() / 2;
        let mut w = Writer {
            kinds,
            ids: [u16::MAX; SLOTS * 3],
            tape: Tape { code: Vec::with_capacity(guess * 2), base: Vec::with_capacity(guess / STRIDE * 4 + 4), ..Tape::default() },
            size: Vec::with_capacity(guess),
            off: Vec::with_capacity(guess),
            tail: Vec::with_capacity(guess),
            base: 0,
            last_start: 0,
            max_off: 0,
            max_tail: 0,
        };
        write(src, index, shift, &mut w)?;
        Ok(w.finish())
    })
    .map_err(Refusal::Simdjson)?
}

#[derive(Clone, Copy, PartialEq)]
enum Open {
    Object,
    Array,
    Pair,
}

/// The kinds a code names, by slot; a code is `slot * 3 + field` (0 none, 1 `key`, 2 `value`).
const DOCUMENT: usize = 0;
const PAIR: usize = 1;
const CONTENT: usize = 2;
const ESCAPE: usize = 3;
const OBJECT: usize = 4;
const ARRAY: usize = 5;
const STRING: usize = 6;
const NUMBER: usize = 7;
const TRUE: usize = 8;
const FALSE: usize = 9;
const NULL: usize = 10;
const SLOTS: usize = 11;
const NO_FIELD: usize = 0;
const KEY: usize = 1;
const VALUE: usize = 2;

/// What the index writer writes a node to (`write`): format 15's columns (`Writer`), or format 16's events
/// (`Events`).
trait Out {
    /// A node's start, of kind `slot` under `field`: its `ord`.
    fn open(&mut self, slot: usize, field: usize, start: u32) -> u32;
    /// Node `ord`'s end (the innermost open node's).
    fn close(&mut self, ord: u32, end: u32);
    /// A node with no children.
    fn leaf(&mut self, slot: usize, field: usize, start: u32, end: u32) {
        let ord = self.open(slot, field, start);
        self.close(ord, end);
    }
}

/// A code's kind and field for a slot and field (`write`'s), and whether it is in a value's place.
fn code_of(k: &Kinds, slot: usize, field: usize) -> (u16, u16, bool) {
    let kind = [k.document, k.pair, k.string_content, k.escape_sequence, k.object, k.array, k.string, k.number, k.true_, k.false_, k.null][slot];
    let field_id = [0, k.key_field, k.value_field][field];
    // A value's place: the kinds a value can be, unless keyed.
    (kind, field_id, slot >= OBJECT && field != KEY)
}

/// The index writer's format 16 events (`events_from_index`), as a kernel's walk gives them (`events_with`): a
/// window of `EVENT_WORDS` at a time to `on`, with the codes numbered so far.
struct Events<'k, 'o> {
    kinds: &'k Kinds,
    /// A code's id in this file, by `slot * 3 + field`; `u16::MAX` until first used.
    ids: [u16; SLOTS * 3],
    kind: Vec<u16>,
    field: Vec<u16>,
    value: Vec<u8>,
    words: Vec<u32>,
    n: u32,
    on: &'o mut dyn FnMut(&Codes, &[u32]),
}

impl Events<'_, '_> {
    fn event(&mut self, head: u32, a: u32, b: u32) {
        if self.words.len() + 3 > EVENT_WORDS {
            self.flush();
        }
        self.words.extend_from_slice(&[head, a, b]);
    }

    fn flush(&mut self) {
        (self.on)(&Codes { kind: &self.kind, field: &self.field, value: &self.value }, &self.words);
        self.words.clear();
    }

    fn id(&mut self, slot: usize, field: usize) -> u32 {
        let key = slot * 3 + field;
        if self.ids[key] == u16::MAX {
            self.ids[key] = self.kind.len() as u16;
            let (kind, field_id, value) = code_of(self.kinds, slot, field);
            self.kind.push(kind);
            self.field.push(field_id);
            self.value.push(u8::from(value));
        }
        u32::from(self.ids[key])
    }
}

impl Out for Events<'_, '_> {
    fn open(&mut self, slot: usize, field: usize, start: u32) -> u32 {
        let id = self.id(slot, field);
        self.event(EV_OPEN | id << 8, start, 0);
        self.n += 1;
        self.n - 1
    }

    fn close(&mut self, _ord: u32, end: u32) {
        self.event(EV_CLOSE, end, 0);
    }

    fn leaf(&mut self, slot: usize, field: usize, start: u32, end: u32) {
        let id = self.id(slot, field);
        self.event(EV_LEAF | id << 8, start, end);
        self.n += 1;
    }
}

/// `src` walked from simdjson's structural index after its DOM parser validated the whole document, for format 16:
/// each node's event given to `on` in preorder, a window at a time, as `events_with` gives a kernel's (the tape on a
/// CPU no kernel runs on). The source is not hashed.
pub fn events_from_index(src: &[u8], kinds: &Kinds, mut on: impl FnMut(&Codes, &[u32])) -> Result<(), Refusal> {
    if src.len() >= u32::MAX as usize {
        return Err(Refusal::TooLarge);
    }
    let shift = if src.starts_with(b"\xef\xbb\xbf") { 3 } else { 0 };
    structurals(src, |index| {
        let mut out = Events { kinds, ids: [u16::MAX; SLOTS * 3], kind: Vec::new(), field: Vec::new(), value: Vec::new(), words: Vec::with_capacity(EVENT_WORDS), n: 0, on: &mut on };
        write(src, index, shift, &mut out)?;
        out.flush();
        Ok(())
    })
    .map_err(Refusal::Simdjson)?
}

/// The columns as they are written: wide cells, narrowed at the end.
struct Writer<'k> {
    kinds: &'k Kinds,
    /// A code's id in this file, by `slot * 3 + field`; `u16::MAX` until first used.
    ids: [u16; SLOTS * 3],
    tape: Tape,
    size: Vec<u32>,
    off: Vec<u32>,
    tail: Vec<u32>,
    base: u32,
    /// The start of the last node written: an ending node's last descendant.
    last_start: u32,
    max_off: u32,
    max_tail: u32,
}

impl Out for Writer<'_> {
    /// Writes a node's start columns; its `ord`.
    #[inline(always)]
    fn open(&mut self, slot: usize, field: usize, start: u32) -> u32 {
        let ord = self.size.len() as u32;
        let key = slot * 3 + field;
        let mut id = self.ids[key];
        if id == u16::MAX {
            id = self.tape.codes.len() as u16;
            self.ids[key] = id;
            self.tape.codes.push(code_of(self.kinds, slot, field));
        }
        self.tape.code.extend_from_slice(&id.to_le_bytes());
        if (ord as usize).is_multiple_of(STRIDE) {
            self.base = start;
            self.tape.base.extend_from_slice(&start.to_le_bytes());
        }
        let off = start - self.base;
        self.max_off = self.max_off.max(off);
        self.off.push(off);
        self.size.push(0);
        self.tail.push(0);
        self.last_start = start;
        ord
    }

    /// Writes an ended node's end columns.
    #[inline(always)]
    fn close(&mut self, ord: u32, end: u32) {
        let tail = end - self.last_start;
        self.max_tail = self.max_tail.max(tail);
        self.size[ord as usize] = self.size.len() as u32 - ord;
        self.tail[ord as usize] = tail;
    }
}

impl Writer<'_> {
    fn finish(mut self) -> Tape {
        let narrow = |cells: &[u32], max: u32, out: &mut Vec<u8>| -> u8 {
            if max <= 0xffff {
                out.reserve(cells.len() * 2);
                out.extend(cells.iter().flat_map(|&c| (c as u16).to_le_bytes()));
                2
            } else {
                out.reserve(cells.len() * 4);
                out.extend(cells.iter().flat_map(|&c| c.to_le_bytes()));
                4
            }
        };
        let count = self.size.len() as u32;
        let t = &mut self.tape;
        t.widths = [narrow(&self.size, count, &mut t.size), narrow(&self.off, self.max_off, &mut t.off), narrow(&self.tail, self.max_tail, &mut t.tail)];
        self.tape
    }
}

/// The nodes of `src`, from its structural `index` (each offset less `shift`), written to `w` in preorder.
fn write(src: &[u8], index: &[u32], shift: u32, w: &mut impl Out) -> Result<(), Refusal> {
    let len = src.len() as u32;
    let end_before = |start: u32, next: u32| {
        let mut end = next;
        while end > start + 1 && matches!(src[end as usize - 1], b' ' | b'\t' | b'\n' | b'\r') {
            end -= 1;
        }
        end
    };
    let first = *index.first().ok_or(Refusal::Shape("an empty index"))? + shift;
    let document = w.open(DOCUMENT, NO_FIELD, first);
    let mut open: Vec<(Open, u32)> = Vec::with_capacity(64);
    // A value ended at `end`: the pair it is the value of ends there too.
    fn close_pair(open: &mut Vec<(Open, u32)>, w: &mut impl Out, end: u32) {
        if let Some(&(Open::Pair, pair)) = open.last() {
            w.close(pair, end);
            open.pop();
        }
    }
    for (i, &at) in index.iter().enumerate() {
        let at = at + shift;
        let byte = src[at as usize];
        match byte {
            b',' | b':' => continue,
            b'}' | b']' => {
                let (_, ord) = open.pop().ok_or(Refusal::Shape("a close with nothing open"))?;
                w.close(ord, at + 1);
                close_pair(&mut open, w, at + 1);
                continue;
            }
            _ => {}
        }
        let next = index.get(i + 1).map_or(len, |&n| n + shift);
        // A key (a string in an object), or a value.
        let field = match open.last() {
            None | Some(&(Open::Array, _)) => NO_FIELD,
            Some(&(Open::Pair, _)) => VALUE,
            Some(&(Open::Object, _)) => {
                let pair = w.open(PAIR, NO_FIELD, at);
                open.push((Open::Pair, pair));
                KEY
            }
        };
        match byte {
            b'{' => {
                let ord = w.open(OBJECT, field, at);
                open.push((Open::Object, ord));
            }
            b'[' => {
                let ord = w.open(ARRAY, field, at);
                open.push((Open::Array, ord));
            }
            b'"' => {
                let end = end_before(at, next);
                let string = w.open(STRING, field, at);
                let (mut from, close) = (at + 1, end - 1);
                while let Some(found) = memchr::memchr(b'\\', &src[from as usize..close as usize]) {
                    let escape = from + found as u32;
                    if escape > from {
                        w.leaf(CONTENT, NO_FIELD, from, escape);
                    }
                    w.leaf(ESCAPE, NO_FIELD, escape, escape + 2);
                    from = escape + 2;
                }
                if close > from {
                    w.leaf(CONTENT, NO_FIELD, from, close);
                }
                w.close(string, end);
                if field != KEY {
                    close_pair(&mut open, w, end);
                }
            }
            _ => {
                let slot = match byte {
                    b't' => TRUE,
                    b'f' => FALSE,
                    b'n' => NULL,
                    _ => NUMBER,
                };
                let end = end_before(at, next);
                w.leaf(slot, field, at, end);
                close_pair(&mut open, w, end);
            }
        }
    }
    if !open.is_empty() {
        return Err(Refusal::Shape("a node left open"));
    }
    w.close(document, len);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The process's instructions retired so far (`proc_pid_rusage`), for the measurements (macOS's, as they are):
    /// their difference over a run on one thread is its count. Zero where the call fails.
    #[cfg(target_os = "macos")]
    fn retired() -> u64 {
        // SAFETY: the call writes the structure, which is plain integers.
        let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
        if unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&raw mut info).cast()) } == 0 {
            return info.ri_instructions;
        }
        0
    }

    /// The kernel a measurement runs: `RERAIL_SIMDJSON_KERNEL` (its `Kernel` name, as `Haswell`), else the
    /// fastest this CPU runs.
    #[allow(dead_code, reason = "the kernel choice of the measurements, which run by hand (none is in the suite now)")]
    fn kernel_under_test() -> Kernel {
        match std::env::var("RERAIL_SIMDJSON_KERNEL") {
            Ok(name) => *Kernel::ALL.iter().find(|k| format!("{k:?}") == name).expect("a kernel's name"),
            Err(_) => Kernel::best().expect("a kernel this CPU runs"),
        }
    }

    #[test]
    fn the_index_is_each_structural_and_scalar_start() {
        let src = br#" {"a": [1, true, "x\"y"], "b": null} "#;
        let index = structurals(src, <[u32]>::to_vec).unwrap();
        let at: String = index.iter().map(|&i| src[i as usize] as char).collect();
        assert_eq!(at, r#"{":[1,t,"],":n}"#);
        assert_ne!(implementation(), "fallback");
    }

    /// The tape's cost alone, for `/usr/bin/time -l`: `RERAIL_TAPE_RUN` is `none` (read the files), `index`
    /// (parse and index) or `tape`, 10 rounds over the repository's `.json` files up to 1 MB.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "a measurement: run by name with RERAIL_TAPE_RUN"]
    #[cfg(target_os = "macos")]
    fn tape_cost() {
        let jsonc_run = std::env::var("RERAIL_TAPE_RUN").is_ok_and(|r| r.starts_with("jsonc"));
        let kinds0 = Kinds { document: 1, object: 2, pair: 3, array: 4, string: 5, string_content: 6, escape_sequence: 7, number: 8, true_: 9, false_: 10, null: 11, comment: 12, key_field: 1, value_field: 2 };
        let files = tape_cost_files(jsonc_run, &kinds0);
        let nodes: usize = files.iter().map(|f| if jsonc_run { tape_jsonc_with(kernel_under_test(), f, &kinds0).unwrap().node_count() } else { tape_from_index(f, &kinds0).unwrap().node_count() }).sum();
        eprintln!("{} bytes, {nodes} nodes", files.iter().map(Vec::len).sum::<usize>());
        let kinds = Kinds { document: 1, object: 2, pair: 3, array: 4, string: 5, string_content: 6, escape_sequence: 7, number: 8, true_: 9, false_: 10, null: 11, comment: 12, key_field: 1, value_field: 2 };
        let run = std::env::var("RERAIL_TAPE_RUN").unwrap_or_default();
        let kernel = kernel_under_test();
        let count = tape_run(&run);
        let mut n = 0usize;
        let mut best = u64::MAX;
        for _ in 0..5 {
            let i0 = retired();
            for f in &files {
                n += count(kernel, f, &kinds);
            }
            best = best.min(retired() - i0);
        }
        eprintln!("{run}: {:.1} M instructions, {:.1} a node", best as f64 / 1e6, best as f64 / nodes as f64);
        eprintln!("{} files, {n}, {} walked again ({})", files.len(), kernel.rewalks(), kernel.name());
    }

    /// The files `tape_cost` measures: the repository's `.json` files up to 1 MB, or a listing's, kept to those the
    /// run reads (with `jsonc_run`, those the strict read refuses and the JSONC read takes) and to a size.
    #[cfg(target_os = "macos")]
    fn tape_cost_files(jsonc_run: bool, kinds: &Kinds) -> Vec<Vec<u8>> {
        let mut files = Vec::new();
        for_each_json_file(1 << 20, |f| files.push(f));
        // `RERAIL_TAPE_FILES`: a listing of files (one path a line) in place of the repository's; with
        // `RERAIL_TAPE_RUN=jsonc`, the files the strict read refuses and the JSONC read takes.
        if let Ok(list) = std::env::var("RERAIL_TAPE_FILES") {
            files = std::fs::read_to_string(list).unwrap().lines().filter_map(|p| std::fs::read(p).ok()).collect();
        }
        if jsonc_run {
            let k = kernel_under_test();
            files.retain(|f| tape_with(k, f, kinds).is_err() && tape_jsonc_with(k, f, kinds).is_ok());
        } else {
            files.retain(|f| structurals(f, |_| ()).is_ok());
        }
        // `RERAIL_TAPE_SIZES`: `small` (under 52,000 bytes) or `large`.
        match std::env::var("RERAIL_TAPE_SIZES").as_deref() {
            Ok("small") => files.retain(|f| f.len() < 52_000),
            Ok("large") => files.retain(|f| f.len() >= 52_000),
            _ => {}
        }
        files
    }

    /// What a `tape_cost` run counts of a file on a kernel.
    #[cfg(target_os = "macos")]
    type TapeRun = fn(Kernel, &[u8], &Kinds) -> usize;

    /// The `RERAIL_TAPE_RUN` run named `run`: what it counts of a file (nodes, structurals, a byte of the digest); a
    /// name not listed reads the file's length.
    #[cfg(target_os = "macos")]
    fn tape_run(run: &str) -> TapeRun {
        let runs: [(&str, TapeRun); 8] = [
            ("index", |_, f, _| structurals(f, <[u32]>::len).unwrap()),
            ("tape", |kernel, f, kinds| tape_with(kernel, f, kinds).unwrap().node_count()),
            // As the product reads a JSONC file: the strict read, refused, then the JSONC one.
            ("jsonc", |kernel, f, kinds| usize::from(tape_with(kernel, f, kinds).is_err()) + tape_jsonc_with(kernel, f, kinds).unwrap().node_count()),
            ("jsonc-only", |kernel, f, kinds| tape_jsonc_with(kernel, f, kinds).unwrap().node_count()),
            ("stage1", |kernel, f, _| kernel.stage1(f, false) as usize),
            ("stage1-hashed", |kernel, f, _| kernel.stage1(f, true) as usize),
            ("index-tape", |_, f, kinds| tape_from_index(f, kinds).unwrap().node_count()),
            ("sha2", |_, f, _| {
                use sha2::Digest;
                sha2::Sha256::digest(f)[0] as usize
            }),
        ];
        let length: TapeRun = |_, f, _| f.len();
        runs.into_iter().find(|&(name, _)| name == run).map_or(length, |(_, count)| count)
    }

    /// Every kernel this CPU runs writes the index writer's tape, byte for byte, and its digest is the
    /// `sha2` crate's; each refuses what the other does: documents made to reach every node and every
    /// check, and the repository's `.json` files.
    #[test]
    fn every_kernels_tape_is_the_index_writers() {
        let kinds = Kinds { document: 1, object: 2, pair: 3, array: 4, string: 5, string_content: 6, escape_sequence: 7, number: 8, true_: 9, false_: 10, null: 11, comment: 12, key_field: 1, value_field: 2 };
        let kernels: Vec<Kernel> = Kernel::ALL.into_iter().filter(|k| k.runs()).collect();
        eprintln!("kernels checked: {:?}", kernels.iter().map(|k| k.name()).collect::<Vec<_>>());
        assert!(!kernels.is_empty());
        let same = |src: &[u8]| same_on_every_kernel(&kernels, &kinds, src);
        let made: &[&[u8]] = &[
            b"1", b" -1.5e3 ", b"true", b"false", b"null", b"\"\"", b"\"a\\nb\"", b"\xef\xbb\xbf{\"a\":1}", b"{}", b"[]", b" [ ] ",
            br#"{"a": [1, true, "x\"y"], "b": null, "c": {}, "d": [], "e": {"f": [[], {}]}}"#,
            br#"["\u00e9", "\ud83d\ude00", "\\\\", "a\/b", "\b\f\n\r\t", "\u0041\u0042x"]"#,
            br#"{"k\"ey": "v\\", "": ""}"#,
            // Refused by both.
            b"", b"{", b"[1,]", b"\"\\x\"", b"01", b"[\"\xff\"]", b"123456789012345678901234567890", br#"["\ud83d"]"#, br#"["\ude00"]"#,
            br#"["\ud83d\u0041"]"#, br#"["\u12g4"]"#, b"[tru]", b"[nul, 1]", b"{\"a\" 1}", b"[1 2]", b"1 2", b"[1.]", b"[-]",
        ];
        for src in made {
            same(src);
        }
        // The walker's own checks: structure, depth, what follows the value.
        let walker: &[&[u8]] = &[
            b"{\"a\":1,}", b"{\"a\"}", b"{\"a\":}", b"[1]]", b"{} {}", b"{\"a\":1 \"b\":2}", b"   ", b"[\"\\u", b"{1:2}", b"[,1]",
            b"[1,,2]", b"{\"a\":1]", b"[{]", b"[[]", b"{\"a\":[}]", b"[true false]", b"[nulll]", b"[-0, -0.0e-0, 1E+2, 0.5]", b"[\"a\"x]",
            b"  {  \"a\"  :  [ 1 , { } , [ ] , \"\" ]  }  ", b"\"x\" ", b"7 ", b" 7", b"-", b"[01]", b"[1e]", b"[18446744073709551615, -9223372036854775808]",
            b"[18446744073709551616]", b"[-9223372036854775809]", b"[1e400]",
        ];
        for src in walker {
            same(src);
        }
        let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        // Both refuse past simdjson's depth: some of these.
        let refused = [1023, 1024, 1025, 1026, 2000].iter().filter(|&&n| !same(deep(n).as_bytes())).count();
        assert!(refused >= 1, "no depth refused");
        // Strings past a block of 32 bytes, with escapes on either side of one's end.
        for s in escaped_strings() {
            assert!(same(&s), "{:?}", String::from_utf8_lossy(&s));
        }
        // Past 2-byte columns: a long file, and a long string (a wide `off` and `tail`).
        let long: String = format!("[{}]", (0..30_000).map(|i| format!("{{\"k{i}\": [{i}, \"v\\n\"]}}")).collect::<Vec<_>>().join(","));
        assert!(same(long.as_bytes()));
        let wide = format!("[\"{}\", 1]", "x".repeat(70_000));
        assert!(same(wide.as_bytes()));
        let mut checked = 0;
        for_each_json_file(4 << 20, |f| checked += usize::from(same(&f)));
        assert!(checked > 2000, "{checked}");
    }

    /// Whether the index writer takes `src`, each of `kernels` checked against it there: the JSONC read the strict
    /// one's, and the index writer's tape and the `sha2` crate's digest, or the index writer's refusal.
    fn same_on_every_kernel(kernels: &[Kernel], kinds: &Kinds, src: &[u8]) -> bool {
        let b = tape_from_index(src, kinds);
        let accepted = b.is_ok();
        for &kernel in kernels {
            jsonc_is_strict(kernel, kinds, src);
            kernel_is_the_index_writer(kernel, kinds, src, &b);
        }
        accepted
    }

    /// The JSONC read of a file with no comment is the strict read's, on `kernel`.
    fn jsonc_is_strict(kernel: Kernel, kinds: &Kinds, src: &[u8]) {
        if let Ok(strict) = tape_with(kernel, src, kinds) {
            let jsonc = tape_jsonc_with(kernel, src, kinds).unwrap_or_else(|e| panic!("{}: JSONC refused {:?}: {e}", kernel.name(), String::from_utf8_lossy(src)));
            let without_slash = !src.contains(&b'/') || src.windows(2).all(|w| w != b"//" && w != b"/*");
            if without_slash {
                assert_eq!((&jsonc.code, &jsonc.codes, &jsonc.size, &jsonc.off, &jsonc.tail, jsonc.digest), (&strict.code, &strict.codes, &strict.size, &strict.off, &strict.tail, strict.digest), "{}: JSONC {:?}", kernel.name(), String::from_utf8_lossy(src));
            }
        }
    }

    /// `kernel`'s tape of `src` is the index writer's `b` with the `sha2` crate's digest, or both refuse `src`.
    fn kernel_is_the_index_writer(kernel: Kernel, kinds: &Kinds, src: &[u8], b: &Result<Tape, Refusal>) {
        match (tape_with(kernel, src, kinds), b) {
            (Ok(a), Ok(b)) => {
                use sha2::Digest;
                assert_eq!(a.digest, Some(<[u8; 32]>::from(sha2::Sha256::digest(src))), "{}: the digest of {:?}", kernel.name(), String::from_utf8_lossy(src));
                assert_eq!((&a.code, &a.codes, &a.size, &a.base, &a.off, &a.tail, a.widths), (&b.code, &b.codes, &b.size, &b.base, &b.off, &b.tail, b.widths), "{}: {:?}", kernel.name(), String::from_utf8_lossy(src));
            }
            (Err(Refusal::Simdjson(_)), Err(Refusal::Simdjson(_))) => {}
            (a, b) => panic!("{}: {:?}: {:?} against {:?}", kernel.name(), String::from_utf8_lossy(src), a.map(|t| t.node_count()), b.as_ref().map(|t| t.node_count())),
        }
    }

    /// Strings past a block of 32 bytes, with escapes on either side of one's end, each in an array.
    fn escaped_strings() -> Vec<Vec<u8>> {
        let mut docs = Vec::new();
        for n in [0, 1, 15, 16, 30, 31, 32, 33, 63, 64, 65, 100] {
            for esc in [&b""[..], b"\\n", b"\\u00e9", b"\\\\"] {
                let mut s = b"[\"".to_vec();
                s.extend(std::iter::repeat_n(b'x', n));
                s.extend_from_slice(esc);
                s.extend(std::iter::repeat_n(b'y', n / 2));
                s.extend_from_slice(b"\", 1]");
                docs.push(s);
            }
        }
        docs
    }

    /// Calls `each` with every `.json` file of the repository up to `max_len` bytes (`.git`, `target` and
    /// `node_modules` passed over).
    fn for_each_json_file(max_len: u64, mut each: impl FnMut(Vec<u8>)) {
        let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../../..");
        let mut work = vec![root];
        while let Some(dir) = work.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                visit_json_entry(&e, max_len, &mut work, &mut each);
            }
        }
    }

    /// One entry of `for_each_json_file`'s walk: a directory is added to `work`; a `.json` file up to `max_len`
    /// bytes is read and given to `each`.
    fn visit_json_entry(e: &std::fs::DirEntry, max_len: u64, work: &mut Vec<std::path::PathBuf>, each: &mut impl FnMut(Vec<u8>)) {
        let p = e.path();
        let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
        if p.is_dir() && !skip {
            work.push(p);
        } else if p.extension().is_some_and(|x| x == "json")
            && e.metadata().is_ok_and(|m| m.len() <= max_len)
            && let Ok(f) = std::fs::read(&p)
        {
            each(f);
        }
    }

    /// Instructions a node by the document's shape: simdjson's DOM parse, stage 1, and the v9np stage 2.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "a measurement: run by name with --nocapture"]
    #[cfg(target_os = "macos")]
    fn shape_costs() {
        let kinds = Kinds { document: 1, object: 2, pair: 3, array: 4, string: 5, string_content: 6, escape_sequence: 7, number: 8, true_: 9, false_: 10, null: 11, comment: 12, key_field: 1, value_field: 2 };
        let kernel = kernel_under_test();
        let n = 20_000;
        let join = |f: &dyn Fn(usize) -> String| (0..n).map(f).collect::<Vec<_>>().join(",");
        let shapes: Vec<(&str, String)> = vec![
            ("ints", format!("[{}]", join(&|i| format!("{}", i % 1000)))),
            ("trues", format!("[{}]", join(&|_| "true".into()))),
            ("short strings", format!("[{}]", join(&|i| format!("\"s{}\"", i % 100)))),
            ("empty strings", format!("[{}]", join(&|_| "\"\"".into()))),
            ("int pairs", format!("{{{}}}", join(&|i| format!("\"k{i}\":{}", i % 1000)))),
            ("objects", format!("[{}]", join(&|i| format!("{{\"a\":{i},\"b\":\"x\"}}")))),
            ("nested arrays", format!("[{}]", join(&|_| "[[1],[2]]".into()))),
        ];
        for (name, doc) in &shapes {
            let d = doc.as_bytes();
            let nodes = tape(d, &kinds).unwrap().node_count();
            let measure = |f: &dyn Fn() -> usize| {
                let mut best = u64::MAX;
                for _ in 0..20 {
                    let i0 = retired();
                    std::hint::black_box(f());
                    best = best.min(retired() - i0);
                }
                best as f64 / nodes as f64
            };
            let s1 = measure(&|| kernel.stage1(d, false) as usize);
            let hashed = measure(&|| kernel.stage1(d, true) as usize);
            let dom = measure(&|| structurals(d, <[u32]>::len).unwrap());
            let ours = measure(&|| tape_with(kernel, d, &kinds).unwrap().node_count());
            eprintln!(
                "{name:14} {nodes:7} nodes: stage 1 {s1:5.1}, SHA-256 in it {:5.1}, DOM stage 2 {:5.1}, v9np stage 2 {:5.1} a node",
                hashed - s1,
                dom - s1,
                ours - hashed
            );
        }
    }

    /// The JSONC read: comments as `comment` leaves of the lowest node holding the tokens on either side
    /// (tree-sitter-json's placement, checked against tree-sitter-json in the engine's
    /// `json_tapes_from_simdjson_are_tree_sitters`), the digest the source's as given, and what is not a
    /// comment refused.
    #[test]
    fn jsonc_comments_are_leaves_where_tree_sitter_places_them() {
        use sha2::Digest;
        let kinds = Kinds { document: 1, object: 2, pair: 3, array: 4, string: 5, string_content: 6, escape_sequence: 7, number: 8, true_: 9, false_: 10, null: 11, comment: 12, key_field: 1, value_field: 2 };
        for kernel in Kernel::ALL.into_iter().filter(|k| k.runs()) {
            // (kind, start, end) of each node in preorder.
            let nodes = |src: &str| -> Vec<(u16, u32, u32)> {
                let t = tape_jsonc_with(kernel, src.as_bytes(), &kinds).unwrap_or_else(|e| panic!("{}: {src:?}: {e}", kernel.name()));
                assert_eq!(t.digest, Some(<[u8; 32]>::from(sha2::Sha256::digest(src.as_bytes()))));
                assert_eq!(t.widths, [2, 2, 2]);
                let n = t.node_count();
                let u16s = |v: &[u8], i: usize| u16::from_le_bytes([v[2 * i], v[2 * i + 1]]) as u32;
                let base = |i: usize| u32::from_le_bytes(t.base[4 * (i / STRIDE)..4 * (i / STRIDE) + 4].try_into().unwrap());
                (0..n)
                    .map(|i| {
                        let start = base(i) + u16s(&t.off, i);
                        // A node's end: its last descendant's start plus its tail.
                        let last = i + u16s(&t.size, i) as usize - 1;
                        let end = base(last) + u16s(&t.off, last) + u16s(&t.tail, i);
                        (t.codes[u16s(&t.code, i) as usize].0, start, end)
                    })
                    .collect()
            };
            // Before a key, after a key, after `:`, after a value, after `,`, in an array, around the document.
            let src = "// a\n{ /*b*/ \"k\" /*c*/ : /*d*/ 1 /*e*/ , /*f*/ \"j\": [ /*g*/ 2 /*h*/ , 3 /*i*/ ] /*j*/ } // k\n";
            let (doc, obj, pair, arr, string, content, num, com) = (1, 2, 3, 4, 5, 6, 8, 12);
            assert_eq!(
                nodes(src),
                vec![
                    (doc, 0, 93), (com, 0, 4), (obj, 5, 87), (com, 7, 12), (pair, 13, 32), (string, 13, 16), (content, 14, 15),
                    (com, 17, 22), (com, 25, 30), (num, 31, 32), (com, 33, 38), (com, 41, 46), (pair, 47, 79), (string, 47, 50),
                    (content, 48, 49), (arr, 52, 79), (com, 54, 59), (num, 60, 61), (com, 62, 67), (num, 70, 71), (com, 72, 77),
                    (com, 80, 85), (com, 88, 92),
                ],
                "{}",
                kernel.name()
            );
            // Comments in empty containers, a `\r` kept in a line comment, a byte order mark, `/*` in a string.
            assert_eq!(nodes("[ /*y*/ ]"), vec![(doc, 0, 9), (arr, 0, 9), (com, 2, 7)]);
            assert_eq!(nodes("{\"a\": 1 // c\r\n}")[6], (com, 8, 13));
            assert_eq!(nodes("\u{feff}// x\n{}"), vec![(doc, 3, 10), (com, 3, 7), (obj, 8, 10)]);
            assert_eq!(nodes("[\"/*x*/\", 1] /* a \" quote */"), vec![(doc, 0, 28), (arr, 0, 12), (string, 1, 8), (content, 2, 7), (num, 10, 11), (com, 13, 28)]);
            // A comment past a block of 64 bytes, a quote in it, then a string.
            let long = format!("[{}] // \"\n", "1,".repeat(40) + "\"//\"");
            assert!(nodes(&long).iter().any(|&(k, _, _)| k == com));
            // A trailing comma is passed over: the tape is the one the file has with a space for it.
            for (with, without) in [
                ("[1,]", "[1 ]"),
                ("{\"a\": 1,}", "{\"a\": 1 }"),
                ("[[1,],]", "[[1 ] ]"),
                ("{\"a\": {\"b\": 1,},}", "{\"a\": {\"b\": 1 } }"),
                ("[\"x\",\n]", "[\"x\" \n]"),
                ("[{\"a\": 1}, // c\n]", "[{\"a\": 1}  // c\n]"),
                ("{\"a\": 1, /* c */ }", "{\"a\": 1  /* c */ }"),
                ("{\"a\": [1, 2,], \"b\": {},}", "{\"a\": [1, 2 ], \"b\": {} }"),
            ] {
                assert_eq!(nodes(with), nodes(without), "{}: {with:?}", kernel.name());
            }
            // Not comments, or not JSONC this read takes: refused.
            for bad in ["[1 / 2]", "/* open", "[1, /* open ]", "1 /", "// only a comment", "[1,,]", "[,1]", "[,]", "{,}", "{\"a\": 1,,}", "{\"a\": 1,]", "[1,}"] {
                assert!(tape_jsonc_with(kernel, bad.as_bytes(), &kinds).is_err(), "{}: {bad:?}", kernel.name());
            }
        }
    }

    #[test]
    fn invalid_documents_are_refused() {
        for bad in [&b""[..], b"{", b"[1,]", b"// c\n1", b"\"\\x\"", b"01", b"[\"\xff\"]", b"123456789012345678901234567890"] {
            assert!(structurals(bad, |_| ()).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
        assert!(structurals(b"\xef\xbb\xbf{}", |_| ()).is_ok());
    }
}
