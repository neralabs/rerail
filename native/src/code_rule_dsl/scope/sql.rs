//! SQL's scope: query blocks, the names their `FROM`/`JOIN` relations, DML targets and CTEs bind,
//! and the qualified column references that use those names, read from the vendored
//! tree-sitter-sql tree.
//!
//! The resolution follows SQLFluff 4.4.0 so that its recorded findings are the oracle:
//!
//! * `RF01` (`rules/references/RF01.py`, `utils/analysis/query.py`,
//!   `core/rules/reference.py::object_ref_matches_table`): a qualified reference names a table
//!   (`t.col`, `s.t.col`: the part before the column, with its schema when there is one). It resolves
//!   when its own block, or any enclosing block, binds that name; a name matches a binding exactly or
//!   by suffix (`baz` matches the table `foo.baz` even when `foo.baz` is aliased). The comparison
//!   is on the unquoted text and *case sensitive*: RF01 hands `object_ref_matches_table` the
//!   references as written and the targets as written (the upper-casing in `RF01._resolve_reference`
//!   only feeds the dot-access dialects' `distinct_targets`), so `b.x` does not resolve to `FROM t AS B`.
//!   Only references that belong directly to a `SELECT` are checked, as RF01 starts at
//!   `select_statement`; the references of an `UPDATE`/`DELETE`/`INSERT` itself are not. RF01 anchors
//!   at the *table part* of the reference (`schema2.agent1.code` at `agent1`), which is the range a
//!   reference carries here.
//! * `AL05` (`rules/aliasing/AL05.py`): an alias on a table or function relation that no
//!   reference in its block or a nested block uses. A reference uses the table part of a
//!   qualified reference, and, as AL05 does for PostgreSQL (`allow_row_references`), a bare
//!   single-part reference (`to_json(t)`, `SELECT t`). Aliases of derived tables and `VALUES` are
//!   required by the dialect and never reported; a table named the same twice in one block (self
//!   joins) is not reported either. The names compare as `AL05._cs_str_id` does for `alias_case_check
//!   = dialect` on PostgreSQL: naked identifiers fold to lower case, quoted ones are exact.
//!
//! A CTE name binds in a wrapper block around its `WITH` statement, so the CTE bodies and the main
//! query (its children) all see it; SQLFluff resolves a CTE only as a `FROM` table.

use ast_grep_core::{Doc, Node};
use foldhash::{HashMap, HashMapExt, HashSet, HashSetExt};
use std::ops::Range;

/// What opened a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockKind {
    /// A `select`: the only kind RF01 and AL05 report on.
    Select,
    /// An `update` or `delete`.
    Dml,
    /// An `insert`: its target table binds here and its `select` is a child block.
    Insert,
    /// The wrapper of a `WITH` statement: it binds the CTE names.
    With,
}

/// What a binding names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingKind {
    /// A relation over a named table.
    Table,
    /// A relation over a function call.
    Function,
    /// A derived table (a subquery or `VALUES`) or anything else a relation can be.
    Derived,
    /// A DML target (`UPDATE t`, `DELETE FROM t`, `INSERT INTO t`) or a CTE name.
    Named,
}

#[derive(Debug)]
struct Block {
    parent: Option<usize>,
    kind: BlockKind,
}

#[derive(Debug)]
struct Binding {
    block: usize,
    /// The identifier that names the binding: the alias, else the last part of the table name.
    name: Range<usize>,
    kind: BindingKind,
    /// The relation has an alias.
    aliased: bool,
    /// The name AL05 compares: naked lower-cased, quoted exact.
    folded: String,
    /// The tuples RF01 matches references against.
    targets: Vec<Vec<String>>,
    /// The folded last part of the table name, for AL05's repeated-table exemption.
    table: Option<String>,
    /// The byte offset from which the binding is visible (a CTE is not visible before it).
    visible_from: usize,
}

#[derive(Debug)]
struct Reference {
    block: usize,
    /// The table part of the reference (what RF01 reports); empty for a bare reference.
    range: Range<usize>,
    /// The possible tables as RF01's `_get_table_refs` lists them: the last qualifier part, or the
    /// last two when the reference has a schema.
    possible: Vec<Vec<String>>,
    /// The folded table part AL05 marks as used.
    used: String,
    bare: bool,
}

/// The query blocks of one SQL file with their bindings and references.
#[derive(Debug, Default)]
pub(crate) struct SqlScopes {
    blocks: Vec<Block>,
    bindings: Vec<Binding>,
    references: Vec<Reference>,
}

impl SqlScopes {
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>) -> Self {
        let mut reader = Reader::default();
        reader.read(root);
        reader.scopes
    }

    /// The byte ranges of qualifiers no binding resolves (RF01).
    pub(crate) fn unresolved(&self) -> Vec<Range<usize>> {
        self.references
            .iter()
            .filter(|reference| {
                !reference.bare
                    && self.blocks[reference.block].kind == BlockKind::Select
                    && self.resolve(reference).is_none()
            })
            .map(|reference| reference.range.clone())
            .collect()
    }

    /// The byte ranges of alias names no reference uses (AL05).
    pub(crate) fn unused_aliases(&self) -> Vec<Range<usize>> {
        let mut used: HashSet<(usize, &str)> = HashSet::new();
        for reference in &self.references {
            let mut block = Some(reference.block);
            while let Some(index) = block {
                if self.bindings.iter().any(|binding| {
                    binding.block == index
                        && binding.kind != BindingKind::Named
                        && binding.folded == reference.used
                }) {
                    used.insert((index, &reference.used));
                    break;
                }
                block = self.blocks[index].parent;
            }
        }
        let mut repeats: HashMap<(usize, &str), usize> = HashMap::new();
        for binding in &self.bindings {
            if let Some(table) = &binding.table {
                *repeats.entry((binding.block, table)).or_default() += 1;
            }
        }
        self.bindings
            .iter()
            .filter(|binding| {
                binding.aliased
                    && matches!(binding.kind, BindingKind::Table | BindingKind::Function)
                    && self.blocks[binding.block].kind == BlockKind::Select
                    && binding.table.as_ref().is_none_or(|table| {
                        repeats.get(&(binding.block, table.as_str())) == Some(&1)
                    })
                    && !used.contains(&(binding.block, binding.folded.as_str()))
            })
            .map(|binding| binding.name.clone())
            .collect()
    }

    /// The binding a reference at `range` resolves to, as the binding's name range.
    pub(crate) fn binding(&self, range: &Range<usize>) -> Option<Range<usize>> {
        let reference = self
            .references
            .iter()
            .find(|reference| !reference.bare && reference.range == *range)?;
        self.resolve(reference)
            .map(|binding| self.bindings[binding].name.clone())
    }

    /// RF01's `_resolve_reference`: the binding of the reference's own block or an enclosing one
    /// that `object_ref_matches_table` accepts; an exact match wins over a suffix match.
    fn resolve(&self, reference: &Reference) -> Option<usize> {
        let mut block = Some(reference.block);
        while let Some(index) = block {
            let visible = || {
                self.bindings.iter().enumerate().filter(|(_, binding)| {
                    binding.block == index && binding.visible_from <= reference.range.start
                })
            };
            let exact = visible().find(|(_, binding)| {
                binding
                    .targets
                    .iter()
                    .any(|target| reference.possible.contains(target))
            });
            if let Some((found, _)) = exact.or_else(|| {
                visible().find(|(_, binding)| {
                    binding.targets.iter().any(|target| {
                        reference
                            .possible
                            .iter()
                            .any(|possible| suffix_match(possible, target))
                    })
                })
            }) {
                return Some(found);
            }
            block = self.blocks[index].parent;
        }
        None
    }
}

/// `object_ref_matches_table`'s "one is shorter than the other" case: the shorter is a suffix of
/// the longer.
fn suffix_match(possible: &[String], target: &[String]) -> bool {
    (possible.len() < target.len() && possible == &target[target.len() - possible.len()..])
        || (target.len() < possible.len() && target == &possible[possible.len() - target.len()..])
}

/// An identifier's text without its quotes: `"a""b"` is `a"b`.
fn unquoted(text: &str) -> String {
    for (open, close, escaped) in [
        ('"', '"', Some("\"\"")),
        ('`', '`', Some("``")),
        ('\'', '\'', Some("''")),
        ('[', ']', None),
    ] {
        if let Some(inner) = text
            .strip_prefix(open)
            .and_then(|rest| rest.strip_suffix(close))
        {
            return match escaped {
                Some(pair) => inner.replace(pair, &pair[..1]),
                None => inner.to_owned(),
            };
        }
    }
    text.to_owned()
}

/// AL05's `_cs_str_id` on PostgreSQL: naked identifiers fold to lower case, quoted ones are exact.
fn folded(text: &str) -> String {
    let name = unquoted(text);
    if name == text {
        name.to_lowercase()
    } else {
        name
    }
}

#[derive(Default)]
struct Reader {
    scopes: SqlScopes,
}

impl Reader {
    /// Depth-first over the tree with an explicit stack, so a deeply nested expression cannot
    /// overflow the call stack. Each node is read with the block it belongs to; a `select`,
    /// `update`, `delete` or `insert` child opens a block, and the siblings after it (`from`,
    /// `where`, `order_by`, ...) belong to that block, as the grammar writes them beside the
    /// keyword node rather than inside it.
    fn read<D: Doc>(&mut self, root: &Node<'_, D>) {
        let mut stack = vec![(root.clone(), None::<usize>)];
        while let Some((node, block)) = stack.pop() {
            let kind = node.kind();
            let children: Vec<_> = node.children().collect();
            let has_cte = children.iter().any(|child| child.kind() == "cte");
            let block = if has_cte {
                Some(self.open(block, BlockKind::With))
            } else {
                block
            };
            self.read_node(&node, &kind, &children, block);
            let mut current = block;
            let mut pushed = Vec::with_capacity(children.len());
            for child in children {
                let child_kind = match child.kind().as_ref() {
                    "select" => Some(BlockKind::Select),
                    "update" | "delete" => Some(BlockKind::Dml),
                    "insert" => Some(BlockKind::Insert),
                    _ => None,
                };
                if let Some(child_kind) = child_kind {
                    current = Some(self.open(block, child_kind));
                }
                pushed.push((child, current));
            }
            stack.extend(pushed.into_iter().rev());
        }
    }

    fn open(&mut self, parent: Option<usize>, kind: BlockKind) -> usize {
        self.scopes.blocks.push(Block { parent, kind });
        self.scopes.blocks.len() - 1
    }

    /// What `node` itself declares or references in `block`.
    fn read_node<D: Doc>(
        &mut self,
        node: &Node<'_, D>,
        kind: &str,
        children: &[Node<'_, D>],
        block: Option<usize>,
    ) {
        let Some(block) = block else { return };
        match kind {
            "relation" | "lateral_join" => self.bind_relation(node, children, block),
            "from" | "insert" => {
                // `DELETE FROM t [AS a]` and `INSERT INTO t [AS a]` write the target table
                // directly, not as a relation.
                if let Some(table) = children.iter().find(|c| c.kind() == "object_reference") {
                    let alias = children.iter().find(|c| node_is_alias(node, c));
                    self.bind_table(table, alias, BindingKind::Named, block);
                }
            }
            "cte" => {
                if let Some(name) = children.iter().find(|c| c.kind() == "identifier") {
                    let text = name.text();
                    self.scopes.bindings.push(Binding {
                        block,
                        name: name.range(),
                        kind: BindingKind::Named,
                        aliased: false,
                        folded: folded(&text),
                        targets: vec![vec![unquoted(&text)]],
                        table: None,
                        visible_from: node.range().start,
                    });
                }
            }
            "field" | "all_fields" => self.read_reference(node, kind, children, block),
            _ => {}
        }
    }

    fn bind_relation<D: Doc>(
        &mut self,
        node: &Node<'_, D>,
        children: &[Node<'_, D>],
        block: usize,
    ) {
        let alias = children.iter().find(|child| node_is_alias(node, child));
        if let Some(table) = children.iter().find(|c| c.kind() == "object_reference") {
            self.bind_table(table, alias, BindingKind::Table, block);
            return;
        }
        let Some(alias) = alias else { return };
        let kind = if children.iter().any(|c| c.kind() == "invocation") {
            BindingKind::Function
        } else {
            BindingKind::Derived
        };
        let text = alias.text();
        self.scopes.bindings.push(Binding {
            block,
            name: alias.range(),
            kind,
            aliased: true,
            folded: folded(&text),
            targets: vec![vec![unquoted(&text)]],
            table: None,
            visible_from: 0,
        });
    }

    /// A table relation, with its alias if it has one: it binds the alias, else the last part of the
    /// name, and always the qualified name (RF01's `_alias_info_as_tuples`).
    fn bind_table<'a, D: Doc>(
        &mut self,
        table: &Node<'a, D>,
        alias: Option<&Node<'a, D>>,
        kind: BindingKind,
        block: usize,
    ) {
        let parts: Vec<_> = table
            .children()
            .filter(|part| part.kind() == "identifier")
            .collect();
        let Some(last) = parts.last() else { return };
        let qualified: Vec<String> = parts.iter().map(|part| unquoted(&part.text())).collect();
        // RF01 lists the alias only when there is one (`_alias_info_as_tuples`): an unaliased table
        // is known by its qualified name alone, which a shorter reference still matches by suffix.
        let (name, targets) = match alias {
            Some(alias) => (alias, vec![vec![unquoted(&alias.text())], qualified]),
            None => (last, vec![qualified]),
        };
        self.scopes.bindings.push(Binding {
            block,
            name: name.range(),
            kind,
            aliased: alias.is_some(),
            folded: folded(&name.text()),
            targets,
            table: Some(folded(&last.text())),
            visible_from: 0,
        });
    }

    fn read_reference<D: Doc>(
        &mut self,
        node: &Node<'_, D>,
        kind: &str,
        children: &[Node<'_, D>],
        block: usize,
    ) {
        let Some(first) = children.first() else {
            return;
        };
        if first.kind() == "object_reference" {
            // The qualifier's parts, then the column (or `*`) that closes the reference.
            let parts: Vec<_> = first
                .children()
                .filter(|part| part.kind() == "identifier")
                .collect();
            let Some(table) = parts.last() else { return };
            let table_text = unquoted(&table.text());
            let possible = if parts.len() >= 2 {
                let schema = &parts[parts.len() - 2];
                vec![vec![unquoted(&schema.text()), table_text]]
            } else {
                vec![vec![table_text]]
            };
            self.scopes.references.push(Reference {
                block,
                range: table.range(),
                possible,
                used: folded(&table.text()),
                bare: false,
            });
        } else if kind == "field"
            && children.len() == 1
            && first.kind() == "identifier"
            && node.field("name").is_some()
        {
            self.scopes.references.push(Reference {
                block,
                range: first.range(),
                possible: Vec::new(),
                used: folded(&first.text()),
                bare: true,
            });
        }
    }
}

/// Whether `candidate` is the alias of `node`: the child under the `alias` field.
fn node_is_alias<D: Doc>(node: &Node<'_, D>, candidate: &Node<'_, D>) -> bool {
    node.field("alias")
        .is_some_and(|alias| alias.range() == candidate.range())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::value::sql::test_grammar;
    use serde_json::Value as Json;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// SQLFluff's recording on its own fixtures: `packs/sql/upstream/sqlfluff`.
    fn sqlfluff_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs/sql/upstream/sqlfluff")
    }

    fn json(name: &str) -> Json {
        let text = std::fs::read_to_string(sqlfluff_dir().join(name)).expect("recording");
        serde_json::from_str(&text).expect("json")
    }

    /// A finding as the oracle states it: the 1-based line and 1-based code-point column.
    type Finding = (String, String, u64, u64);

    fn position(source: &str, offset: usize) -> (u64, u64) {
        let before = &source[..offset];
        let line = before.matches('\n').count() as u64 + 1;
        let column = before.rsplit('\n').next().unwrap().chars().count() as u64 + 1;
        (line, column)
    }

    #[derive(Debug)]
    enum Mismatch {
        /// SQLFluff recorded it; the scope does not report it.
        Missing,
        /// The scope reports it; SQLFluff did not record it.
        Extra,
    }

    const QUOTED_CASE: &str = "recorded under ansi, where a naked identifier folds to upper case so `a` matches `\"A\"`; the scope folds as PostgreSQL does (lower case), and SQLFluff run with dialect postgres reports this AL05 too (checked); `sameFindings` is false in fixtures.cases.json";

    /// Every difference between the recording and the scope: (path, rule, line, column, kind, reason).
    const MISMATCHES: &[(&str, &str, u64, u64, Mismatch, &str)] = &[
        (
            "ansi/AL05/test_ansi_function_not_table_parameter.sql",
            "AL05",
            2,
            18,
            Mismatch::Missing,
            "ansi has no row references, so `TO_JSON_STRING(t)` does not use `t`; PostgreSQL has (AL05 `allow_row_references`, issue #7039) and SQLFluff with dialect postgres reports nothing here (checked); `sameFindings` is false in fixtures.cases.json",
        ),
        (
            "ansi/AL05/test_fail_naked_select_single_quoted_table_wrong_case.sql",
            "AL05",
            2,
            14,
            Mismatch::Missing,
            "ansi-only: `tab1 as 'a'` is not ansi or PostgreSQL (`parses` is false in fixtures.cases.json; dialect postgres fails to parse it); the string alias keeps its case under ansi folding (`a` is `A`, `'a'` stays) and so is unused, the scope reads it as the name `a`",
        ),
        (
            "ansi/AL05/test_pass_naked_select_quoted_table.sql",
            "AL05",
            2,
            14,
            Mismatch::Extra,
            QUOTED_CASE,
        ),
        (
            "ansi/AL05/test_pass_naked_select_quoted_table_subquery.sql",
            "AL05",
            2,
            17,
            Mismatch::Extra,
            QUOTED_CASE,
        ),
        (
            "ansi/AL05/test_pass_naked_select_single_quoted_table.sql",
            "AL05",
            2,
            14,
            Mismatch::Extra,
            "ansi-only: `tab1 as 'A'` is not ansi or PostgreSQL (`parses` is false in fixtures.cases.json); `a.col1` folds to lower case and `'A'` stays upper, as in QUOTED_CASE",
        ),
        (
            "ansi/AL05/test_pass_quoted_select_naked_table.sql",
            "AL05",
            2,
            14,
            Mismatch::Extra,
            QUOTED_CASE,
        ),
        (
            "ansi/AL05/test_pass_quoted_select_naked_table_subquery.sql",
            "AL05",
            2,
            17,
            Mismatch::Extra,
            QUOTED_CASE,
        ),
        (
            "ansi/RF01/test_fail_redundantly_bracketed_join_target_unknown_table.sql",
            "AL05",
            4,
            25,
            Mismatch::Extra,
            "grammar shape: `JOIN ((b JOIN c ON b.id = c.id))` parses with ERROR nodes and `c.id` is a bare object_reference inside one, not a field, so `c` looks unused",
        ),
        (
            "ansi/RF01/test_pass_redundantly_bracketed_join_target_alias.sql",
            "AL05",
            4,
            25,
            Mismatch::Extra,
            "grammar shape: the same redundantly bracketed join as above",
        ),
        (
            "postgres/RF01/test_pass_postgres_create_policy_using_target_table.sql",
            "RF01",
            8,
            23,
            Mismatch::Extra,
            "grammar shape: `CREATE POLICY ... ON table_1 USING (...)` parses as ERROR plus a bogus select, so the policy's target table (RF01's implicit target, `_get_implicit_targets`) is not bound",
        ),
        (
            "sqlite/AL05/test_pass_sqlite_different_escaped_quotes.sql",
            "RF01",
            3,
            1,
            Mismatch::Missing,
            "sqlite-only: `'`''a\"'.id` qualifies a column with a string literal, which sqlite reads as an identifier; the grammar reads a literal, not an object_reference",
        ),
        (
            "sqlite/AL05/test_pass_sqlite_different_escaped_quotes.sql",
            "AL05",
            4,
            17,
            Mismatch::Extra,
            "sqlite-only: the alias is used by the string-literal qualifier above, which the grammar does not read",
        ),
    ];

    fn read_scopes(source: &str) -> SqlScopes {
        SqlScopes::read(&test_grammar::parse(source).root())
    }

    /// The `nth` whole-word occurrence of `needle` in `source`.
    fn at(source: &str, needle: &str, nth: usize) -> Range<usize> {
        let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        let start = source
            .match_indices(needle)
            .filter(|(start, _)| {
                !word(source[..*start].chars().next_back())
                    && !word(source[start + needle.len()..].chars().next())
            })
            .nth(nth)
            .unwrap()
            .0;
        start..start + needle.len()
    }

    #[test]
    fn qualifiers_resolve_to_their_binding() {
        let source = "SELECT t.a, s.u.b FROM s.u JOIN x AS t ON true";
        let scopes = read_scopes(source);
        assert!(scopes.unresolved().is_empty());
        assert!(scopes.unused_aliases().is_empty());
        assert_eq!(
            scopes.binding(&at(source, "t", 0)),
            Some(at(source, "t", 1))
        );
        // `s.u.b` anchors at its table part and resolves to the unaliased `s.u`.
        assert_eq!(
            scopes.binding(&at(source, "u", 0)),
            Some(at(source, "u", 1))
        );
    }

    #[test]
    fn unknown_and_case_different_qualifiers_are_unresolved() {
        let source = "SELECT z.a, B.c FROM x AS b";
        assert_eq!(
            read_scopes(source).unresolved(),
            vec![at(source, "z", 0), at(source, "B", 0)]
        );
        let source = "SELECT * FROM s.x WHERE t.x.a = 1 AND o.x.a = 1";
        assert_eq!(
            read_scopes(source).unresolved(),
            vec![at(source, "x", 1), at(source, "x", 2)]
        );
    }

    #[test]
    fn nested_blocks_see_their_parents_and_ctes_bind() {
        let source = "WITH c AS (SELECT 1 FROM x) SELECT c.a FROM y AS t \
                      WHERE EXISTS (SELECT 1 FROM z WHERE z.k = t.k)";
        let scopes = read_scopes(source);
        assert!(scopes.unresolved().is_empty());
        assert!(scopes.unused_aliases().is_empty());
        assert_eq!(
            scopes.binding(&at(source, "c", 1)),
            Some(at(source, "c", 0))
        );
        let source = "SELECT 1 FROM (SELECT t.a FROM x) AS d WHERE d.a = 1";
        assert_eq!(read_scopes(source).unresolved(), vec![at(source, "t", 0)]);
    }

    #[test]
    fn dml_targets_bind_and_only_selects_report() {
        let source = "UPDATE t SET a = (SELECT 1 FROM b WHERE b.k = t.k) WHERE q.a = 1";
        assert!(read_scopes(source).unresolved().is_empty());
        let source =
            "DELETE FROM s.t AS d WHERE EXISTS (SELECT 1 FROM u WHERE u.k = d.k AND w.k = 1)";
        assert_eq!(read_scopes(source).unresolved(), vec![at(source, "w", 0)]);
    }

    #[test]
    fn unused_aliases_follow_al05() {
        let source = "SELECT 1 FROM x AS a JOIN x AS b ON true";
        assert!(
            read_scopes(source).unused_aliases().is_empty(),
            "a repeated table is exempt"
        );
        let source = "SELECT 1 FROM x AS a JOIN (SELECT 1) AS q ON true JOIN y r ON r.k = 1";
        assert_eq!(
            read_scopes(source).unused_aliases(),
            vec![at(source, "a", 0)]
        );
        let source = "SELECT to_json(t) FROM x AS t";
        assert!(
            read_scopes(source).unused_aliases().is_empty(),
            "a row reference uses the alias"
        );
    }

    #[test]
    fn reproduces_sqlfluff_rf01_and_al05() {
        let recording = json("fixtures.expected.json");
        let mut recorded: BTreeSet<Finding> = BTreeSet::new();
        for finding in recording["findings"].as_array().unwrap() {
            let rule = finding["rule"].as_str().unwrap();
            if rule == "RF01" || rule == "AL05" {
                recorded.insert((
                    rule.to_owned(),
                    finding["path"].as_str().unwrap().to_owned(),
                    finding["line"].as_u64().unwrap(),
                    finding["column"].as_u64().unwrap(),
                ));
            }
        }
        let mut reproduced: BTreeSet<Finding> = BTreeSet::new();
        for path in recording["source"]["files"].as_array().unwrap() {
            let path = path.as_str().unwrap();
            let source = std::fs::read_to_string(sqlfluff_dir().join("fixtures").join(path))
                .expect("fixture");
            let grep = test_grammar::parse(&source);
            let scopes = SqlScopes::read(&grep.root());
            for (rule, ranges) in [
                ("RF01", scopes.unresolved()),
                ("AL05", scopes.unused_aliases()),
            ] {
                for range in ranges {
                    let (line, column) = position(&source, range.start);
                    reproduced.insert((rule.to_owned(), path.to_owned(), line, column));
                }
            }
        }
        let mut missing: Vec<Finding> = recorded.difference(&reproduced).cloned().collect();
        let mut extra: Vec<Finding> = reproduced.difference(&recorded).cloned().collect();
        for (path, rule, line, column, kind, _reason) in MISMATCHES {
            let finding = (rule.to_string(), path.to_string(), *line, *column);
            let unexplained = match kind {
                Mismatch::Missing => &mut missing,
                Mismatch::Extra => &mut extra,
            };
            let Some(position) = unexplained.iter().position(|f| *f == finding) else {
                panic!("stale mismatch entry {finding:?} ({kind:?})");
            };
            unexplained.remove(position);
        }
        assert!(missing.is_empty(), "recorded, not reproduced: {missing:#?}");
        assert!(extra.is_empty(), "reproduced, not recorded: {extra:#?}");
        assert_eq!(recorded.len(), 40, "RF01 28 + AL05 12 recorded");
        assert_eq!(reproduced.len(), 46);
    }
}
