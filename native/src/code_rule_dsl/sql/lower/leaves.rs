//! Lowering of the semantic leaves whose sub-rules the SQL engine composes. A terminal returns
//! what the matcher reads before each sub-rule (a binding's nodes into internal slots, a list
//! of candidates as related rows), and each sub-rule lowers as a relation over them: its first
//! match in the matcher's order binds, strict where the matcher's search is. `Op::Seq` keeps the
//! matcher's order of the steps, the first that is not true deciding.

use super::{Bound, INTERNAL, Lowerer, unsupported};
use crate::code_rule_dsl::matcher::{
    AstMatcher, BindingFilter, BindingTest, Binds, CaptureAnalysis, FormatArgs, ParseTest,
    Semantic, may_bind,
};
use crate::code_rule_dsl::sql::plan::{
    AllTerm, Export, Op, SlotId, SlotKind, Source, Unsupported, Walk,
};
use crate::code_rule_dsl::sql::terminals::{
    Binder, BindingSites, BindsCandidates, BindsOthers, Exported, FactTerm, FormatPart,
    FormatSides, Occurrences, Parsed, Terminal, Verdicts,
};
use std::collections::BTreeSet;

/// Where a parsed tree's rule is lowered (`Op::Parse`): the slots holding its host's file and
/// node and the tree's number, the host's scope, and the tree's own.
#[derive(Clone, Copy)]
pub(super) struct TreeContext {
    host: (SlotId, SlotId),
    tree: SlotId,
    host_scope: usize,
    scope: usize,
}

/// The captures a binding filter reads ranges of (`bindings.within`).
fn filter_reads(filter: &BindingFilter, out: &mut BTreeSet<String>) {
    if let Some(within) = filter
        .bindings
        .as_ref()
        .and_then(|count| count.within.as_ref())
    {
        out.insert(within.clone());
    }
}

impl Lowerer<'_> {
    pub(super) fn semantic(
        &mut self,
        matcher: &AstMatcher,
        semantic: &Semantic,
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        match semantic {
            Semantic::Binding(test) => self.binding(test, scope, bound),
            Semantic::Binds(test) => self.binds(test, scope, bound),
            Semantic::Arguments(arguments) => {
                let binds: Vec<(String, SlotKind)> = arguments
                    .bind
                    .iter()
                    .map(|(name, _)| (name.clone(), SlotKind::Node))
                    .collect();
                self.binder(matcher, binds, scope, bound)
            }
            Semantic::Capture(name) => {
                self.binder(matcher, vec![(name.clone(), SlotKind::Node)], scope, bound)
            }
            Semantic::Calls(test) => match &test.chain {
                Some(chain) => {
                    self.binder(matcher, vec![(chain.clone(), SlotKind::Text)], scope, bound)
                }
                None => self.leaf(matcher, scope, &BTreeSet::new(), true, bound),
            },
            Semantic::FormatArgs(test) => self.format_args(test, scope, bound),
            Semantic::Parse(test) => self.parse(test, scope, bound),
            Semantic::Host(test) => self.host(&test.rule, bound),
            // A rule over the program the file embeds reads another document of the file.
            Semantic::Embedded(_) => unsupported("embedded"),
            Semantic::Resolves(_) | Semantic::Value(_) | Semantic::Layout(_) => {
                unreachable!("plain leaves lower as fact terminals")
            }
        }
    }

    /// A leaf that binds or exports what it reads at the node (`arguments`, `capture`, `calls`'
    /// `chain`): a fact terminal keyed by the bindings it unifies with, binding `binds`.
    fn binder(
        &mut self,
        matcher: &AstMatcher,
        binds: Vec<(String, SlotKind)>,
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        let reads = binds.iter().map(|(name, _)| name.clone()).collect();
        let leaf = matcher.clone();
        let columns = binds.clone();
        self.fact(
            move || {
                Box::new(Binder {
                    leaf,
                    binds: columns,
                })
            },
            None,
            scope,
            &reads,
            true,
            &binds,
            bound,
        )
    }

    /// An internal slot, unique to the statement, of a leaf's own (`prefix` names its role).
    fn internal(&mut self, prefix: &str, kind: SlotKind) -> SlotId {
        let name = format!("\0{prefix}{}", self.slots.len());
        self.slot(INTERNAL, &name, kind)
    }

    /// A fact terminal of the program evaluating `verdicts`, keyed by `reads`.
    fn terminal(&mut self, verdicts: Box<dyn Verdicts>, reads: &[SlotId]) -> usize {
        let reads = reads
            .iter()
            .map(|&slot| (self.slots[slot].name.clone(), self.slots[slot].kind))
            .collect();
        let term = self.program.terminals.len();
        self.program
            .terminals
            .push(Terminal::Fact(FactTerm { verdicts, reads }));
        term
    }

    /// A walk over the related rows terminal `verdicts` returns for each row's key: `(file, node,
    /// bindings of reads)`, writing each row's values into `sets`.
    fn related(
        &mut self,
        verdicts: Box<dyn Verdicts>,
        reads: Vec<SlotId>,
        sets: Vec<SlotId>,
    ) -> Walk {
        let term = self.terminal(verdicts, &reads);
        self.sources.push(Source {
            term,
            reads,
            at_node: true,
            sets,
        });
        Walk::Related(self.sources.len() - 1)
    }

    /// `rule` over `walk`'s candidates, as a relation searches them: its first match binds;
    /// three-valued, an unknown candidate ends the search where `strict`.
    fn search_over(&self, walk: Walk, sub: Op, strict: bool) -> Op {
        let binds = sub.binds();
        if self.three {
            Op::Search {
                walk,
                sub: Box::new(sub),
                binds,
                strict,
            }
        } else {
            Op::Relation {
                walk,
                sub: Box::new(sub),
                binds,
            }
        }
    }

    /// The sub-rules of a binding's nodes, held in `slots` (`binding::match_binding_nodes`):
    /// each at its node in order, sharing the environment.
    fn binding_nodes(
        &mut self,
        rules: [&Option<AstMatcher>; 3],
        slots: [SlotId; 3],
        scope: usize,
        items: &mut Vec<Op>,
        current: &mut Bound,
    ) -> Result<(), Unsupported> {
        for (rule, slot) in rules.into_iter().zip(slots) {
            let Some(rule) = rule else {
                continue;
            };
            let (sub, out) = self.lower(rule, scope, current)?;
            *current = out;
            items.push(self.search_over(Walk::Slot(slot), sub, may_bind(rule)));
        }
        Ok(())
    }

    /// `rule` over the occurrences of a binding's variable (`binding::match_occurrence`): the
    /// first where it matches binds, strict where it may bind.
    fn occurrence(
        &mut self,
        rule: &AstMatcher,
        walk: Walk,
        scope: usize,
        items: &mut Vec<Op>,
        current: &mut Bound,
    ) -> Result<(), Unsupported> {
        let (sub, out) = self.lower(rule, scope, current)?;
        self.leaks(&sub)?;
        *current = out;
        let sub = self.excluding(scope, sub);
        items.push(self.search_over(walk, sub, may_bind(rule)));
        Ok(())
    }

    /// `resolves.binding` (`BindingTest::match_node`): the binding the node resolves to, then
    /// its site, value and annotation sub-rules, then its `occurrence` rule.
    fn binding(
        &mut self,
        test: &BindingTest,
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        let mut reads = BTreeSet::new();
        filter_reads(&test.filter, &mut reads);
        let nodes = [
            self.internal("site", SlotKind::Node),
            self.internal("value", SlotKind::Node),
            self.internal("annotation", SlotKind::Node),
        ];
        let sites = test.clone();
        let (resolved, mut current) = self.fact_slots(
            move || Box::new(BindingSites(sites)),
            None,
            scope,
            &reads,
            true,
            &nodes,
            bound,
        )?;
        let mut items = vec![resolved];
        self.binding_nodes(
            [&test.site, &test.value, &test.annotation],
            nodes,
            scope,
            &mut items,
            &mut current,
        )?;
        if let Some(rule) = &test.occurrence {
            let (read_slots, _) = self.read_slots(scope, &reads);
            let walk = self.related(
                Box::new(Occurrences::Binding(test.clone())),
                read_slots,
                Vec::new(),
            );
            self.occurrence(rule, walk, scope, &mut items, &mut current)?;
        }
        Ok((Op::Seq(items), current))
    }

    /// `binds` (`Binds::match_node`): the bindings the name node names, in order; the first
    /// whose filters hold and whose sub-rules match binds, and an unknown one leaves the answer
    /// open unless a later one matches.
    fn binds(
        &mut self,
        test: &Binds,
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        let mut reads = BTreeSet::new();
        filter_reads(&test.filter, &mut reads);
        if let Some(redefines) = &test.redefines {
            filter_reads(&redefines.filter, &mut reads);
        }
        let (read_slots, _) = self.read_slots(scope, &reads);
        let ordinal = self.internal("ordinal", SlotKind::Count);
        let pre = self.internal("pre", SlotKind::Count);
        let nodes = [
            self.internal("site", SlotKind::Node),
            self.internal("value", SlotKind::Node),
            self.internal("annotation", SlotKind::Node),
        ];
        let shadow = self.internal("shadow", SlotKind::Count);
        let previous = [
            self.internal("site", SlotKind::Node),
            self.internal("value", SlotKind::Node),
            self.internal("annotation", SlotKind::Node),
        ];
        let mut sets = vec![ordinal, pre];
        sets.extend(nodes);
        sets.push(shadow);
        sets.extend(previous);
        let candidates = self.related(
            Box::new(BindsCandidates(test.clone())),
            read_slots.clone(),
            sets,
        );
        let mut current = bound.clone();
        current.extend([ordinal, pre, shadow]);
        current.extend(nodes);
        current.extend(previous);
        let mut items = vec![Op::Gate(pre)];
        self.binding_nodes(
            [&test.site, &test.value, &test.annotation],
            nodes,
            scope,
            &mut items,
            &mut current,
        )?;
        let ordinal_name = self.slots[ordinal].name.clone();
        if let Some(rule) = &test.occurrence {
            let walk = self.related(
                Box::new(Occurrences::Binds(test.clone(), ordinal_name.clone())),
                vec![ordinal],
                Vec::new(),
            );
            self.occurrence(rule, walk, scope, &mut items, &mut current)?;
        }
        if let Some(redefines) = test.redefines.as_deref() {
            let rules = [&redefines.site, &redefines.value, &redefines.annotation];
            if test.redefines_shadowed() {
                // Python: the shadowed binding's nodes, after the binding's own sub-rules.
                items.push(Op::Gate(shadow));
                self.binding_nodes(rules, previous, scope, &mut items, &mut current)?;
            } else {
                let other_pre = self.internal("pre", SlotKind::Count);
                let other_nodes = [
                    self.internal("site", SlotKind::Node),
                    self.internal("value", SlotKind::Node),
                    self.internal("annotation", SlotKind::Node),
                ];
                let mut other_reads = read_slots;
                other_reads.push(ordinal);
                let mut sets = vec![other_pre];
                sets.extend(other_nodes);
                let walk = self.related(
                    Box::new(BindsOthers(test.clone(), ordinal_name)),
                    other_reads,
                    sets,
                );
                let mut inner = current.clone();
                inner.insert(other_pre);
                inner.extend(other_nodes);
                let mut steps = vec![Op::Gate(other_pre)];
                self.binding_nodes(rules, other_nodes, scope, &mut steps, &mut inner)?;
                let sub = Op::Seq(steps);
                if redefines.every {
                    // Each other binding in turn, on one environment: a set operation over them
                    // is exact only where their sub-rules bind nothing, and where no candidate
                    // the enumeration passes over counts as passing.
                    if sub.binds() {
                        return unsupported("binds.redefines.every with binding sub-rules");
                    }
                    if self.exclude.is_some() {
                        return unsupported("binds.redefines.every under violation.each");
                    }
                    items.push(Op::Every {
                        walk,
                        sub: Box::new(sub),
                    });
                } else {
                    current = inner;
                    let sub = self.excluding(scope, sub);
                    items.push(self.search_over(walk, sub, false));
                }
            }
        }
        Ok((self.search_over(candidates, Op::Seq(items), false), current))
    }

    /// `formatArgs` (`FormatArgs::match_node`): its verdict before its sub-rules, then `unused`
    /// over the host's unused arguments and `missing` over the template's unfilled fields, each
    /// the first known match binding (a match where the candidate is not known to be one is
    /// unknown), a miss unknown where more candidates may be; false where either is false.
    fn format_args(
        &mut self,
        test: &FormatArgs,
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        let reads: BTreeSet<String> = [test.string.clone(), test.args.clone()].into();
        let tree = self.internal("tree", SlotKind::Count);
        let verdict = test.clone();
        let (prepared, mut current) = self.fact_slots(
            move || Box::new(FormatSides(verdict, FormatPart::Verdict)),
            None,
            scope,
            &reads,
            false,
            &[tree],
            bound,
        )?;
        let (read_slots, _) = self.read_slots(scope, &reads);
        let mut terms = Vec::new();
        if let Some(rule) = &test.unused {
            let pre = self.internal("pre", SlotKind::Count);
            let cap = self.internal("cap", SlotKind::Count);
            let walk = self.related(
                Box::new(FormatSides(test.clone(), FormatPart::Unused)),
                read_slots.clone(),
                vec![pre, cap],
            );
            let (sub, out) = self.lower(rule, scope, &current)?;
            self.leaks(&sub)?;
            current = out;
            let sub = self.excluding(scope, sub);
            terms.push(self.search_over(
                walk,
                Op::Seq(vec![Op::Gate(pre), sub, Op::Gate(cap)]),
                false,
            ));
        }
        if let Some(rule) = &test.missing {
            let pre = self.internal("pre", SlotKind::Count);
            let cap = self.internal("cap", SlotKind::Count);
            let walk = self.related(
                Box::new(FormatSides(test.clone(), FormatPart::Missing)),
                read_slots,
                vec![pre, cap],
            );
            let (sub, inner) = self.in_tree(rule, scope, None, tree)?;
            let sub = Op::Seq(vec![Op::Gate(pre), sub, Op::Gate(cap)]);
            let search = self.search_over(walk, sub, false);
            let exports = self.exports(&test.exports, inner, scope, &mut current)?;
            terms.push(Op::Seq(vec![search, Op::Exports { tree, exports }]));
        }
        let sides = if self.three {
            Op::AllOpen(
                terms
                    .into_iter()
                    .map(|op| AllTerm {
                        op,
                        mentions: Vec::new(),
                        binds: Vec::new(),
                        withholds: false,
                        checked: false,
                    })
                    .collect(),
            )
        } else {
            Op::All(terms)
        };
        Ok((Op::Seq(vec![prepared, sides]), current))
    }

    /// `rule`, a parsed tree's, lowered in a scope of its own (`ParseTest::match_tree` matches it
    /// in a fresh environment): no stage stands for a rule over its nodes. `host` is where a
    /// `host` rule in it finds its host node. Returns the rule and the tree's scope.
    fn in_tree(
        &mut self,
        rule: &AstMatcher,
        host_scope: usize,
        host: Option<(SlotId, SlotId)>,
        tree: SlotId,
    ) -> Result<(Op, usize), Unsupported> {
        // `violation.each` reads a tree's bindings by their host ranges; a tree whose rule binds
        // the enumerated capture would exclude candidates by them.
        if let Some(name) = &self.exclude
            && CaptureAnalysis::default().binds(rule).contains(name)
        {
            return unsupported("a parsed tree's capture enumerated by violation.each");
        }
        self.scopes += 1;
        let scope = self.scopes;
        let host = match host {
            Some(host) => host,
            None => (
                self.internal("hostfile", SlotKind::Count),
                self.internal("hostnode", SlotKind::Count),
            ),
        };
        let context = TreeContext {
            host,
            tree,
            host_scope,
            scope,
        };
        let saved = self.tree.replace(context);
        let lowered =
            self.with_exclude(None, |lowerer| lowerer.lower(rule, scope, &BTreeSet::new()));
        self.tree = saved;
        Ok((lowered?.0, scope))
    }

    /// The exports of a tree's captures `names` (scope `inner`) into host text slots of
    /// `scope`, read through the `Exported` terminal.
    fn exports(
        &mut self,
        names: &[String],
        inner: usize,
        scope: usize,
        current: &mut Bound,
    ) -> Result<Vec<Export>, Unsupported> {
        let mut exports = Vec::new();
        for name in names {
            if self.existing(inner, name, SlotKind::Text).is_some() {
                return unsupported("parse exporting a nested parse's capture");
            }
            let Some(from) = self.existing(inner, name, SlotKind::Node) else {
                continue;
            };
            let to = self.slot(scope, name, SlotKind::Text);
            current.insert(to);
            let term = self.exported_term();
            exports.push(Export { from, to, term });
        }
        Ok(exports)
    }

    /// The program's `Exported` terminal, one for every export.
    fn exported_term(&mut self) -> usize {
        let key = (usize::MAX, Vec::new());
        if let Some(&term) = self.program.leaves.get(&key) {
            return term;
        }
        let term = self.terminal(Box::new(Exported), &[]);
        self.program.leaves.insert(key, term);
        term
    }

    /// `parse` (`ParseTest::match_node`): the node's tree, its rule at the tree's root in a
    /// fresh environment, and the exports of its captures as host text.
    fn parse(
        &mut self,
        test: &ParseTest,
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        if self.tree.is_some() {
            return unsupported("parse inside parse");
        }
        let parsed = test.clone();
        let term = self.terminal(Box::new(Parsed(parsed)), &[]);
        let tree = self.internal("tree", SlotKind::Count);
        let host = (
            self.internal("hostfile", SlotKind::Count),
            self.internal("hostnode", SlotKind::Count),
        );
        let first = self.slots.len();
        let (body, inner) = self.in_tree(&test.rule, scope, Some(host), tree)?;
        let inner_slots: Vec<SlotId> = (first..self.slots.len())
            .filter(|&slot| self.slots[slot].scope == inner)
            .collect();
        let mut current = bound.clone();
        let exports = self.exports(&test.exports, inner, scope, &mut current)?;
        Ok((
            Op::Parse {
                term,
                inner: inner_slots,
                host,
                tree,
                body: Box::new(body),
                exports,
            },
            current,
        ))
    }

    /// `host` (`HostTest::match_node`): `rule` at the host node in the host's environment as
    /// the `parse` began, with the tree's captures bound so far exported into it; it binds
    /// nothing.
    fn host(&mut self, rule: &AstMatcher, bound: &Bound) -> Result<(Op, Bound), Unsupported> {
        let Some(context) = self.tree else {
            return unsupported("host outside parse");
        };
        // Every capture of the tree bound so far is exported (`parse::host_exports`).
        let captures: Vec<(SlotId, String)> = (0..self.slots.len())
            .filter(|&slot| {
                self.slots[slot].scope == context.scope && self.slots[slot].kind == SlotKind::Node
            })
            .map(|slot| (slot, self.slots[slot].name.clone()))
            .collect();
        let term = self.exported_term();
        let mut host_bound: Bound = bound
            .iter()
            .copied()
            .filter(|&slot| self.slots[slot].scope != context.scope)
            .collect();
        let mut exports = Vec::new();
        for (from, name) in captures {
            let to = self.slot(context.host_scope, &name, SlotKind::Text);
            host_bound.insert(to);
            exports.push(Export { from, to, term });
        }
        self.tree = None;
        let lowered = self.with_exclude(None, |lowerer| {
            lowerer.lower(rule, context.host_scope, &host_bound)
        });
        self.tree = Some(context);
        let (rule, _) = lowered?;
        Ok((
            Op::Host {
                host: context.host,
                tree: context.tree,
                rule: Box::new(rule),
                exports,
            },
            bound.clone(),
        ))
    }
}
