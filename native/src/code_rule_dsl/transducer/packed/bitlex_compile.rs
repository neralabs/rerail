//! The Bitlex compiler: a Bitlex file (`docs/parser-generators/03-bitlex.md`) to a block program
//! (`block`), printed as `rir` rows so it goes through the transducer's own assembler and
//! verifier. It holds no language: everything comes from the file.
//!
//! The parser is the reference evaluator's (`tools/bitlex`), so Bitlex is parsed in one place, and
//! the evaluator is the compiler's test oracle (`bitlex_compile_tests`).
//!
//! Built one construct at a time (`docs/parser-generators/13-tape-on-the-gpu.md` §7, step 2).
//!
//! - **Classes.** Each class is a union of products of a set of high nibbles and a set of low
//!   nibbles. Products are shared between classes and packed eight to a nibble table. A one-byte
//!   class is an `eq`.
//! - **Masks** over `~ & | ^`, `empty`, `prev`, `next`, `starts`, `lasts`, `after`, `span`,
//!   `aligned`, `toggle`, `after_odd_run` and `fill`.
//! - **Counts** as bit-planes, and `C.frame(a)` (below).
//! - **Numbers** (`number`) as bit-planes too: `NUMBER_BITS` of them, a byte's value mod 2^32
//!   (below).
//! - **Tokens**, `keywords` and `skip`: each token kind's start and last-byte masks, and the
//!   coverage and overlap checks.
//! - **The tree section** (`docs/parser-generators/18-tree-as-grammar.md`): the grammar's state
//!   machine (`grammar`), with grammar ids from `Symbols`, lowered to tier 2's words
//!   (`bitlex_tier2`, decision 0087); tier 1 outputs the stream they read, two extracts.
//! - **Errors and escapes.** Each mask joins the fallback output (`FALLBACK`); a file with a bit
//!   of it goes to tree-sitter.
//!
//! Anything else is refused by name.
//!
//! # Counts and frames
//!
//! A count `count(up, down) max M` is kept as `w` bit-planes of its value after each byte, with
//! `2^w >= M + 2`. Bit `k` flips at an up byte whose bits below `k` were all 1, and at a down
//! byte whose bits below `k` were all 0, so plane `k` is a `toggle` of a mask of the planes
//! below it one byte back: no new operation. The value is kept mod `2^w`; a count that leaves
//! `0..=M` passes through `M + 1` or `-1`, both above `M` as `w`-bit numbers, and one compare
//! escapes the file. (The evaluator holds a count at 0 instead: a file that goes below 0 escapes
//! here and is lexed there; such a file is malformed for JSON.)
//!
//! `C.frame(a)` is one `fill` per level `L` in `1..=M` (`10-bit-programs.md` §2): from the
//! openers at level `L` in `a` to the next opener at level `L` not in `a`, kept where the count
//! before the byte is `L`.
//!
//! # Numbers
//!
//! A number is `NUMBER_BITS` planes, plane `k` the bytes whose value has bit `k`. A file is
//! shorter than 2^32 bytes, so a place in it, or a distance, is exact.
//!
//! - `position`: plane `k` a pattern of the word below bit 6 (`const`), the block's place
//!   from bit 6 on (`position k`, a new leaf with no carry).
//! - `fill(s, e, v)`: plane `k` is `fill(s & v_k, e | s)`, a fill that restarts at every byte
//!   of `s` and runs where `v` had bit `k` there; `back_fill` likewise.
//! - `a - b`: a ripple of borrows over the planes, three operations a plane.
//! - `at_least(a, b)`: no borrow out of `a - b`; `above(a, b)` is `~at_least(b, a)`.
//!
//! No new operation but `position`: every plane is a mask, every step a word operation.
//!
//! # Delays: `next` without lookahead
//!
//! Every value is computed some bytes late: its bit `p` is byte `p - d`'s, for its delay `d`.
//! Bytes and classes have delay 0. An operation's operands are brought to the later delay by
//! `prev`, and `next(a, k)` is `a` itself with `k` more delay. The outputs are brought to the
//! largest delay `D`, the program's `delay`; each executor runs `D` bytes past the stream and
//! shifts the outputs back (`block::undelay`).
//!
//! So that no value has bits outside the stream (which `prev` and `next` would carry in):
//!
//! - before the stream every value is zero: `prev` shifts in zeros, `~a` is `within & ~a` with
//!   `within` (the stream's bytes) at the same delay, and `next` clears the bytes it brings in
//!   before the start;
//! - past the end a value may be *dirty*: a byte test that the zero padding passes (`eq 0x00`),
//!   a constant, or a carried operation whose state runs on. `next` cleans its operand with
//!   `within` when it is dirty.

use super::block::{BlockOp, BlockProgram};
use super::{bitlex_prove, bitlex_tier2, grammar};
use ::bitlex::syntax::Stmt;
use rerail_tier1::bitlex_masks::{ByteSet, Lowering, TreeSection, Tier2, Val};
use std::collections::HashMap;

// The masks' lowering is tier 1's, in its crate (`rerail_tier1::bitlex_masks`); the tree
// section (the grammar's state machine, tier 2's words) is here.
pub(in crate::code_rule_dsl) use rerail_tier1::bitlex_masks::{Compiled, Cost, FALLBACK, Named, STREAM_LAST, STREAM_MARK, STREAM_START};

/// The grammar's ids for the tree section's rows.
pub(in crate::code_rule_dsl) struct Symbols<'a> {
    /// A node kind's id (named first, then anonymous), or a supertype's.
    pub(super) kind: &'a dyn Fn(&str) -> Option<u16>,
    pub(super) field: &'a dyn Fn(&str) -> Option<u16>,
}

/// Whether a keyword gives kind `name`.
fn is_keyword_kind(keywords: &[(String, Vec<(Vec<u8>, Option<String>)>, bool)], name: &str) -> bool {
    keywords.iter().any(|(_, words, _)| words.iter().any(|(w, alias)| alias.as_deref().map_or(w.as_slice() == name.as_bytes(), |a| a == name)))
}

/// A program's masks and its tree section, lowered (`rerail_tier1::bitlex_masks::compile`).
pub(in crate::code_rule_dsl) fn compile(stmts: &[Stmt], outputs: &[&str], symbols: Option<&Symbols<'_>>) -> Result<Compiled, String> {
    rerail_tier1::bitlex_masks::compile(stmts, outputs, symbols.map(|s| s as &dyn TreeSection))
}

impl TreeSection for Symbols<'_> {
    fn lower(&self, low: &mut Lowering, token_rows: &[(String, Val, Val)]) -> Result<Tier2, String> {
        let symbols = self;
        let id = |name: &str| (symbols.kind)(name).ok_or_else(|| format!("no kind `{name}` in the grammar"));
        let field_id = |name: &str| (symbols.field)(name).ok_or_else(|| format!("no field `{name}`"));
        // A token row whose every token is a keyword (`required`) has kind 0: the keyword's is its.
        let required: Vec<String> = low.keywords.iter().filter(|k| k.2).map(|k| k.0.clone()).collect();
        // The grammar's state machine (doc 18 §2). A token kind is a token row's that is not all
        // keywords, or a keyword's.
        let mark_kinds = low.marks.iter().map(|m| id(&m.0)).collect::<Result<Vec<u16>, String>>()?;
        if let Some(h) = low.hidden.iter().find(|h| !low.marks.iter().any(|m| &m.0 == *h)) {
            return Err(format!("hidden {h}: no mark of that kind"));
        }
        // A refused mark is a hidden one: skipped elsewhere.
        for s in &low.tree {
            if let Stmt::Refuse { mark, .. } = s
                && !low.hidden.contains(mark)
            {
                return Err(format!("refuse {mark}: no hidden mark of that kind"));
            }
        }
        // The readings (`reading R = …`, `reread R`): each kind's way, an extra's leaf staying as
        // it is; and the provisional fields.
        let mut reading_names: Vec<String> = Vec::new();
        let mut readings = super::stream::Readings::default();
        for s in &low.tree {
            match s {
                Stmt::Reading { name, reading } => {
                    if reading_names.contains(name) {
                        return Err(format!("reading {name}: two of that name"));
                    }
                    reading_names.push(name.clone());
                    let mut r = super::stream::Reading::default();
                    for (k, what) in &reading.kinds {
                        let to = match what {
                            ::bitlex::syntax::ReadKind::Keep => super::stream::KEEP,
                            ::bitlex::syntax::ReadKind::Gone => super::stream::GONE,
                            ::bitlex::syntax::ReadKind::As(m) => id(m.as_str())?,
                        };
                        let k = id(k)?;
                        if r.kinds.iter().any(|x| x.0 == k) {
                            return Err(format!("reading {name}: a kind twice"));
                        }
                        r.kinds.push((k, to));
                    }
                    for e in &low.extra {
                        let e = id(e)?;
                        if !r.kinds.iter().any(|x| x.0 == e) {
                            r.kinds.push((e, e));
                        }
                    }
                    for (f, g) in &reading.fields {
                        r.fields.push((field_id(f)?, field_id(g)?));
                    }
                    r.root = match &reading.root {
                        None => None,
                        Some(::bitlex::syntax::ReadKind::Gone) => Some(super::stream::GONE),
                        Some(::bitlex::syntax::ReadKind::As(m)) => Some(id(m.as_str())?),
                        Some(::bitlex::syntax::ReadKind::Keep) => return Err(format!("reading {name}: a root kept")),
                    };
                    readings.maps.push(r);
                }
                Stmt::Provisional(fields) => {
                    for f in fields {
                        readings.provisional.push(field_id(f)?);
                    }
                }
                _ => {}
            }
        }
        let (states, mut arms, whole) = {
            let is_token = |name: &str| {
                token_rows.iter().any(|t| t.0 == name && !required.contains(&t.0))
                    || is_keyword_kind(&low.keywords, name)
                    || low.soft.iter().any(|s| s.1.iter().any(|w| w.as_slice() == name.as_bytes()))
                    || low.marks.iter().any(|m| m.0 == name)
            };
            let kind = |name: &str| id(name);
            // Every token kind: a token row's that is not all keywords, and each keyword's.
            let mut tokens: Vec<String> = token_rows.iter().filter(|t| !required.contains(&t.0)).map(|t| t.0.clone()).collect();
            for (_, words, _) in &low.keywords {
                for (w, alias) in words {
                    let k = alias.clone().unwrap_or_else(|| String::from_utf8_lossy(w).into_owned());
                    if !tokens.contains(&k) {
                        tokens.push(k);
                    }
                }
            }
            tokens.retain(|t| !low.marks.iter().any(|m| &m.0 == t) && !low.extra.contains(t));
            let reading = |name: &str| reading_names.iter().position(|r| r == name).map(|r| r as u16).ok_or_else(|| format!("reread {name}: no reading of that name"));
            let ids = grammar::Ids { kind: &kind, field: &field_id, is_token: &is_token, tokens: &tokens, reading: &reading };
            grammar::build(&low.tree, &ids)?
        };
        grammar::without_mark_leaves(&mut arms, &mark_kinds)?;
        // A node a reading drops has no supertype: tier 2 places the supertypes by their nodes.
        let gone: Vec<u16> = readings.maps.iter().flat_map(|r| r.kinds.iter().filter(|x| x.1 == super::stream::GONE).map(|x| x.0)).collect();
        for a in &arms {
            for act in &a.acts {
                let k = match act {
                    grammar::Act::Open { kind, supers, .. } if !supers.is_empty() => *kind,
                    grammar::Act::Leaf { kind, supers, .. } if !supers.is_empty() => if *kind != 0 { *kind } else { a.token },
                    _ => continue,
                };
                if gone.contains(&k) {
                    return Err(format!("a reading drops kind {k}, which has a supertype"));
                }
            }
        }
        // Each check, a byte machine over its tokens' bytes (doc 18 §4), compiled into masks
        // over the whole file with the rest of the block program: a token it refuses joins the
        // fallback. Tier 2 keeps only the bytes a token passing it begins with.
        let mut check_of: HashMap<usize, ByteSet> = HashMap::new();
        for (kind, g) in low.checks.clone() {
            let token = token_rows.iter().position(|t| t.0 == kind).ok_or_else(|| format!("check {kind}: no token row of that kind"))?;
            let (accept, arms) = grammar::byte_check(&g, &low.classes).map_err(|e| format!("check {kind}: {e}"))?;
            let first = arms.iter().filter(|a| a.0 == 0).fold([0u64; 4], |s, a| [s[0] | a.1[0], s[1] | a.1[1], s[2] | a.1[2], s[3] | a.1[3]]);
            let words: Vec<Vec<u8>> = low.keywords.iter().filter(|k| k.0 == kind).flat_map(|k| k.1.iter().map(|w| w.0.clone())).collect();
            let (_, start, last) = token_rows[token].clone();
            let refused = low.check(start, last, &accept, &arms, &words).map_err(|e| format!("check {kind}: {e}"))?;
            low.fallbacks.push(refused);
            check_of.insert(token, first);
        }
        let mut rows = Vec::new();
        for (r, (kind, start, last)) in token_rows.iter().enumerate() {
            let mut keywords = Vec::new();
            for (k, words, _) in low.keywords.iter().filter(|k| &k.0 == kind) {
                let _ = k;
                for (word, alias) in words {
                    let name = alias.clone().unwrap_or_else(|| String::from_utf8_lossy(word).into_owned());
                    keywords.push((word.clone(), id(&name)?));
                }
            }
            let mut soft = Vec::new();
            for (_, words) in low.soft.iter().filter(|s| &s.0 == kind) {
                for word in words {
                    soft.push((word.clone(), id(&String::from_utf8_lossy(word))?));
                }
            }
            rows.push(bitlex_tier2::Row {
                kind: if required.contains(kind) { 0 } else { id(kind)? },
                soft,
                one_byte: start.v == last.v && start.d == last.d,
                first: bitlex_prove::first_bytes(&low.program.ops, &low.program.tables, start.v, start.d),
                keywords,
                check: check_of.get(&r).copied(),
                mask: None,
            });
        }
        // Two rows that can begin with one byte and can start at one byte: that byte sends the
        // file to tree-sitter, as the evaluator's "both start at byte" does. A pair whose starts
        // are apart, proved (`never`: `inside` and `~inside`, say), costs nothing.
        let shares = |a: usize, b: usize| (0..4).any(|i| rows[a].first[i] & rows[b].first[i] != 0);
        for a in 0..rows.len() {
            for b in a + 1..rows.len() {
                if shares(a, b) {
                    let both = low.and(token_rows[a].1, token_rows[b].1)?;
                    if !bitlex_prove::never(&low.program.ops, &low.program.tables, both.v, both.d) {
                        low.fallbacks.push(both);
                    }
                }
            }
        }
        // The extras' arms (grammar::with_extras), in every state: tier 2 tells an extra from a
        // token beginning with the same byte by their rows' start masks (`ROW`), and a mark has
        // no bytes at all.
        let (states, arms) = {
            let extras = low.extra.iter().map(|k| id(k)).collect::<Result<Vec<u16>, String>>()?;
            let apart = |_: u16, _: u16| true;
            grammar::with_extras(states, arms, &extras, &apart, whole, &mark_kinds)?
        };
        // Each mark's bits at the grammar's token starts (not the extras'): a token that starts
        // with a bit since the start before it, `starts & (m | prev(fill(m & ~starts, starts)))`.
        // Its mask is the program's `1 + k` (the 0th is the last bytes, `SPAN m=1`).
        let mut grammar_starts: Option<Val> = None;
        for (kind, start, _) in token_rows {
            let is_extra = low.extra.contains(kind);
            if !is_extra {
                grammar_starts = Some(match grammar_starts {
                    Some(g) => low.or(g, *start)?,
                    None => *start,
                });
            }
        }
        let mut mark_masks = Vec::new();
        let mut marks = Vec::new();
        for (k, (kind, m, end)) in low.marks.clone().into_iter().enumerate() {
            let gs = grammar_starts.ok_or("marks with no grammar token")?;
            let before = low.andnot(m, gs)?;
            let since = low.fill(before, gs)?;
            let since = low.prev(since, 1)?;
            let any = low.or(m, since)?;
            mark_masks.push(low.and(gs, any)?);
            marks.push(bitlex_tier2::Mark { kind: mark_kinds[k], mask: 1 + k as u16, hidden: low.hidden.contains(&kind), end });
        }
        // The rows each state can meet, by the modes its tokens are read in (`row_modes`): tier 2
        // tells apart by their start masks (`ROW`) only the rows a state can meet that begin
        // with one byte. A first lowering finds them; their masks follow the marks'.
        let meets = row_modes(low, &token_rows, &rows, &states, &arms, &mark_kinds)?;
        let empties = low.fallback()?.len() as u16;
        let tested = bitlex_tier2::lower(&states, &arms, &rows, &marks, &meets, empties, whole, true)?.tested;
        for (j, &r) in tested.iter().enumerate() {
            rows[r].mask = Some(1 + marks.len() as u16 + j as u16);
            mark_masks.push(token_rows[r].1);
        }
        // The words, once the fallback's totals are known: each must be zero.
        let mut lowered = bitlex_tier2::lower(&states, &arms, &rows, &marks, &meets, empties, whole, false)?;
        // The extras' kinds: the node table flags their nodes (comments).
        readings.extras = low.extra.iter().map(|k| id(k)).collect::<Result<_, _>>()?;
        super::rir::reading_rows(&mut lowered.text, &readings);
        let takes_last: Vec<bool> = rows.iter().map(bitlex_tier2::Row::takes_last).collect();
        Ok((grammar::text(&states, &arms), lowered.text, takes_last, mark_masks))
    }
}

/// The rows each state of tier 2 can meet: `meets[s][r]`, whether a token of row `r` can be
/// the next one in state `s`.
///
/// A token's row is told by its first byte, and two rows that begin with one byte (`"` opening
/// a string in one and inside a string in another, say) by their start masks (`ROW`, a word per
/// token). Most such rows are apart in another way: they are read in different modes. A mode is
/// a `toggle(x)`'s, the parity of `x`'s bits before a token, and is known in each state:
///   - each row's start fixes the toggle or leaves it free (`never(start & ~t)`: it is set
///     there), and `x` at the start likewise, so its mode, when both are fixed;
///   - when every bit of `x` is some token's start (`never(x & ~starts)`), only a token that
///     starts on `x` changes the mode, and it flips it;
///   - the modes the states are entered in follow, from the root's start in mode 0, by the
///     arms (a mark takes no bytes), the calls and the returns (the mode is the stream's, not
///     the stack's: a return lands in every state the level is called to return to).
/// A row a state can meet in none of its modes is no candidate at a byte it shares.
fn row_modes(
    low: &mut Lowering,
    token_rows: &[(String, Val, Val)],
    rows: &[bitlex_tier2::Row],
    states: &[grammar::StateRow],
    arms: &[grammar::ArmRow],
    marks: &[u16],
) -> Result<Vec<Vec<bool>>, String> {
    let proved = |low: &Lowering, v: Val| bitlex_prove::never(&low.program.ops, &low.program.tables, v.v, v.d);
    // The toggles whose `x` bits are all token starts, at most eight.
    let mut starts: Option<Val> = None;
    for (_, s, _) in token_rows {
        starts = Some(match starts {
            Some(a) => low.or(a, *s)?,
            None => *s,
        });
    }
    let mut modes: Vec<(Val, Val)> = Vec::new();
    for (t, x) in low.toggles.clone() {
        let Some(s) = starts else { break };
        let stray = low.andnot(x, s)?;
        if modes.len() < 8 && proved(low, stray) {
            modes.push((t, x));
        }
    }
    // Each row's mode at its start, and its flips: per toggle, `Some(bit)` when fixed.
    let fixed = |low: &mut Lowering, s: Val, v: Val| -> Result<Option<bool>, String> {
        let off = low.andnot(s, v)?;
        let on = low.and(s, v)?;
        Ok(match (proved(low, off), proved(low, on)) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        })
    };
    let mut need: Vec<Vec<Option<bool>>> = Vec::new();
    let mut flip: Vec<Vec<Option<bool>>> = Vec::new();
    for (_, s, _) in token_rows {
        let (mut n, mut f) = (Vec::new(), Vec::new());
        for &(t, x) in &modes {
            let (tv, xv) = (fixed(low, *s, t)?, fixed(low, *s, x)?);
            // The mode before the token: the toggle at its start, less `x` there.
            n.push(match (tv, xv) {
                (Some(a), Some(b)) => Some(a != b),
                _ => None,
            });
            f.push(xv);
        }
        need.push(n);
        flip.push(f);
    }
    let fits = |r: usize, m: u8| need[r].iter().enumerate().all(|(i, n)| n.is_none_or(|n| n == (m >> i & 1 == 1)));
    // The modes after a row's token, from `m`.
    let after = |r: usize, m: u8| -> Vec<u8> {
        let mut out = vec![m];
        for (i, f) in flip[r].iter().enumerate() {
            out = match f {
                Some(false) => out,
                Some(true) => out.into_iter().map(|m| m ^ 1 << i).collect(),
                None => out.into_iter().flat_map(|m| [m, m ^ 1 << i]).collect(),
            };
        }
        out
    };
    let rows_of = |kind: u16| -> Vec<usize> {
        (0..rows.len())
            .filter(|&r| (rows[r].kind == kind && kind != 0) || rows[r].keywords.iter().chain(&rows[r].soft).any(|k| k.1 == kind))
            .collect()
    };
    // The states each level returns to.
    let mut returns: HashMap<u16, Vec<u16>> = HashMap::new();
    for a in arms {
        // The innermost call returns from the level it enters; each outer one from the level of
        // the call inside it, which it returns to.
        let mut from = a.to;
        for &p in a.push.iter().rev() {
            returns.entry(states[usize::from(from)].level).or_default().push(p);
            from = p;
        }
    }
    let root = states.first().ok_or("no states")?.level;
    // The modes each state is entered in: a bit per mode.
    let mut seen = vec![[0u64; 4]; states.len()];
    let mut work = vec![(0u16, 0u8)];
    let enter = |seen: &mut Vec<[u64; 4]>, work: &mut Vec<(u16, u8)>, s: u16, m: u8| {
        let w = &mut seen[usize::from(s)][usize::from(m / 64)];
        if *w >> (m % 64) & 1 == 0 {
            *w |= 1 << (m % 64);
            work.push((s, m));
        }
    };
    seen[0][0] = 1;
    while let Some((s, m)) = work.pop() {
        let st = &states[usize::from(s)];
        if st.end.is_some() && st.level != root {
            for &p in returns.get(&st.level).into_iter().flatten() {
                enter(&mut seen, &mut work, p, m);
            }
        }
        for a in arms.iter().filter(|a| a.from == s) {
            let next: Vec<u8> = if marks.contains(&a.token) {
                vec![m]
            } else {
                rows_of(a.token).into_iter().filter(|&r| fits(r, m)).flat_map(|r| after(r, m)).collect()
            };
            for n in next {
                enter(&mut seen, &mut work, a.to, n);
            }
        }
    }
    Ok((0..states.len())
        .map(|s| (0..rows.len()).map(|r| (0..=255u8).any(|m| seen[s][usize::from(m / 64)] >> (m % 64) & 1 == 1 && fits(r, m))).collect())
        .collect())
}
