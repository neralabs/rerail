//! Kernels compiled at run time from UKL source (decision 0400 §4, §5): the text preprocessed in-process for the
//! process's device (`rerail_ukl`, with the UKL library on its include path), compiled by the device's compiler (the
//! Metal front end, NVRTC for the device's architecture), once a process per program; and a launch whose arguments
//! are bound by their names in the kernel's rows.

use crate::compute_kernel_grid::GridBindContext;
use crate::device::{alloc_uninitialized_array_sync, transfer_to_host_sync, Buffer, BufferRef, ComputeFunction, ComputeFunctionLibrary, ComputeQueue};
use crate::Pod;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The backend the process's device compiles for.
fn backend() -> Result<rerail_ukl::Backend, String> {
    #[cfg(target_os = "macos")]
    {
        crate::shared::get().ok_or("no device")?;
        Ok(rerail_ukl::Backend::Metal { version: 310 })
    }
    #[cfg(target_os = "linux")]
    {
        Ok(rerail_ukl::Backend::Cuda { sm: crate::shared::get().ok_or("no device")?.device.sm })
    }
}

/// `text`, a UKL source named `name`, preprocessed for the process's device.
pub fn preprocess(name: &str, text: &str) -> Result<String, String> {
    let path = format!("/<kernels>/{name}");
    rerail_ukl::Preprocess::new(backend()?).file(&path, text).run(&path).map_err(|e| e.to_string())
}

/// A compiled program: its library, and its functions as they are asked for.
pub struct Program {
    name: &'static str,
    library: ComputeFunctionLibrary,
    functions: Mutex<HashMap<String, &'static ComputeFunction>>,
}

/// The program `text`, compiled the first time `name` is asked for. A failure is the same failure every time.
pub fn program(name: &'static str, text: &'static str) -> Result<&'static Program, String> {
    static PROGRAMS: OnceLock<Mutex<HashMap<&'static str, Result<&'static Program, String>>>> = OnceLock::new();
    let mut programs = PROGRAMS.get_or_init(Default::default).lock().unwrap();
    programs
        .entry(name)
        .or_insert_with(|| {
            let shared = crate::shared::get().ok_or("no device")?;
            let source = preprocess(name, text)?;
            let library = ComputeFunctionLibrary::compile_from_source(&shared.device, &source, name)?;
            Ok(&*Box::leak(Box::new(Program { name, library, functions: Default::default() })))
        })
        .clone()
}

impl Program {
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

    /// A value row (`ARGS`: a `#[repr(C)]` struct laid out as the kernel's).
    pub fn value<T: Pod>(mut self, name: &str, v: &T) -> Self {
        if let Some(i) = self.index(name) {
            // SAFETY: a Pod value's bytes, all of them initialized.
            let bytes = unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), size_of::<T>()) };
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

    /// Launched: `grid` threadgroups of `block` threads.
    pub fn run(mut self, grid: &[u64], block: &[u64]) -> Result<(), String> {
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        self.ctx.launch(grid, block)
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
