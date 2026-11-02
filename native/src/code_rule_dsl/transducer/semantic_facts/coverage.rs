//! O24's coverage: of the Python pack's `ast` clauses S1's lowering refuses (`match_tree`), how
//! many have every semantic input they need once the producers' labels exist, by predicate, and
//! what the lowering would refuse them for next.
//!
//! "Next" is measured, not guessed: each clause is rewritten with every semantic leaf a producer
//! answers replaced by a stand-in label leaf (`Never`, beside the leaf's sub-rules, which become
//! labels of their own and stay in the walk), utilities included, and `match_tree` reads the
//! rewrite. The stand-in binds nothing, so captures a semantic leaf binds (`arguments`, `parse`)
//! are not counted as shared bindings: those are joins on capture columns (O24's direction).

use super::harness::{policies, python_matchers, semantic_leaves};
use super::{binds, resolves};
use crate::code_rule_dsl::matcher::{
    AstMatcher, Constrained, FieldRelation, GrammarMatcher, Semantic, StopBy, UtilCall, UtilRef,
    UtilTable, match_tree,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock};

/// A semantic predicate's name, as S1's refusals name it.
fn predicate(semantic: &Semantic) -> &'static str {
    match semantic {
        Semantic::Resolves(_) => "resolves",
        Semantic::Binding(_) => "resolves.binding",
        Semantic::Binds(_) => "binds",
        Semantic::Value(_) => "value",
        Semantic::Arguments(_) => "arguments",
        Semantic::Parse(_) => "parse",
        Semantic::Host(_) => "host",
        Semantic::Embedded(_) => "embedded",
        Semantic::Capture(_) => "capture",
        Semantic::FormatArgs(_) => "formatArgs",
        Semantic::Layout(_) => "layout",
        Semantic::FilePath(_) => "file path",
        Semantic::Calls(_) => "calls",
    }
}

/// Whether a producer's rows answer this leaf: `Err` names why not.
fn produced(semantic: &Semantic) -> Result<(), String> {
    match semantic {
        Semantic::Resolves(form) => {
            resolves::admits(form).map_err(|refused| format!("resolves: {}", refused.0))
        }
        Semantic::Binds(form) => {
            binds::admits(form).map_err(|refused| format!("binds: {}", refused.0))
        }
        other => Err(format!("{}: no producer", predicate(other))),
    }
}

/// A leaf's sub-rules, which the label world lowers as labels of their own.
fn sub_rules(semantic: &Semantic) -> Vec<&AstMatcher> {
    match semantic {
        Semantic::Binding(test) => test.sub_rules().collect(),
        Semantic::Binds(test) => test.sub_rules().collect(),
        Semantic::Host(test) => vec![&test.rule],
        Semantic::Parse(test) => vec![&test.rule],
        Semantic::Embedded(test) => test.rules().collect(),
        Semantic::FormatArgs(test) => test.unused.iter().collect(),
        _ => Vec::new(),
    }
}

/// Rewrites a clause's matcher with the leaves `replace` accepts as stand-in labels, its utility
/// tables copied with them (a table's rules refer to the table, so each is rewritten once).
struct Rewrite<'a> {
    replace: &'a dyn Fn(&Semantic) -> bool,
    tables: HashMap<usize, Arc<UtilTable>>,
}

impl Rewrite<'_> {
    fn table(&mut self, util: &UtilRef) -> Option<UtilRef> {
        let old = util.table.upgrade()?;
        let key = Arc::as_ptr(&old) as usize;
        let table = match self.tables.get(&key) {
            Some(table) => Arc::clone(table),
            None => {
                let new = Arc::new(UtilTable::new(
                    old.names.clone(),
                    old.framed.clone(),
                    old.origins.clone(),
                ));
                self.tables.insert(key, Arc::clone(&new));
                let rules = old.rules.get().or_else(|| old.draft.get())?;
                let rules = rules.iter().map(|rule| self.matcher(rule)).collect();
                let _ = new.rules.set(rules);
                new
            }
        };
        Some(UtilRef {
            table: Arc::downgrade(&table),
            index: util.index,
        })
    }

    fn stop(&mut self, stop: &StopBy) -> StopBy {
        match stop {
            StopBy::Neighbor => StopBy::Neighbor,
            StopBy::End => StopBy::End,
            StopBy::Rule(rule) => StopBy::Rule(Box::new(self.matcher(rule))),
        }
    }

    fn matcher(&mut self, matcher: &AstMatcher) -> AstMatcher {
        match matcher {
            AstMatcher::Semantic(semantic) if (self.replace)(semantic) => AstMatcher::All(
                std::iter::once(AstMatcher::Never)
                    .chain(
                        sub_rules(semantic)
                            .into_iter()
                            .map(|rule| self.matcher(rule)),
                    )
                    .collect(),
            ),
            AstMatcher::All(items) => {
                AstMatcher::All(items.iter().map(|item| self.matcher(item)).collect())
            }
            AstMatcher::Any(items) => {
                AstMatcher::Any(items.iter().map(|item| self.matcher(item)).collect())
            }
            AstMatcher::Not(item) => AstMatcher::Not(Box::new(self.matcher(item))),
            AstMatcher::Inside(related, stop) => AstMatcher::Inside(
                related.with_rule(self.matcher(related.rule())),
                self.stop(stop),
            ),
            AstMatcher::Has(related, stop) => AstMatcher::Has(
                related.with_rule(self.matcher(related.rule())),
                self.stop(stop),
            ),
            AstMatcher::Precedes(related, stop) => AstMatcher::Precedes(
                related.with_rule(self.matcher(related.rule())),
                self.stop(stop),
            ),
            AstMatcher::Follows(related, stop) => AstMatcher::Follows(
                related.with_rule(self.matcher(related.rule())),
                self.stop(stop),
            ),
            AstMatcher::Field(relation) => AstMatcher::Field(Box::new(FieldRelation {
                inside: relation.inside,
                field: relation.field,
                rule: self.matcher(&relation.rule),
                stop: self.stop(&relation.stop),
                filter: OnceLock::new(),
            })),
            AstMatcher::NthChild(nth) => {
                let mut nth = nth.clone();
                nth.of_rule = nth.of_rule.as_ref().map(|rule| self.matcher(rule));
                AstMatcher::NthChild(nth)
            }
            AstMatcher::Count(count) => {
                let mut count = count.clone();
                count.relation = self.matcher(&count.relation);
                AstMatcher::Count(count)
            }
            AstMatcher::Constrained(constrained) => {
                AstMatcher::Constrained(Box::new(Constrained {
                    rule: self.matcher(&constrained.rule),
                    constraints: constrained
                        .constraints
                        .iter()
                        .map(|(name, rule)| (name.clone(), self.matcher(rule)))
                        .collect(),
                }))
            }
            AstMatcher::Util(util) => self
                .table(util)
                .map_or_else(|| matcher.clone(), AstMatcher::Util),
            AstMatcher::Call(call) => match self.table(&call.util) {
                Some(util) => AstMatcher::Call(Box::new(UtilCall {
                    util,
                    args: call.args.clone(),
                })),
                None => matcher.clone(),
            },
            other => other.clone(),
        }
    }
}

/// What `match_tree` refuses `grammar` for once the leaves `replace` accepts are labels; `None`
/// where it lowers.
fn refusal_with_labels(
    grammar: &GrammarMatcher,
    replace: &dyn Fn(&Semantic) -> bool,
) -> Option<&'static str> {
    let mut rewrite = Rewrite {
        replace,
        tables: HashMap::new(),
    };
    let mut rewritten = grammar.clone();
    rewritten.matcher = rewrite.matcher(&grammar.matcher);
    match_tree(&rewritten)
        .err()
        .map(|unsupported| unsupported.0)
}

#[test]
fn semantic_labels_coverage_of_s1s_refused_clauses() {
    let mut total = 0;
    let mut refused = 0;
    let mut first = BTreeMap::<&str, usize>::new();
    // By the predicates a refused clause reads: clauses, and of those, how many have every
    // semantic input from a producer.
    let mut by_predicates = BTreeMap::<String, (usize, usize)>::new();
    let mut fed = 0;
    let mut missing = BTreeMap::<String, usize>::new();
    let mut after = BTreeMap::<&str, usize>::new();
    let mut after_all = BTreeMap::<&str, usize>::new();
    let (mut lowered, mut lowered_all) = (0, 0);
    for (policy, compiled) in &policies() {
        for (clause, grammar) in python_matchers(compiled) {
            total += 1;
            let Err(reason) = match_tree(grammar) else {
                continue;
            };
            refused += 1;
            *first.entry(reason.0).or_default() += 1;
            let leaves = semantic_leaves(&grammar.matcher);
            let predicates = leaves
                .iter()
                .map(predicate)
                .collect::<std::collections::BTreeSet<_>>();
            let gaps = leaves
                .iter()
                .filter_map(|leaf| produced(leaf).err())
                .collect::<std::collections::BTreeSet<_>>();
            let entry = by_predicates
                .entry(predicates.into_iter().collect::<Vec<_>>().join("+"))
                .or_default();
            entry.0 += 1;
            if gaps.is_empty() {
                entry.1 += 1;
                fed += 1;
            }
            for gap in gaps {
                *missing.entry(gap).or_default() += 1;
            }
            match refusal_with_labels(grammar, &|leaf| produced(leaf).is_ok()) {
                None => lowered += 1,
                Some(reason) => *after.entry(reason).or_default() += 1,
            }
            match refusal_with_labels(grammar, &|_| true) {
                None => lowered_all += 1,
                Some(reason) => *after_all.entry(reason).or_default() += 1,
            }
            if std::env::var_os("RERAIL_O24_LIST").is_some() {
                eprintln!("  {policy}/{clause}: {}", reason.0);
            }
        }
    }
    eprintln!(
        "O24 coverage: {refused} of {total} Python AST clauses refused by S1's lowering; first refusal: {first:?}"
    );
    eprintln!("  every semantic input from a producer (resolves, binds): {fed} of {refused}");
    eprintln!("  by the predicates a clause reads (clauses, of which fed):");
    for (predicates, (clauses, fed)) in &by_predicates {
        let predicates = if predicates.is_empty() {
            "none"
        } else {
            predicates
        };
        eprintln!("    {predicates}: {clauses}, {fed}");
    }
    eprintln!("  leaves without rows, by clause: {missing:?}");
    eprintln!(
        "  with resolves and binds as labels: {lowered} more lower; still refused: {after:?}"
    );
    eprintln!(
        "  with every semantic leaf a label: {lowered_all} more lower; still refused: {after_all:?}"
    );
    assert_eq!(refused, first.values().sum::<usize>());
}
