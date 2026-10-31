//! The first slice's findings layer (06-first-slice S5; architecture §8): after the wave's fixpoint,
//! one non-recursive program over the match-fact families turns matches into finding keys and
//! feeds nothing back. It is the pack's `violation` rule (`FINDINGS`), emitted by the planner as the
//! wave's step is, and run once (`schedule::layer`): each match's violations (`anchored`: one per
//! anchored byte), less those a syntax error region or a suppression comment drops (`quiet`) and
//! those of a rule under its report ceiling, give `finding(rule, code, d, line)`. Each rule's
//! findings are a family (`finding:<rule>`), so an unchanged finding set cuts off;
//! `Findings::publish` writes each (rule, document)'s keys with its coverage.
//!
//! The program reads, besides `match` and `span` (the wave's), facts of the clauses and documents
//! written as tables before it runs (`Layer::write`):
//! - `shape(c, r, k, a, i, g)`: clause `c`'s rule and code (`Layer::rules`, `Layer::codes` from 1),
//!   its anchor (0 the witness's start, 1 its end, 2 the file's start, 3 a regex group), the
//!   `rerail-ignore` name that silences it (from 1; 0 none, or one that only marks), and `g` 1
//!   where `each` names a regex group: a violation per binding, none where it does not bind;
//! - `group(c, d, n, s)`: where the group clause `c` anchors at (or its `each`) binds in node `n`'s
//!   text, as W's `node_groups` and `each_node_group_set` read the clause's same-node regexes;
//! - `applies(c, d, h)`: clause `c` reads document `d` (its code rule's scope, its query's `when`,
//!   `file`, syntax and text gates, as W's `run_clause`), honouring linter comments when `h` is 1;
//! - `around(c)`, `error(d, s, e)`: the clause reads around syntax errors, and `d`'s error regions
//!   (`SyntaxErrors`); a witness meeting one is dropped;
//! - `line(d, l, s, e)`: line `l` of `d` holds the bytes `s..e` (the last runs one past the end);
//! - `noqa(c, k)`, `parented(c)`, `stmt(d, s, e)`: the codes a `noqa` directive must name to
//!   silence clause `c`, whether a directive at its enclosing statement's start does too, and `d`'s
//!   statements that can enclose one (`PARENT_KINDS`);
//! - `exempt(d, k)`, `mapped(d, s, e)`, `directive(d, s, e, x, k)`: `d`'s `noqa` comments
//!   (`NoqaIndex`): file exemptions, the ranges that read the directive where they end, and each
//!   directive's line (`x` 1 where it also holds at the line's end), with `k` 0 for every code;
//! - `nosec(c, k)`, `bandit(d, l, k)`, `after(d, l, p)`: the Bandit tests a `# nosec` comment must
//!   name to silence clause `c`, the comment on line `l` of `d` (`k` 0 for its presence, else a
//!   test it names; `NosecIndex`), and `p`, the first line from `l` holding one;
//! - `cover(d, i, s, e)`: a `rerail-ignore` comment naming `i` covers bytes `s..e`;
//! - `ceiling(r, m)`: rule `r` reports only with at least `m` violations (`reportCountGt` + 1; 0).
//!
//! Coverage is `Unknown` only where W's clause fails on the file: a file its parser did not read,
//! or syntax errors under a clause not sound around them (`source-incomplete`).

use super::owner::Lease;
use super::schedule::{self, Limits, Outcome};
use crate::code_rule_dsl::eval::grammar_matcher;
use crate::code_rule_dsl::facts::Graph;
use crate::code_rule_dsl::facts::families::Families;
use crate::code_rule_dsl::facts::vtab::syntax::Document;
use crate::code_rule_dsl::findings::contract::{Coverage, FindingKey, Findings};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::logic::plan::emit;
use crate::code_rule_dsl::matcher::{FileGates, GrammarMatcher};
use crate::code_rule_dsl::model::{
    Anchor, ClauseKind, CompiledCodeRule, FileInput, Query, ViolationSpec, package_marker_directory,
    python_module_in,
};
use crate::code_rule_dsl::soundness::{SyntaxErrors, SyntaxReading};
use crate::code_rule_dsl::suppress::{NoqaIndex, NosecIndex, PARENT_KINDS, RerailIgnores};
use crate::content_store::{ContentStore, DerivationInput, StoreError, StoreResult};
use ast_grep_core::{AstGrep, Doc as _};
use regex::Regex;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

pub(in crate::code_rule_dsl) const PROGRAM: &str = "findings";
const RELATION: &str = "finding";

/// The pack's `violation` rule over the match facts and the clauses' and documents' facts above.
const FINDINGS: &str = "
.decl match(c: int, d: int, n: int)
.decl span(d: int, n: int, s: int, e: int)
.decl shape(c: int, r: int, k: int, a: int, i: int, g: int)
.decl group(c: int, d: int, n: int, s: int)
.decl applies(c: int, d: int, h: int)
.decl around(c: int)
.decl error(d: int, s: int, e: int)
.decl line(d: int, l: int, s: int, e: int)
.decl noqa(c: int, k: int)
.decl parented(c: int)
.decl stmt(d: int, s: int, e: int)
.decl exempt(d: int, k: int)
.decl mapped(d: int, s: int, e: int)
.decl directive(d: int, s: int, e: int, x: int, k: int)
.decl nosec(c: int, k: int)
.decl bandit(d: int, l: int, k: int)
.decl after(d: int, l: int, p: int)
.decl cover(d: int, i: int, s: int, e: int)
.decl ceiling(r: int, m: int)
.input match, span, shape, group, applies, around, error, line, noqa, parented, stmt, exempt, mapped, directive, nosec, bandit, after, cover, ceiling
.decl grouped(c: int, d: int, n: int)
.decl kept(c: int, d: int, n: int)
.decl anchored(c: int, d: int, n: int, b: int)
.decl broken(c: int, d: int, n: int)
.decl lined(c: int, d: int, n: int, b: int, l: int)
.decl honoured(c: int, d: int, n: int, b: int)
.decl outer(c: int, d: int, n: int, b: int, p: int)
.decl point(c: int, d: int, n: int, b: int, o: int)
.decl remapped(c: int, d: int, n: int, b: int, o: int)
.decl resolved(c: int, d: int, n: int, b: int, o: int)
.decl first(d: int, n: int, l: int)
.decl last(d: int, n: int, l: int)
.decl guarded(c: int, d: int, n: int, b: int)
.decl tested(c: int, d: int, n: int, b: int, l: int)
.decl named(c: int, d: int, n: int, b: int)
.decl quiet(c: int, d: int, n: int, b: int)
.decl violation(r: int, c: int, d: int, n: int, b: int)
.decl total(r: int, x: int)
.decl finding(r: int, k: int, d: int, l: int)
.output finding
grouped($c, $d, $n) :- group($c, $d, $n, _).
kept($c, $d, $n) :- match($c, $d, $n), applies($c, $d, _), shape($c, _, _, _, _, 0).
kept($c, $d, $n) :- match($c, $d, $n), applies($c, $d, _), shape($c, _, _, _, _, 1), grouped($c, $d, $n).
anchored($c, $d, $n, $s) :- kept($c, $d, $n), shape($c, _, _, 0, _, _), span($d, $n, $s, _).
anchored($c, $d, $n, $e) :- kept($c, $d, $n), shape($c, _, _, 1, _, _), span($d, $n, _, $e).
anchored($c, $d, $n, 0) :- kept($c, $d, $n), shape($c, _, _, 2, _, _).
anchored($c, $d, $n, $s) :- kept($c, $d, $n), shape($c, _, _, 3, _, _), group($c, $d, $n, $s).
anchored($c, $d, $n, $s) :- kept($c, $d, $n), shape($c, _, _, 3, _, 0), span($d, $n, $s, _), !grouped($c, $d, $n).
broken($c, $d, $n) :- kept($c, $d, $n), around($c), span($d, $n, $w, $v), error($d, $s, $s), $w <= $s, $s <= $v.
broken($c, $d, $n) :- kept($c, $d, $n), around($c), span($d, $n, $w, $w), error($d, $s, $e), $s < $e, $s <= $w, $w < $e.
broken($c, $d, $n) :- kept($c, $d, $n), around($c), span($d, $n, $w, $v), $w < $v, error($d, $s, $e), $s < $e, $w < $e, $s < $v.
lined($c, $d, $n, $b, $l) :- anchored($c, $d, $n, $b), line($d, $l, $s, $e), $s <= $b, $b < $e.
honoured($c, $d, $n, $b) :- anchored($c, $d, $n, $b), applies($c, $d, 1), noqa($c, _).
point($c, $d, $n, $b, $b) :- honoured($c, $d, $n, $b).
outer($c, $d, $n, $b, $p) :- honoured($c, $d, $n, $b), parented($c), stmt($d, $p, $e), $p <= $b, $b < $e, stmt($d, $q, $f), $q <= $b, $b < $f, $p < $q.
point($c, $d, $n, $b, $p) :- honoured($c, $d, $n, $b), parented($c), stmt($d, $p, $e), $p <= $b, $b < $e, !outer($c, $d, $n, $b, $p).
remapped($c, $d, $n, $b, $o) :- point($c, $d, $n, $b, $o), mapped($d, $s, $e), $s <= $o, $o < $e.
resolved($c, $d, $n, $b, $e) :- point($c, $d, $n, $b, $o), mapped($d, $s, $e), $s <= $o, $o < $e.
resolved($c, $d, $n, $b, $o) :- point($c, $d, $n, $b, $o), !remapped($c, $d, $n, $b, $o).
first($d, $n, $l) :- span($d, $n, $b, _), line($d, $l, $s, $e), $s <= $b, $b < $e.
last($d, $n, $l) :- span($d, $n, $w, $v), $w < $v, line($d, $l, $s, $e), $s < $v, $v <= $e.
last($d, $n, $l) :- span($d, $n, $v, $v), first($d, $n, $l).
guarded($c, $d, $n, $b) :- anchored($c, $d, $n, $b), applies($c, $d, 1), nosec($c, _).
tested($c, $d, $n, $b, $l) :- guarded($c, $d, $n, $b), lined($c, $d, $n, $b, $l), bandit($d, $l, 0).
tested($c, $d, $n, $b, $p) :- guarded($c, $d, $n, $b), first($d, $n, $f), last($d, $n, $z), after($d, $f, $p), $p <= $z.
named($c, $d, $n, $b) :- tested($c, $d, $n, $b, $l), bandit($d, $l, $k), $k > 0.
quiet($c, $d, $n, $b) :- anchored($c, $d, $n, $b), broken($c, $d, $n).
quiet($c, $d, $n, $b) :- honoured($c, $d, $n, $b), exempt($d, 0).
quiet($c, $d, $n, $b) :- honoured($c, $d, $n, $b), noqa($c, $k), exempt($d, $k).
quiet($c, $d, $n, $b) :- resolved($c, $d, $n, $b, $o), directive($d, $s, $e, _, 0), $s <= $o, $o < $e.
quiet($c, $d, $n, $b) :- resolved($c, $d, $n, $b, $o), directive($d, $s, $e, _, $k), noqa($c, $k), $s <= $o, $o < $e.
quiet($c, $d, $n, $b) :- resolved($c, $d, $n, $b, $o), directive($d, $s, $o, 1, 0), $s <= $o.
quiet($c, $d, $n, $b) :- resolved($c, $d, $n, $b, $o), directive($d, $s, $o, 1, $k), noqa($c, $k), $s <= $o.
quiet($c, $d, $n, $b) :- tested($c, $d, $n, $b, _), !named($c, $d, $n, $b).
quiet($c, $d, $n, $b) :- tested($c, $d, $n, $b, $l), bandit($d, $l, $k), nosec($c, $k).
quiet($c, $d, $n, $b) :- anchored($c, $d, $n, $b), shape($c, _, _, $a, $i, _), $a != 1, cover($d, $i, $s, $e), $s <= $b, $b < $e.
quiet($c, $d, $n, $v) :- anchored($c, $d, $n, $v), shape($c, _, _, 1, $i, _), span($d, $n, $w, $v), $w < $v, cover($d, $i, $s, $e), $s < $v, $v <= $e.
quiet($c, $d, $n, $v) :- anchored($c, $d, $n, $v), shape($c, _, _, 1, $i, _), span($d, $n, $v, $v), cover($d, $i, $s, $e), $s <= $v, $v < $e.
violation($r, $c, $d, $n, $b) :- anchored($c, $d, $n, $b), !quiet($c, $d, $n, $b), shape($c, $r, _, _, _, _).
total($r, $x) :- ceiling($r, _), $x = count { violation($r, _, _, _, _) }.
finding($r, $k, $d, $l) :- violation($r, $c, $d, $n, $b), lined($c, $d, $n, $b, $l), shape($c, _, $k, _, _, _), ceiling($r, $m), total($r, $x), $x >= $m.
";

/// The tables the program's inputs read (`Layer::write`), each with its columns.
const TABLES: [(&str, &str); 17] = [
    ("shape", "c, r, k, a, i, g"),
    ("group", "c, d, n, s"),
    ("applies", "c, d, h"),
    ("around", "c"),
    ("error", "d, s, e"),
    ("line", "d, l, s, e"),
    ("noqa", "c, k"),
    ("parented", "c"),
    ("stmt", "d, s, e"),
    ("exempt", "d, k"),
    ("mapped", "d, s, e"),
    ("directive", "d, s, e, x, k"),
    ("nosec", "c, k"),
    ("bandit", "d, l, k"),
    ("after", "d, l, p"),
    ("cover", "d, i, s, e"),
    ("ceiling", "r, m"),
];

/// W's failure code for a clause that cannot read a file: the coverage it leaves `Unknown`.
const INCOMPLETE: &str = "source-incomplete";

fn invalid(error: impl std::fmt::Debug) -> StoreError {
    StoreError::Invalid(format!("{error:?}"))
}

/// The table input `name` of the program reads.
fn table(name: &str) -> String {
    format!("temp.finding_{name}")
}

/// One clause of the wave (its index there is its `c`): its code rule, the clause and the matcher
/// of the clause's query the wave lowered.
pub(in crate::code_rule_dsl) struct Shape {
    pub rule: Arc<CompiledCodeRule>,
    pub clause: usize,
    pub grammar: usize,
}

impl Shape {
    fn grammar(&self) -> &GrammarMatcher {
        let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &self.rule.clauses[self.clause].kind else {
            unreachable!("a wave clause is an ast forbid");
        };
        &matchers[self.grammar]
    }

    fn violation(&self) -> &ViolationSpec {
        &self.rule.clauses[self.clause].violation
    }
}

/// How a clause's violations read the regex groups of its witness's text.
#[derive(Clone, Copy, PartialEq)]
enum Groups {
    /// Neither its anchor nor its `each` names one.
    None,
    /// The anchor is the first binding of the group, the witness's span where it does not bind.
    First,
    /// `each` names the group: a violation per binding, none where it does not bind.
    Each,
}

/// The clause's same-node regexes (`node_regexes`) and how its violations read them, with the
/// group they read. A capture anchor or `each` must name a group of them: a metavariable's binding
/// is not in the match facts.
fn groups(violation: &ViolationSpec) -> StoreResult<(&[Regex], Groups, Option<&str>)> {
    let regexes = violation
        .node_regexes
        .iter()
        .find(|(dialect, _)| *dialect == GrammarDialect::Python)
        .map_or(&[][..], |(_, regexes)| regexes.as_slice());
    let names = |name: &str| regexes.iter().any(|regex| regex.capture_names().flatten().any(|group| group == name));
    let each = violation.each.as_deref().filter(|name| names(name));
    let anchor = match &violation.anchor {
        Anchor::Capture(name) => Some(name.as_str()),
        _ => None,
    };
    match (anchor, each) {
        (Some(name), _) if !names(name) => {
            Err(StoreError::Invalid(format!("{}: anchor capture.{name} is a metavariable, which the match facts do not bind", violation.code)))
        }
        (Some(name), Some(each)) if name != each => {
            Err(StoreError::Invalid(format!("{}: anchor capture.{name} under each {each}", violation.code)))
        }
        (_, Some(each)) => Ok((regexes, Groups::Each, Some(each))),
        (Some(name), None) => Ok((regexes, Groups::First, Some(name))),
        (None, None) => Ok((regexes, Groups::None, None)),
    }
}

/// The starts of `name`'s bindings in `text`, at byte `start` of the file: the first regex's that
/// binds it (`First`), or every binding of every regex holding it, each range once (`Each`).
fn bindings(regexes: &[Regex], groups: Groups, name: &str, text: &str, start: usize) -> Vec<usize> {
    match groups {
        Groups::None => Vec::new(),
        Groups::First => regexes
            .iter()
            .find_map(|regex| regex.captures(text)?.name(name).map(|group| start + group.start()))
            .into_iter()
            .collect(),
        Groups::Each => {
            let mut seen = BTreeSet::new();
            for regex in regexes.iter().filter(|regex| regex.capture_names().flatten().any(|group| group == name)) {
                for found in regex.captures_iter(text) {
                    if let Some(group) = found.name(name) {
                        seen.insert((start + group.start(), start + group.end()));
                    }
                }
            }
            seen.into_iter().map(|(start, _)| start).collect()
        }
    }
}

pub(in crate::code_rule_dsl) struct Layer {
    shapes: Vec<Shape>,
    /// Code rule ids: `r` is an index.
    rules: Vec<String>,
    /// Codes, violations' and `noqa` and `nosec` directives' alike: `k` is an index from 1.
    codes: Vec<String>,
    /// `rerail-ignore` names: `i` is an index from 1.
    ignores: Vec<String>,
}

/// Interns `name` in `names`.
fn intern(names: &mut Vec<String>, name: &str) {
    if !names.iter().any(|known| known == name) {
        names.push(name.to_owned());
    }
}

/// The index from 1 of `name` in `names`.
fn index_of(names: &[String], name: &str) -> Option<i64> {
    names.iter().position(|known| known == name).map(|at| at as i64 + 1)
}

impl Layer {
    pub(in crate::code_rule_dsl) fn new(shapes: Vec<Shape>) -> StoreResult<Self> {
        let (mut rules, mut codes, mut ignores) = (Vec::new(), Vec::new(), Vec::new());
        for shape in &shapes {
            let violation = shape.violation();
            groups(violation)?;
            intern(&mut rules, &shape.rule.id);
            intern(&mut codes, &violation.code);
            let suppress = violation.suppress.as_ref();
            for code in suppress.and_then(|s| s.noqa.as_ref()).iter().flat_map(|noqa| &noqa.codes) {
                intern(&mut codes, code);
            }
            for test in suppress.and_then(|s| s.nosec.as_ref()).into_iter().flatten() {
                intern(&mut codes, test);
            }
            if let Some(name) = &violation.ignored_by {
                intern(&mut ignores, name);
            }
        }
        Ok(Self { shapes, rules, codes, ignores })
    }

    /// The layer's program: the planner's statements over the tables, with `finding` recorded as
    /// one family per code rule, and the statements its steps read (`Emitted::setup`: the views).
    pub(in crate::code_rule_dsl) fn program(&self, store: &ContentStore) -> StoreResult<emit::Emitted> {
        let mut batch = HashMap::from([
            ("match".to_owned(), ("(SELECT c0, c1, c2 FROM match)".to_owned(), self.shapes.len() as f64)),
            ("span".to_owned(), ("(SELECT c0, c1, c2, c3 FROM span)".to_owned(), self.shapes.len() as f64)),
        ]);
        for (name, columns) in TABLES {
            let aliased = columns.split(", ").enumerate().map(|(i, c)| format!("{c} AS c{i}")).collect::<Vec<_>>().join(", ");
            batch.insert(name.to_owned(), (format!("(SELECT {aliased} FROM {})", table(name)), 100.0));
        }
        let intern = &mut |names: &[&str]| store.intern_atoms(names).map_err(|e| crate::code_rule_dsl::logic::plan::Error::Plan(e.to_string()));
        let mut emitted = emit::emit_batch(FINDINGS, PROGRAM, &batch, intern).map_err(invalid)?;
        emitted.program.partitions = vec![(RELATION.to_owned(), self.rules.clone())];
        Ok(emitted)
    }

    /// The findings of the wave's matches on `lease` over `documents` of `scope`: the layer's
    /// facts written, its program run once and its families recorded, each (rule, document) a
    /// clause reads published with its coverage.
    pub(in crate::code_rule_dsl) fn findings(
        &self,
        lease: &Lease<'_>,
        graph: &(impl Graph + ?Sized),
        scope: &str,
        documents: &[Document],
        emitted: &emit::Emitted,
    ) -> StoreResult<(Findings, Outcome)> {
        let coverage = self.write(lease, graph, scope, documents)?;
        // The setup's tables and views, replacing an earlier run's.
        for statement in &emitted.setup {
            for (kind, drop) in [("CREATE TEMP VIEW ", "VIEW"), ("CREATE TEMP TABLE ", "TABLE")] {
                if let Some(name) = statement.strip_prefix(kind).and_then(|rest| rest.split(['(', ' ']).next()) {
                    lease.execute_batch(&format!("DROP {drop} IF EXISTS temp.{name}"))?;
                }
            }
            lease.execute_batch(statement)?;
        }
        let program = &emitted.program;
        let mut inputs = documents.iter().map(|d| DerivationInput { name: Some(d.core.clone()), digest: d.digest }).collect::<Vec<_>>();
        inputs.push(DerivationInput { name: None, digest: self.digest() });
        let outcome = schedule::layer(lease, graph, program, scope, inputs, &Limits::default())?;
        let mut keys = BTreeMap::<(usize, u32), Vec<FindingKey>>::new();
        let rows: Vec<(i64, i64, i64, i64)> = lease
            .prepare(&format!("SELECT c0, c1, c2, c3 FROM {RELATION}"))?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (r, k, d, l) in rows {
            let (rule, doc) = (r as usize, d as u32);
            let key = FindingKey { rule: self.rules[rule].clone(), code: self.codes[k as usize - 1].clone(), doc, line: l as usize };
            keys.entry((rule, doc)).or_default().push(key);
        }
        let mut findings = Findings::default();
        for ((rule, doc), coverage) in coverage {
            let keys = keys.remove(&(rule, doc)).unwrap_or_default();
            findings.publish(&self.rules[rule], doc, coverage, keys);
        }
        Ok((findings, outcome))
    }

    /// The shapes' identity, which the program's text does not hold.
    fn digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"rerail.findings/v2\0");
        for shape in &self.shapes {
            hash.update(format!("{}\0{}\0{}\0{}\0", shape.rule.id, shape.rule.digest, shape.clause, shape.grammar).as_bytes());
        }
        hash.finalize().into()
    }

    /// Writes the program's tables for `documents`. Returns the coverage of each (rule, document)
    /// a clause reads.
    fn write(
        &self,
        lease: &Lease<'_>,
        graph: &(impl Graph + ?Sized),
        scope: &str,
        documents: &[Document],
    ) -> StoreResult<BTreeMap<(usize, u32), Coverage>> {
        for (name, columns) in TABLES {
            let typed = columns.split(", ").map(|c| format!("{c} INTEGER NOT NULL")).collect::<Vec<_>>().join(", ");
            lease.execute_batch(&format!("DROP TABLE IF EXISTS {0}; CREATE TABLE {0}({typed})", table(name)))?;
        }
        let insert = |name: &str, values: &[i64]| -> rusqlite::Result<()> {
            let marks = vec!["?"; values.len()].join(", ");
            lease.prepare_cached(&format!("INSERT INTO {} VALUES ({marks})", table(name)))?.execute(rusqlite::params_from_iter(values))?;
            Ok(())
        };
        let rule_of = |shape: &Shape| self.rules.iter().position(|id| *id == shape.rule.id).expect("interned");
        for (r, id) in self.rules.iter().enumerate() {
            let rule = &self.shapes.iter().find(|shape| shape.rule.id == *id).expect("a rule has a clause").rule;
            insert("ceiling", &[r as i64, rule.report_count_gt.map_or(0, |maximum| maximum as i64 + 1)])?;
        }
        for (c, shape) in self.shapes.iter().enumerate() {
            let violation = shape.violation();
            let c = c as i64;
            let k = index_of(&self.codes, &violation.code).expect("interned");
            let a = match violation.anchor {
                Anchor::Span | Anchor::Start => 0,
                Anchor::End => 1,
                Anchor::FileStart => 2,
                Anchor::Capture(_) => 3,
            };
            let i = violation.ignored_by.as_deref().filter(|_| !violation.ignore_marks).and_then(|name| index_of(&self.ignores, name)).unwrap_or(0);
            let g = i64::from(groups(violation)?.1 == Groups::Each);
            insert("shape", &[c, rule_of(shape) as i64, k, a, i, g])?;
            if shape.grammar().invalid_syntax.is_none() {
                insert("around", &[c])?;
            }
            let suppress = violation.suppress.as_ref();
            if let Some(noqa) = suppress.and_then(|s| s.noqa.as_ref()) {
                for code in &noqa.codes {
                    insert("noqa", &[c, index_of(&self.codes, code).expect("interned")])?;
                }
                if noqa.parent {
                    insert("parented", &[c])?;
                }
            }
            for test in suppress.and_then(|s| s.nosec.as_ref()).into_iter().flatten() {
                insert("nosec", &[c, index_of(&self.codes, test).expect("interned")])?;
            }
        }
        // A Python file with no host module is named by the package markers among the documents,
        // as W names a snapshot's (`parse_snapshot`).
        let packages = documents.iter().filter_map(|d| package_marker_directory(&d.path)).collect::<HashSet<_>>();
        let matched = self.matched(lease)?;
        let mut coverage = BTreeMap::new();
        for document in documents {
            let tape = Families::stored(graph, scope, &document.path)?
                .ok_or_else(|| StoreError::Invalid(format!("{} has no stored tape", document.path)))?;
            let d = i64::from(document.key);
            let source = tape.doc.get_source();
            let lines = tape.doc.file_lines();
            let module = (!packages.is_empty()).then(|| python_module_in(&document.path, |dir| packages.contains(dir))).flatten();
            let file = FileInput::new(document.path.clone(), "python".to_owned(), Arc::new(source.clone()), module.as_ref(), None, "findings")
                .map_err(invalid)?;
            let root = AstGrep::doc(tape.doc.clone());
            let gates = FileGates::new(&file, root.root());
            let errors = SyntaxErrors::of_doc(&tape.doc);
            let unread = !errors.is_empty() && matches!(errors.reading(lines, source.len()), SyntaxReading::Unread(_));
            let (mut noqa, mut parent, mut nosec, mut around) = (false, false, false, false);
            let mut searched = rustc_hash::FxHashMap::default();
            for (c, shape) in self.shapes.iter().enumerate() {
                let (rule, grammar, violation) = (&shape.rule, shape.grammar(), shape.violation());
                // W's `run_clause`, in its order: the scope and the `when` and `file` gates; the
                // syntax gate; a file its parser did not read fails a clause reading around errors;
                // the file gates; a clause not sound under errors fails; the text gate.
                if !rule.scope.matches(&document.path)
                    || grammar_matcher(std::slice::from_ref(grammar), &file).is_none()
                    || grammar.invalid_syntax.is_some_and(|invalid| invalid == errors.is_empty())
                {
                    continue;
                }
                let gated = grammar.invalid_syntax.is_none();
                let entry = coverage.entry((rule_of(shape), document.key)).or_insert(Coverage::Complete);
                if gated && unread {
                    *entry = Coverage::Unknown(INCOMPLETE.to_owned());
                    continue;
                }
                if !gates.admit(grammar) {
                    continue;
                }
                if gated && errors.incomplete(grammar) {
                    *entry = Coverage::Unknown(INCOMPLETE.to_owned());
                    continue;
                }
                if !grammar.admits_source(source, &errors, &mut searched) {
                    continue;
                }
                let honours = rule.scope.honours_suppressions_at(&document.path);
                insert("applies", &[c as i64, d, i64::from(honours)])?;
                let suppress = violation.suppress.as_ref().filter(|_| honours);
                noqa |= suppress.is_some_and(|s| s.noqa.is_some());
                parent |= suppress.and_then(|s| s.noqa.as_ref()).is_some_and(|noqa| noqa.parent);
                nosec |= suppress.is_some_and(|s| s.nosec.is_some());
                around |= gated;
                let (regexes, groups, Some(name)) = groups(violation)? else { continue };
                for &n in matched.get(&(c as i64, d)).into_iter().flatten() {
                    let span = tape.doc.transducer_span(n as u32);
                    for start in bindings(regexes, groups, name, &source[span.clone()], span.start) {
                        insert("group", &[c as i64, d, n, start as i64])?;
                    }
                }
            }
            if !coverage.keys().any(|&(_, doc)| doc == document.key) {
                continue;
            }
            let starts = lines.starts();
            for (l, &s) in starts.iter().enumerate() {
                let e = starts.get(l + 1).copied().unwrap_or(source.len() + 1);
                insert("line", &[d, l as i64, s as i64, e as i64])?;
            }
            if around {
                for region in errors.regions() {
                    insert("error", &[d, region.start as i64, region.end as i64])?;
                }
            }
            if noqa {
                let index = NoqaIndex::read(&root.root(), source, lines);
                let (all, codes) = index.exemptions();
                if all {
                    insert("exempt", &[d, 0])?;
                }
                for code in codes.iter().filter_map(|code| index_of(&self.codes, code)) {
                    insert("exempt", &[d, code])?;
                }
                for range in index.mapping() {
                    insert("mapped", &[d, range.start as i64, range.end as i64])?;
                }
                for (range, end, codes) in index.directives() {
                    let line = [d, range.start as i64, range.end as i64, i64::from(end)];
                    match codes {
                        None => insert("directive", &[&line[..], &[0]].concat())?,
                        Some(codes) => {
                            for code in codes.iter().filter_map(|code| index_of(&self.codes, code)) {
                                insert("directive", &[&line[..], &[code]].concat())?;
                            }
                        }
                    }
                }
            }
            if parent {
                for node in root.root().dfs().filter(|node| PARENT_KINDS.contains(&node.kind().as_ref())) {
                    let range = node.range();
                    insert("stmt", &[d, range.start as i64, range.end as i64])?;
                }
            }
            if nosec {
                let index = NosecIndex::read(&root.root(), source, lines);
                let commented = index.directives().map(|(line, _)| line).collect::<Vec<_>>();
                for (line, tests) in index.directives() {
                    insert("bandit", &[d, line as i64, 0])?;
                    for test in tests.iter().filter_map(|test| index_of(&self.codes, test)) {
                        insert("bandit", &[d, line as i64, test])?;
                    }
                }
                for l in 0..starts.len() {
                    if let Some(&p) = commented.get(commented.partition_point(|&line| line < l)) {
                        insert("after", &[d, l as i64, p as i64])?;
                    }
                }
            }
            if !self.ignores.is_empty() {
                for (range, names) in RerailIgnores::read(&root.root(), source).covers() {
                    for i in names.iter().filter_map(|name| index_of(&self.ignores, name)) {
                        let end = i64::try_from(range.end).unwrap_or(i64::MAX);
                        insert("cover", &[d, i, range.start as i64, end])?;
                    }
                }
            }
        }
        Ok(coverage)
    }

    /// Each (clause, document)'s matched nodes.
    fn matched(&self, lease: &Lease<'_>) -> StoreResult<BTreeMap<(i64, i64), Vec<i64>>> {
        let mut out = BTreeMap::<_, Vec<_>>::new();
        let rows: Vec<(i64, i64, i64)> = lease
            .prepare("SELECT c0, c1, c2 FROM match")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (c, d, n) in rows {
            out.entry((c, d)).or_default().push(n);
        }
        Ok(out)
    }
}
