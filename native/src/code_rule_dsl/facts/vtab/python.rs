//! Semantic facts consume an immutable demand frame, once per SQL cursor.
//! Targets are a scalar JSON array from the query's target relation, not row callbacks.
use super::*;

#[derive(Clone)]
struct Producer {
    binding: Binding,
    kind: &'static str,
    width: usize,
}

pub(super) fn register(db: &Connection, binding: &Binding) -> Result<()> {
    const MODULE: Module<ProducerTable> = Module::eponymous_only_module();
    for (kind, width) in [
        ("qualified", 2),
        ("origins", 2),
        ("assigned", 2),
        ("arguments", 5),
        ("candidates", 4),
        #[cfg(test)]
        ("candidate_spike", 5),
    ] {
        let name = CString::new(format!("rr_batch_python_{kind}")).unwrap();
        db.create_module(
            name.as_c_str(),
            &MODULE,
            Some(Producer {
                binding: binding.clone(),
                kind,
                width,
            }),
        )?;
    }
    Ok(())
}

#[repr(C)]
struct ProducerTable {
    base: rusqlite::ffi::sqlite3_vtab,
    producer: Producer,
}
unsafe impl<'v> VTab<'v> for ProducerTable {
    type Aux = Producer;
    type Cursor = ProducerCursor;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Producer>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        let producer = aux
            .ok_or_else(|| failure("missing semantic batch"))?
            .clone();
        let columns = (0..producer.width)
            .map(|c| {
                format!(
                    "c{c} {}",
                    if (producer.kind == "qualified" && c == 1)
                        || (producer.kind == "arguments" && matches!(c, 2 | 3))
                    {
                        "TEXT"
                    } else {
                        "INTEGER"
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        Ok((
            Cow::Owned(
                CString::new(format!(
                    "CREATE TABLE x({columns}, targets TEXT HIDDEN, frame INTEGER HIDDEN, context TEXT HIDDEN, member TEXT HIDDEN)"
                ))
                .unwrap(),
            ),
            Self {
                base: Default::default(),
                producer,
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let frame = info
            .constraints()
            .enumerate()
            .find(|(_, c)| {
                c.is_usable()
                    && c.column() == self.producer.width as i32 + 1
                    && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            })
            .map(|(i, _)| i);
        let Some(frame) = frame else { return Ok(false) };
        info.constraint_usage(frame).set_argv_index(1);
        info.constraint_usage(frame).set_omit(true);
        let target = info
            .constraints()
            .enumerate()
            .find(|(_, c)| {
                c.is_usable()
                    && c.column() == self.producer.width as i32
                    && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            })
            .map(|(i, _)| i);
        if let Some(t) = target {
            info.constraint_usage(t).set_argv_index(2);
            info.constraint_usage(t).set_omit(true);
        } else if self.producer.kind == "qualified" {
            return Ok(false);
        }
        if self.producer.kind=="candidates" {
            if target.is_none(){return Ok(false);}
            for (offset,arg) in [(2,3),(3,4)] {
                let Some(i)=info.constraints().position(|c|c.is_usable()&&c.column()==self.producer.width as i32+offset&&c.operator()==IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ) else{return Ok(false);};
                info.constraint_usage(i).set_argv_index(arg);info.constraint_usage(i).set_omit(true);
            }
        }
        info.set_idx_num(i32::from(target.is_some()));
        info.set_estimated_rows(100);
        info.set_estimated_cost(100.0);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<ProducerCursor> {
        Ok(ProducerCursor {
            base: Default::default(),
            producer: self.producer.clone(),
            rows: Vec::new(),
            at: 0,
        })
    }
}
#[repr(C)]
struct ProducerCursor {
    base: rusqlite::ffi::sqlite3_vtab_cursor,
    producer: Producer,
    rows: Vec<Vec<Value>>,
    at: usize,
}
unsafe impl VTabCursor for ProducerCursor {
    fn filter(&mut self, index: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.rows.clear();
        self.at = 0;
        #[cfg(test)]
        if self.producer.kind == "candidate_spike" {
            let targets = serde_json::from_str(&args.get::<String>(1)?).map_err(|e| failure(&e.to_string()))?;
            self.rows = self.producer.binding.current().python_batch().ok_or_else(|| failure("missing candidate batch"))?
                .candidate_spike(&targets).map_err(|e| failure(&e))?;
            return Ok(());
        }
        let keys = super::demand::read(args.get::<i64>(0)?)?;
        let targets: std::collections::BTreeSet<String> = if index == 1 {
            serde_json::from_str(&args.get::<String>(1)?).map_err(|e| failure(&e.to_string()))?
        } else {
            Default::default()
        };
        crate::code_rule_dsl::logic::profile::decision("SQL semantic batch", || {
            format!(
                "{}: {} demanded {}, {} targets",
                self.producer.kind,
                keys.len(),
                if self.producer.kind=="candidates" {"documents"}else{"nodes"},
                targets.len()
            )
        });
        if keys.is_empty() {
            return Ok(());
        }
        let batch = self
            .producer
            .binding
            .current()
            .python_batch()
            .ok_or_else(|| failure("semantic producer requires a bound syntax batch"))?;
        self.rows = if self.producer.kind=="candidates" {
            batch.python_candidates(&keys,&targets,&args.get::<String>(2)?,&args.get::<String>(3)?).map_err(|e|failure(&e))?
        }else {batch.python_project(self.producer.kind,&keys,&targets).map_err(|e|failure(&e))?};
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.at += 1;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.at >= self.rows.len()
    }
    fn column(&self, ctx: &mut Context, c: c_int) -> Result<()> {
        ctx.set_result(self.rows[self.at].get(c as usize).unwrap_or(&Value::Null))
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.at as i64 + 1)
    }
}
