//! The token reads a `.rr` program may not make (t31, RER-131). A named-only tape stores no token its grammar
//! implies (the `(` of a call, the `return` of a return statement), and no relation holds one: `op`, `op_at`,
//! `prev_token` and `token` give a node's stored children, `first_token`, `last_token`, `token_before` and
//! `token_after` its stored leaves. A clause that asks one of them for a token the grammar may leave out, by its
//! text, could never see it, so the program is refused, naming the clause. The rule asks about the named structure
//! instead (a `parenthesized_expression` parent, a field, a child's place).
//!
//! The text is the relation's own argument (`op(n, "(")`) or a test on the token (`text(t, "(")`, `match("re", t)`,
//! `t = "("`), in the clause or, where the token goes to the head, where the head's relation is read. The parent is
//! the kind the program gives the node: `kind(n, "k")`, `node(_, n, "k", _, _)`, a kind from a relation of kind
//! facts (`node(_, n, k, _, _), list_kind(k)`), or a column of a relation whose every rule gives it known kinds.
//! Where it gives none, any kind; for a neighbour (`prev_token`) or a leaf of the node (the four leaf relations),
//! any kind the token may sit under.
//!
//! A relation whose rules read a token where the grammar keeps it says so, as a JSON rule's `stored` does:
//! `.stored perf_paren_target "a one-element tuple_pattern's comma is its trailing comma"`. Its rules' reads are
//! not refused; the reason is required.

use std::collections::{BTreeMap, BTreeSet};

use super::Error;
use super::syntax::{Head, Item, Lit, Term};

/// The relations: the argument holding the node, the one holding the token's text (`None`: the token is the
/// second argument, a node whose text the program tests), and whether the token is the node's own child.
const READS: &[(&str, usize, Option<usize>, bool)] = &[
    ("op", 0, Some(1), true),
    ("op_at", 0, Some(2), true),
    ("token", 0, Some(2), true),
    ("prev_token", 0, Some(1), false),
    ("first_token", 0, None, false),
    ("last_token", 0, None, false),
    ("token_before", 0, None, false),
    ("token_after", 0, None, false),
];

/// A clause's literals, groups and negations flattened: `(relation, arguments, negated)`.
fn atoms<'a>(lits: &'a [Lit], out: &mut Vec<(&'a str, &'a [Term], bool)>) {
    for lit in lits {
        match lit {
            Lit::Atom { rel, args, neg } => out.push((rel, args, *neg)),
            Lit::Group { alts, .. } => alts.iter().for_each(|alt| atoms(alt, out)),
            _ => {}
        }
    }
}

fn eqs<'a>(lits: &'a [Lit], out: &mut Vec<(&'a Term, &'a Term)>) {
    for lit in lits {
        match lit {
            Lit::Cmp { l, op: "=", r } => out.push((l, r)),
            Lit::Group { alts, .. } => alts.iter().for_each(|alt| eqs(alt, out)),
            _ => {}
        }
    }
}

fn var(term: &Term) -> Option<&str> {
    if let Term::Var(v) = term { Some(v) } else { None }
}

fn exact(s: &str) -> String {
    format!("^{}$", regex::escape(s))
}

/// One clause, flattened.
struct Clause<'a> {
    file: &'a std::path::Path,
    line: usize,
    heads: &'a [Head],
    atoms: Vec<(&'a str, &'a [Term], bool)>,
    eqs: Vec<(&'a Term, &'a Term)>,
}

impl Clause<'_> {
    /// The regexes the clause gives variable `v`'s text: `match("re", v)`, `text(v, "lit")`, `v = "lit"`.
    fn texts(&self, v: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (rel, args, _) in &self.atoms {
            match (*rel, *args) {
                ("text", [Term::Var(x), Term::Str(s)]) if x == v => out.push(exact(s)),
                ("match", [Term::Str(re), Term::Var(x)]) if x == v => out.push(format!("^(?:{re})$")),
                _ => {}
            }
        }
        for (l, r) in &self.eqs {
            match (l, r) {
                (Term::Var(x), Term::Str(s)) | (Term::Str(s), Term::Var(x)) if x == v => out.push(exact(s)),
                _ => {}
            }
        }
        out
    }
}

/// What the program says of a relation's column: the kinds its nodes have (`None`: any), and the texts it is
/// tested against where it is read.
struct Columns {
    facts: BTreeMap<String, BTreeSet<String>>,
    kinds: BTreeMap<(String, usize), Option<BTreeSet<String>>>,
    texts: BTreeMap<(String, usize), Vec<String>>,
}

impl Columns {
    fn new(clauses: &[Clause<'_>]) -> Self {
        // Relations of kinds: unary relations given by facts alone (`list_kind("tuple").`).
        let mut facts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut derived: BTreeSet<&str> = BTreeSet::new();
        for clause in clauses {
            for head in clause.heads {
                match (clause.atoms.is_empty(), head.args.as_slice()) {
                    (true, [Term::Str(s)]) => {
                        facts.entry(head.rel.clone()).or_default().insert(s.clone());
                    }
                    _ => {
                        derived.insert(&head.rel);
                    }
                }
            }
        }
        facts.retain(|rel, _| !derived.contains(rel.as_str()));
        // The kinds of each derived relation's columns, to a fixpoint: a column is known where every rule for its
        // relation gives the variable there known kinds.
        let mut kinds: BTreeMap<(String, usize), Option<BTreeSet<String>>> = BTreeMap::new();
        for _ in 0..16 {
            let mut next: BTreeMap<(String, usize), Option<BTreeSet<String>>> = BTreeMap::new();
            for clause in clauses.iter().filter(|c| !c.atoms.is_empty()) {
                let given = Self::clause_kinds(clause, &facts, &kinds);
                for head in clause.heads {
                    for (at, arg) in head.args.iter().enumerate() {
                        let here = match arg {
                            Term::Var(v) => given.get(v.as_str()).cloned(),
                            _ => None,
                        };
                        let entry = next.entry((head.rel.clone(), at)).or_insert_with(|| Some(BTreeSet::new()));
                        *entry = match (entry.take(), here) {
                            (Some(mut a), Some(b)) => {
                                a.extend(b);
                                Some(a)
                            }
                            _ => None,
                        };
                    }
                }
            }
            if next == kinds {
                break;
            }
            kinds = next;
        }
        kinds.retain(|_, set| set.as_ref().is_some_and(|set| !set.is_empty()));
        // The texts a relation's column is tested against where it is read.
        let mut texts: BTreeMap<(String, usize), Vec<String>> = BTreeMap::new();
        for clause in clauses {
            for (rel, args, _) in &clause.atoms {
                for (at, arg) in args.iter().enumerate() {
                    let found = match arg {
                        Term::Var(v) => clause.texts(v),
                        Term::Str(s) => vec![exact(s)],
                        _ => Vec::new(),
                    };
                    if !found.is_empty() {
                        texts.entry(((*rel).to_owned(), at)).or_default().extend(found);
                    }
                }
            }
        }
        Self { facts, kinds, texts }
    }

    /// The kinds a clause gives its variables.
    fn clause_kinds(clause: &Clause<'_>, facts: &BTreeMap<String, BTreeSet<String>>, columns: &BTreeMap<(String, usize), Option<BTreeSet<String>>>) -> BTreeMap<String, BTreeSet<String>> {
        let mut given: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let kind_of = |term: &Term| -> Option<BTreeSet<String>> {
            match term {
                Term::Str(k) => Some(BTreeSet::from([k.clone()])),
                Term::Var(k) => clause.atoms.iter().find_map(|(rel, args, neg)| match args {
                    [Term::Var(x)] if x == k && !neg => facts.get(*rel).cloned(),
                    _ => None,
                }),
                _ => None,
            }
        };
        let mut give = |v: &str, set: BTreeSet<String>| {
            given.entry(v.to_owned()).and_modify(|have| *have = have.intersection(&set).cloned().collect()).or_insert(set);
        };
        for (rel, args, neg) in &clause.atoms {
            if *neg {
                continue;
            }
            match (*rel, *args) {
                // A kind, a kind from a relation of kinds; `has(m, c, "k")`'s descendant; `inside(n, a, kinds)` and
                // `nearest(n, a, kinds)`'s ancestor, its kinds a relation of kinds.
                ("kind", [Term::Var(n), k]) | ("node", [_, Term::Var(n), k, _, _]) => {
                    if let Some(set) = kind_of(k) {
                        give(n, set);
                    }
                }
                ("has", [_, Term::Var(n), Term::Str(k)]) => give(n, BTreeSet::from([k.clone()])),
                ("has", [_, Term::Var(n), Term::Var(set)]) if facts.contains_key(set.as_str()) => give(n, facts[set.as_str()].clone()),
                ("inside" | "nearest", [_, Term::Var(a), Term::Var(set)]) if facts.contains_key(set.as_str()) => {
                    give(a, facts[set.as_str()].clone());
                }
                _ => {
                    for (at, arg) in args.iter().enumerate() {
                        if let (Term::Var(n), Some(Some(set))) = (arg, columns.get(&((*rel).to_owned(), at))) {
                            give(n, set.clone());
                        }
                    }
                }
            }
        }
        given
    }
}

/// Refuses a program whose clauses read a token the tape may leave out (above), for the program's `.language`.
pub(super) fn refuse(files: &[(std::path::PathBuf, Vec<super::syntax::Statement>)]) -> Result<(), Error> {
    let language = files.iter().flat_map(|(_, statements)| statements).find_map(|(item, _)| match item {
        Item::Setting { key, text } if key == "language" => Some(text.trim().trim_matches('"').to_owned()),
        _ => None,
    });
    let Some(dialect) = language.as_deref().and_then(crate::code_rule_dsl::language::dialect_by_label) else { return Ok(()) };
    let clauses: Vec<Clause<'_>> = files
        .iter()
        .flat_map(|(file, statements)| statements.iter().map(move |s| (file, s)))
        .filter_map(|(file, (item, line))| match item {
            Item::Clause { heads, body, .. } => {
                let (mut all, mut equal) = (Vec::new(), Vec::new());
                atoms(body, &mut all);
                eqs(body, &mut equal);
                Some(Clause { file, line: *line, heads, atoms: all, eqs: equal })
            }
            _ => None,
        })
        .collect();
    // `.stored r "why"`: r's rules read tokens the grammar keeps where they look.
    let mut stored = BTreeSet::new();
    for (item, line) in files.iter().flat_map(|(_, statements)| statements) {
        if let Item::Setting { key, text } = item
            && key == "stored"
        {
            let (rel, why) = text.trim().split_once(char::is_whitespace).unwrap_or((text.trim(), ""));
            if why.trim().trim_matches('"').trim().is_empty() {
                return Err(Error::Syntax(format!("line {line}: `.stored {rel}` needs the reason the token is stored there")));
            }
            stored.insert(rel.to_owned());
        }
    }
    let columns = Columns::new(&clauses);
    let mut refused = BTreeSet::new();
    for clause in clauses.iter().filter(|c| !c.heads.iter().any(|h| stored.contains(&h.rel))) {
        let given = Columns::clause_kinds(clause, &columns.facts, &columns.kinds);
        for (rel, args, _) in &clause.atoms {
            let Some(&(_, node, text, own)) = READS.iter().find(|(r, ..)| r == rel) else { continue };
            let token = match text {
                Some(at) => args.get(at),
                None => args.get(1),
            };
            let mut regexes = match token {
                Some(Term::Str(s)) if text.is_some() => vec![exact(s)],
                Some(Term::Var(v)) => {
                    let mut found = clause.texts(v);
                    // Where the token's text goes to the head, the tests where the head is read.
                    for head in clause.heads {
                        for (at, arg) in head.args.iter().enumerate() {
                            if var(arg) == Some(v.as_str()) {
                                found.extend(columns.texts.get(&(head.rel.clone(), at)).into_iter().flatten().cloned());
                            }
                        }
                    }
                    found
                }
                _ => Vec::new(),
            };
            regexes.sort();
            regexes.dedup();
            let at = args.get(node).and_then(var).and_then(|n| given.get(n));
            // The token's parent: the node (its own token, or its first or last leaf), or the node's parent (the
            // token beside it).
            let parents = match (own || matches!(*rel, "first_token" | "last_token"), at) {
                (true, Some(kinds)) => Some(kinds.clone()),
                (false, Some(kinds)) => Some(crate::code_rule_dsl::compile::token_tests::parents_of(dialect, kinds)),
                _ => None,
            };
            for re in regexes {
                let Ok(regex) = regex::Regex::new(&re) else { continue };
                let found = crate::code_rule_dsl::compile::token_tests::implied(dialect, parents.as_ref(), &regex);
                if let Some((kind, token)) = found.iter().next() {
                    let more = if found.len() > 1 { format!(" (and {} more)", found.len() - 1) } else { String::new() };
                    let parent = if parents.is_some() { "" } else { ", and the clause names no parent kind" };
                    refused.insert(format!("{}:{}: {rel} reads `{token}`, which the tape leaves out in {kind}{more}{parent}", clause.file.display(), clause.line));
                }
            }
        }
    }
    if refused.is_empty() {
        return Ok(());
    }
    Err(Error::Syntax(format!(
        "a clause reads a token the tape leaves out; ask about the named structure instead:\n{}",
        refused.into_iter().collect::<Vec<_>>().join("\n")
    )))
}
