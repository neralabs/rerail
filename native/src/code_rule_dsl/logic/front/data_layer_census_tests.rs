//! The census of the 60 data-layer rules in Rerail logic (t17): every construct each clause uses, and the 03 §12
//! category it falls in. Derived here from the rule files; `gate/data-layer/census/` holds the result, and the test
//! fails when the two differ. `RERAIL_BLESS_CENSUS=1` rewrites the derived files.
//!
//! - `clauses.tsv`: one row per clause: its kind, its categories other than `today`, and its constructs with counts.
//! - `constructs.tsv`: one row per construct: its category, and how many rules, clauses and occurrences use it.
//! - `rules.tsv`: one row per rule: clauses by kind, distinct constructs per category, and what blocks it.
//! - `recursion.tsv` (written by hand, checked here): one row per recursion (recursive CTE, reachability stage,
//!   evidence closure with edges), with its family, lattice and category; its keys must be exactly those derived.
//!
//! SQL is read at each option's default (`define.options.*.default`). A construct in a clause's final `query` is
//! the findings layer's (§8) unless Datalog has it today; inside a stage it is the logic's.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::{Path, PathBuf};

fn gate() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../gate/data-layer")
}

/// Matcher terms (keys of a rule object), each with its category.
const TERMS: [(&str, &str); 29] = [
    ("kind", "ext:matcher-atom"),
    ("pattern", "ext:matcher-atom"),
    ("regex", "ext:matcher-atom"),
    ("has", "ext:matcher-atom"),
    ("inside", "ext:matcher-atom"),
    ("precedes", "ext:matcher-atom"),
    ("follows", "ext:matcher-atom"),
    ("field", "ext:matcher-atom"),
    ("nthChild", "ext:matcher-atom"),
    ("all", "ext:matcher-atom"),
    ("any", "ext:matcher-atom"),
    ("not", "ext:matcher-atom"),
    ("matches", "ext:matcher-atom"),
    ("constraints", "ext:matcher-atom"),
    ("count", "ext:matcher-atom"),
    ("layout", "ext:matcher-atom"),
    ("compare", "ext:matcher-atom"),
    ("filePath", "ext:matcher-atom"),
    ("parse", "ext:matcher-atom"),
    ("capture", "ext:matcher-atom"),
    ("host", "ext:matcher-atom"),
    ("stopBy", "ext:matcher-atom"),
    ("value", "facts:semantic"),
    ("binds", "facts:semantic"),
    ("resolves", "facts:semantic"),
    ("arguments", "facts:semantic"),
    ("formatArgs", "facts:semantic"),
    ("calls", "facts:crossfile"),
    ("$option", "today"),
];

/// SQL functions by name, with the category inside a stage.
const FUNCTIONS: [(&str, &str); 40] = [
    ("count", "today"),
    ("min", "today"),
    ("max", "today"),
    ("coalesce", "today"),
    ("ifnull", "today"),
    ("nullif", "today"),
    ("row_number", "today"),
    ("sum", "outside:aggregate"),
    ("total", "outside:aggregate"),
    ("avg", "outside:aggregate"),
    ("group_concat", "outside:collection"),
    ("json_group_array", "outside:collection"),
    ("json_group_object", "outside:collection"),
    ("json_array", "outside:collection"),
    ("json_object", "outside:collection"),
    ("json_quote", "outside:collection"),
    ("rank", "outside:window"),
    ("dense_rank", "outside:window"),
    ("lag", "outside:window"),
    ("lead", "outside:window"),
    ("first_value", "outside:window"),
    ("last_value", "outside:window"),
    ("json_extract", "ext:builtins"),
    ("substr", "ext:builtins"),
    ("substring", "ext:builtins"),
    ("instr", "ext:builtins"),
    ("replace", "ext:builtins"),
    ("trim", "ext:builtins"),
    ("ltrim", "ext:builtins"),
    ("rtrim", "ext:builtins"),
    ("lower", "ext:builtins"),
    ("upper", "ext:builtins"),
    ("length", "ext:builtins"),
    ("printf", "ext:builtins"),
    ("char", "ext:builtins"),
    ("cast", "ext:builtins"),
    ("iif", "ext:builtins"),
    ("rx_matches", "ext:builtins"),
    ("rx_replace", "ext:builtins"),
    ("rx_extract", "ext:builtins"),
];

/// Constructs other than terms, functions and recursions, with their categories.
const OTHER: [(&str, &str); 43] = [
    ("clause:forbid", "ext:matcher-atom"),
    ("clause:forbid-text", "outside:text"),
    ("clause:forbid-path", "ext:builtins"),
    ("text:lines", "outside:text"),
    ("text:line-state", "outside:text"),
    ("text:prefilter", "outside:text"),
    ("text:paths", "ext:builtins"),
    ("clause:forbid-option", "today"),
    ("clause:relational", "today"),
    ("clause:budget", "today"),
    ("clause:violation", "findings"),
    ("clause:report_when", "today"),
    ("budget:count", "today"),
    ("budget:group_by", "today"),
    ("evidence:capture", "ext:matcher-captures"),
    ("evidence:each", "ext:matcher-captures"),
    ("evidence:regex-projection", "ext:builtins"),
    ("evidence:line", "ext:builtins"),
    ("evidence:file-path", "today"),
    ("evidence:file-module", "facts:crossfile"),
    ("evidence:anchor", "findings"),
    ("evidence:where", "today"),
    ("evidence:closure-seeds", "today"),
    ("evidence:transform", "ext:matcher-atom"),
    ("evidence:file", "ext:matcher-atom"),
    ("rel:partition-file", "today"),
    ("rel:partition-repository", "today"),
    ("rel:syntax", "ext:tree"),
    ("rel:references", "facts:crossfile"),
    ("rel:locations", "today"),
    ("rel:require-nonempty", "today"),
    ("rel:deferred", "today"),
    ("rel:key", "findings"),
    ("sql:with", "today"),
    ("sql:union", "today"),
    ("sql:join", "today"),
    ("sql:negation", "today"),
    ("sql:group-by", "today"),
    ("sql:case", "today"),
    ("sql:null-test", "today"),
    ("sql:literal-list", "today"),
    ("sql:limit", "outside:choice"),
    ("sql:reference-edges", "facts:crossfile"),
];

/// More SQL constructs, by what they are rather than a function name.
const SQL_MORE: [(&str, &str); 7] = [
    ("sql:limit-ordered", "today"),
    ("sql:distinct", "today"),
    ("sql:order-by", "today"),
    ("sql:concat", "ext:builtins"),
    ("sql:arith", "ext:builtins"),
    ("sql:like", "ext:builtins"),
    ("sql:json-each", "outside:json"),
];

fn category(construct: &str, recursion: &BTreeMap<String, String>) -> String {
    if let Some(c) = recursion.get(construct) {
        return c.clone();
    }
    let (stage, name) = match construct.strip_prefix("query:") {
        Some(rest) => (false, rest),
        None => (true, construct),
    };
    let base = if let Some(term) = name.strip_prefix("matcher:") {
        TERMS.iter().find(|t| t.0 == term).map(|t| t.1)
    } else if let Some(f) = name.strip_prefix("sql:fn:") {
        FUNCTIONS.iter().find(|t| t.0 == f).map(|t| t.1)
    } else {
        OTHER.iter().chain(&SQL_MORE).find(|t| t.0 == name).map(|t| t.1)
    };
    let base = base.unwrap_or_else(|| panic!("unclassified construct {construct}"));
    // The final query is the findings layer's: what Datalog lacks there is §8's, not the logic's.
    if !stage && base != "today" && base != "facts:crossfile" { "findings".to_owned() } else { base.to_owned() }
}

/// The option defaults of a document's define clauses.
fn defaults(doc: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for c in doc["clauses"].as_array().unwrap() {
        for (k, v) in c.pointer("/define/options").and_then(Value::as_object).into_iter().flatten() {
            out.insert(k.clone(), v.get("default").cloned().unwrap_or(Value::Null));
        }
    }
    out
}

/// A value at its options' defaults: `{"$option": o, "cases": {...}, "default": v}` picks a case.
fn pick<'a>(v: &'a Value, opts: &BTreeMap<String, Value>) -> &'a Value {
    let mut v = v;
    while let Some(o) = v.get("$option").and_then(Value::as_str) {
        let key = match opts.get(o) {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        v = match v.get("cases").and_then(|c| c.get(&key)).or_else(|| v.get("default")) {
            Some(next) => next,
            None => return &Value::Null,
        };
    }
    v
}

struct Clause {
    kind: &'static str,
    counts: BTreeMap<String, usize>,
}

impl Clause {
    fn add(&mut self, construct: impl Into<String>) {
        *self.counts.entry(construct.into()).or_default() += 1;
    }
}

/// Counts a rule object's terms, walking into the parts that are rule objects.
fn rule(v: &Value, c: &mut Clause) {
    let Some(obj) = v.as_object() else { return };
    for (k, x) in obj {
        if TERMS.iter().any(|t| t.0 == k) {
            c.add(format!("matcher:{k}"));
        }
        match k.as_str() {
            "all" | "any" => x.as_array().into_iter().flatten().for_each(|r| rule(r, c)),
            "not" | "has" | "inside" | "precedes" | "follows" | "stopBy" | "host" | "count" | "parse" | "ofRule" => rule(x, c),
            "nthChild" => rule(x, c),
            "constraints" => x.as_object().into_iter().flatten().for_each(|(_, r)| rule(r, c)),
            "$option" => {}
            "cases" => x.as_object().into_iter().flatten().for_each(|(_, r)| rule(r, c)),
            "default" => rule(x, c),
            _ => {}
        }
    }
}

/// An `ast` query object: its rule, utils, transform and file filter.
fn ast(q: &Value, c: &mut Clause) {
    rule(&q["rule"], c);
    for (_, u) in q.get("utils").and_then(Value::as_object).into_iter().flatten() {
        rule(u, c);
    }
    if q.get("transform").is_some() {
        c.add("evidence:transform");
    }
    if q.get("file").is_some() {
        c.add("evidence:file");
    }
}

/// An evidence query: its matcher and its projections. Closures are recursions, keyed by `site`.
fn evidence(e: &Value, c: &mut Clause, site: &str, recursions: &mut Vec<String>) {
    ast(&e["query"]["ast"], c);
    for (_, p) in e.get("project").and_then(Value::as_object).into_iter().flatten() {
        let construct = match p {
            Value::String(s) if s.ends_with("Line") || s.ends_with(".line") => "evidence:line",
            Value::String(s) if s.starts_with("capture.") => "evidence:capture",
            Value::String(s) if s == "file.module" => "evidence:file-module",
            Value::String(s) if s.starts_with("file.") => "evidence:file-path",
            Value::Object(o) if o.contains_key("regex") => "evidence:regex-projection",
            other => panic!("unclassified projection {other}"),
        };
        c.add(construct);
    }
    if e.get("each").is_some() {
        c.add("evidence:each");
    }
    if e.get("anchor").is_some() {
        c.add("evidence:anchor");
    }
    if e.get("where").is_some() {
        c.add("evidence:where");
    }
    for cl in e.get("closure").and_then(Value::as_array).into_iter().flatten() {
        if cl.get("edges").is_some() {
            recursions.push(format!("{site}\tclosure:{}", cl["name"].as_str().unwrap()));
        } else {
            c.add("evidence:closure-seeds");
        }
    }
}

/// A SQL text with its string literals emptied and comments removed, lowercased.
fn bare(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                out.push_str("''");
                while let Some(c) = chars.next() {
                    if c == '\'' {
                        if chars.peek() == Some(&'\'') {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
                out.push('\n');
            }
            _ => out.extend(ch.to_lowercase()),
        }
    }
    out
}

fn words(sql: &str) -> Vec<&str> {
    spans(sql).into_iter().map(|(_, w)| w).collect()
}

/// The identifier-like words of a text, with their byte offsets.
fn spans(sql: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in sql.char_indices().chain([(sql.len(), ' ')]) {
        match (c.is_ascii_alphanumeric() || c == '_', start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, &sql[s..i]));
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// The recursive CTEs of a `WITH RECURSIVE` text: each CTE whose body names it.
fn recursive_ctes(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = sql;
    while let Some(at) = rest.find("with recursive") {
        let mut s = &rest[at + "with recursive".len()..];
        loop {
            s = s.trim_start();
            let name: String = s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if name.is_empty() {
                break;
            }
            s = s[name.len()..].trim_start();
            if s.starts_with('(') {
                s = &s[s.find(')').unwrap() + 1..];
            }
            s = s.trim_start();
            let Some(after) = s.strip_prefix("as") else { break };
            s = after.trim_start();
            let Some(open) = s.strip_prefix('(') else { break };
            let (mut depth, mut end) = (1, 0);
            for (i, ch) in open.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            if words(&open[..end]).contains(&name.as_str()) {
                out.push(name);
            }
            s = open[end + 1..].trim_start();
            match s.strip_prefix(',') {
                Some(next) => s = next,
                None => break,
            }
        }
        rest = s;
    }
    out
}

/// The constructs of one SQL text; `stage` is false for the clause's final query.
fn sql(text: &str, stage: bool, c: &mut Clause, site: &str, recursions: &mut Vec<String>) {
    let t = bare(text);
    let spans = spans(&t);
    let w: Vec<&str> = spans.iter().map(|(_, w)| *w).collect();
    let p = if stage { "" } else { "query:" };
    let add = |c: &mut Clause, name: &str| c.add(format!("{p}{name}"));
    let ctes = recursive_ctes(&t);
    for cte in &ctes {
        recursions.push(format!("{site}\t{cte}"));
    }
    if ctes.is_empty() && w.contains(&"with") {
        add(c, "sql:with");
    }
    for (i, word) in w.iter().enumerate() {
        let next = w.get(i + 1).copied().unwrap_or("");
        let prev = if i > 0 { w[i - 1] } else { "" };
        let called = t[spans[i].0 + word.len()..].trim_start().starts_with('(');
        match *word {
            "union" | "intersect" => add(c, "sql:union"),
            "join" if prev != "left" => add(c, "sql:join"),
            "exists" if prev != "not" => add(c, "sql:join"),
            "not" if matches!(next, "exists" | "in") => add(c, "sql:negation"),
            "except" => add(c, "sql:negation"),
            "left" if next == "join" => add(c, "sql:negation"),
            "group" if next == "by" => add(c, "sql:group-by"),
            "case" => add(c, "sql:case"),
            "is" if matches!(next, "null" | "not") => add(c, "sql:null-test"),
            "distinct" => add(c, "sql:distinct"),
            "limit" => {
                // LIMIT after an ORDER BY of the same SELECT picks a least row (an aggregate); otherwise an arbitrary one.
                let from = w[..i].iter().rposition(|x| *x == "select").unwrap_or(0);
                let ordered = w[from..i].windows(2).any(|p| p == ["order", "by"]) && !w[from..i].contains(&"over");
                add(c, if ordered { "sql:limit-ordered" } else { "sql:limit" });
            }
            "like" | "glob" => add(c, "sql:like"),
            "reference_edges" => add(c, "sql:reference-edges"),
            "order" if next == "by" => {
                // A window's ORDER BY is the window's; others order rows.
                let window = w[..i].iter().rev().take_while(|x| **x != "select").any(|x| *x == "over");
                if !window {
                    add(c, "sql:order-by");
                }
            }
            "json_each" if called => add(c, if t[spans[i].0..].starts_with("json_each('')") { "sql:literal-list" } else { "sql:json-each" }),
            f if called && FUNCTIONS.iter().any(|x| x.0 == f) => add(c, &format!("sql:fn:{f}")),
            _ => {}
        }
    }
    let concat = t.matches("||").count();
    for _ in 0..concat {
        add(c, "sql:concat");
    }
    // Binary arithmetic: an operand, then + - * / %, then an operand (`select *`, `d.*` and `count(*)` are not).
    let b = t.as_bytes();
    for (i, &ch) in b.iter().enumerate() {
        if !matches!(ch, b'+' | b'-' | b'/' | b'%' | b'*') || i == 0 {
            continue;
        }
        let before = t[..i].trim_end();
        let prev = before.as_bytes().last().copied();
        let next = t[i + 1..].trim_start().as_bytes().first().copied();
        let operand = |c: Option<u8>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'\'');
        let star = ch == b'*' && (before.ends_with("select") || before.ends_with("distinct") || prev == Some(b'.'));
        if !star && (operand(prev) || prev == Some(b')')) && (operand(next) || next == Some(b'(')) {
            add(c, "sql:arith");
        }
    }
}

struct Census {
    /// (rule, clause) → clause.
    clauses: BTreeMap<(String, String), Clause>,
    /// Recursion keys: `rule \t clause \t stage \t name`.
    recursions: Vec<String>,
}

fn derive() -> Census {
    let dir = gate().join("packs/data-layer/candidates/rules");
    // The 60 rules of Pulling-it-all-together's rewrite (`candidates/rules-ast`); the 10 candidates copied later
    // from Data-layer-rule-expressivity (`require_each` and other new forms) are outside the census (O72).
    let converted = gate().join("packs/data-layer/candidates/rules-ast");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().is_some_and(|s| converted.join(s).exists()))
        .collect();
    files.sort();
    let mut census = Census { clauses: BTreeMap::new(), recursions: Vec::new() };
    for path in files {
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let id = doc["id"].as_str().unwrap().strip_prefix("data-layer-").unwrap().to_owned();
        let opts = defaults(&doc);
        for cl in doc["clauses"].as_array().unwrap() {
            let cid = cl["id"].as_str().unwrap().to_owned();
            let kind = ["define", "forbid", "relational", "budget"].into_iter().find(|k| cl.get(*k).is_some()).unwrap();
            let mut c = Clause { kind, counts: BTreeMap::new() };
            let site = format!("{id}\t{cid}");
            if cl.get("violation").is_some() {
                c.add("clause:violation");
            }
            match kind {
                "define" => {
                    for (_, u) in cl["define"].get("utils").and_then(Value::as_object).into_iter().flatten() {
                        rule(u, &mut c);
                    }
                    if cl["define"].get("report_when").is_some() {
                        c.add("clause:report_when");
                    }
                }
                "forbid" => {
                    let f = &cl["forbid"];
                    if f.get("$option").is_some() {
                        c.add("clause:forbid-option");
                    }
                    let f = pick(f, &opts);
                    if let Some(q) = f.get("ast") {
                        c.add("clause:forbid");
                        ast(q, &mut c);
                    } else {
                        // A regex over the file's text (or path), line by line: no matcher.
                        let r = &f["regex"];
                        c.add(if r["source"] == "file.path" { "clause:forbid-path" } else { "clause:forbid-text" });
                        if let Some(lines) = r.get("lines") {
                            c.add("text:lines");
                            if lines.get("state").is_some() {
                                c.add("text:line-state");
                            }
                        }
                        if ["include", "exclude", "content"].iter().any(|k| r.get(*k).is_some()) {
                            c.add("text:prefilter");
                        }
                        if r.get("paths").is_some() {
                            c.add("text:paths");
                        }
                    }
                }
                "budget" => {
                    let b = &cl["budget"];
                    c.add("clause:budget");
                    c.add("budget:count");
                    if b.get("group_by").is_some() {
                        c.add("budget:group_by");
                    }
                    evidence(&b["query"]["evidence"], &mut c, &format!("{site}\tbudget"), &mut census.recursions);
                }
                _ => {
                    let r = &cl["relational"];
                    c.add("clause:relational");
                    for (_, input) in r["inputs"].as_object().unwrap() {
                        evidence(&input["evidence"], &mut c, &format!("{site}\tinput"), &mut census.recursions);
                    }
                    let partition = pick(r.get("partitionBy").unwrap_or(&Value::Null), &opts);
                    c.add(if partition == "file" { "rel:partition-file" } else { "rel:partition-repository" });
                    for (key, construct) in [
                        ("syntax", "rel:syntax"),
                        ("references", "rel:references"),
                        ("locations", "rel:locations"),
                        ("requireNonemptyInput", "rel:require-nonempty"),
                        ("key", "rel:key"),
                    ] {
                        if r.get(key).is_some() {
                            c.add(construct);
                        }
                    }
                    for s in r.get("stages").and_then(Value::as_array).into_iter().flatten() {
                        let name = s["name"].as_str().unwrap();
                        let stage_site = format!("{site}\t{name}");
                        if s.get("deferred").is_some() {
                            c.add("rel:deferred");
                        }
                        if let Some(reach) = s.get("reachability") {
                            census.recursions.push(format!("{stage_site}\treachability"));
                            for part in ["roots", "edges"] {
                                if let Some(text) = reach.get(part).and_then(Value::as_str) {
                                    sql(text, true, &mut c, &stage_site, &mut census.recursions);
                                }
                            }
                        }
                        if let Some(q) = s.get("query") {
                            let q = pick(q, &opts);
                            let text = q.get("template").unwrap_or(q).as_str().unwrap_or_default();
                            sql(text, true, &mut c, &stage_site, &mut census.recursions);
                        }
                    }
                    if let Some(q) = r.get("query") {
                        let q = pick(q, &opts);
                        let text = q.get("template").unwrap_or(q).as_str().unwrap_or_default();
                        sql(text, false, &mut c, &format!("{site}\tquery"), &mut census.recursions);
                    }
                }
            }
            census.clauses.insert((id.clone(), cid), c);
        }
    }
    census
}

/// `recursion.tsv`: key (rule, clause, stage, name) → category, after its header.
fn recursion_table() -> (BTreeMap<String, String>, String) {
    let text = std::fs::read_to_string(gate().join("census/recursion.tsv")).unwrap();
    let mut header = text.lines().next().unwrap().split('\t');
    let at = header.position(|h| h == "category").unwrap();
    let rows = text
        .lines()
        .skip(1)
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f[..4].join("\t"), f[at].to_owned())
        })
        .collect();
    (rows, text)
}

/// The recursion construct a key counts as in its clause: `recursion:<family>/<lattice>` from the table.
fn recursion_construct(text: &str, key: &str) -> String {
    let header: Vec<&str> = text.lines().next().unwrap().split('\t').collect();
    let row = text.lines().skip(1).find(|l| l.split('\t').take(4).collect::<Vec<_>>().join("\t") == key).unwrap();
    let f: Vec<&str> = row.split('\t').collect();
    let col = |name: &str| f[header.iter().position(|h| *h == name).unwrap()];
    format!("recursion:{}/{}", col("family"), col("lattice"))
}

fn render(census: &Census, table: &BTreeMap<String, String>, table_text: &str) -> BTreeMap<&'static str, String> {
    // Recursion constructs join their clause's counts; their category is the table's.
    let mut clauses: BTreeMap<(String, String), (&str, BTreeMap<String, usize>)> =
        census.clauses.iter().map(|(k, c)| (k.clone(), (c.kind, c.counts.clone()))).collect();
    let mut categories: BTreeMap<String, String> = BTreeMap::new();
    for key in &census.recursions {
        let f: Vec<&str> = key.split('\t').collect();
        let construct = recursion_construct(table_text, key);
        categories.insert(construct.clone(), table[key].clone());
        *clauses.get_mut(&(f[0].to_owned(), f[1].to_owned())).unwrap().1.entry(construct).or_default() += 1;
    }
    let cat = |construct: &str| categories.get(construct).cloned().unwrap_or_else(|| category(construct, &categories));
    let mut out = BTreeMap::new();
    let mut s = String::from("rule\tclause\tkind\tcategories\tconstructs\n");
    for ((rule, clause), (kind, counts)) in &clauses {
        let cats: BTreeSet<String> = counts.keys().map(|k| cat(k)).filter(|c| c != "today").collect();
        let constructs: Vec<String> = counts.iter().map(|(k, n)| format!("{k}×{n}")).collect();
        let cats: Vec<String> = cats.into_iter().collect();
        writeln!(s, "{rule}\t{clause}\t{kind}\t{}\t{}", cats.join(","), constructs.join(",")).unwrap();
    }
    out.insert("clauses.tsv", s);
    let mut per: BTreeMap<String, (BTreeSet<String>, usize, usize)> = BTreeMap::new();
    for ((rule, _), (_, counts)) in &clauses {
        for (k, n) in counts {
            let e = per.entry(k.clone()).or_default();
            e.0.insert(rule.clone());
            e.1 += 1;
            e.2 += n;
        }
    }
    let mut s = String::from("construct\tcategory\trules\tclauses\toccurrences\n");
    for (k, (rules, n, occ)) in &per {
        writeln!(s, "{k}\t{}\t{}\t{n}\t{occ}", cat(k), rules.len()).unwrap();
    }
    out.insert("constructs.tsv", s);
    const CATEGORIES: [&str; 15] = [
        "today",
        "findings",
        "ext:matcher-atom",
        "ext:matcher-captures",
        "ext:builtins",
        "ext:lattice",
        "ext:tree",
        "facts:semantic",
        "facts:crossfile",
        "outside:value-cycle",
        "outside:not-stratifiable",
        "outside:collection",
        "outside:choice",
        "outside:text",
        "outside:other",
    ];
    let mut s = String::from("rule\tdefine\tforbid\trelational\tbudget");
    for c in CATEGORIES {
        write!(s, "\t{c}").unwrap();
    }
    s.push_str("\tblocked by\n");
    let rules: BTreeSet<&String> = clauses.keys().map(|k| &k.0).collect();
    for rule in rules {
        let mine: Vec<_> = clauses.iter().filter(|(k, _)| &k.0 == rule).collect();
        write!(s, "{rule}").unwrap();
        for kind in ["define", "forbid", "relational", "budget"] {
            write!(s, "\t{}", mine.iter().filter(|(_, c)| c.0 == kind).count()).unwrap();
        }
        let mut by: BTreeMap<String, BTreeSet<&String>> = BTreeMap::new();
        for (_, (_, counts)) in &mine {
            for k in counts.keys() {
                let c = cat(k);
                let c = if c.starts_with("outside:") && !CATEGORIES.contains(&c.as_str()) { "outside:other".to_owned() } else { c };
                by.entry(c).or_default().insert(k);
            }
        }
        for c in CATEGORIES {
            write!(s, "\t{}", by.get(c).map_or(0, BTreeSet::len)).unwrap();
        }
        let blocked: Vec<&str> = CATEGORIES.iter().copied().filter(|c| !matches!(*c, "today" | "findings") && by.contains_key(*c)).collect();
        writeln!(s, "\t{}", if blocked.is_empty() { "-".to_owned() } else { blocked.join(",") }).unwrap();
    }
    out.insert("rules.tsv", s);
    out
}

/// The census files are what the rule files say; `recursion.tsv` names exactly the recursions there are.
#[test]
fn data_layer_census_matches_the_rules() {
    let census = derive();
    assert_eq!(census.clauses.keys().map(|k| &k.0).collect::<BTreeSet<_>>().len(), 60, "60 rules");
    let (table, table_text) = recursion_table();
    let derived: BTreeSet<&String> = census.recursions.iter().collect();
    let listed: BTreeSet<&String> = table.keys().collect();
    let missing: Vec<_> = derived.difference(&listed).collect();
    let extra: Vec<_> = listed.difference(&derived).collect();
    assert!(missing.is_empty() && extra.is_empty(), "recursion.tsv: missing {missing:#?}, extra {extra:#?}");
    let bless = std::env::var_os("RERAIL_BLESS_CENSUS").is_some();
    for (name, text) in render(&census, &table, &table_text) {
        let path = gate().join("census").join(name);
        if bless {
            std::fs::write(&path, &text).unwrap();
            continue;
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        let diff: Vec<String> = text
            .lines()
            .filter(|l| !have.lines().any(|h| h == *l))
            .map(|l| format!("+ {l}"))
            .chain(have.lines().filter(|h| !text.lines().any(|l| l == *h)).map(|l| format!("- {l}")))
            .take(20)
            .collect();
        assert!(diff.is_empty(), "{name} drifted from the rules (RERAIL_BLESS_CENSUS=1 rewrites it):\n{}", diff.join("\n"));
    }
}
