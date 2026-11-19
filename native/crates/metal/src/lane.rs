//! The engine's commands on the device layer (decision 0440 step C): its kernels launched on the process's device,
//! whatever its backend (Metal, CUDA, OpenCL, the CPU), under the same launch budget and dispatch records as its
//! Metal command buffers (`launch`).
//!
//! A command holds its launches until it is committed: each is charged to the evaluation's budget when it is asked
//! for (a launch past the budget is not encoded), and a command whose evaluation is past its budget at the commit
//! runs none of them. That is Metal's rule (`commit_checked`), kept on backends whose queue starts a launch as it is
//! enqueued. A kernel's arguments are bound by their names in its rows (`DEFINE_KERNEL2`).
//!
//! Kernels come from the engine's kernel store (`rerail_accel::gpu::kernels::function`): on Metal its pipelines
//! and binary archives, elsewhere the device layer's compile and store.

use crate::Error;
use rerail_device::compute_kernel_grid::GridBindContext;
use rerail_device::device::{ComputeEvent, ComputeFunction, ComputeQueue};
pub use rerail_device::Pod;
pub use rerail_device::device::{Buffer, Device, backend};
pub use crate::runtime::HostMapped;

/// A fresh buffer of the device's holding `values` (a buffer not of the pool: `crate::pool` leases those).
pub fn buffer_of<T: Pod>(device: &Device, values: &[T]) -> Buffer {
    let b = rerail_device::device::alloc_uninitialized_array_sync::<T>(device, values.len().max(1));
    // SAFETY: the buffer holds at least `values.len()` values of `T`; no command uses it yet.
    unsafe { std::ptr::copy_nonoverlapping(values.as_ptr(), b.contents().cast::<T>(), values.len()) };
    b
}
use std::sync::Arc;

/// A kernel as the lane launches it.
pub type Kernel = Arc<ComputeFunction>;

/// The process's device in the device layer, or `UnsupportedBackend` without one.
pub fn device() -> Result<&'static Device, Error> {
    rerail_accel::gpu::shared::layer().map(|l| l.device).ok_or(Error::UnsupportedBackend)
}

/// The kernel `name` of `source` (UKL), with `constant` at specialization constant 0 (`UKL_CONSTANT_U32`).
pub fn kernel(source: &str, name: &str, constant: Option<u32>) -> Kernel {
    rerail_accel::gpu::kernels::function(source, name, constant)
}

/// [`kernel`] of a fixed source (an interpreter's, the same for every rule set): shipped with the engine.
pub fn kernel_fixed(source: &str, name: &str, constant: Option<u32>) -> Kernel {
    rerail_accel::gpu::kernels::function_fixed(source, name, constant)
}

/// One argument of a launch, bound at the commit.
enum Arg {
    Buffer(String, Buffer, u64),
    /// A buffer at the row numbered `.0` (rows are numbered densely in their order on every backend).
    Slot(u64, Buffer, u64),
    /// A value at the row numbered `.0`.
    SlotValue(u64, Vec<u8>),
    Value(String, Vec<u8>),
    Local(String, u64),
    Uses(Buffer),
}

enum Grid {
    Groups([u64; 3], [u64; 3]),
    Indirect(Buffer, u64, [u64; 3], [u64; 3]),
}

/// A step of a command, in order.
enum Step {
    Launch { kernel: Kernel, label: String, args: Vec<Arg>, grid: Grid },
    /// `length` bytes from `src` at its offset to `dst` at its.
    Copy { src: Buffer, at: u64, dst: Buffer, to: u64, length: u64 },
    /// `count` u32s of `dst` from byte `at`, each `value`.
    Fill { dst: Buffer, at: u64, count: u64, value: u32 },
}

/// A command: its steps (launches, copies, fills) in order, each seeing the writes of those before it. It holds its
/// buffers and kernels (a buffer is a reference to its memory), so a caller builds one across calls and commits it
/// when ready.
pub struct Command {
    label: String,
    steps: Vec<Step>,
    /// This command's key in the evaluation's records.
    key: Box<u8>,
}

impl Command {
    pub fn new(label: &str) -> Command {
        Command { label: label.to_owned(), steps: Vec::new(), key: Box::new(0) }
    }

    fn pointer(&self) -> usize {
        std::ptr::from_ref::<u8>(&self.key) as usize
    }

    /// A launch of `kernel`, recorded as `label`.
    pub fn launch<'c>(&'c mut self, kernel: &Kernel, label: &str) -> Launch<'c> {
        Launch { command: self, kernel: Arc::clone(kernel), label: label.to_owned(), args: Vec::new() }
    }

    /// A copy of `length` bytes from byte `at` of `src` to byte `to` of `dst`, after the steps before it.
    pub fn copy(&mut self, src: &Buffer, at: u64, dst: &Buffer, to: u64, length: u64) {
        if length > 0 {
            self.steps.push(Step::Copy { src: src.clone(), at, dst: dst.clone(), to, length });
        }
    }

    /// `count` u32s of `dst` from byte `at` set to `value`, after the steps before it.
    pub fn fill(&mut self, dst: &Buffer, at: u64, count: u64, value: u32) {
        if count > 0 {
            self.steps.push(Step::Fill { dst: dst.clone(), at, count, value });
        }
    }

    /// The launches this command holds (those within the budget).
    pub fn len(&self) -> usize {
        self.steps.iter().filter(|s| matches!(s, Step::Launch { .. })).count()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Submits the command: refused (and nothing of it run) when the evaluation is past its launch budget.
    pub fn commit(self) -> Result<Committed, Error> {
        crate::launch::check_submission()?;
        let layer = rerail_accel::gpu::shared::layer().ok_or(Error::UnsupportedBackend)?;
        let mut queue = ComputeQueue::new(layer.queue);
        for step in &self.steps {
            match step {
                Step::Launch { kernel, label, args, grid } => {
                    bind_and_run(&queue, kernel, args, grid).unwrap_or_else(|e| panic!("{label} ({}): {e}", self.label))
                }
                Step::Copy { src, at, dst, to, length } => queue.enqueue_copy_between_device_regions(src, dst, *at, *to, *length),
                Step::Fill { dst, at, count, value } => {
                    assert_eq!(at % 4, 0, "a fill at a word");
                    queue.enqueue_fill_array(dst, value, (*at / 4) as usize, Some(*count as usize))
                }
            }
        }
        let event = queue.flush();
        crate::launch::mark(self.pointer(), false);
        Ok(Committed { event, key: self.key, label: self.label, started: crate::launch::host_seconds() })
    }

    /// Commits and waits.
    pub fn run(self) -> Result<Times, Error> {
        self.commit()?.wait()
    }
}

fn index(kernel: &ComputeFunction, name: &str) -> Result<u64, String> {
    kernel.get_argument_info().iter().find(|a| a.name == name).map(|a| a.compute_function_arg_index).ok_or_else(|| {
        let names: Vec<&str> = kernel.get_argument_info().iter().map(|a| a.name.as_str()).collect();
        format!("no argument {name} (the kernel's: {names:?})")
    })
}

fn bind_and_run(queue: &ComputeQueue, kernel: &ComputeFunction, args: &[Arg], grid: &Grid) -> Result<(), String> {
    let mut ctx = queue.new_bind_context(kernel);
    for a in args {
        match a {
            Arg::Buffer(name, b, 0) => ctx.set_buffer(index(kernel, name)?, b),
            Arg::Buffer(name, b, offset) => ctx.set_buffer_at(index(kernel, name)?, b, *offset)?,
            Arg::Slot(at, b, 0) => ctx.set_buffer(*at, b),
            Arg::Slot(at, b, offset) => ctx.set_buffer_at(*at, b, *offset)?,
            Arg::SlotValue(at, bytes) => ctx.set_primitive(*at, bytes),
            Arg::Value(name, bytes) => ctx.set_primitive(index(kernel, name)?, bytes),
            Arg::Local(name, bytes) => ctx.set_thread_group_memory(index(kernel, name)?, *bytes),
            Arg::Uses(b) => ctx.use_buffer(b),
        }
    }
    match grid {
        Grid::Groups(groups, block) => ctx.launch(groups, block),
        Grid::Indirect(counts, offset, most, block) => ctx.launch_indirect(counts, *offset, most, block),
    }
}

/// A launch being bound.
pub struct Launch<'c> {
    command: &'c mut Command,
    kernel: Kernel,
    label: String,
    args: Vec<Arg>,
}

impl Launch<'_> {
    /// A buffer row.
    pub fn buffer(mut self, name: &str, buffer: &Buffer) -> Self {
        self.args.push(Arg::Buffer(name.to_owned(), buffer.clone(), 0));
        self
    }

    /// A buffer row bound from byte `offset` of `buffer`.
    pub fn buffer_at(mut self, name: &str, buffer: &Buffer, offset: u64) -> Self {
        self.args.push(Arg::Buffer(name.to_owned(), buffer.clone(), offset));
        self
    }

    /// The buffer of row `at` (by its number: a caller that binds a kernel's rows in order), from byte `offset`.
    pub fn slot(mut self, at: u64, buffer: &Buffer, offset: u64) -> Self {
        self.args.push(Arg::Slot(at, buffer.clone(), offset));
        self
    }

    /// The value of row `at` (by its number), a `#[repr(C)]` value laid out as the kernel's.
    pub fn slot_value<T: rerail_device::Pod>(mut self, at: u64, v: &T) -> Self {
        // SAFETY: a Pod value's bytes, all initialized.
        let bytes = unsafe { std::slice::from_raw_parts(std::ptr::from_ref(v).cast::<u8>(), size_of::<T>()) };
        self.args.push(Arg::SlotValue(at, bytes.to_vec()));
        self
    }

    /// A value row (`ARGS`, `LITERAL`): a `#[repr(C)]` value laid out as the kernel's.
    pub fn value<T: rerail_device::Pod>(mut self, name: &str, v: &T) -> Self {
        // SAFETY: a Pod value's bytes, all initialized.
        let bytes = unsafe { std::slice::from_raw_parts(std::ptr::from_ref(v).cast::<u8>(), size_of::<T>()) };
        self.args.push(Arg::Value(name.to_owned(), bytes.to_vec()));
        self
    }

    /// Threadgroup memory for a `RWLOCAL` row.
    pub fn local(mut self, name: &str, bytes: u64) -> Self {
        self.args.push(Arg::Local(name.to_owned(), bytes));
        self
    }

    /// A buffer the kernel reaches through an address another holds.
    pub fn uses(mut self, buffer: &Buffer) -> Self {
        self.args.push(Arg::Uses(buffer.clone()));
        self
    }

    fn buffers(&self) -> Vec<(usize, u64)> {
        self.args
            .iter()
            .filter_map(|a| match a {
                Arg::Buffer(_, b, _) | Arg::Slot(_, b, _) => Some((b.contents() as usize, b.length())),
                _ => None,
            })
            .collect()
    }

    fn push(self, grid: Grid, shape: ([u64; 3], [u64; 3]), indirect: bool) {
        let buffers = self.buffers();
        if crate::launch::attempt(self.command.pointer(), &self.label, shape.0, shape.1, indirect, buffers) {
            self.command.steps.push(Step::Launch { kernel: self.kernel, label: self.label, args: self.args, grid });
        }
    }

    /// `groups` threadgroups of `block` threads.
    pub fn groups(self, groups: [u64; 3], block: [u64; 3]) {
        let threads = [groups[0] * block[0], groups[1] * block[1], groups[2] * block[2]];
        self.push(Grid::Groups(groups, block), (threads, block), false);
    }

    /// At least `threads` threads (a 1-D grid), in threadgroups of `width`: the kernel bounds its own threads.
    pub fn threads(self, threads: u64, width: u64) {
        let width = width.max(1);
        self.groups([threads.div_ceil(width), 1, 1], [width, 1, 1]);
    }

    /// Exactly `threads` threads (a 1-D grid), in threadgroups of the widest width up to `width` that divides them:
    /// for a kernel that does not bound its own threads.
    pub fn threads_exact(self, threads: u64, width: u64) {
        let mut w = width.clamp(1, threads.max(1));
        while threads % w != 0 {
            w -= 1;
        }
        self.groups([threads / w, 1, 1], [w, 1, 1]);
    }

    /// As many threadgroups as the three u32s at byte `offset` of `counts` say when it runs, at most `most`.
    pub fn indirect(self, counts: &Buffer, offset: u64, most: [u64; 3], block: [u64; 3]) {
        self.push(Grid::Indirect(counts.clone(), offset, most, block), ([0; 3], block), true);
    }
}

/// A committed command.
pub struct Committed {
    event: ComputeEvent,
    key: Box<u8>,
    label: String,
    started: f64,
}

/// A completed command's times, in seconds: on the device, and from its commit to its completion seen here. On
/// Metal also its host-clock times (`launch::command_times`: scheduled, started, ended), else zero.
#[derive(Clone, Copy, Debug, Default)]
pub struct Times {
    pub device: f64,
    pub wall: f64,
    pub committed: f64,
    pub host: [f64; 3],
}

impl Committed {
    /// Waits for the command: `BackendFailure` when it did not complete.
    pub fn wait(self) -> Result<Times, Error> {
        if let Err(e) = self.event.await_() {
            if std::env::var_os("RERAIL_LAUNCHES").is_some() {
                eprintln!("{}: {e}", self.label);
            }
            return Err(Error::BackendFailure);
        }
        crate::launch::mark(std::ptr::from_ref::<u8>(&self.key) as usize, true);
        let host = self.event.metal().map_or([0.0; 3], crate::launch::command_times);
        Ok(Times { device: self.event.gpu_seconds(), wall: crate::launch::host_seconds() - self.started, committed: self.started, host })
    }
}
