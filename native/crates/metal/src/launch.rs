//! Charge every compute dispatch before encoding it. Refuse submission after exhaustion. The engine's commands
//! (`crate::lane`, on every backend) charge each launch here as it is asked for and check the budget at their commit.
use crate::Error;
use metal::CommandBufferRef;
use metal::foreign_types::ForeignTypeRef;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::launch_budget as budget;
use budget::LaunchBudget;
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
    pub private_buffer_bytes: usize,
    pub peak_application_buffer_bytes: usize,
}

#[derive(Default)]
struct Evaluation {
    budget: LaunchBudget,
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
            let budget = slot.get_or_insert_with(|| Rc::new(RefCell::new(Evaluation::default())));
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

/// Takes one launch from the evaluation's budget: false past it, or with no evaluation.
pub fn reserve() -> bool {
    EVALUATION.with_borrow(|slot| {
        slot.as_ref()
            .is_some_and(|budget| budget.borrow_mut().budget.reserve())
    })
}

pub fn check_submission() -> Result<(), Error> {
    EVALUATION.with_borrow(|slot| {
        let budget = slot.as_ref().ok_or(Error::BackendFailure)?;
        if budget.borrow().budget.refused() {
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

/// The host clock Metal's command-buffer times are in (`CACurrentMediaTime`: mach absolute time), in seconds.
pub fn host_seconds() -> f64 {
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }
    unsafe extern "C" {
        fn mach_timebase_info(info: *mut Timebase) -> i32;
        fn mach_absolute_time() -> u64;
    }
    static RATIO: std::sync::LazyLock<f64> = std::sync::LazyLock::new(|| {
        let mut info = Timebase { numer: 0, denom: 0 };
        // SAFETY: fills the struct.
        unsafe { mach_timebase_info(&mut info) };
        f64::from(info.numer) / f64::from(info.denom.max(1)) / 1e9
    });
    // SAFETY: no arguments.
    (unsafe { mach_absolute_time() }) as f64 * *RATIO
}

/// A completed command buffer's host-clock times, in seconds: `kernelStartTime` (the system began scheduling
/// it), `GPUStartTime`, `GPUEndTime`.
pub fn command_times(command: &CommandBufferRef) -> [f64; 3] {
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
        let encoded = evaluation.budget.reserve();
        let (application_buffer_bytes, private_buffer_bytes) = crate::runtime::buffer_usage();
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
            private_buffer_bytes,
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
