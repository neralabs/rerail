//! Physical shape rewrites for known compiler contracts. Stage results contain at most one
//! verdict per stage root. Consumers that only keep input columns therefore need a keyed probe.
use super::graph::Graph;
use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug)]
pub(super) enum StageProbe {
    RowIn,
    Exists,
}
static STAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"SELECT (?P<columns>f\.k, f\.file, f\.node(?:, f\.[A-Za-z0-9_]+)*) FROM (?P<input>q[0-9]+) f CROSS JOIN (?P<domain>q[0-9]+) q ON q\.file = f\.file AND q\.node = f\.node CROSS JOIN (?P<answer>q[0-9]+) o ON o\.k = q\.k").unwrap()
});
pub(super) fn stage_probes(graph: &mut Graph, shape: StageProbe) -> usize {
    let mut count = 0;
    for relation in &mut graph.relations {
        let body=STAGE.replace_all(&relation.body,|c:&regex::Captures<'_>|{
            count+=1;
            let columns=&c["columns"];let input=&c["input"];let domain=&c["domain"];let answer=&c["answer"];
            let filter=match shape {
                StageProbe::RowIn=>format!("(f.file,f.node) IN (SELECT q.file,q.node FROM {domain} q JOIN {answer} o ON o.k=q.k)"),
                StageProbe::Exists=>format!("EXISTS(SELECT 1 FROM {domain} q JOIN {answer} o ON o.k=q.k WHERE q.file=f.file AND q.node=f.node)"),
            };
            format!("SELECT {columns} FROM {input} f WHERE {filter}")
        });
        relation.body = body.into_owned();
    }
    count
}

static ANCESTOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"EXISTS \(SELECT 1 FROM node g WHERE g\.file = n\.file AND g\.id < n\.id AND g\.\"last\" >= n\.id AND g\.kind IN \(([0-9, ]+)\)\)"#).unwrap()
});

/// Seed a root's necessary ancestor guard from rare containers, then enumerate their ranges.
/// DISTINCT preserves existence semantics for nested containers. Selected-root probes retain
/// their original guard; only the all-root domain is changed.
pub(super) fn ancestor_anchors(graph: &mut Graph) -> usize {
    let mut count = 0;
    let root_kinds = Regex::new(r"n\.kind IN \(([0-9, ]+)\)").unwrap();
    for relation in &mut graph.relations {
        if !relation
            .body
            .starts_with("SELECT file*4294967296+id,file,id FROM (")
        {
            continue;
        }
        let split = relation
            .body
            .find(" UNION ALL SELECT n.file, n.id FROM (")
            .unwrap_or(relation.body.len());
        let prefix = &relation.body[..split];
        if super::lex::identifiers(prefix)
            .iter()
            .any(|word| matches!(*word, "OR" | "NOT"))
        {
            continue;
        }
        let Some(found) = ANCESTOR.captures(prefix) else {
            continue;
        };
        let original = &found[0];
        let kinds = &found[1];
        let Some(from) = prefix.find("FROM node n WHERE ") else {
            continue;
        };
        let file_start = prefix.find("n.file IN (").map(|i| i + "n.file IN (".len());
        let files = file_start.and_then(|start| {
            let mut depth = 1i32;
            for (at, ch) in prefix[start..].char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(&prefix[start..start + at]);
                        }
                    }
                    _ => {}
                }
            }
            None
        });
        let Some(files) = files else {
            continue;
        };
        let root_filter = root_kinds
            .captures(prefix)
            .map_or(String::new(), |c| format!(" AND c.kind IN ({})", &c[1]));
        let seed = format!(
            "SELECT DISTINCT c.file,c.id FROM node a CROSS JOIN node c ON c.file=a.file AND c.id>a.id AND c.id<=a.\"last\" WHERE a.kind IN ({kinds}) AND a.file IN ({files}){root_filter}"
        );
        let mut rewritten = prefix.replacen(original, "1", 1);
        // The seed already applies file scope. Keeping the redundant IN lets the virtual
        // table choose a multi-file lookup for every candidate instead of an exact key.
        rewritten = rewritten.replacen(&format!("n.file IN ({files})"), "1", 1);
        rewritten.replace_range(from..from+"FROM node n WHERE ".len(),&format!("FROM ({seed}) anchor CROSS JOIN node n ON n.file=anchor.file AND n.id=anchor.id WHERE "));
        relation.body = format!("{rewritten}{}", &relation.body[split..]);
        count += 1;
    }
    count
}
