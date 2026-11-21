//! The CPU device's kernels run by the vector path's interpreter (decision 0567, step 2), with `RERAIL_CPU_EXEC=vec`:
//! the source preprocessed as OpenCL C (`rerail_ukl_vec`), lowered to its kernel tape, each kernel vectorized on its
//! first launch (refused there if it breaks the barrier rule) and each launch interpreted at 32 lanes,
//! where the CPU backend otherwise compiles the source with the host's compiler. Its kernels' arguments are their
//! rows, as a compiled library's are (`rerail_device::rows`), read from the kernel tape's constants and types.

use crate::compute_kernel_cpu::{CpuComputeFunctionLibrary, CpuInterpreter};
use crate::device::{ComputeFunctionLibrary, Device};
use rerail_ukl_vec::lower::Program;
use rerail_ukl_vec::run::Arg;
use rerail_ukl_vec::ty::Ty;
use rerail_ukl_vec::vector::Kernel;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Whether `device`'s kernels are interpreted: the CPU's, with `RERAIL_CPU_EXEC=vec`.
pub fn on(device: &Device) -> bool {
    matches!(device, Device::Cpu(_)) && std::env::var("RERAIL_CPU_EXEC").is_ok_and(|v| v == "vec")
}

/// The program `text` at `path`, specialized by `defines`, as an interpreted library named `name`.
pub fn library(name: &str, path: &str, text: &str, defines: &[(&str, &str)]) -> Result<ComputeFunctionLibrary, String> {
    let mut p = rerail_ukl_vec::preprocess().file(path, text);
    for (n, v) in defines {
        p = p.define(n, Some(v));
    }
    let program = rerail_ukl_vec::program(&p, path).map_err(|e| format!("{name}: {e}"))?;
    Ok(ComputeFunctionLibrary::Cpu(CpuComputeFunctionLibrary::interpreted(Arc::new(Interpreted { program, kernels: Mutex::default() }), name)))
}

struct Interpreted {
    program: Program,
    /// Each kernel's vector tape, made on its first launch, or why it is refused.
    kernels: Mutex<HashMap<String, Result<Arc<Kernel>, String>>>,
}

/// A type's size, alignment and code (`ukl_type_code`: `u`, `i`, `f`, `d`, `b`; another for a structure).
fn code(p: &Program, t: &Ty) -> [u64; 3] {
    let (size, align) = t.layout(&p.structs);
    let c = match t {
        Ty::Bool => b'b',
        Ty::Int { signed: true, .. } => b'i',
        Ty::Int { .. } => b'u',
        Ty::Float(32) => b'f',
        Ty::Float(_) => b'd',
        _ => b's',
    };
    [size, align, u64::from(c)]
}

/// Whether a row of `access` is an argument (`rows::arguments`): its buffers, its `ARGS`, literals and threadgroup
/// rows.
fn is_argument(access: &str) -> bool {
    matches!(access, "ROBUFFER" | "CONSTBUFFER" | "RWBUFFER" | "WOBUFFER" | "ARGS" | "LITERAL" | "TUNEABLE" | "RWLOCAL")
}

impl Interpreted {
    fn params(&self, kernel: &str) -> Result<&str, String> {
        self.program.string(&format!("{kernel}__ukl_params")).ok_or_else(|| format!("{kernel}: no rows ({kernel}__ukl_params)"))
    }
}

impl CpuInterpreter for Interpreted {
    fn rows(&self, kernel: &str) -> Result<(String, Vec<u64>), String> {
        let text = self.params(kernel)?;
        let fields = rerail_device::rows::split_top(text);
        let mut rows = Vec::new();
        for row in fields.chunks(4) {
            let ty = row.get(1).copied().unwrap_or("");
            let t = self.program.named(ty).ok_or_else(|| format!("{kernel}: a row's type {ty} is no type of the program"))?;
            rows.extend(code(&self.program, &t));
        }
        Ok((text.to_string(), rows))
    }

    fn layout(&self, ty: &str) -> Result<(String, Vec<u64>), String> {
        let names = self.program.string(&format!("{ty}__ukl_fields")).ok_or_else(|| format!("{ty} is an ARGS row's type with no UKL_REFLECT_STRUCT"))?;
        let Some(Ty::Struct(k)) = self.program.named(ty) else { return Err(format!("{ty} is not a structure")) };
        let s = &self.program.structs[k as usize];
        let mut l = vec![s.size, s.align];
        for n in rerail_device::rows::split_top(names) {
            let f = s.fields.iter().find(|f| f.0 == n).ok_or_else(|| format!("{ty} has no field {n}"))?;
            let [size, _, c] = code(&self.program, &f.1);
            l.extend([f.2, size, c]);
        }
        Ok((names.to_string(), l))
    }

    fn launch(&self, kernel: &str, params: &[Vec<u64>], grid: [u32; 3], block: [u32; 3], shared: u64) -> Result<(), String> {
        let k = self.program.kernel(kernel).ok_or_else(|| format!("no kernel {kernel}"))?;
        let rows = rerail_device::rows::split_top(self.params(kernel)?);
        let mut args = Vec::with_capacity(params.len() + 2);
        let mut at = 0;
        for row in rows.chunks(4) {
            if !is_argument(row[0]) {
                continue;
            }
            let words = params.get(at).ok_or_else(|| format!("{kernel}: argument {at} not given"))?;
            args.push(if row[0] == "RWLOCAL" { Arg::Local(words[0]) } else { Arg::Bytes(words.iter().flat_map(|w| w.to_le_bytes()).collect()) });
            at += 1;
        }
        // The entry's indirect counts (`__ukl_groups`, `__ukl_groups_at`): none, the device read them (`launch_indirect`).
        args.push(Arg::Bytes(vec![0; 8]));
        args.push(Arg::Bytes(vec![0; 4]));
        let vk = self.kernels.lock().map_err(|_| "a kernel's vectorizing panicked".to_string())?.entry(kernel.to_string()).or_insert_with(|| rerail_ukl_vec::vector::vectorize(&self.program, k).map(Arc::new).map_err(|e| e.0)).clone()?;
        rerail_ukl_vec::run::run(&self.program, &vk, &args, grid, block, shared).map_err(|e| e.0)
    }
}
