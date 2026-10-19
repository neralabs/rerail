use super::*;
use crate::code_rule_dsl::matcher::Semantic;

pub(super) fn ast_captures<'a>(
    semantic: &'a Semantic,
    utils: &'a UtilTable,
    summaries: &mut CaptureSummaries<'a>,
    names: &mut BTreeSet<&'a str>,
) {
    match semantic {
        Semantic::Arguments(arguments) => {
            names.extend(arguments.bind.iter().map(|(name, _)| name.as_str()))
        }
        Semantic::Parse(test) => names.extend(test.exports.iter().map(String::as_str)),
        Semantic::Capture(name) => {
            names.insert(name.as_str());
        }
        Semantic::FormatArgs(test) => {
            if let Some(unused) = &test.unused {
                super::ast_captures(unused, utils, summaries, names);
            }
            names.extend(test.exports.iter().map(String::as_str));
        }
        Semantic::Calls(test) => names.extend(test.chain.as_deref()),
        Semantic::Binding(test) => {
            for rule in test.sub_rules() {
                super::ast_captures(rule, utils, summaries, names);
            }
        }
        Semantic::Binds(test) => {
            for rule in test.sub_rules() {
                super::ast_captures(rule, utils, summaries, names);
            }
        }
        Semantic::Resolves(_)
        | Semantic::Value(_)
        | Semantic::Layout(_)
        | Semantic::FilePath(_)
        | Semantic::Host(_) => {}
    }
}

pub(super) fn parse_exports<'a>(
    semantic: &'a Semantic,
    utils: &'a UtilTable,
    summaries: &mut ExportSummaries<'_, 'a>,
    exported: &mut BTreeSet<&'a str>,
    bound: &mut BTreeSet<&'a str>,
) {
    match semantic {
        Semantic::Parse(test) => exported.extend(test.exports.iter().map(String::as_str)),
        Semantic::FormatArgs(test) => {
            exported.extend(test.exports.iter().map(String::as_str));
            if let Some(unused) = &test.unused {
                super::parse_exports(unused, utils, summaries, exported, bound);
            }
        }
        Semantic::Binding(test) => {
            for rule in test.sub_rules() {
                super::parse_exports(rule, utils, summaries, exported, bound);
            }
        }
        Semantic::Binds(test) => {
            for rule in test.sub_rules() {
                super::parse_exports(rule, utils, summaries, exported, bound);
            }
        }
        Semantic::Arguments(arguments) => {
            bound.extend(arguments.bind.iter().map(|(name, _)| name.as_str()))
        }
        Semantic::Capture(name) => {
            bound.insert(name.as_str());
        }
        Semantic::Calls(test) => bound.extend(test.chain.as_deref()),
        Semantic::Resolves(_)
        | Semantic::Value(_)
        | Semantic::Layout(_)
        | Semantic::FilePath(_)
        | Semantic::Host(_) => {}
    }
}

pub(super) fn node_regexes<'a>(
    _semantic: &'a Semantic,
    _utils: &'a UtilTable,
    _summaries: &mut UtilitySummary<usize, UtilityRegexes<'a>>,
    _regexes: &mut Vec<&'a Regex>,
) {
}
