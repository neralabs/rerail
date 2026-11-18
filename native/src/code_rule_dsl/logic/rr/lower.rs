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

pub mod core;
#[cfg(any(test, feature = "test-hooks"))]
pub use self::core::drop_unread_core;
pub use self::core::is_core;

use std::collections::{BTreeMap, BTreeSet};

use rerail_rr_syntax::syntax::{Code, Fix, Head, Item, Lit, Statement, Suppress, Term as STerm, quote};
use crate::code_rule_dsl::logic::front::{AXIS, AggOp, Atom, DESCENDANTS, Error, LatticeDecl, List, Literal, Native, Program, RESERVED, Rule, SCOPE_NOT, SpanIn, Term, Value, axis};
use crate::code_rule_dsl::facts::vtab::syntax::GUEST_POLICY;
use crate::code_rule_dsl::logic::strings::SET;
use crate::code_rule_dsl::logic::program_facts::Field;
use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut, ProgramFields};
pub mod findings;

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

fn test_of(rel: &str) -> Option<&'static str> {
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

#[derive(Clone, Default)]
struct Lowerer {
    /// Where the walk writes its rules: the rules written before its clauses, then a text's version (`Written`,
    /// decision 0377 step 3).
    out: Out,
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
    /// aggregate's shared variables (`aggregate`).
    siblings: Vec<Lit>,
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

/// Where `Lowerer::write` puts a rule.
#[derive(Clone)]
enum Out {
    /// The rules written before the module's clauses (its statements': a `.violation`'s), or by a lowerer not
    /// walking a module's clauses.
    Before(Vec<Rule>),
    /// A text's own program, written as a version in the walk (decision 0377 step 3), onto the lowerer's header.
    Version(Box<Written>),
}

impl Default for Out {
    fn default() -> Self {
        Out::Before(Vec::new())
    }
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

/// What the walk reads back of the rules it wrote onto the lowerer's header (`Lowerer::prog`).
#[derive(Clone)]
pub struct Written {
    /// How many rules it wrote: a `must` or `may` head's rules are known by their place (`modal`).
    n: usize,
    /// Each rule's content, by a 128-bit hash: a demand is written once (`demand`, `world_demand`).
    seen: std::collections::HashSet<u128>,
    /// The rules of the relations a `.shape` names, which the module checks once its clauses are written.
    shaped: BTreeSet<String>,
    kept: Vec<Rule>,
    /// The first rule the builder refused.
    error: Option<String>,
}

impl Written {
    fn new(shaped: BTreeSet<String>) -> Self {
        Written { n: 0, seen: Default::default(), shaped, kept: Vec::new(), error: None }
    }

    fn hash(rule: &Rule) -> u128 {
        use std::hash::{BuildHasher, Hash, Hasher};
        let half = |seed: u64| {
            let mut h = foldhash::fast::FixedState::with_seed(seed).build_hasher();
            rule.hash(&mut h);
            h.finish() as u128
        };
        half(0x9e37_79b9_7f4a_7c15) << 64 | half(0xc2b2_ae3d_27d4_eb4f)
    }

    /// `rule` written onto `b`, the lowerer's header.
    fn write(&mut self, b: &mut crate::code_rule_dsl::logic::program_image::Builder<'static>, rule: Rule) {
        self.seen.insert(Written::hash(&rule));
        self.n += 1;
        if let Err(e) = b.add(&rule) {
            self.error.get_or_insert(e);
        }
        if self.shaped.contains(rule.head.rel.as_str()) {
            self.kept.push(rule);
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

/// `.scope text "**/*.md", …`: documents read as text only (`front::Program::text_globs`).
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
pub fn lower_module_seeded_written(statements: Vec<Statement>, seeds: &Seeds) -> Result<Module, Error> {
    let typing = crate::code_rule_dsl::logic::profile::span(|| "type the tapes' terms".to_owned());
    let written: BTreeMap<String, Vec<String>> = statements
        .iter()
        .filter_map(|(item, _)| match item {
            Item::Decl { name, cols, .. } => Some((name.clone(), cols.iter().map(|(_, t)| t.clone()).collect())),
            _ => None,
        })
        .collect();
    super::tape_type::check_in(&statements, &seeds.given)?;
    drop(typing);
    let reading = crate::code_rule_dsl::logic::profile::span(|| "read the statements".to_owned());
    let declares_docof = statements.iter().any(|(item, _)| matches!(item, Item::Decl { name, .. } if name == "docof"));
    let mut l = seeds.l.clone();
    let seeded = seeds.seeded.clone();
    let mut clauses = Vec::new();
    // `.shape` binds to the rule just before it (O-join-shape-directives): its heads, then each directive and its line.
    let mut last_heads: Option<Vec<String>> = None;
    let mut shapes: Vec<(crate::code_rule_dsl::logic::front::Shape, usize)> = Vec::new();
    // The facts its clauses state, by relation, stated on the header once read.
    let mut own_facts: BTreeMap<String, Vec<crate::code_rule_dsl::logic::front::List<Value>>> = BTreeMap::new();
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
                    l.prog.push_setting(if output { "output" } else { "input" }.to_owned(), format!("{name}({text})"));
                }
                for n in &names {
                    if output { l.prog.add_output(n) } else { l.prog.add_input(n) }
                }
            }
            Item::Lattice { name, base, ops } => {
                let Some((_, STerm::Call { name: lub, functor: true, .. })) = ops.iter().find(|(op, _)| op == "Lub") else {
                    return Err(at(Error::Syntax(format!(".lattice {name} has no `Lub -> @f`"))));
                };
                l.lattices.insert(name, (ty(&base), lub.clone()));
            }
            Item::Pragma(words) => match words.as_slice() {
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
                _ => return not_yet(format!(".pragma {words:?}")).map_err(at),
            },
            Item::Policy(document) => l.prog.edit(|b| b.policies(), |p| Field::Policies(p), |p| p.push(document)),
            // A test is never the program's (decision 0130): testing mode splices its statements into a program of
            // its own (`rr::testing`).
            Item::Test { .. } | Item::Fixtures { .. } => {}
            Item::Setting { key, text } if key == "shape" => {
                let Some(heads) = &last_heads else { return Err(Error::Syntax(format!("line {line}: `.shape {text}` follows no rule"))) };
                for h in heads {
                    shapes.push((crate::code_rule_dsl::logic::front::Shape::parse(h, &text).map_err(|m| Error::Syntax(format!("line {line}: {m}")))?, line));
                }
            }
            Item::Setting { key, text } => l.prog.push_setting(key, text),
            Item::Violation { name, cols, code, code_in, tag, suppress, fixes, message, .. } => l.violation(name, cols, code, code_in, tag, suppress, fixes, message).map_err(at)?,
            Item::Clause { heads, body, edits } => {
                if body.is_empty() && edits.is_empty() && let Some(rows) = facts(&heads) {
                    for (rel, row) in rows {
                        own_facts.entry(rel).or_default().push(row.into());
                    }
                    last_heads = None;
                    continue;
                }
                last_heads = edits.is_empty().then(|| heads.iter().map(|h| h.rel.clone()).collect());
                clauses.push((heads, body, edits, line));
            }
        }
    }
    for (rel, rows) in own_facts {
        l.prog.add_facts(&rel, &rows);
    }
    drop(reading);
    let stated: BTreeSet<String> = l.prog.input_names().into_iter().filter(|r| !seeded.inputs.contains(r)).collect();
    let rules = crate::code_rule_dsl::logic::profile::span(|| "lower the rules".to_owned());
    l.derived = clauses.iter().flat_map(|(heads, ..)| heads.iter().map(|h| h.rel.clone())).chain(l.prog.fact_names()).collect();
    let mut own_deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (heads, body, ..) in &clauses {
        let mut rels = BTreeSet::new();
        lit_rels(body, &mut rels);
        for h in heads {
            own_deps.entry(h.rel.clone()).or_default().extend(rels.iter().cloned());
        }
    }
    for (rel, reads) in &own_deps {
        l.deps.entry(rel.clone()).or_default().extend(reads.iter().cloned());
    }
    // The demands the text states (a printed program's): its own declarations, as read, and those its includes
    // stated. A demand an include's lowering made is not one: this module makes its own.
    l.printed_demands = DEMANDS.iter().chain(WORLD.iter().map(|(_, d)| d)).filter(|d| l.prog.declares(d) && !seeded.decls.contains(**d)).map(|d| (*d).to_owned()).collect();
    l.printed_demands.extend(seeds.printed.iter().cloned());
    // From here the rules go to the version; what the statements wrote before goes first, in its order.
    let mut w = Written::new(shapes.iter().map(|(shape, _)| shape.head.clone()).collect());
    let Out::Before(before) = std::mem::take(&mut l.out) else { unreachable!("a module's walk starts once") };
    for rule in before {
        w.write(&mut l.prog, rule);
    }
    l.out = Out::Version(Box::new(w));
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
    // Each directive names what its rules read: a relation of their body, and the variables it waits for.
    for (shape, line) in shapes {
        use crate::code_rule_dsl::logic::front::{ShapeWait, shape_base};
        let rules: Vec<&crate::code_rule_dsl::logic::front::Rule> = l.written_of(&shape.head);
        let reads = |rel: &str| rules.iter().any(|r| r.body.iter().filter_map(|x| x.atom()).any(|a| shape_base(&a.rel) == rel));
        let missing = match &shape.wait {
            _ if !reads(&shape.lit) => Some(format!("no rule of `{}` reads `{}`", shape.head, shape.lit)),
            ShapeWait::After(a) if !reads(a) => Some(format!("no rule of `{}` reads `{a}`", shape.head)),
            ShapeWait::Bound(vars) => vars.iter().find(|v| !rules.iter().any(|r| r.body.iter().any(|x| x.vars().contains(*v)))).map(|v| format!("no rule of `{}` has the variable `{v}`", shape.head)),
            _ => None,
        };
        if let Some(why) = missing {
            return Err(Error::Syntax(format!("line {line}: `.shape {}`: {why}", shape.text)));
        }
        l.prog.edit(|b| b.shapes(), |s| Field::Shapes(s), |s| s.push(shape));
    }
    drop(rules);
    let places = l.unseed(seeded);
    l.stated = l.prog.input_names().iter().take_while(|r| stated.contains(*r)).count();
    l.deps = own_deps;
    let docof = declares_docof.then_some(l.docof_of_locs);
    // The module's header goes into its version with its rules: what the link and the modules that include it read.
    let Out::Version(w) = std::mem::take(&mut l.out) else { unreachable!("the walk writes a version") };
    let version = w.version(std::mem::take(&mut l.prog))?;
    Ok(Module { l, written, docof, places, version })
}

/// A module of the modules `scope` reaches (each once), its rules written as a version in the walk: a library's
/// module (`rr::lower_library`).
pub fn lower_module_in_written(statements: Vec<Statement>, scope: &[&Module]) -> Result<Module, Error> {
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
/// `unit`, `whole_back`, `front::finish` and the tail (aggregate demand, `unit`,
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

/// The relations a program's rules read (an aggregate's among them), with the arity each is read with.
pub fn reads(prog: &Program) -> BTreeSet<(String, usize)> {
    prog.rules.iter().flat_map(|r| r.body.iter().filter_map(Literal::atom)).map(|a| (a.rel.to_string(), a.args.len())).collect()
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

/// A program as `span_patterns` reads it, from the tree (`of_tree`) or a version.
pub struct SpanProgram<'a> {
    pub rules: Vec<SpanRule<'a>>,
    /// Each declared relation's name and arity.
    pub decls: Vec<(&'a str, usize)>,
    pub inputs: Vec<&'a str>,
    pub natives: BTreeSet<&'a str>,
    pub outputs: Vec<&'a str>,
}

impl<'a> SpanProgram<'a> {
    pub fn of_tree(prog: &'a Program) -> Self {
        let term = |t: &'a Term| match t {
            Term::Var(x) => SpanTerm::Var(x.as_str()),
            Term::Str(s) => SpanTerm::Str(s.as_str()),
            _ => SpanTerm::Other,
        };
        let atom = |a: &'a Atom| SpanAtom { rel: a.rel.as_str(), args: a.args.iter().map(term).collect(), neg: a.neg };
        SpanProgram {
            rules: prog.rules.iter().map(|r| SpanRule { head: atom(&r.head), body: r.body.iter().filter_map(Literal::atom).map(atom).collect() }).collect(),
            decls: prog.decls.iter().map(|(r, c)| (r.as_str(), c.len())).collect(),
            inputs: prog.inputs.iter().map(String::as_str).collect(),
            natives: prog.natives.keys().map(String::as_str).collect(),
            outputs: prog.outputs.iter().map(String::as_str).collect(),
        }
    }
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

/// The relations `lits` read, inside groups and aggregates too.
fn lit_rels(lits: &[Lit], out: &mut BTreeSet<String>) {
    fn term(t: &STerm, out: &mut BTreeSet<String>) {
        match t {
            STerm::Agg { over, by, body, .. } => {
                lit_rels(body, out);
                over.iter().for_each(|t| term(t, out));
                by.iter().for_each(|t| term(t, out));
            }
            STerm::Named(_, t) => term(t, out),
            _ => {}
        }
    }
    for l in lits {
        match l {
            Lit::Atom { rel, args, .. } => {
                out.insert(rel.clone());
                args.iter().for_each(|t| term(t, out));
            }
            Lit::Cmp { l, r, .. } => {
                term(l, out);
                term(r, out);
            }
            Lit::Group { alts, .. } => alts.iter().for_each(|a| lit_rels(a, out)),
            Lit::Pattern { .. } => {}
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
                    STerm::Str(s) => Some(Value::Str(s.clone().into())),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            h.modal.is_none().then(|| (h.rel.clone(), row))
        })
        .collect()
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
        let prog = &self.prog;
        fresh_name(&mut self.next, &mut self.made, |n| prog.declares(n), stem)
    }

    /// A helper relation named `name` (a `fresh` one of `stem`), with these columns and rules; or, when one made
    /// before has the same stem, columns and rules, that relation. Each occurrence made its own: the two contexts of
    /// a rule's two aggregates over the same variables were `rr_ctx7` and `rr_ctx8` in most of dl1's 44 policies, and
    /// `congruence` merged them back after lowering (decision 0225 §6). An aggregate's helper, which demand narrows
    /// by its readers, has its readers' heads in its stem (`aggregate`).
    /// A rule the walk made, written where it writes (`out`).
    fn write(&mut self, rule: Rule) {
        match &mut self.out {
            Out::Before(rules) => rules.push(rule),
            Out::Version(w) => w.write(&mut self.prog, rule),
        }
    }

    /// How many rules the walk has written.
    fn written(&self) -> usize {
        match &self.out {
            Out::Before(rules) => rules.len(),
            Out::Version(w) => w.n,
        }
    }

    /// Whether the walk wrote `rule` already.
    fn wrote(&self, rule: &Rule) -> bool {
        match &self.out {
            Out::Before(rules) => rules.contains(rule),
            Out::Version(w) => w.seen.contains(&Written::hash(rule)),
        }
    }

    /// The rules written of `rel`, a relation a `.shape` names.
    fn written_of(&self, rel: &str) -> Vec<&Rule> {
        match &self.out {
            Out::Before(rules) => rules.iter().filter(|r| r.head.rel == rel).collect(),
            Out::Version(w) => w.kept.iter().filter(|r| r.head.rel == rel).collect(),
        }
    }

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
            self.prog.push_setting("suppress".to_owned(), format!("{name} {} {rules}{over}", s.tool));
        }
        for f in fixes {
            self.prog.push_setting("fix".to_owned(), format!("{name} {}{} {}", if f.safe { "safe " } else { "" }, f.name, quote(&f.title)));
        }
        if let Some(re) = code_in {
            self.prog.push_setting("code_in".to_owned(), format!("{name} {}", quote(&re)));
        }
        self.violations.push(Violation { name: name.clone(), arity: cols.len(), code, node, places: false, columns: cols.clone(), message, tag });
        self.decl(name.clone(), cols, &[])?;
        self.prog.add_output(&name);
        Ok(())
    }

    fn clause(&mut self, heads: &[Head], body: &[Lit]) -> Result<(), Error> {
        fn binds_top(lits: &[Lit]) -> bool {
            lits.iter().any(|l| match l {
                Lit::Atom { neg: false, args, .. } => args.iter().any(|a| matches!(a, STerm::Var(v) if v == "top")),
                Lit::Group { alts, neg: false } => alts.iter().any(|a| binds_top(a)),
                _ => false,
            })
        }
        if self.native_definition(heads, body) {
            return Ok(());
        }
        self.top_is_var = binds_top(body);
        self.heads = heads.iter().map(|h| h.rel.clone()).collect();
        self.head_vars = {
            let mut v = Vec::new();
            heads.iter().flat_map(|h| &h.args).for_each(|a| a.vars(&mut v));
            v.into_iter().collect()
        };
        self.first_atom = match body.first() {
            Some(Lit::Atom { rel, neg: false, .. }) => Some(rel.clone()),
            _ => None,
        };
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
                    cols.extend(self.prog.columns_of(&head.rel).expect("a `.violation` is declared").into_iter().skip(1));
                    self.prog.declare_cols(&place_relation(&head.rel), cols);
                }
                rel = place_relation(&head.rel);
                written = [path.clone(), line.clone()].into_iter().chain(head.args.iter().skip(1).cloned()).collect();
            }
            let args = written.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
            for body in &bodies {
                let (args, body) = unset(args.clone(), body.clone())?;
                if let Some(m) = &head.modal {
                    self.modal.insert(self.written(), m.clone());
                }
                self.write(Rule { head: Atom { rel: rel.clone().into(), args: args.into(), neg: false, gated: false }, body: body.into() });
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
            let mut alts = alts?;
            // Alternatives multiply: `(a ; b), (c ; d)` is four rules, and `within_stop` is two. Past OUTLINE_PAST of
            // them, a literal's alternatives are a helper relation (`outline`), and `within_stop(n, a, S)` over a kind
            // set is one atom of `rr_wstop_<S>`, S's relation (`within_stop_relation`). A nest of them was otherwise
            // exponential: a Rust policy of 200 lines lowered to 101,423 rules. Below it, they stay in the rule, whose
            // own literals narrow each.
            if acc.len() * alts.len() > OUTLINE_PAST && alts.len() > 1 {
                if let Lit::Atom { rel, args, neg: false } = lit
                    && rel == "within_stop"
                    && let [n, a, stop] = args.as_slice()
                    && self.node_set(stop).is_none()
                {
                    let r = self.within_stop_relation(stop)?;
                    alts = vec![vec![Literal::Atom(Atom { rel: r.into(), args: [self.term(n)?, self.term(a)?].into(), neg: false, gated: false })]];
                } else {
                    let mut used: Vec<String> = context.iter().cloned().collect();
                    lits.iter().enumerate().filter(|(j, _)| *j != k).for_each(|(_, l)| l.scope_vars(&mut used));
                    let used: BTreeSet<String> = used.into_iter().collect();
                    alts = self.outline(alts, &used);
                }
            }
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
                if !self.prog.has_input(rel) {
                    self.prog.add_input(rel);
                    self.prog.declare_cols(rel, cols.iter().copied());
                    self.prog.edit(|b| b.tapes(), |t| Field::Tapes(t), |t| t.entry(rel.to_owned()).or_default().extend(super::tape_type::tape_columns(rel)));
                }
            }
            // `@tree_digest(n)` reads the tree: a variable bound by the engine's `tree_digest(n, h)`.
            STerm::Call { name, functor: true, args } if name == "tree_digest" && args.len() == 1 => {
                self.hoist(&mut args[0], out, true);
                let n = args[0].clone();
                let v = STerm::Var(self.fresh("digest"));
                *t = v.clone();
                out.push(Lit::Atom { rel: "tree_digest".to_owned(), args: vec![n, v], neg: false });
                if !self.prog.has_input("tree_digest") {
                    self.prog.add_input("tree_digest");
                    self.prog.declare_cols("tree_digest", [("n", "node"), ("s", "sym")]);
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
            Lit::Pattern { subject, pattern, selector, neg } => self.pattern(subject, pattern, selector.as_deref(), *neg)?,
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
            // `match_all("re", n, m)` over a node a sibling binds to a kind (`kind(n, "arguments")`): `rx_span`'s
            // matches in the node, m each match, its groups bound with it. A `source`, `src_line` or comment span, or a
            // column of a derived relation (which may hold such spans), is the batch's `matchall_` relations'.
            Lit::Atom { rel, args, neg: false }
                if rel == "match_all"
                    && matches!(args.as_slice(), [STerm::Str(_), STerm::Var(n), _] if self.span_container(n).is_none() && self.kind_bound(n)) =>
            {
                self.node_matches(args)?
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
                vec![vec![Literal::Cmp { l: Term::Call("rx".into(), [self.term(s)?, Term::Str(re.clone().into())].into()), op: "=", r: Term::Int(0) }]]
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
                            out.push(Literal::Cmp { l: Term::Var(v.clone().into()), op: "=", r: std::mem::replace(t, Term::Var(v.into())) });
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
            ("parse_of", [x, g, t]) => self.demand("rr_parse_req", &[(STerm::Var(LOCATED.to_owned().into()), "int"), (g.clone(), "sym"), (x.clone(), "sym")], t, false),
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
                    let decl = self.prog.columns_of(rel.as_str());
                    let doc = crate::code_rule_dsl::logic::front::document_column(rel);
                    args.iter().enumerate().find_map(|(c, a)| match a {
                        STerm::Var(v) if Some(c) == doc || decl.as_ref().and_then(|d| d.get(c)).is_some_and(|(_, t)| t == "node") => Some(v.clone()),
                        _ => None,
                    })
                }
                _ => None,
            });
            match loc {
                Some(v) => given[0] = (STerm::Var(v), "int"),
                None => {
                    given[0] = (STerm::Var("rr_demand_doc".into()), "int");
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
        let args: Vec<Term> = given.iter().map(|(t, _)| match t {
            STerm::Var(v) => Term::Var(v.clone().into()),
            STerm::Str(s) => Term::Str(s.clone().into()),
            _ => unreachable!("checked above"),
        }).collect();
        let head = Atom { rel: req.to_owned().into(), args: args.into(), neg: false, gated: false };
        let mut body: Vec<Literal> = ctx.into_iter().map(Literal::Atom).collect();
        if every_chunk {
            body.push(Literal::Atom(Atom { rel: "doc".into(), args: [Term::Var("rr_demand_doc".into()), Term::Wild, Term::Wild].into(), neg: false, gated: false }));
        }
        let rule = Rule { head, body: body.into() };
        if !self.wrote(&rule) {
            self.write(rule);
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
        let head = |key: Term| Atom { rel: demand.to_owned().into(), args: [key].into(), neg: false, gated: false };
        self.prog.declare_cols(demand, [("k", "sym")]);
        let s = match key {
            // A constant key is a fact, as the reader takes a bodiless clause: printed and read back, it is the same.
            STerm::Str(k) => {
                let row: crate::code_rule_dsl::logic::front::List<Value> = [Value::Str(k.clone().into())].into();
                if !self.prog.fact_rows(demand).contains(&row) {
                    self.prog.add_facts(demand, &[row]);
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
        let rule = Rule { head: head(Term::Var(s.clone().into())), body: [Literal::Atom(ctx)].into() };
        if !self.wrote(&rule) {
            self.write(rule);
        }
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
    fn precedes_stop(&mut self, set: &STerm) -> String {
        let label = match set {
            STerm::Str(k) | STerm::Var(k) => k.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>(),
            _ => "set".to_owned(),
        };
        let name = format!("{RESERVED}prec_{label}");
        if self.prog.declares(&name) {
            return name;
        }
        let v = |s: &str| Term::Var(s.to_owned().into());
        let atom = |rel: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: rel.to_owned().into(), args: args.into(), neg, gated: false });
        let cmp = |l: &str, r: &str| Literal::Cmp { l: v(l), op: "<", r: v(r) };
        let member = match (set, self.node_set(set)) {
            (_, Some(nodes)) => vec![atom(&nodes, vec![v("s")], false)],
            (STerm::Str(k), _) => vec![atom("kind", vec![v("s"), Term::Str(k.clone().into())], false)],
            (STerm::Var(kinds), _) => vec![atom("kind", vec![v("s"), v("sk")], false), atom(kinds, vec![v("sk")], false)],
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
    fn node_set(&self, set: &STerm) -> Option<String> {
        let STerm::Var(name) = set else { return None };
        let cols = self.prog.columns_of(&name)?;
        (cols.len() == 1 && cols[0].1 == "node").then(|| name.clone())
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
    fn node_axis(&mut self, rel: &str, args: &[STerm], neg: bool, outer: &BTreeSet<String>) -> Result<Vec<Vec<Literal>>, Error> {
        let set = self.node_set(&args[2]).expect("a node set");
        let atom = |r: &str, args: Vec<Term>| Literal::Atom(Atom { rel: r.to_owned().into(), args: args.into(), neg: false, gated: false });
        let named = |me: &mut Self, t: &STerm| -> Result<String, Error> {
            match t {
                STerm::Var(v) => Ok(v.clone()),
                STerm::Wild => Ok(me.fresh("w")),
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
                let below = self.within("within", &[STerm::Var(n.clone().into()), STerm::Var(a.clone().into())], false)?;
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
        self.prog.declare_cols(&within, [("n", "node"), ("a", "node")]);
        if !self.prog.has_input(within) {
            self.prog.add_input(within);
        }
        let below = |neg: bool| vec![Literal::Atom(Atom { rel: within.to_owned().into(), args: [n.clone(), a.clone()].into(), neg, gated: false })];
        let Some(stop) = stop else { return Ok(vec![below(neg)]) };
        let set = self.set_name("within_stop", stop)?;
        let Term::Var(nv) = &n else { return Err(Error::Syntax("within_stop: the node is a variable".to_owned())) };
        let s = self.fresh("stop");
        let nearest = Literal::Nearest { n: nv.to_string(), a: s.clone(), kindset: set.clone() };
        // The nearest stop against a is by the tree, not by comparing ids (ids need not follow preorder, decision
        // 0083 §3).
        if neg {
            // n is not below a, or its nearest stop is strictly below a.
            let between = Literal::Atom(Atom { rel: within.to_owned().into(), args: [Term::Var(s.clone().into()), a.clone()].into(), neg: false, gated: false });
            return Ok(vec![below(true), below(false).into_iter().chain([nearest, between]).collect()]);
        }
        let none = Literal::Atom(Atom { rel: format!("{AXIS}{set}").into(), args: [n.clone(), Term::Wild].into(), neg: true, gated: false });
        // The nearest stop is a, or a is below it: it is not strictly below a (both are above n, so one of the three
        // holds). One alternative, not two: a rule's alternatives multiply, and a policy of several `within_stop`s
        // was exponential in them.
        let not_below = Literal::Atom(Atom { rel: within.to_owned().into(), args: [Term::Var(s.clone().into()), a.clone()].into(), neg: true, gated: false });
        Ok(vec![below(false).into_iter().chain([none]).collect(), below(false).into_iter().chain([nearest, not_below]).collect()])
    }

    /// `within_stop(n, a, S)` as a relation `rr_wstop_<S>(n, a)`: n is below a, and no node of S is between them.
    /// At most `within`'s rows, made once a set.
    fn within_stop_relation(&mut self, stop: &STerm) -> Result<String, Error> {
        let label = match stop {
            STerm::Str(k) | STerm::Var(k) => k.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>(),
            _ => "set".to_owned(),
        };
        let name = format!("{RESERVED}wstop_{label}");
        if self.prog.declares(&name) {
            return Ok(name);
        }
        let bodies = self.within("within_stop", &[STerm::Var("n".to_owned()), STerm::Var("a".to_owned()), stop.clone()], false)?;
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
    fn native_definition(&mut self, heads: &[Head], body: &[Lit]) -> bool {
        if self.python_candidates_definition(heads, body) {
            return true;
        }
        let ([h], [Lit::Atom { rel: input, args: of, neg: false }, Lit::Atom { rel: m, args, neg: false }]) = (heads, body) else { return false };
        let ([STerm::Var(s)], Some(STerm::Str(re)), Some(STerm::Var(scanned))) = (of.as_slice(), args.first(), args.get(1)) else { return false };
        if m != "match_all" || scanned != s || h.modal.is_some() || self.prog.natives().contains_key(h.rel.as_str()) {
            return false;
        }
        let coordinates = &args[2..];
        if !(coordinates.is_empty() || coordinates.len() == 3 && coordinates.iter().all(|t| matches!(t, STerm::Var(_)))) {
            return false;
        }
        let gs = groups(re);
        let want: Vec<STerm> = std::iter::once(STerm::Var(s.clone().into())).chain(gs.iter().map(|g| STerm::Var(g.clone().into()))).chain(coordinates.iter().cloned()).collect();
        if h.args != want {
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
    fn python_candidates_definition(&mut self, heads: &[Head], body: &[Lit]) -> bool {
        let ([h], [Lit::Atom { rel, args, neg: false }]) = (heads, body) else { return false };
        let [c, f, r, o, STerm::Var(input), STerm::Var(targets), STerm::Str(context), STerm::Str(member)] = args.as_slice() else { return false };
        let cols = [c, f, r, o];
        let distinct: BTreeSet<_> = cols.iter().filter_map(|t| if let STerm::Var(x) = t { Some(x) } else { None }).collect();
        if rel != "python_candidates" || h.modal.is_some() || distinct.len() != 4 || h.args.iter().ne(cols.into_iter()) || self.prog.natives().contains_key(h.rel.as_str()) {
            return false;
        }
        if !crate::code_rule_dsl::facts::core::PYTHON_CONTEXTS.contains(&context.as_str()) {
            return false;
        }
        let native = Native::PythonCandidates { input: input.clone(), targets: targets.clone(), context: context.clone(), member: member.clone() };
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
            let atom = |args: Vec<Term>| Literal::Atom(Atom { rel: "rx_span".into(), args: args.into(), neg: false, gated: false });
            let (p, c) = (Term::Str(pattern.into()), Term::Var(v.clone().into()));
            if gs.is_empty() {
                return Ok(vec![vec![atom(vec![p, c, Term::Wild, Term::Str(String::new().into()), Term::Wild])]]);
            }
            let j = Term::Var(self.fresh("j").into());
            return Ok(vec![gs.iter().map(|g| atom(vec![p.clone(), c.clone(), j.clone(), Term::Str(g.clone().into()), Term::Var(g.clone().into())])).collect()]);
        }
        if !all {
            return self.captures(args);
        }
        if gs.is_empty() && positions.is_empty() {
            return Ok(vec![vec![Literal::Cmp { l: Term::Call("rx".into(), [self.term(s)?, Term::Str(re.clone().into())].into()), op: "=", r: Term::Int(1) }]]);
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
        self.match_all_native(&name, &ctx.rel, re, &gs, !positions.is_empty());
        let args = std::iter::once(Term::Var(v.clone().into())).chain(gs.iter().map(|g| Term::Var(g.clone().into())))
            .map(Ok).chain(positions.iter().map(|p| self.term(p))).collect::<Result<Vec<_>, Error>>()?;
        Ok(vec![vec![Literal::Atom(Atom { rel: name.into(), args: args.into(), neg: false, gated: false })]])
    }

    /// `match_all("re", n, m)` over node n: each match j of re in n's source is `rx_span(re, n, j, "", m)`, and each
    /// named group g of that match `rx_span(re, n, j, g, g)`, as `match_all` over a span binds them.
    fn node_matches(&mut self, args: &[STerm]) -> Result<Vec<Vec<Literal>>, Error> {
        let [STerm::Str(re), STerm::Var(n), m] = args else { unreachable!("a node match_all") };
        self.rx_span_input();
        let atom = |args: Vec<Term>| Literal::Atom(Atom { rel: "rx_span".into(), args: args.into(), neg: false, gated: false });
        let (p, c, j) = (Term::Str(re.clone().into()), Term::Var(n.clone().into()), Term::Var(self.fresh("j").into()));
        let m = self.term(m)?;
        let mut out = vec![atom(vec![p.clone(), c.clone(), j.clone(), Term::Str(String::new().into()), m])];
        out.extend(groups(re).into_iter().map(|g| atom(vec![p.clone(), c.clone(), j.clone(), Term::Str(g.clone().into()), Term::Var(g.into())])));
        Ok(vec![out])
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
            let args = vec![Term::Str(format!("^(?:{re})$").into()), Term::Var(v.clone().into()), Term::Wild, Term::Str(String::new().into()), Term::Wild];
            return Ok(vec![vec![Literal::Atom(Atom { rel: "rx_span".into(), args: args.into(), neg, gated: false })]]);
        }
        let t = self.fresh("text");
        let text = Literal::Atom(Atom { rel: "text".into(), args: [Term::Var(v.clone().into()), Term::Var(t.clone().into())].into(), neg: false, gated: false });
        let test = self.atom(rel, &[p.clone(), STerm::Var(t)], neg)?;
        Ok(vec![vec![text, test]])
    }

    /// ``n ~ `pattern` ``: node n is a match of the AST pattern (ast-grep's syntax, in the program's language), and
    /// each metavariable `$X` binds variable X to the node it captured. The batch matches the pattern once over each
    /// document (`core_rr::CoreRows::patterns`) and makes `ast_match(p, n, g, c)`: capture g of the match at n is
    /// node c, and capture "" is n itself. A negated pattern binds nothing: no match at n. A `$$$X` (a run of
    /// nodes) and a `$_X` capture nothing.
    fn pattern(&mut self, subject: &STerm, pattern: &str, selector: Option<&str>, neg: bool) -> Result<Vec<Vec<Literal>>, Error> {
        let key = crate::code_rule_dsl::language::pattern_key(pattern, selector);
        let STerm::Var(n) = subject else {
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
            return Ok(vec![vec![atom("", Term::Wild, true)]]);
        }
        let mut out = vec![atom("", Term::Wild, false)];
        out.extend(super::single_metavars(pattern).into_iter().map(|x| atom(&x, Term::Var(x.clone().into()), false)));
        Ok(vec![out])
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
        self.siblings.iter().any(|l| {
            matches!(l, Lit::Atom { rel, args, neg: false } if rel == "kind"
                && matches!(args.as_slice(), [STerm::Var(x), STerm::Str(k)] if x == v && k != "comment"))
        })
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
            Lit::Atom { .. } | Lit::Group { .. } => self.node_in(l, v),
            _ => false,
        })
    }

    /// Whether `l` binds `v` at a node column: a positive atom, or alternatives that each do
    /// (`(php_arg(c, 1, l) ; php_named_arg(c, "level", l)), match("…", l)` tests l's text, not its id).
    fn node_in(&self, l: &Lit, v: &str) -> bool {
        match l {
            Lit::Atom { rel, args, neg: false } => self.prog.columns_of(rel.as_str()).is_some_and(|cols| {
                args.iter().zip(cols).any(|(a, (_, t))| matches!(a, STerm::Var(x) if x == v) && t == "node")
            }),
            Lit::Group { alts, neg: false } => !alts.is_empty() && alts.iter().all(|alt| alt.iter().any(|l| self.node_in(l, v))),
            _ => false,
        }
    }

    /// `match("(?<x>…)", s)`: s matches, and each named group binds the variable of its name to its text:
    /// `x = @rx_group(s, "^(?:…)$", "x")`, which has no value when s does not match (`strings::group`).
    fn captures(&mut self, args: &[STerm]) -> Result<Vec<Vec<Literal>>, Error> {
        let [STerm::Str(re), s] = args else { unreachable!("match with a pattern") };
        let whole = Term::Str(format!("^(?:{re})$").into());
        let s = self.term(s)?;
        // The test as well as the groups: a group the head alone reads is a column, which SQL does not filter on, so
        // a text that does not match would make a row with no value (`pct_rest`'s `%%` step).
        let mut out = vec![Literal::Cmp { l: Term::Call("rx".into(), [s.clone(), whole.clone()].into()), op: "=", r: Term::Int(1) }];
        for g in groups(re) {
            out.push(Literal::Cmp { l: Term::Var(g.clone().into()), op: "=", r: Term::Call("rx_group".into(), [s.clone(), whole.clone(), Term::Str(g.into())].into()) });
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
                    out.push(Literal::Atom(Atom { rel: "text".into(), args: [Term::Var(v.clone().into()), Term::Var(t.clone().into())].into(), neg: false, gated: false }));
                    Term::Var(t.into())
                }
                s => self.term(s)?,
            };
            let set = Term::Call(SET.to_owned().into(), tests.iter().map(|(_, _, p)| Term::Str(p.clone().into())).collect());
            out.push(Literal::Cmp { l: Term::Call(tests[0].0.to_owned().into(), [s, set].into()), op: "=", r: Term::Int(1) });
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
            let set = Term::Call(SET.to_owned().into(), members.iter().map(|(_, c)| Term::Str(c.clone().into())).collect());
            return Ok(Some(vec![Literal::Cmp { l: Term::Call("equals".into(), [Term::Var(members[0].0.clone().into()), set].into()), op: "=", r: Term::Int(1) }]));
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
                self.prog.set_kindset(&name, vec![kind.clone()]);
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
                    Term::Str(re) => Term::Str(format!("^(?:{re})$").into()),
                    other => anchored(other),
                };
            }
            return Ok(Literal::Cmp { l: Term::Call(op.to_owned().into(), [s, p].into()), op: "=", r: Term::Int(if neg { 0 } else { 1 }) });
        }
        match (rel, args) {
            // `nearest(n, _, set)`: n has an ancestor in the set, which is `inside(n, _, set)`; and negated.
            ("nearest", [n, STerm::Wild, set]) => {
                let set = self.set_name(rel, set)?;
                return Ok(Literal::Atom(Atom { rel: format!("{AXIS}{set}").into(), args: [self.term(n)?, Term::Wild].into(), neg, gated: false }));
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
                return Ok(Literal::Atom(Atom { rel: format!("{axis}{set}").into(), args: args.into(), neg, gated: false }));
            }
            ("ast", _) if !neg => return self.ast(args),
            ("precedes_stop", [a, b, set]) => {
                let rel = self.precedes_stop(set);
                return Ok(Literal::Atom(Atom { rel: rel.into(), args: [self.term(a)?, self.term(b)?].into(), neg, gated: false }));
            }
            ("dfs_first", [r, hit, first, edge, target]) | ("dfs_first_memo", [r, hit, first, edge, target, _]) => {
                let name = |t: &STerm| match t {
                    STerm::Var(v) => Ok(v.clone()),
                    _ => Err(Error::Syntax(format!("{rel}: the edge, target and roots are relations, by name"))),
                };
                let (edge, target) = (name(edge)?, name(target)?);
                let roots = if rel == "dfs_first_memo" { Some(name(&args[5])?) } else { None };
                let rel = self.dfs_first(&edge, &target, roots.as_deref());
                return Ok(Literal::Atom(Atom { rel: rel.into(), args: [self.term(r)?, self.term(hit)?, self.term(first)?].into(), neg, gated: false }));
            }
            ("python_candidates", [call,report,reference,receiver,input,targets,context,member]) => {
                let name=|t:&STerm|match t{STerm::Var(v)=>Ok(v.clone()),_=>Err(Error::Syntax("Python candidate inputs are relation names".into()))};
                let literal=|t:&STerm|match t{STerm::Str(v)=>Ok(v.clone()),_=>Err(Error::Syntax("Python candidate context and member must be string literals".into()))};
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
                let name=|t:&STerm|match t{STerm::Var(v)=>Ok(v.clone()),_=>Err(Error::Syntax("Python producer inputs are relation names".into()))};
                let input=name(&args[width])?;
                let targets=if kind=="qualified" {Some(name(&args[width+1])?)}else{None};
                let producer=self.python_producer(kind,&input,targets.as_deref());
                return Ok(Literal::Atom(Atom{rel:producer.into(),args:args[..width].iter().map(|a|self.term(a)).collect::<Result<Vec<_>,_>>()?.into(),neg,gated:false}));
            }
            ("components", [d, c, k, edge, own]) => {
                let name = |t: &STerm| match t {
                    STerm::Var(v) => Ok(v.clone()),
                    _ => Err(Error::Syntax("components: the edge and own relations are named".to_owned())),
                };
                let rel = self.components(&name(edge)?, &name(own)?);
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
        let Native::PythonCandidates { input, targets, .. } = &native else { unreachable!("python candidates") };
        let v = |s: &str| Term::Var(s.to_owned().into());
        let a = |r: &str, args: Vec<Term>| Literal::Atom(Atom { rel: r.into(), args: args.into(), neg: false, gated: false });
        let mut body = vec![a(input, vec![v("d")]), a(targets, vec![v("q")])];
        for n in ["c", "f", "r", "o"] {
            body.push(a("node", vec![v("d"), v(n), Term::Wild, Term::Wild, Term::Wild]));
        }
        self.prog.declare_cols(name, ["c", "f", "r", "o"].map(|n| (n, "node")));
        self.write(Rule { head: Atom { rel: name.to_owned().into(), args: ["c", "f", "r", "o"].into_iter().map(v).collect(), neg: false, gated: false }, body: body.into() });
        self.prog.set_native(&name, native);
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
        let v=|s:&str|Term::Var(s.to_owned().into());
        let atom=|r:&str,args:Vec<Term>|Literal::Atom(Atom{rel:r.into(),args: args.into(),neg:false,gated:false});
        let mut body=vec![atom(input,vec![v("n")]),atom("node",vec![v("d"),v("n"),Term::Wild,Term::Wild,Term::Wild])];
        if let Some(t)=targets {body.push(atom(t,vec![v("q")]));}
        else {
            body.push(atom("node",vec![v("d"),v("value"),Term::Wild,Term::Wild,Term::Wild]));
            if kind=="arguments" {
                for (name,value) in [("i",Term::Int(0)),("keyword",Term::Str(String::new().into())),("spread",Term::Str(String::new().into()))] {
                    body.push(Literal::Cmp{l:v(name),op:"=",r:value});
                }
            }
        }
        self.prog.declare_cols(&rel, vars.iter().map(|(n, t)| (n.to_string(), t.to_string())));
        self.write(Rule{head:Atom{rel:rel.clone().into(),args:vars.iter().map(|(n,_)|v(n)).collect(),neg:false,gated:false},body: body.into()});
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
        let v = |n: &str| Term::Var(n.to_owned().into());
        let atom = |rel: &str, args: Vec<Term>| Literal::Atom(Atom { rel: rel.to_owned().into(), args: args.into(), neg: false, gated: false });
        let mut body = vec![atom(edge, vec![v("r"), v("hit"), v("first")]), atom(target, vec![v("hit")])];
        if let Some(roots) = roots {
            body.push(atom(roots, vec![v("r"), Term::Wild]));
        }
        self.write(Rule { head: Atom { rel: rel.clone().into(), args: [v("r"), v("hit"), v("first")].into(), neg: false, gated: false }, body: body.into() });
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
        let v = |n: &str| Term::Var(n.to_owned().into());
        let atom = |rel: &str, args: Vec<Term>| Literal::Atom(Atom { rel: rel.to_owned().into(), args: args.into(), neg: false, gated: false });
        let body = vec![atom(own, vec![v("d"), v("k")]), atom(edge, vec![v("d"), v("e")]), atom(own, vec![v("e"), Term::Wild])];
        self.write(Rule { head: Atom { rel: rel.clone().into(), args: [v("d"), v("e"), v("k")].into(), neg: false, gated: false }, body: body.into() });
        let cols = vec![("d".to_owned(), "int".to_owned()), ("c".to_owned(), "int".to_owned()), ("k".to_owned(), "sym".to_owned())];
        self.prog.declare_cols(&rel, cols.iter().map(|(c, t)| (c.to_string(), t.to_string())));
        self.prog.set_native(&rel, Native::Components { edge: edge.to_owned(), own: own.to_owned() });
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
                let (l, r) = if matches!(l, STerm::Agg { .. }) && op != "=" { (Term::Var(v.into()), other) } else { (other, Term::Var(v.into())) };
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
                    // `unset(x)` is the empty value, lowered in a rule's own body only: a context leaves it out (and
                    // holds more rows).
                    Lit::Atom { neg, rel, .. } => !neg && rel != "unset",
                    Lit::Cmp { l, r, .. } => aggregates || !matches!(l, STerm::Agg { .. }) && !matches!(r, STerm::Agg { .. }),
                    // `(v = t ; within(v, t))`: it may be all that ties two asked variables (pyi047's unread_comp_var
                    // ties a comprehension c to its target v only so). `bound_context` keeps it once what it reads is bound.
                    l @ Lit::Group { .. } => Self::plain_group(l),
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
        let demand_of_head = |l: &Lit| match l {
            Lit::Atom { rel, neg: false, args, .. } => {
                !reaches_heads(rel)
                    && !args.is_empty()
                    && (first_atom.as_deref() == Some(rel.as_str()) || args.iter().all(|a| matches!(a, STerm::Var(v) if head_vars.contains(v))))
            }
            _ => false,
        };
        let input_only = |l: &Lit| match l {
            Lit::Atom { rel, .. } if inputs.contains(rel) => true,
            l @ Lit::Atom { .. } => demand_of_head(l),
            Lit::Group { alts, .. } => alts.iter().flatten().all(|l| !matches!(l, Lit::Atom { rel, .. } if !inputs.contains(rel))),
            _ => true,
        };
        let base: Vec<Lit> = context(false).into_iter().filter(|l| input_only(l)).collect();
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
                for l in lits.iter().filter(|l| matches!(l, Lit::Atom { neg: false, .. }) || Self::plain_group(l)) {
                    let lv = body_vars(std::slice::from_ref(l));
                    if !lv.is_disjoint(&reach) && !lv.is_subset(&reach) {
                        reach.extend(lv);
                        grew = true;
                    }
                }
            }
            vars.iter().all(|v| reach.contains(v))
        };
        let whole = if self.request_context { Some(Self::bound_context(context(false))) } else { None };
        let context = if let Some((whole, _)) = whole.filter(|(_, b)| vars.iter().all(|v| b.contains(v))) {
            whole
        } else if !self.span_context && !self.request_context && vars.iter().all(|v| base_bound.contains(v)) && joined(&base) {
            base
        } else if vars.iter().all(|v| bound.contains(v)) {
            first
        } else {
            Self::bound_context(context(true)).0
        };
        // Only the literals tied to `vars` through shared variables: one tied to none tests that some row exists
        // anywhere (`ann_root(_)` once `(s = a ; within(s, a))` is left out), which makes every row of the context
        // depend on every document. Leaving it out widens the context, which the rule joins with its own literals.
        let context = self.covering(Self::connected(Self::placed_texts(Self::connected(context, vars), vars), vars), vars)?;
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

    /// The literals of `context` reachable from `vars` through shared variables, in their order. A literal that
    /// ranges over a file's nodes (`docof(a, d)`, `node(d, a, …)`, `a` not asked about) is not reached through its
    /// document: tied to `vars` only so, it tests that some node exists in the same file, and joined in the context it
    /// is every pair of the file's nodes (`latest_alias`'s `max` beside `docof(a, d), docof(e, d)`: `docof(a, d),
    /// visit_index(a, _)` in the context of `(e, x)`, 18 M rows on 100 files). Left out, it widens the context, which
    /// the rule joins with its own literals. A relation of the document itself (`first_import(d, …)`) still is.
    fn connected(context: Vec<Lit>, vars: &[String]) -> Vec<Lit> {
        let file_nodes = |l: &Lit| -> Option<String> {
            let Lit::Atom { rel, args, neg: false } = l else { return None };
            match (rel.as_str(), args.as_slice()) {
                ("docof", [STerm::Var(n), STerm::Var(d), ..]) | ("node", [STerm::Var(d), STerm::Var(n), ..]) if !vars.contains(n) && !vars.contains(d) => Some(d.clone()),
                _ => None,
            }
        };
        let mut reach: BTreeSet<String> = vars.iter().cloned().collect();
        let of: Vec<BTreeSet<String>> = context
            .iter()
            .map(|l| {
                let mut vs = body_vars(std::slice::from_ref(l));
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
    fn covering(&mut self, context: Vec<Lit>, vars: &[String]) -> Result<Vec<Lit>, Error> {
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
        let groups: Vec<usize> = (0..context.len()).map(|k| root(&mut group, k)).collect();
        let binds = |l: &Lit| matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none() && rel != "earlier");
        // The variables a binding atom binds: its plain arguments. One it computes from (`jb(@sub(p, 1), 92)`) it
        // reads: lowered, that is `t = @sub(p, 1), jb(t, 92)`, and a cover of it alone could not place `p`.
        let bound_by = |l: &Lit| -> BTreeSet<String> {
            match l {
                Lit::Atom { args, .. } if binds(l) => args.iter().filter_map(|a| match a { STerm::Var(v) => Some(v.clone()), _ => None }).collect(),
                _ => BTreeSet::new(),
            }
        };
        // Per group: the asked variables its binding atoms bind, and those any of its literals reads.
        let asked_in = |g: usize, only_binding: bool| -> BTreeSet<String> {
            (0..context.len())
                .filter(|&k| groups[k] == g)
                .flat_map(|k| {
                    let vars = if only_binding { bound_by(&context[k]) } else { of[k].clone() };
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
        let membership = |l: &Lit| match l {
            Lit::Atom { args, neg, .. } => {
                body_vars(std::slice::from_ref(l)).is_subset(&asked)
                    && (*neg || args.iter().all(|a| matches!(a, STerm::Var(_) | STerm::Str(_) | STerm::Num(_) | STerm::Float(_))))
            }
            _ => false,
        };
        let mut members: BTreeMap<usize, Vec<Lit>> = BTreeMap::new();
        for (l, g) in context.iter().zip(&groups) {
            if !whole.contains(g) && !left.contains(g) {
                members.entry(*g).or_default().push(l.clone());
            }
        }
        // A cover of two or more asked variables ties them through variables not asked (c414's `call_nkwargs(c, n),
        // call_nkwargs(s, n)`): alone it is every pair with the same `n`, 38.8 M rows on 50 files where the context
        // reaches a handful. Its body leads with the whole groups that bind its variables, so it holds only the
        // bindings the context can reach, still a row per binding.
        let whole_binding: Vec<(BTreeSet<String>, Vec<Lit>)> = whole
            .iter()
            .map(|&g| {
                let lits = context.iter().zip(&groups).filter(|(_, h)| **h == g).map(|(l, _)| l.clone()).collect();
                (asked_in(g, true), lits)
            })
            .collect();
        let mut out = Vec::new();
        for (l, g) in context.into_iter().zip(groups) {
            if whole.contains(&g) || (left.contains(&g) && membership(&l)) {
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
                let mut narrowed: Vec<Lit> = whole_binding.iter().filter(|(b, _)| cols.iter().any(|c| b.contains(c))).flat_map(|(_, l)| l.iter().cloned()).collect();
                narrowed.extend(lits);
                narrowed
            } else {
                lits
            };
            let alts = self.conjunction(&lits, &cols.iter().cloned().collect())?;
            let types = cols.iter().map(|c| (c.clone(), self.type_of(c, &alts))).collect();
            let args: Vec<Term> = cols.iter().map(|c| Term::Var(c.clone().into())).collect();
            let rel = self.helper_once(rel, "cover", types, alts.into_iter().map(|b| (args.clone(), b)).collect());
            out.push(Lit::Atom { rel, args: cols.into_iter().map(STerm::Var).collect(), neg: false });
        }
        Ok(out)
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
        // A plain disjunction (`plain_group`) stays once each alternative grounds what it reads given `bound`; it binds
        // the variables every alternative binds. Otherwise it is left out, widening the context.
        let mut grew = true;
        while grew {
            grew = false;
            if let Some(k) = cmps.iter().position(|g| Self::group_binds(g, &bound).is_some()) {
                let g = cmps.remove(k);
                bound.extend(Self::group_binds(&g, &bound).expect("ready"));
                context.push(g);
                grew = true;
            }
        }
        (context, bound)
    }

    /// `( a ; b )`, positive, each alternative only positive atoms (no text test, no `earlier`) and comparisons without
    /// aggregates: a disjunction a context can hold.
    fn plain_group(l: &Lit) -> bool {
        let Lit::Group { alts, neg: false } = l else { return false };
        !alts.is_empty()
            && alts.iter().all(|alt| {
                alt.iter().all(|l| match l {
                    // `unset(x)` is the empty value, lowered in a rule's own body only: no relation a context reads.
                    Lit::Atom { rel, neg: false, .. } => test_of(rel).is_none() && rel != "earlier" && rel != "unset",
                    Lit::Cmp { l, r, .. } => !matches!(l, STerm::Agg { .. }) && !matches!(r, STerm::Agg { .. }),
                    _ => false,
                })
            })
    }

    /// The variables plain group `g` binds given `bound` (those every alternative binds), if each alternative grounds
    /// its comparisons: an equality may bind one side from the other, any other comparison reads only bound ones.
    fn group_binds(g: &Lit, bound: &BTreeSet<String>) -> Option<BTreeSet<String>> {
        let Lit::Group { alts, .. } = g else { return None };
        if !Self::plain_group(g) {
            return None;
        }
        let mut all: Option<BTreeSet<String>> = None;
        for alt in alts {
            let mut have = bound.clone();
            have.extend(body_vars(&alt.iter().filter(|l| matches!(l, Lit::Atom { .. })).cloned().collect::<Vec<_>>()));
            let mut cmps: Vec<&Lit> = alt.iter().filter(|l| matches!(l, Lit::Cmp { .. })).collect();
            loop {
                let reads = |t: &STerm| body_vars(&[Lit::Cmp { l: t.clone(), op: "=", r: STerm::Wild }]);
                let ready = cmps.iter().position(|c| {
                    let Lit::Cmp { l, op, r } = c else { return false };
                    match (l, r) {
                        (STerm::Var(x), t) | (t, STerm::Var(x)) if *op == "=" && !have.contains(x) => reads(t).is_subset(&have),
                        _ => reads(l).union(&reads(r)).all(|v| have.contains(v)),
                    }
                });
                let Some(k) = ready else { break };
                have.extend(body_vars(std::slice::from_ref(cmps.remove(k))));
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
        let atoms: Vec<Lit> = self
            .siblings
            .iter()
            .filter(|l| matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none() && rel != "earlier"))
            .cloned()
            .collect();
        let mut bound = body_vars(&atoms);
        // And what the rule's equalities bind from those, to a fixed point: `x = t` once t's variables are bound,
        // and a bound text split by `q = @dot(m, x)` (`cat`'s parts), as lowering writes `split_dot`: a printed
        // program binds by these what its source bound by an atom.
        let vars = |t: &STerm| {
            let mut out = Vec::new();
            t.vars(&mut out);
            out
        };
        loop {
            let before = bound.len();
            for l in &self.siblings {
                let Lit::Cmp { l, op: "=", r } = l else { continue };
                for (a, b) in [(l, r), (r, l)] {
                    match (a, b) {
                        (STerm::Var(x), t) if !bound.contains(x) && !vars(t).contains(x) && vars(t).iter().all(|y| bound.contains(y)) => {
                            bound.insert(x.clone());
                        }
                        (STerm::Var(q), STerm::Call { name, functor: true, args }) if name == "dot" && bound.contains(q) => {
                            for t in args {
                                bound.extend(vars(t));
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
    fn aggregate(&mut self, var: &str, agg: &str, over: Option<&STerm>, by: &[STerm], sep: &Option<String>, body: &[Lit], outer: &BTreeSet<String>) -> Result<Vec<Literal>, Error> {
        // A value or key that is a term (`sum strlen(x) - strlen(a)`) is a variable the body binds.
        let mut body = body.to_vec();
        let mut variable = |t: &STerm, me: &mut Self| match t {
            STerm::Var(v) => v.clone(),
            t => {
                let v = me.fresh("val");
                body.push(Lit::Cmp { l: STerm::Var(v.clone().into()), op: "=", r: t.clone() });
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
        //
        // A `_` in one of the body's atoms ranges over the rows as a variable does (Soufflé counts the tuples): for
        // `count`, `sum` and `product` it is named, so `count : { match_all(",", n, _) }` counts each match rather than
        // the one row the helper had without it.
        let named: Vec<Lit>;
        let body = if matches!(op, AggOp::Count | AggOp::Sum | AggOp::Product) {
            named = body
                .iter()
                .map(|l| match l {
                    Lit::Atom { rel, args, neg: false } if !matches!(rel.as_str(), "nearest" | "inside" | "has" | "split_dot" | "ast" | "within" | "within_stop" | "precedes_stop" | "unset") => {
                        let args = args.iter().map(|t| if matches!(t, STerm::Wild) { STerm::Var(self.fresh("any")) } else { t.clone() }).collect();
                        Lit::Atom { rel: rel.clone(), args, neg: false }
                    }
                    l => l.clone(),
                })
                .collect();
            named.as_slice()
        } else {
            body
        };
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
        let binders = |attributes: bool| -> BTreeSet<String> {
            body_vars(
                &body
                    .iter()
                    .filter(|l| binds_columns(l))
                    .filter(|l| !matches!(l, Lit::Atom { rel, .. } if matches!(rel.as_str(), "match_all" | "match") || (!attributes && matches!(rel.as_str(), "node" | "line" | "end_line"))))
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
        let (with_attributes, without) = (binders(true), binders(false));
        let by_attributes = inner.iter().filter(|v| outer.contains(*v) && with_attributes.contains(*v) && !without.contains(*v)).count();
        let bound = if by_attributes >= 2 { without } else { with_attributes };
        let needed: Vec<String> = inner.iter().filter(|v| outer.contains(*v) && !bound.contains(*v)).cloned().collect();
        if !needed.is_empty() {
            // The context holds every variable the body shares with the rule, those the body binds too: projected on
            // the tested ones alone (`ctx(j)`), it crossed every row of the body (`child(al, a, k)`, every argument of
            // every call) with every j of the rule, 20.6M rows for 4,133 counts on 50 files.
            let shared: Vec<String> = ordered.iter().filter(|v| outer.contains(*v)).cloned().collect();
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

    fn term(&mut self, t: &STerm) -> Result<Term, Error> {
        Ok(match t {
            STerm::Var(v) if v == "top" && !self.top_is_var => Term::Top,
            STerm::Var(v) => Term::Var(v.clone().into()),
            STerm::Wild => Term::Wild,
            STerm::Num(n) => Term::Int(*n),
            STerm::Str(s) => Term::Str(s.clone().into()),
            STerm::Call { name, functor: true, args } => {
                let mut args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
                // The data-layer programs' SQL builtins (decision 0107): the engine's functions of SQLite's meaning.
                if let Some((f, swap)) = crate::code_rule_dsl::logic::strings::functions::catalogue(name, args.len()) {
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
                Term::Call(if name == "dot" { "cat".into() } else { name.clone().into() }, args.into())
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
                    return Ok(args.fold(first, |acc, a| Term::Call("concat".into(), [acc, a].into())));
                }
                Term::Call(f.to_owned().into(), args.into())
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
                Term::Call(f.to_owned().into(), [self.term(l)?, self.term(r)?].into())
            }
            STerm::Agg { op, .. } => return not_yet(format!("the aggregate {op} as a term")),
            STerm::Tick(t) => return not_yet(format!("the template `{t}`")),
            // A decimal number is its float's key, which orders as the floats do.
            STerm::Float(f) => Term::Int(float_key(f.parse::<f64>().map_err(|_| Error::Syntax(format!("bad decimal number {f}")))?)),
            STerm::Json(_) | STerm::Named(..) => return Err(Error::Syntax("a JSON term or an `ast` capture outside `ast(…)`".to_owned())),
        })
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
            return Ok(vec![vec![Literal::Cmp { l: q, op: "!=", r: cat }], vec![dotted(1)]]);
        }
        out.extend([Literal::Cmp { l: q, op: "=", r: cat }, dotted(0)]);
        Ok(vec![out])
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
                    vars.insert(v.to_string());
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
            Term::Var(v) if vars.contains(v.as_str()) => *t = Term::Int(UNSET),
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
                    (t, neg) => Some(Literal::Cmp { l: t, op: if neg { "!=" } else { "=" }, r: Term::Str(String::new().into()) }),
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

/// Whether `rel` is one the rewrite adds (`sources_on_request`): a request, an attribute's eager half or named
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
/// widened copy. They are no reader of anything a program adds: a library's cone (`rr::with_cone`) is taken without
/// them, or it reached, from a relation a program adds to, the widened copies of its readers, their requests, the
/// attribute's answer and then every reader of the attribute (five of the Python pack's programs took 1,300 of the
/// library's rules each).
pub fn on_request_rule(head: &str) -> bool {
    head.starts_with("rr_wide_") || ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| a.rel == head || a.req == head)
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
    ],
];

/// Each per-node source attribute of `ON_REQUEST` answered on request (decision 0174), after 0145's shape: each rule
/// reading `a(n, x)` with `n` bound by its other literals asks for those nodes, `rr_a_req(n) :- <those literals>`,
/// and one rule answers the requests, `a(n, x) :- rr_a_req(n), x = @f(n)`.
///
/// A request's literals are the reader's own, less those that read the attribute's value (`x`, and what is computed
/// from it), those not joined to `n`, negations and aggregates. Dropping a literal widens a request, which is sound;
/// asking too little is not. A relation that reads an attribute is read through its widened copy, `rr_wide_<r>`: its
/// rules with the attributes and the same literals dropped, on the columns they still bind (`Widen`). It holds every
/// row of `r` and reads no attribute, so requests stratify below every reader (read through `r` itself, a request
/// would be recursive with the attribute, and through a negation, not stratifiable).
///
/// A negated read asks as a positive one does (its node bound by the rule's other literals, the rows it tests asked
/// for). Where a reader reads the attribute inside an aggregate, of a constant node, or does not bind `n`, it stays an
/// input, made for every node, as all are with `RERAIL_NODE_SOURCES=eager`. The analysis is made either way and
/// written to the profile's decisions (`<a> on request`, its readers and the widened copies): what each reader asks,
/// what it leaves out and why.
pub fn sources_on_request(prog: &mut crate::code_rule_dsl::logic::front::ast::Program, round: &[OnRequest]) {
    use crate::code_rule_dsl::logic::profile::{decision, on};
    let attrs: Vec<OnRequest> = round.iter().copied().filter(|a| prog.inputs.iter().any(|i| i == a.rel)).collect();
    if attrs.is_empty() {
        return;
    }
    let applied = !std::env::var("RERAIL_NODE_SOURCES").is_ok_and(|v| v == "eager");
    let w = Widen::new(prog, attrs.iter().map(|a| a.rel).collect());
    let mut all: Vec<Rule> = Vec::new();
    let mut answered: Vec<OnRequest> = Vec::new();
    // Each request rule's readers (`Program::asked_by`): `prune` keeps it while one of them is reached.
    let mut asked: BTreeMap<String, Vec<BTreeSet<String>>> = BTreeMap::new();
    // The readers' asks that narrow requests (decision 0291), by relation: each its rules, one per reading site, with
    // the relations whose rules ask it (its `asked_by` gate: an ask whose reader is never reached is pruned with it).
    let mut demands: BTreeMap<String, (Vec<Rule>, Vec<BTreeSet<String>>)> = BTreeMap::new();
    'attr: for &attr in &attrs {
        let (rel, req) = (attr.rel, attr.req);
        let topic = format!("{rel} on request");
        let mut requests: Vec<Rule> = Vec::new();
        let mut readers: Vec<BTreeSet<String>> = Vec::new();
        let mut explained: Vec<String> = Vec::new();
        for rule in &prog.rules {
            for (i, lit) in rule.body.iter().enumerate() {
                let eager = |why: &str| decision(&topic, || format!("no, every node: {} {why}{}", rule.head.rel, if applied { "" } else { " (RERAIL_NODE_SOURCES=eager)" }));
                let a = match lit {
                    Literal::Atom(a) if a.rel == rel => a,
                    Literal::Agg { atom, .. } if atom.rel == rel => {
                        eager(&format!("reads `{lit}` in an aggregate"));
                        continue 'attr;
                    }
                    _ => continue,
                };
                // A negation asks as a positive read does: its node is bound by the rule's other literals, and the
                // rows asked for are the ones it tests.
                let Some(Term::Var(n)) = a.args.first() else {
                    eager(&format!("reads `{lit}`, of a constant node"));
                    continue 'attr;
                };
                let mut out: BTreeSet<String> = BTreeSet::new();
                a.args[1..].iter().for_each(|t| t.vars(&mut out));
                out.remove(n.as_str());
                // What the value is computed into is the value too.
                loop {
                    let before = out.len();
                    for l in &rule.body {
                        if let Literal::Cmp { l: x, op: "=", r: y } = l {
                            for (v, t) in [(x, y), (y, x)] {
                                let mut vars = BTreeSet::new();
                                t.vars(&mut vars);
                                if let Term::Var(v) = v && !vars.is_disjoint(&out) && v != n {
                                    out.insert(v.to_string());
                                }
                            }
                        }
                    }
                    if out.len() == before {
                        break;
                    }
                }
                let to = n.to_string();
                let (body, bound, dropped) = w.widen(&rule.body, Some(i), &out, Some(&to));
                if !bound.contains(&to) {
                    eager(&format!("reads `{lit}`, and no literal it keeps binds ${n}{}", Widen::left_out(&dropped)));
                    continue 'attr;
                }
                // Narrowed by what the reader's own readers ask of it (decision 0291): where `n` is a column of the
                // reader's head that its reading rules bind, the request joins those rules' asks. A reading site that
                // cannot ask (a blocker) asks for every value: the request is made unnarrowed too, serving the
                // blockers, which read the reader, so `prune` keeps it exactly where a blocker is reached.
                let unnarrowed = body.clone();
                let mut blocked: BTreeSet<String> = BTreeSet::new();
                // The narrowed request serves the reader's readers that ask (a narrowed request with no asker reached
                // asks for nothing); unnarrowed, it serves the reader.
                let mut askers: BTreeSet<String> = BTreeSet::from([rule.head.rel.to_string()]);
                let mut body = body;
                let narrowed = w.demand_of(rule, &to).map(|(col, asks, blockers)| {
                    blocked = blockers;
                    askers = asks.iter().flat_map(|(_, by)| by.iter().cloned()).collect();
                    let dem = format!("{req}_by_{}_{col}", rule.head.rel);
                    if !demands.contains_key(&dem) {
                        // Each ask serves the readers that ask it, as a request does (`prune` keeps it while one of
                        // them is reached); two sites asking alike are one ask (`demand_of`), serving both.
                        let (rules, by): (Vec<Rule>, Vec<BTreeSet<String>>) = asks.into_iter().map(|(ask, by)| (Rule { head: Atom { rel: dem.as_str().into(), args: vec![Term::Var("rr_dn".into())].into(), neg: false, gated: false }, body: ask.into() }, by)).unzip();
                        demands.insert(dem.clone(), (rules, by));
                    }
                    body.push(Literal::Atom(Atom { rel: dem.as_str().into(), args: vec![Term::Var(n.clone())].into(), neg: false, gated: false }));
                    dem
                });
                // A variable the request reads once, outside its head, is a witness: a wildcard, so its atom is a test
                // of existence, not a join over every row (`kind(v, k)` beside `kind(e, k)`, asking for `e`).
                let body = wild_singletons(body, &to);
                let request = Rule { head: Atom { rel: req.into(), args: vec![Term::Var(n.clone())].into(), neg: false, gated: false }, body: body.into() };
                if on() {
                    explained.push(format!("{} reads `{lit}`: asks `{request}`{}{}", rule.head.rel, Widen::left_out(&dropped), narrowed.map_or(String::new(), |d| format!("; narrowed by its readers' asks, `{d}`"))));
                }
                let mut made = vec![(request, askers)];
                if !blocked.is_empty() {
                    let body = wild_singletons(unnarrowed, &to);
                    made.push((Rule { head: Atom { rel: req.into(), args: vec![Term::Var(n.clone())].into(), neg: false, gated: false }, body: body.into() }, blocked));
                }
                for (request, by) in made {
                    match requests.iter().position(|r| *r == request) {
                        Some(k) => readers[k].extend(by),
                        None => {
                            requests.push(request);
                            readers.push(by);
                        }
                    }
                }
            }
        }
        decision(&topic, || match explained.len() {
            0 => "no rule reads it: it stays an input, which a run that reads nothing of it binds for no node".to_owned(),
            n => format!("{}: {n} readers ask, by {} request rules", if applied { "yes" } else { "possible (RERAIL_NODE_SOURCES=eager)" }, requests.len()),
        });
        // Read by no rule, an attribute has nothing to answer: answered anyway, its answer and eager rules made it a
        // derived relation of the program, which a caller binding the base relations by name (the SQL executor's
        // snapshot, whose `value` is an input) refused as an output that already exists.
        if requests.is_empty() {
            continue;
        }
        for e in explained {
            decision(&format!("{rel} on request: readers"), || e);
        }
        all.extend(requests);
        if !readers.is_empty() {
            asked.insert(req.to_owned(), readers);
        }
        answered.push(attr);
    }
    let demanded: Vec<String> = demands.keys().cloned().collect();
    for (dem, (rules, by)) in std::mem::take(&mut demands) {
        all.extend(rules);
        asked.insert(dem, by);
    }
    let wide = w.rules(&all);
    for (rel, cols) in &wide.copies {
        decision("on request: widened copies", || format!("{} of {rel}, columns {cols:?} of {}{}", w.wide(rel), w.arity(rel), if cols.len() < w.arity(rel) { " (the others are bound only by what the copy leaves out)" } else { "" }));
    }
    if !applied {
        return;
    }
    let atom = |rel: &str, args: Vec<Term>| Atom { rel: rel.into(), args: args.into(), neg: false, gated: false };
    let var = |v: &str| Term::Var(v.into());
    for OnRequest { rel, req, answer, eager } in answered {
        let cols = prog.decls.get(rel).cloned().unwrap_or_else(|| vec![("n".into(), "node".into()), ("v".into(), "symbol".into())].into());
        let args: Vec<Term> = (0..cols.len()).map(|i| var(&format!("x{i}"))).collect();
        if let Some(input) = eager {
            all.push(Rule { head: atom(rel, args.clone()), body: vec![Literal::Atom(atom(input, args.clone()))].into() });
            prog.decls.insert(input.into(), cols.clone());
            prog.inputs.push(input.to_owned());
        }
        match answer {
            Answer::Function(function) => all.push(Rule {
                head: atom(rel, args.clone()),
                body: vec![Literal::Atom(atom(req, vec![args[0].clone()])), Literal::Cmp { l: args[1].clone(), op: "=", r: Term::Call(function.into(), vec![args[0].clone()].into()) }].into(),
            }),
            Answer::Children { named } => {
                // token(p, i, s, a, b): the named child c in place of its text s, which node_text gives.
                let mut by_node = args.clone();
                by_node[2] = var("c");
                all.push(Rule {
                    head: atom(rel, args.clone()),
                    body: vec![Literal::Atom(atom(req, vec![args[0].clone()])), Literal::Atom(atom(named, by_node)), Literal::Atom(atom("node_text", vec![var("c"), args[2].clone()]))].into(),
                });
                let mut named_cols = cols.clone();
                named_cols.make_mut()[2] = ("c".into(), "node".into());
                prog.decls.insert(named.into(), named_cols);
                prog.inputs.push(named.to_owned());
                if !prog.inputs.iter().any(|i| i == "node_text") {
                    // Typed as the attribute's own columns: its node, and its text.
                    prog.decls.entry("node_text".into()).or_insert_with(|| vec![("n".into(), cols[0].1.clone()), ("s".into(), cols[2].1.clone())].into());
                    prog.inputs.push("node_text".to_owned());
                }
            }
        }
        prog.decls.insert(req.into(), vec![(cols[0].0.clone(), "node".into())].into());
        prog.inputs.retain(|i| i != rel);
    }
    prog.decls.extend(wide.decls.into_iter().map(|(r, cols)| (r.into(), cols.into_iter().map(|(n, t)| (n.into(), t.into())).collect())));
    for dem in demanded {
        prog.decls.insert(dem.as_str().into(), vec![("n".into(), "node".into())].into());
    }
    for (rel, rows) in wide.facts {
        prog.facts.entry(rel).or_default().extend(rows.into_iter().map(List::from));
    }
    prog.rules.extend(all);
    prog.rules.extend(wide.rules);
    prog.asked_by.extend(asked);
    prog.asked_by.extend(wide.asked);
}

/// `prog` as it was before `sources_on_request`: each attribute it answered on request an input again, without its
/// requests, answers, eager rows' rule and widened copies. For programs merged into one and lowered once
/// (`convert::overlay`): the requests are made over the merged program's readers.
pub fn undo_on_request(prog: &mut crate::code_rule_dsl::logic::front::ast::Program) {
    let mut rules = std::mem::take(&mut prog.rules);
    undo_on_request_in(prog, &mut rules);
    prog.rules = rules;
}

/// The rules `undo_on_request` reads and drops, as a tree's or a version's (decision 0281's path, step 7a-2).
pub trait RuleSet {
    /// Drops each rule whose head `gone` names.
    fn drop_heads(&mut self, gone: &dyn Fn(&str) -> bool);
    /// Whether a rule's body atom (not an aggregate's) reads `rel`.
    fn reads(&self, rel: &str) -> bool;
}

impl RuleSet for Vec<Rule> {
    fn drop_heads(&mut self, gone: &dyn Fn(&str) -> bool) {
        self.retain(|r| !gone(r.head.rel.as_str()));
    }
    fn reads(&self, rel: &str) -> bool {
        self.iter().flat_map(|r| &r.body).filter_map(Literal::atom).any(|a| a.rel == rel)
    }
}

/// `undo_on_request` of a program's fields `prog` and its rules `rules`.
pub fn undo_on_request_in(prog: &mut (impl crate::code_rule_dsl::logic::program_image::fields::FieldsMut + ?Sized), rules: &mut dyn RuleSet) {
    for round in ON_REQUEST.iter().rev() {
        undo_round(prog, rules, round);
    }
}

/// One round of `ON_REQUEST` undone (`undo_on_request`) of a program's fields `prog` and its rules `rules`: its
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

/// The widened copies a request reads (`sources_on_request`): for each relation `r` that reads an attribute,
/// `rr_wide_<r>` on the columns its rules bind without one.
struct Widen<'a> {
    prog: &'a crate::code_rule_dsl::logic::front::ast::Program,
    attrs: BTreeSet<&'static str>,
    /// The copies' prefix: `rr_wide_<a>_`, `a` the round's first attribute (a relation may be copied in each round).
    prefix: String,
    /// The relations that read an attribute, and the columns their widened copies keep.
    cols: BTreeMap<String, Vec<usize>>,
}

/// What `Widen::rules` adds to the program.
#[derive(Default)]
struct Widened {
    rules: Vec<Rule>,
    decls: BTreeMap<String, Vec<(String, String)>>,
    facts: BTreeMap<String, Vec<Vec<Value>>>,
    /// The relations copied, and the columns each copy keeps.
    copies: Vec<(String, Vec<usize>)>,
    /// The copies of request relations (`Program::asked_by`): each copied rule with its source rule's readers, so
    /// an unreached reader's request is not kept through its copy.
    asked: BTreeMap<String, Vec<BTreeSet<String>>>,
}

impl<'a> Widen<'a> {
    fn new(prog: &'a crate::code_rule_dsl::logic::front::ast::Program, attrs: BTreeSet<&'static str>) -> Self {
        // The relations that read an attribute, through the rules to a fixpoint.
        let mut reading: BTreeSet<String> = attrs.iter().map(|a| a.to_string()).collect();
        loop {
            let before = reading.len();
            for r in &prog.rules {
                if r.body.iter().filter_map(Literal::atom).any(|a| reading.contains(&*a.rel)) {
                    reading.insert(r.head.rel.to_string());
                }
            }
            if reading.len() == before {
                break;
            }
        }
        attrs.iter().for_each(|a| {
            reading.remove(*a);
        });
        let arity = |r: &str| prog.rules.iter().find(|x| x.head.rel == r).map_or(0, |x| x.head.args.len());
        let prefix = format!("rr_wide_{}_", attrs.iter().next().copied().unwrap_or_default());
        let mut w = Widen { prog, attrs, prefix, cols: reading.iter().map(|r| (r.clone(), (0..arity(r)).collect())).collect() };
        // Each column a rule of r leaves unbound once widened goes, to a fixpoint (a column of a widened atom
        // in the body may go in turn).
        loop {
            let mut changed = false;
            for rule in &prog.rules {
                let Some(cols) = w.cols.get(&*rule.head.rel) else { continue };
                let (_, bound, _) = w.widen(&rule.body, None, &BTreeSet::new(), None);
                let kept: Vec<usize> = cols.iter().copied().filter(|&c| Self::bound_term(&rule.head.args[c], &bound)).collect();
                if kept.len() != cols.len() {
                    w.cols.insert(rule.head.rel.to_string(), kept);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        w
    }

    fn arity(&self, rel: &str) -> usize {
        self.prog.rules.iter().find(|x| x.head.rel == rel).map_or(0, |x| x.head.args.len())
    }

    fn bound_term(t: &Term, bound: &BTreeSet<String>) -> bool {
        let mut vars = BTreeSet::new();
        t.vars(&mut vars);
        !matches!(t, Term::Wild | Term::Top) && vars.is_subset(bound)
    }

    fn wide(&self, rel: &str) -> String {
        format!("{}{rel}", self.prefix)
    }

    /// Decision 0291: what the rules reading `rule`'s relation ask of its column holding `n`, where every one asks:
    /// the column, and per reading site, its other literals widened (as a request's are) and joined to the column's
    /// variable, renamed apart with that variable as `rr_dn` and the others named from the reading rule alone, each
    /// with the relations whose rules ask it (`Program::asked_by`: the ask serves them). A reading site that cannot ask
    /// (the relation read in an aggregate, by name, by its own rules, or with that column a constant or bound by
    /// nothing its reader keeps) asks for every value: its relation is returned among the blockers, whose requests are
    /// made unnarrowed beside the narrowed one. None where the relation is an output or an input, or no site asks.
    fn demand_of(&self, rule: &Rule, n: &str) -> Option<(usize, Vec<(Vec<Literal>, BTreeSet<String>)>, BTreeSet<String>)> {
        let rel = &*rule.head.rel;
        let col = rule.head.args.iter().position(|t| matches!(t, Term::Var(v) if v == n))?;
        if self.prog.outputs.iter().any(|o| o == rel) || self.prog.inputs.iter().any(|i| i == rel) {
            return None;
        }
        let mut asks: Vec<(Vec<Literal>, BTreeSet<String>)> = Vec::new();
        let mut blockers: BTreeSet<String> = BTreeSet::new();
        for r in &self.prog.rules {
            for (i, l) in r.body.iter().enumerate() {
                let mut block = || {
                    blockers.insert(r.head.rel.to_string());
                };
                let a = match l {
                    Literal::Atom(a) if a.rel == rel => a,
                    Literal::Agg { atom, .. } if atom.rel == rel => {
                        block();
                        continue;
                    }
                    // Read by name, not as an atom: no reading site to ask from.
                    Literal::Nearest { kindset, .. } if kindset == rel => {
                        block();
                        continue;
                    }
                    _ => continue,
                };
                if r.head.rel == rel {
                    block();
                    continue;
                }
                let Some(Term::Var(u)) = a.args.get(col) else {
                    block();
                    continue;
                };
                let u = u.to_string();
                let (body, ..) = self.widen(&r.body, Some(i), &BTreeSet::new(), Some(&u));
                // No widened copy: a copy of a recursive relation can lose what bounds it (a counter's step, once its
                // text test is dropped). Its atoms are left out, which only widens the ask; what is left must still
                // bind the column and stay joined to it.
                let body = Self::joined(body.into_iter().filter(|l| !matches!(l, Literal::Atom(a) if a.rel.starts_with(self.prefix.as_str()))).collect(), &u);
                if !body.iter().any(|l| matches!(l, Literal::Atom(a) if a.vars().contains(&u))) {
                    blockers.insert(r.head.rel.to_string());
                    continue;
                }
                // Each ask is a rule of its own (`dem(rr_dn) :- ask`), so its variables need only stay clear of `rr_dn`.
                // Named from the reading rule alone: a name from the reader's place in the program (its index) changed
                // the request's text, and every key made from it, with any rule added or dropped elsewhere.
                let name = |v: &str| if v == u { "rr_dn".to_owned() } else { format!("{v}_rr_d") };
                let ask: Vec<Literal> = wild_singletons(body, &u).iter().map(|l| rename_literal(l, &name)).collect();
                match asks.iter_mut().find(|(a, _)| *a == ask) {
                    Some((_, by)) => {
                        by.insert(r.head.rel.to_string());
                    }
                    None => asks.push((ask, BTreeSet::from([r.head.rel.to_string()]))),
                }
            }
        }
        (!asks.is_empty()).then_some((col, asks, blockers))
    }

    /// The atoms of `lits` joined to `n` through atoms, and the comparisons over what they bind.
    fn joined(lits: Vec<Literal>, n: &str) -> Vec<Literal> {
        let (atoms, rest): (Vec<Literal>, Vec<Literal>) = lits.into_iter().partition(|l| matches!(l, Literal::Atom(_) | Literal::Ast { .. }));
        let mut reach: BTreeSet<String> = BTreeSet::from([n.to_owned()]);
        let mut keep = vec![false; atoms.len()];
        loop {
            let mut grew = false;
            for (l, k) in atoms.iter().zip(keep.iter_mut()) {
                let vars = l.vars();
                if !*k && !vars.is_disjoint(&reach) {
                    *k = true;
                    reach.extend(vars);
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        let mut keep = keep.into_iter();
        let mut out: Vec<Literal> = atoms.into_iter().filter(|_| keep.next().unwrap_or(false)).collect();
        out.extend(rest.into_iter().filter(|l| matches!(l, Literal::Cmp { .. }) && l.vars().is_subset(&reach)));
        out
    }

    /// The literals a request left out, and why, for the profile.
    fn left_out(dropped: &[(String, &'static str)]) -> String {
        if dropped.is_empty() {
            return String::new();
        }
        format!("; left out: {}", dropped.iter().map(|(l, why)| format!("`{l}` ({why})")).collect::<Vec<_>>().join(", "))
    }

    /// `body` widened: without literal `skip`, the attributes, negations, aggregates, and literals that read `out`;
    /// a relation that reads an attribute read through its widened copy; joined to `to` (where given); the
    /// comparisons over what the rest binds. With the variables it binds, and what it left out (but `skip`), and why.
    fn widen(&self, body: &[Literal], skip: Option<usize>, out: &BTreeSet<String>, to: Option<&String>) -> (Vec<Literal>, BTreeSet<String>, Vec<(String, &'static str)>) {
        let mut dropped: Vec<(String, &'static str)> = Vec::new();
        let mut lits: Vec<Literal> = Vec::new();
        for (j, l) in body.iter().enumerate() {
            if Some(j) == skip {
                continue;
            }
            if !l.vars().is_disjoint(out) {
                dropped.push((l.to_string(), "reads the value"));
                continue;
            }
            match l {
                Literal::Atom(a) if a.neg => dropped.push((l.to_string(), "a negation")),
                Literal::Atom(a) if self.attrs.contains(&*a.rel) => dropped.push((l.to_string(), "an attribute on request")),
                Literal::Atom(a) => match self.cols.get(&*a.rel) {
                    Some(cols) if cols.is_empty() => dropped.push((l.to_string(), "its widened copy keeps no column")),
                    Some(cols) => lits.push(Literal::Atom(Atom { rel: self.wide(&a.rel).into(), args: cols.iter().map(|&c| a.args[c].clone()).collect(), neg: false, gated: false })),
                    None => lits.push(l.clone()),
                },
                Literal::Agg { .. } => dropped.push((l.to_string(), "an aggregate")),
                _ => lits.push(l.clone()),
            }
        }
        if let Some(n) = to {
            let mut keep = vec![false; lits.len()];
            let mut reach: BTreeSet<String> = BTreeSet::from([n.clone()]);
            loop {
                let mut grew = false;
                for (l, k) in lits.iter().zip(keep.iter_mut()) {
                    let vars = l.vars();
                    if !*k && !vars.is_disjoint(&reach) {
                        *k = true;
                        reach.extend(vars);
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
            let mut keep = keep.into_iter();
            lits.retain(|l| {
                let k = keep.next().unwrap_or(false);
                if !k {
                    dropped.push((l.to_string(), "not joined to the node"));
                }
                k
            });
        }
        // The atoms and matchers bind; a `nearest` and a comparison stay where what they read is bound, an
        // equality binding one more variable from it.
        let mut bound: BTreeSet<String> = lits.iter().filter(|l| matches!(l, Literal::Atom(_) | Literal::Ast { .. })).flat_map(Literal::vars).collect();
        let (mut kept, mut left): (Vec<Literal>, Vec<Literal>) = lits.into_iter().partition(|l| matches!(l, Literal::Atom(_) | Literal::Ast { .. }));
        loop {
            let before = kept.len();
            left.retain(|l| {
                let vars = l.vars();
                let unbound: Vec<&String> = vars.iter().filter(|v| !bound.contains(*v)).collect();
                let ok = match l {
                    Literal::Cmp { l: x, op, r: y } => {
                        unbound.is_empty() || *op == "=" && unbound.len() == 1 && [x, y].iter().any(|t| matches!(t, Term::Var(v) if v == unbound[0]))
                    }
                    Literal::Nearest { n, .. } => bound.contains(n),
                    _ => false,
                };
                if ok {
                    bound.extend(vars);
                    kept.push(l.clone());
                }
                !ok
            });
            if kept.len() == before {
                break;
            }
        }
        dropped.extend(left.iter().map(|l| (l.to_string(), "reads what nothing kept binds")));
        (kept, bound, dropped)
    }

    /// The widened copies `rules` read, and those they read in turn.
    fn rules(&self, rules: &[Rule]) -> Widened {
        let mut out = Widened::default();
        let reads = |rules: &[Rule]| -> Vec<String> {
            rules.iter().flat_map(|r| &r.body).filter_map(Literal::atom).filter_map(|a| a.rel.strip_prefix(self.prefix.as_str()).map(str::to_owned)).collect()
        };
        let mut todo = reads(rules);
        let mut done: BTreeSet<String> = BTreeSet::new();
        while let Some(rel) = todo.pop() {
            if !done.insert(rel.clone()) {
                continue;
            }
            let cols = &self.cols[&rel];
            let wide = self.wide(&rel);
            let decl = self.prog.decls.get(rel.as_str());
            out.decls.insert(wide.clone(), cols.iter().map(|&c| decl.and_then(|d| d.get(c)).map(|(n, t)| (n.to_string(), t.to_string())).unwrap_or_else(|| (format!("c{c}"), "symbol".to_owned()))).collect());
            if let Some(rows) = self.prog.facts.get(&rel) {
                out.facts.insert(wide.clone(), rows.iter().map(|row| cols.iter().map(|&c| row[c].clone()).collect()).collect());
            }
            let mut made = Vec::new();
            // A request relation's rules each serve their readers; a copy made of an ungated rule is its own reader.
            let asked = self.prog.asked_by.get(&rel).filter(|by| by.len() == self.prog.rules.iter().filter(|r| r.head.rel == rel.as_str()).count());
            let mut readers: Vec<BTreeSet<String>> = Vec::new();
            for (k, rule) in self.prog.rules.iter().filter(|r| r.head.rel == rel).enumerate() {
                let (body, ..) = self.widen(&rule.body, None, &BTreeSet::new(), None);
                let head = Atom { rel: wide.clone().into(), args: cols.iter().map(|&c| rule.head.args[c].clone()).collect(), neg: false, gated: false };
                let r = Rule { head, body: body.into() };
                let by = asked.map_or_else(|| BTreeSet::from([wide.clone()]), |a| a[k].clone());
                match made.iter().position(|m| *m == r) {
                    Some(at) => readers[at].extend(by),
                    None => {
                        made.push(r);
                        readers.push(by);
                    }
                }
            }
            if asked.is_some() {
                out.asked.insert(wide.clone(), readers);
            }
            todo.extend(reads(&made));
            out.rules.extend(made);
            out.copies.push((rel, cols.clone()));
        }
        out
    }
}

/// `body` with each variable that only one of its atoms reads once, other than `head`'s, a wildcard; and without
/// each positive atom another of the same relation is an instance of (`kind(_, k)` beside `kind(e, k)`), which only
/// tests what that one holds. To a fixpoint: a dropped atom can leave another variable read once (decision 0291).
fn wild_singletons(mut body: Vec<Literal>, head: &str) -> Vec<Literal> {
    fn count(t: &Term, n: &mut BTreeMap<String, usize>) {
        match t {
            Term::Var(v) => *n.entry(v.to_string()).or_default() += 1,
            Term::Call(_, args) => args.iter().for_each(|a| count(a, n)),
            _ => {}
        }
    }
    loop {
        let mut n: BTreeMap<String, usize> = BTreeMap::new();
        for l in &body {
            match l {
                Literal::Atom(a) => a.args.iter().for_each(|t| count(t, &mut n)),
                // Any other literal's variables are read where they are, never wildcards.
                other => other.vars().into_iter().for_each(|v| *n.entry(v).or_default() += 2),
            }
        }
        let wild = |t: &Term| matches!(t, Term::Var(v) if v != head && n.get(&**v) == Some(&1));
        body = body
            .into_iter()
            .map(|l| match l {
                Literal::Atom(a) if !a.neg && a.args.iter().any(wild) => Literal::Atom(Atom { args: a.args.iter().map(|t| if wild(t) { Term::Wild } else { t.clone() }).collect(), ..a }),
                other => other,
            })
            .collect();
        let instance = |a: &Atom, b: &Atom| a.rel == b.rel && !b.neg && a.args.len() == b.args.len() && a.args.iter().zip(b.args.iter()).all(|(x, y)| matches!(x, Term::Wild) || x == y);
        let Some(i) = (0..body.len()).find(|&i| {
            matches!(&body[i], Literal::Atom(a) if !a.neg && (0..body.len()).any(|j| j != i && matches!(&body[j], Literal::Atom(b) if instance(a, b) && !(instance(b, a) && j > i))))
        }) else {
            return body;
        };
        body.remove(i);
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

pub const WORLD: [(&str, &str); 3] = [("dir_entry", "rr_dir_req"), ("file_line", "rr_file_req"), ("file_digest", "rr_digest_req")];

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

/// Whether a literal binds every variable of its columns: a positive atom of a relation, not a text test nor a
/// built-in the lowering writes out as tests or computations of variables bound elsewhere (`within`, `earlier`,
/// `split_dot`, `nearest`, `precedes_stop`, …).
fn binds_columns(l: &Lit) -> bool {
    matches!(l, Lit::Atom { rel, neg: false, .. } if test_of(rel).is_none()
        && !matches!(rel.as_str(), "within" | "within_stop" | "earlier" | "split_dot" | "nearest" | "precedes_stop" | "dfs_first" | "dfs_first_memo" | "unset"))
}

/// Adjacent negations of one test of one string, `!glob("a", s), !glob("b", s)`, as one negated test of the set.
pub fn merge_negated_tests(body: Vec<Literal>) -> Vec<Literal> {
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
                Term::Str(_) => [last_p].into(),
                _ => {
                    out.push(literal);
                    continue;
                }
            };
            patterns.push(p);
            *last = Literal::Cmp { l: Term::Call(op, [s, Term::Call(SET.to_owned().into(), patterns)].into()), op: "=", r: Term::Int(0) };
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

/// `prog` reading the base relation `rel` (`child`, `node`): declared as the engine declares it, and an input unless a
/// rule makes it. For rules added to a program already read (`corpus::bridge`), as lowering adds them for its own.
pub fn base_relation(prog: &mut Program, rel: &str) {
    let derived = prog.rules.iter().any(|r| r.head.rel == rel);
    base_relation_derived(prog, rel, derived);
}

/// `base_relation`, told whether a rule derives `rel` (`derived`): `prog` may be a header alone.
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
