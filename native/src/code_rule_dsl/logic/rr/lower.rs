//! `.rr` statements to the engine's `Program`: the same program the engine plans whatever the syntax.
//!
//! Soufflé spellings map to the engine's forms:
//! - `number`, `symbol` are `int`, `sym`; `span` and `loc` are `node`;
//! - a lattice column `v: flat<>` (or `min<>`, `max<>`), or `v: L<>` for `.lattice L<base> { Lub -> @f }`, is the
//!   engine's lattice on that relation, and `widen k` after the declaration bounds it;
//! - `.pragma "magic-transform" "a, b"` is demand on `a` and `b` (`"*"`: every derived relation);
//! - the text tests `glob(p, s)`, `prefix`, `suffix`, `contains`, `word` and their `i` forms, and `match(re, s)`,
//!   are the engine's tests (`logic::strings`), pattern first; a disjunction of one test over several patterns
//!   `(glob("a", s) ; glob("b", s))` is one test of the set, and so are adjacent negations of one test;
//!   `(s = "a" ; s = "b")` is one membership test;
//! - a relation of facts given to `nearest`, `inside` or `has` is a kind set, which the transducer answers:
//!   `inside(n, a, s)` is `rr_inside_<s>(n, a)`, and `has(a, n, s)` is `rr_has_<s>(n, a)`, the set tested on n; a
//!   set of nodes is read as climbs up the tree (`Lowerer::node_axis`), and a set of kinds rules derive as rules
//!   over `child` (`Lowerer::derived_axes`), which read the set negated, so it is complete first;
//! - `@dot` is the engine's `@cat`, and `split_dot(q, m, x)` is `q = @dot(m, x)` with x dot-free (it runs either
//!   way);
//! - `count : { … }`, `min x : { … }`, `max x : { … }`, `sum x : { … }` and `join x by k sep "s" : { … }` are
//!   aggregates; a body of more than one atom, or of one built-in the lowering writes out, gets a helper relation
//!   (count and sum keep every variable: they read a multiset of rows; join reads the distinct rows of its value
//!   and keys); a value or key that is an expression is a variable the body computes; join's keys are variables of
//!   its body, ordered by text for symbols and by value for numbers, ties by the value;
//! - an aggregate inside a term (`1 + count : { … }`, in a head or a comparison) is a variable bound beside it
//!   (`Lowerer::hoist`); an aggregate on one side of a comparison gets a variable;
//! - `!(a, b)` is a helper relation of the variables it shares with the rule, negated
//!   (`Lowerer::negated_conjunction`); `!(a ; b)` is one per alternative;
//! - other disjunctions are one rule per alternative.
//!
//! - `.violation V(at: loc, …)` declares V as an output, and each row of V is a finding
//!   `violation(policy, V, code, path, line)`: at a node, its document's path and its line; at a span, its `docof`
//!   document and its line; a head `V(@place(path, line), …)` writes V's place relation, and a `@place(path, line)`
//!   value elsewhere (the loc value `-2 - (path << 32 | line)`) is found at that path and line. The code is V,
//!   `code "X"`, or the `code {c}` column. `suppress TOOL "rule", … [over {c}]`, `code {c} in "re"` and
//!   `fix [safe] F "title"` are kept as the settings `suppress V …`, `code_in V …` and `fix V …` for the findings
//!   layer, which applies them;
//! - a rule ending in `=> edits` defines a fix, and fixes are not run: it is left out;
//! - `.input r(IO=…, filename="…")` is an input, its parameters kept as the setting `input`;
//! - core.rr is the prelude (`rr::program` reads it before every file): the program keeps the core relations it reads
//!   (a relation read and not declared takes its declaration there); `next_sibling`, `precedes`, `earlier` and
//!   `syntax_error` lower to the relations the engine has (`core`), negated through a relation of their rows;
//!   `kind` and T's `docof` over nodes are projections of `node`, and core.rr's `docof(x: loc, d)` an input;
//! - `within(n, a)` is the engine's relation read off the tree, and `within_stop(n, a, stop)` that less a stop
//!   between (`Lowerer::within`, decision 0050), and so is its negation; `!nearest(n, a, s)` is a helper relation;
//!   `precedes_stop(a, b, S)` is a relation over `child` (`Lowerer::precedes_stop`);
//! - `unset(x)` is the empty value -1, whose text is "", and `!unset(x)` is `x != -1`;
//! - `match("(?<x>…)", s)` binds x to the named group (`Lowerer::captures`); `match_all` and `match_group` are the
//!   text layer's relations (`strings::as_relations`); `+ - * / %` and Soufflé's `cat` of any arity (`concat`
//!   folded from the left), `strlen`, `to_string`, `substr`, `to_number`, `to_float`, `max/2` and `min/2` are
//!   functors. `to_number` reads as Soufflé's `symbol2numeric`; where Soufflé stops with an error (no digits, out
//!   of range, `x % 0`) there is no value, and so no row. `@lower` is Unicode lowercase; `@dot("", b)` is b; a
//!   decimal number is its float's key (`float_key`).
//!
//! Not lowered yet, each a named error: patterns (``n ~ `…` ``), `ordered by`, `mean` (a decimal number),
//! `dfs_first`/`dfs_first_memo`, and a negated built-in with its other node unnamed (`!within_stop(n, _, s)`,
//! `!split_dot(q, _, x)`).

mod core;
#[cfg(test)]
pub(in crate::code_rule_dsl) use self::core::drop_unread_core;
pub(in crate::code_rule_dsl) use self::core::is_core;

use std::collections::{BTreeMap, BTreeSet};

use super::syntax::{Code, Fix, Head, Item, Lit, Statement, Suppress, Term as STerm, quote};
use crate::code_rule_dsl::logic::front::{AXIS, AggOp, Atom, DESCENDANTS, Error, LatticeDecl, Literal, Native, Program, RESERVED, Rule, SCOPE_NOT, SpanIn, Term, Value, axis};
use crate::code_rule_dsl::facts::vtab::syntax::GUEST_POLICY;
use crate::code_rule_dsl::logic::strings::SET;
mod findings;

/// The text tests: their `.rr` names and the engine's.
pub(in crate::code_rule_dsl) const TESTS: [(&str, &str); 11] = [
    ("glob", "glob"),
    ("iglob", "iglob"),
    ("prefix", "prefix"),
    ("iprefix", "iprefix"),
    ("suffix", "suffix"),
    ("isuffix", "isuffix"),
    ("contains", "contains"),
    ("icontains", "icontains"),
    ("word", "word"),
    ("iword", "iword"),
    ("match", "rx"),
];

fn test_of(rel: &str) -> Option<&'static str> {
    TESTS.iter().find(|(rr, _)| *rr == rel).map(|(_, engine)| *engine)
}

/// The whole-string regex of a pattern computed in a rule, `p`: `^(?:p)$` made as the row's pattern is.
pub(super) fn anchored(p: Term) -> Term {
    let concat = |a: Term, b: Term| Term::Call("concat".to_owned(), vec![a, b]);
    concat(Term::Str("^(?:".to_owned()), concat(p, Term::Str(")$".to_owned())))
}

/// The pattern `anchored` made `p` of, if `p` is one.
pub(super) fn unanchored(p: &Term) -> Option<&Term> {
    match p {
        Term::Call(f, args) if f == "concat" => match args.as_slice() {
            [Term::Str(open), Term::Call(g, inner)] if open == "^(?:" && g == "concat" => match inner.as_slice() {
                [p, Term::Str(close)] if close == ")$" => Some(p),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// The built-ins that take a relation, and its position.
pub(super) const RELATION_ARG: [(&str, &[usize]); 13] = [
    ("within_stop", &[2]),
    ("nearest", &[2]),
    ("precedes_stop", &[2]),
    ("inside", &[2]),
    ("has", &[2]),
    ("dfs_first", &[3, 4]),
    ("dfs_first_memo", &[3, 4, 5]),
    ("components", &[3, 4]),
    ("python_candidates", &[4,5]),
    ("python_qualified", &[2,3]),
    ("python_origins", &[2]),
    ("python_assigned", &[2]),
    ("python_arguments", &[5]),
];

fn not_yet<T>(what: impl std::fmt::Display) -> Result<T, Error> {
    Err(Error::Syntax(format!("not lowered yet: {what}")))
}

/// What a set given to an axis holds (`Lowerer::members`).
enum Members {
    /// Kinds known when the program is read: a kind set, which the transducer answers.
    Kinds(Vec<String>),
    /// Kinds that rules derive from the tree (`doc_code_kind(k) :- kind(_, k), k != "comment"`).
    Derived,
}

/// A column type in the engine's names.
fn ty(t: &str) -> String {
    match t {
        "number" | "unsigned" => "int".to_owned(),
        // A tape is stored as a symbol (its content key); `tape_type` checks it before lowering.
        "symbol" | "tape" => "sym".to_owned(),
        "span" | "loc" => "node".to_owned(),
        // A float column holds floats' keys (`float_key`).
        "float" => "int".to_owned(),
        other => other.to_owned(),
    }
}

/// A `.violation V(at, …)`: its relation, its columns, its code, and whether its `at` is a node (else a span or a
/// place); `places`: whether a rule gives it a `@place(path, line)`, kept in its place relation (`place_relation`).
struct Violation {
    name: String,
    arity: usize,
    code: FindingCode,
    node: bool,
    places: bool,
    columns: Vec<(String, String)>,
    message: String,
    tag: Option<usize>,
}

/// A finding's code: the violation's name, a column's value, or a constant.
enum FindingCode {
    Name,
    Column(usize),
    Constant(String),
}

/// The relation of the `@place(path, line)` rows of a violation V: `(path, line, V's other columns)`.
fn place_relation(v: &str) -> String {
    format!("rr_place_{v}")
}

#[derive(Default)]
struct Lowerer {
    prog: Program,
    /// `.lattice` declarations: name → (base type, its `Lub` functor).
    lattices: BTreeMap<String, (String, String)>,
    /// Relations given as a kind set.
    sets: BTreeSet<String>,
    /// `.violation`s, by relation.
    violations: Vec<Violation>,
    next: usize,
    /// The literals beside the one being lowered, in its conjunction and those enclosing it: what binds an
    /// aggregate's shared variables (`aggregate`).
    siblings: Vec<Lit>,
    /// The relations the clause being lowered writes: a context leaves their atoms out where it can (`context`).
    heads: BTreeSet<String>,
    /// A context for a `match_all` over a span (`aggregate`): made of the rule's literals, not inputs alone, so it
    /// binds the span where the rule does and `span_patterns` finds its containers (decision 0081).
    span_context: bool,
    /// Whether the clause being lowered binds a variable named `top` (`path_part(d, top)`); otherwise `top` is a
    /// lattice's top value (`v != top`), as the translated programs write it.
    top_is_var: bool,
    /// Whether `docof` is declared over `loc`s, as `core.rr` declares it: an input that spans have rows in
    /// (decision 0046), not the projection of `node`.
    docof_of_locs: bool,
    /// The relations declared `must, may`, and the rules a `must` or `may` head wrote (by index).
    three: BTreeSet<String>,
    modal: BTreeMap<usize, String>,
    /// The relations the program derives (rules or facts): core.rr's names among them are the program's own.
    derived: BTreeSet<String>,
    /// The demand relations the text declares itself (O101): a printed program (`rr::print`) carries the demands
    /// it was lowered with, so reading it back makes none again.
    printed_demands: BTreeSet<String>,
}

/// The globs a program's `.scope` settings select documents by: (within, out), `dirs` and `include` within, `exclude`
/// and `tests false` out. `None` for no scope and for `.scope given` (the files are a caller's, a test's, not found
/// by walking a repository). `report dirs`, `text` and `embedded` say where findings land and how files are read, not
/// which; an `option("…")` entry's value is a policy option's, not known here.
pub(super) fn scope_globs(settings: &[(String, String)]) -> Option<(Vec<String>, Vec<String>)> {
    if settings.iter().any(|(k, t)| k == "scope" && t.trim() == "given") {
        return None;
    }
    let (mut within, mut out) = (Vec::new(), Vec::new());
    for (_, text) in settings.iter().filter(|(k, _)| k == "scope") {
        // The setting's tokens: comments are not in them.
        let Ok(tokens) = super::syntax::tokens(text) else { continue };
        let tokens: Vec<&super::syntax::Tok> = tokens.iter().map(|t| &t.0).collect();
        let words: Vec<&str> = tokens.iter().filter_map(|t| if let super::syntax::Tok::Id(w) = t { Some(w.as_str()) } else { None }).collect();
        // `report dirs` and `text` say where findings land and how files are read, not which.
        if words.first().is_some_and(|w| matches!(*w, "report" | "text" | "embedded")) {
            continue;
        }
        let mut word = "";
        for (k, t) in tokens.iter().enumerate() {
            match t {
                // `option("…")`: an entry from a policy option, kept by name until the run's options are known
                // (`front::OPTION_GLOB`); the form before it (`dirs`, `include`, `exclude`) stays the word.
                super::syntax::Tok::Id(w) if w == "option" && matches!(tokens.get(k + 1), Some(super::syntax::Tok::P("("))) => {
                    if let Some(super::syntax::Tok::Str(name)) = tokens.get(k + 2) {
                        let entry = format!("{}{word}:{name}", crate::code_rule_dsl::logic::front::OPTION_GLOB);
                        match word {
                            "dirs" | "include" => within.push(entry),
                            "exclude" => out.push(entry),
                            _ => {}
                        }
                    }
                }
                super::syntax::Tok::Id(_) if matches!(tokens.get(k + 1), Some(super::syntax::Tok::P("("))) => word = "",
                super::syntax::Tok::Id(w) => {
                    if w == "false" && word == "tests" {
                        out.extend(["test/**", "**/test/**", "tests/**", "**/tests/**"].map(str::to_owned));
                    }
                    word = w.as_str();
                }
                super::syntax::Tok::Str(g) if !matches!(tokens.get(k.wrapping_sub(1)), Some(super::syntax::Tok::P("("))) => match word {
                    "dirs" => within.push(format!("{}/**", g.trim_end_matches('/'))),
                    "include" => within.push(g.clone()),
                    "exclude" => out.push(g.clone()),
                    _ => {}
                },
                _ => {}
            }
        }
    }
    if within.is_empty() && out.is_empty() {
        return None;
    }
    Some((within, out))
}

/// The globs a `.scope <form> "…", …` setting lists, for a form that is not about which documents are read (`text`,
/// `report dirs`).
fn scope_list(settings: &[(String, String)], form: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for (_, text) in settings.iter().filter(|(k, _)| k == "scope") {
        let Ok(tokens) = super::syntax::tokens(text) else { continue };
        let words: Vec<&str> = tokens.iter().take(form.len()).filter_map(|t| if let super::syntax::Tok::Id(w) = &t.0 { Some(w.as_str()) } else { None }).collect();
        if words != form {
            continue;
        }
        found.extend(tokens.iter().filter_map(|t| if let super::syntax::Tok::Str(g) = &t.0 { Some(g.clone()) } else { None }));
    }
    found
}

/// `.scope text "**/*.md", …`: documents read as text only (`front::Program::text_globs`).
pub(super) fn scope_text_globs(settings: &[(String, String)]) -> Vec<String> {
    scope_list(settings, &["text"])
}

/// `.scope report dirs "api", …`: where findings may land, as globs; facts still come from the whole scope.
fn scope_report_globs(settings: &[(String, String)]) -> Vec<String> {
    scope_list(settings, &["report", "dirs"]).into_iter().map(|d| format!("{}/**", d.trim_end_matches('/'))).collect()
}

pub(super) fn lower(statements: Vec<Statement>) -> Result<Program, Error> {
    super::tape_type::check(&statements)?;
    let mut l = Lowerer::default();
    let mut clauses = Vec::new();
    for (item, line) in statements {
        let at = |e: Error| match e {
            Error::Syntax(m) => Error::Syntax(format!("line {line}: {m}")),
            e => e,
        };
        match item {
            Item::Include(path) => return Err(Error::Syntax(format!("line {line}: #include {path:?} not resolved"))),
            Item::Decl { name, cols, tail } => l.decl(name, cols, &tail).map_err(at)?,
            Item::Edit { .. } | Item::Functor { .. } | Item::Type { .. } => {}
            // `.input r(IO=json, filename="…")`: r is an input whose rows come from that file (or artifact), kept
            // as the setting `input r(IO=…)` for the runner; without them r is empty.
            Item::Io { output, names, params } => {
                for (name, text) in params {
                    l.prog.settings.push((if output { "output" } else { "input" }.to_owned(), format!("{name}({text})")));
                }
                if output { &mut l.prog.outputs } else { &mut l.prog.inputs }.extend(names)
            }
            Item::Lattice { name, base, ops } => {
                let Some((_, STerm::Call { name: lub, functor: true, .. })) = ops.iter().find(|(op, _)| op == "Lub") else {
                    return Err(at(Error::Syntax(format!(".lattice {name} has no `Lub -> @f`"))));
                };
                l.lattices.insert(name, (ty(&base), lub.clone()));
            }
            Item::Pragma(words) => match words.as_slice() {
                [magic, relations] if magic == "magic-transform" => {
                    l.prog.demand.extend(relations.split(',').map(str::trim).filter(|r| !r.is_empty()).map(str::to_owned));
                }
                // `.pragma "cross-rounds" "k"` (decision 0056): kept as written, for the corpus runner.
                [key, k] if key == crate::code_rule_dsl::typing::cross_rounds::PRAGMA && k.parse::<usize>().is_ok() => {
                    l.prog.settings.push(crate::code_rule_dsl::typing::cross_rounds::setting(k));
                }
                // `.pragma "cross-memo" "r s"` (decision 0076): cross-file relations memoised per import component.
                [key, rels] if key == "cross-memo" => {
                    l.prog.settings.push(("pragma".to_owned(), format!("\"cross-memo\" \"{rels}\"")));
                }
                _ => return not_yet(format!(".pragma {words:?}")).map_err(at),
            },
            Item::Policy(document) => l.prog.policies.push(document),
            Item::Setting { key, text } => l.prog.settings.push((key, text)),
            Item::Violation { name, cols, code, code_in, tag, suppress, fixes, message, .. } => l.violation(name, cols, code, code_in, tag, suppress, fixes, message).map_err(at)?,
            Item::Clause { heads, body, edits } => {
                if body.is_empty() && edits.is_empty() && let Some(rows) = facts(&heads) {
                    for (rel, row) in rows {
                        l.prog.facts.entry(rel).or_default().push(row);
                    }
                    continue;
                }
                clauses.push((heads, body, edits, line));
            }
        }
    }
    l.derived = clauses.iter().flat_map(|(heads, ..)| heads.iter().map(|h| h.rel.clone())).chain(l.prog.facts.keys().cloned()).collect();
    l.printed_demands = DEMANDS.iter().chain(WORLD.iter().map(|(_, d)| d)).filter(|d| l.prog.decls.contains_key(**d)).map(|d| (*d).to_owned()).collect();
    for (heads, body, edits, line) in clauses {
        // A rule ending in `=> edits` defines a fix; fixes are not run, so it is left out.
        if !edits.is_empty() {
            continue;
        }
        l.clause(&heads, &body).map_err(|e| match e {
            Error::Syntax(m) => Error::Syntax(format!("line {line}: {m}")),
            e => e,
        })?;
    }
    l.three_valued();
    l.findings()?;
    l.declare_core()?;
    l.projections();
    l.span_patterns()?;
    l.grammar_names()?;
    // The built-ins lowering writes out (`nearest`, `within`, …) and the core relations it rewrites where the program
    // does not derive them (`next_sibling`, `precedes`) are declared in `core.rr` for the checker; they are no
    // relations of the program.
    // `within` stays where a rule reads it: the lowering made it the engine's relation (decision 0050).
    let reads_within = l.prog.rules.iter().flat_map(|r| &r.body).filter_map(Literal::atom).any(|a| a.rel == crate::code_rule_dsl::facts::vtab::syntax::WITHIN);
    let rewritten: Vec<&str> = self::core::REWRITTEN.into_iter().filter(|r| l.rewritten(r)).collect();
    for b in RELATION_ARG.iter().map(|(r, _)| *r).chain(["within", "earlier", "split_dot", "unset"]).chain(rewritten).filter(|b| !(reads_within && *b == "within")) {
        l.prog.decls.remove(b);
        l.prog.inputs.retain(|i| i != b);
    }
    // A program lowered without `core.rr` (its own part, `parse_own`) still declares the `within` it reads.
    if reads_within {
        use crate::code_rule_dsl::facts::vtab::syntax::WITHIN;
        l.prog.decls.entry(WITHIN.to_owned()).or_insert_with(|| vec![("n".to_owned(), "node".to_owned()), ("a".to_owned(), "node".to_owned())]);
        if !l.prog.inputs.iter().any(|i| i == WITHIN) {
            l.prog.inputs.push(WITHIN.to_owned());
        }
    }
    l.kind_sets()?;
    l.unit();
    lower_boolean_relations(&mut l.prog);
    // A printed program (its findings already rules) is read again with its `.scope` settings: the documents it
    // selects are the same.
    l.prog.text_globs = scope_text_globs(&l.prog.settings);
    if l.prog.doc_scopes.is_none() {
        l.prog.doc_scopes = scope_globs(&l.prog.settings).map(|g| vec![g]);
    }
    let mut program = crate::code_rule_dsl::logic::front::finish(l.prog)?;
    super::aggregate_demand::optimize(&mut program);
    Ok(program)
}

/// A nullary relation is a Boolean set: empty, or the single empty tuple. SQL tables need a column,
/// so represent that tuple with the integer unit. Apply the same encoding to facts, heads, and reads
/// (including aggregate reads); joins and negation then keep ordinary relational set semantics.
fn lower_boolean_relations(prog: &mut Program) {
    let names: BTreeSet<_> = prog.decls.iter().filter(|(_, cols)| cols.is_empty()).map(|(name, _)| name.clone()).collect();
    if names.is_empty() {
        return;
    }
    prog.nullary.extend(names.iter().cloned());
    for name in &names {
        prog.decls.get_mut(name).unwrap().push(("rr_unit".to_owned(), "int".to_owned()));
        if let Some(rows) = prog.facts.get_mut(name) {
            for row in rows {
                row.push(Value::Int(1));
            }
        }
    }
    let encode = |atom: &mut Atom| {
        if names.contains(atom.rel.as_str()) && atom.args.is_empty() {
            atom.args.push(Term::Int(1));
        }
    };
    for rule in &mut prog.rules {
        encode(&mut rule.head);
        for lit in &mut rule.body {
            if let Literal::Atom(atom) | Literal::Agg { atom, .. } = lit {
                encode(atom);
            }
        }
    }
}

/// A clause of constant heads: its facts.
fn facts(heads: &[Head]) -> Option<Vec<(String, Vec<Value>)>> {
    heads
        .iter()
        .map(|h| {
            let row = h
                .args
                .iter()
                .map(|a| match a {
                    STerm::Num(n) => Some(Value::Int(*n)),
                    STerm::Str(s) => Some(Value::Str(s.clone())),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            h.modal.is_none().then(|| (h.rel.clone(), row))
        })
        .collect()
}

impl Lowerer {
    /// A new name, none the program declares: a printed program (`rr::print`) declares the helpers it was lowered
    /// with, and lowering it again makes more.
    fn fresh(&mut self, stem: &str) -> String {
        loop {
            self.next += 1;
            let name = format!("rr_{stem}{}", self.next);
            if !self.prog.decls.contains_key(&name) {
                return name;
            }
        }
    }

    fn decl(&mut self, name: String, cols: Vec<(String, String)>, tail: &[String]) -> Result<(), Error> {
        let mut widen = None;
        let mut words = tail.iter();
        while let Some(w) = words.next() {
            match w.as_str() {
                // `must, may`: three-valued (`three_valued`).
                "must" | "may" | "," => {
                    self.three.insert(name.clone());
                }
                "widen" => widen = Some(words.next().and_then(|k| k.parse::<u32>().ok()).ok_or_else(|| Error::Syntax(format!("{name}: `widen` needs a count")))?),
                // What the relation keeps (decision 0099): an override of the planner's choice.
                q if crate::code_rule_dsl::logic::front::QUALIFIERS.contains(&q) => {
                    let own = self.prog.qualifiers.entry(name.clone()).or_default();
                    own.insert(q.to_owned());
                    for (a, b) in [("inline", "no_inline"), ("magic", "no_magic"), ("inline", "magic")] {
                        if own.contains(a) && own.contains(b) {
                            return Err(Error::Syntax(format!(".decl {name}: `{a}` and `{b}` together")));
                        }
                    }
                }
                other => return not_yet(format!("`{other}` on .decl {name}")),
            }
        }
        if name == "docof" {
            self.docof_of_locs = cols.first().is_some_and(|(_, t)| t == "loc");
        }
        let mut out = Vec::new();
        for (i, (c, t)) in cols.iter().enumerate() {
            if t == "tape" {
                self.prog.tapes.entry(name.clone()).or_default().insert(i);
            }
            let Some(lattice) = t.strip_suffix("<>") else {
                out.push((c.clone(), ty(t)));
                continue;
            };
            if i + 1 != cols.len() {
                return Err(Error::Syntax(format!("{name}: the lattice column {c} must be the last")));
            }
            let (column, merge) = match self.lattices.get(lattice) {
                Some((base, lub)) => (base.clone(), Some(lub.clone())),
                None if matches!(lattice, "flat" | "min" | "max") => (lattice.to_owned(), None),
                None => return Err(Error::Syntax(format!("{name}: no .lattice {lattice}"))),
            };
            self.prog.lattice.insert(name.clone(), LatticeDecl { keys: i, ty: column.clone(), merge, widen });
            out.push((c.clone(), column));
        }
        if widen.is_some() && !self.prog.lattice.contains_key(&name) {
            return Err(Error::Syntax(format!("{name}: `widen` on a relation with no lattice column")));
        }
        self.prog.decls.insert(name, out);
        Ok(())
    }

    /// `.decl r(…) must, may` (core.rr 3): r is computed twice. Its may side, which keeps the name, holds what is
    /// proven or unknown: every rule of r. Its must side (`rr_must_r`) holds what is proven: the rules with no
    /// prefix or `must`. A relation that reads a three-valued one is three-valued too, with every rule on both
    /// sides. In a must side's rule a positive atom of a three-valued relation reads its must side and a negated
    /// one its may side; a may side's rule reads the other way. A finding is the may side: a definite one and a
    /// possible one have the same key, and the possible tag is not made yet.
    fn three_valued(&mut self) {
        if self.three.is_empty() {
            return;
        }
        let must = |rel: &str| format!("{RESERVED}must_{rel}");
        let mut three = self.three.clone();
        loop {
            let before = three.len();
            for rule in &self.prog.rules {
                if rule.body.iter().any(|l| l.atom().is_some_and(|a| three.contains(a.rel.as_str()))) {
                    three.insert(rule.head.rel.to_string());
                }
            }
            if three.len() == before {
                break;
            }
        }
        let side = |body: &[Literal], must_side: bool| -> Vec<Literal> {
            body.iter()
                .cloned()
                .map(|mut l| {
                    if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = &mut l
                        && three.contains(a.rel.as_str())
                        && (a.neg != must_side)
                    {
                        a.rel = must(&a.rel).into();
                    }
                    l
                })
                .collect()
        };
        let mut rules = Vec::new();
        for (i, rule) in std::mem::take(&mut self.prog.rules).into_iter().enumerate() {
            if !three.contains(rule.head.rel.as_str()) {
                rules.push(rule);
                continue;
            }
            let modal = self.modal.get(&i).map(String::as_str);
            if modal != Some("may") {
                let mut head = rule.head.clone();
                head.rel = must(&head.rel).into();
                rules.push(Rule { head, body: side(&rule.body, true) });
            }
            rules.push(Rule { head: rule.head.clone(), body: side(&rule.body, false) });
        }
        self.prog.rules = rules;
        for rel in &three {
            if let Some(cols) = self.prog.decls.get(rel).cloned() {
                self.prog.decls.insert(must(rel), cols);
            }
            if let Some(rows) = self.prog.facts.get(rel).cloned() {
                self.prog.facts.insert(must(rel), rows);
            }
        }
    }

    /// `core.rr`'s `kind(n, k)` is a projection of `node`, and so is `docof(n, d)` as T's programs declare it (over
    /// nodes): rules, which the planner reads from `node` in place, not inputs, where a rule reads them (the prelude
    /// declares both for every program). `core.rr`'s `docof(x: loc, d)` stays an input, for spans have a document
    /// too. (`precedes` is core.rr's tree arithmetic, `core::rewrite`.)
    fn projections(&mut self) {
        let v = |s: &str| Term::Var(s.to_owned());
        let read: BTreeSet<String> = self.prog.rules.iter().flat_map(|r| r.body.iter().filter_map(Literal::atom)).map(|a| a.rel.to_string()).collect();
        for (rel, args) in [("kind", [Term::Wild, v("n"), v("k"), Term::Wild, Term::Wild]), ("docof", [v("d"), v("n"), Term::Wild, Term::Wild, Term::Wild])] {
            if !read.contains(rel) || !self.prog.inputs.iter().any(|i| i == rel) || (rel == "docof" && (self.docof_of_locs || !self.prog.decls.contains_key("node"))) {
                continue;
            }
            // `kind` is always `node`'s column: a program that reads it reads `node`.
            self.base("node");
            self.prog.inputs.retain(|i| i != rel);
            let head = if rel == "kind" { vec![v("n"), v("k")] } else { vec![v("n"), v("d")] };
            // First, kind before docof: where T's programs wrote them.
            let at = usize::from(rel == "docof" && self.prog.rules.first().is_some_and(|r| r.head.rel == "kind"));
            self.prog.rules.insert(at, Rule {
                head: Atom { rel: rel.to_owned().into(), args: head, neg: false, gated: false },
                body: vec![Literal::Atom(Atom { rel: "node".to_owned().into(), args: args.to_vec(), neg: false, gated: false })],
            });
        }
    }

    /// `.violation V(at, …)`: V is an output relation, and each of its rows is a finding (`findings`). Its
    /// `suppress` and `fix` lines and `code {c} in "re"` are settings of the findings layer, as `.mode` is:
    /// `suppress V tool "rule", … [over {c}]`, `fix V [safe] F "title"` and `code_in V "re"`.
    #[allow(clippy::too_many_arguments)]
    fn violation(&mut self, name: String, cols: Vec<(String, String)>, code: Option<Code>, code_in: Option<String>, tag: Option<String>, suppress: Vec<Suppress>, fixes: Vec<Fix>, message: String) -> Result<(), Error> {
        let Some((_, at)) = cols.first() else { return Err(Error::Syntax(format!(".violation {name} has no `at` column"))) };
        if !matches!(at.as_str(), "node" | "span" | "loc") {
            return Err(Error::Syntax(format!(".violation {name}: `at` is a node, a span or a loc, not a {at}")));
        }
        let node = at == "node";
        let column = |c: &String| cols.iter().position(|(n, _)| n == c).ok_or_else(|| Error::Syntax(format!(".violation {name}: no column {c}")));
        let tag = tag.as_ref().map(column).transpose()?;
        let code = match code {
            None => FindingCode::Name,
            Some(Code::Column(c)) => FindingCode::Column(column(&c)?),
            Some(Code::Constant(c)) => FindingCode::Constant(c),
        };
        for s in suppress {
            let rules = s.rules.iter().map(|r| quote(r)).collect::<Vec<_>>().join(", ");
            let over = s.over.map(|c| format!(" over {{{c}}}")).unwrap_or_default();
            self.prog.settings.push(("suppress".to_owned(), format!("{name} {} {rules}{over}", s.tool)));
        }
        for f in fixes {
            self.prog.settings.push(("fix".to_owned(), format!("{name} {}{} {}", if f.safe { "safe " } else { "" }, f.name, quote(&f.title))));
        }
        if let Some(re) = code_in {
            self.prog.settings.push(("code_in".to_owned(), format!("{name} {}", quote(&re))));
        }
        self.violations.push(Violation { name: name.clone(), arity: cols.len(), code, node, places: false, columns: cols.clone(), message, tag });
        self.decl(name.clone(), cols, &[])?;
        self.prog.outputs.push(name);
        Ok(())
    }

    /// The findings of the `.violation`s: `violation(policy, V, code, path, line)` for each row of V, at its first
    /// column's line (the engine's findings relation, as the converter's programs give it). A node's document is
    /// `node`'s column; a span's is `docof` (`core.rr`), and so is its line. A `@place(path, line)` row is read from
    /// V's place relation; a span or loc `at` that holds a `@place` value made elsewhere (`PLACE`) is found at its
    /// path and line.
    fn findings(&mut self) -> Result<(), Error> {
        // `.on syntax_error report CODE [at line 1]` and `.on input_error report CODE` (`core.rr`).
        let on: Vec<(String, String, bool)> = self
            .prog
            .settings
            .iter()
            .filter(|(k, _)| k == "on")
            .filter_map(|(_, text)| {
                let words: Vec<&str> = text.split_whitespace().collect();
                match words.as_slice() {
                    [what @ ("syntax_error" | "input_error"), "report", code, rest @ ..] => Some(((*what).to_owned(), (*code).to_owned(), rest == ["at", "line", "1"])),
                    _ => None,
                }
            })
            .collect();
        let guest_errors = self.guest_policy();
        if self.violations.is_empty() && on.is_empty() && guest_errors.is_empty() {
            return Ok(());
        }
        let policy = self
            .prog
            .settings
            .iter()
            .find(|(k, _)| k == "policy")
            .and_then(|(_, text)| text.split('"').nth(1))
            .unwrap_or_default()
            .to_owned();
        let scoped = self.scope_filter();
        self.prog.decls.insert("violation".to_owned(), [("rule", "sym"), ("clause", "sym"), ("code", "sym"), ("path", "sym"), ("line", "int")].iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
        self.prog.outputs.retain(|o| o != "violation");
        self.prog.outputs.push("violation".to_owned());
        let v = |s: &str| Term::Var(s.to_owned());
        let atom = |rel: &str, args: Vec<Term>| Literal::Atom(Atom { rel: rel.to_owned().into(), args, neg: false, gated: false });
        // Whether a rule makes a `@place` value: only then can a span or loc `at` hold one.
        let place_values = self.prog.rules.iter().any(|r| r.head.args.iter().chain(r.body.iter().flat_map(literal_terms)).any(has_place));
        for f in std::mem::take(&mut self.violations) {
            self.diagnostics(&policy, &f)?;
            let code_col = match f.code {
                FindingCode::Column(i) => Some(i),
                _ => None,
            };
            let code = match &f.code {
                FindingCode::Name => Term::Str(f.name.clone()),
                FindingCode::Column(i) => v(&format!("rr_v{i}")),
                FindingCode::Constant(c) => Term::Str(c.clone()),
            };
            let head = Atom { rel: "violation".to_owned().into(), args: vec![Term::Str(policy.clone()), Term::Str(f.name.clone()), code, v("rr_p"), v("rr_l")], neg: false, gated: false };
            let cols: Vec<Term> = (0..f.arity).map(|i| if i == 0 || Some(i) == code_col { v(&format!("rr_v{i}")) } else { Term::Wild }).collect();
            let at = if f.node {
                vec![atom("node", vec![v("rr_d"), v("rr_v0"), Term::Wild, Term::Wild, Term::Wild]), atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild]), atom("line", vec![v("rr_v0"), v("rr_l")])]
            } else {
                vec![atom("docof", vec![v("rr_v0"), v("rr_d")]), atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild]), atom("line", vec![v("rr_v0"), v("rr_l")])]
            };
            self.prog.rules.push(Rule { head: head.clone(), body: std::iter::once(atom(&f.name, cols.clone())).chain(at).chain(scoped.iter().cloned()).collect() });
            if !f.node && place_values {
                let of = |g: &str| Term::Call(g.to_owned(), vec![v("rr_v0")]);
                let mut found = head.clone();
                found.args[3] = of(PLACE_PATH);
                found.args[4] = of(PLACE_LINE);
                let mut body = vec![atom(&f.name, cols.clone()), Literal::Cmp { l: v("rr_v0"), op: "<", r: Term::Int(UNSET) }];
                // The scope tests the place's path, bound for it here.
                if !scoped.is_empty() {
                    body.push(Literal::Cmp { l: v("rr_p"), op: "=", r: of(PLACE_PATH) });
                    body.extend(scoped.iter().cloned());
                }
                self.prog.rules.push(Rule { head: found, body });
            }
            if f.places {
                let mut args = vec![v("rr_p"), v("rr_l")];
                args.extend(cols.into_iter().skip(1));
                self.prog.rules.push(Rule { head, body: std::iter::once(atom(&place_relation(&f.name), args)).chain(scoped.iter().cloned()).collect() });
            }
        }
        // A syntax error at its line (or line 1), in its document; an input error at line 1 of the input's path.
        for (what, code, line_one) in on {
            let head = Atom { rel: "violation".to_owned().into(), args: vec![Term::Str(policy.clone()), Term::Str(what.clone()), Term::Str(code.clone()), v("rr_p"), v("rr_l")], neg: false, gated: false };
            let body = if what == "syntax_error" {
                let line = if line_one { Term::Wild } else { v("rr_l") };
                // A file's own error: a guest's is `.on guest_error`'s.
                let not_guest = Literal::Atom(Atom { rel: "guest".to_owned().into(), args: vec![v("rr_d"), Term::Wild, Term::Wild, Term::Wild], neg: true, gated: false });
                let mut body = vec![atom("syntax_error", vec![v("rr_d"), line, Term::Wild]), atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild]), not_guest];
                body.extend(scoped.iter().cloned());
                if line_one {
                    body.push(Literal::Cmp { l: v("rr_l"), op: "=", r: Term::Int(1) });
                }
                body
            } else {
                vec![atom("input_error", vec![v("rr_p"), Term::Wild, Term::Wild]), Literal::Cmp { l: v("rr_l"), op: "=", r: Term::Int(1) }]
            };
            for rel in [what.as_str(), "doc"] {
                if !self.prog.inputs.iter().any(|i| i == rel) && self.prog.decls.contains_key(rel) {
                    self.prog.inputs.push(rel.to_owned());
                }
            }
            self.error_diagnostic(&policy, &what, &code, body.clone())?;
            if what == "syntax_error" {
                self.guest_input();
            }
            self.prog.rules.push(Rule { head, body });
        }
        // A guest that does not parse, reported at its host's line (`syntax_error` of a guest is a file line).
        for (grammar, code) in guest_errors {
            let head = Atom { rel: "violation".to_owned().into(), args: vec![Term::Str(policy.clone()), Term::Str("guest_error".to_owned()), Term::Str(code.clone()), v("rr_p"), v("rr_l")], neg: false, gated: false };
            let g = grammar.map_or(Term::Wild, Term::Str);
            let body = vec![atom("syntax_error", vec![v("rr_d"), v("rr_l"), Term::Wild]), atom("guest", vec![v("rr_d"), Term::Wild, Term::Wild, g]), atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild])];
            for (rel, cols) in [
                ("syntax_error", &[("d", "int"), ("l", "int"), ("msg", "sym")][..]),
                ("doc", &[("d", "int"), ("path", "sym"), ("module", "sym")][..]),
            ] {
                self.prog.decls.entry(rel.to_owned()).or_insert_with(|| cols.iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
                if !self.prog.inputs.iter().any(|i| i == rel) {
                    self.prog.inputs.push(rel.to_owned());
                }
            }
            self.guest_input();
            self.error_diagnostic(&policy, "guest_error", &code, body.clone())?;
            self.prog.rules.push(Rule { head, body });
        }
        Ok(())
    }

    /// The `.scope` settings as a test of a finding's path `$rr_p` (`core.rr`): a document's path is in scope in one
    /// of `dirs` (each a directory or a glob of one, at the root) or `include`'s globs when there are any, in none
    /// of `exclude`'s, and with `tests false` under no `test` or `tests` path part. Findings in other documents are
    /// dropped; one at a path that is no document (a budget file) is kept. Facts still come from every file. A
    /// `.scope given` program reads the files it is given, all in scope.
    fn scope_filter(&mut self) -> Vec<Literal> {
        let report = scope_report_globs(&self.prog.settings);
        let given = self.prog.settings.iter().any(|(k, t)| k == "scope" && t.trim() == "given");
        let (within, out) = match scope_globs(&self.prog.settings) {
            Some(globs) => globs,
            None if !report.is_empty() && !given => (Vec::new(), Vec::new()),
            None => return Vec::new(),
        };
        // The corpus reads only these documents (`front::Program::doc_scopes`): their facts are the policy's, as the
        // scripts' discovery has them; the drop below still holds for a combined run that reads more.
        if !within.is_empty() || !out.is_empty() {
            self.prog.doc_scopes = Some(vec![(within.clone(), out.clone())]);
        }
        let v = |s: &str| Term::Var(s.to_owned());
        let doc = Literal::Atom(Atom { rel: "doc".to_owned().into(), args: vec![v("d"), v("p"), Term::Wild], neg: false, gated: false });
        let glob = |g: &str| Literal::Cmp { l: Term::Call("glob".to_owned(), vec![v("p"), Term::Str(g.to_owned())]), op: "=", r: Term::Int(1) };
        // An entry from a policy option (`front::OPTION_GLOB`): the option's values, as `option_item` holds them; a
        // directory's documents are those under it.
        let option_entry = |g: &str| g.strip_prefix(crate::code_rule_dsl::logic::front::OPTION_GLOB).and_then(|r| r.split_once(':')).map(|(f, n)| (f.to_owned(), n.to_owned()));
        let by_option = |form: &str, name: &str| -> Vec<Literal> {
            let item = Literal::Atom(Atom { rel: "option_item".to_owned().into(), args: vec![Term::Str(name.to_owned()), Term::Wild, v("rr_o")], neg: false, gated: false });
            let test = if form == "dirs" {
                Literal::Cmp { l: Term::Call("prefix".to_owned(), vec![v("p"), Term::Call("concat".to_owned(), vec![v("rr_o"), Term::Str("/".to_owned())])]), op: "=", r: Term::Int(1) }
            } else {
                Literal::Cmp { l: Term::Call("glob".to_owned(), vec![v("p"), v("rr_o")]), op: "=", r: Term::Int(1) }
            };
            vec![item, test]
        };
        if within.iter().chain(&out).chain(&report).any(|g| option_entry(g).is_some()) {
            self.prog.decls.entry("option_item".to_owned()).or_insert_with(|| [("name", "sym"), ("i", "int"), ("v", "sym")].iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
            if !self.prog.inputs.iter().any(|i| i == "option_item") {
                self.prog.inputs.push("option_item".to_owned());
            }
        }
        let test = |g: &str| -> Vec<Literal> {
            match option_entry(g) {
                Some((form, name)) => by_option(&form, &name),
                None => vec![glob(g)],
            }
        };
        let atom = |rel: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: rel.to_owned().into(), args, neg, gated: false });
        let head = |rel: &str, var: &str| Atom { rel: rel.to_owned().into(), args: vec![v(var)], neg: false, gated: false };
        self.prog.decls.entry("doc".to_owned()).or_insert_with(|| [("d", "int"), ("path", "sym"), ("module", "sym")].iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
        if !self.prog.inputs.iter().any(|i| i == "doc") {
            self.prog.inputs.push("doc".to_owned());
        }
        // rr_scope_drop(p): a document's path out of the scope.
        self.prog.decls.insert("rr_scope_drop".to_owned(), vec![("p".to_owned(), "sym".to_owned())]);
        if !within.is_empty() {
            self.prog.decls.insert("rr_scope_in".to_owned(), vec![("d".to_owned(), "int".to_owned())]);
            for g in &within {
                self.prog.rules.push(Rule { head: head("rr_scope_in", "d"), body: std::iter::once(doc.clone()).chain(test(g)).collect() });
            }
            self.prog.rules.push(Rule { head: head("rr_scope_drop", "p"), body: vec![doc.clone(), atom("rr_scope_in", vec![v("d")], true)] });
        }
        for g in &out {
            self.prog.rules.push(Rule { head: head("rr_scope_drop", "p"), body: std::iter::once(doc.clone()).chain(test(g)).collect() });
        }
        // rr_scope_not(d): a document out of the scope, which a combined run's guards negate (decision 0091): in a
        // pack that reads more documents than this policy's scope, the policy's rules see only the others' rows.
        if !within.is_empty() || !out.is_empty() {
            self.prog.decls.insert(SCOPE_NOT.to_owned(), vec![("d".to_owned(), "int".to_owned())]);
            if !within.is_empty() {
                self.prog.rules.push(Rule { head: head(SCOPE_NOT, "d"), body: vec![doc.clone(), atom("rr_scope_in", vec![v("d")], true)] });
            }
            for g in &out {
                self.prog.rules.push(Rule { head: head(SCOPE_NOT, "d"), body: std::iter::once(doc.clone()).chain(test(g)).collect() });
            }
        }
        // `.scope report dirs`: a finding in a document under none of them is dropped too.
        if !report.is_empty() {
            self.prog.decls.insert("rr_report_in".to_owned(), vec![("d".to_owned(), "int".to_owned())]);
            for g in &report {
                self.prog.rules.push(Rule { head: head("rr_report_in", "d"), body: vec![doc.clone(), glob(g)] });
            }
            self.prog.rules.push(Rule { head: head("rr_scope_drop", "p"), body: vec![doc.clone(), atom("rr_report_in", vec![v("d")], true)] });
        }
        vec![atom("rr_scope_drop", vec![v("rr_p")], true)]
    }

    /// `guest(g, host, at, grammar)` as an input (`core.rr` declares it; a program without it has it here).
    fn guest_input(&mut self) {
        let cols = [("g", "int"), ("host", "int"), ("at", "node"), ("grammar", "sym")];
        self.prog.decls.entry("guest".to_owned()).or_insert_with(|| cols.iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
        if !self.prog.inputs.iter().any(|i| i == "guest") {
            self.prog.inputs.push("guest".to_owned());
        }
    }

    /// Decision 0055's settings as `guest_policy` facts, which the corpus runner and the batch read:
    /// - `.guests on | off`: `guest_policy("library", …)`, whether the libraries' `embed`s are made (on by default);
    /// - `.on guest_error [GRAMMAR] skip | partial [warn | report CODE]`, or `.on guest_error [GRAMMAR] warn | report
    ///   CODE` (skip): `guest_policy("errors" | "errors:GRAMMAR", "skip" | "partial")`. A skipped guest is
    ///   withheld; a partial one keeps the tree it has. `warn` and `report` both give a finding, `warn`'s of severity
    ///   warning (`.severity guest_error warning`); the findings are returned, `(grammar, code)`, for `findings`.
    fn guest_policy(&mut self) -> Vec<(Option<String>, String)> {
        let mut facts = Vec::new();
        let mut findings = Vec::new();
        let mut warn = false;
        for (key, text) in &self.prog.settings {
            let words: Vec<&str> = text.split_whitespace().collect();
            if key == "guests" {
                if let [v @ ("on" | "off")] = words.as_slice() {
                    facts.push(("library".to_owned(), (*v).to_owned()));
                }
                continue;
            }
            // `.scope embedded false` keeps the policy to primary documents: `.guests off` (`core.rr`).
            if key == "scope" {
                if let ["embedded", v @ ("true" | "false")] = words.as_slice() {
                    facts.push(("library".to_owned(), if *v == "true" { "on" } else { "off" }.to_owned()));
                }
                continue;
            }
            let ["guest_error", rest @ ..] = words.as_slice() else { continue };
            if key != "on" {
                continue;
            }
            let (grammar, rest) = match rest {
                [g, rest @ ..] if !matches!(*g, "skip" | "partial" | "warn" | "report") => (Some((*g).to_owned()), rest),
                _ => (None, rest),
            };
            let (mode, rest) = match rest {
                [m @ ("skip" | "partial"), rest @ ..] => (*m, rest),
                _ => ("skip", rest),
            };
            facts.push((grammar.as_ref().map_or("errors".to_owned(), |g| format!("errors:{g}")), mode.to_owned()));
            if let [how @ ("warn" | "report"), code] = rest {
                warn |= *how == "warn";
                findings.push((grammar, (*code).to_owned()));
            }
        }
        if warn {
            self.prog.settings.push(("severity".to_owned(), "guest_error warning".to_owned()));
        }
        // The libraries' `embed`s are made by default: a program that reads the setting and sets none reads
        // `guest_policy("library", "on")`, a stated fact, so `!guest_policy("library", "off")` is decided before the
        // run (`simplify`) instead of reading a relation nothing gives.
        let reads = self.prog.rules.iter().flat_map(|r| &r.body).filter_map(Literal::atom).any(|a| a.rel == GUEST_POLICY);
        if reads && !facts.iter().any(|(k, _)| k == "library") {
            facts.push(("library".to_owned(), "on".to_owned()));
        }
        if !facts.is_empty() {
            self.prog.decls.insert(GUEST_POLICY.to_owned(), vec![("k".to_owned(), "sym".to_owned()), ("v".to_owned(), "sym".to_owned())]);
            let rows = self.prog.facts.entry(GUEST_POLICY.to_owned()).or_default();
            rows.extend(facts.into_iter().map(|(k, v)| vec![Value::Str(k), Value::Str(v)]));
        }
        findings
    }

    fn clause(&mut self, heads: &[Head], body: &[Lit]) -> Result<(), Error> {
        fn binds_top(lits: &[Lit]) -> bool {
            lits.iter().any(|l| match l {
                Lit::Atom { neg: false, args, .. } => args.iter().any(|a| matches!(a, STerm::Var(v) if v == "top")),
                Lit::Group { alts, neg: false } => alts.iter().any(|a| binds_top(a)),
                _ => false,
            })
        }
        self.top_is_var = binds_top(body);
        self.heads = heads.iter().map(|h| h.rel.clone()).collect();
        // An aggregate in a head's argument (`cc(f, 1 + count : { … })`) is a variable the body binds.
        let mut heads = heads.to_vec();
        let mut body = body.to_vec();
        for a in heads.iter_mut().flat_map(|h| &mut h.args) {
            self.hoist(a, &mut body, true);
        }
        let heads = heads.as_slice();
        let mut outer = Vec::new();
        heads.iter().flat_map(|h| &h.args).for_each(|a| a.vars(&mut outer));
        let bodies = self.conjunction(&body, &outer.into_iter().collect())?;
        for head in heads {
            if let Some(m) = &head.modal
                && !self.three.contains(&head.rel)
            {
                return Err(Error::Syntax(format!("`{m}` head {}: the relation is not declared `must, may`", head.rel)));
            }
            // `V(@place(path, line), …)` for a `.violation V`: a row of V's place relation.
            let mut rel = head.rel.clone();
            let mut written = head.args.clone();
            if let Some(STerm::Call { name, functor: true, args: place }) = head.args.first()
                && name == "place"
                && let Some(k) = self.violations.iter().position(|v| v.name == head.rel)
            {
                let [path, line] = place.as_slice() else { return Err(Error::Syntax("@place takes a path and a line".to_owned())) };
                if !self.violations[k].places {
                    self.violations[k].places = true;
                    let mut cols = vec![("path".to_owned(), "sym".to_owned()), ("line".to_owned(), "int".to_owned())];
                    cols.extend(self.prog.decls[&head.rel].iter().skip(1).cloned());
                    self.prog.decls.insert(place_relation(&head.rel), cols);
                }
                rel = place_relation(&head.rel);
                written = [path.clone(), line.clone()].into_iter().chain(head.args.iter().skip(1).cloned()).collect();
            }
            let args = written.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
            for body in &bodies {
                let (args, body) = unset(args.clone(), body.clone())?;
                if let Some(m) = &head.modal {
                    self.modal.insert(self.prog.rules.len(), m.clone());
                }
                self.prog.rules.push(Rule { head: Atom { rel: rel.clone().into(), args, neg: false, gated: false }, body });
            }
        }
        Ok(())
    }

    /// A conjunction's alternatives (one per choice in its disjunctions).
    /// `context`: the variables used outside the conjunction.
    fn conjunction(&mut self, lits: &[Lit], context: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        // An aggregate inside a term (`n = count : { … } + 1`) is a variable bound beside it.
        let mut hoisted = Vec::new();
        for lit in lits {
            let mut lit = lit.clone();
            match &mut lit {
                Lit::Atom { args, .. } => args.iter_mut().for_each(|a| self.hoist(a, &mut hoisted, true)),
                Lit::Cmp { l, r, .. } => {
                    // `count : { … } = count : { … }`: the right one gets a variable.
                    let both = matches!((&*l, &*r), (STerm::Agg { .. }, STerm::Agg { .. }));
                    self.hoist(l, &mut hoisted, false);
                    self.hoist(r, &mut hoisted, both);
                }
                Lit::Pattern { .. } | Lit::Group { .. } => {}
            }
            hoisted.push(lit);
        }
        let lits = hoisted.as_slice();
        let mut acc = vec![Vec::new()];
        for (k, lit) in lits.iter().enumerate() {
            // The variables used outside this literal, which an aggregate inside it is correlated on.
            let mut outer: Vec<String> = context.iter().cloned().collect();
            lits.iter().enumerate().filter(|(j, _)| *j != k).for_each(|(_, l)| l.scope_vars(&mut outer));
            let enclosing = self.siblings.clone();
            self.siblings = lits.iter().enumerate().filter(|(j, _)| *j != k).map(|(_, l)| l.clone()).chain(enclosing.iter().cloned()).collect();
            let alts = self.literal(lit, &outer.into_iter().collect());
            self.siblings = enclosing;
            let alts = alts?;
            acc = acc.iter().flat_map(|a| alts.iter().map(move |b| a.iter().chain(b).cloned().collect::<Vec<_>>())).collect();
        }
        Ok(acc.into_iter().map(merge_negated_tests).collect())
    }

    /// Each aggregate inside `t` (and `t` itself when `whole`) as a fresh variable, bound by `v = aggregate` in `out`.
    fn hoist(&mut self, t: &mut STerm, out: &mut Vec<Lit>, whole: bool) {
        match t {
            STerm::Agg { .. } if whole => {
                let v = STerm::Var(self.fresh("agg"));
                let agg = std::mem::replace(t, v.clone());
                out.push(Lit::Cmp { l: v, op: "=", r: agg });
            }
            // A tape operation (decision 0058 §4) is a value the engine makes on demand: `t = @parse(g, x)` is
            // `parse_of(x, g, t)`, `x = @unparse(t)` is `unparse_of(t, x)`, `t3 = @splice(t, n, t2)` is
            // `splice_of(t, n, t2, t3)`.
            STerm::Call { name, functor: true, args } if TAPE_OPS.iter().any(|(f, n, _, _)| f == name && *n == args.len()) => {
                args.iter_mut().for_each(|a| self.hoist(a, out, true));
                let &(_, _, rel, cols) = TAPE_OPS.iter().find(|(f, n, _, _)| f == name && *n == args.len()).expect("matched");
                let v = STerm::Var(self.fresh("tape"));
                let mut atom_args = match rel {
                    "parse_of" => vec![args[1].clone(), args[0].clone()],
                    _ => args.clone(),
                };
                atom_args.push(v.clone());
                *t = v;
                out.push(Lit::Atom { rel: rel.to_owned(), args: atom_args, neg: false });
                if !self.prog.inputs.iter().any(|i| i == rel) {
                    self.prog.inputs.push(rel.to_owned());
                    self.prog.decls.insert(rel.to_owned(), cols.iter().map(|(c, ty)| ((*c).to_owned(), (*ty).to_owned())).collect());
                    self.prog.tapes.entry(rel.to_owned()).or_default().extend(super::tape_type::tape_columns(rel));
                }
            }
            // `@tree_digest(n)` reads the tree: a variable bound by the engine's `tree_digest(n, h)`.
            STerm::Call { name, functor: true, args } if name == "tree_digest" && args.len() == 1 => {
                self.hoist(&mut args[0], out, true);
                let n = args[0].clone();
                let v = STerm::Var(self.fresh("digest"));
                *t = v.clone();
                out.push(Lit::Atom { rel: "tree_digest".to_owned(), args: vec![n, v], neg: false });
                if !self.prog.inputs.iter().any(|i| i == "tree_digest") {
                    self.prog.inputs.push("tree_digest".to_owned());
                    self.prog.decls.insert("tree_digest".to_owned(), vec![("n".to_owned(), "node".to_owned()), ("s".to_owned(), "sym".to_owned())]);
                }
            }
            STerm::Call { args, .. } => args.iter_mut().for_each(|a| self.hoist(a, out, true)),
            STerm::Bin { l, r, .. } => {
                self.hoist(l, out, true);
                self.hoist(r, out, true);
            }
            _ => {}
        }
    }

    fn literal(&mut self, lit: &Lit, outer: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        Ok(match lit {
            Lit::Group { alts, neg } => {
                if let Some(mut set) = self.test_set(alts, *neg)? {
                    if let Some(Literal::Cmp { r, .. }) = set.last_mut() {
                        *r = Term::Int(if *neg { 0 } else { 1 });
                    }
                    return Ok(vec![set]);
                }
                // `!(a, b)`: no row of the conjunction, through a helper relation of the variables it shares with the
                // rule; `!(a ; b)` is `!a, !b`.
                if *neg {
                    let mut out = Vec::new();
                    for alt in alts {
                        out.push(self.negated_conjunction(alt, outer)?);
                    }
                    return Ok(vec![out]);
                }
                let mut out = Vec::new();
                for alt in alts {
                    out.extend(self.conjunction(alt, outer)?);
                }
                out
            }
            Lit::Pattern { subject, pattern, neg } => self.pattern(subject, pattern, *neg)?,
            Lit::Atom { rel, args, neg } if matches!(rel.as_str(), "nearest" | "inside" | "has" | "within_stop") && args.len() == 3 && self.node_set(&args[2]).is_some() => {
                self.node_axis(rel, args, *neg, outer)?
            }
            Lit::Atom { rel, args, neg } if matches!(rel.as_str(), "within" | "within_stop") => self.within(rel, args, *neg)?,
            Lit::Atom { rel, args, neg } if rel == "split_dot" => self.split_dot(args, *neg)?,
            // `match_all("re", sp, m)` (three arguments: each match in span sp, a span m, as the linter's regex finds
            // them) and `match_group` are relations the batches make (`facts::vtab::syntax`'s `Extra`); the symbol
            // forms of `match_all` (two or five arguments) and a `match` with captures are the text layer's (0054).
            Lit::Atom { rel, args, neg: false }
                if rel == "match_all" && args.len() != 3 || rel == "match" && matches!(args.as_slice(), [STerm::Str(re), _] if !groups(re).is_empty()) =>
            {
                self.text_matches(rel, args)?
            }
            // A text test on a node or span tests its text (`core.rr`).
            Lit::Atom { rel, args, neg }
                if test_of(rel).is_some() && matches!(args.as_slice(), [_, STerm::Var(v)] if self.span_container(v).is_some() || self.node_typed(v)) =>
            {
                self.text_test(rel, args, *neg)?
            }
            // `!match_all("re", s)`: no match anywhere in s, a search (captures bind nothing outside the `!`).
            Lit::Atom { rel, args, neg: true } if rel == "match_all" && args.len() != 3 => {
                let [STerm::Str(re), s] = args.as_slice() else { return Err(Error::Syntax("match_all takes a regex and a text".to_owned())) };
                vec![vec![Literal::Cmp { l: Term::Call("rx".to_owned(), vec![self.term(s)?, Term::Str(re.clone())]), op: "=", r: Term::Int(0) }]]
            }
            Lit::Atom { rel, args, neg } => {
                // core.rr's tree arithmetic (`earlier`, `precedes`, …) as the relations the engine has (`core`).
                if let Some(literals) = self.rewrite(rel, args, *neg)? {
                    return Ok(vec![literals]);
                }
                if !*neg && matches!(rel.as_str(), "parsed" | "sub_tape") {
                    self.embed_demand(args)?;
                }
                if !*neg && matches!(rel.as_str(), "parse_of" | "splice_of") {
                    self.value_demand(rel, args)?;
                }
                if let Some(demand) = world_demand(rel) {
                    self.world_demand(demand, args)?;
                }
                // A computed argument (`loads(@dot(m, x))`) is bound to a variable first: the engine joins on
                // variables (`front::finish`'s computed-term check).
                let mut atom = self.atom(rel, args, *neg)?;
                let mut out = Vec::new();
                if let Literal::Atom(a) = &mut atom {
                    for t in &mut a.args {
                        if matches!(t, Term::Call(..)) {
                            let v = self.fresh("t");
                            out.push(Literal::Cmp { l: Term::Var(v.clone()), op: "=", r: std::mem::replace(t, Term::Var(v)) });
                        }
                    }
                }
                out.push(atom);
                vec![out]
            }
            Lit::Cmp { l, op, r } => self.comparison(l, op, r, outer)?,
        })
    }

    /// `parsed(s, "g", r)` or `sub_tape(s, "g", t)` asks for the guest at s (decision 0055): `rr_embed_req(s, "g")`.
    /// A grammar that is not a constant asks for nothing: the atom reads the guests other rules or libraries asked for.
    fn embed_demand(&mut self, args: &[STerm]) -> Result<(), Error> {
        let [s @ STerm::Var(_), g @ STerm::Str(_), out] = args else { return Ok(()) };
        self.demand("rr_embed_req", &[(s.clone(), "node"), (g.clone(), "sym")], out, true)
    }

    /// A value a rule computes from a tape operation (decision 0058 §4) asks for it: `parse_of(x, g, t)` is
    /// `rr_parse_req(at, g, x)` (the text last, as it may hold a tab; `at` a location of the rule's context, so the
    /// demand is its document's), `splice_of(t, n, t2, t3)` is `rr_splice_req(n, t2)` (n names its tape).
    /// `unparse_of` asks for nothing: every guest and value has its text.
    fn value_demand(&mut self, rel: &str, args: &[STerm]) -> Result<(), Error> {
        match (rel, args) {
            ("parse_of", [x, g, t]) => self.demand("rr_parse_req", &[(STerm::Var(LOCATED.to_owned()), "int"), (g.clone(), "sym"), (x.clone(), "sym")], t, false),
            ("splice_of", [_, n, t2, t3]) => self.demand("rr_splice_req", &[(n.clone(), "node"), (t2.clone(), "sym")], t3, false),
            _ => Ok(()),
        }
    }

    /// A demand `req(given…)` over the rule's other literals that bind the given variables, those that do not read
    /// what is asked for (not `out`; with `guests`, no guest relation either: a guest is what they wait for). A value
    /// may be made from documents of earlier rounds (`@splice` of a node of a `@parse`), so its demand reads guests.
    /// The corpus runner runs these rules first, makes what is asked for and rebinds the batch with it. A given
    /// variable nothing else binds asks for nothing.
    fn demand(&mut self, req: &str, given: &[(STerm, &str)], out: &STerm, guests: bool) -> Result<(), Error> {
        if self.printed_demands.contains(req) {
            return Ok(());
        }
        let vars: Vec<String> = given.iter().filter_map(|(t, _)| if let STerm::Var(v) = t { Some(v.clone()) } else { None }).collect();
        if given.iter().any(|(t, _)| !matches!(t, STerm::Var(_) | STerm::Str(_))) {
            return Ok(());
        }
        // What reads the guest: the literals joined to `out` by variables other than the given ones, and the guest
        // relations.
        let mut reads = body_vars(&[Lit::Cmp { l: out.clone(), op: "=", r: out.clone() }]);
        reads.retain(|x| !vars.contains(x));
        let guest_rel = |l: &Lit| guests && matches!(l, Lit::Atom { rel, .. } if GUEST_RELATIONS.contains(&rel.as_str()));
        let mut keep: Vec<bool> = self.siblings.iter().map(|l| !guest_rel(l)).collect();
        for l in self.siblings.iter().filter(|l| guest_rel(l)) {
            reads.extend(body_vars(std::slice::from_ref(l)).into_iter().filter(|x| !vars.contains(x)));
        }
        loop {
            let mut grew = false;
            for (l, k) in self.siblings.iter().zip(keep.iter_mut()) {
                let vs = body_vars(std::slice::from_ref(l));
                if *k && vs.iter().any(|x| reads.contains(x)) {
                    *k = false;
                    reads.extend(vs.into_iter().filter(|x| !vars.contains(x)));
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        let saved = self.siblings.clone();
        let mut keep = keep.into_iter();
        self.siblings.retain(|_| keep.next().unwrap_or(false));
        // A wildcard in the context is a variable nothing else reads: named, it gives the file-local split the
        // context's location (`q(_, x)` reads q's rows of one document).
        let mut siblings = std::mem::take(&mut self.siblings);
        for l in &mut siblings {
            if let Lit::Atom { args, neg: false, .. } = l {
                for a in args.iter_mut().filter(|a| matches!(a, STerm::Wild)) {
                    *a = STerm::Var(self.fresh("w"));
                }
            }
        }
        self.siblings = siblings;
        // `LOCATED`: a location of the context, the document the demand is made in. A variable in a node column of a
        // kept atom, or a document of the chunk when the rule has no context.
        let located = given.iter().any(|(t, _)| matches!(t, STerm::Var(v) if v == LOCATED));
        let mut given: Vec<(STerm, &str)> = given.to_vec();
        let mut every_chunk_doc = false;
        if located {
            // A value's demand reads only what its inputs are joined to: a demand may ask for more than the rule
            // reads, never less, and one connected context is one document's.
            let mut inputs: BTreeSet<String> = given.iter().filter_map(|(t, _)| if let STerm::Var(v) = t { Some(v.clone()) } else { None }).filter(|v| v != LOCATED).collect();
            let mut joined = vec![false; self.siblings.len()];
            loop {
                let mut grew = false;
                for (l, j) in self.siblings.iter().zip(joined.iter_mut()) {
                    let vs = body_vars(std::slice::from_ref(l));
                    if !*j && vs.iter().any(|v| inputs.contains(v)) {
                        *j = true;
                        inputs.extend(vs);
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
            let mut joined = joined.into_iter();
            self.siblings.retain(|_| joined.next().unwrap_or(false));
            let loc = self.siblings.iter().find_map(|l| match l {
                Lit::Atom { rel, args, neg: false } => {
                    let decl = self.prog.decls.get(rel);
                    let doc = crate::code_rule_dsl::typing::file_local::document_column(rel);
                    args.iter().enumerate().find_map(|(c, a)| match a {
                        STerm::Var(v) if Some(c) == doc || decl.and_then(|d| d.get(c)).is_some_and(|(_, t)| t == "node") => Some(v.clone()),
                        _ => None,
                    })
                }
                _ => None,
            });
            match loc {
                Some(v) => given[0] = (STerm::Var(v), "int"),
                None => {
                    given[0] = (STerm::Var("rr_demand_doc".to_owned()), "int");
                    every_chunk_doc = true;
                }
            }
        }
        let vars: Vec<String> = given.iter().filter_map(|(t, _)| if let STerm::Var(v) = t { Some(v.clone()) } else { None }).filter(|v| v != "rr_demand_doc").collect();
        // Bound by a positive atom, or equal to a variable one binds (`t2 = rr_tape7`, a hoisted value).
        let binds = |v: &String, siblings: &[Lit]| {
            let by_atom = |v: &String| siblings.iter().any(|l| matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none() && rel != "earlier") && body_vars(std::slice::from_ref(l)).contains(v));
            by_atom(v)
                || siblings.iter().any(|l| match l {
                    Lit::Cmp { l: STerm::Var(a), op: "=", r: STerm::Var(b) } => (a == v && by_atom(b)) || (b == v && by_atom(a)),
                    _ => false,
                })
        };
        let all_bound = vars.iter().all(|v| binds(v, &self.siblings));
        // A demand of constants (`@parse("sql", "SELECT 1")`) is made where the rest of the rule holds (a nullary
        // context), or in every chunk when nothing else in the rule is independent of it.
        let ctx = if all_bound && !(vars.is_empty() && self.siblings.is_empty()) { Some(self.context(&vars)) } else { None };
        let every_chunk = every_chunk_doc || (all_bound && vars.is_empty() && ctx.is_none());
        self.siblings = saved;
        if !all_bound {
            return Ok(());
        }
        let ctx = ctx.transpose()?;
        self.prog.decls.insert(req.to_owned(), given.iter().enumerate().map(|(i, (_, ty))| (format!("c{i}"), (*ty).to_owned())).collect());
        let args: Vec<Term> = given.iter().map(|(t, _)| match t {
            STerm::Var(v) => Term::Var(v.clone()),
            STerm::Str(s) => Term::Str(s.clone()),
            _ => unreachable!("checked above"),
        }).collect();
        let head = Atom { rel: req.to_owned().into(), args, neg: false, gated: false };
        let mut body: Vec<Literal> = ctx.into_iter().map(Literal::Atom).collect();
        if every_chunk {
            body.push(Literal::Atom(Atom { rel: "doc".to_owned().into(), args: vec![Term::Var("rr_demand_doc".to_owned()), Term::Wild, Term::Wild], neg: false, gated: false }));
        }
        let rule = Rule { head, body };
        if !self.prog.rules.contains(&rule) {
            self.prog.rules.push(rule);
        }
        Ok(())
    }

    /// `dir_entry(d, …)` or `file_line(p, …)`, read or negated, asks for the listing of d or the lines of p (t21 E1):
    /// `rr_dir_req(d)` or `rr_file_req(p)` over the rule's other literals that bind it, those that read none of what
    /// the atom gives. The corpus runner answers the requests and runs the rules again (`corpus::world`). A key
    /// nothing else binds asks for nothing: the atom reads what other rules asked for.
    fn world_demand(&mut self, demand: &str, args: &[STerm]) -> Result<(), Error> {
        if self.printed_demands.contains(demand) {
            return Ok(());
        }
        let Some(key) = args.first() else { return Ok(()) };
        let head = |key: Term| Atom { rel: demand.to_owned().into(), args: vec![key], neg: false, gated: false };
        self.prog.decls.insert(demand.to_owned(), vec![("k".to_owned(), "sym".to_owned())]);
        let s = match key {
            // A constant key is a fact, as the reader takes a bodiless clause: printed and read back, it is the same.
            STerm::Str(k) => {
                let row = vec![Value::Str(k.clone())];
                let rows = self.prog.facts.entry(demand.to_owned()).or_default();
                if !rows.contains(&row) {
                    rows.push(row);
                }
                return Ok(());
            }
            STerm::Var(s) => s,
            _ => return Ok(()),
        };
        // What the atom gives: its other arguments' variables, and every literal joined to them.
        let mut reads = body_vars(&args[1..].iter().map(|a| Lit::Cmp { l: a.clone(), op: "=", r: a.clone() }).collect::<Vec<_>>());
        reads.remove(s);
        let mut keep: Vec<bool> = vec![true; self.siblings.len()];
        loop {
            let mut grew = false;
            for (l, k) in self.siblings.iter().zip(keep.iter_mut()) {
                let vars = body_vars(std::slice::from_ref(l));
                if *k && vars.iter().any(|x| reads.contains(x)) {
                    *k = false;
                    reads.extend(vars.into_iter().filter(|x| x != s));
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        let saved = self.siblings.clone();
        let mut keep = keep.into_iter();
        self.siblings.retain(|_| keep.next().unwrap_or(false));
        // Bound by an atom, or computed from what atoms bind (`d = cat(r, "/", p)`).
        let binds = self.siblings.iter().any(|l| match l {
            Lit::Atom { rel, neg: false, .. } => test_of(rel).is_none() && rel != "earlier" && body_vars(std::slice::from_ref(l)).contains(s),
            Lit::Cmp { l, op: "=", r } => matches!(l, STerm::Var(v) if v == s) || matches!(r, STerm::Var(v) if v == s),
            _ => false,
        });
        let ctx = if binds { Some(self.context(std::slice::from_ref(s))) } else { None };
        self.siblings = saved;
        let Some(ctx) = ctx.transpose()? else { return Ok(()) };
        let rule = Rule { head: head(Term::Var(s.clone())), body: vec![Literal::Atom(ctx)] };
        if !self.prog.rules.contains(&rule) {
            self.prog.rules.push(rule);
        }
        Ok(())
    }

    /// `!(l, …)`: a helper relation of the conjunction, over the variables it shares with the rule (an aggregate's
    /// inside it too: `shared_vars`), bound inside it by a context relation where the conjunction only tests them,
    /// negated. A variable only the conjunction has is its own (Soufflé's scoping).
    fn negated_conjunction(&mut self, lits: &[Lit], outer: &BTreeSet<String>) -> Result<Literal, Error> {
        let shared: Vec<String> = shared_vars(lits).into_iter().filter(|v| outer.contains(v)).collect();
        let bound: BTreeSet<String> = body_vars(&lits.iter().filter(|l| binds_columns(l)).cloned().collect::<Vec<_>>());
        let needed: Vec<String> = shared.iter().filter(|v| !bound.contains(*v)).cloned().collect();
        let siblings = self.siblings.clone();
        let mut bodies = self.conjunction(lits, &shared.iter().cloned().collect())?;
        self.siblings = siblings.clone();
        if !needed.is_empty() {
            let ctx = self.context(&needed)?;
            for b in &mut bodies {
                b.insert(0, Literal::Atom(ctx.clone()));
            }
        }
        self.siblings = siblings;
        self.negate(bodies, &shared)
    }

    /// A helper relation `rr_not<k>(shared)` of the bodies, negated; `rr_not<k>(0)` when nothing is shared.
    fn negate(&mut self, bodies: Vec<Vec<Literal>>, shared: &[String]) -> Result<Literal, Error> {
        let name = self.fresh("not");
        let (cols, args): (Vec<(String, String)>, Vec<Term>) = if shared.is_empty() {
            (vec![("unit".to_owned(), "int".to_owned())], vec![Term::Int(0)])
        } else {
            (shared.iter().map(|c| (c.clone(), self.type_of(c, &bodies))).collect(), shared.iter().map(|c| Term::Var(c.clone())).collect())
        };
        self.prog.decls.insert(name.clone(), cols);
        let head = Atom { rel: name.into(), args, neg: false, gated: false };
        for body in bodies {
            self.prog.rules.push(Rule { head: head.clone(), body });
        }
        Ok(Literal::Atom(Atom { neg: true, ..head }))
    }

    /// `precedes_stop(a, b, S)` as a relation `rr_prec_<S>(a, b)`: a and b are children of one parent, a first,
    /// and no child between them is in S (a kind, a relation of kinds, or a relation of nodes).
    fn precedes_stop(&mut self, set: &STerm) -> String {
        let label = match set {
            STerm::Str(k) | STerm::Var(k) => k.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>(),
            _ => "set".to_owned(),
        };
        let name = format!("{RESERVED}prec_{label}");
        if self.prog.decls.contains_key(&name) {
            return name;
        }
        let v = |s: &str| Term::Var(s.to_owned());
        let atom = |rel: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: rel.to_owned().into(), args, neg, gated: false });
        let cmp = |l: &str, r: &str| Literal::Cmp { l: v(l), op: "<", r: v(r) };
        let member = match (set, self.node_set(set)) {
            (_, Some(nodes)) => vec![atom(&nodes, vec![v("s")], false)],
            (STerm::Str(k), _) => vec![atom("kind", vec![v("s"), Term::Str(k.clone())], false)],
            (STerm::Var(kinds), _) => vec![atom("kind", vec![v("s"), v("sk")], false), atom(kinds, vec![v("sk")], false)],
            _ => vec![],
        };
        // after(a, k): a later sibling of a at index k is in S; first(a, j): the first such.
        let after = format!("{RESERVED}prec_after_{label}");
        let first = format!("{RESERVED}prec_first_{label}");
        let node_int = vec![("a".to_owned(), "node".to_owned()), ("k".to_owned(), "int".to_owned())];
        self.prog.decls.insert(after.clone(), node_int.clone());
        self.prog.decls.insert(first.clone(), node_int);
        self.prog.decls.insert(name.clone(), vec![("a".to_owned(), "node".to_owned()), ("b".to_owned(), "node".to_owned())]);
        let head = |rel: &str, args: Vec<Term>| Atom { rel: rel.to_owned().into(), args, neg: false, gated: false };
        let mut body = vec![atom("child", vec![v("p"), v("a"), v("i")], false), atom("child", vec![v("p"), v("s"), v("k")], false), cmp("i", "k")];
        body.extend(member);
        self.prog.rules.push(Rule { head: head(&after, vec![v("a"), v("k")]), body });
        self.prog.rules.push(Rule {
            head: head(&first, vec![v("a"), v("j")]),
            body: vec![
                atom(&after, vec![v("a"), Term::Wild], false),
                Literal::Agg { var: "j".to_owned(), op: AggOp::Min, over: Some("k".to_owned()), atom: head(&after, vec![v("a"), v("k")]) },
            ],
        });
        let pair = || vec![atom("child", vec![v("p"), v("a"), v("i")], false), atom("child", vec![v("p"), v("b"), v("l")], false), cmp("i", "l")];
        let mut none = pair();
        none.push(atom(&after, vec![v("a"), Term::Wild], true));
        self.prog.rules.push(Rule { head: head(&name, vec![v("a"), v("b")]), body: none });
        let mut upto = pair();
        upto.push(atom(&first, vec![v("a"), v("j")], false));
        upto.push(Literal::Cmp { l: v("l"), op: "<=", r: v("j") });
        self.prog.rules.push(Rule { head: head(&name, vec![v("a"), v("b")]), body: upto });
        name
    }

    /// A set argument that names a relation of nodes (`.decl block_node(n: node)`), not of kinds.
    fn node_set(&self, set: &STerm) -> Option<String> {
        let STerm::Var(name) = set else { return None };
        let cols = self.prog.decls.get(name)?;
        (cols.len() == 1 && cols[0].1 == "node").then(|| name.clone())
    }

    /// `tape(n, par, size, depth)`, read by the tree's ranges and climbs.
    fn need_tape(&mut self) {
        self.prog.decls.entry("tape".to_owned()).or_insert_with(|| ["n", "par", "size", "depth"].iter().map(|c| ((*c).to_owned(), "int".to_owned())).collect());
        if !self.prog.inputs.iter().any(|i| i == "tape") {
            self.prog.inputs.push("tape".to_owned());
        }
    }

    /// A climb up the tree from the nodes `seed` gives (in its variable `x`): `rr_up<k>(n, a)` for each strict
    /// ancestor a of n, up to and including the first node of `stop` where one is given. Its work is each seed's
    /// path up (to the stop), from the tape's parent column.
    fn climb(&mut self, seed: Vec<Literal>, x: &str, stop: Option<&str>) -> String {
        self.need_tape();
        let rel = self.fresh("up");
        self.prog.decls.insert(rel.clone(), vec![("n".to_owned(), "node".to_owned()), ("a".to_owned(), "node".to_owned())]);
        let v = |s: &str| Term::Var(s.to_owned());
        let atom = |r: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: r.to_owned().into(), args, neg, gated: false });
        let up = |p: &str, q: &str| atom("tape", vec![v(p), v(q), Term::Wild, Term::Wild], false);
        let real = |q: &str| Literal::Cmp { l: v(q), op: ">=", r: Term::Int(0) };
        let mut first = seed;
        first.extend([up(x, "rr_p"), real("rr_p")]);
        self.prog.rules.push(Rule { head: Atom { rel: rel.clone().into(), args: vec![v(x), v("rr_p")], neg: false, gated: false }, body: first });
        let mut next = vec![atom(&rel, vec![v("rr_n"), v("rr_p")], false)];
        if let Some(stop) = stop {
            next.push(atom(stop, vec![v("rr_p")], true));
        }
        next.extend([up("rr_p", "rr_q"), real("rr_q")]);
        self.prog.rules.push(Rule { head: Atom { rel: rel.clone().into(), args: vec![v("rr_n"), v("rr_q")], neg: false, gated: false }, body: next });
        rel
    }

    /// The nodes a climb starts from: the values the rule gives `n` (`context`), or every node.
    fn climb_seed(&mut self, n: &str) -> Result<Vec<Literal>, Error> {
        if self.bound_beside(n) {
            let siblings = self.siblings.clone();
            let ctx = self.context(&[n.to_owned()]);
            self.siblings = siblings;
            return Ok(vec![Literal::Atom(ctx?)]);
        }
        Ok(vec![Literal::Atom(Atom { rel: "node".to_owned().into(), args: vec![Term::Wild, Term::Var(n.to_owned()), Term::Wild, Term::Wild, Term::Wild], neg: false, gated: false })])
    }

    /// The axes over a set of nodes S (`core.rr`: a set may be a relation of nodes), as climbs up the tree:
    /// - `inside(n, a, S)`: a is an ancestor of n, and S(a);
    /// - `nearest(n, a, S)`: the climb from n stops at the first node of S, which is a;
    /// - `has(a, n, S)`: the climb from each node of S, n, passes a;
    /// - `within_stop(n, a, S)`: n is below a, and the climb from n meets no node of S before a.
    ///
    /// Negated, each is a helper relation (`negate`).
    fn node_axis(&mut self, rel: &str, args: &[STerm], neg: bool, outer: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        let set = self.node_set(&args[2]).expect("a node set");
        let atom = |r: &str, args: Vec<Term>| Literal::Atom(Atom { rel: r.to_owned().into(), args, neg: false, gated: false });
        let named = |me: &mut Self, t: &STerm| -> Result<String, Error> {
            match t {
                STerm::Var(v) => Ok(v.clone()),
                STerm::Wild => Ok(me.fresh("w")),
                other => Err(Error::Syntax(format!("{rel}: expected a variable, got {other:?}"))),
            }
        };
        let (first, second) = (named(self, &args[0])?, named(self, &args[1])?);
        let v = |s: &str| Term::Var(s.to_owned());
        let positive = match rel {
            "inside" | "nearest" => {
                let (n, a) = (first.clone(), second.clone());
                let seed = self.climb_seed(&n)?;
                let up = self.climb(seed, &n, (rel == "nearest").then_some(set.as_str()));
                vec![vec![atom(&up, vec![v(&n), v(&a)]), atom(&set, vec![v(&a)])]]
            }
            "has" => {
                let (a, n) = (first.clone(), second.clone());
                let up = self.climb(vec![atom(&set, vec![v(&n)])], &n, None);
                vec![vec![atom(&up, vec![v(&n), v(&a)])]]
            }
            _ => {
                let (n, a) = (first.clone(), second.clone());
                let below = self.within("within", &[STerm::Var(n.clone()), STerm::Var(a.clone())], false)?;
                let seed = self.climb_seed(&n)?;
                let up = self.climb(seed, &n, Some(&set));
                let s = self.fresh("stop");
                let hit = vec![atom(&up, vec![v(&n), v(&s)]), atom(&set, vec![v(&s)])];
                let none = self.negate(vec![hit.clone()], std::slice::from_ref(&n))?;
                let mut out = Vec::new();
                // The nearest stop is a, or a is below it: by the tree, not by comparing ids (ids follow preorder only
                // inside a granule, decision 0094 §2).
                let above = atom(crate::code_rule_dsl::facts::vtab::syntax::WITHIN, vec![v(&a), v(&s)]);
                for b in below {
                    out.push(b.iter().cloned().chain([none.clone()]).collect());
                    out.push(b.iter().cloned().chain(hit.iter().cloned()).chain([Literal::Cmp { l: v(&s), op: "=", r: v(&a) }]).collect());
                    out.push(b.into_iter().chain(hit.iter().cloned()).chain([above.clone()]).collect());
                }
                out
            }
        };
        if !neg {
            return Ok(positive);
        }
        let shared: Vec<String> = [&first, &second].into_iter().filter(|x| outer.contains(*x)).cloned().collect();
        Ok(vec![vec![self.negate(positive, &shared)?]])
    }

    /// `within(n, a)`: n is strictly below a, the engine's relation read off the tree (decision 0050): a's
    /// subtree for a bound `a`, n's ancestors for a bound `n`, made per node looked up. No range over ids and no
    /// climb through `child` (`core.rr`: tape arithmetic, never Datalog recursion). `within_stop(n, a, stop)`: also
    /// no node of `stop` strictly between them. The ancestors of n are ordered by id, so that is: n has no
    /// ancestor in `stop`, or its nearest one is a or above it. Negated (a named): n is not below a, or it is with
    /// its nearest ancestor in `stop` below a.
    fn within(&mut self, rel: &str, args: &[STerm], neg: bool) -> Result<Vec<Vec<Literal>>, Error> {
        let (n, a, stop) = match args {
            [n, a] if rel == "within" => (n, a, None),
            [n, a, stop] if rel == "within_stop" => (n, a, Some(stop)),
            _ => return not_yet(format!("{}{rel}/{}", if neg { "!" } else { "" }, args.len())),
        };
        if neg && stop.is_some() && *a == STerm::Wild {
            return not_yet(format!("`!{rel}` with its ancestor unnamed"));
        }
        let (n, a) = (self.term(n)?, self.term(a)?);
        let within = crate::code_rule_dsl::facts::vtab::syntax::WITHIN;
        self.prog.decls.insert(within.to_owned(), vec![("n".to_owned(), "node".to_owned()), ("a".to_owned(), "node".to_owned())]);
        if !self.prog.inputs.iter().any(|i| i == within) {
            self.prog.inputs.push(within.to_owned());
        }
        let below = |neg: bool| vec![Literal::Atom(Atom { rel: within.to_owned().into(), args: vec![n.clone(), a.clone()], neg, gated: false })];
        let cmp = |l: &Term, op: &'static str, r: &Term| Literal::Cmp { l: l.clone(), op, r: r.clone() };
        let Some(stop) = stop else { return Ok(vec![below(neg)]) };
        let set = self.set_name("within_stop", stop)?;
        let Term::Var(nv) = &n else { return Err(Error::Syntax("within_stop: the node is a variable".to_owned())) };
        let s = self.fresh("stop");
        let nearest = Literal::Nearest { n: nv.clone(), a: s.clone(), kindset: set.clone() };
        // The nearest stop against a is by the tree, not by comparing ids (ids need not follow preorder, decision
        // 0083 §3).
        if neg {
            // n is not below a, or its nearest stop is strictly below a.
            let between = Literal::Atom(Atom { rel: within.to_owned().into(), args: vec![Term::Var(s.clone()), a.clone()], neg: false, gated: false });
            return Ok(vec![below(true), below(false).into_iter().chain([nearest, between]).collect()]);
        }
        let none = Literal::Atom(Atom { rel: format!("{AXIS}{set}").into(), args: vec![n.clone(), Term::Wild], neg: true, gated: false });
        // The nearest stop is a, or a is below it.
        let above = Literal::Atom(Atom { rel: within.to_owned().into(), args: vec![a.clone(), Term::Var(s.clone())], neg: false, gated: false });
        Ok(vec![
            below(false).into_iter().chain([none]).collect(),
            below(false).into_iter().chain([nearest.clone(), cmp(&Term::Var(s.clone()), "=", &a)]).collect(),
            below(false).into_iter().chain([nearest, above]).collect(),
        ])
    }

    /// `match_all("re", s)`, and `match` with named groups (`core.rr`).
    ///
    /// - Over a `source` or `src_line` span (a sibling binds s so), the captures are spans: the batch runs the regex
    ///   over each such span and makes `rx_span(p, c, j, g, m)`, group g of match j of pattern p in span c, as span m
    ///   (`facts::vtab::syntax`'s `Extra`). `match` is the pattern anchored at both ends, so at most one match.
    /// - Over a symbol, `match`'s groups are `@rx_group` (`captures`); `match_all`'s rows are made natively from the
    ///   texts the rule asks about, a context relation of its other literals (`front::Native::MatchAll`). Without
    ///   groups, `match_all` is a search.
    fn text_matches(&mut self, rel: &str, args: &[STerm]) -> Result<Vec<Vec<Literal>>, Error> {
        let (re, s, positions) = match args {
            [STerm::Str(re), s] => (re, s, &[][..]),
            [STerm::Str(re), s, _, _, _] if rel == "match_all" => (re, s, &args[2..5]),
            _ => return Err(Error::Syntax(format!("{rel} takes a regex string and a text; symbol match_all also accepts ordinal, start, end"))),
        };
        let gs = groups(re);
        let all = rel == "match_all";
        // A span `match_all` (or `match` with groups) reads through a derived relation (`t(d, sp) :- source(d, sp),
        // …`, or `kind(c, "comment")`) is a node-typed column here; `span_patterns` finds which spans it holds.
        if let STerm::Var(v) = s
            && (self.span_container(v).is_some() || (all || !gs.is_empty()) && self.node_typed(v))
        {
            if !positions.is_empty() {
                return Err(Error::Syntax("match_all ordinal, start, end outputs require a symbol text".to_owned()));
            }
            let pattern = if all { re.clone() } else { format!("^(?:{re})$") };
            self.rx_span_input();
            let atom = |args: Vec<Term>| Literal::Atom(Atom { rel: "rx_span".to_owned().into(), args, neg: false, gated: false });
            let (p, c) = (Term::Str(pattern), Term::Var(v.clone()));
            if gs.is_empty() {
                return Ok(vec![vec![atom(vec![p, c, Term::Wild, Term::Str(String::new()), Term::Wild])]]);
            }
            let j = Term::Var(self.fresh("j"));
            return Ok(vec![gs.iter().map(|g| atom(vec![p.clone(), c.clone(), j.clone(), Term::Str(g.clone()), Term::Var(g.clone())])).collect()]);
        }
        if !all {
            return self.captures(args);
        }
        if gs.is_empty() && positions.is_empty() {
            return Ok(vec![vec![Literal::Cmp { l: Term::Call("rx".to_owned(), vec![self.term(s)?, Term::Str(re.clone())]), op: "=", r: Term::Int(1) }]]);
        }
        let STerm::Var(v) = s else { return not_yet(format!("match_all over a computed text, {s:?} (bind it to a variable first)")) };
        // The texts asked about: the rule's other literals, those that read no capture (they are bound by the matches).
        let saved = self.siblings.clone();
        let mut produced = gs.clone();
        for term in positions { if let STerm::Var(v) = term { produced.push(v.clone()); } }
        let reads_capture = |l: &Lit| body_vars(std::slice::from_ref(l)).iter().any(|x| produced.contains(x));
        self.siblings.retain(|l| !reads_capture(l) && !matches!(l, Lit::Atom { rel, .. } if rel == "match_all"));
        // The text a pattern scans keeps the derived atoms that say which nodes or spans it is (decision 0081).
        self.span_context = true;
        let ctx = self.context(std::slice::from_ref(v));
        self.span_context = false;
        self.siblings = saved;
        let ctx = ctx?;
        let name = self.fresh("mall");
        let mut cols: Vec<_> = std::iter::once("s".to_owned()).chain(gs.iter().map(|g| format!("g_{g}"))).map(|c| (c, "sym".to_owned())).collect();
        if !positions.is_empty() {
            cols.extend(["ordinal", "start", "end"].map(|c| (c.to_owned(), "int".to_owned())));
        }
        self.prog.decls.insert(name.clone(), cols);
        // Its one rule orders it after the context; the evaluator makes its rows.
        let x = |n: &str| Term::Var(n.to_owned());
        let mut body = vec![Literal::Atom(Atom { rel: ctx.rel.clone(), args: vec![x("s")], neg: false, gated: false })];
        body.extend(gs.iter().map(|g| Literal::Cmp { l: x(&format!("g_{g}")), op: "=", r: x("s") }));
        let mut head_args: Vec<_> = std::iter::once(x("s")).chain(gs.iter().map(|g| x(&format!("g_{g}")))).collect();
        if !positions.is_empty() {
            // The native producer replaces these seed values. They establish dependency and numeric types.
            head_args.extend([Term::Int(0), Term::Int(0), Term::Int(0)]);
        }
        self.prog.rules.push(Rule { head: Atom { rel: name.clone().into(), args: head_args, neg: false, gated: false }, body });
        self.prog.natives.insert(name.clone(), Native::MatchAll { input: ctx.rel.to_string(), pattern: re.clone(), groups: gs.clone(), positions: !positions.is_empty() });
        let args = std::iter::once(Term::Var(v.clone())).chain(gs.iter().map(|g| Term::Var(g.clone())))
            .map(Ok).chain(positions.iter().map(|p| self.term(p))).collect::<Result<Vec<_>, Error>>()?;
        Ok(vec![vec![Literal::Atom(Atom { rel: name.into(), args, neg: false, gated: false })]])
    }

    /// Every constant grammar a rule names, in an `embed` or `embed_value` head or a `parsed` or `sub_tape` atom, is
    /// one the language registry has (decision 0082): a supported dialect's label or a fragment. A name it does not
    /// have would make no guest and say nothing.
    fn grammar_names(&self) -> Result<(), Error> {
        let at = |rel: &str| match rel {
            "embed" | "embed_value" => Some(2),
            "parsed" | "sub_tape" => Some(1),
            _ => None,
        };
        for rule in &self.prog.rules {
            let atoms = std::iter::once(&rule.head).chain(rule.body.iter().filter_map(Literal::atom));
            for a in atoms {
                if let Some(i) = at(&a.rel)
                    && let Some(Term::Str(g)) = a.args.get(i)
                    && !crate::code_rule_dsl::language::is_grammar(g)
                {
                    return Err(Error::Syntax(format!(
                        "{}: no grammar {g:?}; a grammar is a language registry dialect's label or fragment (languages/*.json)",
                        rule.head.rel
                    )));
                }
            }
        }
        Ok(())
    }

    /// `Program::rx_spans`: each `rx_span(p, c, …)` a rule reads, its pattern p a constant and its span c bound in
    /// the same body by `source`, `src_line`, `kind(c, K)` (a comment, or a node of kind K) or another `rx_span`'s
    /// group (a capture; decision 0081). Read off the rules, so a printed program (`rr::print`) has them too.
    /// A derived relation's column holds the spans its rules bind it to: each rule binds it so, directly or through
    /// another such column (a fixpoint); rules that bind it differently give it each (string contents and comments).
    fn span_patterns(&mut self) -> Result<(), Error> {
        let mut columns: BTreeMap<(String, usize), BTreeSet<SpanIn>> = BTreeMap::new();
        let mut rounds = 0;
        let binder = |body: &[Literal], x: &str, columns: &BTreeMap<(String, usize), BTreeSet<SpanIn>>| -> Option<BTreeSet<SpanIn>> {
            body.iter().filter_map(Literal::atom).filter(|b| !b.neg).find_map(|b| match (b.rel.as_str(), b.args.as_slice()) {
                ("source", [_, Term::Var(y)]) if y == x => Some([SpanIn::Source].into()),
                ("src_line", [_, _, Term::Var(y)]) if y == x => Some([SpanIn::Line].into()),
                ("kind", [Term::Var(y), Term::Str(k)]) if y == x && k == "comment" => Some([SpanIn::Comment].into()),
                ("kind", [Term::Var(y), Term::Str(k)]) if y == x => Some([SpanIn::Kind(k.clone())].into()),
                ("rx_span", [Term::Str(p), _, _, Term::Str(g), Term::Var(y)]) if y == x => Some([SpanIn::Capture(p.clone(), g.clone())].into()),
                _ => b.args.iter().enumerate().find_map(|(i, t)| matches!(t, Term::Var(y) if y == x).then(|| columns.get(&(b.rel.to_string(), i)).cloned()).flatten()),
            })
        };
        // A declared relation no rule derives, and that is not read in, has no rows: its columns hold no nodes, and a
        // scan over them is empty. A library a pack's rules include (`…confusables.rr`) is loaded on its own too.
        {
            let heads: BTreeSet<&str> = self.prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
            let inputs: BTreeSet<&str> = self.prog.inputs.iter().map(String::as_str).collect();
            for (rel, cols) in &self.prog.decls {
                if !heads.contains(rel.as_str()) && !inputs.contains(rel.as_str()) && !self.prog.natives.contains_key(rel) {
                    for i in 0..cols.len() {
                        columns.insert((rel.clone(), i), BTreeSet::new());
                    }
                }
            }
        }
        // Each derived relation's rules, in order: one pass, not one over every rule per relation.
        let mut by_head: BTreeMap<&str, Vec<&Rule>> = BTreeMap::new();
        for r in &self.prog.rules {
            by_head.entry(r.head.rel.as_str()).or_default().push(r);
        }
        loop {
            let mut found = BTreeMap::new();
            for (&rel, rules) in &by_head {
                for i in 0..rules[0].head.args.len() {
                    let each: Option<Vec<BTreeSet<SpanIn>>> = rules.iter().map(|r| match r.head.args.get(i) {
                        Some(Term::Var(x)) => binder(&r.body, x, &columns),
                        _ => None,
                    }).collect();
                    if let Some(each) = each {
                        let all: BTreeSet<SpanIn> = each.into_iter().flatten().collect();
                        if columns.get(&(rel.to_owned(), i)) != Some(&all) {
                            found.insert((rel.to_owned(), i), all);
                        }
                    }
                }
            }
            rounds += 1;
            // Sets only grow, and are bounded by the program's patterns and kinds; the bound is a guard.
            if found.is_empty() || rounds > 64 {
                break;
            }
            columns.extend(found);
        }
        // Top-down: a helper's column its own rules do not bind to spans, where every rule that reads the helper binds
        // that argument so (`binder`, or through its own head's such column), holds those spans. The helper derives
        // rows for other nodes too, but none of its readers joins them, so testing only the spans its readers bind
        // gives every reader the same rows. Not for a column the run outputs, which no reader bounds.
        let mut demand: BTreeMap<(String, usize), BTreeSet<SpanIn>> = BTreeMap::new();
        let outputs: BTreeSet<&str> = self.prog.outputs.iter().map(String::as_str).collect();
        let mut rounds = 0;
        loop {
            let mut found = BTreeMap::new();
            for &rel in by_head.keys().filter(|r| !outputs.contains(*r)) {
                for i in 0..by_head[rel][0].head.args.len() {
                    if columns.contains_key(&(rel.to_owned(), i)) {
                        continue;
                    }
                    let mut all = BTreeSet::new();
                    let mut sites = 0;
                    let mut bound = true;
                    for r in &self.prog.rules {
                        for a in r.body.iter().filter_map(Literal::atom).filter(|a| a.rel == rel) {
                            sites += 1;
                            let got = match a.args.get(i) {
                                Some(Term::Var(y)) => binder(&r.body, y, &columns).or_else(|| {
                                    r.head.args.iter().position(|t| matches!(t, Term::Var(h) if h == y)).and_then(|j| demand.get(&(r.head.rel.to_string(), j)).cloned())
                                }),
                                _ => None,
                            };
                            match got {
                                Some(s) => all.extend(s),
                                None => bound = false,
                            }
                        }
                    }
                    if bound && sites > 0 && demand.get(&(rel.to_owned(), i)) != Some(&all) {
                        found.insert((rel.to_owned(), i), all);
                    }
                }
            }
            rounds += 1;
            if found.is_empty() || rounds > 64 {
                break;
            }
            demand.extend(found);
        }
        for rule in &self.prog.rules {
            for a in rule.body.iter().filter_map(Literal::atom).filter(|a| a.rel == "rx_span") {
                let (Some(Term::Str(p)), Some(Term::Var(c))) = (a.args.first(), a.args.get(1)) else {
                    return Err(Error::Syntax(format!("{}: rx_span reads a constant pattern over a span variable", rule.head.rel)));
                };
                let from_readers = || rule.head.args.iter().position(|t| matches!(t, Term::Var(h) if h == c)).and_then(|j| demand.get(&(rule.head.rel.to_string(), j)).cloned());
                let Some(within) = binder(&rule.body, c, &columns).or_else(from_readers) else {
                                        return not_yet(format!("{}: match_all over ${c}, a node or span not bound by `source`, `src_line`, `kind` or a capture", rule.head.rel));
                };
                for w in within {
                    self.prog.rx_spans.insert((w, p.clone()));
                }
            }
            // The AST patterns, read off the rules too.
            for a in rule.body.iter().filter_map(Literal::atom).filter(|a| a.rel == "ast_match") {
                if let Some(Term::Str(p)) = a.args.first() {
                    self.prog.ast_patterns.insert(p.clone());
                }
            }
        }
        Ok(())
    }

    /// A text test (`match`, `glob`, `contains`, …) on node or span `v`: a test of its text. `match` over a `source`
    /// or `src_line` span is `rx_span`'s whole match of the anchored pattern, a scan of the file's bytes, so a line
    /// past `text`'s inline bound is tested too; any other is `text(v, t)` and the test on t, so a node with no
    /// `text` row (a long one, not token-like) passes neither the test nor its negation.
    fn text_test(&mut self, rel: &str, args: &[STerm], neg: bool) -> Result<Vec<Vec<Literal>>, Error> {
        let [p, STerm::Var(v)] = args else { unreachable!("a test of a variable") };
        if test_of(rel) == Some("rx")
            && let STerm::Str(re) = p
            && self.span_container(v).is_some()
        {
            self.rx_span_input();
            let args = vec![Term::Str(format!("^(?:{re})$")), Term::Var(v.clone()), Term::Wild, Term::Str(String::new()), Term::Wild];
            return Ok(vec![vec![Literal::Atom(Atom { rel: "rx_span".to_owned().into(), args, neg, gated: false })]]);
        }
        let t = self.fresh("text");
        let text = Literal::Atom(Atom { rel: "text".to_owned().into(), args: vec![Term::Var(v.clone()), Term::Var(t.clone())], neg: false, gated: false });
        let test = self.atom(rel, &[p.clone(), STerm::Var(t)], neg)?;
        Ok(vec![vec![text, test]])
    }

    /// ``n ~ `pattern` ``: node n is a match of the AST pattern (ast-grep's syntax, in the program's language), and
    /// each metavariable `$X` binds variable X to the node it captured. The batch matches the pattern once over each
    /// document (`core_rr::CoreRows::patterns`) and makes `ast_match(p, n, g, c)`: capture g of the match at n is
    /// node c, and capture "" is n itself. A negated pattern binds nothing: no match at n. A `$$$X` (a run of
    /// nodes) and a `$_X` capture nothing.
    fn pattern(&mut self, subject: &STerm, pattern: &str, neg: bool) -> Result<Vec<Vec<Literal>>, Error> {
        let STerm::Var(n) = subject else {
            return Err(Error::Syntax(format!("`{pattern}`: a pattern's subject is a variable")));
        };
        if !self.prog.inputs.iter().any(|i| i == "ast_match") {
            self.prog.inputs.push("ast_match".to_owned());
            let cols = [("p", "sym"), ("n", "node"), ("g", "sym"), ("c", "node")];
            self.prog.decls.insert("ast_match".to_owned(), cols.iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
        }
        let atom = |g: &str, c: Term, neg: bool| {
            Literal::Atom(Atom { rel: "ast_match".to_owned().into(), args: vec![Term::Str(pattern.to_owned()), Term::Var(n.clone()), Term::Str(g.to_owned()), c], neg, gated: false })
        };
        if neg {
            return Ok(vec![vec![atom("", Term::Wild, true)]]);
        }
        let mut out = vec![atom("", Term::Wild, false)];
        out.extend(super::single_metavars(pattern).into_iter().map(|x| atom(&x, Term::Var(x.clone()), false)));
        Ok(vec![out])
    }

    /// `rx_span` as an input of the program (`facts::vtab::syntax`'s `Extra` makes it).
    fn rx_span_input(&mut self) {
        if !self.prog.inputs.iter().any(|i| i == "rx_span") {
            self.prog.inputs.push("rx_span".to_owned());
            let cols = [("p", "sym"), ("c", "node"), ("j", "int"), ("g", "sym"), ("m", "node")];
            self.prog.decls.insert("rx_span".to_owned(), cols.iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
        }
    }

    /// Whether a sibling binds `v` to a `source` or `src_line` span or a comment, the spans `rx_span` runs over.
    fn span_container(&self, v: &str) -> Option<SpanIn> {
        self.siblings.iter().find_map(|l| match l {
            Lit::Atom { rel, args, neg: false } => match (rel.as_str(), args.as_slice()) {
                ("source", [_, STerm::Var(x)]) if x == v => Some(SpanIn::Source),
                ("src_line", [_, _, STerm::Var(x)]) if x == v => Some(SpanIn::Line),
                ("kind", [STerm::Var(x), STerm::Str(k)]) if x == v && k == "comment" => Some(SpanIn::Comment),
                _ => None,
            },
            _ => None,
        })
    }

    /// Whether a sibling atom reads `v` at a column declared a node (a node or span), or `v` is a named group of a
    /// sibling `match_all` (or `match`) over a node or span: `rx_span`'s capture `m` is a span. Without the second, a
    /// test of the capture in the same body (`match_all("(?<w>…)", t), match("…", w)`, RUF001's words) was a test of
    /// the span's id as a text, and never held.
    fn node_typed(&self, v: &str) -> bool {
        self.node_typed_at(v, 0)
    }

    fn node_typed_at(&self, v: &str, depth: usize) -> bool {
        self.siblings.iter().any(|l| match l {
            Lit::Atom { rel, args, neg: false } if depth < 4 && matches!(rel.as_str(), "match_all" | "match") => match args.as_slice() {
                [STerm::Str(re), STerm::Var(s), ..] if s != v && groups(re).iter().any(|g| g == v) => {
                    self.span_container(s).is_some() || self.node_typed_at(s, depth + 1)
                }
                _ => false,
            },
            Lit::Atom { rel, args, neg: false } => self.prog.decls.get(rel).is_some_and(|cols| {
                args.iter().zip(cols).any(|(a, (_, t))| matches!(a, STerm::Var(x) if x == v) && t == "node")
            }),
            _ => false,
        })
    }

    /// `match("(?<x>…)", s)`: s matches, and each named group binds the variable of its name to its text:
    /// `x = @rx_group(s, "^(?:…)$", "x")`, which has no value when s does not match (`strings::group`).
    fn captures(&mut self, args: &[STerm]) -> Result<Vec<Vec<Literal>>, Error> {
        let [STerm::Str(re), s] = args else { unreachable!("match with a pattern") };
        let whole = Term::Str(format!("^(?:{re})$"));
        let s = self.term(s)?;
        // The test as well as the groups: a group the head alone reads is a column, which SQL does not filter on, so
        // a text that does not match would make a row with no value (`pct_rest`'s `%%` step).
        let mut out = vec![Literal::Cmp { l: Term::Call("rx".to_owned(), vec![s.clone(), whole.clone()]), op: "=", r: Term::Int(1) }];
        for g in groups(re) {
            out.push(Literal::Cmp { l: Term::Var(g.clone()), op: "=", r: Term::Call("rx_group".to_owned(), vec![s.clone(), whole.clone(), Term::Str(g)]) });
        }
        Ok(vec![out])
    }

    /// `(glob("a", s) ; glob("b", s))` as one test of the set, and `(s = "a" ; s = "b")` as one membership. `neg`: the
    /// disjunction is negated.
    fn test_set(&mut self, alts: &[Vec<Lit>], neg: bool) -> Result<Option<Vec<Literal>>, Error> {
        if alts.len() < 2 || alts.iter().any(|a| a.len() != 1) {
            return Ok(None);
        }
        let one = |a: &Vec<Lit>| a[0].clone();
        let tests: Option<Vec<(&'static str, STerm, String)>> = alts
            .iter()
            .map(|a| match one(a) {
                Lit::Atom { rel, args, neg: false } => match (test_of(&rel), args.as_slice()) {
                    (Some(op), [STerm::Str(p), s]) if op != "rx" => Some((op, s.clone(), p.clone())),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        if let Some(tests) = tests
            && tests.iter().all(|(op, s, _)| *op == tests[0].0 && *s == tests[0].1)
        {
            // On a node or span, the tests are of its text, as one test's are (`text_test`).
            let mut out = Vec::new();
            let s = match &tests[0].1 {
                STerm::Var(v) if self.span_container(v).is_some() || self.node_typed(v) => {
                    let t = self.fresh("text");
                    out.push(Literal::Atom(Atom { rel: "text".to_owned().into(), args: vec![Term::Var(v.clone()), Term::Var(t.clone())], neg: false, gated: false }));
                    Term::Var(t)
                }
                s => self.term(s)?,
            };
            let set = Term::Call(SET.to_owned(), tests.iter().map(|(_, _, p)| Term::Str(p.clone())).collect());
            out.push(Literal::Cmp { l: Term::Call(tests[0].0.to_owned(), vec![s, set]), op: "=", r: Term::Int(1) });
            return Ok(Some(out));
        }
        let members: Option<Vec<(String, String)>> = alts
            .iter()
            .map(|a| match one(a) {
                Lit::Cmp { l: STerm::Var(x), op: "=", r: STerm::Str(c) } | Lit::Cmp { l: STerm::Str(c), op: "=", r: STerm::Var(x) } => Some((x, c)),
                _ => None,
            })
            .collect();
        // Only a test when something else binds x: `kind(c, k), (x = "a" ; x = "b")` gives x each value. Negated it
        // binds nothing, so it is a test (`key = cat(q, s), !(key = "a" ; key = "b")`).
        if let Some(members) = members
            && members.iter().all(|(x, _)| *x == members[0].0)
            && (neg || self.bound_beside(&members[0].0))
        {
            let set = Term::Call(SET.to_owned(), members.iter().map(|(_, c)| Term::Str(c.clone())).collect());
            return Ok(Some(vec![Literal::Cmp { l: Term::Call("equals".to_owned(), vec![Term::Var(members[0].0.clone()), set]), op: "=", r: Term::Int(1) }]));
        }
        Ok(None)
    }

    /// A set argument's relation: a relation's name (of kinds, `kind_sets` tells, or of nodes, `node_set`), or one
    /// kind as its own set.
    fn set_name(&mut self, rel: &str, arg: &STerm) -> Result<String, Error> {
        match arg {
            STerm::Var(name) => {
                self.sets.insert(name.clone());
                Ok(name.clone())
            }
            STerm::Str(kind) => {
                let name = kind_set_name(kind);
                self.prog.kindsets.insert(name.clone(), vec![kind.clone()]);
                Ok(name)
            }
            _ => Err(Error::Syntax(format!("{rel}: the set is a relation of kinds or one kind"))),
        }
    }

    fn atom(&mut self, rel: &str, args: &[STerm], neg: bool) -> Result<Literal, Error> {
        let var = |t: &STerm| match t {
            STerm::Var(v) => Ok(v.clone()),
            _ => Err(Error::Syntax(format!("{rel}: expected a variable, got {t:?}"))),
        };
        if let Some(op) = test_of(rel) {
            let [p, s] = args else { return Err(Error::Syntax(format!("{rel} takes a pattern and a string"))) };
            // Negated, a match binds nothing: its named groups are plain groups, `!match(re, s)` as written.
            if op == "rx" && let STerm::Str(re) = p && !groups(re).is_empty() && neg {
                return self.atom(rel, &[STerm::Str(ungrouped(re)), s.clone()], neg);
            }
            if op == "rx" && let STerm::Str(re) = p && !groups(re).is_empty() {
                return not_yet(format!("match with named groups, {re:?}, here (`Lowerer::captures` lowers it in a body)"));
            }
            // The scalar fallback treats an uncompiled matcher as false. A literal in
            // a policy must therefore be validated before it reaches that fallback.
            if op == "rx" && let STerm::Str(re) = p {
                crate::code_rule_dsl::logic::strings::Matcher::new("rx", &[re.clone()])
                    .map_err(|e| Error::Syntax(format!("match regex: {e}")))?;
            }
            let (s, mut p) = (self.term(s)?, self.term(p)?);
            // `match` is whole-string, as in Soufflé; the engine's `rx` searches. A pattern the rule reads (an
            // option's) is anchored as it runs.
            if op == "rx" {
                p = match p {
                    Term::Str(re) => Term::Str(format!("^(?:{re})$")),
                    other => anchored(other),
                };
            }
            return Ok(Literal::Cmp { l: Term::Call(op.to_owned(), vec![s, p]), op: "=", r: Term::Int(if neg { 0 } else { 1 }) });
        }
        match (rel, args) {
            // `nearest(n, _, set)`: n has an ancestor in the set, which is `inside(n, _, set)`; and negated.
            ("nearest", [n, STerm::Wild, set]) => {
                let set = self.set_name(rel, set)?;
                return Ok(Literal::Atom(Atom { rel: format!("{AXIS}{set}").into(), args: vec![self.term(n)?, Term::Wild], neg, gated: false }));
            }
            ("nearest", [n, a, set]) if !neg => {
                let kindset = self.set_name(rel, set)?;
                return Ok(Literal::Nearest { n: var(n)?, a: var(a)?, kindset });
            }
            // `!nearest(n, s, set)` with s named: not that nearest, through a helper relation over (n, s).
            ("nearest", [n, a, set]) => {
                let kindset = self.set_name(rel, set)?;
                let (n, a) = (var(n)?, var(a)?);
                let shared = [n.clone(), a.clone()];
                return self.negate(vec![vec![Literal::Nearest { n, a, kindset }]], &shared);
            }
            // `inside(n, a, set)`: a is an ancestor of n in the set; `has(a, n, set)`: n is a descendant of a in the
            // set. Both are `(n, a)` rows the transducer makes.
            ("inside" | "has", [x, y, set]) => {
                let set = self.set_name(rel, set)?;
                let (x, y) = (self.term(x)?, self.term(y)?);
                let (axis, args) = if rel == "inside" { (AXIS, vec![x, y]) } else { (DESCENDANTS, vec![y, x]) };
                return Ok(Literal::Atom(Atom { rel: format!("{axis}{set}").into(), args, neg, gated: false }));
            }
            ("ast", _) if !neg => return self.ast(args),
            ("precedes_stop", [a, b, set]) => {
                let rel = self.precedes_stop(set);
                return Ok(Literal::Atom(Atom { rel: rel.into(), args: vec![self.term(a)?, self.term(b)?], neg, gated: false }));
            }
            ("dfs_first", [r, hit, first, edge, target]) | ("dfs_first_memo", [r, hit, first, edge, target, _]) => {
                let name = |t: &STerm| match t {
                    STerm::Var(v) => Ok(v.clone()),
                    _ => Err(Error::Syntax(format!("{rel}: the edge, target and roots are relations, by name"))),
                };
                let (edge, target) = (name(edge)?, name(target)?);
                let roots = if rel == "dfs_first_memo" { Some(name(&args[5])?) } else { None };
                let rel = self.dfs_first(&edge, &target, roots.as_deref());
                return Ok(Literal::Atom(Atom { rel: rel.into(), args: vec![self.term(r)?, self.term(hit)?, self.term(first)?], neg, gated: false }));
            }
            ("python_candidates", [call,report,reference,receiver,input,targets,context,member]) => {
                let name=|t:&STerm|match t{STerm::Var(v)=>Ok(v.clone()),_=>Err(Error::Syntax("Python candidate inputs are relation names".into()))};
                let literal=|t:&STerm|match t{STerm::Str(v)=>Ok(v.clone()),_=>Err(Error::Syntax("Python candidate context and member must be string literals".into()))};
                let (input,targets,context,member)=(name(input)?,name(targets)?,literal(context)?,literal(member)?);
                if !crate::code_rule_dsl::facts::producers::python::families::CONTEXTS.contains(&context.as_str()) {return Err(Error::Syntax(format!("unknown Python execution context {context}")));}
                for name in [&input,&targets] {
                    if !self.prog.decls.get(name).is_some_and(|cols|cols.len()==1) {return Err(Error::Syntax(format!("python_candidates: {name} must be a declared unary relation")));}
                }
                let native=Native::PythonCandidates{input:input.clone(),targets:targets.clone(),context,member};
                let rel=if let Some((name,_))=self.prog.natives.iter().find(|(_,n)|**n==native) {name.clone()}else {
                    let mut serial=self.prog.natives.len();
                    let rel=loop {let name=format!("rr_python_candidates_{serial}");if !self.prog.decls.contains_key(&name){break name;}serial+=1;};
                    let v=|s:&str|Term::Var(s.to_owned());
                    let a=|r:&str,args:Vec<Term>|Literal::Atom(Atom{rel:r.into(),args,neg:false,gated:false});
                    // The targets are read as `python_qualified`'s are, so every pass that follows rule bodies
                    // (shared library parts, strata, demand) makes them before the candidates.
                    let mut body=vec![a(&input,vec![v("d")]),a(&targets,vec![v("q")])];
                    // Locality proof only: the native contract emits nodes from these documents.
                    for n in ["c","f","r","o"] {body.push(a("node",vec![v("d"),v(n),Term::Wild,Term::Wild,Term::Wild]));}
                    self.prog.decls.insert(rel.clone(),["c","f","r","o"].into_iter().map(|n|(n.into(),"node".into())).collect());
                    self.prog.rules.push(Rule{head:Atom{rel:rel.clone().into(),args:["c","f","r","o"].into_iter().map(v).collect(),neg:false,gated:false},body});
                    self.prog.natives.insert(rel.clone(),native);rel
                };
                return Ok(Literal::Atom(Atom{rel: rel.into(),args:[call,report,reference,receiver].into_iter().map(|a|self.term(a)).collect::<Result<Vec<_>,_>>()?,neg,gated:false}));
            }
            ("python_qualified" | "python_origins" | "python_assigned" | "python_arguments", _) => {
                let kind=rel.strip_prefix("python_").unwrap();
                let width=if kind=="arguments" {5}else{2};
                let expected=width+1+usize::from(kind=="qualified");
                if args.len()!=expected {return Err(Error::Syntax(format!("{rel}: expected {expected} arguments")));}
                let name=|t:&STerm|match t{STerm::Var(v)=>Ok(v.clone()),_=>Err(Error::Syntax("Python producer inputs are relation names".into()))};
                let input=name(&args[width])?;
                let targets=if kind=="qualified" {Some(name(&args[width+1])?)}else{None};
                let producer=self.python_producer(kind,&input,targets.as_deref());
                return Ok(Literal::Atom(Atom{rel:producer.into(),args:args[..width].iter().map(|a|self.term(a)).collect::<Result<Vec<_>,_>>()?,neg,gated:false}));
            }
            ("components", [d, c, k, edge, own]) => {
                let name = |t: &STerm| match t {
                    STerm::Var(v) => Ok(v.clone()),
                    _ => Err(Error::Syntax("components: the edge and own relations are named".to_owned())),
                };
                let rel = self.components(&name(edge)?, &name(own)?);
                return Ok(Literal::Atom(Atom { rel: rel.into(), args: vec![self.term(d)?, self.term(c)?, self.term(k)?], neg, gated: false }));
            }
            _ => {}
        }
        if let Some((_, _)) = RELATION_ARG.iter().find(|(r, _)| *r == rel) {
            return not_yet(format!("the built-in {rel}"));
        }
        let args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
        Ok(Literal::Atom(Atom { rel: rel.to_owned().into(), args, neg, gated: false }))
    }

    fn python_producer(&mut self, kind:&str, input:&str, targets:Option<&str>)->String {
        let native = Native::Python {kind:kind.into(),input:input.into(),targets:targets.map(str::to_owned)};
        let base = format!("rr_python_{kind}_{input}_{}",targets.unwrap_or(""));
        let mut rel = base.clone();
        let mut collision = 0;
        while let Some(existing) = self.prog.natives.get(&rel) {
            if existing == &native { return rel; }
            collision += 1;
            rel = format!("{base}__{collision}");
        }
        let vars=if kind=="arguments" {vec![("n","node"),("i","int"),("keyword","sym"),("spread","sym"),("value","node")]}else if kind=="qualified"{vec![("n","node"),("q","sym")]}else{vec![("n","node"),("value","node")]};
        let v=|s:&str|Term::Var(s.to_owned());
        let atom=|r:&str,args:Vec<Term>|Literal::Atom(Atom{rel:r.into(),args,neg:false,gated:false});
        let mut body=vec![atom(input,vec![v("n")]),atom("node",vec![v("d"),v("n"),Term::Wild,Term::Wild,Term::Wild])];
        if let Some(t)=targets {body.push(atom(t,vec![v("q")]));}
        else {
            body.push(atom("node",vec![v("d"),v("value"),Term::Wild,Term::Wild,Term::Wild]));
            if kind=="arguments" {
                for (name,value) in [("i",Term::Int(0)),("keyword",Term::Str(String::new())),("spread",Term::Str(String::new()))] {
                    body.push(Literal::Cmp{l:v(name),op:"=",r:value});
                }
            }
        }
        self.prog.decls.insert(rel.clone(),vars.iter().map(|(n,t)|(n.to_string(),t.to_string())).collect());
        self.prog.rules.push(Rule{head:Atom{rel:rel.clone().into(),args:vars.iter().map(|(n,_)|v(n)).collect(),neg:false,gated:false},body});
        self.prog.natives.insert(rel.clone(),native);
        rel
    }

    /// `dfs_first(_, _, _, edge, target)`'s relation `rr_dfs_<edge>_<target>` (`_<roots>` for `dfs_first_memo`):
    /// `(r, hit, first)`, made natively (`front::Native`). Its one rule reads its inputs, so it is evaluated after
    /// them; the evaluator makes its rows (`logic::native::dfs_first`), not the rule.
    fn dfs_first(&mut self, edge: &str, target: &str, roots: Option<&str>) -> String {
        let rel = match roots {
            Some(roots) => format!("rr_dfsm_{edge}_{target}_{roots}"),
            None => format!("rr_dfs_{edge}_{target}"),
        };
        if self.prog.natives.contains_key(&rel) {
            return rel;
        }
        let v = |n: &str| Term::Var(n.to_owned());
        let atom = |rel: &str, args: Vec<Term>| Literal::Atom(Atom { rel: rel.to_owned().into(), args, neg: false, gated: false });
        let mut body = vec![atom(edge, vec![v("r"), v("hit"), v("first")]), atom(target, vec![v("hit")])];
        if let Some(roots) = roots {
            body.push(atom(roots, vec![v("r"), Term::Wild]));
        }
        self.prog.rules.push(Rule { head: Atom { rel: rel.clone().into(), args: vec![v("r"), v("hit"), v("first")], neg: false, gated: false }, body });
        // The edge relation's column types (a node, a number, a symbol); nodes where it is declared later.
        let ty = |c: usize, or: &str| self.prog.decls.get(edge).and_then(|cols| cols.get(c)).map_or_else(|| or.to_owned(), |(_, t)| t.clone());
        let cols = vec![("r".to_owned(), ty(0, "node")), ("hit".to_owned(), ty(1, "node")), ("first".to_owned(), ty(2, "int"))];
        self.prog.decls.insert(rel.clone(), cols);
        self.prog.natives.insert(rel.clone(), Native::DfsFirst { edge: edge.to_owned(), target: target.to_owned(), roots: roots.map(str::to_owned) });
        rel
    }

    /// `components(_, _, _, edge, own)`'s relation `rr_components_<edge>_<own>`: `(d, c, key)`, made natively
    /// (`front::Native::Components`). Its one rule reads both inputs and joins two documents, so it is evaluated
    /// after them and in the cross-file part; the evaluator makes its rows, not the rule.
    fn components(&mut self, edge: &str, own: &str) -> String {
        let rel = format!("rr_components_{edge}_{own}");
        if self.prog.natives.contains_key(&rel) {
            return rel;
        }
        let v = |n: &str| Term::Var(n.to_owned());
        let atom = |rel: &str, args: Vec<Term>| Literal::Atom(Atom { rel: rel.to_owned().into(), args, neg: false, gated: false });
        let body = vec![atom(own, vec![v("d"), v("k")]), atom(edge, vec![v("d"), v("e")]), atom(own, vec![v("e"), Term::Wild])];
        self.prog.rules.push(Rule { head: Atom { rel: rel.clone().into(), args: vec![v("d"), v("e"), v("k")], neg: false, gated: false }, body });
        let cols = vec![("d".to_owned(), "int".to_owned()), ("c".to_owned(), "int".to_owned()), ("k".to_owned(), "sym".to_owned())];
        self.prog.decls.insert(rel.clone(), cols);
        self.prog.natives.insert(rel.clone(), Native::Components { edge: edge.to_owned(), own: own.to_owned() });
        rel
    }

    /// `ast(d, n, { … }, NAME: v, …)`, or `ast(n, { … }, …)` with its document left unnamed.
    fn ast(&mut self, args: &[STerm]) -> Result<Literal, Error> {
        let positional: Vec<&STerm> = args.iter().filter(|a| !matches!(a, STerm::Named(..))).collect();
        let (document, node, matcher) = match positional.as_slice() {
            [STerm::Var(d), STerm::Var(n), STerm::Json(m)] => (d.clone(), n.clone(), m.clone()),
            [STerm::Var(n), STerm::Json(m)] => (self.fresh("d"), n.clone(), m.clone()),
            _ => return Err(Error::Syntax("ast takes (document, node, { … }) or (node, { … })".to_owned())),
        };
        let captures = args
            .iter()
            .filter_map(|a| match a {
                STerm::Named(name, v) => Some(match v.as_ref() {
                    STerm::Var(v) => Ok((name.clone(), v.clone())),
                    other => Err(Error::Syntax(format!("ast capture {name}: expected a variable, got {other:?}"))),
                }),
                _ => None,
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Literal::Ast { document, node, matcher, captures })
    }

    fn comparison(&mut self, l: &STerm, op: &'static str, r: &STerm, outer: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        // A float compared with a whole number: the number as a float (`to_float(t) > 86400`).
        let as_float = |t: &STerm| match t {
            STerm::Num(n) => STerm::Float(format!("{n}.0")),
            t => t.clone(),
        };
        let (l, r) = if is_float(l) { (l.clone(), as_float(r)) } else if is_float(r) { (as_float(l), r.clone()) } else { (l.clone(), r.clone()) };
        let (l, r) = (&l, &r);
        match (l, r) {
            (STerm::Var(v), STerm::Agg { op: agg, over, by, sep, body }) | (STerm::Agg { op: agg, over, by, sep, body }, STerm::Var(v)) if op == "=" => {
                Ok(vec![self.aggregate(v, agg, over.as_deref(), by, sep, body, outer)?])
            }
            (STerm::Agg { op: agg, over, by, sep, body }, other) | (other, STerm::Agg { op: agg, over, by, sep, body }) => {
                let v = self.fresh("agg");
                let mut out = self.aggregate(&v, agg, over.as_deref(), by, sep, body, outer)?;
                let other = self.term(other)?;
                // `$v = 0` with `$v` unbound would assign it (`front::cmp_binds`), and the planner may place it
                // before the aggregate: an equality is written `0 = $v`, a test that waits for the aggregate.
                let (l, r) = if matches!(l, STerm::Agg { .. }) && op != "=" { (Term::Var(v), other) } else { (other, Term::Var(v)) };
                out.push(Literal::Cmp { l, op, r });
                Ok(vec![out])
            }
            _ => Ok(vec![vec![Literal::Cmp { l: self.term(l)?, op, r: self.term(r)? }]]),
        }
    }

    /// A relation of `vars`' values in the rule: the literals beside the one being lowered (`siblings`; their
    /// atoms and comparisons), projected on `vars`. It binds, inside a helper rule, variables its body only tests.
    fn context(&mut self, vars: &[String]) -> Result<Atom, Error> {
        let siblings = std::mem::take(&mut self.siblings);
        let context = |aggregates: bool| -> Vec<Lit> {
            siblings
                .iter()
                .filter(|l| match l {
                    Lit::Atom { neg, .. } => !neg,
                    Lit::Cmp { l, r, .. } => aggregates || !matches!(l, STerm::Agg { .. }) && !matches!(r, STerm::Agg { .. }),
                    _ => false,
                })
                .cloned()
                .collect()
        };
        // Inputs alone when they bind the variables: a context of derived relations ties the helper to them, and
        // to their recursion (`type_app(t, h, i, a) :- ann_input(t), …, field(m, "subscript", a), i = count : {
        // field(m, "subscript", a0), a0 < a }` with ann_input recursive through type_app would aggregate in its
        // own component). The context may hold more rows than the rule; the rule joins it with its own literals.
        let inputs: BTreeSet<String> = self.prog.inputs.iter().cloned().collect();
        let base: Vec<Lit> = context(false).into_iter().filter(|l| !matches!(l, Lit::Atom { rel, .. } if !inputs.contains(rel))).collect();
        // Of the inputs, only those that reach the variables asked about through shared variables: an input that
        // meets them nowhere (`text(nm, x)` beside `text(a, t)` asked for `t`) filters nothing the rule does not,
        // and joined in the context it is a cross product (every text times every text).
        let base = if vars.is_empty() {
            base
        } else {
            let mut reach: BTreeSet<String> = vars.iter().cloned().collect();
            let mut kept = vec![false; base.len()];
            let mut grew = true;
            while grew {
                grew = false;
                for (i, l) in base.iter().enumerate() {
                    let lv = body_vars(std::slice::from_ref(l));
                    if !kept[i] && !lv.is_disjoint(&reach) {
                        kept[i] = true;
                        reach.extend(lv);
                        grew = true;
                    }
                }
            }
            base.into_iter().zip(kept).filter_map(|(l, k)| k.then_some(l)).collect()
        };
        let (base, base_bound) = Self::bound_context(base);
        // Aggregates stay out unless a variable needs one: `m = cat(…, body, …)` over `body = join … : { … }`.
        // So do atoms of the clause's own heads, where the rest binds the variables: through one, the aggregate
        // would sit in the head's recursion (`la_cb(n, e) :- …, la_cb(s, v), count : { la_vis(n, x, s2, …) } = 0`).
        // The context then holds more rows than the rule, which joins it with its own literals.
        let heads = self.heads.clone();
        let own = |l: &Lit| matches!(l, Lit::Atom { rel, .. } if heads.contains(rel));
        let (first, bound) = Self::bound_context(context(false).into_iter().filter(|l| !own(l)).collect());
        let (first, bound) = if vars.iter().all(|v| bound.contains(v)) { (first, bound) } else { Self::bound_context(context(false)) };
        // A context whose literals leave the variables asked about in separate groups is their cross product (every
        // function times every name, where the derived atom that joins them is left out): the next choice is taken.
        let joined = |lits: &[Lit]| {
            let Some(v0) = vars.first() else { return true };
            let mut reach: BTreeSet<String> = BTreeSet::from([v0.clone()]);
            let mut grew = true;
            while grew {
                grew = false;
                for l in lits.iter().filter(|l| matches!(l, Lit::Atom { neg: false, .. })) {
                    let lv = body_vars(std::slice::from_ref(l));
                    if !lv.is_disjoint(&reach) && !lv.is_subset(&reach) {
                        reach.extend(lv);
                        grew = true;
                    }
                }
            }
            vars.iter().all(|v| reach.contains(v))
        };
        let context = if !self.span_context && vars.iter().all(|v| base_bound.contains(v)) && joined(&base) {
            base
        } else if vars.iter().all(|v| bound.contains(v)) {
            first
        } else {
            Self::bound_context(context(true)).0
        };
        // Only the literals tied to `vars` through shared variables: one tied to none tests that some row exists
        // anywhere (`ann_root(_)` once `(s = a ; within(s, a))` is left out), which makes every row of the context
        // depend on every document. Leaving it out widens the context, which the rule joins with its own literals.
        let context = Self::covering(Self::connected(Self::placed_texts(Self::connected(context, vars), vars), vars), vars);
        let ctx = self.fresh("ctx");
        let alts = self.conjunction(&context, &vars.iter().cloned().collect())?;
        let types = vars.iter().map(|c| (c.clone(), self.type_of(c, &alts))).collect();
        self.prog.decls.insert(ctx.clone(), types);
        let head = Atom { rel: ctx.into(), args: vars.iter().map(|c| Term::Var(c.clone())).collect(), neg: false, gated: false };
        for b in alts {
            self.prog.rules.push(Rule { head: head.clone(), body: b });
        }
        Ok(head)
    }

    /// `context` without a `text(v, t)` whose node `v` nothing else in it binds: a text is read of a bound node
    /// (`t = @rr_node_text(v)` past the inline bound, decision 0109), and such a `v` is bound by the rule, not the
    /// context (`perf101_rebinding(n, NAME) :- demand(n, NAME), within_stop(c6, n, …), field(c6, "name", c7),
    /// text(c7, t), text(NAME, t)`, a context of `c6`). Left out, it widens the context, which the rule joins with
    /// its own literals.
    fn placed_texts(context: Vec<Lit>, vars: &[String]) -> Vec<Lit> {
        let text_of = |l: &Lit| match l {
            Lit::Atom { rel, neg: false, args, .. } if rel == "text" => match args.first() {
                Some(STerm::Var(v)) => Some(v.clone()),
                _ => None,
            },
            _ => None,
        };
        let binders: Vec<Lit> = context
            .iter()
            .filter(|l| matches!(l, Lit::Atom { rel, neg: false, .. } if rel != "text" && test_of(rel).is_none()))
            .cloned()
            .collect();
        let bound = body_vars(&binders);
        let kept = context.into_iter().filter(|l| text_of(l).is_none_or(|v| bound.contains(&v) || vars.contains(&v))).collect();
        // A comparison that read a node only the text bound goes with it (`@doc(FN) = @doc(BOUND)`).
        Self::bound_context(kept).0
    }

    /// The literals of `context` reachable from `vars` through shared variables, in their order.
    fn connected(context: Vec<Lit>, vars: &[String]) -> Vec<Lit> {
        let mut reach: BTreeSet<String> = vars.iter().cloned().collect();
        let of: Vec<BTreeSet<String>> = context.iter().map(|l| body_vars(std::slice::from_ref(l))).collect();
        let mut keep = vec![false; context.len()];
        loop {
            let mut grew = false;
            for (k, vs) in of.iter().enumerate() {
                if !keep[k] && !vs.is_disjoint(&reach) {
                    keep[k] = true;
                    reach.extend(vs.iter().cloned());
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        context.into_iter().zip(keep).filter(|(_, k)| *k).map(|(l, _)| l).collect()
    }

    /// Of a context's groups of literals (joined through variables other than `vars`), those that bind a variable of
    /// `vars` no group before them does. Another group only tests that rows exist, and joined in it multiplies the
    /// context's rows by its own: `within(c1, n), within(c2, n)` asked for `n` is every pair of n's descendants (1.3 G
    /// rows on 20 files). The context may hold more rows than the rule, which joins it with its own literals.
    fn covering(context: Vec<Lit>, vars: &[String]) -> Vec<Lit> {
        let asked: BTreeSet<String> = vars.iter().cloned().collect();
        let of: Vec<BTreeSet<String>> = context.iter().map(|l| body_vars(std::slice::from_ref(l))).collect();
        // Each literal's group: literals sharing a variable not asked about are one group.
        let mut group: Vec<usize> = (0..context.len()).collect();
        fn root(g: &mut [usize], mut i: usize) -> usize {
            while g[i] != i {
                g[i] = g[g[i]];
                i = g[i];
            }
            i
        }
        for i in 0..context.len() {
            for j in 0..i {
                if of[i].iter().any(|v| !asked.contains(v) && of[j].contains(v)) {
                    let (a, b) = (root(&mut group, i), root(&mut group, j));
                    group[a] = b;
                }
            }
        }
        let binds = |l: &Lit| matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none() && rel != "earlier");
        let mut covered: BTreeSet<String> = BTreeSet::new();
        let mut kept: BTreeSet<usize> = BTreeSet::new();
        for i in 0..context.len() {
            let g = root(&mut group, i);
            if kept.contains(&g) {
                continue;
            }
            let bound: BTreeSet<String> = (0..context.len())
                .filter(|&k| root(&mut group, k) == g && binds(&context[k]))
                .flat_map(|k| of[k].iter().filter(|v| asked.contains(*v)).cloned().collect::<Vec<_>>())
                .collect();
            if !bound.is_subset(&covered) {
                covered.extend(bound);
                kept.insert(g);
            }
        }
        // A variable no group binds is left to the context as it was.
        if !asked.is_subset(&covered) {
            return context;
        }
        let groups: Vec<usize> = (0..context.len()).map(|k| root(&mut group, k)).collect();
        context.into_iter().zip(groups).filter(|(_, g)| kept.contains(g)).map(|(l, _)| l).collect()
    }

    /// A context's atoms and the comparisons it can evaluate, in order, and the variables they bind.
    fn bound_context(context: Vec<Lit>) -> (Vec<Lit>, BTreeSet<String>) {
        // A comparison stays only if the context binds what it reads (`k > 1` needs the count that binds k, which
        // is left out): the context may hold more rows than the rule, and the rule joins it with its own literals.
        let binds = |l: &Lit| matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none() && rel != "earlier");
        let mut bound = body_vars(&context.iter().filter(|l| binds(l)).cloned().collect::<Vec<_>>());
        let (atoms, mut cmps): (Vec<Lit>, Vec<Lit>) = context.into_iter().partition(|l| matches!(l, Lit::Atom { .. }));
        let mut context = atoms;
        loop {
            let ready = cmps.iter().position(|c| {
                let Lit::Cmp { l, op, r } = c else { return false };
                let reads = |t: &STerm| body_vars(&[Lit::Cmp { l: t.clone(), op: "=", r: STerm::Wild }]);
                match (l, r) {
                    (STerm::Var(x), t) | (t, STerm::Var(x)) if *op == "=" && !bound.contains(x) => reads(t).is_subset(&bound),
                    _ => reads(l).union(&reads(r)).all(|v| bound.contains(v)),
                }
            });
            let Some(k) = ready else { break };
            let c = cmps.remove(k);
            bound.extend(body_vars(std::slice::from_ref(&c)));
            context.push(c);
        }
        (context, bound)
    }

    /// Whether a positive atom beside the literal being lowered (not a text test or `earlier`) binds `v`. `within`
    /// and `within_stop` bind both ends: they are the engine's relation (decision 0050).
    fn bound_beside(&self, v: &str) -> bool {
        let atoms: Vec<Lit> = self
            .siblings
            .iter()
            .filter(|l| matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none() && rel != "earlier"))
            .cloned()
            .collect();
        body_vars(&atoms).contains(v)
    }

    #[allow(clippy::too_many_arguments)]
    fn aggregate(&mut self, var: &str, agg: &str, over: Option<&STerm>, by: &[STerm], sep: &Option<String>, body: &[Lit], outer: &BTreeSet<String>) -> Result<Vec<Literal>, Error> {
        // A value or key that is a term (`sum strlen(x) - strlen(a)`) is a variable the body binds.
        let mut body = body.to_vec();
        let mut variable = |t: &STerm, me: &mut Self| match t {
            STerm::Var(v) => v.clone(),
            t => {
                let v = me.fresh("val");
                body.push(Lit::Cmp { l: STerm::Var(v.clone()), op: "=", r: t.clone() });
                v
            }
        };
        let over = over.map(|t| variable(t, self));
        let by: Vec<String> = by.iter().map(|t| variable(t, self)).collect();
        let body = body.as_slice();
        let op = match agg {
            "count" => AggOp::Count,
            "min" => AggOp::Min,
            "max" => AggOp::Max,
            "sum" => AggOp::Sum,
            "product" => AggOp::Product,
            // With no `sep`, the texts are joined as they are.
            "join" => AggOp::Join { by: by.clone(), sep: sep.clone().unwrap_or_default() },
            // Soufflé's mean is a decimal number, and the engine has none.
            "mean" => return not_yet("the aggregate mean (a decimal number)"),
            other => return not_yet(format!("the aggregate {other}")),
        };
        if (!by.is_empty() || sep.is_some()) && agg != "join" {
            return Err(Error::Syntax(format!("{agg} takes no `by` or `sep`")));
        }
        if op != AggOp::Count && over.is_none() {
            return Err(Error::Syntax(format!("{agg} needs the variable it aggregates")));
        }
        // The body's own variables the aggregate reads: its value, and a join's keys.
        let own = |v: &String| over.as_ref() == Some(v) || by.contains(v);
        let ours = |v: &String| !outer.contains(v);
        // One plain atom: the engine's aggregate reads it. The built-ins lowering writes out (axes, core.rr's tree
        // arithmetic, …) are no relations of the program, so a body of one of them goes through the helper.
        if let [Lit::Atom { rel, args, neg: false }] = body
            && test_of(rel).is_none()
            && !matches!(rel.as_str(), "nearest" | "inside" | "has" | "split_dot" | "ast" | "within" | "within_stop" | "precedes_stop" | "match_all" | "match" | "unset")
            && !self.rewritten(rel)
        {
            let Literal::Atom(mut atom) = self.atom(rel, args, false)? else { unreachable!("a plain atom") };
            let at = |v: &String| atom.args.iter().position(|t| matches!(t, Term::Var(w) if w == v));
            // A variable of the body's own the atom repeats is an equality the engine's aggregate would not test, and
            // a key or value the atom lacks is the helper's error below.
            let repeated = atom.vars().iter().any(|v| ours(v) && atom.args.iter().filter(|t| matches!(t, Term::Var(w) if w == v)).count() > 1);
            let placed = over.iter().chain(&by).all(|v| at(v).is_some());
            if !repeated && placed {
                // A variable only the body uses is counted over, not grouped by: a wildcard to the engine.
                for t in &mut atom.args {
                    if matches!(t, Term::Var(v) if ours(v) && !own(v)) {
                        *t = Term::Wild;
                    }
                }
                return Ok(vec![Literal::Agg { var: var.to_owned(), op, over, atom }]);
            }
        }
        // Otherwise a helper relation of the body. `count` and `sum` range over the body's rows, a multiset of the
        // aggregated value, so they keep every variable; `min`, `max` and `join` (of distinct rows) keep those they
        // read (the aggregated one and a join's keys) and those the rule shares.
        let mut inner = body_vars(body);
        // A nested aggregate's variables that the rule shares are the helper's too: in `count : { ident(x), count : {
        // ref(n, f, x) } = 0 }`, the inner count is per (f, x), so the helper is a relation of f and x, and f is bound
        // inside it by the context. Without f, the inner count read it free, and the outer could not group by it.
        let mut nested = Vec::new();
        body.iter().for_each(|l| l.vars(&mut nested));
        inner.extend(nested.into_iter().filter(|v| outer.contains(v)));
        // `match_all`'s groups are variables of the body too, named in its regex: each match is a row to count.
        for l in body {
            if let Lit::Atom { rel, args, neg: false } = l
                && rel == "match_all"
                && let Some(STerm::Str(re)) = args.first()
            {
                inner.extend(groups(re));
            }
        }
        let cols: Vec<String> = inner.iter().filter(|v| matches!(op, AggOp::Count | AggOp::Sum | AggOp::Product) || own(v) || outer.contains(*v)).cloned().collect();
        if let Some(v) = over.iter().chain(&by).find(|v| !cols.contains(v)) {
            return Err(Error::Syntax(format!("{agg}: {v} is not bound in its body")));
        }
        let name = self.fresh("agg");
        let mut bodies = self.conjunction(body, outer)?;
        // A shared variable the body only tests (`count : { slash_at(ps, j), i < j }`) is bound inside the helper by
        // a context relation: the rule's other literals, projected on those variables.
        let bound: BTreeSet<String> = body_vars(
            // `match_all` over a text the rule binds reads it, as a test does: its spans or texts come from the context.
            &body.iter().filter(|l| binds_columns(l) && !matches!(l, Lit::Atom { rel, .. } if matches!(rel.as_str(), "match_all" | "match"))).cloned().collect::<Vec<_>>(),
        );
        let needed: Vec<String> = inner.iter().filter(|v| outer.contains(*v) && !bound.contains(*v)).cloned().collect();
        if !needed.is_empty() {
            // The context holds every variable the body shares with the rule, those the body binds too: projected on
            // the tested ones alone (`ctx(j)`), it crossed every row of the body (`child(al, a, k)`, every argument of
            // every call) with every j of the rule, 20.6M rows for 4,133 counts on 50 files.
            let shared: Vec<String> = inner.iter().filter(|v| outer.contains(*v)).cloned().collect();
            let span_subject = body.iter().any(|l| matches!(l, Lit::Atom { rel, args, .. } if matches!(rel.as_str(), "match_all" | "match") && matches!(args.get(1), Some(STerm::Var(v)) if needed.contains(v))));
            self.span_context = span_subject;
            let ctx = self.context(&shared);
            self.span_context = false;
            let ctx = ctx?;
            for b in &mut bodies {
                b.insert(0, Literal::Atom(ctx.clone()));
            }
        }
        let types = cols.iter().map(|c| (c.clone(), self.type_of(c, &bodies))).collect();
        self.prog.decls.insert(name.clone(), types);
        let args: Vec<Term> = cols.iter().map(|c| Term::Var(c.clone())).collect();
        for b in bodies {
            // The empty value inside the body is the constant, as in a rule's own body.
            let (args, b) = unset(args.clone(), b)?;
            self.prog.rules.push(Rule { head: Atom { rel: name.clone().into(), args, neg: false, gated: false }, body: b });
        }
        // The helper keeps every column (`count` counts distinct rows), but only the rule's variables group it.
        let args = cols.iter().map(|c| if outer.contains(c) || own(c) { Term::Var(c.clone()) } else { Term::Wild }).collect();
        Ok(vec![Literal::Agg { var: var.to_owned(), op, over, atom: Atom { rel: name.into(), args, neg: false, gated: false } }])
    }

    /// A helper column's type: the column it fills in the body's atoms, else the type of the aggregate or the
    /// computation that assigns it (`k = 0 - i` is a number, which a `join` orders by value).
    fn type_of(&self, v: &str, bodies: &[Vec<Literal>]) -> String {
        for literal in bodies.iter().flatten() {
            match literal {
                Literal::Atom(a) if !a.neg => {
                    for (i, t) in a.args.iter().enumerate() {
                        if *t == Term::Var(v.to_owned()) {
                            if crate::code_rule_dsl::logic::front::is_axis(&a.rel) {
                                return "node".to_owned();
                            }
                            if let Some(cols) = self.prog.decls.get(a.rel.as_str())
                                && let Some((_, ty)) = cols.get(i)
                            {
                                return ty.clone();
                            }
                        }
                    }
                }
                Literal::Nearest { n, a, .. } if n == v || a == v => return "node".to_owned(),
                // A computed value: a number from arithmetic and the functions of numbers, else a symbol.
                Literal::Cmp { l: Term::Var(x), op: "=", r: r @ (Term::Int(_) | Term::Call(..)) } if x == v => {
                    let number = match r {
                        Term::Int(_) => true,
                        Term::Call(f, _) => {
                            matches!(f.as_str(), "add" | "sub" | "mul" | "div" | "mod" | "max" | "min" | "strlen" | "to_number" | "to_float" | "count_matches")
                                || f.starts_with("float_") && f != "float_str"
                        }
                        _ => false,
                    };
                    return if number { "int" } else { "sym" }.to_owned();
                }
                Literal::Cmp { l: Term::Var(x), op: "=", r: Term::Str(_) } if x == v => return "sym".to_owned(),
                Literal::Agg { var, op, over, atom } if var == v => {
                    return match op {
                        AggOp::Count | AggOp::Sum | AggOp::Product => "int".to_owned(),
                        AggOp::Join { .. } => "sym".to_owned(),
                        AggOp::Min | AggOp::Max => {
                            let col = atom.args.iter().position(|t| over.as_ref().is_some_and(|o| *t == Term::Var(o.clone())));
                            col.and_then(|c| self.prog.decls.get(atom.rel.as_str())?.get(c).map(|(_, ty)| ty.clone())).unwrap_or_else(|| "int".to_owned())
                        }
                    };
                }
                Literal::Ast { document, .. } if document == v => return "int".to_owned(),
                Literal::Ast { node, captures, .. } if node == v || captures.iter().any(|(_, c)| c == v) => return "node".to_owned(),
                _ => {}
            }
        }
        "sym".to_owned()
    }

    fn term(&mut self, t: &STerm) -> Result<Term, Error> {
        Ok(match t {
            STerm::Var(v) if v == "top" && !self.top_is_var => Term::Top,
            STerm::Var(v) => Term::Var(v.clone()),
            STerm::Wild => Term::Wild,
            STerm::Num(n) => Term::Int(*n),
            STerm::Str(s) => Term::Str(s.clone()),
            STerm::Call { name, functor: true, args } => {
                let args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
                Term::Call(if name == "dot" { "cat".to_owned() } else { name.clone() }, args)
            }
            STerm::Call { name, functor: false, args } => {
                // Soufflé's functions: `cat` joins strings (the engine's `cat` joins with a dot, so it is `concat`).
                // `to_string` of a float writes the float (`float_str`); `max` and `min` of two are SQL's.
                let f = match (name.as_str(), args.len()) {
                    ("cat", n) if n >= 2 => "concat",
                    ("to_string", 1) if is_float(&args[0]) => "float_str",
                    ("strlen" | "to_string" | "to_number" | "to_float", 1) => name.as_str(),
                    ("substr", 3) => "substr",
                    ("max" | "min", 2) => name.as_str(),
                    _ => return not_yet(format!("the built-in function {name}/{}", args.len())),
                };
                let args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
                // `cat(a, b, c)` is `cat(cat(a, b), c)`.
                if f == "concat" {
                    let mut args = args.into_iter();
                    let first = args.next().expect("two or more");
                    return Ok(args.fold(first, |acc, a| Term::Call("concat".to_owned(), vec![acc, a])));
                }
                Term::Call(f.to_owned(), args)
            }
            STerm::Bin { op, l, r } => {
                // Float arithmetic is on floats' keys (`strings::functions::float_key`), by functions of them.
                let float = is_float(l) || is_float(r);
                let f = match *op {
                    "+" if float => "float_add",
                    "-" if float => "float_sub",
                    "*" if float => "float_mul",
                    "/" if float => "float_div",
                    "+" => "add",
                    "-" => "sub",
                    "*" => "mul",
                    "/" => "div",
                    "%" if !float => "mod",
                    other => return not_yet(format!("arithmetic ({other})")),
                };
                Term::Call(f.to_owned(), vec![self.term(l)?, self.term(r)?])
            }
            STerm::Agg { op, .. } => return not_yet(format!("the aggregate {op} as a term")),
            STerm::Tick(t) => return not_yet(format!("the template `{t}`")),
            // A decimal number is its float's key, which orders as the floats do.
            STerm::Float(f) => Term::Int(float_key(f.parse::<f64>().map_err(|_| Error::Syntax(format!("bad decimal number {f}")))?)),
            STerm::Json(_) | STerm::Named(..) => return Err(Error::Syntax("a JSON term or an `ast` capture outside `ast(…)`".to_owned())),
        })
    }

    /// Kind sets: the relations of facts given to `nearest`, `inside` and `has`. One read nowhere else is only a set.
    /// A set of kinds that rules derive is no kind set: its axes are rules (`derived_axes`). (A set of nodes is read
    /// as climbs where it is given, `node_axis`.)
    fn kind_sets(&mut self) -> Result<(), Error> {
        let mut derived = BTreeSet::new();
        for set in std::mem::take(&mut self.sets) {
            if self.prog.kindsets.contains_key(&set) {
                continue;
            }
            let kinds = match self.members(&set)? {
                Members::Kinds(kinds) => kinds,
                Members::Derived => {
                    derived.insert(set);
                    continue;
                }
            };
            self.prog.kindsets.insert(set.clone(), kinds);
            // A set no rule reads is no relation of the program; one that rules read (`lfscope(k) :- fscope(k)`)
            // stays one as well.
            let read = self.prog.rules.iter().filter(|r| r.head.rel != set).any(|r| r.body.iter().filter_map(Literal::atom).any(|a| a.rel == set));
            if !read {
                self.prog.facts.remove(&set);
                self.prog.decls.remove(&set);
                self.prog.rules.retain(|r| r.head.rel != set);
            }
        }
        self.derived_axes(&derived);
        Ok(())
    }

    /// What a set holds: its kinds when they are known now (a kind set), else kinds its rules (or an input) give.
    fn members(&self, set: &str) -> Result<Members, Error> {
        if let Some(kinds) = self.kinds_of(set, &mut BTreeSet::new()) {
            return Ok(Members::Kinds(kinds));
        }
        if matches!(self.prog.decls.get(set).map(Vec::as_slice), Some([(_, t)]) if t == "sym") {
            return Ok(Members::Derived);
        }
        Err(Error::Syntax(format!("the kind set {set} is not a relation of kinds (one column of facts, or rules `{set}(k) :- s(k)` over such sets)")))
    }

    /// The axes over sets of kinds rules derive, which the transducer cannot read from a node's kind: rules over
    /// `child`, in which each node's membership (`rr_in_<s>(x) :- node(_, x, k, _, _), s(k)`) is read once. A helper
    /// reads its set negated, so the set is complete before it (a lower stratum; a set that depends on its own axis
    /// is refused by the strata).
    /// - `rr_near_<s>(n, a)`, a is n's nearest strict ancestor in s (`nearest`, and `inside(n, _, s)`): down from each
    ///   member to the next, a row per node below one;
    /// - `rr_anc_<s>(n, a)`, every strict ancestor a of n in s (`inside`): the chain of nearest ones;
    /// - `rr_desc_<s>(n, a)`, n in s and a any strict ancestor of it (`has`): up from each member;
    /// - `rr_above_<s>(a)`, a has a member strictly below it (`has(a, _, s)`).
    fn derived_axes(&mut self, derived: &BTreeSet<String>) {
        let mut need = BTreeSet::new();
        for rule in &mut self.prog.rules {
            for l in &mut rule.body {
                let (set, helper, args, neg) = match l {
                    Literal::Nearest { n, a, kindset } if derived.contains(kindset) => (kindset.clone(), "near", vec![Term::Var(n.clone()), Term::Var(a.clone())], false),
                    Literal::Atom(x) => match axis(&x.rel) {
                        Some((set, has)) if derived.contains(set) => {
                            let (helper, args) = match (has, x.args.as_slice()) {
                                (false, [_, Term::Wild]) => ("near", x.args.clone()),
                                (false, _) => ("anc", x.args.clone()),
                                (true, [Term::Wild, a]) => ("above", vec![a.clone()]),
                                (true, _) => ("desc", x.args.clone()),
                            };
                            (set.to_owned(), helper, args, x.neg)
                        }
                        _ => continue,
                    },
                    _ => continue,
                };
                *l = Literal::Atom(Atom { rel: format!("rr_{helper}_{set}").into(), args, neg, gated: false });
                if helper == "anc" {
                    need.insert((set.clone(), "near"));
                }
                need.insert((set, helper));
            }
        }
        if need.is_empty() {
            return;
        }
        for rel in ["child", "node"] {
            self.base(rel);
        }
        let v = |s: &str| Term::Var(s.to_owned());
        let atom = |rel: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: rel.to_owned().into(), args, neg, gated: false });
        let child = |p: &str, c: &str| atom("child", vec![v(p), v(c), Term::Wild], false);
        for (set, helper) in need {
            let member = format!("rr_in_{set}");
            if !self.prog.decls.contains_key(&member) {
                self.prog.decls.insert(member.clone(), vec![("x".to_owned(), "node".to_owned())]);
                let body = vec![atom("node", vec![Term::Wild, v("x"), v("k"), Term::Wild, Term::Wild], false), atom(&set, vec![v("k")], false)];
                self.prog.rules.push(Rule { head: Atom { rel: member.clone().into(), args: vec![v("x")], neg: false, gated: false }, body });
            }
            let is = |x: &str, neg: bool| atom(&member, vec![v(x)], neg);
            let rel = format!("rr_{helper}_{set}");
            let me = |args: &[&str]| atom(&rel, args.iter().map(|a| v(a)).collect(), false);
            let (cols, bodies): (&[&str], Vec<Vec<Literal>>) = match helper {
                "near" => (&["n", "a"], vec![vec![child("a", "n"), is("a", false)], vec![child("p", "n"), is("p", true), me(&["p", "a"])]]),
                "anc" => (&["n", "a"], vec![vec![atom(&format!("rr_near_{set}"), vec![v("n"), v("a")], false)], vec![atom(&format!("rr_near_{set}"), vec![v("n"), v("b")], false), me(&["b", "a"])]]),
                "desc" => (&["n", "a"], vec![vec![is("n", false), child("a", "n")], vec![me(&["n", "p"]), child("a", "p")]]),
                _ => (&["a"], vec![vec![is("n", false), child("a", "n")], vec![me(&["p"]), child("a", "p")]]),
            };
            self.prog.decls.insert(rel.clone(), cols.iter().map(|c| ((*c).to_owned(), "node".to_owned())).collect());
            let head = Atom { rel: rel.clone().into(), args: cols.iter().map(|c| v(c)).collect(), neg: false, gated: false };
            for body in bodies {
                self.prog.rules.push(Rule { head: head.clone(), body });
            }
        }
    }

    /// A base relation lowering reads (`child`, `node`), declared, and an input, when the program has not.
    fn base(&mut self, rel: &str) {
        let cols: &[(&str, &str)] = match rel {
            "child" => &[("p", "node"), ("c", "node"), ("i", "int")],
            "node" => &[("d", "int"), ("n", "node"), ("kind", "sym"), ("start", "int"), ("end", "int")],
            other => unreachable!("no base relation {other}"),
        };
        self.prog.decls.entry(rel.to_owned()).or_insert_with(|| cols.iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
        if !self.prog.inputs.iter().any(|i| i == rel) && !self.prog.rules.iter().any(|r| r.head.rel == rel) {
            self.prog.inputs.push(rel.to_owned());
        }
    }

    /// `split_dot(q, m, x)`: q is m.x split at its last dot, so x has no dot: `q = @cat(m, x), !contains(".", x)`
    /// (a bound q splits at every dot, `@cat` run backwards, which takes variables: a constant is bound to one
    /// first). Negated, every column named: q is not m.x, or x has a dot.
    fn split_dot(&mut self, args: &[STerm], neg: bool) -> Result<Vec<Vec<Literal>>, Error> {
        if args.len() != 3 {
            return not_yet(format!("split_dot/{}", args.len()));
        }
        if neg && args.contains(&STerm::Wild) {
            return not_yet("`!split_dot` with a column unnamed");
        }
        let (mut out, mut terms) = (Vec::new(), Vec::new());
        for a in args {
            let t = self.term(a)?;
            terms.push(match t {
                Term::Var(_) => t,
                _ if neg => t,
                Term::Wild => Term::Var(self.fresh("part")),
                constant => {
                    let v = Term::Var(self.fresh("part"));
                    out.push(Literal::Cmp { l: v.clone(), op: "=", r: constant });
                    v
                }
            });
        }
        let [q, m, x] = <[Term; 3]>::try_from(terms).expect("three columns");
        let dotted = |b: i64| Literal::Cmp { l: Term::Call("contains".to_owned(), vec![x.clone(), Term::Str(".".to_owned())]), op: "=", r: Term::Int(b) };
        let cat = Term::Call("cat".to_owned(), vec![m, x.clone()]);
        if neg {
            return Ok(vec![vec![Literal::Cmp { l: q, op: "!=", r: cat }], vec![dotted(1)]]);
        }
        out.extend([Literal::Cmp { l: q, op: "=", r: cat }, dotted(0)]);
        Ok(vec![out])
    }

    /// A rule that reads no relation (an aggregate's helper of comparisons only, `{ e = "a", q = @json_str(e) }`)
    /// holds once, as in Soufflé: it reads `rr_unit(0)`, one row, since the planner starts every body from an atom.
    /// A body with an aggregate (`ok(v) :- m = join …, v = @f(m).`) needs none: the planner runs a body of filters and
    /// aggregates over the relational unit, and the SQL executor maintains it as a scalar rule over its aggregates'
    /// inputs (a unit atom would hide them from its retained support).
    fn unit(&mut self) {
        const UNIT: &str = "rr_unit";
        let mut used = false;
        for rule in &mut self.prog.rules {
            // A matcher atom (`ast(…)`) or a walk reads the tree itself; an aggregate reads its atom.
            let reads = |l: &Literal| matches!(l, Literal::Atom(a) if !a.neg) || matches!(l, Literal::Ast { .. } | Literal::Nearest { .. } | Literal::Agg { .. });
            if !rule.body.iter().any(reads) {
                rule.body.insert(0, Literal::Atom(Atom { rel: UNIT.to_owned().into(), args: vec![Term::Int(0)], neg: false, gated: false }));
                used = true;
            }
        }
        if used {
            self.prog.decls.insert(UNIT.to_owned(), vec![("x".to_owned(), "int".to_owned())]);
            self.prog.facts.insert(UNIT.to_owned(), vec![vec![Value::Int(0)]]);
        }
    }

    /// The kinds of a unary relation of kinds: its facts, and the kinds of `s` for each rule `set(k) :- s(k)`.
    fn kinds_of(&self, set: &str, seen: &mut BTreeSet<String>) -> Option<Vec<String>> {
        if !seen.insert(set.to_owned()) || self.prog.inputs.iter().any(|i| i == set) || self.prog.decls.get(set).is_some_and(|c| c.len() != 1) {
            return None;
        }
        let mut kinds: Vec<String> = Vec::new();
        for row in self.prog.facts.get(set).into_iter().flatten() {
            let [Value::Str(k)] = row.as_slice() else { return None };
            kinds.push(k.clone());
        }
        for r in self.prog.rules.iter().filter(|r| r.head.rel == set) {
            let ([Term::Var(k)], [Literal::Atom(a)]) = (r.head.args.as_slice(), r.body.as_slice()) else { return None };
            if a.neg || !matches!(a.args.as_slice(), [Term::Var(v)] if v == k) {
                return None;
            }
            kinds.extend(self.kinds_of(&a.rel, seen)?);
        }
        // In the order written, each once.
        let mut once = BTreeSet::new();
        kinds.retain(|k| once.insert(k.clone()));
        Some(kinds)
    }
}

/// Whether a term is a float: a decimal number, `to_float(…)`, or arithmetic on one. (A variable's type is not
/// followed: `0.0 - f` is a float, `f - g` is not.)
fn is_float(t: &STerm) -> bool {
    match t {
        STerm::Float(_) => true,
        STerm::Call { name, functor: false, args } => name == "to_float" || (matches!(name.as_str(), "max" | "min") && args.iter().any(is_float)),
        STerm::Bin { l, r, .. } => is_float(l) || is_float(r),
        _ => false,
    }
}

/// A float as an integer key that orders as the floats do (decision 0008: the engine's values are integers).
pub(super) fn float_key(f: f64) -> i64 {
    crate::code_rule_dsl::logic::strings::functions::float_key(f)
}

/// The empty value `unset(x)` binds (`core.rr`): the id -1, which no node has, so no tree relation holds of it.
/// Each `unset(x)` is taken out and `x` replaced by it in the rule; `!unset(x)` is `x != -1`. Its text is "":
/// `text(x, t)` of it is `t = ""` (O77). (A `@place` with no valid line is `unset` too: `PLACE`.)
pub(super) const UNSET: i64 = -1;

fn unset(mut head: Vec<Term>, body: Vec<Literal>) -> Result<(Vec<Term>, Vec<Literal>), Error> {
    let mut vars = BTreeSet::new();
    let mut rest = Vec::new();
    for l in body {
        match l {
            Literal::Atom(a) if a.rel == "unset" && !a.neg => match a.args.as_slice() {
                [Term::Var(v)] => {
                    vars.insert(v.clone());
                }
                [Term::Wild] => {}
                _ => return not_yet("`unset` of a value"),
            },
            Literal::Atom(a) if a.rel == "unset" => match a.args.as_slice() {
                [x @ Term::Var(_)] => rest.push(Literal::Cmp { l: x.clone(), op: "!=", r: Term::Int(UNSET) }),
                _ => return not_yet("`!unset` of a value"),
            },
            l => rest.push(l),
        }
    }
    if vars.is_empty() {
        return Ok((head, rest));
    }
    fn put(t: &mut Term, vars: &BTreeSet<String>) {
        match t {
            Term::Var(v) if vars.contains(v) => *t = Term::Int(UNSET),
            Term::Call(_, args) => args.iter_mut().for_each(|a| put(a, vars)),
            _ => {}
        }
    }
    head.iter_mut().for_each(|t| put(t, &vars));
    for l in &mut rest {
        match l {
            Literal::Atom(a) => a.args.iter_mut().for_each(|t| put(t, &vars)),
            Literal::Cmp { l, r, .. } => {
                put(l, &vars);
                put(r, &vars);
            }
            // An aggregate's body (its helper atom) reads the unset variable from outside: it reads the empty value
            // there too. Its result and `over` are its own, never unset.
            Literal::Agg { var, over, atom, .. } if !vars.contains(var) && over.as_ref().is_none_or(|o| !vars.contains(o)) => {
                atom.args.iter_mut().for_each(|t| put(t, &vars));
            }
            other if !other.vars().is_disjoint(&vars) => return not_yet("`unset` in an aggregate, `nearest` or `ast`"),
            _ => {}
        }
    }
    // The empty value's text is "".
    let rest = rest
        .into_iter()
        .filter_map(|l| match l {
            Literal::Atom(a) if a.rel == "text" && a.args.first() == Some(&Term::Int(UNSET)) => {
                let t = a.args.get(1).cloned().unwrap_or(Term::Wild);
                match (t, a.neg) {
                    (Term::Wild, false) => None,
                    (Term::Wild, true) => Some(Literal::Cmp { l: Term::Int(1), op: "=", r: Term::Int(0) }),
                    (t, neg) => Some(Literal::Cmp { l: t, op: if neg { "!=" } else { "=" }, r: Term::Str(String::new()) }),
                }
            }
            l => Some(l),
        })
        .collect();
    Ok((head, rest))
}

/// `re` with each named group made non-capturing (`(?P<x>…)`, `(?<x>…)` → `(?:…)`): what a test that binds nothing
/// reads. Escapes and character classes are passed over.
fn ungrouped(re: &str) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(re.len());
    let (mut i, b) = (0, re.as_bytes());
    let mut class = false;
    while i < b.len() {
        let c = b[i];
        if c == b'\\' && i + 1 < b.len() {
            out.extend_from_slice(&b[i..i + 2]);
            i += 2;
            continue;
        }
        if class {
            class = c != b']';
        } else if c == b'[' {
            class = true;
        } else if c == b'(' {
            let open = if re[i..].starts_with("(?P<") { 4 } else if re[i..].starts_with("(?<") && !re[i..].starts_with("(?<=") && !re[i..].starts_with("(?<!") { 3 } else { 0 };
            if open > 0 && let Some(end) = re[i + open..].find('>') {
                out.extend_from_slice(b"(?:");
                i += open + end + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8(out).expect("only ASCII bytes are replaced")
}

/// A regex's named groups, in order (`(?<x>…)` and `(?P<x>…)`).
fn groups(re: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = re;
    while let Some(i) = rest.find("(?<").map(|i| (i, 3)).into_iter().chain(rest.find("(?P<").map(|i| (i, 4))).min() {
        let after = &rest[i.0 + i.1..];
        let Some(end) = after.find('>') else { break };
        let name = &after[..end];
        if !name.is_empty() && !name.starts_with(['=', '!']) && !out.iter().any(|g| g == name) {
            out.push(name.to_owned());
        }
        rest = &after[end..];
    }
    out
}

/// The kind set of one kind, `"call"`: `rr_kind_call`.
fn kind_set_name(kind: &str) -> String {
    format!("rr_kind_{}", kind.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>())
}

/// The variables of literals, the relations passed to built-ins (`nearest(n, a, scope)`'s `scope`) left out.
/// The relations of decision 0055's guests: a rule's request for a guest cannot wait on them.
/// The relations a guest or value read asks through (`demand`).
const DEMANDS: [&str; 3] = ["rr_embed_req", "rr_parse_req", "rr_splice_req"];

/// The placeholder for a demand's location (`value_demand`).
const LOCATED: &str = "rr_located";

const GUEST_RELATIONS: [&str; 10] = ["parsed", "sub_tape", "guest", "doc_root", "doc_role", "tape_of", "host_node", "parse_of", "unparse_of", "splice_of"];

/// The tape operations (decision 0058 §4): function, arity, the relation it reads, and that relation's columns.
const TAPE_OPS: [(&str, usize, &str, &[(&str, &str)]); 3] = [
    ("parse", 2, "parse_of", &[("x", "sym"), ("g", "sym"), ("t", "sym")]),
    ("unparse", 1, "unparse_of", &[("t", "sym"), ("x", "sym")]),
    ("splice", 3, "splice_of", &[("t", "sym"), ("n", "node"), ("t2", "sym"), ("t3", "sym")]),
];

/// The relations of the file system a rule may read (t21 E1), each with the request its reads make: a directory's
/// entries, `dir_entry(dir, name, is_dir)`, a file's lines, `file_line(path, i, text)` (from 1), and a file's content
/// digest, `file_digest(path, c)` (a cache key's, decision 0103: a library's tape is kept by it).
pub(in crate::code_rule_dsl) const WORLD: [(&str, &str); 3] = [("dir_entry", "rr_dir_req"), ("file_line", "rr_file_req"), ("file_digest", "rr_digest_req")];

fn world_demand(rel: &str) -> Option<&'static str> {
    WORLD.iter().find(|(r, _)| *r == rel).map(|(_, d)| *d)
}

fn body_vars(lits: &[Lit]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for l in lits {
        match l {
            Lit::Atom { rel, args, .. } => {
                // `match` and `match_all` bind their named groups, which no argument names.
                if matches!(rel.as_str(), "match" | "match_all")
                    && let Some(STerm::Str(re)) = args.first()
                {
                    out.extend(groups(re));
                }
                let skip: &[usize] = RELATION_ARG.iter().find(|(r, _)| *r == rel).map_or(&[], |(_, k)| k);
                for (k, a) in args.iter().enumerate() {
                    if !(skip.contains(&k) && matches!(a, STerm::Var(_))) {
                        let mut vs = Vec::new();
                        a.vars(&mut vs);
                        out.extend(vs);
                    }
                }
            }
            Lit::Group { alts, .. } => alts.iter().for_each(|a| out.extend(body_vars(a))),
            other => {
                let mut vs = Vec::new();
                other.scope_vars(&mut vs);
                out.extend(vs);
            }
        }
    }
    out
}

/// `body_vars`, with the variables of aggregates inside comparisons too: what a group shares with its rule when an
/// aggregate in it is correlated on the rule's variables (`count : { b(x, _) } > 2` shares x).
fn shared_vars(lits: &[Lit]) -> BTreeSet<String> {
    let mut out = body_vars(lits);
    for l in lits {
        match l {
            Lit::Cmp { l, r, .. } => {
                let mut vs = Vec::new();
                l.vars(&mut vs);
                r.vars(&mut vs);
                out.extend(vs);
            }
            Lit::Group { alts, .. } => alts.iter().for_each(|a| out.extend(shared_vars(a))),
            _ => {}
        }
    }
    out
}

/// Whether a literal binds every variable of its columns: a positive atom of a relation, not a text test nor a
/// built-in the lowering writes out as tests or computations of variables bound elsewhere (`within`, `earlier`,
/// `split_dot`, `nearest`, `precedes_stop`, …).
fn binds_columns(l: &Lit) -> bool {
    matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none()
        && !matches!(rel.as_str(), "within" | "within_stop" | "earlier" | "split_dot" | "nearest" | "precedes_stop" | "dfs_first" | "dfs_first_memo" | "unset"))
}

/// Adjacent negations of one test of one string, `!glob("a", s), !glob("b", s)`, as one negated test of the set.
pub(super) fn merge_negated_tests(body: Vec<Literal>) -> Vec<Literal> {
    let mut out: Vec<Literal> = Vec::new();
    for literal in body {
        let negated_test = |l: &Literal| match l {
            Literal::Cmp { l: Term::Call(op, args), op: "=", r: Term::Int(0) } if args.len() == 2 && crate::code_rule_dsl::logic::strings::is_test(op) && op != "rx" && op != "irx" => Some((op.clone(), args[0].clone(), args[1].clone())),
            _ => None,
        };
        if let (Some((op, s, p)), Some(last)) = (negated_test(&literal), out.last_mut())
            && let Some((last_op, last_s, last_p)) = negated_test(last)
            && op == last_op
            && s == last_s
            && matches!(p, Term::Str(_))
        {
            let mut patterns = match last_p {
                Term::Call(f, items) if f == SET => items,
                Term::Str(_) => vec![last_p],
                _ => {
                    out.push(literal);
                    continue;
                }
            };
            patterns.push(p);
            *last = Literal::Cmp { l: Term::Call(op, vec![s, Term::Call(SET.to_owned(), patterns)]), op: "=", r: Term::Int(0) };
            continue;
        }
        out.push(literal);
    }
    out
}

/// `@place(path, line)` (`core.rr`) outside a `.violation`'s head: a place with no node, as a `loc` value:
/// `-2 - (path << 32 | line)`, `path` the path's symbol (its atom) and `0 <= line < 2^32`, so no node id
/// (`d << 32 | index`, never negative), no span and not `unset` (-1). `@place_path(x)` and `@place_line(x)` take it
/// apart; `plan`'s SQL computes all three, and the native set engine leaves rules that call them to SQL. A place with
/// no valid line is `unset`, which has no finding.
pub(super) const PLACE: &str = "place";
pub(super) const PLACE_PATH: &str = "place_path";
pub(super) const PLACE_LINE: &str = "place_line";

fn has_place(t: &Term) -> bool {
    match t {
        Term::Call(f, args) => f == PLACE || args.iter().any(has_place),
        _ => false,
    }
}

fn literal_terms(l: &Literal) -> Vec<&Term> {
    match l {
        Literal::Atom(a) | Literal::Agg { atom: a, .. } => a.args.iter().collect(),
        Literal::Cmp { l, r, .. } => vec![l, r],
        _ => Vec::new(),
    }
}
