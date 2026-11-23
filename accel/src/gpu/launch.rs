//! The launch ledger of one evaluation (decision 0601): every compute launch on the device is charged here before it
//! is enqueued, by every submitter alike: the engine's commands (`rerail_metal::lane::Command`), the relational passes
//! (`join::Pass`) and the executor's GPU lane (`GpuBackend::run`). There is no other count: a submission's own, or a
//! guard that opens a fresh evaluation, is not a check.
//!
//! Past the budget is a hard failure, never a refusal to route around: the launch that would pass it, or a launch
//! outside any evaluation, prints the evaluation's account and panics with `exec::OverBudget`, which every catch of a
//! job's panic raises again (`exec::resume_over_budget`, `exec::abort_over_budget`). The budget is a bound on how the
//! device code is written: an evaluation over it is code to restructure.
//!
//! A plan may count its launches before it encodes one (`metal_label_passes::run`); one over the bound is the same
//! failure there.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// 200 since list step 26 (user decision, 2026-11-03): the wave doubled to 158 clauses with fact labels.
pub const KERNEL_LAUNCH_LIMIT: usize = 200;

/// The reruns' launches (user decision, 2026-11-21): a rerun of an evaluation (a run again with more levels, more
/// budget, wider lanes, more rounds) charges a budget apart from the evaluation's own `KERNEL_LAUNCH_LIMIT`, which
/// the evaluations of a run share. Each evaluation adds one rerun's worth (`KERNEL_LAUNCH_LIMIT`) as it begins, so the
/// reruns are one an evaluation on average: one that needs more takes what others did not. Outside a run an
/// evaluation's is its own.
#[derive(Default)]
pub struct Reruns {
    granted: std::sync::atomic::AtomicUsize,
    spent: std::sync::atomic::AtomicUsize,
}

impl Reruns {
    /// One evaluation's share.
    pub fn grant(&self) {
        self.granted.fetch_add(KERNEL_LAUNCH_LIMIT, std::sync::atomic::Ordering::Relaxed);
    }

    /// Takes one launch: false where the shares are spent.
    pub fn take(&self) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        self.spent
            .try_update(Relaxed, Relaxed, |spent| (spent < self.granted.load(Relaxed)).then_some(spent + 1))
            .is_ok()
    }

    /// The launches left.
    pub fn available(&self) -> usize {
        use std::sync::atomic::Ordering::Relaxed;
        self.granted.load(Relaxed).saturating_sub(self.spent.load(Relaxed))
    }
}

thread_local! {
    static EVALUATION: RefCell<Option<Rc<RefCell<Evaluation>>>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct DispatchRecord {
    pub command: u64,
    pub kernel: String,
    pub grid: [u64; 3],
    pub group: [u64; 3],
    pub indirect: bool,
    pub submitted: bool,
    pub completed: bool,
    pub buffers: Vec<(usize, u64)>,
    pub application_buffer_bytes: usize,
    pub peak_application_buffer_bytes: usize,
}

#[derive(Default)]
struct Evaluation {
    /// The launches charged to the evaluation's own budget.
    used: usize,
    /// The run's reruns' budget (`Reruns`), and whether the launches now are a rerun's (`rerun`).
    reruns: std::sync::Arc<Reruns>,
    rerunning: bool,
    records: Vec<DispatchRecord>,
    commands: HashMap<usize, u64>,
    /// The stacked commands charged (`attempt_stacked`): one launch each, whatever its dispatches.
    stacks: std::collections::HashSet<u64>,
    next_command: u64,
    peak_application_buffer_bytes: usize,
}

pub fn record_buffer_allocation(bytes: usize) {
    EVALUATION.with_borrow(|slot| {
        if let Some(evaluation) = slot {
            let mut evaluation = evaluation.borrow_mut();
            evaluation.peak_application_buffer_bytes = evaluation.peak_application_buffer_bytes.max(bytes);
        }
    });
}

/// Nested commands, ranges, replay, and pages share the outer evaluation budget.
pub struct LaunchScope {
    owner: bool,
    _budget: Rc<RefCell<Evaluation>>,
}

impl LaunchScope {
    pub fn enter() -> Self {
        EVALUATION.with_borrow_mut(|slot| {
            let owner = slot.is_none();
            let budget = slot.get_or_insert_with(|| {
                // The run's reruns' budget, which this evaluation adds its share to (decision 0540).
                let reruns = rerail_context::local::<Reruns>();
                reruns.grant();
                Rc::new(RefCell::new(Evaluation { reruns, ..Evaluation::default() }))
            });
            Self { owner, _budget: Rc::clone(budget) }
        })
    }

    pub fn snapshot(&self) -> Vec<DispatchRecord> {
        self._budget.borrow().records.clone()
    }
}

impl Drop for LaunchScope {
    fn drop(&mut self) {
        if self.owner {
            EVALUATION.with_borrow_mut(|slot| *slot = None);
        }
    }
}

/// The hard failure (decision 0601): `why`, with the evaluation's account of its launches by kernel, printed, and
/// raised as `exec::OverBudget`.
fn over_budget(why: String, records: &[DispatchRecord]) -> ! {
    let mut kernels = std::collections::BTreeMap::<&str, usize>::new();
    for record in records {
        *kernels.entry(record.kernel.as_str()).or_default() += 1;
    }
    let account = format!("{why}; its launches by kernel: {kernels:?}");
    eprintln!("rerail: past the launch budget (decision 0601): {account}");
    std::panic::panic_any(crate::exec::OverBudget(account))
}

impl Evaluation {
    /// One launch from the budget its launches now charge: a rerun's from the run's reruns' (decision 0540), else the
    /// evaluation's own. Past it, the hard failure.
    fn charge(&mut self, kernel: &str) {
        if self.rerunning {
            if !self.reruns.take() {
                over_budget(format!("{kernel}: a rerun past the run's reruns' budget"), &self.records);
            }
        } else {
            if self.used == KERNEL_LAUNCH_LIMIT {
                over_budget(format!("{kernel}: the {KERNEL_LAUNCH_LIMIT} launches of its evaluation spent"), &self.records);
            }
            self.used += 1;
        }
    }
}

/// While held, the evaluation's launches are a rerun's (decision 0540): charged to the run's reruns' budget, not the
/// evaluation's own.
pub struct Rerun(Option<Rc<RefCell<Evaluation>>>);

pub fn rerun() -> Rerun {
    Rerun(EVALUATION.with_borrow(|slot| {
        slot.as_ref().map(|evaluation| {
            evaluation.borrow_mut().rerunning = true;
            Rc::clone(evaluation)
        })
    }))
}

impl Drop for Rerun {
    fn drop(&mut self) {
        if let Some(evaluation) = &self.0 {
            evaluation.borrow_mut().rerunning = false;
        }
    }
}

/// The launches a rerun may still take (the run's reruns' budget's); 0 with no evaluation.
pub fn rerun_available() -> usize {
    EVALUATION.with_borrow(|slot| slot.as_ref().map_or(0, |evaluation| evaluation.borrow().reruns.available()))
}

/// The launches the evaluation may still take (its own budget's); 0 with no evaluation.
pub fn available() -> usize {
    EVALUATION.with_borrow(|slot| slot.as_ref().map_or(0, |evaluation| KERNEL_LAUNCH_LIMIT - evaluation.borrow().used))
}

/// The hard failure for a plan that would launch past the budget, raised before it encodes (`why` says what it
/// would take): the same failure as the launch past it, with the evaluation's account so far.
pub fn plan_past_budget(why: String) -> ! {
    let records = EVALUATION.with_borrow(|slot| slot.as_ref().map(|evaluation| evaluation.borrow().records.clone()).unwrap_or_default());
    over_budget(why, &records)
}

/// The submission check, at every commit: a submission outside an evaluation is the hard failure (nothing charged
/// its launches).
pub fn check_submission() {
    if EVALUATION.with_borrow(Option::is_none) {
        over_budget("a submission outside any evaluation".into(), &[]);
    }
}

/// Marks the dispatches of the command known by `pointer` (a submission's key: a `lane::Command`'s, a `join::Pass`'s
/// or the GPU lane's) submitted, or completed.
pub fn mark(pointer: usize, completed: bool) {
    EVALUATION.with_borrow(|slot| {
        if let Some(evaluation) = slot {
            let mut evaluation = evaluation.borrow_mut();
            if let Some(&id) = evaluation.commands.get(&pointer) {
                for record in &mut evaluation.records {
                    if record.command == id {
                        if completed {
                            record.completed = record.submitted;
                        } else {
                            record.submitted = true;
                        }
                    }
                }
                if completed {
                    evaluation.commands.remove(&pointer);
                    evaluation.stacks.remove(&id);
                }
            }
        }
    });
}

/// Charges one dispatch of the command known by `pointer` to the evaluation's budget and records it, before it is
/// enqueued. Past the budget, or with no evaluation, the hard failure.
pub fn attempt(pointer: usize, kernel: &str, grid: [u64; 3], group: [u64; 3], indirect: bool, buffers: Vec<(usize, u64)>) {
    record(pointer, kernel, grid, group, indirect, buffers, false);
}

/// `attempt` for a stack's dispatch (decision 0648): a submission of dependent dispatches of one program, each a
/// level's, charged as one launch, at its first; the rest recorded only.
pub fn attempt_stacked(pointer: usize, kernel: &str, grid: [u64; 3], group: [u64; 3], indirect: bool, buffers: Vec<(usize, u64)>) {
    record(pointer, kernel, grid, group, indirect, buffers, true);
}

fn record(pointer: usize, kernel: &str, grid: [u64; 3], group: [u64; 3], indirect: bool, buffers: Vec<(usize, u64)>, stacked: bool) {
    EVALUATION.with_borrow(|slot| {
        let Some(evaluation) = slot else { over_budget(format!("{kernel}: a launch outside any evaluation"), &[]) };
        let mut evaluation = evaluation.borrow_mut();
        let id = if let Some(&id) = evaluation.commands.get(&pointer) {
            id
        } else {
            let id = evaluation.next_command;
            evaluation.next_command += 1;
            evaluation.commands.insert(pointer, id);
            id
        };
        if !stacked || evaluation.stacks.insert(id) {
            evaluation.charge(kernel);
        }
        let application_buffer_bytes = super::shared::other_reserved();
        evaluation.peak_application_buffer_bytes = evaluation.peak_application_buffer_bytes.max(application_buffer_bytes);
        let peak_application_buffer_bytes = evaluation.peak_application_buffer_bytes;
        evaluation.records.push(DispatchRecord {
            command: id,
            kernel: kernel.to_owned(),
            grid,
            group,
            indirect,
            submitted: false,
            completed: false,
            buffers,
            application_buffer_bytes,
            peak_application_buffer_bytes,
        });
    })
}

/// Charges `made` launches the UKL library made (`rerail_gpu::library`, which says its count once it has enqueued
/// them) of the command known by `pointer`: past the budget, the hard failure, before the command is committed.
pub fn charge_made(pointer: usize, kernel: &str, made: usize) {
    for _ in 0..made {
        attempt(pointer, kernel, [0; 3], [0; 3], false, Vec::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch() {
        attempt(1, "k", [1; 3], [1; 3], false, Vec::new());
    }

    /// A stack's dispatches are one launch (decision 0648): its first charges, the rest are recorded; a stack
    /// committed and completed, the next stack of the same key charges again.
    #[test]
    fn a_stack_is_one_launch() {
        let e = LaunchScope::enter();
        let before = available();
        for _ in 0..KERNEL_LAUNCH_LIMIT + 5 {
            attempt_stacked(7, "k", [1; 3], [1; 3], false, Vec::new());
        }
        assert_eq!(available(), before - 1, "one launch");
        assert_eq!(e.snapshot().len(), KERNEL_LAUNCH_LIMIT + 5, "every dispatch recorded");
        mark(7, false);
        mark(7, true);
        attempt_stacked(7, "k", [1; 3], [1; 3], false, Vec::new());
        assert_eq!(available(), before - 2, "the next stack, another launch");
    }

    /// The account the launch past the budget raises (`exec::OverBudget`), or none when `f` completes.
    fn over(f: impl FnOnce()) -> Option<String> {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            Ok(()) => None,
            Err(p) => Some(p.downcast::<crate::exec::OverBudget>().expect("the launch budget's failure").0),
        }
    }

    #[test]
    fn command_boundaries_and_nested_scopes_do_not_reset_the_limit() {
        let _evaluation = LaunchScope::enter();
        // Commands of up to 9 launches each, the last one partial, use the whole limit.
        for command in 0..KERNEL_LAUNCH_LIMIT.div_ceil(9) {
            let _command = LaunchScope::enter();
            for _ in 0..(KERNEL_LAUNCH_LIMIT - command * 9).min(9) {
                launch();
            }
            check_submission();
        }
        assert_eq!(available(), 0);
        let _replay = LaunchScope::enter();
        let account = over(launch).expect("the launch past the limit fails");
        assert!(account.contains("\"k\": 200"), "its account: {account}");
    }

    /// Decision 0540: a rerun's launches are the reruns' budget's, apart from the evaluation's own; and past either
    /// is the hard failure (0601).
    #[test]
    fn a_rerun_charges_the_reruns_budget_not_the_evaluations() {
        let _evaluation = LaunchScope::enter();
        for _ in 0..KERNEL_LAUNCH_LIMIT {
            launch();
        }
        {
            let _rerun = rerun();
            assert_eq!(rerun_available(), KERNEL_LAUNCH_LIMIT);
            for _ in 0..KERNEL_LAUNCH_LIMIT / 2 {
                launch();
            }
        }
        assert!(over(launch).is_some(), "the evaluation's own budget is spent");
        // A second rerun takes what the first left, and fails past it.
        let _rerun = rerun();
        assert_eq!(rerun_available(), KERNEL_LAUNCH_LIMIT - KERNEL_LAUNCH_LIMIT / 2);
        for _ in 0..KERNEL_LAUNCH_LIMIT - KERNEL_LAUNCH_LIMIT / 2 {
            launch();
        }
        assert!(over(launch).expect("past the reruns' budget").contains("rerun"));
    }

    /// The reruns' budget is the run's: each evaluation adds one rerun's worth, which another may take.
    #[test]
    fn the_reruns_budget_grows_by_an_evaluation_share() {
        let reruns = std::sync::Arc::new(Reruns::default());
        reruns.grant();
        reruns.grant();
        for _ in 0..2 * KERNEL_LAUNCH_LIMIT {
            assert!(reruns.take());
        }
        assert!(!reruns.take());
        assert_eq!(reruns.available(), 0);
    }

    #[test]
    fn a_new_evaluation_gets_a_new_budget() {
        {
            let _evaluation = LaunchScope::enter();
            for _ in 0..KERNEL_LAUNCH_LIMIT {
                launch();
            }
            assert_eq!(available(), 0);
        }
        let _evaluation = LaunchScope::enter();
        assert_eq!(available(), KERNEL_LAUNCH_LIMIT);
        launch();
        check_submission();
    }

    /// A launch or a submission outside any evaluation is counted by none: the hard failure.
    #[test]
    fn a_launch_outside_an_evaluation_is_the_hard_failure() {
        assert!(over(launch).expect("a launch outside").contains("outside"));
        assert!(over(check_submission).expect("a submission outside").contains("outside"));
    }
}
