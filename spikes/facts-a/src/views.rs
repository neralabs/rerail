//! Fact schema v1: the syntactic core as SQL relations, and the Python-derived relations as SQL
//! views over it (`docs/facts/SCHEMA.md` § 1–2). The view texts are read from SCHEMA.md itself
//! (its `-- view: <name>` lines), so the document is the one definition both this engine and the
//! Phase 2 compiler run. Views are evaluated per content, in document order.
use crate::facts::{self, Facts, NONE};
use crate::sql::engine::{self, Catalog, Db, Schema, Statement, Table};
use crate::sql::value::{Aff, V};
use std::rc::Rc;
use std::sync::LazyLock;

pub const SCHEMA_MD: &str = include_str!("../../../docs/facts/SCHEMA.md");

/// `(name, sql)` of every view, in document order.
pub static VIEW_TEXTS: LazyLock<Vec<(String, String)>> = LazyLock::new(|| {
    let mut out = Vec::new();
    let mut lines = SCHEMA_MD.lines();
    while let Some(l) = lines.next() {
        if let Some(name) = l.strip_prefix("-- view: ") {
            let sql = lines.next().expect("view text").to_owned();
            out.push((name.trim().to_owned(), sql));
        }
    }
    out
});

fn ints(cols: &[&str]) -> Schema {
    Schema { cols: cols.iter().map(|c| c.to_string()).collect(), affs: vec![Aff::None; cols.len()] }
}

/// The core relations' schemas (SCHEMA.md § 1.2).
pub fn core_catalog() -> Catalog {
    let mut c = Catalog::new();
    c.insert("node".into(), ints(&["id", "kind", "parent", "start", "end", "start_line", "end_line", "subtree_end", "named", "flags", "field"]));
    c.insert("child".into(), ints(&["parent", "idx", "child"]));
    c.insert("field".into(), ints(&["parent", "field", "child", "rank"]));
    c.insert("identifier".into(), ints(&["id", "name"]));
    c.insert("line".into(), ints(&["no", "start", "end", "indent"]));
    c
}

/// The views, planned once per process (per thread: statements hold `Rc`s).
pub struct Views {
    pub stmts: Vec<(String, Statement)>,
}

impl Views {
    pub fn compile() -> Result<Views, String> {
        let mut cat = core_catalog();
        let mut stmts = Vec::new();
        for (name, sql) in VIEW_TEXTS.iter() {
            let st = engine::plan(sql, &cat).map_err(|e| format!("view {name}: {e}"))?;
            cat.insert(name.clone(), st.schema().clone());
            stmts.push((name.clone(), st));
        }
        Ok(Views { stmts })
    }
}

/// The core relations of one content as engine tables.
pub fn core_tables(f: &Facts) -> Vec<(&'static str, Table)> {
    let n = f.len();
    let cat = core_catalog();
    let int = |x: u32| if x == NONE { V::Null } else { V::Int(x as i64) };
    let kind = |id: usize| V::Text(facts::name(f.kind[id]));
    let mut field_names: rustc_hash::FxHashMap<u16, Rc<str>> = Default::default();
    let mut fname = |c: u16| -> V {
        if c == 0 {
            return V::Null;
        }
        V::Text(field_names.entry(c).or_insert_with(|| Rc::from(facts::field_name(c).as_str())).clone())
    };
    let mut node = Vec::with_capacity(n);
    for id in 0..n {
        node.push(
            vec![
                V::Int(id as i64),
                kind(id),
                int(f.parent[id]),
                V::Int(f.start[id] as i64),
                V::Int(f.end[id] as i64),
                V::Int(f.start_line[id] as i64),
                V::Int(f.end_line[id] as i64),
                V::Int(f.subtree_end[id] as i64),
                V::Int((f.flags[id] & 1) as i64),
                V::Int(f.flags[id] as i64),
                fname(f.field[id]),
            ]
            .into_boxed_slice(),
        );
    }
    let mut child = Vec::with_capacity(f.children.len());
    let mut field = Vec::new();
    for p in 0..n as u32 {
        let mut ranks: smallvec_ranks::Ranks = Default::default();
        for (i, &c) in f.children(p).iter().enumerate() {
            child.push(vec![V::Int(p as i64), V::Int(i as i64), V::Int(c as i64)].into_boxed_slice());
            let code = f.field[c as usize];
            if code != 0 {
                let r = ranks.bump(code);
                field.push(vec![V::Int(p as i64), fname(code), V::Int(c as i64), V::Int(r as i64)].into_boxed_slice());
            }
        }
    }
    let ident = facts::intern("identifier");
    let identifier = f.of_kind(ident).iter().filter(|&&i| f.named(i)).map(|&i| vec![V::Int(i as i64), V::text(f.text(i))].into_boxed_slice()).collect();
    let source = f.source.as_bytes();
    let line_count = f.lines.line(source.len()) + 1;
    let line = (0..line_count)
        .map(|l| {
            let (a, b) = (f.lines.start(l), f.lines.end(l));
            let indent = source[a..b].iter().take_while(|&&c| c == b' ' || c == b'\t').count();
            vec![V::Int(l as i64 + 1), V::Int(a as i64), V::Int(b as i64), V::Int(indent as i64)].into_boxed_slice()
        })
        .collect();
    let t = |name: &'static str, rows: Vec<Box<[V]>>| (name, Table::from_rows(cat[name].clone(), rows));
    vec![t("node", node), t("child", child), t("field", field), t("identifier", identifier), t("line", line)]
}

mod smallvec_ranks {
    /// Per-parent counters of children per field code (a parent has few fields).
    #[derive(Default)]
    pub struct Ranks(Vec<(u16, u32)>);
    impl Ranks {
        pub fn bump(&mut self, code: u16) -> u32 {
            match self.0.iter_mut().find(|(c, _)| *c == code) {
                Some((_, r)) => {
                    *r += 1;
                    *r - 1
                }
                None => {
                    self.0.push((code, 1));
                    0
                }
            }
        }
    }
}

/// CTAS storage: each column converted by its declared affinity on read (`rel::store`).
pub fn store(t: Table, schema: &Schema) -> Table {
    let len = t.len();
    let cols = t.cols.iter().take(schema.cols.len()).zip(&schema.affs).map(|(c, a)| engine::Column::with_aff(c, *a)).collect();
    Table::new(schema.clone(), cols, len)
}

/// Every view of one content: the core tables and the views, by name, in a database whose path
/// is `""` (content-level, path-free; `source_text('', s, e)` reads the content).
pub fn evaluate<'s>(views: &Views, f: &'s Facts, ns: &mut Vec<u64>) -> Result<Db<'s>, String> {
    let mut db = Db::new(f.source.as_str(), "");
    for (name, t) in core_tables(f) {
        db.tables.insert(name.to_owned(), Rc::new(t));
    }
    ns.clear();
    for (name, st) in &views.stmts {
        let t0 = std::time::Instant::now();
        let t = engine::execute(&db, st).map_err(|e| format!("view {name}: {e}"))?;
        let t = store(t, st.schema());
        db.tables.insert(name.clone(), Rc::new(t));
        ns.push(t0.elapsed().as_nanos() as u64);
    }
    Ok(db)
}
