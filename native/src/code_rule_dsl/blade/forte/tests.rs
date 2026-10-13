//! The port agrees with Forte 1.1.0 on every template the oracle records
//! (`fixtures/blade/blade/forte`, `forte_tree.php` documents a record): tokens, flat nodes with
//! their extra fields, lexer errors, what Forte's AST API reads (`ast`), and the depth errors
//! Forte throws.
//!
//! Environment, for working on one stage:
//! - `FORTE_PARITY`: `lexer` compares tokens and errors only; `parser` builds nodes from the
//!   oracle's tokens and compares nodes; `ast` reads props from the oracle's tokens and nodes;
//!   unset compares everything from the template.
//! - `FORTE_SUITE`: another suite file, e.g. `generate.php --full`'s, whose trees are all spelled
//!   out (the checked-in file has digests for large templates and for prefixes).
//! - `FORTE_CASE`: only the case with this index; `FORTE_REPORT`: differences shown (default 20).

use super::ast::{Ast, ElementClass, PhpTagType};
use super::directives::{ArgumentRequirement, Directives, DiscoveredDirective, Table};
use super::node::{DirectiveInfo, FlatNode, NodeKind, StructureRole};
use super::token::{ErrorReason, LexerError, State, Token, TokenType};
use super::{Budget, DepthLimits, ParseError, Parsed};
use crate::code_rule_dsl::test_support::php_lossy;
use indexmap::IndexSet;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::fmt::Write as _;

const ORACLES: &str = crate::code_rule_dsl::fixtures_dir!();

pub(super) fn read_oracle(path: &str) -> Json {
    read_json(&format!("{ORACLES}/blade/blade/forte/{path}"))
}

pub(super) fn read_json(path: &str) -> Json {
    let text = std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{path}: {error}"));
    serde_json::from_str(&text).expect("oracle JSON")
}

/// The tree a case records only as `digest`, as Forte writes a tree (`forte_tree.php`): the
/// port's parse of `source`, where it hashes to the digest (`None` otherwise, which
/// `forte_suite_matches_forte` reports).
pub(super) fn digested_tree(source: &[u8], case: &Json, registries: &Json) -> Option<Json> {
    let digest = case.get("digest")?.as_str()?;
    let directives = registry(case["registry"].as_str().expect("registry"), registries);
    let parsed = super::parse(
        source,
        directives,
        DepthLimits::default(),
        &mut Budget::for_template(source.len()),
    );
    let json = tree_json(source, &parsed, true);
    (fnv1a64(&json) == digest).then(|| serde_json::from_str(&json).expect("tree JSON"))
}

/// A case's template.
pub(super) fn source_of(case: &Json) -> Vec<u8> {
    match case.get("source").and_then(Json::as_str) {
        Some(source) => source.as_bytes().to_vec(),
        None => base64_decode(case["source_base64"].as_str().expect("source")),
    }
}

fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("base64"),
        }
    };
    let bytes: Vec<u8> = text.bytes().filter(|c| *c != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            acc |= value(*c) << (18 - 6 * i);
        }
        for i in 0..chunk.len().saturating_sub(1) {
            out.push((acc >> (16 - 8 * i)) as u8);
        }
    }
    out
}

/// The registry a case names (`forte_registry`).
fn registry(name: &str, registries: &Json) -> Directives {
    match name {
        "default" => Directives::with_defaults(),
        "accept-all" => {
            let mut directives = Directives::with_defaults();
            directives.set_accept_all(true);
            directives
        }
        key => registry_from_state(&registries[key]),
    }
}

/// `forte_registry_from_state`.
fn registry_from_state(state: &Json) -> Directives {
    let bytes = |value: &Json| -> Box<[u8]> { value.as_str().expect("name").as_bytes().into() };
    let set = |key: &str| -> IndexSet<Box<[u8]>> {
        state[key]
            .as_array()
            .expect("names")
            .iter()
            .map(bytes)
            .collect()
    };
    let mut table = Table::default();
    table.conditions = set("conditions");
    table.final_terminators = set("finalTerminators");
    let keys = state["keys"].as_array().expect("keys");
    for (key, d) in keys
        .iter()
        .zip(state["directives"].as_array().expect("directives"))
    {
        let names = |field: &str| -> Vec<Box<[u8]>> {
            d[field]
                .as_array()
                .expect("list")
                .iter()
                .map(bytes)
                .collect()
        };
        let optional = |field: &str| d[field].as_str().map(|text| text.as_bytes().into());
        let flag = |field: &str| d[field].as_bool().expect("flag");
        let directive = DiscoveredDirective {
            name: bytes(&d["name"]),
            args: match d["args"].as_str() {
                Some("Required") => ArgumentRequirement::Required,
                Some("NotAllowed") => ArgumentRequirement::NotAllowed,
                _ => ArgumentRequirement::Optional,
            },
            role: role_named(d["role"].as_str().expect("role")),
            is_condition: flag("isCondition"),
            has_condition_like_branches: flag("hasConditionLikeBranches"),
            terminators: names("terminators"),
            condition_like_branches: names("conditionLikeBranches"),
            terminator: optional("terminator"),
            is_switch: flag("isSwitch"),
            switch_parent: optional("switchParent"),
            is_switch_branch: flag("isSwitchBranch"),
            is_switch_terminator: flag("isSwitchTerminator"),
            is_conditional_pair: flag("isConditionalPair"),
            pairing_strategy: d["pairingStrategy"].as_str().map(Box::from),
            is_conditional_close: flag("isConditionalClose"),
        };
        table.directives.insert(bytes(key), directive);
    }
    Directives::from_parts(
        table,
        set("seen"),
        set("advisoryPairs"),
        set("advisoryConditions"),
        state["acceptAll"].as_bool().expect("acceptAll"),
    )
}

fn role_named(name: &str) -> StructureRole {
    match name {
        "Opening" => StructureRole::Opening,
        "Closing" => StructureRole::Closing,
        "Intermediate" => StructureRole::Intermediate,
        "Mixed" => StructureRole::Mixed,
        _ => StructureRole::None,
    }
}

/// A JSON string as PHP writes `$bytes` (`forte_json`).
fn string(out: &mut String, bytes: &[u8]) {
    out.push_str(&serde_json::to_string(&php_lossy(bytes)).expect("string"));
}

fn opt_string(out: &mut String, bytes: Option<&[u8]>) {
    match bytes {
        Some(bytes) => string(out, bytes),
        None => out.push_str("null"),
    }
}

fn opt_index(out: &mut String, index: Option<usize>) {
    match index {
        Some(index) => write!(out, "{index}").expect("write"),
        None => out.push_str("null"),
    }
}

/// The props `forte_tree` records (`p`), as JSON objects keyed by node index, written as PHP
/// writes them: attribute entries first, then a node's own entry over any attribute entry of
/// the same index.
fn props(ast: &Ast<'_>) -> BTreeMap<usize, String> {
    let mut props = BTreeMap::new();
    for (index, node) in ast.nodes().iter().enumerate().skip(1) {
        let mut p = String::from("{");
        match node.kind {
            NodeKind::Element => {
                let element = ast.element(index);
                p.push_str("\"c\":");
                p.push_str(match element.class {
                    ElementClass::Element => "\"element\"",
                    ElementClass::Component => "\"component\"",
                    ElementClass::Slot => "\"slot\"",
                });
                p.push_str(",\"tag\":");
                string(&mut p, &element.tag);
                write!(
                    p,
                    ",\"void\":{},\"self_closing\":{},\"paired\":{}",
                    element.void, element.self_closing, element.paired
                )
                .expect("write");
                if let Some(component) = &element.component {
                    p.push_str(",\"component\":[");
                    string(&mut p, &component.kind);
                    p.push(',');
                    string(&mut p, &component.prefix);
                    p.push(',');
                    string(&mut p, &component.name);
                    p.push(',');
                    opt_string(&mut p, component.slot_name.as_deref());
                    p.push(']');
                }
                for attribute in ast.attributes(index) {
                    let mut a = String::from("{\"type\":");
                    string(&mut a, attribute.kind.as_bytes());
                    a.push_str(",\"raw_name\":");
                    string(&mut a, &attribute.raw_name);
                    a.push_str(",\"name\":");
                    string(&mut a, &attribute.name);
                    a.push_str(",\"value\":");
                    opt_string(&mut a, attribute.value.as_deref());
                    a.push_str(",\"quote\":");
                    opt_string(&mut a, attribute.quote.as_ref().map(std::slice::from_ref));
                    a.push('}');
                    props.insert(attribute.index, a);
                }
            }
            NodeKind::DirectiveBlock => {
                let block = ast.block(index);
                p.push_str("\"start\":");
                opt_index(&mut p, block.start);
                p.push_str(",\"end\":");
                opt_index(&mut p, block.end);
                p.push_str(",\"mid\":[");
                for (i, mid) in block.mid.iter().enumerate() {
                    if i > 0 {
                        p.push(',');
                    }
                    write!(p, "{mid}").expect("write");
                }
                p.push(']');
            }
            NodeKind::Echo | NodeKind::RawEcho | NodeKind::TripleEcho => {
                p.push_str("\"content\":");
                string(&mut p, &ast.source()[ast.echo_content(index)]);
            }
            NodeKind::PhpBlock => {
                p.push_str("\"content\":");
                string(&mut p, &ast.source()[ast.php_block_content(index)]);
            }
            NodeKind::PhpTag => {
                let (kind, content) = ast.php_tag(index);
                p.push_str(match kind {
                    PhpTagType::Php => "\"php\":\"php\"",
                    PhpTagType::Echo => "\"php\":\"echo\"",
                });
                p.push_str(",\"content\":");
                string(&mut p, &ast.source()[content]);
            }
            _ => continue,
        }
        p.push('}');
        props.insert(index, p);
    }
    props
}

/// A tree as `forte_tree` records it, written as `forte_json` writes it.
fn tree_json(source: &[u8], result: &Result<Parsed, ParseError>, with_props: bool) -> String {
    let parsed = match result {
        Ok(parsed) => parsed,
        Err(ParseError::Depth(message)) => {
            let mut out = String::from("{\"x\":");
            string(&mut out, message.as_bytes());
            out.push('}');
            return out;
        }
        Err(ParseError::Budget) => return "\"budget\"".into(),
    };
    let mut out = String::from("{\"t\":[");
    for (i, token) in parsed.tokens.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write!(out, "{},{},{}", token.kind as u8, token.start, token.end).expect("write");
    }
    out.push_str("],\"n\":[");
    for (i, node) in parsed.nodes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write!(
            out,
            "[{},{},{},{},{},{},{},{},{}",
            node.kind as u8,
            node.parent,
            node.first_child,
            node.last_child,
            node.next_sibling,
            node.token_start,
            node.token_count,
            node.generic_offset,
            node.data
        )
        .expect("write");
        if let Some(info) = &node.directive {
            out.push_str(",{\"args\":");
            opt_string(&mut out, info.args.as_deref());
            out.push_str(",\"name\":");
            string(&mut out, &info.name);
            if let Some(role) = info.role {
                write!(out, ",\"role\":\"{}\"", role.name()).expect("write");
            }
            out.push('}');
        }
        out.push(']');
    }
    out.push_str("],\"e\":[");
    for (i, error) in parsed.errors.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write!(
            out,
            "[\"{}\",\"{}\",{}]",
            error.state.name(),
            error.reason.name(),
            error.offset
        )
        .expect("write");
    }
    out.push(']');
    if with_props {
        let props = props(&Ast::new(source, &parsed.tokens, &parsed.nodes));
        if !props.is_empty() {
            out.push_str(",\"p\":{");
            for (i, (index, p)) in props.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write!(out, "\"{index}\":{p}").expect("write");
            }
            out.push('}');
        }
    }
    out.push('}');
    out
}

/// `hash('fnv1a64', $text)`.
fn fnv1a64(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The stage `FORTE_PARITY` names.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    All,
    Lexer,
    Parser,
    Ast,
}

fn stage() -> Stage {
    match std::env::var("FORTE_PARITY").as_deref() {
        Ok("lexer") => Stage::Lexer,
        Ok("parser") => Stage::Parser,
        Ok("ast") => Stage::Ast,
        _ => Stage::All,
    }
}

/// The oracle's tokens.
pub(super) fn oracle_tokens(tree: &Json) -> Vec<Token> {
    let t = tree["t"].as_array().expect("t");
    t.chunks(3)
        .map(|chunk| Token {
            kind: token_type(chunk[0].as_u64().expect("type") as u8),
            start: chunk[1].as_u64().expect("start") as u32,
            end: chunk[2].as_u64().expect("end") as u32,
        })
        .collect()
}

fn token_type(number: u8) -> TokenType {
    assert!(number <= TokenType::PiEnd as u8, "token type {number}");
    // SAFETY: `TokenType` is `repr(u8)` with every discriminant from 0 to `PiEnd`.
    unsafe { std::mem::transmute::<u8, TokenType>(number) }
}

fn node_kind(number: u8) -> NodeKind {
    // Forte's `Fragment` (3) is no kind of the port: Forte's tree builder never creates one.
    assert!(
        number <= NodeKind::ProcessingInstruction as u8 && number != 3,
        "node kind {number}"
    );
    // SAFETY: `NodeKind` is `repr(u8)` with every discriminant from 0 to `ProcessingInstruction`
    // but 3.
    unsafe { std::mem::transmute::<u8, NodeKind>(number) }
}

/// The oracle's nodes.
pub(super) fn oracle_nodes(tree: &Json) -> Vec<FlatNode> {
    tree["n"]
        .as_array()
        .expect("n")
        .iter()
        .map(|row| {
            let row = row.as_array().expect("row");
            let int = |i: usize| row[i].as_i64().expect("field") as i32;
            FlatNode {
                kind: node_kind(int(0) as u8),
                parent: int(1),
                first_child: int(2),
                last_child: int(3),
                next_sibling: int(4),
                token_start: int(5),
                token_count: int(6),
                generic_offset: int(7),
                data: int(8),
                directive: row.get(9).map(|extra| {
                    Box::new(DirectiveInfo {
                        name: extra["name"].as_str().unwrap_or("").as_bytes().into(),
                        args: extra["args"].as_str().map(|args| args.as_bytes().into()),
                        role: extra.get("role").and_then(Json::as_str).map(role_named),
                    })
                }),
            }
        })
        .collect()
}

/// The oracle's lexer errors.
pub(super) fn oracle_errors(tree: &Json) -> Vec<LexerError> {
    tree["e"]
        .as_array()
        .expect("e")
        .iter()
        .map(|row| LexerError {
            state: state_named(row[0].as_str().expect("state")),
            reason: reason_named(row[1].as_str().expect("reason")),
            offset: row[2].as_u64().expect("offset") as u32,
        })
        .collect()
}

fn state_named(name: &str) -> State {
    State::ALL
        .into_iter()
        .find(|state| state.name() == name)
        .expect("state")
}

fn reason_named(name: &str) -> ErrorReason {
    ErrorReason::ALL
        .into_iter()
        .find(|reason| reason.name() == name)
        .expect("reason")
}

/// The expected and actual trees of a template, as `forte_json` writes them, for `stage`.
fn trees(source: &[u8], directives: Directives, expected: &Json, stage: Stage) -> (String, String) {
    let mut budget = Budget::for_template(source.len());
    match stage {
        Stage::All => {
            let actual = super::parse(source, directives, DepthLimits::default(), &mut budget);
            (canonical(expected), tree_json(source, &actual, true))
        }
        Stage::Lexer => {
            let lexed = super::lexer::tokenize(source, &directives, &mut budget);
            let actual = lexed.map_err(ParseError::from).map(|lexed| Parsed {
                tokens: lexed.tokens,
                nodes: Vec::new(),
                errors: lexed.errors,
            });
            // `tree_json`'s key order: "t" before "e" (a `json!` map sorts its keys).
            let only = format!("{{\"t\":{},\"e\":{}}}", expected["t"], expected["e"]);
            let actual = tree_json(source, &actual, false).replace(",\"n\":[]", "");
            (only, actual)
        }
        Stage::Parser => {
            let tokens = oracle_tokens(expected);
            let mut directives = directives;
            if directives.accepts_all_directives() {
                directives.train(
                    tokens
                        .iter()
                        .filter(|token| token.kind == TokenType::Directive)
                        .map(|token| source[token.start as usize..token.end as usize].into()),
                );
            }
            let nodes = super::parser::build(
                &tokens,
                source,
                &directives,
                DepthLimits::default(),
                &mut budget,
            );
            let actual = nodes.map(|nodes| Parsed {
                tokens,
                nodes,
                errors: oracle_errors(expected),
            });
            // The parser stage builds no props: compare the tree without them.
            let mut without_props = expected.clone();
            if let Some(tree) = without_props.as_object_mut() {
                tree.remove("p");
            }
            (canonical(&without_props), tree_json(source, &actual, false))
        }
        Stage::Ast => {
            let parsed = Parsed {
                tokens: oracle_tokens(expected),
                nodes: oracle_nodes(expected),
                errors: oracle_errors(expected),
            };
            (canonical(expected), tree_json(source, &Ok(parsed), true))
        }
    }
}

/// An oracle tree written back with the field order `forte_json` writes (serde_json's maps sort
/// their keys; `forte_tree`'s do not).
fn canonical(tree: &Json) -> String {
    if let Some(message) = tree.get("x") {
        return format!("{{\"x\":{message}}}");
    }
    let mut out = format!("{{\"t\":{},\"n\":[", tree["t"]);
    for (i, row) in tree["n"].as_array().expect("n").iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let row = row.as_array().expect("row");
        out.push('[');
        for (j, field) in row.iter().take(9).enumerate() {
            if j > 0 {
                out.push(',');
            }
            write!(out, "{field}").expect("write");
        }
        if let Some(extra) = row.get(9) {
            write!(
                out,
                ",{{\"args\":{},\"name\":{}",
                extra["args"], extra["name"]
            )
            .expect("write");
            if let Some(role) = extra.get("role") {
                write!(out, ",\"role\":{role}").expect("write");
            }
            out.push('}');
        }
        out.push(']');
    }
    write!(out, "],\"e\":{}", tree["e"]).expect("write");
    if let Some(p) = tree.get("p").and_then(Json::as_object) {
        out.push_str(",\"p\":{");
        let mut entries: Vec<(usize, &Json)> = p
            .iter()
            .map(|(key, value)| (key.parse().expect("index"), value))
            .collect();
        entries.sort_by_key(|(index, _)| *index);
        for (i, (index, value)) in entries.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write!(out, "\"{index}\":{{").expect("write");
            let value = value.as_object().expect("props");
            let order = [
                "c",
                "tag",
                "void",
                "self_closing",
                "paired",
                "component",
                "type",
                "raw_name",
                "name",
                "value",
                "quote",
                "start",
                "end",
                "mid",
                "php",
                "content",
            ];
            let mut first = true;
            for key in order {
                if let Some(field) = value.get(key) {
                    if !first {
                        out.push(',');
                    }
                    first = false;
                    write!(out, "\"{key}\":{field}").expect("write");
                }
            }
            assert_eq!(
                value.len(),
                order.iter().filter(|key| value.contains_key(**key)).count(),
                "props keys"
            );
            out.push('}');
        }
        out.push('}');
    }
    out.push('}');
    out
}

/// Where two trees' JSON first differ, with context.
fn first_difference(expected: &str, actual: &str) -> String {
    let at = expected
        .bytes()
        .zip(actual.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or(expected.len().min(actual.len()));
    let window = |text: &str| {
        let mut start = at.saturating_sub(120);
        while !text.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = (at + 120).min(text.len());
        while !text.is_char_boundary(end) {
            end += 1;
        }
        text[start..end].to_owned()
    };
    format!(
        "at JSON byte {at}\n  expected …{}…\n  actual   …{}…",
        window(expected),
        window(actual)
    )
}

/// A case's label in reports.
fn label(index: usize, case: &Json) -> String {
    let what = case
        .get("path")
        .and_then(Json::as_str)
        .map(str::to_owned)
        .or_else(|| case.get("tests").map(|tests| tests.to_string()))
        .unwrap_or_default();
    format!(
        "case {index} [{}] {what}",
        case["registry"].as_str().unwrap_or("")
    )
}

/// The results of checking the cases of one oracle file.
#[derive(Default)]
struct Outcome {
    trees: usize,
    differences: usize,
    report: Vec<String>,
}

impl Outcome {
    fn record(
        &mut self,
        label: impl FnOnce() -> String,
        source: &[u8],
        expected: &str,
        actual: &str,
    ) {
        self.trees += 1;
        if expected == actual {
            return;
        }
        self.differences += 1;
        let limit = std::env::var("FORTE_REPORT")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(20);
        if self.report.len() < limit {
            let shown = String::from_utf8_lossy(&source[..source.len().min(300)]);
            self.report.push(format!(
                "{}\n  source {shown:?}\n  {}",
                label(),
                first_difference(expected, actual)
            ));
        }
    }

    fn digest(&mut self, label: impl FnOnce() -> String, expected: &str, actual: &str) {
        self.trees += 1;
        if fnv1a64(actual) == expected {
            return;
        }
        self.differences += 1;
        if self.report.len() < 20 {
            self.report.push(format!(
                "{}: digest {expected}, the port's tree hashes to {} (FORTE_SUITE=<generate.php --full> spells it out)",
                label(),
                fnv1a64(actual)
            ));
        }
    }

    fn assert(self, what: &str, minimum: usize) {
        eprintln!(
            "{what}: {} trees, {} differences",
            self.trees, self.differences
        );
        assert!(
            self.differences == 0,
            "{what}: {} of {} trees differ from Forte's\n{}",
            self.differences,
            self.trees,
            self.report.join("\n")
        );
        // A stage alone skips the cases the oracle records only as digests.
        if stage() == Stage::All {
            assert!(
                self.trees >= minimum,
                "{what}: {} trees, expected at least {minimum}",
                self.trees
            );
        }
    }
}

/// Checks every case of an oracle file; cases with only digests are checked when the stage
/// compares whole trees.
fn check_file(oracle: &Json) -> Outcome {
    let stage = stage();
    let only: Option<usize> = std::env::var("FORTE_CASE")
        .ok()
        .and_then(|n| n.parse().ok());
    let registries = oracle.get("registries").cloned().unwrap_or(Json::Null);
    let mut outcome = Outcome::default();
    for (index, case) in oracle["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .enumerate()
    {
        if only.is_some_and(|only| only != index) {
            continue;
        }
        let source = source_of(case);
        let directives = || registry(case["registry"].as_str().expect("registry"), &registries);
        if let Some(tree) = case.get("tree") {
            if stage != Stage::All && stage != Stage::Lexer && tree.get("x").is_some() {
                continue;
            }
            if stage == Stage::Lexer && tree.get("x").is_some() {
                continue;
            }
            let (expected, actual) = trees(&source, directives(), tree, stage);
            outcome.record(|| label(index, case), &source, &expected, &actual);
        } else if let Some(trees_) = case.get("trees").and_then(Json::as_array) {
            for (tree, length) in trees_.iter().zip(prefix_positions(source.len())) {
                if stage != Stage::All && tree.get("x").is_some() {
                    continue;
                }
                let prefix = &source[..length];
                let (expected, actual) = trees(prefix, directives(), tree, stage);
                outcome.record(
                    || format!("{} prefix {length}", label(index, case)),
                    prefix,
                    &expected,
                    &actual,
                );
            }
        } else if stage == Stage::All {
            if let Some(digest) = case.get("digest").and_then(Json::as_str) {
                let actual = super::parse(
                    &source,
                    directives(),
                    DepthLimits::default(),
                    &mut Budget::for_template(source.len()),
                );
                outcome.digest(
                    || label(index, case),
                    digest,
                    &tree_json(&source, &actual, true),
                );
            } else if let Some(digests) = case.get("digests").and_then(Json::as_array) {
                for (digest, length) in digests.iter().zip(prefix_positions(source.len())) {
                    let prefix = &source[..length];
                    let actual = super::parse(
                        prefix,
                        directives(),
                        DepthLimits::default(),
                        &mut Budget::for_template(prefix.len()),
                    );
                    outcome.digest(
                        || format!("{} prefix {length}", label(index, case)),
                        digest.as_str().expect("digest"),
                        &tree_json(prefix, &actual, true),
                    );
                }
            }
        }
    }
    outcome
}

/// `forte_prefix_positions`.
pub(super) fn prefix_positions(length: usize) -> Vec<usize> {
    if length > 1000 {
        let mut positions: Vec<usize> = (0..100)
            .map(|i| (i * length / 100).clamp(1, length))
            .collect();
        positions.dedup();
        positions
    } else {
        (1..=length).collect()
    }
}

#[test]
fn forte_suite_matches_forte() {
    let oracle = match std::env::var("FORTE_SUITE") {
        Ok(path) => read_json(&path),
        Err(_) => read_oracle("suite.expected.json"),
    };
    check_file(&oracle).assert("Forte's test suite", 9_000);
}

#[test]
fn forte_fixtures_match_forte() {
    check_file(&read_oracle("fixtures.expected.json"))
        .assert("fixtures and grammar-gap repros", 60);
}

#[test]
fn forte_corpus_samples_match_forte() {
    for corpus in ["bookstack", "filament", "bagisto", "laravel"] {
        check_file(&read_oracle(&format!("corpus/{corpus}.expected.json"))).assert(corpus, 50);
    }
}

/// The default registry is Forte's: every directive, in Forte's order, with its metadata.
#[test]
fn default_registry_is_fortes() {
    let oracle = read_oracle("fixtures.expected.json");
    let expected = registry_from_state(&oracle["default_registry"]);
    let actual = Directives::with_defaults();
    assert_eq!(
        format!("{:?}", actual.table().directives),
        format!("{:?}", expected.table().directives)
    );
    assert_eq!(actual.table().conditions, expected.table().conditions);
    assert_eq!(
        actual.table().final_terminators,
        expected.table().final_terminators
    );
}

/// Every template of the four corpora (`RERAIL_FORTE_CORPUS`, written by
/// `fixtures/blade/blade/forte/generate.php --corpus`), both registries. With every stage compared,
/// it also prints, per corpus and registry, the templates the port reports an error in (a lexer
/// error or a depth error: the T1 count) and the most budget steps a template spent per byte.
#[test]
#[ignore = "Forte over every template of the Blade corpora"]
fn benchmark_forte_corpus() {
    let path = std::env::var("RERAIL_FORTE_CORPUS").expect("RERAIL_FORTE_CORPUS");
    let oracle: Json =
        serde_json::from_str(&std::fs::read_to_string(path).expect("corpus")).expect("corpus JSON");
    let mut per_corpus: BTreeMap<String, Outcome> = BTreeMap::new();
    // (corpus, registry) -> (templates, templates with an error, most steps per byte)
    let mut t1: BTreeMap<(String, String), (usize, usize, f64)> = BTreeMap::new();
    let stage = stage();
    for (index, case) in oracle["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .enumerate()
    {
        let source = source_of(case);
        let registry_name = case["registry"].as_str().expect("registry");
        let directives = registry(registry_name, &Json::Null);
        let corpus = case["corpus"].as_str().expect("corpus").to_owned();
        let (expected, actual) = if stage == Stage::All {
            let mut budget = Budget::for_template(source.len());
            let start = budget.remaining();
            let parsed = super::parse(&source, directives, DepthLimits::default(), &mut budget);
            let row = t1
                .entry((corpus.clone(), registry_name.to_owned()))
                .or_default();
            row.0 += 1;
            if parsed
                .as_ref()
                .map_or(true, |parsed| !parsed.errors.is_empty())
            {
                row.1 += 1;
            }
            row.2 = row
                .2
                .max((start - budget.remaining()) as f64 / source.len().max(1) as f64);
            (canonical(&case["tree"]), tree_json(&source, &parsed, true))
        } else {
            trees(&source, directives, &case["tree"], stage)
        };
        per_corpus.entry(corpus).or_default().record(
            || label(index, case),
            &source,
            &expected,
            &actual,
        );
    }
    for ((corpus, registry), (templates, errors, steps)) in &t1 {
        eprintln!(
            "{corpus} {registry}: {templates} templates, {errors} with an error, at most {steps:.1} steps per byte"
        );
    }
    let mut failed = Vec::new();
    for (corpus, outcome) in per_corpus {
        eprintln!(
            "{corpus}: {} trees, {} differences",
            outcome.trees, outcome.differences
        );
        if outcome.differences > 0 {
            failed.push(format!("{corpus}:\n{}", outcome.report.join("\n")));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// SplitMix64: the robustness test's seeded inputs.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// What the robustness test assembles templates from: Blade, HTML, PHP and JavaScript syntax,
/// so random sequences open and leave open every construct the lexer and tree builder track.
const FRAGMENTS: &[&str] = &[
    "{{",
    "}}",
    "{!!",
    "!!}",
    "{{{",
    "}}}",
    "{{--",
    "--}}",
    "@{{",
    "@@if",
    "@if(",
    "@if",
    "@elseif(",
    "@else",
    "@endif",
    "@unless(",
    "@endunless",
    "@isset(",
    "@endisset",
    "@foreach(",
    "@endforeach",
    "@forelse(",
    "@empty",
    "@endforelse",
    "@for(",
    "@endfor",
    "@while(",
    "@endwhile",
    "@switch(",
    "@case(",
    "@break",
    "@default",
    "@endswitch",
    "@php",
    "@php(",
    "@endphp",
    "@verbatim",
    "@endverbatim",
    "@section(",
    "@endsection",
    "@show",
    "@stop",
    "@yield(",
    "@push(",
    "@endpush",
    "@pushOnce",
    "@endPushOnce",
    "@once",
    "@endonce",
    "@include(",
    "@csrf",
    "@json(",
    "@class(",
    "@auth",
    "@endauth",
    "@can(",
    "@endcan",
    "@unknown",
    "@x",
    "<?php",
    "<?=",
    "?>",
    "<div",
    "</div>",
    "<p>",
    "<li>",
    "<table>",
    "<td>",
    "<x-alert",
    "</x-alert>",
    "<x-slot:name>",
    "<x-slot name=\"a\">",
    "</x-slot>",
    "<livewire:counter",
    "<flux:button",
    "<script>",
    "</script>",
    "<style>",
    "</style>",
    "<textarea>",
    "</textarea>",
    "<!--",
    "-->",
    "<!--[if IE]>",
    "<![endif]-->",
    "<![CDATA[",
    "]]>",
    "<!DOCTYPE html>",
    "<?xml",
    "<br/>",
    "<input",
    "<a href=",
    ">",
    "/>",
    "<",
    "</",
    " ",
    "\n",
    "\t",
    "=",
    "\"",
    "'",
    "`",
    "${",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    ":",
    "::",
    ":class=",
    "::x=",
    ":$x",
    "x-on:click=",
    "@click=",
    "{...props}",
    "<T,>",
    "$x",
    "->",
    "a",
    "text",
    "\\",
    "/*",
    "*/",
    "//",
    "#",
    "\0",
    "\r\n",
    "é",
    "\u{fffd}",
    "\u{b5}",
];

/// Input no template is: seeded random bytes, seeded random assemblies of Blade fragments and
/// prefixes of every fixture.
pub(super) fn robustness_inputs() -> Vec<Vec<u8>> {
    let mut random = Seeded(0x0001_0698_F0E7_E000);
    let mut inputs: Vec<Vec<u8>> = Vec::new();
    for _ in 0..1_500 {
        let length = random.below(400);
        inputs.push((0..length).map(|_| random.next() as u8).collect());
    }
    for _ in 0..3_000 {
        let mut template = Vec::new();
        for _ in 0..random.below(80) {
            template.extend_from_slice(FRAGMENTS[random.below(FRAGMENTS.len())].as_bytes());
        }
        inputs.push(template);
    }
    let fixtures = read_oracle("fixtures.expected.json");
    let sources: IndexSet<Vec<u8>> = fixtures["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .map(source_of)
        .collect();
    for source in &sources {
        let step = (source.len() / 150).max(1);
        inputs.extend(
            (1..source.len())
                .step_by(step)
                .map(|length| source[..length].to_vec()),
        );
    }
    inputs
}

/// The port finishes, without a panic, within its budget, stack and depth limits on input no
/// template is (`robustness_inputs`), under both registries; a tree it returns stays inside the
/// template and the token array. Nesting far past Forte's limits ends as Forte's does (recorded
/// with Forte 1.1.0), except where Forte's work is quadratic: there the budget ends it.
#[test]
fn forte_survives_random_and_truncated_templates() {
    let inputs = robustness_inputs();
    // 20,000 of `open` then 20,000 of `close`, and how Forte's `Document::parse` ends on it with
    // either registry: its depth exception, a tree, or (`None`) a tree after quadratic work
    // (20,000 unclosed `@section`s: each searches to the end for its `@endsection`; 5 s in PHP).
    const ELEMENT_DEPTH: Option<&str> = Some("Maximum element nesting depth (512) exceeded.");
    let stress: [(&str, &str, Option<Option<&str>>); 13] = [
        ("<div>", "", Some(ELEMENT_DEPTH)),
        ("<x-a>", "", Some(ELEMENT_DEPTH)),
        ("@if(1)", "", Some(ELEMENT_DEPTH)),
        ("@foreach($a as $b)", "", Some(ELEMENT_DEPTH)),
        ("@section('a')", "", None),
        ("@php", "", Some(None)),
        ("{{ (", "", Some(None)),
        ("@if((", "", Some(None)),
        ("<div x-data=\"`${", "", Some(ELEMENT_DEPTH)),
        ("<script>`${", "", Some(None)),
        ("<!--[if IE]>", "", Some(ELEMENT_DEPTH)),
        ("<div>", "</div>", Some(ELEMENT_DEPTH)),
        ("@if(1)", "@endif", Some(ELEMENT_DEPTH)),
    ];
    for (open, close, forte) in stress {
        let input = format!("{}{}", open.repeat(20_000), close.repeat(20_000)).into_bytes();
        for name in ["default", "accept-all"] {
            let mut budget = Budget::for_template(input.len());
            let parsed = super::parse(
                &input,
                registry(name, &Json::Null),
                DepthLimits::default(),
                &mut budget,
            );
            let ended = match parsed {
                Ok(_) => Some(None),
                Err(ParseError::Depth(message)) => Some(Some(message)),
                Err(ParseError::Budget) => None,
            };
            let expected = forte.map(|depth| depth.map(str::to_owned));
            assert_eq!(ended, expected, "{open}{close} with the {name} registry");
        }
    }

    let (mut finished, mut depth, mut exhausted) = (0, 0, Vec::new());
    for input in &inputs {
        for directives in [
            registry("default", &Json::Null),
            registry("accept-all", &Json::Null),
        ] {
            let mut budget = Budget::for_template(input.len());
            match super::parse(input, directives, DepthLimits::default(), &mut budget) {
                Ok(parsed) => {
                    finished += 1;
                    for token in &parsed.tokens {
                        assert!(
                            token.start <= token.end && token.end as usize <= input.len(),
                            "{input:?}: token {token:?}"
                        );
                    }
                    for node in &parsed.nodes {
                        let end = node.token_start as usize + node.token_count as usize;
                        assert!(end <= parsed.tokens.len(), "{input:?}: node {node:?}");
                    }
                }
                Err(ParseError::Depth(_)) => depth += 1,
                Err(ParseError::Budget) => exhausted.push(format!(
                    "{} bytes {:?}",
                    input.len(),
                    String::from_utf8_lossy(&input[..input.len().min(60)])
                )),
            }
        }
    }
    eprintln!(
        "{} inputs: {finished} trees, {depth} depth errors, {} budgets spent",
        inputs.len(),
        exhausted.len()
    );
    assert!(
        exhausted.is_empty(),
        "templates that exhausted their budget: {exhausted:?}"
    );
}

/// Throughput and peak memory of a parser over one corpus (release build): the port's lexer and
/// tree builder (`FORTE_BENCH_PARSER=port`), or the same plus Rerail's tree over it (`port-tree`,
/// `tree::parse_tree`), over every template of `FORTE_BENCH_CORPUS` (`bookstack`, `filament`,
/// `bagisto`, `laravel`) under `RERAIL_BLADE_CORPORA` (`<checkout>/.rerail/corpora`), selected as
/// `generate.php --corpus` selects
/// them. Ten runs; prints one JSON line: the median run's seconds and MB/s, and the growth of the
/// process's peak resident set while parsing (one tree alive at a time) over its peak after
/// reading the templates. Run each parser and corpus in its own process.
#[test]
#[ignore = "parser throughput over a Blade corpus"]
fn benchmark_forte_throughput() {
    let parser = std::env::var("FORTE_BENCH_PARSER").expect("FORTE_BENCH_PARSER");
    let corpus = std::env::var("FORTE_BENCH_CORPUS").expect("FORTE_BENCH_CORPUS");
    let root = std::env::var("RERAIL_BLADE_CORPORA").expect("RERAIL_BLADE_CORPORA");
    let dir = match corpus.as_str() {
        "laravel" => format!("{root}/php/laravel"),
        name => format!("{root}/blade/{name}"),
    };
    let mut templates: Vec<(String, Vec<u8>)> = Vec::new();
    let mut pending = vec![std::path::PathBuf::from(&dir)];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(&path).expect("corpus dir") {
            let entry = entry.expect("entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            let file_type = entry.file_type().expect("file type");
            if file_type.is_dir() && name != ".git" {
                pending.push(entry.path());
            } else if file_type.is_file() && name.ends_with(".blade.php") {
                let bytes = std::fs::read(entry.path()).expect("template");
                if !bytes.is_empty()
                    && bytes.len() <= 4_000_000
                    && std::str::from_utf8(&bytes).is_ok()
                {
                    templates.push((entry.path().to_string_lossy().into_owned(), bytes));
                }
            }
        }
    }
    templates.sort();
    let bytes: usize = templates.iter().map(|(_, bytes)| bytes.len()).sum();
    let peak_rss = || {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        // SAFETY: getrusage writes a `rusage` into the pointer it is given.
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        // SAFETY: zeroed is a valid `rusage`, and getrusage filled it in.
        let max = unsafe { usage.assume_init() }.ru_maxrss as u64;
        // ru_maxrss is bytes on macOS, KiB on Linux.
        if cfg!(target_os = "macos") {
            max
        } else {
            max * 1024
        }
    };
    let before = peak_rss();
    let mut seconds = Vec::new();
    let mut checksum = 0usize;
    for _ in 0..10 {
        let start = std::time::Instant::now();
        for (_, template) in &templates {
            checksum += match parser.as_str() {
                "port" => {
                    let mut budget = Budget::for_template(template.len());
                    match super::parse(
                        template,
                        Directives::with_defaults(),
                        DepthLimits::default(),
                        &mut budget,
                    ) {
                        Ok(parsed) => parsed.nodes.len(),
                        Err(_) => 0,
                    }
                }
                "port-tree" => super::tree::parse_tree(template).len(),
                other => panic!("FORTE_BENCH_PARSER={other}"),
            };
        }
        seconds.push(start.elapsed().as_secs_f64());
    }
    seconds.sort_by(f64::total_cmp);
    let median = (seconds[4] + seconds[5]) / 2.0;
    let line = serde_json::json!({
        "parser": parser,
        "corpus": corpus,
        "templates": templates.len(),
        "bytes": bytes,
        "medianSeconds": median,
        "mbPerSecond": bytes as f64 / 1e6 / median,
        "peakRssBeforeBytes": before,
        "peakRssGrowthBytes": peak_rss().saturating_sub(before),
        "nodes": checksum / 10,
    });
    println!("FORTE_BENCH {line}");
}
