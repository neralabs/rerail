//! Output: error records from error masks. This is where set bits become positions, as
//! simdjson's indexer turns structural bits into indexes; nothing reads the records back.

use super::interp::{Batch, Record};

/// Records of `parts` ((message, mask)) per file. `all`: every set byte, by position across the
/// parts; otherwise each part's first byte. Without `byte`, one record per part and file.
pub(in crate::code_rule_dsl) fn records(batch: &Batch, parts: &[(&str, &Vec<u64>)], all: bool, byte: bool) -> Vec<Record> {
    let mut out = vec![];
    for (f, &(first, blocks, _)) in batch.files.iter().enumerate() {
        let words = first..first + blocks;
        if all {
            let mut hits: Vec<(usize, usize)> = vec![];
            for (k, (_, m)) in parts.iter().enumerate() {
                for b in words.clone() {
                    let mut w = m[b];
                    while w != 0 {
                        hits.push(((b - first) * 64 + w.trailing_zeros() as usize, k));
                        w &= w - 1;
                    }
                }
            }
            hits.sort();
            for (at, k) in hits {
                out.push((f, if byte { format!("{} (byte {at})", parts[k].0) } else { parts[k].0.to_string() }));
            }
        } else {
            for (msg, m) in parts {
                if let Some(b) = words.clone().find(|&b| m[b] != 0) {
                    let at = (b - first) * 64 + m[b].trailing_zeros() as usize;
                    out.push((f, if byte { format!("{msg} (byte {at})") } else { msg.to_string() }));
                }
            }
        }
    }
    out
}
