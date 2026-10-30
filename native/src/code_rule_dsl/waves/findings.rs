//! The first slice's findings layer (06-first-slice S5; architecture §8): after the wave's fixpoint,
//! one non-recursive program over the match-fact families turns matches into finding keys and
//! feeds nothing back. It is the pack's `violation` rule (`FINDINGS`), emitted by the planner as the
//! wave's step is, and run once (`schedule::layer`): each match's anchored byte (`span`, from the
//! wave), its clause's violation shape, the document's lines and the suppression comments that
//! silence it give `finding(rule, code, d, line)`. Each rule's findings are a family
//! (`finding:<rule>`), so an unchanged finding set cuts off; `Findings::publish` writes each
//! (rule, document)'s keys with its coverage.
//!
//! The program reads, besides `match` and `span`, facts of the clauses and documents written as
//! tables before it runs:
//! - `shape(c, r, k, a, i)`: clause `c`'s rule and code (`Layer::rules`, `Layer::codes` from 1),
//!   its anchor (0 the witness's start, 1 its end, 2 the file's start, 3 a capture) and the
//!   `rerail-ignore` name that silences it (from 1; 0 none, or one that only marks);
//! - `noqa(c, k)`: the codes a `noqa` directive must name to silence clause `c`;
//! - `applies(c, d, h)`: clause `c` reads document `d` (its code rule's scope, its query's `when`
//!   and `file` gates, as W's `grammar_matcher`), honouring linter comments there when `h` is 1;
//! - `line(d, l, s, e)`: line `l` of `d` holds the bytes `s..e` (the last runs one past the end);
//! - `exempt(d, k)`, `mapped(d, s, e)`, `directive(d, s, e, x, k)`: `d`'s `noqa` comments
//!   (`NoqaIndex`): file exemptions, the ranges that read the directive where they end, and each
//!   directive's line (`x` 1 where it also holds at the line's end), with `k` 0 for every code;
//! - `nosec(c, k)`, `bandit(d, l, k)`, `after(d, l, p)`: the Bandit tests a `# nosec` comment must
//!   name to silence clause `c`, the comment on line `l` of `d` (`k` 0 for its presence, else a
//!   test it names; `NosecIndex`), and `p`, the first line from `l` holding one;
//! - `cover(d, i, s, e)`: a `rerail-ignore` comment naming `i` covers bytes `s..e`.
//!
//! Coverage is `Unknown` for a (rule, document) the layer does not decide yet: syntax errors in a
//! file a clause reads around them (W drops the witnesses meeting them), a capture anchor, a `noqa`
//! parent statement, and a rule's report ceiling (open item O27).

use super::owner::Lease;
use super::schedule::{self, Limits, Outcome, Program};
use crate::code_rule_dsl::eval::grammar_matcher;
use crate::code_rule_dsl::facts::Graph;
use crate::code_rule_dsl::facts::families::Families;
use crate::code_rule_dsl::facts::vtab::syntax::Document;
use crate::code_rule_dsl::findings::contract::{Coverage, FindingKey, Findings};
use crate::code_rule_dsl::logic::plan::emit;
use crate::code_rule_dsl::matcher::GrammarMatcher;
use crate::code_rule_dsl::model::{
    Anchor, ClauseKind, CompiledCodeRule, FileInput, Query, package_marker_directory, python_module_in,
};
use crate::code_rule_dsl::suppress::{NoqaIndex, RerailIgnores};
use crate::content_store::{ContentStore, DerivationInput, StoreError, StoreResult};
use ast_grep_core::{AstGrep, Doc as _};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

pub(in crate::code_rule_dsl) const PROGRAM: &str = "findings";
const RELATION: &str = "finding";

/// The pack's `violation` rule over the match facts and the clauses' and documents' facts above.
const FINDINGS: &str = "
.decl match(c: int, d: int, n: int)
.decl span(d: int, n: int, s: int, e: int)
.decl shape(c: int, r: int, k: int, a: int, i: int)
.decl noqa(c: int, k: int)
.decl applies(c: int, d: int, h: int)
.decl line(d: int, l: int, s: int, e: int)
.decl exempt(d: int, k: int)
.decl mapped(d: int, s: int, e: int)
.decl directive(d: int, s: int, e: int, x: int, k: int)
.decl nosec(c: int, k: int)
.decl bandit(d: int, l: int, k: int)
.decl after(d: int, l: int, p: int)
.decl cover(d: int, i: int, s: int, e: int)
.input match, span, shape, noqa, applies, line, exempt, mapped, directive, nosec, bandit, after, cover
.decl anchored(c: int, d: int, n: int, b: int)
.decl lined(c: int, d: int, n: int, l: int)
.decl first(d: int, n: int, l: int)
.decl last(d: int, n: int, l: int)
.decl guarded(c: int, d: int, n: int)
.decl tested(c: int, d: int, n: int, l: int)
.decl named(c: int, d: int, n: int)
.decl honoured(c: int, d: int, n: int, b: int)
.decl remapped(c: int, d: int, n: int)
.decl resolved(c: int, d: int, n: int, o: int)
.decl quiet(c: int, d: int, n: int)
.decl finding(r: int, k: int, d: int, l: int)
.output finding
anchored($c, $d, $n, $s) :- match($c, $d, $n), applies($c, $d, _), shape($c, _, _, 0, _), span($d, $n, $s, _).
anchored($c, $d, $n, $e) :- match($c, $d, $n), applies($c, $d, _), shape($c, _, _, 1, _), span($d, $n, _, $e).
anchored($c, $d, $n, 0) :- match($c, $d, $n), applies($c, $d, _), shape($c, _, _, 2, _).
lined($c, $d, $n, $l) :- anchored($c, $d, $n, $b), line($d, $l, $s, $e), $s <= $b, $b < $e.
honoured($c, $d, $n, $b) :- anchored($c, $d, $n, $b), applies($c, $d, 1), noqa($c, _).
remapped($c, $d, $n) :- honoured($c, $d, $n, $b), mapped($d, $s, $e), $s <= $b, $b < $e.
resolved($c, $d, $n, $e) :- honoured($c, $d, $n, $b), mapped($d, $s, $e), $s <= $b, $b < $e.
resolved($c, $d, $n, $b) :- honoured($c, $d, $n, $b), !remapped($c, $d, $n).
quiet($c, $d, $n) :- honoured($c, $d, $n, _), exempt($d, 0).
quiet($c, $d, $n) :- honoured($c, $d, $n, _), noqa($c, $k), exempt($d, $k).
quiet($c, $d, $n) :- resolved($c, $d, $n, $o), directive($d, $s, $e, _, 0), $s <= $o, $o < $e.
quiet($c, $d, $n) :- resolved($c, $d, $n, $o), directive($d, $s, $e, _, $k), noqa($c, $k), $s <= $o, $o < $e.
quiet($c, $d, $n) :- resolved($c, $d, $n, $o), directive($d, $s, $o, 1, 0), $s <= $o.
quiet($c, $d, $n) :- resolved($c, $d, $n, $o), directive($d, $s, $o, 1, $k), noqa($c, $k), $s <= $o.
quiet($c, $d, $n) :- anchored($c, $d, $n, $b), shape($c, _, _, $a, $i), $a != 1, cover($d, $i, $s, $e), $s <= $b, $b < $e.
quiet($c, $d, $n) :- anchored($c, $d, $n, $v), shape($c, _, _, 1, $i), span($d, $n, $w, $v), $w < $v, cover($d, $i, $s, $e), $s < $v, $v <= $e.
quiet($c, $d, $n) :- anchored($c, $d, $n, $v), shape($c, _, _, 1, $i), span($d, $n, $v, $v), cover($d, $i, $s, $e), $s <= $v, $v < $e.
first($d, $n, $l) :- span($d, $n, $b, _), line($d, $l, $s, $e), $s <= $b, $b < $e.
last($d, $n, $l) :- span($d, $n, $w, $v), $w < $v, line($d, $l, $s, $e), $s < $v, $v <= $e.
last($d, $n, $l) :- span($d, $n, $v, $v), first($d, $n, $l).
guarded($c, $d, $n) :- anchored($c, $d, $n, _), applies($c, $d, 1), nosec($c, _).
tested($c, $d, $n, $l) :- guarded($c, $d, $n), lined($c, $d, $n, $l), bandit($d, $l, 0).
tested($c, $d, $n, $p) :- guarded($c, $d, $n), first($d, $n, $f), last($d, $n, $z), after($d, $f, $p), $p <= $z.
named($c, $d, $n) :- tested($c, $d, $n, $l), bandit($d, $l, $k), $k > 0.
quiet($c, $d, $n) :- tested($c, $d, $n, _), !named($c, $d, $n).
quiet($c, $d, $n) :- tested($c, $d, $n, $l), bandit($d, $l, $k), nosec($c, $k).
finding($r, $k, $d, $l) :- lined($c, $d, $n, $l), !quiet($c, $d, $n), shape($c, $r, $k, _, _).
";

/// The tables the program's inputs read (`Layer::write`), each with its columns.
const TABLES: [(&str, &str); 11] = [
    ("shape", "c, r, k, a, i"),
    ("noqa", "c, k"),
    ("applies", "c, d, h"),
    ("line", "d, l, s, e"),
    ("exempt", "d, k"),
    ("mapped", "d, s, e"),
    ("directive", "d, s, e, x, k"),
    ("nosec", "c, k"),
    ("bandit", "d, l, k"),
    ("after", "d, l, p"),
    ("cover", "d, i, s, e"),
];

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
}

pub(in crate::code_rule_dsl) struct Layer {
    shapes: Vec<Shape>,
    /// Code rule ids: `r` is an index.
    rules: Vec<String>,
    /// Codes, violations' and `noqa` directives' alike: `k` is an index from 1.
    codes: Vec<String>,
    /// `rerail-ignore` names: `i` is an index from 1.
    ignores: Vec<String>,
}

/// Index `name` takes in `names`, from `base`, adding it if new.
fn intern(names: &mut Vec<String>, name: &str, base: usize) -> i64 {
    let at = names.iter().position(|known| known == name).unwrap_or_else(|| {
        names.push(name.to_owned());
        names.len() - 1
    });
    (at + base) as i64
}

impl Layer {
    pub(in crate::code_rule_dsl) fn new(shapes: Vec<Shape>) -> Self {
        let (mut rules, mut codes, mut ignores) = (Vec::new(), Vec::new(), Vec::new());
        for shape in &shapes {
            let violation = &shape.rule.clauses[shape.clause].violation;
            intern(&mut rules, &shape.rule.id, 0);
            intern(&mut codes, &violation.code, 1);
            for code in violation.suppress.as_ref().and_then(|s| s.noqa.as_ref()).iter().flat_map(|noqa| &noqa.codes) {
                intern(&mut codes, code, 1);
            }
            for test in violation.suppress.as_ref().and_then(|s| s.nosec.as_ref()).into_iter().flatten() {
                intern(&mut codes, test, 1);
            }
            if let Some(name) = &violation.ignored_by {
                intern(&mut ignores, name, 1);
            }
        }
        Self { shapes, rules, codes, ignores }
    }

    /// The layer's program: the planner's statements over the tables, with `finding` recorded as
    /// one family per code rule.
    pub(in crate::code_rule_dsl) fn program(&self, store: &ContentStore) -> StoreResult<Program> {
        let mut batch = HashMap::from([
            ("match".to_owned(), ("(SELECT c0, c1, c2 FROM match)".to_owned(), self.shapes.len() as f64)),
            ("span".to_owned(), ("(SELECT c0, c1, c2, c3 FROM span)".to_owned(), self.shapes.len() as f64)),
        ]);
        for (name, columns) in TABLES {
            let aliased = columns.split(", ").enumerate().map(|(i, c)| format!("{c} AS c{i}")).collect::<Vec<_>>().join(", ");
            batch.insert(name.to_owned(), (format!("(SELECT {aliased} FROM {})", table(name)), 100.0));
        }
        let intern = &mut |names: &[&str]| store.intern_atoms(names).map_err(|e| crate::code_rule_dsl::logic::plan::Error::Plan(e.to_string()));
        let mut program = emit::emit_batch(FINDINGS, PROGRAM, &batch, intern).map_err(invalid)?.program;
        program.partitions = vec![(RELATION.to_owned(), self.rules.clone())];
        Ok(program)
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
        program: &Program,
    ) -> StoreResult<(Findings, Outcome)> {
        let unknown = self.write(lease, graph, scope, documents)?;
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
        for ((rule, doc), coverage) in unknown {
            let keys = keys.remove(&(rule, doc)).unwrap_or_default();
            findings.publish(&self.rules[rule], doc, coverage, keys);
        }
        Ok((findings, outcome))
    }

    /// The shapes' identity, which the program's text does not hold.
    fn digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"rerail.findings/v1\0");
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
        for (c, shape) in self.shapes.iter().enumerate() {
            let violation = &shape.rule.clauses[shape.clause].violation;
            let r = self.rules.iter().position(|id| *id == shape.rule.id).expect("interned") as i64;
            let k = index_of(&self.codes, &violation.code).expect("interned");
            let a = match violation.anchor {
                Anchor::Span | Anchor::Start => 0,
                Anchor::End => 1,
                Anchor::FileStart => 2,
                Anchor::Capture(_) => 3,
            };
            let i = violation.ignored_by.as_deref().filter(|_| !violation.ignore_marks).and_then(|name| index_of(&self.ignores, name)).unwrap_or(0);
            insert("shape", &[c as i64, r, k, a, i])?;
            for code in violation.suppress.as_ref().and_then(|s| s.noqa.as_ref()).iter().flat_map(|noqa| &noqa.codes) {
                insert("noqa", &[c as i64, index_of(&self.codes, code).expect("interned")])?;
            }
            for test in violation.suppress.as_ref().and_then(|s| s.nosec.as_ref()).into_iter().flatten() {
                insert("nosec", &[c as i64, index_of(&self.codes, test).expect("interned")])?;
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
            let module = (!packages.is_empty()).then(|| python_module_in(&document.path, |dir| packages.contains(dir))).flatten();
            let file = FileInput::new(document.path.clone(), "python".to_owned(), Arc::new(source.clone()), module.as_ref(), None, "findings")
                .map_err(invalid)?;
            let errors = !tape.doc.syntax_error_ranges().is_empty();
            let (mut noqa, mut nosec) = (false, false);
            for (c, shape) in self.shapes.iter().enumerate() {
                let (rule, grammar) = (&shape.rule, shape.grammar());
                let violation = &rule.clauses[shape.clause].violation;
                let reads = rule.scope.matches(&document.path)
                    && grammar_matcher(std::slice::from_ref(grammar), &file).is_some()
                    && grammar.invalid_syntax.is_none_or(|invalid| invalid == errors);
                if !reads {
                    continue;
                }
                let honours = rule.scope.honours_suppressions_at(&document.path);
                insert("applies", &[c as i64, d, i64::from(honours)])?;
                let suppress = violation.suppress.as_ref();
                noqa |= honours && suppress.is_some_and(|s| s.noqa.is_some());
                nosec |= honours && suppress.is_some_and(|s| s.nosec.is_some());
                let lower = || source.to_ascii_lowercase();
                let reason = if errors && grammar.invalid_syntax.is_none() {
                    Some("syntax-errors")
                } else if matches!(violation.anchor, Anchor::Capture(_)) && matched.contains(&(c as i64, d)) {
                    Some("capture-anchor")
                } else if honours && suppress.and_then(|s| s.noqa.as_ref()).is_some_and(|n| n.parent) && lower().contains("noqa") {
                    Some("noqa-parent")
                } else if rule.report_count_gt.is_some() {
                    Some("report-ceiling")
                } else {
                    None
                };
                let r = self.rules.iter().position(|id| *id == rule.id).expect("interned");
                let entry = coverage.entry((r, document.key)).or_insert(Coverage::Complete);
                if let (Coverage::Complete, Some(reason)) = (&entry, reason) {
                    *entry = Coverage::Unknown(reason.to_owned());
                }
            }
            if !coverage.keys().any(|&(_, doc)| doc == document.key) {
                continue;
            }
            let lines = tape.doc.file_lines();
            let starts = lines.starts();
            for (l, &s) in starts.iter().enumerate() {
                let e = starts.get(l + 1).copied().unwrap_or(source.len() + 1);
                insert("line", &[d, l as i64, s as i64, e as i64])?;
            }
            let root = AstGrep::doc(tape.doc.clone());
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
            if nosec {
                let index = crate::code_rule_dsl::suppress::NosecIndex::read(&root.root(), source, lines);
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

    /// The (clause, document) pairs with a match.
    fn matched(&self, lease: &Lease<'_>) -> StoreResult<BTreeSet<(i64, i64)>> {
        Ok(lease
            .prepare("SELECT DISTINCT c0, c1 FROM match")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }
}

/// The index from 1 of `name` in `names`.
fn index_of(names: &[String], name: &str) -> Option<i64> {
    names.iter().position(|known| known == name).map(|at| at as i64 + 1)
}
