//! Generic field-child operator. SQL selects parents and owns all consumer predicates.
use super::*;
use crate::reader::{Column, Shared};
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::{
    ffi,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor,
    },
};
use std::{
    borrow::Cow,
    ffi::{CStr, c_int},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Batch {
    reader: Shared,
    // id, scope, fid, parent, subtree end. IDs preserve the complete correlated request.
    columns: [Vec<i64>; 5],
    field: i64,
    kind: i64,
    estimate: i64,
    filters: AtomicUsize,
    visits: AtomicUsize,
    output: AtomicUsize,
    callbacks: AtomicUsize,
}
#[repr(C)]
struct Table {
    base: ffi::sqlite3_vtab,
    batch: Arc<Batch>,
}
#[repr(C)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    batch: Arc<Batch>,
    ids: Vec<usize>,
    task: usize,
    next: usize,
    current: Option<[i64; 5]>,
    ordinal: i64,
}
fn invalid(s: &str) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(s.into())
}
// SAFETY: the SQLite base fields are first. Arc ownership bounds every reader to the connection.
unsafe impl<'a> VTab<'a> for Table {
    type Aux = Arc<Batch>;
    type Cursor = Cursor;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<Batch>>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> rusqlite::Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        Ok((Cow::Borrowed(c"CREATE TABLE x(scope INTEGER,fid INTEGER,parent INTEGER,child INTEGER,request INTEGER HIDDEN)"),Self{base:ffi::sqlite3_vtab::default(),batch:aux.unwrap().clone()}))
    }
    fn best_index(&self, info: &mut IndexInfo) -> rusqlite::Result<bool> {
        let selected = info.constraints().enumerate().find_map(|(i, c)| {
            (c.is_usable()
                && c.column() == 4
                && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ)
                .then_some(i)
        });
        let Some(i) = selected else { return Ok(false) };
        let batch = info.is_in_constraint(i)?;
        if batch {
            info.set_in_constraint(i, true)?;
        }
        info.constraint_usage(i).set_argv_index(1);
        info.constraint_usage(i).set_omit(true);
        info.set_idx_num(i32::from(batch));
        // Input-parent work estimate from persisted SQL statistics, loaded before planning.
        // Output multiplicity depends on the selected field; this is not an output bound.
        let estimate = if batch { self.batch.estimate } else { 1 };
        info.set_estimated_rows(estimate);
        info.set_estimated_cost(estimate.max(1) as f64);
        Ok(true)
    }
    fn open(&'a mut self) -> rusqlite::Result<Cursor> {
        Ok(Cursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            batch: self.batch.clone(),
            ids: Vec::new(),
            task: 0,
            next: 0,
            current: None,
            ordinal: 0,
        })
    }
}
impl Cursor {
    fn advance(&mut self) -> rusqlite::Result<()> {
        self.current = None;
        let mut reader = self.batch.reader.lock();
        while self.task < self.ids.len() {
            let id = self.ids[self.task];
            let fid = self.batch.columns[2][id];
            let parent = self.batch.columns[3][id] as usize;
            let end = self.batch.columns[4][id] as usize;
            if self.next == 0 {
                self.next = parent + 1;
            }
            if self.next >= end {
                self.task += 1;
                self.next = 0;
                continue;
            }
            let child = self.next;
            let rec = reader.rec(fid, child)?;
            let size = reader.value(fid, Column::Size, child, rec[2])? as usize;
            if size == 0 || child + size > end {
                return Err(invalid("invalid child subtree"));
            }
            self.next += size;
            self.batch.visits.fetch_add(1, Ordering::Relaxed);
            let flags = reader.flags(fid, child)?;
            let kind = if flags & 8 != 0 {
                65535
            } else {
                i64::from(rec[0] & 1023)
            };
            if i64::from(rec[0] >> 10) == self.batch.field && kind == self.batch.kind {
                self.current = Some([
                    self.batch.columns[1][id],
                    fid,
                    parent as i64,
                    child as i64,
                    id as i64,
                ]);
                self.batch.output.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        }
        Ok(())
    }
}
// SAFETY: each cursor owns its request IDs and current typed output. xColumn never traverses.
unsafe impl VTabCursor for Cursor {
    fn filter(
        &mut self,
        index: c_int,
        _: Option<&str>,
        args: &Filters<'_>,
    ) -> rusqlite::Result<()> {
        self.batch.filters.fetch_add(1, Ordering::Relaxed);
        self.ids.clear();
        self.task = 0;
        self.next = 0;
        self.ordinal = 0;
        let mut push = |id: i64| -> rusqlite::Result<()> {
            let id = usize::try_from(id).map_err(|_| invalid("negative request"))?;
            if id >= self.batch.columns[0].len() {
                return Err(invalid("unknown request"));
            }
            self.ids.push(id);
            Ok(())
        };
        if index == 1 {
            let mut values = args.in_values(0)?;
            while let Some(value) = values.next()? {
                push(value.as_i64().map_err(|_| invalid("noninteger request"))?)?;
            }
        } else {
            push(args.get(0)?)?;
        }
        self.advance()
    }
    fn next(&mut self) -> rusqlite::Result<()> {
        self.ordinal += 1;
        self.advance()
    }
    fn eof(&self) -> bool {
        self.current.is_none()
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> rusqlite::Result<()> {
        self.batch.callbacks.fetch_add(1, Ordering::Relaxed);
        ctx.set_result(&self.current.unwrap()[col as usize])
    }
    fn rowid(&self) -> rusqlite::Result<i64> {
        Ok(self.ordinal)
    }
}

pub(super) fn run(
    store: &crate::store::Store,
    v: &BTreeMap<String, i64>,
    _chosen: &str,
    variant: &str,
    m: &mut Meter,
) -> rusqlite::Result<(Vec<Vec<i64>>, Value)> {
    let db = &store.db;
    let f = code(v, "kind:function_definition");
    let field = code(v, "field:name");
    let kind = code(v, "kind:identifier");
    let direct = variant.starts_with("direct-");
    if direct {
        // The production bare token is k + base36(language*1024+kind). Compute it
        // from SQL vocabulary metadata once, never by pricing candidate postings.
        m.statement(db,&format!("CREATE TEMP TABLE requests AS WITH RECURSIVE encoding(value,word) AS (SELECT lang*1024+{f},'' FROM file WHERE fid=1 UNION ALL SELECT value/36,substr('0123456789abcdefghijklmnopqrstuvwxyz',value%36+1,1)||word FROM encoding WHERE value>0), token(term) AS (SELECT 'k'||word FROM encoding WHERE value=0) SELECT row_number() OVER ()-1 AS id,d.scope,i.doc AS fid,i.offset AS pre,-1 AS end FROM demand d CROSS JOIN token t CROSS JOIN ftsk_inst i WHERE i.term=t.term AND i.doc=d.fid AND i.offset>=d.lo AND i.offset<d.hi"))?;
    } else {
        m.statement(db,&format!("CREATE TEMP TABLE requests AS SELECT row_number() OVER ()-1 AS id,d.scope,p.fid,p.pre,p.end FROM demand d CROSS JOIN n p INDEXED BY n_kind WHERE p.kind={f} AND p.fid=d.fid AND p.pre>=d.lo AND p.pre<d.hi"))?;
    }
    let stats=m.statement(db,&format!("SELECT coalesce(sum(s.n),0) FROM file_kind_counts s JOIN demand d ON d.fid=s.fid WHERE s.code={f}"))?;
    let rows = if variant == "sql" {
        m.statement(db,&format!("SELECT r.scope,r.fid,r.pre,c.pre FROM requests r CROSS JOIN n c INDEXED BY n_parent WHERE c.fid=r.fid AND c.parent=r.pre AND c.field={field} AND c.kind={kind}"))?
    } else if variant == "scalar" || variant.starts_with("direct-scalar") {
        let projection = if variant.ends_with("consumers") {
            "SELECT consumers.id,scope,fid,pre,child FROM children CROSS JOIN (SELECT 1 id,2 modulus UNION ALL SELECT 2,3) consumers WHERE child%consumers.modulus=0"
        } else {
            "SELECT scope,fid,pre,child FROM children"
        };
        m.statement(db,&format!("WITH found AS MATERIALIZED (SELECT scope,fid,pre,nera_child(fid,pre,{field}) AS child FROM requests), children AS NOT MATERIALIZED (SELECT * FROM found WHERE child IS NOT NULL AND nera_kind(fid,child)={kind}) {projection}"))?
    } else {
        let mut requests =
            m.statement(db, "SELECT id,scope,fid,pre,end FROM requests ORDER BY id")?;
        if requests.is_empty() {
            m.statement(db, "DROP TABLE requests")?;
            return Ok((
                Vec::new(),
                json!({"requests":0,"filters":0,"visitedDirectChildren":0,"outputRows":0,"columnCallbacks":0,"estimatedRows":stats[0][0]}),
            ));
        }
        if direct {
            let mut reader = store.readers.0.lock();
            for r in &mut requests {
                let pre = r[3] as usize;
                let rec = reader.rec(r[2], pre)?;
                r[4] = r[3] + i64::from(reader.value(r[2], Column::Size, pre, rec[2])?);
            }
        }
        let columns = std::array::from_fn(|col| requests.iter().map(|r| r[col]).collect());
        let batch = Arc::new(Batch {
            reader: store.readers.0.clone(),
            columns,
            field,
            kind,
            estimate: stats[0][0],
            filters: AtomicUsize::new(0),
            visits: AtomicUsize::new(0),
            output: AtomicUsize::new(0),
            callbacks: AtomicUsize::new(0),
        });
        const MODULE: Module<Table> = Module::eponymous_only_module();
        db.create_module(c"field_children", &MODULE, Some(batch.clone()))?;
        let query = if variant == "batch-consumers" || variant == "direct-batch-consumers" {
            "WITH children AS MATERIALIZED (SELECT scope,fid,parent,child FROM field_children WHERE request IN (SELECT id FROM requests)), consumers(id,modulus) AS (VALUES(1,2),(2,3)) SELECT consumers.id,children.* FROM children CROSS JOIN consumers WHERE children.child%consumers.modulus=0"
        } else {
            "SELECT scope,fid,parent,child FROM field_children WHERE request IN (SELECT id FROM requests)"
        };
        let result = m.statement(db, query)?;
        let counters = json!({"requests":batch.columns[0].len(),"filters":batch.filters.load(Ordering::Relaxed),"visitedDirectChildren":batch.visits.load(Ordering::Relaxed),"outputRows":batch.output.load(Ordering::Relaxed),"columnCallbacks":batch.callbacks.load(Ordering::Relaxed),"estimatedRows":batch.estimate,"inputLayout":"five contiguous i64 columns; correlated request IDs via sqlite3_vtab_in","maxBufferedOutputRows":1});
        store.readers.clear();
        m.statement(db, "DROP TABLE requests")?;
        return Ok((result, counters));
    };
    store.readers.clear();
    m.statement(db, "DROP TABLE requests")?;
    Ok((rows, json!({"estimatedRows":stats[0][0]})))
}
