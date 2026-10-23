//! Share functional AST paths within one bound predicate. Child witnesses remain
//! independent existential queries; only parent/previous/next are unique keys.
use super::ir::{Edge, Set};

#[derive(Default)]
pub(super) struct Bound {
    paths: Vec<(String, Edge, String)>,
    pub joins: Vec<String>,
    known: Vec<String>,
}
impl Bound {
    pub fn parent(&mut self, root: &str, parent: &str) {
        self.paths.push((root.into(), Edge::Parent, parent.into()));
        self.known.push(parent.into());
    }

    pub fn condition(&mut self, set: &Set, root: &str) -> Result<String, String> {
        super::physical::expand(set)?;
        self.expression(&normalize(set), root)
    }
    fn expression(&mut self, set: &Set, root: &str) -> Result<String, String> {
        Ok(match set {
            Set::Intersect(items) | Set::Union(items) => {
                let intersect = matches!(set, Set::Intersect(_));
                if items.is_empty() {
                    return Ok(if intersect { "1" } else { "0" }.into());
                }
                let conditions = items
                    .iter()
                    .map(|s| self.expression(s, root))
                    .collect::<Result<Vec<_>, _>>()?;
                format!(
                    "({})",
                    conditions.join(if intersect { " AND " } else { " OR " })
                )
            }
            Set::Complement(inner) => format!("NOT ({})", self.expression(inner, root)?),
            Set::Preimage(edge @ (Edge::Parent | Edge::Previous | Edge::Next), inner) => {
                let alias = if let Some((_, _, name)) =
                    self.paths.iter().find(|(at, e, _)| at == root && e == edge)
                {
                    name.clone()
                } else {
                    let name = format!("bound{}", self.paths.len());
                    let column = match edge {
                        Edge::Parent => "parent",
                        Edge::Previous => "prev",
                        Edge::Next => "next",
                        _ => unreachable!(),
                    };
                    self.joins.push(format!(" LEFT JOIN node {name} ON {name}.file={root}.file AND {name}.id={root}.{column}"));
                    self.paths.push((root.into(), *edge, name.clone()));
                    name
                };
                // Missing functional witnesses mean false, even for an inner complement.
                // CASE also prevents native leaf calls with NULL node arguments.
                let condition = self.expression(inner, &alias)?;
                if self.known.contains(&alias) {
                    condition
                } else if matches!(inner.as_ref(), Set::Kinds(_) | Set::Nonempty) {
                    format!("COALESCE({condition},0)")
                } else {
                    format!("CASE WHEN {alias}.id IS NOT NULL THEN {condition} ELSE 0 END")
                }
            }
            _ => super::emit::unshared_condition(set, root)?,
        })
    }
}

fn normalize(set: &Set) -> Set {
    match set {
        Set::Intersect(items) | Set::Union(items) => {
            let union = matches!(set, Set::Union(_));
            let mut flat = Vec::new();
            for item in items {
                let item = normalize(item);
                match item {
                    Set::Union(items) if union => flat.extend(items),
                    Set::Intersect(items) if !union => flat.extend(items),
                    Set::Empty if union => {}
                    Set::AllNodes if !union => {}
                    item => flat.push(item),
                }
            }
            if union {
                let mut kinds = Vec::new();
                let mut edges: Vec<(Edge, Vec<Set>)> = Vec::new();
                let mut rest = Vec::new();
                for item in flat {
                    match item {
                        Set::Kinds(values) => kinds.extend(values),
                        Set::Preimage(edge, inner) => {
                            if let Some((_, values)) = edges.iter_mut().find(|(e, _)| *e == edge) {
                                values.push(*inner);
                            } else {
                                edges.push((edge, vec![*inner]));
                            }
                        }
                        item => rest.push(item),
                    }
                }
                if !kinds.is_empty() {
                    kinds.sort_unstable();
                    kinds.dedup();
                    rest.push(Set::Kinds(kinds));
                }
                rest.extend(edges.into_iter().map(|(edge, items)| {
                    Set::Preimage(edge, Box::new(normalize(&Set::Union(items))))
                }));
                flat = rest;
            }
            if flat.len() == 1 {
                flat.remove(0)
            } else if union {
                Set::Union(flat)
            } else {
                Set::Intersect(flat)
            }
        }
        Set::Complement(inner) => Set::Complement(Box::new(normalize(inner))),
        Set::Preimage(edge, inner) => Set::Preimage(*edge, Box::new(normalize(inner))),
        other => other.clone(),
    }
}

/// Recover a total rooted set from a physical join tree for shared seed emission.
pub(super) fn arm_set(arm: &super::physical::Arm, node: usize) -> Set {
    use super::physical::Predicate;
    let mut parts: Vec<Set> = arm
        .references
        .iter()
        .filter(|(at, _)| *at == node)
        .map(|(_, id)| Set::Reference(*id))
        .collect();
    for p in &arm.predicates {
        let (at, set) = match p {
            Predicate::Kinds(at, kinds) => (*at, Set::Kinds(kinds.clone())),
            Predicate::Nonempty(at) => (*at, Set::Nonempty),
            Predicate::Regex(at, term) => (*at, Set::Regex(*term)),
            Predicate::Pattern(at, term) => (*at, Set::Pattern(*term)),
            Predicate::Region(at, region) => (*at, Set::Within(*region)),
            Predicate::Absent(at, inner) => (*at, Set::Complement(Box::new(inner.clone()))),
            Predicate::Keys(..) => unreachable!("key restrictions are added after seed planning"),
        };
        if at == node {
            parts.push(set);
        }
    }
    for (from, edge, to) in &arm.edges {
        if *from == node {
            parts.push(Set::Preimage(*edge, Box::new(arm_set(arm, *to))));
        }
    }
    Set::Intersect(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_functional_paths_keep_absence_and_independent_children() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE node(file,id,parent,prev,next,kind); INSERT INTO node VALUES(0,1,NULL,NULL,NULL,1),(0,2,1,NULL,3,2),(0,3,1,2,NULL,3),(1,1,NULL,NULL,NULL,1);").unwrap();
        let parent = |set| Set::Preimage(Edge::Parent, Box::new(set));
        let child = |set| Set::Preimage(Edge::Child, Box::new(set));
        let not = |set| Set::Complement(Box::new(set));
        for (set, expected, joins) in [
            (
                Set::Union(vec![
                    parent(Set::Kinds(vec![1])),
                    parent(Set::Kinds(vec![9])),
                ]),
                vec![(0, 2), (0, 3)],
                1,
            ),
            (parent(not(Set::Empty)), vec![(0, 2), (0, 3)], 1),
            (not(parent(Set::AllNodes)), vec![(0, 1), (1, 1)], 1),
            (
                Set::Intersect(vec![child(Set::Kinds(vec![2])), child(Set::Kinds(vec![3]))]),
                vec![(0, 1)],
                0,
            ),
            (
                child(Set::Intersect(vec![
                    Set::Kinds(vec![2]),
                    Set::Kinds(vec![3]),
                ])),
                vec![],
                0,
            ),
        ] {
            let mut bound = Bound::default();
            let condition = bound.condition(&set, "n").unwrap();
            assert_eq!(bound.joins.len(), joins);
            let sql = format!(
                "SELECT n.file,n.id FROM node n{} WHERE {condition} ORDER BY n.file,n.id",
                bound.joins.join("")
            );
            let actual: Vec<(i64, i64)> = db
                .prepare(&sql)
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(actual, expected, "{sql}");
        }
    }
}

#[test]
fn reference_shape_nonempty_text_preserves_native_regex_semantics() {
    use crate::code_rule_dsl::test_support::grammar_matchers;
    use serde_json::json;
    let sources = vec![
        ("empty.py".into(), String::new()),
        ("unicode.py".into(), "café = 'é'\nprint(café)\n".into()),
        ("multiline.py".into(), "x = '''a\nb'''\n\n".into()),
    ];
    for rule in [
        json!({"regex":"(?s)."}),
        json!({"not":{"regex":"(?s)."}}),
        json!({"follows":{"stopBy":"neighbor","regex":"(?s)."}}),
        json!({"not":{"follows":{"stopBy":"neighbor","regex":"(?s)."}}}),
    ] {
        let policy = super::tests::policy(rule, json!({}));
        let measured = super::tests::measure(&grammar_matchers(&policy)[0], &sources);
        assert_eq!(measured["parity"], true);
        assert!(!measured["sql"].as_str().unwrap().contains("node_regex("));
    }
}
