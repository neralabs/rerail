//! The converter's run over the data-layer pack, and checks of what it writes.

use super::*;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::test_support::CAPABILITIES;
use std::path::{Path, PathBuf};

fn pack() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../gate/data-layer/packs/data-layer/candidates")
}

/// What one rule's conversion gave: its program, and per clause whether it converted and why not.
struct Converted {
    program: String,
    clauses: Vec<(String, Result<(), Gap>)>,
    approx: BTreeSet<String>,
}

fn scope_globs(document: &serde_json::Value, key: &str) -> Vec<String> {
    document.get("scope").and_then(|s| s.get(key)).and_then(|v| v.as_array()).into_iter().flatten().filter_map(|g| g.as_str().map(str::to_owned)).collect()
}

/// One rule file as a program: its scope as path globs and each forbid clause's matcher as rules deriving
/// `violation(rule, clause, path, line)`.
fn convert(path: &Path) -> Converted {
    let text = std::fs::read_to_string(path).unwrap();
    let document: serde_json::Value = serde_json::from_str(&text).unwrap();
    let id = document["id"].as_str().unwrap().to_owned();
    let compiled = CodeRuleDslRuntime::compile(text, "json".to_owned(), CAPABILITIES.to_owned()).unwrap_or_else(|e| panic!("{id}: {e:?}"));
    let mut e = Emitter::new(GrammarDialect::Python);
    // The scope: a document in an include glob and in no exclude glob.
    for g in scope_globs(&document, "include") {
        e.rule("in_scope($d)", "d: int", vec!["doc($d, $p, _)".into(), format!("@glob($p, {})", quote(&g))]);
    }
    for g in scope_globs(&document, "exclude") {
        e.rule("out_scope($d)", "d: int", vec!["doc($d, $p, _)".into(), format!("@glob($p, {})", quote(&g))]);
    }
    let excludes = !scope_globs(&document, "exclude").is_empty();
    e.decls.insert("violation".into(), "rule: sym, clause: sym, path: sym, line: int".into());
    let mut clauses = Vec::new();
    for clause in &compiled.inner.clauses {
        let result = (|| -> Res<()> {
            let matchers = match &clause.kind {
                ClauseKind::Forbid(Query::Ast { matchers, .. }) => matchers,
                ClauseKind::Forbid(Query::Regex { .. }) => return gap("a regex over file content or paths"),
                ClauseKind::Forbid(Query::SelectWhen { .. }) => return gap("select/when"),
                ClauseKind::RequireEach { .. } => return gap("require each (define)"),
                ClauseKind::Budget { .. } => return gap("budget"),
                _ => return gap("another clause kind"),
            };
            let Some(grammar) = matchers.iter().find(|m| m.language == GrammarDialect::Python) else { return gap("no Python matcher") };
            if !grammar.applies {
                return Ok(());
            }
            if grammar.document.is_some() {
                return gap("an embedded program");
            }
            e.captures.clear();
            let mut body = e.body(&grammar.matcher, "$n")?;
            body = nonempty(body, "$n");
            if let Some(file) = &grammar.file {
                if file.module_name.is_some() || file.package.is_some() || file.in_package.is_some() || file.manifest.is_some() || file.declaration.is_some() {
                    return gap("a file condition on module names or packages");
                }
                body.push("doc($d, $p, $m)".into());
                if let Some(c) = &file.path {
                    body.extend(e.condition(c, "$p")?);
                }
                if let Some(c) = &file.module {
                    body.extend(e.condition(c, "$m")?);
                }
            }
            body.extend(["node($d, $n, _, _, _)".to_owned(), "in_scope($d)".to_owned()]);
            if excludes {
                body.push("!out_scope($d)".into());
            }
            body.extend(["doc($d, $p, _)".to_owned(), "line($n, $l)".to_owned()]);
            e.rules.push(format!("violation({}, {}, $p, $l) :- {}.", quote(&id), quote(&clause.id), body.join(", ")));
            Ok(())
        })();
        clauses.push((clause.id.clone(), result));
    }
    Converted { program: e.program(&["violation"]), clauses, approx: e.approx.clone() }
}

fn rule_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(pack().join("rules")).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files
}

/// The converter's run: every rule of the pack as a program in `candidates/datalog/<rule>.rdl`, and the
/// coverage (`coverage.txt`): clauses converted, the gaps by kind, the approximations.
#[test]
#[ignore = "writes the converted rules into the pack"]
fn convert_the_data_layer_pack() {
    let out = pack().join("datalog");
    std::fs::create_dir_all(&out).unwrap();
    let (mut total, mut done) = (0, 0);
    let mut gaps = BTreeMap::<String, usize>::new();
    let mut approx = BTreeMap::<String, usize>::new();
    let mut per_rule = Vec::new();
    for path in rule_files() {
        let id = path.file_stem().unwrap().to_string_lossy().into_owned();
        let c = convert(&path);
        crate::code_rule_dsl::logic::front::parse(&c.program).unwrap_or_else(|e| panic!("{id}: {e:?}\n{}", c.program));
        std::fs::write(out.join(format!("{id}.rdl")), &c.program).unwrap();
        let ok = c.clauses.iter().filter(|(_, r)| r.is_ok()).count();
        total += c.clauses.len();
        done += ok;
        for (_, r) in &c.clauses {
            if let Err(Gap(g)) = r {
                // Gaps by kind: the text before any detail.
                let kind = if g.starts_with("a regex on the text") { g.clone() } else { g.split(" of ").next().unwrap_or(g).to_owned() };
                *gaps.entry(kind).or_default() += 1;
            }
        }
        for a in &c.approx {
            *approx.entry(a.clone()).or_default() += 1;
        }
        per_rule.push(format!("{id}: {ok}/{}", c.clauses.len()));
    }
    let mut report = format!("{done}/{total} clauses converted\n\ngaps (clauses):\n");
    let mut by_count: Vec<_> = gaps.into_iter().collect();
    by_count.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (g, n) in by_count {
        report.push_str(&format!("  {n:5}  {g}\n"));
    }
    report.push_str("\napproximations (rules):\n");
    for (a, n) in approx {
        report.push_str(&format!("  {n:5}  {a}\n"));
    }
    report.push_str("\nper rule:\n");
    for r in per_rule {
        report.push_str(&format!("  {r}\n"));
    }
    std::fs::write(out.join("coverage.txt"), &report).unwrap();
    println!("{report}");
}

#[test]
fn regexes_become_names_and_string_tests() {
    let parse = |s: &str| regex_syntax::ParserBuilder::new().build().parse(s).unwrap();
    assert!(matches!(shape(&parse("(?s).*")), Some(Shape::Anything)));
    assert!(matches!(shape(&parse(r"\b\B")), Some(Shape::Nothing)));
    let Some(Shape::Names(n)) = shape(&parse("^(?:lazy|pure_lazy)$")) else { panic!() };
    assert_eq!(n.into_iter().collect::<Vec<_>>(), ["lazy", "pure_lazy"]);
    let Some(Shape::Tests(t)) = shape(&parse("_lazy$")) else { panic!() };
    assert_eq!(t, [("suffix", "_lazy".to_owned())]);
    let Some(Shape::Tests(t)) = shape(&parse("^_")) else { panic!() };
    assert_eq!(t, [("prefix", "_".to_owned())]);
    let Some(Shape::Tests(t)) = shape(&parse("Connection|Transaction")) else { panic!() };
    assert_eq!(t, [("contains", "Connection".to_owned()), ("contains", "Transaction".to_owned())]);
    let Some(Shape::Tests(t)) = shape(&parse("(?i)validat")) else { panic!() };
    assert_eq!(t, [("icontains", "validat".to_owned())]);
    let Some(Shape::Names(n)) = shape(&parse(r"^(asyncio\.)?(run|to_thread)$")) else { panic!() };
    assert_eq!(n.into_iter().collect::<Vec<_>>(), ["asyncio.run", "asyncio.to_thread", "run", "to_thread"]);
    assert!(shape(&parse(r"Callable\s*\[")).is_none());
}
