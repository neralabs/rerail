//! A migration tool, never an execution fallback. An unsupported clause refuses the whole candidate.
//! Converted candidates still need the full original assertion suite before acceptance.
use super::*;
use crate::code_rule_dsl::{CodeRuleDslRuntime, model::{Anchor, ClauseKind, Query, Template, TemplatePart}};

pub(in crate::code_rule_dsl) fn policy(source: &str) -> Result<serde_json::Value, String> {
    let mut document: serde_json::Value = serde_json::from_str(source).map_err(|e| e.to_string())?;
    let id = document["id"].as_str().ok_or("policy id missing")?.to_owned();
    let mut gaps = Vec::<serde_json::Value>::new();
    if let Some(clauses) = document["clauses"].as_array_mut() {
        clauses.retain(|clause| {
            if clause.get("relational").is_some() {
                gaps.push(serde_json::json!({"clause": clause["id"], "reason": "relational SQL requires a handwritten port"}));
                false
            } else { true }
        });
    }
    let reports = document["clauses"].as_array().into_iter().flatten().any(|c| c.get("define").is_none());
    if !reports {
        return Ok(serde_json::json!({"id": id, "status": "needs-manual-port", "gaps": gaps, "program": null}));
    }
    let compiled = CodeRuleDslRuntime::compile(document.to_string(), "json".to_owned(), crate::code_rule_dsl::tools::CAPABILITIES.to_owned()).map_err(|e| e.reason)?;
    let dialect = if id.contains("rust-sql") { GrammarDialect::Rust } else { GrammarDialect::Python };
    let mut emitter = Emitter::new(dialect);
    let mut findings = Vec::new();
    let mut converted = 0;
    for (index, clause) in compiled.inner.clauses.iter().enumerate() {
        let saved = emitter.clone();
        let result = (|| -> Res<()> {
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind else { return gap("non-AST-forbid clause") };
            let grammar = matchers.iter().find(|m| m.language == dialect && m.applies).ok_or_else(|| Gap("no matcher in the policy's language".into()))?;
            if grammar.document.is_some() { return gap("embedded-language matcher") }
            if clause.violation.each.is_some() || clause.violation.group_transform.is_some() || clause.violation.fix.is_some() {
                return gap("per-capture multiplicity, transform or fix requires explicit translation")
            }
            emitter.captures.clear();
            let mut body = emitter.body(&grammar.matcher, "$n")?;
            body = nonempty(body, "$n");
            body.push("docof($n, $d)".to_owned());
            if let Some(file) = &grammar.file {
                if file.module_name.is_some() || file.package.is_some() || file.in_package.is_some() || file.manifest.is_some() || file.declaration.is_some() {
                    return gap("module/package/manifest/declaration condition")
                }
                body.push("doc($d, $p, $m)".to_owned());
                if let Some(c) = &file.path { body.extend(emitter.condition(c, "$p")?); }
                if let Some(c) = &file.module { body.extend(emitter.condition(c, "$m")?); }
            }
            if let Some(invalid) = grammar.invalid_syntax {
                body.push(format!("{}syntax_error($d, _, _)", if invalid { "" } else { "!" }));
            }
            let at = match &clause.violation.anchor {
                Anchor::Span | Anchor::Start => "$n".to_owned(),
                Anchor::Capture(name) => emitter.captures.get(name).cloned().ok_or_else(|| Gap(format!("unbound anchor {name}")))?,
                _ => return gap("file-start/end anchor requires explicit translation"),
            };
            let mut message = String::new();
            let mut columns = vec!["at: node".to_owned()];
            let mut args = vec![at];
            let template = Template::parse_message(&clause.violation.message);
            let mut bound = BTreeSet::new();
            for part in template.parts() {
                match part {
                    TemplatePart::Text(text) => message.push_str(&text.replace('{', "{{").replace('}', "}}")),
                    TemplatePart::Var { name, multi } => {
                        if multi { return gap("list capture in message") }
                        let value = emitter.captures.get(name).ok_or_else(|| Gap(format!("unbound message capture {name}")))?;
                        if bound.insert(name.to_owned()) { columns.push(format!("{name}: node")); args.push(value.clone()); }
                        message.push_str(&format!("{{{name}}}"));
                    }
                }
            }
            let name = format!("finding_{index}");
            let mut text = format!("// Original clause: {}\n.violation {name}({}) code {} {}\n", clause.id, columns.join(", "), quote(&clause.violation.code), quote(&message));
            if let Some(suppress) = &clause.violation.suppress {
                // Suppression shape is checked separately; never silently discard it.
                return gap(format!("suppression directive {suppress:?}"))
            }
            text += &format!("{name}({}) :- {}.\n", args.join(", "), body.join(", "));
            findings.push(text);
            Ok(())
        })();
        match result {
            Ok(()) => converted += 1,
            Err(error) => { emitter = saved; gaps.push(serde_json::json!({"clause": clause.id, "reason": error.0})); }
        }
    }
    if !gaps.is_empty() {
        return Ok(serde_json::json!({"id": id, "status": "needs-manual-port", "clausesConverted": converted, "gaps": gaps, "approximations": emitter.approx, "program": null}));
    }
    // The old emitter prints base declarations; public files read the engine-owned ones from core.rr.
    let old = emitter.program(&[]);
    let base: BTreeSet<&str> = BASE_RELATIONS.iter().copied().collect();
    let body = old.lines().filter(|line| !line.starts_with(".input ") && !line.strip_prefix(".decl ").and_then(|l| l.split('(').next()).is_some_and(|name| base.contains(name))).collect::<Vec<_>>().join("\n");
    let body = crate::code_rule_dsl::logic::rr::translate(&body).map_err(|e| e.to_string())?;
    let mut program = format!(".policy {} revision {}\n.description {}\n.language {}\n.mode {}\n", quote(&id), document["revision"].as_u64().unwrap_or(1), quote(document["description"].as_str().unwrap_or_default()), if dialect == GrammarDialect::Rust { "rust" } else { "python" }, document["mode"].as_str().unwrap_or("block"));
    for key in ["include", "exclude"] {
        if let Some(globs) = document["scope"][key].as_array() {
            program += &format!(".scope {key} {}\n", globs.iter().filter_map(|v| v.as_str()).map(quote).collect::<Vec<_>>().join(", "));
        }
    }
    program += &body;
    program.push('\n');
    for finding in findings { program += &finding; }
    Ok(serde_json::json!({"id": id, "status": "draft", "clausesConverted": converted, "gaps": [], "approximations": emitter.approx, "program": program}))
}
