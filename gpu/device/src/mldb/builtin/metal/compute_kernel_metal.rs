//! compute_kernel_metal.cc (and the parts of compute_kernel_metal.h and compute_kernel_grid.h they define), in Rust
//! (in progress: what is here has been cut from the .cc, the rest is still there).
//!
//! Jeremy Barnes, 21 July 2021. Copyright (c) 2021 Jeremy Barnes. This file is part of MLDB. Copyright 2016 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `forest/src/mldb/`).
//!
//! Metal compute functions and their libraries: a library compiled from (preprocessed) source on a device, its
//! functions with their pipeline states, and each function's arguments as Metal's reflection describes them, which
//! the grid runtime binds a kernel's parameters to by name.
//!
//! Here so far: `MetalComputeQueue` (launches, copies from the host, fills, device copies, `flush` and `finish`
//! over a command buffer at a time) with its event, the array allocation and transfers over shared storage,
//! `MetalBindContext`,
//! `MetalComputeFunctionLibrary` (`compileFromSource`, `getFunction`), `MetalComputeFunction`
//! (pipeline state with reflection, `getArgumentInfo`), and `getKernelType` with the data type, struct and array
//! translations. `compileFromSourceFile`'s preprocessing step is the build step (`accel/build/forest.rs`); the
//! Metal version it queried with `xcrun metal` is fixed there, as no Metal toolchain is installed. MLDB's value
//! descriptions of the types are `ComputeKernelType`, a plain tree of the layout.

#![allow(deprecated)] // MTLArgument: the reflection the pipeline state hands back.
#![allow(unexpected_cfgs)] // `msg_send!` expands to a `cargo-clippy` cfg check.

use objc::{msg_send, sel, sel_impl};

pub use crate::compute_kernel::*;

use metal::{ArgumentRef, ArrayTypeRef, ComputePipelineDescriptor, ComputePipelineState, DeviceRef, Function, Library, MTLArgumentAccess, MTLArgumentType, MTLDataType, MTLPipelineOption, StructTypeRef};

fn convert_access(access: MTLArgumentAccess) -> MemoryRegionAccess {
    match access {
        MTLArgumentAccess::ReadOnly => MemoryRegionAccess::Read,
        MTLArgumentAccess::ReadWrite => MemoryRegionAccess::ReadWrite,
        MTLArgumentAccess::WriteOnly => MemoryRegionAccess::Write,
    }
}

fn basic(name: &'static str, width: u64, simd: &[u32]) -> KernelBaseType {
    KernelBaseType::Basic { ty: BasicType { name, width }, simd: simd.to_vec() }
}

fn get_kernel_type_from_data_type(data_type: MTLDataType) -> Result<KernelBaseType, String> {
    use MTLDataType::*;
    Ok(match data_type {
        None | Struct | Array => return Err("getKernelTypeFromDataType: not a basic type".into()),
        Float => basic("f32", 4, &[1]),
        Float2 => basic("f32", 4, &[2]),
        Float3 => basic("f32", 4, &[3]),
        Float4 => basic("f32", 4, &[4]),
        Float2x2 => basic("f32", 4, &[2, 2]),
        Float2x3 => basic("f32", 4, &[2, 3]),
        Float2x4 => basic("f32", 4, &[2, 4]),
        Float3x2 => basic("f32", 4, &[3, 2]),
        Float3x3 => basic("f32", 4, &[3, 3]),
        Float3x4 => basic("f32", 4, &[3, 4]),
        Float4x2 => basic("f32", 4, &[4, 2]),
        Float4x3 => basic("f32", 4, &[4, 3]),
        Float4x4 => basic("f32", 4, &[4, 4]),
        Half => basic("f16", 2, &[1]),
        Half2 => basic("f16", 2, &[2]),
        Half3 => basic("f16", 2, &[3]),
        Half4 => basic("f16", 2, &[4]),
        Half2x2 => basic("f16", 2, &[2, 2]),
        Half2x3 => basic("f16", 2, &[2, 3]),
        Half2x4 => basic("f16", 2, &[2, 4]),
        Half3x2 => basic("f16", 2, &[3, 2]),
        Half3x3 => basic("f16", 2, &[3, 3]),
        Half3x4 => basic("f16", 2, &[3, 4]),
        Half4x2 => basic("f16", 2, &[4, 2]),
        Half4x3 => basic("f16", 2, &[4, 3]),
        Half4x4 => basic("f16", 2, &[4, 4]),
        Int => basic("i32", 4, &[1]),
        Int2 => basic("i32", 4, &[2]),
        Int3 => basic("i32", 4, &[3]),
        Int4 => basic("i32", 4, &[4]),
        UInt => basic("u32", 4, &[1]),
        UInt2 => basic("u32", 4, &[2]),
        UInt3 => basic("u32", 4, &[3]),
        UInt4 => basic("u32", 4, &[4]),
        Short => basic("i16", 2, &[1]),
        Short2 => basic("i16", 2, &[2]),
        Short3 => basic("i16", 2, &[3]),
        Short4 => basic("i16", 2, &[4]),
        UShort => basic("u16", 2, &[1]),
        UShort2 => basic("u16", 2, &[2]),
        UShort3 => basic("u16", 2, &[3]),
        UShort4 => basic("u16", 2, &[4]),
        Char => basic("i8", 1, &[1]),
        Char2 => basic("i8", 1, &[2]),
        Char3 => basic("i8", 1, &[3]),
        Char4 => basic("i8", 1, &[4]),
        UChar => basic("u8", 1, &[1]),
        UChar2 => basic("u8", 1, &[2]),
        UChar3 => basic("u8", 1, &[3]),
        UChar4 => basic("u8", 1, &[4]),
        Bool => basic("bool", 1, &[1]),
        Bool2 => basic("bool", 1, &[2]),
        Bool3 => basic("bool", 1, &[3]),
        Bool4 => basic("bool", 1, &[4]),
        Long => basic("i64", 8, &[1]),
        Long2 => basic("i64", 8, &[2]),
        Long3 => basic("i64", 8, &[3]),
        Long4 => basic("i64", 8, &[4]),
        ULong => basic("u64", 8, &[1]),
        ULong2 => basic("u64", 8, &[2]),
        ULong3 => basic("u64", 8, &[3]),
        ULong4 => basic("u64", 8, &[4]),
        // A device pointer a structure holds (a directory's tape): its address, which the host writes
        // (`Buffer::device_address`), as the CUDA and CPU shims' rows give it.
        Pointer => basic("u64", 8, &[1]),
        other => return Err(format!("getKernelTypeFromDataType: unknown type {other:?}")),
    })
}

/// The name of the device's target (decision 0645, `rerail_ukl::Target`): Metal's highest tier the device has,
/// `apple10` on Apple's GPU family 10 (the M5: Metal 4's `matmul2d`), else `apple7`. `RERAIL_METAL_MATRIX_UNITS=0`
/// takes `apple7` on any device (`RERAIL_METAL_UNITS` is decision 0571's).
pub fn target_name(device: &DeviceRef) -> &'static str {
    let off = std::env::var("RERAIL_METAL_MATRIX_UNITS").is_ok_and(|v| v == "0");
    // `MTLGPUFamilyApple10` (1010), which the metal crate does not name.
    let apple10: bool = !off && unsafe { msg_send![device, supportsFamily: 1010_i64] };
    if apple10 { "apple10" } else { "apple7" }
}

// `StructMemberRef::struct_type`/`array_type` and `ArrayTypeRef::element_struct_type`/`element_array_type` in the
// metal crate return the (uninhabited) Objective-C types by value; these ask for the same properties by reference.
fn member_struct_type(m: &metal::StructMemberRef) -> &StructTypeRef {
    unsafe { msg_send![m, structType] }
}
fn member_array_type(m: &metal::StructMemberRef) -> &ArrayTypeRef {
    unsafe { msg_send![m, arrayType] }
}
fn element_struct_type(a: &ArrayTypeRef) -> &StructTypeRef {
    unsafe { msg_send![a, elementStructType] }
}
fn element_array_type(a: &ArrayTypeRef) -> &ArrayTypeRef {
    unsafe { msg_send![a, elementArrayType] }
}

fn get_kernel_type_from_struct_type(struct_type: &StructTypeRef, name: &str, align: Option<u64>, width: Option<u64>) -> Result<KernelBaseType, String> {
    let members = struct_type.members();
    let mut fields = Vec::new();
    for i in 0..members.count() {
        let member = members.object_at(i).ok_or("struct member")?;
        let member_name = member.name().to_string();
        let sub = format!("{name}__m__{member_name}");
        let ty = match member.data_type() {
            MTLDataType::None => return Err("Can't handle void (MtlDataType::None) types".into()),
            MTLDataType::Struct => get_kernel_type_from_struct_type(member_struct_type(member), &sub, None, None)?,
            MTLDataType::Array => get_kernel_type_from_array_type(member_array_type(member), &sub, None, None)?,
            dt => get_kernel_type_from_data_type(dt)?,
        };
        fields.push(StructField { name: member_name, offset: member.offset(), ty });
    }
    Ok(KernelBaseType::Struct { name: name.to_string(), align, width, fields })
}

fn get_kernel_type_from_array_type(array_type: &ArrayTypeRef, name: &str, align: Option<u64>, width: Option<u64>) -> Result<KernelBaseType, String> {
    let length = array_type.array_length();
    let stride = array_type.stride();
    let element = match array_type.element_type() {
        MTLDataType::None => return Err("Can't handle void (MtlDataType::None) types".into()),
        MTLDataType::Struct => get_kernel_type_from_struct_type(element_struct_type(array_type), &format!("{name}__ul__"), None, Some(stride))?,
        MTLDataType::Array => get_kernel_type_from_array_type(element_array_type(array_type), &format!("{name}__ul__"), None, Some(stride))?,
        dt => get_kernel_type_from_data_type(dt)?,
    };
    let width = match width {
        None => length * stride,
        Some(w) => {
            if element.width() != stride || element.width() * length != w {
                return Err(format!("{name}: array of {length} x {} does not fill {w} bytes at stride {stride}", element.width()));
            }
            w
        }
    };
    let align = align.unwrap_or_else(|| element.align());
    Ok(KernelBaseType::Array { name: name.to_string(), width, align, length, element: Box::new(element) })
}

/// Parses a Metal kernel argument info structure, and turns it into a compute kernel type.
fn get_kernel_type(arg: &ArgumentRef) -> Result<ComputeKernelType, String> {
    let name = format!("__metal_arg_{}", arg.name());
    let (base_type, dims) = match arg.type_() {
        MTLArgumentType::Buffer => {
            let align = arg.buffer_alignment();
            let width = arg.buffer_data_size();
            match arg.buffer_data_type() {
                MTLDataType::None => return Err("Can't handle void (MtlDataType::None) types".into()),
                MTLDataType::Struct => (get_kernel_type_from_struct_type(arg.buffer_struct_type(), &name, Some(align), Some(width))?, 0),
                MTLDataType::Array => return Err("Can't handle arrays as Metal arguments".into()),
                // If it's a basic type, it must be an array.
                dt => (get_kernel_type_from_data_type(dt)?, 1),
            }
        }
        MTLArgumentType::ThreadgroupMemory => (KernelBaseType::Atom { name, width: arg.threadgroup_memory_data_size(), align: arg.threadgroup_memory_alignment() }, 0),
        _ => return Err("Not implemented: non-buffer and non-thread group argument types".into()),
    };
    Ok(ComputeKernelType { base_type, dims, access: convert_access(arg.access()) })
}

fn get_disposition(arg_type: MTLArgumentType) -> Result<GridComputeFunctionArgumentDisposition, String> {
    match arg_type {
        MTLArgumentType::Buffer => Ok(GridComputeFunctionArgumentDisposition::Buffer),
        MTLArgumentType::ThreadgroupMemory => Ok(GridComputeFunctionArgumentDisposition::Threadgroup),
        _ => Err("Unimplemented: Metal sampler or texture arguments".into()),
    }
}

/// A function of a library, with its pipeline state and its arguments.
pub struct MetalComputeFunction {
    pub function: Function,
    pub compute_pipeline_state: ComputePipelineState,
    arguments: Vec<GridComputeFunctionArgument>,
}

impl MetalComputeFunction {
    pub fn new(device: &DeviceRef, function: Function) -> Result<Self, String> {
        let desc = ComputePipelineDescriptor::new();
        desc.set_compute_function(Some(&function));
        let options = MTLPipelineOption::ArgumentInfo | MTLPipelineOption::BufferTypeInfo;
        // A debugging aid: `RERAIL_METAL_BINARY_ARCHIVE` names a binary archive whose GPU code a pipeline is made
        // from, where it holds the function's (a compiler's code run in place of this OS's, `metal-tt`); a function
        // it does not hold is compiled as usual. Each says which.
        if let Some(path) = std::env::var_os("RERAIL_METAL_BINARY_ARCHIVE") {
            let ad = metal::BinaryArchiveDescriptor::new();
            ad.set_url(&metal::URL::new_with_string(&format!("file://{}", path.to_string_lossy())));
            match device.new_binary_archive_with_descriptor(&ad) {
                Ok(archive) => {
                    desc.set_binary_archives(&[&archive]);
                    match device.new_compute_pipeline_state_with_reflection(&desc, options | MTLPipelineOption::FailOnBinaryArchiveMiss) {
                        Ok((state, reflection)) => {
                            eprintln!("RERAIL_METAL_BINARY_ARCHIVE: {}: from the archive", function.name());
                            return Self::from_reflection(function, state, &reflection);
                        }
                        Err(e) => eprintln!("RERAIL_METAL_BINARY_ARCHIVE: {}: not in the archive ({e}), compiled", function.name()),
                    }
                    desc.set_binary_archives(&[]);
                }
                Err(e) => eprintln!("RERAIL_METAL_BINARY_ARCHIVE: {}: {e}", path.to_string_lossy()),
            }
        }
        let (state, reflection) = device.new_compute_pipeline_state_with_reflection(&desc, options).map_err(|e| format!("Error making pipeline state for function: {e}"))?;
        Self::from_reflection(function, state, &reflection)
    }

    /// A function whose pipeline was made elsewhere (the engine's kernel store, from its binary archives), with
    /// the reflection made with it (`ArgumentInfo | BufferTypeInfo`).
    pub fn from_reflection(function: Function, state: ComputePipelineState, reflection: &metal::ComputePipelineReflectionRef) -> Result<Self, String> {
        let args = reflection.arguments();
        let mut arguments = Vec::new();
        for i in 0..args.count() {
            let arg = args.object_at(i).ok_or("argument")?;
            arguments.push(GridComputeFunctionArgument { name: arg.name().to_string(), disposition: get_disposition(arg.type_())?, ty: get_kernel_type(arg)?, compute_function_arg_index: arg.index() });
        }
        Ok(MetalComputeFunction { function, compute_pipeline_state: state, arguments })
    }

    pub fn get_argument_info(&self) -> &[GridComputeFunctionArgument] {
        &self.arguments
    }
}

/// A library of compute functions on a device.
pub struct MetalComputeFunctionLibrary {
    pub mtl_library: Library,
}

impl MetalComputeFunctionLibrary {
    /// The names of the AIR intrinsics `preprocessed_source` compiles to, one a line, for a test to read which
    /// instructions a kernel took (decision 0617, R7): the source compiled as a dynamic library, serialized to a
    /// file, and the `air.` names read from its bitcode's string table. (A Mac with no Metal toolchain has no
    /// `xcrun metal` to print the IR; the device's compiler is the one to ask.)
    pub fn native_code(device: &DeviceRef, preprocessed_source: &str, file_name_to_appear_in_error_messages: &str) -> Result<Option<String>, String> {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rerail-native-code-{}-{n}.metallib", std::process::id()));
        let bytes = objc::rc::autoreleasepool(|| -> Result<Vec<u8>, String> {
            let options = metal::CompileOptions::new();
            options.set_library_type(metal::MTLLibraryType::Dynamic);
            options.set_install_name(&format!("@executable_path/{}", path.file_name().unwrap().to_string_lossy()));
            let lib = device.new_library_with_source(preprocessed_source, &options).map_err(|e| format!("compiling {file_name_to_appear_in_error_messages} as a dynamic library: {e}"))?;
            let dylib = device.new_dynamic_library(&lib).map_err(|e| format!("{file_name_to_appear_in_error_messages}'s dynamic library: {e}"))?;
            // `serializeToURL:error:` sent here: metal-rs's `URL::new_with_string` owns an autoreleased NSURL, and
            // its release with the pool's crashed. This NSURL is the pool's alone.
            let file = std::ffi::CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
            let ok: bool = unsafe {
                use objc::runtime::Object;
                let s: *mut Object = objc::msg_send![objc::class!(NSString), stringWithUTF8String: file.as_ptr()];
                let url: *mut Object = objc::msg_send![objc::class!(NSURL), fileURLWithPath: s];
                let mut err: *mut Object = std::ptr::null_mut();
                let done: objc::runtime::BOOL = objc::msg_send![&*dylib, serializeToURL: url error: &mut err];
                done != objc::runtime::NO
            };
            if !ok {
                return Err(format!("serializing {file_name_to_appear_in_error_messages}'s dynamic library failed"));
            }
            std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
        });
        let _ = std::fs::remove_file(&path);
        let bytes = bytes?;
        let mut names: Vec<String> = bytes
            .split(|b| !(b.is_ascii_alphanumeric() || *b == b'.' || *b == b'_'))
            .filter_map(|w| std::str::from_utf8(w).ok())
            .filter_map(|w| w.find("air.").map(|at| w[at..].to_owned()))
            .collect();
        names.sort();
        names.dedup();
        Ok(Some(names.join("\n")))
    }

    /// Compiles (preprocessed) source on the device.
    pub fn compile_from_source(device: &DeviceRef, preprocessed_source: &str, file_name_to_appear_in_error_messages: &str) -> Result<Self, String> {
        // A measurement aid: `RERAIL_DEVICE_TRACE` prints where each library is compiled.
        if std::env::var_os("RERAIL_DEVICE_TRACE").is_some() {
            eprintln!("RERAIL_DEVICE_TRACE a library compiled, {file_name_to_appear_in_error_messages} (driver {} MB):\n{}", device.current_allocated_size() >> 20, std::backtrace::Backtrace::force_capture());
        }
        // A debugging aid: `RERAIL_METAL_SOURCE_DIR` keeps each library's source as Metal compiles it, for an offline
        // compile of the same text (its AIR and GPU code, `xcrun metal`, `metal-objdump`).
        if let Some(dir) = std::env::var_os("RERAIL_METAL_SOURCE_DIR") {
            let name: String = file_name_to_appear_in_error_messages.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect();
            if let Err(e) = std::fs::write(std::path::Path::new(&dir).join(format!("{name}.metal")), preprocessed_source) {
                eprintln!("RERAIL_METAL_SOURCE_DIR: {name}: {e}");
            }
        }
        let lib = objc::rc::autoreleasepool(|| device.new_library_with_source(preprocessed_source, &metal::CompileOptions::new()));
        let mtl_library = lib.map_err(|e| format!("Error compiling Metal library from source ({file_name_to_appear_in_error_messages}): {e}"))?;
        // Compiled: the device layer keeps no Metal form (the engine's Metal kernels are `rerail_accel::gpu::kernels`').
        crate::store::count(false);
        Ok(MetalComputeFunctionLibrary { mtl_library })
    }

    /// A library of AIR compiled ahead of time for a tier this device runs (decision 0645: a shipped kernel).
    pub fn from_compiled(device: &DeviceRef, code: &[u8]) -> Result<Self, String> {
        let mtl_library = objc::rc::autoreleasepool(|| device.new_library_with_data(code)).map_err(|e| format!("a compiled Metal library does not load: {e}"))?;
        crate::store::count(true);
        Ok(MetalComputeFunctionLibrary { mtl_library })
    }

    pub fn get_function(&self, device: &DeviceRef, function_name: &str) -> Result<MetalComputeFunction, String> {
        let function = self.mtl_library.get_function(function_name, None).map_err(|e| format!("Error getting function '{function_name}' from Metal library ({}): {e}", self.mtl_library.function_names().join(", ")))?;
        MetalComputeFunction::new(device, function)
    }
}

/// The Metal language standard a text prepared for `__METAL_VERSION__` `version` is compiled with ahead of time, and
/// the oldest macOS its library loads on. The device's own compile takes the newest its system has: a text of
/// `apple7` (310) uses Metal 3.2's coherent memory and thread scopes (UKL's atomics), so Metal 3.2 and macOS 15;
/// `apple10`'s (400) Metal 4 and macOS 26.
fn standard(version: u32) -> (&'static str, &'static str) {
    if version >= 400 { ("metal4.0", "26.0") } else { ("metal3.2", "15.0") }
}

/// Compiles Metal source (prepared for `__METAL_VERSION__` `version`) ahead of time to a library of AIR, without a
/// device (decision 0645 step 3), by Xcode's Metal compiler (`xcrun metal`, `xcrun metallib`).
pub fn compile_offline(version: u32, source: &str, name: &str) -> Result<Vec<u8>, String> {
    let (std, macos) = standard(version);
    let stem: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let dir = std::env::temp_dir().join(format!("rerail-ship-{}-{}-{stem}", std::process::id(), crate::store::unique()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (src, air, lib) = (dir.join("k.metal"), dir.join("k.air"), dir.join("k.metallib"));
    let run = |args: &[&std::ffi::OsStr]| -> Result<(), String> {
        let out = std::process::Command::new("xcrun").args(args).output().map_err(|e| format!("xcrun: {e} (shipping Metal kernels needs Xcode's Metal compiler)"))?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            let lines: Vec<&str> = err.lines().filter(|l| l.contains("error")).take(20).collect();
            Err(format!("{name} ({std}): {} (the Metal Toolchain: `xcodebuild -downloadComponent MetalToolchain`)", if lines.is_empty() { err.trim().to_owned() } else { lines.join("\n") }))
        }
    };
    let result = std::fs::write(&src, source).map_err(|e| e.to_string()).and_then(|()| {
        let (std, min) = (format!("-std={std}"), format!("-mmacosx-version-min={macos}"));
        // The device's compile (`newLibraryWithSource:`) includes Metal's standard library itself; a prepared text
        // leaves it to the compiler (UKL's shim, under `__UKL_METAL_PREPROCESSING`).
        let args: [&std::ffi::OsStr; 12] =
            ["-sdk".as_ref(), "macosx".as_ref(), "metal".as_ref(), std.as_ref(), min.as_ref(), "-include".as_ref(), "metal_stdlib".as_ref(), "-c".as_ref(), src.as_os_str(), "-o".as_ref(), air.as_os_str(), "-w".as_ref()];
        run(&args)?;
        run(&["-sdk".as_ref(), "macosx".as_ref(), "metallib".as_ref(), air.as_os_str(), "-o".as_ref(), lib.as_os_str()])?;
        std::fs::read(&lib).map_err(|e| e.to_string())
    });
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// A queue of operations on the shared device (`MetalComputeQueue`): kernel launches, each in its own compute
/// encoder, and copies and fills, each in its own blit encoder, all in order on the current command buffer.
/// `flush` commits it (and starts another) and gives an event to await; `finish` flushes and waits.
pub struct MetalComputeQueue {
    pub queue: metal::CommandQueue,
    pub command_buffer: metal::CommandBuffer,
    /// The command buffer's open encoder, kept across launches (a launch after a launch, a copy after a copy, in
    /// the same encoder: a serial compute encoder runs its dispatches in order, each seeing the last's writes), and
    /// ended when the other kind is needed or at `flush`.
    open: std::cell::RefCell<Open>,
    /// The command buffer's staging memory for `enqueue_copy_from_host` (filled up, then another), and how much is
    /// used: one allocation a command buffer, not one a copy. The command buffer keeps it until it completes.
    staging: std::cell::RefCell<Option<(metal::Buffer, u64)>>,
}

/// A command buffer's open encoder.
enum Open {
    None,
    Compute(metal::ComputeCommandEncoder),
    Blit(metal::BlitCommandEncoder),
}

/// The bytes a staging buffer holds (a larger copy gets its own).
const STAGING_BYTES: u64 = 64 << 10;

/// A committed command buffer (`MetalComputeEvent`).
/// The start of a failed command's message when the system stopped it, the work not at fault (`await_`).
pub const PREEMPTED: &str = "preempted: ";

pub struct MetalComputeEvent {
    pub command_buffer: metal::CommandBuffer,
}

impl MetalComputeEvent {
    /// Waits for the work to complete; an error if it did not.
    pub fn await_(&self) -> Result<(), String> {
        self.command_buffer.wait_until_completed();
        match self.command_buffer.status() {
            metal::MTLCommandBufferStatus::Completed => {
                GPU_NANOS.fetch_add((self.gpu_seconds() * 1e9) as u64, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }
            s => {
                // The command buffer's error: its code (MTLCommandBufferError: 2 a timeout, 3 a page fault, …) and text.
                let error: *mut objc::runtime::Object = unsafe { msg_send![&*self.command_buffer, error] };
                let why = if error.is_null() {
                    String::new()
                } else {
                    let code: i64 = unsafe { msg_send![error, code] };
                    let text: *mut objc::runtime::Object = unsafe { msg_send![error, localizedDescription] };
                    let utf8: *const std::os::raw::c_char = unsafe { msg_send![text, UTF8String] };
                    let text = if utf8.is_null() { String::new() } else { unsafe { std::ffi::CStr::from_ptr(utf8) }.to_string_lossy().into_owned() };
                    format!(" (code {code}: {text})")
                };
                // Stopped by the system for the display (IOGPU's `kIOGPUCommandBufferCallbackErrorImpactingInteractivity`,
                // under `MTLCommandBufferErrorInternal`), or discarded in its recovery from another command's hang
                // (`kIOGPUCommandBufferCallbackErrorInnocentVictim`): marked, so a caller may run the work again
                // (`PREEMPTED`).
                let mark = if why.contains("ImpactingInteractivity") || why.contains("InnocentVictim") { PREEMPTED } else { "" };
                Err(format!("{mark}Metal command buffer: {s:?}{why}"))
            }
        }
    }

    /// Whether the work has completed (or failed), polled.
    pub fn is_complete(&self) -> bool {
        matches!(self.command_buffer.status(), metal::MTLCommandBufferStatus::Completed | metal::MTLCommandBufferStatus::Error)
    }

    /// The seconds the GPU spent on the work, from its start to its end (`GPUStartTime`, `GPUEndTime`; the work must
    /// have completed): what the device took, whatever the host's scheduling.
    pub fn gpu_seconds(&self) -> f64 {
        use objc::{msg_send, sel, sel_impl};
        let cb: &metal::CommandBufferRef = &self.command_buffer;
        let (start, end): (f64, f64) = unsafe { (msg_send![cb, GPUStartTime], msg_send![cb, GPUEndTime]) };
        end - start
    }
}

static GPU_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The GPU seconds of every command buffer awaited so far in this process (`MetalComputeEvent::await_`): the
/// difference over a run is the device time of its kernels (training's, prediction's), however the host waited.
pub fn gpu_seconds_awaited() -> f64 {
    GPU_NANOS.load(std::sync::atomic::Ordering::Relaxed) as f64 * 1e-9
}

/// Runs `f` in an autorelease pool: the Metal objects it makes come autoreleased, and a thread with no pool never
/// releases them. Command buffers left that way stay counted against the queue (64 at most), and the queue's next
/// command buffer then waits for ever (upstream ran on threads whose run loop drained a pool).
fn pooled<R>(f: impl FnOnce() -> R) -> R {
    objc::rc::autoreleasepool(f)
}

impl MetalComputeQueue {
    pub fn new(queue: &metal::CommandQueueRef) -> Self {
        MetalComputeQueue { queue: queue.to_owned(), command_buffer: pooled(|| queue.new_command_buffer().to_owned()), open: std::cell::RefCell::new(Open::None), staging: Default::default() }
    }

    /// The open compute encoder (ending an open blit encoder first).
    fn compute_encoder(&self) -> metal::ComputeCommandEncoder {
        let mut open = self.open.borrow_mut();
        if let Open::Compute(e) = &*open {
            return e.clone();
        }
        Self::end(&mut open);
        let e = pooled(|| self.command_buffer.new_compute_command_encoder().to_owned());
        *open = Open::Compute(e.clone());
        e
    }

    /// The open blit encoder (ending an open compute encoder first).
    fn blit_encoder(&self) -> metal::BlitCommandEncoder {
        let mut open = self.open.borrow_mut();
        if let Open::Blit(e) = &*open {
            return e.clone();
        }
        Self::end(&mut open);
        let e = pooled(|| self.command_buffer.new_blit_command_encoder().to_owned());
        *open = Open::Blit(e.clone());
        e
    }

    fn end(open: &mut Open) {
        match std::mem::replace(open, Open::None) {
            Open::None => {}
            Open::Compute(e) => e.end_encoding(),
            Open::Blit(e) => e.end_encoding(),
        }
    }

    /// `newBindContext`: one launch of `function`, on the command buffer's open compute encoder.
    pub fn new_bind_context(&self, function: &MetalComputeFunction) -> MetalBindContext {
        let encoder = self.compute_encoder();
        encoder.set_compute_pipeline_state(&function.compute_pipeline_state);
        MetalBindContext { encoder }
    }

    /// `enqueueCopyFromHost`: the bytes are staged now, and copied into `dst` at `dst_offset` in queue order.
    pub fn enqueue_copy_from_host(&self, dst: &metal::BufferRef, dst_offset: u64, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let n = bytes.len() as u64;
        // The bytes at the next 16-byte boundary of the command buffer's staging memory (written now: the copy reads
        // them when the command buffer runs).
        let (staging, at) = {
            let mut st = self.staging.borrow_mut();
            let fits = st.as_ref().is_some_and(|(b, used)| used.next_multiple_of(16) + n <= b.length());
            if !fits {
                let len = n.max(STAGING_BYTES);
                *st = Some((pooled(|| dst.device().new_buffer(len, metal::MTLResourceOptions::StorageModeShared)), 0));
            }
            let (b, used) = st.as_mut().unwrap();
            let at = used.next_multiple_of(16);
            *used = at + n;
            (b.clone(), at)
        };
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), staging.contents().cast::<u8>().add(at as usize), bytes.len()) };
        let blit = self.blit_encoder();
        blit.copy_from_buffer(&staging, at, dst, dst_offset, n);
    }

    /// `enqueueFillArray`: `count` copies of `value` into `dst` from element `offset` (all of it when `count` is
    /// `None`).
    pub fn enqueue_fill_array<T: crate::Pod>(&self, dst: &metal::BufferRef, value: &T, offset: usize, count: Option<usize>) {
        let size = std::mem::size_of::<T>();
        let count = count.unwrap_or(dst.length() as usize / size - offset);
        let one = unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size) };
        if one.iter().all(|&b| b == 0) {
            self.blit_encoder().fill_buffer(dst, metal::NSRange::new((offset * size) as u64, (count * size) as u64), 0);
            return;
        }
        let bytes: Vec<u8> = one.iter().copied().cycle().take(count * size).collect();
        self.enqueue_copy_from_host(dst, (offset * size) as u64, &bytes);
    }

    /// `enqueueCopyBetweenDeviceRegions`.
    pub fn enqueue_copy_between_device_regions(&self, src: &metal::BufferRef, dst: &metal::BufferRef, src_offset: u64, dst_offset: u64, length: u64) {
        self.blit_encoder().copy_from_buffer(src, src_offset, dst, dst_offset, length);
    }

    /// Commits the work so far; its event.
    pub fn flush(&mut self) -> MetalComputeEvent {
        Self::end(&mut self.open.borrow_mut());
        // The next command buffer stages in memory of its own (this one's is read when it runs).
        *self.staging.borrow_mut() = None;
        let next = pooled(|| self.queue.new_command_buffer().to_owned());
        let cb = std::mem::replace(&mut self.command_buffer, next);
        cb.commit();
        MetalComputeEvent { command_buffer: cb }
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.flush().await_()
    }

    /// Holds the queue's later work until the host signals `event` with `value` or more (`encodeWaitForEvent`).
    pub fn wait_for_event(&self, event: &metal::SharedEventRef, value: u64) {
        self.command_buffer.encode_wait_for_event(event, value);
    }
}

/// `allocUninitializedArraySync`: device memory for `length` elements (shared storage, so the host reads it back
/// after the work completes; at least 16 bytes, as Metal will not bind an empty buffer).
pub fn alloc_uninitialized_array_sync<T>(device: &DeviceRef, length: usize) -> metal::Buffer {
    if length * std::mem::size_of::<T>() >= 4 << 20 && std::env::var_os("RERAIL_DEVICE_TRACE").is_some() {
        eprintln!("RERAIL_DEVICE_TRACE an array of {} MB (driver {} MB):\n{}", (length * std::mem::size_of::<T>()) >> 20, device.current_allocated_size() >> 20, std::backtrace::Backtrace::force_capture());
    }
    pooled(|| device.new_buffer((length * std::mem::size_of::<T>()).max(16) as u64, metal::MTLResourceOptions::StorageModeShared))
}

/// A buffer over `len` bytes of the host's memory at `ptr`, read in place (decision 0353): a no-copy buffer, whose
/// address and length Metal requires page-aligned.
///
/// # Safety
/// The memory outlives the buffer and its clones, and every use of them by queued work.
pub unsafe fn wrap_host_memory(device: &DeviceRef, ptr: *mut u8, len: usize) -> Result<metal::Buffer, String> {
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as usize;
    if !(ptr as usize).is_multiple_of(page) || !len.is_multiple_of(page) || len == 0 {
        return Err(format!("Metal: a no-copy buffer at {ptr:p} ({len} bytes) is not page-aligned"));
    }
    Ok(pooled(|| device.new_buffer_with_bytes_no_copy(ptr.cast(), len as u64, metal::MTLResourceOptions::StorageModeShared, None)))
}

/// `transferToDeviceImmutable`/`manageMemoryRegionSync`: device memory holding `data`.
pub fn transfer_to_device_immutable<T: crate::Pod>(device: &DeviceRef, data: &[T]) -> metal::Buffer {
    let b = alloc_uninitialized_array_sync::<T>(device, data.len());
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr().cast::<u8>(), b.contents().cast::<u8>(), std::mem::size_of_val(data)) };
    b
}

/// `transferToHostSync`: the first `length` elements (the work writing them must have completed).
pub fn transfer_to_host_sync<T: crate::Pod + Clone>(buffer: &metal::BufferRef, length: usize) -> Vec<T> {
    assert!(length * std::mem::size_of::<T>() <= buffer.length() as usize);
    unsafe { std::slice::from_raw_parts(buffer.contents().cast::<T>(), length).to_vec() }
}

/// The bind context of one launch (`MetalBindContext`): its encoder, ended when it is dropped.
pub struct MetalBindContext {
    pub encoder: metal::ComputeCommandEncoder,
}

impl MetalBindContext {
    pub fn set_primitive(&mut self, arg_num: u64, bytes: &[u8]) {
        self.encoder.set_bytes(arg_num, bytes.len() as u64, bytes.as_ptr().cast());
    }

    pub fn set_buffer(&mut self, arg_num: u64, buffer: &metal::BufferRef) {
        self.encoder.set_buffer(arg_num, Some(buffer), 0);
    }

    pub fn set_buffer_at(&mut self, arg_num: u64, buffer: &metal::BufferRef, offset: u64) -> Result<(), String> {
        self.encoder.set_buffer(arg_num, Some(buffer), offset);
        Ok(())
    }

    pub fn set_thread_group_memory(&mut self, arg_num: u64, n_bytes: u64) {
        self.encoder.set_threadgroup_memory_length(arg_num, n_bytes);
    }

    pub fn launch(&mut self, grid: &[u64], block: &[u64]) -> Result<(), String> {
        let d = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(1);
        let grid_size = metal::MTLSize::new(d(grid, 0), d(grid, 1), d(grid, 2));
        let block_size = metal::MTLSize::new(d(block, 0), d(block, 1), d(block, 2));
        self.encoder.dispatch_thread_groups(grid_size, block_size);
        Ok(())
    }

    pub fn use_buffer(&mut self, buffer: &metal::BufferRef) {
        self.encoder.use_resource(buffer, metal::MTLResourceUsage::Read | metal::MTLResourceUsage::Write);
    }

    /// Metal's indirect dispatch: its counts are read when it runs (`max_groups` is the other backends').
    pub fn launch_indirect(&mut self, groups: &metal::BufferRef, offset: u64, _max_groups: &[u64], block: &[u64]) -> Result<(), String> {
        let d = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(1);
        self.encoder.dispatch_thread_groups_indirect(groups, offset, metal::MTLSize::new(d(block, 0), d(block, 1), d(block, 2)));
        Ok(())
    }
}

impl Drop for MetalComputeQueue {
    /// An encoder left open is ended (Metal requires it before the command buffer goes).
    fn drop(&mut self) {
        Self::end(&mut self.open.borrow_mut());
    }
}

/// A placement sparse buffer (Metal 4, decision 0603's fewer reruns): `len` bytes of the GPU's address space, at one
/// address for its life, whose pages are real memory only as `map` maps them, each from a heap of its own. Private
/// storage (Metal refuses a shared one): the host reads none of it. The mapping is the host's, between launches: a
/// kernel uses no page past `mapped`.
pub struct Sparse {
    pub buffer: metal::Buffer,
    /// The Metal 4 queue the mappings are made on (retained), the event its updates signal, and the last value.
    queue: *mut objc::runtime::Object,
    event: metal::SharedEvent,
    signaled: std::sync::Mutex<u64>,
    heaps: std::sync::Mutex<Vec<metal::Heap>>,
    /// Its pages mapped, the first ones.
    pages: std::sync::atomic::AtomicUsize,
}

// SAFETY: the Metal objects it holds are thread-safe (Metal's), and its own state is behind a mutex or atomic.
unsafe impl Send for Sparse {}
unsafe impl Sync for Sparse {}

/// `MTLSparsePageSize64`: a sparse page of 64 KB.
const SPARSE_PAGE_SIZE: i64 = 102;
pub const SPARSE_PAGE: usize = 64 << 10;

impl Sparse {
    /// `len` bytes of address space, none of it mapped; `None` where the device has no placement sparse buffers.
    pub fn new(device: &DeviceRef, len: usize) -> Option<Sparse> {
        let len = len.next_multiple_of(SPARSE_PAGE) as u64;
        pooled(|| {
            let sel = sel!(newBufferWithLength:options:placementSparsePageSize:);
            // SAFETY: a selector check, then Metal's own calls on the device, each returning a retained object or nil.
            unsafe {
                let has: bool = msg_send![device, respondsToSelector: sel];
                if !has {
                    return None;
                }
                let b: *mut metal::MTLBuffer = msg_send![device, newBufferWithLength: len options: metal::MTLResourceOptions::StorageModePrivate placementSparsePageSize: SPARSE_PAGE_SIZE];
                if b.is_null() {
                    return None;
                }
                let queue: *mut objc::runtime::Object = msg_send![device, newMTL4CommandQueue];
                use metal::foreign_types::ForeignType;
                if queue.is_null() {
                    drop(metal::Buffer::from_ptr(b));
                    return None;
                }
                Some(Sparse {
                    buffer: metal::Buffer::from_ptr(b),
                    queue,
                    event: device.new_shared_event(),
                    signaled: std::sync::Mutex::new(0),
                    heaps: std::sync::Mutex::new(Vec::new()),
                    pages: std::sync::atomic::AtomicUsize::new(0),
                })
            }
        })
    }

    /// Its bytes mapped, from its start.
    pub fn mapped(&self) -> usize {
        self.pages.load(std::sync::atomic::Ordering::Acquire) * SPARSE_PAGE
    }

    /// Its first `bytes` mapped (whole pages, within its length): the pages past those mapped so far from a new heap,
    /// done when this returns. No work that uses the buffer may be running.
    pub fn map(&self, device: &DeviceRef, bytes: usize) -> Result<(), String> {
        let want = bytes.div_ceil(SPARSE_PAGE).min(self.buffer.length() as usize / SPARSE_PAGE);
        let have = self.pages.load(std::sync::atomic::Ordering::Acquire);
        if want <= have {
            return Ok(());
        }
        let n = want - have;
        pooled(|| {
            // SAFETY: Metal's calls: a placement heap of `n` pages, then one update of the buffer's pages `have..want`
            // to it, on the Metal 4 queue, waited for by its event.
            unsafe {
                let d: *mut objc::runtime::Object = msg_send![objc::class!(MTLHeapDescriptor), new];
                let () = msg_send![d, setType: 1i64];
                let () = msg_send![d, setStorageMode: metal::MTLStorageMode::Private];
                let () = msg_send![d, setSize: (n * SPARSE_PAGE) as u64];
                let () = msg_send![d, setMaxCompatiblePlacementSparsePageSize: SPARSE_PAGE_SIZE];
                let heap: *mut metal::MTLHeap = msg_send![device, newHeapWithDescriptor: d];
                let () = msg_send![d, release];
                if heap.is_null() {
                    return Err(format!("Metal: a placement heap of {} KB refused", (n * SPARSE_PAGE) >> 10));
                }
                use metal::foreign_types::ForeignType;
                let heap = metal::Heap::from_ptr(heap);
                #[repr(C)]
                struct Op {
                    mode: u64,
                    location: u64,
                    length: u64,
                    heap_offset: u64,
                }
                let op = Op { mode: 0, location: have as u64, length: n as u64, heap_offset: 0 };
                let heap_ref: &metal::HeapRef = &heap;
                let buffer_ref: &metal::BufferRef = &self.buffer;
                let () = msg_send![self.queue, updateBufferMappings: buffer_ref heap: heap_ref operations: &op as *const Op count: 1u64];
                let mut signaled = self.signaled.lock().unwrap_or_else(|e| e.into_inner());
                *signaled += 1;
                let event: &metal::SharedEventRef = &self.event;
                let () = msg_send![self.queue, signalEvent: event value: *signaled];
                let done: bool = msg_send![event, waitUntilSignaledValue: *signaled timeoutMS: 10_000u64];
                if !done {
                    return Err("Metal: a sparse buffer's mapping not done in 10 s".into());
                }
                self.heaps.lock().unwrap_or_else(|e| e.into_inner()).push(heap);
            }
            self.pages.store(want, std::sync::atomic::Ordering::Release);
            Ok(())
        })
    }
}

impl Drop for Sparse {
    fn drop(&mut self) {
        // SAFETY: the queue was retained at `new` and is released once.
        unsafe {
            let () = msg_send![self.queue, release];
        }
    }
}
