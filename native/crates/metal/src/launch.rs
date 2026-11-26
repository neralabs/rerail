//! The engine's launch budget, on Metal and the device layer: the evaluation's ledger is `rerail_accel`'s
//! (`rerail_accel::gpu::launch`), which every submitter charges, the engine's commands (`crate::lane`) and the
//! relational passes (`rerail_accel_gpu::gpu::join::Pass`) alike. Past the budget, or a launch outside an evaluation, is
//! the hard failure (decision 0601): `rerail_accel::exec::OverBudget`, never an error to fall back on. Here: Metal's
//! command-buffer times.
pub use rerail_accel::gpu::launch::{
    DispatchRecord, KERNEL_LAUNCH_LIMIT, LaunchScope, Rerun, Reruns, attempt, available, charge_made, check_submission, mark, plan_past_budget,
    record_buffer_allocation, rerun, rerun_available,
};

/// The host clock Metal's command-buffer times are in (mach absolute time on macOS), in seconds: `rerail_accel`'s.
pub fn host_seconds() -> f64 {
    rerail_accel_gpu::gpu::join::host_clock()
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


#[cfg(test)]
mod tests {
    use super::*;

    /// The account the launch past the budget raises (`OverBudget`), or none when `f` completes.
    fn over(f: impl FnOnce()) -> Option<String> {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            Ok(()) => None,
            Err(p) => Some(p.downcast::<rerail_accel::exec::OverBudget>().expect("the launch budget's failure").0),
        }
    }

    #[test]
    fn dispatch_and_submission_require_an_evaluation_scope() {
        assert!(over(|| attempt(1, "k", [1; 3], [1; 3], false, Vec::new())).is_some());
        assert!(over(check_submission).is_some());
    }

    /// A UKL kernel that counts its runs: launched by the engine's commands (`crate::lane`), on the process's device.
    const MARK: &str = "#include <ukl/backend.h>\nDEFINE_KERNEL2(guard, launch_guard_mark, RWBUFFER, uint32_t, count, \"[1]\", GID0, uint32_t, i, ) { if (i == 0u) ukl_atomic_add(count, 1u); }\n";

    fn word(b: &crate::lane::Buffer) -> u32 {
        // SAFETY: a buffer of at least one u32 the device is done with (its command waited for).
        unsafe { b.contents().cast::<u32>().read() }
    }

    /// Every form of launch is charged to one budget, across commands and nested scopes; the launch past it is the
    /// hard failure, before it is held, and nothing of its command runs.
    #[test]
    fn gpu_launch_guard_counts_all_dispatch_apis_and_fails_past_the_budget() {
        let Ok(device) = crate::lane::device() else { return };
        let _scope = crate::runtime::evaluation_scope();
        let kernel = crate::lane::kernel(MARK, "launch_guard_mark", None);
        let count = crate::lane::buffer_of(device, &[0u32]);
        let indirect = crate::lane::buffer_of(device, &[1u32, 1, 1]);
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
        let account = over(|| command.launch(&kernel, "launch_guard_probe").buffer("count", &count).threads(1, 1));
        assert!(account.is_some_and(|a| a.contains("launch_guard_probe")), "the account names the launches");
        assert_eq!(command.len(), 0, "the launch past the budget is not held");
        assert_eq!(word(&count), KERNEL_LAUNCH_LIMIT as u32);
    }
}
