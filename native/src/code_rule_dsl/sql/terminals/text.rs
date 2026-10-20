//! The `text_match(term)` terminal for `regex`: an input-set terminal. It receives the distinct
//! `(file, node)` keys of a frame (`file << 32 | node`) in one `xFilter` call and returns those
//! whose node text the regex matches (`Regex::is_match`, unanchored, as the matcher tests it).
//! A shared literal fact's hole (`Terminal::Regexes`) is a list of regexes: it returns each key
//! once per member whose regex matches, the member in `idx` (0 for a single regex).
//!
//! # One scan of a file for many of its nodes
//!
//! Testing each node's text alone reads a file once per level of its tree. Where a file's keys
//! hold more text than the file, the regex runs over the file instead (`Scanner`), exactly when
//! the regex has no look-around assertion (`^`, `$`, `\A`, `\z`, `\b`, `\B`, multi-line or not) and
//! matches no empty text. Then whether it matches a substring depends on the substring alone, not
//! on where it lies: it matches node text `file[s..e]` iff some match `[a, b)` of the file has
//! `s <= a` and `b <= e`. With `E(p)` the least end of a match starting at or after `p`, that is
//! `E(s) <= e`. `E` is non-decreasing, and constant up to `A(b)`, the latest start of a match
//! ending at `b = E(p)`: one forward search for the earliest end from `p` and one reverse search
//! anchored at that end for its latest start settle every key starting in `p..=A(b)`. A regex with
//! look-around reads the characters around a match, which differ between the file and the node's
//! text at the node's ends (`^` holds at a node's start but not mid-line in the file), so it is
//! tested per node; so is an empty-matching one, and a file whose keys' text is shorter than it.
//! Both ways answer `Regex::is_match` on each node's text.

use super::Terminal;
use crate::code_rule_dsl::sql::exec::BatchSlot;
use crate::code_rule_dsl::sql::nodes::integer;
use ast_grep_core::Doc;
use regex_automata::hybrid::dfa::{Cache, DFA};
use regex_automata::nfa::thompson;
use regex_automata::{Anchored, Input, MatchKind};
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::{Value, ValueRef};
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, VTab, VTabConfig, VTabConnection, VTabCursor,
    sqlite3_vtab, sqlite3_vtab_cursor,
};
use rusqlite::{Error, Result};
use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::sync::Arc;

/// A regex tested over a whole file for many of its nodes (the module doc): its forward and
/// reverse lazy DFAs and their caches.
struct Scanner {
    forward: DFA,
    forward_cache: Cache,
    reverse: DFA,
    reverse_cache: Cache,
}

impl Scanner {
    /// `None` where `pattern` has a look-around assertion or matches empty text (or the DFAs do
    /// not build).
    fn of(pattern: &str) -> Option<Self> {
        let hir = regex_syntax::parse(pattern).ok()?;
        let properties = hir.properties();
        if !properties.look_set().is_empty() || properties.minimum_len().is_none_or(|len| len == 0)
        {
            return None;
        }
        // Every match, not only the leftmost-first: the searches stop at the earliest one.
        let config = DFA::config().match_kind(MatchKind::All);
        let forward = DFA::builder()
            .configure(config.clone())
            .build(pattern)
            .ok()?;
        let reverse = DFA::builder()
            .configure(config)
            .thompson(thompson::Config::new().reverse(true))
            .build(pattern)
            .ok()?;
        Some(Self {
            forward_cache: forward.create_cache(),
            forward,
            reverse_cache: reverse.create_cache(),
            reverse,
        })
    }

    /// Whether the regex matches `text[s..e]`, for each span `(s, e)` of `spans` sorted by `s`;
    /// `None` where a search fails.
    fn matches(&mut self, text: &str, spans: &[(usize, usize)]) -> Option<Vec<bool>> {
        let mut out = vec![false; spans.len()];
        let mut at = 0;
        while at < spans.len() {
            let from = spans[at].0;
            let input = Input::new(text).range(from..).earliest(true);
            let Some(end) = self
                .forward
                .try_search_fwd(&mut self.forward_cache, &input)
                .ok()?
            else {
                // No match starts at or after `from`: no later span holds one.
                break;
            };
            let end = end.offset();
            let input = Input::new(text)
                .range(from..end)
                .anchored(Anchored::Yes)
                .earliest(true);
            let start = self
                .reverse
                .try_search_rev(&mut self.reverse_cache, &input)
                .ok()??
                .offset();
            while at < spans.len() && spans[at].0 <= start {
                out[at] = end <= spans[at].1;
                at += 1;
            }
        }
        Some(out)
    }
}

const COL_TERM: c_int = 0;
const COL_EK: c_int = 1;
const COL_IDX: c_int = 2;

#[repr(C)]
pub(in crate::code_rule_dsl::sql) struct TextTable {
    base: sqlite3_vtab,
    slot: Arc<BatchSlot>,
    /// Per pattern, its scanner, where it has one (`Scanner::of`).
    scanners: Arc<parking_lot::Mutex<HashMap<String, Option<Scanner>>>>,
}

unsafe impl<'vtab> VTab<'vtab> for TextTable {
    type Aux = Arc<BatchSlot>;
    type Cursor = TextCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<BatchSlot>>,
        _module: &[u8],
        _database: &[u8],
        _table: &[u8],
        _args: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let slot =
            Arc::clone(aux.ok_or_else(|| Error::ModuleError("text_match: no batch slot".into()))?);
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(term HIDDEN, ek, idx)"),
            Self {
                base: sqlite3_vtab::default(),
                slot,
                scanners: Arc::default(),
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut term = None;
        let mut ek = None;
        for (index, constraint) in info.constraints().enumerate() {
            if !constraint.is_usable()
                || constraint.operator() != IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                continue;
            }
            match constraint.column() {
                COL_TERM => {
                    term.get_or_insert(index);
                }
                COL_EK => {
                    ek.get_or_insert(index);
                }
                _ => {}
            }
        }
        let (Some(term), Some(ek)) = (term, ek) else {
            info.set_estimated_cost(f64::MAX);
            info.set_estimated_rows(i64::MAX);
            info.set_idx_num(0);
            return Ok(true);
        };
        let list = info.is_in_constraint(ek)? && info.set_in_constraint(ek, true)?;
        for (argv, index) in [term, ek].into_iter().enumerate() {
            let mut usage = info.constraint_usage(index);
            usage.set_argv_index(argv as c_int + 1);
            usage.set_omit(true);
        }
        info.set_idx_num(if list { 3 } else { 1 });
        info.set_estimated_cost(100.0);
        info.set_estimated_rows(100);
        Ok(true)
    }

    fn open(&'vtab mut self) -> Result<TextCursor<'vtab>> {
        Ok(TextCursor {
            base: sqlite3_vtab_cursor::default(),
            slot: Arc::clone(&self.slot),
            scanners: Arc::clone(&self.scanners),
            term: 0,
            keys: Vec::new(),
            members: Vec::new(),
            pos: 0,
            phantom: PhantomData,
        })
    }
}

#[repr(C)]
pub(in crate::code_rule_dsl::sql) struct TextCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    slot: Arc<BatchSlot>,
    scanners: Arc<parking_lot::Mutex<HashMap<String, Option<Scanner>>>>,
    term: i64,
    keys: Vec<i64>,
    /// Each key's member, where the term is a hole's regexes (`Terminal::Regexes`).
    members: Vec<i64>,
    pos: usize,
    phantom: PhantomData<&'vtab TextTable>,
}

unsafe impl VTabCursor for TextCursor<'_> {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        #[cfg(test)]
        super::calls::note(&super::calls::TEXT_FILTERS);
        if idx_num & 1 == 0 {
            return Err(Error::ModuleError(
                "text_match requires a term and an input set".into(),
            ));
        }
        let batch = self
            .slot
            .current()
            .ok_or_else(|| Error::ModuleError("text_match: no batch installed".into()))?;
        let term_id = integer(ValueRef::from(&args.get::<Value>(0)?))
            .ok_or_else(|| Error::ModuleError("text_match: term must be an integer".into()))?;
        let terminal = usize::try_from(term_id)
            .ok()
            .and_then(|index| batch.program.terminals.get(index));
        let regex = match terminal {
            Some(Terminal::Regex(regex)) => regex,
            Some(Terminal::Regexes(list)) => {
                let mut keys = Vec::new();
                if idx_num & 2 != 0 {
                    let mut items = args.in_values(1)?;
                    while let Some(item) = items.next()? {
                        keys.extend(integer(item));
                    }
                } else {
                    keys.extend(integer(ValueRef::from(&args.get::<Value>(1)?)));
                }
                keys.sort_unstable();
                keys.dedup();
                let (mut found, mut members) = (Vec::new(), Vec::new());
                for key in keys {
                    let file = (key >> 32) as usize;
                    let node = (key & 0xffff_ffff) as u32;
                    let mut test = |text: &str| {
                        let mut matched = |index: usize| {
                            for &member in &list.members[index] {
                                found.push(key);
                                members.push(i64::from(member));
                            }
                        };
                        match &list.set {
                            Some(set) => set.matches(text).iter().for_each(&mut matched),
                            None => (0..list.regexes.len())
                                .filter(|&index| list.regexes[index].is_match(text))
                                .for_each(&mut matched),
                        }
                    };
                    match batch.files.get(file) {
                        Some(entry) => {
                            if node < entry.facts.len() {
                                test(entry.root.root().get_doc().transducer_text_str(node));
                            }
                        }
                        // A parsed tree's node: its text, as the matcher reads it.
                        None => {
                            if let Some(sub) = batch.subfile(file)
                                && node < sub.facts.len()
                            {
                                test(
                                    &sub.root
                                        .adopt(sub.root.root().get_doc().node(node))
                                        .text(),
                                );
                            }
                        }
                    }
                }
                self.term = term_id;
                self.keys = found;
                self.members = members;
                self.pos = 0;
                return Ok(());
            }
            _ => {
                return Err(Error::ModuleError(format!(
                    "text_match: no regex term {term_id}"
                )));
            }
        };
        let mut keys = Vec::new();
        if idx_num & 2 != 0 {
            let mut items = args.in_values(1)?;
            while let Some(item) = items.next()? {
                keys.extend(integer(item));
            }
        } else {
            keys.extend(integer(ValueRef::from(&args.get::<Value>(1)?)));
        }
        keys.sort_unstable();
        keys.dedup();
        let mut verdicts = Vec::with_capacity(keys.len());
        // Keys ascend by file: each file's keys at once.
        for group in keys.chunk_by(|a, b| a >> 32 == b >> 32) {
            let file = (group[0] >> 32) as usize;
            let node = |key: i64| (key & 0xffff_ffff) as u32;
            let Some(entry) = batch.files.get(file) else {
                // A parsed tree's node: its text, as the matcher reads it.
                let sub = batch.subfile(file);
                verdicts.extend(group.iter().map(|&key| {
                    sub.as_ref().is_some_and(|sub| {
                        node(key) < sub.facts.len()
                            && regex.is_match(
                                &sub.root
                                    .adopt(sub.root.root().get_doc().node(node(key)))
                                    .text(),
                            )
                    })
                }));
                continue;
            };
            let doc = entry.root.root().get_doc();
            let len = entry.facts.len();
            let spans: Vec<(usize, usize)> = group
                .iter()
                .map(|&key| {
                    let node = node(key);
                    if node < len {
                        let span = doc.transducer_span(node);
                        (span.start, span.end)
                    } else {
                        (0, 0)
                    }
                })
                .collect();
            let text = doc.get_source().as_str();
            let read: usize = spans.iter().map(|(start, end)| end - start).sum();
            let scanned = (read > text.len())
                .then(|| {
                    let mut scanners = self.scanners.lock();
                    let scanner = scanners
                        .entry(regex.as_str().to_owned())
                        .or_insert_with(|| Scanner::of(regex.as_str()))
                        .as_mut()?;
                    let mut order: Vec<usize> = (0..spans.len()).collect();
                    order.sort_by_key(|&at| spans[at].0);
                    let sorted: Vec<(usize, usize)> = order.iter().map(|&at| spans[at]).collect();
                    let found = scanner.matches(text, &sorted)?;
                    let mut out = vec![false; spans.len()];
                    for (at, matched) in order.into_iter().zip(found) {
                        out[at] = matched;
                    }
                    Some(out)
                })
                .flatten();
            match scanned {
                Some(found) => verdicts.extend(
                    group
                        .iter()
                        .zip(found)
                        .map(|(&key, matched)| matched && node(key) < len),
                ),
                None => verdicts.extend(group.iter().map(|&key| {
                    node(key) < len && regex.is_match(doc.transducer_text_str(node(key)))
                })),
            }
        }
        let mut verdicts = verdicts.into_iter();
        keys.retain(|_| verdicts.next().expect("a verdict per key"));
        self.term = term_id;
        self.keys = keys;
        self.members.clear();
        self.pos = 0;
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.pos += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        self.pos >= self.keys.len()
    }

    fn column(&self, context: &mut Context, column: c_int) -> Result<()> {
        match column {
            COL_TERM => context.set_result(&self.term),
            COL_IDX => context.set_result(&self.members.get(self.pos).copied().unwrap_or(0)),
            _ => context.set_result(&self.keys[self.pos]),
        }
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.pos as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::Scanner;
    use regex::Regex;

    /// One scan answers `is_match` on every substring, for regexes without look-around that match
    /// no empty text; the others have no scanner.
    #[test]
    fn a_file_scan_answers_each_spans_is_match() {
        let text = "ab xab b\nfoo(bar, baz) é€x 12 ab\naab ba\n";
        let bounds: Vec<usize> = (0..=text.len())
            .filter(|&at| text.is_char_boundary(at))
            .collect();
        let mut spans = Vec::new();
        for (index, &start) in bounds.iter().enumerate() {
            for &end in &bounds[index..] {
                spans.push((start, end));
            }
        }
        for pattern in [
            "ab|b", "a+b", "b", "x", "ba?z", "[a-z]{3}", r"\(\w+,", "é€", "€x 1", ".b", r"\d+",
            "zzz", "a.*a", "(?s)b.a", "o{2}",
        ] {
            let regex = Regex::new(pattern).expect("a regex");
            let mut scanner = Scanner::of(pattern).expect("a scanner");
            let found = scanner.matches(text, &spans).expect("searches");
            for ((start, end), found) in spans.iter().zip(found) {
                assert_eq!(
                    found,
                    regex.is_match(&text[*start..*end]),
                    "{pattern} over {:?}",
                    &text[*start..*end]
                );
            }
        }
        for pattern in ["^a", "b$", r"\bab", r"\Bb", "(?m)^f", "a*", "x?", r"\A"] {
            assert!(Scanner::of(pattern).is_none(), "{pattern}");
        }
    }
}
