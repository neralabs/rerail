//! Format 16's shipped tables as text (`tables/<tag>.tsv`, the lab's `tape16 table`), and what the engine reads of
//! them as constants (decision 0697): each language's codes' kind atoms and field atoms, the views' `kind` and `field`
//! columns, laid out once in the binary (`dir`) and read in place by every reader: the host's view reads
//! (`view_reads::Doc`), the JIT's and the kernels' (`rerail_clif::view_reads::CodesDir`, a home of the views'
//! directory). Nothing makes them per document, batch or run.
//!
//! This file has no dependency but `std`: the crate's build script includes it (`#[path]`) to write the directory's
//! image from the same tables, by the same lines and atoms the crate reads them by. One definition of each.

/// A code's flags (`Entry::flags`).
pub const CODE_EXTRA: u8 = 1;
pub const CODE_ERROR: u8 = 2;
pub const CODE_MISSING: u8 = 4;

/// Kind codes per dialect in a kind atom (the tape's `KIND_LIMIT`).
pub const KIND_STRIDE: u32 = 1024;
/// Tree-sitter's ERROR symbol, past every grammar's kinds and so past the stride.
pub const ERROR_SYMBOL: u16 = u16::MAX;

/// The kind atom of `code` in dialect `tag`: the `kind` column's value. ERROR is the dialect's code 0.
pub const fn kind_atom(tag: u8, code: u16) -> u32 {
    tag as u32 * KIND_STRIDE + if code == ERROR_SYMBOL { 0 } else { code as u32 }
}

/// The field atom of field `code` in dialect `tag`: the `field` column's value.
pub const fn field_atom(tag: u8, code: u16) -> u32 {
    (tag as u32) << 16 | code as u32
}

/// A line of a table's text (`f16::Lang::to_text`), its names unescaped.
pub enum Line<'a> {
    Revision(u8),
    Widths([usize; 3]),
    /// A code: kind, field, flags, alias (`(name, named)`), classes.
    Code { kind: u16, field: u16, flags: u8, alias: Option<(String, bool)>, classes: Vec<u16> },
    /// A kind id, its namedness and name.
    Kind(u16, bool, String),
    /// A lookup: namedness, name, the id it gives.
    Lookup(bool, String, u16),
    Field(u16, String),
    Class(u16, String),
    Supertype(String, u16),
    /// A comment or an empty line.
    None(&'a str),
}

/// Line `line` of a table's text, or the line itself where it is no line of the format.
pub fn line(line: &str) -> Result<Line<'_>, &str> {
    if line.is_empty() || line.starts_with('#') {
        return Ok(Line::None(line));
    }
    let c: Vec<&str> = line.split('\t').collect();
    let num = |s: &str| s.parse::<u64>().map_err(|_| line);
    let name = |s: &str| unescaped(s).ok_or(line);
    Ok(match c[0] {
        "revision" if c.len() == 2 => Line::Revision(num(c[1])? as u8),
        "widths" if c.len() == 4 => Line::Widths([num(c[1])? as usize, num(c[2])? as usize, num(c[3])? as usize]),
        "code" if c.len() == 6 => {
            let alias = match c[4] {
                "-" => None,
                a => {
                    let (named, n) = a.split_once(':').ok_or(line)?;
                    Some((n.to_owned(), named == "1"))
                }
            };
            let classes = if c[5] == "-" { Vec::new() } else { c[5].split(',').map(|x| x.parse::<u16>().map_err(|_| line)).collect::<Result<_, _>>()? };
            Line::Code { kind: num(c[1])? as u16, field: num(c[2])? as u16, flags: num(c[3])? as u8, alias, classes }
        }
        "kind" if c.len() == 4 => Line::Kind(num(c[1])? as u16, c[2] == "1", name(c[3])?),
        "lookup" if c.len() == 4 => Line::Lookup(c[1] == "1", name(c[2])?, num(c[3])? as u16),
        "field" if c.len() == 3 => Line::Field(num(c[1])? as u16, name(c[2])?),
        "class" if c.len() == 3 => Line::Class(num(c[1])? as u16, name(c[2])?),
        "supertype" if c.len() == 3 => Line::Supertype(name(c[1])?, num(c[2])? as u16),
        _ => return Err(line),
    })
}

/// A name in a table's text: `\\`, `\n`, `\r`, `\t` and `\0` escaped (C's and Go's tokens hold a newline and a NUL).
pub fn escaped(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            '\0' => s.push_str("\\0"),
            c => s.push(c),
        }
    }
    s
}

/// `escaped`'s inverse; `None` for an escape it does not write.
pub fn unescaped(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next()? {
            '\\' => '\\',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '0' => '\0',
            _ => return None,
        });
    }
    Some(out)
}

/// The languages the directory has a place for: every tape tag.
pub const LANGS: usize = 256;
/// A language's place in the directory: its kinds' and its fields' offsets from the directory's first byte, u32s
/// (`rerail_clif::view_reads::CodesLang`); 0 for a tag with no table.
pub const LANG_BYTES: usize = 8;
/// The directory's alignment and its length's multiple: a page of every host the device wraps it on (Apple's 16 KiB),
/// as a device's wrap of the host's memory in place takes it (`rerail_device::device::wrap_host_memory`).
pub const ALIGN: usize = 16 << 10;

/// Each code's kind atom and field atom, by number, of the table `text` of dialect `tag`. A code's kind is its
/// name's canonical code (`lookup` of its name as a named kind, where it is named and the lookup gives one: the
/// grammar's `id_for_node_kind`, what a program's constant lowers to), as the `kind` column holds it; ERROR's is the
/// dialect's code 0.
pub fn atoms(tag: u8, text: &str) -> Result<(Vec<u32>, Vec<u32>), String> {
    let (mut codes, mut kinds, mut lookups) = (Vec::new(), std::collections::HashMap::new(), std::collections::HashMap::new());
    for l in text.lines() {
        match line(l).map_err(|l| format!("table {tag}: {l}"))? {
            Line::Code { kind, field, flags, .. } => codes.push((kind, field, flags)),
            Line::Kind(id, named, name) => {
                kinds.insert(id, (named, name));
            }
            Line::Lookup(true, name, id) => {
                lookups.insert(name, id);
            }
            _ => {}
        }
    }
    let canonical = |k: u16| match kinds.get(&k) {
        Some((true, name)) => lookups.get(name).copied().filter(|&c| c != 0).unwrap_or(k),
        _ => k,
    };
    let kind = codes.iter().map(|&(k, _, flags)| kind_atom(tag, if flags & CODE_ERROR != 0 { ERROR_SYMBOL } else { canonical(k) })).collect();
    let field = codes.iter().map(|&(_, f, _)| field_atom(tag, f)).collect();
    Ok((kind, field))
}

/// The directory's image (`dir`): each language's place (`LANG_BYTES`, by tag), then each table's kinds and fields
/// (`atoms`), in tag order, its length a multiple of `ALIGN`.
pub fn image(tables: &[(u8, &str)]) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; LANGS * LANG_BYTES];
    for &(tag, text) in tables {
        let (kinds, fields) = atoms(tag, text)?;
        for (k, run) in [kinds, fields].iter().enumerate() {
            let at = u32::try_from(out.len()).map_err(|_| "a directory past 4 GiB")?;
            let place = usize::from(tag) * LANG_BYTES + 4 * k;
            out[place..place + 4].copy_from_slice(&at.to_le_bytes());
            out.extend(run.iter().flat_map(|v| v.to_le_bytes()));
        }
    }
    out.resize(out.len().next_multiple_of(ALIGN), 0);
    Ok(out)
}
