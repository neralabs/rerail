//! Atomic binding resolver transport. Column three is an ordered occurrence JSON array.
use crate::code_rule_dsl::matcher::node_at;
use crate::code_rule_dsl::sql::relational::capture_cell::Cell;
use crate::code_rule_dsl::sql::{
    exec::Batch,
    plan::SlotKind,
    terminals::{Terminal, with_scopes},
};
use ast_grep_core::meta_var::MetaVarEnv;
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

const WIDTH: usize = 5;
const TERM: c_int = 2 + WIDTH as c_int;
const FILE: c_int = TERM + 1;
const NODE: c_int = FILE + 1;
const ALLOWANCE: c_int = NODE + 1;
const INPUT: c_int = ALLOWANCE + 1;

fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("binding facts: {message}"))
}

pub(in crate::code_rule_dsl::sql::relational) fn register(
    db: &Connection,
    batch: &Arc<Batch>,
) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(c"relation_binding_budget", &MODULE, Some(Arc::clone(batch)))
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
            outputs: vec![Value::Null; WIDTH],
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
    outputs: Vec<Value>,
    /// Actual hidden-column values, including the input allowance before work is spent.
    keys: [i64; 4],
    inputs: Vec<Cell>,
    done: bool,
    marker: PhantomData<&'v Table>,
}

// SAFETY: SQLite's base is first. Each cursor owns its outcome and capture columns.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, width: c_int, plan: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.done = true;
        self.outputs.fill(Value::Null);
        self.inputs.clear();
        let width = usize::try_from(width).map_err(|_| invalid("negative width"))?;
        let plan = plan.unwrap_or("");
        if !(4..=WIDTH).contains(&width)
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
        let file = self
            .batch
            .files
            .get(file_id)
            .ok_or_else(|| invalid("file"))?;
        if node_id >= file.facts.len() {
            return Err(invalid("node"));
        }
        let mut env = MetaVarEnv::new();
        let mut argument = 4;
        for ((name, _), code) in term.reads.iter().zip(plan.bytes()) {
            let input = if code == b'n' {
                Cell::Null
            } else {
                let value = args.get::<Cell>(argument)?;
                argument += 1;
                value
            };
            let node = input.single_node()?;
            self.inputs.push(input);
            if let Some(input) = node {
                if input >= file.facts.len() {
                    return Err(invalid("input node outside file"));
                }
                env.insert(name, node_at(&file.root, input))
                    .ok_or_else(|| invalid("conflicting input names"))?;
            }
        }
        crate::code_rule_dsl::sql::relational::leaves::VALUE_REQUESTS
            .set(crate::code_rule_dsl::sql::relational::leaves::VALUE_REQUESTS.get() + 1);
        let probe_start = crate::code_rule_dsl::matcher::value_probe::position();
        let (rows, left) = with_scopes(file_id, file, || {
            term.verdicts
                .budget_rows(node_at(&file.root, node_id), &env, work)
        });
        if left > work {
            return Err(invalid("remaining allowance increased"));
        }
        self.work = i64::try_from(left).map_err(|_| invalid("remaining allowance overflow"))?;
        self.truth = match rows {
            Err(error) => crate::code_rule_dsl::sql::relational::failure::encode(&error)
                .ok_or(Error::ModuleError(error))?,
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
                            // The optional region column is a read-only input, including
                            // a parsed span's node shadow. Resolution never rewrites it.
                            if index == 4 {
                                *output = match &self.inputs[index] {
                                    Cell::Null => Value::Null,
                                    Cell::Integer(value) => Value::Integer(*value),
                                    Cell::Span(bytes) => Value::Blob(bytes.clone()),
                                    Cell::List(_) => {
                                        return Err(invalid("region must be a single capture"));
                                    }
                                };
                                continue;
                            }
                            *output = match value {
                                Value::Null => Value::Null,
                                Value::Integer(value)
                                    if u32::try_from(value).is_ok()
                                        && value < i64::from(file.facts.len()) =>
                                {
                                    Value::Integer(value)
                                }
                                Value::Text(value) if index == 3 => Value::Text(value),
                                _ => {
                                    return Err(invalid(
                                        "output is not a node or ordered occurrences",
                                    ));
                                }
                            };
                        }
                        1
                    }
                } else {
                    0
                }
            }
        };
        crate::code_rule_dsl::sql::relational::budget_leaf::record(serde_json::json!({
            "terminal":term_id,"file":file_id,"node":node_id,"input":work,
            "truth":self.truth,"remaining":self.work,
            "text":node_at(&file.root,node_id).text(),
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
            2..=6 => ctx.set_result(&self.outputs[(column - 2) as usize]),
            TERM..=ALLOWANCE => ctx.set_result(&self.keys[(column - TERM) as usize]),
            column if (INPUT..INPUT + WIDTH as c_int).contains(&column) => ctx.set_result(
                self.inputs
                    .get((column - INPUT) as usize)
                    .unwrap_or(&Cell::Null),
            ),
            _ => Err(invalid("invalid column")),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(0)
    }
}
