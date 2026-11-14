//! Typed identities for generated relational templates, before physical table boundaries.
//! References use interned child identities and bound values, never SQL aliases or Arc addresses.
use rusqlite::types::Value;
use std::collections::HashMap;
use std::sync::LazyLock;
#[path = "graph/program.rs"]
pub(super) mod program;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Class {
    File,
    Vocabulary,
    Request,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Sql(String),
    Reference(usize),
    Parameter(Vec<u8>),
    FileDomain,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct Definition {
    version: u32,
    language: u32,
    columns: String,
    parts: Vec<Part>,
    class: Class,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Node {
    pub id: usize,
    pub class: Class,
}

#[derive(Default)]
pub(super) struct Graph {
    definitions: HashMap<Definition, usize>,
    #[cfg(feature = "code-rule-profile")]
    constructions: HashMap<usize, usize>,
}

fn parameter(value: &Value) -> Vec<u8> {
    let (tag, bytes) = match value {
        Value::Null => (0, Vec::new()),
        Value::Integer(v) => (1, v.to_le_bytes().to_vec()),
        Value::Real(v) => (2, v.to_bits().to_le_bytes().to_vec()),
        Value::Text(v) => (3, v.as_bytes().to_vec()),
        Value::Blob(v) => (4, v.clone()),
    };
    let mut encoded = vec![tag];
    encoded.extend(bytes);
    encoded
}

fn parse_parts(body: &str, values: &[Value], nodes: &HashMap<String, Node>) -> (Vec<Part>, bool) {
    static TOKENS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"\?[0-9]+|[A-Za-z_][A-Za-z_0-9]*").expect("generated tokens")
    });
    let mut parts = Vec::new();
    let mut start = 0;
    let mut request = false;
    for token in TOKENS.find_iter(body) {
        let replacement = if let Some(index) = token.as_str().strip_prefix('?') {
            let index: usize = index.parse().expect("bound parameter");
            Some(Part::Parameter(parameter(&values[index - 1])))
        } else if let Some(child) = nodes.get(token.as_str()) {
            request |= child.class == Class::Request;
            Some(Part::Reference(child.id))
        } else {
            None
        };
        if let Some(replacement) = replacement {
            parts.push(Part::Sql(body[start..token.start()].to_owned()));
            parts.push(replacement);
            start = token.end();
        }
    }
    parts.push(Part::Sql(body[start..].to_owned()));
    (parts, request)
}

impl Graph {
    pub fn register(
        &mut self,
        ctes: &[String],
        values: &[Value],
        language: u32,
    ) -> HashMap<String, Node> {
        let mut nodes: HashMap<String, Node> = HashMap::new();
        for cte in ctes {
            let (head, body) = cte.split_once(") AS ").expect("generated CTE");
            let (name, columns) = head.split_once('(').expect("generated columns");
            let mut parts = Vec::new();
            let mut class = if columns == "term" {
                Class::Vocabulary
            } else {
                Class::File
            };
            if name == "cf" && !body.contains("GLOB") {
                // Domain restriction commutes with admitted file-local operations. Consumers
                // still apply the actual request domain; it never becomes a cached universe.
                parts.push(Part::FileDomain);
            } else {
                if name == "cf" {
                    class = Class::Request;
                }
                let (parsed, request) = parse_parts(body, values, &nodes);
                parts = parsed;
                if request {
                    class = Class::Request;
                }
                // Global aggregate/choice results depend on the requested domain. Window
                // frontiers remain scratch initially, even when partitioned by file.
                if body.contains("count(") || body.contains(" OVER") || body.contains(" LIMIT ") {
                    class = Class::Request;
                }
            }
            let definition = Definition {
                version: 1,
                language,
                columns: columns.to_owned(),
                parts,
                class,
            };
            let next = self.definitions.len() + 1;
            // HashMap compares complete typed definitions on hash collisions.
            let id = *self.definitions.entry(definition).or_insert(next);
            nodes.insert(name.to_owned(), Node { id, class });
        }
        nodes
    }

    #[cfg(feature = "code-rule-profile")]
    pub fn constructed(&mut self, id: usize) -> usize {
        let n = self.constructions.entry(id).or_default();
        let previous = *n;
        *n += 1;
        previous
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_parameter_positions_and_values_have_distinct_roles() {
        let mut graph = Graph::default();
        let a = graph.register(
            &[
                "a(fid,pre) AS MATERIALIZED (SELECT ?1,?2)".into(),
                "b(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM a)".into(),
            ],
            &[1.into(), 7.into()],
            1,
        );
        let b = graph.register(
            &[
                "renamed(fid,pre) AS MATERIALIZED (SELECT ?2,?1)".into(),
                "consumer(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM renamed)".into(),
            ],
            &[7.into(), 1.into()],
            1,
        );
        assert_eq!(a["b"].id, b["consumer"].id);
        let different = graph.register(
            &["a(fid,pre) AS MATERIALIZED (SELECT ?1,?2)".into()],
            &[1.into(), 8.into()],
            1,
        );
        assert_ne!(a["a"].id, different["a"].id);
        let grammar = graph.register(
            &["a(fid,pre) AS MATERIALIZED (SELECT ?1,?2)".into()],
            &[1.into(), 7.into()],
            2,
        );
        assert_ne!(a["a"].id, grammar["a"].id);
    }

    #[test]
    fn request_dependent_choices_taint_their_consumers() {
        let mut graph = Graph::default();
        let nodes = graph.register(
            &[
                "s(fid,pre) AS MATERIALIZED (SELECT 1,2)".into(),
                "choice(id) AS MATERIALIZED (SELECT count(*) FROM s)".into(),
                "result(fid,pre) AS MATERIALIZED (SELECT s.fid,s.pre FROM s JOIN choice)".into(),
            ],
            &[],
            1,
        );
        assert_eq!(nodes["s"].class, Class::File);
        assert_eq!(nodes["choice"].class, Class::Request);
        assert_eq!(nodes["result"].class, Class::Request);
    }

    #[test]
    fn filtered_domains_regex_flags_and_parameter_types_do_not_alias() {
        let mut graph = Graph::default();
        let def = vec!["cf(fid) AS MATERIALIZED (SELECT fid FROM file WHERE path GLOB ?1)".into()];
        let a = graph.register(&def, &["a/*".to_owned().into()], 1);
        let b = graph.register(&def, &["b/*".to_owned().into()], 1);
        assert_ne!(a["cf"].id, b["cf"].id);
        let def = vec![
            "terms(term) AS MATERIALIZED (SELECT term FROM vlex WHERE nera_term_matches(?1,term))"
                .into(),
        ];
        let a = graph.register(&def, &["target".to_owned().into()], 1);
        let b = graph.register(&def, &["(?i)target".to_owned().into()], 1);
        assert_ne!(a["terms"].id, b["terms"].id);
        let a = graph.register(&def, &[7.into()], 1);
        let b = graph.register(&def, &["7".to_owned().into()], 1);
        assert_ne!(a["terms"].id, b["terms"].id);
    }
}
