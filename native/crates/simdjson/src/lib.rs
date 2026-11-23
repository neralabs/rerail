//! simdjson (the C++ library, vendored at `native/vendor/simdjson-5.0.5`) and the engine's JSON tape.
//!
//! The portable side: the native code (simdjson, the kernels) is `rerail-simdjson-native`'s.
//!
//! `events_with` is a kernel's: simdjson's stage 1, the source hashed in it, and a stage 2 of ours, which
//! walks the index itself, validates the document as simdjson's does, and gives each node of the tape, in
//! preorder, as format 16's event (decision 0434): the caller passes them to format 16's writer, which writes
//! the tape. `structurals` is simdjson's DOM parser and the structural index its stage 1 found
//! (`events_from_index`, the kernels' oracle and the tape on a CPU no kernel runs on, walks it).
//!
//! The index holds, in order, the offset of each structural byte (`{ } [ ] : ,`) and of each
//! scalar's first byte (a string's opening quote, a number's or a literal's first byte). Each
//! thread parses with a parser of its own.

pub use rerail_simdjson_native::{Error, Kernel, implementation, structurals};
use rerail_simdjson_native::{KEEP, Sink, release_above};

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
    /// tree-sitter-json's `comment`: a JSONC file's (`events_with`'s `jsonc`).
    pub comment: u16,
    pub key_field: u16,
    pub value_field: u16,
}

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

/// A sink for `kinds` whose `ctx` is `ctx`, the events to `ev`'s `cap` words.
fn sink(k: &Kinds, ctx: *mut std::ffi::c_void, ev: &mut [u32]) -> Sink {
    Sink {
        ctx,
        kind: [k.document, k.pair, k.string_content, k.escape_sequence, k.object, k.array, k.string, k.number, k.true_, k.false_, k.null, k.comment],
        key_field: k.key_field,
        value_field: k.value_field,
        codes: 0,
        code_kind: [0; 36],
        code_field: [0; 36],
        code_value: [0; 36],
        digest: [0; 32],
        ev: ev.as_mut_ptr(),
        ev_len: 0,
        ev_cap: ev.len() as u32,
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
    // SAFETY: the shim passes the sink `events_as` made, whose `ctx` is its `OnEvents` and whose `ev` holds
    // `ev_len` words it wrote; `on` is apart from the sink and its buffer.
    unsafe {
        let sink: &Sink = &*sink;
        let on = &mut *sink.ctx.cast::<OnEvents>();
        let words = std::slice::from_raw_parts(sink.ev, sink.ev_len as usize);
        let n = sink.codes as usize;
        on(&Codes { kind: &sink.code_kind[..n], field: &sink.code_field[..n], value: &sink.code_value[..n] }, words);
    }
}

/// How a kernel reads its source.
#[derive(Clone, Copy, PartialEq)]
enum Read {
    Strict,
    Jsonc,
    Unhashed,
}

/// `src` walked by `kernel` for format 16 (decision 0434): each node's event (`EV_OPEN`, its code id and start;
/// `EV_LEAF`, its code id, start and end; `EV_CLOSE` and its end; three words each) is given to `on` in preorder,
/// a window of them at a time, with the codes numbered so far. The walk never edits a node it gave:
/// - stage 1 is simdjson's (the kernel's implementation's), each 64-byte block of the source also hashed (with the
///   CPU's SHA-256 instructions, where the kernel has them);
/// - stage 2 walks the index itself, as simdjson's walker does, and checks each scalar as simdjson's tape builder
///   does (escapes, numbers, atoms). No DOM tape, no unescaped strings, no number values.
///
/// `jsonc` reads the file as JSONC: each comment (`//` to the line's end, its `\n` not included; `/* … */`) is also
/// a node, a `comment` leaf of the lowest node holding the tokens on either side of it, as tree-sitter-json places
/// it (an extra). Stage 1 reads the source with its comments blanked; the digest is the source's as given. A
/// trailing comma (before `]` or `}`) is passed over, as TypeScript reads JSONC: the tape is the one the file has
/// without it. For a file the strict read refused: a strict file's events are the strict read's.
///
/// The source's SHA-256, or why it has no tape: then `on` may have had some of its events. The events are
/// `events_from_index`'s (`every_kernels_walk_is_the_index_writers`). `Refusal::NotHere` where this CPU does not
/// run the kernel.
pub fn events_with(kernel: Kernel, src: &[u8], kinds: &Kinds, jsonc: bool, on: impl FnMut(&Codes, &[u32])) -> Result<[u8; 32], Refusal> {
    events_as(kernel, src, kinds, if jsonc { Read::Jsonc } else { Read::Strict }, on)
}

/// `events_with`, the source not hashed (the digest zeros): for a measurement, against a reader that does not hash
/// (`rerail-json-lean`).
pub fn events_unhashed_with(kernel: Kernel, src: &[u8], kinds: &Kinds, on: impl FnMut(&Codes, &[u32])) -> Result<(), Refusal> {
    events_as(kernel, src, kinds, Read::Unhashed, on).map(|_| ())
}

fn events_as(kernel: Kernel, src: &[u8], kinds: &Kinds, read: Read, mut on: impl FnMut(&Codes, &[u32])) -> Result<[u8; 32], Refusal> {
    if src.len() >= u32::MAX as usize / 2 {
        return Err(Refusal::TooLarge);
    }
    let mut on: OnEvents = &mut on;
    let mut buf = vec![0u32; EVENT_WORDS];
    let mut sink = sink(kinds, (&raw mut on).cast(), &mut buf);
    // SAFETY: the sink's events go to `buf`, `ev_cap` words, and `flush_events` gives them to `on`.
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
        0 => Ok(sink.digest),
        error => Err(Refusal::Simdjson(Error(error))),
    }
}

/// A walk's events held whole, to compare two walks or count a walk's nodes (a test's, a measurement's): the
/// engine gives the events to format 16's writer as they come (`events_with`) and holds none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Walk {
    /// The file's codes, in first-use order: kind, field, and whether the node is in a value's place
    /// (tree-sitter-json's `_value` supertype: the document's child, an array element, a pair's value).
    pub codes: Vec<(u16, u16, bool)>,
    /// The events, three words each (`events_with`'s).
    pub events: Vec<u32>,
    /// The source's SHA-256, where the pass that read it computed it (a kernel's, not `walk_from_index`).
    pub digest: Option<[u8; 32]>,
}

impl Walk {
    /// The walk's nodes: its events but closes.
    pub fn node_count(&self) -> usize {
        self.events.as_chunks::<3>().0.iter().filter(|e| e[0] & 0xff != EV_CLOSE).count()
    }

    /// The nodes in preorder: (kind, start, end) of each, its end from its close.
    pub fn nodes(&self) -> Vec<(u16, u32, u32)> {
        let (mut nodes, mut open) = (Vec::new(), Vec::new());
        for e in self.events.as_chunks::<3>().0 {
            let kind = || self.codes[(e[0] >> 8) as usize].0;
            match e[0] & 0xff {
                EV_OPEN => {
                    open.push(nodes.len());
                    nodes.push((kind(), e[1], 0));
                }
                EV_LEAF => nodes.push((kind(), e[1], e[2])),
                _ => nodes[open.pop().expect("a close of an open node")].2 = e[1],
            }
        }
        nodes
    }

    /// A walk given to `on` (`events_with`'s), held: its events, with the codes the last window had.
    fn of(walk: impl FnOnce(&mut dyn FnMut(&Codes, &[u32])) -> Result<Option<[u8; 32]>, Refusal>) -> Result<Walk, Refusal> {
        let mut w = Walk::default();
        let digest = walk(&mut |codes, words| {
            w.codes = (0..codes.len()).map(|i| codes.get(i)).collect();
            w.events.extend_from_slice(words);
        })?;
        w.digest = digest;
        Ok(w)
    }
}

/// `events_with`'s walk of `src` by `kernel`, held, with its digest.
pub fn walk_with(kernel: Kernel, src: &[u8], kinds: &Kinds, jsonc: bool) -> Result<Walk, Refusal> {
    Walk::of(|on| events_with(kernel, src, kinds, jsonc, on).map(Some))
}

/// `events_unhashed_with`'s walk, held, with no digest.
pub fn walk_unhashed_with(kernel: Kernel, src: &[u8], kinds: &Kinds) -> Result<Walk, Refusal> {
    Walk::of(|on| events_unhashed_with(kernel, src, kinds, on).map(|()| None))
}

/// `events_from_index`'s walk, held, with no digest.
pub fn walk_from_index(src: &[u8], kinds: &Kinds) -> Result<Walk, Refusal> {
    Walk::of(|on| events_from_index(src, kinds, on).map(|()| None))
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

    /// A node's start, of kind `slot` under `field`.
    fn open(&mut self, slot: usize, field: usize, start: u32) {
        let id = self.id(slot, field);
        self.event(EV_OPEN | id << 8, start, 0);
    }

    /// The innermost open node's end.
    fn close(&mut self, end: u32) {
        self.event(EV_CLOSE, end, 0);
    }

    /// A node with no children.
    fn leaf(&mut self, slot: usize, field: usize, start: u32, end: u32) {
        let id = self.id(slot, field);
        self.event(EV_LEAF | id << 8, start, end);
    }
}

/// `src` walked from simdjson's structural index after its DOM parser validated the whole document, for format 16:
/// each node's event given to `on` in preorder, a window at a time, as `events_with` gives a kernel's (the tape on a
/// CPU no kernel runs on). The source is not hashed.
pub fn events_from_index(src: &[u8], kinds: &Kinds, mut on: impl FnMut(&Codes, &[u32])) -> Result<(), Refusal> {
    if src.len() >= u32::MAX as usize {
        return Err(Refusal::TooLarge);
    }
    // simdjson skips a UTF-8 byte order mark, and its offsets start after it.
    let shift = if src.starts_with(b"\xef\xbb\xbf") { 3 } else { 0 };
    structurals(src, |index| {
        let mut out = Events { kinds, ids: [u16::MAX; SLOTS * 3], kind: Vec::new(), field: Vec::new(), value: Vec::new(), words: Vec::with_capacity(EVENT_WORDS), on: &mut on };
        write(src, index, shift, &mut out)?;
        out.flush();
        Ok(())
    })
    .map_err(Refusal::Simdjson)?
}

/// The nodes of `src`, from its structural `index` (each offset less `shift`), given to `w` in preorder.
fn write(src: &[u8], index: &[u32], shift: u32, w: &mut Events) -> Result<(), Refusal> {
    let len = src.len() as u32;
    let end_before = |start: u32, next: u32| {
        let mut end = next;
        while end > start + 1 && matches!(src[end as usize - 1], b' ' | b'\t' | b'\n' | b'\r') {
            end -= 1;
        }
        end
    };
    let first = *index.first().ok_or(Refusal::Shape("an empty index"))? + shift;
    w.open(DOCUMENT, NO_FIELD, first);
    let mut open: Vec<Open> = Vec::with_capacity(64);
    // A value ended at `end`: the pair it is the value of ends there too.
    fn close_pair(open: &mut Vec<Open>, w: &mut Events, end: u32) {
        if open.last() == Some(&Open::Pair) {
            w.close(end);
            open.pop();
        }
    }
    // The close of an empty object or array, written with it as one leaf.
    let mut skip = false;
    for (i, &at) in index.iter().enumerate() {
        if std::mem::take(&mut skip) {
            continue;
        }
        let at = at + shift;
        let byte = src[at as usize];
        match byte {
            b',' | b':' => continue,
            b'}' | b']' => {
                open.pop().ok_or(Refusal::Shape("a close with nothing open"))?;
                w.close(at + 1);
                close_pair(&mut open, w, at + 1);
                continue;
            }
            _ => {}
        }
        let next = index.get(i + 1).map_or(len, |&n| n + shift);
        // A key (a string in an object), or a value.
        let field = match open.last() {
            None | Some(Open::Array) => NO_FIELD,
            Some(Open::Pair) => VALUE,
            Some(Open::Object) => {
                w.open(PAIR, NO_FIELD, at);
                open.push(Open::Pair);
                KEY
            }
        };
        match byte {
            // An empty object or array has no children: a leaf to just past its close, as the kernels write it.
            b'{' | b'[' if next < len && src[next as usize] == if byte == b'{' { b'}' } else { b']' } => {
                w.leaf(if byte == b'{' { OBJECT } else { ARRAY }, field, at, next + 1);
                skip = true;
                close_pair(&mut open, w, next + 1);
            }
            b'{' => {
                w.open(OBJECT, field, at);
                open.push(Open::Object);
            }
            b'[' => {
                w.open(ARRAY, field, at);
                open.push(Open::Array);
            }
            // An empty string has no children: a leaf, as the kernels write it.
            b'"' if end_before(at, next) == at + 2 => {
                w.leaf(STRING, field, at, at + 2);
                if field != KEY {
                    close_pair(&mut open, w, at + 2);
                }
            }
            b'"' => {
                let end = end_before(at, next);
                w.open(STRING, field, at);
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
                w.close(end);
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
    w.close(len);
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

    /// The nodes of a walk by `kernel` (`events_with`), counted as they come: a measurement's.
    #[cfg(target_os = "macos")]
    fn counted(kernel: Kernel, f: &[u8], kinds: &Kinds, jsonc: bool) -> Result<usize, Refusal> {
        let mut n = 0;
        events_with(kernel, f, kinds, jsonc, |_, w| n += w.chunks_exact(3).filter(|e| e[0] & 0xff != EV_CLOSE).count())?;
        Ok(n)
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
        let nodes: usize = files.iter().map(|f| if jsonc_run { counted(kernel_under_test(), f, &kinds0, true).unwrap() } else { walk_from_index(f, &kinds0).unwrap().node_count() }).sum();
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
        eprintln!("{} files, {n} ({})", files.len(), kernel.name());
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
            files.retain(|f| counted(k, f, kinds, false).is_err() && counted(k, f, kinds, true).is_ok());
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
            ("tape", |kernel, f, kinds| counted(kernel, f, kinds, false).unwrap()),
            // As the product reads a JSONC file: the strict read, refused, then the JSONC one.
            ("jsonc", |kernel, f, kinds| usize::from(counted(kernel, f, kinds, false).is_err()) + counted(kernel, f, kinds, true).unwrap()),
            ("jsonc-only", |kernel, f, kinds| counted(kernel, f, kinds, true).unwrap()),
            ("stage1", |kernel, f, _| kernel.stage1(f, false) as usize),
            ("stage1-hashed", |kernel, f, _| kernel.stage1(f, true) as usize),
            ("index-tape", |_, f, kinds| {
                let mut n = 0;
                events_from_index(f, kinds, |_, w| n += w.len() / 3).unwrap();
                n
            }),
            ("sha2", |_, f, _| {
                use sha2::Digest;
                sha2::Sha256::digest(f)[0] as usize
            }),
        ];
        let length: TapeRun = |_, f, _| f.len();
        runs.into_iter().find(|&(name, _)| name == run).map_or(length, |(_, count)| count)
    }

    /// Every kernel this CPU runs gives the index writer's events, word for word, and its digest is the
    /// `sha2` crate's; each refuses what the other does: documents made to reach every node and every
    /// check, and the repository's `.json` files.
    #[test]
    fn every_kernels_walk_is_the_index_writers() {
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
        // A long file (past 65,536 nodes), and a long string (past 65,536 bytes).
        let long: String = format!("[{}]", (0..30_000).map(|i| format!("{{\"k{i}\": [{i}, \"v\\n\"]}}")).collect::<Vec<_>>().join(","));
        assert!(same(long.as_bytes()));
        let wide = format!("[\"{}\", 1]", "x".repeat(70_000));
        assert!(same(wide.as_bytes()));
        let mut checked = 0;
        for_each_json_file(4 << 20, |f| checked += usize::from(same(&f)));
        assert!(checked > 2000, "{checked}");
    }

    /// Whether the index writer takes `src`, each of `kernels` checked against it there: the JSONC read the strict
    /// one's, and the index writer's events and the `sha2` crate's digest, or the index writer's refusal.
    fn same_on_every_kernel(kernels: &[Kernel], kinds: &Kinds, src: &[u8]) -> bool {
        let b = walk_from_index(src, kinds);
        let accepted = b.is_ok();
        for &kernel in kernels {
            jsonc_is_strict(kernel, kinds, src);
            kernel_is_the_index_writer(kernel, kinds, src, &b);
        }
        accepted
    }

    /// The JSONC read of a file with no comment is the strict read's, on `kernel`.
    fn jsonc_is_strict(kernel: Kernel, kinds: &Kinds, src: &[u8]) {
        if let Ok(strict) = walk_with(kernel, src, kinds, false) {
            let jsonc = walk_with(kernel, src, kinds, true).unwrap_or_else(|e| panic!("{}: JSONC refused {:?}: {e}", kernel.name(), String::from_utf8_lossy(src)));
            let without_slash = !src.contains(&b'/') || src.windows(2).all(|w| w != b"//" && w != b"/*");
            if without_slash {
                assert_eq!(jsonc, strict, "{}: JSONC {:?}", kernel.name(), String::from_utf8_lossy(src));
            }
        }
    }

    /// `kernel`'s walk of `src` is the index writer's `b` with the `sha2` crate's digest, or both refuse `src`.
    fn kernel_is_the_index_writer(kernel: Kernel, kinds: &Kinds, src: &[u8], b: &Result<Walk, Refusal>) {
        match (walk_with(kernel, src, kinds, false), b) {
            (Ok(a), Ok(b)) => {
                use sha2::Digest;
                assert_eq!(a.digest, Some(<[u8; 32]>::from(sha2::Sha256::digest(src))), "{}: the digest of {:?}", kernel.name(), String::from_utf8_lossy(src));
                assert_eq!((&a.codes, &a.events), (&b.codes, &b.events), "{}: {:?}", kernel.name(), String::from_utf8_lossy(src));
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
            let nodes = walk_from_index(d, &kinds).unwrap().node_count();
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
            let ours = measure(&|| counted(kernel, d, &kinds, false).unwrap());
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
                let t = walk_with(kernel, src.as_bytes(), &kinds, true).unwrap_or_else(|e| panic!("{}: {src:?}: {e}", kernel.name()));
                assert_eq!(t.digest, Some(<[u8; 32]>::from(sha2::Sha256::digest(src.as_bytes()))));
                t.nodes()
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
                assert!(walk_with(kernel, bad.as_bytes(), &kinds, true).is_err(), "{}: {bad:?}", kernel.name());
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
