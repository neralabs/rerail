//! Format analysis is atomic; ordered candidate sub-rules stay in the SQL plan.
use super::super::{CaptureKind, Op, Plan, Slots, effects::Effects};
use super::Lower;
use crate::code_rule_dsl::{
    matcher::{FormatArgs, Style},
    sql::{
        plan::SlotKind,
        terminals::{FactTerm, Terminal},
    },
};
impl Lower {
    pub(super) fn format_args(&mut self, test: &FormatArgs) -> Result<Plan, String> {
        if !test.exports.is_empty() {
            return Err(
                "format outcomes: missing-field text/span exports need a typed output contract"
                    .into(),
            );
        }
        let string = self.slot(&test.string, CaptureKind::Node)?;
        let arg_name = format!("{}{}", self.namespace, test.args);
        let args_kind = self
            .slots
            .iter()
            .find(|slot| slot.name == arg_name)
            .map(|slot| slot.kind)
            .unwrap_or(if matches!(test.style, Style::Brace | Style::Rust) {
                CaptureKind::List
            } else {
                CaptureKind::Node
            });
        if args_kind == CaptureKind::Count {
            return Err("format outcomes: arguments require node or list captures".into());
        }
        let args = self.slot(&test.args, args_kind)?;
        let unused = test
            .unused
            .as_ref()
            .map(|rule| self.plan(rule).map(Box::new))
            .transpose()?;
        let missing = test
            .missing
            .as_ref()
            .map(|rule| self.plan(rule).map(Box::new))
            .transpose()?;
        if missing
            .as_ref()
            .is_some_and(|child| !child.writes.is_empty() || !child.mentions.is_empty())
        {
            return Err("format outcomes: missing-field capture inputs/exports need a fresh typed document contract".into());
        }
        let mut mentions = Slots::from([string, args]);
        let mut writes = Slots::new();
        let mut node_writes = Slots::new();
        if let Some(child) = &unused {
            mentions.extend(&child.mentions);
            writes.extend(&child.writes);
            node_writes.extend(&child.node_writes);
        }
        let term = self.pure.program.terminals.len();
        self.pure.program.terminals.push(Terminal::Fact(FactTerm {
            reads: vec![
                (test.string.clone(), SlotKind::Node),
                (
                    test.args.clone(),
                    if args_kind == CaptureKind::List {
                        SlotKind::List
                    } else {
                        SlotKind::Node
                    },
                ),
            ],
            verdicts: Box::new(super::super::super::format_facts::FormatAtomic {
                test: test.clone(),
                owner: self.scope_owner,
            }),
        }));
        let op = Op::FormatArgs {
            term,
            inputs: vec![string, args],
            unused,
            missing,
        };
        let mut effects = Effects::of(&op);
        effects.cache_fills |= test.reading.as_ref().is_some_and(|reading| reading.follow);
        Ok(Plan {
            op,
            effects,
            mentions,
            writes,
            node_writes,
            walk_filter: None,
        })
    }
}
