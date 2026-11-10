//! Compile one program while keeping logical analysis separate from execution.
use super::*;
impl CodeRuleSqlExecutor {
    /// Lower and emit a program. Its SQL producers' steps are added last; their relations are
    /// returned beside the program, each with the session inputs its query reads.
    pub(super) fn compile(
        &mut self,
        mut program: front::Program,
        dialect: &str,
    ) -> Result<(plan::emit::Emitted, BTreeMap<String, Vec<String>>)> {
        let producers = producers::take(&mut program)?;
        let _scope = profiling::scope(&self.db, "compile");
        for name in &program.inputs {
            if let Some((arity, types)) = self.input_shapes.get(name)
                && let Some(cols) = program.decls.get(name)
            {
                if program::logical_arity(&program, name) != *arity {
                    return Err(error(format!(
                        "{name}: input logical arity differs from its declaration"
                    )));
                }
                if let Some(types) = types {
                    for ((_, declared), actual) in cols.iter().zip(types) {
                        let actual = match actual.as_str() {
                            "number" => "int",
                            "symbol" => "sym",
                            "loc" => "node",
                            other => other,
                        };
                        if actual != declared {
                            return Err(error(format!(
                                "{name}: bound {actual} column differs from declared {declared}"
                            )));
                        }
                    }
                }
            }
        }
        program::physical(&mut program);
        // The file front end includes all of core.rr. An extension needs bindings only for
        // inputs its retained rules or outputs actually read, not every possible core family.
        let read: BTreeSet<String> = program
            .rules
            .iter()
            .flat_map(|r| {
                r.body
                    .iter()
                    .filter_map(front::Literal::atom)
                    .map(|a| a.rel.clone())
            })
            .chain(program.outputs.iter().cloned())
            .collect();
        let unused: Vec<String> = program
            .inputs
            .iter()
            .filter(|name| !read.contains(*name))
            .cloned()
            .collect();
        for name in unused {
            program.inputs.retain(|input| *input != name);
            program.decls.remove(&name);
            program.facts.remove(&name);
        }
        let mut lattice_operations = BTreeMap::new();
        for (name, lattice) in &program.lattice {
            if lattice.lattice() == "flat" {
                continue;
            }
            let mut ops = lattices::specification(lattice.lattice())
                .or_else(|| self.lattice_extensions.get(lattice.lattice()).cloned())
                .or_else(|| lattices::registered(&self.db, lattice.lattice()).ok())
                .ok_or_else(|| {
                    error(format!(
                        "{name}: register SQL operations for lattice {}",
                        lattice.lattice()
                    ))
                })?;
            ops.after = lattice.widen;
            if ops.after.is_some() && ops.widen.is_none() {
                return Err(error(format!(
                    "{name}: this lattice needs a registered widening function"
                )));
            }
            lattice_operations.insert(name.clone(), ops);
        }
        let tops = program
            .lattice
            .iter()
            .map(|(name, l)| {
                (
                    name.clone(),
                    (
                        l.keys,
                        lattice_operations
                            .get(name)
                            .map_or(crate::code_rule_dsl::logic::native::TOP, |ops| ops.top),
                    ),
                )
            })
            .collect();
        program::tops(&mut program, &tops)?;
        let dialect = crate::code_rule_dsl::language::dialect_by_label(dialect)
            .ok_or_else(|| error(format!("unknown dialect: {dialect}")))?;
        let lowered = plan::lower::lower(&program, dialect, &mut |names| {
            self.store
                .intern_atoms(names)
                .map_err(|e| plan::Error::Plan(e.to_string()))
        })
        .map_err(error)?;
        let mut batch = HashMap::new();
        let has_wave_stats: bool = if lowered.program.inputs.iter().all(|name| {
            self.inputs.contains_key(name)
                || self.executions.iter().any(|e| e.counts.contains_key(name))
        }) {
            false
        } else if self.statistics.snapshot().is_some() {
            true
        } else {
            self.db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM temp.sqlite_schema WHERE name = 'wave_stats')",
                    [],
                    |r| r.get(0),
                )
                .map_err(error)?
        };
        for name in &lowered.program.inputs {
            if !identifier(name) || reserved(name) {
                return Err(error(format!("invalid input relation: {name}")));
            }
            let statement = self
                .db
                .prepare(&format!("SELECT * FROM {}", quote(name)))
                .map_err(error)?;
            let columns: Vec<&str> = statement.column_names().collect();
            if columns.len() != lowered.program.decls[name].len() {
                return Err(error(format!(
                    "{name}: input arity differs from its declaration"
                )));
            }
            let source = select::Source::positional(name, &columns);
            let rows = match self.inputs.get(name) {
                Some(source) => source.estimated_len() as f64,
                // Planning reads recorded counts, never rescans an input table. An arbitrary
                // SQL table with no recorded count gets an estimate; this changes plans only.
                None if self.executions.iter().any(|e| e.counts.contains_key(name)) => {
                    self.executions
                        .iter()
                        .rev()
                        .find_map(|e| e.counts.get(name))
                        .copied()
                        .unwrap() as f64
                }
                None if has_wave_stats => self
                    .db
                    .query_row(
                        "SELECT rows FROM wave_stats WHERE tbl = ?1",
                        [name],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()
                    .map_err(error)?
                    .map_or(1_000.0, |n| n as f64),
                None => 1_000.0,
            };
            batch.insert(name.clone(), (source, rows));
        }
        let mut emitted =
            plan::emit::emit(&lowered, &self.db, "main", "sql-extension", None, &batch)
                .map_err(error)?;
        use sha2::Digest;
        let key = crate::lower_hex(&sha2::Sha256::digest(format!("{program:?}:{dialect:?}")));
        lattices::transfers(&mut emitted, &tops);
        emitted.for_session(&key[..16], &self.inputs.keys().cloned().collect());
        walks::optimize(&mut emitted);
        emitted.program.lattice_operations = lattice_operations;
        let other = producers::steps(self.db.connection_mut(), &mut emitted, &program, &producers)?;
        // A table a query reads that is a session input is watched under the input's own name.
        let inputs = |name: &String| {
            self.inputs
                .keys()
                .find(|i| i.eq_ignore_ascii_case(name))
                .cloned()
        };
        let producers = other
            .into_iter()
            .map(|(rel, reads)| (rel, reads.iter().filter_map(inputs).collect()))
            .collect();
        Ok((emitted, producers))
    }
}
