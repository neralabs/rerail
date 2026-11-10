//! One SQL statement evaluates dependent relational outputs against frozen inputs.
//! Native code only validates and publishes the resulting storage patches.
use super::scratch::Work;
use super::*;

mod reads;

struct Definition {
    sql: String,
    materialized: bool,
    width: usize,
    expansion: usize,
}

enum Keys {
    Native(versions::Version),
    Query(String),
    Incoming,
    Replace,
    Insert(bool),
}

struct Output {
    table: String,
    source: String,
    old: versions::Version,
    width: usize,
    keys: Keys,
    key_width: usize,
}
pub(super) struct Counts {
    pub keys: usize,
    pub rows: usize,
    pub strategy: crate::code_rule_dsl::logic::analysis::storage::Publication,
}
#[derive(Default)]
pub(super) struct Phase {
    pub analyses: Vec<Value>,
    definitions: Vec<Definition>,
    outputs: Vec<Output>,
    key_sets: BTreeMap<String, crate::code_rule_dsl::logic::analysis::key_sets::KeySet>,
    read_arms: usize,
}
impl Phase {
    // Reserve preparation headroom for key journals, output arms, and outer folds.
    pub const EXPANSION_BUDGET: usize = 20000;
    pub fn arm_bound(&self) -> usize {
        self.read_arms
    }
    /// SQLite expands references behind materialization fences during preparation.
    /// Count the dependency graph with multiplicity, rather than only its definitions.
    pub fn expansion_cost(&self) -> usize {
        self.outputs.iter().fold(0usize,|cost,output| {
            let cost=cost.saturating_add(self.source_expansion(&output.source));
            if let Keys::Query(keys)=&output.keys {cost.saturating_add(self.source_expansion(keys))} else {cost}
        })
    }
    fn source_expansion(&self,source:&str)->usize {
        let mut cost=1usize;
        for word in source.split(|c:char| !(c.is_ascii_alphanumeric() || c=='_')) {
            if let Some(id)=word.strip_prefix("__rr_phase_").and_then(|id|id.parse::<usize>().ok()) {
                if let Some(definition)=self.definitions.get(id) {cost=cost.saturating_add(definition.expansion);}
            } else if word.eq_ignore_ascii_case("SELECT") || word.eq_ignore_ascii_case("JOIN") || word.eq_ignore_ascii_case("FROM") {
                cost=cost.saturating_add(1);
            }
        }
        cost
    }
    pub fn defines(&self, name: &str) -> bool {
        name.strip_prefix("__rr_phase_")
            .and_then(|n| n.parse::<usize>().ok())
            .is_some_and(|n| n < self.definitions.len())
    }
    pub fn relation(&mut self, width: usize, query: &str) -> String {
        self.define(width, query, "MATERIALIZED")
    }
    pub fn checked_relation(&mut self,width:usize,query:&select::Query)->String {
        fn block_cost(block:&select::Select,ctes:&BTreeMap<String,usize>)->usize {
            use super::super::logic::sql::sql_ast::SqlExpression as E;
            let mut cost=1usize;
            for item in &block.from {
                cost=cost.saturating_add(1);
                if let select::Relation::Cte(name)=&item.relation {
                    cost=cost.saturating_add(ctes.get(name).copied().unwrap_or(1));
                }
            }
            for expression in block.columns.iter().map(|(e,_)|e).chain(&block.predicates)
                .chain(block.from.iter().flat_map(|i|match &i.join {select::Join::On(on)=>on.as_slice(),_=>&[]})) {
                expression.visit(&mut |e| {
                    if let E::Scalar(q) | E::Exists {query:q,..}=e {
                        cost=cost.saturating_add(block_cost(q,ctes));
                    }
                });
            }
            cost
        }
        let mut ctes=BTreeMap::new();
        for cte in &query.ctes {
            let cost=cte.arms.iter().fold(0usize,|sum,arm|sum.saturating_add(block_cost(arm,&ctes)));
            ctes.insert(cte.name.clone(),cost);
        }
        let name=self.relation(width,&query.render());
        let definition=self.definitions.last_mut().unwrap();
        definition.expansion=definition.expansion.saturating_add(block_cost(&query.body,&ctes));
        name
    }
    /// Projections, set operations, and pure aggregates over frozen inputs.
    /// Checked rule expressions and native functions stay behind `relation` fences.
    pub fn view(&mut self, width: usize, query: &str) -> String {
        self.define(width, query, "NOT MATERIALIZED")
    }
    fn define(&mut self, width: usize, query: &str, hint: &str) -> String {
        let name = format!("__rr_phase_{}", self.definitions.len());
        let columns = (0..width)
            .map(|i| format!("c{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let expansion=self.source_expansion(query);
        self.definitions.push(Definition {
            expansion,
            sql: format!("{name}({columns}) AS {hint} ({query})"),
            materialized: hint == "MATERIALIZED",
            width,
        });
        name
    }
    /// Membership lineage is separate from SQL strings and bag evaluation.
    pub fn key_set(
        &self,
        source: &str,
        width: usize,
    ) -> crate::code_rule_dsl::logic::analysis::key_sets::KeySet {
        self.key_sets
            .get(source)
            .map(|s| s.project(&(0..width).collect::<Vec<_>>()))
            .unwrap_or_else(|| {
                crate::code_rule_dsl::logic::analysis::key_sets::KeySet::source(source, width)
            })
    }
    pub fn union_keys(&mut self, width: usize, sources: &[String]) -> String {
        use crate::code_rule_dsl::logic::analysis::key_sets::KeySet;
        // A direct copy of a phase output preserves its rows and evaluation fence.
        // Reuse the name so publication can also share its SQL result arm.
        if sources.len() == 1
            && sources[0]
                .strip_prefix("__rr_phase_")
                .and_then(|n| n.parse::<usize>().ok())
                .and_then(|n| self.definitions.get(n))
                .is_some_and(|d| d.width == width)
        {
            return sources[0].clone();
        }
        let set = KeySet::union(width, sources.iter().map(|s| self.key_set(s, width)));
        let sql = if sources.is_empty() {
            format!("SELECT {} WHERE 0", vec!["NULL"; width].join(","))
        } else {
            let fields = (0..width)
                .map(|c| format!("c{c}"))
                .collect::<Vec<_>>()
                .join(",");
            sources
                .iter()
                .map(|s| format!("SELECT {fields} FROM {}", quote(s)))
                .collect::<Vec<_>>()
                .join(" UNION ")
        };
        let name = self.relation(width, &sql);
        self.key_sets.insert(name.clone(), set);
        name
    }
    /// Compute membership subtraction once. A correlated probe over a materialized
    /// CTE can scan the complete replacement set for every affected row.
    pub fn difference_keys(&mut self, width: usize, included: &str, excluded: &str) -> String {
        let fields = (0..width)
            .map(|c| format!("c{c}"))
            .collect::<Vec<_>>()
            .join(",");
        self.relation(
            width,
            &format!(
                "SELECT {fields} FROM {} EXCEPT SELECT {fields} FROM {}",
                quote(included),
                quote(excluded)
            ),
        )
    }
    pub fn patch(
        &mut self,
        store: &versions::Store,
        table: &str,
        keys: &str,
        source: &str,
    ) -> Result<()> {
        let keys = store
            .available(keys)
            .ok_or_else(|| error("missing SQL phase keys"))?;
        self.output(store, table, source, Keys::Native(keys))
    }
    pub fn patch_computed(
        &mut self,
        store: &versions::Store,
        table: &str,
        keys: &str,
        source: &str,
    ) -> Result<()> {
        self.output(store, table, source, Keys::Query(keys.into()))
    }
    pub fn load_empty(&mut self, store: &versions::Store, table: &str, source: &str) -> Result<()> {
        if store.available(table).is_none_or(|v| v.len() != 0) {
            return Err(error("SQL phase load requires an empty native table"));
        }
        self.output(store, table, source, Keys::Incoming)
    }
    pub fn insert(&mut self,store:&versions::Store,table:&str,source:&str,ignore:bool)->Result<()> {
        self.output(store,table,source,Keys::Insert(ignore))
    }
    pub fn replace_all(
        &mut self,
        store: &versions::Store,
        table: &str,
        source: &str,
    ) -> Result<()> {
        self.output(store, table, source, Keys::Replace)
    }
    fn output(
        &mut self,
        store: &versions::Store,
        table: &str,
        source: &str,
        keys: Keys,
    ) -> Result<()> {
        if self.outputs.iter().any(|o| o.table == table) {
            return Err(error("duplicate SQL phase output"));
        }
        let old = store
            .available(table)
            .ok_or_else(|| error("missing SQL phase output"))?;
        self.read_arms += 1 + usize::from(matches!(keys, Keys::Query(_)));
        self.outputs.push(Output {
            table: table.into(),
            source: source.into(),
            width: old.columns().len(),
            key_width: old.primary_key_len().unwrap(),
            old,
            keys,
        });
        Ok(())
    }
    pub fn execute(self, w: &Work<'_>) -> Result<BTreeMap<String, Counts>> {
        if self.outputs.is_empty() { return Ok(BTreeMap::new()); }
        crate::code_rule_dsl::logic::lanes::sql_executor().local(
            rerail_accel::Lane::Sql, "SQL retained phase", 0.0,
            || self.execute_owned(w),
        )
    }
    fn execute_owned(self, w: &Work<'_>) -> Result<BTreeMap<String, Counts>> {
        if self.outputs.is_empty() {
            return Ok(BTreeMap::new());
        }
        let store = w
            .versions
            .ok_or_else(|| error("SQL phase requires native storage"))?;
        let width = self.outputs.iter().map(|o| o.width).max().unwrap();
        let reads = reads::Plan::new(&self, width);
        let sql = format!(
            "WITH {} {}",
            self.definitions
                .iter()
                .map(|d| d.sql.as_str())
                .collect::<Vec<_>>()
                .join(","),
            reads.arms.join(" UNION ALL ")
        );
        let _context = w.profile_scope(||json!({"stage":"fused relational phase","phaseOutputs":self.outputs.iter().map(|o|&o.table).collect::<Vec<_>>(),"publication":self.outputs.iter().map(|o|json!({"table":o.table,"previous_rows":o.old.len(),"strategy":if o.old.len()==0{"bulk empty root"}else if matches!(o.keys,Keys::Replace){"bulk replacement"}else{"validated patch; affected-key density selects bulk rebuilding"}})).collect::<Vec<_>>(),"logicalAnalyses":self.analyses,"outputProtocol":{"sqlArms":reads.arms.len(),"logicalReads":reads.logical_reads,"dependencyExpansionCost":self.expansion_cost()}}));
        // Delta-first CROSS JOINs and primary-key probes fix the sparse access
        // paths. Reuse preparation across wave scopes while xFilter reads their
        // current immutable roots; rows and expressions are never cached here.
        let mut statement = {
            let _cpu = super::super::logic::profile::span(|| "SQL phase preparation".into());
            let operation = w.profiler.map(|p| {
                p.operation(
                    "SQL phase preparation",
                    ("sql-extension", 0),
                    0,
                    vec![
                        ("sql_bytes".into(), sql.len() as i64),
                        ("definitions".into(), self.definitions.len() as i64),
                        ("output_arms".into(), reads.arms.len() as i64),
                        ("logical_reads".into(), reads.logical_reads as i64),
                        ("dependency_expansion_cost".into(), self.expansion_cost().min(i64::MAX as usize) as i64),
                    ],
                )
            });
            let prepared = w.db.prepare_cached(&sql).map_err(|e|error(format!("{e}; dependency expansion cost {}, definitions {}, result arms {}",self.expansion_cost(),self.definitions.len(),reads.arms.len())));
            if let Some(operation) = operation {
                operation.finish_unobserved();
            }
            prepared?
        };
        let mut cursor = statement.query([]).map_err(error)?;
        let mut rows = vec![Vec::new(); self.outputs.len()];
        let mut key_rows = vec![Vec::new(); self.outputs.len()];
        while let Some(row) = cursor.next().map_err(error)? {
            reads.collect(row, &mut rows, &mut key_rows)?;
        }
        drop(cursor);
        drop(statement);
        let mut counts = self
            .outputs
            .iter()
            .zip(&rows)
            .zip(&key_rows)
            .map(|((o, rows), keys)| {
                let keys = match &o.keys {
                    Keys::Native(k) => k.len(),
                    Keys::Query(_) => keys.len(),
                    Keys::Incoming | Keys::Replace | Keys::Insert(_) => rows.len(),
                };
                (
                    o.table.clone(),
                    Counts {
                        keys,
                        rows: rows.len(),
                        strategy: if o.old.len() == 0 {
                            crate::code_rule_dsl::logic::analysis::storage::Publication::BulkEmpty
                        } else if matches!(o.keys, Keys::Replace) {
                            crate::code_rule_dsl::logic::analysis::storage::Publication::BulkReplacement
                        } else { crate::code_rule_dsl::logic::analysis::storage::Publication::SparsePatch },
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let operation = w
            .profiler
            .map(|p| p.operation("SQL phase publication", ("sql-extension", 0), 0, vec![]));
        let _cpu = super::super::logic::profile::span(|| "SQL phase publication".into());
        // Prepare every replacement before changing any root. A failing later output
        // cannot leave an earlier output partially published, even without a savepoint.
        let mut reusable = BTreeMap::<(usize, usize), Vec<versions::Version>>::new();
        let mut shared_roots = 0;
        let mut retained_roots = 0;
        let prepared = self
            .outputs
            .into_iter()
            .zip(rows)
            .zip(key_rows)
            .map(|((o, rows), keys)| {
                let candidates = if o.old.len() == 0 && rows.len() >= 512 {
                    reusable
                        .get(&(o.width, rows.len()))
                        .map_or(&[][..], Vec::as_slice)
                } else {
                    &[][..]
                };
                let reused = if matches!(o.keys, Keys::Incoming | Keys::Replace)
                    && o.old.len() == 0
                    && rows.len() >= 512
                {
                    candidates
                        .iter()
                        .find_map(|v| o.old.reuse_empty_rows(&rows, v))
                } else {
                    None
                };
                let next = if let Some(next) = reused {
                    shared_roots += 1;
                    next
                } else {
                    match o.keys {
                        Keys::Native(keys) => {
                            if keys.columns().len() != o.key_width {
                                return Err(error(
                                    "SQL phase patch requires the complete primary key",
                                ));
                            }
                            let (next, shared, strategy) =
                                o.old
                                    .patched_rows_reusing(keys.rows(), rows, true, candidates)?;
                            counts.get_mut(&o.table).unwrap().strategy = strategy;
                            shared_roots += usize::from(shared);
                            next
                        }
                        Keys::Query(_) => {
                            let (next, shared, strategy) =
                                o.old.patched_rows_reusing(keys, rows, true, candidates)?;
                            counts.get_mut(&o.table).unwrap().strategy = strategy;
                            shared_roots += usize::from(shared);
                            next
                        }
                        Keys::Incoming => {
                            if o.old.len() != 0 {
                                return Err(error("SQL phase load requires an empty native table"));
                            }
                            o.old.loaded(rows, false, false)?
                        }
                        Keys::Replace => o.old.loaded(rows, false, true)?,
                        Keys::Insert(ignore) => {
                            let next=o.old.inserted_rows(rows,ignore)?;
                            let inserted=next.len()-o.old.len();
                            let count=counts.get_mut(&o.table).unwrap();
                            count.rows=inserted;
                            count.keys=inserted;
                            count.strategy=crate::code_rule_dsl::logic::analysis::storage::publication(o.old.len(),inserted);
                            next
                        }
                    }
                };
                if o.old.len() > 0 && o.old.shares_rows(&next) {
                    retained_roots += 1;
                }
                if o.old.len() == 0 && next.len() >= 512 {
                    reusable
                        .entry((o.width, next.len()))
                        .or_default()
                        .push(next.clone());
                }
                Ok((o.table, next))
            })
            .collect::<Result<Vec<_>>>();
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                if let Some(operation) = operation {
                    operation.finish_unobserved();
                }
                return Err(error);
            }
        };
        let _publication_context = w.profile_scope(|| {
            json!({"logicalAnalyses":[{
                "analysis":"publication strategy","outputs":counts.iter().map(|(table,c)|json!({
                    "table":table,"affected_keys":c.keys,"rows":c.rows,"strategy":c.strategy
                })).collect::<Vec<_>>()
            }]})
        });
        if let Some(profiler) = w.profiler {
            profiler
                .operation(
                    "SQL phase publication strategies",
                    ("sql-extension", 0),
                    0,
                    vec![],
                )
                .finish_unobserved();
        }
        super::super::logic::profile::decision("SQL phase root reuse", || {
            format!(
                "{shared_roots} equal ordered initial outputs shared validated roots; {retained_roots} nonempty outputs retained their row roots within this phase"
            )
        });
        if let Some(profiler) = w.profiler {
            profiler
                .operation(
                    "SQL phase shared initial roots",
                    ("sql-extension", 0),
                    0,
                    vec![
                        ("shared_roots".into(), shared_roots as i64),
                        ("retained_nonempty_roots".into(), retained_roots as i64),
                    ],
                )
                .finish(shared_roots);
        }
        for (table, version) in prepared {
            store.replace_available(&table, version)?;
        }
        if let Some(operation) = operation {
            operation.finish_unobserved();
        }
        Ok(counts)
    }
}

#[cfg(test)]
mod tests;
