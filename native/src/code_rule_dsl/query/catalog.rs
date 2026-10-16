//! What compiled statements reference by index: terminals (node sets `rerail_matches` computes
//! once per batch), terms (the data a function needs to test one node), and the structural
//! kernel's operations (`structure`).

use super::recursion::Sql;
use super::structure::{self, Op, OpId};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{AstMatcher, UtilCall};
use crate::code_rule_dsl::transducer::sql_backend::{PatternTerminal, Terminal};
use ast_grep_core::Pattern;
use ast_grep_core::matcher::KindMatcher;
use regex::Regex;
use std::sync::Arc;

pub(super) enum Term {
    /// A pattern and the terminal that finds its matches (`pattern` is that terminal's).
    Pattern {
        pattern: Arc<PatternTerminal>,
        terminal: usize,
    },
    Kind(KindMatcher),
    Regex(Regex),
    /// A utility's own statement, or a term too deep to stay inline (`step` false: it spends no
    /// step of the budget, as the inline term would not). `memoize` is false for `violation.each`,
    /// whose exclusion scope makes the same input observably different between attempts.
    /// `env_free`: its verdict at a node is the same in every environment (`compile::env_free`),
    /// so its memo is shared by every caller at that node.
    Util {
        sql: Sql,
        memoize: bool,
        step: bool,
        env_free: bool,
    },
    /// A Rerail predicate without sub-rules (`resolves`, `value`, `compare`, `layout`,
    /// `arguments`, `capture`, `nthChild`), tested at the node the statement holds by its own
    /// code (`tq_pred`); `nthChild` with `ofRule` also reads its position here (`tq_nth`).
    Predicate(AstMatcher),
    /// A Rerail predicate with sub-rules (`resolves.binding`, `binds`, `formatArgs`, `parse`),
    /// whose facts `rerail_facts` reports and whose sub-rules the statement matches over them.
    Semantic(AstMatcher),
    /// A parameterized call: the utility's statement (`body`, a `Util` term), run in a frame.
    Call {
        body: usize,
        call: UtilCall,
    },
    /// A structural sub-rule the kernel answers (`tq_struct`).
    Structure(OpId),
}

/// The terminals and terms of a set of compiled statements.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct CatalogBuilder {
    terminals: Vec<Terminal>,
    terms: Vec<Term>,
    structure: structure::Builder,
    /// The term of each structural operation, which every sub-rule lowering to it shares.
    structure_terms: std::collections::HashMap<OpId, usize>,
}

impl CatalogBuilder {
    pub(super) fn terminal(&mut self, terminal: Terminal) -> usize {
        self.terminals.push(terminal);
        self.terminals.len() - 1
    }

    /// A pattern's terminal and the term that tests it at one node; returns the term.
    pub(super) fn pattern(&mut self, pattern: &Pattern, language: GrammarDialect) -> usize {
        let pattern = Arc::new(PatternTerminal::new(pattern.clone(), language));
        let terminal = self.terminal(Terminal::Pattern(Arc::clone(&pattern)));
        self.term(Term::Pattern { pattern, terminal })
    }

    pub(super) fn term(&mut self, term: Term) -> usize {
        self.terms.push(term);
        self.terms.len() - 1
    }

    /// The term answering `matcher`, a sub-rule that `structure::lowers`, in a three-valued clause
    /// (`three`) or not.
    pub(super) fn structure(&mut self, matcher: &AstMatcher, three: bool) -> usize {
        let op = self.structure.lower(matcher, three);
        if let Some(&term) = self.structure_terms.get(&op) {
            return term;
        }
        let term = self.term(Term::Structure(op));
        self.structure_terms.insert(op, term);
        term
    }

    pub(super) fn set_util_sql(&mut self, term: usize, compiled: String, memoize: bool) {
        if let Term::Util {
            sql,
            memoize: cached,
            ..
        } = &mut self.terms[term]
        {
            *sql = Sql::new(compiled);
            *cached = memoize;
        }
    }

    pub(super) fn finish(self) -> (Vec<Terminal>, Catalog) {
        (
            self.terminals,
            Catalog {
                terms: self.terms,
                ops: self.structure.finish(),
            },
        )
    }
}

pub(in crate::code_rule_dsl) struct Catalog {
    pub(super) terms: Vec<Term>,
    pub(super) ops: Vec<Op>,
}
