use super::*;
use crate::code_rule_dsl::matcher::Semantic;

pub(super) fn compile_in(semantic: &Semantic, _inlining: &mut Inlining) -> AnchorPlan {
    match semantic {
        // Lines and bytes hold on any node.
        Semantic::Layout(_) | Semantic::Host(_) => AnchorPlan::unrestricted(),
        // Every value predicate except `constant: false` needs a known value, which only the
        // kinds the decoder reads can have. Following bindings, a name has a value too, and in
        // PHP a named or class constant.
        Semantic::Value(test) if test.constant != Some(false) => {
            let kinds = super::value_kinds(&test.reading);
            AnchorPlan {
                root_kinds: Some(kinds),
                condition: Condition::Always,
                exact: false,
            }
        }
        Semantic::Value(_) => AnchorPlan::unrestricted(),
        // Only a string literal parses.
        Semantic::Parse(test) => AnchorPlan {
            root_kinds: Some(test.host_kinds.as_ref().clone()),
            condition: Condition::Always,
            exact: false,
        },
        // A capture holds on every node (inside a parsed tree only); format arguments read the bindings.
        Semantic::Capture(_) | Semantic::FormatArgs(_) => AnchorPlan::unrestricted(),
        // Only a call has arguments.
        Semantic::Arguments(arguments) => AnchorPlan {
            root_kinds: Some(arguments.call_kinds.clone()),
            condition: Condition::Always,
            exact: false,
        },
        // Only a binding's name node binds.
        Semantic::Binds(test) => AnchorPlan {
            root_kinds: Some(test.name_kinds.clone()),
            condition: Condition::Always,
            exact: false,
        },
        // Only a reference, possibly inside parentheses, has a binding; `write` also anchors
        // on a binding's name node.
        Semantic::Binding(test) => AnchorPlan {
            root_kinds: Some(
                test.reference_kinds
                    .iter()
                    .chain(&test.paren_kinds)
                    .chain(&test.name_kinds)
                    .copied()
                    .collect(),
            ),
            condition: Condition::Always,
            exact: false,
        },
        // `calls` holds only at a call.
        Semantic::Calls(test) => AnchorPlan {
            root_kinds: Some(test.call_kinds.iter().copied().collect()),
            condition: Condition::Always,
            exact: false,
        },
        // Only a reference (or, for `import`, an attribute chain) can resolve, possibly inside
        // parentheses; a `global`-only predicate without `/* global */` comments also needs a
        // listed name. Whether it is shadowed is the matcher's to decide.
        // With language options a name must be listed even under `/* global */` comments;
        // `this: "global"` adds the `this` kind and its text. `seenBefore` holds on any node.
        Semantic::Resolves(resolves) if resolves.seen_before => AnchorPlan::unrestricted(),
        Semantic::Resolves(resolves) => AnchorPlan {
            root_kinds: Some(
                resolves
                    .reference_kinds
                    .iter()
                    .chain(&resolves.paren_kinds)
                    .chain(&resolves.this_kinds)
                    .chain(&resolves.dotted_kinds)
                    .chain(&resolves.string_kinds)
                    .copied()
                    .collect(),
            ),
            condition: if (resolves.directives && resolves.options.is_none())
                || resolves.name_less()
                || !resolves.imports.is_empty()
                || !resolves.imported.is_empty()
            {
                Condition::Always
            } else {
                let this = resolves.this_global.then_some("this".to_owned());
                let names = resolves.globals.iter().map(|name| regex::escape(name));
                let names = names.chain(this).collect::<Vec<_>>().join("|");
                let pattern = format!(r"^(?:{names}|\((?s:.*))$");
                match crate::code_rule_dsl::regex_cache::compile(&pattern) {
                    Ok(regex) => Condition::Text {
                        probe: TextProbe::compile(regex.as_str()),
                        regex,
                    },
                    Err(_) => Condition::Always,
                }
            },
            exact: false,
        },
    }
}
