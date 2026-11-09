//! Headless verification controls. These methods do not alter the Node API.
use super::*;

impl CodeRuleSqlExecutor {
    #[doc(hidden)]
    pub fn use_cold_oracle(&mut self) {
        self.cold_oracle = true;
    }

    #[doc(hidden)]
    pub fn verification_files(&self) -> Option<Value> {
        self.documents
            .as_ref()
            .map(documents::Documents::verification_files)
    }

    /// The bound inputs read by the rule program, with their physical columns.
    #[doc(hidden)]
    pub fn verification_inputs(&self) -> BTreeMap<String, Vec<String>> {
        self.executions
            .iter()
            .flat_map(|e| &e.inputs)
            .filter_map(|name| self.inputs.get(name).map(|s| (name.clone(), s.columns())))
            .collect()
    }

    /// Includes projection views, which ordinary wave inventories omit.
    #[doc(hidden)]
    pub fn verification_relations(&self) -> Result<Vec<(String, usize)>> {
        let mut relations = BTreeMap::new();
        for run in &self.executions {
            relations.extend(run.emitted.program.relations.iter().cloned());
            for (name, _) in &run.emitted.views {
                let width = self
                    .db
                    .prepare(&format!("SELECT * FROM {} LIMIT 0", quote(name)))
                    .map_err(error)?
                    .column_count();
                relations.insert(name.clone(), width);
            }
        }
        Ok(relations.into_iter().collect())
    }

    /// Semantic column domains, including direct copies of core document keys.
    #[doc(hidden)]
    pub fn verification_types(&self) -> BTreeMap<String, Vec<String>> {
        let mut types: BTreeMap<_, _> = self
            .executions
            .iter()
            .flat_map(|e| {
                e.emitted
                    .value_types
                    .iter()
                    .map(|(r, c)| (r.clone(), c.clone()))
            })
            .collect();
        for run in &self.executions {
            for (name, columns) in &run.program.decls {
                types
                    .entry(name.clone())
                    .or_insert_with(|| columns.iter().map(|(_, t)| t.clone()).collect());
            }
        }
        for (name, (_, bound)) in &self.input_shapes {
            if let Some(bound) = bound {
                types.entry(name.clone()).or_insert_with(|| bound.clone());
            }
        }
        if self.documents.is_none() {
            return types;
        }
        let mut domains = BTreeMap::new();
        for (name, source) in &self.inputs {
            if self.mutable_inputs.contains(name) {
                continue;
            }
            for (column, field) in source.columns().iter().enumerate() {
                let domain = if field == "d" {
                    Some("document")
                } else if matches!(
                    (name.as_str(), column),
                    ("node", 2) | ("kind", 1) | ("field", 1)
                ) {
                    Some("int")
                } else {
                    None
                };
                if let Some(domain) = domain {
                    domains.insert((name.clone(), column), domain);
                }
            }
        }
        loop {
            let before = domains.len();
            for run in &self.executions {
                let program = &run.program;
                let heads = program
                    .rules
                    .iter()
                    .map(|r| r.head.rel.clone())
                    .collect::<BTreeSet<_>>();
                for rel in heads {
                    if program.facts.get(&rel).is_some_and(|r| !r.is_empty()) {
                        continue;
                    }
                    let rules = program
                        .rules
                        .iter()
                        .filter(|r| r.head.rel == rel)
                        .collect::<Vec<_>>();
                    for column in 0..rules[0].head.args.len() {
                        let copied = rules
                            .iter()
                            .map(|r| {
                                let front::Term::Var(head) = &r.head.args[column] else {
                                    return None;
                                };
                                r.body
                                    .iter()
                                    .filter_map(front::Literal::atom)
                                    .filter(|a| !a.neg)
                                    .find_map(|a| {
                                        a.args.iter().enumerate().find_map(|(i, t)| {
                                            matches!(t,front::Term::Var(v) if v==head)
                                                .then(|| domains.get(&(a.rel.clone(), i)).copied())
                                                .flatten()
                                        })
                                    })
                            })
                            .collect::<Option<Vec<_>>>();
                        if let Some(copied) = copied.filter(|v| v.iter().all(|d| *d == v[0])) {
                            domains.insert((rel.clone(), column), copied[0]);
                        }
                    }
                }
                // Private projection views preserve the domain of directly copied columns.
                for (rel, q) in &run.emitted.views {
                    for (column, (expr, _)) in q.body.columns.iter().enumerate() {
                        let super::super::logic::sql::sql_ast::SqlExpression::Qualified {
                            alias,
                            column: field,
                        } = expr
                        else {
                            continue;
                        };
                        let Some(source) = q
                            .body
                            .from
                            .iter()
                            .find(|f| f.alias == *alias)
                            .and_then(|f| f.relation.program_relation())
                        else {
                            continue;
                        };
                        let index = self
                            .inputs
                            .get(source)
                            .and_then(|s| s.columns().iter().position(|c| c == field))
                            .or_else(|| {
                                field
                                    .strip_prefix('c')
                                    .and_then(|n| n.parse::<usize>().ok())
                            })
                            .or_else(|| {
                                program
                                    .decls
                                    .get(source)
                                    .and_then(|c| c.iter().position(|(n, _)| n == field))
                            });
                        if let Some(domain) =
                            index.and_then(|i| domains.get(&(source.to_owned(), i)).copied())
                        {
                            domains.insert((rel.clone(), column), domain);
                        }
                    }
                }
            }
            if domains.len() == before {
                break;
            }
        }
        for ((rel, column), domain) in domains {
            if let Some(ty) = types.get_mut(&rel).and_then(|c| c.get_mut(column)) {
                *ty = domain.into();
            }
        }
        for run in &self.executions {
            for (rel, keys, _) in &run.emitted.program.lattices {
                if run
                    .emitted
                    .program
                    .lattice_operations
                    .get(rel)
                    .is_some_and(|o| o.after.is_some() && o.widen.is_some())
                {
                    if let Some(columns) = types.get(rel) {
                        let columns = columns
                            .iter()
                            .take(*keys)
                            .cloned()
                            .chain(["int".into()])
                            .collect();
                        types.insert(format!("__sql_changes_{rel}"), columns);
                    }
                }
            }
        }
        types
    }

    /// Store-local symbol and document ids become exact semantic cells across cold stores.
    #[doc(hidden)]
    pub fn verification_cell(&self, ty: &str, value: Option<i64>) -> Result<Value> {
        let Some(value) = value else {
            return Ok(Value::Null);
        };
        let atom = |n: i64| -> Result<String> {
            self.store
                .atom_value(u32::try_from(n).map_err(error)?)
                .map_err(error)?
                .ok_or_else(|| error("verification references an unknown symbol"))
        };
        Ok(match ty {
            "sym" | "symbol" => json!({"symbol":atom(value)?}),
            "document" => json!({"document":atom(value)?}),
            "node" | "loc" if value >= 0 => {
                json!({"node":[atom(value >> 32)?,(value as u32).to_string()]})
            }
            _ => json!(value.to_string()),
        })
    }

    #[doc(hidden)]
    pub fn verification_rows(&self, sql: &str) -> Result<Vec<Vec<Option<i64>>>> {
        self.profiled_read("verification snapshot", |this| {
            let mut statement = this.db.prepare(sql).map_err(error)?;
            if !statement.readonly() || statement.column_count() == 0 {
                return Err(error("verification requires a read-only row query"));
            }
            let width = statement.column_count();
            statement
                .query_map([], |r| {
                    (0..width)
                        .map(|c| r.get(c))
                        .collect::<rusqlite::Result<Vec<Option<i64>>>>()
                })
                .map_err(error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(error)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_and_symbol_cells_compare_across_different_store_local_ids() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let mut one =
            CodeRuleSqlExecutor::new(a.path().join("s.sqlite").to_string_lossy().into()).unwrap();
        let mut two =
            CodeRuleSqlExecutor::new(b.path().join("s.sqlite").to_string_lossy().into()).unwrap();
        let file =
            |path: &str, source: &str| json!({"path":path,"language":"python","source":source});
        let first = file("a.py", "alpha = 1\n");
        let second = file("b.py", "beta = 2\n");
        two.bind_snapshot(json!({"files":[file("z.py","reserved = 9\n")]}).to_string())
            .unwrap();
        let p = ".decl doc(d:number,path:symbol,module:symbol)\n.decl name(n:node,s:symbol)\n.decl node(d:number,n:node,k:symbol,start:number,end:number)\n.decl kind(n:node,k:symbol)\n.input doc,name,node,kind\n.decl seen(d:number,p:symbol)\nseen(d,p) :- doc(d,p,_).\n.decl names(n:node,s:symbol)\nnames(n,s) :- name(n,s).\n.decl kinds(k:symbol)\nkinds(k) :- kind(_,k).\n.output seen,names,kinds\n";
        for e in [&mut one, &mut two] {
            e.bind_snapshot(json!({"files":[first,second]}).to_string())
                .unwrap();
            e.run(p.into(), "python".into(), None).unwrap();
        }
        for relation in ["doc", "seen", "names", "kind", "kinds"] {
            let read = |e: &CodeRuleSqlExecutor| {
                let types = e.verification_types();
                let mut rows = e
                    .verification_rows(&format!("SELECT * FROM {relation}"))
                    .unwrap()
                    .into_iter()
                    .map(|r| {
                        r.into_iter()
                            .enumerate()
                            .map(|(c, v)| e.verification_cell(&types[relation][c], v).unwrap())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                rows.sort_by_key(|r| serde_json::to_string(r).unwrap());
                rows
            };
            assert_eq!(read(&one), read(&two), "{relation}");
        }
    }
}
