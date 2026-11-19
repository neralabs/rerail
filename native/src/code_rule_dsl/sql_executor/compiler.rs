//! Compile one program while keeping logical analysis separate from execution.
use super::*;
impl CodeRuleSqlExecutor {
    /// Lower and emit a program. Its SQL producers' steps are added last; their relations are
    /// returned beside the program, each with the session inputs its query reads.
    pub fn compile(
        &mut self,
        program: Arc<Version>,
        dialect: &str,
    ) -> Result<(plan::emit::Emitted, BTreeMap<String, Vec<String>>)> {
        let (program, producers) = producers::take(program)?;
        let _scope = profiling::scope(&self.db, "compile");
        for name in &program.input_names() {
            if let Some((arity, types)) = self.input_shapes.get(name)
                && let Some(cols) = program.columns_of(name)
            {
                if program::logical_arity(&*program, name) != *arity {
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
        // The file front end includes all of core.rr. An extension needs bindings only for
        // inputs its retained rules or outputs actually read, not every possible core family.
        let read: BTreeSet<String> = program
            .rules()
            .flat_map(|r| r.body().filter_map(|l| l.atom()).map(|a| program.rel_name(a.rel()).to_owned()).collect::<Vec<_>>())
            .chain(program.output_names())
            .collect();
        let inputs = program.input_names();
        let program = if inputs.iter().all(|name| read.contains(name)) {
            program
        } else {
            let mut b = Builder::on(&program);
            for name in inputs.iter().filter(|name| !read.contains(*name)) {
                b.fields().undeclare(name);
                b.fields().drop_facts(name);
            }
            b.fields().set_inputs(&inputs.iter().filter(|name| read.contains(*name)).collect::<Vec<_>>());
            b.finish().map_err(error)?.0
        };
        let lattices = ProgramFields::lattice(&*program);
        let mut lattice_operations = BTreeMap::new();
        for (name, lattice) in &lattices {
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
        // Each relation of no columns, or lattice of no keys, gets its presence column (`program::physical`'s), and
        // each `top` is its lattice's value (`program::tops`'): the version normalized as the SQL kernel reads it.
        let given: BTreeMap<String, i64> = lattices
            .keys()
            .map(|name| (name.clone(), lattice_operations.get(name).map_or(crate::code_rule_dsl::logic::native::TOP, |ops| ops.top)))
            .collect();
        let (program, _) = crate::code_rule_dsl::logic::program_image::passes::normalize_with_tops(&program, given).map_err(error)?;
        // The lattices' value columns after the presence column, for the transfers.
        let tops: BTreeMap<String, (usize, i64)> = ProgramFields::lattice(&*program)
            .iter()
            .map(|(name, l)| (name.clone(), (l.keys, lattice_operations.get(name).map_or(crate::code_rule_dsl::logic::native::TOP, |ops| ops.top))))
            .collect();
        let dialect = crate::code_rule_dsl::language::dialect_by_label(dialect)
            .ok_or_else(|| error(format!("unknown dialect: {dialect}")))?;
        let lowered = plan::lower::lowered(Arc::clone(&program), dialect, &mut |names| {
            self.store
                .intern_atoms(names)
                .map_err(|e| plan::Error::Plan(e.to_string()))
        })
        .map_err(error)?;
        let lowered_inputs = lowered.version.input_names();
        let mut batch = HashMap::new();
        let has_wave_stats: bool = if lowered_inputs.iter().all(|name| {
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
        for name in &lowered_inputs {
            if !identifier(name) || reserved(name) {
                return Err(error(format!("invalid input relation: {name}")));
            }
            let statement = self
                .db
                .prepare(&format!("SELECT * FROM {}", quote(name)))
                .map_err(error)?;
            let columns: Vec<&str> = statement.column_names().collect();
            if columns.len() != lowered.version.columns_of(name).map_or(0, |c| c.len()) {
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
        let key = crate::code_rule_dsl::prof::lower_hex(&sha2::Sha256::digest(format!("{:016x}:{}:{dialect:?}", program.digest(), program.size())));
        lattices::transfers(&mut emitted, &tops);
        emitted.for_session(&key[..16], &self.inputs.keys().cloned().collect());
        walks::optimize(&mut emitted);
        emitted.program.lattice_operations = lattice_operations;
        let other = producers::steps(&mut self.db, &mut emitted, &program, &producers)?;
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
