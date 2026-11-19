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
enum Arg<'a> {
    Buffer(&'a str, &'a Buffer, u64),
    Value(&'a str, Vec<u8>),
    Local(&'a str, u64),
    Uses(&'a Buffer),
}

enum Grid<'a> {
    Groups([u64; 3], [u64; 3]),
    Indirect(&'a Buffer, u64, [u64; 3], [u64; 3]),
}

struct Pending<'a> {
    kernel: &'a ComputeFunction,
    label: String,
    args: Vec<Arg<'a>>,
    grid: Grid<'a>,
}

/// A command: launches, in order, each seeing the writes of those before it.
pub struct Command<'a> {
    label: String,
    launches: Vec<Pending<'a>>,
    /// This command's key in the evaluation's records.
    key: Box<u8>,
}

impl<'a> Command<'a> {
    pub fn new(label: &str) -> Command<'a> {
        Command { label: label.to_owned(), launches: Vec::new(), key: Box::new(0) }
    }

    fn pointer(&self) -> usize {
        std::ptr::from_ref::<u8>(&self.key) as usize
    }

    /// A launch of `kernel`, recorded as `label`.
    pub fn launch<'c>(&'c mut self, kernel: &'a ComputeFunction, label: &str) -> Launch<'c, 'a> {
        Launch { command: self, kernel, label: label.to_owned(), args: Vec::new() }
    }

    /// The launches this command holds (those within the budget).
    pub fn len(&self) -> usize {
        self.launches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.launches.is_empty()
    }

    /// Submits the command: refused (and nothing of it run) when the evaluation is past its launch budget.
    pub fn commit(self) -> Result<Committed, Error> {
        crate::launch::check_submission()?;
        let layer = rerail_accel::gpu::shared::layer().ok_or(Error::UnsupportedBackend)?;
        let mut queue = ComputeQueue::new(layer.queue);
        for p in &self.launches {
            bind_and_run(&queue, p).unwrap_or_else(|e| panic!("{} ({}): {e}", p.label, self.label));
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

fn bind_and_run(queue: &ComputeQueue, p: &Pending<'_>) -> Result<(), String> {
    let mut ctx = queue.new_bind_context(p.kernel);
    for a in &p.args {
        match a {
            Arg::Buffer(name, b, 0) => ctx.set_buffer(index(p.kernel, name)?, b),
            Arg::Buffer(name, b, offset) => ctx.set_buffer_at(index(p.kernel, name)?, b, *offset)?,
            Arg::Value(name, bytes) => ctx.set_primitive(index(p.kernel, name)?, bytes),
            Arg::Local(name, bytes) => ctx.set_thread_group_memory(index(p.kernel, name)?, *bytes),
            Arg::Uses(b) => ctx.use_buffer(b),
        }
    }
    match &p.grid {
        Grid::Groups(groups, block) => ctx.launch(groups, block),
        Grid::Indirect(counts, offset, most, block) => ctx.launch_indirect(counts, *offset, most, block),
    }
}

/// A launch being bound.
pub struct Launch<'c, 'a> {
    command: &'c mut Command<'a>,
    kernel: &'a ComputeFunction,
    label: String,
    args: Vec<Arg<'a>>,
}

impl<'a> Launch<'_, 'a> {
    /// A buffer row.
    pub fn buffer(mut self, name: &'a str, buffer: &'a Buffer) -> Self {
        self.args.push(Arg::Buffer(name, buffer, 0));
        self
    }

    /// A buffer row bound from byte `offset` of `buffer`.
    pub fn buffer_at(mut self, name: &'a str, buffer: &'a Buffer, offset: u64) -> Self {
        self.args.push(Arg::Buffer(name, buffer, offset));
        self
    }

    /// A value row (`ARGS`, `LITERAL`): a `#[repr(C)]` value laid out as the kernel's.
    pub fn value<T: rerail_device::Pod>(mut self, name: &'a str, v: &T) -> Self {
        // SAFETY: a Pod value's bytes, all initialized.
        let bytes = unsafe { std::slice::from_raw_parts(std::ptr::from_ref(v).cast::<u8>(), size_of::<T>()) };
        self.args.push(Arg::Value(name, bytes.to_vec()));
        self
    }

    /// Threadgroup memory for a `RWLOCAL` row.
    pub fn local(mut self, name: &'a str, bytes: u64) -> Self {
        self.args.push(Arg::Local(name, bytes));
        self
    }

    /// A buffer the kernel reaches through an address another holds.
    pub fn uses(mut self, buffer: &'a Buffer) -> Self {
        self.args.push(Arg::Uses(buffer));
        self
    }

    fn buffers(&self) -> Vec<(usize, u64)> {
        self.args.iter().filter_map(|a| if let Arg::Buffer(_, b, _) = a { Some((b.contents() as usize, b.length())) } else { None }).collect()
    }

    fn push(self, grid: Grid<'a>, shape: ([u64; 3], [u64; 3]), indirect: bool) {
        let buffers = self.buffers();
        if crate::launch::attempt(self.command.pointer(), &self.label, shape.0, shape.1, indirect, buffers) {
            self.command.launches.push(Pending { kernel: self.kernel, label: self.label, args: self.args, grid });
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

    /// As many threadgroups as the three u32s at byte `offset` of `counts` say when it runs, at most `most`.
    pub fn indirect(self, counts: &'a Buffer, offset: u64, most: [u64; 3], block: [u64; 3]) {
        self.push(Grid::Indirect(counts, offset, most, block), ([0; 3], block), true);
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
