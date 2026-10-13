//! The view oracle (`fixtures/vue/language/view/view.expected.json`, recorded with
//! @vue/compiler-sfc and @vue/compiler-dom 3.5.43): blocks, template regions, and the
//! references and declarations Vue resolves, read off the view's program.
use super::view::View;
use super::{sfc, template};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::scope::{Binding, ScopeTable};
use ast_grep_language::LanguageExt;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;

/// The view oracle's directory.
fn root() -> PathBuf {
    PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("vue/language/view")
}

/// The oracle fixture `path` and its tree-sitter-vue tree.
fn parse(path: &str) -> (String, tree_sitter::Tree) {
    let source = std::fs::read_to_string(root().join("fixtures").join(path)).expect("fixture");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Vue.get_ts_language())
        .expect("grammar");
    let tree = parser.parse(&source, None).expect("tree");
    (source, tree)
}

/// The oracle's name for how Vue parses a region.
fn parse_name(p: &template::Parse) -> &'static str {
    match p {
        template::Parse::Expression => "expression",
        template::Parse::Params => "params",
        template::Parse::HandlerMember => "handler-member",
        template::Parse::HandlerFunction => "handler-function",
        template::Parse::Statements => "statements",
    }
}

/// compiler-core 3.5.43's `indexOf(RHS, LHS.length)` can locate the `n` in `in` for `m in n`
/// (the oracle's `for-source` region and the reference in it); the view deliberately maps real
/// bytes. Each quirk: file, region kind, the oracle's range.
const QUIRKS: &[(&str, &str, usize, usize)] = &[("slots-and-scopes.vue", "for-source", 588, 589)];

/// The range the oracle puts at `start..end` in `path`, where a quirk misplaces it: the real one,
/// the next region of the quirk's kind.
fn unquirked(path: &str, start: usize, end: usize, regions: &[template::Region]) -> Range<usize> {
    QUIRKS
        .iter()
        .find(|&&(qp, _, qs, qe)| (qp, qs, qe) == (path, start, end))
        .and_then(|&(_, kind, _, _)| {
            regions
                .iter()
                .find(|region| region.kind == kind && region.range.start > start)
        })
        .map_or(start..end, |region| region.range.clone())
}

/// The oracle's `blocks` and the `regions` of each HTML template.
#[test]
fn vue_view_oracle() {
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(root().join("view.expected.json")).expect("oracle"),
    )
    .expect("json");
    assert_eq!(expected["tools"][0]["version"], "3.5.43");
    let mut errors = Vec::new();
    let mut quirks = 0;
    for file in expected["files"].as_array().expect("files") {
        let path = file["path"].as_str().expect("path");
        let (source, tree) = parse(path);
        let blocks = sfc::blocks(tree.root_node(), &source);
        let got: Vec<Value> = blocks
            .iter()
            .map(|block| {
                let mut value = json!({
                    "type": block.name,
                    "lang": block.lang,
                    "start": block.content.start,
                    "end": block.content.end,
                });
                if block.setup {
                    value["setup"] = json!(true);
                }
                if let Some(src) = block.src {
                    value["src"] = json!(src);
                }
                value
            })
            .collect();
        if got != *file["blocks"].as_array().expect("blocks") {
            errors.push(format!(
                "{path}: blocks got {got:?} expected {}",
                file["blocks"]
            ));
        }
        let Some(template) = blocks
            .iter()
            .find(|block| block.name == "template" && block.lang.is_none_or(|lang| lang == "html"))
        else {
            continue;
        };
        let regions = template::regions(template.node, &source, GrammarDialect::JavaScript);
        let got: Vec<Value> = regions
            .iter()
            .map(|region| {
                json!({
                    "kind": region.kind,
                    "start": region.range.start,
                    "end": region.range.end,
                    "parse": parse_name(&region.parse),
                })
            })
            .collect();
        let expected: Vec<Value> = file["regions"]
            .as_array()
            .expect("regions")
            .iter()
            .map(|region| {
                let start = region["start"].as_u64().expect("start") as usize;
                let end = region["end"].as_u64().expect("end") as usize;
                let range = unquirked(path, start, end, &regions);
                quirks += usize::from(range != (start..end));
                json!({
                    "kind": region["kind"],
                    "start": range.start,
                    "end": range.end,
                    "parse": region["parse"],
                })
            })
            .collect();
        if got != expected {
            errors.push(format!("{path}: regions got {got:?} expected {expected:?}"));
        }
    }
    assert_eq!(quirks, QUIRKS.len(), "stale Vue oracle quirks");
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// The innermost program node spanning exactly program bytes `start..end`.
fn program_node(view: &View, start: usize, end: usize) -> Option<tree_sitter::Node<'_>> {
    view.root()
        .descendant_for_byte_range(start, end)
        .filter(|node| node.byte_range() == (start..end))
}

/// The identifiers of the view's program, by the `.vue` bytes they stand for.
fn identifiers(view: &View) -> HashMap<Range<usize>, Range<usize>> {
    let mut out = HashMap::new();
    let mut stack = vec![view.root()];
    while let Some(node) = stack.pop() {
        if node.kind().contains("identifier") {
            out.entry(view.span(node)).or_insert(node.byte_range());
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    out
}

/// Whether the program identifier at `range` is read in an inline `v-on` handler's arrow: its
/// innermost arrow stands for an `@x` or `v-on:x` attribute.
fn in_inline_handler(view: &View, source: &str, range: &Range<usize>) -> bool {
    let mut node = program_node(view, range.start, range.end);
    while let Some(current) = node {
        if current.kind() == "arrow_function" {
            let text = &source[view.span(current)];
            return text.starts_with('@') || text.starts_with("v-on");
        }
        node = current.parent();
    }
    false
}

/// The oracle's `references` and `declarations`, read off an ECMAScript scope table over the
/// view's program: a `local` name resolves to a binding the template declares (a `v-for` alias
/// or slot parameter), but `$event`, which the view leaves unresolved in an inline handler; a
/// `setup:` name to one a script block declares; any other name (`props`, `data`, `options`,
/// `instance`, `global`) to none. Each declaration is a binding of the table.
#[test]
fn vue_view_scope_oracle() {
    let expected: Value = serde_json::from_str(
        &std::fs::read_to_string(root().join("view.expected.json")).expect("oracle"),
    )
    .expect("json");
    let mut errors = Vec::new();
    let mut checked: BTreeMap<String, usize> = BTreeMap::new();
    let mut declarations = 0;
    for file in expected["files"].as_array().expect("files") {
        let path = file["path"].as_str().expect("path");
        let (source, tree) = parse(path);
        let blocks = sfc::blocks(tree.root_node(), &source);
        let Some(template) = blocks
            .iter()
            .find(|block| block.name == "template" && block.lang.is_none_or(|lang| lang == "html"))
        else {
            continue;
        };
        let dialect = sfc::script_dialect(&blocks).unwrap_or(GrammarDialect::JavaScript);
        let regions = template::regions(template.node, &source, dialect);
        let template = template.node.byte_range();
        let scripts: Vec<Range<usize>> = blocks
            .iter()
            .filter(|block| block.name == "script")
            .map(|block| block.content.clone())
            .collect();
        let Some(view) = View::build(tree.root_node(), &source) else {
            errors.push(format!("{path}: no program"));
            continue;
        };
        let program = view.dialect().ast_grep(view.text());
        let table = ScopeTable::build(program.root(), view.dialect(), usize::MAX, &|| false)
            .expect("scope table");
        let by_span = identifiers(&view);
        // Where a binding's name stands in the file.
        let declared = |binding: &Binding| {
            let name = binding.name?;
            program_node(&view, name.start as usize, name.end as usize).map(|node| view.span(node))
        };
        for reference in file["references"].as_array().expect("references") {
            let name = reference["name"].as_str().expect("name");
            let resolution = reference["resolution"].as_str().expect("resolution");
            let start = reference["start"].as_u64().expect("start") as usize;
            let end = reference["end"].as_u64().expect("end") as usize;
            let at = format!("{path} {start}..{end} `{name}` ({resolution})");
            let Some(range) = by_span.get(&unquirked(path, start, end, &regions)) else {
                errors.push(format!("{at}: no program identifier"));
                continue;
            };
            let Some(read) = table.reference(range) else {
                errors.push(format!("{at}: not a reference"));
                continue;
            };
            let binding = table.resolve(read, name).map(&declared);
            let inside = |within: &Range<usize>| match &binding {
                Some(Some(span)) => within.start <= span.start && span.end <= within.end,
                _ => false,
            };
            let agrees = match resolution {
                "local" if name == "$event" => {
                    binding.is_none() && in_inline_handler(&view, &source, range)
                }
                "local" => inside(&template),
                setup if setup.starts_with("setup:") => scripts.iter().any(inside),
                _ => binding.is_none(),
            };
            if !agrees {
                errors.push(format!("{at}: resolves to {binding:?}"));
            }
            let category = resolution.split(':').next().unwrap_or(resolution);
            *checked.entry(category.to_string()).or_default() += 1;
        }
        let bound: HashSet<Range<usize>> = table
            .scopes
            .iter()
            .flat_map(|scope| scope.bindings.values().flatten())
            .filter_map(&declared)
            .collect();
        for declaration in file["declarations"].as_array().expect("declarations") {
            let start = declaration["start"].as_u64().expect("start") as usize;
            let end = declaration["end"].as_u64().expect("end") as usize;
            declarations += 1;
            if !bound.contains(&(start..end)) {
                errors.push(format!(
                    "{path} {start}..{end} `{}`: no binding declared there",
                    declaration["name"]
                ));
            }
        }
    }
    eprintln!("references checked: {checked:?}; declarations checked: {declarations}");
    assert!(checked.values().sum::<usize>() > 0 && declarations > 0);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
