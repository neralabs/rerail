//! `define` clauses (a Rerail extension): definitions a document shares among its clauses. A define
//! reports nothing. Its `utils` are visible to every ast query of its `language`, as if each query
//! that names one listed it in its own `utils`: the query compiles it with its own options.

use super::ast_rule::{compile_ast_rule, validate_ast_rule};
use super::config::Options;
use super::dialect::language_capability;
use super::documents::{DocumentScopes, Side};
use super::refuse_unavailable_semantics;
use super::shared::SharedTables;
use super::utils::{UtilDef, UtilIds, UtilScope, reject_util_cycles, util_map};
use crate::code_rule_dsl::json::{invalid, object, reject_unknown, required_str};
use crate::code_rule_dsl::language::{CodeRuleLanguage, GrammarDialect};
use crate::code_rule_dsl::matcher::{AstMatcher, UtilTable};
use crate::code_rule_dsl::model::Capabilities;
use napi::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Constraint forms, including the compatibility surface used by recorded data-layer policies.
pub(super) const CLAUSE_KINDS: [&str; 5] = ["forbid", "require_each", "budget", "relational", "llm"];

/// Each language's shared utilities, by id.
pub(super) type SharedUtils<'a> = BTreeMap<String, BTreeMap<String, UtilDef<'a>>>;

/// What the document's defines hold: shared utilities and options.
pub(in crate::code_rule_dsl) struct Defines<'a> {
    pub(super) utils: &'a SharedUtils<'a>,
    pub(super) options: Options,
    /// The shared utilities' tables, which the queries that reach them fill.
    pub(super) tables: SharedTables<'a>,
}

/// Whether a clause is a define.
pub(super) fn is_define(clause: &Value) -> bool {
    clause.get("define").is_some()
}

/// Reads and validates the document's defines. Their utilities are checked as a query's are: shape,
/// cycles, and whole in some dialect of their language, used or not.
pub(super) fn collect_defines<'a>(
    clauses: &'a [Value],
    capabilities: &Capabilities,
) -> Result<SharedUtils<'a>> {
    let mut shared = SharedUtils::new();
    for (index, clause) in clauses.iter().enumerate() {
        let Some(define) = clause.get("define") else {
            continue;
        };
        let path = format!("/clauses/{index}");
        let fields = object(clause, &path)?;
        if fields.contains_key("violation") {
            return Err(invalid(format!(
                "{path}/violation: a define clause reports nothing, so it has no violation"
            )));
        }
        if CLAUSE_KINDS.iter().any(|form| fields.contains_key(*form)) {
            return Err(invalid(format!(
                "{path} must contain exactly one constraint form"
            )));
        }
        reject_unknown(fields, &["id", "define"], &path)?;
        required_str(clause, "id", &path)?;
        let define_path = format!("{path}/define");
        let body = object(define, &define_path)?;
        reject_unknown(
            body,
            &["language", "utils", "options", "config", "report_when"],
            &define_path,
        )?;
        let language = body
            .get("language")
            .map(|language| {
                let unknown = || {
                    invalid(format!(
                        "{define_path}/language must equal python, typescript, javascript or php"
                    ))
                };
                let name = language.as_str().ok_or_else(unknown)?;
                CodeRuleLanguage::read(name, &format!("{define_path}/language"), unknown)
                    .map(CodeRuleLanguage::name)
            })
            .transpose()?;
        // Options and their configuration were read before the document was resolved.
        let Some(utils) = body.get("utils") else {
            if body.contains_key("options")
                || body.contains_key("config")
                || body.contains_key("report_when")
            {
                continue;
            }
            return Err(invalid(format!(
                "{define_path} must hold utils, options, config or report_when"
            )));
        };
        let Some(language) = language else {
            return Err(invalid(format!(
                "{define_path}/language is required: utilities compile for one language"
            )));
        };
        let known = shared.entry(language.to_owned()).or_default();
        for (id, util) in util_map(Some(utils), &format!("{define_path}/utils"))? {
            if known.contains_key(&id) {
                return Err(invalid(format!(
                    "{}: utility rule `{id}` is already defined by another define of {language}",
                    util.path
                )));
            }
            known.insert(id, util);
        }
    }
    for (language, utils) in &shared {
        let ids = utils
            .iter()
            .map(|(id, util)| (id.as_str(), util.params.as_slice()))
            .collect::<UtilIds>();
        let code_rule_language =
            CodeRuleLanguage::parse(language).expect("a define's language was parsed above");
        for util in utils.values() {
            refuse_unavailable_semantics(util.rule, &util.rule_path, code_rule_language)?;
            validate_ast_rule(util.rule, &util.rule_path, &capabilities.limits, &ids)?;
        }
        reject_util_cycles(utils)?;
        check_dialects(language, utils, &ids, capabilities)?;
    }
    Ok(shared)
}

/// Compiles the shared utilities of one language for each of its dialects, and for an embedding
/// language each dialect of the program its files embed (`embed`): each must be whole in at least
/// one, as a query's rule must. A query reaches a utility from either document.
fn check_dialects<'u>(
    language: &str,
    utils: &'u BTreeMap<String, UtilDef<'u>>,
    ids: &UtilIds,
    capabilities: &Capabilities,
) -> Result<()> {
    let (grammars, _) = language_capability(language, capabilities)?;
    let embedding = super::Embedding::of(&grammars);
    let sides = grammars
        .iter()
        .map(|&grammar| (grammar, Side::Host))
        .chain(
            embedding
                .iter()
                .flat_map(|embedding| embedding.guests.iter().map(|&guest| (guest, Side::Guest))),
        )
        .collect::<Vec<_>>();
    let mut rejections = BTreeMap::<&str, Vec<(String, &str)>>::new();
    let mut whole = BTreeSet::new();
    // An `all` ordering conflict depends on what the callers bind; the queries that call a shared
    // utility settle it there.
    let ignored = std::cell::RefCell::new(BTreeSet::new());
    // A shared utility's `calls` terms are the calling queries' (their seeds carry its rule).
    let calls = crate::code_rule_dsl::calls::CallsCollector::new(
        String::new(),
        false,
        capabilities.limits.max_regex_pattern_bytes,
    );
    let new_table = || {
        Arc::new(UtilTable::new(
            utils.keys().cloned().collect(),
            utils.values().map(|util| !util.params.is_empty()).collect(),
            vec![None; utils.len()],
        ))
    };
    for ((dialect, grammar), side) in &sides {
        let table = new_table();
        // The other document's rules (`embedded`, `host`) compile against tables of their own,
        // which this check does not fill: it asks only whether each utility is whole here.
        let others = embedding
            .as_ref()
            .map_or_else(Vec::new, |embedding| match side {
                Side::Host => embedding
                    .guests
                    .iter()
                    .map(|&(_, guest)| (guest, new_table()))
                    .collect(),
                Side::Guest => vec![(embedding.host.1, new_table())],
            });
        let weak =
            |dialect: GrammarDialect, table: &Arc<UtilTable>| (dialect, Arc::downgrade(table));
        let documents = embedding.as_ref().map(|embedding| {
            let own = weak(*grammar, &table);
            let others = others.iter().map(|(dialect, table)| weak(*dialect, table));
            match side {
                Side::Host => DocumentScopes {
                    via: embedding.via,
                    host: own,
                    guests: others.collect(),
                },
                Side::Guest => DocumentScopes {
                    via: embedding.via,
                    host: others.into_iter().next().expect("one host dialect"),
                    guests: vec![own],
                },
            }
        });
        let scope = UtilScope {
            table: Arc::downgrade(&table),
            ids,
            options: None,
            python: None,
            transform: None,
            conflicts: Some(&ignored),
            captures: Default::default(),
            calls: Some(&calls),
            documents: documents.as_ref(),
            instances: None,
            side: *side,
        };
        let compile = |gaps: &mut BTreeMap<&'u str, Vec<String>>| {
            utils
                .iter()
                .map(|(id, util)| {
                    let gaps = gaps.entry(id.as_str()).or_default();
                    match compile_ast_rule(util.rule, *grammar, &util.rule_path, gaps, &scope) {
                        // A utility of a language that embeds a program may be written for the
                        // other document: what this one refuses (a `parse` its grammar hosts no
                        // literal for) is a gap here, as an unknown kind is.
                        Err(error) if embedding.is_some() => {
                            gaps.push(error.reason);
                            Ok(AstMatcher::Never)
                        }
                        compiled => compiled,
                    }
                })
                .collect::<Result<Vec<_>>>()
        };
        let mut gaps = BTreeMap::new();
        let compiled = compile(&mut gaps)?;
        if table.was_unfilled() {
            table.draft_all(compiled);
            gaps.clear();
            compile(&mut gaps)?;
        }
        for (id, gaps) in gaps {
            match gaps.into_iter().next() {
                Some(reason) => rejections.entry(id).or_default().push((reason, *dialect)),
                None => {
                    whole.insert(id);
                }
            }
        }
    }
    let Some((_, rejections)) = rejections
        .into_iter()
        .find(|(id, reasons)| !whole.contains(id) && !reasons.is_empty())
    else {
        return Ok(());
    };
    let (first, _) = &rejections[0];
    let message = if rejections.iter().all(|(reason, _)| reason == first) {
        let dialects = rejections.iter().map(|(_, dialect)| *dialect);
        let dialects = dialects.collect::<Vec<_>>().join(", ");
        let plural = if rejections.len() > 1 { "s" } else { "" };
        format!("{first} ({dialects} grammar{plural})")
    } else {
        let each = rejections
            .iter()
            .map(|(reason, dialect)| format!("{reason} ({dialect} grammar)"));
        each.collect::<Vec<_>>().join("; ")
    };
    Err(invalid(message))
}

// cratify: `tests` is rerail-engine's (it mounts it)
