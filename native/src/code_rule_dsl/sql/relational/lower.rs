use super::ir::{Edge, Program, Relation, Set};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    matcher::{AstMatcher, GrammarMatcher, StopBy},
    sql::terminals::{PatternTerm, Terminal},
};
use ast_grep_core::matcher::Matcher;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Lowerer {
    pub program: Program,
    utilities: HashMap<(usize, usize), usize>,
    regexes: HashMap<String, usize>,
    dialect: Option<GrammarDialect>,
}

pub(super) fn compile(grammar: &GrammarMatcher) -> Result<Program, String> {
    let mut lower = Lowerer {
        dialect: Some(grammar.language),
        ..Lowerer::default()
    };
    lower.program.relations.push(Relation {
        source: "clause".into(),
        expression: Set::Empty,
        domain: None,
        keys: None,
    });
    lower.program.relations[0].expression = if grammar.applies {
        lower.set(&grammar.matcher)?
    } else {
        Set::Empty
    };
    Ok(lower.program)
}

impl Lowerer {
    pub(super) fn new(dialect: GrammarDialect) -> Self {
        Self {
            dialect: Some(dialect),
            ..Self::default()
        }
    }

    pub(super) fn relation(&mut self, expression: Set, source: &str) -> usize {
        fn singleton(set: Set) -> Set {
            match set {
                Set::Intersect(mut items) => {
                    items.retain(|s| *s != Set::AllNodes);
                    if items.len() == 1 {
                        singleton(items.remove(0))
                    } else {
                        Set::Intersect(items)
                    }
                }
                Set::Union(mut items) if items.len() == 1 => singleton(items.remove(0)),
                other => other,
            }
        }
        let expression = singleton(expression);
        if let Set::Reference(id) = expression {
            return id;
        }
        if expression != Set::Empty
            && let Some((id, _)) =
                self.program
                    .relations
                    .iter()
                    .enumerate()
                    .skip(1)
                    .find(|(_, relation)| {
                        relation.domain.is_none()
                            && relation.keys.is_none()
                            && relation.expression == expression
                    })
        {
            return id;
        }
        let id = self.program.relations.len();
        self.program.relations.push(Relation {
            source: source.into(),
            expression,
            domain: None,
            keys: None,
        });
        id
    }

    pub(super) fn set(&mut self, rule: &AstMatcher) -> Result<Set, String> {
        Ok(match rule {
            AstMatcher::Pattern(pattern) => {
                let term = PatternTerm::new(pattern.clone(), self.dialect.unwrap());
                if !term.vars.is_empty() {
                    return Err(
                        "relational compiler: pattern captures require witness relations".into(),
                    );
                }
                let id = self.program.terminals.len();
                self.program.terminals.push(Terminal::Pattern(term));
                Set::Pattern(id)
            }
            AstMatcher::Kind(kind) => Set::Kinds(
                kind.potential_kinds()
                    .ok_or("relational compiler: missing kind identity")?
                    .iter()
                    .map(|k| k as u16)
                    .collect(),
            ),
            AstMatcher::Regex(regex) => {
                let id = *self
                    .regexes
                    .entry(regex.as_str().to_owned())
                    .or_insert_with(|| {
                        let id = self.program.terminals.len();
                        self.program.terminals.push(Terminal::Regex(regex.clone()));
                        id
                    });
                if regex.as_str() == "(?s)." {
                    Set::Nonempty
                } else {
                    Set::Regex(id)
                }
            }
            AstMatcher::Never => Set::Empty,
            AstMatcher::All(items) => {
                if items.is_empty() {
                    Set::AllNodes
                } else {
                    Set::Intersect(
                        items
                            .iter()
                            .map(|r| self.set(r))
                            .collect::<Result<_, _>>()?,
                    )
                }
            }
            AstMatcher::Any(items) => {
                let sets: Vec<_> = items
                    .iter()
                    .map(|r| self.set(r))
                    .collect::<Result<_, _>>()?;
                if sets.iter().all(|s| matches!(s, Set::Kinds(_))) {
                    let mut kinds: Vec<_> = sets
                        .into_iter()
                        .flat_map(|s| match s {
                            Set::Kinds(kinds) => kinds,
                            _ => unreachable!(),
                        })
                        .collect();
                    kinds.sort_unstable();
                    kinds.dedup();
                    if kinds.is_empty() {
                        Set::Empty
                    } else {
                        Set::Kinds(kinds)
                    }
                } else {
                    Set::Union(sets)
                }
            }
            AstMatcher::Not(rule) => Set::Complement(Box::new(self.set(rule)?)),
            AstMatcher::Inside(rule, StopBy::Neighbor) => {
                Set::Preimage(Edge::Parent, Box::new(self.set(rule)?))
            }
            AstMatcher::Has(rule, StopBy::Neighbor) => {
                Set::Preimage(Edge::Child, Box::new(self.set(rule)?))
            }
            AstMatcher::Follows(rule, StopBy::Neighbor) => {
                Set::Preimage(Edge::Previous, Box::new(self.set(rule)?))
            }
            AstMatcher::Precedes(rule, StopBy::Neighbor) => {
                Set::Preimage(Edge::Next, Box::new(self.set(rule)?))
            }
            AstMatcher::Inside(rule, StopBy::End) => {
                let related = self.set(rule)?;
                Set::Within(self.relation(related, "ancestor predicate"))
            }
            AstMatcher::Has(rule, by @ (StopBy::End | StopBy::Rule(_))) => {
                let seed = self.set(rule)?;
                let seeds = self.relation(seed, "descendant predicate");
                let stop = match by {
                    StopBy::Rule(rule) => {
                        let predicate = self.set(rule)?;
                        Some(self.relation(predicate, "traversal stop"))
                    }
                    _ => None,
                };
                Set::Reference(
                    self.relation(Set::Ancestors { seeds, stop }, "inverse descendant closure"),
                )
            }
            AstMatcher::Util(util) => {
                let key = (util.table.as_ptr() as usize, util.index);
                if let Some(&id) = self.utilities.get(&key) {
                    Set::Reference(id)
                } else {
                    let id = self.program.relations.len();
                    self.utilities.insert(key, id);
                    self.program.relations.push(Relation {
                        source: format!("utility[{}]", util.index),
                        expression: Set::Empty,
                        domain: None,
                        keys: None,
                    });
                    let expression = util
                        .with_target(|r| self.set(r))
                        .ok_or("relational compiler: utility definition is unavailable")??;
                    self.program.relations[id].expression = expression;
                    Set::Reference(id)
                }
            }
            _ => {
                return Err(
                    "relational compiler: operator outside the total neighbor-set subset".into(),
                );
            }
        })
    }
}
