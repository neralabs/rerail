//! compute_kernel_metal.cc (and the parts of compute_kernel_metal.h and compute_kernel_grid.h they define), in Rust
//! (in progress: what is here has been cut from the .cc, the rest is still there).
//!
//! Jeremy Barnes, 21 July 2021. Copyright (c) 2021 Jeremy Barnes. This file is part of MLDB. Copyright 2016 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `accel/src/forest/mldb/`).
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

use metal::{ArgumentRef, ArrayTypeRef, ComputePipelineDescriptor, ComputePipelineState, DeviceRef, Function, Library, MTLArgumentAccess, MTLArgumentType, MTLDataType, MTLPipelineOption, StructTypeRef};

/// `MemoryRegionAccess`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryRegionAccess {
    Read,
    Write,
    ReadWrite,
}

fn convert_access(access: MTLArgumentAccess) -> MemoryRegionAccess {
    match access {
        MTLArgumentAccess::ReadOnly => MemoryRegionAccess::Read,
        MTLArgumentAccess::ReadWrite => MemoryRegionAccess::ReadWrite,
        MTLArgumentAccess::WriteOnly => MemoryRegionAccess::Write,
    }
}

/// A basic (scalar) type: its name as the kernel registrations spell it (`f32`, `u32`, ...) and its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BasicType {
    pub name: &'static str,
    pub width: u64,
}

/// The layout of a kernel argument's type (MLDB's value descriptions, as `getKernelType` built them).
#[derive(Clone, Debug, PartialEq)]
pub enum KernelBaseType {
    /// A basic type with its SIMD shape (`[1]` scalar, `[2]` a float2, `[2,2]` a float2x2, ...).
    Basic { ty: BasicType, simd: Vec<u32> },
    /// A structure: its fields at their offsets.
    Struct { name: String, align: Option<u64>, width: Option<u64>, fields: Vec<StructField> },
    /// A fixed-length array.
    Array { name: String, width: u64, align: u64, length: u64, element: Box<KernelBaseType> },
    /// Opaque memory of a width and alignment (threadgroup memory).
    Atom { name: String, width: u64, align: u64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct StructField {
    pub name: String,
    pub offset: u64,
    pub ty: KernelBaseType,
}

impl KernelBaseType {
    pub fn width(&self) -> u64 {
        match self {
            KernelBaseType::Basic { ty, simd } => ty.width * simd.iter().map(|&s| s as u64).product::<u64>(),
            KernelBaseType::Struct { width, fields, .. } => width.unwrap_or_else(|| fields.iter().map(|f| f.offset + f.ty.width()).max().unwrap_or(0)),
            KernelBaseType::Array { width, .. } | KernelBaseType::Atom { width, .. } => *width,
        }
    }

    pub fn align(&self) -> u64 {
        match self {
            KernelBaseType::Basic { ty, .. } => ty.width,
            KernelBaseType::Struct { align, fields, .. } => align.unwrap_or_else(|| fields.iter().map(|f| f.ty.align()).max().unwrap_or(1)),
            KernelBaseType::Array { align, .. } | KernelBaseType::Atom { align, .. } => *align,
        }
    }

    /// The field called `name`, for a structure.
    pub fn field(&self, name: &str) -> Option<&StructField> {
        match self {
            KernelBaseType::Struct { fields, .. } => fields.iter().find(|f| f.name == name),
            _ => None,
        }
    }
}

/// `ComputeKernelType`: a base type, the array dimensions over it (one, unsized, for a pointer to a basic type),
/// and the access.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputeKernelType {
    pub base_type: KernelBaseType,
    pub dims: usize,
    pub access: MemoryRegionAccess,
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
        other => return Err(format!("getKernelTypeFromDataType: unknown type {other:?}")),
    })
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

/// `GridComputeFunctionArgumentDisposition`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridComputeFunctionArgumentDisposition {
    Buffer,
    Threadgroup,
}

/// `GridComputeFunctionArgument`: one argument of a compute function, as its reflection describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct GridComputeFunctionArgument {
    pub name: String,
    pub compute_function_arg_index: u64,
    pub ty: ComputeKernelType,
    pub disposition: GridComputeFunctionArgumentDisposition,
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
        let (state, reflection) = device.new_compute_pipeline_state_with_reflection(&desc, options).map_err(|e| format!("Error making pipeline state for function: {e}"))?;
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
    /// Compiles (preprocessed) source on the device.
    pub fn compile_from_source(device: &DeviceRef, preprocessed_source: &str, file_name_to_appear_in_error_messages: &str) -> Result<Self, String> {
        let lib = objc::rc::autoreleasepool(|| device.new_library_with_source(preprocessed_source, &metal::CompileOptions::new()));
        let mtl_library = lib.map_err(|e| format!("Error compiling Metal library from source ({file_name_to_appear_in_error_messages}): {e}"))?;
        Ok(MetalComputeFunctionLibrary { mtl_library })
    }

    pub fn get_function(&self, device: &DeviceRef, function_name: &str) -> Result<MetalComputeFunction, String> {
        let function = self.mtl_library.get_function(function_name, None).map_err(|e| format!("Error getting function '{function_name}' from Metal library ({}): {e}", self.mtl_library.function_names().join(", ")))?;
        MetalComputeFunction::new(device, function)
    }
}

/// A queue of operations on the shared device (`MetalComputeQueue`): kernel launches, each in its own compute
/// encoder, and copies and fills, each in its own blit encoder, all in order on the current command buffer.
/// `flush` commits it (and starts another) and gives an event to await; `finish` flushes and waits.
pub struct MetalComputeQueue {
    pub queue: metal::CommandQueue,
    pub command_buffer: metal::CommandBuffer,
}

/// A committed command buffer (`MetalComputeEvent`).
pub struct MetalComputeEvent {
    pub command_buffer: metal::CommandBuffer,
}

impl MetalComputeEvent {
    /// Waits for the work to complete; an error if it did not.
    pub fn await_(&self) -> Result<(), String> {
        self.command_buffer.wait_until_completed();
        match self.command_buffer.status() {
            metal::MTLCommandBufferStatus::Completed => Ok(()),
            s => Err(format!("Metal command buffer: {s:?}")),
        }
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

/// Runs `f` in an autorelease pool: the Metal objects it makes come autoreleased, and a thread with no pool never
/// releases them. Command buffers left that way stay counted against the queue (64 at most), and the queue's next
/// command buffer then waits for ever (upstream ran on threads whose run loop drained a pool).
fn pooled<R>(f: impl FnOnce() -> R) -> R {
    objc::rc::autoreleasepool(f)
}

impl MetalComputeQueue {
    pub fn new(queue: &metal::CommandQueueRef) -> Self {
        MetalComputeQueue { queue: queue.to_owned(), command_buffer: pooled(|| queue.new_command_buffer().to_owned()) }
    }

    /// `newBindContext`: an encoder for one launch of `function`.
    pub fn new_bind_context(&self, function: &MetalComputeFunction) -> MetalBindContext {
        let encoder = pooled(|| self.command_buffer.new_compute_command_encoder().to_owned());
        encoder.set_compute_pipeline_state(&function.compute_pipeline_state);
        MetalBindContext { encoder }
    }

    /// `enqueueCopyFromHost`: the bytes are staged now, and copied into `dst` at `dst_offset` in queue order.
    pub fn enqueue_copy_from_host(&self, dst: &metal::BufferRef, dst_offset: u64, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let device = dst.device();
        pooled(|| {
            let staging = device.new_buffer_with_data(bytes.as_ptr().cast(), bytes.len() as u64, metal::MTLResourceOptions::StorageModeShared);
            let blit = self.command_buffer.new_blit_command_encoder();
            blit.copy_from_buffer(&staging, 0, dst, dst_offset, bytes.len() as u64);
            blit.end_encoding();
        });
    }

    /// `enqueueFillArray`: `count` copies of `value` into `dst` from element `offset` (all of it when `count` is
    /// `None`).
    pub fn enqueue_fill_array<T: crate::forest::memory_region::Pod>(&self, dst: &metal::BufferRef, value: &T, offset: usize, count: Option<usize>) {
        let size = std::mem::size_of::<T>();
        let count = count.unwrap_or(dst.length() as usize / size - offset);
        let one = unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size) };
        if one.iter().all(|&b| b == 0) {
            pooled(|| {
                let blit = self.command_buffer.new_blit_command_encoder();
                blit.fill_buffer(dst, metal::NSRange::new((offset * size) as u64, (count * size) as u64), 0);
                blit.end_encoding();
            });
            return;
        }
        let bytes: Vec<u8> = one.iter().copied().cycle().take(count * size).collect();
        self.enqueue_copy_from_host(dst, (offset * size) as u64, &bytes);
    }

    /// `enqueueCopyBetweenDeviceRegions`.
    pub fn enqueue_copy_between_device_regions(&self, src: &metal::BufferRef, dst: &metal::BufferRef, src_offset: u64, dst_offset: u64, length: u64) {
        pooled(|| {
            let blit = self.command_buffer.new_blit_command_encoder();
            blit.copy_from_buffer(src, src_offset, dst, dst_offset, length);
            blit.end_encoding();
        });
    }

    /// Commits the work so far; its event.
    pub fn flush(&mut self) -> MetalComputeEvent {
        let next = pooled(|| self.queue.new_command_buffer().to_owned());
        let cb = std::mem::replace(&mut self.command_buffer, next);
        cb.commit();
        MetalComputeEvent { command_buffer: cb }
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.flush().await_()
    }
}

/// `allocUninitializedArraySync`: device memory for `length` elements (shared storage, so the host reads it back
/// after the work completes; at least 16 bytes, as Metal will not bind an empty buffer).
pub fn alloc_uninitialized_array_sync<T>(device: &DeviceRef, length: usize) -> metal::Buffer {
    pooled(|| device.new_buffer((length * std::mem::size_of::<T>()).max(16) as u64, metal::MTLResourceOptions::StorageModeShared))
}

/// `transferToDeviceImmutable`/`manageMemoryRegionSync`: device memory holding `data`.
pub fn transfer_to_device_immutable<T: crate::forest::memory_region::Pod>(device: &DeviceRef, data: &[T]) -> metal::Buffer {
    let b = alloc_uninitialized_array_sync::<T>(device, data.len());
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr().cast::<u8>(), b.contents().cast::<u8>(), std::mem::size_of_val(data)) };
    b
}

/// `transferToHostSync`: the first `length` elements (the work writing them must have completed).
pub fn transfer_to_host_sync<T: crate::forest::memory_region::Pod + Clone>(buffer: &metal::BufferRef, length: usize) -> Vec<T> {
    assert!(length * std::mem::size_of::<T>() <= buffer.length() as usize);
    unsafe { std::slice::from_raw_parts(buffer.contents().cast::<T>(), length).to_vec() }
}

/// The bind context of one launch (`MetalBindContext`): its encoder, ended when it is dropped.
pub struct MetalBindContext {
    pub encoder: metal::ComputeCommandEncoder,
}

impl crate::forest::compute_kernel_grid::GridBindContext for MetalBindContext {
    fn set_primitive(&mut self, arg_num: u64, bytes: &[u8]) {
        self.encoder.set_bytes(arg_num, bytes.len() as u64, bytes.as_ptr().cast());
    }

    fn set_buffer(&mut self, arg_num: u64, buffer: &metal::BufferRef) {
        self.encoder.set_buffer(arg_num, Some(buffer), 0);
    }

    fn set_thread_group_memory(&mut self, arg_num: u64, n_bytes: u64) {
        self.encoder.set_threadgroup_memory_length(arg_num, n_bytes);
    }

    fn launch(&mut self, grid: &[u64], block: &[u64]) -> Result<(), String> {
        let d = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(1);
        let grid_size = metal::MTLSize::new(d(grid, 0), d(grid, 1), d(grid, 2));
        let block_size = metal::MTLSize::new(d(block, 0), d(block, 1), d(block, 2));
        self.encoder.dispatch_thread_groups(grid_size, block_size);
        Ok(())
    }
}

impl Drop for MetalBindContext {
    fn drop(&mut self) {
        self.encoder.end_encoding();
    }
}
