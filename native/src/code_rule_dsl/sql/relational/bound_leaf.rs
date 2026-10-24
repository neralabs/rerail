//! Atomic capture-returning leaves. All inputs and outputs are typed SQL columns.
use super::super::{
    exec::Batch,
    plan::SlotKind,
    terminals::{Terminal, Verdict, Verdicts, values_of, with_scopes},
};
use crate::code_rule_dsl::{
    matcher::{
        AstMatcher, Semantic, node_at, sql_bound_leaf::leaf_env_budget, with_sql_clause_owner,
    },
    tape::TapeDoc,
};
use ast_grep_core::{Node, meta_var::MetaVarEnv};
use rusqlite::{
    Connection, Error, Result,
    types::Value,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor, sqlite3_vtab, sqlite3_vtab_cursor,
    },
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    ffi::{CStr, CString, c_int},
    marker::PhantomData,
    sync::Arc,
};

const WIDTH: usize = 128;
const TERM: c_int = 2 + WIDTH as c_int;
const FILE: c_int = TERM + 1;
const NODE: c_int = FILE + 1;
const ALLOWANCE: c_int = NODE + 1;
const INPUT: c_int = ALLOWANCE + 1;

type Call = (usize, usize, u32, usize, usize);

thread_local! { static CALLS: std::cell::RefCell<Option<Vec<Call>>> = const { std::cell::RefCell::new(None) }; }

pub(super) fn trace<T>(body: impl FnOnce() -> T) -> (T, Vec<Call>) {
    struct Reset(Option<Vec<Call>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            CALLS.set(self.0.take());
        }
    }
    let _reset = Reset(CALLS.replace(Some(vec![])));
    let result = body();
    (
        result,
        CALLS.with_borrow_mut(|calls| std::mem::take(calls.as_mut().unwrap())),
    )
}

pub(super) struct OwnedBoundLeaf {
    pub leaf: AstMatcher,
    pub bindings: Vec<(String, SlotKind)>,
    pub owner: usize,
}

impl OwnedBoundLeaf {
    fn evaluate<'t, D: ast_grep_core::Doc>(
        &self,
        node: Node<'t, D>,
        env: &MetaVarEnv<'t, D>,
        work: usize,
    ) -> (std::result::Result<Vec<Verdict>, String>, usize) {
        // Keep the runtime boundary closed even if a future lowerer is mistaken.
        let arguments = matches!(self.leaf, AstMatcher::Semantic(Semantic::Arguments(_)));
        let predicate = matches!(self.leaf, AstMatcher::Semantic(Semantic::Layout(_)))
            || matches!(&self.leaf, AstMatcher::Compare(test) if test.transform.is_none());
        if !arguments && !predicate {
            return (
                Err("bound leaf: matcher has no atomic capture contract".into()),
                work,
            );
        }
        if self.bindings.len() > WIDTH
            || self
                .bindings
                .iter()
                .any(|(_, kind)| *kind != SlotKind::Node)
        {
            return (
                Err("bound leaf: output contract requires at most 128 node columns".into()),
                work,
            );
        }
        with_sql_clause_owner(self.owner, || {
            let (result, left) = leaf_env_budget(&self.leaf, node, env, work);
            let rows = match result {
                Ok((Some(true), Some(bound))) => {
                    let values = values_of(&self.bindings, &bound);
                    if values.len() != self.bindings.len()
                        || values.iter().any(|value| {
                            !(matches!(value, Value::Integer(_))
                                || predicate && matches!(value, Value::Null))
                        })
                    {
                        Err(
                            "bound leaf: successful atomic binding has an invalid node output"
                                .into(),
                        )
                    } else {
                        Ok(vec![Verdict::holds(values)])
                    }
                }
                Ok((Some(true), None)) => {
                    Err("bound leaf: true verdict omitted its environment".into())
                }
                Ok((None, _)) => Ok(vec![Verdict::unknown()]),
                Ok(_) => Ok(vec![]),
                Err(limit) => Err(format!("{limit:?}")),
            };
            (rows, left)
        })
    }
}
impl Verdicts for OwnedBoundLeaf {
    fn budget_rows<'t>(
        &self,
        node: Node<'t, TapeDoc>,
        env: &MetaVarEnv<'t, TapeDoc>,
        work: usize,
    ) -> (std::result::Result<Vec<Verdict>, String>, usize) {
        self.evaluate(node, env, work)
    }
    fn tree_budget_rows<'t>(
        &self,
        node: Node<'t, crate::code_rule_dsl::literal::SubDoc>,
        env: &MetaVarEnv<'t, crate::code_rule_dsl::literal::SubDoc>,
        work: usize,
    ) -> (std::result::Result<Vec<Verdict>, String>, usize) {
        self.evaluate(node, env, work)
    }
    fn supports_tree_rows(&self) -> bool {
        true
    }
    fn rows<'t>(
        &self,
        _: Node<'t, TapeDoc>,
        _: &MetaVarEnv<'t, TapeDoc>,
    ) -> std::result::Result<Vec<Verdict>, String> {
        Err("bound leaf: finite allowance and typed inputs are required".into())
    }
}

fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("bound leaf: {message}"))
}

pub(super) fn register(db: &Connection, batch: &Arc<Batch>) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(c"relation_bound_budget", &MODULE, Some(Arc::clone(batch)))
}

#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    batch: Arc<Batch>,
}

// SAFETY: SQLite's base is first and cursors retain the batch.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = Arc<Batch>;
    type Cursor = Cursor<'v>;
    fn connect(
        db: &mut VTabConnection,
        batch: Option<&Arc<Batch>>,
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
        let schema = CString::new(format!("CREATE TABLE x(truth INTEGER,work INTEGER,{outputs},term INTEGER HIDDEN,file INTEGER HIDDEN,node INTEGER HIDDEN,allowance INTEGER HIDDEN,{inputs})")).map_err(|_| invalid("schema"))?;
        Ok((
            Cow::Owned(schema),
            Self {
                base: sqlite3_vtab::default(),
                batch: batch.ok_or_else(|| invalid("missing batch"))?.clone(),
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut keys = [None; 4];
        let mut inputs = BTreeMap::new();
        for (index, c) in info.constraints().enumerate() {
            if !c.is_usable() {
                continue;
            }
            let column = c.column();
            let operator = c.operator();
            if (TERM..=ALLOWANCE).contains(&column)
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
        for (arg, key) in keys.into_iter().enumerate() {
            let mut usage = info.constraint_usage(key.unwrap());
            usage.set_argv_index((arg + 1) as c_int);
            usage.set_omit(true);
        }
        let mut plan = String::with_capacity(inputs.len());
        let mut argument = 5;
        for &(index, null) in inputs.values() {
            let mut usage = info.constraint_usage(index);
            if null {
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
            truth: 0,
            work: 0,
            outputs: vec![super::capture_cell::Cell::Null; WIDTH],
            keys: [0; 4],
            inputs: vec![],
            done: true,
            marker: PhantomData,
        })
    }
}

#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    batch: Arc<Batch>,
    truth: i64,
    work: i64,
    outputs: Vec<super::capture_cell::Cell>,
    /// Actual hidden-column values, including the input allowance before work is spent.
    keys: [i64; 4],
    inputs: Vec<super::capture_cell::Cell>,
    done: bool,
    marker: PhantomData<&'v Table>,
}

// SAFETY: SQLite's base is first. Each cursor owns its outcome and capture columns.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, width: c_int, plan: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.done = true;
        self.outputs.fill(super::capture_cell::Cell::Null);
        self.inputs.clear();
        let width = usize::try_from(width).map_err(|_| invalid("negative width"))?;
        let plan = plan.unwrap_or("");
        if width > WIDTH
            || plan.len() != width
            || !plan.bytes().all(|b| b == b'n' || b == b'v')
            || args.len() != 4 + plan.bytes().filter(|&b| b == b'v').count()
        {
            return Err(invalid("input plan"));
        }
        let Some(allowance) = args.get::<Option<i64>>(3)? else {
            return Ok(());
        };
        let work = usize::try_from(allowance).map_err(|_| invalid("negative allowance"))?;
        let term_id = args.get::<u32>(0)? as usize;
        let file_id = args.get::<u32>(1)? as usize;
        let node_id = args.get::<u32>(2)?;
        self.keys = [
            term_id as i64,
            file_id as i64,
            i64::from(node_id),
            allowance,
        ];
        let Some(Terminal::Fact(term)) = self.batch.program.terminals.get(term_id) else {
            return Err(invalid("terminal"));
        };
        if term.reads.len() != width || term.reads.iter().any(|(_, kind)| *kind != SlotKind::Node) {
            return Err(invalid(
                "input columns must be nodes and match terminal width",
            ));
        }
        let mut argument = 4;
        for code in plan.bytes() {
            let input = if code == b'n' {
                super::capture_cell::Cell::Null
            } else {
                let value = args.get::<super::capture_cell::Cell>(argument)?;
                argument += 1;
                value
            };
            self.inputs.push(input);
        }
        super::leaves::VALUE_REQUESTS.set(super::leaves::VALUE_REQUESTS.get() + 1);
        let probe_start = crate::code_rule_dsl::matcher::value_probe::position();
        let (node_count, trace_host, text, (rows, left)) =
            if let Some(file) = self.batch.files.get(file_id) {
                if node_id >= file.facts.len() {
                    return Err(invalid("node"));
                }
                let env = super::capture_env::environment(
                    file_id as u32,
                    &file.facts,
                    |id| node_at(&file.root, id),
                    &term.reads,
                    &self.inputs,
                )?;
                let at = node_at(&file.root, node_id);
                let text = at.text().into_owned();
                (
                    file.facts.len(),
                    file_id,
                    text,
                    with_scopes(file_id, file, || term.verdicts.budget_rows(at, &env, work)),
                )
            } else if let Some(sub) = self.batch.subfile(file_id) {
                if node_id >= sub.facts.len() {
                    return Err(invalid("parsed node"));
                }
                let at = |id| sub.root.adopt(sub.root.root().get_doc().node(id));
                let env = super::capture_env::environment(
                    file_id as u32,
                    &sub.facts,
                    at,
                    &term.reads,
                    &self.inputs,
                )?;
                let text = at(node_id).text().into_owned();
                let host = self
                    .batch
                    .files
                    .get(sub.host)
                    .ok_or_else(|| invalid("parsed host"))?;
                let _tables = sub
                    .table
                    .clone()
                    .map(crate::code_rule_dsl::matcher::SubTables::enter);
                (
                    sub.facts.len(),
                    sub.host,
                    text,
                    with_scopes(sub.host, host, || {
                        term.verdicts.tree_budget_rows(at(node_id), &env, work)
                    }),
                )
            } else {
                return Err(invalid("file"));
            };
        CALLS.with_borrow_mut(|calls| {
            if let Some(calls) = calls {
                calls.push((term_id, file_id, node_id, work, left));
            }
        });
        if left > work {
            return Err(invalid("remaining allowance increased"));
        }
        self.work = i64::try_from(left).map_err(|_| invalid("remaining allowance overflow"))?;
        self.truth = match rows {
            Err(error) => super::failure::encode(&error).ok_or(Error::ModuleError(error))?,
            Ok(rows) => {
                if rows.len() > 1 {
                    return Err(invalid("atomic leaf returned multiple rows"));
                }
                if let Some(row) = rows.into_iter().next() {
                    if row.values.len() > WIDTH {
                        return Err(invalid("output width"));
                    }
                    if row.unknown {
                        if !row.values.is_empty() {
                            return Err(invalid("unknown leaf published captures"));
                        }
                        2
                    } else {
                        if row.values.len() != width {
                            return Err(invalid(
                                "output width differs from typed binding contract",
                            ));
                        }
                        for (index, (output, value)) in
                            self.outputs.iter_mut().zip(row.values).enumerate()
                        {
                            *output = match value {
                                Value::Null => super::capture_cell::Cell::Null,
                                Value::Integer(value)
                                    if u32::try_from(value).is_ok()
                                        && value < i64::from(node_count) =>
                                {
                                    if let Some(super::capture_cell::Cell::Span(bytes)) =
                                        self.inputs.get(index)
                                    {
                                        let mut span = super::span_capture::decode(bytes)?;
                                        span.node = Some(value as u32);
                                        super::capture_cell::Cell::Span(
                                            super::span_capture::encode(&span)?,
                                        )
                                    } else {
                                        super::capture_cell::Cell::Integer(value)
                                    }
                                }
                                _ => return Err(invalid("output is not a node in the file")),
                            };
                        }
                        1
                    }
                } else {
                    0
                }
            }
        };
        super::budget_leaf::record(serde_json::json!({
            "terminal":term_id,"file":trace_host,"parsedFile":(trace_host!=file_id).then_some(file_id),"node":node_id,"input":work,
            "truth":self.truth,"remaining":self.work,
            "text":text,
            "valueCalls":crate::code_rule_dsl::matcher::value_probe::since(probe_start)
        }));
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
            0 => ctx.set_result(&self.truth),
            1 => ctx.set_result(&self.work),
            2..=129 => ctx.set_result(&self.outputs[(column - 2) as usize]),
            TERM..=ALLOWANCE => ctx.set_result(&self.keys[(column - TERM) as usize]),
            column if (INPUT..INPUT + WIDTH as c_int).contains(&column) => {
                ctx.set_result(&self.inputs.get((column - INPUT) as usize).cloned())
            }
            _ => Err(invalid("invalid column")),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(0)
    }
}

#[test]
fn wide_hidden_columns_preserve_inputs_and_cursor_reuse() {
    use crate::code_rule_dsl::{
        CodeRuleDslRuntime,
        sql::{exec, tests::differential::tape},
        test_support::{CAPABILITIES, grammar_matchers},
    };
    use serde_json::json;
    let signature: Vec<_> = (0..32).map(|i| format!("a{i}")).collect();
    let bindings: serde_json::Map<String, serde_json::Value> = signature
        .iter()
        .enumerate()
        .map(|(i, name)| (format!("X{i:02}"), json!(name)))
        .collect();
    let p = CodeRuleDslRuntime::compile(
        json!({
            "format":"rerail.policy/v1","id":"hidden-bound-inputs","revision":1,"mode":"steer",
            "clauses":[{"id":"c","forbid":{"ast":{"language":"python","rule":{
                "arguments":{"signature":signature,"bind":bindings}
            }}},"violation":{"code":"C","message":"match","anchor":"witness.span"}}]
        })
        .to_string(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap();
    let grammars = grammar_matchers(&p);
    let compiled = super::outcomes::compile_job(&[&grammars[0]], 100).unwrap();
    let source = format!(
        "f({})\n",
        (0..32).map(|i| i.to_string()).collect::<Vec<_>>().join(",")
    );
    let doc = tape(grammars[0].language, &source);
    let tree = ast_grep_core::AstGrep::doc(doc.clone());
    let call = tree
        .root()
        .dfs()
        .find(|node| node.kind() == "call")
        .unwrap()
        .node_id() as u32;
    let numbers: Vec<_> = tree
        .root()
        .dfs()
        .filter(|node| node.kind() == "integer")
        .map(|node| node.node_id() as u32)
        .collect();
    exec::with_terminals(
        vec![(doc, Default::default())],
        compiled.terminals,
        |db, batch, _| {
            register(db, batch).unwrap();
            let inputs = (1..32)
                .map(|i| format!(" AND i{i} IS NULL"))
                .collect::<String>();
            let query = format!(
                "SELECT term,file,node,allowance,i0,i31,i100,c31,truth,work FROM relation_bound_budget WHERE term=0 AND file=0 AND node=?1 AND allowance=?2 AND i0 IS ?3{inputs}"
            );
            let mut statement = db.prepare(&query).unwrap();
            let read = |row: &rusqlite::Row<'_>| -> Result<Vec<Option<i64>>> {
                (0..10).map(|index| row.get(index)).collect()
            };
            let fresh = statement
                .query_row(rusqlite::params![call, 100, Option::<i64>::None], read)
                .unwrap();
            assert_eq!(
                &fresh[..7],
                &[
                    Some(0),
                    Some(0),
                    Some(i64::from(call)),
                    Some(100),
                    None,
                    None,
                    None
                ]
            );
            assert_eq!(
                &fresh[7..],
                &[Some(i64::from(numbers[31])), Some(1), Some(100)]
            );
            let unified = statement
                .query_row(rusqlite::params![call, 73, numbers[0]], read)
                .unwrap();
            assert_eq!(unified[3], Some(73));
            assert_eq!(unified[4], Some(i64::from(numbers[0])));
            assert_eq!(unified[7], Some(i64::from(numbers[31])));
            assert_eq!(unified[8], Some(1));
            let conflict = statement
                .query_row(rusqlite::params![call, 51, numbers[31]], read)
                .unwrap();
            assert_eq!(conflict[3], Some(51));
            assert_eq!(conflict[4], Some(i64::from(numbers[31])));
            assert_eq!(
                conflict[7], None,
                "a reused cursor must clear prior captures"
            );
            assert_eq!(conflict[8], Some(0));
            assert!(
                statement
                    .query_row(rusqlite::params![call, 100, "invalid node"], read)
                    .is_err()
            );
            let reused = statement
                .query_row(rusqlite::params![call, 100, Option::<i64>::None], read)
                .unwrap();
            assert_eq!(
                reused, fresh,
                "valid input must work after an invalid filter"
            );
        },
    );
}
