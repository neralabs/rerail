//! Admission proof for the fixed-source experiment, independent of allocation
//! identities. This module is test-only and never selects production SQL.
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    matcher::{AstMatcher, GrammarMatcher, Semantic, StopBy},
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};
struct Numeric {
    origin: Vec<u8>,
    framed: bool,
    shape: Value,
}
struct Expected {
    root: Value,
    numeric: BTreeMap<String, Numeric>,
    language: String,
    three: bool,
    applies: bool,
    sound: bool,
    anchors: Option<Vec<u16>>,
}
fn descriptor(m: &AstMatcher, g: &GrammarMatcher) -> Result<Value, String> {
    let related = |tag: &str,
                   r: &crate::code_rule_dsl::matcher::Related,
                   stop: &StopBy|
     -> Result<Value, String> {
        let by = match stop {
            StopBy::Neighbor => json!(["neighbor"]),
            StopBy::End => json!(["end"]),
            StopBy::Rule(m) => json!(["rule", descriptor(m, g)?]),
        };
        Ok(json!([tag, descriptor(r.rule(), g)?, by]))
    };
    Ok(match m {
        AstMatcher::Kind(_) => {
            let p = super::super::lower::Lower::new(g.language, true).plan(m)?;
            let super::super::Op::Predicate(super::super::Set::Kinds(k)) = p.op else {
                return Err("source kind descriptor unavailable".into());
            };
            json!(["kind", k])
        }
        AstMatcher::Regex(r) => json!(["regex", r.as_str()]),
        AstMatcher::Never => json!(["never"]),
        AstMatcher::All(v) => json!([
            "all",
            v.iter()
                .map(|m| descriptor(m, g))
                .collect::<Result<Vec<_>, _>>()?
        ]),
        AstMatcher::Any(v) => json!([
            "any",
            v.iter()
                .map(|m| descriptor(m, g))
                .collect::<Result<Vec<_>, _>>()?
        ]),
        AstMatcher::Not(m) => json!(["not", descriptor(m, g)?]),
        AstMatcher::Has(r, s) => related("has", r, s)?,
        AstMatcher::Inside(r, s) => related("inside", r, s)?,
        AstMatcher::Follows(r, s) => related("follows", r, s)?,
        AstMatcher::Precedes(r, s) => related("precedes", r, s)?,
        AstMatcher::Util(u) => {
            if u.table.as_ptr() != std::sync::Arc::as_ptr(&g.utils) {
                return Err("external utility source outside reference".into());
            }
            json!([
                "utility",
                g.utils.names.get(u.index).ok_or("utility index missing")?
            ])
        }
        AstMatcher::Semantic(Semantic::Value(v)) => {
            json!(["value-reading",{"language":v.reading.language.to_string(),"expressions":v.reading.expressions.iter().copied().collect::<Vec<_>>(),"follow":v.reading.follow,"literal":v.reading.literal,"languageOptions":v.reading.options.is_some(),"pythonOptions":v.reading.python.is_some()}])
        }
        _ => return Err("source operator outside reference descriptor".into()),
    })
}
fn numeric(g: &GrammarMatcher, name: &str) -> Result<Numeric, String> {
    let index = g
        .utils
        .names
        .iter()
        .position(|n| n == name)
        .ok_or("numeric utility missing")?;
    let origin = g
        .utils
        .origins
        .get(index)
        .and_then(Option::as_ref)
        .ok_or("numeric source origin missing")?
        .digest
        .to_vec();
    let util = crate::code_rule_dsl::matcher::UtilRef {
        table: std::sync::Arc::downgrade(&g.utils),
        index,
    };
    let shape = util
        .with_target(|m| descriptor(m, g))
        .ok_or("numeric source unavailable")??;
    Ok(Numeric {
        origin,
        shape,
        framed: g.utils.framed[index],
    })
}
fn expected() -> Result<&'static Expected, String> {
    static EXPECTED: OnceLock<Result<Expected, String>> = OnceLock::new();
    EXPECTED
        .get_or_init(|| {
            let policy = CodeRuleDslRuntime::compile(
                include_str!(
                    "../../../../../../packs/python/rules/ruff-sim905-split-static-string.json"
                )
                .into(),
                "json".into(),
                CAPABILITIES.into(),
            )
            .map_err(|e| e.to_string())?;
            let g = &grammar_matchers(&policy)[0];
            let mut numbers = BTreeMap::new();
            for name in ["int32", "neg-int32", "maxsplit"] {
                numbers.insert(name.into(), numeric(g, name)?);
            }
            Ok(Expected {
                root: descriptor(&g.matcher, g)?,
                numeric: numbers,
                language: g.language.to_string(),
                three: g.three_valued,
                applies: g.applies,
                sound: g.sound_under_errors,
                anchors: g.anchors.root_kinds().map(|k| k.iter().copied().collect()),
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}
pub(in crate::code_rule_dsl::sql::relational::outcomes) fn validate_numeric(
    g: &GrammarMatcher,
) -> Result<(), String> {
    let e = expected()?;
    if g.file.is_some() || g.transform.is_some() {
        return Err("SIM905 reference rejects file/transform context".into());
    }
    if g.language.to_string() != e.language || g.three_valued != e.three || g.applies != e.applies {
        return Err("SIM905 reference numeric context differs".into());
    }
    for (name, want) in &e.numeric {
        let got = numeric(g, name)?;
        if got.origin != want.origin || got.framed != want.framed || got.shape != want.shape {
            return Err(format!("SIM905 reference numeric source differs: {name}"));
        }
    }
    Ok(())
}
pub(in crate::code_rule_dsl::sql::relational::outcomes) fn validate_root(
    g: &GrammarMatcher,
) -> Result<(), String> {
    validate_numeric(g)?;
    let e = expected()?;
    if descriptor(&g.matcher, g)? != e.root
        || g.sound_under_errors != e.sound
        || g.anchors
            .root_kinds()
            .map(|k| k.iter().copied().collect::<Vec<_>>())
            != e.anchors
    {
        return Err("SIM905 reference original normalized root semantics differ".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn compile(mutator: impl FnOnce(&mut Value)) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
        let mut source: Value = serde_json::from_str(include_str!(
            "../../../../../../packs/python/rules/ruff-sim905-split-static-string.json"
        ))
        .unwrap();
        mutator(&mut source);
        CodeRuleDslRuntime::compile(source.to_string(), "json".into(), CAPABILITIES.into()).unwrap()
    }
    #[test]
    fn source_proof_ignores_identities_but_refuses_changed_root_and_numeric_semantics() {
        for _ in 0..2 {
            let p = compile(|_| {});
            let g = &grammar_matchers(&p)[0];
            validate_root(g).unwrap();
        }
        let method = compile(|p| {
            p["clauses"][0]["forbid"]["ast"]["rule"]["has"]["all"][1]["has"]["regex"] =
                json!("^bogus$")
        });
        let g = &grammar_matchers(&method)[0];
        assert!(validate_root(g).is_err());
        assert!(super::super::super::sim905_full_spike::compile(&[g], 1000).is_err());
        assert!(validate_numeric(g).is_ok());
        let guard =
            compile(|p| p["clauses"][0]["forbid"]["ast"]["rule"]["has"]["stopBy"] = json!("end"));
        assert!(validate_root(&grammar_matchers(&guard)[0]).is_err());
        for change in ["bound", "kind", "minus"] {
            let p = compile(|p| {
                let u = &mut p["clauses"][0]["forbid"]["ast"]["utils"];
                match change {
                    "bound" => u["int32"]["any"][0]["value"]["le"] = json!(2147483646i64),
                    "kind" => u["int32"]["any"][0]["kind"] = json!("float"),
                    _ => u["neg-int32"]["any"][0]["has"]["regex"] = json!("^\\+$"),
                }
            });
            let g = &grammar_matchers(&p)[0];
            assert!(validate_numeric(g).is_err(), "changed numeric {change}");
            assert!(super::super::frontier(g).is_err());
            assert!(validate_root(g).is_err());
        }
        let helper =
            compile(|p| p["clauses"][0]["forbid"]["ast"]["rule"] = json!({"matches":"maxsplit"}));
        let g = &grammar_matchers(&helper)[0];
        validate_numeric(g).unwrap();
        assert!(validate_root(g).is_err());
        let file = compile(|p| {
            p["clauses"][0]["forbid"]["ast"]["file"] = json!({"module":{"regex":"^test$"}})
        });
        assert!(
            validate_root(&grammar_matchers(&file)[0])
                .unwrap_err()
                .contains("file/transform")
        );
        let transform = compile(|p| {
            let a = &mut p["clauses"][0]["forbid"]["ast"];
            a["rule"]["regex"] = json!("(?<X>.)");
            a["transform"] = json!({"L":{"convert":{"source":"$X","toCase":"lowerCase"}}});
        });
        assert!(
            validate_root(&grammar_matchers(&transform)[0])
                .unwrap_err()
                .contains("file/transform")
        );
    }
}
