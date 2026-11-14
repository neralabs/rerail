//! SQL programs use the executor's shared host pool and its ordinary scoreboard.
//! Each program owns its connection and declares whether it has a SQL form.
//! Physical SQL planning, snapshot binding and statement budgets belong to the caller.
use crate::exec::Backend;
use crate::kernel::{Kernel, Output, ProgramResult};
use crate::lanes::Lane;

#[derive(Default)]
pub struct SqlBackend;

impl Backend for SqlBackend {
    fn lane(&self) -> Lane {
        Lane::Sql
    }

    fn can(&self, kernel: &Kernel) -> bool {
        matches!(kernel, Kernel::Program(p) if p.0.per_unit(Lane::Sql).is_some())
    }

    fn submit(&self, kernels: &[&Kernel]) -> Result<Vec<Output>, String> {
        kernels
            .iter()
            .map(|kernel| {
                let Kernel::Program(p) = kernel else {
                    return Err(format!("{} has no SQL form", kernel.form().name()));
                };
                if !self.can(kernel) {
                    return Err(format!("program {} has no SQL form", p.0.name()));
                }
                p.0.run(Lane::Sql)
                    .map(|value| Output::Program(ProgramResult(value)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::CpuBackend;
    use crate::kernel::{Program, ProgramKernel, ProgramOutput};
    use crate::{Batch, Executor, Launch};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Default)]
    struct State {
        active: AtomicUsize,
        peak: AtomicUsize,
        entered: AtomicUsize,
    }
    struct Query {
        id: usize,
        state: Arc<State>,
        dual: bool,
        fail: bool,
    }
    impl Program for Query {
        fn name(&self) -> &'static str {
            "SQL test query"
        }
        fn units(&self) -> f64 {
            1.0
        }
        fn per_unit(&self, lane: Lane) -> Option<f64> {
            match lane {
                Lane::Sql => Some(1e-4),
                Lane::Cpu if self.dual => Some(1e-6),
                _ => None,
            }
        }
        fn run(&self, lane: Lane) -> Result<ProgramOutput, String> {
            if self.fail {
                return Err("SQL query failed".into());
            }
            assert!(
                rayon::current_thread_index().is_some(),
                "SQL work must use the shared pool"
            );
            let active = self.state.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.state.peak.fetch_max(active, Ordering::SeqCst);
            let order = self.state.entered.fetch_add(1, Ordering::SeqCst);
            if !self.dual && order < 2 && crate::exec::threads() > 1 {
                let start = std::time::Instant::now();
                while self.state.peak.load(Ordering::SeqCst) < 2 && start.elapsed().as_secs() < 3 {
                    std::thread::yield_now();
                }
            }
            self.state.active.fetch_sub(1, Ordering::SeqCst);
            Ok(Arc::new((self.id, lane)))
        }
    }

    #[test]
    fn sql_only_programs_run_concurrently_and_use_the_shared_scoreboard() {
        let executor = Executor::with(vec![Arc::new(CpuBackend::default()), Arc::new(SqlBackend)]);
        let state = Arc::new(State::default());
        let batch = Batch(
            (0..8)
                .map(|id| {
                    Launch::new(Kernel::Program(ProgramKernel(Arc::new(Query {
                        id,
                        state: state.clone(),
                        dual: false,
                        fail: false,
                    }))))
                })
                .collect(),
        );
        let (done, report) = executor.run(batch).unwrap();
        for (id, item) in done.iter().enumerate() {
            assert_eq!(item.lane, Lane::Sql);
            let Output::Program(value) = &item.out else {
                panic!("SQL program output")
            };
            assert_eq!(
                value.0.downcast_ref::<(usize, Lane)>(),
                Some(&(id, Lane::Sql))
            );
        }
        assert_eq!(report.launches[Lane::Sql.index()], 8);
        let score = executor.score();
        let sql = &score.units[Lane::Sql.index()];
        assert_eq!(
            (sql.jobs, sql.failed, sql.queued, sql.running),
            (8, 0, 0, 0)
        );
        assert_eq!(score.units[Lane::Cpu.index()].jobs, 0);
        assert_eq!(sql.slots, crate::exec::threads());
        assert!(state.peak.load(Ordering::SeqCst) <= sql.slots);
        if sql.slots > 1 {
            assert!(state.peak.load(Ordering::SeqCst) >= 2);
        }
    }

    #[test]
    fn sql_pinning_prevents_other_executors_and_failures_are_scored() {
        let executor = Executor::with(vec![Arc::new(CpuBackend::default()), Arc::new(SqlBackend)]);
        let query = |fail| {
            Kernel::Program(ProgramKernel(Arc::new(Query {
                id: 0,
                state: Arc::new(State::default()),
                dual: true,
                fail,
            })))
        };
        let (done, report) = executor
            .run(Batch(vec![Launch::on(query(false), Lane::Sql)]))
            .unwrap();
        assert_eq!(done[0].lane, Lane::Sql);
        assert_eq!(report.cpu_freed, 0.0, "SQL uses host CPU");
        let error = executor
            .run(Batch(vec![Launch::on(query(true), Lane::Sql)]))
            .unwrap_err();
        assert!(error.contains("SQL query failed"), "{error}");
        assert_eq!(executor.score().units[Lane::Sql.index()].failed, 1);
        assert_eq!(executor.score().units[Lane::Cpu.index()].jobs, 0);
    }

    #[test]
    fn sql_mode_refuses_missing_forms_and_scores_owner_failures() {
        let executor = Executor::sql_only();
        assert_eq!(executor.lanes(), vec![Lane::Sql]);
        let non_sql = Kernel::Histogram {
            off: vec![0, 1].into(),
            codes: vec![0].into(),
            width: 1,
        };
        assert!(executor.run(Batch(vec![Launch::new(non_sql)])).is_err());
        let result: Result<(), &str> =
            executor.local(Lane::Sql, "SQL owner", 0.0, || Err("owner error"));
        assert_eq!(result, Err("owner error"));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            executor.local(Lane::Sql, "SQL owner", 0.0, || -> Result<(), ()> {
                panic!("owner panic")
            })
        }));
        assert!(result.is_err());
        let unit = &executor.score().units[Lane::Sql.index()];
        assert_eq!((unit.jobs, unit.failed, unit.running), (2, 2, 0));
    }
}
