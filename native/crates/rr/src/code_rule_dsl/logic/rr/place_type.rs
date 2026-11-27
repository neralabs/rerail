//! The place types (decision 0724 §3): `node`, `line`, `match` and `token`. Each is an id to the engine (`lower::ty`),
//! and none is another: a variable that stands in a column of one of them stands in no column of another. Converting
//! between them is a function a rule calls (`node_to_line`, `line_to_node`, `match_to_node`, …, core.rerail), never
//! a column that holds either. Checked here, before lowering.
//!
//! ```text
//! fn_head(f, h) :- fn_holder(f, h).                      h: a node (fn_head(f: node, h: node))
//! fn_head(f, m) :- arrow_line(f, m).                     m: a line: refused, "m is a node … and a line …"
//! fn_head(f, h) :- arrow_line(f, m), line_to_node(m, h). h: a node, by the conversion
//! ```
//!
//! - **`place`**, any of the four: a relation's column declared `place` (a finding's place, a metavariable a regex
//!   or a node binds) takes any of them from a rule's head. A value read from it is a `place`, no other type: a rule
//!   that needs a node of it converts it (`place_to_node`, …), as from any other type.
//! - **core.rerail's own reads.** The engine's reads that take a node or a span alike (`text`, `docof`) are declared
//!   over `place`: in a rule, a `place` column is a node's. A line's, a match's and a token's readers are typed
//!   relations of their own (`line_text`, `match_text`, `token_text`, …), which the lowering reads as the engine's
//!   (`lower::core::ALIASES`). The engine's reads a typed one stands for (`line`, `src_line`, `first_token`, …) are
//!   not read by a rule at all: each has a typed name.
//! - **Regexes.** `match(re, x)` is a text test: over a symbol, as Soufflé's, its named groups are symbols; over a
//!   place it tests the place's text, as `glob` and every text test does, and its named groups are matches. A regex's
//!   every match in a place is typed by its subject's type:
//!   `node_regex_match(re, n, m)` (each match `m` of `re` in `n`'s text) and `node_regex_group(re, n, m, name, g)`,
//!   and the same over a line, a match and a token. A match and a named group of one are `match`es.

use std::collections::{BTreeMap, BTreeSet};

use super::clause::{At, L, LitAt, Reads, T};
use super::from_tape::{S, Statements};
use super::tape::Field;
use crate::code_rule_dsl::logic::front::Error;

/// The place types: a variable is at most one of them.
const PLACES: [&str; 5] = ["node", "line", "match", "token", "place"];

/// Whether a written column type is a place (decision 0724 §3): an id the engine holds as a node's.
pub fn is_place(ty: &str) -> bool {
    PLACES.contains(&ty)
}

/// The engine's reads that a rule reads through a typed name instead: each with the names to read.
/// Each engine read a typed name lowers to, and the name a printed program writes it by: the one of them that takes
/// any place (`lower::core::ALIASES`). The lowering holds every place as a node (`lower::ty`), so a print cannot say
/// which typed name a rule wrote; this one lowers to the same read, and the check takes it for any one type.
const PRINTED: &[(&str, &str)] = &[
    ("text", "place_text"),
    ("docof", "place_document"),
    ("line", "place_line_number"),
    ("end_line", "place_end_line_number"),
    ("col", "place_start_column"),
    ("source", "document_source"),
    ("src_line", "document_line_by_number"),
    ("first_token", "place_to_first_token"),
    ("last_token", "place_to_last_token"),
    ("token_before", "place_to_token_before"),
    ("token_after", "place_to_token_after"),
    ("end_point", "place_to_end_position"),
];

/// The name a printed program writes engine read `rel` by (`PRINTED`), where a rule cannot read it by its own.
pub(crate) fn printed_name(rel: &str) -> Option<&'static str> {
    PRINTED.iter().find(|(r, _)| *r == rel).map(|(_, p)| *p)
}

const ENGINE_ONLY: &[(&str, &str)] = &[
    ("line", "node_line_number(n, k) for a node's line number, node_to_line(n, l) for its line, line_number, token_line_number or match_line_number"),
    ("end_line", "node_end_line_number(n, k), node_to_end_line(n, l), token_end_line_number or match_end_line_number"),
    ("col", "node_start_column(n, c), token_start_column or match_start_column"),
    ("source", "document_source(d, l)"),
    ("src_line", "document_line(d, l), with line_number(l, k) and line_text(l, s), or document_line_by_number(d, k, l)"),
    ("first_token", "node_to_first_token(n, t)"),
    ("last_token", "node_to_last_token(n, t)"),
    ("token_before", "node_to_token_before(n, t) or token_previous(t, p)"),
    ("token_after", "node_to_token_after(n, t) or token_next(t, x)"),
    ("end_point", "node_to_end_position(n, p)"),
];

const CORE: &str = include_str!("../../../../../../../docs/rule-syntax/lib/core.rerail");

/// core.rerail's declarations, their column types as written (the engine's reads' `place` read as a node's), read
/// off its `.decl` lines' text: a declaration of core's is one line.
fn core_decls() -> &'static BTreeMap<String, Vec<String>> {
    static DECLS: std::sync::LazyLock<BTreeMap<String, Vec<String>>> = std::sync::LazyLock::new(|| {
        let mut out = BTreeMap::new();
        for line in CORE.lines() {
            let Some(rest) = line.strip_prefix(".decl ") else { continue };
            let Some((name, cols)) = rest.split_once('(') else { continue };
            let Some((cols, _)) = cols.split_once(')') else { continue };
            let name = name.trim().to_owned();
            // The engine's reads over `place` are a node's in a rule; `place_*`'s are a place's, and `unset`'s (the
            // empty value a metavariable its match left unbound takes) any place's.
            let own = name.starts_with("place_") || name == "unset";
            let types = cols
                .split(',')
                .map(|c| c.split_once(':').map_or("", |(_, t)| t).split_whitespace().next().unwrap_or("").to_owned())
                .map(|t| if t == "place" && !own { "node".to_owned() } else { t })
                .collect();
            out.insert(name, types);
        }
        out
    });
    &DECLS
}

/// Each variable's place types in one clause, as its columns, its regexes and its equalities give them.
struct Seen<'f, F: Reads> {
    f: &'f F,
    /// The relations the program declares as its own under a name of core's: an engine read's name (a program in T's
    /// shape, which reads no core.rerail, has its own `line` or `src_line`), or another arity than core's.
    own: &'f BTreeSet<String>,
    types: BTreeMap<String, BTreeSet<&'static str>>,
    same: Vec<(String, String)>,
}

impl<F: Reads> Seen<'_, F> {
    fn add(&mut self, v: &str, t: &str) {
        if let Some(p) = PLACES.iter().find(|p| **p == t) {
            self.types.entry(v.to_owned()).or_default().insert(p);
        }
    }
    fn var(&self, a: &At) -> Option<String> {
        match self.f.tv(a) {
            T::Var(v) => Some(v),
            _ => None,
        }
    }
    /// A head's columns (`head`) or a body atom's: a `place` column takes any place in a head, and gives a `place`,
    /// which is no other type, in a body.
    fn args(&mut self, cols: Option<&Vec<String>>, args: &[At], head: bool) {
        for (i, a) in args.iter().enumerate() {
            if let (Some(v), Some(t)) = (self.var(a), cols.and_then(|c| c.get(i))) {
                if !(head && t == "place") {
                    self.add(&v, t);
                }
            }
        }
    }
    fn lits(&mut self, decls: &BTreeMap<String, Vec<String>>, lits: Vec<LitAt>, line: usize) -> Result<(), Error> {
        let mut stack = lits;
        while let Some(l) = stack.pop() {
            let written = self.f.written_rel(&l);
            match self.f.lit(&l) {
                L::Atom { rel, args, .. } => {
                    let written = written.unwrap_or_else(|| rel.clone());
                    self.atom(decls, &written, &rel, &args, line)?;
                }
                L::Group { alts, .. } => stack.extend(alts.into_iter().flatten()),
                L::Cmp { l: a, op, r: b } => {
                    if op == "="
                        && let (Some(x), Some(y)) = (self.var(&a), self.var(&b))
                    {
                        self.same.push((x, y));
                    }
                    for t in [&a, &b] {
                        if let T::Agg { body, .. } = self.f.tv(t) {
                            stack.extend(body);
                        }
                    }
                }
                L::Pattern { .. } => {}
            }
        }
        Ok(())
    }
    fn atom(&mut self, decls: &BTreeMap<String, Vec<String>>, written: &str, rel: &str, args: &[At], line: usize) -> Result<(), Error> {
        if let Some((_, names)) = ENGINE_ONLY.iter().find(|(r, _)| *r == written).filter(|_| !self.own.contains(written)) {
            return Err(Error::Syntax(format!("line {line}: `{written}` is the engine's read of a node or a span alike: read {names} (decision 0724 §4)")));
        }
        let groups = |me: &Self| match args.first().map(|a| me.f.tv(a)) {
            Some(T::Str(re)) => super::lower::groups(&re),
            _ => Vec::new(),
        };
        match (written, super::clause::regex_read(written)) {
            (_, Some((read, ty))) => {
                if let Some(x) = args.get(1).and_then(|a| self.var(a)) {
                    self.add(&x, ty);
                }
                let at: &[usize] = if read == "match_all" { &[2] } else { &[2, 4] };
                let ms: Vec<String> = at.iter().filter_map(|&i| args.get(i)).filter_map(|a| self.var(a)).collect();
                for v in ms {
                    self.add(&v, "match");
                }
                if read == "match_all" {
                    for g in groups(self) {
                        self.add(&g, "match");
                    }
                }
            }
            ("match_all", None) if args.len() == 3 => {
                return Err(Error::Syntax(format!("line {line}: `match_all(re, x, m)` over a place is typed by x's type: node_regex_match, line_regex_match, match_regex_match or token_regex_match (decision 0724 §4)")));
            }
            ("match_group", None) => {
                return Err(Error::Syntax(format!("line {line}: `match_group` is typed by its subject's type: node_regex_group, line_regex_group, match_regex_group or token_regex_group (decision 0724 §4)")));
            }
            // `match(re, s)` and `match_all(re, s, …)` over a symbol: checked once the subject's types are known.
            ("match" | "match_all", None) => {}
            _ => self.args(decls.get(written).or_else(|| decls.get(rel)), args, false),
        }
        Ok(())
    }
    /// The types joined through `x = y`, to a fixpoint.
    fn join(&mut self) {
        loop {
            let mut changed = false;
            for (x, y) in self.same.clone() {
                let all: BTreeSet<&'static str> = self.types.get(&x).into_iter().chain(self.types.get(&y)).flatten().copied().collect();
                for v in [&x, &y] {
                    let e = self.types.entry(v.clone()).or_default();
                    if e.len() != all.len() {
                        *e = all.clone();
                        changed = true;
                    }
                }
            }
            if !changed {
                return;
            }
        }
    }
}

/// A clause's `match(re, x)`s: each subject and its regex's named groups. Over a place, `match` tests its text, as
/// every text test does, and its groups are matches; over a symbol they are symbols.
fn text_matches<F: Reads>(f: &F, lits: Vec<LitAt>, out: &mut Vec<(String, Vec<String>)>) {
    let mut stack = lits;
    while let Some(l) = stack.pop() {
        let written = f.written_rel(&l);
        match f.lit(&l) {
            L::Atom { rel, args, neg: false } if rel == "match" && written.as_deref().unwrap_or("match") == "match" => {
                if let (Some(T::Str(re)), Some(T::Var(v))) = (args.first().map(|a| f.tv(a)), args.get(1).map(|a| f.tv(a))) {
                    out.push((v, super::lower::groups(&re)));
                }
            }
            L::Group { alts, .. } => stack.extend(alts.into_iter().flatten()),
            L::Cmp { l: a, r: b, .. } => {
                for t in [&a, &b] {
                    if let T::Agg { body, .. } = f.tv(t) {
                        stack.extend(body);
                    }
                }
            }
            _ => {}
        }
    }
}

/// `check`, with the column types `given` declares besides the statements' own (a module's includes'): an error at
/// the first variable of two place types, and at the first engine read a rule reads by its own name.
pub(super) fn check_in(statements: &Statements, given: &BTreeMap<String, Vec<String>>) -> Result<(), Error> {
    let mut decls: BTreeMap<String, Vec<String>> = given.clone();
    let mut own = BTreeSet::new();
    for s in statements.iter() {
        let (t, n) = s.tape();
        // A `.violation`'s columns are a relation's as a `.decl`'s are: its `at: node` takes no line.
        let read = match s.what() {
            Some(S::Decl(_)) => t.decl(n).map(|(name, cols, _)| (name, cols)),
            Some(S::Violation(_)) => t.violation(n).map(|(name, cols, _)| (name, cols)),
            _ => continue,
        };
        if let Ok((name, cols)) = read {
            // Its own relation where core has none of the name's arity, or reads it by the engine's name alone.
            if ENGINE_ONLY.iter().any(|(r, _)| *r == name) || core_decls().get(&name).is_some_and(|core| core.len() != cols.len()) {
                own.insert(name.clone());
            }
            decls.insert(name, cols.into_iter().map(|(_, t)| t).collect());
        }
    }
    // core's own relations are read as core types them for a rule (its engine reads' `place` as a node's), whatever
    // a program that includes or repeats their declarations wrote.
    for (rel, cols) in core_decls().iter().filter(|(rel, _)| !own.contains(*rel)) {
        decls.insert(rel.clone(), cols.clone());
    }
    for s in statements.iter().filter(|s| s.what() == Some(S::Clause)) {
        let (f, n) = (s.file().as_ref(), s.node());
        let line = s.line();
        let body: Vec<LitAt> = f.tape().get(n, Field::Body).map_or_else(Vec::new, |b| f.body_at(b, 0));
        // Each alternative of a disjunction is a rule of its own, as the lowering makes it: its variables are typed in
        // it alone (`rr_at = n ; rr_at = SPAN` gives `rr_at` a node in one and a match in the other).
        for body in conjunctions(f, body) {
            check_clause(f, n, line, &decls, &own, body)?;
        }
    }
    Ok(())
}

/// The most conjunctions a clause's disjunctions are checked as apart; past it, their alternatives are checked
/// together.
const MOST_CONJUNCTIONS: usize = 64;

/// `body`'s conjunctions: one for each choice of an alternative in each of its disjunctions (nested ones too), or
/// `body` itself where they are more than `MOST_CONJUNCTIONS`.
fn conjunctions<F: Reads>(f: &F, body: Vec<LitAt>) -> Vec<Vec<LitAt>> {
    fn expand<F: Reads>(f: &F, lits: &[LitAt]) -> Option<Vec<Vec<LitAt>>> {
        let mut out: Vec<Vec<LitAt>> = vec![Vec::new()];
        for l in lits {
            match f.lit(l) {
                L::Group { alts, .. } => {
                    let mut each = Vec::new();
                    for alt in &alts {
                        each.extend(expand(f, alt)?);
                    }
                    let mut next = Vec::with_capacity(out.len() * each.len());
                    for o in &out {
                        for e in &each {
                            next.push(o.iter().cloned().chain(e.iter().cloned()).collect());
                        }
                    }
                    if next.len() > MOST_CONJUNCTIONS {
                        return None;
                    }
                    out = next;
                }
                _ => out.iter_mut().for_each(|o| o.push(l.clone())),
            }
        }
        Some(out)
    }
    expand(f, &body).unwrap_or_else(|| vec![body])
}

/// One conjunction of clause `n`'s body checked: an error at its first variable of two place types.
fn check_clause<F: Reads>(f: &F, n: u32, line: usize, decls: &BTreeMap<String, Vec<String>>, own: &BTreeSet<String>, body: Vec<LitAt>) -> Result<(), Error> {
    {
        let mut seen = Seen { f, own, types: BTreeMap::new(), same: Vec::new() };
        for h in f.heads_at(n) {
            seen.args(decls.get(&h.rel), &h.args, true);
        }
        seen.lits(decls, body.clone(), line)?;
        let mut matches = Vec::new();
        text_matches(f, body, &mut matches);
        // A match's groups are matches once its subject is a place: to a fixpoint through the equalities.
        loop {
            seen.join();
            let before: usize = seen.types.values().map(BTreeSet::len).sum();
            for (x, groups) in &matches {
                if seen.types.get(x).is_some_and(|t| !t.is_empty()) {
                    for g in groups {
                        seen.add(g, "match");
                    }
                }
            }
            if seen.types.values().map(BTreeSet::len).sum::<usize>() == before {
                break;
            }
        }
        for (v, types) in &seen.types {
            // A `place` column holds any place: it agrees with the one type the variable's other columns give it.
            let types: BTreeSet<&&str> = types.iter().filter(|t| **t != "place").collect();
            if types.len() > 1 {
                let mut ts = types.iter().copied();
                let (a, b) = (ts.next().unwrap_or(&""), ts.next().unwrap_or(&""));
                return Err(Error::Syntax(format!(
                    "line {line}: {v} is a {a} in one column and a {b} in another: give each column one type, and convert with a function (node_to_line, line_to_node, match_to_node, node_to_first_token, …: decision 0724 §4)"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Decision 0724 §4: each name a print writes an engine read by lowers to that read again, so a print reads back.
    #[test]
    fn printed_names_lower_to_their_reads() {
        for (read, name) in super::PRINTED {
            let lowered = super::super::lower::core::alias(name).map(|(_, r, _)| r);
            assert_eq!(lowered, Some(*read), "{name} lowers to {read}");
        }
        for (read, _) in super::ENGINE_ONLY {
            assert!(super::PRINTED.iter().any(|(r, _)| r == read), "{read}, refused by its own name, has a printed name");
        }
    }
}
