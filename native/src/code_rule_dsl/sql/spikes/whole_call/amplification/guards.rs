//! Move total, capture-free guards across total pattern matching only. Facts, recursive calls,
//! and unknown-producing operations are barriers. Alternatives and witness order never change.
use crate::code_rule_dsl::sql::plan::{OfRule, Op, Statement};

fn guard(op: &Op) -> bool {
    match op {
        Op::Kinds(_) | Op::Regex(_) | Op::Never => true,
        Op::Not(sub) => guard(sub),
        Op::All(items) | Op::Any(items) => items.iter().all(guard),
        _ => false,
    }
}
fn total(op: &Op) -> bool {
    guard(op) || matches!(op, Op::Pattern { .. })
}
fn visit(op: &mut Op) {
    match op {
        Op::All(items) | Op::Any(items) | Op::Seq(items) => {
            for item in items.iter_mut() {
                visit(item);
            }
        }
        Op::AllOpen(items) => {
            for t in items {
                visit(&mut t.op);
            }
        }
        Op::AnyOpen(items) => {
            for (t, _) in items {
                visit(t);
            }
        }
        Op::Not(sub)
        | Op::Unknowable { op: sub, .. }
        | Op::Relation { sub, .. }
        | Op::Search { sub, .. }
        | Op::Count { sub, .. }
        | Op::Every { sub, .. }
        | Op::Call { body: sub, .. }
        | Op::Follow { sub, .. }
        | Op::Parse { body: sub, .. }
        | Op::Host { rule: sub, .. } => visit(sub),
        Op::Constrained { rule, constraints } => {
            visit(rule);
            for (_, c) in constraints {
                visit(c);
            }
        }
        Op::NthChild {
            of_rule: Some(OfRule::Frame(rule)),
            ..
        } => visit(rule),
        Op::NthCommit { nth, commit } => {
            visit(nth);
            visit(commit);
        }
        _ => {}
    }
    match op {
        Op::All(items) | Op::Seq(items) => {
            for group in items.split_mut(|t| !total(t)) {
                group.sort_by_key(|t| !guard(t));
            }
        }
        Op::AllOpen(items) => {
            for group in items.split_mut(|t| !total(&t.op)) {
                // These guards mention and bind no captures, so they do not change withholding.
                group.sort_by_key(|t| !guard(&t.op));
            }
        }
        _ => {}
    }
}
pub(in super::super) fn apply(statement: &mut Statement) {
    visit(&mut statement.op);
    for unit in &mut statement.units {
        visit(&mut unit.body);
    }
}
