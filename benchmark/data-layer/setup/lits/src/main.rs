// Data helper for the clause cover (no engine): native-identical regex literal extraction
// (RequiredText::regex_clause), Rust-regex path tests, globset scopes, literal presence per file.
use aho_corasick::AhoCorasick;
use globset::{GlobBuilder, GlobSetBuilder};
use regex_syntax::hir::literal::Extractor;
use serde_json::{json, Value};
use std::io::Read;

fn regex_clause(pattern: &str) -> Value {
    let Ok(hir) = regex_syntax::parse(pattern) else { return json!({"error": "parse"}) };
    let seq = Extractor::new().extract(&hir);
    let Some(lits) = seq.literals() else { return Value::Null };
    if lits.is_empty() || lits.len() > 16 { return Value::Null; }
    let mut out = Vec::new();
    for l in lits {
        match std::str::from_utf8(l.as_bytes()) {
            Ok(t) if !t.is_empty() => out.push(t.to_owned()),
            _ => return Value::Null,
        }
    }
    json!(out)
}

fn globset(patterns: &Value) -> globset::GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns.as_array().unwrap() {
        b.add(GlobBuilder::new(p.as_str().unwrap()).literal_separator(true).build().unwrap());
    }
    b.build().unwrap()
}

fn main() {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).unwrap();
    let v: Value = serde_json::from_str(&s).unwrap();
    let paths: Vec<&str> = v["paths"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
    let extract: Vec<Value> = v["extract"].as_array().unwrap().iter()
        .map(|p| regex_clause(p.as_str().unwrap())).collect();
    let path_match: Vec<Value> = v["path_regexes"].as_array().unwrap().iter().map(|p| {
        match regex::Regex::new(p.as_str().unwrap()) {
            Ok(re) => json!(paths.iter().enumerate().filter(|(_, p)| re.is_match(p)).map(|(i, _)| i).collect::<Vec<_>>()),
            Err(e) => json!({"error": e.to_string()}),
        }
    }).collect();
    let glob_match: Vec<Value> = v["globs"].as_array().unwrap().iter().map(|g| {
        let inc = globset(&g["include"]);
        let exc = globset(&g["exclude"]);
        json!(paths.iter().enumerate().filter(|(_, p)| inc.is_match(p) && !exc.is_match(p)).map(|(i, _)| i).collect::<Vec<_>>())
    }).collect();
    // presence[l] = indices of files whose bytes contain literal l (memmem semantics).
    let lits: Vec<String> = v["literals"].as_array().unwrap().iter().map(|l| l.as_str().unwrap().to_owned()).collect();
    let mut presence: Vec<Vec<usize>> = vec![Vec::new(); lits.len()];
    if !lits.is_empty() {
        let root = v["root"].as_str().unwrap();
        let ac = AhoCorasick::new(&lits).unwrap();
        let mut seen = vec![usize::MAX; lits.len()];
        for (i, p) in paths.iter().enumerate() {
            let bytes = std::fs::read(format!("{root}/{p}")).unwrap();
            for m in ac.find_overlapping_iter(&bytes) {
                let l = m.pattern().as_usize();
                if seen[l] != i { seen[l] = i; presence[l].push(i); }
            }
        }
    }
    // content_match[r] = indices of files whose text the regex matches (file.content source).
    let content_match: Vec<Value> = v["content_regexes"].as_array().map(|rs| rs.iter().map(|p| {
        let re = regex::Regex::new(p.as_str().unwrap()).unwrap();
        let root = v["root"].as_str().unwrap();
        json!(paths.iter().enumerate().filter(|(_, p)| {
            std::fs::read_to_string(format!("{root}/{p}")).map(|t| re.is_match(&t)).unwrap_or(false)
        }).map(|(i, _)| i).collect::<Vec<_>>())
    }).collect::<Vec<_>>()).unwrap_or_default();
    println!("{}", json!({"extract": extract, "path_match": path_match, "glob_match": glob_match, "presence": presence, "content_match": content_match}));
}
