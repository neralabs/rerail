//! The preprocessor tape (decision 0436), written by the lean reading: the reference the Bitlex
//! grammar's tape is held to.
//!
//! A file's tape is a format 16 document (decision 0468 §1). Every preprocessing token of the file is a leaf, exactly
//! once and in order, of a token kind (`identifier`, `number`, `literal`, `punct`, `digraph`,
//! `hash`, `hashhash`, `other`). The nodes over the leaves are:
//!
//! - a directive, named by its word (`define`, `include`, …, `null`, `unknown`, `line_marker`),
//!   from its `#` to its line's last token, its `#` and its word leaves of it;
//! - a conditional group, from its `#if…` to its `#endif` (the file's end where it has none),
//!   its chain its own directive children;
//! - `code`, a line of text: the tokens from one at a line's start (the lexer's `BOL`) to the
//!   next such, or to a directive;
//! - an operand, a field's node over its tokens (`params`, `header`, `macro_header`, `operands`,
//!   `pragma_text`, `line_rest`, `marker_rest`), or a field on a token's leaf where the operand is
//!   one token (`name`, `param`, `variadic`, a quoted `spelled`, `first`, `second`, `n`, `file`).
//!
//! The tokens are the lean lexer's (`rerail_c_lean::pp::PpLexer` over the phase-2 text, mapped to
//! the file's bytes), the directives and their operands the lean scan's (`directives::scan`).

use crate::directives::{self, Form, Kind, Span, Variadic};
use rerail_c_lean::pp::{DIGRAPH, Options, P, Phase2, PpLexer};
use rerail_tape_format::f16::{Lang, Tape16, Written};
use rerail_tape_format::tape::{CodeTable, Entry};
use rerail_tape_format::tape::Tape as _;
use rerail_tape_format::{Redo, Sixteen, Target, Widths, Writer, key};

/// The tape's kinds, by id.
pub const KINDS: &[&str] = &[
    "pp", "code", "group", "null", "unknown", "line_marker", "define", "undef", "include", "include_next", "import", "embed", "if", "ifdef", "ifndef", "elif",
    "elifdef", "elifndef", "else", "endif", "line", "error", "warning", "pragma", "ident", "sccs", "assert", "unassert", "identifier", "number", "literal",
    "punct", "digraph", "hash", "hashhash", "other", "params", "header", "macro_header", "operands", "pragma_text", "line_rest", "marker_rest",
];

/// The operands' fields, by id less one (0: no field).
pub const FIELDS: &[&str] = &["name", "params", "param", "variadic", "body", "spelled", "expr", "text", "first", "second", "n", "file", "rest"];

pub const PP: u16 = 0;
pub const CODE: u16 = 1;
pub const GROUP: u16 = 2;
/// The first directive kind (`null`); the directive kinds run to `IDENTIFIER`.
pub const DIRECTIVES: u16 = 3;
pub const IDENTIFIER: u16 = 28;
pub const NUMBER: u16 = 29;
pub const LITERAL: u16 = 30;
pub const PUNCT: u16 = 31;
pub const DIGRAPH_KIND: u16 = 32;
pub const HASH: u16 = 33;
pub const HASHHASH: u16 = 34;
pub const OTHER: u16 = 35;
pub const PARAMS: u16 = 36;
pub const HEADER: u16 = 37;
pub const MACRO_HEADER: u16 = 38;
pub const OPERANDS: u16 = 39;
pub const PRAGMA_TEXT: u16 = 40;
pub const LINE_REST: u16 = 41;
pub const MARKER_REST: u16 = 42;

/// A field's id, by name.
fn field(name: &str) -> u16 {
    FIELDS.iter().position(|&f| f == name).expect("a field of the tape") as u16 + 1
}

/// A directive kind's id.
pub fn kind_id(k: Kind) -> u16 {
    let name = k.name();
    KINDS[DIRECTIVES as usize..IDENTIFIER as usize].iter().position(|&n| n == name).expect("a directive kind of the tape") as u16 + DIRECTIVES
}

/// A token: its kind (`IDENTIFIER` to `OTHER`), its bytes (the file's) and the lexer's flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: u16,
    pub start: u32,
    pub end: u32,
    pub flags: u8,
}

/// Every preprocessing token of `src`, the lean lexer's, at the file's bytes.
pub fn tokens(src: &[u8], opts: Options) -> Vec<Token> {
    let p2 = Phase2::of(src, opts.trigraphs);
    let mut lx = PpLexer::new(&p2.text, opts);
    let mut out = Vec::new();
    loop {
        let t = lx.next_token();
        let kind = match t.kind {
            P::Eof => break,
            P::Ident => IDENTIFIER,
            P::Number => NUMBER,
            P::Char | P::Str | P::RawStr => LITERAL,
            P::Hash => HASH,
            P::HashHash => HASHHASH,
            P::Punct if t.flags & DIGRAPH != 0 => DIGRAPH_KIND,
            P::Punct => PUNCT,
            P::Header | P::Other => OTHER,
        };
        let (s, e) = (t.start as usize, t.end as usize);
        out.push(Token { kind, start: p2.orig(s) as u32, end: if e > s { p2.orig(e - 1) as u32 + 1 } else { p2.orig(s) as u32 }, flags: t.flags });
    }
    out
}

/// A node to write: its bytes, its rank among nodes of the same bytes (a group holds its
/// opener, a directive or a line its leaves, a field's node its leaves), and its key.
struct Node {
    start: u32,
    end: u32,
    rank: u8,
    key: u32,
}

/// The nodes of a file, gathered before they are written in preorder.
struct Gather<'a> {
    toks: &'a [Token],
    src: &'a [u8],
    nodes: Vec<Node>,
    /// Each token's field (0: none).
    fields: Vec<u16>,
}

impl Gather<'_> {
    /// The index of the first token at or after the file's byte `at`.
    fn at(&self, at: u32) -> usize {
        self.toks.partition_point(|t| t.start < at)
    }

    /// The tokens of the file's bytes `s..e`: those that start inside them.
    fn range(&self, (s, e): (u32, u32)) -> (usize, usize) {
        (self.at(s), self.at(e))
    }

    /// A node over tokens `a..b`, from the first's start to the last's end; none where empty.
    fn over(&mut self, (a, b): (usize, usize), kind: u16, field: u16) {
        if a < b {
            self.nodes.push(Node { start: self.toks[a].start, end: self.toks[b - 1].end, rank: 2, key: key(kind, field, 0) });
        }
    }

    /// The field `field` on the token that starts at the file's byte `at`.
    fn leaf(&mut self, at: u32, field: u16) {
        let i = self.at(at);
        if i < self.toks.len() && self.toks[i].start == at {
            self.fields[i] = field;
        }
    }

    fn text(&self, i: usize) -> &[u8] {
        &self.src[self.toks[i].start as usize..self.toks[i].end as usize]
    }
}

/// `src`'s preprocessor tape, read as `opts` say, in format 16 (decision 0468 §1): its nodes written in one pass,
/// with the source's newline index and each block's presence, as every parser's run writes it. Every reader reads
/// this one, and the Bitlex grammar's tape is held to it (0436 R4).
pub fn image(src: &[u8], opts: Options) -> Tape16 {
    let nodes = nodes(src, opts);
    let mut widths = Widths::guess(src.len());
    let target: Target = Ok(Sixteen { lang: lang(), terminators: LINE_ENDS, tag: TAG, z: false, blocks: true, number: std::sync::Arc::new(number), rooms: None });
    loop {
        match write(&nodes, src, widths, target.clone()).and_then(|mut w| w.finish_16()) {
            Ok(Ok(f)) => match f.tape {
                Written::F16(t) => return t,
                Written::F16Z(_) => unreachable!("16z was not asked for"),
            },
            Ok(Err(e)) => panic!("the preprocessor tape in format 16: {e}"),
            Err(Redo(w)) => widths = w,
        }
    }
}

/// The tape's tag (its language's; the preprocessor tape is no grammar's, so its language is
/// given to every reader, `lang`, never looked up by tag).
pub const TAG: u8 = 0;

/// A file's line (from 1) at a position, counted as a scan goes forward (a directive's): the terminators of C's lines
/// (`LINE_ENDS`, decision 0612) whose last byte is before it.
#[derive(Default)]
pub struct LineCount {
    /// Past the last terminator counted.
    at: usize,
    line: u32,
}

impl LineCount {
    /// The line (from 1) of the file's position `to` (not before the last asked). A terminator ending at `to` or
    /// after is on its line, and is seen again by the next ask.
    pub fn of(&mut self, src: &[u8], to: usize) -> u32 {
        let from = self.at.min(to);
        let (mut line, mut at) = (self.line, self.at);
        LINE_ENDS.each(&src[from..(to + 1).min(src.len())], |k, len| {
            if from + k + len - 1 < to {
                line += 1;
                at = from + k + len;
            }
        });
        (self.line, self.at) = (line, at);
        self.line + 1
    }
}
/// C's line terminators, clang's and GCC's: `\r\n`, `\r` and `\n` (decision 0612, the C adapter's), which the
/// tape's line index holds and `__LINE__` counts.
pub const LINE_ENDS: rerail_tape_format::newlines::Terminators = rerail_tape_format::newlines::Terminators::Newlines;

/// The tape's format 16 language: a code a kind with no field first (its number is its kind's,
/// below the one-byte cut), then each kind with each field (`number`).
pub fn lang() -> &'static Lang {
    static LANG: std::sync::LazyLock<Lang> = std::sync::LazyLock::new(|| {
        let entry = |kind: usize, field: usize| Entry { kind: kind as u16, field: field as u16, flags: 0, alias: None, classes: Vec::new() };
        let entries = (0..=FIELDS.len()).flat_map(|f| (0..KINDS.len()).map(move |k| entry(k, f))).collect();
        Lang::new(CodeTable::new(entries), [1, 1, 1], 1).expect("the preprocessor tape's language")
    });
    &LANG
}

/// A key's number in `lang`'s table: its field's row of kinds, then its kind.
fn number(key: u32) -> Option<u32> {
    let (kind, field) = (key & 0x3ff, key >> rerail_tape_format::KIND_BITS);
    ((kind as usize) < KINDS.len() && (field as usize) <= FIELDS.len()).then(|| field * KINDS.len() as u32 + kind)
}

/// The tape's nodes, sorted by start, then the longest first, then by rank.
fn nodes(src: &[u8], opts: Options) -> Vec<Node> {
    let toks = tokens(src, opts);
    let d = directives::scan(src, opts);
    let mut g = Gather { toks: &toks, src, nodes: Vec::new(), fields: vec![0; toks.len()] };
    let file = |s: Span| d.file(s);
    // Directives, and which tokens are theirs.
    let mut in_directive = vec![false; toks.len()];
    let mut dir_tokens = Vec::with_capacity(d.all.len());
    for x in &d.all {
        let (s, e) = file(x.span);
        let r = g.range((s, e));
        in_directive[r.0..r.1].iter_mut().for_each(|b| *b = true);
        dir_tokens.push(r);
        g.nodes.push(Node { start: s, end: e, rank: 1, key: key(kind_id(x.kind), 0, 0) });
    }
    // Groups: each `#if…`'s chain to its `#endif`, or to the file's end.
    let file_end = toks.last().map_or(0, |t| t.end);
    let mut next = vec![None; d.all.len()];
    for c in &d.conds {
        next[c.i as usize] = c.next;
    }
    for c in &d.conds {
        if matches!(c.kind, Kind::If | Kind::Ifdef | Kind::Ifndef) {
            let mut j = c.i as usize;
            while let Some(n) = next[j] {
                j = n as usize;
            }
            let end = if d.all[j].kind == Kind::Endif { file(d.all[j].span).1 } else { file_end };
            g.nodes.push(Node { start: file(d.all[c.i as usize].span).0, end, rank: 0, key: key(GROUP, 0, 0) });
        }
    }
    // Operands.
    for def in &d.defines {
        let i = def.i as usize;
        g.leaf(file(def.name).0, field("name"));
        let (_, b) = dir_tokens[i];
        if def.fnlike {
            let open = g.at(file(def.name).0) + 1;
            let close = (open..b).find(|&k| g.toks[k].kind == PUNCT && g.text(k) == b")").map_or(b, |k| k + 1);
            g.over((open, close), PARAMS, field("params"));
            for p in &def.params {
                g.leaf(file(*p).0, field("param"));
            }
            if def.variadic != Variadic::No
                && let Some(k) = (open..close).rev().find(|&k| g.text(k) == b"...")
            {
                g.fields[k] = field("variadic");
            }
        }
        if !def.body.is_empty() {
            let r = g.range(file(def.body));
            g.over(r, OPERANDS, field("body"));
        }
    }
    for &(_, name) in &d.undefs {
        if !name.is_empty() {
            g.leaf(file(name).0, field("name"));
        }
    }
    for inc in &d.includes {
        if inc.spelled.is_empty() {
            continue;
        }
        let s = file(inc.spelled);
        match inc.form {
            Form::Quote => g.leaf(s.0, field("spelled")),
            Form::Angle => {
                let r = g.range(s);
                g.over(r, HEADER, field("spelled"))
            }
            Form::Macro => {
                let r = g.range(s);
                g.over(r, MACRO_HEADER, field("spelled"))
            }
        }
    }
    for c in &d.conds {
        if !c.expr.is_empty() {
            let r = g.range(file(c.expr));
            g.over(r, OPERANDS, field("expr"));
        }
    }
    for t in &d.texts {
        if !t.text.is_empty() {
            let r = g.range(file(t.text));
            g.over(r, OPERANDS, field("text"));
        }
    }
    for p in &d.pragmas {
        if !p.text.is_empty() {
            let r = g.range(file(p.text));
            g.over(r, PRAGMA_TEXT, field("text"));
        }
        if !p.first.is_empty() {
            g.leaf(file(p.first).0, field("first"));
        }
        if !p.second.is_empty() {
            g.leaf(file(p.second).0, field("second"));
        }
    }
    for l in &d.lines {
        let i = l.i as usize;
        // A line marker's rest is all after its `#`; `#line`'s, all after its word.
        let (a, b) = dir_tokens[i];
        let (a, kind) = if d.all[i].kind == Kind::LineMarker { (a + 1, MARKER_REST) } else { (a + 2, LINE_REST) };
        let a = a.min(b);
        g.over((a, b), kind, field("rest"));
        if l.n.is_some() && a < b {
            g.fields[a] = field("n");
        }
        if !l.file.is_empty() {
            g.leaf(file(l.file).0, field("file"));
        }
    }
    // Lines of text: from a token at a line's start, or after a directive, to the next such.
    let mut k = 0;
    while k < toks.len() {
        if in_directive[k] {
            k += 1;
            continue;
        }
        let a = k;
        k += 1;
        while k < toks.len() && !in_directive[k] && toks[k].flags & rerail_c_lean::pp::BOL == 0 {
            k += 1;
        }
        g.nodes.push(Node { start: toks[a].start, end: toks[k - 1].end, rank: 1, key: key(CODE, 0, 0) });
    }
    for (i, t) in toks.iter().enumerate() {
        g.nodes.push(Node { start: t.start, end: t.end, rank: 3, key: key(t.kind, g.fields[i], 0) });
    }
    let mut nodes = g.nodes;
    nodes.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)).then(a.rank.cmp(&b.rank)));
    nodes
}

/// The nodes, sorted by start, then the longest first, then by rank, written in preorder under the
/// root: a node is inside the open ones that end after its start.
fn write(nodes: &[Node], src: &[u8], widths: Widths, target: Target) -> Result<Writer<Widths>, Redo> {
    let mut w = Writer::new(widths, nodes.len() + 16).to(target, src);
    let root = w.push(key(PP, 0, 0), 0);
    let mut open: Vec<(usize, u32)> = Vec::new();
    for n in nodes {
        while open.last().is_some_and(|&(_, e)| e <= n.start) {
            let (i, e) = open.pop().expect("an open node");
            w.close(i, e);
        }
        debug_assert!(open.last().is_none_or(|&(_, e)| n.end <= e), "a node across its parent's end");
        let i = w.push(n.key, n.start);
        if n.rank == 3 {
            w.close(i, n.end);
        } else {
            open.push((i, n.end));
        }
    }
    while let Some((i, e)) = open.pop() {
        w.close(i, e);
    }
    w.close(root, src.len() as u32);
    Ok(w)
}

/// A directive as a tape gives it: its kind's name, its bytes, and the conditional directive that
/// opened the group it sits in (`c_cond_parent`, as `directives::Directive::parent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: &'static str,
    pub start: u32,
    pub end: u32,
    pub parent: Option<u32>,
}

/// A tape's directives, read in place in one walk. A conditional directive that is a group's child
/// is of the group's chain: its parent is the directive the group sits under. Any other directive sits under
/// its innermost group's chain directive before it.
pub fn rows(tape: &Tape16) -> Result<Vec<Row>, String> {
    let v = tape.reader(lang())?;
    let mut out = Vec::new();
    // The open groups: each its end (a node index), the parent of its chain, and its chain's
    // directive so far.
    let mut groups: Vec<(usize, Option<u32>, Option<u32>)> = Vec::new();
    // The open nodes' ends and kinds, to know a directive's parent node.
    let mut open: Vec<(usize, u16)> = Vec::new();
    for i in 1..v.len() {
        while open.last().is_some_and(|&(e, _)| e <= i) {
            open.pop();
        }
        while groups.last().is_some_and(|g| g.0 <= i) {
            groups.pop();
        }
        let kind = v.table().entry(v.code(i)).kind;
        let end = v.subtree_end(i);
        if kind == GROUP {
            let parent = groups.last().and_then(|g| g.2);
            groups.push((end, parent, None));
        } else if (DIRECTIVES..IDENTIFIER).contains(&kind) {
            let n = out.len() as u32;
            // A conditional directive that is a group's child is of its chain (a nested `#if…`
            // opens a group of its own).
            let cond = matches!(KINDS[kind as usize], "if" | "ifdef" | "ifndef" | "elif" | "elifdef" | "elifndef" | "else" | "endif");
            let in_chain = cond && open.last().is_some_and(|&(_, k)| k == GROUP);
            let parent = match groups.last_mut() {
                Some(g) if in_chain => {
                    let p = g.1;
                    g.2 = Some(n);
                    p
                }
                Some(g) => g.2,
                None => None,
            };
            out.push(Row { kind: KINDS[kind as usize], start: v.start(i) as u32, end: v.end(i) as u32, parent });
        }
        open.push((end, kind));
    }
    Ok(out)
}

/// A tape's leaves: each token's kind and bytes, in order.
pub fn leaves(tape: &Tape16) -> Result<Vec<(u16, u32, u32)>, String> {
    let v = tape.reader(lang())?;
    let kind = |i: usize| v.table().entry(v.code(i)).kind;
    Ok((1..v.len()).filter(|&i| (IDENTIFIER..PARAMS).contains(&kind(i))).map(|i| (kind(i), v.start(i) as u32, v.end(i) as u32)).collect())
}

#[cfg(test)]
#[path = "tape/tests.rs"]
mod tests;
