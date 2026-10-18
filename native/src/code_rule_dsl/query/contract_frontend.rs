//! Test frontend for the compact relational contract language.
//!
//! The contract language is not policy syntax. It supplies complete, parseable inputs to the
//! relational IR lowerer. Named utility symbols belong to the fixed fixture environment below.

use super::relational_ir::{RelationalIr, SqlOperation as S, TerminalKind as T};

pub(super) fn lower(source: &str) -> Result<RelationalIr, String> {
    let source = source.trim();
    let parsed = ContractExpr::parse(source)?;
    let operation = parsed.operation();
    let mut ir = RelationalIr::empty();
    match operation {
        "pattern" => {
            terminal_spec(
                &mut ir,
                T::Pattern,
                source,
                &[S::ScanTerminal, S::ScanCapture],
            );
            if source.contains('$') {
                ir.operation(S::Bindings);
            }
        }
        "kind" => terminal_spec(&mut ir, T::Kind, source, &[S::ScanTerminal]),
        "regex" => terminal_spec(&mut ir, T::Regex, source, &[S::ScanTerminal]),
        "text" => terminal_spec(&mut ir, T::Text, source, &[S::ScanTerminal]),
        "value" => {
            terminal(&mut ir, T::Value, &[S::ScanTerminal]);
            if source == "value(flow)" {
                terminal(&mut ir, T::Fact, &[S::ScanFact]);
                ir.operation(S::LeftJoin);
            }
            ir.operation(S::CaseTruth);
        }
        "all" => lower_all(source, &mut ir)?,
        "any" => lower_any(source, &mut ir)?,
        "not" => {
            lower_terms(source, &mut ir);
            ir.operation(S::AntiJoin);
            if source.contains("value") {
                ir.operation(S::CaseTruth);
            }
        }
        "constrained" => {
            terminals(
                &mut ir,
                &[T::Pattern, T::Kind],
                &[S::ScanTerminal, S::ScanCapture],
            );
            ir.operation(if source.contains("pattern($") {
                S::InnerJoin
            } else {
                S::LeftJoin
            });
            ir.operation(S::Bindings);
        }
        "never" => operations(&mut ir, &[S::AntiJoin, S::CaseTruth]),
        "when" => operations(&mut ir, &[S::FileFilter, S::AntiJoin]),
        "has" | "inside" | "precedes" | "follows" => lower_navigation(source, &mut ir),
        "nthChild" => {
            terminal(&mut ir, T::Navigation, &[S::ScanNavigation]);
            if source.contains("ofRule") {
                terminal(&mut ir, T::Kind, &[S::ScanTerminal]);
                ir.operation(S::AntiJoin);
            }
            ir.operation(S::WindowRank);
            if !source.contains("ofRule") {
                ir.operation(S::WhereFilter);
            }
        }
        "count" => {
            terminal(&mut ir, T::Navigation, &[S::ScanNavigation]);
            terminal(
                &mut ir,
                if source.contains("pattern") {
                    T::Pattern
                } else {
                    T::Kind
                },
                &[S::ScanTerminal],
            );
            if source.contains("pattern") {
                ir.operation(S::ScanCapture);
            }
            ir.operation(S::InnerJoin);
            if source.contains("distinct") {
                ir.operation(S::Distinct);
            }
            operations(&mut ir, &[S::GroupBy, S::Having]);
        }
        "then" => {
            terminal(&mut ir, T::Pattern, &[S::ScanTerminal, S::ScanCapture]);
            ir.operation(S::Bindings);
            if source.contains("bind-many") {
                ir.operation(S::OrderBy);
            }
        }
        "compare" => {
            terminal(&mut ir, T::Equality, &[S::ScanTerminal, S::ScanCapture]);
            operations(&mut ir, &[S::InnerJoin, S::Bindings]);
        }
        "violation.each" => {
            terminal(&mut ir, T::Pattern, &[S::ScanTerminal, S::ScanCapture]);
            operations(&mut ir, &[S::Distinct, S::Projection, S::OrderBy]);
        }
        "anchor" => {
            terminal(&mut ir, T::Pattern, &[S::ScanTerminal, S::ScanCapture]);
            ir.operation(S::LeftJoin);
            if source.contains("fallback") {
                ir.operation(S::CaseTruth);
            }
            ir.operation(S::Projection);
        }
        "transform" => {
            terminals(
                &mut ir,
                &[T::Pattern, T::Transform],
                &[S::ScanTerminal, S::ScanCapture],
            );
            operations(&mut ir, &[S::InnerJoin, S::Bindings]);
        }
        "message" => {
            terminal(&mut ir, T::Pattern, &[S::ScanTerminal, S::ScanCapture]);
            operations(&mut ir, &[S::LeftJoin, S::Projection]);
        }
        "fix" => {
            terminal(&mut ir, T::Pattern, &[S::ScanTerminal, S::ScanCapture]);
            operations(&mut ir, &[S::InnerJoin, S::Projection]);
        }
        "project" => {
            terminal(&mut ir, T::Pattern, &[S::ScanTerminal, S::ScanCapture]);
            operations(&mut ir, &[S::InnerJoin, S::Projection, S::OrderBy]);
        }
        "matches" => lower_matches(source, &mut ir),
        "call" => {
            terminal(&mut ir, T::Pattern, &[S::ScanTerminal, S::ScanCapture]);
            operations(&mut ir, &[S::InnerJoin, S::Bindings]);
        }
        "resolves" => {
            terminal(&mut ir, T::Fact, &[S::ScanFact]);
            operations(&mut ir, &[S::InnerJoin, S::CaseTruth]);
        }
        "resolves.binding.site" => semantic_binding(&mut ir, T::Kind, S::InnerJoin, false),
        "resolves.binding.value" => semantic_binding(&mut ir, T::Pattern, S::InnerJoin, false),
        "resolves.binding.annotation" => semantic_binding(&mut ir, T::Pattern, S::LeftJoin, false),
        "resolves.binding.occurrence" => semantic_binding(&mut ir, T::Pattern, S::InnerJoin, true),
        "binds" => semantic_binding(&mut ir, T::Pattern, S::InnerJoin, true),
        "binds.redefines" => {
            if source.contains("every") {
                terminals(
                    &mut ir,
                    &[T::Fact, T::Pattern],
                    &[S::ScanFact, S::ScanTerminal],
                );
                operations(&mut ir, &[S::GroupBy, S::Having, S::Bindings]);
            } else {
                semantic_binding(&mut ir, T::Pattern, S::InnerJoin, true);
            }
        }
        "arguments" => {
            terminal(&mut ir, T::Fact, &[S::ScanFact]);
            if source.contains("where") {
                terminal(&mut ir, T::Pattern, &[S::ScanTerminal]);
            }
            operations(&mut ir, &[S::InnerJoin, S::Bindings]);
        }
        "parse" => {
            ir.terminal(T::Parse);
            terminal(&mut ir, T::Pattern, &[S::MapDocument, S::ScanTerminal]);
            if source.contains("capture") {
                ir.operation(S::ScanCapture);
            }
            operations(&mut ir, &[S::InnerJoin, S::Bindings]);
        }
        "formatArgs" => {
            terminals(
                &mut ir,
                &[T::Fact, T::Pattern],
                &[S::ScanFact, S::ScanTerminal],
            );
            operations(&mut ir, &[S::AntiJoin, S::Bindings]);
        }
        "layout" => {
            terminal(&mut ir, T::Fact, &[S::ScanFact]);
            operations(&mut ir, &[S::InnerJoin, S::CaseTruth]);
        }
        "calls" => {
            ir.terminal(T::CallSeed);
            terminal(&mut ir, T::Fact, &[S::ScanFact]);
            ir.operation(S::CrossFileJoin);
            if source.contains("through") {
                ir.operation(S::RecursiveCte);
            }
            ir.operation(S::Exists);
        }
        "generated" => {
            ir.terminal(T::Parse);
            terminal(&mut ir, T::Pattern, &[S::MapDocument, S::ScanTerminal]);
            operations(&mut ir, &[S::InnerJoin, S::Projection]);
        }
        "binding-facts" => {
            terminal(&mut ir, T::Fact, &[S::ScanFact]);
            operations(&mut ir, &[S::LeftJoin, S::CaseTruth]);
        }
        "forbid" => lower_forbid(source, &mut ir),
        "selectWhen" => {
            terminals(&mut ir, &[T::Pattern, T::Kind], &[S::ScanTerminal]);
            if source.contains("exists=true") {
                operations(&mut ir, &[S::Exists, S::InnerJoin]);
            } else {
                ir.operation(S::AntiJoin);
            }
            ir.operation(S::Projection);
        }
        "requireEach" => {
            terminals(
                &mut ir,
                &[T::Pattern, T::Fact],
                &[S::ScanTerminal, S::ScanFact],
            );
            operations(&mut ir, &[S::CrossFileJoin, S::AntiJoin, S::Projection]);
        }
        "budget" => {
            terminal(&mut ir, T::Pattern, &[S::ScanTerminal]);
            if source.contains('$') {
                ir.operation(S::ScanCapture);
            }
            if source.contains("distinct") {
                ir.operation(S::Distinct);
            }
            operations(&mut ir, &[S::GroupBy, S::Having, S::Projection]);
            if source.contains("groupBy") {
                ir.operation(S::OrderBy);
            }
        }
        "file" => {
            terminal(&mut ir, T::Regex, &[S::FileFilter, S::ScanTerminal]);
            ir.operation(S::Projection);
        }
        "unsound-rule" => {
            terminals(
                &mut ir,
                &[T::Pattern, T::Fact],
                &[S::ScanTerminal, S::ScanFact],
            );
            operations(&mut ir, &[S::LeftJoin, S::CaseTruth, S::Projection]);
        }
        "violation" => lower_violation(source, &mut ir),
        other => return Err(format!("unknown relational contract operation `{other}`")),
    }
    ir.validate()?;
    Ok(ir)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ContractExpr {
    Atom(String),
    Call {
        operation: String,
        arguments: Vec<Self>,
    },
    Then {
        left: Box<Self>,
        right: Box<Self>,
    },
}

impl ContractExpr {
    fn parse(source: &str) -> Result<Self, String> {
        parse_expr(source.trim())
    }

    fn operation(&self) -> &str {
        match self {
            Self::Atom(value) => value,
            Self::Call { operation, .. } => operation,
            Self::Then { .. } => "then",
        }
    }
}

fn parse_expr(source: &str) -> Result<ContractExpr, String> {
    if source.is_empty() {
        return Err("empty relational form".to_owned());
    }
    if let Some(index) = find_top_level(source, " then ")? {
        return Ok(ContractExpr::Then {
            left: Box::new(parse_expr(&source[..index])?),
            right: Box::new(parse_expr(&source[index + " then ".len()..])?),
        });
    }
    let Some(open) = source.find('(') else {
        return Ok(ContractExpr::Atom(source.to_owned()));
    };
    if !source.ends_with(')') {
        return Err(format!("unclosed call in `{source}`"));
    }
    let operation = source[..open].trim();
    if operation.is_empty() {
        return Err(format!("missing operation in `{source}`"));
    }
    let arguments = split_arguments(&source[open + 1..source.len() - 1])?
        .into_iter()
        .map(parse_expr)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ContractExpr::Call {
        operation: operation.to_owned(),
        arguments,
    })
}

fn find_top_level(source: &str, needle: &str) -> Result<Option<usize>, String> {
    let mut depth = 0_u32;
    for (index, character) in source.char_indices() {
        match character {
            '(' => depth += 1,
            ')' if depth == 0 => return Err(format!("unexpected `)` in `{source}`")),
            ')' => depth -= 1,
            _ if depth == 0 && source[index..].starts_with(needle) => return Ok(Some(index)),
            _ => {}
        }
    }
    if depth == 0 {
        Ok(None)
    } else {
        Err(format!("unclosed `(` in `{source}`"))
    }
}

fn split_arguments(source: &str) -> Result<Vec<&str>, String> {
    if source.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut arguments = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    for (index, character) in source.char_indices() {
        match character {
            '(' => depth += 1,
            ')' if depth == 0 => return Err(format!("unexpected `)` in `{source}`")),
            ')' => depth -= 1,
            ',' if depth == 0 => {
                arguments.push(source[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(format!("unclosed `(` in `{source}`"));
    }
    arguments.push(source[start..].trim());
    if arguments.iter().any(|argument| argument.is_empty()) {
        return Err(format!("empty argument in `{source}`"));
    }
    Ok(arguments)
}

fn lower_all(source: &str, ir: &mut RelationalIr) -> Result<(), String> {
    lower_terms(source, ir);
    if source.contains("matches(shared)") {
        terminal(ir, T::Pattern, &[S::ScanTerminal]);
    }
    if source.contains("ne(") || source.contains("not(") {
        ir.operation(S::InnerJoin);
        ir.operation(S::AntiJoin);
    } else if source.contains("unknown") && source.contains("compare") {
        ir.operation(S::LeftJoin);
    } else {
        ir.operation(S::InnerJoin);
    }
    if source.contains("any(") {
        insert_before(ir, S::InnerJoin, &[S::OrderedUnion]);
        ir.operation(S::WindowRank);
    }
    if source.contains("unknown") {
        ir.operation(S::CaseTruth);
    }
    if source.contains('$') || source == "all(kind, regex)" {
        ir.operation(S::Bindings);
    }
    Ok(())
}

fn lower_any(source: &str, ir: &mut RelationalIr) -> Result<(), String> {
    lower_terms(source, ir);
    if source.contains("call(util") {
        terminal(ir, T::Pattern, &[S::ScanTerminal, S::ScanCapture]);
    }
    operations(ir, &[S::OrderedUnion, S::WindowRank]);
    if source.contains("unknown") {
        ir.operation(S::CaseTruth);
    }
    if source.contains('$') || source.contains("pattern") {
        ir.operation(S::Bindings);
    }
    Ok(())
}

fn lower_terms(source: &str, ir: &mut RelationalIr) {
    let mut found = [
        ("kind", T::Kind),
        ("regex", T::Regex),
        ("pattern", T::Pattern),
        ("value", T::Value),
        ("compare", T::Equality),
        ("eq(", T::Equality),
        ("ne(", T::Equality),
    ]
    .into_iter()
    .filter_map(|(needle, terminal_kind)| {
        source
            .find(needle)
            .map(|position| (position, terminal_kind))
    })
    .collect::<Vec<_>>();
    found.sort_by_key(|(position, _)| *position);
    for (_, terminal_kind) in found {
        ir.terminal(terminal_kind);
    }
    if !ir.terminals().is_empty() {
        ir.operation(S::ScanTerminal);
        if source.contains('$') {
            ir.operation(S::ScanCapture);
        }
    }
}

fn lower_navigation(source: &str, ir: &mut RelationalIr) {
    let axis = source.split_once('(').map_or(source, |(axis, _)| axis);
    terminal_spec(ir, T::Navigation, axis, &[S::ScanNavigation]);
    terminal(ir, T::Kind, &[S::ScanTerminal]);
    ir.operation(S::InnerJoin);
    if source.contains("stopBy=pattern") {
        ir.terminal(T::Pattern);
        ir.operation(S::AntiJoin);
    }
    if source.contains("field=") {
        ir.operation(S::WhereFilter);
    } else if source.starts_with("has") && source.contains("stopBy=end") {
        ir.operation(S::Exists);
    } else {
        ir.operation(S::MinPosition);
    }
}

fn lower_matches(source: &str, ir: &mut RelationalIr) {
    if source.contains("recursive") || source.contains("a -> b -> a") {
        terminal(ir, T::Navigation, &[S::ScanNavigation]);
        if source.contains("self") || source.contains("a ->") {
            terminal(ir, T::Kind, &[S::ScanTerminal]);
        }
        ir.operation(S::RecursiveCte);
        if source.contains("a ->") {
            ir.operation(S::OrderedUnion);
        }
        ir.operation(S::CaseTruth);
    } else if source.contains("outer -> inner") {
        terminals(ir, &[T::Kind, T::Regex], &[S::ScanTerminal, S::InnerJoin]);
    } else if source.contains("util-binds") {
        terminal(ir, T::Pattern, &[S::ScanTerminal, S::ScanCapture]);
        operations(ir, &[S::InnerJoin, S::Bindings]);
    } else if source.contains("simple-kind") {
        terminal(ir, T::Kind, &[S::ScanTerminal, S::InnerJoin]);
    } else {
        terminal(ir, T::Pattern, &[S::ScanTerminal, S::InnerJoin]);
    }
}

fn semantic_binding(ir: &mut RelationalIr, rule: T, join: S, ranked: bool) {
    terminal(ir, T::Fact, &[S::ScanFact]);
    terminal(ir, rule, &[S::ScanTerminal]);
    ir.operation(join);
    if ranked {
        ir.operation(S::WindowRank);
    }
    ir.operation(S::Bindings);
}

fn lower_forbid(source: &str, ir: &mut RelationalIr) {
    if source.contains("regex(path)") {
        terminal(ir, T::Regex, &[S::FileFilter, S::ScanTerminal]);
    } else if source.contains("regex(content)") {
        terminal(ir, T::Regex, &[S::ScanTerminal]);
    } else {
        terminal(ir, T::Pattern, &[S::ScanTerminal]);
    }
    if source.contains("no terminal matches") {
        ir.operation(S::AntiJoin);
    }
    ir.operation(S::Projection);
    if !source.contains("no terminal matches") {
        ir.operation(S::OrderBy);
    }
}

fn lower_violation(source: &str, ir: &mut RelationalIr) {
    terminal(ir, T::Pattern, &[S::ScanTerminal, S::ScanCapture]);
    if source.contains("fix") {
        ir.terminal(T::Fact);
        ir.terminal(T::Transform);
        ir.operation(S::ScanFact);
        ir.operation(S::AntiJoin);
    } else {
        operations(ir, &[S::OrderedUnion, S::Distinct]);
    }
    ir.operation(S::Projection);
    if source.contains("each") {
        ir.operation(S::OrderBy);
    }
}

fn terminal(ir: &mut RelationalIr, terminal_kind: T, sql: &[S]) {
    ir.terminal(terminal_kind);
    operations(ir, sql);
}

fn terminal_spec(ir: &mut RelationalIr, terminal_kind: T, semantic_key: &str, sql: &[S]) {
    ir.terminal_spec(terminal_kind, semantic_key);
    operations(ir, sql);
}

fn terminals(ir: &mut RelationalIr, terminal_kinds: &[T], sql: &[S]) {
    for terminal_kind in terminal_kinds {
        ir.terminal(*terminal_kind);
    }
    operations(ir, sql);
}

fn operations(ir: &mut RelationalIr, operations: &[S]) {
    for operation in operations {
        ir.operation(*operation);
    }
}

fn insert_before(ir: &mut RelationalIr, before: S, operations: &[S]) {
    ir.insert_operations_before(before, operations)
        .expect("the compact form created the target relation");
}

#[cfg(test)]
mod tests {
    use super::{ContractExpr, lower};

    #[test]
    fn parses_nested_and_infix_contract_forms() {
        let parsed = ContractExpr::parse("pattern($A) then bind($A)").expect("valid form");
        assert_eq!(parsed.operation(), "then");
        let parsed = ContractExpr::parse("all(pattern($A), not(kind(identifier)))")
            .expect("valid nested form");
        assert_eq!(parsed.operation(), "all");
    }

    #[test]
    fn rejects_invalid_contract_syntax_and_unknown_operations() {
        assert!(lower("all(kind, regex").is_err());
        assert!(lower("all(kind), regex)").is_err());
        assert!(lower("invented(kind)").is_err());
    }

    #[test]
    fn relational_ir_preserves_terminal_and_navigation_meaning() {
        assert_ne!(
            lower("pattern(named terminal)").expect("named pattern"),
            lower("pattern(internal tree)").expect("structural pattern"),
            "different terminal specifications must not collapse to one feature list"
        );
        assert_ne!(
            lower("has(kind, stopBy=neighbor)").expect("descendant relation"),
            lower("inside(kind, stopBy=neighbor)").expect("ancestor relation"),
            "different navigation axes must remain explicit in relational IR"
        );
    }
}
