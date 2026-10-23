//! Budget evidence `closure` (native `query/name_closure.rs` `derive`, compiled by
//! `compile/evidence.rs` `compile_closure`): per stage, the names reachable from the stage's seeds
//! along its edges within each partition of the evidence rows; each `contains` output is a boolean
//! field of every row (its input field's string is reachable in the row's partition). The budget's
//! `where` and grouping read the rows after every stage (`eval/evidence.rs` `derived_relation`).
//!
//! Native evaluates it over the clause's whole relation, every file's rows: it fails the clause
//! (`source-incomplete`) when some file's rows are unknown, and (`budget-exceeded`) when the
//! derivation spends more steps than `maxVisitedNodes`. Both are reproduced here over the same
//! rows: the step count is order independent (every row, seed, inherited name, popped name and
//! edge is counted once), so it is computed, not simulated.
use serde_json::Value as Json;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::LazyLock;

/// A projected field's value, owned (rows cross worker threads): what native's projection JSON
/// holds, with booleans for closure outputs.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Null,
    Int(i64),
    Real(f64),
    Text(Box<str>),
    Bool(bool),
}

impl Val {
    fn as_str(&self) -> Option<&str> {
        match self {
            Val::Text(s) => Some(s),
            _ => None,
        }
    }
    /// A typed key part, as native's `serde_json::to_string` of the value tells types apart.
    fn key_into(&self, out: &mut String) {
        use std::fmt::Write;
        match self {
            Val::Null => out.push_str("n\u{1f}"),
            Val::Int(i) => {
                let _ = write!(out, "i{i}\u{1f}");
            }
            Val::Real(r) => {
                let _ = write!(out, "r{r:?}\u{1f}");
            }
            Val::Text(t) => {
                let _ = write!(out, "t{}:{t}\u{1f}", t.len());
            }
            Val::Bool(b) => {
                let _ = write!(out, "b{b}\u{1f}");
            }
        }
    }
}

/// One stage; fields are indexes into the input's projected fields (name order).
#[derive(Clone, Debug)]
pub struct Stage {
    pub name: String,
    pub seeds: Vec<String>,
    pub seed_field: Option<usize>,
    pub edges: Option<(usize, usize)>,
    /// Earlier stages (indexes) whose known names seed this one, partition by partition.
    pub extends: Vec<usize>,
    pub partition_by: Vec<usize>,
    /// (output index, input field): outputs are numbered across stages in declaration order.
    pub contains: Vec<(usize, usize)>,
}

/// A compiled `closure`: its stages and output names (in output index order).
#[derive(Clone, Debug)]
pub struct Closure {
    pub stages: Vec<Stage>,
    pub outputs: Vec<String>,
}

impl Closure {
    /// Native `compile_closure`: 1 to 32 stages, known keys, projected fields, `extends` naming
    /// an earlier stage with the same `partition_by`, outputs new and nonempty.
    pub fn compile(v: &Json, fields: &[String], at: &str) -> Result<Closure, String> {
        let at = format!("{at}/closure");
        let stages = v.as_array().ok_or_else(|| format!("{at} must be an array"))?;
        if stages.is_empty() || stages.len() > 32 {
            return Err(format!("{at} must contain 1 to 32 stages"));
        }
        let mut out = Closure { stages: Vec::new(), outputs: Vec::new() };
        for (index, st) in stages.iter().enumerate() {
            let at = format!("{at}/{index}");
            let o = st.as_object().ok_or_else(|| format!("{at} must be an object"))?;
            if let Some(k) = o.keys().find(|k| !["name", "seeds", "seed_field", "edges", "extends", "partition_by", "contains"].contains(&k.as_str())) {
                return Err(format!("unknown field `{k}` at {at}"));
            }
            let name = o.get("name").and_then(Json::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("{at}/name must be a non-empty string"))?.to_owned();
            if out.stages.iter().any(|s| s.name == name) {
                return Err(format!("{at}/name: duplicate closure set `{name}`"));
            }
            let list = |k: &str| -> Result<Vec<String>, String> {
                match o.get(k) {
                    None => Ok(Vec::new()),
                    Some(Json::Array(a)) => a.iter().map(|x| x.as_str().map(str::to_owned).ok_or_else(|| format!("{at}/{k} must be an array of strings"))).collect(),
                    Some(_) => Err(format!("{at}/{k} must be an array of strings")),
                }
            };
            let field = |v: Option<&Json>, k: &str| -> Result<usize, String> {
                let n = v.and_then(Json::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("{at}/{k} must be a non-empty string"))?;
                fields.iter().position(|f| f == n).ok_or_else(|| format!("{at}/{k}: `{n}` names no projected field"))
            };
            let seeds = list("seeds")?;
            let partition_by = list("partition_by")?.iter().map(|p| field(Some(&Json::String(p.clone())), "partition_by")).collect::<Result<Vec<_>, _>>()?;
            let extends = list("extends")?
                .iter()
                .map(|d| out.stages.iter().position(|s| s.name == *d && s.partition_by == partition_by).ok_or_else(|| format!("{at}/extends: `{d}` must be an earlier set with the same partition_by")))
                .collect::<Result<Vec<_>, _>>()?;
            let seed_field = o.get("seed_field").map(|v| field(Some(v), "seed_field")).transpose()?;
            let edges = match o.get("edges") {
                None => None,
                Some(e) => {
                    let e = e.as_object().ok_or_else(|| format!("{at}/edges must be an object"))?;
                    if let Some(k) = e.keys().find(|k| !["from", "to"].contains(&k.as_str())) {
                        return Err(format!("unknown field `{k}` at {at}/edges"));
                    }
                    Some((field(e.get("from"), "edges/from")?, field(e.get("to"), "edges/to")?))
                }
            };
            let contains = o.get("contains").ok_or_else(|| format!("{at}/contains is required"))?.as_object().ok_or_else(|| format!("{at}/contains must be an object"))?;
            let mut cs = Vec::new();
            for (output, input) in contains {
                if output.is_empty() || fields.contains(output) || out.outputs.contains(output) {
                    return Err(format!("{at}/contains: output `{output}` must be new and nonempty"));
                }
                out.outputs.push(output.clone());
                cs.push((out.outputs.len() - 1, field(Some(input), "contains")?));
            }
            out.stages.push(Stage { name, seeds, seed_field, edges, extends, partition_by, contains: cs });
        }
        Ok(out)
    }

    /// The outputs of every row (`out[row][output]`), or native's failure (code, message).
    /// `rows[r][f]` is projected field `f` of row `r`.
    pub fn derive(&self, rows: &[&[Val]]) -> Result<Vec<Vec<bool>>, (&'static str, &'static str)> {
        let mut out = vec![vec![false; self.outputs.len()]; rows.len()];
        // Steps native's `Work::spend` takes; it fails on the step past the allowance.
        let mut steps: u64 = 0;
        let mut completed: Vec<HashMap<String, HashSet<String>>> = Vec::with_capacity(self.stages.len());
        let key = |row: &[Val], by: &[usize]| -> Option<String> {
            let mut k = String::new();
            for &f in by {
                if row[f] == Val::Null {
                    return None;
                }
                row[f].key_into(&mut k);
            }
            Some(k)
        };
        for stage in &self.stages {
            #[derive(Default)]
            struct Partition {
                known: HashSet<String>,
                edges: HashMap<String, Vec<String>>,
                pending: VecDeque<String>,
            }
            impl Partition {
                fn seed(&mut self, name: &str) {
                    if self.known.insert(name.to_owned()) {
                        self.pending.push_back(name.to_owned());
                    }
                }
            }
            let mut partitions: HashMap<String, Partition> = HashMap::new();
            for row in rows {
                steps += 1;
                let Some(k) = key(row, &stage.partition_by) else { continue };
                let p = partitions.entry(k).or_insert_with_key(|k| {
                    let mut p = Partition::default();
                    for s in &stage.seeds {
                        steps += 1;
                        p.seed(s);
                    }
                    for &d in &stage.extends {
                        if let Some(earlier) = completed[d].get(k) {
                            for n in earlier {
                                steps += 1;
                                p.seed(n);
                            }
                        }
                    }
                    p
                });
                if let Some(s) = stage.seed_field.and_then(|f| row[f].as_str()) {
                    p.seed(s);
                }
                if let Some((from, to)) = stage.edges {
                    if let (Some(a), Some(b)) = (row[from].as_str(), row[to].as_str()) {
                        p.edges.entry(a.to_owned()).or_default().push(b.to_owned());
                    }
                }
            }
            for p in partitions.values_mut() {
                while let Some(n) = p.pending.pop_front() {
                    steps += 1;
                    for next in p.edges.remove(&n).into_iter().flatten() {
                        steps += 1;
                        p.seed(&next);
                    }
                }
            }
            for (r, row) in rows.iter().enumerate() {
                steps += 1;
                let p = key(row, &stage.partition_by).and_then(|k| partitions.get(&k));
                for &(o, f) in &stage.contains {
                    steps += 1;
                    out[r][o] = p.is_some_and(|p| row[f].as_str().is_some_and(|v| p.known.contains(v)));
                }
            }
            completed.push(partitions.into_iter().map(|(k, p)| (k, p.known)).collect());
        }
        if steps > *MAX_VISITED_NODES {
            return Err(("budget-exceeded", "name closure exceeded maxVisitedNodes or was interrupted"));
        }
        Ok(out)
    }
}

/// The clause failure native reports when some file's rows are unknown.
pub const INCOMPLETE: (&str, &str) = ("source-incomplete", "name closure cannot decide membership over incomplete evidence");

/// `maxVisitedNodes` of the capabilities the host compiles code rules against (the engine's
/// bundled `capabilities.json`, which `src/engine/code-rule/code-rule-compile.ts` reads).
static MAX_VISITED_NODES: LazyLock<u64> = LazyLock::new(|| {
    let caps: Json = serde_json::from_str(include_str!("../../../native/src/code_rule_dsl/capabilities.json")).expect("capabilities.json");
    caps["limits"]["maxVisitedNodes"].as_u64().expect("limits.maxVisitedNodes")
});

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Val {
        Val::Text(s.into())
    }

    #[test]
    fn reaches_within_partitions_and_extends() {
        let fields: Vec<String> = ["base", "class", "leaf", "path"].map(String::from).to_vec();
        let c = Closure::compile(
            &serde_json::json!([
                { "name": "classes", "seeds": ["G"], "edges": { "from": "base", "to": "class" }, "partition_by": ["path"], "contains": {} },
                { "name": "more", "extends": ["classes"], "partition_by": ["path"], "contains": { "grammar": "base", "safe": "leaf" } }
            ]),
            &fields,
            "e",
        )
        .unwrap();
        let rows: Vec<Box<[Val]>> = vec![
            vec![t("G"), t("A"), t("A"), t("p")].into(),
            vec![t("A"), t("B"), t("X"), t("p")].into(),
            vec![t("A"), Val::Null, t("B"), t("q")].into(),
            vec![t("B"), t("C"), t("C"), Val::Null].into(),
        ];
        let out = c.derive(&rows.iter().map(|r| &**r).collect::<Vec<_>>()).unwrap();
        assert_eq!(out, vec![vec![true, true], vec![true, false], vec![false, false], vec![false, false]]);
    }

    #[test]
    fn compile_refuses_like_native() {
        let fields = vec!["a".to_owned()];
        assert!(Closure::compile(&serde_json::json!([]), &fields, "e").is_err());
        assert!(Closure::compile(&serde_json::json!([{ "name": "x", "contains": { "a": "a" } }]), &fields, "e").is_err());
        assert!(Closure::compile(&serde_json::json!([{ "name": "x", "extends": ["y"], "contains": {} }]), &fields, "e").is_err());
        assert!(Closure::compile(&serde_json::json!([{ "name": "x", "seed_field": "b", "contains": {} }]), &fields, "e").is_err());
    }
}
