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
//! Here so far: `MetalComputeFunctionLibrary` (`compileFromSource`, `getFunction`), `MetalComputeFunction`
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
