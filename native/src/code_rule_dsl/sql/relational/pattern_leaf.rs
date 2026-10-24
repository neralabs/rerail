//! One atomic native pattern match with nullable node/list inputs and typed outputs.
//! SQL owns traversal and composition. This cursor does not publish or read frames.
use super::super::{
    exec::Batch,
    plan::SlotKind,
    terminals::{Terminal, typed_match},
};
use super::capture_cell::{Cell, decode_list, encode_list};
use crate::code_rule_dsl::matcher::node_at;
use ast_grep_core::{matcher::Matcher, meta_var::MetaVarEnv};
use rusqlite::{
    Connection, Error, Result,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor, sqlite3_vtab, sqlite3_vtab_cursor,
    },
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    ffi::{CStr, CString, c_int},
    marker::PhantomData,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

type ListKey = (u32, u32, u32, Vec<Cell>);
#[derive(Default)]
struct ListCache {
    values: HashMap<ListKey, Option<Vec<Cell>>>,
    bytes: usize,
}
const WIDTH: usize = 128;
const TERM: c_int = 1 + WIDTH as c_int;
const FILE: c_int = TERM + 1;
const NODE: c_int = FILE + 1;
const INPUT: c_int = NODE + 1;

fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("typed pattern leaf: {message}"))
}

pub(super) fn register(
    db: &Connection,
    batch: &Arc<Batch>,
    calls: &Arc<AtomicUsize>,
) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(
        c"relation_pattern_typed",
        &MODULE,
        Some((Arc::clone(batch), Arc::clone(calls))),
    )
}

#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    batch: Arc<Batch>,
    calls: Arc<AtomicUsize>,
    lists: Arc<Mutex<ListCache>>,
}

// SAFETY: SQLite's base is the first field. Each cursor retains the immutable batch.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = (Arc<Batch>, Arc<AtomicUsize>);
    type Cursor = Cursor<'v>;

    fn connect(
        db: &mut VTabConnection,
        batch: Option<&Self::Aux>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let outputs = (0..WIDTH)
            .map(|i| format!("c{i} INTEGER"))
            .collect::<Vec<_>>()
            .join(",");
        let inputs = (0..WIDTH)
            .map(|i| format!("i{i} INTEGER HIDDEN"))
            .collect::<Vec<_>>()
            .join(",");
        let schema = CString::new(format!(
            "CREATE TABLE x(truth INTEGER,{outputs},term INTEGER HIDDEN,file INTEGER HIDDEN,node INTEGER HIDDEN,{inputs})"
        ))
        .map_err(|_| invalid("invalid schema"))?;
        Ok((
            Cow::Owned(schema),
            Self {
                base: sqlite3_vtab::default(),
                batch: batch.ok_or_else(|| invalid("missing batch"))?.0.clone(),
                calls: batch.ok_or_else(|| invalid("missing counters"))?.1.clone(),
                lists: Arc::new(Mutex::new(ListCache::default())),
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut keys = [None; 3];
        let mut inputs = BTreeMap::new();
        for (index, constraint) in info.constraints().enumerate() {
            if !constraint.is_usable() {
                continue;
            }
            let column = constraint.column();
            let operator = constraint.operator();
            if (TERM..=NODE).contains(&column)
                && operator == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                keys[(column - TERM) as usize].get_or_insert(index);
            } else if (INPUT..INPUT + WIDTH as c_int).contains(&column)
                && matches!(
                    operator,
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_IS
                        | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_ISNULL
                )
            {
                inputs.entry((column - INPUT) as usize).or_insert((
                    index,
                    operator == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_ISNULL,
                ));
            }
        }
        if keys.iter().any(Option::is_none) || inputs.keys().copied().ne(0..inputs.len()) {
            return Ok(false);
        }
        for (arg, index) in keys.into_iter().enumerate() {
            let mut usage = info.constraint_usage(index.unwrap());
            usage.set_argv_index((arg + 1) as c_int);
            usage.set_omit(true);
        }
        // IS NULL has no SQLite argument. Record it in the plan so it remains a supplied input.
        let mut plan = String::with_capacity(inputs.len());
        let mut argument = 4;
        for &(index, is_null) in inputs.values() {
            let mut usage = info.constraint_usage(index);
            if is_null {
                plan.push('n');
            } else {
                usage.set_argv_index(argument);
                argument += 1;
                plan.push('v');
            }
            usage.set_omit(true);
        }
        info.set_idx_num(inputs.len() as c_int);
        info.set_idx_str(&plan);
        info.set_estimated_cost(1.);
        info.set_estimated_rows(1);
        Ok(true)
    }

    fn open(&'v mut self) -> Result<Self::Cursor> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            batch: self.batch.clone(),
            calls: self.calls.clone(),
            lists: self.lists.clone(),
            term: 0,
            file: 0,
            node: 0,
            inputs: vec![],
            outputs: vec![Cell::Null; WIDTH],
            truth: false,
            done: true,
            marker: PhantomData,
        })
    }
}

#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    batch: Arc<Batch>,
    calls: Arc<AtomicUsize>,
    term: u32,
    file: u32,
    node: u32,
    inputs: Vec<Cell>,
    outputs: Vec<Cell>,
    lists: Arc<Mutex<ListCache>>,
    truth: bool,
    done: bool,
    marker: PhantomData<&'v Table>,
}

// SAFETY: SQLite's base is first. The cursor owns its input tuple and published result.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, width: c_int, plan: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.done = true;
        self.truth = false;
        self.inputs.clear();
        self.outputs.fill(Cell::Null);
        let width = usize::try_from(width).map_err(|_| invalid("negative input width"))?;
        let plan = plan.unwrap_or("");
        if width > WIDTH || plan.len() != width || !plan.bytes().all(|b| b == b'n' || b == b'v') {
            return Err(invalid("invalid input plan"));
        }
        if args.len() != 3 + plan.bytes().filter(|&b| b == b'v').count() {
            return Err(invalid("input argument count differs from plan"));
        }
        self.term = args.get::<u32>(0)?;
        self.file = args.get::<u32>(1)?;
        self.node = args.get::<u32>(2)?;
        let Some(Terminal::Pattern(term)) = self.batch.program.terminals.get(self.term as usize)
        else {
            return Err(invalid("invalid pattern terminal"));
        };
        let has_lists = term.vars.iter().any(|(_, kind)| *kind == SlotKind::List);
        if term
            .vars
            .iter()
            .any(|(_, kind)| !matches!(kind, SlotKind::Node | SlotKind::List))
        {
            return Err(invalid("only node and list captures are supported"));
        }
        if term.vars.len() > WIDTH || term.vars.len() != width {
            return Err(invalid("supplied input width differs from terminal width"));
        }
        if term.reads.iter().copied().ne(0..width) {
            return Err(invalid(
                "terminal reads must contain every variable in column order",
            ));
        }
        let mut argument = 3;
        for input in plan.bytes() {
            let value = if input == b'n' {
                Cell::Null
            } else {
                let value = args.get::<Cell>(argument)?;
                argument += 1;
                value
            };
            self.inputs.push(value);
        }
        if let Some(sub) = self.batch.subfile(self.file as usize) {
            if self.node >= sub.facts.len() {
                return Err(invalid("invalid parsed node"));
            }
            self.calls.fetch_add(1, Ordering::Relaxed);
            let at = |id| sub.root.adopt(sub.root.root().get_doc().node(id));
            if let Some(outputs) = super::pattern_capture::match_at(
                term,
                self.file,
                self.node,
                &sub.facts,
                at,
                &self.inputs,
            )? {
                self.outputs[..width].clone_from_slice(&outputs);
                self.truth = true;
            }
            self.done = false;
            return Ok(());
        }
        let file = self
            .batch
            .files
            .get(self.file as usize)
            .ok_or_else(|| invalid("invalid file"))?;
        if self.node >= file.facts.len() {
            return Err(invalid("invalid node"));
        }
        if self.inputs.iter().any(|cell| matches!(cell, Cell::Span(_))) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if file.dialect == term.dialect
                && let Some(outputs) = super::pattern_capture::match_at(
                    term,
                    self.file,
                    self.node,
                    &file.facts,
                    |id| node_at(&file.root, id),
                    &self.inputs,
                )?
            {
                self.outputs[..width].clone_from_slice(&outputs);
                self.truth = true;
            }
            self.done = false;
            return Ok(());
        }
        for (index, value) in self.inputs.iter().enumerate() {
            match (value, term.vars[index].1) {
                (Cell::Null, _) => {}
                (Cell::Integer(value), SlotKind::Node) => {
                    let node =
                        u32::try_from(*value).map_err(|_| invalid("invalid input capture node"))?;
                    if node >= file.facts.len() {
                        return Err(invalid("input capture node is outside the file"));
                    }
                }
                (Cell::List(bytes), SlotKind::List) => {
                    decode_list(bytes, self.file, file.facts.len())?;
                }
                _ => return Err(invalid("input capture type differs from terminal kind")),
            }
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        if file.dialect == term.dialect
            && term
                .kinds
                .as_ref()
                .is_none_or(|kinds| kinds.contains(&file.facts.kind(self.node)))
        {
            let outputs = if has_lists {
                list_match(
                    &self.batch,
                    term,
                    self.term,
                    self.file,
                    self.node,
                    &self.inputs,
                    &self.lists,
                )?
            } else {
                let inputs: Vec<Option<i64>> = self
                    .inputs
                    .iter()
                    .map(|v| match v {
                        Cell::Integer(v) => Some(*v),
                        _ => None,
                    })
                    .collect();
                typed_match(&self.batch, term, self.file, self.node, &inputs).map(|values| {
                    values
                        .iter()
                        .map(|v| v.map_or(Cell::Null, Cell::Integer))
                        .collect()
                })
            };
            if let Some(outputs) = outputs {
                if outputs.len() != width {
                    return Err(invalid("pattern returned an invalid output width"));
                }
                for (output, (_, kind)) in outputs.iter().zip(&term.vars) {
                    match (output, kind) {
                        (Cell::Null, _) => {}
                        (Cell::Integer(value), SlotKind::Node)
                            if u32::try_from(*value).is_ok()
                                && *value < i64::from(file.facts.len()) => {}
                        (Cell::List(bytes), SlotKind::List) => {
                            decode_list(bytes, self.file, file.facts.len())?;
                        }
                        _ => return Err(invalid("pattern returned an invalid capture")),
                    }
                }
                self.outputs[..width].clone_from_slice(&outputs);
                self.truth = true;
            }
        }
        self.done = false;
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.done = true;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.done
    }
    fn column(&self, ctx: &mut Context, column: c_int) -> Result<()> {
        match column {
            0 => ctx.set_result(&i64::from(self.truth)),
            1..=128 => ctx.set_result(&self.outputs[(column - 1) as usize]),
            TERM => ctx.set_result(&self.term),
            FILE => ctx.set_result(&self.file),
            NODE => ctx.set_result(&self.node),
            column if (INPUT..INPUT + WIDTH as c_int).contains(&column) => ctx.set_result(
                &self
                    .inputs
                    .get((column - INPUT) as usize)
                    .cloned()
                    .unwrap_or(Cell::Null),
            ),
            _ => Err(invalid("invalid output column")),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(1)
    }
}

/// Atomic native pattern matching only. All bindings exist before ellipsis alignment.
fn list_match(
    batch: &Batch,
    term: &super::super::terminals::PatternTerm,
    terminal: u32,
    file: u32,
    node: u32,
    inputs: &[Cell],
    cache: &Mutex<ListCache>,
) -> Result<Option<Vec<Cell>>> {
    let key = (terminal, file, node, inputs.to_vec());
    if let Some(result) = cache
        .lock()
        .map_err(|_| invalid("poisoned pattern cache"))?
        .values
        .get(&key)
    {
        return Ok(result.clone());
    }
    let entry = &batch.files[file as usize];
    let mut env = MetaVarEnv::new();
    let mut compatible = true;
    for ((name, kind), input) in term.vars.iter().zip(inputs) {
        match (kind, input) {
            (_, Cell::Null) => {}
            (SlotKind::Node, Cell::Integer(id)) => {
                compatible &= env.insert(name, node_at(&entry.root, *id as u32)).is_some();
            }
            (SlotKind::List, Cell::List(bytes)) => {
                let nodes = decode_list(bytes, file, entry.facts.len())?
                    .into_iter()
                    .map(|id| node_at(&entry.root, id))
                    .collect();
                compatible &= env.insert_multi(name, nodes).is_some();
            }
            _ => return Err(invalid("invalid seeded capture type")),
        }
    }
    let mut env = Cow::Owned(env);
    let result = if compatible
        && term
            .pattern
            .match_node_with_env(node_at(&entry.root, node), &mut env)
            .is_some()
    {
        let mut outputs = Vec::with_capacity(term.vars.len());
        for (name, kind) in &term.vars {
            outputs.push(match kind {
                SlotKind::Node => env
                    .get_match(name)
                    .map_or(Cell::Null, |n| Cell::Integer(n.node_id() as i64)),
                SlotKind::List => {
                    if let Some(nodes) = env.get_labels(name) {
                        let ids: Vec<u32> = nodes.iter().map(|n| n.node_id() as u32).collect();
                        Cell::List(encode_list(file, &ids)?)
                    } else {
                        Cell::Null
                    }
                }
                _ => return Err(invalid("invalid output capture type")),
            });
        }
        Some(outputs)
    } else {
        None
    };
    // Account for both owned tuples, vector backing storage, hash entry, and blob bytes.
    let cells_bytes = |cells: &[Cell]| {
        std::mem::size_of_val(cells)
            + cells
                .iter()
                .map(|c| match c {
                    Cell::List(b) => b.len(),
                    _ => 0,
                })
                .sum::<usize>()
    };
    let bytes = 256usize
        .saturating_add(cells_bytes(inputs))
        .saturating_add(result.as_ref().map_or(0, |cells| cells_bytes(cells)));
    let mut shared = cache
        .lock()
        .map_err(|_| invalid("poisoned pattern cache"))?;
    if shared.values.len() < 65_536 && shared.bytes.saturating_add(bytes) <= 32 * 1024 * 1024 {
        shared.bytes += bytes;
        shared.values.insert(key, result.clone());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        language::GrammarDialect,
        matcher::node_at,
        sql::{exec, profiling, terminals::PatternTerm, tests::differential::tape},
    };
    use ast_grep_core::matcher::{Matcher, Pattern};
    use rusqlite::params;

    fn term(pattern: &str) -> PatternTerm {
        let mut term = PatternTerm::new(
            Pattern::new(pattern, GrammarDialect::Python),
            GrammarDialect::Python,
        );
        term.reads = (0..term.vars.len()).collect();
        term
    }
    fn nodes(batch: &Batch, kind: &str, text: &str) -> Vec<u32> {
        let file = &batch.files[0];
        (0..file.facts.len())
            .filter(|&id| {
                let node = node_at(&file.root, id);
                node.kind() == kind && node.text() == text
            })
            .collect()
    }
    fn query(
        db: &Connection,
        terminal: u32,
        node: u32,
        bound: Option<i64>,
    ) -> Result<(i64, Option<i64>)> {
        db.query_row("SELECT truth,c0 FROM relation_pattern_typed WHERE term=?1 AND file=0 AND node=?2 AND i0 IS ?3",params![terminal,node,bound],|row|Ok((row.get(0)?,row.get(1)?)))
    }

    #[test]
    fn pattern_leaf_rebinds_equal_composites_and_clears_failed_exports() {
        let doc = tape(
            GrammarDialect::Python,
            "f(obj.attr)\nf(obj.attr)\nf(other.attr)\n",
        );
        let (_, frames) = profiling::frame_counts(|| {
            exec::with_terminals(
                vec![(doc, Default::default())],
                vec![Terminal::Pattern(term("f($X)"))],
                |db, batch, _| {
                    register(db, batch, &Arc::new(AtomicUsize::new(0))).unwrap();
                    let calls = nodes(batch, "call", "f(obj.attr)");
                    let values = nodes(batch, "attribute", "obj.attr");
                    let other = nodes(batch, "call", "f(other.attr)")[0];
                    assert_eq!(calls.len(), 2);
                    assert_eq!(values.len(), 2);
                    assert_eq!(
                        query(db, 0, calls[0], None).unwrap(),
                        (1, Some(i64::from(values[0])))
                    );
                    assert_eq!(
                        query(db, 0, calls[1], Some(i64::from(values[0]))).unwrap(),
                        (1, Some(i64::from(values[1])))
                    );
                    assert_eq!(
                        query(db, 0, other, Some(i64::from(values[0]))).unwrap(),
                        (0, None)
                    );
                    let result: (i64,Option<i64>) = db.query_row("SELECT truth,c0 FROM relation_pattern_typed WHERE term=0 AND file=0 AND node=?1 AND i0 IS NULL",[calls[0]],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
                    assert_eq!(result, (1, Some(i64::from(values[0]))));
                    let mut stmt = db.prepare("SELECT truth,c0 FROM relation_pattern_typed WHERE term=0 AND file=0 AND node=?1 AND i0 IS ?2").unwrap();
                    for (node, expected) in [
                        (calls[0], Some(i64::from(values[0]))),
                        (other, None),
                        (calls[1], Some(i64::from(values[1]))),
                    ] {
                        let result: (i64, Option<i64>) = stmt
                            .query_row(params![node, values[0]], |r| Ok((r.get(0)?, r.get(1)?)))
                            .unwrap();
                        assert_eq!(result, (i64::from(expected.is_some()), expected));
                    }
                },
            )
        });
        assert_eq!(frames[4], 0);
    }

    #[test]
    fn pattern_leaf_uses_bound_inputs_during_anonymous_ellipsis_alignment() {
        let doc = tape(GrammarDialect::Python, "f(a, b, c)\nwrong\n");
        exec::with_terminals(
            vec![(doc, Default::default())],
            vec![Terminal::Pattern(term("f($$$, $X, $$$)"))],
            |db, batch, _| {
                register(db, batch, &Arc::new(AtomicUsize::new(0))).unwrap();
                let call = nodes(batch, "call", "f(a, b, c)")[0];
                let Terminal::Pattern(pattern) = &batch.program.terminals[0] else {
                    panic!("pattern terminal required")
                };
                let tree = &batch.files[0].root;
                // The native matcher owns anonymous-ellipsis greediness and separators.
                // Prebinding does not make every argument a valid native alignment.
                let native = |bound: Option<u32>| {
                    let mut env = ast_grep_core::meta_var::MetaVarEnv::new();
                    if let Some(bound) = bound {
                        env.insert("X", node_at(tree, bound)).unwrap();
                    }
                    let mut env = std::borrow::Cow::Owned(env);
                    if pattern
                        .pattern
                        .match_node_with_env(node_at(tree, call), &mut env)
                        .is_some()
                    {
                        (1, env.get_match("X").map(|node| node.node_id() as i64))
                    } else {
                        (0, None)
                    }
                };
                let fresh = native(None);
                assert_eq!(query(db, 0, call, None).unwrap(), fresh);
                let mut successful_alignment_changed = false;
                for text in ["a", "b", "c", "wrong"] {
                    let bound = nodes(batch, "identifier", text)[0];
                    let expected = native(Some(bound));
                    assert_eq!(
                        query(db, 0, call, Some(i64::from(bound))).unwrap(),
                        expected,
                        "prebound X={text}"
                    );
                    successful_alignment_changed |= expected.0 == 1 && expected != fresh;
                    if text == "wrong" {
                        assert_eq!(expected, (0, None));
                    }
                }
                assert!(
                    successful_alignment_changed,
                    "fixture must distinguish bound matching from a fresh match followed by equality"
                );
            },
        );
    }

    #[test]
    fn pattern_leaf_rejects_invalid_keys_widths_types_and_lists() {
        let doc = tape(GrammarDialect::Python, "f(a)\n");
        let mut unread = term("f($X)");
        unread.reads.clear();
        exec::with_terminals(
            vec![(doc, Default::default())],
            vec![
                Terminal::Pattern(term("f($X)")),
                Terminal::Pattern(term("f($$$X)")),
                Terminal::Pattern(unread),
                Terminal::Regex(regex::Regex::new("x").unwrap()),
            ],
            |db, batch, _| {
                register(db, batch, &Arc::new(AtomicUsize::new(0))).unwrap();
                let call = nodes(batch, "call", "f(a)")[0];
                for (terminal, node, bound) in [
                    (9, call, None),
                    (3, call, None),
                    (0, u32::MAX, None),
                    (0, call, Some(-1)),
                    (0, call, Some(i64::MAX)),
                    (2, call, None),
                ] {
                    assert!(
                        query(db, terminal, node, bound).is_err(),
                        "terminal={terminal},node={node},bound={bound:?}"
                    );
                }
                for conditions in [
                    "term=0 AND file=0",
                    "term=0 AND file=0 AND node=0",
                    "term=0 AND file=0 AND node=0 AND i1 IS NULL",
                    "term=0 AND file=0 AND node=0 AND i0 IS NULL AND i1 IS NULL",
                    "term=0 AND file=9 AND node=0 AND i0 IS NULL",
                    "term=0 AND file=0 AND node=0 AND i0 IS 'bad'",
                    "term=0 AND file=0 AND node=0 AND i0=0",
                ] {
                    let sql =
                        format!("SELECT truth FROM relation_pattern_typed WHERE {conditions}");
                    assert!(
                        db.query_row::<i64, _, _>(&sql, [], |r| r.get(0)).is_err(),
                        "{conditions}"
                    );
                }
            },
        );
    }

    #[test]
    fn pattern_leaf_constant_and_wrong_dialect_each_return_one_truth_row() {
        let docs = vec![
            (tape(GrammarDialect::Python, "f(a)\n"), Default::default()),
            (
                tape(GrammarDialect::TypeScript, "f(a);"),
                Default::default(),
            ),
        ];
        exec::with_terminals(
            docs,
            vec![Terminal::Pattern(term("f(a)"))],
            |db, batch, _| {
                register(db, batch, &Arc::new(AtomicUsize::new(0))).unwrap();
                let call = nodes(batch, "call", "f(a)")[0];
                let hit: (i64,Option<i64>)=db.query_row("SELECT truth,c0 FROM relation_pattern_typed WHERE term=0 AND file=0 AND node=?1",[call],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
                assert_eq!(hit, (1, None));
                let misses: Vec<(i64,Option<i64>)>=db.prepare("SELECT truth,c0 FROM relation_pattern_typed WHERE term=0 AND file=1 AND node=0").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<Result<_>>().unwrap();
                assert_eq!(misses, vec![(0, None)]);
            },
        );
    }
    #[test]
    fn pattern_leaf_lists_preserve_native_sequences_and_seed_alignment() {
        let doc = tape(GrammarDialect::Python, "f(a, b, c)\nf()\nwrong\n");
        exec::with_terminals(
            vec![(doc, Default::default())],
            vec![
                Terminal::Pattern(term("f($$$A, $X, $$$B)")),
                Terminal::Pattern(term("f($$$X)")),
            ],
            |db, batch, _| {
                register(db, batch, &Arc::new(AtomicUsize::new(0))).unwrap();
                let call = nodes(batch, "call", "f(a, b, c)")[0];
                let empty = nodes(batch, "call", "f()")[0];
                let tree = &batch.files[0].root;
                let Terminal::Pattern(pattern) = &batch.program.terminals[0] else {
                    panic!()
                };
                let sql = "SELECT truth,c0,c1,c2 FROM relation_pattern_typed WHERE term=0 AND file=0 AND node=?1 AND i0 IS ?2 AND i1 IS ?3 AND i2 IS ?4";
                let mut statement = db.prepare(sql).unwrap();
                let mut changed = false;
                let mut fresh = None;
                for bound in [None, Some("a"), Some("b"), Some("c"), Some("wrong")] {
                    let mut env = MetaVarEnv::new();
                    let input = bound.map(|text| nodes(batch, "identifier", text)[0]);
                    if let Some(id) = input {
                        env.insert("X", node_at(tree, id)).unwrap();
                    }
                    let mut env = Cow::Owned(env);
                    let matched = pattern
                        .pattern
                        .match_node_with_env(node_at(tree, call), &mut env)
                        .is_some();
                    let expected: Vec<Cell> = if matched {
                        pattern
                            .vars
                            .iter()
                            .map(|(name, kind)| match kind {
                                SlotKind::Node => env
                                    .get_match(name)
                                    .map_or(Cell::Null, |n| Cell::Integer(n.node_id() as i64)),
                                SlotKind::List => Cell::List(
                                    encode_list(
                                        0,
                                        &env.get_multiple_matches(name)
                                            .iter()
                                            .map(|n| n.node_id() as u32)
                                            .collect::<Vec<_>>(),
                                    )
                                    .unwrap(),
                                ),
                                _ => panic!(),
                            })
                            .collect()
                    } else {
                        vec![Cell::Null; 3]
                    };
                    let result: (i64, Vec<Cell>) = statement
                        .query_row(
                            params![call, Cell::Null, Cell::Null, input.map(i64::from)],
                            |r| Ok((r.get(0)?, vec![r.get(1)?, r.get(2)?, r.get(3)?])),
                        )
                        .unwrap();
                    assert_eq!(result, (i64::from(matched), expected.clone()));
                    if input.is_none() {
                        fresh = Some(expected.clone());
                    }
                    changed |= matched && fresh.as_ref().is_some_and(|f| f != &expected);
                }
                assert!(changed, "bound node must change native list alignment");
                // Seed an actual list before matching, including native separators.
                let a = nodes(batch, "identifier", "a")[0];
                for ids in [vec![], vec![a]] {
                    let mut env = MetaVarEnv::new();
                    env.insert_multi("A", ids.iter().map(|&id| node_at(tree, id)).collect())
                        .unwrap();
                    let mut env = Cow::Owned(env);
                    let matched = pattern
                        .pattern
                        .match_node_with_env(node_at(tree, call), &mut env)
                        .is_some();
                    let output: (i64, Cell, Cell, Cell) = statement
                        .query_row(
                            params![
                                call,
                                Cell::List(encode_list(0, &ids).unwrap()),
                                Cell::Null,
                                Cell::Null
                            ],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                        )
                        .unwrap();
                    assert_eq!(output.0, i64::from(matched));
                    if matched {
                        for (slot, name) in [(output.1, "A"), (output.2, "B")] {
                            let ids: Vec<u32> = env
                                .get_multiple_matches(name)
                                .iter()
                                .map(|n| n.node_id() as u32)
                                .collect();
                            assert_eq!(slot, Cell::List(encode_list(0, &ids).unwrap()));
                        }
                        assert_eq!(
                            output.3,
                            env.get_match("X")
                                .map_or(Cell::Null, |n| Cell::Integer(n.node_id() as i64))
                        );
                    } else {
                        assert_eq!(
                            (output.1, output.2, output.3),
                            (Cell::Null, Cell::Null, Cell::Null)
                        );
                    }
                }
                let list_sql = "SELECT truth,c0 FROM relation_pattern_typed WHERE term=1 AND file=0 AND node=?1 AND i0 IS ?2";
                let mut statement = db.prepare(list_sql).unwrap();
                let row = |statement: &mut rusqlite::Statement<'_>, node, input: Cell| {
                    statement.query_row(params![node, input], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, Cell>(1)?))
                    })
                };
                let present_empty = Cell::List(encode_list(0, &[]).unwrap());
                assert_eq!(
                    row(&mut statement, empty, Cell::Null).unwrap(),
                    (1, present_empty.clone())
                );
                assert_eq!(
                    row(&mut statement, empty, present_empty.clone()).unwrap(),
                    (1, present_empty)
                );
                let full = row(&mut statement, call, Cell::Null).unwrap();
                let Cell::List(bytes) = &full.1 else { panic!() };
                let ids = decode_list(bytes, 0, batch.files[0].facts.len()).unwrap();
                assert!(
                    ids.iter().any(|&id| node_at(tree, id).text() == ","),
                    "native lists retain punctuation"
                );
                assert_eq!(row(&mut statement, call, full.1.clone()).unwrap(), full);
                assert_eq!(row(&mut statement, empty, full.1).unwrap(), (0, Cell::Null));
                for invalid in [
                    Cell::Integer(0),
                    Cell::List(encode_list(1, &[]).unwrap()),
                    Cell::List(encode_list(0, &[u32::MAX]).unwrap()),
                ] {
                    assert!(row(&mut statement, call, invalid).is_err());
                }
                assert!(row(&mut statement, call, Cell::List(b"bad".to_vec())).is_err());
                assert_eq!(row(&mut statement, empty, Cell::Null).unwrap().0, 1);
            },
        );
    }

    #[test]
    fn pattern_leaf_accepts_128_slots_and_rejects_129() {
        let source = format!(
            "f({})\n",
            (0..128)
                .map(|i| format!("a{i}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let pattern = |width| {
            format!(
                "f({})",
                (0..width)
                    .map(|i| format!("$X{i:03}"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let doc = tape(GrammarDialect::Python, &source);
        exec::with_terminals(
            vec![(doc, Default::default())],
            vec![
                Terminal::Pattern(term(&pattern(128))),
                Terminal::Pattern(term(&pattern(129))),
            ],
            |db, batch, _| {
                register(db, batch, &Arc::new(AtomicUsize::new(0))).unwrap();
                let call = nodes(batch, "call", source.trim())[0];
                let inputs = (0..128)
                    .map(|i| format!("i{i} IS NULL"))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                let sql = format!(
                    "SELECT truth,c127 FROM relation_pattern_typed WHERE term=?1 AND file=0 AND node=?2 AND {inputs}"
                );
                let row: (i64, Option<i64>) = db
                    .query_row(&sql, params![0, call], |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap();
                assert_eq!(
                    row,
                    (1, Some(i64::from(nodes(batch, "identifier", "a127")[0])))
                );
                assert!(
                    db.query_row::<i64, _, _>(&sql, params![1, call], |r| r.get(0))
                        .is_err()
                );
            },
        );
    }
}
