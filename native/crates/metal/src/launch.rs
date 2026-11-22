//! Charge every compute dispatch before encoding it. Refuse submission after exhaustion. The engine's commands
//! (`crate::lane`, on every backend) charge each launch here as it is asked for and check the budget at their commit.
use crate::Error;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::launch_budget as budget;
use budget::{LaunchBudget, Reruns};
pub use budget::KERNEL_LAUNCH_LIMIT;

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
    pub encoded: bool,
    pub submitted: bool,
    pub completed: bool,
    pub buffers: Vec<(usize, u64)>,
    pub application_buffer_bytes: usize,
    pub peak_application_buffer_bytes: usize,
}

#[derive(Default)]
struct Evaluation {
    budget: LaunchBudget,
    /// The run's reruns' budget (`Reruns`), and whether the launches now are a rerun's (`rerun`), and one was refused.
    reruns: std::sync::Arc<Reruns>,
    rerunning: bool,
    rerun_refused: bool,
    records: Vec<DispatchRecord>,
    commands: HashMap<usize, u64>,
    next_command: u64,
    peak_application_buffer_bytes: usize,
}

pub fn record_buffer_allocation(bytes: usize) {
    EVALUATION.with_borrow(|slot| {
        if let Some(evaluation) = slot {
            let mut evaluation = evaluation.borrow_mut();
            evaluation.peak_application_buffer_bytes =
                evaluation.peak_application_buffer_bytes.max(bytes);
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
            Self {
                owner,
                _budget: Rc::clone(budget),
            }
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

/// Takes one launch from the evaluation's budget (a rerun's from the run's reruns', `rerun`): false past it, or with
/// no evaluation.
pub fn reserve() -> bool {
    EVALUATION.with_borrow(|slot| slot.as_ref().is_some_and(|budget| budget.borrow_mut().charge()))
}

impl Evaluation {
    /// One launch from the budget its launches now charge: a rerun's from the run's reruns' (decision 0540), else the
    /// evaluation's own. A refusal is recorded where `check_submission` reads it for that budget.
    fn charge(&mut self) -> bool {
        if self.rerunning {
            let taken = self.reruns.take();
            self.rerun_refused |= !taken;
            taken
        } else {
            self.budget.reserve()
        }
    }
}

/// While held, the evaluation's launches are a rerun's (decision 0540): charged to the run's reruns' budget, not the
/// evaluation's own. Its first run's refusal is not the rerun's.
pub struct Rerun(Option<Rc<RefCell<Evaluation>>>);

pub fn rerun() -> Rerun {
    Rerun(EVALUATION.with_borrow(|slot| {
        slot.as_ref().map(|evaluation| {
            let mut e = evaluation.borrow_mut();
            e.rerunning = true;
            e.rerun_refused = false;
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

pub fn check_submission() -> Result<(), Error> {
    EVALUATION.with_borrow(|slot| {
        let budget = slot.as_ref().ok_or(Error::BackendFailure)?;
        let refused = {
            let e = budget.borrow();
            if e.rerunning { e.rerun_refused } else { e.budget.refused() }
        };
        if refused {
            if std::env::var_os("RERAIL_LAUNCHES").is_some() {
                let mut kernels = std::collections::BTreeMap::<String, usize>::new();
                for record in &budget.borrow().records {
                    *kernels.entry(record.kernel.clone()).or_default() += 1;
                }
                eprintln!("REFUSED dispatches {kernels:?}");
            }
            Err(Error::BackendLaunchLimit)
        } else {
            Ok(())
        }
    })
}

/// The host clock Metal's command-buffer times are in (mach absolute time on macOS), in seconds: `rerail_accel`'s.
pub fn host_seconds() -> f64 {
    rerail_accel::gpu::join::host_clock()
}

/// A completed command's host-clock times, in seconds: `kernelStartTime` (the system began scheduling it),
/// `GPUStartTime`, `GPUEndTime`. Metal's command buffers have them; elsewhere zero.
pub fn command_times(event: &rerail_device::device::ComputeEvent) -> [f64; 3] {
    #[cfg(target_os = "macos")]
    {
        event.metal().map_or([0.0; 3], metal_command_times)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = event;
        [0.0; 3]
    }
}

/// `command_times` of a Metal command buffer.
#[cfg(target_os = "macos")]
fn metal_command_times(command: &metal::CommandBufferRef) -> [f64; 3] {
    use metal::foreign_types::ForeignTypeRef;
    unsafe extern "C" {
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn objc_msgSend();
    }
    // SAFETY: each selector takes no arguments and returns a CFTimeInterval (a double).
    let send: unsafe extern "C" fn(*const std::ffi::c_void, *mut std::ffi::c_void) -> f64 =
        unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    let object = command.as_ptr().cast::<std::ffi::c_void>().cast_const();
    unsafe {
        [
            send(object, sel_registerName(c"kernelStartTime".as_ptr())),
            send(object, sel_registerName(c"GPUStartTime".as_ptr())),
            send(object, sel_registerName(c"GPUEndTime".as_ptr())),
        ]
    }
}

/// Marks the dispatches of the command known by `pointer` (a Metal command buffer's address, or a device-layer
/// command's key, `crate::lane`) submitted, or completed.
pub(crate) fn mark(pointer: usize, completed: bool) {
    EVALUATION.with_borrow(|slot| {
        if let Some(evaluation) = slot {
            let mut evaluation = evaluation.borrow_mut();
            if let Some(&id) = evaluation.commands.get(&pointer) {
                for record in &mut evaluation.records {
                    if record.command == id && record.encoded {
                        if completed {
                            record.completed = record.submitted;
                        } else {
                            record.submitted = true;
                        }
                    }
                }
                if completed {
                    evaluation.commands.remove(&pointer);
                }
            }
        }
    });
}

/// Charges one dispatch of the command known by `pointer` to the evaluation's budget and records it: false past
/// the budget (the dispatch is not encoded), or with no evaluation.
pub(crate) fn attempt(pointer: usize, kernel: &str, grid: [u64; 3], group: [u64; 3], indirect: bool, buffers: Vec<(usize, u64)>) -> bool {
    EVALUATION.with_borrow(|slot| {
        let Some(evaluation) = slot else { return false };
        let mut evaluation = evaluation.borrow_mut();
        let id = if let Some(&id) = evaluation.commands.get(&pointer) {
            id
        } else {
            let id = evaluation.next_command;
            evaluation.next_command += 1;
            evaluation.commands.insert(pointer, id);
            id
        };
        let encoded = evaluation.charge();
        let application_buffer_bytes = crate::runtime::reserved();
        evaluation.peak_application_buffer_bytes = evaluation
            .peak_application_buffer_bytes
            .max(application_buffer_bytes);
        let peak_application_buffer_bytes = evaluation.peak_application_buffer_bytes;
        evaluation.records.push(DispatchRecord {
            command: id,
            kernel: kernel.to_owned(),
            grid,
            group,
            indirect,
            encoded,
            submitted: false,
            completed: false,
            buffers,
            application_buffer_bytes,
            peak_application_buffer_bytes,
        });
        encoded
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_boundaries_and_nested_scopes_do_not_reset_the_limit() {
        let _evaluation = LaunchScope::enter();
        // Commands of up to 9 launches each, the last one partial, use the whole limit.
        for command in 0..KERNEL_LAUNCH_LIMIT.div_ceil(9) {
            let _command = LaunchScope::enter();
            for _ in 0..(KERNEL_LAUNCH_LIMIT - command * 9).min(9) {
                assert!(reserve());
            }
            assert_eq!(check_submission(), Ok(()));
        }
        assert!(!reserve());
        assert_eq!(check_submission(), Err(Error::BackendLaunchLimit));
        let _replay = LaunchScope::enter();
        assert!(!reserve());
        assert_eq!(check_submission(), Err(Error::BackendLaunchLimit));
    }

    /// Decision 0540: a rerun's launches are the reruns' budget's, apart from the evaluation's own: an evaluation that
    /// spent its own reruns with one rerun's worth, and its own refusal is not the rerun's.
    #[test]
    fn a_rerun_charges_the_reruns_budget_not_the_evaluations() {
        let _evaluation = LaunchScope::enter();
        for _ in 0..KERNEL_LAUNCH_LIMIT {
            assert!(reserve());
        }
        assert!(!reserve());
        assert_eq!(check_submission(), Err(Error::BackendLaunchLimit));
        {
            let _rerun = rerun();
            assert_eq!(rerun_available(), KERNEL_LAUNCH_LIMIT);
            assert_eq!(check_submission(), Ok(()));
            for _ in 0..KERNEL_LAUNCH_LIMIT / 2 {
                assert!(reserve());
            }
            assert_eq!(check_submission(), Ok(()));
        }
        // A second rerun takes what the first left, and is refused past it.
        let _rerun = rerun();
        assert_eq!(rerun_available(), KERNEL_LAUNCH_LIMIT - KERNEL_LAUNCH_LIMIT / 2);
        for _ in 0..KERNEL_LAUNCH_LIMIT - KERNEL_LAUNCH_LIMIT / 2 {
            assert!(reserve());
        }
        assert!(!reserve());
        assert_eq!(check_submission(), Err(Error::BackendLaunchLimit));
    }

    /// A command's launches (`attempt`, as `lane::Launch` charges them) during a rerun are the reruns' budget's: an
    /// evaluation whose own budget its first runs spent still encodes a rerun's launches, and a rerun past its budget
    /// is refused at submission, never run with launches left out (the wave's labels lost on a third run, 0540).
    #[test]
    fn a_reruns_command_launches_charge_the_reruns_budget() {
        let _evaluation = LaunchScope::enter();
        let launch = || attempt(1, "k", [1; 3], [1; 3], false, Vec::new());
        for _ in 0..KERNEL_LAUNCH_LIMIT {
            assert!(launch());
        }
        assert!(!launch());
        assert_eq!(check_submission(), Err(Error::BackendLaunchLimit));
        let _rerun = rerun();
        for _ in 0..KERNEL_LAUNCH_LIMIT {
            assert!(launch());
        }
        assert_eq!(check_submission(), Ok(()));
        assert!(!launch());
        assert_eq!(check_submission(), Err(Error::BackendLaunchLimit));
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
                assert!(reserve());
            }
            assert!(!reserve());
        }
        let _evaluation = LaunchScope::enter();
        assert!(reserve());
        assert_eq!(check_submission(), Ok(()));
    }

    #[test]
    fn dispatch_and_submission_require_an_evaluation_scope() {
        assert!(!reserve());
        assert_eq!(check_submission(), Err(Error::BackendFailure));
    }

    /// A UKL kernel that counts its runs, and one that sets its word: launched by the engine's commands
    /// (`crate::lane`), on the process's device.
    const MARK: &str = "#include <ukl/backend.h>\nDEFINE_KERNEL2(guard, launch_guard_mark, RWBUFFER, uint32_t, count, \"[1]\", GID0, uint32_t, i, ) { if (i == 0u) ukl_atomic_add(count, 1u); }\n";
    const ZERO: &str = "#include <ukl/backend.h>\nDEFINE_KERNEL2(guard, launch_guard_zero, RWBUFFER, uint32_t, count, \"[1]\", GID0, uint32_t, i, ) { if (i == 0u) count[0] = 1u; }\n";

    fn word(b: &crate::lane::Buffer) -> u32 {
        // SAFETY: a buffer of at least one u32 the device is done with (its command waited for).
        unsafe { b.contents().cast::<u32>().read() }
    }

    #[test]
    fn gpu_launch_guard_counts_all_dispatch_apis_and_refuses_submission() {
        let Ok(device) = crate::lane::device() else { return };
        let _scope = crate::runtime::evaluation_scope();
        let kernel = crate::lane::kernel(MARK, "launch_guard_mark", None);
        let count = crate::lane::buffer_of(device, &[0u32]);
        let indirect = crate::lane::buffer_of(device, &[1u32, 1, 1]);
        // Separate commands and nested scopes share the same budget; each form of launch is charged.
        for command_index in 0..KERNEL_LAUNCH_LIMIT.div_ceil(9) {
            let _nested = LaunchScope::enter();
            let mut command = crate::lane::Command::new("launch_guard_probe");
            for launch in 0..(KERNEL_LAUNCH_LIMIT - command_index * 9).min(9) {
                let l = command.launch(&kernel, "launch_guard_probe").buffer("count", &count);
                match (command_index * 9 + launch) % 3 {
                    0 => l.threads(1, 1),
                    1 => l.groups([1, 1, 1], [1, 1, 1]),
                    _ => l.indirect(&indirect, 0, [1, 1, 1], [1, 1, 1]),
                }
            }
            command.run().unwrap();
        }
        assert_eq!(word(&count), KERNEL_LAUNCH_LIMIT as u32);
        let mut command = crate::lane::Command::new("launch_guard_probe");
        command.launch(&kernel, "launch_guard_probe").buffer("count", &count).threads(1, 1);
        assert_eq!(command.len(), 0, "the launch past the budget is not held");
        assert_eq!(command.commit().err(), Some(Error::BackendLaunchLimit));
        assert_eq!(word(&count), KERNEL_LAUNCH_LIMIT as u32);
    }

    #[test]
    fn an_over_budget_command_does_not_run_its_allowed_prefix() {
        let Ok(device) = crate::lane::device() else { return };
        let _scope = crate::runtime::evaluation_scope();
        let kernel = crate::lane::kernel(ZERO, "launch_guard_zero", None);
        let count = crate::lane::buffer_of(device, &[0u32]);
        let mut command = crate::lane::Command::new("launch_guard_probe");
        for _ in 0..=KERNEL_LAUNCH_LIMIT {
            command.launch(&kernel, "launch_guard_probe").buffer("count", &count).groups([1, 1, 1], [1, 1, 1]);
        }
        assert_eq!(command.commit().err(), Some(Error::BackendLaunchLimit));
        assert_eq!(word(&count), 0);
    }
}
