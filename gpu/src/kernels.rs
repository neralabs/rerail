//! Kernels compiled at run time from UKL source (decision 0440 §4, §5): the text preprocessed in-process for the
//! process's device (`rerail_ukl`, with the UKL library on its include path), compiled by the device's compiler (the
//! Metal front end, NVRTC for the device's architecture), once a process per program; and a launch whose arguments
//! are bound by their names in the kernel's rows.

use crate::compute_kernel_grid::GridBindContext;
use crate::device::{alloc_uninitialized_array_sync, transfer_to_host_sync, Buffer, BufferRef, ComputeFunction, ComputeFunctionLibrary, ComputeQueue};
use crate::{Pod, Preprocessing};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};


/// `text`, a UKL source named `name`, preprocessed for the process's device.
pub fn preprocess(name: &str, text: &str) -> Result<String, String> {
    preprocess_with(name, text, &[])
}

/// `text` preprocessed for the process's device with `defines` (a specialization: `program_with`).
pub fn preprocess_with(name: &str, text: &str, defines: &[(&str, &str)]) -> Result<String, String> {
    let path = format!("/<kernels>/{name}");
    let mut p = crate::shared::get().ok_or("no device")?.device.preprocess().file(&path, text);
    for (n, v) in defines {
        p = p.define(n, Some(v));
    }
    p.run(&path).map_err(|e| e.to_string())
}

/// A compiled program: its library, and its functions as they are asked for.
pub struct Program {
    name: String,
    library: ComputeFunctionLibrary,
    functions: Mutex<HashMap<String, &'static ComputeFunction>>,
}

/// The program `text`, compiled the first time `name` is asked for. A failure is the same failure every time.
pub fn program(name: &str, text: &str) -> Result<&'static Program, String> {
    program_with(name, text, &[])
}

/// The program `text` specialized by `defines` (decision 0440 step B): each value a program of its own, compiled the
/// first time it is asked for, as Metal's function constants make a pipeline a value (the label passes' tape format).
/// A define is portable where a function constant is Metal's alone, and the compiler folds it the same way.
///
/// A program is kept by its name, its text and its defines: two texts of one name are two programs.
///
/// Each program has a slot of its own: two threads that ask for one program compile it once (the second waits for
/// it), and two programs compile at once (`compile_ahead` compiles a list so).
pub fn program_with(name: &str, text: &str, defines: &[(&str, &str)]) -> Result<&'static Program, String> {
    type Slot = std::sync::Arc<OnceLock<Result<&'static Program, String>>>;
    static PROGRAMS: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    let digest = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        h.finish()
    };
    let key: String = std::iter::once(format!("{name} {digest:016x}")).chain(defines.iter().map(|(n, v)| format!(" -D{n}={v}"))).collect();
    let slot: Slot = std::sync::Arc::clone(PROGRAMS.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner()).entry(key.clone()).or_default());
    slot.get_or_init(|| {
        let shared = crate::shared::get().ok_or("no device")?;
        let started = std::time::Instant::now();
        let (library, preprocessed, bytes) = if crate::vec::on(&shared.device) {
            (crate::vec::library(name, &format!("/<kernels>/{name}"), text, defines)?, std::time::Duration::ZERO, 0)
        } else {
            let source = preprocess_with(name, text, defines)?;
            let preprocessed = started.elapsed();
            (ComputeFunctionLibrary::compile_from_source(&shared.device, &source, name)?, preprocessed, source.len())
        };
        // `RERAIL_GPU_TIMES`, a measurement aid: each program's preprocessing and compile (or load), on stderr.
        if std::env::var_os("RERAIL_GPU_TIMES").is_some() {
            eprintln!("gpu program {name}: preprocessed {:.3} s ({bytes} bytes), compiled {:.3} s", preprocessed.as_secs_f64(), (started.elapsed() - preprocessed).as_secs_f64());
        }
        Ok(&*Box::leak(Box::new(Program { name: key, library, functions: Default::default() })))
    })
    .clone()
}

/// A program compiled ahead of its first use (decision 0395's tiers on the device layer): `make` asks for it as its
/// use does (the same call, so the same name, defines and text, and the same entry in the store), and its use then
/// finds it, in this process or in the store.
pub struct Ahead {
    pub name: String,
    pub make: Box<dyn Fn() -> Result<(), String> + Send + Sync>,
}

impl Ahead {
    pub fn new(name: impl Into<String>, make: impl Fn() -> Result<(), String> + Send + Sync + 'static) -> Ahead {
        Ahead { name: name.into(), make: Box::new(make) }
    }
}

/// What compiling one program ahead did: its time, the libraries it loaded (shipped or from the store) and
/// compiled, and the entries it used (`kernels ship` copies them).
#[derive(Debug)]
pub struct Made {
    pub name: String,
    pub seconds: f64,
    pub loaded: u64,
    pub compiled: u64,
    pub entries: Vec<std::path::PathBuf>,
    pub result: Result<(), String>,
    /// Not started: the list's time to start was up (`Bound::start_by`). Its use compiles it, or a later list.
    pub skipped: bool,
    /// Still compiling when the list's time was up (`Bound::end_by`): left to its thread, which the process may end.
    pub unfinished: bool,
}

/// How long `compile_ahead` may take: no compile starts after `start_by`, and it returns at `end_by` whatever is still
/// compiling (a compile cannot be stopped; its result is a rename into the store, so one cut off leaves nothing).
#[derive(Clone, Copy, Debug, Default)]
pub struct Bound {
    pub start_by: Option<std::time::Instant>,
    pub end_by: Option<std::time::Instant>,
}

/// Every program of `list`, `threads` at a time (each compile is the device compiler's, on the host's cores): in the
/// list's order. A program that fails is reported, not fatal: its use fails the same way, saying why. Within `bound`:
/// a bounded step, which the next one goes on from, since the store keeps what this one made.
pub fn compile_ahead(list: Vec<Ahead>, threads: usize, bound: Bound) -> Vec<Made> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Work {
        list: Vec<Ahead>,
        next: AtomicUsize,
        done: AtomicUsize,
        made: Mutex<Vec<Option<Made>>>,
        /// Each entry's start, while it compiles.
        running: Mutex<Vec<Option<std::time::Instant>>>,
    }
    let n = list.len();
    let work = Arc::new(Work { list, next: AtomicUsize::new(0), done: AtomicUsize::new(0), made: Mutex::new((0..n).map(|_| None).collect()), running: Mutex::new(vec![None; n]) });
    for _ in 0..threads.clamp(1, n.max(1)) {
        let work = Arc::clone(&work);
        std::thread::spawn(move || {
            loop {
                let i = work.next.fetch_add(1, Ordering::Relaxed);
                let Some(a) = work.list.get(i) else { break };
                let m = if bound.start_by.is_some_and(|u| std::time::Instant::now() >= u) {
                    Made { name: a.name.clone(), seconds: 0.0, loaded: 0, compiled: 0, entries: Vec::new(), result: Ok(()), skipped: true, unfinished: false }
                } else {
                    let started = std::time::Instant::now();
                    work.running.lock().unwrap_or_else(std::sync::PoisonError::into_inner)[i] = Some(started);
                    let (loaded, compiled) = rerail_device::store::here();
                    let (result, entries) = rerail_device::store::recording(|| (a.make)());
                    let (l, c) = rerail_device::store::here();
                    let seconds = started.elapsed().as_secs_f64();
                    Made { name: a.name.clone(), seconds, loaded: l - loaded, compiled: c - compiled, entries, result, skipped: false, unfinished: false }
                };
                work.made.lock().unwrap_or_else(std::sync::PoisonError::into_inner)[i] = Some(m);
                work.done.fetch_add(1, Ordering::Release);
            }
        });
    }
    while work.done.load(Ordering::Acquire) < n {
        if bound.end_by.is_some_and(|e| std::time::Instant::now() >= e) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // Each slot taken, not the list: a compile left running still writes its own slot.
    let made: Vec<Option<Made>> = work.made.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter_mut().map(Option::take).collect();
    let running = work.running.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    made.into_iter()
        .enumerate()
        .map(|(i, m)| {
            m.unwrap_or_else(|| {
                let a = &work.list[i];
                let seconds = running[i].map_or(0.0, |s| s.elapsed().as_secs_f64());
                Made { name: a.name.clone(), seconds, loaded: 0, compiled: 0, entries: Vec::new(), result: Ok(()), skipped: running[i].is_none(), unfinished: running[i].is_some() }
            })
        })
        .collect()
}

/// The threads `compile_ahead` takes by default: half the host's cores (a compile is one core's), at least one.
pub fn ahead_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1))
}

impl Program {
    /// The compiled library.
    pub fn library(&self) -> &ComputeFunctionLibrary {
        &self.library
    }

    /// The kernel `name`, with its arguments (made once).
    pub fn function(&self, name: &str) -> Result<&'static ComputeFunction, String> {
        let mut fns = self.functions.lock().unwrap();
        if let Some(f) = fns.get(name) {
            return Ok(f);
        }
        let shared = crate::shared::get().ok_or("no device")?;
        let f = self.library.get_function(&shared.device, name).map_err(|e| format!("{}: {e}", self.name))?;
        let f: &'static ComputeFunction = Box::leak(Box::new(f));
        fns.insert(name.to_string(), f);
        Ok(f)
    }
}

/// One launch of a kernel on a queue, its arguments bound by name. The first binding that fails is the launch's error.
pub struct Launch<'a, B: GridBindContext> {
    function: &'a ComputeFunction,
    ctx: B,
    error: Option<String>,
}

/// A launch of `function` on `queue`.
pub fn launch<'a>(queue: &'a ComputeQueue, function: &'a ComputeFunction) -> Launch<'a, impl GridBindContext + 'a> {
    Launch { function, ctx: queue.new_bind_context(function), error: None }
}

impl<B: GridBindContext> Launch<'_, B> {
    fn index(&mut self, name: &str) -> Option<u64> {
        let found = self.function.get_argument_info().iter().find(|a| a.name == name).map(|a| a.compute_function_arg_index);
        if found.is_none() && self.error.is_none() {
            let names: Vec<&str> = self.function.get_argument_info().iter().map(|a| a.name.as_str()).collect();
            self.error = Some(format!("no argument {name} (the kernel's: {names:?})"));
        }
        found
    }

    /// A buffer row (`ROBUFFER`, `RWBUFFER`, `CONSTBUFFER`).
    pub fn buffer(mut self, name: &str, buffer: &BufferRef) -> Self {
        if let Some(i) = self.index(name) {
            self.ctx.set_buffer(i, buffer);
        }
        self
    }

    /// A buffer row bound from byte `offset` of `buffer` (a block's allocation): the row's index 0 is that byte.
    pub fn buffer_at(mut self, name: &str, buffer: &BufferRef, offset: u64) -> Self {
        if let Some(i) = self.index(name)
            && let Err(e) = self.ctx.set_buffer_at(i, buffer, offset)
        {
            self.error.get_or_insert(e);
        }
        self
    }

    /// A value row (`ARGS`: a `#[repr(C)]` struct laid out as the kernel's).
    pub fn value<T: Pod>(mut self, name: &str, v: &T) -> Self {
        if let Some(i) = self.index(name) {
            // SAFETY: a Pod value's bytes, all of them initialized.
            let bytes = unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), size_of::<T>()) };
            self.ctx.set_primitive(i, bytes);
        }
        self
    }

    /// A value row (`ARGS`) from its bytes, laid out as the kernel's struct (a value a plan keeps as bytes).
    pub fn bytes(mut self, name: &str, bytes: &[u8]) -> Self {
        if let Some(i) = self.index(name) {
            self.ctx.set_primitive(i, bytes);
        }
        self
    }

    /// Threadgroup memory for a `RWLOCAL` row.
    pub fn local(mut self, name: &str, bytes: u64) -> Self {
        if let Some(i) = self.index(name) {
            self.ctx.set_thread_group_memory(i, bytes);
        }
        self
    }

    /// A buffer the kernel reaches through an address another buffer holds (`Buffer::device_address`).
    pub fn uses(mut self, buffer: &BufferRef) -> Self {
        self.ctx.use_buffer(buffer);
        self
    }

    /// `uses` of a buffer the kernel only reads through the address (a view's directory, a slab).
    pub fn uses_read(mut self, buffer: &BufferRef) -> Self {
        self.ctx.use_buffer_read(buffer);
        self
    }

    /// Launched: `grid` threadgroups of `block` threads.
    pub fn run(mut self, grid: &[u64], block: &[u64]) -> Result<(), String> {
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        self.ctx.launch(grid, block)
    }

    /// Launched indirectly: as many threadgroups as the three u32s at byte `offset` of `groups` say when it runs,
    /// at most `max_groups` (`GridBindContext::launch_indirect`).
    pub fn run_indirect(mut self, groups: &BufferRef, offset: u64, max_groups: &[u64], block: &[u64]) -> Result<(), String> {
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        self.ctx.launch_indirect(groups, offset, max_groups, block)
    }
}

/// A buffer of `n` values of `T` (at least one), uninitialized.
pub fn alloc<T: Pod>(n: usize) -> Result<Buffer, String> {
    let shared = crate::shared::get().ok_or("no device")?;
    Ok(alloc_uninitialized_array_sync::<T>(&shared.device, n.max(1)))
}

/// A buffer holding `data` (written in place: the host's view of the device's memory).
pub fn buffer_of<T: Pod>(data: &[T]) -> Result<Buffer, String> {
    let b = alloc::<T>(data.len())?;
    // SAFETY: the buffer holds at least `data.len()` values of `T`.
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), b.contents().cast::<T>(), data.len()) };
    Ok(b)
}

/// The first `n` values of a buffer.
pub fn read<T: Pod + Clone>(buffer: &BufferRef, n: usize) -> Vec<T> {
    transfer_to_host_sync::<T>(buffer, n)
}
