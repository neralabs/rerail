//! The semantic leaves whose sub-rules the SQL engine composes: each terminal returns what the
//! matcher reads of a node before a sub-rule (a binding's nodes, its candidates, a format call's
//! sides, a literal's tree), the sub-rules then being relations over those rows in SQL
//! (`Walk::Slot`, `Walk::Related`). No terminal here matches a sub-rule.
//!
//! A related-node row's values are `file` (NULL: the key's own), `node`, `ord` (the order the
//! matcher walks them in), then the values the walk writes into the candidate's row.

use super::fact::{Verdict, Verdicts, local_id, tree_file};
use crate::code_rule_dsl::literal::{Parsed as LiteralParsed, SubDoc, sub_tree};
use crate::code_rule_dsl::matcher::{
    BindingTest, Binds, FormatArgs, ParseTest, Resolution, Sides, OPEN, PASSES,
};
use crate::code_rule_dsl::tape::TapeDoc;
use crate::code_rule_dsl::value::NodeRef;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::source::Content;
use ast_grep_core::{Doc, Node};
use rusqlite::types::Value;

type Rows = Result<Vec<Verdict>, String>;

/// A node near `anchor` as a value: its number, or NULL where the tree has none there.
fn near<D: Doc>(node: Option<NodeRef>, anchor: &Node<'_, D>) -> Value {
    node.and_then(|node| node.find_near(anchor.root().root(), anchor))
        .map_or(Value::Null, |node| Value::Integer(local_id(node.node_id())))
}

fn integer(value: impl TryInto<i64>) -> Value {
    value.try_into().map_or(Value::Null, Value::Integer)
}

/// A related-node row: `node` (in the key's file), walked at `ord`, writing `sets`.
fn related(node: Value, ord: i64, sets: impl IntoIterator<Item = Value>) -> Verdict {
    let mut values = vec![Value::Null, node, Value::Integer(ord)];
    values.extend(sets);
    Verdict::holds(values)
}

/// The integer a count slot the key carries holds, under its internal name.
fn count<D: Doc>(env: &MetaVarEnv<'_, D>, name: &str) -> Option<i64> {
    let bytes = env.get_transformed(name)?;
    <D::Source as Content>::encode_bytes(bytes).parse().ok()
}

/// `resolves.binding` before its sub-rules (`BindingTest::resolve`): false, unknown, or true
/// with the binding's site, value and annotation (NULL where it has none near the reference); a
/// listed sub-rule whose node the binding lacks makes it false.
pub(in crate::code_rule_dsl::sql) struct BindingSites(pub(in crate::code_rule_dsl::sql) BindingTest);

impl Verdicts for BindingSites {
    fn rows<'t>(&self, node: Node<'t, TapeDoc>, env: &MetaVarEnv<'t, TapeDoc>) -> Rows {
        let test = &self.0;
        Ok(match test.resolve(&node, env) {
            Resolution::No => Vec::new(),
            Resolution::Unknown => vec![Verdict::unknown()],
            Resolution::Found {
                anchor,
                site,
                value,
                annotation,
                ..
            } => {
                let targets = [
                    (&test.site, site),
                    (&test.value, value),
                    (&test.annotation, annotation),
                ];
                if targets
                    .iter()
                    .any(|(rule, target)| rule.is_some() && target.is_none())
                {
                    return Ok(Vec::new());
                }
                vec![Verdict::holds(
                    targets
                        .iter()
                        .map(|(_, target)| near(*target, &anchor))
                        .collect(),
                )]
            }
        })
    }
}

/// The occurrences of a binding's variable an `occurrence` rule searches, in the matcher's order
/// (`binding::match_occurrence`): `resolves.binding`'s, or a `binds` candidate's (its ordinal in
/// the key, under `ordinal`).
pub(in crate::code_rule_dsl::sql) enum Occurrences {
    Binding(BindingTest),
    Binds(Binds, String),
}

impl Verdicts for Occurrences {
    fn rows<'t>(&self, node: Node<'t, TapeDoc>, env: &MetaVarEnv<'t, TapeDoc>) -> Rows {
        let (anchor, occurrences) = match self {
            Self::Binding(test) => match test.resolve(&node, env) {
                Resolution::Found {
                    anchor,
                    occurrences,
                    ..
                } => (anchor, occurrences),
                _ => return Ok(Vec::new()),
            },
            Self::Binds(test, ordinal) => {
                let Some(ordinal) = count(env, ordinal) else {
                    return Ok(Vec::new());
                };
                (node.clone(), test.occurrences(&node, ordinal))
            }
        };
        let root = anchor.root().root();
        Ok(occurrences
            .iter()
            .filter_map(|occurrence| occurrence.find_near(root.clone(), &anchor))
            .enumerate()
            .map(|(ord, found)| {
                related(
                    Value::Integer(local_id(found.node_id())),
                    ord as i64,
                    [],
                )
            })
            .collect())
    }
}

/// `binds`' candidates (`Binds::candidates`): each binding the name node names, in the matcher's
/// order, writing its ordinal, its verdict before its sub-rules, its site, value and annotation,
/// the shadowed binding's check and its site, value and annotation (Python's `redefines`).
pub(in crate::code_rule_dsl::sql) struct BindsCandidates(pub(in crate::code_rule_dsl::sql) Binds);

impl Verdicts for BindsCandidates {
    fn rows<'t>(&self, node: Node<'t, TapeDoc>, env: &MetaVarEnv<'t, TapeDoc>) -> Rows {
        Ok(self
            .0
            .candidates(&node, env)
            .into_iter()
            .enumerate()
            .map(|(ord, candidate)| {
                let mut sets = vec![
                    Value::Integer(candidate.ordinal),
                    Value::Integer(i64::from(candidate.pre)),
                ];
                sets.extend(candidate.nodes.iter().map(|target| near(*target, &node)));
                sets.push(Value::Integer(i64::from(candidate.shadow)));
                sets.extend(candidate.previous.iter().map(|target| near(*target, &node)));
                related(Value::Integer(local_id(node.node_id())), ord as i64, sets)
            })
            .collect())
    }
}

/// The bindings `redefines` reads beside a `binds` candidate (its ordinal in the key, under
/// `ordinal`), in the matcher's order (`Binds::others_of`), writing each one's verdict before its
/// sub-rules and its site, value and annotation.
pub(in crate::code_rule_dsl::sql) struct BindsOthers(
    pub(in crate::code_rule_dsl::sql) Binds,
    pub(in crate::code_rule_dsl::sql) String,
);

impl Verdicts for BindsOthers {
    fn rows<'t>(&self, node: Node<'t, TapeDoc>, env: &MetaVarEnv<'t, TapeDoc>) -> Rows {
        let Some(ordinal) = count(env, &self.1) else {
            return Ok(Vec::new());
        };
        Ok(self
            .0
            .others_of(&node, env, ordinal)
            .into_iter()
            .enumerate()
            .map(|(ord, other)| {
                let mut sets = vec![Value::Integer(i64::from(other.pre))];
                sets.extend(other.nodes.iter().map(|target| near(*target, &node)));
                related(Value::Integer(local_id(node.node_id())), ord as i64, sets)
            })
            .collect())
    }
}

/// What `formatArgs` reads before its sub-rules (`FormatArgs::sides`), by `part`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::code_rule_dsl::sql) enum FormatPart {
    /// Its verdict: false, unknown, or true with its template's tree (a document of the batch).
    Verdict,
    /// The arguments no field consumes: each, writing `PASSES` and whether a match there is only
    /// unknown (not known to be unused); and where others may be, an `OPEN` one last.
    Unused,
    /// The template's fields no argument fills, in its tree, alike.
    Missing,
}

/// `formatArgs`' sides.
pub(in crate::code_rule_dsl::sql) struct FormatSides(
    pub(in crate::code_rule_dsl::sql) FormatArgs,
    pub(in crate::code_rule_dsl::sql) FormatPart,
);

impl Verdicts for FormatSides {
    fn rows<'t>(&self, _node: Node<'t, TapeDoc>, env: &MetaVarEnv<'t, TapeDoc>) -> Rows {
        let (tree, candidates, incomplete) = match self.0.sides(env) {
            Sides::NoMatch => return Ok(Vec::new()),
            Sides::Unknown => {
                return Ok(match self.1 {
                    FormatPart::Verdict => vec![Verdict::unknown()],
                    _ => Vec::new(),
                });
            }
            Sides::Sides {
                tree,
                unused,
                unused_incomplete,
                missing,
                missing_incomplete,
            } => match self.1 {
                FormatPart::Verdict => {
                    return Ok(vec![Verdict::holds(vec![integer(
                        tree_file(&tree).unwrap_or(-1),
                    )])]);
                }
                FormatPart::Unused => (
                    None,
                    unused
                        .into_iter()
                        .map(|(node, known)| (Value::Integer(local_id(node.node_id())), known))
                        .collect::<Vec<_>>(),
                    unused_incomplete,
                ),
                FormatPart::Missing => (
                    tree_file(&tree),
                    missing
                        .into_iter()
                        .map(|(index, known)| (Value::Integer(i64::from(index)), known))
                        .collect(),
                    missing_incomplete,
                ),
            },
        };
        let file = tree.map_or(Value::Null, Value::Integer);
        let mut rows: Vec<Verdict> = candidates
            .into_iter()
            .enumerate()
            .map(|(ord, (node, known))| {
                Verdict::holds(vec![
                    file.clone(),
                    node,
                    Value::Integer(ord as i64),
                    Value::Integer(i64::from(PASSES)),
                    Value::Integer(i64::from(if known { PASSES } else { OPEN })),
                ])
            })
            .collect();
        if incomplete {
            // More candidates may be than the analysis names: a miss is unknown.
            rows.push(Verdict::holds(vec![
                file,
                Value::Integer(0),
                Value::Integer(rows.len() as i64),
                Value::Integer(i64::from(OPEN)),
                Value::Integer(i64::from(PASSES)),
            ]));
        }
        Ok(rows)
    }
}

/// `parse`'s tree at a node (`ParseTest::text_tree`): false, unknown, or true with the tree's
/// number among the batch's documents.
pub(in crate::code_rule_dsl::sql) struct Parsed(pub(in crate::code_rule_dsl::sql) ParseTest);

impl Verdicts for Parsed {
    fn rows<'t>(&self, node: Node<'t, TapeDoc>, _env: &MetaVarEnv<'t, TapeDoc>) -> Rows {
        Ok(match self.0.text_tree(&node) {
            Some(LiteralParsed::Tree(tree)) => {
                vec![Verdict::holds(vec![integer(tree_file(&tree).unwrap_or(-1))])]
            }
            Some(LiteralParsed::Unknown) => vec![Verdict::unknown()],
            Some(LiteralParsed::NotString) | None => Vec::new(),
        })
    }
}

/// A parsed tree's node as its host reads an export of it: its text and its host range
/// (`parse::host_exports`, `ParseTest::match_tree`); none where the tree maps it to none.
pub(in crate::code_rule_dsl::sql) struct Exported;

impl Verdicts for Exported {
    fn rows<'t>(&self, _node: Node<'t, TapeDoc>, _env: &MetaVarEnv<'t, TapeDoc>) -> Rows {
        Ok(Vec::new())
    }

    fn tree_rows<'t>(&self, node: Node<'t, SubDoc>, _env: &MetaVarEnv<'t, SubDoc>) -> Rows {
        let Some(range) = sub_tree(&node).and_then(|tree| tree.host_range(node.node_id())) else {
            return Ok(Vec::new());
        };
        Ok(vec![Verdict::holds(vec![
            Value::Text(node.text().into_owned()),
            Value::Text(format!("{}:{}", range.start, range.end)),
        ])])
    }
}
