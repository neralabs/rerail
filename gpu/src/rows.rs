//! A kernel's arguments from its rows, for the backends without reflection (CUDA, the CPU; decision 0400 §6): UKL's
//! shims emit beside each kernel the text of its `DEFINE_KERNEL2` rows (`Name__ukl_params`) and each row's type's
//! size, alignment and kind (`Name__ukl_rows`), and beside each `ARGS` structure its fields' names
//! (`Type__ukl_fields`) and their layout (`Type__ukl_layout`). Read here, they give the same
//! `GridComputeFunctionArgument`s Metal's reflection gives, so the grid binds a kernel the same way on every backend.

use crate::compute_kernel::*;

/// A kernel's arguments, and each launch parameter's size in the kernel's order, from its rows' text and layout
/// numbers. `struct_type` gives an `ARGS` row's structure by its type's name.
pub fn arguments(
    function_name: &str,
    text: &str,
    rows: &[u64],
    mut struct_type: impl FnMut(&str) -> Result<KernelBaseType, String>,
) -> Result<(Vec<GridComputeFunctionArgument>, Vec<usize>), String> {
    let fields = split_top(text);
    if fields.len() % 4 != 0 || rows.len() != fields.len() / 4 * 3 {
        return Err(format!("{function_name}: {} row fields and {} layout numbers", fields.len(), rows.len()));
    }
    let mut arguments = Vec::new();
    let mut param_sizes = Vec::new();
    for (r, row) in fields.chunks(4).enumerate() {
        let (access, ty, name) = (row[0], row[1], row[2]);
        let (size, align, code) = (rows[3 * r], rows[3 * r + 1], rows[3 * r + 2]);
        let index = param_sizes.len() as u64;
        let atom = || KernelBaseType::Atom { name: ty.to_string(), width: size, align };
        // A buffer's element: a scalar as Metal reflects one, else opaque memory of its size.
        let element = || basic_type(code, size).unwrap_or_else(atom);
        let buffer = |access| ComputeKernelType { base_type: element(), dims: 1, access };
        let (disposition, kt, param) = match access {
            "ROBUFFER" | "CONSTBUFFER" => (GridComputeFunctionArgumentDisposition::Buffer, buffer(MemoryRegionAccess::Read), 8),
            "RWBUFFER" => (GridComputeFunctionArgumentDisposition::Buffer, buffer(MemoryRegionAccess::ReadWrite), 8),
            "WOBUFFER" => (GridComputeFunctionArgumentDisposition::Buffer, buffer(MemoryRegionAccess::Write), 8),
            "ARGS" => (GridComputeFunctionArgumentDisposition::Buffer, ComputeKernelType { base_type: struct_type(ty)?, dims: 0, access: MemoryRegionAccess::Read }, size),
            "LITERAL" | "TUNEABLE" => {
                let b = basic_type(code, size).ok_or_else(|| format!("{function_name}: {name} is a literal of a type that is not a scalar"))?;
                (GridComputeFunctionArgumentDisposition::Buffer, ComputeKernelType { base_type: b, dims: 0, access: MemoryRegionAccess::Read }, size)
            }
            "RWLOCAL" => (GridComputeFunctionArgumentDisposition::Threadgroup, ComputeKernelType { base_type: atom(), dims: 0, access: MemoryRegionAccess::ReadWrite }, 4),
            // Positions, sizes, lanes and the kernel's own threadgroup storage: no argument.
            _ => continue,
        };
        arguments.push(GridComputeFunctionArgument { name: name.to_string(), compute_function_arg_index: index, ty: kt, disposition });
        param_sizes.push(param as usize);
    }
    Ok((arguments, param_sizes))
}

/// An `ARGS` structure `ty` from its fields' names (`Type__ukl_fields`, as written) and its layout (`Type__ukl_layout`'s
/// numbers: the size and alignment, then each field's offset, size and kind). Refused unless the fields cover every
/// byte that is not padding (a field left out of `UKL_REFLECT_STRUCT` would go unfilled).
pub fn struct_type(ty: &str, names: &str, layout: &[u64]) -> Result<KernelBaseType, String> {
    let names: Vec<String> = split_top(names).into_iter().map(|s| s.to_string()).collect();
    if layout.len() < 2 + 3 * names.len() {
        return Err(format!("{ty}: {} layout numbers for {} fields", layout.len(), names.len()));
    }
    let (width, align) = (layout[0], layout[1]);
    let mut fields = Vec::new();
    for (i, name) in names.into_iter().enumerate() {
        let (offset, size, code) = (layout[2 + 3 * i], layout[3 + 3 * i], layout[4 + 3 * i]);
        fields.push(StructField { name, offset, ty: basic_type(code, size).ok_or_else(|| format!("{ty}: field {i} is not a scalar"))? });
    }
    // In offset order, each field starts at the end of the last rounded up to its own alignment, and the structure
    // ends at the last field's end rounded up to the structure's.
    let mut sorted: Vec<&StructField> = fields.iter().collect();
    sorted.sort_by_key(|f| f.offset);
    let mut end = 0u64;
    for f in &sorted {
        if f.offset != end.next_multiple_of(f.ty.align()) {
            return Err(format!("{ty}: bytes {end}..{} are no reflected field's; name every field in UKL_REFLECT_STRUCT", f.offset));
        }
        end = f.offset + f.ty.width();
    }
    if end.next_multiple_of(align) != width {
        return Err(format!("{ty}: bytes {end}..{width} are no reflected field's; name every field in UKL_REFLECT_STRUCT"));
    }
    Ok(KernelBaseType::Struct { name: ty.to_string(), align: Some(align), width: Some(width), fields })
}

/// The number of fields `Type__ukl_fields` names: its layout has two numbers and three a field.
pub fn layout_len(names: &str) -> usize {
    2 + 3 * split_top(names).len()
}

/// The scalar type of a kind and size (`ukl_type_code`), named as the kernel registrations spell it.
pub fn basic_type(code: u64, size: u64) -> Option<KernelBaseType> {
    let name = match (code as u8, size) {
        (b'f', 4) => "f32",
        (b'd', 8) => "f64",
        (b'b', 1) => "bool",
        (b'i', 1) => "i8",
        (b'i', 2) => "i16",
        (b'i', 4) => "i32",
        (b'i', 8) => "i64",
        (b'u', 1) => "u8",
        (b'u', 2) => "u16",
        (b'u', 4) => "u32",
        (b'u', 8) => "u64",
        _ => return None,
    };
    Some(KernelBaseType::Basic { ty: BasicType { name, width: size }, simd: vec![1] })
}

/// A list's items at its top level: split at commas outside quotes, parentheses and brackets, each trimmed.
pub fn split_top(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut quoted, mut start) = (Vec::new(), 0i32, false, 0);
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' if quoted => i += 1,
            b'"' => quoted = !quoted,
            b'(' | b'[' | b'<' if !quoted => depth += 1,
            b')' | b']' | b'>' if !quoted => depth -= 1,
            b',' if !quoted && depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(s[start..].trim());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_split_at_their_top_level_commas() {
        let v = split_top(r#"ARGS, A, args, , ROBUFFER, uint32_t, x, "[a, b]", GID0, uint32_t, id,"#);
        assert_eq!(v, ["ARGS", "A", "args", "", "ROBUFFER", "uint32_t", "x", "\"[a, b]\"", "GID0", "uint32_t", "id", ""]);
    }
}
