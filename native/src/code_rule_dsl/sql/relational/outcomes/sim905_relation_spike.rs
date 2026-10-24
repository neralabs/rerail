//! Result-only SIM905 relational reference. This deliberately permits speculative
//! atomic integer evaluation; it does not claim work/memo/probe equivalence.
use super::{Compiled, Op, lower::Lower};
use crate::code_rule_dsl::{
    matcher::{AstMatcher, GrammarMatcher},
    sql::terminals::Terminal,
};
use serde_json::json;
use std::collections::HashSet;
fn value_leaf(m: &AstMatcher, seen: &mut HashSet<(usize, usize)>) -> Option<AstMatcher> {
    use crate::code_rule_dsl::matcher::Semantic;
    match m {
        AstMatcher::Semantic(Semantic::Value(_)) => Some(m.clone()),
        AstMatcher::All(v) | AstMatcher::Any(v) => v.iter().find_map(|m| value_leaf(m, seen)),
        AstMatcher::Not(m) => value_leaf(m, seen),
        AstMatcher::Has(r, _)
        | AstMatcher::Inside(r, _)
        | AstMatcher::Follows(r, _)
        | AstMatcher::Precedes(r, _) => value_leaf(r.rule(), seen),
        AstMatcher::Util(u) => {
            let key = (u.table.as_ptr() as usize, u.index);
            if !seen.insert(key) {
                return None;
            }
            u.with_target(|m| value_leaf(m, seen)).flatten()
        }
        _ => None,
    }
}
pub(super) fn compile(grammar: &GrammarMatcher) -> Result<Compiled, String> {
    let atom = value_leaf(&grammar.matcher, &mut HashSet::new())
        .ok_or("SIM905 reference needs original atomic integer Value leaf")?;
    let mut lower = Lower::new(grammar.language, grammar.three_valued);
    let plan = lower.plan(&atom)?;
    let Op::Value(value) = plan.op else {
        return Err("SIM905 reference expected capture-free atomic Value".into());
    };
    let mut terminals = lower.pure.program.terminals;
    let mut sql =
        include_str!("sim905_relation_spike.sql").replace("@integer_value", &value.to_string());
    for kind in [
        "call",
        "attribute",
        "identifier",
        "comment",
        "argument_list",
        "keyword_argument",
        "list_splat",
        "dictionary_splat",
        "parenthesized_expression",
        "unary_operator",
        "string_start",
        "concatenated_string",
        "string",
        "integer",
        "none",
    ] {
        sql = sql.replace(
            &format!("@{kind}"),
            &grammar.language.kinds().kind_id(kind).to_string(),
        );
    }
    for (name, regex) in [
        ("dot", r"^\.$"),
        ("method", r"^(split|rsplit)$"),
        ("punct", r"^[\[\](){},]$"),
        ("open", r"^\($"),
        ("comma", r"^,$"),
        ("sep", r"^sep$"),
        ("maxsplit", r"^maxsplit$"),
        ("equals", r"^=$"),
        ("badprefix", r"^[A-Za-z]*[bBfFtT]"),
        ("minus", r"^-$"),
    ] {
        let id = terminals.len();
        terminals.push(Terminal::Regex(
            regex::Regex::new(regex).map_err(|e| e.to_string())?,
        ));
        sql = sql.replace(&format!("@{name}"), &id.to_string());
    }
    sql = sql.replace("@integer_value", &value.to_string()).replace(
        "@roots",
        &format!(
            "{} AND ({})",
            usize::from(grammar.applies),
            super::root_filter(grammar, None)
        ),
    );
    if sql.contains('@') {
        return Err("SIM905 reference unresolved SQL placeholder".into());
    }
    let owners = vec![0; terminals.len()];
    Ok(Compiled {
        sql,
        terminals,
        relations: json!({"captures":[],"terminalOwners":owners,"spikeContract":"result-only; speculative atomic integer probes; no work/memo/depth equivalence","atomicValueTerm":value,"physical":"shared demanded helper closure; no native rule body"}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        CodeRuleDslRuntime,
        sql::relational::cutover_tests::execution::{Fixture, check, fixtures},
        test_support::{CAPABILITIES, grammar_matchers},
    };
    fn policy() -> crate::code_rule_dsl::CompiledCodeRuleDsl {
        CodeRuleDslRuntime::compile(
            include_str!(
                "../../../../../../packs/python/rules/ruff-sim905-split-static-string.json"
            )
            .into(),
            "json".into(),
            CAPABILITIES.into(),
        )
        .unwrap()
    }
    #[test]
    fn structural_spike_sim905_full_catalog_result_parity_and_gate() {
        let p = policy();
        let g = &grammar_matchers(&p)[0];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let batch = fixtures(root);
        assert_eq!(batch.len(), 597);
        let report = check(g, compile(g).unwrap(), &batch).unwrap();
        if let Ok(path) = std::env::var("RERAIL_SIM905_RELATION_SPIKE_EVIDENCE") {
            std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
        assert_eq!(report["passed"], true, "{report}");
        assert!(report["vm"].as_u64().unwrap() <= 44_908_386, "{report}");
        assert!(
            report["typedReturnRows"].as_u64().unwrap() <= 158_186,
            "{report}"
        );
    }
    #[test]
    fn structural_spike_sim905_translated_argument_and_parenthesis_semantics() {
        let p = policy();
        let g = &grammar_matchers(&p)[0];
        let cases = [
            "'a b'.split()",
            "( ('a b') ).rsplit(None,(-2))",
            "b'a'.split()",
            "f'a'.split()",
            "'a'.split(sep=x)",
            "'a'.split(maxsplit=2147483648)",
            "'a'.split(maxsplit=-2147483648)",
            "'a'.split(*(x,))",
            "'a'.split(sep=None,sep=x)",
            "'a'.split(maxsplit=1,maxsplit=x)",
            "('a' # comment\n).split( # one\n None, #two\n -2)",
            "'a'.split(unknown=thing)",
            "'a'.split(None,1,third)",
            "'a'.split(maxsplit=(+2))",
            "('a' 'b').split()",
        ];
        let batch = cases
            .iter()
            .enumerate()
            .map(|(i, s)| Fixture {
                path: format!("spike/{i}.py"),
                source: (*s).into(),
            })
            .collect::<Vec<_>>();
        let report = check(g, compile(g).unwrap(), &batch).unwrap();
        assert_eq!(report["passed"], true, "{report}");
    }
}
