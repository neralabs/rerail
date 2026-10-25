//! Closed pure utility-entry traces for structurally proved self tails.
//! Boolean operations are projections. Native work/depth failure is an ordered
//! prefix of the entry relation, so no per-Boolean control interpreter is needed.
use super::super::{projected_tail::Projection, utility_projection_producer};
use super::{Demand, Output, Sql};
use crate::code_rule_dsl::sql::relational::emit::condition;
use serde_json::json;
impl Sql {
    pub(super) fn projected_tail(
        &mut self,
        p: &Projection,
        input: &Demand,
        root_filter: &str,
    ) -> Result<Option<Output>, String> {
        let roots = self.generation.components.borrow().roots.clone();
        if roots.is_empty()
            || self.generation.memo_bypass
            || !self.generation.parse_hosts.is_empty()
        {
            return Ok(None);
        }
        let key = p as *const Projection as usize;
        if !input.slots.is_empty()
            && !self
                .generation
                .components
                .borrow()
                .closed_demands
                .contains_key(&key)
        {
            self.profile.push(json!({"operator":"retained reached pure recursion","reason":"captured invocation has no proved closed producer demand"}));
            return Ok(None);
        }
        let existing = self
            .generation
            .components
            .borrow()
            .projected
            .get(&key)
            .cloned();
        let prefix = if let Some(prefix) = existing {
            prefix
        } else {
            let prefix = self.projection_namespace();
            let guard = condition(&p.guard, "n")?;
            let child_filter = p
                .filter
                .as_ref()
                .map(|s| condition(s, "c"))
                .transpose()?
                .unwrap_or("1".into());
            // One closed document domain. Metadata pruning below the utility
            // boundary is performed by the common base projection compiler.
            let domain = format!("{prefix}_nodes");
            let requested = format!("{prefix}_requested");
            let closure = format!("{prefix}_closure");
            let closed = self
                .generation
                .components
                .borrow()
                .closed_demands
                .get(&key)
                .cloned();
            let requested_query=closed.map_or_else(||format!("SELECT n.file,n.id FROM node n WHERE n.file IN (SELECT DISTINCT file FROM {roots}) AND ({root_filter})"),|d|format!("SELECT n.file,n.id FROM {d} d JOIN node n ON n.file=d.file AND n.id=d.node WHERE ({root_filter})"));
            let mut definitions = vec![
                format!("{requested}(file,node) AS MATERIALIZED ({requested_query})"),
                format!(
                    "{closure}(file,node) AS (SELECT file,node FROM {requested} UNION SELECT c.file,c.id FROM {closure} d JOIN node n ON n.file=d.file AND n.id=d.node CROSS JOIN node c ON c.file=n.file AND c.parent=n.id WHERE ({guard}) AND ({child_filter}))"
                ),
                format!(
                    "{domain}(file,node,parent,guard) AS MATERIALIZED (SELECT n.file,n.id,n.parent,({guard}) FROM {closure} d JOIN node n ON n.file=d.file AND n.id=d.node)"
                ),
            ];
            let mut produced = utility_projection_producer::compile_complete(
                &p.base,
                &format!("SELECT file,node,0 incoming FROM {domain}"),
                &format!("{prefix}_base"),
            )?;
            let logical_entries = produced.entry_select().to_owned();
            let searches = p
                .base
                .expressions
                .iter()
                .filter(|e| {
                    matches!(
                        e,
                        super::super::utility_projection_lower::Expr::Search { .. }
                    )
                })
                .count();
            for _ in 0..p.base.functions.len() + 2 * searches + 8 {
                self.generation
                    .operator_as("projected recursive boundary")?;
            }
            let stage_root = self.generation.components.borrow().stage_root.clone();
            if let Some(stage_root) = stage_root {
                let table = format!("{prefix}_materialized_demand");
                let mut seed = vec![stage_root];
                if let Some(closed) = self.generation.components.borrow().stage_demands.get(&key) {
                    seed.extend(closed.iter().cloned());
                }
                seed.extend(definitions.iter().cloned());
                seed.extend(produced.seed_definitions.iter().cloned());
                let create = format!(
                    "CREATE TEMP TABLE IF NOT EXISTS {table}(owner INTEGER NOT NULL,file INTEGER NOT NULL,node INTEGER NOT NULL,incoming INTEGER NOT NULL,root INTEGER NOT NULL,PRIMARY KEY(owner,file,node,incoming)) WITHOUT ROWID"
                );
                let insert = format!(
                    "INSERT INTO {table} WITH RECURSIVE {} SELECT d.owner,d.file,d.node,d.incoming,EXISTS(SELECT 1 FROM {requested} r WHERE r.file=d.file AND r.node=d.node) FROM {} d",
                    seed.join(","),
                    produced.demand
                );
                if insert.len() > 256000 {
                    return Err(
                        "projected demand population exceeds256000 byte statement cap".into(),
                    );
                }
                self.generation
                    .components
                    .borrow_mut()
                    .physical_stages
                    .extend([create, format!("DELETE FROM {table}"), insert]);
                // The table is the physical boundary. Downstream references use
                // its key instead of expanding the recursive demand graph again.
                definitions = vec![
                    format!(
                        "{requested}(file,node) AS NOT MATERIALIZED (SELECT file,node FROM {table} WHERE owner=0 AND root=1)"
                    ),
                    format!(
                        "{domain}(file,node,parent,guard) AS MATERIALIZED (SELECT n.file,n.id,n.parent,({guard}) FROM {table} d JOIN node n ON n.file=d.file AND n.id=d.node WHERE d.owner=0)"
                    ),
                ];
                let seed_count = produced.seed_definitions.len();
                produced.definitions.drain(..seed_count);
                for summary in &mut produced.summaries {
                    summary.index -= seed_count;
                    for reduction in &mut summary.reductions {
                        *reduction -= seed_count;
                    }
                }
                definitions.push(format!("{}(entry,file,node,incoming) AS NOT MATERIALIZED (SELECT owner,file,node,incoming FROM {table} WHERE owner=0)",produced.roots));
                definitions.push(format!("{}(owner,file,node,incoming) AS NOT MATERIALIZED (SELECT owner,file,node,incoming FROM {table})",produced.demand));
            }
            let staged = self.generation.components.borrow().stage_root.is_some();
            definitions = self.cut_projection_definitions(
                definitions,
                produced.definitions,
                &produced.summaries,
                staged,
            )?;
            definitions.push(format!(
                "{prefix}_base_answers AS MATERIALIZED ({})",
                produced.summary_select
            ));
            definitions.push(format!(
                "{prefix}_base_entries AS MATERIALIZED ({})",
                logical_entries
            ));
            definitions.push(format!("{prefix}_links AS MATERIALIZED (SELECT n.file,n.node parent,c.id child FROM {domain} n JOIN {prefix}_base_answers b ON b.file=n.file AND b.node=n.node AND b.incoming=0 CROSS JOIN node c ON c.file=n.file AND c.parent=n.node WHERE n.guard AND b.truth=0 AND ({child_filter}))"));
            definitions.push(format!("{prefix}_truth(file,node) AS (SELECT file,node FROM {prefix}_base_answers WHERE incoming=0 AND truth=1 UNION SELECT e.file,e.parent FROM {prefix}_truth t JOIN {prefix}_links e ON e.file=t.file AND e.child=t.node)"));
            definitions.push(format!("{prefix}_edges AS MATERIALIZED (SELECT file,parent,child FROM (SELECT e.*,min(CASE WHEN t.node IS NOT NULL THEN e.child END) OVER (PARTITION BY e.file,e.parent) stop FROM {prefix}_links e LEFT JOIN {prefix}_truth t ON t.file=e.file AND t.node=e.child) WHERE stop IS NULL OR child<=stop)"));
            definitions.push(format!("{prefix}_visits(file,root,node,depth,path) AS (SELECT file,node,node,0,'' FROM {requested} UNION ALL SELECT v.file,v.root,e.child,v.depth+1,v.path||'2/'||printf('%012d/',e.child) FROM {prefix}_visits v JOIN {prefix}_edges e ON e.file=v.file AND e.parent=v.node)"));
            definitions.push(format!("{prefix}_entries AS MATERIALIZED (SELECT file,root,depth,ROW_NUMBER() OVER (PARTITION BY file,root ORDER BY path) ordinal FROM (SELECT file,root,depth,path||'0' path FROM {prefix}_visits UNION ALL SELECT v.file,v.root,v.depth+1+c.incoming,v.path||'1/'||substr(c.path,1,length(c.path)-1) FROM {prefix}_visits v JOIN {prefix}_base_entries c ON c.root_file=v.file AND c.root_node=v.node AND c.root_incoming=0))"));
            // Merge truth and entry metrics by key. A lookup here repeats a
            // truth-set scan when SQLite underestimates grouped root cardinality.
            definitions.push(format!("{prefix}_metrics AS MATERIALIZED (SELECT file,root node,count(*) cost,max(depth+1) peak FROM {prefix}_entries GROUP BY file,root)"));
            definitions.push(format!("{prefix}_summary AS MATERIALIZED (SELECT file,node,max(cost) cost,max(peak) peak,max(truth) truth FROM (SELECT file,node,cost,peak,0 truth FROM {prefix}_metrics UNION ALL SELECT file,node,NULL,NULL,1 FROM {prefix}_truth) GROUP BY file,node HAVING max(cost) IS NOT NULL)"));
            self.register_projection_definitions(definitions);
            self.generation
                .components
                .borrow_mut()
                .projected
                .insert(key, prefix.clone());
            prefix
        };
        self.generation
            .operator_as("projected recursive invocation")?;
        let depth = self.generation.depth_expression();
        let classified=self.relation("projected recursive invocation",format!("SELECT d.*,s.truth,s.cost,s.peak,CASE WHEN ({depth})+s.peak>256 THEN (SELECT MIN(e.ordinal) FROM {prefix}_entries e WHERE e.file=d.file AND e.root=d.node AND ({depth})+e.depth>=256) END breach FROM {} d JOIN {prefix}_summary s ON s.file=d.file AND s.node=d.node",input.name),true,"closed summary and first prohibited entry");
        let measured=self.relation("pure utility prefix budget",format!("SELECT d.*,min(d.work,d.cost,coalesce(d.breach-1,d.cost)) spent,CASE WHEN d.breach IS NOT NULL AND d.breach-1<=d.work THEN -1 WHEN d.work<d.cost THEN -2 ELSE d.truth END result FROM {classified} d"),true,"depth check precedes work charge");
        let note = if self.generation.peak_tracking {
            format!(
                " AND relation_peak_note(({depth})+CASE WHEN d.breach IS NULL AND d.work>=d.cost THEN d.peak ELSE coalesce((SELECT max(e.depth+1) FROM {prefix}_entries e WHERE e.file=d.file AND e.root=d.node AND e.ordinal-1<=d.work AND (d.breach IS NULL OR e.ordinal<d.breach)),0) END)=1"
            )
        } else {
            String::new()
        };
        self.profile.push(json!({"operator":"projected pure self tail","summary":prefix,"booleanContinuations":0,"entryWork":1,"failure":"ordered entry prefix; native depth before charge"}));
        Ok(Some(Output {
            name: self.relation(
                "projected recursive completion",
                format!(
                    "SELECT d.k,d.work-d.spent work,d.result truth FROM {measured} d WHERE 1{note}"
                ),
                true,
                "exact native work/depth/peak",
            ),
        }))
    }
}
