//! Compile the former statement scheduler into recursive SQL with materialized set producers.
//! Native callbacks provide relation storage and structural validation, never SQL execution.
use super::*;
use crate::code_rule_dsl::sql::fused::lex;
use std::collections::{BTreeMap, BTreeSet};
mod dml;
mod materialize;
mod profile;
mod storage;
#[cfg(test)]
mod tests;
use dml::Schema;

pub(super) struct Program {
    pub queries: Vec<String>,
    schemas: BTreeMap<String, Schema>,
    pub clauses: Vec<usize>,
}
struct Instruction {
    effect: String,
    next: String,
    loops: String,
    stack: String,
}
struct Scope {
    start: usize,
    end: usize,
    kind: &'static str,
    statement: usize,
}
struct Builder<'a> {
    plan: &'a Plan,
    schemas: &'a mut BTreeMap<String, Schema>,
    code: Vec<Instruction>,
    stages: BTreeSet<usize>,
    serial: usize,
    blocks: HashMap<(usize, u32), usize>,
    pending: Vec<(Bound<'a>, u32)>,
    calls: Vec<(usize, (usize, u32))>,
    phase: usize,
    free: HashMap<String, Vec<String>>,
    scopes: Vec<Scope>,
}
impl<'a> Builder<'a> {
    fn temporary(&mut self, columns: &str) -> Result<String, String> {
        if let Some(name) = self.free.get_mut(columns).and_then(Vec::pop) {
            return Ok(name);
        }
        let name = format!("p_cte_{}", self.serial);
        self.serial += 1;
        self.schema(&name, columns)?;
        Ok(name)
    }
    fn action(&mut self, effect: String) -> usize {
        let id = self.code.len();
        self.code.push(Instruction {
            effect,
            next: (id + 1).to_string(),
            loops: "driver.loops".into(),
            stack: "driver.stack".into(),
        });
        id
    }
    fn branch(&mut self, predicate: String) -> usize {
        let id = self.action("0".into());
        self.code[id].next = predicate;
        id
    }
    fn materialized(&mut self, sql: &str) -> Result<(String, Vec<String>), String> {
        // Resolve proven key membership before the general nullable-set materializer.
        // Otherwise a direct indexed probe becomes a copied relation and three probes.
        let mut sql = dml::key_probes(sql, self.schemas);
        let mut transient = Vec::new();
        if let Some((producers, body)) = materialize::split(&sql, self.schemas) {
            let mut names = HashMap::new();
            for (at, producer) in producers.iter().enumerate() {
                let source = materialize::bindings(&producer.body, &names);
                let consumers = producers[at + 1..]
                    .iter()
                    .map(|p| p.body.as_str())
                    .chain(std::iter::once(body.as_str()))
                    .collect::<Vec<_>>();
                // Resolve nested WITH scopes through the materializer first. Moving them
                // under a FROM wrapper would hide local names from membership extraction.
                if !lex::identifiers(&source).contains(&"WITH")
                    && materialize::pipeline(&producer.name, &consumers)
                {
                    names.insert(
                        producer.name.clone(),
                        format!(
                            "(WITH {}({}) AS NOT MATERIALIZED ({source}) SELECT * FROM {})",
                            producer.name,
                            producer.columns.join(","),
                            producer.name
                        ),
                    );
                    continue;
                }
                let mut columns = producer.columns.clone();
                for key in ["k", "ek"] {
                    if producer.columns.iter().any(|c| c == key) {
                        columns.push(format!("INDEX ({key})"));
                    }
                }
                if producer.columns.iter().any(|c| c == "file")
                    && producer.columns.iter().any(|c| c == "node")
                {
                    columns.push("INDEX (file,node)".into());
                }
                let name = self.temporary(&columns.join(","))?;
                let (source, nested) = self.materialized(&source)?;
                self.action(dml::publish(self.schemas, &name, &source, "replace")?);
                self.release(nested);
                transient.push(name.clone());
                names.insert(producer.name.clone(), name);
            }
            let body = if body.contains("t.ek = f.file * 4294967296") {
                body.replace(" JOIN t ON ", " CROSS JOIN t ON ")
            } else {
                body
            };
            sql = materialize::bindings(&body, &names);
        }
        while let Some(member) = materialize::membership(&sql) {
            let name = self.temporary("value,INDEX (value)")?;
            let (query, nested) = self.materialized(member.query)?;
            self.action(dml::publish(self.schemas, &name, &query, "replace")?);
            self.release(nested);
            // Preserve SQL's NULL and empty-set behavior, including under NOT and OR.
            let lhs = member.lhs;
            let replacement = format!(
                "{}(CASE WHEN ({lhs}) IS NULL THEN CASE WHEN p_rows(driver.tick,'{name}')>0 THEN NULL ELSE 0 END WHEN EXISTS(SELECT 1 FROM {name} WHERE value=({lhs})) THEN 1 WHEN EXISTS(SELECT 1 FROM {name} WHERE value IS (NULL + driver.tick)) THEN NULL ELSE 0 END)",
                if member.negated { "NOT " } else { "" }
            );
            sql.replace_range(member.start..member.end, &replacement);
            transient.push(name);
        }
        Ok((sql, transient))
    }
    fn release(&mut self, names: Vec<String>) {
        for name in names {
            self.action(format!("p_release(driver.tick,'{name}')"));
            let declaration = &self.schemas[&name].declaration;
            let key = declaration
                .split_once('(')
                .unwrap()
                .1
                .strip_suffix(')')
                .unwrap()
                .to_owned();
            self.free.entry(key).or_default().push(name);
        }
    }
    fn statement(&mut self, sql: &str) -> Result<(), String> {
        let (sql, transient) = self.materialized(sql)?;
        match dml::lower(self.schemas, &sql)? {
            dml::Action::Clear(name) => {
                self.action(format!("p_clear(driver.tick,'{name}')"));
            }
            dml::Action::Publish { target, body, mode } => {
                let (body, nested) = self.materialized(&body)?;
                self.action(dml::publish(self.schemas, &target, &body, mode)?);
                self.release(nested);
            }
        }
        self.release(transient);
        Ok(())
    }
    fn clear(&mut self, bound: Bound<'_>, steps: &[Step]) -> Result<(), String> {
        for step in steps {
            let mut sql = String::new();
            bound.text(step, &mut sql);
            if let Some(table) = sql.strip_prefix(frames::CLEAR) {
                self.action(format!("p_clear(driver.tick,'{table}')"));
            } else {
                self.statement(&sql)?;
            }
        }
        Ok(())
    }
    fn steps(&mut self, bound: Bound<'a>, steps: &[Step]) -> Result<(), String> {
        let mut at = 0;
        while at < steps.len() {
            let step = &steps[at];
            let mut sql = String::new();
            bound.text(step, &mut sql);
            match bound.action(step).kind {
                Kind::Statement | Kind::Until => {
                    use crate::code_rule_dsl::sql::emit::Rule;
                    let action = bound.action(step);
                    let inputs = bound
                        .inputs(action)
                        .map(|id| format!("p_rows(driver.tick,'{}')>0", self.plan.tables[id].name))
                        .collect::<Vec<_>>();
                    let condition = match action.rule {
                        Rule::AlwaysRun => None,
                        Rule::EmptyIfAnyInputEmpty if inputs.is_empty() => None,
                        Rule::EmptyIfAnyInputEmpty => Some(inputs.join(" AND ")),
                        Rule::EmptyIfAllInputsEmpty if inputs.is_empty() => None,
                        Rule::EmptyIfAllInputsEmpty => Some(inputs.join(" OR ")),
                        Rule::EmptyIfPrimaryEmpty | Rule::CopyOf => inputs.first().cloned(),
                    };
                    let guard = condition.as_ref().map(|_| self.action("0".into()));
                    self.statement(&sql)?;
                    if let Some(guard) = guard {
                        self.code[guard].next = format!(
                            "CASE WHEN {} THEN {} ELSE {} END",
                            condition.unwrap(),
                            guard + 1,
                            self.code.len()
                        );
                    }
                }
                Kind::Clear(_) => {
                    self.action(format!(
                        "p_clear(driver.tick,'{}')",
                        sql.strip_prefix(frames::CLEAR)
                            .ok_or("program clear marker")?
                    ));
                }
                Kind::Run(index) => {
                    let key = (bound.instance() as *const Instance as usize, index);
                    if let std::collections::hash_map::Entry::Vacant(entry) = self.blocks.entry(key)
                    {
                        entry.insert(usize::MAX);
                        self.pending.push((bound, index));
                    }
                    let call = self.action("0".into());
                    self.code[call].stack =
                        format!("json_insert(driver.stack,'$[#]',{})", call + 1);
                    self.calls.push((call, key));
                }
                Kind::Acyclic => {
                    let query = sql
                        .strip_prefix(emit::ACYCLIC)
                        .ok_or("program cycle marker")?;
                    let query = dml::correlate(query, self.schemas);
                    self.action(format!("(WITH edges(a,b,c,d,e) AS MATERIALIZED ({query}) SELECT p_acyclic(driver.tick,a,b,c,d,e) FROM edges)"));
                }
                Kind::Loop | Kind::Frontier => {
                    let mut depth = 0;
                    let end = (at..steps.len())
                        .find(|&i| {
                            match bound.action(&steps[i]).kind {
                                Kind::Loop | Kind::Frontier => depth += 1,
                                Kind::Until => depth -= 1,
                                _ => {}
                            }
                            depth == 0
                        })
                        .ok_or("program loop end")?;
                    let mut until = String::new();
                    bound.text(&steps[end], &mut until);
                    let until = until
                        .strip_prefix(emit::UNTIL)
                        .ok_or("program until marker")?;
                    if let Some(description) = until.strip_prefix(emit::FRONTIER) {
                        self.frontier(bound, &steps[at + 1..end], description)?;
                    } else {
                        let query = dml::correlate(until, self.schemas);
                        let enter = self.action("0".into());
                        self.code[enter].loops = format!("json_set(driver.loops,'$.l{enter}',0)");
                        let guard = self.action("0".into());
                        let body = self.code.len();
                        self.steps(bound, &steps[at + 1..end])?;
                        let back = self.action("0".into());
                        self.code[back].loops = format!(
                            "json_set(driver.loops,'$.l{enter}',COALESCE(json_extract(driver.loops,'$.l{enter}'),0)+1)"
                        );
                        self.code[back].next = guard.to_string();
                        self.code[guard].next = format!(
                            "CASE WHEN ({query}) > 0 THEN CASE WHEN p_depth(COALESCE(json_extract(driver.loops,'$.l{enter}'),0)) IS NOT NULL THEN {body} ELSE {after} END ELSE {after} END",
                            after = self.code.len()
                        );
                    }
                    at = end;
                }
            }
            at += 1;
        }
        Ok(())
    }
    fn schema(&mut self, name: &str, columns: &str) -> Result<(), String> {
        self.schemas
            .insert(name.into(), Schema::of(format!("{name}({columns})"))?);
        Ok(())
    }
    fn frontier(
        &mut self,
        bound: Bound<'a>,
        steps: &[Step],
        description: &str,
    ) -> Result<(), String> {
        let parts: Vec<_> = description.split_whitespace().collect();
        if parts.len() != 8 {
            return Err("program frontier descriptor".into());
        }
        let (all, target, matched, opened, mode, three, min, max) = (
            parts[0],
            parts[1],
            parts[2],
            parts[3],
            parts[4],
            parts[5] == "1",
            parts[6],
            parts[7],
        );
        let serial = self.serial;
        self.serial += 1;
        let ranked = format!("p_rank_{serial}");
        let state = format!("p_frontier_{serial}");
        let original = self
            .schemas
            .get(all)
            .ok_or("program frontier source")?
            .clone();
        let cols = original.columns.join(",");
        self.schema(
            &ranked,
            &format!(
                "{},position INTEGER",
                original
                    .columns
                    .iter()
                    .map(|c| if c == "k" {
                        "k INTEGER PRIMARY KEY".into()
                    } else {
                        c.clone()
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        )?;
        self.schema(
            &state,
            "pk INTEGER PRIMARY KEY,total INTEGER,lim INTEGER,done INTEGER,last_ord INTEGER",
        )?;
        self.action(dml::publish(
            self.schemas,
            &ranked,
            &format!("SELECT *,ROW_NUMBER() OVER (PARTITION BY pk ORDER BY ord,k) FROM {all}"),
            "replace",
        )?);
        self.action(dml::publish(
            self.schemas,
            &state,
            &format!("SELECT pk,COUNT(*),0,0,0 FROM {all} GROUP BY pk"),
            "replace",
        )?);
        let head = self.code.len();
        self.action(format!(
            "p_loop_pass(driver.tick,'{}{description}',{})",
            emit::FRONTIER,
            steps.len()
        ));
        self.statement(&format!("UPDATE {state} SET lim = CASE WHEN done THEN lim WHEN lim=0 THEN MIN(total,8) WHEN total <= lim*4 THEN total ELSE lim*2 END"))?;
        self.statement(&format!("UPDATE {state} SET done = done OR lim=total, last_ord = COALESCE((SELECT r.ord FROM {ranked} r WHERE r.pk={state}.pk AND r.position={state}.lim),0)"))?;
        self.action(dml::publish(self.schemas,target,&format!("SELECT {} FROM {ranked} r JOIN {state} s ON r.pk=s.pk WHERE r.position<=s.lim ORDER BY r.k",original.columns.iter().map(|c|format!("r.{c}")).collect::<Vec<_>>().join(",")),"replace")?);
        self.steps(bound, steps)?;
        let unknown = if three
            && mode != "strict"
            && self.schemas[matched].columns.iter().any(|c| c == "unk")
        {
            " AND m.unk=0"
        } else {
            ""
        };
        let count = format!(
            "(SELECT COUNT(*) FROM {matched} m JOIN {ranked} r ON r.k=m.k WHERE r.pk={state}.pk{unknown})"
        );
        let decided = if mode == "count" {
            if max == "-" {
                format!("{count}>={min}")
            } else {
                format!("{count}>{max}")
            }
        } else {
            format!("{count}>0")
        };
        let open = if mode == "strict" && opened != "-" {
            let columns = &self.schemas[opened].columns;
            format!(
                " OR EXISTS(SELECT 1 FROM {opened} o WHERE o.{}={state}.pk AND o.{}<=2*{state}.last_ord AND {state}.lim>0)",
                columns[0], columns[1]
            )
        } else {
            String::new()
        };
        self.statement(&format!(
            "UPDATE {state} SET done = done OR ({decided}){open}"
        ))?;
        let after = self.code.len() + 1;
        let query = dml::correlate(
            &format!("EXISTS(SELECT 1 FROM {state} WHERE done=0)"),
            self.schemas,
        );
        self.branch(format!("CASE WHEN {query} THEN {head} ELSE {after} END"));
        let _ = cols;
        Ok(())
    }
    fn stage(&mut self, stage: usize) -> Result<(), String> {
        if !self.stages.insert(stage) {
            return Ok(());
        }
        let ready = self.action("0".into());
        let bound = self.plan.prepared(stage);
        for &dependency in &bound.shape.stages {
            self.stage(dependency)?;
        }
        let mut empty = None;
        if let Some(root) = bound.root() {
            self.steps(bound, &bound.shape.steps[..=root.at])?;
            empty = Some(self.action("0".into()));
            self.steps(bound, &bound.shape.steps[root.at + 1..])?;
        } else {
            self.steps(bound, &bound.shape.steps)?;
        }
        let mut sql = String::new();
        bound.text(&bound.shape.last, &mut sql);
        self.statement(&sql)?;
        let finished = self.code.len();
        if let Some(guard) = empty {
            let table = &self.plan.tables[bound.root().unwrap().table].name;
            let query = dml::correlate(&format!("EXISTS(SELECT 1 FROM {table})"), self.schemas);
            self.code[guard].next =
                format!("CASE WHEN {query} THEN {} ELSE {finished} END", guard + 1);
        }
        self.statement(&format!("INSERT INTO p_stage_ready VALUES ({stage})"))?;
        let query = dml::correlate(
            &format!("EXISTS(SELECT 1 FROM p_stage_ready WHERE stage={stage})"),
            self.schemas,
        );
        self.code[ready].next = format!(
            "CASE WHEN {query} THEN {} ELSE {} END",
            self.code.len(),
            ready + 1
        );
        self.scopes.push(Scope {
            start: ready,
            end: self.code.len(),
            kind: "stage",
            statement: stage,
        });
        Ok(())
    }
    fn clause(&mut self, clause: usize) -> Result<(), String> {
        let bound = self.plan.prepared(self.plan.stages.len() + clause);
        let initial = self.action("0".into());
        let each = bound.each();
        if let Some((prefix, _, _)) = each {
            let enter = self.action("0".into());
            self.code[enter].loops = format!("json_set(driver.loops,'$.{prefix}r',0)");
        }
        let restart = self.code.len();
        let mut empty = None;
        if let Some(root) = bound.root() {
            self.steps(bound, &bound.shape.steps[..=root.at])?;
            empty = Some(self.action("0".into()));
            for &stage in &bound.shape.stages {
                if self.plan.sole[stage] == Some(clause) {
                    self.stage(stage)?;
                }
            }
            self.steps(bound, &bound.shape.steps[root.at + 1..])?;
        } else {
            self.steps(bound, &bound.shape.steps)?;
        }
        let captures = &self.plan.clauses[clause].1;
        let width = captures
            .iter()
            .map(|(_, kind, at)| {
                at + match kind {
                    SlotKind::Node | SlotKind::Count => 1,
                    SlotKind::Text => 2,
                    SlotKind::List => 4,
                }
            })
            .max()
            .unwrap_or(2);
        let labels = (0..width).map(|i| format!("v{i}")).collect::<Vec<_>>();
        let mut sql = String::new();
        bound.text(&bound.shape.last, &mut sql);
        let round = each.map_or("0".into(), |(prefix, _, _)| {
            format!("(SELECT round FROM {prefix}r)")
        });
        self.statement(&format!("WITH returned({}) AS MATERIALIZED ({sql}) INSERT INTO p_results SELECT {clause},v0,v1,json_array({}),{round} FROM returned",labels.join(","),labels[2..].join(",")))?;
        let cleanup = self.code.len();
        self.clear(bound, &bound.shape.clear)?;
        if let Some(guard) = empty {
            let table = &self.plan.tables[bound.root().unwrap().table].name;
            let query = dml::correlate(&format!("EXISTS(SELECT 1 FROM {table})"), self.schemas);
            self.code[guard].next =
                format!("CASE WHEN {query} THEN {} ELSE {cleanup} END", guard + 1);
        }
        if let Some((prefix, _, _)) = each {
            let guard = self.action("0".into());
            let again = self.code.len();
            let increment = self.action("0".into());
            self.code[increment].loops = format!(
                "json_set(driver.loops,'$.{prefix}r',json_extract(driver.loops,'$.{prefix}r')+1)"
            );
            let jump = self.action("0".into());
            self.code[jump].next = restart.to_string();
            let end = self.code.len();
            let query = dml::correlate(&format!("EXISTS(SELECT 1 FROM {prefix}a)"), self.schemas);
            self.code[guard].next = format!("CASE WHEN {query} THEN {again} ELSE {end} END");
            self.clear(bound, &bound.shape.each.as_ref().unwrap().clear)?;
        }
        let active = format!(
            "EXISTS(SELECT 1 FROM runs WHERE run={clause}) OR EXISTS(SELECT 1 FROM roots WHERE run={clause})"
        );
        self.code[initial].next = format!(
            "CASE WHEN {active} THEN {} ELSE {} END",
            initial + 1,
            self.code.len()
        );
        self.scopes.push(Scope {
            start: initial,
            end: self.code.len(),
            kind: "clause",
            statement: clause,
        });
        Ok(())
    }
    fn render(mut self) -> Result<String, String> {
        let halt = self.action("0".into());
        self.code[halt].next = "-1".into();
        let mut at = 0;
        while at < self.pending.len() {
            let (bound, index) = self.pending[at];
            at += 1;
            let start = self.code.len();
            self.blocks.insert(
                (bound.instance() as *const Instance as usize, index),
                self.code.len(),
            );
            self.steps(bound, &bound.shape.blocks[index as usize].steps)?;
            let ret = self.action("0".into());
            self.code[ret].next = "CAST(json_extract(driver.stack,'$[#-1]') AS INTEGER)".into();
            self.code[ret].stack = "json_remove(driver.stack,'$[#-1]')".into();
            let source = self
                .plan
                .clauses
                .iter()
                .position(|(instance, _)| std::ptr::eq(instance, bound.instance()))
                .map(|id| ("clause", id))
                .or_else(|| {
                    self.plan
                        .stages
                        .iter()
                        .position(|(_, instance)| std::ptr::eq(instance, bound.instance()))
                        .map(|id| ("stage", id))
                })
                .unwrap_or(("program", self.phase));
            self.scopes.push(Scope {
                start,
                end: self.code.len(),
                kind: source.0,
                statement: source.1,
            });
        }
        for &(call, key) in &self.calls {
            self.code[call].next = self.blocks[&key].to_string();
        }
        let end = self.code.len();
        let forward = format!("CASE WHEN pc+1 >= {end} THEN -1 ELSE pc+1 END");
        for (id, instruction) in self.code.iter_mut().enumerate() {
            if instruction.next == end.to_string() {
                instruction.next = "-1".into();
            } else if instruction.next == (id + 1).to_string() {
                instruction.next = forward.clone();
            }
        }
        let cases = |f: fn(&Instruction) -> &str, default: &str| {
            let values = self
                .code
                .iter()
                .enumerate()
                .filter_map(|(i, op)| {
                    let value = f(op);
                    (value != default).then_some((i, value))
                })
                .collect::<Vec<_>>();
            dispatch(
                &values,
                default,
                (usize::BITS - (end - 1).leading_zeros()).saturating_sub(1) as usize,
            )
        };
        static PROGRAM: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        self.phase = PROGRAM.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        profile::definitions(self.phase, &self.code, &self.scopes)?;
        Ok(format!(
            "WITH RECURSIVE driver(pc,tick,token,loops,stack) AS (VALUES(0,0,0,json('{{}}'),json('[]')) UNION ALL SELECT {},tick+1,CASE WHEN p_position({},pc,tick,token) THEN {} ELSE 0 END,{},{} FROM driver WHERE pc>=0 AND token IS NOT NULL), ready AS MATERIALIZED (SELECT CASE WHEN p_position({},-1,tick,token) THEN token END AS token FROM driver WHERE pc=-1 LIMIT 1) SELECT r.clause,r.file,r.node,r.payload,r.round FROM ready CROSS JOIN p_results r WHERE ready.token IS NOT NULL",
            cases(|i| &i.next, &forward),
            self.phase,
            cases(|i| &i.effect, "0"),
            cases(|i| &i.loops, "driver.loops"),
            cases(|i| &i.stack, "driver.stack"),
            self.phase
        ))
    }
}

pub(super) fn compile(plan: &Plan) -> Result<Option<Program>, String> {
    let clauses: Vec<_> = (0..plan.clauses.len())
        .filter(|&c| !plan.fused.as_ref().is_some_and(|f| f.contains(c)))
        .collect();
    if clauses.is_empty() {
        return Ok(None);
    }
    let mut schemas = BTreeMap::new();
    for table in &plan.tables {
        let declaration = match &table.make {
            Make::Frame(declaration) => declaration.clone(),
            Make::BTree(create) => {
                let open = create.find('(').ok_or("program CREATE")?;
                let close = create
                    .split(';')
                    .next()
                    .and_then(|first| first.rfind(')'))
                    .ok_or("program CREATE closing")?;
                format!("{}({})", table.name, &create[open + 1..close])
            }
        };
        schemas.insert(table.name.clone(), Schema::of(declaration)?);
    }
    schemas.insert(
        "p_results".into(),
        Schema::of(
            "p_results(clause INTEGER,file INTEGER,node INTEGER,payload,round INTEGER)".into(),
        )?,
    );
    schemas.insert(
        "p_stage_ready".into(),
        Schema::of("p_stage_ready(stage INTEGER PRIMARY KEY)".into())?,
    );
    let mut queries = Vec::new();
    let mut serial = 0;
    let mut stages = BTreeSet::new();
    // Small fixed number of physical programs. A program contains all control-flow rounds of
    // its clauses; SQL execution count never depends on their data-dependent iteration count.
    let available = 18usize
        .checked_sub(plan.fused.as_ref().map_or(0, |f| f.plan.queries.len()))
        .filter(|&n| n > 0)
        .ok_or("SQL program exceeds the whole-job phase allocation")?;
    for chunk in clauses.chunks(clauses.len().div_ceil(available).max(8)) {
        let mut builder = Builder {
            plan,
            schemas: &mut schemas,
            code: Vec::new(),
            stages,
            serial,
            blocks: HashMap::new(),
            pending: Vec::new(),
            calls: Vec::new(),
            phase: queries.len(),
            free: HashMap::new(),
            scopes: Vec::new(),
        };
        builder.action("p_clear(driver.tick,'p_results')".into());
        for &clause in chunk {
            for &stage in &plan.prepared(plan.stages.len() + clause).shape.stages {
                if plan.sole[stage].is_none() {
                    builder.stage(stage)?;
                }
            }
        }
        for &clause in chunk {
            builder.clause(clause)?;
        }
        // Rendering appends each shared block once; it can discover more nested blocks.
        let compiled_stages = builder.stages.clone();
        let query = builder.render()?;
        // Helper names are unique across physical phases, including out-of-line blocks.
        serial = schemas
            .keys()
            .filter_map(|name| {
                name.strip_prefix("p_frontier_")
                    .or_else(|| name.strip_prefix("p_cte_"))
                    .and_then(|n| n.parse::<usize>().ok())
            })
            .max()
            .map_or(serial, |n| n + 1);
        stages = compiled_stages;
        queries.push(query);
    }
    Ok(Some(Program {
        queries,
        schemas,
        clauses,
    }))
}
// Dispatch on bits, not thousands of distinct PC constants. SQLite factors comparison
// constants through a linear expression list during preparation. Distinct PC literals make
// that work quadratic; bit masks and the eight leaf labels reuse a small constant set.
fn dispatch(values: &[(usize, &str)], default: &str, bit: usize) -> String {
    if values.is_empty() {
        return default.into();
    }
    if bit <= 2 {
        return format!(
            "CASE (pc & 7) {} ELSE {default} END",
            values
                .iter()
                .map(|(id, sql)| format!("WHEN {} THEN {sql}", id & 7))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    let mask = 1usize << bit;
    let mid = values.partition_point(|(id, _)| id & mask == 0);
    format!(
        "CASE (pc & {mask}) WHEN 0 THEN {} ELSE {} END",
        dispatch(&values[..mid], default, bit - 1),
        dispatch(&values[mid..], default, bit - 1)
    )
}

impl Program {
    pub fn table_count(&self) -> usize {
        self.schemas.len()
    }
    pub fn register(&self, db: &Connection, frames: &Arc<FrameStore>) -> Result<(), SqlError> {
        storage::register(db, frames, &self.schemas)
    }
    pub fn clear(&self, frames: &FrameStore) {
        for name in self.schemas.keys() {
            frames.clear(name);
        }
    }
    pub fn contains(&self, clause: usize) -> bool {
        self.clauses.binary_search(&clause).is_ok()
    }
}
