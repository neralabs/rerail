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
//! A CTE name binds in a wrapper block around its `WITH` statement, so the later CTE bodies (all
//! of them under `RECURSIVE`) and the main query (its children) see it, as PostgreSQL scopes WITH
//! queries. That binding is no RF01 target: SQLFluff resolves a CTE only as a `FROM` table, so
//! `c.a` resolves through a relation over `c` and is unresolved without one. A relation over the
//! CTE's unqualified name uses it, as a qualifier naming it does; a DML target is used by its own
//! statement. A `CREATE POLICY` or `ALTER POLICY` binds its table in a block around its `USING` and
//! `WITH CHECK` expressions, so the queries nested in them resolve a qualifier naming it, as RF01's
//! `_get_implicit_targets` adds the policy's table on PostgreSQL.
//!
//! `visit` puts the blocks, bindings and qualified references into the file's `ScopeTable` (each
//! block a `ScopeKind::Query` scope, each binding keyed by its folded name) and keeps the
//! `SqlScopes` beside them, which resolve a reference and count a binding's uses
//! (`ScopeRules for Sql`, `Usage::sql`).

use super::{Binding as TableBinding, BuildError, Builder, Form, ScopeKind, Site, VALUE};
use crate::code_rule_dsl::value::NodeRef;
use ast_grep_core::{Doc, Node};
use foldhash::{HashMap, HashMapExt};
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
    /// A `CREATE POLICY` or `ALTER POLICY`: it binds the policy's table, which its `USING` and
    /// `WITH CHECK` expressions and the queries nested in them see.
    Policy,
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
    /// A DML target (`DELETE FROM t`, `INSERT INTO t`; the grammar writes `UPDATE t`'s as a
    /// relation), a policy's table or a CTE name.
    Named,
}

#[derive(Debug)]
struct Block {
    parent: Option<usize>,
    kind: BlockKind,
    /// From the node that opens the block to the end of its statement.
    range: Range<usize>,
}

#[derive(Debug)]
struct Binding {
    block: usize,
    /// The identifier that names the binding: the alias, else the last part of the table name.
    name: NodeRef,
    /// The relation, CTE or DML statement clause that binds it.
    site: NodeRef,
    form: Form,
    kind: BindingKind,
    /// The relation has an alias. Read by the AL05 oracle (`unused_aliases`); the pack's AL05
    /// rule states the exemptions in its own terms.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    aliased: bool,
    /// The name AL05 compares, and the table's key: naked lower-cased, quoted exact.
    folded: String,
    /// The tuples RF01 matches references against.
    targets: Vec<Vec<String>>,
    /// The folded last part of the table name, for AL05's repeated-table exemption (the
    /// oracle's, as `aliased`).
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    table: Option<String>,
    /// For a relation over a table named by one part, that name folded: the `WITH` query of the
    /// name it selects from, where one is visible (`SqlScopes::uses`).
    cte: Option<String>,
    /// The byte offset from which the binding is visible: a CTE after its own definition, or
    /// throughout its `WITH` under `RECURSIVE`.
    visible_from: usize,
}

#[derive(Debug)]
struct Reference {
    block: usize,
    /// The table part of the reference (what RF01 reports); for a bare reference, its name.
    range: Range<usize>,
    /// The possible tables as RF01's `_get_table_refs` lists them: the last qualifier part, or the
    /// last two when the reference has a schema.
    possible: Vec<Vec<String>>,
    /// The folded table part AL05 marks as used.
    used: String,
    bare: bool,
}

pub use super::Uses;

/// The query blocks of one SQL file with their bindings and references.
#[derive(Debug, Default)]
pub(crate) struct SqlScopes {
    blocks: Vec<Block>,
    bindings: Vec<Binding>,
    /// Sorted by start byte.
    references: Vec<Reference>,
    /// Each block's bindings, by index.
    by_block: Vec<Vec<usize>>,
    /// Each binding's scope in the file's `ScopeTable` and its index among that scope's
    /// bindings of its name (`visit`).
    ids: Vec<(u32, usize)>,
}

/// Visits the SQL file at `root` into `builder`'s table: a `Query` scope per block (block `i` is
/// scope `i + 1`, under the file's scope 0), a binding per relation, CTE and DML target under its
/// folded name, and a reference per qualified column reference's table part; the table keeps
/// the `SqlScopes`, which resolve those references.
pub(crate) fn visit<D: Doc>(
    builder: &mut Builder<'_>,
    root: &Node<'_, D>,
) -> Result<(), BuildError> {
    let mut scopes = SqlScopes::build(root, &mut || {
        builder.enter()?;
        builder.leave();
        Ok(())
    })?;
    for block in &scopes.blocks {
        builder.current = block.parent.map_or(0, |parent| parent as u32 + 1);
        builder.push(ScopeKind::Query, block.range.clone());
    }
    let mut ids = Vec::with_capacity(scopes.bindings.len());
    for binding in &scopes.bindings {
        let scope = binding.block as u32 + 1;
        let index = builder
            .scope(scope)
            .bindings
            .get(binding.folded.as_str())
            .map_or(0, Vec::len);
        let mut bound = TableBinding::new(
            binding.name.start as usize,
            VALUE,
            Site {
                form: binding.form,
                node: Some(binding.site),
                value: None,
                annotation: None,
            },
        );
        bound.name = Some(binding.name);
        builder.bind(scope, &binding.folded, bound);
        ids.push((scope, index));
    }
    for reference in scopes.references.iter().filter(|reference| !reference.bare) {
        builder.current = reference.block as u32 + 1;
        builder.reference(reference.range.clone(), VALUE);
    }
    builder.current = 0;
    scopes.ids = ids;
    builder.table.sql = Some(Box::new(scopes));
    Ok(())
}

impl super::FileResolution for SqlScopes {
    fn resolved(&self, start: usize, end: usize) -> Option<(u32, &str, usize)> {
        SqlScopes::resolved(self, start, end)
    }
    fn reported(&self, start: usize, end: usize) -> bool {
        SqlScopes::reported(self, start, end)
    }
    fn usage(&self) -> Vec<(u32, &str, usize, Uses)> {
        SqlScopes::usage(self).collect()
    }
}

impl SqlScopes {
    /// The scopes of `root`'s file, `charge` paying for each node read.
    fn build<D: Doc>(
        root: &Node<'_, D>,
        charge: &mut dyn FnMut() -> Result<(), BuildError>,
    ) -> Result<Self, BuildError> {
        let mut reader = Reader::default();
        reader.read(root, charge)?;
        let mut scopes = reader.scopes;
        scopes
            .references
            .sort_by_key(|reference| reference.range.start);
        scopes.by_block = vec![Vec::new(); scopes.blocks.len()];
        for (index, binding) in scopes.bindings.iter().enumerate() {
            scopes.by_block[binding.block].push(index);
        }
        Ok(scopes)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>) -> Self {
        Self::build(root, &mut || Ok(())).expect("no budget")
    }

    /// The byte ranges of qualifiers no binding resolves (RF01).
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn unresolved(&self) -> Vec<Range<usize>> {
        self.references
            .iter()
            .filter(|reference| {
                !reference.bare && self.checked(reference) && self.resolve(reference).is_none()
            })
            .map(|reference| reference.range.clone())
            .collect()
    }

    /// The byte ranges of alias names no reference uses (AL05).
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn unused_aliases(&self) -> Vec<Range<usize>> {
        let uses = self.uses();
        let mut repeats: HashMap<(usize, &str), usize> = HashMap::new();
        for binding in &self.bindings {
            if let Some(table) = &binding.table {
                *repeats.entry((binding.block, table)).or_default() += 1;
            }
        }
        self.bindings
            .iter()
            .zip(&uses)
            .filter(|(binding, uses)| {
                binding.aliased
                    && matches!(binding.kind, BindingKind::Table | BindingKind::Function)
                    && self.blocks[binding.block].kind == BlockKind::Select
                    && binding.table.as_ref().is_none_or(|table| {
                        repeats.get(&(binding.block, table.as_str())) == Some(&1)
                    })
                    && uses.reads == 0
            })
            .map(|(binding, _)| binding.name.start as usize..binding.name.end as usize)
            .collect()
    }

    /// The binding a reference at `range` resolves to, as the binding's name range.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn binding(&self, range: &Range<usize>) -> Option<Range<usize>> {
        let reference = self.reference_at(range.start, range.end)?;
        self.resolve(reference).map(|binding| {
            let name = self.bindings[binding].name;
            name.start as usize..name.end as usize
        })
    }

    /// The qualified reference whose table part spans exactly `start..end`.
    fn reference_at(&self, start: usize, end: usize) -> Option<&Reference> {
        let first = self
            .references
            .partition_point(|reference| reference.range.start < start);
        self.references[first..]
            .iter()
            .take_while(|reference| reference.range.start == start)
            .find(|reference| !reference.bare && reference.range.end == end)
    }

    /// The binding the qualified reference spanning `start..end` resolves to: its scope in the
    /// file's table, its name there and its index among that scope's bindings of the name.
    pub(crate) fn resolved(&self, start: usize, end: usize) -> Option<(u32, &str, usize)> {
        let binding = self.resolve(self.reference_at(start, end)?)?;
        let (scope, index) = self.ids[binding];
        Some((scope, &self.bindings[binding].folded, index))
    }

    /// Whether RF01 reads the qualified reference spanning `start..end`: one of a `SELECT`'s
    /// own clauses, as RF01 starts at `select_statement`.
    pub(crate) fn reported(&self, start: usize, end: usize) -> bool {
        self.reference_at(start, end)
            .is_some_and(|reference| self.checked(reference))
    }

    fn checked(&self, reference: &Reference) -> bool {
        self.blocks[reference.block].kind == BlockKind::Select
    }

    /// Each binding's uses, with the file table's scope, name and index of the binding. AL05's
    /// walk: a reference (qualified, or a bare row reference) uses, in its own block or the
    /// nearest enclosing block that binds its folded name, every relation of that name; a CTE
    /// name or DML target is used by the same walk over those alone. A CTE is used, besides, by
    /// each relation that selects from it, and a DML target by its own statement.
    pub(crate) fn usage(&self) -> impl Iterator<Item = (u32, &str, usize, Uses)> {
        self.uses()
            .into_iter()
            .zip(&self.bindings)
            .zip(&self.ids)
            .map(|((uses, binding), &(scope, index))| (scope, binding.folded.as_str(), index, uses))
    }

    /// `usage`, by binding.
    fn uses(&self) -> Vec<Uses> {
        let mut marks: HashMap<(usize, &str, bool), Uses> = HashMap::new();
        for reference in &self.references {
            for named in [false, true] {
                let mut block = Some(reference.block);
                while let Some(index) = block {
                    if self.by_block[index].iter().any(|&binding| {
                        let binding = &self.bindings[binding];
                        (binding.kind == BindingKind::Named) == named
                            && binding.folded == reference.used
                    }) {
                        let uses = marks
                            .entry((index, reference.used.as_str(), named))
                            .or_default();
                        uses.reads += 1;
                        uses.nested += u32::from(index != reference.block);
                        break;
                    }
                    block = self.blocks[index].parent;
                }
            }
        }
        // PostgreSQL 18.6 looks a `FROM` name without a schema up among the visible `WITH`
        // queries first (`parse_clause.c`, `getNSItemForSpecialRelationTypes`, then
        // `parse_relation.c`, `scanNameSpaceForCTE`, from the innermost query level out).
        for relation in &self.bindings {
            let Some(name) = relation.cte.as_deref() else {
                continue;
            };
            let at = relation.site.start as usize;
            let mut block = Some(relation.block);
            while let Some(index) = block {
                if self.by_block[index].iter().any(|&binding| {
                    let binding = &self.bindings[binding];
                    binding.form == Form::Cte
                        && binding.folded == name
                        && binding.visible_from <= at
                }) {
                    let uses = marks.entry((index, name, true)).or_default();
                    uses.reads += 1;
                    uses.nested += u32::from(index != relation.block);
                    break;
                }
                block = self.blocks[index].parent;
            }
        }
        self.bindings
            .iter()
            .map(|binding| {
                let named = binding.kind == BindingKind::Named;
                let mut uses = marks
                    .get(&(binding.block, binding.folded.as_str(), named))
                    .copied()
                    .unwrap_or_default();
                // `UPDATE`, `DELETE` and `INSERT` write their target whether or not anything
                // qualifies it.
                uses.reads += u32::from(binding.form == Form::Target);
                uses
            })
            .collect()
    }

    /// RF01's `_resolve_reference`: the binding of the reference's own block or an enclosing one
    /// that `object_ref_matches_table` accepts; an exact match wins over a suffix match.
    fn resolve(&self, reference: &Reference) -> Option<usize> {
        let mut block = Some(reference.block);
        while let Some(index) = block {
            let visible = || {
                self.by_block[index]
                    .iter()
                    .copied()
                    .filter(|&binding| self.bindings[binding].visible_from <= reference.range.start)
            };
            let exact = visible().find(|&binding| {
                self.bindings[binding]
                    .targets
                    .iter()
                    .any(|target| reference.possible.contains(target))
            });
            if let Some(found) = exact.or_else(|| {
                visible().find(|&binding| {
                    self.bindings[binding].targets.iter().any(|target| {
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
    /// overflow the call stack; `charge` pays for each node. Each node is read with the block it
    /// belongs to; a `select`, `update`, `delete` or `insert` child opens a block, and the siblings
    /// after it (`from`, `where`, `order_by`, ...) belong to that block, as the grammar writes them
    /// beside the keyword node rather than inside it. An `INSERT`'s `ON CONFLICT` or `ON DUPLICATE
    /// KEY UPDATE` action, from its `ON` on, is the insert's again, not its `SELECT`'s: SQLFluff
    /// 4.4.0 RF01 (dialect postgres) reports no qualifier there, neither `EXCLUDED.y` nor an
    /// unknown `zz.y`, as for `INSERT ... VALUES`. A `CREATE POLICY` or `ALTER POLICY` statement
    /// is a block of its own, around its expressions.
    fn read<D: Doc>(
        &mut self,
        root: &Node<'_, D>,
        charge: &mut dyn FnMut() -> Result<(), BuildError>,
    ) -> Result<(), BuildError> {
        let mut stack = vec![(root.clone(), None::<usize>)];
        while let Some((node, block)) = stack.pop() {
            charge()?;
            let kind = node.kind();
            let children: Vec<_> = node.children().collect();
            let has_cte = children.iter().any(|child| child.kind() == "cte");
            let block = if has_cte {
                Some(self.open(block, BlockKind::With, node.range()))
            } else if is_policy(&kind, &children) {
                Some(self.open(block, BlockKind::Policy, node.range()))
            } else {
                block
            };
            self.read_node(&node, &kind, &children, block);
            let mut current = block;
            let action = if kind == "insert" {
                children
                    .iter()
                    .find(|child| child.kind() == "keyword_on")
                    .map(|child| child.range().start)
            } else {
                None
            };
            let mut pushed = Vec::with_capacity(children.len());
            for child in children {
                let child_kind = match child.kind().as_ref() {
                    "select" => Some(BlockKind::Select),
                    "update" | "delete" => Some(BlockKind::Dml),
                    "insert" => Some(BlockKind::Insert),
                    _ => None,
                };
                let start = child.range().start;
                if let Some(child_kind) = child_kind {
                    let end = action
                        .filter(|&action| action > start)
                        .unwrap_or(node.range().end);
                    current = Some(self.open(block, child_kind, start..end));
                } else if action == Some(start) {
                    current = block;
                }
                pushed.push((child, current));
            }
            stack.extend(pushed.into_iter().rev());
        }
        Ok(())
    }

    fn open(&mut self, parent: Option<usize>, kind: BlockKind, range: Range<usize>) -> usize {
        self.scopes.blocks.push(Block {
            parent,
            kind,
            range,
        });
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
                    self.bind_table(node, table, alias, BindingKind::Named, block);
                }
            }
            "cte" => {
                if let Some(name) = children.iter().find(|c| c.kind() == "identifier") {
                    let text = name.text();
                    self.scopes.bindings.push(Binding {
                        block,
                        name: NodeRef::of(name),
                        site: NodeRef::of(node),
                        form: Form::Cte,
                        kind: BindingKind::Named,
                        aliased: false,
                        folded: folded(&text),
                        // RF01 resolves a qualifier against `FROM` relations only: the CTE
                        // is a target once a relation selects from it, through that relation.
                        targets: Vec::new(),
                        table: None,
                        cte: None,
                        // PostgreSQL 18.6 (`parse_cte.c`, `transformWithClause`): a
                        // non-recursive WITH query sees only the ones before it, not itself;
                        // under `RECURSIVE` each sees them all.
                        visible_from: match node.parent() {
                            Some(with)
                                if with.children().any(|c| c.kind() == "keyword_recursive") =>
                            {
                                with.range().start
                            }
                            _ => node.range().end,
                        },
                    });
                }
            }
            "field" | "all_fields" => self.read_reference(node, kind, children, block),
            // SQLFluff 4.4.0 RF01 (`_get_implicit_targets`, dialect postgres): a query inside a
            // `CREATE POLICY` or `ALTER POLICY` statement resolves against the policy's table, the
            // statement's first table reference, besides its own.
            _ if is_policy(kind, children) => {
                if let Some(table) = children.iter().find(|c| c.kind() == "object_reference") {
                    self.bind_table(node, table, None, BindingKind::Named, block);
                }
            }
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
            self.bind_table(node, table, alias, BindingKind::Table, block);
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
            name: NodeRef::of(alias),
            site: NodeRef::of(node),
            form: Form::Relation,
            kind,
            aliased: true,
            folded: folded(&text),
            targets: vec![vec![unquoted(&text)]],
            table: None,
            cte: None,
            visible_from: 0,
        });
    }

    /// A table relation (or a DML target or policy table, `kind` `Named`, at `site`), with its
    /// alias if it has one: it binds the alias, else the last part of the name, and always the
    /// qualified name (RF01's `_alias_info_as_tuples`).
    fn bind_table<'a, D: Doc>(
        &mut self,
        site: &Node<'_, D>,
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
            name: NodeRef::of(name),
            site: NodeRef::of(site),
            form: if kind == BindingKind::Named {
                Form::Target
            } else {
                Form::Relation
            },
            kind,
            aliased: alias.is_some(),
            folded: folded(&name.text()),
            targets,
            table: Some(folded(&last.text())),
            // An `UPDATE`'s own relation is its target, a table and never a WITH query
            // (PostgreSQL 18.6 `parse_clause.c`, `setTargetTable`, opens the table by name).
            cte: (kind == BindingKind::Table
                && parts.len() == 1
                && site.parent().is_none_or(|parent| parent.kind() != "update"))
            .then(|| folded(&last.text())),
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

/// Whether a node of `kind` with `children` is a `CREATE POLICY` (`create_policy`) or an `ALTER
/// POLICY` (the `alter_object` that starts `ALTER POLICY`).
fn is_policy<D: Doc>(kind: &str, children: &[Node<'_, D>]) -> bool {
    kind == "create_policy"
        || (kind == "alter_object"
            && children
                .get(1)
                .is_some_and(|child| child.kind() == "keyword_policy"))
}

#[cfg(test)]
mod tests;
