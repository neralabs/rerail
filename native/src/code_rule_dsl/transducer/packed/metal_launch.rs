//! Charge every compute dispatch before encoding it. Refuse submission after exhaustion.
use super::Error;
use metal::foreign_types::ForeignTypeRef;
use metal::{BufferRef, CommandBufferRef, ComputeCommandEncoderRef, MTLSize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[path = "metal_launch_budget.rs"]
mod budget;
use budget::LaunchBudget;
pub(crate) use budget::KERNEL_LAUNCH_LIMIT;

thread_local! {
    static EVALUATION: RefCell<Option<Rc<RefCell<Evaluation>>>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct DispatchRecord {
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
    bindings: HashMap<(usize, usize), Vec<(usize, u64)>>,
    peak_application_buffer_bytes: usize,
}

pub(crate) fn record_buffer_allocation(bytes: usize) {
    EVALUATION.with_borrow(|slot| {
        if let Some(evaluation) = slot {
            let mut evaluation = evaluation.borrow_mut();
            evaluation.peak_application_buffer_bytes =
                evaluation.peak_application_buffer_bytes.max(bytes);
        }
    });
}

/// Nested commands, ranges, replay, and pages share the outer evaluation budget.
pub(crate) struct LaunchScope {
    owner: bool,
    _budget: Rc<RefCell<Evaluation>>,
}

impl LaunchScope {
    pub(crate) fn enter() -> Self {
        EVALUATION.with_borrow_mut(|slot| {
            let owner = slot.is_none();
            let budget = slot.get_or_insert_with(|| Rc::new(RefCell::new(Evaluation::default())));
            Self {
                owner,
                _budget: Rc::clone(budget),
            }
        })
    }

    pub(crate) fn snapshot(&self) -> Vec<DispatchRecord> {
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

fn reserve() -> bool {
    EVALUATION.with_borrow(|slot| {
        slot.as_ref()
            .is_some_and(|budget| budget.borrow_mut().budget.reserve())
    })
}

pub(crate) fn check_submission() -> Result<(), Error> {
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

pub(crate) fn commit_checked(command: &CommandBufferRef) -> Result<(), Error> {
    check_submission()?;
    command.commit();
    mark_command(command, false);
    Ok(())
}

/// Releases a command buffer whose submission `commit_checked` refused: it still holds a place in its queue (a queue
/// holds 64) until it completes, so it is committed and waited for. Its dispatches were refused when encoded
/// (`checked_dispatch_thread_groups`), and its results are not read.
pub(crate) fn release_refused(command: &CommandBufferRef) {
    command.commit();
    command.wait_until_completed();
}

/// A completed command buffer's time on the GPU, in seconds (`GPUEndTime - GPUStartTime`).
pub(crate) fn gpu_seconds(command: &CommandBufferRef) -> f64 {
    unsafe extern "C" {
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn objc_msgSend();
    }
    // SAFETY: `GPUStartTime` and `GPUEndTime` take no arguments and return a CFTimeInterval (a double).
    let send: unsafe extern "C" fn(*const std::ffi::c_void, *mut std::ffi::c_void) -> f64 =
        unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    let object = command.as_ptr().cast::<std::ffi::c_void>().cast_const();
    let (start, end) = unsafe { (send(object, sel_registerName(c"GPUStartTime".as_ptr())), send(object, sel_registerName(c"GPUEndTime".as_ptr()))) };
    (end - start).max(0.0)
}

/// The host clock Metal's command-buffer times are in (`CACurrentMediaTime`: mach absolute time), in seconds.
pub(crate) fn host_seconds() -> f64 {
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
pub(crate) fn command_times(command: &CommandBufferRef) -> [f64; 3] {
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

pub(crate) fn wait_checked(command: &CommandBufferRef) -> Result<(), Error> {
    command.wait_until_completed();
    if command.status() != metal::MTLCommandBufferStatus::Completed {
        return Err(Error::BackendFailure);
    }
    mark_command(command, true);
    Ok(())
}

fn mark_command(command: &CommandBufferRef, completed: bool) {
    EVALUATION.with_borrow(|slot| {
        if let Some(evaluation) = slot {
            let mut evaluation = evaluation.borrow_mut();
            let pointer = command.as_ptr() as usize;
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
                    evaluation
                        .bindings
                        .retain(|(owner, _), _| *owner != pointer);
                }
            }
        }
    });
}

fn record_attempt(
    command: &CommandBufferRef,
    encoder: &ComputeCommandEncoderRef,
    grid: MTLSize,
    group: MTLSize,
    indirect: bool,
) -> bool {
    EVALUATION.with_borrow(|slot| {
        let Some(evaluation) = slot else { return false };
        let mut evaluation = evaluation.borrow_mut();
        let pointer = command.as_ptr() as usize;
        let id = if let Some(&id) = evaluation.commands.get(&pointer) {
            id
        } else {
            let id = evaluation.next_command;
            evaluation.next_command += 1;
            evaluation.commands.insert(pointer, id);
            id
        };
        let encoded = evaluation.budget.reserve();
        let buffers = evaluation
            .bindings
            .remove(&(pointer, encoder.as_ptr() as usize))
            .unwrap_or_default();
        let (application_buffer_bytes, private_buffer_bytes) = super::buffer_usage();
        evaluation.peak_application_buffer_bytes = evaluation
            .peak_application_buffer_bytes
            .max(application_buffer_bytes);
        let peak_application_buffer_bytes = evaluation.peak_application_buffer_bytes;
        evaluation.records.push(DispatchRecord {
            command: id,
            kernel: encoder.label().to_owned(),
            grid: [grid.width, grid.height, grid.depth],
            group: [group.width, group.height, group.depth],
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

pub(crate) trait CheckedComputeEncoder {
    fn checked_set_buffers(&self, command: &CommandBufferRef, buffers: &[&BufferRef]);
    fn checked_dispatch_threads(&self, command: &CommandBufferRef, grid: MTLSize, group: MTLSize);
    fn checked_dispatch_thread_groups(
        &self,
        command: &CommandBufferRef,
        grid: MTLSize,
        group: MTLSize,
    );
    fn checked_dispatch_thread_groups_indirect(
        &self,
        command: &CommandBufferRef,
        buffer: &BufferRef,
        offset: u64,
        group: MTLSize,
    );
}

impl CheckedComputeEncoder for ComputeCommandEncoderRef {
    fn checked_set_buffers(&self, command: &CommandBufferRef, buffers: &[&BufferRef]) {
        for (index, buffer) in buffers.iter().enumerate() {
            self.set_buffer(index as u64, Some(buffer), 0);
        }
        EVALUATION.with_borrow(|slot| {
            if let Some(evaluation) = slot {
                evaluation.borrow_mut().bindings.insert(
                    (command.as_ptr() as usize, self.as_ptr() as usize),
                    buffers
                        .iter()
                        .map(|buffer| (buffer.as_ptr() as usize, buffer.length()))
                        .collect(),
                );
            }
        });
    }
    fn checked_dispatch_threads(&self, command: &CommandBufferRef, grid: MTLSize, group: MTLSize) {
        if record_attempt(command, self, grid, group, false) {
            self.dispatch_threads(grid, group);
        }
    }
    fn checked_dispatch_thread_groups(
        &self,
        command: &CommandBufferRef,
        grid: MTLSize,
        group: MTLSize,
    ) {
        if record_attempt(command, self, grid, group, false) {
            self.dispatch_thread_groups(grid, group);
        }
    }
    fn checked_dispatch_thread_groups_indirect(
        &self,
        command: &CommandBufferRef,
        buffer: &BufferRef,
        offset: u64,
        group: MTLSize,
    ) {
        if record_attempt(command, self, MTLSize::new(0, 0, 0), group, true) {
            self.dispatch_thread_groups_indirect(buffer, offset, group);
        }
    }
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

    #[test]
    fn production_entry_preserves_the_outer_budget_and_returns_launch_limit() {
        use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
        use crate::code_rule_dsl::transducer::packed::{Program, metal_program_backend};
        use ast_grep_core::tree_sitter::StrDoc;
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_core::tree_sitter::LanguageExt;
        use std::sync::Arc;

        let _evaluation = LaunchScope::enter();
        for _ in 0..KERNEL_LAUNCH_LIMIT {
            assert!(reserve());
        }
        assert!(!reserve());
        let source = "name\n";
        let grammar = Grammar::new(GrammarDialect::Python).unwrap();
        let native = StrDoc::new(source, GrammarDialect::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        let doc =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        let anchors = vec![vec![(0..doc.transducer_len() as u32).collect()]];
        let program = Program::compile_tree_sitter_query_pattern(
            "(identifier) @name",
            &GrammarDialect::Python.get_ts_language(),
        )
        .unwrap();
        let result = metal_program_backend::execute_packed_with_result_limits(
            &metal_program_backend::compile_interpreter().unwrap(),
            &[program],
            &[doc],
            &anchors,
            None,
        );
        assert!(
            matches!(result, Err(Error::BackendLaunchLimit)),
            "production must preserve the evaluation budget"
        );
        assert_eq!(check_submission(), Err(Error::BackendLaunchLimit));
    }

    #[test]
    fn production_encoders_cannot_bypass_the_launch_guard() {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/transducer/packed");
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            // `*_tests.rs` files are test modules (mounted under `#[cfg(test)]`), not encoders.
            if !name.starts_with("metal")
                || !name.ends_with(".rs")
                || name.ends_with("_tests.rs")
                || name == "metal_launch.rs"
            {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            let source = source.split("#[cfg(test)]\nmod ").next().unwrap();
            for forbidden in [
                ".dispatch_threads(",
                ".dispatch_thread_groups(",
                ".dispatch_thread_groups_indirect(",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{name} bypasses the launch guard: {forbidden}"
                );
            }
            if name != "metal_backend.rs" {
                assert!(
                    !source.contains(".commit()"),
                    "{name} bypasses checked submission"
                );
            } else {
                // Only the upload/readback blit commands can submit without a launch check.
                for prefix in source.split(".commit()").collect::<Vec<_>>().windows(2) {
                    let before = prefix[0];
                    assert!(
                        before.rfind("new_blit_command_encoder")
                            > before.rfind("new_compute_command_encoder"),
                        "unchecked compute submission in {name}"
                    );
                }
            }
        }
    }

    #[test]
    fn gpu_launch_guard_counts_all_dispatch_apis_and_refuses_submission() {
        use crate::code_rule_dsl::transducer::packed::metal_backend::{
            MappedBuffer, metal_execution_guard, pipeline_named,
        };
        let _guard = metal_execution_guard().unwrap();
        let device = metal::Device::system_default().unwrap();
        let pipeline = pipeline_named(
            "#include <metal_stdlib>\nusing namespace metal; kernel void launch_guard_mark(device atomic_uint* count [[buffer(0)]]) { atomic_fetch_add_explicit(count, 1u, memory_order_relaxed); }",
            "launch_guard_mark",
        ).unwrap();
        let count = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        let indirect = MappedBuffer::from_slice(&device, &[1_u32, 1, 1]).unwrap();
        let queue = device.new_command_queue();
        // Separate command buffers and nested scopes must share the same budget.
        for command_index in 0..KERNEL_LAUNCH_LIMIT.div_ceil(9) {
            let _nested = LaunchScope::enter();
            let command = queue.new_command_buffer();
            let encoder = command.new_compute_command_encoder();
            encoder.set_label("launch_guard_probe");
            encoder.set_compute_pipeline_state(&pipeline);
            encoder.set_buffer(0, Some(&count.buffer), 0);
            for launch in 0..(KERNEL_LAUNCH_LIMIT - command_index * 9).min(9) {
                match (command_index * 9 + launch) % 3 {
                    0 => encoder.checked_dispatch_threads(
                        command,
                        MTLSize::new(1, 1, 1),
                        MTLSize::new(1, 1, 1),
                    ),
                    1 => encoder.checked_dispatch_thread_groups(
                        command,
                        MTLSize::new(1, 1, 1),
                        MTLSize::new(1, 1, 1),
                    ),
                    _ => encoder.checked_dispatch_thread_groups_indirect(
                        command,
                        &indirect.buffer,
                        0,
                        MTLSize::new(1, 1, 1),
                    ),
                }
            }
            encoder.end_encoding();
            commit_checked(command).unwrap();
            wait_checked(command).unwrap();
            assert_eq!(command.status(), metal::MTLCommandBufferStatus::Completed);
        }
        assert_eq!(unsafe { count.buffer.contents().cast::<u32>().read() }, KERNEL_LAUNCH_LIMIT as u32);
        let command = queue.new_command_buffer();
        let encoder = command.new_compute_command_encoder();
        encoder.set_label("launch_guard_probe");
        encoder.set_compute_pipeline_state(&pipeline);
        encoder.set_buffer(0, Some(&count.buffer), 0);
        encoder.checked_dispatch_threads(command, MTLSize::new(1, 1, 1), MTLSize::new(1, 1, 1));
        encoder.end_encoding();
        assert_eq!(commit_checked(command), Err(Error::BackendLaunchLimit));
        assert_eq!(command.status(), metal::MTLCommandBufferStatus::NotEnqueued);
        assert_eq!(unsafe { count.buffer.contents().cast::<u32>().read() }, KERNEL_LAUNCH_LIMIT as u32);
    }

    #[test]
    fn an_over_budget_command_does_not_run_its_allowed_prefix() {
        use crate::code_rule_dsl::transducer::packed::metal_backend::{
            MappedBuffer, metal_execution_guard, pipeline_named,
        };
        let _guard = metal_execution_guard().unwrap();
        let device = metal::Device::system_default().unwrap();
        let pipeline = pipeline_named(
            "#include <metal_stdlib>\nusing namespace metal; kernel void launch_guard_zero(device uint* count [[buffer(0)]]) { count[0] = 1u; }",
            "launch_guard_zero",
        ).unwrap();
        let count = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let encoder = command.new_compute_command_encoder();
        encoder.set_label("launch_guard_probe");
        encoder.set_compute_pipeline_state(&pipeline);
        encoder.set_buffer(0, Some(&count.buffer), 0);
        for _ in 0..=KERNEL_LAUNCH_LIMIT {
            encoder.checked_dispatch_thread_groups(
                command,
                MTLSize::new(1, 1, 1),
                MTLSize::new(1, 1, 1),
            );
        }
        encoder.end_encoding();
        assert_eq!(commit_checked(command), Err(Error::BackendLaunchLimit));
        assert_eq!(command.status(), metal::MTLCommandBufferStatus::NotEnqueued);
        assert_eq!(unsafe { count.buffer.contents().cast::<u32>().read() }, 0);
    }
}
