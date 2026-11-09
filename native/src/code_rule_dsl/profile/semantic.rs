use super::*;
use crate::code_rule_dsl::matcher::Semantic;
use serde_json::json;

pub(super) fn describe_matcher(semantic: &Semantic, language: GrammarDialect) -> Value {
    match semantic {
        Semantic::Value(test) => json!({ "value": {
            "types": &test.types, "eq": test.eq.is_some(), "constant": test.constant,
            "truthy": test.truthy, "follow": test.reading.follow,
        } }),
        Semantic::Arguments(arguments) => json!({ "arguments": {
            "slots": arguments.slots.len(), "bind": arguments.bind.iter().map(|(name, _)| name).collect::<Vec<_>>(), "absent": arguments.absent.len(),
        } }),
        Semantic::Parse(test) => {
            json!({ "parse": { "grammar": test.grammar.name(), "exports": &test.exports } })
        }
        Semantic::Host(test) => json!({ "host": super::describe_matcher(&test.rule, language) }),
        Semantic::Embedded(test) => json!({ "embedded": {
            "via": test.via.id(),
            "rules": test.rules.iter().map(|(dialect, rule)| super::describe_matcher(rule, *dialect)).collect::<Vec<_>>(),
            "exports": &test.exports,
        } }),
        Semantic::Capture(name) => json!({ "capture": name }),
        Semantic::FormatArgs(test) => json!({ "formatArgs": {
            "style": format!("{:?}", test.style), "string": &test.string, "args": &test.args,
            "unused": test.unused.as_ref().map(|rule| super::describe_matcher(rule, language)), "missing": test.missing.is_some(),
        } }),
        Semantic::Binding(test) => json!({ "resolves": { "write": test.write_last, "binding": {
            "filter": format!("{:?}", test.filter), "site": test.site.as_ref().map(|rule| super::describe_matcher(rule, language)),
            "value": test.value.as_ref().map(|rule| super::describe_matcher(rule, language)), "annotation": test.annotation.as_ref().map(|rule| super::describe_matcher(rule, language)),
            "occurrence": test.occurrence.as_ref().map(|rule| super::describe_matcher(rule, language)),
        } } }),
        Semantic::Binds(test) => json!({ "binds": {
            "filter": format!("{:?}", test.filter), "redefines": test.redefines.as_ref().map(|redefines| json!({
                "filter": format!("{:?}", redefines.filter), "sameScope": redefines.same_scope, "sameImport": redefines.same_import,
                "site": redefines.site.as_ref().map(|rule| super::describe_matcher(rule, language)),
            })), "site": test.site.as_ref().map(|rule| super::describe_matcher(rule, language)),
            "value": test.value.as_ref().map(|rule| super::describe_matcher(rule, language)), "annotation": test.annotation.as_ref().map(|rule| super::describe_matcher(rule, language)),
            "occurrence": test.occurrence.as_ref().map(|rule| super::describe_matcher(rule, language)),
        } }),
        Semantic::Resolves(resolves) => json!({ "resolves": {
            "global": &resolves.globals, "import": &resolves.imports, "directives": resolves.directives, "this": resolves.this_global,
            "imported": &resolves.imported, "prefix": resolves.prefix, "globalAny": resolves.global_any, "importAny": resolves.import_any,
            "unresolved": resolves.unresolved.map(|mode| format!("{mode:?}")), "beforeLocal": resolves.before_local,
            "seenBefore": resolves.seen_before, "languageOptions": resolves.options.is_some(),
        } }),
        Semantic::Layout(layout) => json!({ "layout": format!("{layout:?}") }),
        Semantic::Calls(_) => json!({ "calls": true }),
        Semantic::ImportTarget(test) => json!({ "importTarget": {
            "via": format!("{:?}", test.via),
            "moduleType": test.module_types.as_ref().map(|types| types.iter().map(|kind| format!("{kind:?}")).collect::<Vec<_>>()),
            "builtin": test.builtin, "resolved": test.resolved, "failed": test.failed,
            "extensionSpelled": test.extension_spelled, "published": test.published,
            "devDependency": test.dev_dependency, "declared": test.declared, "aliased": test.aliased,
        } }),
    }
}
