//! The parser's Cranelift IR as Rust source: the translator is `rerail_clif::rust` (decision 0310), which the
//! tape's reads share; what is the compiler's own is tier 1's source of a compiled parser.

pub use rerail_clif::rust::*;
use std::fmt::Write;

/// Tier 1's source, from a parser compiled with `native_cpu_stream::KEEP_IR`: `tier1_entry_K`
/// a pass, the record pass's tables as data, `TABLES_DATA`; it ends with its fingerprint,
/// `SOURCE`.
/// (Tier 2's is `rust_stream`'s, from the native IR.)
pub fn rust_source(parser: &crate::native_cpu_stream::Parser) -> Result<String, String> {
    let t1 = &parser.tier1;
    let names: Vec<String> = (0..t1.entry_ids.len()).map(|k| format!("tier1_entry_{k}")).collect();
    let entries: Vec<(u32, &str)> = t1.entry_ids.iter().zip(&names).map(|(&id, n)| (id, n.as_str())).collect();
    // The tables as the binary's data, as the lean lexer's: an address in them is a constant
    // the code makes again where it needs it (`adrp`), not a register or a load.
    let words = t1.tables_words().unwrap_or(&[]);
    let addresses: Vec<(i64, i64, &str, Option<&[u64]>)> = t1.tables_span().map(|(a, n)| (a, n, "TABLES", Some(words))).into_iter().collect();
    let mut one = translate(&t1.ir, &entries, &addresses).map_err(|e| format!("tier 1: {e}"))?;
    writeln!(one, "/// The passes' entries, in order.\npub const ENTRIES: &[unsafe extern \"C\" fn(u64, u64, u64, u64, u64, u64, u64)] = &[{}];", names.join(", ")).unwrap();
    let h = fingerprint(&one);
    Ok(format!("{one}\n/// This source's fingerprint (`clif_rust::fingerprint` of what comes before).\npub const SOURCE: u64 = {h:#x};\n"))
}

