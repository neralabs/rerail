//! What a violation reports beyond its witness: the anchored span, `$NAME` message
//! interpolation, and a suggested fix. None of it changes which witnesses exist.
//!
//! Interpolation and fix templates follow ast-grep-config 0.45.3, where `message` and `fix`
//! are both `TemplateFix` templates over the match's metavariables: `$NAME` is a single capture's
//! text, `$$$NAME` the source from a multi-capture's first node to its last, and an unbound
//! name renders as nothing. Two deliberate differences: a name the query can never bind is a
//! compile error (upstream renders it empty), and a capture interpolated into a message is cut
//! to its first line and `MESSAGE_CAPTURE_CHARS` characters, since messages steer an agent.
//!
//! One extension beyond ast-grep: a named group (`(?<NAME>…)`) in an ast rule's `regex` that
//! tests the witness node itself (not under `not` or a relation) is a capture of that sub-span,
//! taken from the first such regex that matches the witness text. Metavariables win a name
//! clash. These sub-node captures anchor and interpolate but cannot appear in an ast `fix`,
//! whose `TemplateFix` reads only metavariables.
use super::json::invalid;
use super::matcher::{
    AstMatcher, Capture, CaptureAnalysis, GrammarMatcher, UtilTable, UtilitySummary, binding_range,
    collect_variables,
};
use super::model::{
    Anchor, ClauseKind, FIX_TEXT_PREFIX, FixPlan, Query, RegexSource, Template, ViolationSpec,
};
#[cfg(test)]
use super::test_support::{Walk, record_walk};
mod semantic;
mod suppression;
pub(crate) use super::lines::Span;
use crate::code_rule_dsl::language::adapter::{Adapter, with_adapter};
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::origin::InFile;
use ast_grep_core::matcher::NodeMatch;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::replacer::{Content, Replacer, TemplateFix};
use ast_grep_core::{Doc, Node};
use napi::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::borrow::Cow;
#[cfg(test)]
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;
pub(super) use suppression::{names_rerail_ignore, rerail_ignores, suppresses};

#[cfg(test)]
thread_local! {
    static NODE_GROUP_ENUMERATIONS: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(super) fn reset_node_group_enumerations() {
    NODE_GROUP_ENUMERATIONS.with(|count| count.set(0));
}

#[cfg(test)]
pub(super) fn node_group_enumerations() -> usize {
    NODE_GROUP_ENUMERATIONS.with(Cell::get)
}

/// Longest capture text a message interpolates before it is cut with an ellipsis.
pub(super) const MESSAGE_CAPTURE_CHARS: usize = 80;

/// A capture's text as a message shows it: the first line, at most `MESSAGE_CAPTURE_CHARS`
/// characters, with `…` marking anything cut.
pub(super) fn message_capture(text: &str) -> Cow<'_, str> {
    let first = text.split('\n').next().unwrap_or("");
    let cut_line = first.len() < text.len();
    if !cut_line && first.chars().count() <= MESSAGE_CAPTURE_CHARS {
        return Cow::Borrowed(text);
    }
    let kept = first
        .chars()
        .take(MESSAGE_CAPTURE_CHARS - 1)
        .collect::<String>();
    Cow::Owned(format!("{}…", kept.trim_end()))
}

/// The query whose witnesses become violations: `select_when` reports its `select` side.
fn emitting(query: &Query) -> &Query {
    match query {
        Query::SelectWhen { select, .. } => emitting(select),
        other => other,
    }
}

/// Every capture name a matcher can bind, following ast-grep-config 0.45.3. Negated sub-rules
/// and counts never contribute bindings (they probe a throwaway environment), so their metavariables are
/// excluded; a count's own value (`as`) is text only a message reads (`CaptureAnalysis::counts`).
/// `any` branches and relations are included, and a name only some of them bind may be
/// absent at runtime. A `matches` reference shares the caller's environment both ways
/// (`referent_rule.rs`), so a local utility's names are the caller's; `constraints` run against
/// the same environment after the rule (`rule_core.rs`), so their names count too. A relation's
/// stop rule matches in a fresh environment (`Node::matches` in `stop_by.rs`), so its names never
/// bind; upstream's `defined_vars` lists them for `inside`/`precedes`/`follows` anyway, and a
/// message there renders them empty. Each utility is summarized once per `summaries` (recursive
/// ones together with the utilities they reach); walks sharing `summaries` reuse each other's.
pub(super) fn ast_captures<'a>(
    matcher: &'a AstMatcher,
    utils: &'a UtilTable,
    summaries: &mut CaptureSummaries<'a>,
    names: &mut BTreeSet<&'a str>,
) {
    match matcher {
        AstMatcher::Pattern(pattern) => names.extend(pattern.defined_vars()),
        AstMatcher::Semantic(semantic) => semantic::ast_captures(semantic, utils, summaries, names),
        AstMatcher::Kind(_)
        | AstMatcher::Regex(_)
        | AstMatcher::Not(_)
        | AstMatcher::Compare(_)
        | AstMatcher::Count(_)
        | AstMatcher::Never => {}
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => {}
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for item in items {
                ast_captures(item, utils, summaries, names);
            }
        }
        AstMatcher::Inside(item, _stop)
        | AstMatcher::Has(item, _stop)
        | AstMatcher::Precedes(item, _stop)
        | AstMatcher::Follows(item, _stop) => ast_captures(item, utils, summaries, names),
        AstMatcher::Field(relation) => ast_captures(&relation.rule, utils, summaries, names),
        AstMatcher::NthChild(nth) => {
            if let Some(rule) = &nth.of_rule {
                ast_captures(rule, utils, summaries, names);
            }
        }
        AstMatcher::Util(util) => names.extend(utility_captures(util.index, utils, summaries)),
        // A call returns the parameters its utility binds, under the caller's names.
        AstMatcher::Call(call) => {
            let inner = utility_captures(call.util.index, utils, summaries);
            names.extend(call.outward(inner));
        }
        AstMatcher::Constrained(core) => {
            ast_captures(&core.rule, utils, summaries, names);
            for (_, constraint) in &core.constraints {
                ast_captures(constraint, utils, summaries, names);
            }
        }
    }
}

/// Utility summaries of `ast_captures`: each utility's names in its own environment.
pub(super) type CaptureSummaries<'a> = UtilitySummary<usize, BTreeSet<&'a str>>;

/// The names utility `index` binds in its own environment.
fn utility_captures<'a>(
    index: usize,
    utils: &'a UtilTable,
    summaries: &mut CaptureSummaries<'a>,
) -> BTreeSet<&'a str> {
    let Some(rule) = utils.rules.get().and_then(|rules| rules.get(index)) else {
        return BTreeSet::new();
    };
    summaries.summarize(index, BTreeSet::new(), |summaries| {
        #[cfg(test)]
        record_walk(Walk::AstCaptures);
        let mut summary = BTreeSet::new();
        ast_captures(rule, utils, summaries, &mut summary);
        summary
    })
}

/// What `parse_exports` collects: the names parsed trees export, and the names a node capture
/// binds.
type Exports<'a> = (BTreeSet<&'a str>, BTreeSet<&'a str>);

/// `parse_exports`' utility summaries, each utility's names in its own environment, and
/// `ast_captures`' for the terms it reads as node captures.
pub(super) struct ExportSummaries<'s, 'a> {
    utilities: &'s mut UtilitySummary<usize, Exports<'a>>,
    captures: &'s mut CaptureSummaries<'a>,
}

/// The names `matcher` exports from parsed trees and binds as node captures, in one walk.
fn exports<'a>(matcher: &'a AstMatcher, utils: &'a UtilTable) -> Exports<'a> {
    let mut summary = Exports::default();
    parse_exports(
        matcher,
        utils,
        &mut ExportSummaries {
            utilities: &mut UtilitySummary::new(),
            captures: &mut CaptureSummaries::new(),
        },
        &mut summary.0,
        &mut summary.1,
    );
    summary
}

/// The names parsed trees export (`exported`) and the names a node capture binds (`bound`).
fn parse_exports<'a>(
    matcher: &'a AstMatcher,
    utils: &'a UtilTable,
    summaries: &mut ExportSummaries<'_, 'a>,
    exported: &mut BTreeSet<&'a str>,
    bound: &mut BTreeSet<&'a str>,
) {
    match matcher {
        AstMatcher::Semantic(semantic) => {
            semantic::parse_exports(semantic, utils, summaries, exported, bound)
        }
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for item in items {
                parse_exports(item, utils, summaries, exported, bound);
            }
        }
        AstMatcher::Inside(item, _)
        | AstMatcher::Has(item, _)
        | AstMatcher::Precedes(item, _)
        | AstMatcher::Follows(item, _) => parse_exports(item, utils, summaries, exported, bound),
        AstMatcher::Field(relation) => {
            parse_exports(&relation.rule, utils, summaries, exported, bound);
        }
        AstMatcher::Util(util) => {
            let (inner_exported, inner_bound) = utility_exports(util.index, utils, summaries);
            exported.extend(inner_exported);
            bound.extend(inner_bound);
        }
        // A call returns the parameters its utility exports or binds, under the caller's names.
        AstMatcher::Call(call) => {
            let (inner_exported, inner_bound) = utility_exports(call.util.index, utils, summaries);
            exported.extend(call.outward(inner_exported));
            bound.extend(call.outward(inner_bound));
        }
        AstMatcher::Constrained(core) => {
            parse_exports(&core.rule, utils, summaries, exported, bound);
            for (_, constraint) in &core.constraints {
                parse_exports(constraint, utils, summaries, exported, bound);
            }
        }
        other => ast_captures(other, utils, summaries.captures, bound),
    }
}

/// What utility `index` exports and binds in its own environment.
fn utility_exports<'a>(
    index: usize,
    utils: &'a UtilTable,
    summaries: &mut ExportSummaries<'_, 'a>,
) -> Exports<'a> {
    let Some(rule) = utils.rules.get().and_then(|rules| rules.get(index)) else {
        return Exports::default();
    };
    let ExportSummaries {
        utilities,
        captures,
    } = summaries;
    utilities.summarize(index, Exports::default(), |utilities| {
        let mut summary = Exports::default();
        let mut summaries = ExportSummaries {
            utilities,
            captures,
        };
        parse_exports(rule, utils, &mut summaries, &mut summary.0, &mut summary.1);
        summary
    })
}

/// The named groups of the regexes that test the witness node itself (`node_regexes`).
pub(super) fn witness_groups(matcher: &AstMatcher, utils: &UtilTable) -> Vec<String> {
    let mut regexes = Vec::new();
    node_regexes(matcher, utils, &mut regexes);
    regexes
        .iter()
        .flat_map(|regex| regex.capture_names().flatten().map(str::to_owned))
        .collect()
}

/// The same-node regexes a matcher contributes to a violation or evidence projection.
pub(super) fn witness_regexes(matcher: &AstMatcher, utils: &UtilTable) -> Vec<Regex> {
    let mut regexes = Vec::new();
    node_regexes(matcher, utils, &mut regexes);
    regexes.into_iter().cloned().collect()
}

/// Regexes with named groups that test the witness node itself. `not` and relations test other
/// nodes (or none), so their groups never describe the witness; neither do constraints, which
/// test captured nodes. A `matches` reference tests the same node, so its utility's regexes count.
fn node_regexes<'a>(matcher: &'a AstMatcher, utils: &'a UtilTable, regexes: &mut Vec<&'a Regex>) {
    node_regexes_in(matcher, utils, &mut UtilitySummary::new(), regexes);
}

/// Regexes a utility contributes, each once, told apart by identity: the same compiled regex
/// reached along several paths. Two summaries are the same when they hold the same regexes.
#[derive(Clone, Default)]
struct UtilityRegexes<'a>(Vec<&'a Regex>);

impl PartialEq for UtilityRegexes<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .all(|regex| other.0.iter().any(|known| std::ptr::eq(*known, *regex)))
    }
}

fn node_regexes_in<'a>(
    matcher: &'a AstMatcher,
    utils: &'a UtilTable,
    summaries: &mut UtilitySummary<usize, UtilityRegexes<'a>>,
    regexes: &mut Vec<&'a Regex>,
) {
    match matcher {
        AstMatcher::Regex(regex) => {
            if regex.capture_names().flatten().next().is_some() {
                regexes.push(regex);
            }
        }
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for item in items {
                node_regexes_in(item, utils, summaries, regexes);
            }
        }
        AstMatcher::Util(util) => node_regexes_utility(util.index, utils, summaries, regexes),
        // A parameterized utility still tests the caller's witness. Its private frame changes
        // capture names, not which regexes describe that node.
        AstMatcher::Call(call) => {
            node_regexes_utility(call.util.index, utils, summaries, regexes);
        }
        AstMatcher::Constrained(core) => node_regexes_in(&core.rule, utils, summaries, regexes),
        AstMatcher::Pattern(_)
        | AstMatcher::Kind(_)
        | AstMatcher::Not(_)
        | AstMatcher::Compare(_)
        | AstMatcher::Count(_)
        | AstMatcher::Never
        | AstMatcher::Inside(..)
        | AstMatcher::Has(..)
        | AstMatcher::Precedes(..)
        | AstMatcher::Follows(..)
        | AstMatcher::Field(_)
        | AstMatcher::NthChild(_) => {}
        AstMatcher::Semantic(semantic) => {
            semantic::node_regexes(semantic, utils, summaries, regexes)
        }
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => {}
    }
}

/// Adds a utility's regexes, each once: a utility reached along several paths tests the same node,
/// so its regexes describe the witness once.
fn node_regexes_utility<'a>(
    index: usize,
    utils: &'a UtilTable,
    summaries: &mut UtilitySummary<usize, UtilityRegexes<'a>>,
    regexes: &mut Vec<&'a Regex>,
) {
    let Some(rule) = utils.rules.get().and_then(|rules| rules.get(index)) else {
        return;
    };
    let summary = summaries.summarize(index, UtilityRegexes::default(), |summaries| {
        #[cfg(test)]
        record_walk(Walk::NodeRegexes);
        let mut summary = Vec::new();
        node_regexes_in(rule, utils, summaries, &mut summary);
        UtilityRegexes(summary)
    });
    for regex in summary.0 {
        if !regexes.iter().any(|known| std::ptr::eq(*known, regex)) {
            regexes.push(regex);
        }
    }
}

fn describe(names: &BTreeSet<&str>) -> String {
    if names.is_empty() {
        "the query binds no captures".to_owned()
    } else {
        let names = names.iter().copied().collect::<Vec<_>>().join(", ");
        format!("the query can bind: {names}")
    }
}

/// Checks the violation's anchor, message references, and fix against the query that emits its
/// witnesses, and compiles the fix. `fix` is the raw `violation.fix` template.
pub(super) fn bind(
    violation: &mut ViolationSpec,
    fix: Option<&str>,
    kind: &ClauseKind,
    path: &str,
) -> Result<()> {
    let query = match kind {
        ClauseKind::Forbid(query) => Some(emitting(query)),
        ClauseKind::RequireEach { .. } | ClauseKind::Budget { .. } | ClauseKind::Llm => None,
    };
    let mut names = BTreeSet::new();
    // The values counts bind (`as`): text a message alone may interpolate.
    let mut counted = BTreeSet::new();
    let mut regex_group = false;
    // Sub-node regex group names that no metavariable of the same name shadows.
    let mut node_groups = BTreeSet::new();
    match query {
        Some(Query::Ast { matchers, .. }) => {
            let mut analysis = CaptureAnalysis::default();
            for grammar in matchers {
                counted.extend(analysis.counts(&grammar.matcher));
                ast_captures(
                    &grammar.matcher,
                    &grammar.utils,
                    &mut UtilitySummary::new(),
                    &mut names,
                );
            }
            for grammar in matchers {
                let mut regexes = Vec::new();
                node_regexes(&grammar.matcher, &grammar.utils, &mut regexes);
                for regex in &regexes {
                    node_groups.extend(
                        regex
                            .capture_names()
                            .flatten()
                            .filter(|name| !names.contains(name)),
                    );
                }
                if !regexes.is_empty() {
                    violation
                        .node_regexes
                        .push((grammar.language, regexes.into_iter().cloned().collect()));
                }
            }
            if let Some(each) = &violation.each {
                check_each(each, matchers, &names, &node_groups, path)?;
            }
            // A transform of a group is computed again with each witness's groups.
            violation.group_transform = matchers
                .first()
                .and_then(|grammar| grammar.transform.as_ref())
                .filter(|transform| {
                    transform
                        .steps
                        .iter()
                        .any(|(_, trans)| node_groups.contains(trans.source_name()))
                })
                .cloned();
            names.extend(node_groups.iter().copied());
        }
        Some(Query::Regex { regex, .. }) => {
            regex_group = true;
            names.extend(regex.capture_names().flatten());
        }
        Some(Query::SelectWhen { .. }) | None => {}
    }
    if let Some(suppress) = &violation.suppress {
        let Some(Query::Ast { matchers, .. }) = query else {
            return Err(invalid(format!(
                "{path}/suppress: suppress removes an ast query's violations"
            )));
        };
        // A query's matchers are the dialects of one code-rule language, so of one family, which
        // honours its own linters' comments or none: the file's, for a query over the program
        // it embeds too.
        let family = matchers.first().map(|grammar| match grammar.document {
            Some(via) => via.host_family(),
            None => Family::of(grammar.language),
        });
        let language = family.map_or("empty", |family| with_adapter!(family, A => A::LABEL));
        for linter in suppress.requested() {
            if !family.is_some_and(|family| family.honours(linter)) {
                return Err(invalid(format!(
                    "{path}/suppress/{}; a {language} query has none",
                    linter.detail()
                )));
            }
        }
    }
    if violation.each.is_some() && !matches!(query, Some(Query::Ast { .. })) {
        return Err(invalid(format!(
            "{path}/each: each enumerates the captures of an ast query"
        )));
    }
    if query.is_none() && violation.anchor != Anchor::Span {
        return Err(invalid(format!(
            "{path}/anchor: require_each, budget, and llm violations support only \"witness.span\"; \
             fact violations carry no source span to anchor within"
        )));
    }
    if let Anchor::Capture(name) = &violation.anchor
        && !names.contains(name.as_str())
    {
        return Err(invalid(format!(
            "{path}/anchor: `capture.{name}` names no capture; {}",
            describe(&names)
        )));
    }
    // A sub-range fix replaces a capture's range, which only an ast query's captures have.
    if let Some(name) = &violation.fix_range {
        if !matches!(query, Some(Query::Ast { .. })) {
            return Err(invalid(format!(
                "{path}/fix/range: a fix range names a capture of an ast query"
            )));
        }
        if !names.contains(name.as_str()) {
            return Err(invalid(format!(
                "{path}/fix/range: `capture.{name}` names no capture; {}",
                describe(&names)
            )));
        }
    }
    // Transform variables are text a message or fix may interpolate; they have no span to anchor.
    let transformed = match query {
        Some(Query::Ast { matchers, .. }) => matchers
            .first()
            .and_then(|grammar| grammar.transform.as_ref())
            .map(|transform| transform.names().map(str::to_owned).collect::<Vec<_>>())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    names.extend(transformed.iter().map(String::as_str));
    let references = |template: &Template, field: &str| -> Result<()> {
        for (name, multi) in template.vars() {
            let sigils = if multi { "$$$" } else { "$" };
            if counted.contains(name) {
                if field != "message" {
                    return Err(invalid(format!(
                        "{path}/{field}: `{sigils}{name}` is a count's value, which only a \
                         message may interpolate"
                    )));
                }
                if names.contains(name) {
                    return Err(invalid(format!(
                        "{path}/message: `${name}` names both a capture and a count's value; \
                         rename one"
                    )));
                }
                if multi {
                    return Err(invalid(format!(
                        "{path}/message: `$$${name}`: a count's value is text; use `${name}`"
                    )));
                }
                continue;
            }
            if !names.contains(name) {
                let hint = if field == "message" {
                    format!("; write `\\{sigils}{name}` for literal text")
                } else {
                    String::new()
                };
                return Err(invalid(format!(
                    "{path}/{field}: `{sigils}{name}` names no capture; {}{hint}",
                    describe(&names)
                )));
            }
            if multi && transformed.iter().any(|known| known == name) {
                return Err(invalid(format!(
                    "{path}/{field}: `$$${name}`: a transform variable is text; use `${name}`"
                )));
            }
            if multi && (regex_group || node_groups.contains(name)) {
                return Err(invalid(format!(
                    "{path}/{field}: `$$${name}`: regex groups are single captures; use `${name}`"
                )));
            }
            if field == "fix" && node_groups.contains(name) {
                return Err(invalid(format!(
                    "{path}/fix: `${name}` is a regex group inside an ast node; an ast fix can \
                     use only metavariables"
                )));
            }
        }
        Ok(())
    };
    let message = Template::parse_message(&violation.message);
    references(&message, "message")?;
    if message.is_literal() {
        // Escapes resolved once; a literal message needs no per-witness rendering.
        violation.message = message.render(|_, _| None);
        violation.message_template = None;
    } else {
        violation.message_template = Some(message);
    }
    let Some(fix) = fix else {
        return Ok(());
    };
    let template = Template::parse(fix);
    references(&template, "fix")?;
    let upstream = template.upstream();
    if upstream.is_some()
        && let Some(name) = names.iter().find(|name| name.starts_with(FIX_TEXT_PREFIX))
    {
        return Err(invalid(format!(
            "{path}/fix: `${name}` is reserved for a braced fix's text; rename the capture"
        )));
    }
    let (fix, literals) = match &upstream {
        Some((text, literals)) => (text.as_str(), literals.clone()),
        None => (fix, Vec::new()),
    };
    let plan = match query {
        Some(Query::Ast { matchers, .. }) => FixPlan::Ast(
            matchers
                .iter()
                .map(|grammar| {
                    // A parsed tree's captures and transform variables are text, which a
                    // template reads as a transform.
                    let (exported, bound) = exports(&grammar.matcher, &grammar.utils);
                    let mut text = exported
                        .difference(&bound)
                        .map(|name| (*name).to_owned())
                        .collect::<Vec<_>>();
                    text.extend(transformed.iter().cloned());
                    text.extend(literals.iter().map(|(name, _)| name.clone()));
                    if text.is_empty() {
                        TemplateFix::try_new(fix, &grammar.language)
                            .map_err(|error| invalid(format!("{path}/fix: {error}")))
                    } else {
                        Ok(TemplateFix::with_transform(fix, &grammar.language, &text))
                    }
                    .map(|compiled| (grammar.language, compiled))
                })
                .collect::<Result<_>>()?,
            literals,
        ),
        Some(Query::Regex {
            source: RegexSource::Content,
            ..
        }) => FixPlan::Regex(template),
        Some(Query::Regex {
            source: RegexSource::Path,
            ..
        }) => {
            return Err(invalid(format!(
                "{path}/fix: a file.path regex match is not source text a fix can replace"
            )));
        }
        _ => {
            return Err(invalid(format!(
                "{path}/fix: only forbid clauses over an ast or file.content regex query can suggest a fix"
            )));
        }
    };
    violation.fix = Some(Arc::new(plan));
    Ok(())
}

/// `each` names a single capture of the query: a metavariable, or a named group of a regex that
/// tests the witness node itself (whose every match enumerates, `each_node_group_set`).
fn check_each(
    each: &str,
    matchers: &[GrammarMatcher],
    names: &BTreeSet<&str>,
    node_groups: &BTreeSet<&str>,
    path: &str,
) -> Result<()> {
    let valid = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_';
    if !each.chars().all(valid) {
        return Err(invalid(format!(
            "{path}/each: `{each}` is not a metavariable name (A-Z, 0-9, _, without `$`)"
        )));
    }
    if node_groups.contains(each) {
        return Ok(());
    }
    if !names.contains(each) {
        let mut all = names.clone();
        all.extend(node_groups.iter().copied());
        return Err(invalid(format!(
            "{path}/each: `${each}` names no capture; {}",
            describe(&all)
        )));
    }
    let mut variables = BTreeMap::new();
    for grammar in matchers {
        collect_variables(&grammar.matcher, &mut variables);
        for util in grammar.utils.shared_rules() {
            collect_variables(util, &mut variables);
        }
    }
    if variables.get(each) == Some(&Capture::Multiple) {
        return Err(invalid(format!(
            "{path}/each: `$$${each}` captures a node list; each enumerates single-node captures"
        )));
    }
    Ok(())
}

/// Text a capture interpolates: a single capture's node, or a multi-capture's source from its
/// first node to its last (ast-grep-core 0.45.3 `maybe_get_var`, without re-indentation).
fn ast_capture_text<'e, D: Doc>(
    env: &'e MetaVarEnv<'_, D>,
    name: &str,
    multi: bool,
) -> Option<Cow<'e, str>> {
    if !multi {
        return env.get_match(name).map(|node| node.text()).or_else(|| {
            env.get_transformed(name)
                .map(|bytes| Cow::Owned(<D::Source as Content>::encode_bytes(bytes).into_owned()))
        });
    }
    let nodes = env.get_multiple_matches(name);
    let (first, last) = (nodes.first()?, nodes.last()?);
    let bytes = first
        .get_doc()
        .get_source()
        .get_range(first.range().start..last.range().end);
    Some(Cow::Owned(
        <D::Source as Content>::encode_bytes(bytes).into_owned(),
    ))
}

/// A sub-node regex group of the witness: its name, text, and absolute byte range.
pub(super) type NodeGroup = (String, std::sync::Arc<str>, Range<usize>);

/// Named groups of the same-node regexes that match the witness text, each name once, first
/// regex first.
pub(super) fn node_groups<D: Doc>(
    violation: &ViolationSpec,
    language: GrammarDialect,
    node: &Node<'_, D>,
) -> Vec<NodeGroup> {
    let Some((_, regexes)) = violation
        .node_regexes
        .iter()
        .find(|(dialect, _)| *dialect == language)
    else {
        return Vec::new();
    };
    let text = node.text();
    let start = node.range().start;
    let mut groups: Vec<NodeGroup> = Vec::new();
    for regex in regexes {
        let Some(found) = regex.captures(&text) else {
            continue;
        };
        for name in regex.capture_names().flatten() {
            if let Some(group) = found.name(name)
                && groups.iter().all(|(known, _, _)| known != name)
            {
                groups.push((
                    name.to_owned(),
                    std::sync::Arc::from(group.as_str()),
                    start + group.start()..start + group.end(),
                ));
            }
        }
    }
    groups
}

/// The node groups of each violation one ast match reports, passed to `visit` in order until it
/// returns false: one set, `node_groups`', unless `violation.each` names a regex group no
/// metavariable shadows. Then each match of each same-node regex holding that group (in order,
/// non-overlapping, as `captures_iter` walks the witness text) that binds it to a range not seen
/// yet is one set: that match's groups, then the other regexes' as `node_groups` reads them. A
/// witness where no match binds it has none: it reports nothing, as the linter reports nothing
/// where no run is found. `cancelled` is polled before each match; once it holds, the walk ends.
pub(super) fn each_node_group_set<D: Doc>(
    violation: &ViolationSpec,
    language: GrammarDialect,
    matched: &NodeMatch<'_, D>,
    cancelled: impl Fn() -> bool,
    mut visit: impl FnMut(Vec<NodeGroup>) -> bool,
) {
    let node = matched.get_node();
    let first = node_groups(violation, language, node);
    let Some(each) = violation.each.as_deref() else {
        visit(first);
        return;
    };
    let env = matched.get_env();
    let bound = env.get_match(each).is_some()
        || !env.get_multiple_matches(each).is_empty()
        || binding_range(env, each).is_some();
    let regexes = violation
        .node_regexes
        .iter()
        .find(|(dialect, _)| *dialect == language)
        .map(|(_, regexes)| regexes.as_slice())
        .unwrap_or_default();
    let holding = regexes
        .iter()
        .filter(|regex| regex.capture_names().flatten().any(|name| name == each))
        .collect::<Vec<_>>();
    if bound || holding.is_empty() {
        visit(first);
        return;
    }
    let text = node.text();
    let start = node.range().start;
    let mut seen = BTreeSet::new();
    for regex in holding {
        for found in regex.captures_iter(&text) {
            if cancelled() {
                return;
            }
            #[cfg(test)]
            NODE_GROUP_ENUMERATIONS.with(|count| count.set(count.get() + 1));
            let Some(group) = found.name(each) else {
                continue;
            };
            let range = start + group.start()..start + group.end();
            if !seen.insert((range.start, range.end)) {
                continue;
            }
            let mut groups: Vec<NodeGroup> = regex
                .capture_names()
                .flatten()
                .filter_map(|name| {
                    let group = found.name(name)?;
                    Some((
                        name.to_owned(),
                        std::sync::Arc::from(group.as_str()),
                        start + group.start()..start + group.end(),
                    ))
                })
                .collect();
            // Another regex's groups as `node_groups` reads them; this regex's that the match
            // leaves out stay unbound.
            for other in &first {
                if regex.capture_names().flatten().all(|name| name != other.0) {
                    groups.push(other.clone());
                }
            }
            if !visit(groups) {
                return;
            }
        }
    }
}

/// The interpolated message and suggested fix for an ast witness; `(None, None)` when the
/// violation needs neither.
pub(super) fn ast_output<D: Doc>(
    violation: &ViolationSpec,
    language: GrammarDialect,
    matched: &NodeMatch<'_, D>,
    groups: &[NodeGroup],
    place: &InFile<'_>,
) -> (Option<String>, Option<Value>) {
    let env = matched.get_env();
    let message = violation.message_template.as_ref().map(|template| {
        template.render(|name, multi| {
            ast_capture_text(env, name, multi)
                .or_else(|| {
                    groups
                        .iter()
                        .find(|(group, _, _)| group == name)
                        .map(|(_, text, _)| Cow::Borrowed(text.as_ref()))
                })
                .map(|text| Cow::Owned(message_capture(&text).into_owned()))
        })
    });
    let fix = match violation.fix.as_deref() {
        Some(FixPlan::Ast(dialects, literals)) => dialects
            .iter()
            .find(|(dialect, _)| *dialect == language)
            .and_then(|(_, template)| {
                // A braced template's literal text, as the transform variables it reads.
                let with_literals;
                let (matched, env) = if literals.is_empty() {
                    (matched, env)
                } else {
                    let mut env = env.clone();
                    for (name, text) in literals {
                        let bytes = <D::Source as Content>::decode_str(text).into_owned();
                        env.insert_transformation(
                            &MetaVariable::Capture(name.clone(), false),
                            name,
                            bytes,
                        );
                    }
                    with_literals = NodeMatch::new(matched.get_node().clone(), env);
                    (&with_literals, with_literals.get_env())
                };
                let (range, bytes) = match &violation.fix_range {
                    None => (
                        matched.get_node().range(),
                        template.generate_replacement(matched),
                    ),
                    Some(name) => {
                        let (range, node) = fix_target(env, name, groups)?;
                        // A fix replaces text inside its witness only.
                        let witness = matched.get_node().range();
                        if range.start < witness.start || range.end > witness.end {
                            return None;
                        }
                        // Re-indented as at the target node, else as at the witness.
                        let node = node.unwrap_or_else(|| matched.get_node().clone());
                        let target = NodeMatch::new(node, env.clone());
                        (range, template.generate_replacement(&target))
                    }
                };
                // A fix rewrites the file's bytes: only those the range copies (in an embedded
                // program, `embed`, the program wrote the rest).
                let range = place.copied(range)?;
                Some(json!({
                    "span": place.span(range),
                    "text": <D::Source as Content>::encode_bytes(&bytes),
                }))
            }),
        _ => None,
    };
    (message, fix)
}

/// The range a `fix.range` capture names in this match, and its node where it has one: a single
/// capture, a `$$$` capture from its first node to its last, a regex group of the witness, or a
/// capture a parsed sub-tree exported. `None` where it is unbound.
fn fix_target<'t, D: Doc>(
    env: &MetaVarEnv<'t, D>,
    name: &str,
    groups: &[NodeGroup],
) -> Option<(Range<usize>, Option<Node<'t, D>>)> {
    if let Some(node) = env.get_match(name) {
        return Some((node.range(), Some(node.clone())));
    }
    let nodes = env.get_multiple_matches(name);
    if let (Some(first), Some(last)) = (nodes.first(), nodes.last()) {
        return Some((first.range().start..last.range().end, Some(first.clone())));
    }
    if let Some((_, _, range)) = groups.iter().find(|(group, _, _)| group == name) {
        return Some((range.clone(), None));
    }
    binding_range(env, name).map(|range| (range, None))
}

#[derive(Clone)]
pub(crate) struct Witness {
    pub(crate) path: String,
    pub(crate) span: Span,
    pub(crate) captures: Captures,
    /// The violation message with captures interpolated, when it references any.
    pub(crate) message: Option<String>,
    /// The suggested fix: `{ span, text }` replacing the witness span.
    pub(crate) fix: Option<Value>,
    /// A `rerail-ignore` naming the code rule covers it, where its violation marks such witnesses
    /// instead of dropping them (`ViolationSpec::ignore_marks`: a `budget`'s rows).
    pub(crate) ignored: bool,
}

/// The key under which a witness of an evidence producer carries its projected row.
const PROJECTION: &str = "$project";

/// A witness's captures: each bound name's spans, and an evidence projection when its clause
/// projects one. It serializes as the JSON object of each name's span list and the projection
/// under `PROJECTION`, in key order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Captures {
    /// Each name's spans, in name order.
    spans: Vec<(Box<str>, Vec<Span>)>,
    projection: Option<Box<Value>>,
}

impl Captures {
    pub(crate) fn owned_heap_bytes(&self) -> usize {
        self.spans.capacity() * std::mem::size_of::<(Box<str>, Vec<Span>)>()
            + self
                .spans
                .iter()
                .map(|(name, spans)| name.len() + spans.capacity() * std::mem::size_of::<Span>())
                .sum::<usize>()
    }

    pub(crate) fn from_spans(spans: BTreeMap<String, Vec<Span>>) -> Self {
        Self {
            spans: spans
                .into_iter()
                .map(|(name, spans)| (name.into_boxed_str(), spans))
                .collect(),
            projection: None,
        }
    }

    /// The spans `name` captured, if it is bound.
    pub(crate) fn get(&self, name: &str) -> Option<&[Span]> {
        self.spans
            .binary_search_by(|(known, _)| known.as_ref().cmp(name))
            .ok()
            .map(|index| self.spans[index].1.as_slice())
    }

    pub(crate) fn projection(&self) -> Option<&Value> {
        self.projection.as_deref()
    }

    pub(crate) fn set_projection(&mut self, projection: Option<Value>) {
        self.projection = projection.map(Box::new);
    }

    /// The captures a stored witness's JSON object holds; `None` for any other shape.
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        let mut captures = Self::default();
        for (name, value) in value.as_object()? {
            if name == PROJECTION {
                captures.projection = Some(Box::new(value.clone()));
            } else {
                let spans = value
                    .as_array()?
                    .iter()
                    .map(|span| Span::deserialize(span).ok())
                    .collect::<Option<Vec<_>>>()?;
                captures.spans.push((name.as_str().into(), spans));
            }
        }
        Some(captures)
    }
}

impl Serialize for Captures {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(
            self.spans.len() + usize::from(self.projection.is_some()),
        ))?;
        let mut projection = self.projection.as_deref();
        for (name, spans) in &self.spans {
            if let Some(value) = projection.filter(|_| PROJECTION < name.as_ref()) {
                map.serialize_entry(PROJECTION, value)?;
                projection = None;
            }
            map.serialize_entry(name.as_ref(), spans)?;
        }
        if let Some(value) = projection {
            map.serialize_entry(PROJECTION, value)?;
        }
        map.end()
    }
}

/// A regex witness: named groups that matched become captures; the message and fix
/// substitute their text.
pub(super) fn regex_witness(
    path: &str,
    regex: &Regex,
    found: &regex::Captures<'_>,
    violation: Option<&ViolationSpec>,
    span: impl Fn(usize, usize) -> Span,
) -> Witness {
    let whole = found.get(0).expect("group 0 always matches");
    let mut captures = BTreeMap::new();
    for name in regex.capture_names().flatten() {
        if let Some(group) = found.name(name) {
            captures.insert(name.to_owned(), vec![span(group.start(), group.end())]);
        }
    }
    let lookup =
        |name: &str, _multi: bool| found.name(name).map(|group| Cow::Borrowed(group.as_str()));
    let message = violation
        .and_then(|violation| violation.message_template.as_ref())
        .map(|template| {
            template.render(|name, multi| {
                lookup(name, multi).map(|text| Cow::Owned(message_capture(&text).into_owned()))
            })
        });
    let fix = match violation.and_then(|violation| violation.fix.as_deref()) {
        Some(FixPlan::Regex(template)) => Some(json!({
            "span": span(whole.start(), whole.end()),
            "text": template.render(lookup),
        })),
        _ => None,
    };
    Witness {
        path: path.to_owned(),
        span: span(whole.start(), whole.end()),
        captures: Captures::from_spans(captures),
        message,
        fix,
        ignored: false,
    }
}

/// The span a violation reports and whether a capture anchor fell back to the witness span.
pub(super) fn anchored_span(anchor: &Anchor, witness: &Witness) -> (Span, bool) {
    let span = witness.span;
    match anchor {
        Anchor::Span => (span, false),
        Anchor::Start => (
            Span {
                end_byte: span.start_byte,
                end_column: span.start_column,
                end_line: span.start_line,
                ..span
            },
            false,
        ),
        Anchor::End => (
            Span {
                start_byte: span.end_byte,
                start_column: span.end_column,
                start_line: span.end_line,
                ..span
            },
            false,
        ),
        Anchor::FileStart => (Span::default(), false),
        Anchor::Capture(name) => match witness.captures.get(name) {
            Some([first, .., last]) => (
                Span {
                    end_byte: last.end_byte,
                    end_column: last.end_column,
                    end_line: last.end_line,
                    ..*first
                },
                false,
            ),
            Some([only]) => (*only, false),
            _ => (span, true),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::{CodeRuleDslRuntime, CompiledCodeRuleDsl};
    use super::*;

    const CAPABILITIES: &str = include_str!("capabilities.json");

    fn compile(forbid: Value, violation: Value) -> Result<CompiledCodeRuleDsl> {
        compile_clause(json!({ "id": "c", "forbid": forbid, "violation": violation }))
    }

    fn compile_clause(clause: Value) -> Result<CompiledCodeRuleDsl> {
        let mut capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
        capabilities["factRelations"] = json!({ "symbols": {} });
        let document = json!({ "format": "rerail.policy/v1", "id": "p", "revision": 1, "mode": "block", "clauses": [clause] });
        CodeRuleDslRuntime::compile(
            document.to_string(),
            "json".to_owned(),
            capabilities.to_string(),
        )
    }

    fn ast(rule: Value) -> Value {
        json!({ "ast": { "language": "typescript", "rule": rule } })
    }

    fn violation(anchor: &str, message: &str) -> Value {
        json!({ "anchor": anchor, "code": "X", "message": message })
    }

    fn with_fix(anchor: &str, fix: &str) -> Value {
        json!({ "anchor": anchor, "code": "X", "message": "m", "fix": fix })
    }

    fn violations_for(compiled: &CompiledCodeRuleDsl, language: &str, source: &str) -> Vec<Value> {
        let snapshot = json!({
            "facts": [],
            "files": [{ "language": language, "path": "a.ts", "source": source }],
            "format": "rerail.repository-snapshot/v1",
            "revision": 0,
        });
        let output = compiled
            .evaluate(snapshot.to_string(), "report".to_owned())
            .expect("evaluation succeeds");
        let result: Value = serde_json::from_str(&output).expect("JSON");
        assert_eq!(result["failures"], json!([]), "{result}");
        result["violations"].as_array().expect("findings").clone()
    }

    fn only(compiled: &CompiledCodeRuleDsl, source: &str) -> Value {
        let found = violations_for(compiled, "typescript", source);
        assert_eq!(found.len(), 1, "{found:?}");
        found[0].clone()
    }

    fn point(span: &Value) -> (u64, u64, u64, u64) {
        let field = |name: &str| span[name].as_u64().expect(name);
        (
            field("startLine"),
            field("startColumn"),
            field("endLine"),
            field("endColumn"),
        )
    }

    fn rejection(result: Result<CompiledCodeRuleDsl>) -> String {
        result.err().expect("compile is rejected").reason
    }

    #[test]
    fn messages_keep_escaped_and_digit_dollar_names_literal() {
        // Review finding 5 on #2561: a message had no way to say `$HOME` or `$100`. `\$NAME` is
        // literal text, and a name starting with a digit, which nothing can bind, stays text too
        // (ast-grep renders both empty). Fix templates keep ast-grep's reading.
        let compiled = compile(
            ast(json!({ "pattern": "foo($A)" })),
            violation("witness.span", r"set \$HOME to $A, pay $100, keep \x"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(
            only(&compiled, "foo(bar);\n")["message"],
            r"set $HOME to bar, pay $100, keep \x"
        );
        let literal = compile(
            json!({ "regex": { "source": "file.content", "pattern": "foo" } }),
            violation("witness.span", r"costs $5, see \$DOC"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(only(&literal, "foo\n")["message"], "costs $5, see $DOC");
        assert_eq!(
            rejection(compile(
                ast(json!({ "pattern": "foo($A)" })),
                with_fix("witness.span", "bar($5)")
            )),
            "/clauses/0/violation/fix: `$5` names no capture; the query can bind: A"
        );
    }

    #[test]
    fn a_regex_without_named_groups_carries_its_literal_fix() {
        // Review finding 4 on #2561: the group-less fast path built witnesses without a fix.
        for pattern in ["debugger;", "(?<D>debugger;)"] {
            let compiled = compile(
                json!({ "regex": { "source": "file.content", "pattern": pattern } }),
                with_fix("witness.span", ""),
            )
            .unwrap_or_else(|error| panic!("{}", error.reason));
            let found = only(&compiled, "debugger;\n");
            assert_eq!(found["fix"]["text"], "", "{pattern}: {found}");
            assert_eq!(point(&found["fix"]["span"]), (0, 0, 0, 9), "{pattern}");
        }
    }

    #[test]
    fn templates_split_metavariables_like_ast_grep() {
        let names = |text: &str| {
            Template::parse(text)
                .vars()
                .map(|(name, multi)| format!("{}{name}", if multi { "$$$" } else { "$" }))
                .collect::<Vec<_>>()
        };
        assert_eq!(names("Found $A in $$$ARGS."), ["$A", "$$$ARGS"]);
        assert_eq!(names("$$A"), ["$A"]);
        assert_eq!(names("costs $ 5, $lower, $$$$A"), ["$$$A"]);
        assert_eq!(names("$A1_B-c"), ["$A1_B"]);
        let rendered = Template::parse("<$A|$B>")
            .render(|name, _| (name == "A").then_some(Cow::Borrowed("x")));
        assert_eq!(rendered, "<x|>");
        assert!(Template::parse("plain $ text").is_literal());
    }

    /// `${NAME}` (a Rerail extension; ast-grep 0.45.3 renders it literally) names a variable next
    /// to text that would continue its name, in messages and fixes alike; `\${NAME}` is a
    /// backslash before the variable, where `\$NAME` stays the escape.
    #[test]
    fn braced_variables_sit_next_to_name_characters() {
        let names = |text: &str| {
            Template::parse(text)
                .vars()
                .map(|(name, multi)| format!("{}{name}", if multi { "$$$" } else { "$" }))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names("${A}B $$${ARGS}_1 ${a} ${A $${B}"),
            ["$A", "$$$ARGS", "$B"]
        );
        let compiled = compile(
            ast(json!({ "pattern": "f($A, $$$R)" })),
            violation("witness.span", r"${A}_X [$$${R}] \${A} \$A ${a}"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(
            only(&compiled, "f(b, c, d);\n")["message"],
            r"b_X [c, d] \b $A ${a}"
        );
        let fixed = |fix: &str, source: &str| {
            let compiled = compile(
                ast(json!({ "pattern": "f($A, $$$R)" })),
                with_fix("witness.span", fix),
            )
            .unwrap_or_else(|error| panic!("{}", error.reason));
            only(&compiled, source)["fix"]["text"].clone()
        };
        assert_eq!(
            fixed("g_${A}_SUFFIX(${A}X, $$${R}) $${A}$", "f(b, c, d);\n"),
            "g_b_SUFFIX(bX, c, d) $b$"
        );
        // Re-indented as the unbraced template is.
        let source = "function k() {\n  f(a, {\n    b,\n  });\n}\n";
        assert_eq!(
            fixed("if (x) {\n  ${A}Z($$$R);\n}", source),
            json!(
                fixed("if (x) {\n  $A($$$R);\n}", source)
                    .as_str()
                    .expect("text")
                    .replacen("a(", "aZ(", 1)
            )
        );
        let regex = compile(
            json!({ "regex": { "source": "file.content", "pattern": "(?<N>[0-9]+)" } }),
            with_fix("witness.span", "${N}0"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(only(&regex, "7\n")["fix"]["text"], "70");
        assert_eq!(
            rejection(compile(
                ast(json!({ "pattern": "f($A)" })),
                violation("witness.span", "${NOPE}")
            )),
            r"/clauses/0/violation/message: `$NOPE` names no capture; the query can bind: A; write `\$NOPE` for literal text"
        );
    }

    #[test]
    fn long_or_multi_line_captures_are_cut_in_messages() {
        assert_eq!(message_capture("short"), "short");
        assert_eq!(message_capture("f(\n  a,\n)"), "f(…");
        let long = "x".repeat(MESSAGE_CAPTURE_CHARS + 5);
        let cut = message_capture(&long);
        assert_eq!(cut.chars().count(), MESSAGE_CAPTURE_CHARS);
        assert!(cut.ends_with('…'));
        let exact = "y".repeat(MESSAGE_CAPTURE_CHARS);
        assert_eq!(message_capture(&exact), exact);
    }

    #[test]
    fn witness_start_and_end_are_zero_width_points() {
        let source = "let a = 1;\nconsole.log(a,\n  b);\n";
        let rule = json!({ "pattern": "console.log($$$ARGS)" });
        let span = only(
            &compile(ast(rule.clone()), violation("witness.span", "m")).unwrap(),
            source,
        );
        assert_eq!(point(&span["span"]), (1, 0, 2, 4));
        let start = only(
            &compile(ast(rule.clone()), violation("witness.start", "m")).unwrap(),
            source,
        );
        assert_eq!(point(&start["span"]), (1, 0, 1, 0));
        assert_eq!(start["span"]["startByte"], start["span"]["endByte"]);
        let end = only(
            &compile(ast(rule), violation("witness.end", "m")).unwrap(),
            source,
        );
        assert_eq!(point(&end["span"]), (2, 4, 2, 4));
        // Anchors never change violation identity: the id stays the witness's.
        assert_eq!(start["id"], span["id"]);
        assert_eq!(end["id"], span["id"]);
    }

    #[test]
    fn capture_anchors_point_at_the_capture() {
        let source = "foo(first, second, third);\n";
        let rule = || ast(json!({ "pattern": "foo($A, $$$REST)" }));
        let single = compile(rule(), violation("capture.A", "m")).unwrap();
        assert_eq!(point(&only(&single, source)["span"]), (0, 4, 0, 9));
        let multi = compile(rule(), violation("capture.REST", "m")).unwrap();
        assert_eq!(point(&only(&multi, source)["span"]), (0, 11, 0, 24));
        // A capture bound through a relation anchors on the related node's capture.
        let related = compile(
            ast(json!({
                "kind": "identifier",
                "regex": "^second$",
                "inside": { "pattern": "foo($$$ARGS)", "stopBy": "end" },
            })),
            violation("capture.ARGS", "m"),
        )
        .unwrap();
        assert_eq!(point(&only(&related, source)["span"]), (0, 4, 0, 24));
    }

    #[test]
    fn a_capture_absent_at_runtime_falls_back_to_the_witness_span() {
        let compiled = compile(
            ast(json!({ "any": [{ "pattern": "foo($A)" }, { "pattern": "bar()" }] })),
            violation("capture.A", "call of $A"),
        )
        .unwrap();
        let found = violations_for(&compiled, "typescript", "foo(x);\nbar();\n");
        assert_eq!(found.len(), 2);
        assert_eq!(point(&found[0]["span"]), (0, 4, 0, 5));
        assert_eq!(found[0]["variables"], json!({}));
        assert_eq!(found[0]["message"], "call of x");
        assert_eq!(point(&found[1]["span"]), (1, 0, 1, 5));
        assert_eq!(found[1]["variables"], json!({ "anchorFallback": true }));
        assert_eq!(found[1]["message"], "call of ");
    }

    #[test]
    fn messages_interpolate_single_multi_and_long_captures() {
        let compiled = compile(
            ast(json!({ "pattern": "$F($A, $$$REST)" })),
            violation("witness.span", "Call $F($A) with [$$$REST]"),
        )
        .unwrap();
        let found = only(&compiled, "run(first, a + b, c);\n");
        assert_eq!(found["message"], "Call run(first) with [a + b, c]");
        let found = only(&compiled, &format!("run({}, x);\n", "v".repeat(100)));
        let cut = "v".repeat(MESSAGE_CAPTURE_CHARS - 1);
        assert_eq!(found["message"], format!("Call run({cut}…) with [x]"));
        let bare = compile(
            ast(json!({ "pattern": "$F($$$ARGS)" })),
            violation("witness.span", "$F takes [$$$ARGS]"),
        )
        .unwrap();
        assert_eq!(only(&bare, "run();\n")["message"], "run takes []");
    }

    #[test]
    fn ast_fixes_substitute_captures_and_reindent() {
        let compiled = compile(
            ast(json!({ "pattern": "console.log($$$ARGS)" })),
            with_fix("witness.span", "logger.info($$$ARGS)"),
        )
        .unwrap();
        let found = only(
            &compiled,
            "function f() {\n  console.log(a, {\n    b,\n  });\n}\n",
        );
        assert_eq!(found["fix"]["text"], "logger.info(a, {\n    b,\n  })");
        assert_eq!(point(&found["fix"]["span"]), (1, 2, 3, 4));
        assert_eq!(found["fix"]["span"]["startByte"], 17);
        let block = compile(
            ast(json!({ "pattern": "console.log($A)" })),
            with_fix("witness.end", "if (debug) {\n  console.log($A);\n}"),
        )
        .unwrap();
        let found = only(&block, "function f() {\n  console.log(a);\n}\n");
        assert_eq!(
            found["fix"]["text"],
            "if (debug) {\n    console.log(a);\n  }"
        );
        assert_eq!(point(&found["span"]), (1, 16, 1, 16));

        // Each dialect compiles its own template: the same rule fixes JSX in a .tsx file.
        let jsx = compile(
            ast(json!({ "pattern": "<button $$$ATTRS>$$$BODY</button>" })),
            with_fix("witness.span", "<Button $$$ATTRS>$$$BODY</Button>"),
        )
        .unwrap();
        let found = violations_for(
            &jsx,
            "tsx",
            "const v = <button type=\"button\">Go</button>;\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0]["fix"]["text"],
            "<Button type=\"button\">Go</Button>"
        );
        assert!(violations_for(&jsx, "typescript", "const v = 1;\n").is_empty());
    }

    #[test]
    fn captures_bound_in_utilities_and_constraints_reach_violations() {
        // A `matches` target shares the caller's environment, so its captures anchor, interpolate,
        // and fix; a recursive utility is walked once.
        let util = || {
            json!({ "ast": {
                "language": "typescript",
                "rule": { "kind": "return_statement", "has": { "matches": "tail" } },
                "utils": { "tail": { "any": [
                    { "pattern": "await $E" },
                    { "kind": "parenthesized_expression", "has": { "matches": "tail" } },
                ] } },
            } })
        };
        let compiled = compile(
            util(),
            json!({ "anchor": "capture.E", "code": "X", "message": "awaits $E", "fix": "return $E;" }),
        )
        .unwrap();
        let found = only(
            &compiled,
            "async function f() {\n  return ((await g()));\n}\n",
        );
        assert_eq!(found["message"], "awaits g()");
        assert_eq!(point(&found["span"]), (1, 17, 1, 20));
        assert_eq!(found["fix"]["text"], "return g();");
        assert_eq!(point(&found["fix"]["span"]), (1, 2, 1, 23));

        // A constraint runs on the captured node with the same environment; what it binds counts.
        let constrained = || {
            json!({ "ast": {
                "language": "typescript",
                "rule": { "pattern": "$F($A)" },
                "constraints": { "A": { "pattern": "$L + $R" } },
            } })
        };
        let compiled = compile(
            constrained(),
            json!({ "anchor": "capture.R", "code": "X", "message": "$F adds $L and $R", "fix": "$F(`${$L}${$R}`)" }),
        )
        .unwrap();
        let found = only(&compiled, "log(a + b);\nlog(c);\n");
        assert_eq!(found["message"], "log adds a and b");
        assert_eq!(point(&found["span"]), (0, 8, 0, 9));
        assert_eq!(found["fix"]["text"], "log(`${a}${b}`)");

        // Names bound only under `not`, in a utility or a constraint, never exist.
        for (forbid, name, bindable) in [
            (
                json!({ "ast": {
                    "language": "typescript",
                    "rule": { "matches": "call" },
                    "utils": { "call": { "kind": "call_expression", "not": { "pattern": "bar($N)" } } },
                } }),
                "N",
                "the query binds no captures",
            ),
            (
                json!({ "ast": {
                    "language": "typescript",
                    "rule": { "pattern": "$F($A)" },
                    "constraints": { "A": { "kind": "identifier", "not": { "pattern": "$M.x" } } },
                } }),
                "M",
                "the query can bind: A, F",
            ),
        ] {
            assert_eq!(
                rejection(compile(forbid, violation(&format!("capture.{name}"), "m"))),
                format!(
                    "/clauses/0/violation/anchor: `capture.{name}` names no capture; {bindable}"
                )
            );
        }
    }

    #[test]
    fn regex_named_groups_are_captures() {
        let pattern = "(?<KEY>[a-z]+)=(?<VALUE>\\d+)(?<unit>ms)?";
        let regex = || json!({ "regex": { "source": "file.content", "pattern": pattern } });
        let compiled = compile(
            regex(),
            json!({ "anchor": "capture.VALUE", "code": "X", "message": "set $KEY to $VALUE", "fix": "$KEY: $VALUE" }),
        )
        .unwrap();
        let found = only(&compiled, "  width=10\n");
        assert_eq!(point(&found["span"]), (0, 8, 0, 10));
        assert_eq!(found["message"], "set width to 10");
        assert_eq!(found["fix"]["text"], "width: 10");
        assert_eq!(point(&found["fix"]["span"]), (0, 2, 0, 10));
        let captures = found["captures"].as_object().expect("captures");
        assert_eq!(captures.keys().collect::<Vec<_>>(), ["KEY", "VALUE"]);
        let unit = compile(regex(), violation("capture.unit", "m")).unwrap();
        let found = violations_for(&unit, "typescript", "a=1ms b=2\n");
        assert_eq!(point(&found[0]["span"]), (0, 3, 0, 5));
        assert_eq!(found[1]["variables"], json!({ "anchorFallback": true }));
    }

    #[test]
    fn named_groups_in_a_same_node_regex_capture_sub_spans() {
        let comment = compile(
            ast(json!({ "kind": "comment", "any": [
                { "regex": "^//+(?<BODY>[^\\s/].*)$" },
                { "regex": "^/\\*\\*?\\s*(?<BODY>.*?)\\s*\\*/$" },
            ] })),
            violation("capture.BODY", "comment `$BODY`"),
        )
        .unwrap();
        let found = violations_for(&comment, "typescript", "x(); ////Tight\n/** doc */\n");
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(point(&found[0]["span"]), (0, 9, 0, 14));
        assert_eq!(found[0]["message"], "comment `Tight`");
        assert_eq!(point(&found[1]["span"]), (1, 4, 1, 7));
        assert_eq!(found[1]["captures"]["BODY"][0]["startByte"], 19);
        // A metavariable of the same name wins over the group.
        let shadowed = compile(
            ast(json!({ "pattern": "foo($A)", "regex": "^(?<A>f)oo" })),
            violation("capture.A", "$A"),
        )
        .unwrap();
        let found = only(&shadowed, "foo(bar);\n");
        assert_eq!(point(&found["span"]), (0, 4, 0, 7));
        assert_eq!(found["message"], "bar");
        // Groups of a regex that tests another node (a relation or `not`) are not captures.
        for rule in [
            json!({ "kind": "identifier", "inside": { "kind": "call_expression", "regex": "^(?<G>f)" } }),
            json!({ "kind": "identifier", "not": { "regex": "^(?<G>f)" } }),
        ] {
            let reason = rejection(compile(ast(rule), violation("capture.G", "m")));
            assert_eq!(
                reason,
                "/clauses/0/violation/anchor: `capture.G` names no capture; the query binds no captures"
            );
        }
        let reason = rejection(compile(
            ast(json!({ "kind": "comment", "regex": "^//(?<B>.*)" })),
            with_fix("witness.span", "// $B"),
        ));
        assert_eq!(
            reason,
            "/clauses/0/violation/fix: `$B` is a regex group inside an ast node; an ast fix can use only metavariables"
        );
    }

    #[test]
    fn select_when_reports_the_select_side_captures() {
        let compiled = compile(
            json!({ "select_when": {
                "select": ast(json!({ "pattern": "foo($A)" })),
                "exists": { "regex": { "source": "file.content", "pattern": "(?<MARK>@legacy)" } },
            } }),
            violation("capture.A", "foo of $A"),
        )
        .unwrap();
        let found = only(&compiled, "// @legacy\nfoo(bar);\n");
        assert_eq!(found["message"], "foo of bar");
        assert_eq!(point(&found["span"]), (1, 4, 1, 7));
    }

    #[test]
    fn anchors_messages_and_fixes_are_checked_against_the_query() {
        let pattern = || ast(json!({ "pattern": "foo($A)", "not": { "pattern": "foo($B)" } }));
        for (forbid, violation, expected) in [
            (
                pattern(),
                violation("subject.span", "m"),
                "/clauses/0/violation/anchor: unknown anchor `subject.span`; expected witness.span, witness.start, witness.end, file.start, or capture.<NAME>",
            ),
            (
                pattern(),
                violation("capture.B", "m"),
                "/clauses/0/violation/anchor: `capture.B` names no capture; the query can bind: A",
            ),
            (
                pattern(),
                violation("witness.span", "bad $NAME"),
                "/clauses/0/violation/message: `$NAME` names no capture; the query can bind: A; write `\\$NAME` for literal text",
            ),
            (
                json!({ "regex": { "source": "file.content", "pattern": "x" } }),
                violation("capture.X", "m"),
                "/clauses/0/violation/anchor: `capture.X` names no capture; the query binds no captures",
            ),
            (
                json!({ "regex": { "source": "file.content", "pattern": "(?<X>x)" } }),
                violation("witness.span", "$$$X"),
                "/clauses/0/violation/message: `$$$X`: regex groups are single captures; use `$X`",
            ),
            (
                json!({ "regex": { "source": "file.path", "pattern": "(?<X>x)" } }),
                with_fix("capture.X", "y"),
                "/clauses/0/violation/fix: a file.path regex match is not source text a fix can replace",
            ),
            (
                pattern(),
                json!({ "anchor": "witness.span", "code": "X", "message": "m", "fix": ["x"] }),
                "/clauses/0/violation/fix must be an ast-grep fix template string or { template, range }",
            ),
            (
                pattern(),
                with_fix("witness.span", "bar($B)"),
                "/clauses/0/violation/fix: `$B` names no capture; the query can bind: A",
            ),
        ] {
            assert_eq!(rejection(compile(forbid, violation)), expected);
        }
        for removed in [
            json!({ "regex": { "source": "file.content", "pattern": "(x)", "captures": ["x"] } }),
            json!({ "ast": { "language": "typescript", "rule": { "pattern": "f($A)" }, "capture": "A" } }),
        ] {
            let reason = rejection(compile(removed, violation("witness.span", "m")));
            assert!(reason.starts_with("unknown field `capture"), "{reason}");
        }
    }

    fn ranged(fix: Value) -> Value {
        json!({ "anchor": "witness.span", "code": "X", "message": "m", "fix": fix })
    }

    #[test]
    fn a_fix_range_replaces_a_capture_inside_the_witness() {
        // no-div-regex: ESLint replaces the `=` after the opening slash with `[=]`.
        let compiled = compile(
            ast(json!({ "kind": "regex", "regex": "^/(?<EQ>=)" })),
            ranged(json!({ "template": "[=]", "range": "capture.EQ" })),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "const r = /=foo/;\n";
        let fix = &only(&compiled, source)["fix"];
        assert_eq!(fix["text"], "[=]");
        assert_eq!(
            (
                fix["span"]["startByte"].as_u64(),
                fix["span"]["endByte"].as_u64()
            ),
            (Some(11), Some(12))
        );
        // A metavariable and a transform of it; `witness.span` is the whole witness.
        let compiled = compile(
            json!({ "ast": {
                "language": "typescript",
                "rule": { "pattern": "f($A, $B)" },
                "transform": { "U": { "convert": { "source": "$A", "toCase": "upperCase" } } },
            } }),
            ranged(json!({ "template": "$U", "range": "capture.A" })),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let fix = &only(&compiled, "f(abc, d);\n")["fix"];
        assert_eq!(fix["text"], "ABC");
        assert_eq!(fix["span"]["startByte"], 2);
        let compiled = compile(
            ast(json!({ "pattern": "f($$$A)" })),
            ranged(json!({ "template": "[$$$A]", "range": "capture.A" })),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let fix = &only(&compiled, "f(a, b);\n")["fix"];
        assert_eq!(
            (
                fix["text"].as_str(),
                fix["span"]["startByte"].as_u64(),
                fix["span"]["endByte"].as_u64()
            ),
            (Some("[a, b]"), Some(2), Some(6))
        );
        // A capture outside the witness gets no fix; the violation stays.
        let compiled = compile(
            ast(json!({ "kind": "identifier", "regex": "^a$", "inside": { "stopBy": "end", "pattern": "$P", "kind": "call_expression" } })),
            ranged(json!({ "template": "x", "range": "capture.P" })),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(only(&compiled, "f(a);\n").get("fix"), None);
        for (fix, expected) in [
            (
                json!({ "template": "x", "range": "capture.Z" }),
                "/clauses/0/violation/fix/range: `capture.Z` names no capture; the query can bind: A",
            ),
            (
                json!({ "template": "x", "range": "witness.start" }),
                "/clauses/0/violation/fix/range: unknown range `witness.start`; expected witness.span or capture.<NAME>",
            ),
            (
                json!({ "template": "x" }),
                "/clauses/0/violation/fix/range is required: witness.span or capture.<NAME>",
            ),
            (
                json!({ "template": 1, "range": "capture.A" }),
                "/clauses/0/violation/fix/template must be an ast-grep fix template string",
            ),
            (
                json!({ "template": "x", "range": "capture.A", "at": 1 }),
                "unknown field `at` at /clauses/0/violation/fix",
            ),
        ] {
            assert_eq!(
                rejection(compile(ast(json!({ "pattern": "f($A)" })), ranged(fix))),
                expected
            );
        }
        let regex = json!({ "regex": { "source": "file.content", "pattern": "(?<X>a)" } });
        assert_eq!(
            rejection(compile(
                regex,
                ranged(json!({ "template": "b", "range": "capture.X" }))
            )),
            "/clauses/0/violation/fix/range: a fix range names a capture of an ast query"
        );
    }

    /// Capture names and witness regexes summarize each utility once and map the summary at each
    /// call, so a diamond chain costs each utility a constant number of expansions.
    #[test]
    fn parameterized_utility_diamonds_summarize_violations_in_linear_visits() {
        use crate::code_rule_dsl::test_support::{
            DIAMOND_DEPTH, Walk, compiled_parameterized_utility_diamonds, grammar_matchers,
            linear_walk, reset_walks, walks,
        };
        let compiled = compiled_parameterized_utility_diamonds(DIAMOND_DEPTH);
        let grammar = &grammar_matchers(&compiled)[0];
        let bound = linear_walk(DIAMOND_DEPTH);
        reset_walks();
        let mut names = BTreeSet::new();
        ast_captures(
            &grammar.matcher,
            &grammar.utils,
            &mut UtilitySummary::new(),
            &mut names,
        );
        assert!(names.contains("X"), "{names:?}");
        assert!(
            walks(Walk::AstCaptures) <= bound,
            "capture names expanded {} utilities (bound {bound})",
            walks(Walk::AstCaptures)
        );
        witness_groups(&grammar.matcher, &grammar.utils);
        assert!(
            walks(Walk::NodeRegexes) <= bound,
            "witness regexes expanded {} utilities (bound {bound})",
            walks(Walk::NodeRegexes)
        );
    }

    /// A utility reached along many paths tests the witness once: its regex describes it once.
    #[test]
    fn utility_diamonds_contribute_each_witness_regex_once() {
        use crate::code_rule_dsl::test_support::{compiled, grammar_matchers, utility_diamonds};
        let mut utils = utility_diamonds(12);
        utils["u0"] = json!({ "kind": "identifier", "regex": "^(?<N>x)$" });
        let compiled = compiled(json!({ "rule": { "matches": "u12" }, "utils": utils }));
        let grammar = &grammar_matchers(&compiled)[0];
        assert_eq!(witness_regexes(&grammar.matcher, &grammar.utils).len(), 1);
        assert_eq!(witness_groups(&grammar.matcher, &grammar.utils), ["N"]);
    }

    /// Compiling a query collects capture names from its rule and every shared utility with one
    /// summary per utility, so doubling a utility chain at most doubles that work.
    #[test]
    fn capture_names_of_a_compiled_utility_chain_grow_linearly() {
        use crate::code_rule_dsl::test_support::{
            Walk, compiled_utility_diamonds, reset_walks, walks,
        };
        let expansions = |depth| {
            reset_walks();
            compiled_utility_diamonds(depth);
            walks(Walk::AstCaptures)
        };
        let (half, full) = (expansions(6), expansions(12));
        assert!(
            full <= 2 * half,
            "capture names expanded {half} utilities at depth 6, {full} at depth 12"
        );
    }

    /// `W` binds its parameter through `V`, which binds its own through `W` or an outward relation:
    /// the violation may anchor on what `W` binds whichever `any` branch the walk meets first.
    #[test]
    fn captures_bound_through_recursive_calls_do_not_depend_on_branch_order() {
        use crate::code_rule_dsl::test_support::{
            mutually_recursive_calls, recursive_call_branch_orders,
        };
        for branches in recursive_call_branch_orders() {
            let forbid = json!({ "ast": { "language": "typescript",
                "rule": { "kind": "expression_statement", "any": branches },
                "utils": mutually_recursive_calls() } });
            if let Err(error) = compile(forbid, violation("capture.X", "x")) {
                panic!("{branches}: {}", error.reason);
            }
        }
    }
}

#[cfg(test)]
mod each_limit_tests {
    use super::*;
    use crate::code_rule_dsl::CodeRuleDslRuntime;

    /// Evaluates `source` as `a.ts` under the one clause `clause` of a code rule `bounded-each`,
    /// with a one-witness allowance.
    fn evaluate_bounded(clause: Value, source: &str) -> Value {
        let mut capabilities: Value =
            serde_json::from_str(include_str!("capabilities.json")).expect("capabilities");
        capabilities["limits"]["maxWitnesses"] = json!(1);
        let compiled = CodeRuleDslRuntime::compile(
            json!({
                "format": "rerail.policy/v1",
                "id": "bounded-each",
                "revision": 1,
                "mode": "steer",
                "clauses": [clause]
            })
            .to_string(),
            "json".to_owned(),
            capabilities.to_string(),
        )
        .expect("policy compiles");
        let output = compiled
            .evaluate(
                json!({
                    "format": "rerail.repository-snapshot/v1",
                    "revision": 1,
                    "facts": [],
                    "files": [{
                        "path": "a.ts",
                        "language": "typescript",
                        "source": source,
                    }]
                })
                .to_string(),
                "report".to_owned(),
            )
            .expect("evaluation succeeds");
        serde_json::from_str(&output).expect("evaluation JSON")
    }

    /// One witness per `a` of a comment, with `suppress` where given.
    fn comment_characters(suppress: Option<Value>) -> Value {
        let mut violation = json!({
            "anchor": "capture.X",
            "code": "E",
            "message": "$X",
            "each": "X"
        });
        if let Some(suppress) = suppress {
            violation["suppress"] = suppress;
        }
        json!({
            "id": "comment-characters",
            "forbid": { "ast": { "language": "typescript", "rule": {
                "kind": "comment",
                "regex": "(?<X>a)"
            } } },
            "violation": violation
        })
    }

    #[test]
    fn same_node_regex_each_stops_after_the_witness_overflow_sentinel() {
        let source = format!("//{}\n", "a".repeat(50_000));
        reset_node_group_enumerations();
        let result = evaluate_bounded(comment_characters(None), &source);
        assert_eq!(node_group_enumerations(), 2, "{result}");
        assert_eq!(result["metrics"]["witnesses"], 2, "{result}");
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(result["failures"][0]["code"], "witness-limit", "{result}");
    }

    /// A violation whose `suppress` a comment may answer reads past suppressed witnesses, yet
    /// still stops once the unsuppressed ones overflow the allowance: none of these 50,000 groups
    /// is suppressed, so the second ends the enumeration.
    #[test]
    fn suppressible_same_node_regex_each_stops_after_the_witness_overflow_sentinel() {
        let source = format!("//{}\n", "a".repeat(50_000));
        let suppress = json!({ "eslint": { "rules": ["no-warning-comments"] } });
        reset_node_group_enumerations();
        let result = evaluate_bounded(comment_characters(Some(suppress)), &source);
        assert_eq!(node_group_enumerations(), 2, "{result}");
        assert_eq!(result["metrics"]["witnesses"], 2, "{result}");
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(result["failures"][0]["code"], "witness-limit", "{result}");
    }

    /// A binding a comment suppresses spends none of the one-witness allowance: the enumeration
    /// reads past two suppressed bindings to the third, which is reported, whether a
    /// `rerail-ignore` or a linter directive the violation opts into silences them.
    #[test]
    fn each_reads_past_suppressed_bindings_under_a_one_witness_allowance() {
        for (directive, suppress) in [
            ("// rerail-ignore bounded-each", None),
            (
                "// eslint-disable-line no-console",
                Some(json!({ "eslint": { "rules": ["no-console"] } })),
            ),
        ] {
            let mut violation = json!({
                "anchor": "capture.X",
                "code": "E",
                "message": "statement",
                "each": "X"
            });
            if let Some(suppress) = suppress {
                violation["suppress"] = suppress;
            }
            // Statements only: `$X` alone also binds the comments, which no directive covers.
            let clause = json!({
                "id": "statement",
                "forbid": { "ast": { "language": "typescript", "rule": {
                    "kind": "program",
                    "has": { "kind": "expression_statement", "pattern": "$X" }
                } } },
                "violation": violation
            });
            let source = format!("first; {directive}\nsecond; {directive}\nthird;\n");
            let result = evaluate_bounded(clause, &source);
            assert_eq!(result["failures"], json!([]), "{directive}: {result}");
            let reported = result["violations"]
                .as_array()
                .expect("violations")
                .iter()
                .map(|violation| {
                    let start = violation["span"]["startByte"].as_u64().expect("start");
                    let end = violation["span"]["endByte"].as_u64().expect("end");
                    &source[start as usize..end as usize]
                })
                .collect::<Vec<_>>();
            assert_eq!(reported, ["third;"], "{directive}: {result}");
        }
    }
}
