use crate::code_rule_dsl::tree::Tree;
use super::{Arguments, Passed, every, unparenthesized};
use ast_grep_core::{Doc, Node};

/// Rust's passed arguments of a call (`ArgumentRules`).
pub trait RustArguments {
    /// Returns the positional arguments passed by a Rust call expression.
    fn rust<'r, D: Doc>(&self, call: &Node<'r, D>) -> Vec<Passed<'r, D>>;
}

impl RustArguments for Arguments {
    /// Returns the positional arguments passed by a Rust call expression.
    fn rust<'r, D: Doc>(&self, call: &Node<'r, D>) -> Vec<Passed<'r, D>> {
        let unknown = || every(self.slots.len(), || Passed::Unknown);
        let Some(arguments) = call.field("arguments") else {
            return unknown();
        };
        if arguments
            .dfs()
            .any(|node| node.is_error() || node.is_missing())
        {
            return unknown();
        }

        let mut passed = every(self.slots.len(), || Passed::Absent);
        let mut slot = 0;
        let mut attributed = false;
        for argument in arguments.named_code().collect::<Vec<_>>() {
            if argument.kind() == "attribute_item" {
                attributed = true;
                continue;
            }
            if slot >= passed.len() {
                break;
            }
            if attributed {
                for item in passed.iter_mut().skip(slot) {
                    *item = Passed::Unknown;
                }
                break;
            }
            passed[slot] = Passed::Value(unparenthesized(argument));
            slot += 1;
        }
        if attributed {
            for item in passed.iter_mut().skip(slot) {
                *item = Passed::Unknown;
            }
        }

        passed
    }
}
