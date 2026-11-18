//! compute_kernel.h's types of kernel arguments (and compute_kernel_grid.h's argument dispositions), in Rust: what
//! a backend's compute function says of each of its arguments, and what the grid runtime binds a kernel's parameters
//! by. Metal fills them from its reflection (compute_kernel_metal.rs); CUDA from the descriptors UKL's CUDA shim
//! emits beside each kernel (compute_kernel_cuda.rs, Rerail decision 0400).
//!
//! Jeremy Barnes, 21 July 2021. Copyright (c) 2021 Jeremy Barnes. This file is part of MLDB. Copyright 2016 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `forest/src/mldb/`).

/// `MemoryRegionAccess`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryRegionAccess {
    Read,
    Write,
    ReadWrite,
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

