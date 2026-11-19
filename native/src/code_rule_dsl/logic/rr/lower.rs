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
//!   text layer's relations (`preparation::text::TextRelations`); `+ - * / %` and Soufflé's `cat` of any arity (`concat`
//!   folded from the left), `strlen`, `to_string`, `substr`, `to_number`, `to_float`, `max/2` and `min/2` are
//!   functors. `to_number` reads as Soufflé's `symbol2numeric`; where Soufflé stops with an error (no digits, out
//!   of range, `x % 0`) there is no value, and so no row. `@lower` is Unicode lowercase; `@dot("", b)` is b; a
//!   decimal number is its float's key (`float_key`).
//!
//! Not lowered yet, each a named error: patterns (``n ~ `…` ``), `ordered by`, `mean` (a decimal number),
//! `dfs_first`/`dfs_first_memo`, and a negated built-in with its other node unnamed (`!within_stop(n, _, s)`,
//! `!split_dot(q, _, x)`).

pub mod core;
#[cfg(any(test, feature = "test-hooks"))]
pub use self::core::drop_unread_core;
pub use self::core::is_core;

use std::collections::{BTreeMap, BTreeSet};

use super::from_tape::{S, Stmt};
use rerail_rr_syntax::syntax::quote;
use crate::code_rule_dsl::logic::front::{AXIS, AggOp, Atom, DESCENDANTS, Error, LatticeDecl, List, Literal, Native, RESERVED, Rule, SCOPE_NOT, SpanIn, Term, Value, axis};
use crate::code_rule_dsl::facts::vtab::syntax::GUEST_POLICY;
use crate::code_rule_dsl::logic::strings::SET;
use crate::code_rule_dsl::logic::program_facts::Field;
use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut, ProgramFields};
pub mod findings;
use super::clause::{self as reads, At, HeadAt, L, LitAt, Reads, T};

/// The text tests: their `.rr` names and the engine's.
pub const TESTS: [(&str, &str); 11] = [
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

pub(super) fn test_of(rel: &str) -> Option<&'static str> {
    TESTS.iter().find(|(rr, _)| *rr == rel).map(|(_, engine)| *engine)
}

/// The whole-string regex of a pattern computed in a rule, `p`: `^(?:p)$` made as the row's pattern is.
pub(super) fn anchored(p: Term) -> Term {
    let concat = |a: Term, b: Term| Term::Call("concat".into(), [a, b].into());
    concat(Term::Str("^(?:".into()), concat(p, Term::Str(")$".into())))
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
/// Relations the lowering makes itself rather than a `.decl` (`check::undeclared`): regex matches, tree axes and
/// walks, the dotted-name split.
pub(super) const BUILT_INS: &[&str] = &["match_all", "match_group", "nearest", "inside", "has", "within", "within_stop", "precedes_stop", "split_dot", "ast", "unset"];

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
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
enum FindingCode {
    Name,
    Column(usize),
    Constant(String),
}

/// The relation of the `@place(path, line)` rows of a violation V: `(path, line, V's other columns)`.
fn place_relation(v: &str) -> String {
    format!("rr_place_{v}")
}

/// What a deciding walk logs for its writing walks to read again (decision 0276 A3.3).
#[derive(Clone, Debug)]
enum Logged {
    /// A fresh name, in the order made.
    Name(String),
    /// A conjunction's literals' counts of alternatives, and whether each is outlined (`conjunction_put`): a replay
    /// reads them before it lowers the literals, to split its combination into each one's pick.
    Counts(Vec<(usize, bool)>),
}

/// Where a writing walk's literals go (`Lowerer::open`). A lowerer is copied (its seeds) with none.
#[derive(Default)]
struct Writing(Option<Sink>);

enum Sink {
    /// The rule open on the header, its variables `unset(x)` binds empty, and the negated test held back.
    Rule(crate::code_rule_dsl::logic::program_image::Open, BTreeSet<String>, Option<Literal>),
    /// The literals of one alternative, for a helper that still reads whole bodies (`alts_of`, decision 0276 A3.4).
    Trees(Vec<Literal>),
}

impl Clone for Writing {
    fn clone(&self) -> Self {
        assert!(self.0.is_none(), "a lowerer is copied with no rule open");
        Writing(None)
    }
}

#[derive(Clone, Default)]
struct Lowerer {
    /// The rules the walk writes onto its header, a text's version (`Written`, decision 0377 step 3).
    out: Written,
    /// The fresh names the clause's deciding walk made, in order, which its writing walks read again (decision 0276
    /// A3.3).
    log: Vec<Logged>,
    /// While a writing walk replays the clause: the next name of `log` it reads. Its rule writes then do nothing:
    /// the deciding walk wrote them.
    replay: Option<usize>,
    /// The rule a writing walk writes into (`emit`), its variables `unset(x)` binds empty (`unset_vars`), and the
    /// negated test held back until the next literal shows whether it merges with it (`merge_negated_test`).
    open: Writing,
    /// Whether `emit` writes: off in the alternatives of a lowering other than the one it is told to write (`alt`).
    off: bool,
    /// Its header (decision 0276 §3): a module's declarations, inputs, facts and fields as the walk states them, its
    /// rules then written onto it (`Written`); the scope's, seeded; the modules' linked so far.
    prog: crate::code_rule_dsl::logic::program_image::Builder<'static>,
    /// `.lattice` declarations: name → (base type, its `Lub` functor).
    lattices: BTreeMap<String, (String, String)>,
    /// Relations given as a kind set.
    sets: BTreeSet<String>,
    /// `.violation`s, by relation.
    violations: Vec<Violation>,
    next: usize,
    /// The literals beside the one being lowered, in its conjunction and those enclosing it: what binds an
    /// aggregate's shared variables (`aggregate`). Positions in the clause's tape (`reads`).
    siblings: Vec<LitAt>,
    /// The file of the clause being lowered, whose tape its literals are read off (`reads`).
    file: Option<std::sync::Arc<super::from_tape::RrFile>>,
    /// The clause's substitution sets: the variables standing for the terms its hoistings moved (`reads::Sub`).
    subs: Vec<reads::Sub>,
    /// The relations the clause being lowered writes: a context leaves their atoms out where it can (`context`).
    heads: BTreeSet<String>,
    /// The variables of the clause's heads: a derived atom over them alone filters the clause by its own columns
    /// (a demand), and a context may read it (`context`).
    head_vars: BTreeSet<String>,
    /// The clause's first body literal, where it is a positive atom: a translated helper's demand stands there
    /// (`demand_alt_8(p, N) :- demand_read_elsewhere(n, S), …`), and a context may read it (`context`).
    first_atom: Option<String>,
    /// Each derived relation's body relations, over every clause (`context`: a derived atom joins a context only
    /// where it does not depend on the clause's heads).
    deps: BTreeMap<String, BTreeSet<String>>,
    /// A context for a `match_all` over a span (`aggregate`): made of the rule's literals, not inputs alone, so it
    /// binds the span where the rule does and `span_patterns` finds its containers (decision 0081).
    span_context: bool,
    /// A context for a request (`demand`: a guest to parse, a value to make): made of every literal beside it, own
    /// heads and derived relations included. A request's context is all that limits what is asked for, so the
    /// narrowing an ordinary context may take (inputs alone, which the rule joins with its own literals again)
    /// would ask for every row the inputs give: `type_ctx(r) :- type_ctx(s), str_node(s), parsed(s, …)` asked to
    /// parse every string of a file.
    request_context: bool,
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
    /// The helper relations made so far (`helper_once`), by their stem, columns and rules: an identical one is that
    /// relation again.
    helpers: std::collections::HashMap<HelperKey, String>,
    /// The variable names a helper's rules are written with in its key (`front::template`), made once.
    template_names: Option<crate::code_rule_dsl::logic::front::template::CanonicalNames>,
    /// The helpers `helper_once` declared, in the order made: each name, its stem and its rules' count then (the key's
    /// rules: a helper's first rules; demand may add more). Linking shares a module's helper that one an earlier
    /// module made equals, as one lowering of them all would (`append`), so a module's image keeps them.
    once: Vec<(String, String, usize)>,
    /// The helper names `fresh` made, by stem and number: linking numbers a module's after those of the modules
    /// linked before it (decision 0246 §2), as one lowering of them all would have.
    made: Vec<(String, usize)>,
    /// How many of its inputs, first in `prog.inputs`, its `.input` statements state: the rest are those its clauses'
    /// lowering added. Linking keeps every module's stated inputs before the added ones, as one lowering of them all
    /// would order them.
    stated: usize,
    /// The rules linked as versions so far (`link`), which `prog` does not hold: a `must` or `may` rule's place.
    appended: usize,
}

/// The lowerer's edits of its header (`Lowerer::prog`), on the builder through the fields API (decision 0276 §3): a
/// field read, changed and stated again.
trait Header: ProgramFields {
    /// Declares `rel` with these columns (names and types), replacing a declaration it had.
    fn declare_cols<C: AsRef<str>, T: AsRef<str>>(&mut self, rel: &str, cols: impl IntoIterator<Item = (C, T)>);
    /// Field `read` reads, changed by `f` and stated again as `field` makes it.
    fn edit<X>(&mut self, read: impl FnOnce(&Self) -> X, field: for<'a> fn(&'a X) -> Field<'a>, f: impl FnOnce(&mut X));
    fn push_setting(&mut self, key: String, text: String) {
        self.edit(|b| ProgramFields::settings(b), |s| Field::Settings(s), |s| s.push((key, text)));
    }
    fn set_native(&mut self, rel: &str, native: Native) {
        self.edit(|b| ProgramFields::natives(b), |n| Field::Natives(n), |n| {
            n.insert(rel.to_owned(), native);
        });
    }
    fn set_kindset(&mut self, rel: &str, kinds: Vec<String>) {
        self.edit(|b| ProgramFields::kindsets(b), |k| Field::Kindsets(k), |k| {
            k.insert(rel.to_owned(), kinds);
        });
    }
}

impl Header for crate::code_rule_dsl::logic::program_image::Builder<'static> {
    fn declare_cols<C: AsRef<str>, T: AsRef<str>>(&mut self, rel: &str, cols: impl IntoIterator<Item = (C, T)>) {
        let cols: Vec<(C, T)> = cols.into_iter().collect();
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_ref(), t.as_ref())).collect();
        self.declare(rel, &pairs);
    }
    fn edit<X>(&mut self, read: impl FnOnce(&Self) -> X, field: for<'a> fn(&'a X) -> Field<'a>, f: impl FnOnce(&mut X)) {
        let mut x = read(self);
        f(&mut x);
        self.set_field(field(&x));
    }
}

/// The rules the walk writes onto the lowerer's header (`Lowerer::prog`), through its open-rule writer
/// (`Builder::open_rule`, decision 0276 A3).
#[derive(Clone, Default)]
pub struct Written {
    /// How many rules it wrote: a `must` or `may` head's rules are known by their place (`modal`).
    n: usize,
    /// The hashes of the rules written once (`Builder::close_rule_once`): a demand is written once (`demand`,
    /// `world_demand`).
    seen: std::collections::HashSet<u64>,
    /// The first rule the builder refused.
    error: Option<String>,
}

impl Written {
    /// `rule` written onto `b`, the lowerer's header; once, when `once`.
    fn write(&mut self, b: &mut crate::code_rule_dsl::logic::program_image::Builder<'static>, rule: &Rule, once: bool) {
        match put_rule(b, rule) {
            Ok(o) => self.close(b, o, once),
            Err(e) => {
                self.error.get_or_insert(e);
            }
        }
    }

    /// Ends rule `o` open on `b`; once, when `once` (left out when a rule of its hash was written once before).
    fn close(&mut self, b: &mut crate::code_rule_dsl::logic::program_image::Builder<'static>, o: crate::code_rule_dsl::logic::program_image::Open, once: bool) {
        let closed = if once { b.close_rule_once(o, &mut self.seen) } else { b.close_rule(o).map(Some) };
        match closed {
            Ok(Some(_)) => self.n += 1,
            Ok(None) => {}
            Err(e) => {
                self.error.get_or_insert(e);
            }
        }
    }

    /// The version written: `b`, the lowerer's header with the rules written onto it.
    fn version(self, b: crate::code_rule_dsl::logic::program_image::Builder<'static>) -> Result<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, Error> {
        if let Some(e) = self.error {
            return Err(Error::Syntax(e));
        }
        let bytes = b.finish_bytes().map_err(Error::Syntax)?;
        crate::code_rule_dsl::logic::program_image::Version::open(&bytes).map_err(Error::Syntax)
    }
}

/// `rule` put on `b` literal by literal (`Builder::open_rule`), open for `Written::close`. (Until each lowering
/// writes its literals itself, decision 0276 A3.)
fn put_rule(b: &mut crate::code_rule_dsl::logic::program_image::Builder<'static>, rule: &Rule) -> Result<crate::code_rule_dsl::logic::program_image::Open, String> {
    let mut o = b.open_rule();
    b.put_head(&mut o, rule.head.rel.as_ref(), &ts(&rule.head.args));
    for l in rule.body.iter() {
        if let Err(e) = put_literal(b, &mut o, l) {
            b.abandon_rule(o);
            return Err(e);
        }
    }
    Ok(o)
}

/// Term `x` to write (`Builder::put_*`).
fn t(x: &Term) -> crate::code_rule_dsl::logic::program_image::T<'_> {
    use crate::code_rule_dsl::logic::program_image::T;
    match x {
        Term::Var(v) => T::Var(v.as_ref()),
        Term::Wild => T::Wild,
        Term::Int(i) => T::Int(*i),
        Term::Str(s) => T::Str(s.as_ref()),
        Term::Top => T::Top,
        Term::Call(f, args) => T::Call(f.as_ref(), args.iter().map(t).collect()),
        Term::Branded(b) => T::Branded(*b),
    }
}

fn ts(args: &[Term]) -> Vec<crate::code_rule_dsl::logic::program_image::T<'_>> {
    args.iter().map(t).collect()
}

/// Literal `l` written into rule `o`, open on `b`.
fn put_literal(b: &mut crate::code_rule_dsl::logic::program_image::Builder<'static>, o: &mut crate::code_rule_dsl::logic::program_image::Open, l: &Literal) -> Result<(), String> {
    use crate::code_rule_dsl::logic::program_image::CmpOp;
    match l {
        Literal::Atom(a) => b.put_atom(o, a.rel.as_ref(), &ts(&a.args), a.neg, a.gated),
        Literal::Cmp { l, op, r } => {
            let Some(op) = CmpOp::of(op) else { return Err(format!("comparison {op:?}")) };
            b.put_cmp(o, t(l), op, t(r));
        }
        Literal::Agg { var, op, over, atom } => b.put_agg(o, var, op, over.as_deref(), atom.rel.as_ref(), &ts(&atom.args)),
        Literal::Nearest { n, a, kindset } => b.put_nearest(o, n, a, kindset),
        Literal::Ast { document, node, matcher, captures } => {
            let caps: Vec<(&str, &str)> = captures.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
            b.put_ast(o, document, node, matcher, &caps);
        }
    }
    Ok(())
}

/// A helper relation's stem, column types and rules (their templates: variables numbered in order, so the fresh
/// names one occurrence made inside it do not tell it from another).
type HelperKey = (String, Vec<String>, Vec<String>);

/// The globs a program's `.scope` settings select documents by: (within, out), `dirs` and `include` within, `exclude`
/// and `tests false` out. `None` for no scope and for `.scope given` (the files are a caller's, a test's, not found
/// by walking a repository). `report dirs`, `text` and `embedded` say where findings land and how files are read, not
/// which; an `option("…")` entry's value is a policy option's, not known here.
pub fn scope_globs(settings: &[(String, String)]) -> Option<(Vec<String>, Vec<String>)> {
    if settings.iter().any(|(k, t)| k == "scope" && t.trim() == "given") {
        return None;
    }
    let (mut within, mut out) = (Vec::new(), Vec::new());
    for (_, text) in settings.iter().filter(|(k, _)| k == "scope") {
        // The setting's tokens: comments are not in them.
        let Ok(tokens) = rerail_rr_syntax::syntax::tokens(text) else { continue };
        let tokens: Vec<&rerail_rr_syntax::syntax::Tok> = tokens.iter().map(|t| &t.0).collect();
        let words: Vec<&str> = tokens.iter().filter_map(|t| if let rerail_rr_syntax::syntax::Tok::Id(w) = t { Some(*w) } else { None }).collect();
        // `report dirs` and `text` say where findings land and how files are read, not which.
        if words.first().is_some_and(|w| matches!(*w, "report" | "text" | "embedded")) {
            continue;
        }
        let mut word = "";
        for (k, t) in tokens.iter().enumerate() {
            match t {
                // `option("…")`: an entry from a policy option, kept by name until the run's options are known
                // (`front::OPTION_GLOB`); the form before it (`dirs`, `include`, `exclude`) stays the word.
                rerail_rr_syntax::syntax::Tok::Id(w) if *w == "option" && matches!(tokens.get(k + 1), Some(rerail_rr_syntax::syntax::Tok::P("("))) => {
                    if let Some(rerail_rr_syntax::syntax::Tok::Str(name)) = tokens.get(k + 2) {
                        let entry = format!("{}{word}:{name}", crate::code_rule_dsl::logic::front::OPTION_GLOB);
                        match word {
                            "dirs" | "include" => within.push(entry),
                            "exclude" => out.push(entry),
                            _ => {}
                        }
                    }
                }
                rerail_rr_syntax::syntax::Tok::Id(_) if matches!(tokens.get(k + 1), Some(rerail_rr_syntax::syntax::Tok::P("("))) => word = "",
                rerail_rr_syntax::syntax::Tok::Id(w) => {
                    if *w == "false" && word == "tests" {
                        out.extend(["test/**", "**/test/**", "tests/**", "**/tests/**"].map(str::to_owned));
                    }
                    word = w;
                }
                rerail_rr_syntax::syntax::Tok::Str(g) if !matches!(tokens.get(k.wrapping_sub(1)), Some(rerail_rr_syntax::syntax::Tok::P("("))) => match word {
                    "dirs" => within.push(format!("{}/**", g.trim_end_matches('/'))),
                    "include" => within.push(g.to_string()),
                    "exclude" => out.push(g.to_string()),
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
        let Ok(tokens) = rerail_rr_syntax::syntax::tokens(text) else { continue };
        let words: Vec<&str> = tokens.iter().take(form.len()).filter_map(|t| if let rerail_rr_syntax::syntax::Tok::Id(w) = &t.0 { Some(*w) } else { None }).collect();
        if words != form {
            continue;
        }
        found.extend(tokens.iter().filter_map(|t| if let rerail_rr_syntax::syntax::Tok::Str(g) = &t.0 { Some(g.to_string()) } else { None }));
    }
    found
}

/// `.scope text "**/*.md", …`: documents read as text only (`Field::TextGlobs`).
pub fn scope_text_globs(settings: &[(String, String)]) -> Vec<String> {
    scope_list(settings, &["text"])
}

/// `.scope report dirs "api", …`: where findings may land, as globs; facts still come from the whole scope.
fn scope_report_globs(settings: &[(String, String)]) -> Vec<String> {
    scope_list(settings, &["report", "dirs"]).into_iter().map(|d| format!("{}/**", d.trim_end_matches('/'))).collect()
}

/// A rule file lowered alone (decision 0246 §1): each clause rewritten to the core rules it lowers to, its
/// declarations, facts and settings, and what lowering's whole-program half reads of them (its `.violation`s, its
/// `must, may` relations and heads, its kind sets). What needs the whole program is `link`'s.
#[derive(Clone)]
pub struct Module {
    l: Lowerer,
    /// Its relations' column types as written (`tape_type`), for the modules that include it.
    written: BTreeMap<String, Vec<String>>,
    /// Whether its `docof` is over `loc`s, when it declares `docof`.
    docof: Option<bool>,
    /// The `.violation`s of its includes that its rules give a `@place` (`Lowerer::clause`).
    places: BTreeSet<String>,
    /// Its program, rules and header, as its walk wrote it (decision 0377 step 3) or its image holds it.
    version: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
}

/// `statements` lowered as a module whose includes are `scope` (each module it reaches, once): its clauses read their
/// declarations, inputs, lattices, kind sets and natives, and the result holds only its own (`Module`).
/// A scope seeded once (decision 0246 §2): a lowerer holding what a scope's modules give the modules lowered in it,
/// with what it took and the scope's tape types. Every program of a run in the same scope starts from a copy.
pub struct Seeds {
    l: Lowerer,
    seeded: Seeded,
    given: BTreeMap<String, Vec<String>>,
    /// The demands the scope's modules stated.
    printed: BTreeSet<String>,
}

/// `scope`, seeded (`Seeds`).
pub fn seeds(scope: &[&Module]) -> Seeds {
    let _seeding = crate::code_rule_dsl::logic::profile::span(|| "seed a scope".to_owned());
    let mut l = Lowerer::default();
    let seeded = l.seed(scope);
    let mut given: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for m in scope {
        given.extend(m.written.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    let printed = scope.iter().flat_map(|m| m.l.printed_demands.iter().cloned()).collect();
    Seeds { l, seeded, given, printed }
}

/// `statements` lowered as a module of a scope seeded already (`seeds`), its rules written as a version in the walk
/// (decision 0377 step 3), which `link_seeded_parts` links without editing its rules and `rr::link_seeded_version`
/// appends onto its scope.
pub fn lower_module_seeded_written(statements: Vec<Stmt>, seeds: &Seeds) -> Result<Module, Error> {
    let typing = crate::code_rule_dsl::logic::profile::span(|| "type the tapes' terms".to_owned());
    let written: BTreeMap<String, Vec<String>> = statements
        .iter()
        .filter(|s| matches!(s.what(), Some(S::Decl(_))))
        .filter_map(|s| {
            let (t, n) = s.tape();
            let (name, cols, _) = t.decl(n).ok()?;
            Some((name, cols.into_iter().map(|(_, t)| t).collect()))
        })
        .collect();
    super::tape_type::check_in(&statements, &seeds.given)?;
    drop(typing);
    let reading = crate::code_rule_dsl::logic::profile::span(|| "read the statements".to_owned());
    let declares_docof = statements.iter().any(|s| s.what() == Some(S::Decl("docof")));
    let mut l = seeds.l.clone();
    let seeded = seeds.seeded.clone();
    let mut clauses = Vec::new();
    // `.shape` binds to the rule just before it (O-join-shape-directives): its heads, then each directive and its line.
    let mut last_heads: Option<Vec<String>> = None;
    let mut shapes: Vec<(crate::code_rule_dsl::logic::front::Shape, usize)> = Vec::new();
    // `.layout r …` (decision 0361): read once every declaration is, as `r`'s columns name its facts.
    let mut layouts: Vec<(String, usize)> = Vec::new();
    // The facts its clauses state, by relation, stated on the header once read.
    let mut own_facts: BTreeMap<String, Vec<crate::code_rule_dsl::logic::front::List<Value>>> = BTreeMap::new();
    // Each statement is read off its node as the walk reaches it, and what it states written at once.
    for st in &statements {
        let line = st.line();
        let at = |e: Error| match e {
            Error::Syntax(m) => Error::Syntax(format!("line {line}: {m}")),
            e => e,
        };
        let (t, node) = st.tape();
        match st.read().map_err(at)? {
            S::Include(path) => return Err(Error::Syntax(format!("line {line}: #include {path:?} not resolved"))),
            S::Decl(_) => {
                let (name, cols, tail) = t.decl(node).map_err(at)?;
                l.decl(name, cols, &tail).map_err(at)?
            }
            S::Edit(_) | S::Functor(_) | S::Type(_) => {}
            // `.input r(IO=json, filename="…")`: r is an input whose rows come from that file (or artifact), kept
            // as the setting `input r(IO=…)` for the runner; without them r is empty.
            S::Io { .. } => {
                let (output, names, params) = t.io(node).map_err(at)?;
                for (name, text) in params {
                    l.prog.push_setting(if output { "output" } else { "input" }.to_owned(), format!("{name}({text})"));
                }
                for n in &names {
                    if output { l.prog.add_output(n) } else { l.prog.add_input(n) }
                }
            }
            S::Lattice(_) => {
                let (name, base, ops) = t.lattice(node).map_err(at)?;
                let lub = ops.iter().find(|(op, _)| op == "Lub").and_then(|&(_, v)| if t.is(v, super::tape::Kind::FunctorCall) { t.name(v, super::tape::Field::Function).ok() } else { None });
                let Some(lub) = lub else {
                    return Err(at(Error::Syntax(format!(".lattice {name} has no `Lub -> @f`"))));
                };
                l.lattices.insert(name, (ty(&base), lub));
            }
            S::Pragma => match t.pragma(node).map_err(at)?.as_slice() {
                [magic, relations] if magic == "magic-transform" => {
                    l.prog.edit(|b| b.demand(), |d| Field::Demand(d), |d| d.extend(relations.split(',').map(str::trim).filter(|r| !r.is_empty()).map(str::to_owned)));
                }
                // `.pragma "cross-rounds" "k"` (decision 0056), refused: bounded cross-file rounds are retired: a program's cross-file part is one fixpoint (decisions 0065, 0276 §9).
                [key, _] if key == crate::code_rule_dsl::logic::front::PRAGMA => {
                    return Err(at(Error::Syntax(format!(".pragma \"{key}\": bounded cross-file rounds are retired: a program's cross-file part is one fixpoint (decisions 0065, 0276 §9)"))));
                }
                // `.pragma "cross-memo" "r s"` (decision 0076): cross-file relations memoised per import component.
                [key, rels] if key == "cross-memo" => {
                    l.prog.push_setting("pragma".to_owned(), format!("\"cross-memo\" \"{rels}\""));
                }
                words => return not_yet(format!(".pragma {words:?}")).map_err(at),
            },
            S::Policy => {
                let document = t.policy(node)?;
                l.prog.edit(|b| b.policies(), |p| Field::Policies(p), |p| p.push(document))
            }
            // A test is never the program's (decision 0130): testing mode splices its statements into a program of
            // its own (`rr::testing`).
            S::Test | S::Fixtures => {}
            S::Setting { key, text } if key == "shape" => {
                let Some(heads) = &last_heads else { return Err(Error::Syntax(format!("line {line}: `.shape {text}` follows no rule"))) };
                for h in heads {
                    shapes.push((crate::code_rule_dsl::logic::front::Shape::parse(h, text).map_err(|m| Error::Syntax(format!("line {line}: {m}")))?, line));
                }
            }
            S::Setting { key, text } if key == "layout" => layouts.push((text.to_owned(), line)),
            S::Setting { key, text } => l.prog.push_setting(key.to_owned(), text.to_owned()),
            S::Violation(_) => {
                l.violation(&t, node).map_err(at)?
            }
            S::Clause => {
                l.start_clause(st.file().clone());
                l.check_clause(node).map_err(at)?;
                let edits = st.has_edits();
                if !st.has_body() && !edits && let Some(rows) = l.facts_at(&l.heads_at(node)) {
                    for (rel, row) in rows {
                        own_facts.entry(rel).or_default().push(row.into());
                    }
                    last_heads = None;
                    continue;
                }
                last_heads = (!edits).then(|| st.heads().into_iter().map(str::to_owned).collect());
                clauses.push((st.clone(), edits, line));
            }
        }
    }
    for (rel, rows) in own_facts {
        l.prog.add_facts(&rel, &rows);
    }
    for (text, line) in layouts {
        let (rel, facts) = layout(&l.prog, &text).map_err(|m| Error::Syntax(format!("line {line}: `.layout {}`: {m}", text.trim())))?;
        l.prog.edit(|b| ProgramFields::layouts(b), |x| Field::Layouts(x), |x| crate::code_rule_dsl::logic::front::add_layout(x.entry(rel).or_default(), facts));
    }
    drop(reading);
    let stated: BTreeSet<String> = l.prog.input_names().into_iter().filter(|r| !seeded.inputs.contains(r)).collect();
    let rules = crate::code_rule_dsl::logic::profile::span(|| "lower the rules".to_owned());
    l.derived = clauses.iter().flat_map(|(st, ..)| st.heads().into_iter().map(str::to_owned)).chain(l.prog.fact_names()).collect();
    let mut own_deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (st, ..) in &clauses {
        l.start_clause(st.file().clone());
        let body = st.tape().0.get(st.node(), super::tape::Field::Body).map_or_else(Vec::new, |b| l.body_at(b, 0));
        let mut rels = BTreeSet::new();
        l.lit_rels(&body, &mut rels);
        for h in st.heads() {
            own_deps.entry(h.to_owned()).or_default().extend(rels.iter().cloned());
        }
    }
    for (rel, reads) in &own_deps {
        l.deps.entry(rel.clone()).or_default().extend(reads.iter().cloned());
    }
    // The demands the text states (a printed program's): its own declarations, as read, and those its includes
    // stated. A demand an include's lowering made is not one: this module makes its own.
    l.printed_demands = DEMANDS.iter().chain(WORLD.iter().map(|(_, d)| d)).filter(|d| l.prog.declares(d) && !seeded.decls.contains(**d)).map(|d| (*d).to_owned()).collect();
    l.printed_demands.extend(seeds.printed.iter().cloned());
    for (st, edits, line) in clauses {
        // A rule ending in `=> edits` defines a fix; fixes are not run, so it is left out.
        if edits {
            continue;
        }
        l.clause(&st).map_err(|e| match e {
            Error::Syntax(m) => Error::Syntax(format!("line {line}: {m}")),
            e => e,
        })?;
    }
    for (shape, _) in &shapes {
        l.prog.edit(|b| b.shapes(), |s| Field::Shapes(s), |s| s.push(shape.clone()));
    }
    drop(rules);
    let places = l.unseed(seeded);
    l.stated = l.prog.input_names().iter().take_while(|r| stated.contains(*r)).count();
    l.deps = own_deps;
    let docof = declares_docof.then_some(l.docof_of_locs);
    // The module's header goes into its version with its rules: what the link and the modules that include it read.
    let version = std::mem::take(&mut l.out).version(std::mem::take(&mut l.prog))?;
    // Each `.shape` names what its rules read, in the version: a relation of their body, and the variables it waits
    // for.
    shapes_hold(&version, &shapes)?;
    Ok(Module { l, written, docof, places, version })
}

/// Each `.shape`'s rules, read in `version`, read its relation (and what it waits after), and have each variable it
/// waits for: else the directive's line and why.
fn shapes_hold(version: &crate::code_rule_dsl::logic::program_image::Version, shapes: &[(crate::code_rule_dsl::logic::front::Shape, usize)]) -> Result<(), Error> {
    use crate::code_rule_dsl::logic::front::{ShapeWait, shape_base};
    use crate::code_rule_dsl::logic::program_image::{RuleRef, Slot, kind};
    // A variable of a rule's body: a variable leaf after its head, or an aggregate's result.
    let in_body = |r: &RuleRef<'_>, v: &str| {
        let im = r.image();
        let head = r.head().node();
        (head + im.node(head).1..r.nodes().end).any(|n| {
            let (k, _, val) = im.node(n);
            matches!(k & kind::MASK, kind::VAR | kind::AGG) && version.text(r.var(Slot(val))) == v
        })
    };
    for (shape, line) in shapes {
        let rules: Vec<RuleRef<'_>> = version.rules().filter(|r| version.rel_name(r.head().rel()) == shape.head).collect();
        let reads = |rel: &str| rules.iter().any(|r| r.body().filter_map(|l| l.atom()).any(|a| shape_base(version.rel_name(a.rel())) == rel));
        let missing = match &shape.wait {
            _ if !reads(&shape.lit) => Some(format!("no rule of `{}` reads `{}`", shape.head, shape.lit)),
            ShapeWait::After(a) if !reads(a) => Some(format!("no rule of `{}` reads `{a}`", shape.head)),
            ShapeWait::Bound(vars) => vars.iter().find(|v| !rules.iter().any(|r| in_body(r, v))).map(|v| format!("no rule of `{}` has the variable `{v}`", shape.head)),
            _ => None,
        };
        if let Some(why) = missing {
            return Err(Error::Syntax(format!("line {line}: `.shape {}`: {why}", shape.text)));
        }
    }
    Ok(())
}

/// A `.layout` line's relation and facts (decision 0361): the relation must be declared, and each fact names its
/// columns.
fn layout(prog: &(impl ProgramFields + ?Sized), text: &str) -> Result<(String, Vec<crate::code_rule_dsl::logic::front::LayoutFact>), String> {
    let text = text.trim();
    let (rel, rest) = text.split_once(|c: char| c.is_whitespace()).unwrap_or((text, ""));
    let cols = prog.columns_of(rel).ok_or_else(|| format!("`{rel}` is not declared"))?;
    let col = |w: &str| cols.iter().position(|(c, _)| c.as_str() == w).ok_or_else(|| format!("`{rel}` has no column `{w}`"));
    Ok((rel.to_owned(), crate::code_rule_dsl::logic::front::layout_facts(rest, &col)?))
}

/// A module of the modules `scope` reaches (each once), its rules written as a version in the walk: a library's
/// module (`rr::lower_library`).
pub fn lower_module_in_written(statements: Vec<Stmt>, scope: &[&Module]) -> Result<Module, Error> {
    lower_module_seeded_written(statements, &seeds(scope))
}

/// A module's parts as its image keeps them (decision 0246 §6): its program, and what lowering's whole-program half
/// and the modules that include it read of it, each as rows of names and numbers.
#[derive(Clone, Debug, Default)]
pub struct ModuleParts {
    /// Each `.violation`: name, arity, code (`name`, `column` with its index, or `constant` with its text), whether
    /// its `at` is a node, whether it has a place relation, its columns, its message and its tag column.
    pub violations: Vec<ViolationParts>,
    pub three: BTreeSet<String>,
    /// The rules a `must` or `may` head wrote, by index.
    pub modal: Vec<(usize, String)>,
    pub sets: BTreeSet<String>,
    /// `.lattice` declarations: name, base type, `Lub` functor.
    pub lattices: Vec<(String, String, String)>,
    /// The helpers `fresh` made (stem, number), and its count.
    pub made: Vec<(String, usize)>,
    pub next: usize,
    /// How many of `prog.inputs`, first, its `.input` statements state.
    pub stated: usize,
    /// The helpers `helper_once` made: name, stem, its rules' count then (`Lowerer::once`).
    pub once: Vec<(String, String, usize)>,
    /// What each derived relation's rules read.
    pub deps: Vec<(String, String)>,
    /// Its relations' column types as written.
    pub written: Vec<(String, Vec<String>)>,
    pub docof: Option<bool>,
    pub places: BTreeSet<String>,
    /// The demands its text, or an include's, states (`Lowerer::printed_demands`).
    pub printed: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViolationParts {
    pub name: String,
    pub arity: usize,
    pub code: (String, usize, String),
    pub node: bool,
    pub places: bool,
    pub columns: Vec<(String, String)>,
    pub message: String,
    pub tag: Option<usize>,
}

impl Module {
    /// Its parts, as its image keeps them.
    pub fn parts(&self) -> ModuleParts {
        let l = &self.l;
        ModuleParts {
            violations: l
                .violations
                .iter()
                .map(|v| ViolationParts {
                    name: v.name.clone(),
                    arity: v.arity,
                    code: match &v.code {
                        FindingCode::Name => ("name".to_owned(), 0, String::new()),
                        FindingCode::Column(i) => ("column".to_owned(), *i, String::new()),
                        FindingCode::Constant(c) => ("constant".to_owned(), 0, c.clone()),
                    },
                    node: v.node,
                    places: v.places,
                    columns: v.columns.clone(),
                    message: v.message.clone(),
                    tag: v.tag,
                })
                .collect(),
            three: l.three.clone(),
            modal: l.modal.iter().map(|(i, m)| (*i, m.clone())).collect(),
            sets: l.sets.clone(),
            lattices: l.lattices.iter().map(|(n, (b, f))| (n.clone(), b.clone(), f.clone())).collect(),
            made: l.made.clone(),
            next: l.next,
            stated: l.stated,
            once: l.once.clone(),
            deps: l.deps.iter().flat_map(|(r, reads)| reads.iter().map(move |x| (r.clone(), x.clone()))).collect(),
            written: self.written.iter().map(|(r, t)| (r.clone(), t.clone())).collect(),
            docof: self.docof,
            places: self.places.clone(),
            printed: l.printed_demands.clone(),
        }
    }

    /// Its program, rules and header (its image's).
    pub fn version(&self) -> &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version> {
        &self.version
    }

    /// The module its image's parts are, its program `version`.
    pub fn of_parts(p: ModuleParts, version: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Result<Module, Error> {
        let mut l = Lowerer { three: p.three, sets: p.sets, made: p.made, next: p.next, stated: p.stated, once: p.once, printed_demands: p.printed, docof_of_locs: p.docof.unwrap_or(false), ..Lowerer::default() };
        for v in p.violations {
            let code = match v.code.0.as_str() {
                "name" => FindingCode::Name,
                "column" => FindingCode::Column(v.code.1),
                "constant" => FindingCode::Constant(v.code.2),
                other => return Err(Error::Syntax(format!("a module image's violation code {other:?}"))),
            };
            l.violations.push(Violation { name: v.name, arity: v.arity, code, node: v.node, places: v.places, columns: v.columns, message: v.message, tag: v.tag });
        }
        l.modal = p.modal.into_iter().collect();
        l.lattices = p.lattices.into_iter().map(|(n, b, f)| (n, (b, f))).collect();
        for (r, x) in p.deps {
            l.deps.entry(r).or_default().insert(x);
        }
        Ok(Module { l, written: p.written.into_iter().collect(), docof: p.docof, places: p.places, version })
    }
}

/// What `Lowerer::seed` took from a module's includes, which `Lowerer::unseed` takes out again.
#[derive(Clone)]
struct Seeded {
    decls: BTreeSet<String>,
    inputs: BTreeSet<String>,
    natives: BTreeSet<String>,
    kindsets: BTreeSet<String>,
    lattice: BTreeSet<String>,
    tapes: BTreeSet<String>,
    qualifiers: BTreeSet<String>,
    lattices: BTreeSet<String>,
    three: BTreeSet<String>,
    /// The includes' `.violation`s, first in `violations`, and whether each had a place relation already.
    violations: Vec<bool>,
}

impl Lowerer {
    /// Starts the clause at `node` of `file`: no substitution yet.
    pub(super) fn start_clause(&mut self, file: std::sync::Arc<super::from_tape::RrFile>) {
        self.file = Some(file);
        self.subs.clear();
        self.subs.push(reads::Sub::default());
    }

    /// A substitution set over `parent`'s.
    pub(super) fn new_sub(&mut self, parent: u32) -> u32 {
        self.subs.push(reads::Sub { parent, map: Default::default() });
        (self.subs.len() - 1) as u32
    }

    /// Variable `v` stands in place of node `n` under set `s`.
    pub(super) fn set_sub(&mut self, s: u32, n: u32, v: String) {
        self.subs[s as usize].map.insert(n, v);
    }
}

/// The lowering reads its clause under the substitutions its hoistings made.
impl Reads for Lowerer {
    /// The tape of the clause being lowered.
    fn tape(&self) -> super::from_tape::Tape<'_> {
        self.file.as_ref().expect("a clause being lowered").tape()
    }

    /// The variable standing in place of node `n` under set `s`.
    fn sub_of(&self, mut s: u32, n: u32) -> Option<&String> {
        loop {
            let sub = &self.subs[s as usize];
            if let Some(v) = sub.map.get(&n) {
                return Some(v);
            }
            if s == 0 {
                return None;
            }
            s = sub.parent;
        }
    }
}

impl Lowerer {
    /// The names a module's clauses read from its includes (`scope`): their declarations, inputs, natives, kind sets,
    /// lattices, `must, may` relations, `.violation`s and what each derived relation reads. Their helpers (`made`)
    /// are their own: a module numbers its own from one.
    fn seed(&mut self, scope: &[&Module]) -> Seeded {
        let mut seeded = Seeded { decls: BTreeSet::new(), inputs: BTreeSet::new(), natives: BTreeSet::new(), kindsets: BTreeSet::new(), lattice: BTreeSet::new(), tapes: BTreeSet::new(), qualifiers: BTreeSet::new(), lattices: BTreeSet::new(), three: BTreeSet::new(), violations: Vec::new() };
        for m in scope {
            let helpers: BTreeSet<String> = m.l.made.iter().map(|(stem, n)| format!("rr_{stem}{n}")).collect();
            // Its header, read from its version.
            use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as F;
            let p = &*m.version;
            for rel in F::declared_names(p).into_iter().filter(|r| !helpers.contains(r)) {
                let cols = F::columns_of(p, &rel).unwrap_or_default();
                seeded.decls.insert(rel.clone());
                self.prog.declare_cols(&rel, cols);
            }
            for rel in F::input_names(p).into_iter().filter(|r| !helpers.contains(r)) {
                if seeded.inputs.insert(rel.clone()) {
                    self.prog.add_input(&rel);
                }
            }
            let natives = F::natives(p);
            seeded.natives.extend(natives.keys().cloned());
            if !natives.is_empty() {
                self.prog.edit(|b| b.natives(), |n| Field::Natives(n), |n| n.extend(natives));
            }
            let kindsets = F::kindsets(p);
            seeded.kindsets.extend(kindsets.keys().cloned());
            if !kindsets.is_empty() {
                self.prog.edit(|b| b.kindsets(), |k| Field::Kindsets(k), |k| k.extend(kindsets));
            }
            let lattice = F::lattice(p);
            seeded.lattice.extend(lattice.keys().cloned());
            if !lattice.is_empty() {
                self.prog.edit(|b| b.lattice(), |d| Field::Lattice(d), |d| d.extend(lattice));
            }
            let tapes = F::tapes(p);
            seeded.tapes.extend(tapes.keys().cloned());
            if !tapes.is_empty() {
                self.prog.edit(|b| b.tapes(), |t| Field::Tapes(t), |t| tapes.into_iter().for_each(|(rel, cols)| t.entry(rel).or_default().extend(cols)));
            }
            let qualifiers = F::qualifiers(p);
            seeded.qualifiers.extend(qualifiers.keys().cloned());
            if !qualifiers.is_empty() {
                self.prog.edit(|b| b.qualifiers(), |q| Field::Qualifiers(q), |q| qualifiers.into_iter().for_each(|(rel, qs)| q.entry(rel).or_default().extend(qs)));
            }
            for (name, l) in &m.l.lattices {
                seeded.lattices.insert(name.clone());
                self.lattices.insert(name.clone(), l.clone());
            }
            seeded.three.extend(m.l.three.iter().cloned());
            self.three.extend(m.l.three.iter().cloned());
            for v in &m.l.violations {
                seeded.violations.push(v.places || m.places.contains(&v.name));
                let mut v = v.clone();
                v.places = *seeded.violations.last().expect("pushed");
                self.violations.push(v);
            }
            for (rel, reads) in &m.l.deps {
                self.deps.entry(rel.clone()).or_default().extend(reads.iter().cloned());
            }
            if let Some(d) = m.docof {
                self.docof_of_locs = d;
            }
        }
        seeded
    }

    /// The module's own part: what `seed` took from its includes taken out. The includes' `.violation`s its rules
    /// gave a place to are returned.
    fn unseed(&mut self, seeded: Seeded) -> BTreeSet<String> {
        let p = &mut self.prog;
        for r in p.declared_names() {
            if seeded.decls.contains(&r) {
                p.undeclare(&r);
            }
        }
        p.retain_inputs(|r| !seeded.inputs.contains(r));
        p.edit(|b| b.natives(), |n| Field::Natives(n), |n| n.retain(|r, _| !seeded.natives.contains(r)));
        p.edit(|b| b.kindsets(), |k| Field::Kindsets(k), |k| k.retain(|r, _| !seeded.kindsets.contains(r)));
        p.edit(|b| b.lattice(), |d| Field::Lattice(d), |d| d.retain(|r, _| !seeded.lattice.contains(r)));
        p.edit(|b| b.tapes(), |t| Field::Tapes(t), |t| t.retain(|r, _| !seeded.tapes.contains(r)));
        p.edit(|b| b.qualifiers(), |q| Field::Qualifiers(q), |q| q.retain(|r, _| !seeded.qualifiers.contains(r)));
        self.lattices.retain(|r, _| !seeded.lattices.contains(r));
        self.three.retain(|r| !seeded.three.contains(r));
        let given: Vec<Violation> = self.violations.drain(..seeded.violations.len()).collect();
        given.into_iter().zip(seeded.violations).filter(|(v, had)| v.places && !had).map(|(v, _)| v.name).collect()
    }

    /// Module `m` after the modules linked so far: its helpers numbered after theirs, and what the lowerer knows of
    /// them added to theirs. Its program, so renamed, is returned (`relinked`) with the count of its inputs stated
    /// first, for `append_fields`.
    ///
    /// Its helpers' keys read their rules, decoded, and their column types; the renames and the merged helpers'
    /// rules and declarations dropped are then written over its version (`relinked`).
    fn link(&mut self, m: Module) -> Result<(std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, usize), Error> {
        use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as F;
        let Module { l: mut m, docof, places, version: v, .. } = m;
        let (mut rules, mut types) = helpers_of(&v, &m.once);
        let mut merged: Vec<(String, String, usize)> = Vec::new();
        let mut rename: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for (stem, n) in &m.made {
            if self.next > 0 {
                rename.insert(format!("rr_{stem}{n}"), format!("rr_{stem}{}", n + self.next));
            }
            self.made.push((stem.clone(), n + self.next));
        }
        // A Python candidates relation is numbered by the natives made before it: another module's may hold the
        // number for another native.
        let linked = self.prog.natives();
        for (name, native) in &F::natives(&*v) {
            if name.starts_with("rr_python_candidates_") && linked.get(name).is_some_and(|n| n != native) {
                let mut serial = linked.len();
                let new = loop {
                    let new = format!("rr_python_candidates_{serial}");
                    if !self.prog.declares(&new) && !F::declares(&*v, &new) {
                        break new;
                    }
                    serial += 1;
                };
                rename.insert(name.clone(), new);
            }
        }
        if !rename.is_empty() {
            rename_rules(&mut rules, &rename);
            types = types.into_iter().map(|(k, t)| (rename.get(&k).cloned().unwrap_or(k), t)).collect();
        }
        // A helper `helper_once` made that equals one an earlier module made is that one, as in one lowering of them
        // all (decision 0225 §6 with 0246): in the order made, each renamed before the next's key reads it.
        for (name, stem, k) in std::mem::take(&mut m.once) {
            let name = rename.get(&name).cloned().unwrap_or(name);
            let key = self.once_key(types.get(&name).cloned().unwrap_or_default(), &rules, &name, &stem, k);
            match self.helpers.get(&key) {
                Some(made) => {
                    let made = made.clone();
                    merged.push((name.clone(), made.clone(), k));
                    types.remove(&name);
                    let mut seen = 0;
                    rules.retain(|r| r.head.rel != name.as_str() || {
                        seen += 1;
                        seen > k
                    });
                    rename_rules(&mut rules, &std::collections::HashMap::from([(name, made)]));
                }
                None => {
                    self.helpers.insert(key, name.clone());
                    self.once.push((name, stem, k));
                }
            }
        }
        self.next += m.next;
        let at = self.appended;
        self.modal.extend(m.modal.into_iter().map(|(i, s)| (i + at, s)));
        for v in self.violations.iter_mut().filter(|v| places.contains(&v.name)) {
            v.places = true;
        }
        self.violations.extend(m.violations);
        self.three.extend(m.three);
        self.sets.extend(m.sets);
        self.lattices.extend(m.lattices);
        if let Some(d) = docof {
            self.docof_of_locs = d;
        }
        let own = relinked(&v, &rename, &merged)?;
        self.appended += own.size();
        Ok((own, m.stated))
    }
}

/// The rules of the helpers `helper_once` made (`once`) in version `v`, decoded, and their column types: what their
/// keys read (`Lowerer::once_key`).
fn helpers_of(v: &crate::code_rule_dsl::logic::program_image::Version, once: &[(String, String, usize)]) -> (Vec<Rule>, BTreeMap<String, Vec<String>>) {
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as F;
    let helpers: BTreeSet<&str> = once.iter().map(|(name, _, _)| name.as_str()).collect();
    let lists = Default::default();
    let mut tree = crate::code_rule_dsl::logic::program_image::Tree::new(v, &lists);
    let rules = v.rules().filter(|r| helpers.contains(v.rel_name(r.head().rel()))).map(|r| tree.rule(r)).collect();
    let types = helpers.iter().map(|h| (h.to_string(), F::columns_of(v, h).unwrap_or_default().into_iter().map(|(_, t)| t).collect())).collect();
    (rules, types)
}

/// A module's program, from version `v`, as `Lowerer::link` leaves it: each relation renamed by `rename`, then the
/// first `k` rules of each helper `merged` into another (`(name, made, k)`, in order) and its declaration dropped and
/// the helper read as that one; its header renamed the same (`renamed_fields`). No rule is decoded.
pub fn relinked(v: &crate::code_rule_dsl::logic::program_image::Version, rename: &std::collections::HashMap<String, String>, merged: &[(String, String, usize)]) -> Result<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, Error> {
    use crate::code_rule_dsl::logic::program_image::{AtomId, AtomRef, Builder, Rewrite, Slot};
    let into: std::collections::HashMap<&str, &str> = merged.iter().map(|(n, m, _)| (n.as_str(), m.as_str())).collect();
    let to = |rel: &str| -> Option<String> {
        let renamed = rename.get(rel).map(String::as_str);
        let r = renamed.unwrap_or(rel);
        into.get(r).map(|m| (*m).to_owned()).or_else(|| renamed.map(str::to_owned))
    };
    struct Renamed<'a, F: Fn(&str) -> Option<String>> {
        v: &'a crate::code_rule_dsl::logic::program_image::Version,
        to: &'a F,
        vars: BTreeMap<Slot, AtomId>,
    }
    impl<F: Fn(&str) -> Option<String>> Rewrite for Renamed<'_, F> {
        fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
            (self.to)(self.v.rel_name(a.rel())).map(|r| (r, a.neg(), a.gated()))
        }
        fn head_as(&mut self, a: AtomRef<'_>) -> Option<String> {
            (self.to)(self.v.rel_name(a.rel()))
        }
        fn kindset(&mut self, set: AtomId) -> Option<String> {
            (self.to)(self.v.text(set))
        }
        fn var(&mut self, s: Slot) -> Option<AtomId> {
            self.vars.get(&s).copied()
        }
    }
    let mut left: std::collections::HashMap<&str, usize> = merged.iter().map(|(n, _, k)| (n.as_str(), *k)).collect();
    let mut b = Builder::base();
    let edits = !rename.is_empty() || !merged.is_empty();
    for r in v.rules() {
        let head = v.rel_name(r.head().rel());
        let renamed = rename.get(head).map(String::as_str).unwrap_or(head);
        if let Some(k) = left.get_mut(renamed)
            && *k > 0
        {
            *k -= 1;
            continue;
        }
        if !edits {
            b.copy(v, r);
            continue;
        }
        // A variable named as a renamed relation is renamed too, as `rename_relations` renames it.
        let mut vars = BTreeMap::new();
        for (i, a) in r.vars().enumerate() {
            if let Some(new) = to(v.text(a)) {
                vars.insert(Slot(i as u32), b.atom(&new));
            }
        }
        let mut rw = Renamed { v, to: &to, vars };
        b.rewrite(v, r, &mut rw).finish().map_err(Error::Syntax)?;
    }
    if edits {
        renamed_fields(&mut b, v, &to, rename, &into);
    } else {
        b.copy_fields(v, &|r: &str| !r.starts_with("module_"));
    }
    let bytes = b.finish_bytes().map_err(Error::Syntax)?;
    crate::code_rule_dsl::logic::program_image::Version::open(&bytes).map_err(Error::Syntax)
}

/// `rules` as a version of no header (what `relinked` reads of a module's rules).
fn rules_version(rules: &[Rule]) -> Result<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, Error> {
    let mut b = crate::code_rule_dsl::logic::program_image::Builder::base();
    for r in rules {
        b.add(r).map_err(Error::Syntax)?;
    }
    crate::code_rule_dsl::logic::program_image::Version::open(&b.finish_bytes().map_err(Error::Syntax)?).map_err(Error::Syntax)
}

/// A library's modules linked as versions (`link`'s steps, the rules kept as versions): the first module's program,
/// each module after it as `link` leaves it with the inputs stated before it and its own stated count (what
/// `lowering_tail::AppendModule` appends), and what the lowerer knows of them all (`whole_front`), for the
/// lowering's tail on the version (`rr::lower_library`).
pub struct LinkedVersions {
    pub first: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    pub appended: Vec<(std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, usize, usize)>,
    pub lowered: Lowered,
}

/// `link` with the modules' rules kept as versions (`LinkedVersions`).
pub fn link_versions(modules: Vec<Module>) -> Result<LinkedVersions, Error> {
    let mut modules = modules.into_iter();
    let Some(Module { mut l, version: v, .. }) = modules.next() else {
        let lowered = whole_front(Lowerer::default())?;
        let first = relinked(rules_version(&[])?.as_ref(), &Default::default(), &[])?;
        return Ok(LinkedVersions { first, appended: Vec::new(), lowered });
    };
    // The first module's helpers by their keys (read from an image, it has none), as `link` keys them.
    l.helpers.clear();
    {
        let (rules, types) = helpers_of(&v, &l.once);
        for (name, stem, k) in l.once.clone() {
            let key = l.once_key(types.get(&name).cloned().unwrap_or_default(), &rules, &name, &stem, k);
            l.helpers.entry(key).or_insert(name);
        }
    }
    // The lowerer's header is the modules' linked so far: what the next module's renames are checked against.
    let first = relinked(&v, &Default::default(), &[])?;
    let mut stated = 0;
    append_fields(&mut l.prog, &*first, &mut stated, l.stated);
    l.stated = stated;
    l.appended = first.size();
    let mut appended = Vec::new();
    for m in modules {
        let (own, b_stated) = l.link(m)?;
        appended.push((std::sync::Arc::clone(&own), l.stated, b_stated));
        append_fields(&mut l.prog, &*own, &mut l.stated, b_stated);
    }
    let lowered = whole_front(l)?;
    Ok(LinkedVersions { first, appended, lowered })
}

/// A module's fields (`Lowerer::link`), `b`'s, added to `a`'s, the modules' linked so far: a version's edit's or a
/// tree's. A declaration, a lattice, a kind set, a native or an answer of `b` replaces `a`'s; facts, qualifiers and
/// tape columns add to `a`'s; the inputs `b` states first (`b_stated`) go after the `stated` `a` states first, the rest
/// last; each list keeps its first.
pub fn append_fields(a: &mut (impl crate::code_rule_dsl::logic::program_image::fields::FieldsMut + ?Sized), b: &(impl crate::code_rule_dsl::logic::program_image::fields::ProgramFields + ?Sized), stated: &mut usize, b_stated: usize) {
    use crate::code_rule_dsl::logic::program_facts::Field;
    for rel in b.declared_names() {
        let cols = b.columns_of(&rel).unwrap_or_default();
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
        a.declare(&rel, &pairs);
    }
    let more = b.nullary();
    if !more.is_empty() {
        let mut all = a.nullary();
        all.extend(more);
        a.set_field(Field::Nullary(&all));
    }
    let more = b.lattice();
    if !more.is_empty() {
        let mut all = a.lattice();
        all.extend(more);
        a.set_field(Field::Lattice(&all));
    }
    for rel in b.fact_names() {
        let rows = b.fact_rows(&rel);
        if rows.is_empty() {
            if !a.states_facts(&rel) {
                a.set_facts(&rel, &[]);
            }
        } else {
            a.add_facts(&rel, &rows);
        }
    }
    let mut inputs = a.input_names();
    let was = inputs.len();
    for (k, rel) in b.input_names().into_iter().enumerate() {
        if inputs.contains(&rel) {
            continue;
        }
        if k < b_stated {
            inputs.insert(*stated, rel);
            *stated += 1;
        } else {
            inputs.push(rel);
        }
    }
    if inputs.len() != was {
        a.set_inputs(&inputs);
    }
    for rel in b.output_names() {
        a.add_output(&rel);
    }
    let more = b.demand();
    if !more.is_empty() {
        let mut all = a.demand();
        let n = all.len();
        for rel in more {
            if !all.contains(&rel) {
                all.push(rel);
            }
        }
        if all.len() != n {
            a.set_field(Field::Demand(&all));
        }
    }
    let more = b.kindsets();
    if !more.is_empty() {
        let mut all = a.kindsets();
        all.extend(more);
        a.set_field(Field::Kindsets(&all));
    }
    let more = b.demand_dropped();
    if !more.is_empty() {
        let mut all = a.demand_dropped();
        all.extend(more);
        a.set_field(Field::DemandDropped(&all));
    }
    let more = b.gates();
    if !more.is_empty() {
        let mut all = a.gates();
        all.extend(more);
        a.set_field(Field::Gates(&all));
    }
    let more = b.policies();
    if !more.is_empty() {
        let mut all = a.policies();
        all.extend(more);
        a.set_field(Field::Policies(&all));
    }
    let more = b.settings();
    if !more.is_empty() {
        let mut all = a.settings();
        all.extend(more);
        a.set_field(Field::Settings(&all));
    }
    let more = b.natives();
    if !more.is_empty() {
        let mut all = a.natives();
        all.extend(more);
        a.set_field(Field::Natives(&all));
    }
    let more = b.rx_spans();
    if !more.is_empty() {
        let mut all = a.rx_spans();
        all.extend(more);
        a.set_field(Field::RxSpans(&all));
    }
    let more = b.ast_patterns();
    if !more.is_empty() {
        let mut all = a.ast_patterns();
        all.extend(more);
        a.set_field(Field::AstPatterns(&all));
    }
    let more = b.answers();
    if !more.is_empty() {
        let mut all = a.answers();
        all.extend(more);
        a.set_field(Field::Answers(&all));
    }
    let more = b.qualifiers();
    if !more.is_empty() {
        let mut all = a.qualifiers();
        for (rel, q) in more {
            all.entry(rel).or_default().extend(q);
        }
        a.set_field(Field::Qualifiers(&all));
    }
    let more = b.tapes();
    if !more.is_empty() {
        let mut all = a.tapes();
        for (rel, cols) in more {
            all.entry(rel).or_default().extend(cols);
        }
        a.set_field(Field::Tapes(&all));
    }
    let more = b.shapes();
    if !more.is_empty() {
        let mut all = a.shapes();
        all.extend(more);
        a.set_field(Field::Shapes(&all));
    }
    // Its relations' layouts (decision 0361).
    let more = b.layouts();
    if !more.is_empty() {
        let mut all = a.layouts();
        for (rel, facts) in more {
            crate::code_rule_dsl::logic::front::add_layout(all.entry(rel).or_default(), facts);
        }
        a.set_field(Field::Layouts(&all));
    }
}

/// A name's place in a helper's columns: its text, a helper's (`rr_<stem><n>`) as its stem and number.
fn helper_order(v: &str) -> (&str, u64) {
    if let Some(rest) = v.strip_prefix(RESERVED) {
        let digits = rest.len() - rest.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        if digits > 0 && digits < rest.len() && let Ok(n) = rest[rest.len() - digits..].parse::<u64>() {
            return (&v[..v.len() - digits], n);
        }
    }
    (v, 0)
}

/// `rules`' helpers named as `rename` says (a module's, numbered when it is linked): relations, and the variables
/// `fresh` named too (`renamed_fields` renames a header the same).
fn rename_rules(rules: &mut [Rule], rename: &std::collections::HashMap<String, String>) {
    let to = |rel: &str| rename.get(rel).cloned().unwrap_or_else(|| rel.to_owned());
    fn term(t: &mut Term, rename: &std::collections::HashMap<String, String>) {
        match t {
            Term::Var(v) => {
                if let Some(r) = rename.get(v.as_str()) {
                    *v = r.clone().into();
                }
            }
            Term::Call(_, args) => args.make_mut().iter_mut().for_each(|a| term(a, rename)),
            _ => {}
        }
    }
    let atom = |a: &mut Atom| {
        if let Some(r) = rename.get(a.rel.as_str()) {
            a.rel = r.clone().into();
        }
        a.args.make_mut().iter_mut().for_each(|t| term(t, rename));
    };
    for rule in rules {
        atom(&mut rule.head);
        for l in rule.body.make_mut() {
            match l {
                Literal::Atom(a) => atom(a),
                Literal::Agg { atom: a, var, over, .. } => {
                    atom(a);
                    *var = to(var);
                    if let Some(o) = over {
                        *o = to(o);
                    }
                }
                Literal::Nearest { kindset, n, a } => {
                    *kindset = to(kindset);
                    *n = to(n);
                    *a = to(a);
                }
                Literal::Cmp { l, r, .. } => {
                    term(l, rename);
                    term(r, rename);
                }
                Literal::Ast { document, node, captures, .. } => {
                    *document = to(document);
                    *node = to(node);
                    for (_, v) in captures.iter_mut() {
                        *v = to(v);
                    }
                }
            }
        }
    }
}

/// Version `v`'s header stated on `b`, each relation named as `to` says (`relinked`'s renames, then merges), the
/// helpers merged into another (`into`'s, by their names after `rename`) declared no more: its declarations and their columns, facts, inputs,
/// outputs, and the fields that name relations (lattices, kind sets, qualifiers, tape columns, natives and what they
/// read, nullary relations, demands, shapes); its other `.about` rows as they are.
fn renamed_fields(b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>, v: &crate::code_rule_dsl::logic::program_image::Version, to: &dyn Fn(&str) -> Option<String>, rename: &std::collections::HashMap<String, String>, into: &std::collections::HashMap<&str, &str>) {
    use crate::code_rule_dsl::logic::program_facts::Field;
    use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut, ProgramFields as F};
    let to = |r: &str| to(r).unwrap_or_else(|| r.to_owned());
    b.set_about(&v.about_kept(&|r: &str| !r.starts_with("module_")));
    for rel in F::declared_names(v) {
        if into.contains_key(rename.get(&rel).map_or(rel.as_str(), String::as_str)) {
            continue;
        }
        let cols: Vec<(String, String)> = F::columns_of(v, &rel).unwrap_or_default().into_iter().map(|(c, t)| (to(&c), t)).collect();
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
        FieldsMut::declare(b, &to(&rel), &pairs);
    }
    for rel in F::fact_names(v) {
        b.set_facts(&to(&rel), &F::fact_rows(v, &rel));
    }
    let names = |l: Vec<String>| -> Vec<String> { l.iter().map(|r| to(r)).collect() };
    b.set_inputs(&names(F::input_names(v)));
    b.set_outputs(&names(F::output_names(v)));
    fn keyed<X>(m: BTreeMap<String, X>, to: &dyn Fn(&str) -> String) -> BTreeMap<String, X> {
        m.into_iter().map(|(k, x)| (to(&k), x)).collect()
    }
    b.set_field(Field::Lattice(&keyed(F::lattice(v), &to)));
    b.set_field(Field::Kindsets(&keyed(F::kindsets(v), &to)));
    b.set_field(Field::Qualifiers(&keyed(F::qualifiers(v), &to)));
    b.set_field(Field::Tapes(&keyed(F::tapes(v), &to)));
    let natives: BTreeMap<String, _> = F::natives(v).into_iter().map(|(k, mut n)| {
        n.rename_reads(&to);
        (to(&k), n)
    }).collect();
    b.set_field(Field::Natives(&natives));
    b.set_field(Field::Nullary(&F::nullary(v).iter().map(|r| to(r)).collect()));
    b.set_field(Field::Demand(&names(F::demand(v))));
    // Its layouts (decision 0361), each fact's columns' relation renamed too.
    let layouts: BTreeMap<String, _> = F::layouts(v)
        .into_iter()
        .map(|(rel, mut facts)| {
            facts.iter_mut().for_each(|f| f.rename(&to));
            (to(&rel), facts)
        })
        .collect();
    b.set_field(Field::Layouts(&layouts));
    let mut shapes = F::shapes(v);
    for shape in &mut shapes {
        shape.head = to(&shape.head);
        shape.lit = to(&shape.lit);
        if let crate::code_rule_dsl::logic::front::ShapeWait::After(a) = &mut shape.wait {
            *a = to(a);
        }
    }
    b.set_field(Field::Shapes(&shapes));
}

/// The scope's own declarations taken out of a module's program linked in it (`rr::link_seeded_version`), but for
/// `core`'s.
pub fn unseed(program: &mut (impl crate::code_rule_dsl::logic::program_image::fields::FieldsMut + ?Sized), seeds: &Seeds, core: &BTreeSet<String>) {
    use crate::code_rule_dsl::logic::program_facts::Field;
    let seeded = &seeds.seeded;
    let gone = |r: &str| !core.contains(r);
    for r in program.declared_names() {
        if seeded.decls.contains(&r) && gone(&r) {
            program.undeclare(&r);
        }
    }
    if program.input_names().iter().any(|r| seeded.inputs.contains(r) && gone(r)) {
        program.retain_inputs(&|r: &str| !(seeded.inputs.contains(r) && gone(r)));
    }
    let natives = program.natives();
    if natives.keys().any(|r| seeded.natives.contains(r) && gone(r)) {
        let kept: BTreeMap<_, _> = natives.into_iter().filter(|(r, _)| !(seeded.natives.contains(r) && gone(r))).collect();
        program.set_field(Field::Natives(&kept));
    }
    let lattice = program.lattice();
    if lattice.keys().any(|r| seeded.lattice.contains(r) && gone(r)) {
        let kept: BTreeMap<_, _> = lattice.into_iter().filter(|(r, _)| !(seeded.lattice.contains(r) && gone(r))).collect();
        program.set_field(Field::Lattice(&kept));
    }
    let tapes = program.tapes();
    if tapes.keys().any(|r| seeded.tapes.contains(r) && gone(r)) {
        let kept: BTreeMap<_, _> = tapes.into_iter().filter(|(r, _)| !(seeded.tapes.contains(r) && gone(r))).collect();
        program.set_field(Field::Tapes(&kept));
    }
    let qualifiers = program.qualifiers();
    if qualifiers.keys().any(|r| seeded.qualifiers.contains(r) && gone(r)) {
        let kept: BTreeMap<_, _> = qualifiers.into_iter().filter(|(r, _)| !(seeded.qualifiers.contains(r) && gone(r))).collect();
        program.set_field(Field::Qualifiers(&kept));
    }
}

/// `link_seeded` up to the program's analyses, apart: the scope's program (its seeds' header: no rules), the
/// module's own program linked into it (`Lowerer::link`), and what the lowerer knows of them (`Lowered`, its program
/// the scope's). `rr::link_seeded_version` makes the scope's version once (`Scoped`), appends the module's onto it
/// (`lowering_tail::AppendModule`), checks it is `declared`, and runs the analyses on it.
pub fn link_seeded_parts(m: Module, seeds: &Seeds) -> Result<Linked, Error> {
    let mut l = seeds.l.clone();
    l.violations.clear();
    let (own, own_stated) = l.link(m)?;
    let stated = l.stated;
    let lowered = whole_front(l)?;
    Ok(Linked { lowered, own, stated, own_stated })
}

/// `link_seeded_parts`'s parts.
pub struct Linked {
    pub lowered: Lowered,
    /// The module's own program, rules and header, as `Lowerer::link` leaves it.
    pub own: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    /// The scope's inputs stated first, and the module's (`append_fields`).
    pub stated: usize,
    pub own_stated: usize,
}

impl Seeds {
    /// The scope's program as a version: its seeds' header, which every module linked in it starts from
    /// (`link_seeded_parts`).
    pub fn version(&self) -> Result<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, Error> {
        let bytes = self.l.prog.clone().finish_bytes().map_err(Error::Syntax)?;
        crate::code_rule_dsl::logic::program_image::Version::open(&bytes).map_err(Error::Syntax)
    }
}

/// `whole` but the lowerer's analyses, with what the lowerer knows of the program
/// (`Lowered`): what `three_valued`, `findings`, `core_and_projections`, `span_patterns`, `grammar_names`, `builtins_out`, `kind_sets`,
/// `unit`, `whole_back`, `front::finish_checks_on` and the tail (aggregate demand, `unit`,
/// `sets`, `widen_cyclic_contexts`) make of it `rr::link_seeded_version` makes on the version.
fn whole_front(mut l: Lowerer) -> Result<Lowered, Error> {
    let analyses = crate::code_rule_dsl::logic::profile::span(|| "the program's analyses (findings, spans, kinds)".to_owned());
    let rewritten: Vec<String> = self::core::REWRITTEN.into_iter().filter(|r| l.rewritten(r)).map(str::to_owned).collect();
    drop(analyses);
    let findings = PendingFindings { violations: std::mem::take(&mut l.violations), next: l.next };
    Ok(Lowered { sets: l.sets, rewritten, docof_of_locs: l.docof_of_locs, findings, three: l.three, modal: l.modal })
}

/// What the lowerer knows of the program `whole_front` leaves, which the passes after read: the sets its
/// clauses named (`kind_sets`), and the core relations it rewrote, which it does not derive (`builtins_out`).
pub struct Lowered {
    pub sets: BTreeSet<String>,
    pub rewritten: Vec<String>,
    /// Whether the program's `docof` is core.rr's, over spans (`core_and_projections`).
    pub docof_of_locs: bool,
    /// The `.violation`s the clauses declared, for `findings`.
    pub findings: PendingFindings,
    /// The relations declared `must, may`, and the rules' `must`/`may` prefixes by index, for `three_valued`.
    pub three: BTreeSet<String>,
    pub modal: BTreeMap<usize, String>,
}

/// The `.violation`s a program's clauses declared, and the lowerer's count of fresh names, with which `findings` writes
/// their rules after the lowerer is gone.
#[derive(Clone, Default)]
pub struct PendingFindings {
    violations: Vec<Violation>,
    next: usize,
}

impl PendingFindings {
    /// Its bytes, for a pass's settings.
    pub fn key(&self) -> Vec<u8> {
        format!("{} {:?}", self.next, self.violations).into_bytes()
    }
}

/// What `findings` reads of a program's rules, which it does not see.
#[derive(Clone, Copy)]
pub struct RulesSeen {
    /// Whether a rule makes a `@place` value: only then can a span or loc `at` hold one.
    pub place_values: bool,
    /// Whether a rule reads `guest_policy`.
    pub reads_guest_policy: bool,
}

impl RulesSeen {
    pub fn of(rules: &[Rule]) -> Self {
        RulesSeen {
            place_values: rules.iter().any(|r| r.head.args.iter().chain(r.body.iter().flat_map(literal_terms)).any(has_place)),
            reads_guest_policy: rules.iter().flat_map(|r| &r.body).filter_map(Literal::atom).any(|a| a.rel == GUEST_POLICY),
        }
    }
}

/// A program that derives `violation` itself, in the older five columns, is read at column 0.
pub fn widen_violation_heads<'r>(heads: impl Iterator<Item = &'r mut Atom>) {
    for h in heads {
        if h.rel == "violation" && h.args.len() == 5 {
            h.args.push(Term::Int(0));
        }
    }
}

/// Whether a term makes a `@place` value (`RulesSeen`).
pub fn is_place_call(name: &str) -> bool {
    name == PLACE
}

/// `Program::rx_spans`: each `rx_span(p, c, …)` a rule reads, its pattern p a constant and its span c bound in
/// the same body by `source`, `src_line`, `kind(c, K)` (a comment, or a node of kind K) or another `rx_span`'s
/// group (a capture; decision 0081). Read off the rules, so a printed program (`rr::print`) has them too.
/// A derived relation's column holds the spans its rules bind it to: each rule binds it so, directly or through
/// another such column (a fixpoint); rules that bind it differently give it each (string contents and comments).
pub fn span_patterns(p: &SpanProgram<'_>) -> Result<SpanPatterns, Error> {
    use SpanTerm as Term;
    // The relations by number, and each column's spans a vector per relation.
    let mut id: rustc_hash::FxHashMap<&str, u32> = Default::default();
    {
        let mut number = |rel| {
            let n = id.len() as u32;
            id.entry(rel).or_insert(n);
        };
        for r in &p.rules {
            number(r.head.rel);
            r.body.iter().for_each(|a| number(a.rel));
        }
        p.decls.iter().for_each(|(r, _)| number(r));
    }
    let rels = id.len();
    let of = |rel: &str| id.get(rel).copied();
    /// Each relation's columns' span sets, by number.
    struct Cols(Vec<Vec<Option<BTreeSet<SpanIn>>>>);
    impl Cols {
        fn get(&self, rel: Option<u32>, c: usize) -> Option<&BTreeSet<SpanIn>> {
            self.0.get(rel? as usize)?.get(c)?.as_ref()
        }
        fn has(&self, rel: u32, c: usize) -> bool {
            self.get(Some(rel), c).is_some()
        }
        fn insert(&mut self, rel: u32, c: usize, set: BTreeSet<SpanIn>) {
            let cols = &mut self.0[rel as usize];
            if cols.len() <= c {
                cols.resize(c + 1, None);
            }
            cols[c] = Some(set);
        }
    }
    let mut columns = Cols(vec![Vec::new(); rels]);
    let mut rounds = 0;
    let binder = |body: &[SpanAtom<'_>], x: &str, columns: &Cols| -> Option<BTreeSet<SpanIn>> {
        body.iter().filter(|b| !b.neg).find_map(|b| match (b.rel, b.args.as_slice()) {
            ("source", [_, Term::Var(y)]) if *y == x => Some([SpanIn::Source].into()),
            ("src_line", [_, _, Term::Var(y)]) if *y == x => Some([SpanIn::Line].into()),
            ("kind", [Term::Var(y), Term::Str(k)]) if *y == x && *k == "comment" => Some([SpanIn::Comment].into()),
            ("kind", [Term::Var(y), Term::Str(k)]) if *y == x => Some([SpanIn::Kind(k.to_string())].into()),
            ("rx_span", [Term::Str(p), _, _, Term::Str(g), Term::Var(y)]) if *y == x => Some([SpanIn::Capture(p.to_string(), g.to_string())].into()),
            // A child of a node whose kinds the body gives (a `kind` atom, or a column of nodes of known kinds):
            // the children of nodes of those kinds.
            ("child", [Term::Var(par), Term::Var(y), _]) if *y == x && *par != x => {
                let atoms: BTreeSet<SpanIn> = body.iter().filter(|a| !a.neg && a.rel == "kind").filter_map(|a| match a.args.as_slice() {
                    [Term::Var(z), Term::Str(k)] if z == par => Some(SpanIn::ChildOf(k.to_string())),
                    _ => None,
                }).collect();
                if !atoms.is_empty() {
                    return Some(atoms);
                }
                body.iter().filter(|a| !a.neg && a.rel != "child").find_map(|a| {
                    a.args.iter().enumerate().find_map(|(i, t)| matches!(t, Term::Var(z) if z == par).then(|| columns.get(of(a.rel), i)).flatten())
                }).and_then(|set| set.iter().map(|s| match s { SpanIn::Kind(k) => Some(SpanIn::ChildOf(k.to_string())), _ => None }).collect::<Option<BTreeSet<_>>>())
            }
            _ => b.args.iter().enumerate().find_map(|(i, t)| matches!(t, Term::Var(y) if *y == x).then(|| columns.get(of(b.rel), i).cloned()).flatten()),
        })
    };
    // A declared relation no rule derives, and that is not read in, has no rows: its columns hold no nodes, and a
    // scan over them is empty. A library a pack's rules include (`…confusables.rr`) is loaded on its own too.
    {
        let heads: rustc_hash::FxHashSet<&str> = p.rules.iter().map(|r| r.head.rel).collect();
        let inputs: rustc_hash::FxHashSet<&str> = p.inputs.iter().copied().collect();
        for &(rel, cols) in &p.decls {
            if !heads.contains(rel) && !inputs.contains(rel) && !p.natives.contains(rel) {
                let r = of(rel).expect("a declared relation's number");
                for i in 0..cols {
                    columns.insert(r, i, BTreeSet::new());
                }
            }
        }
    }
    // Each derived relation's rules, in order: one pass, not one over every rule per relation.
    let mut by_head: Vec<Vec<&SpanRule<'_>>> = vec![Vec::new(); rels];
    let mut heads: Vec<u32> = Vec::new();
    for r in &p.rules {
        let h = of(r.head.rel).expect("a head's number");
        if by_head[h as usize].is_empty() {
            heads.push(h);
        }
        by_head[h as usize].push(r);
    }
    // The relations whose rules read each relation (positive atoms, what `binder` reads): a round derives again
    // only the readers of the columns the round before changed, the rest being what they were.
    let mut readers: Vec<Vec<u32>> = vec![Vec::new(); rels];
    for r in &p.rules {
        let h = of(r.head.rel).expect("a head's number");
        for a in r.body.iter().filter(|a| !a.neg) {
            readers[of(a.rel).expect("a read's number") as usize].push(h);
        }
    }
    for rs in &mut readers {
        rs.sort_unstable();
        rs.dedup();
    }
    let mut dirty: Vec<u32> = heads.clone();
    loop {
        let mut found: Vec<(u32, usize, BTreeSet<SpanIn>)> = Vec::new();
        for &rel in &dirty {
            let rules = &by_head[rel as usize];
            for i in 0..rules[0].head.args.len() {
                let each: Option<Vec<BTreeSet<SpanIn>>> = rules.iter().map(|r| match r.head.args.get(i) {
                    Some(Term::Var(x)) => binder(&r.body, x, &columns),
                    _ => None,
                }).collect();
                if let Some(each) = each {
                    let all: BTreeSet<SpanIn> = each.into_iter().flatten().collect();
                    if columns.get(Some(rel), i) != Some(&all) {
                        found.push((rel, i, all));
                    }
                }
            }
        }
        rounds += 1;
        // Sets only grow, and are bounded by the program's patterns and kinds; the bound is a guard.
        if found.is_empty() || rounds > 64 {
            break;
        }
        let mut next: Vec<u32> = found.iter().flat_map(|(r, _, _)| readers[*r as usize].iter().copied()).collect();
        next.sort_unstable();
        next.dedup();
        dirty = next;
        for (r, i, set) in found {
            columns.insert(r, i, set);
        }
    }
    // Top-down: a helper's column its own rules do not bind to spans, where every rule that reads the helper binds
    // that argument so (`binder`, or through its own head's such column), holds those spans. The helper derives
    // rows for other nodes too, but none of its readers joins them, so testing only the spans its readers bind
    // gives every reader the same rows. Not for a column the run outputs, which no reader bounds.
    let mut demand = Cols(vec![Vec::new(); rels]);
    let outputs: rustc_hash::FxHashSet<&str> = p.outputs.iter().copied().collect();
    let mut helpers: Vec<u32> = heads.iter().copied().filter(|&h| !outputs.contains(by_head[h as usize][0].head.rel)).collect();
    helpers.sort_unstable();
    // Each relation's reading atoms with their rules, in the rules' and bodies' order: a column's readers, not a
    // scan of every rule for each column and round.
    let mut sites_of: Vec<Vec<(&SpanRule<'_>, &SpanAtom<'_>)>> = vec![Vec::new(); rels];
    for r in &p.rules {
        for a in &r.body {
            sites_of[of(a.rel).expect("a read's number") as usize].push((r, a));
        }
    }
    let mut rounds = 0;
    loop {
        let mut found: Vec<(u32, usize, BTreeSet<SpanIn>)> = Vec::new();
        for &rel in &helpers {
            for i in 0..by_head[rel as usize][0].head.args.len() {
                if columns.has(rel, i) {
                    continue;
                }
                let mut all = BTreeSet::new();
                let mut sites = 0;
                let mut bound = true;
                for &(r, a) in &sites_of[rel as usize] {
                    sites += 1;
                    let got = match a.args.get(i) {
                        Some(Term::Var(y)) => binder(&r.body, y, &columns).or_else(|| {
                            r.head.args.iter().position(|t| matches!(t, Term::Var(h) if h == y)).and_then(|j| demand.get(of(r.head.rel), j).cloned())
                        }),
                        _ => None,
                    };
                    match got {
                        Some(s) => all.extend(s),
                        None => bound = false,
                    }
                }
                if bound && sites > 0 && demand.get(Some(rel), i) != Some(&all) {
                    found.push((rel, i, all));
                }
            }
        }
        rounds += 1;
        if found.is_empty() || rounds > 64 {
            break;
        }
        for (r, i, set) in found {
            demand.insert(r, i, set);
        }
    }
    let mut out = SpanPatterns::default();
    for rule in &p.rules {
        for a in rule.body.iter().filter(|a| a.rel == "rx_span") {
            let (Some(Term::Str(p)), Some(Term::Var(c))) = (a.args.first(), a.args.get(1)) else {
                return Err(Error::Syntax(format!("{}: rx_span reads a constant pattern over a span variable", rule.head.rel)));
            };
            let from_readers = || rule.head.args.iter().position(|t| matches!(t, Term::Var(h) if h == c)).and_then(|j| demand.get(of(rule.head.rel), j).cloned());
            let Some(within) = binder(&rule.body, c, &columns).or_else(from_readers) else {
                                    return not_yet(format!("{}: match_all over ${c}, a node or span not bound by `source`, `src_line`, `kind` or a capture", rule.head.rel));
            };
            for w in within {
                out.0.insert((w, p.to_string()));
            }
        }
        // The AST patterns, read off the rules too.
        for a in rule.body.iter().filter(|a| a.rel == "ast_match") {
            if let Some(Term::Str(p)) = a.args.first() {
                out.1.insert(p.to_string());
            }
        }
    }
    Ok(out)
}

/// `core.rr`'s `kind(n, k)` is a projection of `node`, and so is `docof(n, d)` as T's programs declare it (over
/// nodes): rules, which the planner reads from `node` in place, not inputs, where a rule reads them (the prelude
/// declares both for every program). `core.rr`'s `docof(x: loc, d)` stays an input, for spans have a document
/// too. (`precedes` is core.rr's tree arithmetic, `core::rewrite`.)
/// The rules to add go first, each where `projection_at` puts it; `read` is what the rules read (`reads`), `derives_node`
/// whether a rule derives `node`. `prog` may be a header alone (a version's).
pub fn projections(prog: &mut (impl crate::code_rule_dsl::logic::program_image::fields::FieldsMut + ?Sized), read: &BTreeSet<(String, usize)>, docof_of_locs: bool, derives_node: bool) -> Vec<Rule> {
    let v = |s: &str| Term::Var(s.to_owned().into());
    let mut made = Vec::new();
    for (rel, args) in [("kind", [Term::Wild, v("n"), v("k"), Term::Wild, Term::Wild]), ("docof", [v("d"), v("n"), Term::Wild, Term::Wild, Term::Wild])] {
        if !read.iter().any(|(r, _)| r == rel) || !prog.input_names().iter().any(|i| i == rel) || (rel == "docof" && (docof_of_locs || !prog.declares("node"))) {
            continue;
        }
        // `kind` is always `node`'s column: a program that reads it reads `node`.
        base_relation_derived(prog, "node", derives_node);
        prog.retain_inputs(&|i| i != rel);
        let head = if rel == "kind" { vec![v("n"), v("k")] } else { vec![v("n"), v("d")] };
        made.push(Rule {
            head: Atom { rel: rel.to_owned().into(), args: head.into(), neg: false, gated: false },
            body: [Literal::Atom(Atom { rel: "node".into(), args: args.to_vec().into(), neg: false, gated: false })].into(),
        });
    }
    made
}

/// Where `projections` puts its rule of `rel`, given the program's first rule's head: first, kind before docof (where
/// T's programs wrote them).
pub fn projection_at(rel: &str, first_head: Option<&str>) -> usize {
    usize::from(rel == "docof" && first_head == Some("kind"))
}

/// The three-valued relations: those declared so, and every relation a rule of which reads one (`rules`: each rule's
/// head and the relations its body's atoms read), to a fixed point.
pub fn three_closure(declared: &BTreeSet<String>, rules: &[(&str, Vec<&str>)]) -> BTreeSet<String> {
    let mut three = declared.clone();
    loop {
        let before = three.len();
        for (head, reads) in rules {
            if reads.iter().any(|r| three.contains(*r)) {
                three.insert((*head).to_owned());
            }
        }
        if three.len() == before {
            return three;
        }
    }
}

/// A three-valued relation's must side.
pub fn must_name(rel: &str) -> String {
    format!("{RESERVED}must_{rel}")
}

/// Whether an atom of `rel` (negated: `neg`) in a rule of the must side (`must_side`) or the may side reads `rel`'s
/// must side: a positive atom on the must side, a negated one on the may side.
pub fn reads_must(three: &BTreeSet<String>, rel: &str, neg: bool, must_side: bool) -> bool {
    three.contains(rel) && neg != must_side
}

/// Each three-valued relation's must side declared, and given its facts, as the relation is.
pub fn three_valued_header(prog: &mut (impl crate::code_rule_dsl::logic::program_image::fields::FieldsMut + ?Sized), three: &BTreeSet<String>) {
    for rel in three {
        if let Some(cols) = prog.columns_of(rel) {
            let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            prog.declare(&must_name(rel), &pairs);
        }
        if prog.states_facts(rel) {
            let rows = prog.fact_rows(rel);
            prog.set_facts(&must_name(rel), &rows);
        }
    }
}

/// What `span_patterns` finds: the rx spans and the AST patterns.
#[derive(Default)]
pub struct SpanPatterns(pub BTreeSet<(SpanIn, String)>, pub BTreeSet<String>);

/// A term as `span_patterns` reads it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpanTerm<'a> {
    Var(&'a str),
    Str(&'a str),
    Other,
}

/// An atom as `span_patterns` reads it.
pub struct SpanAtom<'a> {
    pub rel: &'a str,
    pub args: Vec<SpanTerm<'a>>,
    pub neg: bool,
}

/// A rule as `span_patterns` reads it: its head, and its body's atoms (an aggregate's among them).
pub struct SpanRule<'a> {
    pub head: SpanAtom<'a>,
    pub body: Vec<SpanAtom<'a>>,
}

/// A program as `span_patterns` reads it, from a version.
pub struct SpanProgram<'a> {
    pub rules: Vec<SpanRule<'a>>,
    /// Each declared relation's name and arity.
    pub decls: Vec<(&'a str, usize)>,
    pub inputs: Vec<&'a str>,
    pub natives: BTreeSet<&'a str>,
    pub outputs: Vec<&'a str>,
}

/// The column of a relation that names a grammar (`grammar_names`).
pub fn grammar_column(rel: &str) -> Option<usize> {
    match rel {
        "embed" | "embed_value" => Some(2),
        "parsed" | "sub_tape" => Some(1),
        _ => None,
    }
}

/// `grammar_names`'s error: rule `head` names grammar `g`, which the registry does not have.
pub fn no_grammar(head: &str, g: &str) -> Error {
    Error::Syntax(format!("{head}: no grammar {g:?}; a grammar is a language registry dialect's label or fragment (languages/*.json)"))
}

/// The relations `builtins_out` takes out, `within` among them.
pub fn builtins(rewritten: &[String]) -> impl Iterator<Item = &str> {
    RELATION_ARG.iter().map(|(r, _)| *r).chain(["within", "earlier", "split_dot", "unset"]).chain(rewritten.iter().map(String::as_str))
}





/// A new name of `stem`, none `declared` holds, its number after `next`, recorded in `made`.
fn fresh_name(next: &mut usize, made: &mut Vec<(String, usize)>, declared: impl Fn(&str) -> bool, stem: &str) -> String {
    loop {
        *next += 1;
        let name = format!("rr_{stem}{next}");
        if !declared(name.as_str()) {
            made.push((stem.to_owned(), *next));
            return name;
        }
    }
}

impl Lowerer {
    /// A new name, none the program declares: a printed program (`rr::print`) declares the helpers it was lowered
    /// with, and lowering it again makes more.
    fn fresh(&mut self, stem: &str) -> String {
        if let Some(at) = self.replay.as_mut() {
            let Some(Logged::Name(name)) = self.log.get(*at).cloned() else { panic!("a replayed walk makes the names its deciding walk made") };
            *at += 1;
            return name;
        }
        let prog = &self.prog;
        let name = fresh_name(&mut self.next, &mut self.made, |n| prog.declares(n), stem);
        self.log.push(Logged::Name(name.clone()));
        name
    }

    /// A rule the walk made, written onto its header (`out`); nothing in a replay, whose deciding walk wrote it.
    fn write(&mut self, rule: Rule) {
        if self.replay.is_none() {
            self.out.write(&mut self.prog, &rule, false);
        }
    }

    /// `rule`, unless the walk wrote it already: a demand is written once.
    fn write_once(&mut self, rule: Rule) {
        if self.replay.is_none() {
            self.out.write(&mut self.prog, &rule, true);
        }
    }

    /// How many rules the walk has written.
    fn written(&self) -> usize {
        self.out.n
    }

    /// Opens a rule of the walk: head `rel(args)`, its body's `unset(x)` binding `unset` empty (`unset_vars`); its
    /// literals are `emit`'s, then `end_rule`.
    fn begin_rule(&mut self, rel: &str, mut args: Vec<Term>, unset: BTreeSet<String>) {
        args.iter_mut().for_each(|t| unset_term(t, &unset));
        let mut o = self.prog.open_rule();
        self.prog.put_head(&mut o, rel, &ts(&args));
        self.open.0 = Some(Sink::Rule(o, unset, None));
    }

    /// Literal `l` of the open rule (`begin_rule`), when one is: its `unset` applied, and a negated test merged with
    /// the one before it.
    fn emit(&mut self, l: Literal) -> Result<(), Error> {
        if self.off {
            return Ok(());
        }
        let (unset, held) = match self.open.0.as_mut() {
            None => return Ok(()),
            Some(Sink::Trees(lits)) => {
                lits.push(l);
                return Ok(());
            }
            Some(Sink::Rule(_, unset, held)) => (unset, held),
        };
        let is_unset = matches!(&l, Literal::Atom(a) if a.rel == "unset" && !a.neg);
        let Some(l) = unset_literal(l, unset)? else {
            // `unset(x)` goes; the tests either side of it are not adjacent.
            if is_unset && let Some(h) = held.take() {
                self.put_open(&h)?;
            }
            return Ok(());
        };
        let l = match held.as_mut() {
            Some(h) => match merge_negated_test(h, l) {
                None => return Ok(()),
                Some(l) => {
                    let h = held.take().expect("held");
                    self.put_open(&h)?;
                    l
                }
            },
            None => l,
        };
        if negated_test(&l).is_some() {
            if let Some(Sink::Rule(_, _, held)) = self.open.0.as_mut() {
                *held = Some(l);
            }
            return Ok(());
        }
        self.put_open(&l)
    }

    fn put_open(&mut self, l: &Literal) -> Result<(), Error> {
        let Some(Sink::Rule(o, ..)) = self.open.0.as_mut() else { unreachable!("a rule open") };
        put_literal(&mut self.prog, o, l).map_err(Error::Syntax)
    }

    /// Ends the open rule (`begin_rule`).
    fn end_rule(&mut self) -> Result<(), Error> {
        let held = match self.open.0.as_mut() {
            Some(Sink::Rule(_, _, h)) => h.take(),
            _ => unreachable!("a rule open"),
        };
        if let Some(h) = held
            && let Err(e) = self.put_open(&h)
        {
            self.drop_rule();
            return Err(e);
        }
        let Some(Sink::Rule(o, ..)) = self.open.0.take() else { unreachable!("a rule open") };
        self.close(o);
        Ok(())
    }

    /// Leaves the open rule out (an error while it was written).
    fn drop_rule(&mut self) {
        if let Some(Sink::Rule(o, ..)) = self.open.0.take() {
            self.prog.abandon_rule(o);
        }
    }

    /// Runs alternative `i` of a lowering told to write alternative `pick`: its literals are written (`emit`) only
    /// when it is the one; every alternative runs, so the names each makes replay in order.
    fn alt(&mut self, i: usize, pick: usize, f: impl FnOnce(&mut Self) -> Result<(), Error>) -> Result<(), Error> {
        let off = self.off;
        self.off = off || i != pick;
        let r = f(self);
        self.off = off;
        r
    }

    /// Where the walk is in its log: the next name a replay reads, or the next one the deciding walk makes.
    fn position(&self) -> usize {
        self.replay.unwrap_or(self.log.len())
    }

    /// The alternatives of a lowering that writes the alternative it is told to (`f(me, pick)`, its count of
    /// alternatives), as bodies: it runs once to decide, writing none, then once per alternative into a sink of
    /// literals, replaying the names its first run made. A bridge for the helpers that still read whole bodies
    /// (decision 0276 A3.4).
    fn alts_of(&mut self, mut f: impl FnMut(&mut Self, usize) -> Result<usize, Error>) -> Result<Vec<Vec<Literal>>, Error> {
        let start = self.position();
        let replaying = self.replay.is_some();
        let sink = self.open.0.take();
        let off = std::mem::replace(&mut self.off, true);
        let mut run = |me: &mut Self| -> Result<Vec<Vec<Literal>>, Error> {
            let n = f(me, usize::MAX)?;
            let end = me.position();
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                me.replay = Some(start);
                me.off = false;
                me.open.0 = Some(Sink::Trees(Vec::new()));
                let r = f(me, i);
                let Some(Sink::Trees(lits)) = me.open.0.take() else { unreachable!("the sink of literals") };
                r?;
                debug_assert_eq!(me.replay, Some(end), "an alternative replays its lowering's names");
                out.push(lits);
            }
            me.replay = replaying.then_some(end);
            Ok(out)
        };
        let out = run(self);
        if out.is_err() {
            // An error ends the clause; where its log stands no longer matters.
            self.replay = if replaying { self.replay } else { None };
        }
        self.open.0 = sink;
        self.off = off;
        out
    }

    /// Bodies `alts`, alternatives of a lowering that still makes them whole: the one picked written (`alt`). Their
    /// count. (A bridge, decision 0276 A3.4.)
    fn put_alts(&mut self, alts: Vec<Vec<Literal>>, pick: usize) -> Result<usize, Error> {
        let n = alts.len();
        for (i, body) in alts.into_iter().enumerate() {
            self.alt(i, pick, |me| body.into_iter().try_for_each(|l| me.emit(l)))?;
        }
        Ok(n)
    }

    /// The literals `f` writes, replaying the log from `start` into a sink of literals: one body. (A bridge,
    /// decision 0276 A3.4.)
    fn body_of<R>(&mut self, start: usize, f: impl FnOnce(&mut Self) -> Result<R, Error>) -> Result<(R, Vec<Literal>), Error> {
        let replay = std::mem::replace(&mut self.replay, Some(start));
        let sink = self.open.0.replace(Sink::Trees(Vec::new()));
        let off = std::mem::replace(&mut self.off, false);
        let r = f(self);
        let Some(Sink::Trees(lits)) = std::mem::replace(&mut self.open.0, sink) else { unreachable!("the sink of literals") };
        self.off = off;
        self.replay = replay;
        Ok((r?, lits))
    }

    /// Ends rule `o`, open on the header (`Builder::open_rule`), as one the walk wrote; left out in a replay.
    fn close(&mut self, o: crate::code_rule_dsl::logic::program_image::Open) {
        if self.replay.is_some() {
            self.prog.abandon_rule(o);
        } else {
            self.out.close(&mut self.prog, o, false);
        }
    }

    /// A helper relation named `name` (a `fresh` one of `stem`), with these columns and rules; or, when one made
    /// before has the same stem, columns and rules, that relation. Each occurrence made its own: the two contexts of
    /// a rule's two aggregates over the same variables were `rr_ctx7` and `rr_ctx8` in most of dl1's 44 policies, and
    /// `congruence` merged them back after lowering (decision 0225 §6). An aggregate's helper, which demand narrows
    /// by its readers, has its readers' heads in its stem (`aggregate`).
    fn helper_once(&mut self, name: String, stem: &str, cols: Vec<(String, String)>, rules: Vec<(Vec<Term>, Vec<Literal>)>) -> String {
        let rules: Vec<Rule> = rules.into_iter().map(|(args, body)| Rule { head: Atom { rel: name.clone().into(), args: args.into(), neg: false, gated: false }, body: body.into() }).collect();
        let names = self.template_names.get_or_insert_with(crate::code_rule_dsl::logic::front::template::CanonicalNames::new);
        let key = (stem.to_owned(), cols.iter().map(|(_, t)| t.clone()).collect(), rules.iter().map(|r| crate::code_rule_dsl::logic::front::template::template(r, names)).collect());
        if let Some(made) = self.helpers.get(&key) {
            return made.clone();
        }
        self.prog.declare_cols(&name, cols);
        self.once.push((name.clone(), stem.to_owned(), rules.len()));
        for rule in rules {
            self.write(rule);
        }
        self.helpers.insert(key, name.clone());
        name
    }

    /// The key `helper_once` made helper `name` under (stem `stem`, its first `k` rules), from `prog` as it is now:
    /// its relations' names as linking renamed them.
    /// Helper `name`'s key: its stem, its column types `cols`, and the templates of its first `k` rules in `rules`.
    fn once_key(&mut self, cols: Vec<String>, rules: &[Rule], name: &str, stem: &str, k: usize) -> HelperKey {
        let names = self.template_names.get_or_insert_with(crate::code_rule_dsl::logic::front::template::CanonicalNames::new);
        let rules = rules.iter().filter(|r| r.head.rel == name).take(k).map(|r| crate::code_rule_dsl::logic::front::template::template(r, names)).collect();
        (stem.to_owned(), cols, rules)
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
                // Open to the rules and facts of the files that include its own (decision 0263): checked on reading.
                "extensible" => {}
                "widen" => widen = Some(words.next().and_then(|k| k.parse::<u32>().ok()).ok_or_else(|| Error::Syntax(format!("{name}: `widen` needs a count")))?),
                // What the relation keeps (decision 0099): an override of the planner's choice.
                q if crate::code_rule_dsl::logic::front::QUALIFIERS.contains(&q) => {
                    let mut own = BTreeSet::new();
                    self.prog.edit(|b| b.qualifiers(), |q| Field::Qualifiers(q), |all| {
                        let mine = all.entry(name.clone()).or_default();
                        mine.insert(q.to_owned());
                        own = mine.clone();
                    });
                    for (a, b) in [("inline", "no_inline"), ("magic", "no_magic"), ("inline", "magic"), ("reach", "no_reach")] {
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
                self.prog.edit(|b| b.tapes(), |t| Field::Tapes(t), |t| {
                    t.entry(name.clone()).or_default().insert(i);
                });
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
            let decl = LatticeDecl { keys: i, ty: column.clone(), merge, widen };
            self.prog.edit(|b| b.lattice(), |d| Field::Lattice(d), |d| {
                d.insert(name.clone(), decl);
            });
            out.push((c.clone(), column));
        }
        if widen.is_some() && !self.prog.lattice().contains_key(&name) {
            return Err(Error::Syntax(format!("{name}: `widen` on a relation with no lattice column")));
        }
        self.prog.declare_cols(&name, out);
        Ok(())
    }

    /// `.violation V(at, …)`: V is an output relation, and each of its rows is a finding (`findings`). Its
    /// `suppress` and `fix` lines and `code {c} in "re"` are settings of the findings layer, as `.mode` is:
    /// `suppress V tool "rule", … [over {c}]`, `fix V [safe] F "title"` and `code_in V "re"`.
    #[allow(clippy::too_many_arguments)]
    fn violation(&mut self, t: &super::from_tape::Tape<'_>, s: u32) -> Result<(), Error> {
        use super::tape::{Field as F, Kind as K};
        let (name, cols, message) = t.violation(s)?;
        let Some((_, at)) = cols.first() else { return Err(Error::Syntax(format!(".violation {name} has no `at` column"))) };
        if !matches!(at.as_str(), "node" | "span" | "loc") {
            return Err(Error::Syntax(format!(".violation {name}: `at` is a node, a span or a loc, not a {at}")));
        }
        let node = at == "node";
        let column = |c: &str| cols.iter().position(|(n, _)| n == c).ok_or_else(|| Error::Syntax(format!(".violation {name}: no column {c}")));
        let tag = match t.all(s, F::Tag).last() {
            Some(x) => Some(column(&t.brace_column(t.one(x, F::Column)?))?),
            None => None,
        };
        // `code {column} in "re"` or `code "C"`: the last one written.
        let (mut code, mut code_in) = (FindingCode::Name, None);
        for c in t.all(s, F::Code) {
            if t.is(c, K::CodeColumn) {
                code = FindingCode::Column(column(&t.brace_column(t.one(c, F::Column)?))?);
                code_in = t.get(c, F::Codes).map(|x| t.string(x)).transpose()?;
            } else {
                code = FindingCode::Constant(t.string(c)?);
            }
        }
        for l in t.all(s, F::Suppress) {
            let rules = t.all(l, F::Rule).map(|r| t.string(r).map(|r| quote(&r))).collect::<Result<Vec<_>, _>>()?.join(", ");
            let over = t.get(l, F::Over).map(|o| format!(" over {{{}}}", t.brace_column(o))).unwrap_or_default();
            self.prog.push_setting("suppress".to_owned(), format!("{name} {} {rules}{over}", t.name(l, F::Tool)?));
        }
        for f in t.all(s, F::Fix) {
            let safe = if t.has_token(f, "safe") { "safe " } else { "" };
            self.prog.push_setting("fix".to_owned(), format!("{name} {safe}{} {}", t.name(f, F::Relation)?, quote(&t.string(t.one(f, F::Description)?)?)));
        }
        if let Some(re) = code_in {
            self.prog.push_setting("code_in".to_owned(), format!("{name} {}", quote(&re)));
        }
        self.violations.push(Violation { name: name.clone(), arity: cols.len(), code, node, places: false, columns: cols.clone(), message, tag });
        self.decl(name.clone(), cols, &[])?;
        self.prog.add_output(&name);
        Ok(())
    }

    /// A clause of constant heads: its facts.
    fn facts_at(&self, heads: &[HeadAt]) -> Option<Vec<(String, Vec<Value>)>> {
        heads
            .iter()
            .map(|h| {
                let row = h
                    .args
                    .iter()
                    .map(|a| match self.tv(a) {
                        T::Num(n) => Some(Value::Int(n)),
                        T::Str(s) => Some(Value::Str(s.into())),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()?;
                h.modal.is_none().then(|| (h.rel.clone(), row))
            })
            .collect()
    }

    /// Clause `st` (no edits), its heads and body read off its tape: each rule written as it is made.
    fn clause(&mut self, st: &Stmt) -> Result<(), Error> {
        fn binds_top(me: &Lowerer, lits: &[LitAt]) -> bool {
            lits.iter().any(|l| match me.lit(l) {
                L::Atom { neg: false, args, .. } => args.iter().any(|a| matches!(me.tv(a), T::Var(v) if v == "top")),
                L::Group { alts, neg: false } => alts.iter().any(|a| binds_top(me, a)),
                _ => false,
            })
        }
        self.start_clause(st.file().clone());
        let c = st.node();
        let heads = self.heads_at(c);
        let body: Vec<LitAt> = self.tape().get(c, super::tape::Field::Body).map_or_else(Vec::new, |b| self.body_at(b, 0));
        if self.native_definition(&heads, &body) {
            return Ok(());
        }
        self.top_is_var = binds_top(self, &body);
        self.heads = heads.iter().map(|h| h.rel.clone()).collect();
        self.head_vars = {
            let mut v = Vec::new();
            heads.iter().flat_map(|h| &h.args).for_each(|a| self.term_vars(a, &mut v));
            v.into_iter().collect()
        };
        self.first_atom = match body.first().map(|l| self.lit(l)) {
            Some(L::Atom { rel, neg: false, .. }) => Some(rel),
            _ => None,
        };
        // The deciding walk (decision 0276 A3.3): each literal's count of alternatives. Each combination is written by
        // a walk that replays it and takes one alternative of each literal.
        self.log.clear();
        self.replay = None;
        let (heads, combinations) = self.clause_put(heads, body.clone(), usize::MAX)?;
        for head in &heads {
            if let Some(m) = &head.modal
                && !self.three.contains(&head.rel)
            {
                return Err(Error::Syntax(format!("`{m}` head {}: the relation is not declared `must, may`", head.rel)));
            }
            // `V(@place(path, line), …)` for a `.violation V`: a row of V's place relation.
            let mut rel = head.rel.clone();
            let mut written = head.args.clone();
            if let Some(T::Call { name, functor: true, args: place }) = head.args.first().map(|a| self.tv(a))
                && name == "place"
                && let Some(k) = self.violations.iter().position(|v| v.name == head.rel)
            {
                let [path, line] = place.as_slice() else { return Err(Error::Syntax("@place takes a path and a line".to_owned())) };
                if !self.violations[k].places {
                    self.violations[k].places = true;
                    let mut cols = vec![("path".to_owned(), "sym".to_owned()), ("line".to_owned(), "int".to_owned())];
                    cols.extend(self.prog.columns_of(&head.rel).expect("a `.violation` is declared").into_iter().skip(1));
                    self.prog.declare_cols(&place_relation(&head.rel), cols);
                }
                rel = place_relation(&head.rel);
                written = [path.clone(), line.clone()].into_iter().chain(head.args.iter().skip(1).cloned()).collect();
            }
            let args = written.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
            for k in 0..combinations {
                self.subs.truncate(1);
                let heads = self.heads_at(c);
                let (_, chosen) = self.body_of(0, |me| me.clause_put(heads, body.clone(), k))?;
                let unset = unset_vars(&chosen)?;
                if let Some(m) = &head.modal {
                    self.modal.insert(self.written(), m.clone());
                }
                self.begin_rule(&rel, args.clone(), unset);
                for l in chosen {
                    if let Err(e) = self.emit(l) {
                        self.drop_rule();
                        return Err(e);
                    }
                }
                self.end_rule()?;
            }
        }
        Ok(())
    }

    /// A clause's heads, each argument read under the substitution its hoisting made, and its body's count of
    /// combinations, writing combination `k` (`conjunction_put`): the aggregates in the heads' arguments hoisted into
    /// the body (`cc(f, 1 + count : { … })` is a variable the body binds).
    fn clause_put(&mut self, heads: Vec<HeadAt>, mut body: Vec<LitAt>, k: usize) -> Result<(Vec<HeadAt>, usize), Error> {
        let mut sub = None;
        for a in heads.iter().flat_map(|h| h.args.iter()) {
            self.hoist(a, 0, &mut sub, &mut body, true);
        }
        let s = sub.unwrap_or(0);
        let heads: Vec<HeadAt> = heads.into_iter().map(|h| HeadAt { args: h.args.into_iter().map(|a| reads::with_sub_at(a, s)).collect(), ..h }).collect();
        let mut outer = Vec::new();
        heads.iter().flat_map(|h| &h.args).for_each(|a| self.term_vars(a, &mut outer));
        let n = self.conjunction_put(&body, &outer.into_iter().collect(), k)?;
        Ok((heads, n))
    }

    /// A conjunction's alternatives (one per choice in its disjunctions).
    /// `context`: the variables used outside the conjunction.
    fn conjunction(&mut self, lits: &[LitAt], context: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        Ok(self.alts_of(|me, k| me.conjunction_put(lits, context, k))?.into_iter().map(merge_negated_tests).collect())
    }

    /// A conjunction, writing its combination `k` (one alternative of each literal, the first literal's the most
    /// significant; none past its count): its count of combinations. A literal of many is outlined past OUTLINE_PAST
    /// (decision 0276 A3.4: its literals' counts are logged for a replay to split `k` by).
    fn conjunction_put(&mut self, lits: &[LitAt], context: &BTreeSet<String>, k: usize) -> Result<usize, Error> {
        // An aggregate inside a term (`n = count : { … } + 1`) is a variable bound beside it.
        let mut hoisted = Vec::new();
        for lit in lits {
            let base = reads::sub_of_lit(lit);
            let mut sub = None;
            match self.lit(lit) {
                L::Atom { args, .. } => args.iter().for_each(|a| self.hoist(a, base, &mut sub, &mut hoisted, true)),
                L::Cmp { l, r, .. } => {
                    // `count : { … } = count : { … }`: the right one gets a variable.
                    let both = matches!((self.tv(&l), self.tv(&r)), (T::Agg { .. }, T::Agg { .. }));
                    self.hoist(&l, base, &mut sub, &mut hoisted, false);
                    self.hoist(&r, base, &mut sub, &mut hoisted, both);
                }
                L::Pattern { .. } | L::Group { .. } => {}
            }
            hoisted.push(sub.map_or_else(|| lit.clone(), |s| reads::with_sub(lit, s)));
        }
        let lits = hoisted.as_slice();
        // The literals' counts: logged by the deciding walk, read back by a replay.
        let slot = self.position();
        let logged = match self.replay.as_mut() {
            Some(at) => {
                let Some(Logged::Counts(c)) = self.log.get(*at).cloned() else { panic!("a replayed conjunction reads its literals' counts") };
                *at += 1;
                Some(c)
            }
            None => {
                self.log.push(Logged::Counts(Vec::new()));
                None
            }
        };
        let mut picks = vec![usize::MAX; lits.len()];
        if let Some(c) = &logged
            && k < c.iter().map(|(n, _)| n).product()
        {
            let mut rest = k;
            for (p, (n, _)) in picks.iter_mut().zip(c).rev() {
                *p = rest % n;
                rest /= n;
            }
        }
        let mut combinations = 1;
        let mut counts = Vec::with_capacity(lits.len());
        for (i, lit) in lits.iter().enumerate() {
            // The variables used outside this literal, which an aggregate inside it is correlated on.
            let mut outer: Vec<String> = context.iter().cloned().collect();
            lits.iter().enumerate().filter(|(j, _)| *j != i).for_each(|(_, l)| self.scope_vars(l, &mut outer));
            let outer: BTreeSet<String> = outer.into_iter().collect();
            let enclosing = self.siblings.clone();
            self.siblings = lits.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, l)| l.clone()).chain(enclosing.iter().cloned()).collect();
            let start = self.position();
            let outlined = match &logged {
                Some(c) => c[i].1,
                None => false,
            };
            let r = if outlined { Ok(usize::MAX) } else { self.literal_put(lit, &outer, picks[i]) };
            self.siblings = enclosing;
            let mut n = r?;
            // Alternatives multiply: `(a ; b), (c ; d)` is four rules, and `within_stop` is two. Past OUTLINE_PAST of
            // them, a literal's alternatives are a helper relation (`outline`), and `within_stop(n, a, S)` over a kind
            // set is one atom of `rr_wstop_<S>`, S's relation (`within_stop_relation`). A nest of them was otherwise
            // exponential: a Rust policy of 200 lines lowered to 101,423 rules. Below it, they stay in the rule, whose
            // own literals narrow each.
            let outline = outlined || (logged.is_none() && combinations * n > OUTLINE_PAST && n > 1);
            if outline {
                // Its alternatives whole: replayed from where the literal began.
                let replaying = self.replay.is_some();
                self.replay = Some(start);
                let enclosing = self.siblings.clone();
                self.siblings = lits.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, l)| l.clone()).chain(enclosing.iter().cloned()).collect();
                let alts = self.alts_of(|me, p| me.literal_put(lit, &outer, p));
                self.siblings = enclosing;
                let alts = alts?;
                let end = self.replay.expect("replaying");
                self.replay = replaying.then_some(end);
                let out = if let L::Atom { rel, args, neg: false } = self.lit(lit)
                    && rel == "within_stop"
                    && let [n, a, stop] = args.as_slice()
                    && self.node_set(stop).is_none()
                {
                    let r = self.within_stop_relation(stop)?;
                    vec![vec![Literal::Atom(Atom { rel: r.into(), args: [self.term(n)?, self.term(a)?].into(), neg: false, gated: false })]]
                } else {
                    let mut used: Vec<String> = context.iter().cloned().collect();
                    lits.iter().enumerate().filter(|(j, _)| *j != i).for_each(|(_, l)| self.scope_vars(l, &mut used));
                    let used: BTreeSet<String> = used.into_iter().collect();
                    self.outline(alts, &used)
                };
                n = self.put_alts(out, picks[i])?;
            }
            combinations *= n;
            counts.push((n, outline));
        }
        if logged.is_none() {
            self.log[slot] = Logged::Counts(counts);
        }
        Ok(combinations)
    }

    /// Each aggregate inside term `a` (and `a` itself when `whole`) as a fresh variable, bound by `v = aggregate` in
    /// `out`. The variable stands in the term's place under substitution set `sub`, made over `base` at the first
    /// term moved.
    fn hoist(&mut self, a: &At, base: u32, sub: &mut Option<u32>, out: &mut Vec<LitAt>, whole: bool) {
        let At::Node(n, _) = a else { return };
        let n = *n;
        let place = |me: &mut Self, sub: &mut Option<u32>, stem: &str| -> u32 {
            let v = me.fresh(stem);
            let s = *sub.get_or_insert_with(|| me.new_sub(base));
            me.set_sub(s, n, v);
            s
        };
        match self.tv(&At::Node(n, sub.unwrap_or(base))) {
            T::Agg { .. } if whole => {
                let s = place(self, sub, "agg");
                out.push(LitAt::Bound(n, s));
            }
            // A tape operation (decision 0058 §4) is a value the engine makes on demand: `t = @parse(g, x)` is
            // `parse_of(x, g, t)`, `x = @unparse(t)` is `unparse_of(t, x)`, `t3 = @splice(t, n, t2)` is
            // `splice_of(t, n, t2, t3)`.
            T::Call { name, functor: true, args } if TAPE_OPS.iter().any(|(f, k, _, _)| *f == name && *k == args.len()) => {
                for x in &args {
                    self.hoist(x, base, sub, out, true);
                }
                let &(_, _, rel, cols) = TAPE_OPS.iter().find(|(f, k, _, _)| *f == name && *k == args.len()).expect("matched");
                let s = place(self, sub, "tape");
                out.push(LitAt::Op(n, s));
                if !self.prog.has_input(rel) {
                    self.prog.add_input(rel);
                    self.prog.declare_cols(rel, cols.iter().copied());
                    self.prog.edit(|b| b.tapes(), |t| Field::Tapes(t), |t| t.entry(rel.to_owned()).or_default().extend(super::tape_type::tape_columns(rel)));
                }
            }
            // `@tree_digest(n)` reads the tree: a variable bound by the engine's `tree_digest(n, h)`.
            T::Call { name, functor: true, args } if name == "tree_digest" && args.len() == 1 => {
                self.hoist(&args[0], base, sub, out, true);
                let s = place(self, sub, "digest");
                out.push(LitAt::Op(n, s));
                if !self.prog.has_input("tree_digest") {
                    self.prog.add_input("tree_digest");
                    self.prog.declare_cols("tree_digest", [("n", "node"), ("s", "sym")]);
                }
            }
            T::Call { args, .. } => args.iter().for_each(|x| self.hoist(x, base, sub, out, true)),
            T::Bin { l, r, .. } => {
                self.hoist(&l, base, sub, out, true);
                self.hoist(&r, base, sub, out, true);
            }
            T::Neg(x) => self.hoist(&x, base, sub, out, true),
            _ => {}
        }
    }

    /// Literal `lit`, writing its alternative `pick` (`alt`; none past its count): its count of alternatives.
    fn literal_put(&mut self, lit: &LitAt, outer: &BTreeSet<String>, pick: usize) -> Result<usize, Error> {
        let str_of = |me: &Self, a: &At| match me.tv(a) {
            T::Str(s) => Some(s),
            _ => None,
        };
        let var_of = |me: &Self, a: &At| match me.tv(a) {
            T::Var(v) => Some(v),
            _ => None,
        };
        // A literal of one alternative: its lowering writes it when it is the one picked.
        let one = |me: &mut Self, f: &mut dyn FnMut(&mut Self) -> Result<(), Error>| -> Result<usize, Error> {
            me.alt(0, pick, |me| f(me))?;
            Ok(1)
        };
        match self.lit(lit) {
            L::Group { alts, neg: false } => {
                if let Some(set) = self.test_set(&alts, false)? {
                    return self.put_alts(vec![set], pick);
                }
                // `(a ; b)`: each alternative's combinations, in turn.
                let mut offset = 0;
                for alt in &alts {
                    let k = pick.checked_sub(offset).unwrap_or(usize::MAX);
                    offset += self.conjunction_put(alt, outer, k)?;
                }
                Ok(offset)
            }
            L::Group { alts, neg: true } => {
                if let Some(mut set) = self.test_set(&alts, true)? {
                    if let Some(Literal::Cmp { r, .. }) = set.last_mut() {
                        *r = Term::Int(0);
                    }
                    return self.put_alts(vec![set], pick);
                }
                // `!(a, b)`: no row of the conjunction, through a helper relation of the variables it shares with the
                // rule; `!(a ; b)` is `!a, !b`.
                let mut out = Vec::new();
                for alt in &alts {
                    out.push(self.negated_conjunction(alt, outer)?);
                }
                self.put_alts(vec![out], pick)
            }
            L::Pattern { subject, pattern, selector, neg } => one(self, &mut |me| me.pattern(&subject, &pattern, selector.as_deref(), neg)),
            L::Atom { rel, args, neg } if matches!(rel.as_str(), "nearest" | "inside" | "has" | "within_stop") && args.len() == 3 && self.node_set(&args[2]).is_some() => {
                let alts = self.node_axis(&rel, &args, neg, outer)?;
                self.put_alts(alts, pick)
            }
            L::Atom { rel, args, neg } if matches!(rel.as_str(), "within" | "within_stop") => self.within_put(&rel, &args, neg, pick),
            L::Atom { rel, args, neg } if rel == "split_dot" => self.split_dot(&args, neg, pick),
            // `match_all("re", sp, m)` (three arguments: each match in span sp, a span m, as the linter's regex finds
            // them) and `match_group` are relations the batches make (`facts::vtab::syntax`'s `Extra`); the symbol
            // forms of `match_all` (two or five arguments) and a `match` with captures are the text layer's (0054).
            L::Atom { rel, args, neg: false }
                if rel == "match_all" && args.len() != 3 || rel == "match" && args.len() == 2 && str_of(self, &args[0]).is_some_and(|re| !groups(&re).is_empty()) =>
            {
                one(self, &mut |me| me.text_matches(&rel, &args))
            }
            // `match_all("re", n, m)` over a node a sibling binds to a kind (`kind(n, "arguments")`): `rx_span`'s
            // matches in the node, m each match, its groups bound with it. A `source`, `src_line` or comment span, or a
            // column of a derived relation (which may hold such spans), is the batch's `matchall_` relations'.
            L::Atom { rel, args, neg: false }
                if rel == "match_all"
                    && args.len() == 3
                    && str_of(self, &args[0]).is_some()
                    && var_of(self, &args[1]).is_some_and(|n| self.span_container(&n).is_none() && self.kind_bound(&n)) =>
            {
                one(self, &mut |me| me.node_matches(&args))
            }
            // A text test on a node or span tests its text (`core.rr`).
            L::Atom { rel, args, neg } if test_of(&rel).is_some() && args.len() == 2 && var_of(self, &args[1]).is_some_and(|v| self.span_container(&v).is_some() || self.node_typed(&v)) => {
                one(self, &mut |me| me.text_test(&rel, &args, neg))
            }
            // `!match_all("re", s)`: no match anywhere in s, a search (captures bind nothing outside the `!`).
            L::Atom { rel, args, neg: true } if rel == "match_all" && args.len() != 3 => {
                let (Some(re), [_, s]) = (args.first().and_then(|a| str_of(self, a)), args.as_slice()) else {
                    return Err(Error::Syntax("match_all takes a regex and a text".to_owned()));
                };
                let l = Literal::Cmp { l: Term::Call("rx".into(), [self.term(s)?, Term::Str(re.into())].into()), op: "=", r: Term::Int(0) };
                one(self, &mut |me| me.emit(l.clone()))
            }
            L::Atom { rel, args, neg } => one(self, &mut |me| me.atom_literal(&rel, &args, neg)),
            L::Cmp { l, op, r } => one(self, &mut |me| me.comparison(&l, op, &r, outer)),
        }
    }

    /// Atom `rel(args)` (negated: `neg`), and the demands it makes.
    fn atom_literal(&mut self, rel: &str, args: &[At], neg: bool) -> Result<(), Error> {
        // core.rr's tree arithmetic (`earlier`, `precedes`, …) as the relations the engine has (`core`).
        if let Some(literals) = self.rewrite(rel, args, neg)? {
            return literals.into_iter().try_for_each(|l| self.emit(l));
        }
        if !neg && matches!(rel, "parsed" | "sub_tape") {
            self.embed_demand(args)?;
        }
        if !neg && matches!(rel, "parse_of" | "splice_of") {
            self.value_demand(rel, args)?;
        }
        if let Some(demand) = world_demand(rel) {
            self.world_demand(demand, args)?;
        }
        // A computed argument (`loads(@dot(m, x))`) is bound to a variable first: the engine joins on
        // variables (`front::finish_checks_on`'s computed-term check).
        let mut atom = self.atom(rel, args, neg)?;
        if let Literal::Atom(a) = &mut atom {
            for t in &mut a.args {
                if matches!(t, Term::Call(..)) {
                    let v = self.fresh("t");
                    let bound = Literal::Cmp { l: Term::Var(v.clone().into()), op: "=", r: std::mem::replace(t, Term::Var(v.into())) };
                    self.emit(bound)?;
                }
            }
        }
        self.emit(atom)
    }

    /// `parsed(s, "g", r)` or `sub_tape(s, "g", t)` asks for the guest at s (decision 0055): `rr_embed_req(s, "g")`.
    /// A grammar that is not a constant asks for nothing: the atom reads the guests other rules or libraries asked for.
    fn embed_demand(&mut self, args: &[At]) -> Result<(), Error> {
        let [s, g, out] = args else { return Ok(()) };
        if !matches!(self.tv(s), T::Var(_)) || !matches!(self.tv(g), T::Str(_)) {
            return Ok(());
        }
        self.demand("rr_embed_req", &[(s.clone(), "node"), (g.clone(), "sym")], out, true)
    }

    /// A value a rule computes from a tape operation (decision 0058 §4) asks for it: `parse_of(x, g, t)` is
    /// `rr_parse_req(at, g, x)` (the text last, as it may hold a tab; `at` a location of the rule's context, so the
    /// demand is its document's), `splice_of(t, n, t2, t3)` is `rr_splice_req(n, t2)` (n names its tape).
    /// `unparse_of` asks for nothing: every guest and value has its text.
    fn value_demand(&mut self, rel: &str, args: &[At]) -> Result<(), Error> {
        match (rel, args) {
            ("parse_of", [x, g, t]) => self.demand("rr_parse_req", &[(At::Var(LOCATED.to_owned()), "int"), (g.clone(), "sym"), (x.clone(), "sym")], t, false),
            ("splice_of", [_, n, t2, t3]) => self.demand("rr_splice_req", &[(n.clone(), "node"), (t2.clone(), "sym")], t3, false),
            _ => Ok(()),
        }
    }

    /// A demand `req(given…)` over the rule's other literals that bind the given variables, those that do not read
    /// what is asked for (not `out`; with `guests`, no guest relation either: a guest is what they wait for). A value
    /// may be made from documents of earlier rounds (`@splice` of a node of a `@parse`), so its demand reads guests.
    /// The corpus runner runs these rules first, makes what is asked for and rebinds the batch with it. A given
    /// variable nothing else binds asks for nothing.
    fn demand(&mut self, req: &str, given: &[(At, &str)], out: &At, guests: bool) -> Result<(), Error> {
        if self.printed_demands.contains(req) {
            return Ok(());
        }
        let var_of = |me: &Self, t: &At| match me.tv(t) {
            T::Var(v) => Some(v),
            _ => None,
        };
        let vars: Vec<String> = given.iter().filter_map(|(t, _)| var_of(self, t)).collect();
        if given.iter().any(|(t, _)| !matches!(self.tv(t), T::Var(_) | T::Str(_))) {
            return Ok(());
        }
        // What reads the guest: the literals joined to `out` by variables other than the given ones, and the guest
        // relations.
        let mut reads = self.term_var_set(out);
        reads.retain(|x| !vars.contains(x));
        let guest_rel = |me: &Self, l: &LitAt| guests && matches!(me.lit(l), L::Atom { rel, .. } if GUEST_RELATIONS.contains(&rel.as_str()));
        let mut keep: Vec<bool> = self.siblings.iter().map(|l| !guest_rel(self, l)).collect();
        for l in self.siblings.iter().filter(|l| guest_rel(self, l)) {
            reads.extend(self.vars_of(l).into_iter().filter(|x| !vars.contains(x)));
        }
        loop {
            let mut grew = false;
            for (l, k) in self.siblings.iter().zip(keep.iter_mut()) {
                let vs = self.vars_of(l);
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
            if let L::Atom { args, neg: false, .. } = self.lit(l) {
                let mut sub = None;
                let wild: Vec<u32> = args.iter().filter_map(|a| if let At::Node(w, _) = a && matches!(self.tv(a), T::Wild) { Some(*w) } else { None }).collect();
                for w in &wild {
                    let v = self.fresh("w");
                    let base = reads::sub_of_lit(l);
                    let s = *sub.get_or_insert_with(|| self.new_sub(base));
                    self.set_sub(s, *w, v);
                }
                if let Some(s) = sub {
                    *l = reads::with_sub(l, s);
                }
            }
        }
        self.siblings = siblings;
        // `LOCATED`: a location of the context, the document the demand is made in. A variable in a node column of a
        // kept atom, or a document of the chunk when the rule has no context.
        let located = given.iter().any(|(t, _)| matches!(self.tv(t), T::Var(v) if v == LOCATED));
        let mut given: Vec<(At, &str)> = given.to_vec();
        let mut every_chunk_doc = false;
        if located {
            // A value's demand reads only what its inputs are joined to: a demand may ask for more than the rule
            // reads, never less, and one connected context is one document's.
            let mut inputs: BTreeSet<String> = given.iter().filter_map(|(t, _)| var_of(self, t)).filter(|v| v != LOCATED).collect();
            let mut joined = vec![false; self.siblings.len()];
            loop {
                let mut grew = false;
                for (l, j) in self.siblings.iter().zip(joined.iter_mut()) {
                    let vs = self.vars_of(l);
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
            let loc = self.siblings.iter().find_map(|l| match self.lit(l) {
                L::Atom { rel, args, neg: false } => {
                    let decl = self.prog.columns_of(rel.as_str());
                    let doc = crate::code_rule_dsl::logic::front::document_column(&rel);
                    args.iter().enumerate().find_map(|(c, a)| match self.tv(a) {
                        T::Var(v) if Some(c) == doc || decl.as_ref().and_then(|d| d.get(c)).is_some_and(|(_, t)| t == "node") => Some(v),
                        _ => None,
                    })
                }
                _ => None,
            });
            match loc {
                Some(v) => given[0] = (At::Var(v), "int"),
                None => {
                    given[0] = (At::Var("rr_demand_doc".into()), "int");
                    every_chunk_doc = true;
                }
            }
        }
        let vars: Vec<String> = given.iter().filter_map(|(t, _)| var_of(self, t)).filter(|v| v != "rr_demand_doc").collect();
        // Bound by a positive atom, or equal to a variable one binds (`t2 = rr_tape7`, a hoisted value).
        let binds = |v: &String| {
            let by_atom = |v: &String| self.siblings.iter().any(|l| matches!(self.lit(l), L::Atom { rel, neg: false, .. } if test_of(&rel).is_none() && rel != "earlier") && self.vars_of(l).contains(v));
            by_atom(v)
                || self.siblings.iter().any(|l| match self.lit(l) {
                    L::Cmp { l, op: "=", r } => match (self.tv(&l), self.tv(&r)) {
                        (T::Var(a), T::Var(b)) => (a == *v && by_atom(&b)) || (b == *v && by_atom(&a)),
                        _ => false,
                    },
                    _ => false,
                })
        };
        let all_bound = vars.iter().all(binds);
        // A demand of constants (`@parse("sql", "SELECT 1")`) is made where the rest of the rule holds (a nullary
        // context), or in every chunk when nothing else in the rule is independent of it.
        self.request_context = true;
        let ctx = if all_bound && !(vars.is_empty() && self.siblings.is_empty()) { Some(self.context(&vars)) } else { None };
        self.request_context = false;
        let every_chunk = every_chunk_doc || (all_bound && vars.is_empty() && ctx.is_none());
        self.siblings = saved;
        if !all_bound {
            return Ok(());
        }
        let ctx = ctx.transpose()?;
        self.prog.declare_cols(req, given.iter().enumerate().map(|(i, (_, ty))| (format!("c{i}"), (*ty).to_owned())));
        let args: Vec<Term> = given
            .iter()
            .map(|(t, _)| match self.tv(t) {
                T::Var(v) => Term::Var(v.into()),
                T::Str(s) => Term::Str(s.into()),
                _ => unreachable!("checked above"),
            })
            .collect();
        let head = Atom { rel: req.to_owned().into(), args: args.into(), neg: false, gated: false };
        let mut body: Vec<Literal> = ctx.into_iter().map(Literal::Atom).collect();
        if every_chunk {
            body.push(Literal::Atom(Atom { rel: "doc".into(), args: [Term::Var("rr_demand_doc".into()), Term::Wild, Term::Wild].into(), neg: false, gated: false }));
        }
        let rule = Rule { head, body: body.into() };
        self.write_once(rule);
        Ok(())
    }

    /// `dir_entry(d, …)` or `file_line(p, …)`, read or negated, asks for the listing of d or the lines of p (t21 E1):
    /// `rr_dir_req(d)` or `rr_file_req(p)` over the rule's other literals that bind it, those that read none of what
    /// the atom gives. The corpus runner answers the requests and runs the rules again (`corpus::world`). A key
    /// nothing else binds asks for nothing: the atom reads what other rules asked for.
    fn world_demand(&mut self, demand: &str, args: &[At]) -> Result<(), Error> {
        if self.printed_demands.contains(demand) {
            return Ok(());
        }
        let Some(key) = args.first() else { return Ok(()) };
        let head = |key: Term| Atom { rel: demand.to_owned().into(), args: [key].into(), neg: false, gated: false };
        self.prog.declare_cols(demand, [("k", "sym")]);
        let s = match self.tv(key) {
            // A constant key is a fact, as the reader takes a bodiless clause: printed and read back, it is the same.
            T::Str(k) => {
                let row: crate::code_rule_dsl::logic::front::List<Value> = [Value::Str(k.into())].into();
                if !self.prog.fact_rows(demand).contains(&row) {
                    self.prog.add_facts(demand, &[row]);
                }
                return Ok(());
            }
            T::Var(s) => s,
            _ => return Ok(()),
        };
        let s = &s;
        // What the atom gives: its other arguments' variables, and every literal joined to them.
        let mut reads: BTreeSet<String> = args[1..].iter().flat_map(|a| self.term_var_set(a)).collect();
        reads.remove(s);
        let mut keep: Vec<bool> = vec![true; self.siblings.len()];
        loop {
            let mut grew = false;
            for (l, k) in self.siblings.iter().zip(keep.iter_mut()) {
                let vars = self.vars_of(l);
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
        // Bound by an atom, or computed from what atoms bind (`d = cat(r, "/", p)`, or a `match`'s named group: `match("(?<d>.*)/[^/]*", p)`
        // is `d = @rx_group(p, …)` once lowered).
        let binds = self.siblings.iter().any(|l| match self.lit(l) {
            L::Atom { rel, neg: false, args, .. } if matches!(rel.as_str(), "match" | "match_all") => matches!(args.first().map(|a| self.tv(a)), Some(T::Str(re)) if groups(&re).contains(s)),
            L::Atom { rel, neg: false, .. } => test_of(&rel).is_none() && rel != "earlier" && self.vars_of(l).contains(s),
            L::Cmp { l, op: "=", r } => matches!(self.tv(&l), T::Var(v) if v == *s) || matches!(self.tv(&r), T::Var(v) if v == *s),
            _ => false,
        });
        let ctx = if binds { Some(self.context(std::slice::from_ref(s))) } else { None };
        self.siblings = saved;
        let Some(ctx) = ctx.transpose()? else { return Ok(()) };
        let rule = Rule { head: head(Term::Var(s.clone().into())), body: [Literal::Atom(ctx)].into() };
        self.write_once(rule);
        Ok(())
    }

    /// `!(l, …)`: a helper relation of the conjunction, over the variables it shares with the rule (an aggregate's
    /// inside it too: `shared_vars`), bound inside it by a context relation where the conjunction only tests them,
    /// negated. A variable only the conjunction has is its own (Soufflé's scoping).
    /// A literal's alternatives with those that ground the variables they share with `used` (the rest of the rule)
    /// as one helper relation `rr_or<k>(shared)`, a rule each, and one atom in their place (as `!(…)` is
    /// `rr_not<k>`). An alternative that leaves one unbound (it does not name it) stays as it is.
    fn outline(&mut self, alts: Vec<Vec<Literal>>, used: &BTreeSet<String>) -> Vec<Vec<Literal>> {
        let mut shared = BTreeSet::new();
        for b in &alts {
            for l in b {
                shared.extend(l.vars().into_iter().filter(|v| used.contains(v)));
            }
        }
        let shared: Vec<String> = shared.into_iter().collect();
        // `unset(x)` is lowered in a rule's own body only (`unset`).
        let unset = |b: &[Literal]| b.iter().any(|l| matches!(l, Literal::Atom(a) if &*a.rel == "unset"));
        let (out, keep): (Vec<_>, Vec<_>) = alts.into_iter().partition(|b| !unset(b) && grounds(b, &shared));
        if out.len() < 2 {
            return keep.into_iter().chain(out).collect();
        }
        let name = self.fresh("or");
        let cols: Vec<(String, String)> = shared.iter().map(|c| (c.clone(), self.type_of(c, &out).to_string())).collect();
        self.prog.declare_cols(&name, cols);
        let head = Atom { rel: name.into(), args: shared.iter().map(|c| Term::Var(c.as_str().into())).collect(), neg: false, gated: false };
        for body in out {
            self.write(Rule { head: head.clone(), body: merge_negated_tests(body).into() });
        }
        keep.into_iter().chain([vec![Literal::Atom(head)]]).collect()
    }

    fn negated_conjunction(&mut self, lits: &[LitAt], outer: &BTreeSet<String>) -> Result<Literal, Error> {
        let shared: Vec<String> = self.shared_vars(lits).into_iter().filter(|v| outer.contains(v)).collect();
        let bound: BTreeSet<String> = self.body_vars(&lits.iter().filter(|l| self.binds_columns(l)).cloned().collect::<Vec<_>>());
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
            (shared.iter().map(|c| (c.clone(), self.type_of(c, &bodies))).collect(), shared.iter().map(|c| Term::Var(c.clone().into())).collect())
        };
        self.prog.declare_cols(&name, cols.iter().map(|(c, t)| (c.to_string(), t.to_string())));
        let head = Atom { rel: name.into(), args: args.into(), neg: false, gated: false };
        for body in bodies {
            self.write(Rule { head: head.clone(), body: body.into() });
        }
        Ok(Literal::Atom(Atom { neg: true, ..head }))
    }

    /// `precedes_stop(a, b, S)` as a relation `rr_prec_<S>(a, b)`: a and b are children of one parent, a first,
    /// and no child between them is in S (a kind, a relation of kinds, or a relation of nodes).
    fn precedes_stop(&mut self, set: &At) -> String {
        let label = match self.tv(set) {
            T::Str(k) | T::Var(k) => k.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>(),
            _ => "set".to_owned(),
        };
        let name = format!("{RESERVED}prec_{label}");
        if self.prog.declares(&name) {
            return name;
        }
        let v = |s: &str| Term::Var(s.to_owned().into());
        let atom = |rel: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: rel.to_owned().into(), args: args.into(), neg, gated: false });
        let cmp = |l: &str, r: &str| Literal::Cmp { l: v(l), op: "<", r: v(r) };
        let member = match (self.tv(set), self.node_set(set)) {
            (_, Some(nodes)) => vec![atom(&nodes, vec![v("s")], false)],
            (T::Str(k), _) => vec![atom("kind", vec![v("s"), Term::Str(k.into())], false)],
            (T::Var(kinds), _) => vec![atom("kind", vec![v("s"), v("sk")], false), atom(&kinds, vec![v("sk")], false)],
            _ => vec![],
        };
        // after(a, k): a later sibling of a at index k is in S; first(a, j): the first such.
        let after = format!("{RESERVED}prec_after_{label}");
        let first = format!("{RESERVED}prec_first_{label}");
        let node_int = vec![("a".to_owned(), "node".to_owned()), ("k".to_owned(), "int".to_owned())];
        self.prog.declare_cols(&after, node_int.iter().map(|(c, t)| (c.to_string(), t.to_string())));
        self.prog.declare_cols(&first, node_int.iter().map(|(c, t)| (c.to_string(), t.to_string())));
        self.prog.declare_cols(&name, [("a", "node"), ("b", "node")]);
        let head = |rel: &str, args: Vec<Term>| Atom { rel: rel.to_owned().into(), args: args.into(), neg: false, gated: false };
        let mut body = vec![atom("child", vec![v("p"), v("a"), v("i")], false), atom("child", vec![v("p"), v("s"), v("k")], false), cmp("i", "k")];
        body.extend(member);
        self.write(Rule { head: head(&after, vec![v("a"), v("k")]), body: body.into() });
        self.write(Rule {
            head: head(&first, vec![v("a"), v("j")]),
            body: vec![
                atom(&after, vec![v("a"), Term::Wild], false),
                Literal::Agg { var: "j".to_owned(), op: AggOp::Min, over: Some("k".to_owned()), atom: head(&after, vec![v("a"), v("k")]) },
            ].into(),
        });
        let pair = || vec![atom("child", vec![v("p"), v("a"), v("i")], false), atom("child", vec![v("p"), v("b"), v("l")], false), cmp("i", "l")];
        let mut none = pair();
        none.push(atom(&after, vec![v("a"), Term::Wild], true));
        self.write(Rule { head: head(&name, vec![v("a"), v("b")]), body: none.into() });
        let mut upto = pair();
        upto.push(atom(&first, vec![v("a"), v("j")], false));
        upto.push(Literal::Cmp { l: v("l"), op: "<=", r: v("j") });
        self.write(Rule { head: head(&name, vec![v("a"), v("b")]), body: upto.into() });
        name
    }

    /// A set argument that names a relation of nodes (`.decl block_node(n: node)`), not of kinds.
    fn node_set(&self, set: &At) -> Option<String> {
        let T::Var(name) = self.tv(set) else { return None };
        let cols = self.prog.columns_of(&name)?;
        (cols.len() == 1 && cols[0].1 == "node").then_some(name)
    }

    /// `tape(n, par, size, depth)`, read by the tree's ranges and climbs.
    fn need_tape(&mut self) {
        if !self.prog.declares("tape") {
            self.prog.declare_cols("tape", ["n", "par", "size", "depth"].map(|c| (c, "int")));
        }
        if !self.prog.has_input("tape") {
            self.prog.add_input("tape");
        }
    }

    /// A climb up the tree from the nodes `seed` gives (in its variable `x`): `rr_up<k>(n, a)` for each strict
    /// ancestor a of n, up to and including the first node of `stop` where one is given. Its work is each seed's
    /// path up (to the stop), from the tape's parent column.
    fn climb(&mut self, seed: Vec<Literal>, x: &str, stop: Option<&str>) -> String {
        self.need_tape();
        let rel = self.fresh("up");
        self.prog.declare_cols(&rel, [("n", "node"), ("a", "node")]);
        let v = |s: &str| Term::Var(s.to_owned().into());
        let atom = |r: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: r.to_owned().into(), args: args.into(), neg, gated: false });
        let up = |p: &str, q: &str| atom("tape", vec![v(p), v(q), Term::Wild, Term::Wild], false);
        let real = |q: &str| Literal::Cmp { l: v(q), op: ">=", r: Term::Int(0) };
        let mut first = seed;
        first.extend([up(x, "rr_p"), real("rr_p")]);
        self.write(Rule { head: Atom { rel: rel.clone().into(), args: [v(x), v("rr_p")].into(), neg: false, gated: false }, body: first.into() });
        let mut next = vec![atom(&rel, vec![v("rr_n"), v("rr_p")], false)];
        if let Some(stop) = stop {
            next.push(atom(stop, vec![v("rr_p")], true));
        }
        next.extend([up("rr_p", "rr_q"), real("rr_q")]);
        self.write(Rule { head: Atom { rel: rel.clone().into(), args: [v("rr_n"), v("rr_q")].into(), neg: false, gated: false }, body: next.into() });
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
        Ok(vec![Literal::Atom(Atom { rel: "node".into(), args: [Term::Wild, Term::Var(n.to_owned().into()), Term::Wild, Term::Wild, Term::Wild].into(), neg: false, gated: false })])
    }

    /// The axes over a set of nodes S (`core.rr`: a set may be a relation of nodes), as climbs up the tree:
    /// - `inside(n, a, S)`: a is an ancestor of n, and S(a);
    /// - `nearest(n, a, S)`: the climb from n stops at the first node of S, which is a;
    /// - `has(a, n, S)`: the climb from each node of S, n, passes a;
    /// - `within_stop(n, a, S)`: n is below a, and the climb from n meets no node of S before a.
    ///
    /// Negated, each is a helper relation (`negate`).
    fn node_axis(&mut self, rel: &str, args: &[At], neg: bool, outer: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        let set = self.node_set(&args[2]).expect("a node set");
        let atom = |r: &str, args: Vec<Term>| Literal::Atom(Atom { rel: r.to_owned().into(), args: args.into(), neg: false, gated: false });
        let named = |me: &mut Self, t: &At| -> Result<String, Error> {
            match me.tv(t) {
                T::Var(v) => Ok(v),
                T::Wild => Ok(me.fresh("w")),
                other => Err(Error::Syntax(format!("{rel}: expected a variable, got {other:?}"))),
            }
        };
        let (first, second) = (named(self, &args[0])?, named(self, &args[1])?);
        let v = |s: &str| Term::Var(s.to_owned().into());
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
                let below = self.within("within", &[At::Var(n.clone()), At::Var(a.clone())], false)?;
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
    fn within(&mut self, rel: &str, args: &[At], neg: bool) -> Result<Vec<Vec<Literal>>, Error> {
        self.alts_of(|me, pick| me.within_put(rel, args, neg, pick))
    }

    /// `within`, writing alternative `pick` (`alt`): its count of alternatives.
    fn within_put(&mut self, rel: &str, args: &[At], neg: bool, pick: usize) -> Result<usize, Error> {
        let (n, a, stop) = match args {
            [n, a] if rel == "within" => (n, a, None),
            [n, a, stop] if rel == "within_stop" => (n, a, Some(stop)),
            _ => return not_yet(format!("{}{rel}/{}", if neg { "!" } else { "" }, args.len())),
        };
        if neg && stop.is_some() && matches!(self.tv(a), T::Wild) {
            return not_yet(format!("`!{rel}` with its ancestor unnamed"));
        }
        let (n, a) = (self.term(n)?, self.term(a)?);
        let within = crate::code_rule_dsl::facts::vtab::syntax::WITHIN;
        self.prog.declare_cols(&within, [("n", "node"), ("a", "node")]);
        if !self.prog.has_input(within) {
            self.prog.add_input(within);
        }
        let below = |neg: bool| Literal::Atom(Atom { rel: within.to_owned().into(), args: [n.clone(), a.clone()].into(), neg, gated: false });
        let Some(stop) = stop else {
            self.emit(below(neg))?;
            return Ok(1);
        };
        let set = self.set_name("within_stop", stop)?;
        let Term::Var(nv) = &n else { return Err(Error::Syntax("within_stop: the node is a variable".to_owned())) };
        let s = self.fresh("stop");
        let nearest = Literal::Nearest { n: nv.to_string(), a: s.clone(), kindset: set.clone() };
        // The nearest stop against a is by the tree, not by comparing ids (ids need not follow preorder, decision
        // 0083 §3).
        if neg {
            // n is not below a, or its nearest stop is strictly below a.
            let between = Literal::Atom(Atom { rel: within.to_owned().into(), args: [Term::Var(s.clone().into()), a.clone()].into(), neg: false, gated: false });
            self.alt(0, pick, |me| me.emit(below(true)))?;
            self.alt(1, pick, |me| {
                me.emit(below(false))?;
                me.emit(nearest)?;
                me.emit(between)
            })?;
            return Ok(2);
        }
        let none = Literal::Atom(Atom { rel: format!("{AXIS}{set}").into(), args: [n.clone(), Term::Wild].into(), neg: true, gated: false });
        // The nearest stop is a, or a is below it: it is not strictly below a (both are above n, so one of the three
        // holds). One alternative, not two: a rule's alternatives multiply, and a policy of several `within_stop`s
        // was exponential in them.
        let not_below = Literal::Atom(Atom { rel: within.to_owned().into(), args: [Term::Var(s.clone().into()), a.clone()].into(), neg: true, gated: false });
        self.alt(0, pick, |me| {
            me.emit(below(false))?;
            me.emit(none)
        })?;
        self.alt(1, pick, |me| {
            me.emit(below(false))?;
            me.emit(nearest)?;
            me.emit(not_below)
        })?;
        Ok(2)
    }

    /// `within_stop(n, a, S)` as a relation `rr_wstop_<S>(n, a)`: n is below a, and no node of S is between them.
    /// At most `within`'s rows, made once a set.
    fn within_stop_relation(&mut self, stop: &At) -> Result<String, Error> {
        let label = match self.tv(stop) {
            T::Str(k) | T::Var(k) => k.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>(),
            _ => "set".to_owned(),
        };
        let name = format!("{RESERVED}wstop_{label}");
        if self.prog.declares(&name) {
            return Ok(name);
        }
        let bodies = self.within("within_stop", &[At::Var("n".to_owned()), At::Var("a".to_owned()), stop.clone()], false)?;
        self.prog.declare_cols(&name, [("n", "node"), ("a", "node")]);
        let head = Atom { rel: name.as_str().into(), args: [Term::Var("n".into()), Term::Var("a".into())].into(), neg: false, gated: false };
        for body in bodies {
            self.write(Rule { head: head.clone(), body: body.into() });
        }
        Ok(name)
    }

    /// `name(s, g_…)`: the matches of `re` over the texts of `input` (a unary relation), each named group a column
    /// (and the match's ordinal, start and end with `positions`), made natively (`front::Native::MatchAll`). Its one
    /// rule orders it after its input; the evaluator makes its rows.
    fn match_all_native(&mut self, name: &str, input: &str, re: &str, gs: &[String], positions: bool) {
        let mut cols: Vec<_> = std::iter::once("s".to_owned()).chain(gs.iter().map(|g| format!("g_{g}"))).map(|c| (c, "sym".to_owned())).collect();
        if positions {
            cols.extend(["ordinal", "start", "end"].map(|c| (c.to_owned(), "int".to_owned())));
        }
        self.prog.declare_cols(&name, cols.iter().map(|(c, t)| (c.to_string(), t.to_string())));
        let x = |n: &str| Term::Var(n.to_owned().into());
        let mut body = vec![Literal::Atom(Atom { rel: input.to_owned().into(), args: [x("s")].into(), neg: false, gated: false })];
        body.extend(gs.iter().map(|g| Literal::Cmp { l: x(&format!("g_{g}")), op: "=", r: x("s") }));
        let mut head_args: Vec<_> = std::iter::once(x("s")).chain(gs.iter().map(|g| x(&format!("g_{g}")))).collect();
        if positions {
            // The native producer replaces these seed values. They establish dependency and numeric types.
            head_args.extend([Term::Int(0), Term::Int(0), Term::Int(0)]);
        }
        self.write(Rule { head: Atom { rel: name.to_owned().into(), args: head_args.into(), neg: false, gated: false }, body: body.into() });
        self.prog.set_native(&name, Native::MatchAll { input: input.to_owned(), pattern: re.to_owned(), groups: gs.to_vec(), positions });
    }

    /// A clause that is a natively made relation's definition as `print` writes it, whose relation lowering would
    /// otherwise name afresh: `h(s, g…[, o, b, e]) :- c(s), match_all(re, s[, o, b, e])` (the head the text, each
    /// group by its name, then the positions), or `python_candidates_definition`'s. The relation is made natively
    /// under its own name (`match_all_native`), as the clause says, not as another that h copies, so lowering a
    /// printed program gives the program again.
    fn native_definition(&mut self, heads: &[HeadAt], body: &[LitAt]) -> bool {
        if self.python_candidates_definition(heads, body) {
            return true;
        }
        let ([h], [b0, b1]) = (heads, body) else { return false };
        let (L::Atom { rel: input, args: of, neg: false }, L::Atom { rel: m, args, neg: false }) = (self.lit(b0), self.lit(b1)) else { return false };
        let ([s], Some(T::Str(re)), Some(T::Var(scanned))) = (of.as_slice(), args.first().map(|a| self.tv(a)), args.get(1).map(|a| self.tv(a))) else { return false };
        let T::Var(s) = self.tv(s) else { return false };
        let (input, re) = (input.as_str(), re.as_str());
        if m != "match_all" || scanned != s || h.modal.is_some() || self.prog.natives().contains_key(h.rel.as_str()) {
            return false;
        }
        let coordinates = &args[2..];
        if !(coordinates.is_empty() || coordinates.len() == 3 && coordinates.iter().all(|t| matches!(self.tv(t), T::Var(_)))) {
            return false;
        }
        let gs = groups(re);
        let want: Vec<At> = std::iter::once(At::Var(s.clone())).chain(gs.iter().map(|g| At::Var(g.clone()))).chain(coordinates.iter().cloned()).collect();
        if h.args.len() != want.len() || !h.args.iter().zip(&want).all(|(a, b)| self.same(a, b)) {
            return false;
        }
        // Over texts, as print writes it: a relation of one text column read, and the head declared (if it is) with
        // the native's columns. Over a node or span (`piece(t: loc)`) the matches are spans placed in the file,
        // which `text_matches` makes.
        let types = |rel: &str| self.prog.columns_of(rel).map(|cols| cols.into_iter().map(|(_, t)| t).collect::<Vec<String>>());
        if !types(input).is_some_and(|t| t == ["sym"]) {
            return false;
        }
        let made: Vec<&str> = std::iter::repeat_n("sym", 1 + gs.len()).chain(std::iter::repeat_n("int", coordinates.len())).collect();
        if types(&h.rel).is_some_and(|t| t != made) {
            return false;
        }
        self.match_all_native(&h.rel, input, re, &gs, !coordinates.is_empty());
        true
    }

    /// A clause that is a `python_candidates` relation's definition as `print` writes it, `h(c, f, r, o) :-
    /// python_candidates(c, f, r, o, input, targets, "context", "member")`: the relation made natively under its own
    /// name, not another that h copies (as `native_definition`'s `match_all`).
    fn python_candidates_definition(&mut self, heads: &[HeadAt], body: &[LitAt]) -> bool {
        let ([h], [b]) = (heads, body) else { return false };
        let L::Atom { rel, args, neg: false } = self.lit(b) else { return false };
        let [c, f, r, o, input, targets, context, member] = args.as_slice() else { return false };
        let (T::Var(input), T::Var(targets), T::Str(context), T::Str(member)) = (self.tv(input), self.tv(targets), self.tv(context), self.tv(member)) else { return false };
        let cols = [c, f, r, o];
        let distinct: BTreeSet<String> = cols.iter().filter_map(|t| if let T::Var(x) = self.tv(t) { Some(x) } else { None }).collect();
        if rel != "python_candidates" || h.modal.is_some() || distinct.len() != 4 || h.args.len() != 4 || !h.args.iter().zip(cols).all(|(a, b)| self.same(a, b)) || self.prog.natives().contains_key(h.rel.as_str()) {
            return false;
        }
        if !crate::code_rule_dsl::facts::core::PYTHON_CONTEXTS.contains(&context.as_str()) {
            return false;
        }
        let native = Native::PythonCandidates { input, targets, context, member };
        if self.prog.natives().values().any(|n| *n == native) {
            return false;
        }
        self.python_candidates_native(&h.rel, native);
        true
    }

    /// `match_all("re", s)`, and `match` with named groups (`core.rr`).
    ///
    /// - Over a `source` or `src_line` span (a sibling binds s so), the captures are spans: the batch runs the regex
    ///   over each such span and makes `rx_span(p, c, j, g, m)`, group g of match j of pattern p in span c, as span m
    ///   (`facts::vtab::syntax`'s `Extra`). `match` is the pattern anchored at both ends, so at most one match.
    /// - Over a symbol, `match`'s groups are `@rx_group` (`captures`); `match_all`'s rows are made natively from the
    ///   texts the rule asks about, a context relation of its other literals (`front::Native::MatchAll`). Without
    ///   groups, `match_all` is a search.
    fn text_matches(&mut self, rel: &str, args: &[At]) -> Result<(), Error> {
        let first = args.first().map(|a| self.tv(a));
        let (re, s, positions) = match (first, args) {
            (Some(T::Str(re)), [_, s]) => (re, s, &[][..]),
            (Some(T::Str(re)), [_, s, _, _, _]) if rel == "match_all" => (re, s, &args[2..5]),
            _ => return Err(Error::Syntax(format!("{rel} takes a regex string and a text; symbol match_all also accepts ordinal, start, end"))),
        };
        let re = &re;
        let gs = groups(re);
        let all = rel == "match_all";
        // A span `match_all` (or `match` with groups) reads through a derived relation (`t(d, sp) :- source(d, sp),
        // …`, or `kind(c, "comment")`) is a node-typed column here; `span_patterns` finds which spans it holds.
        if let T::Var(v) = self.tv(s)
            && (self.span_container(&v).is_some() || (all || !gs.is_empty()) && self.node_typed(&v))
        {
            if !positions.is_empty() {
                return Err(Error::Syntax("match_all ordinal, start, end outputs require a symbol text".to_owned()));
            }
            let pattern = if all { re.clone() } else { format!("^(?:{re})$") };
            self.rx_span_input();
            let atom = |args: Vec<Term>| Literal::Atom(Atom { rel: "rx_span".into(), args: args.into(), neg: false, gated: false });
            let (p, c) = (Term::Str(pattern.into()), Term::Var(v.clone().into()));
            if gs.is_empty() {
                return self.emit(atom(vec![p, c, Term::Wild, Term::Str(String::new().into()), Term::Wild]));
            }
            let j = Term::Var(self.fresh("j").into());
            for g in &gs {
                self.emit(atom(vec![p.clone(), c.clone(), j.clone(), Term::Str(g.clone().into()), Term::Var(g.clone().into())]))?;
            }
            return Ok(());
        }
        if !all {
            return self.captures(args);
        }
        if gs.is_empty() && positions.is_empty() {
            let l = Literal::Cmp { l: Term::Call("rx".into(), [self.term(s)?, Term::Str(re.clone().into())].into()), op: "=", r: Term::Int(1) };
            return self.emit(l);
        }
        let T::Var(v) = self.tv(s) else { return not_yet(format!("match_all over a computed text, {:?} (bind it to a variable first)", self.tv(s))) };
        let v = &v;
        // The texts asked about: the rule's other literals, those that read no capture (they are bound by the matches).
        let saved = self.siblings.clone();
        let mut produced = gs.clone();
        for term in positions {
            if let T::Var(v) = self.tv(term) {
                produced.push(v);
            }
        }
        let keep: Vec<bool> = self.siblings.iter().map(|l| !self.vars_of(l).iter().any(|x| produced.contains(x)) && !matches!(self.lit(l), L::Atom { rel, .. } if rel == "match_all")).collect();
        let mut keep = keep.into_iter();
        self.siblings.retain(|_| keep.next().unwrap_or(false));
        // The text a pattern scans keeps the derived atoms that say which nodes or spans it is (decision 0081).
        self.span_context = true;
        let ctx = self.context(std::slice::from_ref(v));
        self.span_context = false;
        self.siblings = saved;
        let ctx = ctx?;
        let name = self.fresh("mall");
        self.match_all_native(&name, &ctx.rel, re, &gs, !positions.is_empty());
        let args = std::iter::once(Term::Var(v.clone().into())).chain(gs.iter().map(|g| Term::Var(g.clone().into())))
            .map(Ok).chain(positions.iter().map(|p| self.term(p))).collect::<Result<Vec<_>, Error>>()?;
        self.emit(Literal::Atom(Atom { rel: name.into(), args: args.into(), neg: false, gated: false }))
    }

    /// `match_all("re", n, m)` over node n: each match j of re in n's source is `rx_span(re, n, j, "", m)`, and each
    /// named group g of that match `rx_span(re, n, j, g, g)`, as `match_all` over a span binds them.
    fn node_matches(&mut self, args: &[At]) -> Result<(), Error> {
        let ([re, n, m], ) = (args,) else { unreachable!("a node match_all") };
        let (T::Str(re), T::Var(n)) = (self.tv(re), self.tv(n)) else { unreachable!("a node match_all") };
        let (re, n) = (&re, &n);
        self.rx_span_input();
        let atom = |args: Vec<Term>| Literal::Atom(Atom { rel: "rx_span".into(), args: args.into(), neg: false, gated: false });
        let (p, c, j) = (Term::Str(re.clone().into()), Term::Var(n.clone().into()), Term::Var(self.fresh("j").into()));
        let m = self.term(m)?;
        self.emit(atom(vec![p.clone(), c.clone(), j.clone(), Term::Str(String::new().into()), m]))?;
        for g in groups(re) {
            self.emit(atom(vec![p.clone(), c.clone(), j.clone(), Term::Str(g.clone().into()), Term::Var(g.into())]))?;
        }
        Ok(())
    }

    /// A text test (`match`, `glob`, `contains`, …) on node or span `v`: a test of its text. `match` over a `source`
    /// or `src_line` span is `rx_span`'s whole match of the anchored pattern, a scan of the file's bytes, so a line
    /// past `text`'s inline bound is tested too; any other is `text(v, t)` and the test on t, so a node with no
    /// `text` row (a long one, not token-like) passes neither the test nor its negation.
    fn text_test(&mut self, rel: &str, args: &[At], neg: bool) -> Result<(), Error> {
        let [p, v] = args else { unreachable!("a test of a variable") };
        let T::Var(v) = self.tv(v) else { unreachable!("a test of a variable") };
        let v = &v;
        if test_of(rel) == Some("rx")
            && let T::Str(re) = self.tv(p)
            && self.span_container(v).is_some()
        {
            self.rx_span_input();
            let args = vec![Term::Str(format!("^(?:{re})$").into()), Term::Var(v.clone().into()), Term::Wild, Term::Str(String::new().into()), Term::Wild];
            return self.emit(Literal::Atom(Atom { rel: "rx_span".into(), args: args.into(), neg, gated: false }));
        }
        let t = self.fresh("text");
        let text = Literal::Atom(Atom { rel: "text".into(), args: [Term::Var(v.clone().into()), Term::Var(t.clone().into())].into(), neg: false, gated: false });
        let test = self.atom(rel, &[p.clone(), At::Var(t)], neg)?;
        self.emit(text)?;
        self.emit(test)
    }

    /// ``n ~ `pattern` ``: node n is a match of the AST pattern (ast-grep's syntax, in the program's language), and
    /// each metavariable `$X` binds variable X to the node it captured. The batch matches the pattern once over each
    /// document (`core_rr::CoreRows::patterns`) and makes `ast_match(p, n, g, c)`: capture g of the match at n is
    /// node c, and capture "" is n itself. A negated pattern binds nothing: no match at n. A `$$$X` (a run of
    /// nodes) and a `$_X` capture nothing.
    fn pattern(&mut self, subject: &At, pattern: &str, selector: Option<&str>, neg: bool) -> Result<(), Error> {
        let key = crate::code_rule_dsl::language::pattern_key(pattern, selector);
        let T::Var(n) = self.tv(subject) else {
            return Err(Error::Syntax(format!("`{pattern}`: a pattern's subject is a variable")));
        };
        if !self.prog.has_input("ast_match") {
            self.prog.add_input("ast_match");
            let cols = [("p", "sym"), ("n", "node"), ("g", "sym"), ("c", "node")];
            self.prog.declare_cols("ast_match", cols.iter().copied());
        }
        let atom = |g: &str, c: Term, neg: bool| {
            Literal::Atom(Atom { rel: "ast_match".into(), args: [Term::Str(key.clone().into()), Term::Var(n.clone().into()), Term::Str(g.to_owned().into()), c].into(), neg, gated: false })
        };
        if neg {
            return self.emit(atom("", Term::Wild, true));
        }
        self.emit(atom("", Term::Wild, false))?;
        for x in super::single_metavars(pattern) {
            self.emit(atom(&x, Term::Var(x.clone().into()), false))?;
        }
        Ok(())
    }

    /// `rx_span` as an input of the program (`facts::vtab::syntax`'s `Extra` makes it).
    fn rx_span_input(&mut self) {
        if !self.prog.has_input("rx_span") {
            self.prog.add_input("rx_span");
            let cols = [("p", "sym"), ("c", "node"), ("j", "int"), ("g", "sym"), ("m", "node")];
            self.prog.declare_cols("rx_span", cols.iter().copied());
        }
    }

    /// Whether a sibling binds `v` to a node of a named kind other than a comment: `kind(v, "K")`.
    fn kind_bound(&self, v: &str) -> bool {
        self.siblings.iter().any(|l| match self.lit(l) {
            L::Atom { rel, args, neg: false } if rel == "kind" && args.len() == 2 => {
                matches!((self.tv(&args[0]), self.tv(&args[1])), (T::Var(x), T::Str(k)) if x == v && k != "comment")
            }
            _ => false,
        })
    }

    /// Whether a sibling binds `v` to a `source` or `src_line` span or a comment, the spans `rx_span` runs over.
    fn span_container(&self, v: &str) -> Option<SpanIn> {
        self.siblings.iter().find_map(|l| match self.lit(l) {
            L::Atom { rel, args, neg: false } => {
                let is = |a: &At| matches!(self.tv(a), T::Var(x) if x == v);
                match (rel.as_str(), args.as_slice()) {
                    ("source", [_, x]) if is(x) => Some(SpanIn::Source),
                    ("src_line", [_, _, x]) if is(x) => Some(SpanIn::Line),
                    ("kind", [x, k]) if is(x) && matches!(self.tv(k), T::Str(k) if k == "comment") => Some(SpanIn::Comment),
                    _ => None,
                }
            }
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
        self.siblings.iter().any(|l| match self.lit(l) {
            L::Atom { rel, args, neg: false } if depth < 4 && matches!(rel.as_str(), "match_all" | "match") => match (args.first().map(|a| self.tv(a)), args.get(1).map(|a| self.tv(a))) {
                (Some(T::Str(re)), Some(T::Var(s))) if s != v && groups(&re).iter().any(|g| g == v) => self.span_container(&s).is_some() || self.node_typed_at(&s, depth + 1),
                _ => false,
            },
            L::Atom { .. } | L::Group { .. } => self.node_in(l, v),
            _ => false,
        })
    }

    /// Whether `l` binds `v` at a node column: a positive atom, or alternatives that each do
    /// (`(php_arg(c, 1, l) ; php_named_arg(c, "level", l)), match("…", l)` tests l's text, not its id).
    fn node_in(&self, l: &LitAt, v: &str) -> bool {
        match self.lit(l) {
            L::Atom { rel, args, neg: false } => self.prog.columns_of(rel.as_str()).is_some_and(|cols| {
                args.iter().zip(cols).any(|(a, (_, t))| matches!(self.tv(a), T::Var(x) if x == v) && t == "node")
            }),
            L::Group { alts, neg: false } => !alts.is_empty() && alts.iter().all(|alt| alt.iter().any(|l| self.node_in(l, v))),
            _ => false,
        }
    }

    /// `match("(?<x>…)", s)`: s matches, and each named group binds the variable of its name to its text:
    /// `x = @rx_group(s, "^(?:…)$", "x")`, which has no value when s does not match (`strings::group`).
    fn captures(&mut self, args: &[At]) -> Result<(), Error> {
        let [re, s] = args else { unreachable!("match with a pattern") };
        let T::Str(re) = self.tv(re) else { unreachable!("match with a pattern") };
        let re = &re;
        let whole = Term::Str(format!("^(?:{re})$").into());
        let s = self.term(s)?;
        // The test as well as the groups: a group the head alone reads is a column, which SQL does not filter on, so
        // a text that does not match would make a row with no value (`pct_rest`'s `%%` step).
        self.emit(Literal::Cmp { l: Term::Call("rx".into(), [s.clone(), whole.clone()].into()), op: "=", r: Term::Int(1) })?;
        for g in groups(re) {
            self.emit(Literal::Cmp { l: Term::Var(g.clone().into()), op: "=", r: Term::Call("rx_group".into(), [s.clone(), whole.clone(), Term::Str(g.into())].into()) })?;
        }
        Ok(())
    }

    /// `(glob("a", s) ; glob("b", s))` as one test of the set, and `(s = "a" ; s = "b")` as one membership. `neg`: the
    /// disjunction is negated.
    fn test_set(&mut self, alts: &[Vec<LitAt>], neg: bool) -> Result<Option<Vec<Literal>>, Error> {
        if alts.len() < 2 || alts.iter().any(|a| a.len() != 1) {
            return Ok(None);
        }
        let tests: Option<Vec<(&'static str, At, String)>> = alts
            .iter()
            .map(|a| match self.lit(&a[0]) {
                L::Atom { rel, args, neg: false } => match (test_of(&rel), args.as_slice()) {
                    (Some(op), [p, s]) if op != "rx" => match self.tv(p) {
                        T::Str(p) => Some((op, s.clone(), p)),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            })
            .collect();
        if let Some(tests) = tests
            && tests.iter().all(|(op, s, _)| *op == tests[0].0 && self.same(s, &tests[0].1))
        {
            // On a node or span, the tests are of its text, as one test's are (`text_test`).
            let mut out = Vec::new();
            let s = match self.tv(&tests[0].1) {
                T::Var(v) if self.span_container(&v).is_some() || self.node_typed(&v) => {
                    let t = self.fresh("text");
                    out.push(Literal::Atom(Atom { rel: "text".into(), args: [Term::Var(v.into()), Term::Var(t.clone().into())].into(), neg: false, gated: false }));
                    Term::Var(t.into())
                }
                _ => self.term(&tests[0].1)?,
            };
            let set = Term::Call(SET.to_owned().into(), tests.iter().map(|(_, _, p)| Term::Str(p.clone().into())).collect());
            out.push(Literal::Cmp { l: Term::Call(tests[0].0.to_owned().into(), [s, set].into()), op: "=", r: Term::Int(1) });
            return Ok(Some(out));
        }
        let members: Option<Vec<(String, String)>> = alts
            .iter()
            .map(|a| match self.lit(&a[0]) {
                L::Cmp { l, op: "=", r } => match (self.tv(&l), self.tv(&r)) {
                    (T::Var(x), T::Str(c)) | (T::Str(c), T::Var(x)) => Some((x, c)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        // Only a test when something else binds x: `kind(c, k), (x = "a" ; x = "b")` gives x each value. Negated it
        // binds nothing, so it is a test (`key = cat(q, s), !(key = "a" ; key = "b")`).
        if let Some(members) = members
            && members.iter().all(|(x, _)| *x == members[0].0)
            && (neg || self.bound_beside(&members[0].0))
        {
            let set = Term::Call(SET.to_owned().into(), members.iter().map(|(_, c)| Term::Str(c.clone().into())).collect());
            return Ok(Some(vec![Literal::Cmp { l: Term::Call("equals".into(), [Term::Var(members[0].0.clone().into()), set].into()), op: "=", r: Term::Int(1) }]));
        }
        Ok(None)
    }

    /// A set argument's relation: a relation's name (of kinds, `kind_sets` tells, or of nodes, `node_set`), or one
    /// kind as its own set.
    fn set_name(&mut self, rel: &str, arg: &At) -> Result<String, Error> {
        match self.tv(arg) {
            T::Var(name) => {
                self.sets.insert(name.clone());
                Ok(name)
            }
            T::Str(kind) => {
                let name = kind_set_name(&kind);
                self.prog.set_kindset(&name, vec![kind]);
                Ok(name)
            }
            _ => Err(Error::Syntax(format!("{rel}: the set is a relation of kinds or one kind"))),
        }
    }

    fn atom(&mut self, rel: &str, args: &[At], neg: bool) -> Result<Literal, Error> {
        let var = |me: &Self, t: &At| match me.tv(t) {
            T::Var(v) => Ok(v),
            t => Err(Error::Syntax(format!("{rel}: expected a variable, got {t:?}"))),
        };
        // A relation an argument names (`dfs_first`'s edge, `components`' own), a string a constant gives.
        let name_of = |me: &Self, t: &At, what: &str| match me.tv(t) {
            T::Var(v) => Ok(v),
            _ => Err(Error::Syntax(what.to_owned())),
        };
        if let Some(op) = test_of(rel) {
            let [p, s] = args else { return Err(Error::Syntax(format!("{rel} takes a pattern and a string"))) };
            // The pattern where it is a constant.
            let mut constant = match self.tv(p) {
                T::Str(re) => Some(re),
                _ => None,
            };
            if op == "rx" && let Some(re) = &constant && !groups(re).is_empty() {
                // Negated, a match binds nothing: its named groups are plain groups, `!match(re, s)` as written.
                if !neg {
                    return not_yet(format!("match with named groups, {re:?}, here (`Lowerer::captures` lowers it in a body)"));
                }
                constant = Some(ungrouped(re));
            }
            // The scalar fallback treats an uncompiled matcher as false. A literal in
            // a policy must therefore be validated before it reaches that fallback.
            if op == "rx" && let Some(re) = &constant {
                crate::code_rule_dsl::logic::strings::Matcher::new("rx", &[re.clone()])
                    .map_err(|e| Error::Syntax(format!("match regex: {e}")))?;
            }
            let s = self.term(s)?;
            let mut p = match constant {
                Some(re) => Term::Str(re.into()),
                None => self.term(p)?,
            };
            // `match` is whole-string, as in Soufflé; the engine's `rx` searches. A pattern the rule reads (an
            // option's) is anchored as it runs.
            if op == "rx" {
                p = match p {
                    Term::Str(re) => Term::Str(format!("^(?:{re})$").into()),
                    other => anchored(other),
                };
            }
            return Ok(Literal::Cmp { l: Term::Call(op.to_owned().into(), [s, p].into()), op: "=", r: Term::Int(if neg { 0 } else { 1 }) });
        }
        match (rel, args) {
            // `nearest(n, _, set)`: n has an ancestor in the set, which is `inside(n, _, set)`; and negated.
            ("nearest", [n, a, set]) if matches!(self.tv(a), T::Wild) => {
                let set = self.set_name(rel, set)?;
                return Ok(Literal::Atom(Atom { rel: format!("{AXIS}{set}").into(), args: [self.term(n)?, Term::Wild].into(), neg, gated: false }));
            }
            ("nearest", [n, a, set]) if !neg => {
                let kindset = self.set_name(rel, set)?;
                return Ok(Literal::Nearest { n: var(self, n)?, a: var(self, a)?, kindset });
            }
            // `!nearest(n, s, set)` with s named: not that nearest, through a helper relation over (n, s).
            ("nearest", [n, a, set]) => {
                let kindset = self.set_name(rel, set)?;
                let (n, a) = (var(self, n)?, var(self, a)?);
                let shared = [n.clone(), a.clone()];
                return self.negate(vec![vec![Literal::Nearest { n, a, kindset }]], &shared);
            }
            // `inside(n, a, set)`: a is an ancestor of n in the set; `has(a, n, set)`: n is a descendant of a in the
            // set. Both are `(n, a)` rows the transducer makes.
            ("inside" | "has", [x, y, set]) => {
                let set = self.set_name(rel, set)?;
                let (x, y) = (self.term(x)?, self.term(y)?);
                let (axis, args) = if rel == "inside" { (AXIS, vec![x, y]) } else { (DESCENDANTS, vec![y, x]) };
                return Ok(Literal::Atom(Atom { rel: format!("{axis}{set}").into(), args: args.into(), neg, gated: false }));
            }
            ("ast", _) if !neg => return self.ast(args),
            ("precedes_stop", [a, b, set]) => {
                let rel = self.precedes_stop(set);
                return Ok(Literal::Atom(Atom { rel: rel.into(), args: [self.term(a)?, self.term(b)?].into(), neg, gated: false }));
            }
            ("dfs_first", [r, hit, first, edge, target]) | ("dfs_first_memo", [r, hit, first, edge, target, _]) => {
                let what = format!("{rel}: the edge, target and roots are relations, by name");
                let (edge, target) = (name_of(self, edge, &what)?, name_of(self, target, &what)?);
                let roots = if rel == "dfs_first_memo" { Some(name_of(self, &args[5], &what)?) } else { None };
                let rel = self.dfs_first(&edge, &target, roots.as_deref());
                return Ok(Literal::Atom(Atom { rel: rel.into(), args: [self.term(r)?, self.term(hit)?, self.term(first)?].into(), neg, gated: false }));
            }
            ("python_candidates", [call,report,reference,receiver,input,targets,context,member]) => {
                let name=|t:&At|name_of(self, t, "Python candidate inputs are relation names");
                let literal=|t:&At|match self.tv(t){T::Str(v)=>Ok(v),_=>Err(Error::Syntax("Python candidate context and member must be string literals".into()))};
                let (input,targets,context,member)=(name(input)?,name(targets)?,literal(context)?,literal(member)?);
                if !crate::code_rule_dsl::logic::front::PYTHON_CONTEXTS.contains(&context.as_str()) {return Err(Error::Syntax(format!("unknown Python execution context {context}")));}
                for name in [&input,&targets] {
                    if !self.prog.columns_of(name.as_str()).is_some_and(|cols| cols.len() == 1) {return Err(Error::Syntax(format!("python_candidates: {name} must be a declared unary relation")));}
                }
                let native=Native::PythonCandidates{input:input.clone(),targets:targets.clone(),context,member};
                let natives = self.prog.natives();
                let rel=if let Some((name,_))=natives.iter().find(|(_,n)|**n==native) {name.clone()}else {
                    let mut serial=natives.len();
                    let rel=loop {let name=format!("rr_python_candidates_{serial}");if !self.prog.declares(&name) && !natives.contains_key(name.as_str()){break name;}serial+=1;};
                    self.python_candidates_native(&rel, native);
                    rel
                };
                return Ok(Literal::Atom(Atom{rel: rel.into(),args:[call,report,reference,receiver].into_iter().map(|a|self.term(a)).collect::<Result<Vec<_>,_>>()?.into(),neg,gated:false}));
            }
            ("python_qualified" | "python_origins" | "python_assigned" | "python_arguments", _) => {
                let kind=rel.strip_prefix("python_").unwrap();
                let width=if kind=="arguments" {5}else{2};
                let expected=width+1+usize::from(kind=="qualified");
                if args.len()!=expected {return Err(Error::Syntax(format!("{rel}: expected {expected} arguments")));}
                let what = "Python producer inputs are relation names";
                let input=name_of(self, &args[width], what)?;
                let targets=if kind=="qualified" {Some(name_of(self, &args[width+1], what)?)}else{None};
                let producer=self.python_producer(kind,&input,targets.as_deref());
                return Ok(Literal::Atom(Atom{rel:producer.into(),args:args[..width].iter().map(|a|self.term(a)).collect::<Result<Vec<_>,_>>()?.into(),neg,gated:false}));
            }
            ("components", [d, c, k, edge, own]) => {
                let what = "components: the edge and own relations are named";
                let (edge, own) = (name_of(self, edge, what)?, name_of(self, own, what)?);
                let rel = self.components(&edge, &own);
                return Ok(Literal::Atom(Atom { rel: rel.into(), args: [self.term(d)?, self.term(c)?, self.term(k)?].into(), neg, gated: false }));
            }
            _ => {}
        }
        if let Some((_, _)) = RELATION_ARG.iter().find(|(r, _)| *r == rel) {
            return not_yet(format!("the built-in {rel}"));
        }
        let args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
        Ok(Literal::Atom(Atom { rel: rel.to_owned().into(), args: args.into(), neg, gated: false }))
    }

    /// `name(c, f, r, o)`: `native`'s Python call candidates (`Native::PythonCandidates`), made natively. Its one rule
    /// reads the input and the targets, so every pass that follows rule bodies (shared library parts, strata, demand)
    /// makes them before the candidates; its node atoms are a locality proof only (the native contract emits nodes
    /// from these documents).
    fn python_candidates_native(&mut self, name: &str, native: Native) {
        use crate::code_rule_dsl::logic::program_image::T;
        let Native::PythonCandidates { input, targets, .. } = &native else { unreachable!("python candidates") };
        self.prog.declare_cols(name, ["c", "f", "r", "o"].map(|n| (n, "node")));
        let p = &mut self.prog;
        let mut o = p.open_rule();
        p.put_head(&mut o, name, &["c", "f", "r", "o"].map(T::Var));
        p.put_atom(&mut o, input.as_str(), &[T::Var("d")], false, false);
        p.put_atom(&mut o, targets.as_str(), &[T::Var("q")], false, false);
        for n in ["c", "f", "r", "o"] {
            p.put_atom(&mut o, "node", &[T::Var("d"), T::Var(n), T::Wild, T::Wild, T::Wild], false, false);
        }
        self.close(o);
        self.prog.set_native(name, native);
    }

    fn python_producer(&mut self, kind:&str, input:&str, targets:Option<&str>)->String {
        let native = Native::Python {kind:kind.into(),input:input.into(),targets:targets.map(str::to_owned)};
        let base = format!("rr_python_{kind}_{input}_{}",targets.unwrap_or(""));
        let mut rel = base.clone();
        let mut collision = 0;
        let natives = self.prog.natives();
        while let Some(existing) = natives.get(&rel) {
            if existing == &native { return rel; }
            collision += 1;
            rel = format!("{base}__{collision}");
        }
        let vars=if kind=="arguments" {vec![("n","node"),("i","int"),("keyword","sym"),("spread","sym"),("value","node")]}else if kind=="qualified"{vec![("n","node"),("q","sym")]}else{vec![("n","node"),("value","node")]};
        use crate::code_rule_dsl::logic::program_image::{CmpOp, T};
        self.prog.declare_cols(&rel, vars.iter().map(|(n, t)| (n.to_string(), t.to_string())));
        let p = &mut self.prog;
        let mut o = p.open_rule();
        p.put_head(&mut o, rel.as_str(), &vars.iter().map(|(n, _)| T::Var(n)).collect::<Vec<_>>());
        p.put_atom(&mut o, input, &[T::Var("n")], false, false);
        p.put_atom(&mut o, "node", &[T::Var("d"), T::Var("n"), T::Wild, T::Wild, T::Wild], false, false);
        if let Some(t) = targets {
            p.put_atom(&mut o, t, &[T::Var("q")], false, false);
        } else {
            p.put_atom(&mut o, "node", &[T::Var("d"), T::Var("value"), T::Wild, T::Wild, T::Wild], false, false);
            if kind == "arguments" {
                for (name, value) in [("i", T::Int(0)), ("keyword", T::Str("")), ("spread", T::Str(""))] {
                    p.put_cmp(&mut o, T::Var(name), CmpOp::Eq, value);
                }
            }
        }
        self.close(o);
        self.prog.set_native(&rel, native);
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
        if self.prog.natives().contains_key(rel.as_str()) {
            return rel;
        }
        use crate::code_rule_dsl::logic::program_image::T;
        let p = &mut self.prog;
        let mut o = p.open_rule();
        p.put_head(&mut o, rel.as_str(), &[T::Var("r"), T::Var("hit"), T::Var("first")]);
        p.put_atom(&mut o, edge, &[T::Var("r"), T::Var("hit"), T::Var("first")], false, false);
        p.put_atom(&mut o, target, &[T::Var("hit")], false, false);
        if let Some(roots) = roots {
            p.put_atom(&mut o, roots, &[T::Var("r"), T::Wild], false, false);
        }
        self.close(o);
        // The edge relation's column types (a node, a number, a symbol); nodes where it is declared later.
        let edge_cols = self.prog.columns_of(edge);
        let ty = |c: usize, or: &str| edge_cols.as_ref().and_then(|cols| cols.get(c)).map_or_else(|| or.to_owned(), |(_, t)| t.clone());
        let cols = vec![("r".to_owned(), ty(0, "node")), ("hit".to_owned(), ty(1, "node")), ("first".to_owned(), ty(2, "int"))];
        self.prog.declare_cols(&rel, cols.iter().map(|(c, t)| (c.to_string(), t.to_string())));
        self.prog.set_native(&rel, Native::DfsFirst { edge: edge.to_owned(), target: target.to_owned(), roots: roots.map(str::to_owned) });
        rel
    }

    /// `components(_, _, _, edge, own)`'s relation `rr_components_<edge>_<own>`: `(d, c, key)`, made natively
    /// (`front::Native::Components`). Its one rule reads both inputs and joins two documents, so it is evaluated
    /// after them and in the cross-file part; the evaluator makes its rows, not the rule.
    fn components(&mut self, edge: &str, own: &str) -> String {
        let rel = format!("rr_components_{edge}_{own}");
        if self.prog.natives().contains_key(rel.as_str()) {
            return rel;
        }
        use crate::code_rule_dsl::logic::program_image::T;
        let p = &mut self.prog;
        let mut o = p.open_rule();
        p.put_head(&mut o, rel.as_str(), &[T::Var("d"), T::Var("e"), T::Var("k")]);
        p.put_atom(&mut o, own, &[T::Var("d"), T::Var("k")], false, false);
        p.put_atom(&mut o, edge, &[T::Var("d"), T::Var("e")], false, false);
        p.put_atom(&mut o, own, &[T::Var("e"), T::Wild], false, false);
        self.close(o);
        let cols = vec![("d".to_owned(), "int".to_owned()), ("c".to_owned(), "int".to_owned()), ("k".to_owned(), "sym".to_owned())];
        self.prog.declare_cols(&rel, cols.iter().map(|(c, t)| (c.to_string(), t.to_string())));
        self.prog.set_native(&rel, Native::Components { edge: edge.to_owned(), own: own.to_owned() });
        rel
    }

    /// `ast(d, n, { … }, NAME: v, …)`, or `ast(n, { … }, …)` with its document left unnamed.
    fn ast(&mut self, args: &[At]) -> Result<Literal, Error> {
        let positional: Vec<T> = args.iter().map(|a| self.tv(a)).filter(|a| !matches!(a, T::Named(..))).collect();
        let (document, node, matcher) = match positional.as_slice() {
            [T::Var(d), T::Var(n), T::Json(m)] => (d.clone(), n.clone(), m.clone()),
            [T::Var(n), T::Json(m)] => (self.fresh("d"), n.clone(), m.clone()),
            _ => return Err(Error::Syntax("ast takes (document, node, { … }) or (node, { … })".to_owned())),
        };
        let captures = args
            .iter()
            .filter_map(|a| match self.tv(a) {
                T::Named(name, v) => Some(match self.tv(&v) {
                    T::Var(v) => Ok((name, v)),
                    other => Err(Error::Syntax(format!("ast capture {name}: expected a variable, got {other:?}"))),
                }),
                _ => None,
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Literal::Ast { document, node, matcher, captures })
    }

    /// Term `a` lowered; a whole number as a float where `float` (`comparison`: compared with a float).
    fn term_as(&mut self, a: &At, float: bool) -> Result<Term, Error> {
        match self.tv(a) {
            T::Num(n) if float => Ok(Term::Int(float_key(format!("{n}.0").parse::<f64>().map_err(|_| Error::Syntax(format!("bad decimal number {n}.0")))?))),
            _ => self.term(a),
        }
    }

    fn comparison(&mut self, l: &At, op: &'static str, r: &At, outer: &BTreeSet<String>) -> Result<(), Error> {
        // A float compared with a whole number: the number as a float (`to_float(t) > 86400`).
        let (lf, rf) = (self.is_float(l), self.is_float(r));
        let (l_float, r_float) = (!lf && rf, lf);
        match (self.tv(l), self.tv(r)) {
            (T::Var(v), T::Agg { op: agg, over, by, sep, body }) | (T::Agg { op: agg, over, by, sep, body }, T::Var(v)) if op == "=" => {
                let out = self.aggregate(&v, &agg, over.as_ref(), &by, &sep, &body, outer)?;
                out.into_iter().try_for_each(|l| self.emit(l))
            }
            (T::Agg { op: agg, over, by, sep, body }, _) | (_, T::Agg { op: agg, over, by, sep, body }) => {
                let left = matches!(self.tv(l), T::Agg { .. });
                let (other, float) = if left { (r, r_float) } else { (l, l_float) };
                let v = self.fresh("agg");
                let mut out = self.aggregate(&v, &agg, over.as_ref(), &by, &sep, &body, outer)?;
                let other = self.term_as(other, float)?;
                // `$v = 0` with `$v` unbound would assign it (`front::cmp_binds`), and the planner may place it
                // before the aggregate: an equality is written `0 = $v`, a test that waits for the aggregate.
                let (l, r) = if left && op != "=" { (Term::Var(v.into()), other) } else { (other, Term::Var(v.into())) };
                out.push(Literal::Cmp { l, op, r });
                out.into_iter().try_for_each(|l| self.emit(l))
            }
            _ => {
                let c = Literal::Cmp { l: self.term_as(l, l_float)?, op, r: self.term_as(r, r_float)? };
                self.emit(c)
            }
        }
    }



    /// A relation of `vars`' values in the rule: the literals beside the one being lowered (`siblings`; their
    /// atoms and comparisons), projected on `vars`. It binds, inside a helper rule, variables its body only tests.
    fn context(&mut self, vars: &[String]) -> Result<Atom, Error> {
        let siblings = std::mem::take(&mut self.siblings);
        let context = |aggregates: bool| -> Vec<LitAt> {
            siblings
                .iter()
                .filter(|l| match self.lit(l) {
                    // `unset(x)` is the empty value, lowered in a rule's own body only: a context leaves it out (and
                    // holds more rows).
                    L::Atom { neg, rel, .. } => !neg && rel != "unset",
                    L::Cmp { l, r, .. } => aggregates || !matches!(self.tv(&l), T::Agg { .. }) && !matches!(self.tv(&r), T::Agg { .. }),
                    // `(v = t ; within(v, t))`: it may be all that ties two asked variables (pyi047's unread_comp_var
                    // ties a comprehension c to its target v only so). `bound_context` keeps it once what it reads is bound.
                    L::Group { .. } => self.plain_group(l),
                    _ => false,
                })
                .cloned()
                .collect()
        };
        // Inputs alone when they bind the variables: a context of derived relations ties the helper to them, and
        // to their recursion (`type_app(t, h, i, a) :- ann_input(t), …, field(m, "subscript", a), i = count : {
        // field(m, "subscript", a0), a0 < a }` with ann_input recursive through type_app would aggregate in its
        // own component). The context may hold more rows than the rule; the rule joins it with its own literals.
        let inputs: BTreeSet<String> = self.prog.input_names().into_iter().collect();
        // A derived atom over the head's variables alone, not of the clause's own heads, is the clause's demand
        // (`read_elsewhere(n, S) :- demand_read_elsewhere(n, S), …`): it narrows the context to the rows the rule is
        // asked for. Without it, a context of inputs over `n` is every identifier of every scope (up028: 1.7 M rows
        // of 9,032 nodes, where the demand asks for a loop target's few).
        // So is the clause's first literal, where the translator puts a demand whose columns are not the head's.
        // Neither may depend on the clause's heads: through one, the context would sit in the heads' recursion.
        let (head_vars, heads_now, first_atom) = (self.head_vars.clone(), self.heads.clone(), self.first_atom.clone());
        let deps = &self.deps;
        let reaches_heads = |rel: &str| -> bool {
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            let mut todo = vec![rel];
            while let Some(r) = todo.pop() {
                if heads_now.contains(r) {
                    return true;
                }
                if seen.insert(r) {
                    todo.extend(deps.get(r).into_iter().flatten().map(String::as_str));
                }
            }
            false
        };
        let demand_of_head = |l: &LitAt| match self.lit(l) {
            L::Atom { rel, neg: false, args, .. } => {
                !reaches_heads(&rel)
                    && !args.is_empty()
                    && (first_atom.as_deref() == Some(rel.as_str()) || args.iter().all(|a| matches!(self.tv(a), T::Var(v) if head_vars.contains(&v))))
            }
            _ => false,
        };
        let input_only = |l: &LitAt| match self.lit(l) {
            L::Atom { rel, .. } if inputs.contains(&rel) => true,
            L::Atom { .. } => demand_of_head(l),
            L::Group { alts, .. } => alts.iter().flatten().all(|l| !matches!(self.lit(l), L::Atom { rel, .. } if !inputs.contains(&rel))),
            _ => true,
        };
        let base: Vec<LitAt> = context(false).into_iter().filter(|l| input_only(l)).collect();
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
                    let lv = self.vars_of(l);
                    if !kept[i] && !lv.is_disjoint(&reach) {
                        kept[i] = true;
                        reach.extend(lv);
                        grew = true;
                    }
                }
            }
            base.into_iter().zip(kept).filter_map(|(l, k)| k.then_some(l)).collect()
        };
        let (base, base_bound) = self.bound_context(base);
        // Aggregates stay out unless a variable needs one: `m = cat(…, body, …)` over `body = join … : { … }`.
        // So do atoms of the clause's own heads, where the rest binds the variables: through one, the aggregate
        // would sit in the head's recursion (`la_cb(n, e) :- …, la_cb(s, v), count : { la_vis(n, x, s2, …) } = 0`).
        // The context then holds more rows than the rule, which joins it with its own literals.
        let heads = self.heads.clone();
        let own = |l: &LitAt| matches!(self.lit(l), L::Atom { rel, .. } if heads.contains(&rel));
        let (first, bound) = self.bound_context(context(false).into_iter().filter(|l| !own(l)).collect());
        let (first, bound) = if vars.iter().all(|v| bound.contains(v)) { (first, bound) } else { self.bound_context(context(false)) };
        // A context whose literals leave the variables asked about in separate groups is their cross product (every
        // function times every name, where the derived atom that joins them is left out): the next choice is taken.
        let joined = |lits: &[LitAt]| {
            let Some(v0) = vars.first() else { return true };
            let mut reach: BTreeSet<String> = BTreeSet::from([v0.clone()]);
            let mut grew = true;
            while grew {
                grew = false;
                for l in lits.iter().filter(|l| matches!(self.lit(l), L::Atom { neg: false, .. }) || self.plain_group(l)) {
                    let lv = self.vars_of(l);
                    if !lv.is_disjoint(&reach) && !lv.is_subset(&reach) {
                        reach.extend(lv);
                        grew = true;
                    }
                }
            }
            vars.iter().all(|v| reach.contains(v))
        };
        let whole = if self.request_context { Some(self.bound_context(context(false))) } else { None };
        let context = if let Some((whole, _)) = whole.filter(|(_, b)| vars.iter().all(|v| b.contains(v))) {
            whole
        } else if !self.span_context && !self.request_context && vars.iter().all(|v| base_bound.contains(v)) && joined(&base) {
            base
        } else if vars.iter().all(|v| bound.contains(v)) {
            first
        } else {
            self.bound_context(context(true)).0
        };
        // Only the literals tied to `vars` through shared variables: one tied to none tests that some row exists
        // anywhere (`ann_root(_)` once `(s = a ; within(s, a))` is left out), which makes every row of the context
        // depend on every document. Leaving it out widens the context, which the rule joins with its own literals.
        let context = self.connected(context, vars);
        let context = self.placed_texts(context, vars);
        let context = self.connected(context, vars);
        let context = self.covering(context, vars)?;
        let ctx = self.fresh("ctx");
        let alts = self.conjunction(&context, &vars.iter().cloned().collect())?;
        let types = vars.iter().map(|c| (c.clone(), self.type_of(c, &alts))).collect();
        let args: Vec<Term> = vars.iter().map(|c| Term::Var(c.clone().into())).collect();
        let ctx = self.helper_once(ctx, "ctx", types, alts.into_iter().map(|b| (args.clone(), b)).collect());
        Ok(Atom { rel: ctx.into(), args: args.into(), neg: false, gated: false })
    }

    /// `context` without a `text(v, t)` whose node `v` nothing else in it binds: a text is read of a bound node
    /// (`t = @rr_node_text(v)` past the inline bound, decision 0109), and such a `v` is bound by the rule, not the
    /// context (`perf101_rebinding(n, NAME) :- demand(n, NAME), within_stop(c6, n, …), field(c6, "name", c7),
    /// text(c7, t), text(NAME, t)`, a context of `c6`). Left out, it widens the context, which the rule joins with
    /// its own literals.
    fn placed_texts(&self, context: Vec<LitAt>, vars: &[String]) -> Vec<LitAt> {
        let text_of = |l: &LitAt| match self.lit(l) {
            L::Atom { rel, neg: false, args, .. } if rel == "text" => match args.first().map(|a| self.tv(a)) {
                Some(T::Var(v)) => Some(v),
                _ => None,
            },
            _ => None,
        };
        let binders: Vec<LitAt> = context
            .iter()
            .filter(|l| matches!(self.lit(l), L::Atom { rel, neg: false, .. } if rel != "text" && test_of(&rel).is_none()))
            .cloned()
            .collect();
        let bound = self.body_vars(&binders);
        let kept = context.into_iter().filter(|l| text_of(l).is_none_or(|v| bound.contains(&v) || vars.contains(&v))).collect();
        // A comparison that read a node only the text bound goes with it (`@doc(FN) = @doc(BOUND)`).
        self.bound_context(kept).0
    }

    /// The literals of `context` reachable from `vars` through shared variables, in their order. A literal that
    /// ranges over a file's nodes (`docof(a, d)`, `node(d, a, …)`, `a` not asked about) is not reached through its
    /// document: tied to `vars` only so, it tests that some node exists in the same file, and joined in the context it
    /// is every pair of the file's nodes (`latest_alias`'s `max` beside `docof(a, d), docof(e, d)`: `docof(a, d),
    /// visit_index(a, _)` in the context of `(e, x)`, 18 M rows on 100 files). Left out, it widens the context, which
    /// the rule joins with its own literals. A relation of the document itself (`first_import(d, …)`) still is.
    fn connected(&self, context: Vec<LitAt>, vars: &[String]) -> Vec<LitAt> {
        let file_nodes = |l: &LitAt| -> Option<String> {
            let L::Atom { rel, args, neg: false } = self.lit(l) else { return None };
            let var = |k: usize| match args.get(k).map(|a| self.tv(a)) {
                Some(T::Var(v)) => Some(v),
                _ => None,
            };
            let (n, d) = match rel.as_str() {
                "docof" => (var(0)?, var(1)?),
                "node" => (var(1)?, var(0)?),
                _ => return None,
            };
            (!vars.contains(&n) && !vars.contains(&d)).then_some(d)
        };
        let mut reach: BTreeSet<String> = vars.iter().cloned().collect();
        let of: Vec<BTreeSet<String>> = context
            .iter()
            .map(|l| {
                let mut vs = self.vars_of(l);
                if let Some(d) = file_nodes(l) {
                    vs.remove(&d);
                }
                vs
            })
            .collect();
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

    /// A context's groups of literals (joined through variables other than `vars`), each joined once: the first
    /// group to bind a variable of `vars` as it is, a later one only as the asked variables it binds. Joined in
    /// whole, a later group multiplies the context's rows by its own: `within(c1, n), within(c2, n)` asked for `n`
    /// is every pair of n's descendants (1.3 G rows on 20 files). Left out, it no longer narrows the context, which
    /// then holds every row the first group does (O125's cold run 346 -> 437 G instructions). So such a group is a
    /// relation of its own over the asked variables it binds, a row per distinct binding, joined in its place. A
    /// group that reads an asked variable it does not bind, or binds none, stays as it is.
    fn covering(&mut self, context: Vec<LitAt>, vars: &[String]) -> Result<Vec<LitAt>, Error> {
        let asked: BTreeSet<String> = vars.iter().cloned().collect();
        let of: Vec<BTreeSet<String>> = context.iter().map(|l| self.vars_of(l)).collect();
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
        let groups: Vec<usize> = (0..context.len()).map(|k| root(&mut group, k)).collect();
        // The variables a binding atom binds: its plain arguments. One it computes from (`jb(@sub(p, 1), 92)`) it
        // reads: lowered, that is `t = @sub(p, 1), jb(t, 92)`, and a cover of it alone could not place `p`.
        let bound_by: Vec<BTreeSet<String>> = context
            .iter()
            .map(|l| match self.lit(l) {
                L::Atom { args, .. } if self.binding_atom(l) => args.iter().filter_map(|a| match self.tv(a) { T::Var(v) => Some(v), _ => None }).collect(),
                _ => BTreeSet::new(),
            })
            .collect();
        // Per group: the asked variables its binding atoms bind, and those any of its literals reads.
        let asked_in = |g: usize, only_binding: bool| -> BTreeSet<String> {
            (0..context.len())
                .filter(|&k| groups[k] == g)
                .flat_map(|k| {
                    let vars = if only_binding { bound_by[k].clone() } else { of[k].clone() };
                    vars.into_iter().filter(|v| asked.contains(v)).collect::<Vec<_>>()
                })
                .collect()
        };
        let mut covered: BTreeSet<String> = BTreeSet::new();
        // A group's place: kept whole, or joined as the asked variables it binds.
        let mut whole: BTreeSet<usize> = BTreeSet::new();
        let mut projected: Vec<(usize, Vec<String>)> = Vec::new();
        // Or left out: a group binding no asked variable (`root(a), (s = a ; within(s, a))` before `kind(s,
        // "string")` asked for s) only narrows what later groups bind, at the cost of its own rows.
        let mut left: BTreeSet<usize> = BTreeSet::new();
        for k in 0..context.len() {
            let g = groups[k];
            if whole.contains(&g) || left.contains(&g) || projected.iter().any(|(p, _)| *p == g) {
                continue;
            }
            let bound = asked_in(g, true);
            // A group naming two asked variables ties them (`child(c, fc, _), field(fc, "left", t), (v = t ;
            // within(v, t))` ties c to v): left out, the groups binding each are a product, c's rows times v's
            // (pyi047's unread_comp_var: 122 M rows, tens of GB). It stays.
            if bound.is_empty() && asked_in(g, false).len() <= 1 {
                left.insert(g);
            } else if bound.is_empty() || !bound.is_subset(&covered) {
                covered.extend(bound);
                whole.insert(g);
            } else if asked_in(g, false).is_subset(&bound) {
                projected.push((g, vars.iter().filter(|v| bound.contains(*v)).cloned().collect()));
            } else {
                whole.insert(g);
            }
        }
        // A variable no group binds is left to the context as it was.
        if !asked.is_subset(&covered) || (projected.is_empty() && left.is_empty()) {
            return Ok(context);
        }
        // A left-out group's tests of the requested keys alone stay: they introduce no witness cross product, and
        // without them annotation demand asks to parse every type expression.
        let membership: Vec<bool> = context
            .iter()
            .map(|l| match self.lit(l) {
                L::Atom { args, neg, .. } => self.vars_of(l).is_subset(&asked) && (neg || args.iter().all(|a| matches!(self.tv(a), T::Var(_) | T::Str(_) | T::Num(_) | T::Float(_)))),
                _ => false,
            })
            .collect();
        let mut members: BTreeMap<usize, Vec<LitAt>> = BTreeMap::new();
        for (l, g) in context.iter().zip(&groups) {
            if !whole.contains(g) && !left.contains(g) {
                members.entry(*g).or_default().push(l.clone());
            }
        }
        // A cover of two or more asked variables ties them through variables not asked (c414's `call_nkwargs(c, n),
        // call_nkwargs(s, n)`): alone it is every pair with the same `n`, 38.8 M rows on 50 files where the context
        // reaches a handful. Its body leads with the whole groups that bind its variables, so it holds only the
        // bindings the context can reach, still a row per binding.
        let whole_binding: Vec<(BTreeSet<String>, Vec<LitAt>)> = whole
            .iter()
            .map(|&g| {
                let lits = context.iter().zip(&groups).filter(|(_, h)| **h == g).map(|(l, _)| l.clone()).collect();
                (asked_in(g, true), lits)
            })
            .collect();
        let mut out = Vec::new();
        for ((l, g), member) in context.into_iter().zip(groups).zip(membership) {
            if whole.contains(&g) || (left.contains(&g) && member) {
                out.push(l);
                continue;
            }
            if left.contains(&g) {
                continue;
            }
            // In the first literal's place: the group as the asked variables it binds.
            let Some(lits) = members.remove(&g) else { continue };
            let cols = projected.iter().find(|(p, _)| *p == g).map(|(_, c)| c.clone()).expect("a projected group");
            let rel = self.fresh("cover");
            let lits = if cols.len() > 1 {
                let mut narrowed: Vec<LitAt> = whole_binding.iter().filter(|(b, _)| cols.iter().any(|c| b.contains(c))).flat_map(|(_, l)| l.iter().cloned()).collect();
                narrowed.extend(lits);
                narrowed
            } else {
                lits
            };
            let alts = self.conjunction(&lits, &cols.iter().cloned().collect())?;
            let types = cols.iter().map(|c| (c.clone(), self.type_of(c, &alts))).collect();
            let args: Vec<Term> = cols.iter().map(|c| Term::Var(c.clone().into())).collect();
            let rel = self.helper_once(rel, "cover", types, alts.into_iter().map(|b| (args.clone(), b)).collect());
            out.push(LitAt::Made(rel, cols));
        }
        Ok(out)
    }

    /// A context's atoms and the comparisons it can evaluate, in order, and the variables they bind.
    fn bound_context(&self, context: Vec<LitAt>) -> (Vec<LitAt>, BTreeSet<String>) {
        // A comparison stays only if the context binds what it reads (`k > 1` needs the count that binds k, which
        // is left out): the context may hold more rows than the rule, and the rule joins it with its own literals.
        let mut bound = self.body_vars(&context.iter().filter(|l| self.binding_atom(l)).cloned().collect::<Vec<_>>());
        let (atoms, mut cmps): (Vec<LitAt>, Vec<LitAt>) = context.into_iter().partition(|l| matches!(self.lit(l), L::Atom { .. }));
        let mut context = atoms;
        loop {
            let ready = cmps.iter().position(|c| self.cmp_ready(c, &bound));
            let Some(k) = ready else { break };
            let c = cmps.remove(k);
            bound.extend(self.vars_of(&c));
            context.push(c);
        }
        // A plain disjunction (`plain_group`) stays once each alternative grounds what it reads given `bound`; it binds
        // the variables every alternative binds. Otherwise it is left out, widening the context.
        let mut grew = true;
        while grew {
            grew = false;
            if let Some(k) = cmps.iter().position(|g| self.group_binds(g, &bound).is_some()) {
                let g = cmps.remove(k);
                bound.extend(self.group_binds(&g, &bound).expect("ready"));
                context.push(g);
                grew = true;
            }
        }
        (context, bound)
    }

    /// Whether comparison `c` can be evaluated given `bound`: an equality may bind one side from the other, any other
    /// comparison reads only bound variables.
    fn cmp_ready(&self, c: &LitAt, bound: &BTreeSet<String>) -> bool {
        let L::Cmp { l, op, r } = self.lit(c) else { return false };
        let unbound = |t: &At| match self.tv(t) {
            T::Var(x) if op == "=" && !bound.contains(&x) => true,
            _ => false,
        };
        if unbound(&l) {
            self.term_var_set(&r).is_subset(bound)
        } else if unbound(&r) {
            self.term_var_set(&l).is_subset(bound)
        } else {
            self.term_var_set(&l).union(&self.term_var_set(&r)).all(|v| bound.contains(v))
        }
    }

    /// `( a ; b )`, positive, each alternative only positive atoms (no text test, no `earlier`) and comparisons without
    /// aggregates: a disjunction a context can hold.
    fn plain_group(&self, l: &LitAt) -> bool {
        let L::Group { alts, neg: false } = self.lit(l) else { return false };
        !alts.is_empty()
            && alts.iter().all(|alt| {
                alt.iter().all(|l| match self.lit(l) {
                    // `unset(x)` is the empty value, lowered in a rule's own body only: no relation a context reads.
                    L::Atom { rel, neg: false, .. } => test_of(&rel).is_none() && rel != "earlier" && rel != "unset",
                    L::Cmp { l, r, .. } => !matches!(self.tv(&l), T::Agg { .. }) && !matches!(self.tv(&r), T::Agg { .. }),
                    _ => false,
                })
            })
    }

    /// The variables plain group `g` binds given `bound` (those every alternative binds), if each alternative grounds
    /// its comparisons: an equality may bind one side from the other, any other comparison reads only bound ones.
    fn group_binds(&self, g: &LitAt, bound: &BTreeSet<String>) -> Option<BTreeSet<String>> {
        let L::Group { alts, .. } = self.lit(g) else { return None };
        if !self.plain_group(g) {
            return None;
        }
        let mut all: Option<BTreeSet<String>> = None;
        for alt in &alts {
            let mut have = bound.clone();
            have.extend(self.body_vars(&alt.iter().filter(|l| matches!(self.lit(l), L::Atom { .. })).cloned().collect::<Vec<_>>()));
            let mut cmps: Vec<&LitAt> = alt.iter().filter(|l| matches!(self.lit(l), L::Cmp { .. })).collect();
            loop {
                let ready = cmps.iter().position(|c| self.cmp_ready(c, &have));
                let Some(k) = ready else { break };
                have.extend(self.vars_of(cmps.remove(k)));
            }
            if !cmps.is_empty() {
                return None;
            }
            all = Some(match all {
                None => have,
                Some(a) => a.intersection(&have).cloned().collect(),
            });
        }
        all
    }

    /// Whether a positive atom beside the literal being lowered (not a text test or `earlier`) binds `v`. `within`
    /// and `within_stop` bind both ends: they are the engine's relation (decision 0050).
    fn bound_beside(&self, v: &str) -> bool {
        let atoms: Vec<LitAt> = self.siblings.iter().filter(|l| self.binding_atom(l)).cloned().collect();
        let mut bound = self.body_vars(&atoms);
        // And what the rule's equalities bind from those, to a fixed point: `x = t` once t's variables are bound,
        // and a bound text split by `q = @dot(m, x)` (`cat`'s parts), as lowering writes `split_dot`: a printed
        // program binds by these what its source bound by an atom.
        loop {
            let before = bound.len();
            for l in &self.siblings {
                let L::Cmp { l, op: "=", r } = self.lit(l) else { continue };
                for (a, b) in [(&l, &r), (&r, &l)] {
                    let tb = self.term_var_set(b);
                    match (self.tv(a), self.tv(b)) {
                        (T::Var(x), _) if !bound.contains(&x) && !tb.contains(&x) && tb.iter().all(|y| bound.contains(y)) => {
                            bound.insert(x);
                        }
                        (T::Var(q), T::Call { name, functor: true, args }) if name == "dot" && bound.contains(&q) => {
                            for t in &args {
                                bound.extend(self.term_var_set(t));
                            }
                        }
                        _ => {}
                    }
                }
            }
            if bound.len() == before {
                break;
            }
        }
        bound.contains(v)
    }







    #[allow(clippy::too_many_arguments)]
    fn aggregate(&mut self, var: &str, agg: &str, over: Option<&At>, by: &[At], sep: &Option<String>, body: &[LitAt], outer: &BTreeSet<String>) -> Result<Vec<Literal>, Error> {
        // A value or key that is a term (`sum strlen(x) - strlen(a)`) is a variable the body binds.
        let mut body = body.to_vec();
        let mut variable = |t: &At, me: &mut Self| match me.tv(t) {
            T::Var(v) => v,
            _ => {
                let v = me.fresh("val");
                body.push(LitAt::Eq(v.clone(), t.clone()));
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
        let own = |v: &str| over.as_deref() == Some(v) || by.iter().any(|b| b == v);
        let ours = |v: &str| !outer.contains(v);
        // One plain atom: the engine's aggregate reads it. The built-ins lowering writes out (axes, core.rr's tree
        // arithmetic, …) are no relations of the program, so a body of one of them goes through the helper.
        if let [one] = body
            && let L::Atom { rel, args, neg: false } = self.lit(one)
            && test_of(&rel).is_none()
            && !matches!(rel.as_str(), "nearest" | "inside" | "has" | "split_dot" | "ast" | "within" | "within_stop" | "precedes_stop" | "match_all" | "match" | "unset")
            && !self.rewritten(&rel)
        {
            let Literal::Atom(mut atom) = self.atom(&rel, &args, false)? else { unreachable!("a plain atom") };
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
        //
        // A `_` in one of the body's atoms ranges over the rows as a variable does (Soufflé counts the tuples): for
        // `count`, `sum` and `product` it is named, so `count : { match_all(",", n, _) }` counts each match rather than
        // the one row the helper had without it.
        let named: Vec<LitAt>;
        let body = if matches!(op, AggOp::Count | AggOp::Sum | AggOp::Product) {
            let mut out = Vec::new();
            for l in body {
                match self.lit(l) {
                    L::Atom { rel, args, neg: false } if !matches!(rel.as_str(), "nearest" | "inside" | "has" | "split_dot" | "ast" | "within" | "within_stop" | "precedes_stop" | "unset") => {
                        let mut sub = None;
                        for a in &args {
                            if let At::Node(w, _) = a
                                && matches!(self.tv(a), T::Wild)
                            {
                                let v = self.fresh("any");
                                let base = reads::sub_of_lit(l);
                                let s = *sub.get_or_insert_with(|| self.new_sub(base));
                                self.set_sub(s, *w, v);
                            }
                        }
                        out.push(sub.map_or_else(|| l.clone(), |s| reads::with_sub(l, s)));
                    }
                    _ => out.push(l.clone()),
                }
            }
            named = out;
            named.as_slice()
        } else {
            body
        };
        let mut inner = self.body_vars(body);
        // A nested aggregate's variables that the rule shares are the helper's too: in `count : { ident(x), count : {
        // ref(n, f, x) } = 0 }`, the inner count is per (f, x), so the helper is a relation of f and x, and f is bound
        // inside it by the context. Without f, the inner count read it free, and the outer could not group by it.
        let mut nested = Vec::new();
        body.iter().for_each(|l| self.lit_vars(l, &mut nested));
        inner.extend(nested.into_iter().filter(|v| outer.contains(v)));
        // `match_all`'s groups are variables of the body too, named in its regex: each match is a row to count.
        for l in body {
            if let L::Atom { rel, args, neg: false } = self.lit(l)
                && rel == "match_all"
                && let Some(T::Str(re)) = args.first().map(|a| self.tv(a))
            {
                inner.extend(groups(&re));
            }
        }
        // In order of name, a helper's (`rr_any12`) by its number: linking renumbers a module's helpers after the
        // modules before it (decision 0246 §2), which keeps the order of numbers, not of their digits.
        let mut ordered: Vec<String> = inner.iter().cloned().collect();
        ordered.sort_by(|a, b| helper_order(a).cmp(&helper_order(b)));
        let cols: Vec<String> = ordered.iter().filter(|v| matches!(op, AggOp::Count | AggOp::Sum | AggOp::Product) || own(v) || outer.contains(*v)).cloned().collect();
        if let Some(v) = over.iter().chain(&by).find(|v| !cols.contains(v)) {
            return Err(Error::Syntax(format!("{agg}: {v} is not bound in its body")));
        }
        let name = self.fresh("agg");
        let mut bodies = self.conjunction(body, outer)?;
        // A shared variable the body only tests (`count : { slash_at(ps, j), i < j }`) is bound inside the helper by
        // a context relation: the rule's other literals, projected on those variables.
        // `match_all` over a text the rule binds reads it, as a test does: its spans or texts come from the context.
        let binders = |me: &Self, attributes: bool| -> BTreeSet<String> {
            me.body_vars(
                &body
                    .iter()
                    .filter(|l| me.binds_columns(l))
                    .filter(|l| !matches!(me.lit(l), L::Atom { rel, .. } if matches!(rel.as_str(), "match_all" | "match") || (!attributes && matches!(rel.as_str(), "node" | "line" | "end_line"))))
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        };
        // A node's attribute (`node`, `line`, `end_line`) holds of every node: a shared variable it alone binds ranges
        // over the corpus's nodes in the helper. One such variable is a row per node, as the body's other literals
        // allow; two or more are every pair of them (RUF018's `count : { node(_, c, …), node(_, R, …), s >= e, … }`, 53
        // M rows of 7,303 nodes), so they are bound by the context instead. Bound by the context, one alone is no
        // better and may be worse: a context tying it to another of the body's through a document (TRY004's
        // `node(d, n, …), seen_import(s, …), node(d, s, …)`) is a cover of every pair of a file's nodes, 386 k rows on
        // one file.
        let (with_attributes, without) = (binders(self, true), binders(self, false));
        let by_attributes = inner.iter().filter(|v| outer.contains(*v) && with_attributes.contains(*v) && !without.contains(*v)).count();
        let bound = if by_attributes >= 2 { without } else { with_attributes };
        let needed: Vec<String> = inner.iter().filter(|v| outer.contains(*v) && !bound.contains(*v)).cloned().collect();
        if !needed.is_empty() {
            // The context holds every variable the body shares with the rule, those the body binds too: projected on
            // the tested ones alone (`ctx(j)`), it crossed every row of the body (`child(al, a, k)`, every argument of
            // every call) with every j of the rule, 20.6M rows for 4,133 counts on 50 files.
            let shared: Vec<String> = ordered.iter().filter(|v| outer.contains(*v)).cloned().collect();
            let span_subject = body.iter().any(|l| matches!(self.lit(l), L::Atom { rel, args, .. } if matches!(rel.as_str(), "match_all" | "match") && matches!(args.get(1).map(|a| self.tv(a)), Some(T::Var(v)) if needed.contains(&v))));
            self.span_context = span_subject;
            let ctx = self.context(&shared);
            self.span_context = false;
            let ctx = ctx?;
            for b in &mut bodies {
                b.insert(0, Literal::Atom(ctx.clone()));
            }
        }
        let types = cols.iter().map(|c| (c.clone(), self.type_of(c, &bodies))).collect();
        let args: Vec<Term> = cols.iter().map(|c| Term::Var(c.clone().into())).collect();
        // The empty value inside the body is the constant, as in a rule's own body.
        let rules = bodies.into_iter().map(|b| unset(args.clone(), b)).collect::<Result<Vec<_>, _>>()?;
        // Shared only by the clauses of the same heads: aggregate demand narrows a helper by its readers' contexts
        // (`rr_demand_agg_…`), so one shared with a reader the program does not reach (`body_len`'s count beside
        // `catch_all_body`'s in b110) was computed for that reader's rows too. A relation's clauses are reached, and
        // kept (`rr::unreached_dropped`), together.
        let stem = format!("agg {}", self.heads.iter().cloned().collect::<Vec<_>>().join(","));
        let name = self.helper_once(name, &stem, types, rules);
        // The helper keeps every column (`count` counts distinct rows), but only the rule's variables group it.
        let args = cols.iter().map(|c| if outer.contains(c) || own(c) { Term::Var(c.clone().into()) } else { Term::Wild }).collect();
        Ok(vec![Literal::Agg { var: var.to_owned(), op, over, atom: Atom { rel: name.into(), args, neg: false, gated: false } }])
    }

    /// A helper column's type: the column it fills in the body's atoms, else the type of the aggregate or the
    /// computation that assigns it (`k = 0 - i` is a number, which a `join` orders by value).
    fn type_of(&self, v: &str, bodies: &[Vec<Literal>]) -> String {
        self.type_through(v, bodies, &mut BTreeSet::new())
    }

    /// `type_of`, following `x = y` between variables: a context's `$F = $n` gives F n's type (a node), not the
    /// symbol a column no literal types falls back to, which an ordered comparison refuses.
    fn type_through(&self, v: &str, bodies: &[Vec<Literal>], seen: &mut BTreeSet<String>) -> String {
        if !seen.insert(v.to_owned()) {
            return "sym".to_owned();
        }
        let mut alias: Option<String> = None;
        for literal in bodies.iter().flatten() {
            if let Literal::Cmp { l: Term::Var(x), op: "=", r: Term::Var(y) } = literal
                && alias.is_none()
            {
                if x == v && !seen.contains(y.as_str()) {
                    alias = Some(y.to_string());
                } else if y == v && !seen.contains(x.as_str()) {
                    alias = Some(x.to_string());
                }
            }
        }
        for literal in bodies.iter().flatten() {
            match literal {
                Literal::Atom(a) if !a.neg => {
                    for (i, t) in a.args.iter().enumerate() {
                        if *t == Term::Var(v.to_owned().into()) {
                            if crate::code_rule_dsl::logic::front::is_axis(&a.rel) {
                                return "node".to_owned();
                            }
                            if let Some(cols) = self.prog.columns_of(a.rel.as_str())
                                && let Some((_, ty)) = cols.get(i)
                            {
                                return ty.to_string();
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
                            let col = atom.args.iter().position(|t| over.as_ref().is_some_and(|o| *t == Term::Var(o.clone().into())));
                            col.and_then(|c| self.prog.columns_of(atom.rel.as_str())?.get(c).map(|(_, ty)| ty.clone())).unwrap_or_else(|| "int".to_owned())
                        }
                    };
                }
                Literal::Ast { document, .. } if document == v => return "int".to_owned(),
                Literal::Ast { node, captures, .. } if node == v || captures.iter().any(|(_, c)| c == v) => return "node".to_owned(),
                _ => {}
            }
        }
        match alias {
            Some(y) => self.type_through(&y, bodies, seen),
            None => "sym".to_owned(),
        }
    }

    fn term(&mut self, t: &At) -> Result<Term, Error> {
        Ok(match self.tv(t) {
            T::Var(v) if v == "top" && !self.top_is_var => Term::Top,
            T::Var(v) => Term::Var(v.into()),
            T::Wild => Term::Wild,
            T::Num(n) => Term::Int(n),
            T::Str(s) => Term::Str(s.into()),
            T::Call { name, functor: true, args } => {
                let mut args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
                // The data-layer programs' SQL builtins (decision 0107): the engine's functions of SQLite's meaning.
                if let Some((f, swap)) = crate::code_rule_dsl::logic::strings::functions::catalogue(&name, args.len()) {
                    if swap {
                        args.swap(0, 1);
                    }
                    return Ok(Term::Call(f.to_owned().into(), args.into()));
                }
                // `@printf` of a constant format: its pieces concatenated (`printf_pieces`), each one a function every
                // executor has.
                if name == "printf"
                    && let Some(Term::Str(format)) = args.first()
                    && let Some(t) = printf_pieces(format, &args[1..])
                {
                    return Ok(t);
                }
                // `@concat(a, b, c)` is `@concat(@concat(a, b), c)`, as Soufflé's `cat` is.
                if name == "concat" && args.len() > 2 {
                    let mut args = args.into_iter();
                    let first = args.next().expect("three or more");
                    return Ok(args.fold(first, |acc, a| Term::Call("concat".into(), [acc, a].into())));
                }
                Term::Call(if name == "dot" { "cat".into() } else { name.into() }, args.into())
            }
            T::Call { name, functor: false, args } => {
                // Soufflé's functions: `cat` joins strings (the engine's `cat` joins with a dot, so it is `concat`).
                // `to_string` of a float writes the float (`float_str`); `max` and `min` of two are SQL's.
                let f = match (name.as_str(), args.len()) {
                    ("cat", n) if n >= 2 => "concat",
                    ("to_string", 1) if self.is_float(&args[0]) => "float_str",
                    ("strlen" | "to_string" | "to_number" | "to_float", 1) => name.as_str(),
                    ("substr", 3) => "substr",
                    ("max" | "min", 2) => name.as_str(),
                    _ => return not_yet(format!("the built-in function {name}/{}", args.len())),
                };
                let f = f.to_owned();
                let args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
                // `cat(a, b, c)` is `cat(cat(a, b), c)`.
                if f == "concat" {
                    let mut args = args.into_iter();
                    let first = args.next().expect("two or more");
                    return Ok(args.fold(first, |acc, a| Term::Call("concat".into(), [acc, a].into())));
                }
                Term::Call(f.into(), args.into())
            }
            T::Bin { op, l, r } => {
                // Float arithmetic is on floats' keys (`strings::functions::float_key`), by functions of them.
                let float = self.is_float(&l) || self.is_float(&r);
                let f = match op {
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
                Term::Call(f.to_owned().into(), [self.term(&l)?, self.term(&r)?].into())
            }
            // `-t` is `0 - t`.
            T::Neg(x) => {
                let f = if self.is_float(&x) { "float_sub" } else { "sub" };
                Term::Call(f.to_owned().into(), [Term::Int(0), self.term(&x)?].into())
            }
            T::Agg { op, .. } => return not_yet(format!("the aggregate {op} as a term")),
            T::Tick(t) => return not_yet(format!("the template `{t}`")),
            // A decimal number is its float's key, which orders as the floats do.
            T::Float(f) => Term::Int(float_key(f.parse::<f64>().map_err(|_| Error::Syntax(format!("bad decimal number {f}")))?)),
            T::Json(_) | T::Named(..) => return Err(Error::Syntax("a JSON term or an `ast` capture outside `ast(…)`".to_owned())),
        })
    }




    /// `split_dot(q, m, x)`: q is m.x split at its last dot, so x has no dot: `q = @cat(m, x), !contains(".", x)`
    /// (a bound q splits at every dot, `@cat` run backwards, which takes variables: a constant is bound to one
    /// first). Negated, every column named: q is not m.x, or x has a dot.
    fn split_dot(&mut self, args: &[At], neg: bool, pick: usize) -> Result<usize, Error> {
        if args.len() != 3 {
            return not_yet(format!("split_dot/{}", args.len()));
        }
        if neg && args.iter().any(|a| matches!(self.tv(a), T::Wild)) {
            return not_yet("`!split_dot` with a column unnamed");
        }
        let (mut out, mut terms) = (Vec::new(), Vec::new());
        // The constants' parts, bound in the (one) positive alternative.
        for a in args {
            let t = self.term(a)?;
            terms.push(match t {
                Term::Var(_) => t,
                _ if neg => t,
                Term::Wild => Term::Var(self.fresh("part").into()),
                constant => {
                    let v = Term::Var(self.fresh("part").into());
                    out.push(Literal::Cmp { l: v.clone(), op: "=", r: constant });
                    v
                }
            });
        }
        let [q, m, x] = <[Term; 3]>::try_from(terms).expect("three columns");
        let dotted = |b: i64| Literal::Cmp { l: Term::Call("contains".into(), [x.clone(), Term::Str(".".into())].into()), op: "=", r: Term::Int(b) };
        let cat = Term::Call("cat".into(), [m, x.clone()].into());
        if neg {
            self.alt(0, pick, |me| me.emit(Literal::Cmp { l: q, op: "!=", r: cat }))?;
            self.alt(1, pick, |me| me.emit(dotted(1)))?;
            return Ok(2);
        }
        out.extend([Literal::Cmp { l: q, op: "=", r: cat }, dotted(0)]);
        self.alt(0, pick, |me| out.into_iter().try_for_each(|l| me.emit(l)))?;
        Ok(1)
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
    let vars = unset_vars(&body)?;
    head.iter_mut().for_each(|t| unset_term(t, &vars));
    let mut rest = Vec::with_capacity(body.len());
    for l in body {
        rest.extend(unset_literal(l, &vars)?);
    }
    Ok((head, rest))
}

/// The variables a body's `unset(x)` bind to the empty value.
fn unset_vars(body: &[Literal]) -> Result<BTreeSet<String>, Error> {
    let mut vars = BTreeSet::new();
    for l in body {
        if let Literal::Atom(a) = l
            && a.rel == "unset"
            && !a.neg
        {
            match a.args.as_slice() {
                [Term::Var(v)] => {
                    vars.insert(v.to_string());
                }
                [Term::Wild] => {}
                _ => return not_yet("`unset` of a value"),
            }
        }
    }
    Ok(vars)
}

/// `t` with each of `vars` the empty value.
fn unset_term(t: &mut Term, vars: &BTreeSet<String>) {
    match t {
        Term::Var(v) if vars.contains(v.as_str()) => *t = Term::Int(UNSET),
        Term::Call(_, args) => args.iter_mut().for_each(|a| unset_term(a, vars)),
        _ => {}
    }
}

/// Literal `l` of a body whose `unset(x)` bind `vars` (`unset_vars`) to the empty value: `unset(x)` goes, `!unset(x)`
/// is `x != -1`, each of `vars` is the empty value, and the empty value's text is "". `None` where it goes.
fn unset_literal(l: Literal, vars: &BTreeSet<String>) -> Result<Option<Literal>, Error> {
    let mut l = match l {
        Literal::Atom(a) if a.rel == "unset" && !a.neg => return Ok(None),
        Literal::Atom(a) if a.rel == "unset" => match a.args.as_slice() {
            [x @ Term::Var(_)] => Literal::Cmp { l: x.clone(), op: "!=", r: Term::Int(UNSET) },
            _ => return not_yet("`!unset` of a value"),
        },
        l => l,
    };
    if vars.is_empty() {
        return Ok(Some(l));
    }
    match &mut l {
        Literal::Atom(a) => a.args.iter_mut().for_each(|t| unset_term(t, vars)),
        Literal::Cmp { l, r, .. } => {
            unset_term(l, vars);
            unset_term(r, vars);
        }
        // An aggregate's body (its helper atom) reads the unset variable from outside: it reads the empty value
        // there too. Its result and `over` are its own, never unset.
        Literal::Agg { var, over, atom, .. } if !vars.contains(var) && over.as_ref().is_none_or(|o| !vars.contains(o)) => {
            atom.args.iter_mut().for_each(|t| unset_term(t, vars));
        }
        other if !other.vars().is_disjoint(vars) => return not_yet("`unset` in an aggregate, `nearest` or `ast`"),
        _ => {}
    }
    // The empty value's text is "".
    Ok(match l {
        Literal::Atom(a) if a.rel == "text" && a.args.first() == Some(&Term::Int(UNSET)) => {
            let t = a.args.get(1).cloned().unwrap_or(Term::Wild);
            match (t, a.neg) {
                (Term::Wild, false) => None,
                (Term::Wild, true) => Some(Literal::Cmp { l: Term::Int(1), op: "=", r: Term::Int(0) }),
                (t, neg) => Some(Literal::Cmp { l: t, op: if neg { "!=" } else { "=" }, r: Term::Str(String::new().into()) }),
            }
        }
        l => Some(l),
    })
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
pub(super) fn groups(re: &str) -> Vec<String> {
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
pub(super) const TAPE_OPS: [(&str, usize, &str, &[(&str, &str)]); 3] = [
    ("parse", 2, "parse_of", &[("x", "sym"), ("g", "sym"), ("t", "sym")]),
    ("unparse", 1, "unparse_of", &[("t", "sym"), ("x", "sym")]),
    ("splice", 3, "splice_of", &[("t", "sym"), ("n", "node"), ("t2", "sym"), ("t3", "sym")]),
];

/// The relations of the file system a rule may read (t21 E1), each with the request its reads make: a directory's
/// entries, `dir_entry(dir, name, is_dir)`, a file's lines, `file_line(path, i, text)` (from 1), and a file's content
/// digest, `file_digest(path, c)` (a cache key's, decision 0103: a library's tape is kept by it).
/// A per-node source attribute answered on request (decision 0174): its relation, keyed by its first column; its
/// request relation; what answers a request; and the input holding the rows still made eagerly (`value`: the literals
/// of documents W reads no values of; `token`: the anonymous children, whose text is their kind).
#[derive(Clone, Copy)]
pub struct OnRequest {
    pub rel: &'static str,
    pub req: &'static str,
    pub answer: Answer,
    pub eager: Option<&'static str>,
}

/// Whether `rel` is one the rewrite adds (`on_request::SourcesOnRequest`): a request, an attribute's eager half or named
/// children, or a widened copy. `core.rr`'s names hold them (it is read with the rewrite), and a program with a round
/// undone (`undo_round`) has none of that round's: a check that a program declares every core name passes over them.
pub fn made_on_request(rel: &str) -> bool {
    rel.starts_with("rr_wide_")
        || ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| {
            let named = match a.answer {
                Answer::Children { named } => Some(named),
                Answer::Function(_) => None,
            };
            a.req == rel || a.eager == Some(rel) || named == Some(rel)
        })
}

/// Whether rules of `head` are the on-request rewrite's own (decision 0174): an attribute's answer, its requests, a
/// widened copy, a request's readers' asks (decision 0291, `{req}_by_{reader}_{col}`, which `undo_on_request` removes). They are no reader of anything a program adds: a library's cone (`rr::with_cone`) is taken without
/// them, or it reached, from a relation a program adds to, the widened copies of its readers, their requests, the
/// attribute's answer and then every reader of the attribute (five of the Python pack's programs took 1,300 of the
/// library's rules each).
pub fn on_request_rule(head: &str) -> bool {
    head.starts_with("rr_wide_")
        || ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| a.rel == head || a.req == head || head.strip_prefix(a.req).is_some_and(|rest| rest.starts_with("_by_")))
}

/// What answers an attribute's request for a node.
#[derive(Clone, Copy)]
pub enum Answer {
    /// `a(n, x) :- req(n), x = @f(n)`.
    Function(&'static str),
    /// `token(p, i, s, a, b) :- req(p), named(p, i, c, a, b), node_text(c, s)`: the named children by node, their
    /// text `node_text`'s, itself answered on request in the next round.
    Children { named: &'static str },
}

/// The attributes, by round: an answer may read an attribute of a later round (`token` reads `node_text`).
pub const ON_REQUEST: [&[OnRequest]; 2] = [
    &[OnRequest { rel: "token", req: "rr_token_req", answer: Answer::Children { named: "rr_token_named" }, eager: Some("rr_token_anon") }],
    &[
        OnRequest { rel: "node_text", req: "rr_node_text_req", answer: Answer::Function("rr_node_text_any"), eager: None },
        OnRequest { rel: "value", req: "rr_value_req", answer: Answer::Function("rr_node_value"), eager: Some("rr_value_lit") },
        // A string literal's content: a derived value, computed where a rule asks for it (decision 0353 step 3).
        OnRequest { rel: "str", req: "rr_str_req", answer: Answer::Function("rr_node_str"), eager: None },
    ],
];

/// The rules `undone_on_request` reads and drops (decision 0281's path, step 7a-2).
pub trait RuleSet {
    /// Drops each rule whose head `gone` names.
    fn drop_heads(&mut self, gone: &dyn Fn(&str) -> bool);
    /// Whether a rule's body atom (not an aggregate's) reads `rel`.
    fn reads(&self, rel: &str) -> bool;
}

/// Version `v` as it was before its node attributes were answered on request (`on_request::SourcesOnRequest`;
/// `plan::corpus`'s `Versioned::undo_on_request`, at once): each attribute it answered on request an input again,
/// without its requests, answers, eager rows' rule and widened copies. For programs merged into one and lowered once
/// (`convert::overlay`): the requests are made over the merged program's readers.
pub fn undone_on_request(v: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Result<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, String> {
    undone_with(v, |head, rules| undo_on_request_in(head, rules))
}

/// Version `v` with `undo` (`undo_on_request_in`, `undo_round`) made of its fields and rules: the rules it drops by
/// head gone, its fields as it leaves them.
pub fn undone_with(
    v: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    undo: impl FnOnce(&mut dyn crate::code_rule_dsl::logic::program_image::fields::FieldsMut, &mut dyn RuleSet),
) -> Result<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, String> {
    let mut dropped = BTreeSet::new();
    let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(v);
    undo(&mut b, &mut KeptRules { v, dropped: &mut dropped });
    b.retain(|r| !dropped.contains(v.rel_name(r.head().rel())));
    Ok(b.finish()?.0)
}

/// A version's rules as `undone_on_request` reads and drops them: those of a head in `dropped` are gone.
pub struct KeptRules<'a> {
    pub v: &'a crate::code_rule_dsl::logic::program_image::Version,
    pub dropped: &'a mut BTreeSet<String>,
}

impl RuleSet for KeptRules<'_> {
    fn drop_heads(&mut self, gone: &dyn Fn(&str) -> bool) {
        for r in self.v.rules() {
            let head = self.v.rel_name(r.head().rel());
            if gone(head) {
                self.dropped.insert(head.to_owned());
            }
        }
    }
    fn reads(&self, rel: &str) -> bool {
        use crate::code_rule_dsl::logic::program_image::Lit;
        self.v
            .rules()
            .filter(|r| !self.dropped.contains(self.v.rel_name(r.head().rel())))
            .flat_map(|r| r.body())
            .any(|l| matches!(l.view(), Lit::Atom(a) if self.v.rel_name(a.rel()) == rel))
    }
}

/// `undone_on_request` of a program's fields `prog` and its rules `rules`.
pub fn undo_on_request_in(prog: &mut (impl crate::code_rule_dsl::logic::program_image::fields::FieldsMut + ?Sized), rules: &mut dyn RuleSet) {
    for round in ON_REQUEST.iter().rev() {
        undo_round(prog, rules, round);
    }
}

/// One round of `ON_REQUEST` undone (`undone_on_request`) of a program's fields `prog` and its rules `rules`: its
/// attributes inputs again, made for every node.
pub fn undo_round(prog: &mut (impl crate::code_rule_dsl::logic::program_image::fields::FieldsMut + ?Sized), rules: &mut dyn RuleSet, round: &[OnRequest]) {
    let answered: Vec<&OnRequest> = round.iter().filter(|a| prog.declares(a.req)).collect();
    if answered.is_empty() {
        return;
    }
    // A round's copies are named after its first attribute present (`Widen::new`): any of the round's.
    let wide: Vec<String> = round.iter().map(|a| format!("rr_wide_{}_", a.rel)).collect();
    // A request's narrowing relations (`<req>_by_<reader>_<col>`, decision 0291) go with it: made again with the
    // requests, over the readers then.
    let narrowing: Vec<String> = round.iter().map(|a| format!("{}_by_", a.req)).collect();
    let copy = |rel: &str| wide.iter().chain(&narrowing).any(|w| rel.starts_with(w.as_str()));
    let mut gone: BTreeSet<String> = BTreeSet::new();
    // A request's readers' asks (decision 0291, `{req}_by_{reader}_{col}`) go with it.
    let asks: Vec<String> = answered.iter().map(|a| format!("{}_by_", a.req)).collect();
    let demand = |rel: &str| asks.iter().any(|d| rel.starts_with(d.as_str()));
    for k in prog.declared_names().into_iter().filter(|k| demand(k)) {
        prog.undeclare(&k);
        gone.insert(k);
    }
    for a in answered {
        gone.insert(a.rel.to_owned());
        gone.insert(a.req.to_owned());
        if prog.declares(a.req) {
            prog.undeclare(a.req);
        }
        let named = match a.answer {
            Answer::Children { named } => Some(named),
            Answer::Function(_) => None,
        };
        for input in a.eager.into_iter().chain(named) {
            if prog.declares(input) {
                prog.undeclare(input);
            }
            if prog.input_names().iter().any(|i| i == input) {
                prog.retain_inputs(&|i: &str| i != input);
            }
        }
        prog.add_input(a.rel);
    }
    rules.drop_heads(&|head| gone.contains(head) || copy(head));
    for k in prog.declared_names().into_iter().filter(|k| copy(k)) {
        prog.undeclare(&k);
    }
    let asked = prog.asked();
    if asked.keys().any(|k| gone.contains(k) || copy(k)) {
        let kept: BTreeMap<String, Vec<BTreeSet<String>>> = asked.into_iter().filter(|(k, _)| !gone.contains(k) && !copy(k)).collect();
        prog.set_field(crate::code_rule_dsl::logic::program_facts::Field::Asked(&kept));
    }
    // A token's answer read `node_text`, declared for it where the program had none: unread now, it goes.
    if round.iter().any(|a| matches!(a.answer, Answer::Children { .. }) && gone.contains(a.rel))
        && !rules.reads("node_text")
    {
        prog.retain_inputs(&|i: &str| i != "node_text");
        if prog.declares("node_text") {
            prog.undeclare("node_text");
        }
    }
}

/// `l` with each variable `v` named `name(v)`.
fn rename_literal(l: &Literal, name: &dyn Fn(&str) -> String) -> Literal {
    fn term(t: &Term, name: &dyn Fn(&str) -> String) -> Term {
        match t {
            Term::Var(v) => Term::Var(name(v).as_str().into()),
            Term::Call(f, args) => Term::Call(f.clone(), args.iter().map(|a| term(a, name)).collect()),
            other => other.clone(),
        }
    }
    let atom = |a: &Atom| Atom { rel: a.rel.clone(), args: a.args.iter().map(|t| term(t, name)).collect(), neg: a.neg, gated: a.gated };
    match l {
        Literal::Atom(a) => Literal::Atom(atom(a)),
        Literal::Cmp { l, op, r } => Literal::Cmp { l: term(l, name), op, r: term(r, name) },
        Literal::Agg { var, op, over, atom: a } => Literal::Agg { var: name(var), op: op.clone(), over: over.as_deref().map(name), atom: atom(a) },
        Literal::Nearest { n, a, kindset } => Literal::Nearest { n: name(n), a: name(a), kindset: kindset.clone() },
        Literal::Ast { document, node, matcher, captures } => Literal::Ast { document: name(document), node: name(node), matcher: matcher.clone(), captures: captures.iter().map(|(c, v)| (c.clone(), name(v))).collect() },
    }
}

/// The file system's answers and their requests (t21 E1).
pub const FILE_SYSTEM: [(&str, &str); 3] = [("dir_entry", "rr_dir_req"), ("file_line", "rr_file_req"), ("file_digest", "rr_digest_req")];

/// Every answer of a file and its request: the file system's (`FILE_SYSTEM`), then each language's
/// (`answerers`), one row an answer's relation. A request of several relations has a row for each.
pub static WORLD: World = World;

/// `WORLD`'s rows. Made once the languages are installed, and not before: a process whose languages install after
/// its first program is lowered (a test runner with no load hook, where the first store opened installs them) kept
/// the file system's alone, and answered no language's request.
pub struct World;

impl World {
    pub fn iter(&self) -> std::slice::Iter<'static, (&'static str, &'static str)> {
        static FS: [(&str, &str); 3] = FILE_SYSTEM;
        static ALL: std::sync::OnceLock<Vec<(&'static str, &'static str)>> = std::sync::OnceLock::new();
        if !crate::code_rule_dsl::language::answerers_installed() {
            return FS.iter();
        }
        ALL.get_or_init(|| {
            let languages = crate::code_rule_dsl::language::answerers().iter().flat_map(|a| a.produces().iter().map(|r| (*r, a.request())));
            FILE_SYSTEM.into_iter().chain(languages).collect()
        })
        .iter()
    }
}

/// The request an answer's relation asks by its first column: the file system's, and a language's of one column. A
/// request of several columns is made by rules (`c_pp_tu_req(tu, spec)`).
fn world_demand(rel: &str) -> Option<&'static str> {
    WORLD.iter().find(|(r, _)| *r == rel).map(|(_, d)| *d).filter(|d| crate::code_rule_dsl::language::answerer(d).is_none_or(|a| a.key_width() == 1))
}





/// How many alternatives a conjunction multiplies into before a disjunction in it is outlined (`conjunction`).
const OUTLINE_PAST: usize = 16;

/// Whether `body`, as a rule body of its own, binds each of `vars`: by a positive atom, `nearest` from a bound
/// node, or an equality from bound ones, to a fixed point.
fn grounds(body: &[Literal], vars: &[String]) -> bool {
    let mut bound: BTreeSet<String> = BTreeSet::new();
    for l in body {
        if let Literal::Atom(a) = l
            && !a.neg
        {
            bound.extend(a.vars());
        }
    }
    loop {
        let before = bound.len();
        for l in body {
            match l {
                Literal::Cmp { l: Term::Var(a), op: "=", r } | Literal::Cmp { l: r, op: "=", r: Term::Var(a) } if !bound.contains(a.as_str()) => {
                    let mut vs = BTreeSet::new();
                    r.vars(&mut vs);
                    if vs.iter().all(|v| bound.contains(v)) {
                        bound.insert(a.to_string());
                    }
                }
                Literal::Nearest { n, a, .. } if bound.contains(n.as_str()) => {
                    bound.insert(a.to_string());
                }
                _ => {}
            }
        }
        if bound.len() == before {
            break;
        }
    }
    vars.iter().all(|v| bound.contains(v))
}



/// Adjacent negations of one test of one string, `!glob("a", s), !glob("b", s)`, as one negated test of the set.
pub fn merge_negated_tests(body: Vec<Literal>) -> Vec<Literal> {
    let mut out: Vec<Literal> = Vec::new();
    for literal in body {
        match out.last_mut() {
            Some(last) => {
                if let Some(l) = merge_negated_test(last, literal) {
                    out.push(l);
                }
            }
            None => out.push(literal),
        }
    }
    out
}

/// A negated test of a string, `!glob("a", s)` (`op(s, p) = 0`): its operation, string and pattern.
fn negated_test(l: &Literal) -> Option<(&str, &Term, &Term)> {
    match l {
        Literal::Cmp { l: Term::Call(op, args), op: "=", r: Term::Int(0) } if args.len() == 2 && crate::code_rule_dsl::logic::strings::is_test(op) && op != "rx" && op != "irx" => Some((op, &args[0], &args[1])),
        _ => None,
    }
}

/// `literal` merged into `last` where both negate one test of one string, the second of a constant pattern (`!glob("a",
/// s), !glob("b", s)` is one negated test of the set); else `literal` back, to follow it.
fn merge_negated_test(last: &mut Literal, literal: Literal) -> Option<Literal> {
    let mergeable = match (negated_test(&literal), negated_test(last)) {
        (Some((op, s, p)), Some((last_op, last_s, last_p))) => {
            op == last_op && s == last_s && matches!(p, Term::Str(_)) && (matches!(last_p, Term::Str(_)) || matches!(last_p, Term::Call(f, _) if f == SET))
        }
        _ => false,
    };
    if !mergeable {
        return Some(literal);
    }
    let (Literal::Cmp { l: Term::Call(op, args), .. }, Literal::Cmp { l: Term::Call(_, new), .. }) = (std::mem::replace(last, Literal::Cmp { l: Term::Wild, op: "=", r: Term::Wild }), literal) else { unreachable!("negated tests") };
    let [s, last_p]: [Term; 2] = args.to_vec().try_into().expect("a test of two");
    let p = new[1].clone();
    let mut patterns = match last_p {
        Term::Call(f, items) if f == SET => items,
        last_p => [last_p].into(),
    };
    patterns.push(p);
    *last = Literal::Cmp { l: Term::Call(op, [s, Term::Call(SET.to_owned().into(), patterns)].into()), op: "=", r: Term::Int(0) };
    None
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

/// `@printf(format, args…)` of a constant format as the concatenation of its pieces: literal text, `%d` an
/// integer's digits (`rr_to_string`), `%0Nd` its digits zero-padded to N columns, the sign first, never truncated
/// (`zero_pad`), `%s` a text as it is, `%%` a percent sign. `None`: another conversion, or not one argument each.
fn printf_pieces(format: &str, args: &[Term]) -> Option<Term> {
    let mut pieces: Vec<Term> = Vec::new();
    let mut literal = String::new();
    let mut args = args.iter();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            literal.push(c);
            continue;
        }
        let mut width = String::new();
        while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
            width.push(*d);
            chars.next();
        }
        let piece = match (chars.next()?, width.as_str()) {
            ('%', "") => {
                literal.push('%');
                continue;
            }
            ('d', "") => Term::Call("rr_to_string".into(), [args.next()?.clone()].into()),
            ('d', w) if w.starts_with('0') && w.len() > 1 => Term::Call("zero_pad".into(), [args.next()?.clone(), Term::Int(w[1..].parse().ok()?)].into()),
            ('s', "") => args.next()?.clone(),
            _ => return None,
        };
        if !literal.is_empty() {
            pieces.push(Term::Str(std::mem::take(&mut literal).into()));
        }
        pieces.push(piece);
    }
    if args.next().is_some() {
        return None;
    }
    if !literal.is_empty() || pieces.is_empty() {
        pieces.push(Term::Str(literal.into()));
    }
    let mut pieces = pieces.into_iter();
    let first = pieces.next()?;
    Some(pieces.fold(first, |acc, p| Term::Call("concat".into(), [acc, p].into())))
}

/// `prog` reading the base relation `rel` (`child`, `node`): declared as the engine declares it, and an input unless
/// a rule makes it (`derived`). For rules added to a program already read (`corpus::bridge`), as lowering adds them for
/// its own: `prog` may be a header alone.
pub fn base_relation_derived(prog: &mut (impl crate::code_rule_dsl::logic::program_image::fields::FieldsMut + ?Sized), rel: &str, derived: bool) {
    let cols: &[(&str, &str)] = match rel {
        "child" => &[("p", "node"), ("c", "node"), ("i", "int")],
        "node" => &[("d", "int"), ("n", "node"), ("kind", "sym"), ("start", "int"), ("end", "int")],
        other => unreachable!("no base relation {other}"),
    };
    prog.declare_absent(rel, cols);
    if !derived {
        prog.add_input(rel);
    }
}
