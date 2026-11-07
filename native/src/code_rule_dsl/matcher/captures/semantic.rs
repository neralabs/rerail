use super::*;
use crate::code_rule_dsl::matcher::{BindingCount, Semantic};

/// What a semantic term binds into the environment it runs in.
pub(super) fn collect_binds(
    semantic: &Semantic,
    summaries: &mut UtilitySummary<UtilKey, Names>,
    names: &mut Names,
) {
    match semantic {
        Semantic::Binding(test) => {
            for rule in test.sub_rules() {
                super::collect_binds(rule, summaries, names);
            }
        }
        Semantic::Binds(test) => {
            for rule in test.sub_rules() {
                super::collect_binds(rule, summaries, names);
            }
        }
        Semantic::Arguments(arguments) => {
            names.extend(arguments.bind.iter().map(|(name, _)| name.clone()));
        }
        // A parsed tree's rule runs in an environment of its own; its captures come back as exports
        // (read through the rule: a utility's may not have been compiled when `exports` was).
        Semantic::Parse(test) => parse_exported(&test.rule, names),
        Semantic::Embedded(test) => {
            for rule in test.rules() {
                parse_exported(rule, names);
            }
        }
        Semantic::Capture(name) => {
            names.insert(name.clone());
        }
        Semantic::FormatArgs(test) => {
            if let Some(unused) = &test.unused {
                super::collect_binds(unused, summaries, names);
            }
            names.extend(test.exports.iter().cloned());
        }
        // A reaching call binds its chain, for messages.
        Semantic::Calls(test) => names.extend(test.chain.iter().cloned()),
        Semantic::Resolves(_)
        | Semantic::Value(_)
        | Semantic::Layout(_)
        | Semantic::FilePath(_)
        | Semantic::Host(_) => {}
    }
}

/// What a semantic term reads, before less what it binds itself (`binds_and_reads` removes that).
pub(super) fn reads(
    semantic: &Semantic,
    binds: &mut UtilitySummary<UtilKey, Names>,
    reads: &mut UtilitySummary<UtilKey, Names>,
) -> Names {
    let mut rule_reads = |rule: &AstMatcher, names: &mut Names| {
        names.extend(super::binds_and_reads(rule, binds, reads).1);
    };
    match semantic {
        // A gap to a capture reads it.
        Semantic::Layout(layout) => layout.reads().map(str::to_owned).collect(),
        Semantic::FormatArgs(test) => {
            let mut names = Names::from([test.string.clone(), test.args.clone()]);
            if let Some(unused) = &test.unused {
                rule_reads(unused, &mut names);
            }
            names
        }
        Semantic::Binding(test) => {
            let mut names = within(test.filter.bindings.as_ref());
            for rule in test.sub_rules() {
                rule_reads(rule, &mut names);
            }
            names
        }
        Semantic::Binds(test) => {
            let mut names = within(test.filter.bindings.as_ref());
            if let Some(redefines) = &test.redefines {
                names.extend(within(redefines.filter.bindings.as_ref()));
            }
            for rule in test.sub_rules() {
                rule_reads(rule, &mut names);
            }
            names
        }
        Semantic::Resolves(_)
        | Semantic::Calls(_)
        | Semantic::Value(_)
        | Semantic::Arguments(_)
        | Semantic::Parse(_)
        | Semantic::Host(_)
        | Semantic::Embedded(_)
        | Semantic::Capture(_)
        | Semantic::FilePath(_) => Names::new(),
    }
}

/// Every capture a semantic term's verdict may depend on or that it may bind: what it binds and
/// reads, and what its sub-rules mention.
pub(super) fn collect_mentions(
    semantic: &Semantic,
    summaries: &mut UtilitySummary<UtilKey, Names>,
    names: &mut Names,
) {
    match semantic {
        Semantic::Binding(test) => {
            names.extend(within(test.filter.bindings.as_ref()));
            for rule in test.sub_rules() {
                super::collect_mentions(rule, summaries, names);
            }
        }
        Semantic::Binds(test) => {
            names.extend(within(test.filter.bindings.as_ref()));
            if let Some(redefines) = &test.redefines {
                names.extend(within(redefines.filter.bindings.as_ref()));
            }
            for rule in test.sub_rules() {
                super::collect_mentions(rule, summaries, names);
            }
        }
        Semantic::FormatArgs(test) => {
            names.extend([test.string.clone(), test.args.clone()]);
            names.extend(test.exports.iter().cloned());
            if let Some(unused) = &test.unused {
                super::collect_mentions(unused, summaries, names);
            }
        }
        Semantic::Arguments(arguments) => {
            names.extend(arguments.bind.iter().map(|(name, _)| name.clone()));
        }
        Semantic::Parse(test) => parse_exported(&test.rule, names),
        Semantic::Embedded(test) => {
            for rule in test.rules() {
                parse_exported(rule, names);
            }
        }
        Semantic::Capture(name) => {
            names.insert(name.clone());
        }
        Semantic::Calls(test) => names.extend(test.chain.iter().cloned()),
        Semantic::Layout(layout) => names.extend(layout.reads().map(str::to_owned)),
        Semantic::Resolves(_) | Semantic::Value(_) | Semantic::FilePath(_) | Semantic::Host(_) => {}
    }
}

/// The metavariables a semantic term defines, by the names its environment sees.
pub(super) fn collect_in(
    semantic: &Semantic,
    variables: &mut BTreeMap<String, Capture>,
    summaries: &mut UtilitySummary<usize, BTreeMap<String, Capture>>,
    framed: bool,
) {
    match semantic {
        Semantic::Resolves(_)
        | Semantic::Layout(_)
        | Semantic::FilePath(_)
        | Semantic::Host(_)
        | Semantic::Value(_) => {}
        Semantic::Calls(test) => {
            if let Some(name) = &test.chain {
                merge_capture(variables, name, Capture::Single);
            }
        }
        Semantic::Binding(test) => {
            for rule in test.sub_rules() {
                super::collect_in(rule, variables, summaries, framed);
            }
        }
        Semantic::Binds(test) => {
            for rule in test.sub_rules() {
                super::collect_in(rule, variables, summaries, framed);
            }
        }
        Semantic::Arguments(arguments) => {
            for (name, _) in &arguments.bind {
                merge_capture(variables, name, Capture::Single);
            }
        }
        Semantic::Embedded(test) => {
            for name in &test.exports {
                merge_capture(variables, name, Capture::Single);
            }
        }
        Semantic::Parse(test) => {
            for name in &test.exports {
                merge_capture(variables, name, Capture::Single);
            }
        }
        Semantic::Capture(name) => merge_capture(variables, name, Capture::Single),
        Semantic::FormatArgs(test) => {
            if let Some(unused) = &test.unused {
                super::collect_in(unused, variables, summaries, framed);
            }
            for name in &test.exports {
                merge_capture(variables, name, Capture::Single);
            }
        }
    }
}

fn within(count: Option<&BindingCount>) -> Names {
    count
        .and_then(|count| count.within.clone())
        .into_iter()
        .collect()
}

/// The captures a parsed tree's rule may bind: not under `not`, `count` or a stop rule, which bind
/// in environments of their own. A nested parse's exports pass through, and so do those of the
/// utilities the rule calls, at any depth: a utility compiled before the table was filled reads its
/// draft (`UtilTable::draft`).
pub(in crate::code_rule_dsl) fn parse_exported(matcher: &AstMatcher, names: &mut Names) {
    exported_in(matcher, &mut UtilitySummary::new(), names);
}

fn exported_in(
    matcher: &AstMatcher,
    summaries: &mut UtilitySummary<usize, Names>,
    names: &mut Names,
) {
    match matcher {
        AstMatcher::Semantic(Semantic::Capture(name)) => {
            names.insert(name.clone());
        }
        AstMatcher::Pattern(pattern) => {
            names.extend(pattern.defined_vars().into_iter().map(str::to_owned));
        }
        AstMatcher::Semantic(Semantic::Parse(test)) => exported_in(&test.rule, summaries, names),
        AstMatcher::Semantic(Semantic::Embedded(test)) => {
            for rule in test.rules() {
                exported_in(rule, summaries, names);
            }
        }
        AstMatcher::Util(util) => names.extend(utility_exported(util, summaries)),
        // A call returns the parameters its utility exports, under the caller's names.
        AstMatcher::Call(call) => {
            let inner = utility_exported(&call.util, summaries);
            names.extend(call.outward(inner).into_iter().map(str::to_owned));
        }
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for item in items {
                exported_in(item, summaries, names);
            }
        }
        AstMatcher::Inside(item, _)
        | AstMatcher::Has(item, _)
        | AstMatcher::Precedes(item, _)
        | AstMatcher::Follows(item, _) => exported_in(item, summaries, names),
        _ => {}
    }
}

/// What a utility's rule exports, in the utility's own names.
fn utility_exported(util: &UtilRef, summaries: &mut UtilitySummary<usize, Names>) -> Names {
    summaries.summarize(util.index, Names::new(), |summaries| {
        let mut names = Names::new();
        util.with_target(|rule| exported_in(rule, summaries, &mut names));
        names
    })
}
